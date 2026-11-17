//! Commands: data health, compacting the database, the plain-text export, and the
//! network log. The work is in `dispatch::health` and `dispatch::netlog`.

use chrono::Utc;
use dispatch::{Store, health, netlog};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::State;

/// Settings → Data → Check: on its own connection, so a big database doesn't hold the
/// window's store while SQLite checks it.
#[tauri::command]
pub async fn data_health(app: AppHandle) -> Result<health::Health, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let topics = state.topics.lock().map(|t| t.clone()).map_err(|_| "busy")?;
        let settings = state
            .settings
            .lock()
            .map(|s| s.clone())
            .map_err(|_| "busy")?;
        let store = Store::open(&state.db())?;
        health::check(&store, &topics, &settings, &state.dir, Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Compact database": VACUUM; returns the bytes given back to macOS.
#[tauri::command]
pub async fn compact_now(app: AppHandle) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let db = state.db();
        let size = || {
            std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0)
                + std::fs::metadata(db.with_extension("sqlite-wal"))
                    .map(|m| m.len())
                    .unwrap_or(0)
        };
        let before = size();
        state.store.lock().map_err(|_| "store busy")?.compact()?;
        Ok(before.saturating_sub(size()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Export everything…": a folder of JSON + CSV inside the folder you pick; `None` = cancelled.
#[tauri::command]
pub async fn export_everything(app: AppHandle) -> Result<Option<String>, String> {
    let Some(parent) = app
        .dialog()
        .file()
        .set_title("Choose a folder for the export")
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let topics = state.topics.lock().map(|t| t.clone()).map_err(|_| "busy")?;
        let settings = state
            .settings
            .lock()
            .map(|s| s.clone())
            .map_err(|_| "busy")?;
        let store = Store::open(&state.db())?;
        health::export_all(&store, &topics, &settings, &parent, Utc::now())
            .map(|d| Some(d.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The network log: hosts contacted in the last 7 days, and why.
#[tauri::command]
pub fn network_log(state: tauri::State<State>) -> Vec<netlog::Host> {
    netlog::recent(&state.dir, Utc::now())
}
