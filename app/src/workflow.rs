//! In the shell: the live `.bib` file and the Markdown notes folder (kept in step
//! by a background thread whenever saved papers, notes or tags change), LaTeX snippets, the
//! week's reading log, PDF hand-off and the PDF app list. The logic is in
//! `dispatch::{bibfile, vault, pdf}`.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::{Datelike, Local, TimeZone, Utc};
use dispatch::bibfile::{self, KeyStyle};
use dispatch::{Item, Store, deadlines, documents, fetch, pdf, vault};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::State;

/// What the last export did, for Settings → Research workflow.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExportStatus {
    /// "Wrote 42 papers at 14:02", or the problem.
    pub bib: Option<String>,
    pub bib_ok: bool,
    pub notes: Option<String>,
    pub notes_ok: bool,
}

/// Every 3 s: if something changed, bring the `.bib` and the notes folder up to date.
pub fn start_thread(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let mut deadlines_seen = None;
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let state = app.state::<State>();
            if state.export_dirty.swap(false, Ordering::SeqCst) {
                export(&app);
            }
            // deadlines.json edited by hand: redraw the menu.
            let now = std::fs::metadata(state.dir.join(deadlines::FILE))
                .and_then(|m| m.modified())
                .ok();
            if now != deadlines_seen {
                deadlines_seen = now;
                state.tray_dirty.store(true, Ordering::SeqCst);
            }
        }
    });
}

fn export(app: &AppHandle) {
    let state = app.state::<State>();
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let bib = settings.bib_path();
    if bib.is_none() && settings.notes_dir.is_empty() {
        return;
    }
    let Ok(store) = Store::open(&state.db()) else {
        return;
    };
    let when = Local::now().format("%H:%M");
    let mut status = state
        .export_status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    if let Some(bib) = bib {
        let tag = Some(settings.bib_tag.as_str()).filter(|t| !t.is_empty());
        let result = store.list_saved(tag, 100_000).and_then(|rows| {
            let items: Vec<Item> = rows.into_iter().map(|r| r.item).collect();
            let text = bibfile::render(&items, settings.cite_keys);
            bibfile::write(&bib, &text).map(|_| items.len())
        });
        status.bib_ok = result.is_ok();
        status.bib = Some(match result {
            Ok(n) => format!("{n} papers, checked at {when}"),
            Err(e) => e,
        });
    } else {
        status.bib = None;
    }
    if !settings.notes_dir.is_empty() {
        let result = store
            .list_saved(None, 100_000)
            .and_then(|rows| vault::sync(&state.dir, &PathBuf::from(&settings.notes_dir), &rows));
        status.notes_ok = result.is_ok();
        status.notes = Some(match result {
            Ok(r) if r.yours > 0 => format!(
                "{} written, {} up to date, {} edited by you (left alone), checked at {when}",
                r.written, r.unchanged, r.yours
            ),
            Ok(r) => format!(
                "{} written, {} up to date, checked at {when}",
                r.written, r.unchanged
            ),
            Err(e) => e,
        });
    } else {
        status.notes = None;
    }
    if let Ok(mut s) = state.export_status.lock() {
        *s = status;
    }
    let _ = app.emit("exports", ());
}

// ---- choosing places -----------------------------------------------------------

/// "Choose…" / "Move…" for the Dispatch documents folder (the `.bib` and PDFs).
/// When there was one already, Dispatch's files in it move along (`documents::move_all`).
/// Returns what happened for a toast, or `None` when cancelled.
#[tauri::command]
pub async fn choose_documents(app: AppHandle) -> Result<Option<String>, String> {
    let Some(to) = app
        .dialog()
        .file()
        .set_title("Choose the Dispatch documents folder (your .bib file and PDFs)")
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let mut settings = state.settings.lock().map_err(|_| "busy")?.clone();
        let from = PathBuf::from(&settings.docs_dir);
        let message = if settings.docs_dir.is_empty() || from == to {
            format!("Dispatch documents: {}", to.display())
        } else {
            let papers = Store::open(&state.db())?.research_items()?;
            let moved = documents::move_all(
                &from,
                &to,
                &settings.bib_name,
                &documents::pdf_names(&papers),
            )?;
            moved.summary(&settings.bib_name)
        };
        settings.docs_dir = to.display().to_string();
        settings.save(&state.dir.join(dispatch::settings::FILE))?;
        *state.settings.lock().map_err(|_| "busy")? = settings;
        state.export_dirty.store(true, Ordering::SeqCst);
        Ok(Some(message))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Choose…" for a folder (notes or PDFs).
#[tauri::command]
pub async fn pick_folder(app: AppHandle, title: String) -> Option<String> {
    app.dialog()
        .file()
        .set_title(title)
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
}

/// Settings → PDF app: the known PDF apps installed on this Mac.
#[tauri::command]
pub fn pdf_apps() -> Vec<String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    pdf::installed_apps(&pdf::app_dirs(&home))
}

/// "Other app…": pick any app; returns its path (`open -a` takes a path too).
#[tauri::command]
pub async fn pick_app(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Which app should open PDFs?")
        .set_directory("/Applications")
        .add_filter("App", &["app"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
pub fn export_status(state: tauri::State<State>) -> ExportStatus {
    state
        .export_status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

/// "Update now" in Settings.
#[tauri::command]
pub fn export_now(state: tauri::State<State>) {
    state.export_dirty.store(true, Ordering::SeqCst);
}

// ---- LaTeX snippets --------------------------------------------------------------

/// `\cite{a,b}` or one `\bibitem` per paper, with the keys the live `.bib` uses.
#[tauri::command]
pub fn latex_snippet(
    state: tauri::State<State>,
    keys: Vec<String>,
    kind: String,
) -> Result<String, String> {
    let (style, tag) = state
        .settings
        .lock()
        .map(|s| (s.cite_keys, s.bib_tag.clone()))
        .unwrap_or((KeyStyle::Author, String::new()));
    let store = state.store.lock().map_err(|_| "store busy")?;
    let items: Vec<Item> = keys
        .iter()
        .filter_map(|k| store.get(k).ok().flatten())
        .collect();
    if items.is_empty() {
        return Err("no papers".into());
    }
    // The same unique keys as the .bib: work them out over the .bib's papers plus these.
    let tag = Some(tag.as_str()).filter(|t| !t.is_empty());
    let mut all: Vec<Item> = store
        .list_saved(tag, 100_000)?
        .into_iter()
        .map(|r| r.item)
        .collect();
    for i in &items {
        if !all.iter().any(|a| a.key == i.key) {
            all.push(i.clone());
        }
    }
    let unique = bibfile::keys(&all, style);
    let key_of = |i: &Item| unique[&i.key].clone();
    Ok(if kind == "bibitem" {
        items
            .iter()
            .map(|i| bibfile::bibitem(i, &key_of(i)))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        bibfile::cite_command(&items.iter().map(key_of).collect::<Vec<_>>().join(","))
    })
}

// ---- reading log -----------------------------------------------------------------

/// This week's notes (Monday to now, this Mac's time zone) as Markdown, copied to the
/// clipboard and, with a notes folder set, written there too. Returns what was done.
#[tauri::command]
pub fn reading_log(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let state = app.state::<State>();
    let now = Local::now();
    let monday = now.date_naive() - chrono::Days::new(now.weekday().num_days_from_monday() as u64);
    let start = Local
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .ok_or("no local midnight")?;
    let rows = {
        let store = state.store.lock().map_err(|_| "store busy")?;
        store.notes_between(start.timestamp_millis(), Utc::now().timestamp_millis() + 1)?
    };
    let week = vault::week_label(&now);
    let text = vault::reading_log(
        &format!("week of {} ({week})", monday.format("%-d %b %Y")),
        &rows,
    );
    app.clipboard()
        .write_text(text.clone())
        .map_err(|e| e.to_string())?;
    let dir = state
        .settings
        .lock()
        .map(|s| s.notes_dir.clone())
        .unwrap_or_default();
    let what = match rows.len() {
        1 => "1 note".to_owned(),
        n => format!("{n} notes"),
    };
    if dir.is_empty() {
        return Ok(format!("Copied this week's reading log ({what})"));
    }
    let path = vault::write_log(&state.dir, &PathBuf::from(dir), &week, &text)?;
    Ok(format!(
        "Copied this week's reading log ({what}) and wrote {}",
        path.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

// ---- PDF hand-off ----------------------------------------------------------------

/// Downloads the paper's PDF (once; a file already there is reused) with a tidy name and
/// opens it in the chosen PDF app (or the Mac's default for PDFs). Returns the file name.
#[tauri::command]
pub async fn open_pdf(app: AppHandle, key: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let (app_name, dir) = state
            .settings
            .lock()
            .map(|s| (s.pdf_app.clone(), s.pdf_folder(&home())))
            .unwrap_or_default();
        std::fs::create_dir_all(&dir).map_err(|e| format!("can't make {}: {e}", dir.display()))?;
        let item = Store::open(&state.db())?
            .get(&key)?
            .ok_or("paper not found")?;
        let http = fetch::Http::new()?;
        let path = pdf::fetch_to(&item, &dir, &http)?;
        let with = Some(app_name.as_str()).filter(|a| !a.is_empty());
        app.opener()
            .open_path(path.display().to_string(), with)
            .map_err(|e| match with {
                Some(a) => format!(
                    "couldn't open it in {} ({e}); is it installed?",
                    pdf::app_label(a)
                ),
                None => e.to_string(),
            })?;
        Ok(path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

// ---- deadlines -------------------------------------------------------

#[derive(Serialize)]
pub struct Deadlines {
    pub upcoming: Vec<deadlines::Upcoming>,
    /// Why the file couldn't be read, if it couldn't.
    pub error: Option<String>,
}

/// The coming deadlines for Stats (up to 10, the next 90 days).
#[tauri::command]
pub fn deadlines(state: tauri::State<State>) -> Deadlines {
    let today = Local::now().date_naive();
    match deadlines::load(&state.dir) {
        Ok(list) => Deadlines {
            upcoming: deadlines::upcoming(&list, today, 10),
            error: None,
        },
        Err(e) => Deadlines {
            upcoming: vec![],
            error: Some(e),
        },
    }
}

/// "Edit deadlines…": makes `deadlines.json` with an example if needed and opens it.
#[tauri::command]
pub fn edit_deadlines(app: AppHandle) -> Result<(), String> {
    let path = deadlines::ensure(&app.state::<State>().dir, Local::now().date_naive())?;
    app.opener()
        .open_path(path.display().to_string(), Some("TextEdit"))
        .map_err(|e| e.to_string())
}
