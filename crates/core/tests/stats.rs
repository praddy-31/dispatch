//! Stats: numbers built by hand in an in-memory store, at chosen times, in
//! UTC+8 so local days differ from UTC days.

use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use dispatch::settings::Settings;
use dispatch::stats::{self, Range, ReadingLog, Stats, week_progress};
use dispatch::store::Store;
use dispatch::topics::{Category, Feed, NewsTopic, ResearchTopic, Topics};
use dispatch::{Item, Section};

const LOUD: &str = "https://loud.example/feed";
const QUIET: &str = "https://quiet.example/feed";

/// Friday 30 October 2026, 15:00 at UTC+8.
fn now() -> DateTime<FixedOffset> {
    utc8("2026-10-30T15:00:00")
}

fn utc8(s: &str) -> DateTime<FixedOffset> {
    format!("{s}+08:00").parse().unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    utc8(s).with_timezone(&Utc)
}

fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}

fn research(id: &str, name: &str, cat: &str) -> ResearchTopic {
    ResearchTopic {
        id: id.into(),
        name: name.into(),
        enabled: true,
        categories: vec![Category {
            id: cat.into(),
            enabled: true,
        }],
        include: vec![],
        exclude: vec![],
        category_include: Default::default(),
    }
}

fn topics() -> Topics {
    Topics {
        version: 1,
        research: vec![
            research("quantum", "Quantum", "quant-ph"),
            research("optics", "Optics", "physics.optics"),
        ],
        news: vec![NewsTopic {
            id: "science".into(),
            name: "Science".into(),
            enabled: true,
            sources: vec![
                Feed {
                    name: "Loud Weekly".into(),
                    url: LOUD.into(),
                    enabled: true,
                },
                Feed {
                    name: "Quiet Review".into(),
                    url: QUIET.into(),
                    enabled: true,
                },
            ],
            include: vec![],
            exclude: vec![],
        }],
        follow: Default::default(),
        pinned: vec![],
        mute: vec![],
    }
}

fn paper(id: &str, cats: &[&str], published: Option<DateTime<Utc>>, authors: &[&str]) -> Item {
    Item {
        key: format!("arxiv:{id}"),
        section: Section::Research,
        source: "arxiv".into(),
        source_name: "arXiv".into(),
        title: format!("Paper {id}"),
        authors: authors.iter().map(|a| (*a).into()).collect(),
        summary: String::new(),
        link: format!("https://arxiv.org/abs/{id}"),
        pdf: Some(format!("https://arxiv.org/pdf/{id}")),
        categories: cats.iter().map(|c| (*c).into()).collect(),
        published,
        announce_type: Some("new".into()),
        version: Some(1),
        doi: None,
        journal_ref: None,
    }
}

fn news(n: usize, feed: &str) -> Item {
    Item {
        key: format!("news:{feed}/{n}"),
        section: Section::News,
        source: feed.into(),
        source_name: "Feed".into(),
        title: format!("Story {n}"),
        authors: vec![],
        summary: String::new(),
        link: format!("{feed}/{n}"),
        pdf: None,
        categories: vec![],
        published: None,
        announce_type: None,
        version: None,
        doi: None,
        journal_ref: None,
    }
}

fn keys(items: &[Item]) -> Vec<String> {
    items.iter().map(|i| i.key.clone()).collect()
}

fn store_with(items: &[Item], at: DateTime<Utc>) -> Store {
    let mut s = Store::in_memory().unwrap();
    s.upsert(items, at).unwrap();
    s.retag(&topics()).unwrap();
    s
}

fn build(store: &Store, settings: &Settings, range: Range) -> Stats {
    Stats::build(store, &topics(), settings, now(), range).unwrap()
}

/// Index of a local date in the 4-week range (which starts on 3 October).
fn day(s: &str) -> usize {
    (date(s) - date("2026-10-03")).num_days() as usize
}

/// Reads items as the user does: each one shown in the preview (seen), then its page or PDF
/// opened, which is what Stats counts as read. Bulk marking is neither;
/// see `bulk_mark_all_read_is_not_reading`.
trait ReadEach {
    fn read_each(&mut self, keys: &[String], read: bool, at: DateTime<Utc>) -> Result<(), String>;
}

impl ReadEach for Store {
    fn read_each(&mut self, keys: &[String], read: bool, at: DateTime<Utc>) -> Result<(), String> {
        keys.iter().try_for_each(|k| {
            self.set_read(k, read, at)?;
            self.set_opened(k, at)
        })
    }
}

#[test]
fn reading_counts_per_day_and_week_a_streak_and_the_weekly_goal() {
    let papers: Vec<Item> = (1..=5)
        .map(|i| paper(&format!("2610.0000{i}"), &["quant-ph"], None, &[]))
        .collect();
    let n = news(1, QUIET);
    let mut all = papers.clone();
    all.push(n.clone());
    let mut store = store_with(&all, utc("2026-10-20T13:30:00"));
    let k = keys(&papers);
    store
        .read_each(&k[0..1], true, utc("2026-10-30T10:00:00"))
        .unwrap();
    store
        .read_each(&k[1..2], true, utc("2026-10-29T10:00:00"))
        .unwrap();
    store
        .read_each(&k[2..3], true, utc("2026-10-28T10:00:00"))
        .unwrap();
    // 07:00 on Monday at UTC+8 is still Sunday in UTC: it counts for this week.
    store
        .read_each(&k[3..4], true, utc("2026-10-26T07:00:00"))
        .unwrap();
    store
        .read_each(
            std::slice::from_ref(&n.key),
            true,
            utc("2026-10-30T11:00:00"),
        )
        .unwrap();
    store
        .set_saved(&k[4..5], true, utc("2026-10-30T12:00:00"))
        .unwrap();
    store
        .set_note(&k[0], "nice bound", utc("2026-10-30T12:30:00"))
        .unwrap();
    store.set_opened(&k[0], utc("2026-10-30T10:01:00")).unwrap();

    let settings = Settings {
        weekly_goal: 20,
        ..Settings::default()
    };
    let s = build(&store, &settings, Range::Weeks4);
    assert_eq!(s.bucket, "day");
    assert_eq!(s.start, date("2026-10-03"));
    assert_eq!(s.reading.buckets.len(), 28);
    let b = |d: &str| &s.reading.buckets[day(d)].counts;
    assert_eq!(
        s.reading.buckets[day("2026-10-30")].date,
        date("2026-10-30")
    );
    assert_eq!((b("2026-10-30").read, b("2026-10-30").news_read), (1, 1));
    assert_eq!(b("2026-10-26").read, 1, "local Monday, not UTC Sunday");
    assert_eq!(b("2026-10-25").read, 0);
    assert_eq!(b("2026-10-27").read, 0);
    let t = &s.reading.totals;
    // Saved is 2: the noted paper was saved by its note (a note saves its paper).
    assert_eq!(
        (t.read, t.news_read, t.seen, t.saved, t.noted),
        (4, 1, 5, 2, 1)
    );
    // Today, yesterday and the day before; the 27th is a gap.
    assert_eq!((s.reading.streak, s.reading.streak_today), (3, true));
    assert_eq!(
        (s.goal.target, s.goal.done, s.goal.week_start),
        (20, 4, date("2026-10-26"))
    );
    assert_eq!(
        week_progress(&store, now()).unwrap(),
        (date("2026-10-26"), 4)
    );

    let s = build(&store, &settings, Range::Months3);
    assert_eq!((s.bucket, s.reading.buckets.len()), ("week", 13));
    let last = s.reading.buckets.last().unwrap();
    assert_eq!((last.date, last.counts.read), (date("2026-10-26"), 4));
    assert_eq!(
        build(&store, &settings, Range::Year).reading.buckets.len(),
        52
    );

    // Nothing read today yet: the streak still ends yesterday.
    let tomorrow = utc8("2026-10-31T09:00:00");
    let s = Stats::build(&store, &topics(), &settings, tomorrow, Range::Weeks4).unwrap();
    assert_eq!((s.reading.streak, s.reading.streak_today), (3, false));
    let later = utc8("2026-11-02T09:00:00");
    let s = Stats::build(&store, &topics(), &settings, later, Range::Weeks4).unwrap();
    assert_eq!(s.reading.streak, 0, "two quiet days end it");
    assert_eq!(s.goal.done, 0, "a new week starts on Monday");
}

#[test]
fn the_backlog_counts_what_was_waiting_at_the_end_of_each_day() {
    let early: Vec<Item> = (1..=3)
        .map(|i| paper(&format!("2610.1000{i}"), &["quant-ph"], None, &[]))
        .collect();
    let late: Vec<Item> = (1..=2)
        .map(|i| paper(&format!("2610.2000{i}"), &["physics.optics"], None, &[]))
        .collect();
    // In no topic: never counted.
    let stray = paper("2610.30001", &["cs.LG"], None, &[]);
    let mut store = Store::in_memory().unwrap();
    store
        .upsert(
            &[early.clone(), vec![stray]].concat(),
            utc("2026-10-10T13:30:00"),
        )
        .unwrap();
    store.upsert(&late, utc("2026-10-25T13:30:00")).unwrap();
    let old_news = news(1, QUIET);
    store
        .upsert(std::slice::from_ref(&old_news), utc("2026-09-01T08:00:00"))
        .unwrap();
    store.retag(&topics()).unwrap();
    store
        .set_read(&early[0].key, true, utc("2026-10-20T09:00:00"))
        .unwrap();
    store
        .set_read(&late[0].key, true, utc("2026-10-30T09:00:00"))
        .unwrap();

    let s = build(&store, &Settings::default(), Range::Weeks4);
    let r = |d: &str| s.backlog[day(d)].research;
    assert_eq!(s.backlog[day("2026-10-09")].date, date("2026-10-09"));
    assert_eq!(r("2026-10-09"), 0);
    assert_eq!(r("2026-10-10"), 3);
    assert_eq!(r("2026-10-19"), 3);
    assert_eq!(r("2026-10-20"), 2);
    assert_eq!(r("2026-10-25"), 4);
    assert_eq!(r("2026-10-30"), 3);
    // Fetched before the range began and never read: waiting all along.
    assert!(s.backlog.iter().all(|b| b.news == 1));

    // Read before the range began: gone from the start.
    store
        .set_read(&old_news.key, true, utc("2026-09-02T08:00:00"))
        .unwrap();
    let s = build(&store, &Settings::default(), Range::Weeks4);
    assert!(s.backlog.iter().all(|b| b.news == 0));
}

#[test]
fn time_to_read_is_the_median_from_announcement() {
    let announced = utc("2026-10-20T08:00:00");
    let papers: Vec<Item> = (1..=3)
        .map(|i| {
            let cat = if i == 3 { "physics.optics" } else { "quant-ph" };
            paper(&format!("2610.4000{i}"), &[cat], Some(announced), &[])
        })
        .collect();
    let store = store_with(&papers, utc("2026-10-20T13:30:00"));
    store
        .set_opened(&papers[0].key, announced + Duration::hours(12))
        .unwrap();
    store
        .set_opened(&papers[1].key, announced + Duration::days(1))
        .unwrap();
    store
        .set_opened(&papers[2].key, announced + Duration::days(3))
        .unwrap();
    let s = build(&store, &Settings::default(), Range::Weeks4);
    let t = &s.time_to_read;
    assert_eq!((t.median_days, t.count), (Some(1.0), 3));
    assert!((t.within_day - 2.0 / 3.0).abs() < 1e-9);
    let by: Vec<(&str, f64, usize)> = t
        .topics
        .iter()
        .map(|m| (m.id.as_str(), m.median_days, m.count))
        .collect();
    assert_eq!(by, vec![("quantum", 0.8, 2), ("optics", 3.0, 1)]);
}

#[test]
fn topics_feeds_and_people_show_what_is_worth_it_with_gentle_suggestions() {
    let at = utc("2026-10-15T13:30:00");
    let mut items: Vec<Item> = (0..10)
        .map(|i| {
            paper(
                &format!("2610.5{i:04}"),
                if i == 0 {
                    &["quant-ph", "cs.LG"]
                } else {
                    &["quant-ph"]
                },
                Some(at),
                &["Ana Lima", &format!("Coauthor {i}")],
            )
        })
        .collect();
    items.extend(
        (0..120).map(|i| paper(&format!("2610.6{i:04}"), &["physics.optics"], Some(at), &[])),
    );
    items.extend((0..40).map(|i| news(i, LOUD)));
    items.extend((0..5).map(|i| news(i, QUIET)));
    let mut store = store_with(&items, at);
    let k = keys(&items);
    let read_at = utc("2026-10-15T20:00:00");
    store.read_each(&k[0..8], true, read_at).unwrap();
    store.set_saved(&k[8..10], true, read_at).unwrap();
    store.set_opened(&k[0], read_at).unwrap();
    store.read_each(&k[10..11], true, read_at).unwrap(); // one optics paper
    store.read_each(&k[130..131], true, read_at).unwrap(); // one Loud story
    store.set_read(&k[11], true, read_at).unwrap(); // an optics paper only seen
    store.read_each(&k[170..173], true, read_at).unwrap(); // three Quiet ones
    store.set_tags(&k[0], &["thesis".into()], read_at).unwrap();

    let mut t = topics();
    t.follow.authors = vec!["A. Lima".into()];
    let s = Stats::build(&store, &t, &Settings::default(), now(), Range::Weeks4).unwrap();

    let q = &s.topics[0];
    assert_eq!(
        (q.id.as_str(), q.fetched, q.read, q.seen, q.saved),
        ("quantum", 10, 8, 8, 2)
    );
    assert!((q.rate - 0.8).abs() < 1e-9);
    let o = &s.topics[1];
    assert_eq!(
        (o.id.as_str(), o.fetched, o.read, o.seen),
        ("optics", 120, 1, 2)
    );

    let cats: Vec<(&str, i64)> = s
        .categories
        .iter()
        .map(|c| (c.id.as_str(), c.fetched))
        .collect();
    assert_eq!(
        cats,
        vec![("physics.optics", 120), ("quant-ph", 10)],
        "cs.LG isn't followed"
    );
    assert!(!s.categories[0].name.is_empty(), "named from the catalogue");

    let feeds: Vec<(&str, i64, i64, bool)> = s
        .feeds
        .iter()
        .map(|f| (f.name.as_str(), f.fetched, f.read, f.noisy))
        .collect();
    assert_eq!(
        feeds,
        vec![("Loud Weekly", 40, 1, true), ("Quiet Review", 5, 3, false)]
    );

    let lima = &s.people.authors[0];
    assert_eq!(
        (lima.name.as_str(), lima.count, lima.followed),
        ("Ana Lima", 10, true)
    );
    assert!(
        s.people.authors[1..]
            .iter()
            .all(|p| p.count == 1 && !p.followed)
    );
    assert_eq!(s.people.tags, vec![("thesis".to_owned(), 1)]);

    let texts: Vec<&str> = s.suggestions.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(s.suggestions.len(), 2, "{texts:?}");
    assert!(
        texts[0].starts_with(
            "Your research backlog grew by 120 in 4 weeks. You read under 1% of Optics (120 papers)"
        ),
        "{texts:?}"
    );
    assert_eq!(
        texts[1],
        "Loud Weekly: 40 items, 1 read — switching it off would make News calmer."
    );
    assert!(
        s.suggestions
            .iter()
            .all(|x| x.action.as_deref() == Some("topics"))
    );

    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["range"], "4w");
    assert_eq!(json["reading"]["buckets"][0]["date"], "2026-10-03");
    assert!(
        json["reading"]["buckets"][0]["read"].is_i64(),
        "counts are flattened"
    );
}

#[test]
fn keeping_up_gets_a_kind_word_and_quiet_feeds_none() {
    let at = utc("2026-10-28T13:30:00");
    let papers: Vec<Item> = (0..12)
        .map(|i| paper(&format!("2610.7{i:04}"), &["quant-ph"], Some(at), &[]))
        .collect();
    let mut store = store_with(&papers, at);
    store
        .read_each(&keys(&papers), true, at + Duration::hours(3))
        .unwrap();
    let s = build(&store, &Settings::default(), Range::Weeks4);
    let texts: Vec<&str> = s.suggestions.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(
        texts,
        vec!["You keep up: most papers you open, you open within a day."]
    );
    assert_eq!(s.suggestions[0].action, None);
    assert_eq!(s.goal.target, 0, "no goal unless one is set");
}

#[test]
fn a_version_4_database_gains_opened_and_keeps_everything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.sqlite");
    let p = paper("2610.80001", &["quant-ph"], None, &[]);
    {
        let mut s = Store::open(&path).unwrap();
        s.upsert(std::slice::from_ref(&p), utc("2026-10-20T13:30:00"))
            .unwrap();
        s.retag(&topics()).unwrap();
        s.set_read(&p.key, true, utc("2026-10-21T09:00:00"))
            .unwrap();
        s.set_note(&p.key, "kept", utc("2026-10-21T09:00:00"))
            .unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("ALTER TABLE items DROP COLUMN opened_at; PRAGMA user_version = 4;")
        .unwrap();
    drop(conn);
    let s = Store::open(&path).unwrap();
    // Brought up through every later step too (v6: the "Read later" column;
    // v7: LaTeX accents in author names).
    assert_eq!(s.schema_version(), 7);
    assert_eq!(dispatch::store::SCHEMA_VERSION, 7);
    let rows = s.list(Section::Research, None, 10).unwrap();
    assert!(rows[0].later_at.is_none());
    assert_eq!(rows.len(), 1);
    assert!(rows[0].read);
    assert_eq!(rows[0].note, "kept");
    assert_eq!(s.indexed().unwrap(), 1, "search index untouched");
    let stats = build(&s, &Settings::default(), Range::Weeks4);
    assert_eq!(
        (stats.reading.totals.read, stats.reading.totals.seen),
        (0, 1),
        "seen, never opened"
    );
    s.set_opened(&p.key, utc("2026-10-30T10:00:00")).unwrap();
    let stats = build(&s, &Settings::default(), Range::Weeks4);
    assert_eq!(stats.reading.totals.read, 1);
    assert_eq!(stats.reading.buckets[day("2026-10-30")].counts.read, 1);
}

#[test]
fn only_opening_counts_as_reading_and_a_reset_starts_from_zero() {
    let papers: Vec<Item> = (1..=3)
        .map(|i| paper(&format!("2610.8100{i}"), &["quant-ph"], None, &[]))
        .collect();
    let mut store = store_with(&papers, utc("2026-10-20T13:30:00"));
    let k = keys(&papers);
    let settings = Settings::default();
    // Seen in the preview only: not read, no streak, nothing for the goal.
    store
        .set_read(&k[0], true, utc("2026-10-29T09:00:00"))
        .unwrap();
    let s = build(&store, &settings, Range::Weeks4);
    assert_eq!((s.reading.totals.read, s.reading.totals.seen), (0, 1));
    assert_eq!((s.reading.streak, s.goal.done), (0, 0));
    // Opened: read.
    store
        .read_each(&k[1..3], true, utc("2026-10-29T10:00:00"))
        .unwrap();
    let s = build(&store, &settings, Range::Weeks4);
    assert_eq!(
        (s.reading.totals.read, s.goal.done, s.reading.streak),
        (2, 2, 1)
    );
    // "Don't count": back to seen.
    store.clear_opened(&k[2]).unwrap();
    let s = build(&store, &settings, Range::Weeks4);
    assert_eq!((s.reading.totals.read, s.reading.totals.seen), (1, 3));

    let opened = |store: &Store| -> Vec<bool> {
        let mut rows = store.list(Section::Research, None, 10).unwrap();
        rows.sort_by(|a, b| a.item.key.cmp(&b.item.key));
        rows.iter().map(|r| r.opened).collect()
    };
    assert_eq!(
        opened(&store),
        vec![false, true, false],
        "the preview's Read ✓"
    );

    // Reset: everything before it is left out, and nothing is deleted.
    stats::reset(&store, utc("2026-10-30T08:00:00")).unwrap();
    let s = build(&store, &settings, Range::Weeks4);
    let t = &s.reading.totals;
    assert_eq!(
        (t.read, t.seen, s.goal.done, s.reading.streak),
        (0, 0, 0, 0)
    );
    assert!(
        s.topics.iter().all(|w| w.fetched == 0),
        "arrivals counted from the reset too"
    );
    assert_eq!(
        s.backlog.last().unwrap().research,
        0,
        "the backlog is a state: still everything waiting minus what's marked read"
    );
    assert!(
        store.list(Section::Research, None, 10).unwrap()[0..3]
            .iter()
            .filter(|r| r.read)
            .count()
            == 3
    );
    assert_eq!(
        opened(&store),
        vec![false; 3],
        "opens before the reset aren't shown as read"
    );
    store
        .read_each(&k[0..1], true, utc("2026-10-30T09:00:00"))
        .unwrap();
    let s = build(&store, &settings, Range::Weeks4);
    assert_eq!((s.reading.totals.read, s.reading.streak), (1, 1));
    assert_eq!(s.since, Some(date("2026-10-30")));
    assert_eq!(
        ReadingLog::build(&store, now()).unwrap().days.len(),
        1,
        "the Habit Tracker's log starts fresh too"
    );
}

#[test]
fn the_reading_log_for_the_habit_tracker_counts_papers_per_local_day() {
    let papers: Vec<Item> = (1..=4)
        .map(|i| paper(&format!("2610.9000{i}"), &["quant-ph"], None, &[]))
        .collect();
    let n = news(1, QUIET);
    let mut store = store_with(
        &[papers.clone(), vec![n.clone()]].concat(),
        utc("2026-10-20T13:30:00"),
    );
    let k = keys(&papers);
    store
        .read_each(&k[0..2], true, utc("2026-10-30T00:30:00"))
        .unwrap(); // 29th in UTC
    store
        .read_each(&k[2..3], true, utc("2026-10-29T23:30:00"))
        .unwrap();
    store
        .set_read(&n.key, true, utc("2026-10-30T09:00:00"))
        .unwrap(); // news: not counted
    // Older than 400 days: left out.
    store
        .set_read(&k[3], true, utc("2025-09-01T09:00:00"))
        .unwrap();

    // Saved and noted that day too (version 2's counts).
    store
        .set_note(&k[0], "Nice.", utc("2026-10-30T02:00:00"))
        .unwrap();

    let log = ReadingLog::build(&store, now()).unwrap();
    assert_eq!(log.version, 2);
    assert_eq!(log.updated, "2026-10-30T15:00:00+08:00");
    let days: Vec<(&str, i64)> = log.days.iter().map(|(d, n)| (d.as_str(), *n)).collect();
    assert_eq!(days, vec![("2026-10-29", 1), ("2026-10-30", 2)]);
    let day = log.counts["2026-10-30"];
    assert_eq!(
        (day.read, day.saved, day.noted),
        (2, 1, 1),
        "a note saves its paper"
    );
    assert_eq!(log.counts["2026-10-29"].read, 1);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(dispatch::stats::READING_LOG_FILE);
    log.write(&path).unwrap();
    let back: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back["days"]["2026-10-30"], 2);
    assert_eq!(back["version"], 2);
    assert_eq!(back["counts"]["2026-10-30"]["noted"], 1);
}

#[test]
fn bulk_mark_all_read_is_not_reading() {
    let papers: Vec<Item> = (1..=6)
        .map(|i| paper(&format!("2610.1000{i}"), &["quant-ph"], None, &[]))
        .collect();
    let mut store = store_with(&papers, utc("2026-10-28T13:30:00"));
    let k = keys(&papers);
    // One read, five swept away with "Mark all read".
    store
        .read_each(&k[0..1], true, utc("2026-10-30T09:00:00"))
        .unwrap();
    let swept = store
        .mark_all_read(Section::Research, None, utc("2026-10-30T09:05:00"))
        .unwrap();
    assert_eq!(swept.len(), 5);
    let s = build(&store, &Settings::default(), Range::Weeks4);
    assert_eq!(
        (s.reading.totals.read, s.reading.totals.seen),
        (1, 1),
        "only the paper read"
    );
    assert_eq!(week_progress(&store, now()).unwrap().1, 1);
    assert_eq!(
        s.backlog.last().unwrap().research,
        0,
        "but nothing is waiting"
    );
    // Opening one of them later counts it.
    store.set_opened(&k[1], utc("2026-10-30T10:00:00")).unwrap();
    let s = build(&store, &Settings::default(), Range::Weeks4);
    assert_eq!(s.reading.totals.read, 2);
}

#[test]
fn a_version_6_database_gets_unicode_author_names_and_finds_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.sqlite");
    let p = paper("2610.80002", &["quant-ph"], None, &["Ada Lovelace"]);
    {
        let mut s = Store::open(&path).unwrap();
        s.upsert(std::slice::from_ref(&p), utc("2026-10-20T13:30:00"))
            .unwrap();
    }
    // As arXiv sent names before Dispatch decoded accents.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE items SET authors = ?1",
        [r#"["C. M\\\"ostl","Micha{\\l} Studzi\\'nski"]"#],
    )
    .unwrap();
    conn.execute_batch("PRAGMA user_version = 6;").unwrap();
    drop(conn);
    let s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version(), 7);
    let row = s.get(&p.key).unwrap().unwrap();
    assert_eq!(row.authors, ["C. Möstl", "Michał Studziński"]);
    let hits = s
        .search("au:Möstl", &dispatch::store::SearchFilter::default(), 10)
        .unwrap()
        .hits;
    assert_eq!(hits.len(), 1);
}
