//! Backup and restore. A backup is a folder named after its time,
//! `backups/2026-11-01_1330/`, holding a consistent copy of the database (`VACUUM INTO`, safe
//! while the app is writing) plus `topics.json` and `settings.json`. Nothing leaves the Mac.
//!
//! The app makes one a day by itself and keeps the newest [`KEEP`]; "Back up now" makes one on
//! request and "Export…" writes the same folder somewhere you choose (never pruned). Restore
//! puts back the database and topics (settings are this Mac's preferences and stay as they
//! are), after first backing up the current state, so a restore can itself be undone.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use serde::Serialize;

use crate::{DB_FILE, Store, TOPICS_FILE, settings};

/// The folder inside the data folder.
pub const DIR: &str = "backups";
/// Automatic and "Back up now" backups kept; older ones are removed.
pub const KEEP: usize = 5;
/// `meta` key: when the last backup was made (the daily one waits 24 h after it).
pub const LAST: &str = "last_backup";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Backup {
    /// Folder name, e.g. `2026-11-01_1330`; also what restore is given.
    pub name: String,
    pub path: String,
    pub bytes: u64,
    /// When it was made (from the database file's time).
    pub at: Option<DateTime<Utc>>,
}

/// Writes a backup folder into `parent` (named after the local time) and returns its path.
/// `store` is the open database: `VACUUM INTO` copies it as one consistent snapshot.
pub fn write(store: &Store, data_dir: &Path, parent: &Path) -> Result<PathBuf, String> {
    let base = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let mut dir = parent.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = parent.join(format!("{base}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let done = (|| {
        store.copy_to(&dir.join(DB_FILE))?;
        for file in [TOPICS_FILE, settings::FILE] {
            let from = data_dir.join(file);
            if from.exists() {
                std::fs::copy(&from, dir.join(file))
                    .map_err(|e| format!("can't copy {file}: {e}"))?;
            }
        }
        Ok::<(), String>(())
    })();
    if let Err(e) = done {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    Ok(dir)
}

/// A backup into the data folder's `backups/`, then removes all but the newest [`KEEP`].
pub fn rolling(store: &Store, data_dir: &Path) -> Result<PathBuf, String> {
    let parent = data_dir.join(DIR);
    let dir = write(store, data_dir, &parent)?;
    store.set_time_meta(LAST, Utc::now())?;
    for old in list(data_dir).into_iter().skip(KEEP) {
        let _ = std::fs::remove_dir_all(&old.path);
    }
    Ok(dir)
}

/// The backups in `backups/`, newest first. Folders without a database are ignored.
pub fn list(data_dir: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(data_dir.join(DIR)) else {
        return Vec::new();
    };
    let mut out: Vec<Backup> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join(DB_FILE).is_file())
        .map(|p| {
            let meta = std::fs::metadata(p.join(DB_FILE)).ok();
            Backup {
                name: p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                bytes: meta.as_ref().map_or(0, |m| m.len()),
                at: meta
                    .and_then(|m| m.modified().ok())
                    .map(DateTime::<Utc>::from),
                path: p.display().to_string(),
            }
        })
        .collect();
    // Names sort by time (`YYYY-MM-DD_HHMMSS`), so the newest is last alphabetically.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Due once a day: 24 hours after the last backup (or never made).
pub fn due(store: &Store, now: DateTime<Utc>) -> bool {
    store
        .time_meta(LAST)
        .is_none_or(|t| now - t >= chrono::Duration::hours(24))
}

/// Checks a backup before restoring it: the folder exists inside `backups/` (no paths from
/// elsewhere) and its database opens as a Dispatch database. Returns the folder.
pub fn check(data_dir: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return Err("not a backup name".into());
    }
    let dir = data_dir.join(DIR).join(name);
    let db = dir.join(DB_FILE);
    if !db.is_file() {
        return Err(format!("no backup called {name}"));
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("the backup's database won't open: {e}"))?;
    conn.query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
        .map_err(|e| format!("the backup's database isn't Dispatch's: {e}"))?;
    Ok(dir)
}

/// Puts a checked backup's database and topics in place. The caller must have closed every
/// connection to the live database first (the app swaps its `Store` for an in-memory one).
/// The database is copied beside the live one and renamed over it, and the old WAL files go,
/// so SQLite never mixes the two.
pub fn put_back(backup_dir: &Path, data_dir: &Path) -> Result<(), String> {
    let live = data_dir.join(DB_FILE);
    let tmp = data_dir.join(format!(".{DB_FILE}.restore"));
    std::fs::copy(backup_dir.join(DB_FILE), &tmp)
        .map_err(|e| format!("can't copy the backup: {e}"))?;
    for ext in ["sqlite-wal", "sqlite-shm"] {
        let _ = std::fs::remove_file(live.with_extension(ext));
    }
    std::fs::rename(&tmp, &live).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't replace the database: {e}")
    })?;
    let topics = backup_dir.join(TOPICS_FILE);
    if topics.is_file() {
        let bytes = std::fs::read(&topics).map_err(|e| e.to_string())?;
        crate::paths::write_atomic(&data_dir.join(TOPICS_FILE), &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Item, Section};

    fn paper(id: &str) -> Item {
        Item {
            key: format!("arxiv:{id}"),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: format!("Paper {id}"),
            authors: vec!["A. Author".into()],
            summary: "Text".into(),
            link: format!("https://arxiv.org/abs/{id}"),
            pdf: None,
            categories: vec!["quant-ph".into()],
            published: None,
            announce_type: Some("new".into()),
            version: Some(1),
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn backup_restore_round_trip_and_rolling_limit() {
        let home = tempfile::tempdir().unwrap();
        let data = home.path();
        std::fs::write(data.join(TOPICS_FILE), b"{\"old\":1}").unwrap();
        let mut store = Store::open(&data.join(DB_FILE)).unwrap();
        store.upsert(&[paper("2410.00001")], Utc::now()).unwrap();
        assert!(due(&store, Utc::now()));
        let first = rolling(&store, data).unwrap();
        assert!(!due(&store, Utc::now()));
        assert!(first.join(DB_FILE).is_file() && first.join(TOPICS_FILE).is_file());

        // Change things after the backup...
        store.upsert(&[paper("2410.00002")], Utc::now()).unwrap();
        std::fs::write(data.join(TOPICS_FILE), b"{\"new\":1}").unwrap();
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        let dir = check(data, &name).unwrap();
        assert!(check(data, "../x").is_err() && check(data, "nope").is_err());

        // ...and put the backup back, as the app does: close, replace, reopen.
        drop(store);
        put_back(&dir, data).unwrap();
        let store = Store::open(&data.join(DB_FILE)).unwrap();
        assert!(store.get("arxiv:2410.00001").unwrap().is_some());
        assert!(store.get("arxiv:2410.00002").unwrap().is_none());
        assert_eq!(
            std::fs::read(data.join(TOPICS_FILE)).unwrap(),
            b"{\"old\":1}"
        );

        for _ in 0..KEEP + 2 {
            rolling(&store, data).unwrap();
        }
        assert_eq!(list(data).len(), KEEP);
    }
}
