//! The global shortcut (default ⌃⌥S) that opens Search from anywhere, through
//! Tauri's official global-shortcut plugin, as in Palette and the Habit Tracker. The combo is
//! `Settings::hotkey`; a combo another app owns is refused and the old one kept.

use std::sync::Mutex;

use dispatch::settings::{self, normalise_hotkey};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::{State, show_main};

/// The combo registered now ("" = none).
pub struct Hotkey(pub Mutex<String>);

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                show_main(app);
                let _ = app.emit("show-view", "search");
            }
        })
        .build()
}

/// Registers `combo` in place of the current one. On failure the old one is put back.
pub fn register(app: &AppHandle, combo: &str) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let state = app.state::<Hotkey>();
    let mut current = state.0.lock().map_err(|_| "busy")?;
    if !current.is_empty() {
        let _ = shortcuts.unregister(current.as_str());
    }
    if combo.is_empty() {
        current.clear();
        return Ok(());
    }
    match shortcuts.register(combo) {
        Ok(()) => {
            *current = combo.to_owned();
            Ok(())
        }
        Err(e) => {
            if !current.is_empty() {
                let _ = shortcuts.register(current.as_str());
            }
            Err(format!("can't use {combo}: {e} (another app may own it)"))
        }
    }
}

/// Settings' recorder: checks, registers and saves a new combo ("" turns it off). Returns
/// the combo now in force.
#[tauri::command]
pub fn set_hotkey(app: AppHandle, combo: String) -> Result<String, String> {
    let combo = normalise_hotkey(&combo)?;
    register(&app, &combo)?;
    let state = app.state::<State>();
    let mut s = state.settings.lock().map_err(|_| "busy")?;
    s.hotkey = combo.clone();
    s.save(&state.dir.join(settings::FILE))?;
    Ok(combo)
}
