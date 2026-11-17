//! Data health and the plain-text export.
//!
//! - `check`: items per source, sources gone quiet, notes without a paper, Obsidian notes whose
//!   paper is no longer saved, and SQLite's own integrity check. Read only; "compact" is
//!   `Store::compact`.
//! - `export_all`: everything in files any program can read: `dispatch.json` (every item with
//!   your saves, notes, tags and read state, plus topics and settings) and `saved.csv` (saved
//!   papers, one row each), in a new dated folder inside the one you chose.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::item::Section;
use crate::paths::write_atomic;
use crate::settings::Settings;
use crate::store::{Row, Store};
use crate::topics::Topics;

/// A followed news feed with nothing new for this long is "quiet".
pub const QUIET_NEWS_DAYS: i64 = 7;
/// arXiv: a long weekend plus a holiday is four days without papers.
pub const QUIET_ARXIV_DAYS: i64 = 4;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceHealth {
    pub name: String,
    pub section: String,
    /// Items stored from it.
    pub items: i64,
    /// The newest item's first sighting.
    pub newest: Option<DateTime<Utc>>,
    /// Followed, but nothing new for a while (or ever).
    pub quiet: bool,
    /// The last fetch failed with this.
    pub error: Option<String>,
    /// No longer in your topics (its items stay).
    pub unfollowed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Health {
    pub sources: Vec<SourceHealth>,
    /// Saves, notes or tags for items this database doesn't have.
    pub orphaned: Vec<String>,
    /// Obsidian notes (file names) whose paper isn't saved any more; Dispatch leaves them.
    pub orphaned_notes: Vec<String>,
    /// "ok", or what SQLite found.
    pub integrity: String,
}

pub fn check(
    store: &Store,
    topics: &Topics,
    settings: &Settings,
    data_dir: &Path,
    now: DateTime<Utc>,
) -> Result<Health, String> {
    let statuses = store.statuses()?;
    let mut followed: Vec<(String, String, Section)> = topics
        .news_feeds()
        .into_iter()
        .map(|f| (f.url, f.name, Section::News))
        .collect();
    if !topics.arxiv_categories().is_empty() {
        followed.push((
            crate::sources::arxiv::SOURCE_ID.into(),
            "arXiv".into(),
            Section::Research,
        ));
    }
    let stored = store.items_by_source()?;
    let mut sources: Vec<SourceHealth> = Vec::new();
    for (url, name, section) in &followed {
        let s = stored.iter().find(|s| &s.0 == url);
        let newest = s.and_then(|s| s.4);
        let days = match section {
            Section::Research => QUIET_ARXIV_DAYS,
            Section::News => QUIET_NEWS_DAYS,
        };
        sources.push(SourceHealth {
            name: name.clone(),
            section: section.as_str().into(),
            items: s.map_or(0, |s| s.3),
            newest,
            quiet: newest.is_none_or(|t| now - t > Duration::days(days)),
            error: statuses
                .iter()
                .find(|st| &st.source == url)
                .and_then(|st| st.last_error.clone()),
            unfollowed: false,
        });
    }
    for (url, name, section, items, newest) in &stored {
        if !followed.iter().any(|f| &f.0 == url) {
            sources.push(SourceHealth {
                name: name.clone(),
                section: section.clone(),
                items: *items,
                newest: *newest,
                quiet: false,
                error: None,
                unfollowed: true,
            });
        }
    }

    let mut orphaned_notes = Vec::new();
    if !settings.notes_dir.is_empty() {
        let saved: Vec<String> = store
            .list_saved(None, usize::MAX >> 1)?
            .into_iter()
            .map(|r| r.item.key)
            .collect();
        let m = crate::vault::Manifest::load(data_dir, Path::new(&settings.notes_dir));
        orphaned_notes = m
            .notes
            .iter()
            .filter(|(k, _)| !k.starts_with("log:") && !saved.contains(k))
            .map(|(_, w)| w.file.clone())
            .collect();
    }

    Ok(Health {
        sources,
        orphaned: store.orphaned_user_rows()?,
        orphaned_notes,
        integrity: store.integrity()?,
    })
}

// ---- plain export -------------------------------------------------------------------

#[derive(Serialize)]
struct Everything<'a> {
    app: &'static str,
    exported: DateTime<Utc>,
    topics: &'a Topics,
    settings: &'a Settings,
    items: &'a [Row],
}

/// One CSV field: quoted when it has to be, quotes doubled (RFC 4180).
fn csv(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

pub fn saved_csv(rows: &[Row]) -> String {
    let mut out = String::from("title,authors,year,arxiv,doi,link,tags,note,saved\r\n");
    for r in rows.iter().filter(|r| r.saved) {
        let i = &r.item;
        let year = i
            .published
            .map(|t| t.format("%Y").to_string())
            .unwrap_or_default();
        let arxiv = i.key.strip_prefix("arxiv:").unwrap_or("");
        let saved = r
            .saved_at
            .map(|t| t.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let fields = [
            i.title.as_str(),
            &i.authors.join("; "),
            &year,
            arxiv,
            i.doi.as_deref().unwrap_or(""),
            &i.link,
            &r.tags.join("; "),
            &r.note,
            &saved,
        ];
        out += &fields.iter().map(|f| csv(f)).collect::<Vec<_>>().join(",");
        out += "\r\n";
    }
    out
}

/// Writes `Dispatch export YYYY-MM-DD/` inside `parent` (a second export the same day gets
/// " (2)"); returns the folder.
pub fn export_all(
    store: &Store,
    topics: &Topics,
    settings: &Settings,
    parent: &Path,
    now: DateTime<Utc>,
) -> Result<PathBuf, String> {
    let base = format!("Dispatch export {}", now.format("%Y-%m-%d"));
    let mut dir = parent.join(&base);
    let mut n = 2;
    while dir.exists() {
        dir = parent.join(format!("{base} ({n})"));
        n += 1;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let rows = store.all_rows()?;
    let all = Everything {
        app: "Dispatch",
        exported: now,
        topics,
        settings,
        items: &rows,
    };
    let json = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())? + "\n";
    write_atomic(&dir.join("dispatch.json"), json.as_bytes())?;
    write_atomic(&dir.join("saved.csv"), saved_csv(&rows).as_bytes())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Item;

    fn paper(id: &str, title: &str) -> Item {
        Item {
            key: format!("arxiv:{id}"),
            section: Section::Research,
            source: crate::sources::arxiv::SOURCE_ID.into(),
            source_name: "arXiv".into(),
            title: title.into(),
            authors: vec!["A. Smith".into(), "B. Jones".into()],
            summary: "Abstract.".into(),
            link: format!("https://arxiv.org/abs/{id}"),
            pdf: None,
            categories: vec!["quant-ph".into()],
            published: "2026-10-01T00:00:00Z".parse().ok(),
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn health_and_export() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("t.sqlite")).unwrap();
        let now: DateTime<Utc> = "2026-11-03T06:00:00Z".parse().unwrap();
        store
            .upsert(
                &[
                    paper("2410.00001", "Spin chains, \"quoted\""),
                    paper("2410.00002", "Other"),
                ],
                now - Duration::days(10),
            )
            .unwrap();
        store
            .set_saved(&["arxiv:2410.00001".into()], true, now)
            .unwrap();
        store
            .set_note("arxiv:2410.00001", "Read §3, then, the appendix", now)
            .unwrap();
        // A note synced from another Mac for a paper this one never fetched.
        store
            .set_note("arxiv:2410.09999", "elsewhere", now)
            .unwrap();

        let topics = Topics::default();
        let settings = Settings::default();
        let h = check(&store, &topics, &settings, dir.path(), now).unwrap();
        assert_eq!(h.integrity, "ok");
        assert_eq!(h.orphaned, ["arxiv:2410.09999"]);
        if let Some(arxiv) = h.sources.iter().find(|s| s.name == "arXiv") {
            assert_eq!(arxiv.items, 2);
            assert!(arxiv.quiet, "nothing new for ten days");
        }

        let out = export_all(&store, &topics, &settings, dir.path(), now).unwrap();
        assert!(out.ends_with("Dispatch export 2026-11-03"));
        let csv = std::fs::read_to_string(out.join("saved.csv")).unwrap();
        let lines: Vec<_> = csv.lines().collect();
        assert_eq!(lines.len(), 2, "header + the one saved paper");
        assert!(
            lines[1]
                .starts_with("\"Spin chains, \"\"quoted\"\"\",A. Smith; B. Jones,2026,2410.00001")
        );
        assert!(lines[1].contains("\"Read §3, then, the appendix\""));
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(out.join("dispatch.json")).unwrap())
                .unwrap();
        assert_eq!(json["items"].as_array().unwrap().len(), 2);
        // A second export the same day doesn't replace the first.
        let again = export_all(&store, &topics, &settings, dir.path(), now).unwrap();
        assert!(again.ends_with("Dispatch export 2026-11-03 (2)"));
    }
}
