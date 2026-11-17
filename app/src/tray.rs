//! The menu-bar companion: the menu under Dispatch's menu-bar icon. Its content
//! comes from `dispatch::companion::Companion`; this file only turns it into a native menu
//! and handles the clicks.
//!
//! The menu is rebuilt when something changed (`State::tray_dirty`, checked every 2 s) and
//! when a minute has passed and its text would read differently ("today 13:30" → "Fri 13:30").

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use dispatch::cite::Format;
use dispatch::companion::{Badge, Companion};
use dispatch::store::SourceStatus;
use serde::Serialize;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::{State, TRAY_ICON, citation_text, show_main, start_refresh};

pub const TRAY: &str = "dispatch";

/// macOS menus treat `&` as a shortcut marker; `&&` shows one `&`.
fn label(s: &str) -> String {
    s.replace('&', "&&")
}

/// "13:30" today, "Fri 13:30" within a week, else "3 Nov".
fn when(t: DateTime<Utc>) -> String {
    let t = t.with_timezone(&Local);
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if (t - now).num_days().abs() < 6 {
        t.format("%a %H:%M").to_string()
    } else {
        t.format("%-d %b").to_string()
    }
}

fn source_line(s: &SourceStatus) -> String {
    match (&s.last_error, s.last_ok) {
        (Some(e), _) => format!("🔴 {}: {e}", s.name),
        (None, Some(t)) => format!(
            "🟢 {} · {} items · {}",
            s.name,
            s.last_count.unwrap_or(0),
            when(t)
        ),
        (None, None) => format!("⚪ {} · not fetched yet", s.name),
    }
}

fn info(app: &AppHandle, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::new(app, label(text), false, None::<&str>)
}

fn item(app: &AppHandle, id: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, label(text), true, None::<&str>)
}

/// The whole menu. `c` is None before the first build (just Open, Refresh and Quit).
pub fn tray_menu(app: &AppHandle, c: Option<&Companion>) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let sep = || PredefinedMenuItem::separator(app);
    menu.append(&item(app, "open", "Open Dispatch")?)?;
    menu.append(&item(app, "search", "Search the archive…")?)?;
    let refreshing = app.state::<State>().refreshing.load(Ordering::SeqCst);
    if let Some(c) = c {
        menu.append(&sep()?)?;
        let unread = |n: i64| {
            if n == 0 {
                "all read".to_owned()
            } else {
                format!("{n} unread")
            }
        };
        menu.append(&item(
            app,
            "go:research",
            &format!("Research · {}", unread(c.research_unread)),
        )?)?;
        menu.append(&item(
            app,
            "go:news",
            &format!("News · {}", unread(c.news_unread)),
        )?)?;
        if c.weekly_goal > 0 {
            menu.append(&info(
                app,
                &format!("This week: {} / {} papers", c.week_read, c.weekly_goal),
            )?)?;
        }

        for d in &c.deadlines {
            menu.append(&info(app, &format!("⏰ {} · {}", d.name, d.when))?)?;
        }

        menu.append(&sep()?)?;
        if c.arxiv_weekend {
            menu.append(&info(app, "No new arXiv papers today (weekend)")?)?;
        }
        menu.append(&info(
            app,
            if c.top_is_latest {
                "Today's papers"
            } else {
                "Suggested papers"
            },
        )?)?;
        if c.top.is_empty() {
            menu.append(&info(app, "    All caught up")?)?;
        }
        for p in &c.top {
            let k = &p.key;
            let title = if p.followed {
                format!("👤 {}", p.menu_title)
            } else {
                p.menu_title.clone()
            };
            let sub = Submenu::new(app, label(&title), true)?;
            sub.append(&info(app, &p.authors)?)?;
            sub.append(&item(app, &format!("top:show:{k}"), "Show in Dispatch")?)?;
            sub.append(&item(app, &format!("top:link:{k}"), "Open arXiv page")?)?;
            if p.pdf.is_some() {
                sub.append(&item(app, &format!("top:pdf:{k}"), "Open PDF")?)?;
            }
            sub.append(&sep()?)?;
            sub.append(&item(app, &format!("top:bibtex:{k}"), "Copy BibTeX")?)?;
            sub.append(&item(
                app,
                &format!("top:save:{k}"),
                if p.saved {
                    "★ Saved (remove)"
                } else {
                    "☆ Save for later"
                },
            )?)?;
            if !p.read {
                sub.append(&item(app, &format!("top:read:{k}"), "Mark read")?)?;
            }
            menu.append(&sub)?;
        }

        // The paper you opened last, and the ones you saved last.
        if c.last_opened.is_some() || !c.recent_saved.is_empty() {
            menu.append(&sep()?)?;
        }
        if let Some(p) = &c.last_opened {
            menu.append(&item(
                app,
                &format!("top:bibtex:{}", p.key),
                &format!("Copy BibTeX: {}", p.menu_title),
            )?)?;
        }
        if !c.recent_saved.is_empty() {
            let saved = Submenu::new(app, "Recently saved", true)?;
            for p in &c.recent_saved {
                let sub = Submenu::new(app, label(&p.menu_title), true)?;
                sub.append(&item(
                    app,
                    &format!("top:show:{}", p.key),
                    "Show in Dispatch",
                )?)?;
                sub.append(&item(
                    app,
                    &format!("top:link:{}", p.key),
                    "Open arXiv page",
                )?)?;
                sub.append(&item(app, &format!("top:bibtex:{}", p.key), "Copy BibTeX")?)?;
                saved.append(&sub)?;
            }
            menu.append(&saved)?;
        }

        menu.append(&sep()?)?;
        let n = c.sources.len();
        let title = match (c.failed, n) {
            (_, 0) => "Sources: none fetched yet".to_owned(),
            (0, n) => format!("🟢 Sources: all {n} ok"),
            (f, n) => format!("🔴 Sources: {f} of {n} failed"),
        };
        let sources = Submenu::new(app, label(&title), true)?;
        for s in &c.sources {
            if s.last_error.is_some() {
                // A failed source can be fetched again on its own.
                sources.append(&MenuItem::with_id(
                    app,
                    format!("retry:{}", s.source),
                    label(&format!("{} · Retry", source_line(s))),
                    !refreshing,
                    None::<&str>,
                )?)?;
            } else {
                sources.append(&info(app, &source_line(s))?)?;
            }
        }
        sources.append(&sep()?)?;
        sources.append(&item(app, "sources", "Show in Dispatch")?)?;
        menu.append(&sources)?;
        menu.append(&info(
            app,
            &match c.last_ok {
                Some(t) => format!("Last refresh {}", when(t)),
                None => "Not refreshed yet".into(),
            },
        )?)?;
        menu.append(&info(
            app,
            &format!(
                "Next: arXiv {} · news {}",
                when(c.research_next),
                when(c.news_next)
            ),
        )?)?;
    }
    if let Some(paused) = notifications_menu(app)? {
        menu.append(&paused)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        "refresh",
        if refreshing {
            "Refreshing…"
        } else {
            "Refresh now"
        },
        !refreshing,
        None::<&str>,
    )?)?;
    menu.append(&sep()?)?;
    menu.append(&item(app, "quit", "Quit Dispatch")?)?;
    Ok(menu)
}

/// Pause notifications for an hour or until tomorrow morning, or resume. Only when
/// some notification is switched on. (macOS holds them back during a Focus by itself.)
fn notifications_menu(app: &AppHandle) -> tauri::Result<Option<Submenu<Wry>>> {
    let state = app.state::<State>();
    let Ok(s) = state.settings.lock().map(|s| s.clone()) else {
        return Ok(None);
    };
    if !(s.notify_authors || s.notify_keywords || s.notify_updates) {
        return Ok(None);
    }
    let now = Utc::now();
    let sub = match s.notify_paused_until.filter(|t| *t > now) {
        Some(t) => {
            let sub = Submenu::new(
                app,
                label(&format!("Notifications paused until {}", when(t))),
                true,
            )?;
            sub.append(&item(app, "pause:off", "Resume notifications")?)?;
            sub
        }
        None => {
            let sub = Submenu::new(app, "Pause notifications", true)?;
            sub.append(&item(app, "pause:hour", "For an hour")?)?;
            sub.append(&item(
                app,
                "pause:tomorrow",
                "Until tomorrow morning (08:00)",
            )?)?;
            sub
        }
    };
    Ok(Some(sub))
}

/// Until when "Pause" holds notifications back.
fn pause_until(choice: &str, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    match choice {
        "hour" => Some((now + chrono::Duration::hours(1)).with_timezone(&Utc)),
        "tomorrow" => (now.date_naive() + chrono::Duration::days(1))
            .and_hms_opt(8, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).earliest())
            .map(|t| t.with_timezone(&Utc)),
        _ => None,
    }
}

fn set_pause(app: &AppHandle, choice: &str) {
    let state = app.state::<State>();
    if let Ok(mut s) = state.settings.lock() {
        s.notify_paused_until = pause_until(choice, Local::now());
        let _ = s.save(&state.dir.join(dispatch::settings::FILE));
    }
    state.tray_dirty.store(true, Ordering::SeqCst);
    let _ = app.emit("settings", ());
}

/// Fetches one failed source again, in the background (one at a time, like a
/// refresh).
fn retry_source(app: &AppHandle, source: &str) {
    let state = app.state::<State>();
    if state.refreshing.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = app.emit("refreshing", true);
    state.tray_dirty.store(true, Ordering::SeqCst);
    let (app, source) = (app.clone(), source.to_owned());
    std::thread::spawn(move || {
        let state = app.state::<State>();
        let topics = state.topics.lock().map(|t| t.clone()).unwrap_or_default();
        let result = dispatch::Store::open(&state.db()).and_then(|mut store| {
            let http = dispatch::fetch::Http::new()?;
            dispatch::refresh::retry(&mut store, &topics, &http, Utc::now(), &source)
        });
        state.refreshing.store(false, Ordering::SeqCst);
        state.tray_dirty.store(true, Ordering::SeqCst);
        let text = match &result {
            Ok(r) if r.error.is_none() => format!("{} ok", r.name),
            Ok(r) => format!("{} failed again", r.name),
            Err(_) => "Couldn't retry".into(),
        };
        let _ = app.emit("refreshed", ());
        flash(&app, &text);
    });
}

/// What the window should show after a menu click.
#[derive(Clone, Serialize)]
struct OpenItem {
    key: String,
    section: String,
}

pub fn on_tray_menu(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main(app),
        "search" => {
            show_main(app);
            let _ = app.emit("show-view", "search");
        }
        "go:research" | "go:news" => {
            show_main(app);
            let _ = app.emit("show-view", &id[3..]);
        }
        "sources" => {
            show_main(app);
            let _ = app.emit("show-sources", ());
        }
        "refresh" => {
            start_refresh(app, dispatch::refresh::Scope::All);
        }
        "quit" => app.exit(0),
        _ if id.starts_with("pause:") => set_pause(app, &id[6..]),
        _ if id.starts_with("retry:") => retry_source(app, &id[6..]),
        _ => {
            if let Some(rest) = id.strip_prefix("top:")
                && let Some((action, key)) = rest.split_once(':')
            {
                paper_action(app, action, key);
            }
        }
    }
}

fn paper_action(app: &AppHandle, action: &str, key: &str) {
    let state = app.state::<State>();
    let item = state
        .store
        .lock()
        .ok()
        .and_then(|s| s.get(key).ok().flatten());
    let Some(item) = item else { return };
    match action {
        "show" => {
            show_main(app);
            let _ = app.emit(
                "open-item",
                OpenItem {
                    key: key.to_owned(),
                    section: item.section.as_str().to_owned(),
                },
            );
        }
        "link" | "pdf" => {
            let url = if action == "pdf" {
                item.pdf
            } else {
                Some(item.link)
            };
            if let Some(url) = url.filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            {
                let _ = app.opener().open_url(url, None::<&str>);
                // For Stats' "opened".
                if let Ok(store) = state.store.lock() {
                    let _ = store.set_opened(key, Utc::now());
                }
            }
        }
        "bibtex" => {
            let app = app.clone();
            let keys = vec![key.to_owned()];
            tauri::async_runtime::spawn(async move {
                let text = match citation_text(&app, keys, Format::Bibtex).await {
                    Ok((text, _)) => app
                        .clipboard()
                        .write_text(text)
                        .map(|_| "BibTeX copied")
                        .unwrap_or("Couldn't copy"),
                    Err(_) => "Couldn't copy",
                };
                flash(&app, text);
            });
        }
        "save" | "read" => {
            if let Ok(mut store) = state.store.lock() {
                let now = Utc::now();
                let keys = [key.to_owned()];
                let _ = if action == "save" {
                    let saved = store.user(key).ok().flatten().and_then(|u| u.saved_at);
                    store.set_saved(&keys, saved.is_none(), now)
                } else {
                    store.set_read(key, true, now)
                };
            }
            state.changed();
            // The window (if open) reloads its lists and counts.
            let _ = app.emit("refreshed", ());
        }
        _ => {}
    }
}

/// A short message beside the icon (the menu has closed, and the window may be hidden),
/// cleared by the next rebuild.
pub(crate) fn flash(app: &AppHandle, text: &str) {
    if let Some(tray) = app.tray_by_id(TRAY) {
        let _ = tray.set_title(Some(text));
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        app.state::<State>()
            .tray_dirty
            .store(true, Ordering::SeqCst);
    });
}

/// Rebuilds the menu if it changed, plus the count beside the icon.
fn rebuild(
    app: &AppHandle,
    last: &mut String,
    last_icon: &mut Option<(Badge, bool)>,
    last_status: &mut crate::status::Last,
) {
    let state = app.state::<State>();
    let topics = state.topics.lock().map(|t| t.clone()).unwrap_or_default();
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let built = match state.store.lock() {
        Ok(store) => Companion::build(&store, &topics, &settings, Utc::now()),
        Err(_) => return,
    };
    let c = match built {
        Ok(mut c) => {
            let today = chrono::Local::now().date_naive();
            c.deadlines = dispatch::deadlines::upcoming(
                &dispatch::deadlines::load(&state.dir).unwrap_or_default(),
                today,
                3,
            );
            c
        }
        Err(e) => {
            eprintln!("dispatch: menu-bar menu: {e}");
            return;
        }
    };
    // The same numbers for SketchyBar.
    crate::status::export(app, &c, last_status);
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    let _ = tray.set_title(c.icon_title(settings.menu_bar_count));
    let badge = c.badge(settings.menu_bar_count, settings.menu_bar_alert);
    let dark = badge == Badge::Alert && menu_bar_dark();
    if *last_icon != Some((badge, dark)) {
        let template = badge != Badge::Alert;
        if let Ok(image) = icon(badge, dark) {
            let _ = tray.set_icon(Some(image));
            let _ = tray.set_icon_as_template(template);
            *last_icon = Some((badge, dark));
        }
    }
    // Same content and same day: leave the menu alone (replacing it while it is open would
    // be wasted work).
    let print = format!(
        "{c:?}{}{}{:?}{}",
        Local::now().date_naive(),
        state.refreshing.load(Ordering::SeqCst),
        settings.notify_paused_until,
        settings.notifications_paused(Utc::now())
    );
    if *last == print {
        return;
    }
    if let Ok(menu) = tray_menu(app, Some(&c)) {
        let _ = tray.set_menu(Some(menu));
        *last = print;
    }
}

pub fn spawn_tray(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last = String::new();
        let mut last_icon = None;
        let mut last_log = None;
        let mut last_status = None;
        let mut minute = Instant::now();
        loop {
            let dirty = app
                .state::<State>()
                .tray_dirty
                .swap(false, Ordering::SeqCst);
            if dirty || minute.elapsed() >= Duration::from_secs(60) {
                if !dirty {
                    // Times only: force a rebuild if the day changed (handled by `print`).
                    minute = Instant::now();
                }
                rebuild(&app, &mut last, &mut last_icon, &mut last_status);
                // The Habit Tracker link (Stats): only rewritten when its counts changed.
                crate::stats::export_reading_log(&app, &mut last_log);
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

// ---- the icon's states ---------------------------------------------------

/// The menu-bar icon with its mark. A plain or dotted icon stays a template (macOS colours
/// it for a light or dark menu bar); the red dot can't be a template, so the glyph is drawn
/// in the menu bar's text colour itself.
fn icon(badge: Badge, dark: bool) -> tauri::Result<Image<'static>> {
    let base = Image::from_bytes(TRAY_ICON)?;
    let (w, h) = (base.width(), base.height());
    let mut px = base.rgba().to_vec();
    if badge == Badge::None {
        return Ok(Image::new_owned(px, w, h));
    }
    // A dot in the top right corner, with a clear ring so it stands apart from the glyph.
    let r = w as f32 * 0.15;
    let (cx, cy) = (w as f32 - r - 1.0, r + 1.0);
    let ink: [u8; 3] = if dark { [255, 255, 255] } else { [0, 0, 0] };
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            if badge == Badge::Alert {
                px[i..i + 3].copy_from_slice(&ink);
            }
            if d <= r {
                let colour = if badge == Badge::Alert {
                    [255, 59, 48]
                } else {
                    [0, 0, 0]
                };
                px[i..i + 3].copy_from_slice(&colour);
                px[i + 3] = 255;
            } else if d <= r + 2.5 {
                px[i + 3] = 0;
            }
        }
    }
    Ok(Image::new_owned(px, w, h))
}

/// Dark mode on (the menu bar's text is white). Read from the global preferences, as
/// `defaults read -g AppleInterfaceStyle` does.
#[cfg(target_os = "macos")]
fn menu_bar_dark() -> bool {
    use core_foundation_sys::base::{CFRelease, kCFAllocatorDefault};
    use core_foundation_sys::preferences::{
        CFPreferencesAppSynchronize, CFPreferencesCopyAppValue, kCFPreferencesAnyApplication,
    };
    use core_foundation_sys::string::{CFStringCreateWithBytes, kCFStringEncodingUTF8};
    let name = b"AppleInterfaceStyle";
    // SAFETY: plain CoreFoundation calls; every object created or copied here is released.
    unsafe {
        let key = CFStringCreateWithBytes(
            kCFAllocatorDefault,
            name.as_ptr(),
            name.len() as _,
            kCFStringEncodingUTF8,
            0,
        );
        if key.is_null() {
            return false;
        }
        CFPreferencesAppSynchronize(kCFPreferencesAnyApplication);
        let value = CFPreferencesCopyAppValue(key, kCFPreferencesAnyApplication);
        CFRelease(key.cast());
        if value.is_null() {
            return false; // the key only exists in dark mode
        }
        CFRelease(value);
        true
    }
}

#[cfg(not(target_os = "macos"))]
fn menu_bar_dark() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_until_an_hour_or_tomorrow_at_eight() {
        let now: DateTime<Local> = "2026-11-03T22:15:00+08:00"
            .parse::<DateTime<chrono::FixedOffset>>()
            .unwrap()
            .with_timezone(&Local);
        assert_eq!(
            pause_until("hour", now),
            Some((now + chrono::Duration::hours(1)).with_timezone(&Utc))
        );
        let t = pause_until("tomorrow", now).unwrap().with_timezone(&Local);
        assert_eq!(t.format("%H:%M").to_string(), "08:00");
        assert_eq!(t.date_naive(), now.date_naive() + chrono::Duration::days(1));
        assert_eq!(pause_until("off", now), None);
    }

    #[test]
    fn icons_keep_their_size_and_get_a_dot() {
        let plain = icon(Badge::None, false).unwrap();
        let dot = icon(Badge::Dot, false).unwrap();
        let red = icon(Badge::Alert, true).unwrap();
        assert_eq!((dot.width(), dot.height()), (plain.width(), plain.height()));
        // The dot's centre is opaque; the red one is red.
        let w = red.width() as usize;
        let r = (w as f32 * 0.15) as usize;
        let i = ((r + 1) * w + (w - r - 1)) * 4;
        assert_eq!(dot.rgba()[i + 3], 255);
        assert_eq!(&red.rgba()[i..i + 3], &[255, 59, 48]);
    }
}
