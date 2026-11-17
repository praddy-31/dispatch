//! Launch at login (opt-in, Settings). A LaunchAgent that runs
//! `open -g -a Dispatch.app --args --autostart` at login. Going through `open` means
//! LaunchServices knows Dispatch is running, so a Dock click or link later reaches this copy
//! instead of starting a second one (tauri-mac-app skill, "One instance only"). `--autostart`
//! keeps the start quiet: menu-bar icon only, no window.
//!
//! Only offered when Dispatch runs from an `.app` bundle (not `cargo tauri dev`). The agent is
//! not loaded now (that would start a second copy); macOS picks it up at the next login.

use std::path::PathBuf;
use std::process::Command;

pub const LABEL: &str = "dev.dispatch.reader.login";

fn agent_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(format!("Library/LaunchAgents/{LABEL}.plist")))
}

/// `/Applications/Dispatch.app` when running from a bundle.
pub fn bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let app = exe.ancestors().nth(3)?.to_path_buf();
    (app.extension().and_then(|e| e.to_str()) == Some("app")).then_some(app)
}

pub fn is_enabled() -> bool {
    agent_path().is_some_and(|p| p.exists())
}

pub fn enable() -> Result<(), String> {
    let app =
        bundle().ok_or("Launch at login works from the installed app, not a development run")?;
    let path = agent_path().ok_or("no home folder")?;
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \x20 <key>Label</key>\n  <string>{LABEL}</string>\n\
         \x20 <key>AssociatedBundleIdentifiers</key>\n  <array><string>dev.dispatch.reader</string></array>\n\
         \x20 <key>ProgramArguments</key>\n  <array><string>/usr/bin/open</string><string>-g</string><string>-a</string><string>{}</string><string>--args</string><string>--autostart</string></array>\n\
         \x20 <key>RunAtLoad</key>\n  <true/>\n\
         </dict>\n</plist>\n",
        esc(&app.to_string_lossy())
    );
    dispatch::paths::write_atomic(&path, plist.as_bytes())
}

pub fn disable() -> Result<(), String> {
    let Some(path) = agent_path() else {
        return Ok(());
    };
    // In case it is loaded (it is, after a login): unload it, then remove the file.
    if let Ok(out) = Command::new("/usr/bin/id").arg("-u").output() {
        let uid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let _ = Command::new("/bin/launchctl")
            .args(["bootout", &format!("gui/{uid}/{LABEL}")])
            .output();
    }
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("can't remove {}: {e}", path.display())),
    }
}
