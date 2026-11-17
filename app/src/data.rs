//! In the shell: backups (daily, "Back up now", Export…, Restore), Send to Zotero
//! through the local Zotero app, and `dispatch://` links. The logic is in
//! `dispatch::{backup, zotero, links}`; this wires it to commands, the scheduler and macOS.

use std::sync::atomic::Ordering;
use std::time::Duration;

use chrono::Utc;
use dispatch::backup::{self, Backup};
use dispatch::links::{self, Link};
use dispatch::{Store, TOPICS_FILE, Topics, fetch, s2, zotero};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use crate::State;

// ---- backups -----------------------------------------------------------------------

#[tauri::command]
pub fn backups(state: tauri::State<State>) -> Vec<Backup> {
    backup::list(&state.dir)
}

/// "Back up now": one more in `backups/` (the newest five are kept).
#[tauri::command]
pub async fn backup_now(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let store = Store::open(&state.db())?;
        backup::rolling(&store, &state.dir).map(|p| p.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Export…": the same folder, written where you choose (kept until you delete it).
/// `None` when cancelled.
#[tauri::command]
pub async fn export_backup(app: AppHandle) -> Result<Option<String>, String> {
    let Some(parent) = app
        .dialog()
        .file()
        .set_title("Choose a folder for the Dispatch backup")
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
    else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let store = Store::open(&state.db())?;
        let dir = backup::write(&store, &state.dir, &parent.join("Dispatch backups"))?;
        Ok(Some(dir.display().to_string()))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Puts a backup back: the current state is backed up first (so this can be undone by
/// restoring that one), then the database and topics are replaced and reopened.
/// Asks first (a native dialog); `None` when the user says no.
#[tauri::command]
pub async fn restore_backup(
    app: AppHandle,
    name: String,
    when: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogKind};
    let yes = app
        .dialog()
        .message(format!(
            "Your papers, read state, saved items, notes, tags and topics go back to {when}. \
             What you have now is backed up first, so you can restore it again. \
             Settings stay as they are."
        ))
        .title(format!("Restore the backup from {when}?"))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Restore".into(),
            "Cancel".into(),
        ))
        .blocking_show();
    if !yes {
        return Ok(None);
    }
    tauri::async_runtime::spawn_blocking(move || restore(&app, &name).map(Some))
        .await
        .map_err(|e| e.to_string())?
}

fn restore(app: &AppHandle, name: &str) -> Result<String, String> {
    let state = app.state::<State>();
    let dir = backup::check(&state.dir, name)?;
    if state.refreshing.load(Ordering::SeqCst) || state.backfilling.load(Ordering::SeqCst) {
        return Err("a refresh is running; try again in a minute".into());
    }
    let before = {
        let store = state.store.lock().map_err(|_| "store busy")?;
        backup::rolling(&store, &state.dir)?
    };
    {
        // Close the live connection (an in-memory one holds its place), swap, reopen.
        let mut store = state.store.lock().map_err(|_| "store busy")?;
        *store = Store::in_memory()?;
        let put = backup::put_back(&dir, &state.dir);
        *store = Store::open(&state.db())?;
        put?;
    }
    let topics = Topics::load_or_seed(&state.dir.join(TOPICS_FILE))?;
    if let Ok(mut t) = state.topics.lock() {
        *t = topics;
    }
    state.tray_dirty.store(true, Ordering::SeqCst);
    state.export_dirty.store(true, Ordering::SeqCst);
    let _ = app.emit("topics", ());
    let _ = app.emit("refreshed", ());
    Ok(before
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// From the scheduler, once a minute: a backup when the last is a day old.
pub fn daily_backup(app: &AppHandle) {
    let state = app.state::<State>();
    let Ok(store) = Store::open(&state.db()) else {
        return;
    };
    if backup::due(&store, Utc::now())
        && let Err(e) = backup::rolling(&store, &state.dir)
    {
        eprintln!("dispatch: backup: {e}");
    }
}

// ---- Zotero, local -----------------------------------------------------------------

/// Sends papers to Zotero: the running Zotero app first (its local connector port), else the
/// web API when it's set up. Returns what happened, for the toast.
#[tauri::command]
pub async fn send_to_zotero(app: AppHandle, keys: Vec<String>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let store = Store::open(&state.db())?;
        let items: Vec<_> = keys
            .iter()
            .filter(|k| k.starts_with("arxiv:"))
            .filter_map(|k| store.get(k).ok().flatten())
            .collect();
        if items.is_empty() {
            return Err("no papers to send".into());
        }
        let n = items.len();
        let what = if n == 1 { "1 paper".to_owned() } else { format!("{n} papers") };
        if zotero::running() {
            zotero::send(&items)?;
            return Ok(format!("Sent {what} to the Zotero app"));
        }
        let (user, collection, name) = state
            .settings
            .lock()
            .map(|s| (s.zotero_user, s.zotero_collection.clone(), s.zotero_collection_name.clone()))
            .unwrap_or_default();
        if user == 0 {
            return Err("Zotero isn't open. Start it, or set up Zotero online in Settings → Research workflow.".into());
        }
        let key = keychain::get(ZOTERO_KEY)
            .ok_or("the Zotero API key isn't in the Keychain any more; set it up again in Settings")?;
        let collection = Some(collection.as_str()).filter(|c| !c.is_empty());
        zotero::web_send(user, &key, &items, collection)?;
        Ok(match collection {
            Some(_) => format!("Sent {what} to Zotero online ({name})"),
            None => format!("Sent {what} to Zotero online"),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Keychain account for the Zotero API key (service "Dispatch").
pub const ZOTERO_KEY: &str = "zotero-api-key";

/// Settings → "Zotero online": checks the key with zotero.org, keeps it in the Keychain and
/// the user ID in settings. Returns the zotero.org user name.
#[tauri::command]
pub async fn zotero_connect(app: AppHandle, key: String) -> Result<String, String> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err("paste the key first".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let who = zotero::check_key(&key)?;
        if !who.can_write {
            return Err(
                "this key can't add items: on zotero.org, edit it and tick \"Allow write access\""
                    .into(),
            );
        }
        keychain::set(ZOTERO_KEY, &key)?;
        let state = app.state::<State>();
        let mut settings = state.settings.lock().map_err(|_| "busy")?.clone();
        settings.zotero_user = who.user_id;
        settings.zotero_name = who.username.clone();
        settings.zotero_collection.clear();
        settings.zotero_collection_name.clear();
        settings.save(&state.dir.join(dispatch::settings::FILE))?;
        *state.settings.lock().map_err(|_| "busy")? = settings;
        Ok(who.username)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets the key (removed from the Keychain) and the user.
#[tauri::command]
pub fn zotero_disconnect(state: tauri::State<State>) -> Result<(), String> {
    keychain::delete(ZOTERO_KEY);
    let mut settings = state.settings.lock().map_err(|_| "busy")?.clone();
    settings.zotero_user = 0;
    settings.zotero_name.clear();
    settings.zotero_collection.clear();
    settings.zotero_collection_name.clear();
    settings.save(&state.dir.join(dispatch::settings::FILE))?;
    *state.settings.lock().map_err(|_| "busy")? = settings;
    Ok(())
}

/// The web library's collections, for Settings' menu.
#[tauri::command]
pub async fn zotero_collections(app: AppHandle) -> Result<Vec<zotero::Collection>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let user = app
            .state::<State>()
            .settings
            .lock()
            .map(|s| s.zotero_user)
            .unwrap_or(0);
        if user == 0 {
            return Ok(vec![]);
        }
        let key = keychain::get(ZOTERO_KEY).ok_or("the Zotero API key isn't in the Keychain")?;
        zotero::collections(user, &key)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Semantic Scholar ---------------------------------------

/// Keychain account for the Semantic Scholar API key.
pub const S2_KEY: &str = "semantic-scholar-api-key";

/// One request stream to Semantic Scholar at a time, `s2::PAUSE` apart.
static S2_TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn set_s2(state: &State, on: bool) -> Result<(), String> {
    let mut settings = state.settings.lock().map_err(|_| "busy")?.clone();
    settings.s2_connected = on;
    if on {
        settings.s2_on = true; // a key means he wants it
    }
    settings.save(&state.dir.join(dispatch::settings::FILE))?;
    *state.settings.lock().map_err(|_| "busy")? = settings;
    Ok(())
}

/// Settings → Discovery → "Connect": checks the key with one request, keeps it in the Keychain.
#[tauri::command]
pub async fn s2_connect(app: AppHandle, key: String) -> Result<(), String> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err("paste the key first".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _turn = S2_TURN.lock().map_err(|_| "busy")?;
        s2::check_key(&fetch::Http::new()?, &key)?;
        keychain::set(S2_KEY, &key)?;
        set_s2(&app.state::<State>(), true)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets the key (removed from the Keychain). Cached lists stay readable.
#[tauri::command]
pub fn s2_disconnect(state: tauri::State<State>) -> Result<(), String> {
    keychain::delete(S2_KEY);
    set_s2(&state, false)
}

/// The preview's Semantic Scholar box: cached lists (no network), or with `fetch` true, asks
/// Semantic Scholar (three requests: about 3 s with a key, 9 s without) and caches the answer.
#[tauri::command]
pub async fn s2_lists(
    app: AppHandle,
    key: String,
    fetch: bool,
) -> Result<Option<s2::Lists>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<State>();
        let store = Store::open(&state.db())?;
        if !fetch {
            return Ok(s2::cached(&store, &key));
        }
        let (on, connected) = state
            .settings
            .lock()
            .map(|s| (s.s2_on, s.s2_connected))
            .map_err(|_| "busy")?;
        if !on && !connected {
            return Err("Semantic Scholar is off; turn it on in Settings → Discovery".into());
        }
        // A key is optional; without one, requests go to the shared pool.
        let api_key = if connected {
            Some(keychain::get(S2_KEY).ok_or(
                "the Semantic Scholar key isn't in the Keychain any more; connect again in Settings → Discovery",
            )?)
        } else {
            None
        };
        let item = store.get(&key)?.ok_or("paper not found")?;
        let _turn = S2_TURN.lock().map_err(|_| "busy")?;
        let pause = s2::pause_for(api_key.as_deref());
        let lists = s2::fetch(&store, &item, &fetch::Http::new()?, api_key.as_deref(), pause, Utc::now());
        std::thread::sleep(pause);
        lists.map(Some)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The macOS Keychain (generic passwords under the service "Dispatch").
mod keychain {
    const SERVICE: &str = "Dispatch";

    #[cfg(target_os = "macos")]
    pub fn set(account: &str, secret: &str) -> Result<(), String> {
        security_framework::passwords::set_generic_password(SERVICE, account, secret.as_bytes())
            .map_err(|e| format!("couldn't save to the Keychain: {e}"))
    }

    #[cfg(target_os = "macos")]
    pub fn get(account: &str) -> Option<String> {
        security_framework::passwords::get_generic_password(SERVICE, account)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    }

    #[cfg(target_os = "macos")]
    pub fn delete(account: &str) {
        let _ = security_framework::passwords::delete_generic_password(SERVICE, account);
    }

    #[cfg(not(target_os = "macos"))]
    pub fn set(_account: &str, _secret: &str) -> Result<(), String> {
        Err("the Keychain is macOS only".into())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn get(_account: &str) -> Option<String> {
        None
    }

    #[cfg(not(target_os = "macos"))]
    pub fn delete(_account: &str) {}
}

// ---- dispatch:// links -------------------------------------------------------------

/// macOS handed the app one or more `dispatch://` links (`RunEvent::Opened`).
pub fn opened(app: &AppHandle, urls: &[tauri::Url]) {
    for url in urls {
        match links::parse(url.as_str()) {
            Ok(Link::Refresh) => {
                crate::start_refresh(app, dispatch::refresh::Scope::All);
            }
            Ok(Link::Add { input }) => add_paper(app, input),
            Ok(link) => show_link(app, link),
            Err(e) => {
                eprintln!("dispatch: {e}");
                // Never silent: a Shortcut run from another app gets an answer.
                failed(app, e);
            }
        }
    }
}

/// Shows the window with the reason a link or an add didn't work (a toast), and says so
/// beside the menu-bar icon. The page may still be loading after a cold start, so the
/// message goes out a moment later.
fn failed(app: &AppHandle, message: String) {
    crate::tray::flash(app, "Couldn't use that link");
    crate::show_main(app);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        let _ = app.emit("add-failed", message);
    });
}

/// Shows the window and hands the page a link (kept until the page takes it).
fn show_link(app: &AppHandle, link: Link) {
    crate::show_main(app);
    // With LSUIElement, macOS re-reads the app's type when it delivers a link and can drop
    // the Dock icon; put it back once things settle (as Palette does).
    let later = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        if later
            .get_webview_window("main")
            .is_some_and(|w| w.is_visible().unwrap_or(false))
        {
            #[cfg(target_os = "macos")]
            let _ = later.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
    });
    // The page may still be loading after a cold start: keep it until it asks.
    if let Ok(mut p) = app.state::<State>().pending_link.lock() {
        *p = Some(link.clone());
    }
    let _ = app.emit("link", link);
}

/// `dispatch://add?q=…` (a Shortcut, Settings → Links): saves the paper in the
/// background (arXiv's API can take half a minute), then shows it in Saved. Progress and
/// errors appear beside the menu-bar icon, so a Shortcut run from another app gets an answer.
fn add_paper(app: &AppHandle, input: String) {
    crate::tray::flash(app, "Adding to Dispatch…");
    let app = app.clone();
    std::thread::spawn(move || match add_now(&app, &input) {
        Ok(a) => {
            crate::tray::flash(&app, if a.new { "Added to Saved" } else { "Saved" });
            show_link(
                &app,
                Link::Added {
                    key: a.key,
                    title: a.title,
                },
            );
        }
        Err(e) => {
            eprintln!("dispatch: add: {e}");
            failed(&app, e);
        }
    });
}

/// One add at a time, at least 3 s apart (arXiv's API rule, as for backfill).
static ADD_TURN: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn add_now(app: &AppHandle, input: &str) -> Result<dispatch::add::Added, String> {
    let mut turn = ADD_TURN.lock().map_err(|_| "busy")?;
    if let Some(wait) = turn.and_then(|t| dispatch::refresh::API_PAUSE.checked_sub(t.elapsed())) {
        std::thread::sleep(wait);
    }
    let state = app.state::<State>();
    let topics = state.topics.lock().map(|t| t.clone()).map_err(|_| "busy")?;
    // Its own connection, so the window's store isn't held during the request.
    let mut store = Store::open(&state.db())?;
    let added = dispatch::add::add(&mut store, &topics, input, &fetch::Http::new()?, Utc::now());
    *turn = Some(std::time::Instant::now());
    state.changed();
    added
}

/// Settings → Links → Add a paper: the same as the link, with the answer for the page.
#[tauri::command]
pub async fn add_paper_now(app: AppHandle, input: String) -> Result<dispatch::add::Added, String> {
    tauri::async_runtime::spawn_blocking(move || add_now(&app, &input))
        .await
        .map_err(|e| e.to_string())?
}

/// Copies a `dispatch://` link (or any short text) to the clipboard (Settings → Links, the
/// preview's "Copy Dispatch link").
#[tauri::command]
pub fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}

/// The page asks once it's loaded, so a link that started the app isn't lost. Also clears
/// it after the `link` event was handled.
#[tauri::command]
pub fn take_link(state: tauri::State<State>) -> Option<Link> {
    state.pending_link.lock().ok().and_then(|mut p| p.take())
}
