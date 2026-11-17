//! Stats: how the user reads, computed locally from the store for the Stats tab,
//! the weekly goal in the menu-bar menu and the Habit Tracker's `reading-log.json`.
//!
//! Everything is a SQL aggregate (GROUP BY a bucket number), so it stays quick with ~100k
//! items. Times are grouped by the user's local days: callers pass "now" with its UTC offset
//! (`Local::now().fixed_offset()`), and that one offset is used for the whole range (fine where
//! there is no daylight saving; elsewhere a daylight-saving change shifts old buckets by an hour).
//!
//! What the numbers mean:
//! - "read" is an item whose page or PDF the user opened (`opened_at`); the preview's
//!   "Don't count" clears it. "Seen" is one shown in the preview (`read_at` without
//!   `read_dismissed`): "Mark all read" and multi-select "Mark read" are neither. The backlog
//!   still treats every read-marked item (seen or dismissed) as no longer waiting.
//! - Settings → "Reset stats" stores [`SINCE_KEY`]: nothing before it is counted (the backlog,
//!   a state rather than an activity, is the exception). Nothing is deleted.
//! - Topics and feeds are judged by the items that *arrived* in the range (`first_seen`): of
//!   those, how many were read, saved or opened.
//! - The backlog on a day is what had arrived by its end and wasn't read by then, counted only
//!   for items in an enabled topic (as the lists show them).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Utc};
use rusqlite::{ToSql, params};
use serde::{Deserialize, Serialize};

use crate::follow::fold;
use crate::paths::write_atomic;
use crate::settings::Settings;
use crate::store::Store;
use crate::topics::Topics;

/// The Habit Tracker link's file, in the data folder.
pub const READING_LOG_FILE: &str = "reading-log.json";
/// Days of history in `reading-log.json`.
pub const READING_LOG_DAYS: i64 = 400;
/// A feed is "noisy" with at least this many items in the range...
pub const NOISY_MIN_ITEMS: i64 = 30;
/// ...a topic is "skimmed" with at least this many papers...
pub const SKIMMED_MIN_ITEMS: i64 = 100;
/// ...when fewer than 1 in this many were read (or opened): under 5 %.
pub const LOW_RATE_ONE_IN: i64 = 20;
/// The backlog "grew" when it rose by at least this many over the range.
pub const GREW_BY: i64 = 50;
/// People shown.
pub const TOP_PEOPLE: usize = 10;
pub const TOP_TAGS: usize = 10;

const DAY: i64 = 86_400;

/// `meta` key: Unix time from which Stats counts (Settings → Reset stats).
pub const SINCE_KEY: &str = "stats_since";

/// When Stats starts counting (0 = from the beginning).
pub fn since(store: &Store) -> i64 {
    store
        .meta(SINCE_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Settings → Reset stats: count from `now` on.
pub fn reset(store: &Store, now: DateTime<Utc>) -> Result<(), String> {
    set_since(store, now.timestamp())
}

/// Sets the reset time directly (Undo puts the old one back; 0 = from the beginning).
pub fn set_since(store: &Store, t: i64) -> Result<(), String> {
    store.set_meta(SINCE_KEY, &t.to_string())
}

/// The range switch on the tab: 4 weeks by day, 3 months or a year by week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Range {
    #[default]
    #[serde(rename = "4w")]
    Weeks4,
    #[serde(rename = "3m")]
    Months3,
    #[serde(rename = "1y")]
    Year,
}

impl Range {
    pub fn parse(s: &str) -> Option<Range> {
        match s {
            "4w" => Some(Range::Weeks4),
            "3m" => Some(Range::Months3),
            "1y" => Some(Range::Year),
            _ => None,
        }
    }

    /// (number of buckets, days per bucket)
    fn shape(self) -> (i64, i64) {
        match self {
            Range::Weeks4 => (28, 1),
            Range::Months3 => (13, 7),
            Range::Year => (52, 7),
        }
    }

    /// For sentences: "in 4 weeks".
    fn words(self) -> &'static str {
        match self {
            Range::Weeks4 => "4 weeks",
            Range::Months3 => "3 months",
            Range::Year => "a year",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    pub range: Range,
    /// "day" or "week": what one bucket is.
    pub bucket: &'static str,
    /// The first bucket's first day and today (local dates).
    pub start: NaiveDate,
    pub today: NaiveDate,
    pub reading: Reading,
    pub goal: Goal,
    pub topics: Vec<Worth>,
    pub categories: Vec<Worth>,
    pub feeds: Vec<FeedWorth>,
    pub people: People,
    pub backlog: Vec<Backlog>,
    pub suggestions: Vec<Suggestion>,
    pub time_to_read: TimeToRead,
    /// The local day of the last "Reset stats" (None = counting from the beginning).
    pub since: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Counts {
    /// Papers read (page or PDF opened).
    pub read: i64,
    /// News items read (article opened).
    pub news_read: i64,
    /// Items seen in the preview (either section), read or not.
    pub seen: i64,
    /// Items saved (either section).
    pub saved: i64,
    /// Items whose note was last written then (only the latest edit is known).
    pub noted: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bucket {
    /// The bucket's first day (a Monday for weeks).
    pub date: NaiveDate,
    #[serde(flatten)]
    pub counts: Counts,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reading {
    pub buckets: Vec<Bucket>,
    pub totals: Counts,
    /// Days in a row with at least one item read (opened), ending today or yesterday (0 = none).
    pub streak: i64,
    /// Whether today already counts in the streak.
    pub streak_today: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Goal {
    /// Papers per week; 0 = no goal (the tab then shows nothing).
    pub target: u32,
    /// Papers read since Monday.
    pub done: i64,
    pub week_start: NaiveDate,
}

/// A research topic or arXiv category, judged by what arrived in the range.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Worth {
    pub id: String,
    pub name: String,
    pub fetched: i64,
    /// Opened.
    pub read: i64,
    pub seen: i64,
    pub saved: i64,
    /// read / fetched, 0–1 (0 when nothing arrived).
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FeedWorth {
    pub source: String,
    pub name: String,
    pub fetched: i64,
    pub read: i64,
    pub seen: i64,
    pub rate: f64,
    /// Many items, very few seen or read.
    pub noisy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    pub name: String,
    /// Papers read (opened) or saved in the range.
    pub count: i64,
    pub followed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct People {
    pub authors: Vec<Person>,
    /// Most-used tags overall, with how many items carry each.
    pub tags: Vec<(String, i64)>,
}

/// Unread items waiting at the end of a bucket (the last one: now).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Backlog {
    pub date: NaiveDate,
    pub research: i64,
    pub news: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    pub text: String,
    /// What its button does: `"topics"` opens the Topics tab; None = no button.
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Median {
    pub id: String,
    pub name: String,
    /// Median days from announcement to read, one decimal.
    pub median_days: f64,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimeToRead {
    /// Papers read in the range, from announcement (`published`, else `first_seen`) to opened.
    pub median_days: Option<f64>,
    pub count: usize,
    /// Share read within a day of announcement, 0–1.
    pub within_day: f64,
    pub topics: Vec<Median>,
}

/// The buckets of a range: equal widths from local midnight of `first`. `from` is the
/// earliest time counted: `start`, or the reset time if that is later.
struct Plan {
    first: NaiveDate,
    n: i64,
    days: i64,
    start: i64,
    width: i64,
    end: i64,
    from: i64,
}

impl Plan {
    fn new(range: Range, now: DateTime<FixedOffset>, since: i64) -> Plan {
        let (n, days) = range.shape();
        let today = now.date_naive();
        let last = if days == 1 { today } else { monday(today) };
        let first = last - Duration::days(days * (n - 1));
        let start = midnight(first, now.offset());
        let width = days * DAY;
        Plan {
            first,
            n,
            days,
            start,
            width,
            end: start + n * width,
            from: start.max(since),
        }
    }

    fn date(&self, i: i64) -> NaiveDate {
        self.first + Duration::days(i * self.days)
    }
}

fn monday(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// A local date's midnight as a Unix time.
fn midnight(d: NaiveDate, offset: &FixedOffset) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc()
        .timestamp()
        - offset.local_minus_utc() as i64
}

/// Local day number (days since 1970-01-01 in local time) of a Unix time.
fn day_number(t: i64, offset: &FixedOffset) -> i64 {
    (t + offset.local_minus_utc() as i64).div_euclid(DAY)
}

fn day_date(n: i64) -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).unwrap_or_default() + Duration::days(n)
}

fn err(e: rusqlite::Error) -> String {
    format!("database: {e}")
}

/// Runs a `SELECT bucket, count … GROUP BY 1` and spreads it over `n` buckets.
fn bucketed(store: &Store, sql: &str, args: &[&dyn ToSql], n: i64) -> Result<Vec<i64>, String> {
    let mut out = vec![0; n as usize];
    let mut st = store.conn().prepare_cached(sql).map_err(err)?;
    let rows = st
        .query_map(args, |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(err)?;
    for row in rows {
        let (b, c) = row.map_err(err)?;
        if (0..n).contains(&b) {
            out[b as usize] += c;
        }
    }
    Ok(out)
}

fn one(store: &Store, sql: &str, args: &[&dyn ToSql]) -> Result<i64, String> {
    store.conn().query_row(sql, args, |r| r.get(0)).map_err(err)
}

const IN_TOPIC: &str = "EXISTS (SELECT 1 FROM item_topics t WHERE t.key = i.key)";

/// Papers read since Monday (local), for the weekly goal.
pub fn week_progress(
    store: &Store,
    now: DateTime<FixedOffset>,
) -> Result<(NaiveDate, i64), String> {
    let week_start = monday(now.date_naive());
    let from = midnight(week_start, now.offset()).max(since(store));
    let done = one(
        store,
        "SELECT COUNT(*) FROM items WHERE section = 'research' AND opened_at >= ?1",
        &[&from],
    )?;
    Ok((week_start, done))
}

/// Papers read per local day since `days` days ago (days with none left out).
pub fn papers_per_day(
    store: &Store,
    now: DateTime<FixedOffset>,
    days: i64,
) -> Result<BTreeMap<NaiveDate, i64>, String> {
    let off = now.offset().local_minus_utc() as i64;
    let from =
        midnight(now.date_naive() - Duration::days(days - 1), now.offset()).max(since(store));
    let mut st = store
        .conn()
        .prepare_cached(
            "SELECT (opened_at + ?1) / 86400, COUNT(*) FROM items
             WHERE section = 'research' AND opened_at >= ?2 GROUP BY 1",
        )
        .map_err(err)?;
    let rows = st
        .query_map(params![off, from], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (d, n) = row.map_err(err)?;
        out.insert(day_date(d), n);
    }
    Ok(out)
}

/// `reading-log.json` for the Habit Tracker. Version 1 had `days` only (papers read per local
/// day) and stays as it was, so an older reader keeps working; version 2 adds
/// `counts`, one entry per local day with anything in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingLog {
    pub version: u32,
    pub updated: String,
    pub days: BTreeMap<String, i64>,
    #[serde(default)]
    pub counts: BTreeMap<String, DayCounts>,
}

/// One local day, papers only (News isn't counted).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayCounts {
    /// Page or PDF opened (Stats' "read"; the same number as `days`).
    pub read: i64,
    /// Shown in the preview (Stats' "seen"), not swept away by Mark all read.
    pub seen: i64,
    /// Saved that day.
    pub saved: i64,
    /// With a note last edited that day.
    pub noted: i64,
}

/// Unix times → counts per local day, from `from` on.
fn per_day(
    store: &Store,
    sql: &str,
    off: i64,
    from: i64,
) -> Result<BTreeMap<NaiveDate, i64>, String> {
    let mut st = store.conn().prepare_cached(sql).map_err(err)?;
    let rows = st
        .query_map(params![off, from], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?;
    let mut out = BTreeMap::new();
    for row in rows {
        let (d, n) = row.map_err(err)?;
        out.insert(day_date(d), n);
    }
    Ok(out)
}

/// Read, seen, saved and noted papers per local day since `days` days ago.
pub fn counts_per_day(
    store: &Store,
    now: DateTime<FixedOffset>,
    days: i64,
) -> Result<BTreeMap<NaiveDate, DayCounts>, String> {
    let off = now.offset().local_minus_utc() as i64;
    let from =
        midnight(now.date_naive() - Duration::days(days - 1), now.offset()).max(since(store));
    let mut out: BTreeMap<NaiveDate, DayCounts> = BTreeMap::new();
    for (d, n) in papers_per_day(store, now, days)? {
        out.entry(d).or_default().read = n;
    }
    let seen = per_day(
        store,
        "SELECT (read_at + ?1) / 86400, COUNT(*) FROM items
         WHERE section = 'research' AND read_at >= ?2 AND read_dismissed = 0 GROUP BY 1",
        off,
        from,
    )?;
    for (d, n) in seen {
        out.entry(d).or_default().seen = n;
    }
    let saved = per_day(
        store,
        "SELECT (u.saved_at + ?1) / 86400, COUNT(*) FROM user_items u
         JOIN items i ON i.key = u.key
         WHERE i.section = 'research' AND u.saved_at >= ?2 GROUP BY 1",
        off,
        from,
    )?;
    for (d, n) in saved {
        out.entry(d).or_default().saved = n;
    }
    let noted = per_day(
        store,
        "SELECT (u.note_stamp / 1000 + ?1) / 86400, COUNT(*) FROM user_items u
         JOIN items i ON i.key = u.key
         WHERE i.section = 'research' AND u.note != '' AND u.note_stamp / 1000 >= ?2 GROUP BY 1",
        off,
        from,
    )?;
    for (d, n) in noted {
        out.entry(d).or_default().noted = n;
    }
    Ok(out)
}

impl ReadingLog {
    pub fn build(store: &Store, now: DateTime<FixedOffset>) -> Result<ReadingLog, String> {
        let counts = counts_per_day(store, now, READING_LOG_DAYS)?;
        let days = counts
            .iter()
            .filter(|(_, c)| c.read > 0)
            .map(|(d, c)| (d.format("%Y-%m-%d").to_string(), c.read))
            .collect();
        Ok(ReadingLog {
            version: 2,
            updated: now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            days,
            counts: counts
                .into_iter()
                .map(|(d, c)| (d.format("%Y-%m-%d").to_string(), c))
                .collect(),
        })
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let mut json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        json.push('\n');
        write_atomic(path, json.as_bytes())
    }
}

/// Days in a row with something read, ending today or yesterday; and whether today counts.
fn streak(store: &Store, now: DateTime<FixedOffset>) -> Result<(i64, bool), String> {
    let off = now.offset().local_minus_utc() as i64;
    let from = (now.timestamp() - READING_LOG_DAYS * DAY).max(since(store));
    let mut st = store
        .conn()
        .prepare_cached("SELECT DISTINCT (opened_at + ?1) / 86400 FROM items WHERE opened_at >= ?2")
        .map_err(err)?;
    let days: std::collections::HashSet<i64> = st
        .query_map(params![off, from], |r| r.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let today = day_number(now.timestamp(), now.offset());
    let today_counts = days.contains(&today);
    let mut d = if today_counts { today } else { today - 1 };
    let mut n = 0;
    while days.contains(&d) {
        n += 1;
        d -= 1;
    }
    Ok((n, today_counts))
}

fn rate(part: i64, whole: i64) -> f64 {
    if whole > 0 {
        part as f64 / whole as f64
    } else {
        0.0
    }
}

fn low(engaged: i64, fetched: i64, min: i64) -> bool {
    fetched >= min && engaged * LOW_RATE_ONE_IN < fetched
}

/// "2", "under 1" or "0" (per cent, for sentences).
fn percent(part: i64, whole: i64) -> String {
    let p = rate(part, whole) * 100.0;
    if part == 0 {
        "0".into()
    } else if p < 1.0 {
        "under 1".into()
    } else {
        format!("{}", p.round())
    }
}

fn median(v: &mut [i64]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    let m = v.len() / 2;
    let secs = if v.len().is_multiple_of(2) {
        (v[m - 1] + v[m]) as f64 / 2.0
    } else {
        v[m] as f64
    };
    Some((secs / DAY as f64 * 10.0).round() / 10.0)
}

/// `cs` (a whole archive) covers `cs.LG`; otherwise the ids must be equal.
fn followed_category(followed: &[String], id: &str) -> bool {
    followed.iter().any(|f| {
        f == id
            || (!f.contains('.')
                && id
                    .strip_prefix(f.as_str())
                    .is_some_and(|rest| rest.starts_with('.')))
    })
}

impl Stats {
    pub fn build(
        store: &Store,
        topics: &Topics,
        settings: &Settings,
        now: DateTime<FixedOffset>,
        range: Range,
    ) -> Result<Stats, String> {
        let p = Plan::new(range, now, since(store));
        let reading = reading(store, &p, now)?;
        let (week_start, done) = week_progress(store, now)?;
        let goal = Goal {
            target: settings.weekly_goal,
            done,
            week_start,
        };
        let topic_worth = topic_worth(store, topics, &p)?;
        let categories = category_worth(store, topics, &p)?;
        let feeds = feed_worth(store, topics, &p)?;
        let people = people(store, topics, &p)?;
        let backlog = backlog(store, &p)?;
        let time_to_read = time_to_read(store, topics, &p)?;
        let mut stats = Stats {
            range,
            bucket: if p.days == 1 { "day" } else { "week" },
            start: p.first,
            today: now.date_naive(),
            reading,
            goal,
            topics: topic_worth,
            categories,
            feeds,
            people,
            backlog,
            suggestions: Vec::new(),
            time_to_read,
            since: Some(since(store))
                .filter(|t| *t > 0)
                .and_then(|t| DateTime::from_timestamp(t, 0))
                .map(|t| t.with_timezone(now.offset()).date_naive()),
        };
        stats.suggestions = suggestions(store, &stats, &p)?;
        Ok(stats)
    }

    /// A short text summary (for `dispatch-cli stats`).
    pub fn summary(&self) -> String {
        let t = &self.reading.totals;
        let mut out = format!(
            "{} → {} (by {})\nread {} papers, {} news · seen {} · saved {} · noted {} · streak {} days\n",
            self.start,
            self.today,
            self.bucket,
            t.read,
            t.news_read,
            t.seen,
            t.saved,
            t.noted,
            self.reading.streak
        );
        if self.goal.target > 0 {
            out.push_str(&format!(
                "this week: {} / {} papers\n",
                self.goal.done, self.goal.target
            ));
        }
        if let (Some(first), Some(last)) = (self.backlog.first(), self.backlog.last()) {
            out.push_str(&format!(
                "backlog: research {} → {}, news {} → {}\n",
                first.research, last.research, first.news, last.news
            ));
        }
        if let Some(m) = self.time_to_read.median_days {
            out.push_str(&format!(
                "time to read: median {m} days over {} papers\n",
                self.time_to_read.count
            ));
        }
        out.push_str("topics:\n");
        for w in &self.topics {
            out.push_str(&format!(
                "  {:<24} {:>6} in, {:>5} read ({:.0}%), {} seen, {} saved\n",
                w.name,
                w.fetched,
                w.read,
                w.rate * 100.0,
                w.seen,
                w.saved
            ));
        }
        out.push_str("feeds:\n");
        for f in &self.feeds {
            out.push_str(&format!(
                "  {:<24} {:>6} in, {:>5} read, {} seen{}\n",
                f.name,
                f.fetched,
                f.read,
                f.seen,
                if f.noisy { "  (noisy)" } else { "" }
            ));
        }
        if !self.people.authors.is_empty() {
            let names: Vec<String> = self
                .people
                .authors
                .iter()
                .map(|a| format!("{} ({})", a.name, a.count))
                .collect();
            out.push_str(&format!("people: {}\n", names.join(", ")));
        }
        for s in &self.suggestions {
            out.push_str(&format!("* {}\n", s.text));
        }
        out
    }
}

fn reading(store: &Store, p: &Plan, now: DateTime<FixedOffset>) -> Result<Reading, String> {
    // ?1 is the buckets' origin, ?4 the earliest time counted (after a reset).
    let a: [&dyn ToSql; 4] = [&p.start, &p.width, &p.end, &p.from];
    let read = bucketed(
        store,
        "SELECT (opened_at - ?1) / ?2, COUNT(*) FROM items
         WHERE section = 'research' AND opened_at >= ?4 AND opened_at < ?3 GROUP BY 1",
        &a,
        p.n,
    )?;
    let news_read = bucketed(
        store,
        "SELECT (opened_at - ?1) / ?2, COUNT(*) FROM items
         WHERE section = 'news' AND opened_at >= ?4 AND opened_at < ?3 GROUP BY 1",
        &a,
        p.n,
    )?;
    let seen = bucketed(
        store,
        "SELECT (read_at - ?1) / ?2, COUNT(*) FROM items
         WHERE read_at >= ?4 AND read_at < ?3 AND read_dismissed = 0 GROUP BY 1",
        &a,
        p.n,
    )?;
    let saved = bucketed(
        store,
        "SELECT (saved_at - ?1) / ?2, COUNT(*) FROM user_items
         WHERE saved_at >= ?4 AND saved_at < ?3 GROUP BY 1",
        &a,
        p.n,
    )?;
    let noted = bucketed(
        store,
        "SELECT (note_stamp / 1000 - ?1) / ?2, COUNT(*) FROM user_items
         WHERE note != '' AND note_stamp / 1000 >= ?4 AND note_stamp / 1000 < ?3 GROUP BY 1",
        &a,
        p.n,
    )?;
    let mut totals = Counts::default();
    let buckets = (0..p.n as usize)
        .map(|i| {
            let counts = Counts {
                read: read[i],
                news_read: news_read[i],
                seen: seen[i],
                saved: saved[i],
                noted: noted[i],
            };
            totals.read += counts.read;
            totals.news_read += counts.news_read;
            totals.seen += counts.seen;
            totals.saved += counts.saved;
            totals.noted += counts.noted;
            Bucket {
                date: p.date(i as i64),
                counts,
            }
        })
        .collect();
    let (streak, streak_today) = streak(store, now)?;
    Ok(Reading {
        buckets,
        totals,
        streak,
        streak_today,
    })
}

fn topic_worth(store: &Store, topics: &Topics, p: &Plan) -> Result<Vec<Worth>, String> {
    let mut st = store
        .conn()
        .prepare_cached(
            "SELECT t.topic, COUNT(*), SUM(IFNULL(i.opened_at, 0) >= ?1), SUM(IFNULL(i.read_at, 0) >= ?1 AND i.read_dismissed = 0),
                    SUM(IFNULL(u.saved_at, 0) >= ?1)
             FROM items i JOIN item_topics t ON t.key = i.key
             LEFT JOIN user_items u ON u.key = i.key
             WHERE i.section = 'research' AND i.first_seen >= ?1 AND i.first_seen < ?2
             GROUP BY t.topic",
        )
        .map_err(err)?;
    let mut found: HashMap<String, [i64; 4]> = HashMap::new();
    let rows = st
        .query_map(params![p.from, p.end], |r| {
            Ok((
                r.get::<_, String>(0)?,
                [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
            ))
        })
        .map_err(err)?;
    for row in rows {
        let (t, c) = row.map_err(err)?;
        found.insert(t, c);
    }
    Ok(topics
        .research
        .iter()
        .filter(|t| t.enabled)
        .map(|t| {
            let [fetched, read, seen, saved] = found.get(&t.id).copied().unwrap_or_default();
            Worth {
                id: t.id.clone(),
                name: t.name.clone(),
                fetched,
                read,
                seen,
                saved,
                rate: rate(read, fetched),
            }
        })
        .collect())
}

fn category_worth(store: &Store, topics: &Topics, p: &Plan) -> Result<Vec<Worth>, String> {
    let followed = topics.arxiv_categories();
    let mut st = store
        .conn()
        .prepare_cached(&format!(
            "SELECT c.value, COUNT(*), SUM(IFNULL(i.opened_at, 0) >= ?1), SUM(IFNULL(i.read_at, 0) >= ?1 AND i.read_dismissed = 0),
                    SUM(IFNULL(u.saved_at, 0) >= ?1)
             FROM items i JOIN json_each(i.categories) c
             LEFT JOIN user_items u ON u.key = i.key
             WHERE i.section = 'research' AND i.first_seen >= ?1 AND i.first_seen < ?2
               AND {IN_TOPIC}
             GROUP BY c.value"
        ))
        .map_err(err)?;
    let rows = st
        .query_map(params![p.from, p.end], |r| {
            let (fetched, read): (i64, i64) = (r.get(1)?, r.get(2)?);
            Ok(Worth {
                id: r.get(0)?,
                name: String::new(),
                fetched,
                read,
                seen: r.get(3)?,
                saved: r.get(4)?,
                rate: rate(read, fetched),
            })
        })
        .map_err(err)?;
    let catalogue = crate::catalogue::load();
    let mut out: Vec<Worth> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?
        .into_iter()
        .filter(|w| followed_category(&followed, &w.id))
        .map(|mut w| {
            w.name = catalogue.name_of(&w.id).unwrap_or_default().to_owned();
            w
        })
        .collect();
    out.sort_by(|a, b| b.fetched.cmp(&a.fetched).then(a.id.cmp(&b.id)));
    Ok(out)
}

fn feed_worth(store: &Store, topics: &Topics, p: &Plan) -> Result<Vec<FeedWorth>, String> {
    let feeds = topics.news_feeds();
    let mut st = store
        .conn()
        .prepare_cached(&format!(
            "SELECT i.source, MAX(i.source_name), COUNT(*), SUM(IFNULL(i.opened_at, 0) >= ?1),
                    SUM(IFNULL(i.read_at, 0) >= ?1 AND i.read_dismissed = 0)
             FROM items i
             WHERE i.section = 'news' AND i.first_seen >= ?1 AND i.first_seen < ?2 AND {IN_TOPIC}
             GROUP BY i.source"
        ))
        .map_err(err)?;
    let rows = st
        .query_map(params![p.from, p.end], |r| {
            let source: String = r.get(0)?;
            let (fetched, read, seen): (i64, i64, i64) = (r.get(2)?, r.get(3)?, r.get(4)?);
            Ok(FeedWorth {
                name: feeds
                    .iter()
                    .find(|f| f.url == source)
                    .map(|f| f.name.clone())
                    .unwrap_or(r.get(1)?),
                source,
                fetched,
                read,
                seen,
                rate: rate(read, fetched),
                noisy: low(read.max(seen), fetched, NOISY_MIN_ITEMS),
            })
        })
        .map_err(err)?;
    let mut out = rows.collect::<Result<Vec<_>, _>>().map_err(err)?;
    out.sort_by(|a, b| b.fetched.cmp(&a.fetched).then(a.name.cmp(&b.name)));
    Ok(out)
}

fn people(store: &Store, topics: &Topics, p: &Plan) -> Result<People, String> {
    let mut st = store
        .conn()
        .prepare_cached(
            "SELECT i.authors FROM items i LEFT JOIN user_items u ON u.key = i.key
             WHERE i.section = 'research'
               AND ((i.opened_at >= ?1 AND i.opened_at < ?2)
                    OR (u.saved_at >= ?1 AND u.saved_at < ?2))",
        )
        .map_err(err)?;
    let rows = st
        .query_map(params![p.from, p.end], |r| r.get::<_, String>(0))
        .map_err(err)?;
    // Keyed by the folded name so "Weißler" and "Weissler" are one person; the first spelling
    // seen is the one shown.
    let mut counts: HashMap<String, (String, i64)> = HashMap::new();
    for row in rows {
        let authors: Vec<String> = serde_json::from_str(&row.map_err(err)?).unwrap_or_default();
        for a in authors {
            let a = a.split_whitespace().collect::<Vec<_>>().join(" ");
            if a.is_empty() {
                continue;
            }
            counts.entry(fold(&a)).or_insert_with(|| (a, 0)).1 += 1;
        }
    }
    let mut authors: Vec<Person> = counts
        .into_values()
        .map(|(name, count)| Person {
            followed: topics.follow.is_followed(&name),
            name,
            count,
        })
        .collect();
    authors.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    authors.truncate(TOP_PEOPLE);
    let mut tags = store.tags()?;
    tags.truncate(TOP_TAGS);
    Ok(People { authors, tags })
}

/// Waiting at the end of each bucket = waiting at the range's start + arrivals − reads so far.
/// An item read "before" it was fetched (another Mac's read state) leaves when it arrives.
fn backlog(store: &Store, p: &Plan) -> Result<Vec<Backlog>, String> {
    let gone = "MAX(i.read_at, i.first_seen)";
    let per = |section: &str| -> Result<Vec<i64>, String> {
        let a: [&dyn ToSql; 4] = [&p.start, &p.width, &p.end, &section];
        let base = backlog_at_start(store, p, section)?;
        let arrived = bucketed(
            store,
            &format!(
                "SELECT (i.first_seen - ?1) / ?2, COUNT(*) FROM items i
                 WHERE i.section = ?4 AND i.first_seen >= ?1 AND i.first_seen < ?3 AND {IN_TOPIC}
                 GROUP BY 1"
            ),
            &a,
            p.n,
        )?;
        let left = bucketed(
            store,
            &format!(
                "SELECT ({gone} - ?1) / ?2, COUNT(*) FROM items i
                 WHERE i.section = ?4 AND i.read_at IS NOT NULL AND {gone} >= ?1 AND {gone} < ?3
                   AND {IN_TOPIC}
                 GROUP BY 1"
            ),
            &a,
            p.n,
        )?;
        let mut n = base;
        Ok(arrived
            .iter()
            .zip(&left)
            .map(|(a, l)| {
                n += a - l;
                n
            })
            .collect())
    };
    let research = per("research")?;
    let news = per("news")?;
    Ok((0..p.n as usize)
        .map(|i| Backlog {
            date: p.date(i as i64),
            research: research[i],
            news: news[i],
        })
        .collect())
}

/// What was waiting when the range began.
fn backlog_at_start(store: &Store, p: &Plan, section: &str) -> Result<i64, String> {
    one(
        store,
        &format!(
            "SELECT COUNT(*) FROM items i WHERE i.section = ?2 AND i.first_seen < ?1
               AND (i.read_at IS NULL OR MAX(i.read_at, i.first_seen) >= ?1) AND {IN_TOPIC}"
        ),
        &[&p.start, &section],
    )
}

fn time_to_read(store: &Store, topics: &Topics, p: &Plan) -> Result<TimeToRead, String> {
    let mut st = store
        .conn()
        .prepare_cached(
            "SELECT MAX(0, i.opened_at - COALESCE(i.published, i.first_seen)),
                    (SELECT group_concat(topic, ',') FROM item_topics t WHERE t.key = i.key)
             FROM items i
             WHERE i.section = 'research' AND i.opened_at >= ?1 AND i.opened_at < ?2",
        )
        .map_err(err)?;
    let rows = st
        .query_map(params![p.from, p.end], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })
        .map_err(err)?;
    let mut all = Vec::new();
    let mut per: HashMap<String, Vec<i64>> = HashMap::new();
    for row in rows {
        let (d, ts) = row.map_err(err)?;
        all.push(d);
        for t in ts.iter().flat_map(|s| s.split(',')) {
            per.entry(t.to_owned()).or_default().push(d);
        }
    }
    let within = all.iter().filter(|d| **d <= DAY).count();
    let within_day = if all.is_empty() {
        0.0
    } else {
        within as f64 / all.len() as f64
    };
    let topics_out = topics
        .research
        .iter()
        .filter(|t| t.enabled)
        .filter_map(|t| {
            let v = per.get_mut(&t.id)?;
            Some(Median {
                id: t.id.clone(),
                name: t.name.clone(),
                count: v.len(),
                median_days: median(v)?,
            })
        })
        .collect();
    Ok(TimeToRead {
        count: all.len(),
        median_days: median(&mut all),
        within_day,
        topics: topics_out,
    })
}

/// 0–3 calm sentences about what would make the feed lighter.
fn suggestions(store: &Store, s: &Stats, p: &Plan) -> Result<Vec<Suggestion>, String> {
    let mut out = Vec::new();
    let topics_action = || Some("topics".to_owned());
    let start = backlog_at_start(store, p, "research")?;
    let now = s.backlog.last().map(|b| b.research).unwrap_or(start);
    let grew = now - start;
    let range = s.range.words();
    let skimmed = s
        .topics
        .iter()
        .filter(|t| low(t.read.max(t.seen), t.fetched, SKIMMED_MIN_ITEMS))
        .max_by_key(|t| t.fetched);
    match (grew >= GREW_BY, skimmed) {
        (true, Some(t)) => out.push(Suggestion {
            text: format!(
                "Your research backlog grew by {grew} in {range}. You read {}% of {} ({} papers): \
                 narrowing it with keywords in Topics would trim it.",
                percent(t.read, t.fetched),
                t.name,
                t.fetched
            ),
            action: topics_action(),
        }),
        (false, Some(t)) => out.push(Suggestion {
            text: format!(
                "You read {}% of {} ({} papers in {range}): narrowing it with keywords in Topics \
                 would trim it.",
                percent(t.read, t.fetched),
                t.name,
                t.fetched
            ),
            action: topics_action(),
        }),
        (true, None) => out.push(Suggestion {
            text: format!(
                "Your research backlog grew by {grew} in {range}. Mark all read on a topic you \
                 only skim gives a fresh start, and it has Undo."
            ),
            action: None,
        }),
        (false, None) => {}
    }
    if let Some(f) = s.feeds.iter().filter(|f| f.noisy).max_by_key(|f| f.fetched) {
        out.push(Suggestion {
            text: format!(
                "{}: {} items, {} read — switching it off would make News calmer.",
                f.name, f.fetched, f.read
            ),
            action: topics_action(),
        });
    }
    let t = &s.time_to_read;
    if grew <= 0 && t.count >= 10 && t.within_day >= 0.5 {
        out.push(Suggestion {
            text: "You keep up: most papers you open, you open within a day.".into(),
            action: None,
        });
    }
    out.truncate(3);
    Ok(out)
}

/// Now in this Mac's time zone, for callers outside tests.
pub fn local_now() -> DateTime<FixedOffset> {
    chrono::Local::now().fixed_offset()
}

/// A UTC time seen from this Mac's time zone (the companion's "this week").
pub fn local(t: DateTime<Utc>) -> DateTime<FixedOffset> {
    t.with_timezone(&chrono::Local).fixed_offset()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_line_up_with_local_days_and_mondays() {
        let utc8 = FixedOffset::east_opt(8 * 3600).unwrap();
        // Friday 30 October 2026, 01:00 at UTC+8 (still the 29th in UTC).
        let now: DateTime<FixedOffset> = "2026-10-30T01:00:00+08:00".parse().unwrap();
        let p = Plan::new(Range::Weeks4, now, 0);
        assert_eq!(p.first, NaiveDate::from_ymd_opt(2026, 10, 3).unwrap());
        assert_eq!(p.start, midnight(p.first, &utc8));
        assert_eq!(p.date(27), now.date_naive());
        let p = Plan::new(Range::Months3, now, 0);
        assert_eq!(
            p.date(12),
            NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(),
            "this Monday"
        );
        assert_eq!(p.end - p.start, 13 * 7 * DAY);
        assert_eq!(
            day_number(now.timestamp(), &utc8),
            day_number(midnight(now.date_naive(), &utc8), &utc8)
        );
    }

    #[test]
    fn medians_and_percentages_read_well() {
        assert_eq!(median(&mut [3 * DAY, DAY, 2 * DAY]), Some(2.0));
        assert_eq!(median(&mut [0, DAY]), Some(0.5));
        assert_eq!(median(&mut []), None);
        assert_eq!(percent(2, 600), "under 1");
        assert_eq!(percent(12, 600), "2");
        assert_eq!(percent(0, 600), "0");
        assert!(followed_category(&["cs".into()], "cs.LG"));
        assert!(!followed_category(&["cs".into()], "cond-mat.mes-hall"));
        assert!(!followed_category(&["physics.optics".into()], "physics"));
    }
}
