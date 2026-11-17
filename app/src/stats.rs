//! Stats in the shell: the Stats tab's two commands and the Habit Tracker link.
//! The numbers themselves come from `dispatch::stats`.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use chrono::Utc;
use dispatch::stats::{self, DayCounts, Range, ReadingLog, Stats};
use tauri::{AppHandle, Manager};

use crate::State;

/// The Stats tab for a range (`4w`, `3m` or `1y`).
#[tauri::command]
pub fn stats(state: tauri::State<State>, range: String) -> Result<Stats, String> {
    let range = Range::parse(&range).ok_or("unknown range")?;
    let topics = state.topics.lock().map_err(|_| "busy")?.clone();
    let settings = state.settings.lock().map_err(|_| "busy")?.clone();
    let store = state.store.lock().map_err(|_| "store busy")?;
    Stats::build(&store, &topics, &settings, stats::local_now(), range)
}

/// The user opened an item's page or PDF: Stats counts it as read (not synced).
#[tauri::command]
pub fn opened(state: tauri::State<State>, key: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.set_opened(&key, Utc::now())?;
    state.tray_dirty.store(true, Ordering::SeqCst); // the menu's "This week: N / goal"
    Ok(())
}

/// "Don't count" in the preview: the open was an accident; the item is only seen again.
#[tauri::command]
pub fn unopened(state: tauri::State<State>, key: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    store.clear_opened(&key)?;
    state.tray_dirty.store(true, Ordering::SeqCst);
    Ok(())
}

/// Settings → Reset stats: count from now on (nothing is deleted), or from `back_to` (Undo).
/// Returns the reset time it replaced, for the Undo.
#[tauri::command]
pub fn reset_stats(state: tauri::State<State>, back_to: Option<i64>) -> Result<i64, String> {
    let store = state.store.lock().map_err(|_| "store busy")?;
    let before = stats::since(&store);
    match back_to {
        Some(t) => stats::set_since(&store, t)?,
        None => stats::reset(&store, Utc::now())?,
    }
    state.tray_dirty.store(true, Ordering::SeqCst);
    Ok(before)
}

/// Rewrites `reading-log.json` when the Habit Tracker link is on and the counts changed.
/// Called from the menu-bar thread whenever something changed (so at most every 2 s) and
/// once at start; `last` holds what was written last, so an unchanged log isn't rewritten.
pub fn export_reading_log(app: &AppHandle, last: &mut Option<BTreeMap<String, DayCounts>>) {
    let state = app.state::<State>();
    if !state
        .settings
        .lock()
        .map(|s| s.habit_export)
        .unwrap_or(false)
    {
        *last = None;
        return;
    }
    let log = match state.store.lock() {
        Ok(store) => ReadingLog::build(&store, stats::local_now()),
        Err(_) => return,
    };
    let log = match log {
        Ok(l) => l,
        Err(e) => {
            eprintln!("dispatch: reading log: {e}");
            return;
        }
    };
    if last.as_ref() == Some(&log.counts) {
        return;
    }
    match log.write(&state.dir.join(stats::READING_LOG_FILE)) {
        Ok(()) => *last = Some(log.counts),
        Err(e) => eprintln!("dispatch: reading log: {e}"),
    }
}
