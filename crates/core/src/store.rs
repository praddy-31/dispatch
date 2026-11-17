//! The SQLite store: `dispatch.sqlite` in the data folder.
//!
//! Items are only ever added or updated, never removed by a refresh, so a failed or empty
//! fetch can't wipe what the user already has. The schema version is SQLite's
//! `user_version`; `migrate` brings an older file up to date step by step.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::item::{Item, Section};
use crate::search;
use crate::topics::Topics;

pub const SCHEMA_VERSION: i32 = 7;

pub struct Store {
    conn: Connection,
}

/// An item as the UI sees it: the item plus what Dispatch knows about it.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    #[serde(flatten)]
    pub item: Item,
    pub topics: Vec<String>,
    pub first_seen: DateTime<Utc>,
    pub read: bool,
    /// Its page or PDF was opened since Stats last started counting (read, not only seen).
    pub opened: bool,
    /// When it went on the "Read later" queue; the UI keeps only today's.
    pub later_at: Option<DateTime<Utc>>,
    pub saved: bool,
    pub saved_at: Option<DateTime<Utc>>,
    pub note: String,
    pub tags: Vec<String>,
    /// Followed authors who wrote it (:mark`, not stored).
    pub followed: Vec<String>,
    /// "For you" score and its reasons (:score_rows`, not stored).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub why: Vec<String>,
    /// Code and data links in the abstract (`hints.rs`; papers only, not stored).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub code: Vec<crate::hints::CodeLink>,
    /// "long abstract", "maths-heavy" (`hints.rs`; papers only, not stored).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
    /// A news story's discussion page, when its feed names one (`news.rs`; not stored).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discussion: Option<crate::news::Discussion>,
    /// The same story from other feeds, folded into this row (`news::merge`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<crate::news::Also>,
}

/// The user's own data about one item: saved, note, tags. Each field has its own
/// change time in milliseconds, so two Macs changing different fields of the same item both
/// keep their change when they sync (`sync.rs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize)]
pub struct UserData {
    pub saved_at: Option<i64>,
    pub saved_stamp: i64,
    pub note: String,
    pub note_stamp: i64,
    pub tags: Vec<String>,
    pub tags_stamp: i64,
}

impl UserData {
    /// Field by field, the newer side wins.
    pub fn merged(&self, other: &UserData) -> UserData {
        let mut m = self.clone();
        if other.saved_stamp > m.saved_stamp {
            m.saved_at = other.saved_at;
            m.saved_stamp = other.saved_stamp;
        }
        if other.note_stamp > m.note_stamp {
            m.note = other.note.clone();
            m.note_stamp = other.note_stamp;
        }
        if other.tags_stamp > m.tags_stamp {
            m.tags = other.tags.clone();
            m.tags_stamp = other.tags_stamp;
        }
        m
    }
}

const USER_COLS: &str = "saved_at, saved_stamp, note, note_stamp, tags, tags_stamp";

fn user_from(r: &rusqlite::Row, at: usize) -> rusqlite::Result<UserData> {
    Ok(UserData {
        saved_at: r.get(at)?,
        saved_stamp: r.get(at + 1)?,
        note: r.get(at + 2)?,
        note_stamp: r.get(at + 3)?,
        tags: serde_json::from_str(&r.get::<_, String>(at + 4)?).unwrap_or_default(),
        tags_stamp: r.get(at + 5)?,
    })
}

/// What a search looks in: one section or both, one topic, only saved or unread items, only
/// items published (or first seen) since a time, and the order. These replaced the
/// list's Filter box.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct SearchFilter {
    pub section: Option<Section>,
    pub topic: Option<String>,
    pub saved_only: bool,
    pub unread_only: bool,
    /// Unix seconds.
    pub since: Option<i64>,
    pub sort: search::Sort,
}

/// One search result: the row as lists show it, plus a highlighted piece of the abstract.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub row: Row,
    pub snippet: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    /// All matches, which can be more than `hits` (the list stops at a limit).
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceStatus {
    pub source: String,
    pub name: String,
    pub section: String,
    pub last_try: Option<DateTime<Utc>>,
    pub last_ok: Option<DateTime<Utc>>,
    /// Set when the latest attempt failed; cleared by the next success.
    pub last_error: Option<String>,
    pub last_count: Option<i64>,
}

/// (source, name, section, items, newest first_seen): `Store::items_by_source`.
pub type SourceItems = (String, String, String, i64, Option<DateTime<Utc>>);

impl Store {
    pub fn open(path: &Path) -> Result<Store, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        }
        let conn =
            Connection::open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
        Store::setup(conn)
    }

    pub fn in_memory() -> Result<Store, String> {
        Store::setup(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn setup(conn: Connection) -> Result<Store, String> {
        // busy_timeout: the window and a background refresh or backfill can both write;
        // the second waits up to 5 s instead of failing with "database is locked".
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;",
        )
        .map_err(err)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn schema_version(&self) -> i32 {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0)
    }

    fn migrate(&mut self) -> Result<(), String> {
        let from = self.schema_version();
        if from > SCHEMA_VERSION {
            return Err(format!(
                "this database is from a newer Dispatch (schema {from}, this app knows {SCHEMA_VERSION})"
            ));
        }
        let tx = self.conn.transaction().map_err(err)?;
        if from < 1 {
            tx.execute_batch(
                "CREATE TABLE items (
                    key TEXT PRIMARY KEY,
                    section TEXT NOT NULL,
                    source TEXT NOT NULL,
                    source_name TEXT NOT NULL,
                    title TEXT NOT NULL,
                    authors TEXT NOT NULL,
                    summary TEXT NOT NULL,
                    link TEXT NOT NULL,
                    pdf TEXT,
                    categories TEXT NOT NULL,
                    published INTEGER,
                    announce_type TEXT,
                    version INTEGER,
                    doi TEXT,
                    journal_ref TEXT,
                    first_seen INTEGER NOT NULL,
                    last_seen INTEGER NOT NULL,
                    read_at INTEGER
                 );
                 CREATE INDEX items_section_published ON items(section, published DESC);
                 CREATE TABLE item_topics (
                    key TEXT NOT NULL REFERENCES items(key) ON DELETE CASCADE,
                    topic TEXT NOT NULL,
                    PRIMARY KEY (key, topic)
                 );
                 CREATE INDEX item_topics_topic ON item_topics(topic);
                 CREATE TABLE source_status (
                    source TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    section TEXT NOT NULL,
                    last_try INTEGER,
                    last_ok INTEGER,
                    last_error TEXT,
                    last_count INTEGER
                 );
                 CREATE TABLE fetch_log (
                    id INTEGER PRIMARY KEY,
                    at INTEGER NOT NULL,
                    source TEXT NOT NULL,
                    ok INTEGER NOT NULL,
                    count INTEGER,
                    error TEXT,
                    build_date TEXT
                 );
                 CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 PRAGMA user_version = 1;",
            )
            .map_err(err)?;
        }
        if from < 2 {
            // Citations fetched from arXiv or doi.org, so a second copy needs no network.
            tx.execute_batch(
                "CREATE TABLE citations (
                    key TEXT NOT NULL,
                    format TEXT NOT NULL,
                    text TEXT NOT NULL,
                    origin TEXT NOT NULL,
                    fetched INTEGER NOT NULL,
                    PRIMARY KEY (key, format)
                 );
                 PRAGMA user_version = 2;",
            )
            .map_err(err)?;
        }
        if from < 3 {
            // The user's own data, and when read state last changed (for sync).
            tx.execute_batch(
                "ALTER TABLE items ADD COLUMN read_stamp INTEGER NOT NULL DEFAULT 0;
                 CREATE TABLE user_items (
                    key TEXT PRIMARY KEY,
                    saved_at INTEGER,
                    saved_stamp INTEGER NOT NULL DEFAULT 0,
                    note TEXT NOT NULL DEFAULT '',
                    note_stamp INTEGER NOT NULL DEFAULT 0,
                    tags TEXT NOT NULL DEFAULT '[]',
                    tags_stamp INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX user_items_saved ON user_items(saved_at);
                 PRAGMA user_version = 3;",
            )
            .map_err(err)?;
        }
        if from < 4 {
            // Full-text search over everything ever fetched (SQLite's FTS5).
            // `search_ids` gives each key a stable integer for the index's rowid (the items
            // table's own rowid could be renumbered by a VACUUM). Triggers keep the index in
            // step with every write, whichever code path makes it (refresh, backfill, sync).
            tx.execute_batch(&format!(
                "CREATE TABLE search_ids (id INTEGER PRIMARY KEY, key TEXT NOT NULL UNIQUE);
                 CREATE VIRTUAL TABLE search USING fts5(
                    ftitle, fauthors, fabstract, fmine, fmeta,
                    tokenize = 'unicode61 remove_diacritics 2',
                    prefix = '2 3'
                 );
                 CREATE TRIGGER search_item_new AFTER INSERT ON items BEGIN
                    INSERT OR IGNORE INTO search_ids (key) VALUES (new.key);
                    INSERT INTO search (rowid, ftitle, fauthors, fabstract, fmine, fmeta)
                    VALUES ((SELECT id FROM search_ids WHERE key = new.key),
                            new.title, new.authors, new.summary,
                            COALESCE((SELECT {MINE} FROM user_items u WHERE u.key = new.key), ''),
                            {META});
                 END;
                 CREATE TRIGGER search_item_changed AFTER UPDATE ON items
                 WHEN old.title IS NOT new.title OR old.authors IS NOT new.authors
                   OR old.summary IS NOT new.summary OR old.categories IS NOT new.categories
                   OR old.doi IS NOT new.doi OR old.journal_ref IS NOT new.journal_ref
                   OR old.source_name IS NOT new.source_name
                 BEGIN
                    UPDATE search SET ftitle = new.title, fauthors = new.authors,
                        fabstract = new.summary, fmeta = {META}
                    WHERE rowid = (SELECT id FROM search_ids WHERE key = new.key);
                 END;
                 CREATE TRIGGER search_item_gone AFTER DELETE ON items BEGIN
                    DELETE FROM search WHERE rowid = (SELECT id FROM search_ids WHERE key = old.key);
                    DELETE FROM search_ids WHERE key = old.key;
                 END;
                 CREATE TRIGGER search_mine_new AFTER INSERT ON user_items BEGIN
                    UPDATE search SET fmine = {MINE_NEW}
                    WHERE rowid = (SELECT id FROM search_ids WHERE key = new.key);
                 END;
                 CREATE TRIGGER search_mine_changed AFTER UPDATE ON user_items
                 WHEN old.note IS NOT new.note OR old.tags IS NOT new.tags
                 BEGIN
                    UPDATE search SET fmine = {MINE_NEW}
                    WHERE rowid = (SELECT id FROM search_ids WHERE key = new.key);
                 END;
                 INSERT INTO search_ids (key) SELECT key FROM items;
                 INSERT INTO search (rowid, ftitle, fauthors, fabstract, fmine, fmeta)
                    SELECT s.id, new.title, new.authors, new.summary,
                           COALESCE((SELECT {MINE} FROM user_items u WHERE u.key = new.key), ''),
                           {META}
                    FROM items new JOIN search_ids s ON s.key = new.key;
                 PRAGMA user_version = 4;",
                MINE = "u.note || ' ' || u.tags",
                MINE_NEW = "new.note || ' ' || new.tags",
                META = "new.key || ' ' || new.categories || ' ' || new.source_name || ' ' \
                        || COALESCE(new.doi, '') || ' ' || COALESCE(new.journal_ref, '')",
            ))
            .map_err(err)?;
        }
        if from < 5 {
            // Stats: when the user last opened an item's page or PDF. Local only,
            // not synced; counts start with this version. Added only if missing, so a file
            // whose version number was set back by hand still upgrades.
            let has: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('items') WHERE name = 'opened_at'",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !has {
                tx.execute_batch("ALTER TABLE items ADD COLUMN opened_at INTEGER;")
                    .map_err(err)?;
            }
            // Read in bulk ("Mark all read", multi-select "Mark read") rather than looked at:
            // Stats doesn't count these as reading. Local, not synced.
            let has: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('items') WHERE name = 'read_dismissed'",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !has {
                tx.execute_batch(
                    "ALTER TABLE items ADD COLUMN read_dismissed INTEGER NOT NULL DEFAULT 0;",
                )
                .map_err(err)?;
            }
            tx.execute_batch("PRAGMA user_version = 5;").map_err(err)?;
        }
        if from < 6 {
            // "Read later today". When the item went on the queue; the queue is the
            // items queued since local midnight (the UI passes that time), so it empties itself
            // without a timer. Local, not synced.
            let has: bool = tx
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('items') WHERE name = 'later_at'",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if !has {
                tx.execute_batch("ALTER TABLE items ADD COLUMN later_at INTEGER;")
                    .map_err(err)?;
            }
            tx.execute_batch("PRAGMA user_version = 6;").map_err(err)?;
        }
        if from < 7 {
            // LaTeX accents in author names (`M\"ostl`) → Unicode, as feeds are now
            // parsed; the search triggers re-index the changed rows, so `au:Möstl` finds them.
            let rows: Vec<(String, String)> = {
                let mut st = tx
                    .prepare(
                        "SELECT key, authors FROM items
                         WHERE authors LIKE '%\\%' OR authors LIKE '%{%'",
                    )
                    .map_err(err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(err)?;
                rows.collect::<Result<_, _>>().map_err(err)?
            };
            for (key, json) in rows {
                let names: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                let fixed: Vec<String> = names.iter().map(|a| crate::accents::decode(a)).collect();
                if fixed != names {
                    let text = serde_json::to_string(&fixed).map_err(|e| e.to_string())?;
                    tx.execute(
                        "UPDATE items SET authors = ?2 WHERE key = ?1",
                        params![key, text],
                    )
                    .map_err(err)?;
                }
            }
            tx.execute_batch("PRAGMA user_version = 7;").map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    /// The connection, for the read-only aggregate queries in `stats.rs`.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Records that the user opened an item's page or PDF: Stats counts it as read, and it
    /// leaves the "Read later" queue (it has been read).
    pub fn set_opened(&self, key: &str, now: DateTime<Utc>) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE items SET opened_at = ?2, later_at = NULL WHERE key = ?1",
                params![key, now.timestamp()],
            )
            .map(|_| ())
            .map_err(err)
    }

    /// Puts items on the "Read later" queue (`on`) or takes them off.
    pub fn set_later(&self, keys: &[String], on: bool, now: DateTime<Utc>) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(err)?;
        for key in keys {
            tx.execute(
                "UPDATE items SET later_at = ?2 WHERE key = ?1",
                params![key, on.then(|| now.timestamp())],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)
    }

    /// The "Read later" queue of a section: items queued at or after `since` (local midnight),
    /// oldest first, so it reads in the order you queued them.
    pub fn later(&self, section: Section, since: DateTime<Utc>) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             WHERE section = ?1 AND later_at >= ?2
             ORDER BY later_at, items.key"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st
            .query_map(params![section.as_str(), since.timestamp()], row_from)
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// "Don't count": the open was an accident, so the item is only seen again.
    pub fn clear_opened(&self, key: &str) -> Result<(), String> {
        self.conn
            .execute("UPDATE items SET opened_at = NULL WHERE key = ?1", [key])
            .map(|_| ())
            .map_err(err)
    }

    pub fn citation(&self, key: &str, format: &str) -> Option<(String, String)> {
        self.conn
            .query_row(
                "SELECT text, origin FROM citations WHERE key = ?1 AND format = ?2",
                params![key, format],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_citation(
        &self,
        key: &str,
        format: &str,
        text: &str,
        origin: &str,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO citations (key, format, text, origin, fetched) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(key, format) DO UPDATE SET text = ?3, origin = ?4, fetched = ?5",
                params![key, format, text, origin, now.timestamp()],
            )
            .map(|_| ())
            .map_err(err)
    }

    /// Unread news stories (muting marks the matching ones read).
    pub fn unread_news(&self) -> Result<Vec<Item>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {ITEM_COLS} FROM items WHERE section = 'news' AND read_at IS NULL"
            ))
            .map_err(err)?;
        let rows = stmt.query_map([], item_from_row).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Every paper in the archive (to recognise the PDFs Dispatch downloaded).
    pub fn research_items(&self) -> Result<Vec<Item>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {ITEM_COLS} FROM items WHERE section = 'research'"
            ))
            .map_err(err)?;
        let rows = stmt.query_map([], item_from_row).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// One item by key, as the UI sees it.
    pub fn get(&self, key: &str) -> Result<Option<Item>, String> {
        self.conn
            .query_row(
                &format!("SELECT {ITEM_COLS} FROM items WHERE key = ?1"),
                [key],
                item_from_row,
            )
            .optional()
            .map_err(err)
    }

    /// Adds new items and refreshes known ones, in one transaction. Keeps `first_seen` and
    /// read state; a paper's version only goes up. Returns how many were new.
    pub fn upsert(&mut self, items: &[Item], now: DateTime<Utc>) -> Result<usize, String> {
        let tx = self.conn.transaction().map_err(err)?;
        let mut new = 0;
        {
            let mut exists = tx
                .prepare("SELECT 1 FROM items WHERE key = ?1")
                .map_err(err)?;
            let mut up = tx
                .prepare(
                    "INSERT INTO items (key, section, source, source_name, title, authors, summary,
                        link, pdf, categories, published, announce_type, version, doi, journal_ref,
                        first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
                     ON CONFLICT(key) DO UPDATE SET
                        title = excluded.title,
                        authors = excluded.authors,
                        summary = excluded.summary,
                        link = excluded.link,
                        pdf = excluded.pdf,
                        categories = excluded.categories,
                        published = COALESCE(excluded.published, items.published),
                        announce_type = excluded.announce_type,
                        version = MAX(COALESCE(items.version, 0), COALESCE(excluded.version, 0)),
                        doi = COALESCE(excluded.doi, items.doi),
                        journal_ref = COALESCE(excluded.journal_ref, items.journal_ref),
                        last_seen = excluded.last_seen",
                )
                .map_err(err)?;
            for it in items {
                if !exists.exists([&it.key]).map_err(err)? {
                    new += 1;
                }
                up.execute(params![
                    it.key,
                    it.section.as_str(),
                    it.source,
                    it.source_name,
                    it.title,
                    json(&it.authors),
                    it.summary,
                    it.link,
                    it.pdf,
                    json(&it.categories),
                    it.published.map(|d| d.timestamp()),
                    it.announce_type,
                    it.version,
                    it.doi,
                    it.journal_ref,
                    now.timestamp(),
                ])
                .map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(new)
    }

    /// Records one fetch attempt for a source. A failure keeps `last_ok` and `last_count`.
    pub fn record(
        &mut self,
        source: &str,
        name: &str,
        section: Section,
        now: DateTime<Utc>,
        result: &Result<usize, String>,
        build_date: Option<&str>,
    ) -> Result<(), String> {
        let t = now.timestamp();
        let tx = self.conn.transaction().map_err(err)?;
        match result {
            Ok(n) => tx.execute(
                "INSERT INTO source_status (source, name, section, last_try, last_ok, last_error, last_count)
                 VALUES (?1, ?2, ?3, ?4, ?4, NULL, ?5)
                 ON CONFLICT(source) DO UPDATE SET name = ?2, section = ?3, last_try = ?4,
                    last_ok = ?4, last_error = NULL, last_count = ?5",
                params![source, name, section.as_str(), t, *n as i64],
            ),
            Err(e) => tx.execute(
                "INSERT INTO source_status (source, name, section, last_try, last_error)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(source) DO UPDATE SET name = ?2, section = ?3, last_try = ?4,
                    last_error = ?5",
                params![source, name, section.as_str(), t, e],
            ),
        }
        .map_err(err)?;
        tx.execute(
            "INSERT INTO fetch_log (at, source, ok, count, error, build_date) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                t,
                source,
                result.is_ok(),
                result.as_ref().ok().map(|n| *n as i64),
                result.as_ref().err(),
                build_date
            ],
        )
        .map_err(err)?;
        // Keep the log small: the newest 2000 attempts are plenty to check feed timing.
        tx.execute(
            "DELETE FROM fetch_log WHERE id <= (SELECT MAX(id) FROM fetch_log) - 2000",
            [],
        )
        .map_err(err)?;
        tx.commit().map_err(err)
    }

    /// Recomputes which topics every item belongs to (after a refresh or a topic change).
    pub fn retag(&mut self, topics: &Topics) -> Result<(), String> {
        let items = self.all_items()?;
        let tx = self.conn.transaction().map_err(err)?;
        tx.execute("DELETE FROM item_topics", []).map_err(err)?;
        {
            let mut ins = tx
                .prepare("INSERT INTO item_topics (key, topic) VALUES (?1, ?2)")
                .map_err(err)?;
            for it in &items {
                for t in topics.topics_for(it) {
                    ins.execute(params![it.key, t]).map_err(err)?;
                }
            }
        }
        tx.commit().map_err(err)
    }

    fn all_items(&self) -> Result<Vec<Item>, String> {
        let mut st = self
            .conn
            .prepare(&format!("SELECT {ITEM_COLS} FROM items"))
            .map_err(err)?;
        let rows = st.query_map([], item_from_row).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Items in a section, newest first, optionally only one topic's. Items that match no
    /// enabled topic are hidden (a topic switched off hides its papers without deleting them).
    pub fn list(
        &self,
        section: Section,
        topic: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Row>, String> {
        self.list_hiding(section, topic, limit, None, false)
    }

    /// `list`, leaving out read items published before `hide_read_before` unless saved (the
    /// "hide old read papers" setting; Search and Saved still have them), and, with
    /// `hide_replaced`, arXiv's replacements (new versions of older papers).
    pub fn list_hiding(
        &self,
        section: Section,
        topic: Option<&str>,
        limit: usize,
        hide_read_before: Option<DateTime<Utc>>,
        hide_replaced: bool,
    ) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             WHERE section = ?1
               AND EXISTS (SELECT 1 FROM item_topics t WHERE t.key = items.key AND (?2 IS NULL OR t.topic = ?2))
               AND NOT (?4 IS NOT NULL AND read_at IS NOT NULL AND u.saved_at IS NULL
                        AND COALESCE(published, first_seen) < ?4)
               AND NOT (?5 AND {REPLACED})
             ORDER BY COALESCE(published, first_seen) DESC, first_seen DESC, items.key
             LIMIT ?3"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st
            .query_map(
                params![
                    section.as_str(),
                    topic,
                    limit as i64,
                    hide_read_before.map(|t| t.timestamp()),
                    hide_replaced
                ],
                row_from,
            )
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Every paper's author list (Search → People scans these).
    pub fn author_lists(&self) -> Result<Vec<Vec<String>>, String> {
        let mut st = self
            .conn
            .prepare("SELECT authors FROM items WHERE section = 'research'")
            .map_err(err)?;
        let rows = st
            .query_map([], |r| {
                Ok(serde_json::from_str(&r.get::<_, String>(0)?).unwrap_or_default())
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Every item with a note, most recently written first (Search → Notes).
    pub fn list_noted(&self, limit: usize) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items JOIN user_items u ON u.key = items.key
             WHERE u.note != ''
             ORDER BY u.note_stamp DESC, items.key
             LIMIT ?1"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st.query_map([limit as i64], row_from).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// The archive limit (optional setting): removes items published before `before` that the
    /// user hasn't saved, noted or tagged, with their topics and cached citations. Returns
    /// how many went. The search index follows by its triggers.
    pub fn prune(&mut self, before: DateTime<Utc>) -> Result<usize, String> {
        let tx = self.conn.transaction().map_err(err)?;
        let doomed = "SELECT i.key FROM items i LEFT JOIN user_items u ON u.key = i.key
             WHERE COALESCE(i.published, i.first_seen) < ?1
               AND (u.key IS NULL OR (u.saved_at IS NULL AND u.note = '' AND u.tags = '[]'))";
        tx.execute(
            &format!("DELETE FROM citations WHERE key IN ({doomed})"),
            [before.timestamp()],
        )
        .map_err(err)?;
        let n = tx
            .execute(
                &format!("DELETE FROM items WHERE key IN ({doomed})"),
                [before.timestamp()],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(n)
    }

    /// Gives the space of removed items back to macOS (rewrites the file; seconds for a big one).
    /// A consistent copy of the whole database in one new file (`VACUUM INTO`), safe while
    /// other connections write. Used by backups.
    pub fn copy_to(&self, path: &Path) -> Result<(), String> {
        self.conn
            .execute("VACUUM INTO ?1", [path.to_string_lossy()])
            .map(|_| ())
            .map_err(err)
    }

    pub fn compact(&self) -> Result<(), String> {
        self.conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
            .map_err(err)
    }

    /// The oldest item's date, for Settings' storage line.
    pub fn oldest(&self) -> Option<DateTime<Utc>> {
        self.conn
            .query_row(
                "SELECT MIN(COALESCE(published, first_seen)) FROM items",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .map(ts)
    }

    /// Saved items from both sections, newest save first, optionally only one tag's. Saved
    /// items show even if their topic is switched off.
    /// Items whose note changed between two times (ms), oldest change first (the reading
    /// log).
    pub fn notes_between(&self, from_ms: i64, to_ms: i64) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items JOIN user_items u ON u.key = items.key
             WHERE u.note != '' AND u.note_stamp >= ?1 AND u.note_stamp < ?2
             ORDER BY u.note_stamp, items.key"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st.query_map([from_ms, to_ms], row_from).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Every item with the user's data, newest first.
    pub fn all_rows(&self) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             ORDER BY section DESC, COALESCE(published, first_seen) DESC, items.key"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st.query_map([], row_from).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// The research paper whose page or PDF was opened last.
    pub fn last_opened(&self) -> Result<Option<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             WHERE section = 'research' AND opened_at IS NOT NULL
             ORDER BY opened_at DESC LIMIT 1"
        );
        self.conn
            .query_row(&sql, [], row_from)
            .optional()
            .map_err(err)
    }

    /// Per source: (source, name, section, items, newest first_seen), for the data health
    /// page. Sources no longer followed are included (their items stay).
    pub fn items_by_source(&self) -> Result<Vec<SourceItems>, String> {
        let mut st = self
            .conn
            .prepare(
                "SELECT source, MAX(source_name), section, COUNT(*), MAX(first_seen)
                 FROM items GROUP BY source, section ORDER BY section DESC, MAX(source_name)",
            )
            .map_err(err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<i64>>(4)?.map(ts),
                ))
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// SQLite's own check of the file: "ok", or what it found (first ten problems).
    pub fn integrity(&self) -> Result<String, String> {
        let mut st = self
            .conn
            .prepare("PRAGMA integrity_check(10)")
            .map_err(err)?;
        let lines: Vec<String> = st
            .query_map([], |r| r.get(0))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        Ok(lines.join("; "))
    }

    /// Notes, saves or tags whose item isn't in the database (possible after a sync from a
    /// Mac that had papers this one never fetched). Their keys.
    pub fn orphaned_user_rows(&self) -> Result<Vec<String>, String> {
        let mut st = self
            .conn
            .prepare(
                "SELECT u.key FROM user_items u LEFT JOIN items i ON i.key = u.key
                 WHERE i.key IS NULL
                   AND (u.saved_at IS NOT NULL OR u.note != '' OR u.tags != '[]')
                 ORDER BY u.key",
            )
            .map_err(err)?;
        let rows = st.query_map([], |r| r.get(0)).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Full rows for these keys, in the order given (missing keys left out).
    pub fn rows(&self, keys: &[String]) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             WHERE items.key IN (SELECT value FROM json_each(?1))"
        );
        let json = serde_json::to_string(keys).map_err(|e| e.to_string())?;
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let mut rows: Vec<Row> = st
            .query_map([json], row_from)
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?;
        rows.sort_by_key(|r| keys.iter().position(|k| *k == r.item.key));
        Ok(rows)
    }

    /// Research papers first seen in `[from, to)` (Unix seconds) whose page or PDF was never
    /// opened (seen in the preview or not), new submissions and cross-lists only, newest first
    /// (Suggested and "what you missed": "not opened" rather than "unread", since nearly
    /// everything gets seen).
    pub fn unopened_research_between(&self, from: i64, to: i64) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items LEFT JOIN user_items u ON u.key = items.key
             WHERE section = 'research' AND opened_at IS NULL
               AND first_seen >= ?1 AND first_seen < ?2
               AND NOT {REPLACED}
             ORDER BY COALESCE(published, first_seen) DESC, items.key
             LIMIT 2000"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st.query_map([from, to], row_from).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn list_saved(&self, tag: Option<&str>, limit: usize) -> Result<Vec<Row>, String> {
        let sql = format!(
            "SELECT {ROW_COLS}
             FROM items JOIN user_items u ON u.key = items.key
             WHERE u.saved_at IS NOT NULL
               AND (?1 IS NULL OR EXISTS (SELECT 1 FROM json_each(u.tags) WHERE value = ?1))
             ORDER BY u.saved_at DESC, items.key
             LIMIT ?2"
        );
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st
            .query_map(params![tag, limit as i64], row_from)
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Full-text search over everything ever fetched, including items whose topic
    /// is switched off. `query` is what the user typed (see `search.rs`). The snippet is a
    /// piece of the abstract with matches between `search::MARK_START` and `MARK_END`.
    pub fn search(
        &self,
        query: &str,
        filter: &SearchFilter,
        limit: usize,
    ) -> Result<SearchResult, String> {
        let Some(fts) = search::fts_query(query) else {
            return Ok(SearchResult::default());
        };
        let section = filter.section.map(Section::as_str);
        let from = "FROM search
             JOIN search_ids s ON s.id = search.rowid
             JOIN items ON items.key = s.key
             LEFT JOIN user_items u ON u.key = items.key
             WHERE search MATCH ?1
               AND (?2 IS NULL OR items.section = ?2)
               AND (?3 = 0 OR u.saved_at IS NOT NULL)
               AND (?5 IS NULL OR EXISTS (SELECT 1 FROM item_topics t WHERE t.key = items.key AND t.topic = ?5))
               AND (?6 = 0 OR items.read_at IS NULL)
               AND (?7 IS NULL OR COALESCE(published, first_seen) >= ?7)";
        let weights = search::WEIGHTS.map(|w| w.to_string()).join(", ");
        let order = match filter.sort {
            search::Sort::Best => {
                format!("bm25(search, {weights}), COALESCE(published, first_seen) DESC, items.key")
            }
            search::Sort::Newest => "COALESCE(published, first_seen) DESC, items.key".to_owned(),
        };
        let sql = format!(
            "SELECT {ROW_COLS}, snippet(search, 2, char(2), char(3), '…', 24)
             {from} ORDER BY {order} LIMIT ?4"
        );
        // One parameter list for both statements; the count names ?4 (the limit) only so that
        // every number is used.
        let args = params![
            fts,
            section,
            filter.saved_only,
            limit as i64,
            filter.topic,
            filter.unread_only,
            filter.since
        ];
        let mut st = self.conn.prepare(&sql).map_err(err)?;
        let rows = st
            .query_map(args, |r| {
                Ok(Hit {
                    row: row_from(r)?,
                    snippet: r.get(23)?,
                })
            })
            .map_err(err)?;
        let hits = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
        let total = self
            .conn
            .query_row(
                &format!("SELECT COUNT(*) {from} AND ?4 IS NOT NULL"),
                args,
                |r| r.get(0),
            )
            .map_err(err)?;
        Ok(SearchResult { hits, total })
    }

    /// Items of a section first stored at or after `since` (a refresh's own time), for alerts.
    pub fn new_since(&self, section: Section, since: DateTime<Utc>) -> Result<Vec<Item>, String> {
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT {ITEM_COLS} FROM items WHERE section = ?1 AND first_seen >= ?2
                 ORDER BY COALESCE(published, first_seen) DESC"
            ))
            .map_err(err)?;
        let rows = st
            .query_map(params![section.as_str(), since.timestamp()], item_from_row)
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// How many items the search index holds (equals the item count when it is in step).
    pub fn indexed(&self) -> Result<i64, String> {
        self.conn
            .query_row("SELECT COUNT(*) FROM search", [], |r| r.get(0))
            .map_err(err)
    }

    /// Every tag in use, with how many items carry it, most used first.
    pub fn tags(&self) -> Result<Vec<(String, i64)>, String> {
        let mut st = self
            .conn
            .prepare(
                "SELECT j.value, COUNT(*) FROM user_items u, json_each(u.tags) j
                 GROUP BY j.value ORDER BY COUNT(*) DESC, j.value",
            )
            .map_err(err)?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn user(&self, key: &str) -> Result<Option<UserData>, String> {
        self.conn
            .query_row(
                &format!("SELECT {USER_COLS} FROM user_items WHERE key = ?1"),
                [key],
                |r| user_from(r, 0),
            )
            .optional()
            .map_err(err)
    }

    /// Writes one item's user data as given (stamp included). Rows with nothing left in them
    /// are kept, so a "removed" state still syncs over an older "saved".
    pub fn put_user(&self, key: &str, u: &UserData) -> Result<(), String> {
        put_user(&self.conn, key, u)
    }

    /// Applies `f` to each item's user data and stamps every field it changed.
    fn edit_user(
        &mut self,
        keys: &[String],
        now: DateTime<Utc>,
        f: impl Fn(&mut UserData),
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        for k in keys {
            let before: UserData = tx
                .query_row(
                    &format!("SELECT {USER_COLS} FROM user_items WHERE key = ?1"),
                    [k],
                    |r| user_from(r, 0),
                )
                .optional()
                .map_err(err)?
                .unwrap_or_default();
            let mut u = before.clone();
            f(&mut u);
            let ms = now.timestamp_millis();
            if u.saved_at != before.saved_at {
                u.saved_stamp = ms.max(before.saved_stamp + 1);
            }
            if u.note != before.note {
                u.note_stamp = ms.max(before.note_stamp + 1);
            }
            if u.tags != before.tags {
                u.tags_stamp = ms.max(before.tags_stamp + 1);
            }
            if u != before {
                put_user(&tx, k, &u)?;
            }
        }
        tx.commit().map_err(err)
    }

    /// Saves or unsaves items ("save for later").
    pub fn set_saved(
        &mut self,
        keys: &[String],
        saved: bool,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        self.edit_user(keys, now, |u| {
            u.saved_at = match (saved, u.saved_at) {
                (true, Some(t)) => Some(t),
                (true, None) => Some(now.timestamp()),
                (false, _) => None,
            }
        })
    }

    /// Writing a note also saves the item, so every noted paper can be
    /// found in Saved. Removing it from Saved later keeps the note.
    pub fn set_note(&mut self, key: &str, note: &str, now: DateTime<Utc>) -> Result<(), String> {
        let note = note.trim_end().to_owned();
        self.edit_user(&[key.to_owned()], now, |u| {
            if !note.is_empty() && u.note.is_empty() && u.saved_at.is_none() {
                u.saved_at = Some(now.timestamp());
            }
            u.note = note.clone();
        })
    }

    /// Replaces an item's tags (trimmed, lower-case, no duplicates, in the order given).
    pub fn set_tags(
        &mut self,
        key: &str,
        tags: &[String],
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let clean = clean_tags(tags);
        self.edit_user(&[key.to_owned()], now, |u| u.tags = clean.clone())
    }

    /// Adds one tag to several items.
    pub fn add_tag(
        &mut self,
        keys: &[String],
        tag: &str,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let tag = clean_tags(&[tag.to_owned()]);
        let Some(tag) = tag.first().cloned() else {
            return Ok(());
        };
        self.edit_user(keys, now, |u| {
            if !u.tags.contains(&tag) {
                u.tags.push(tag.clone());
            }
        })
    }

    /// All user rows (for sync).
    pub fn user_rows(&self) -> Result<Vec<(String, UserData)>, String> {
        let mut st = self
            .conn
            .prepare(&format!("SELECT key, {USER_COLS} FROM user_items"))
            .map_err(err)?;
        let rows = st
            .query_map([], |r| Ok((r.get(0)?, user_from(r, 1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Read-state changes made at or after `since_ms` (key, read_at, read_stamp), for sync.
    pub fn read_rows(&self, since_ms: i64) -> Result<Vec<(String, Option<i64>, i64)>, String> {
        let mut st = self
            .conn
            .prepare("SELECT key, read_at, read_stamp FROM items WHERE read_stamp > 0 AND read_stamp >= ?1")
            .map_err(err)?;
        let rows = st
            .query_map([since_ms], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Applies another Mac's read state if it is newer than ours. Returns whether it changed.
    pub fn apply_read(&self, key: &str, read_at: Option<i64>, stamp: i64) -> Result<bool, String> {
        self.conn
            .execute(
                "UPDATE items SET read_at = ?2, read_stamp = ?3 WHERE key = ?1 AND read_stamp < ?3",
                params![key, read_at, stamp],
            )
            .map(|n| n > 0)
            .map_err(err)
    }

    /// Adds an item another Mac has (a saved paper this Mac never fetched); keeps ours if we
    /// have it.
    pub fn insert_if_missing(&mut self, item: &Item, now: DateTime<Utc>) -> Result<bool, String> {
        if self.get(&item.key)?.is_some() {
            return Ok(false);
        }
        self.upsert(std::slice::from_ref(item), now).map(|n| n > 0)
    }

    /// Marks one item read or unread.
    pub fn set_read(&self, key: &str, read: bool, now: DateTime<Utc>) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE items SET read_at = ?2, read_stamp = ?3, read_dismissed = 0 WHERE key = ?1",
                params![key, read.then_some(now.timestamp()), now.timestamp_millis()],
            )
            .map(|_| ())
            .map_err(err)
    }

    /// Marks several items read or unread in one transaction. Marked read this way they count
    /// as dismissed, not read, in Stats.
    pub fn set_read_many(
        &mut self,
        keys: &[String],
        read: bool,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(err)?;
        {
            let mut st = tx
                .prepare(
                    "UPDATE items SET read_at = ?2, read_stamp = ?3, read_dismissed = ?4 WHERE key = ?1",
                )
                .map_err(err)?;
            for k in keys {
                st.execute(params![
                    k,
                    read.then_some(now.timestamp()),
                    now.timestamp_millis(),
                    read
                ])
                .map_err(err)?;
            }
        }
        tx.commit().map_err(err)
    }

    /// Marks every unread item of a section (optionally one topic) read. Returns the keys it
    /// changed, so the window can offer Undo (`set_read_many(keys, false)`).
    pub fn mark_all_read(
        &mut self,
        section: Section,
        topic: Option<&str>,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, String> {
        let keys: Vec<String> = {
            let mut st = self
                .conn
                .prepare(
                    "SELECT key FROM items
                     WHERE section = ?1 AND read_at IS NULL
                       AND EXISTS (SELECT 1 FROM item_topics t WHERE t.key = items.key
                                   AND (?2 IS NULL OR t.topic = ?2))",
                )
                .map_err(err)?;
            let rows = st
                .query_map(params![section.as_str(), topic], |r| r.get(0))
                .map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        self.set_read_many(&keys, true, now)?;
        Ok(keys)
    }

    /// Unread items per topic in a section, plus `""` for the section total (each item once,
    /// even when it is in several topics).
    pub fn unread_counts(&self, section: Section) -> Result<BTreeMap<String, i64>, String> {
        self.unread_counts_hiding(section, false)
    }

    /// `unread_counts`, leaving out arXiv's replacements when the list hides them, so the
    /// chips' numbers match the list.
    pub fn unread_counts_hiding(
        &self,
        section: Section,
        hide_replaced: bool,
    ) -> Result<BTreeMap<String, i64>, String> {
        let mut out = BTreeMap::new();
        let mut st = self
            .conn
            .prepare(&format!(
                "SELECT t.topic, COUNT(*) FROM items i JOIN item_topics t ON t.key = i.key
                 WHERE i.section = ?1 AND i.read_at IS NULL AND NOT (?2 AND {REPLACED})
                 GROUP BY t.topic"
            ))
            .map_err(err)?;
        let rows = st
            .query_map(params![section.as_str(), hide_replaced], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(err)?;
        for row in rows {
            let (t, n) = row.map_err(err)?;
            out.insert(t, n);
        }
        let total: i64 = self
            .conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM items i WHERE i.section = ?1 AND i.read_at IS NULL
                 AND EXISTS (SELECT 1 FROM item_topics t WHERE t.key = i.key)
                 AND NOT (?2 AND {REPLACED})"
                ),
                params![section.as_str(), hide_replaced],
                |r| r.get(0),
            )
            .map_err(err)?;
        out.insert(String::new(), total);
        Ok(out)
    }

    pub fn count(&self, section: Section) -> Result<i64, String> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE section = ?1",
                [section.as_str()],
                |r| r.get(0),
            )
            .map_err(err)
    }

    /// Removes the status rows of every source not in `keep`.
    pub fn keep_statuses(&self, keep: &[String]) -> Result<(), String> {
        let gone: Vec<String> = self
            .statuses()?
            .into_iter()
            .map(|s| s.source)
            .filter(|s| !keep.contains(s))
            .collect();
        for s in gone {
            self.conn
                .execute("DELETE FROM source_status WHERE source = ?1", [&s])
                .map_err(err)?;
        }
        Ok(())
    }

    pub fn statuses(&self) -> Result<Vec<SourceStatus>, String> {
        let mut st = self
            .conn
            .prepare(
                "SELECT source, name, section, last_try, last_ok, last_error, last_count
                 FROM source_status ORDER BY section DESC, name",
            )
            .map_err(err)?;
        let rows = st
            .query_map([], |r| {
                Ok(SourceStatus {
                    source: r.get(0)?,
                    name: r.get(1)?,
                    section: r.get(2)?,
                    last_try: r.get::<_, Option<i64>>(3)?.map(ts),
                    last_ok: r.get::<_, Option<i64>>(4)?.map(ts),
                    last_error: r.get(5)?,
                    last_count: r.get(6)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(err)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
            .map_err(err)
    }

    pub fn time_meta(&self, key: &str) -> Option<DateTime<Utc>> {
        self.meta(key)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .map(ts)
    }

    pub fn set_time_meta(&self, key: &str, t: DateTime<Utc>) -> Result<(), String> {
        self.set_meta(key, &t.timestamp().to_string())
    }
}

/// An arXiv replacement (`replace` or `replace-cross`): a new version of an older paper.
/// COALESCE because news and backfilled papers have no announce type, and `NOT (1 AND NULL)`
/// would hide them too.
const REPLACED: &str = "COALESCE(announce_type, '') LIKE 'replace%'";

/// Item columns, then first_seen (15), read (16), topics (17), saved_at (18), note (19), tags (20),
/// opened (21: since the Stats reset, `stats::SINCE_KEY`), later_at (22). Queries that add
/// columns start at 23.
const ROW_COLS: &str = "items.key, section, source, source_name, title, authors, summary, link, pdf,
    categories, published, announce_type, version, doi, journal_ref, first_seen, read_at IS NOT NULL,
    (SELECT group_concat(topic, ',') FROM item_topics t WHERE t.key = items.key),
    u.saved_at, COALESCE(u.note, ''), COALESCE(u.tags, '[]'),
    IFNULL(items.opened_at, 0) >= COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'stats_since'), 1),
    items.later_at";

fn row_from(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    let saved_at: Option<i64> = r.get(18)?;
    let mut row = Row {
        item: item_from_row(r)?,
        first_seen: ts(r.get(15)?),
        read: r.get(16)?,
        opened: r.get(21)?,
        later_at: r.get::<_, Option<i64>>(22)?.map(ts),
        topics: r
            .get::<_, Option<String>>(17)?
            .map(|s| s.split(',').map(str::to_owned).collect())
            .unwrap_or_default(),
        saved: saved_at.is_some(),
        saved_at: saved_at.map(ts),
        note: r.get(19)?,
        tags: serde_json::from_str(&r.get::<_, String>(20)?).unwrap_or_default(),
        followed: Vec::new(),
        score: None,
        why: Vec::new(),
        code: Vec::new(),
        hints: Vec::new(),
        discussion: None,
        also: Vec::new(),
    };
    crate::hints::annotate(std::slice::from_mut(&mut row));
    crate::news::annotate(std::slice::from_mut(&mut row));
    Ok(row)
}

fn put_user(conn: &Connection, key: &str, u: &UserData) -> Result<(), String> {
    conn.execute(
        &format!(
            "INSERT INTO user_items (key, {USER_COLS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(key) DO UPDATE SET saved_at = ?2, saved_stamp = ?3, note = ?4,
                note_stamp = ?5, tags = ?6, tags_stamp = ?7"
        ),
        params![
            key,
            u.saved_at,
            u.saved_stamp,
            u.note,
            u.note_stamp,
            json(&u.tags),
            u.tags_stamp
        ],
    )
    .map(|_| ())
    .map_err(err)
}

/// Trimmed, lower-case, without empties or duplicates.
pub fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().trim_start_matches('#').to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

const ITEM_COLS: &str = "key, section, source, source_name, title, authors, summary, link, pdf,
    categories, published, announce_type, version, doi, journal_ref";

fn item_from_row(r: &rusqlite::Row) -> rusqlite::Result<Item> {
    let section: String = r.get(1)?;
    Ok(Item {
        key: r.get(0)?,
        section: Section::parse(&section).unwrap_or(Section::News),
        source: r.get(2)?,
        source_name: r.get(3)?,
        title: r.get(4)?,
        authors: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        summary: r.get(6)?,
        link: r.get(7)?,
        pdf: r.get(8)?,
        categories: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
        published: r.get::<_, Option<i64>>(10)?.map(ts),
        announce_type: r.get(11)?,
        version: r.get(12)?,
        doi: r.get(13)?,
        journal_ref: r.get(14)?,
    })
}

fn ts(t: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(t, 0).unwrap_or_default()
}

fn json(v: &[String]) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "[]".into())
}

fn err(e: rusqlite::Error) -> String {
    format!("database: {e}")
}
