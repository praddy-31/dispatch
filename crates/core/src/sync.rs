//! iCloud sync of the user's own data.
//!
//! One JSON file, `Dispatch/dispatch-sync.json` in iCloud Drive, holds what can't be fetched
//! again: topics, saved items (with enough of the item to show it on a Mac that never fetched
//! it), notes, tags and read state. Fetched items and settings stay on each Mac.
//!
//! Merging is field by field with change times (milliseconds): per item, saved, note, tags
//! and read state each keep the newer side; topics as a whole keep the newer side. So two Macs can both change things between syncs without losing either change.
//!
//! iCloud details (learned in the Habit Tracker):
//! - when two Macs write at once, iCloud keeps both: "dispatch-sync 2.json" or
//!   "dispatch-sync (conflicted copy …).json". Those are merged in too, then moved to the
//!   local `sync-conflicts/` folder (never deleted);
//! - iCloud may offload a file to save space (only `.dispatch-sync.json.icloud` is left):
//!   `brctl download` brings it back; this round is skipped if it isn't back in 10 s.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::item::Item;
use crate::paths::write_atomic;
use crate::store::{Store, UserData};
use crate::topics::Topics;

pub const FILE: &str = "dispatch-sync.json";
const STEM: &str = "dispatch-sync";
/// Read state older than this isn't carried in the file (keeps it small).
pub const READ_DAYS: i64 = 60;
pub const TOPICS_STAMP: &str = "topics_stamp";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topics: Option<StampedTopics>,
    #[serde(default)]
    pub items: BTreeMap<String, Entry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StampedTopics {
    pub stamp: i64,
    pub value: Topics,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserData>,
    /// (read_at in seconds or None for unread, change time in ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<(Option<i64>, i64)>,
    /// The item itself, for saved items, so another Mac can show it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<Item>,
}

/// The default folder: iCloud Drive's `Dispatch` folder.
pub fn icloud_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let drive = PathBuf::from(home).join("Library/Mobile Documents/com~apple~CloudDocs");
    drive.is_dir().then(|| drive.join("Dispatch"))
}

/// Field by field, the newer side wins. Items only in one file are kept.
pub fn merge(a: SyncFile, b: SyncFile) -> SyncFile {
    let topics = match (a.topics, b.topics) {
        (Some(x), Some(y)) => Some(if y.stamp > x.stamp { y } else { x }),
        (x, y) => x.or(y),
    };
    let mut items = a.items;
    for (key, theirs) in b.items {
        let mine = items.entry(key).or_default();
        mine.user = match (mine.user.take(), theirs.user) {
            (Some(m), Some(t)) => Some(m.merged(&t)),
            (m, t) => m.or(t),
        };
        let newer_read = match (&mine.read, &theirs.read) {
            (Some(m), Some(t)) => t.1 > m.1,
            (None, Some(_)) => true,
            _ => false,
        };
        if newer_read {
            mine.read = theirs.read;
        }
        if mine.item.is_none() {
            mine.item = theirs.item;
        }
    }
    SyncFile {
        version: 1,
        topics,
        items,
    }
}

/// This Mac's side, from the store and topics.json.
pub fn export(store: &Store, topics: &Topics, now: DateTime<Utc>) -> Result<SyncFile, String> {
    let mut items: BTreeMap<String, Entry> = BTreeMap::new();
    for (key, user) in store.user_rows()? {
        let saved = user.saved_at.is_some();
        let e = items.entry(key.clone()).or_default();
        if saved {
            e.item = store.get(&key)?;
        }
        e.user = Some(user);
    }
    let since = (now - chrono::Duration::days(READ_DAYS)).timestamp_millis();
    for (key, read_at, stamp) in store.read_rows(since)? {
        items.entry(key).or_default().read = Some((read_at, stamp));
    }
    let stamp = store
        .meta(TOPICS_STAMP)?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok(SyncFile {
        version: 1,
        topics: Some(StampedTopics {
            stamp,
            value: topics.clone(),
        }),
        items,
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub at: Option<DateTime<Utc>>,
    /// Changes taken from the other Mac(s).
    pub pulled: usize,
    /// Whether the topics came from another Mac (the caller reloads them).
    pub topics_updated: bool,
    /// Conflict copies merged and moved aside.
    pub conflicts: usize,
    /// Set when this round was skipped (file still downloading).
    pub skipped: Option<String>,
}

/// One sync round: read the file (and any conflict copies), merge with this Mac, apply what is
/// newer here, and write the merged file back if it changed.
pub fn run(
    store: &mut Store,
    topics: &Topics,
    topics_path: &Path,
    folder: &Path,
    local_dir: &Path,
    now: DateTime<Utc>,
) -> Result<Report, String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("can't create {}: {e}", folder.display()))?;
    let main = folder.join(FILE);
    if !ensure_downloaded(&main) {
        return Ok(Report {
            at: Some(now),
            skipped: Some("iCloud is still downloading the sync file".into()),
            ..Report::default()
        });
    }

    let local = export(store, topics, now)?;
    let mut merged = local.clone();
    let mut on_disk: Option<String> = None;
    if let Ok(text) = std::fs::read_to_string(&main) {
        let theirs: SyncFile = serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} is not valid ({e}); not syncing until it is fixed",
                main.display()
            )
        })?;
        merged = merge(merged, theirs);
        on_disk = Some(text);
    }
    let conflicts = conflict_copies(folder);
    for c in &conflicts {
        if let Ok(text) = std::fs::read_to_string(c)
            && let Ok(theirs) = serde_json::from_str::<SyncFile>(&text)
        {
            merged = merge(merged, theirs);
        }
    }

    let mut report = Report {
        at: Some(now),
        conflicts: conflicts.len(),
        ..Report::default()
    };

    // Apply to this Mac whatever is newer than ours.
    for (key, e) in &merged.items {
        let mine = local.items.get(key);
        if let Some(item) = &e.item {
            store.insert_if_missing(item, now)?;
        }
        if let Some(u) = &e.user
            && mine.and_then(|m| m.user.as_ref()) != Some(u)
        {
            store.put_user(key, u)?;
            report.pulled += 1;
        }
        if let Some((read_at, stamp)) = e.read
            && mine.and_then(|m| m.read).is_none_or(|m| stamp > m.1)
            && store.apply_read(key, read_at, stamp)?
        {
            report.pulled += 1;
        }
    }
    if let (Some(theirs), Some(ours)) = (&merged.topics, &local.topics)
        && theirs.stamp > ours.stamp
    {
        theirs.value.validate()?;
        theirs.value.save(topics_path)?;
        store.set_meta(TOPICS_STAMP, &theirs.stamp.to_string())?;
        store.retag(&theirs.value)?;
        report.topics_updated = true;
    }

    // Drop old read-only entries so the file stays small.
    let since = (now - chrono::Duration::days(READ_DAYS)).timestamp_millis();
    merged
        .items
        .retain(|_, e| e.user.is_some() || e.read.is_some_and(|(_, stamp)| stamp >= since));
    let text = serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())? + "\n";
    if on_disk.as_deref() != Some(text.as_str()) {
        write_atomic(&main, text.as_bytes())?;
    }

    // Conflict copies are merged now: move them out of iCloud, keep them locally.
    if !conflicts.is_empty() {
        let keep = local_dir.join("sync-conflicts");
        std::fs::create_dir_all(&keep).map_err(|e| e.to_string())?;
        for c in &conflicts {
            if let Some(name) = c.file_name() {
                let to = keep.join(format!(
                    "{}-{}",
                    now.format("%Y%m%d-%H%M%S"),
                    name.to_string_lossy()
                ));
                let _ = std::fs::rename(c, &to)
                    .or_else(|_| std::fs::copy(c, &to).and_then(|_| std::fs::remove_file(c)));
            }
        }
    }
    Ok(report)
}

/// "dispatch-sync 2.json", "dispatch-sync (conflicted copy …).json".
pub fn is_conflict_name(n: &str) -> bool {
    n != FILE
        && n.starts_with(STEM)
        && n.ends_with(".json")
        && n[STEM.len()..].starts_with([' ', '-', '_', '('])
}

fn conflict_copies(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return vec![];
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| is_conflict_name(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// False if only iCloud's placeholder is there and the file didn't come back in 10 s.
fn ensure_downloaded(file: &Path) -> bool {
    if file.exists() {
        return true;
    }
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return true;
    };
    let placeholder = dir.join(format!(".{}.icloud", name.to_string_lossy()));
    if !placeholder.exists() {
        return true; // no file yet: the first sync creates it
    }
    let _ = std::process::Command::new("/usr/bin/brctl")
        .arg("download")
        .arg(file)
        .status();
    for _ in 0..40 {
        if file.exists() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(saved: bool, note: &str, stamp: i64) -> UserData {
        UserData {
            saved_at: saved.then_some(1),
            saved_stamp: stamp,
            note: note.into(),
            note_stamp: stamp,
            tags: vec![],
            tags_stamp: 0,
        }
    }

    #[test]
    fn merge_keeps_the_newer_side_per_field() {
        let mut a = SyncFile::default();
        let mut b = SyncFile::default();
        a.items.insert(
            "k".into(),
            Entry {
                user: Some(user(true, "mine", 10)),
                read: Some((Some(5), 30)),
                item: None,
            },
        );
        b.items.insert(
            "k".into(),
            Entry {
                user: Some(user(true, "theirs", 20)),
                read: Some((None, 25)),
                item: None,
            },
        );
        b.items.insert(
            "only-b".into(),
            Entry {
                user: Some(user(true, "", 1)),
                read: None,
                item: None,
            },
        );
        let m = merge(a, b);
        assert_eq!(
            m.items["k"].user.as_ref().unwrap().note,
            "theirs",
            "their note is newer"
        );
        assert_eq!(
            m.items["k"].read,
            Some((Some(5), 30)),
            "our read state is newer"
        );
        assert!(m.items.contains_key("only-b"));
    }

    #[test]
    fn conflict_names() {
        assert!(is_conflict_name("dispatch-sync 2.json"));
        assert!(is_conflict_name(
            "dispatch-sync (conflicted copy 2026-10-30).json"
        ));
        assert!(!is_conflict_name("dispatch-sync.json"));
        assert!(!is_conflict_name("dispatch-sync.json.tmp"));
        assert!(!is_conflict_name("other.json"));
    }
}
