//! Writes `status.json` (see `dispatch::status`) from the menu-bar thread and nudges
//! SketchyBar so its Dispatch pill updates at once instead of at its next poll.

use std::path::Path;
use std::process::{Command, Stdio};

use chrono::Utc;
use dispatch::companion::Companion;
use dispatch::status::{self, Status};
use tauri::{AppHandle, Manager};

use crate::State;

/// What was written last: the fingerprint, or "off" once the off-file was written. `None` at
/// start, so the first round always writes (an old file from before a switch-off is fixed).
pub type Last = Option<String>;

pub fn export(app: &AppHandle, c: &Companion, last: &mut Last) {
    let state = app.state::<State>();
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    let path = state.dir.join(status::FILE);
    if !settings.status_file {
        // Tell the pill to hide once; afterwards leave the file alone.
        if last.as_deref() != Some("off") && (last.is_some() || path.exists()) {
            let text = status::turned_off(Utc::now()).to_string() + "\n";
            if dispatch::paths::write_atomic(&path, text.as_bytes()).is_ok() {
                trigger();
            }
        }
        *last = Some("off".into());
        return;
    }
    let built = match state.store.lock() {
        Ok(store) => Status::build(c, &store, &settings, Utc::now()),
        Err(_) => return,
    };
    let s = match built {
        Ok(s) => s,
        Err(e) => {
            eprintln!("dispatch: status.json: {e}");
            return;
        }
    };
    let print = s.fingerprint();
    if last.as_ref() == Some(&print) {
        return;
    }
    match s.write(&path) {
        Ok(()) => {
            *last = Some(print);
            trigger();
        }
        Err(e) => eprintln!("dispatch: status.json: {e}"),
    }
}

/// `sketchybar --trigger dispatch_update`, if SketchyBar is installed. An app opened from
/// Finder doesn't get the shell's PATH, so the Homebrew folders are tried by name.
fn trigger() {
    let Some(bin) = ["/opt/homebrew/bin/sketchybar", "/usr/local/bin/sketchybar"]
        .into_iter()
        .find(|p| Path::new(p).exists())
    else {
        return;
    };
    // Off the menu thread's back: SketchyBar answers in milliseconds, but never wait on it.
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args(["--trigger", "dispatch_update"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}
