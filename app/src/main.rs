//! Dispatch's shell: the main window, the menu-bar icon, the daily schedule and the theme
//! watcher. All logic lives in `dispatch-core`; this file only wires it to Tauri.
//!
//! Window behaviour: the menu-bar icon is always there. The app shows in
//! the Dock while the window is open (`Regular`), and closing the window hides it and makes
//! Dispatch a menu-bar-only app again (`Accessory`). See the tauri-mac-app skill for why a
//! menu-bar-only app with a normal window open misbehaves under AeroSpace. `LSUIElement` in
//! `Info.plist` makes it start as a menu-bar app, so a login start never
//! flashes a Dock icon; the switch to `Regular` happens in `show_main`.
//!
//! The menu-bar menu is rebuilt from `companion::Companion` whenever counts, papers
//! or sources change (`tray_dirty`, checked every 2 s) and once a minute for the times.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod data;
mod glass;
mod health;
mod hotkey;
mod login;
mod stats;
mod status;
mod tray;
mod workflow;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dispatch::alerts;
use dispatch::catalogue::{self, Catalogue};
use dispatch::cite::{self, Format};
use dispatch::follow::{self, Follow};
use dispatch::settings::{self, Settings};
use dispatch::store::{Row, SearchFilter, SearchResult, SourceStatus};
use dispatch::sync;
use dispatch::theme::Theme;
use dispatch::{
    DB_FILE, Section, Store, TOPICS_FILE, Topics, fetch, news, paths, rank, refresh, theme,
};
use serde::Serialize;
use tauri::image::Image;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tray::{TRAY, on_tray_menu, spawn_tray, tray_menu};

pub(crate) const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");

pub(crate) struct State {
    dir: PathBuf,
    store: Mutex<Store>,
    topics: Mutex<Topics>,
    settings: Mutex<Settings>,
    refreshing: AtomicBool,
    backfilling: AtomicBool,
    /// Something of the user's changed since the last sync.
    sync_dirty: AtomicBool,
    sync_status: Mutex<SyncStatus>,
    theme: Mutex<theme::Watch>,
    /// The menu-bar menu needs rebuilding (counts, papers or sources changed).
    tray_dirty: AtomicBool,
    /// Held during an on-demand arXiv API request, so two can't overlap.
    arxiv_api: Mutex<()>,
    /// Why the blur radius couldn't be used (shown in Settings → Look), if it couldn't.
    blur_problem: Mutex<Option<String>>,
    /// The window has focus ("Solid when not in front" uses it).
    focused: AtomicBool,
    /// A `dispatch://` link the page hasn't picked up yet (it may still be loading).
    pending_link: Mutex<Option<dispatch::links::Link>>,
    /// Saved papers, notes or tags changed: the `.bib` and notes folder need a look (16a).
    export_dirty: AtomicBool,
    export_status: Mutex<workflow::ExportStatus>,
}

impl State {
    fn db(&self) -> PathBuf {
        self.dir.join(DB_FILE)
    }

    /// Something of the user's changed: sync it, and redraw the menu-bar menu.
    fn follow(&self) -> Follow {
        self.topics
            .lock()
            .map(|t| t.follow.clone())
            .unwrap_or_default()
    }

    pub(crate) fn changed(&self) {
        self.sync_dirty.store(true, Ordering::SeqCst);
        self.export_dirty.store(true, Ordering::SeqCst);
        self.tray_dirty.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone, Default, Serialize)]
struct SyncStatus {
    last: Option<sync::Report>,
    error: Option<String>,
    /// The sync check, kept in `meta` across launches: the last round that merged
    /// (not skipped), and the conflict copies merged since sync was first turned on.
    last_merge: Option<DateTime<Utc>>,
    conflicts_total: i64,
}

/// `meta` keys for the sync check.
const SYNC_LAST_MERGE: &str = "sync_last_merge";
const SYNC_CONFLICTS: &str = "sync_conflicts_total";

/// The sync check's numbers from the database.
fn sync_check(store: &Store) -> (Option<DateTime<Utc>>, i64) {
    (
        store.time_meta(SYNC_LAST_MERGE),
        store
            .meta(SYNC_CONFLICTS)
            .ok()
            .flatten()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
    )
}

#[derive(Serialize)]
struct StatusView {
    last_ok: Option<DateTime<Utc>>,
    research_last_ok: Option<DateTime<Utc>>,
    news_last_ok: Option<DateTime<Utc>>,
    research_next: DateTime<Utc>,
    news_next: DateTime<Utc>,
    refreshing: bool,
    backfilling: bool,
    research: i64,
    news: i64,
    sources: Vec<SourceStatus>,
    data_dir: String,
}

#[tauri::command]
fn items(
    state: tauri::State<State>,
    section: String,
    topic: Option<String>,
    limit: Option<usize>,
    rank: Option<bool>,
    hide_replaced: Option<bool>,
) -> Result<Vec<Row>, String> {
    let section = Section::parse(&section).ok_or("unknown section")?;
    let follow = state.follow();
    let hide = state
        .settings
        .lock()
        .map(|s| s.hide_read_after_days)
        .unwrap_or(0);
    let before = (hide > 0).then(|| Utc::now() - chrono::Duration::days(hide as i64));
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut rows = store.list_hiding(
        section,
        topic.as_deref(),
        limit.unwrap_or(300),
        before,
        hide_replaced.unwrap_or(false),
    )?;
    follow::mark(&follow, &mut rows);
    // Muted stories out, the same story from several feeds shown once.
    if section == Section::News {
        let mutes = state
            .topics
            .lock()
            .map(|t| t.mute.clone())
            .unwrap_or_default();
        let now = Utc::now();
        rows.retain(|r| !news::muted(&mutes, &r.item, now));
        rows = news::merge(rows);
    }
    // "For you": scores only; the UI orders each day's papers by them.
    if rank == Some(true) && section == Section::Research {
        let profile = rank::Profile::build(&store)?;
        rank::score_rows(&store, &follow, &profile, &mut rows)?;
    }
    Ok(rows)
}

#[tauri::command]
fn get_topics(state: tauri::State<State>) -> Result<Topics, String> {
    Ok(state.topics.lock().map_err(|_| "busy")?.clone())
}

#[tauri::command]
fn status(state: tauri::State<State>) -> Result<StatusView, String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    let now = Utc::now();
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    Ok(StatusView {
        last_ok: store.time_meta(refresh::LAST_OK),
        research_last_ok: refresh::last_ok(&store, Section::Research),
        news_last_ok: refresh::last_ok(&store, Section::News),
        research_next: refresh::next_due(&store, &settings, Section::Research, now),
        news_next: refresh::next_due(&store, &settings, Section::News, now),
        refreshing: state.refreshing.load(Ordering::SeqCst),
        backfilling: state.backfilling.load(Ordering::SeqCst),
        research: store.count(Section::Research)?,
        news: store.count(Section::News)?,
        sources: store.statuses()?,
        data_dir: state.dir.display().to_string(),
    })
}

#[tauri::command]
fn refresh_now(app: AppHandle) -> bool {
    start_refresh(&app, refresh::Scope::All)
}

#[tauri::command]
fn set_read(state: tauri::State<State>, key: String, read: bool) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.set_read(&key, read, Utc::now())?;
    state.changed();
    Ok(())
}

/// Marks every unread item in the section (or topic) read; returns their keys so the
/// window's Undo can mark exactly those unread again (`set_read_many`).
#[tauri::command]
fn mark_all_read(
    state: tauri::State<State>,
    section: String,
    topic: Option<String>,
) -> Result<Vec<String>, String> {
    let section = Section::parse(&section).ok_or("unknown section")?;
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    let keys = store.mark_all_read(section, topic.as_deref(), Utc::now())?;
    state.changed();
    Ok(keys)
}

#[tauri::command]
fn set_read_many(state: tauri::State<State>, keys: Vec<String>, read: bool) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    store.set_read_many(&keys, read, Utc::now())?;
    state.changed();
    Ok(())
}

// ---- Save for later, notes, tags -------------------------------------

#[tauri::command]
fn set_saved(state: tauri::State<State>, keys: Vec<String>, saved: bool) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    store.set_saved(&keys, saved, Utc::now())?;
    state.changed();
    Ok(())
}

// ---- Read later today -----------------------------------------------

/// Puts items on today's "Read later" queue, or takes them off. Local only (not synced): the
/// queue lasts a day.
#[tauri::command]
fn set_later(state: tauri::State<State>, keys: Vec<String>, on: bool) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.set_later(&keys, on, Utc::now())?;
    state.tray_dirty.store(true, Ordering::SeqCst);
    Ok(())
}

/// The queue: items queued since `since` (Unix seconds of local midnight, from the UI).
#[tauri::command]
fn later_items(
    state: tauri::State<State>,
    section: String,
    since: i64,
) -> Result<Vec<Row>, String> {
    let section = Section::parse(&section).ok_or("unknown section")?;
    let since = DateTime::from_timestamp(since, 0).ok_or("bad time")?;
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut rows = store.later(section, since)?;
    follow::mark(&follow, &mut rows);
    Ok(rows)
}

#[tauri::command]
fn set_note(state: tauri::State<State>, key: String, note: String) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    store.set_note(&key, &note, Utc::now())?;
    state.changed();
    Ok(())
}

#[tauri::command]
fn set_tags(
    state: tauri::State<State>,
    key: String,
    tags: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    store.set_tags(&key, &tags, Utc::now())?;
    state.changed();
    Ok(store.user(&key)?.map(|u| u.tags).unwrap_or_default())
}

#[tauri::command]
fn add_tag(state: tauri::State<State>, keys: Vec<String>, tag: String) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|_| "store busy")?;
    store.add_tag(&keys, &tag, Utc::now())?;
    state.changed();
    Ok(())
}

#[tauri::command]
fn saved_items(state: tauri::State<State>, tag: Option<String>) -> Result<Vec<Row>, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut rows = store.list_saved(tag.as_deref(), 2000)?;
    follow::mark(&follow, &mut rows);
    Ok(rows)
}

#[tauri::command]
fn all_tags(state: tauri::State<State>) -> Result<Vec<(String, i64)>, String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.tags()
}

// ---- Search archive -----------------------------------------------------

/// Searches everything ever fetched (`store::Store::search`; the query language is in
/// `dispatch::search`).
#[tauri::command]
fn search(
    state: tauri::State<State>,
    query: String,
    filter: SearchFilter,
    limit: Option<usize>,
) -> Result<SearchResult, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut result = store.search(&query, &filter, limit.unwrap_or(300).min(1000))?;
    for h in &mut result.hits {
        follow::mark(&follow, std::slice::from_mut(&mut h.row));
    }
    Ok(result)
}

// ---- Authors and alerts ---------------------------------------------------

/// Search → Notes: every item with a note.
#[tauri::command]
fn noted_items(state: tauri::State<State>) -> Result<Vec<Row>, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut rows = store.list_noted(2000)?;
    follow::mark(&follow, &mut rows);
    Ok(rows)
}

/// Search → People: authors in the archive matching what was typed.
#[tauri::command]
fn find_authors(state: tauri::State<State>, query: String) -> Result<Vec<(String, usize)>, String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    follow::find_authors(&store, &query, 40)
}

#[derive(Serialize)]
struct StorageInfo {
    /// The database with its write-ahead log, in bytes.
    bytes: u64,
    research: i64,
    news: i64,
    oldest: Option<DateTime<Utc>>,
}

#[tauri::command]
fn storage_info(state: tauri::State<State>) -> Result<StorageInfo, String> {
    let db = state.db();
    let size = |p: PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let bytes = size(db.clone()) + size(db.with_extension("sqlite-wal"));
    let store = state.store.lock().map_err(|_| "store busy")?;
    Ok(StorageInfo {
        bytes,
        research: store.count(Section::Research)?,
        news: store.count(Section::News)?,
        oldest: store.oldest(),
    })
}

/// Applies the archive limit now (Settings' "Clean up now"); returns how many items went.
#[tauri::command]
async fn prune_now(app: AppHandle) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || run_prune(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// The archive limit, on its own connection: remove, then give the space back.
fn run_prune(app: &AppHandle) -> Result<usize, String> {
    let state = app.state::<State>();
    let months = state.settings.lock().map(|s| s.archive_months).unwrap_or(0);
    if months == 0 {
        return Ok(0);
    }
    let mut store = Store::open(&state.db())?;
    let n = store.prune(Utc::now() - chrono::Duration::days(30 * months as i64))?;
    let _ = store.set_time_meta(PRUNED, Utc::now());
    if n > 0 {
        store.compact()?;
        state.tray_dirty.store(true, Ordering::SeqCst);
        let _ = app.emit("refreshed", ());
    }
    Ok(n)
}

/// When the archive limit last ran (meta), so it runs at most once a day.
const PRUNED: &str = "last_prune";

#[tauri::command]
fn author_card(state: tauri::State<State>, name: String) -> Result<follow::Card, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    follow::card(&store, &follow, &name)
}

/// Replaces the followed authors and watched keywords (Settings, or Follow on a card). They
/// live in topics.json, so they sync with the topics.
#[tauri::command]
fn set_follow(app: AppHandle, follow: Follow) -> Result<Follow, String> {
    let state = app.state::<State>();
    let follow = follow.cleaned();
    let mut topics = state.topics.lock().map_err(|_| "busy")?.clone();
    topics.follow = follow.clone();
    topics.save(&state.dir.join(TOPICS_FILE))?;
    if let Ok(store) = state.store.lock() {
        let _ = store.set_meta(
            sync::TOPICS_STAMP,
            &Utc::now().timestamp_millis().to_string(),
        );
    }
    *state.topics.lock().map_err(|_| "busy")? = topics;
    state.changed();
    let _ = app.emit("topics", ());
    Ok(follow)
}

/// Pins a Search query as a Research chip, or unpins it. Kept in topics.json,
/// so it syncs with the topics. Returns the pinned list.
#[tauri::command]
fn pin_search(app: AppHandle, query: String, on: bool) -> Result<Vec<String>, String> {
    let state = app.state::<State>();
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Err("nothing to pin".into());
    }
    let mut topics = state.topics.lock().map_err(|_| "busy")?.clone();
    topics.pinned.retain(|q| *q != query);
    if on {
        topics.pinned.push(query);
    }
    topics.save(&state.dir.join(TOPICS_FILE))?;
    if let Ok(store) = state.store.lock() {
        let _ = store.set_meta(
            sync::TOPICS_STAMP,
            &Utc::now().timestamp_millis().to_string(),
        );
    }
    let pinned = topics.pinned.clone();
    *state.topics.lock().map_err(|_| "busy")? = topics;
    state.changed();
    let _ = app.emit("topics", ());
    Ok(pinned)
}

/// Mute a news source or word (`span`: "1d", "7d", "30d", "" = until unmuted), or
/// unmute it (`on` false). Unread stories it hides are marked read at once, so counts agree.
/// Returns the mutes still in force.
#[tauri::command]
fn set_mute(
    app: AppHandle,
    kind: news::MuteKind,
    value: String,
    label: String,
    span: String,
    on: bool,
) -> Result<Vec<news::Mute>, String> {
    let state = app.state::<State>();
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err("nothing to mute".into());
    }
    let now = Utc::now();
    let mut topics = state.topics.lock().map_err(|_| "busy")?.clone();
    let same = |m: &news::Mute| m.kind == kind && m.value.eq_ignore_ascii_case(&value);
    topics.mute.retain(|m| !same(m));
    news::prune(&mut topics.mute, now);
    if on {
        topics.mute.push(news::Mute {
            kind,
            label: if label.trim().is_empty() {
                value.clone()
            } else {
                label.trim().to_owned()
            },
            value,
            until: news::until(&span, now),
        });
    }
    topics.save(&state.dir.join(TOPICS_FILE))?;
    {
        let mut store = state.store.lock().map_err(|_| "store busy")?;
        let _ = store.set_meta(sync::TOPICS_STAMP, &now.timestamp_millis().to_string());
        let keys = news::to_dismiss(&topics.mute, &store.unread_news()?, now);
        if !keys.is_empty() {
            store.set_read_many(&keys, true, now)?;
        }
    }
    let mutes = topics.mute.clone();
    *state.topics.lock().map_err(|_| "busy")? = topics;
    state.changed();
    let _ = app.emit("topics", ());
    Ok(mutes)
}

/// Unread research papers from the last 30 days matching each pinned query (the chips'
/// counts).
#[tauri::command]
fn pinned_counts(state: tauri::State<State>) -> Result<Vec<(String, usize)>, String> {
    let pinned = state.topics.lock().map_err(|_| "busy")?.pinned.clone();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let filter = SearchFilter {
        section: Some(Section::Research),
        unread_only: true,
        since: Some((Utc::now() - chrono::Duration::days(30)).timestamp()),
        ..Default::default()
    };
    pinned
        .into_iter()
        .map(|q| Ok((q.clone(), store.search(&q, &filter, 1)?.total as usize)))
        .collect()
}

/// A pinned chip's list: matching research papers, unread first, then newest.
#[tauri::command]
fn pinned_items(state: tauri::State<State>, query: String) -> Result<Vec<Row>, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let filter = SearchFilter {
        section: Some(Section::Research),
        sort: dispatch::search::Sort::Newest,
        ..Default::default()
    };
    let mut rows: Vec<Row> = store
        .search(&query, &filter, 300)?
        .hits
        .into_iter()
        .map(|h| h.row)
        .collect();
    rows.sort_by_key(|r| r.read); // stable: newest first within unread and read
    follow::mark(&follow, &mut rows);
    Ok(rows)
}

/// Suggested: the best unread papers first seen between two times (Unix seconds),
/// across all topics. The Research chip asks for the past 7 days; Monday's note for last week.
#[tauri::command]
fn suggested(state: tauri::State<State>, from: i64, to: i64) -> Result<Vec<Row>, String> {
    let follow = state.follow();
    let store = state.store.lock().map_err(|_| "store busy")?;
    let mut rows = rank::suggested(&store, &follow, from, to)?;
    follow::mark(&follow, &mut rows);
    Ok(rows)
}

/// Related: papers like this one, from the whole archive.
#[tauri::command]
fn related(state: tauri::State<State>, key: String) -> Result<Vec<Row>, String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    rank::related(&store, &key, rank::RELATED)
}

/// Follow or unfollow one person (the card's button, Settings' ×). Unfollowing removes every
/// followed entry that names the same person.
#[tauri::command]
fn follow_author(app: AppHandle, name: String, on: bool) -> Result<Follow, String> {
    let mut f = app.state::<State>().follow();
    if on {
        f.authors.push(name);
    } else {
        f.authors.retain(|a| !follow::same_person(a, &name));
    }
    set_follow(app, f)
}

/// The card's "Search arXiv" button: one API request, at least 3 s after the last one.
#[tauri::command]
async fn author_arxiv(app: AppHandle, name: String) -> Result<follow::ArxivSearch, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let _turn = state.arxiv_api.lock().map_err(|_| "busy")?;
        let topics = state.topics.lock().map(|t| t.clone()).map_err(|_| "busy")?;
        let mut store = Store::open(&state.db())?;
        let http = fetch::Http::new()?;
        let r = follow::search_arxiv(&mut store, &topics, &name, &http, Utc::now());
        // Keep the next request (from here) at least 3 s away, as arXiv asks.
        std::thread::sleep(refresh::API_PAUSE);
        if r.is_ok() {
            state.tray_dirty.store(true, Ordering::SeqCst);
            let _ = app.emit("refreshed", ());
        }
        r
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn test_notification(app: AppHandle) -> Result<(), String> {
    notify(
        &app,
        "Dispatch",
        "Notifications work. This is how new papers by people you follow will arrive.",
    )
}

fn notify(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

// ---- iCloud sync ----------------------------------------------------------------

/// Async: a sync can wait up to 10 s for iCloud to download the file; the window stays usable.
#[tauri::command]
async fn sync_now(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || run_sync(&app))
        .await
        .map_err(|e| e.to_string())?
}

/// One sync round on its own store connection. Called from the sync thread and "Sync now".
fn run_sync(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<State>();
    let folder = sync::icloud_dir().ok_or("iCloud Drive isn't turned on for this Mac")?;
    let topics = state.topics.lock().map(|t| t.clone()).map_err(|_| "busy")?;
    state.sync_dirty.store(false, Ordering::SeqCst);
    let result = Store::open(&state.db()).and_then(|mut store| {
        sync::run(
            &mut store,
            &topics,
            &state.dir.join(TOPICS_FILE),
            &folder,
            &state.dir,
            Utc::now(),
        )
    });
    let mut status = state.sync_status.lock().map_err(|_| "busy")?;
    match result {
        Ok(report) => {
            if report.topics_updated
                && let Ok(t) = Topics::load_or_seed(&state.dir.join(TOPICS_FILE))
            {
                if let Ok(mut cur) = state.topics.lock() {
                    *cur = t;
                }
                let _ = app.emit("topics", ());
            }
            if report.pulled > 0 || report.topics_updated {
                state.tray_dirty.store(true, Ordering::SeqCst);
                state.export_dirty.store(true, Ordering::SeqCst);
                let _ = app.emit("refreshed", ());
            }
            if report.skipped.is_none()
                && let Ok(store) = state.store.lock()
            {
                let (_, total) = sync_check(&store);
                let _ = store.set_time_meta(SYNC_LAST_MERGE, report.at.unwrap_or_else(Utc::now));
                let _ = store.set_meta(
                    SYNC_CONFLICTS,
                    &(total + report.conflicts as i64).to_string(),
                );
            }
            status.last = Some(report);
            status.error = None;
            let _ = app.emit("synced", ());
            Ok(())
        }
        Err(e) => {
            // Try again next round.
            state.sync_dirty.store(true, Ordering::SeqCst);
            status.error = Some(e.clone());
            let _ = app.emit("synced", ());
            Err(e)
        }
    }
}

/// Every 20 s while sync is on: sync if something changed here, the file in iCloud changed
/// (another Mac wrote it), or 5 minutes passed.
fn spawn_sync(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_run: Option<std::time::Instant> = None;
        let mut last_seen: Option<std::time::SystemTime> = None;
        loop {
            std::thread::sleep(Duration::from_secs(20));
            let state = app.state::<State>();
            let on = state
                .settings
                .lock()
                .map(|s| s.icloud_sync)
                .unwrap_or(false);
            let Some(folder) = sync::icloud_dir().filter(|_| on) else {
                continue;
            };
            let modified = std::fs::metadata(folder.join(sync::FILE))
                .and_then(|m| m.modified())
                .ok();
            let due = last_run.is_none()
                || state.sync_dirty.load(Ordering::SeqCst)
                || modified != last_seen
                || last_run.is_some_and(|t| t.elapsed() > Duration::from_secs(300));
            if due {
                let _ = run_sync(&app);
                last_run = Some(std::time::Instant::now());
                last_seen = std::fs::metadata(folder.join(sync::FILE))
                    .and_then(|m| m.modified())
                    .ok();
            }
        }
    });
}

#[derive(Serialize)]
struct CiteResult {
    count: usize,
    /// "3 from arXiv, 1 from doi.org"
    origins: String,
    path: Option<String>,
}

/// The citations for `keys` as one text. Network lookups happen on a worker thread with a
/// fresh store connection, so the window never waits on the store lock for them.
async fn citation_text(
    app: &AppHandle,
    keys: Vec<String>,
    format: Format,
) -> Result<(String, CiteResult), String> {
    let db = app.state::<State>().db();
    tauri::async_runtime::spawn_blocking(move || {
        let store = Store::open(&db)?;
        let items: Vec<_> = keys
            .iter()
            .filter_map(|k| store.get(k).ok().flatten())
            .collect();
        if items.is_empty() {
            return Err("nothing selected".to_owned());
        }
        let http = fetch::Http::new().ok();
        let (text, origins) = cite::citations(
            &store,
            &items,
            format,
            http.as_ref().map(|h| h as &dyn fetch::Fetch),
            Utc::now(),
        );
        let origins = origins
            .iter()
            .map(|(o, n)| format!("{n} from {o}"))
            .collect::<Vec<_>>()
            .join(", ");
        Ok((
            text,
            CiteResult {
                count: items.len(),
                origins,
                path: None,
            },
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn copy_citations(
    app: AppHandle,
    keys: Vec<String>,
    format: Format,
) -> Result<CiteResult, String> {
    let (text, result) = citation_text(&app, keys, format).await?;
    app.clipboard()
        .write_text(text)
        .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Asks where to save, then writes a `.bib` or `.ris` file. `path` is None if cancelled.
#[tauri::command]
async fn export_citations(
    app: AppHandle,
    keys: Vec<String>,
    format: Format,
) -> Result<CiteResult, String> {
    let (text, mut result) = citation_text(&app, keys, format).await?;
    let name = if result.count == 1 {
        format!("citation.{}", format.extension())
    } else {
        format!("dispatch-{}-papers.{}", result.count, format.extension())
    };
    let chosen = app
        .dialog()
        .file()
        .set_file_name(&name)
        .add_filter(format.as_str(), &[format.extension()])
        .blocking_save_file();
    let Some(path) = chosen.and_then(|p| p.into_path().ok()) else {
        return Ok(result);
    };
    dispatch::paths::write_atomic(&path, text.as_bytes())?;
    result.path = Some(path.display().to_string());
    Ok(result)
}

fn zotero_app() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    [
        PathBuf::from("/Applications/Zotero.app"),
        home.join("Applications/Zotero.app"),
    ]
    .into_iter()
    .find(|p| p.exists())
}

#[derive(Serialize)]
struct SettingsView {
    settings: Settings,
    /// iCloud Drive is on for this Mac (the sync folder can exist).
    icloud_available: bool,
    sync_folder: Option<String>,
    sync: SyncStatus,
    /// Launch at login can only be offered from an installed `.app`.
    login_available: bool,
    data_dir: String,
    version: &'static str,
    /// The look in force (Palette's glass or these settings).
    look: settings::Look,
    /// Why the blur radius isn't used, if it isn't.
    blur_problem: Option<String>,
    /// The Palette file the colours come from ("" = built-in theme).
    theme_file: String,
    /// Daily and "Back up now" backups, newest first.
    backups: Vec<dispatch::backup::Backup>,
    /// Your tags (the `.bib` file can hold one tag's papers).
    tags: Vec<String>,
    /// The global shortcut registered now ("" = none, or it couldn't be used).
    hotkey_active: String,
    /// The SketchyBar pill's choices (id, label) and what it would show now.
    bar_metrics: Vec<(&'static str, &'static str)>,
    bar_preview: String,
}

/// What the SketchyBar pill would say now (Settings shows it beside the choices).
fn bar_preview(state: &State, settings: &Settings) -> String {
    let topics = state.topics.lock().map(|t| t.clone()).unwrap_or_default();
    let Ok(store) = state.store.lock() else {
        return String::new();
    };
    let now = Utc::now();
    dispatch::companion::Companion::build(&store, &topics, settings, now)
        .and_then(|c| dispatch::status::Status::build(&c, &store, settings, now))
        .map(|s| s.label)
        .unwrap_or_default()
}

#[tauri::command]
fn get_settings(app: AppHandle) -> Result<SettingsView, String> {
    let state = app.state::<State>();
    let mut settings = state.settings.lock().map_err(|_| "busy")?.clone();
    // The login item is the truth (it can be removed in System Settings → Login Items).
    settings.launch_at_login = login::is_enabled();
    let glass = state.theme.lock().ok().and_then(|w| w.current().glass);
    let look = settings.look(glass.as_ref());
    let settings_now = settings.clone();
    Ok(SettingsView {
        settings,
        icloud_available: sync::icloud_dir().is_some(),
        sync_folder: sync::icloud_dir().map(|d| d.display().to_string()),
        sync: {
            let mut st = state
                .sync_status
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default();
            if let Ok(store) = state.store.lock() {
                (st.last_merge, st.conflicts_total) = sync_check(&store);
            }
            st
        },
        login_available: login::bundle().is_some(),
        data_dir: state.dir.display().to_string(),
        version: env!("CARGO_PKG_VERSION"),
        look,
        blur_problem: state.blur_problem.lock().ok().and_then(|b| b.clone()),
        theme_file: state
            .theme
            .lock()
            .map(|w| w.current().file.clone())
            .unwrap_or_default(),
        backups: dispatch::backup::list(&state.dir),
        tags: state
            .store
            .lock()
            .ok()
            .and_then(|s| s.tags().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|(t, _)| t)
            .collect(),
        hotkey_active: app
            .state::<hotkey::Hotkey>()
            .0
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default(),
        bar_metrics: dispatch::status::METRICS.to_vec(),
        bar_preview: bar_preview(&state, &settings_now),
    })
}

/// The window's look now: Palette's glass if it chose one, else Settings.
fn current_look(app: &AppHandle) -> settings::Look {
    let state = app.state::<State>();
    let glass = state.theme.lock().ok().and_then(|w| w.current().glass);
    let focused = state.focused.load(Ordering::SeqCst);
    state
        .settings
        .lock()
        .map(|s| {
            if focused {
                s.look(glass.as_ref())
            } else {
                s.look_unfocused(glass.as_ref())
            }
        })
        .unwrap_or_else(|_| Settings::default().look(None))
}

/// Puts the look on the window (material or blur radius, `glass.rs`) and tells the page
/// (its background opacity, and Settings' note when the blur radius isn't available).
fn apply_look(app: &AppHandle) {
    let look = current_look(app);
    let solid = app
        .state::<State>()
        .theme
        .lock()
        .map(|w| w.current().background_rgb())
        .unwrap_or((0x1a, 0x1b, 0x26));
    let handle = app.clone();
    glass::apply(app, look, solid, move |result| {
        let problem = result.err();
        if let Some(p) = &problem {
            eprintln!("dispatch: blur: {p}");
        }
        if let Ok(mut b) = handle.state::<State>().blur_problem.lock() {
            *b = problem;
        }
        let _ = handle.emit("look", look);
    });
}

/// Saves settings (clamped to their limits), adding or removing the login item to match.
#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<SettingsView, String> {
    let mut settings = settings.clamped();
    // Edited only by their own commands (the shortcut recorder, the menu's Pause): a Settings
    // page drawn earlier must not put back an old value.
    if let Ok(live) = app.state::<State>().settings.lock() {
        settings.hotkey = live.hotkey.clone();
        settings.notify_paused_until = live.notify_paused_until;
    }
    if settings.launch_at_login != login::is_enabled() {
        if settings.launch_at_login {
            login::enable()?;
        } else {
            login::disable()?;
        }
    }
    if settings.icloud_sync && sync::icloud_dir().is_none() {
        return Err(
            "iCloud Drive isn't turned on for this Mac (System Settings → Apple Account → iCloud)"
                .into(),
        );
    }
    let state = app.state::<State>();
    let turning_on = settings.icloud_sync
        && !state
            .settings
            .lock()
            .map(|s| s.icloud_sync)
            .unwrap_or(false);
    settings.save(&state.dir.join(settings::FILE))?;
    *state.settings.lock().map_err(|_| "busy")? = settings;
    apply_look(&app);
    state.tray_dirty.store(true, Ordering::SeqCst);
    state.export_dirty.store(true, Ordering::SeqCst); // the .bib file, key style or folder
    if turning_on {
        // First sync straight away (in the background), so the switch shows a result.
        let app = app.clone();
        std::thread::spawn(move || {
            let _ = run_sync(&app);
        });
    }
    let _ = app.emit("settings", ());
    get_settings(app.clone())
}

#[tauri::command]
fn reveal_data_folder(app: AppHandle) -> Result<(), String> {
    let dir = app.state::<State>().dir.join(dispatch::DB_FILE);
    app.opener()
        .reveal_item_in_dir(dir)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn has_zotero() -> bool {
    zotero_app().is_some()
}

/// Hands the papers to the Zotero app as a BibTeX file (Zotero imports any `.bib` it is asked
/// to open, and recognises arXiv entries as preprints). No API key; Zotero shows its import.
#[tauri::command]
async fn open_in_zotero(app: AppHandle, keys: Vec<String>) -> Result<CiteResult, String> {
    let zotero = zotero_app().ok_or("Zotero isn't installed in Applications")?;
    let (text, result) = citation_text(&app, keys, Format::Bibtex).await?;
    let dir = app.state::<State>().dir.join("zotero");
    let path = dir.join(format!(
        "dispatch-{}.bib",
        Utc::now().format("%Y%m%d-%H%M%S")
    ));
    dispatch::paths::write_atomic(&path, text.as_bytes())?;
    // Keep the folder small: only the newest 10 hand-over files.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut files: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        files.sort();
        for old in files.iter().rev().skip(10) {
            let _ = std::fs::remove_file(old);
        }
    }
    app.opener()
        .open_path(
            path.display().to_string(),
            Some(zotero.display().to_string()),
        )
        .map_err(|e| e.to_string())?;
    Ok(result)
}

/// Fetches recent papers for categories that have none yet (first run, or newly added),
/// one arXiv API request every 3 s, on a background thread.
fn start_backfill(app: &AppHandle) {
    let state = app.state::<State>();
    let topics = state.topics.lock().map(|t| t.clone()).unwrap_or_default();
    let cats = match state.store.lock() {
        Ok(s) => refresh::needs_backfill(&s, &topics),
        Err(_) => return,
    };
    if cats.is_empty() || state.backfilling.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = app.emit("refreshing", true);
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<State>();
        let result = Store::open(&state.db()).and_then(|mut store| {
            let http = fetch::Http::new()?;
            Ok(refresh::backfill(
                &mut store,
                &topics,
                &cats,
                &http,
                Utc::now(),
                &|| std::thread::sleep(refresh::API_PAUSE),
            ))
        });
        state.backfilling.store(false, Ordering::SeqCst);
        match result {
            Ok(r) => {
                for (cat, e) in &r.failed {
                    eprintln!("dispatch: backfill {cat}: {e}");
                }
                state.tray_dirty.store(true, Ordering::SeqCst);
                let _ = app.emit("refreshed", ());
            }
            Err(e) => {
                eprintln!("dispatch: backfill failed: {e}");
                let _ = app.emit("refresh-error", e);
            }
        }
    });
}

#[tauri::command]
fn unread_counts(
    state: tauri::State<State>,
    section: String,
    hide_replaced: Option<bool>,
) -> Result<BTreeMap<String, i64>, String> {
    let section = Section::parse(&section).ok_or("unknown section")?;
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.unread_counts_hiding(section, hide_replaced.unwrap_or(false))
}

#[tauri::command]
fn arxiv_catalogue() -> Catalogue {
    catalogue::load()
}

/// Saves the topic picker's changes: checks them, writes `topics.json` atomically, retags
/// every item, and fetches straight away only if a new category or feed was added.
#[tauri::command]
fn save_topics(app: AppHandle, topics: Topics) -> Result<(), String> {
    topics.validate()?;
    let state = app.state::<State>();
    let old = state.topics.lock().map_err(|_| "busy")?.clone();
    // Following is edited elsewhere (Settings, author cards); a picker draft opened earlier
    // must not undo it.
    let mut topics = topics;
    topics.follow = old.follow.clone();
    topics.pinned = old.pinned.clone();
    topics.mute = old.mute.clone();
    topics.save(&state.dir.join(TOPICS_FILE))?;
    // When the topics last changed here, so sync knows which Mac's topics are newer.
    if let Ok(store) = state.store.lock() {
        let _ = store.set_meta(
            sync::TOPICS_STAMP,
            &Utc::now().timestamp_millis().to_string(),
        );
    }
    state.changed();
    {
        let mut store = state.store.lock().map_err(|_| "store busy")?;
        store.retag(&topics)?;
        refresh::forget_old_sources(&store, &topics)?;
    }
    state.tray_dirty.store(true, Ordering::SeqCst);
    let (research, news) = topics.new_sources_since(&old);
    *state.topics.lock().map_err(|_| "busy")? = topics;
    let _ = app.emit("topics", ());
    if research {
        start_backfill(&app);
    }
    match (research, news) {
        (true, true) => start_refresh(&app, refresh::Scope::All),
        (true, false) => start_refresh(&app, refresh::Scope::Research),
        (false, true) => start_refresh(&app, refresh::Scope::News),
        (false, false) => false,
    };
    Ok(())
}

#[derive(Serialize)]
struct FeedTest {
    items: usize,
    first: Option<String>,
}

/// The picker's "Test feed": downloads and parses a feed without storing anything.
/// Async so the window stays responsive while it waits on the network.
#[tauri::command]
async fn test_feed(url: String) -> Result<FeedTest, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let http = fetch::Http::new()?;
        refresh::test_feed(&http, &url).map(|(items, first)| FeedTest { items, first })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn current_theme(state: tauri::State<State>) -> Result<Theme, String> {
    Ok(state.theme.lock().map_err(|_| "busy")?.current().clone())
}

/// Opens a web link in the default browser. Only http(s), so a feed can't make the app
/// open a local file or another app's URL scheme.
#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("only web links can be opened".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// Runs a refresh on a background thread unless one is already running. The thread opens
/// its own database connection (SQLite's WAL mode lets the window keep reading meanwhile).
pub(crate) fn start_refresh(app: &AppHandle, scope: refresh::Scope) -> bool {
    let state = app.state::<State>();
    if state.refreshing.swap(true, Ordering::SeqCst) {
        return false;
    }
    let _ = app.emit("refreshing", true);
    state.tray_dirty.store(true, Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<State>();
        let topics = state.topics.lock().map(|t| t.clone()).unwrap_or_default();
        let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
        let result = Store::open(&state.db()).and_then(|mut store| {
            let http = fetch::Http::new()?;
            let now = Utc::now();
            let before = alerts::snapshot(&store).unwrap_or_default();
            let report = refresh::run(&mut store, &topics, &http, now, scope);
            let found = alerts::collect(&store, &topics.follow, &settings, now, &before)
                .unwrap_or_default();
            Ok((report, found))
        });
        state.refreshing.store(false, Ordering::SeqCst);
        state.tray_dirty.store(true, Ordering::SeqCst);
        state.export_dirty.store(true, Ordering::SeqCst); // a saved paper's journal or DOI
        match result {
            Ok((report, found)) => {
                let paused = state
                    .settings
                    .lock()
                    .is_ok_and(|s| s.notifications_paused(Utc::now()));
                if !paused
                    && let Some((title, body)) = alerts::summary(&found)
                    && let Err(e) = notify(&app, &title, &body)
                {
                    eprintln!("dispatch: notification: {e}");
                }
                let _ = app.emit("refreshed", report);
            }
            Err(e) => {
                eprintln!("dispatch: refresh failed: {e}");
                let _ = app.emit("refresh-error", e);
            }
        }
    });
    true
}

pub(crate) fn show_main(app: &AppHandle) {
    window_seen(app);
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// The window is being looked at, so the icon's dot (new papers since then) goes.
pub(crate) fn window_seen(app: &AppHandle) {
    let state = app.state::<State>();
    if let Ok(store) = state.store.lock() {
        let _ = store.set_time_meta(dispatch::companion::WINDOW_SEEN, Utc::now());
    }
    state.tray_dirty.store(true, Ordering::SeqCst);
}

/// Every minute: refresh what is due (arXiv after the daily slot, news every 3 hours).
/// Checking the clock (instead of sleeping until 13:30) is what makes a Mac that slept
/// through the slot catch up within a minute of waking.
fn spawn_scheduler(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        start_backfill(&app);
        loop {
            let state = app.state::<State>();
            let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
            let due = state
                .store
                .lock()
                .ok()
                .and_then(|s| refresh::due_scope(&s, &settings, Utc::now()));
            if let Some(scope) = due {
                start_refresh(&app, scope);
            }
            let prune_due = state.settings.lock().is_ok_and(|s| s.archive_months > 0)
                && !state.refreshing.load(Ordering::SeqCst)
                && state.store.lock().ok().is_some_and(|s| {
                    s.time_meta(PRUNED)
                        .is_none_or(|t| Utc::now() - t > chrono::Duration::days(1))
                });
            if prune_due && let Err(e) = run_prune(&app) {
                eprintln!("dispatch: archive limit: {e}");
            }
            if !state.refreshing.load(Ordering::SeqCst) {
                data::daily_backup(&app);
            }
            std::thread::sleep(Duration::from_secs(60));
        }
    });
}

/// Every 2 s: one `stat` of Palette's colour file; the window is told only on a change.
fn spawn_theme_watch(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let changed = app
                .state::<State>()
                .theme
                .lock()
                .ok()
                .and_then(|mut w| w.poll());
            if let Some(t) = changed {
                let _ = app.emit("theme", t);
                apply_look(&app); // Palette's glass may have changed with it
            }
        }
    });
}

fn main() {
    let dir = paths::data_dir();
    let store = Store::open(&dir.join(DB_FILE)).unwrap_or_else(|e| {
        eprintln!("dispatch: {e}");
        std::process::exit(1)
    });
    // A broken topics.json must not be overwritten: show the defaults this session and
    // say so in the log.
    let topics = Topics::load_or_seed(&dir.join(TOPICS_FILE)).unwrap_or_else(|e| {
        eprintln!("dispatch: {e}; using the default topics until it's fixed");
        Topics::default()
    });
    let (app_settings, settings_problem) = Settings::load(&dir.join(settings::FILE));
    if let Some(p) = settings_problem {
        eprintln!("dispatch: {p}");
    }
    let quiet_start = std::env::args().any(|a| a == "--autostart");
    // Count every request in the network log from here on.
    dispatch::netlog::init(&dir);

    tauri::Builder::default()
        // First, so a second launch is caught before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if !argv.iter().any(|a| a == "--autostart") {
                show_main(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(hotkey::plugin())
        .manage(hotkey::Hotkey(Mutex::new(String::new())))
        .manage(State {
            dir,
            store: Mutex::new(store),
            topics: Mutex::new(topics),
            settings: Mutex::new(app_settings),
            refreshing: AtomicBool::new(false),
            backfilling: AtomicBool::new(false),
            sync_dirty: AtomicBool::new(false),
            sync_status: Mutex::new(SyncStatus::default()),
            theme: Mutex::new(theme::Watch::new(&paths::palette_dir())),
            tray_dirty: AtomicBool::new(true),
            arxiv_api: Mutex::new(()),
            blur_problem: Mutex::new(None),
            focused: AtomicBool::new(true),
            pending_link: Mutex::new(None),
            export_dirty: AtomicBool::new(true),
            export_status: Mutex::new(workflow::ExportStatus::default()),
        })
        .invoke_handler(tauri::generate_handler![
            items,
            get_topics,
            status,
            refresh_now,
            current_theme,
            open_link,
            set_read,
            mark_all_read,
            unread_counts,
            arxiv_catalogue,
            save_topics,
            test_feed,
            set_read_many,
            copy_citations,
            export_citations,
            has_zotero,
            open_in_zotero,
            get_settings,
            save_settings,
            reveal_data_folder,
            set_saved,
            set_note,
            set_tags,
            add_tag,
            saved_items,
            all_tags,
            sync_now,
            search,
            author_card,
            noted_items,
            find_authors,
            storage_info,
            prune_now,
            set_follow,
            follow_author,
            author_arxiv,
            test_notification,
            stats::stats,
            stats::opened,
            stats::unopened,
            stats::reset_stats,
            set_later,
            later_items,
            data::backups,
            data::backup_now,
            data::export_backup,
            data::restore_backup,
            data::send_to_zotero,
            data::take_link,
            data::copy_text,
            workflow::choose_documents,
            workflow::pick_folder,
            workflow::pdf_apps,
            workflow::pick_app,
            workflow::export_status,
            workflow::export_now,
            workflow::latex_snippet,
            workflow::reading_log,
            workflow::open_pdf,
            pin_search,
            pinned_counts,
            pinned_items,
            suggested,
            related,
            workflow::deadlines,
            workflow::edit_deadlines,
            data::zotero_connect,
            data::zotero_disconnect,
            data::s2_connect,
            data::s2_disconnect,
            data::s2_lists,
            set_mute,
            hotkey::set_hotkey,
            data::add_paper_now,
            health::data_health,
            health::compact_now,
            health::export_everything,
            health::network_log,
            data::zotero_collections
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            apply_look(&handle);
            workflow::start_thread(&handle);
            let combo = handle
                .state::<State>()
                .settings
                .lock()
                .map(|s| s.hotkey.clone())
                .unwrap_or_default();
            if let Err(e) = hotkey::register(&handle, &combo) {
                eprintln!("dispatch: shortcut: {e}");
            }
            if let Some(window) = app.get_webview_window("main") {
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::Focused(now) = event {
                        let app = w.app_handle();
                        let state = app.state::<State>();
                        let was = state.focused.swap(*now, Ordering::SeqCst);
                        if *now {
                            window_seen(app);
                        }
                        let matters = state.settings.lock().is_ok_and(|s| s.solid_in_background);
                        if was != *now && matters {
                            apply_look(app);
                        }
                    }
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                        #[cfg(target_os = "macos")]
                        let _ = w
                            .app_handle()
                            .set_activation_policy(tauri::ActivationPolicy::Accessory);
                    }
                });
            }

            TrayIconBuilder::with_id(TRAY)
                .icon(Image::from_bytes(TRAY_ICON)?)
                .icon_as_template(true)
                .tooltip("Dispatch")
                .menu(&tray_menu(&handle, None)?)
                .on_menu_event(|app, event| on_tray_menu(app, event.id().as_ref()))
                .build(app)?;

            if quiet_start {
                #[cfg(target_os = "macos")]
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            } else {
                show_main(&handle);
            }
            spawn_scheduler(handle.clone());
            spawn_sync(handle.clone());
            spawn_tray(handle.clone());
            spawn_theme_watch(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("Dispatch failed to start")
        .run(|app, event| {
            // Opening the app again (Finder, Spotlight, `open -a Dispatch`) shows the window.
            #[cfg(target_os = "macos")]
            match &event {
                RunEvent::Reopen { .. } => show_main(app),
                // `dispatch://…` links.
                RunEvent::Opened { urls } => data::opened(app, urls),
                _ => {}
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}
