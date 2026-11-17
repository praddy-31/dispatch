//! A refresh end to end, with saved feed files instead of the network.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use dispatch::fetch::Fetch;
use dispatch::settings::Settings;
use dispatch::sources::{arxiv, rss};
use dispatch::store::Store;
use dispatch::topics::{Category, Feed, NewsTopic, ResearchTopic, Topics};
use dispatch::{Section, refresh};

const ARXIV: &[u8] = include_bytes!("fixtures/arxiv_rss.xml");
const ARXIV_EMPTY: &[u8] = include_bytes!("fixtures/arxiv_empty.xml");
const SCIENCE: &[u8] = include_bytes!("fixtures/science.xml");
const FORUM: &[u8] = include_bytes!("fixtures/forum.xml");

/// Answers from a table; a URL missing from it fails like a dead feed.
struct Fake(Mutex<HashMap<String, Result<Vec<u8>, String>>>);

impl Fake {
    fn new(pairs: &[(&str, Result<&[u8], &str>)]) -> Fake {
        Fake(Mutex::new(
            pairs
                .iter()
                .map(|(u, r)| (u.to_string(), r.map(<[u8]>::to_vec).map_err(str::to_owned)))
                .collect(),
        ))
    }
    fn set(&self, url: &str, r: Result<&[u8], &str>) {
        self.0
            .lock()
            .unwrap()
            .insert(url.into(), r.map(<[u8]>::to_vec).map_err(str::to_owned));
    }
}

impl Fetch for Fake {
    /// `url|accept` in the table answers a request for that format.
    fn get_as(&self, url: &str, accept: &str) -> Result<Vec<u8>, String> {
        let keyed = format!("{url}|{accept}");
        if self.0.lock().unwrap().contains_key(&keyed) {
            return self.get(&keyed);
        }
        self.get(url)
    }

    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        self.0
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or_else(|| Err(format!("no such URL in the test: {url}")))
    }
}

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn research(id: &str, cats: &[&str], include: &[&str], exclude: &[&str]) -> ResearchTopic {
    ResearchTopic {
        id: id.into(),
        name: id.into(),
        enabled: true,
        categories: cats
            .iter()
            .map(|c| Category {
                id: (*c).into(),
                enabled: true,
            })
            .collect(),
        include: include.iter().map(|s| (*s).into()).collect(),
        exclude: exclude.iter().map(|s| (*s).into()).collect(),
        category_include: Default::default(),
    }
}

const SCIENCE_URL: &str = "https://science.example.org/feed/";
const FORUM_URL: &str = "https://forum.example.org/rss";

fn topics() -> Topics {
    Topics {
        version: 1,
        research: vec![
            research("quantum", &["quant-ph"], &[], &[]),
            research("optics", &["physics.optics"], &[], &["optical fibre"]),
            research(
                "adiabatic",
                &["quant-ph", "physics.optics"],
                &["adiabatic"],
                &[],
            ),
        ],
        news: vec![NewsTopic {
            id: "science".into(),
            name: "Science".into(),
            enabled: true,
            sources: vec![
                Feed {
                    name: "Science Weekly".into(),
                    url: SCIENCE_URL.into(),
                    enabled: true,
                },
                Feed {
                    name: "Example Forum".into(),
                    url: FORUM_URL.into(),
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

fn arxiv_url() -> String {
    arxiv::feed_url(&["quant-ph".into(), "physics.optics".into()])
}

#[test]
fn arxiv_feed_parses_with_authors_versions_and_announce_types() {
    let items = arxiv::parse(ARXIV).unwrap();
    assert_eq!(items.len(), 3, "the repeated cross-listing comes back once");
    let emitter = &items[0];
    assert_eq!(emitter.key, "arxiv:2610.91665");
    assert_eq!(emitter.authors.len(), 7);
    assert_eq!(emitter.authors[1], "Nico Weißler");
    assert_eq!(emitter.announce_type.as_deref(), Some("new"));
    assert_eq!(emitter.version, Some(1));
    assert!(emitter.summary.starts_with("We follow an ensemble of $N$"));
    assert_eq!(emitter.categories, ["quant-ph", "physics.optics"]);
    assert_eq!(
        emitter.pdf.as_deref(),
        Some("https://arxiv.org/pdf/2610.91665")
    );

    let replaced = &items[2];
    assert_eq!(replaced.version, Some(2));
    assert_eq!(replaced.announce_type.as_deref(), Some("replace"));
    assert_eq!(replaced.doi.as_deref(), Some("10.5555/demo.2026.012345"));
    assert_eq!(
        replaced.journal_ref.as_deref(),
        Some("J. Demo Phys. 112, 012345 (2026)")
    );
    assert!(
        replaced.title.contains(r"$\mathcal{O}(1)$"),
        "LaTeX kept for KaTeX"
    );
}

#[test]
fn an_empty_arxiv_feed_is_a_success_with_no_items() {
    assert!(arxiv::parse(ARXIV_EMPTY).unwrap().is_empty());
}

#[test]
fn news_feeds_parse_with_discussion_links() {
    let q = rss::parse(SCIENCE, SCIENCE_URL, "Science Weekly").unwrap();
    let h = rss::parse(FORUM, FORUM_URL, "Example Forum").unwrap();
    assert!(!q.is_empty() && !h.is_empty());
    for it in q.iter().chain(&h) {
        assert!(!it.title.is_empty(), "{it:?}");
        assert!(it.link.starts_with("http"), "{it:?}");
        assert!(!it.summary.contains("<p>"), "HTML removed: {}", it.summary);
    }
    // RSS <comments> is kept as the story's discussion link, for any feed that has one.
    let marker = rss::DISCUSSION_MARKER;
    assert!(h.iter().all(|it| it.summary.contains(marker)));
    assert!(
        q[0].summary
            .contains(&format!("{marker} https://science.example.org/"))
    );
}

#[test]
fn refresh_stores_once_tags_topics_and_keeps_last_good_results() {
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Err("HTTP 503")),
    ]);

    let r = refresh::run(
        &mut store,
        &topics,
        &fake,
        at("2026-10-27T06:00:00Z"),
        refresh::Scope::All,
    );
    assert!(r.any_ok());
    let forum = r
        .sources
        .iter()
        .find(|s| s.name == "Example Forum")
        .unwrap();
    assert_eq!(forum.error.as_deref(), Some("HTTP 503"));
    assert_eq!(store.count(Section::Research).unwrap(), 3);

    // Topics: the cross-listed emitter paper is in Quantum and Optics but stored once;
    // Optics excludes "optical fibre"; the keyword topic only has the adiabatic paper.
    let all = store.list(Section::Research, None, 50).unwrap();
    assert_eq!(all.len(), 3);
    let emitter = all
        .iter()
        .find(|r| r.item.key == "arxiv:2610.91665")
        .unwrap();
    assert_eq!(emitter.topics, ["optics", "quantum"]);
    let optics: Vec<_> = store
        .list(Section::Research, Some("optics"), 50)
        .unwrap()
        .into_iter()
        .map(|r| r.item.key)
        .collect();
    assert!(
        !optics.contains(&"arxiv:2610.91660".to_string()),
        "excluded by keyword"
    );
    let adiabatic = store
        .list(Section::Research, Some("adiabatic"), 50)
        .unwrap();
    assert_eq!(adiabatic.len(), 1);
    assert_eq!(adiabatic[0].item.key, "arxiv:2609.01234");

    let science_count = store.count(Section::News).unwrap();
    assert!(science_count > 0);

    // Next day: arXiv is empty (weekend) and Science is down. Nothing is lost.
    fake.set(&arxiv_url(), Ok(ARXIV_EMPTY));
    fake.set(SCIENCE_URL, Err("timed out"));
    let r2 = refresh::run(
        &mut store,
        &topics,
        &fake,
        at("2026-10-28T06:00:00Z"),
        refresh::Scope::All,
    );
    assert_eq!(r2.new_items(), 0);
    assert_eq!(store.count(Section::Research).unwrap(), 3);
    assert_eq!(store.count(Section::News).unwrap(), science_count);

    let statuses = store.statuses().unwrap();
    let science = statuses
        .iter()
        .find(|s| s.name == "Science Weekly")
        .unwrap();
    assert_eq!(science.last_error.as_deref(), Some("timed out"));
    assert_eq!(
        science.last_ok,
        Some(at("2026-10-27T06:00:00Z")),
        "last good kept"
    );
    let ax = statuses.iter().find(|s| s.name == "arXiv").unwrap();
    assert_eq!(ax.last_error, None);
    assert_eq!(ax.last_count, Some(0));

    // The same papers again: no duplicates, first_seen unchanged.
    fake.set(&arxiv_url(), Ok(ARXIV));
    let r3 = refresh::run(
        &mut store,
        &topics,
        &fake,
        at("2026-10-29T06:00:00Z"),
        refresh::Scope::All,
    );
    assert_eq!(r3.sources[0].new, 0);
    let again = store.list(Section::Research, None, 50).unwrap();
    assert_eq!(again.len(), 3);
    assert!(
        again
            .iter()
            .all(|r| r.first_seen == at("2026-10-27T06:00:00Z"))
    );
}

#[test]
fn everything_failing_does_not_count_as_a_refresh() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[]);
    let r = refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-27T06:00:00Z"),
        refresh::Scope::All,
    );
    assert!(!r.any_ok());
    assert_eq!(store.time_meta(refresh::LAST_OK), None);
    assert_eq!(
        store.time_meta(refresh::LAST_ATTEMPT),
        Some(at("2026-10-27T06:00:00Z"))
    );
}

#[test]
fn switching_a_topic_off_hides_its_items_without_deleting_them() {
    let mut store = Store::in_memory().unwrap();
    let mut topics = topics();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    refresh::run(
        &mut store,
        &topics,
        &fake,
        at("2026-10-27T06:00:00Z"),
        refresh::Scope::All,
    );
    for t in &mut topics.research {
        t.enabled = t.id == "adiabatic";
    }
    store.retag(&topics).unwrap();
    assert_eq!(store.list(Section::Research, None, 50).unwrap().len(), 1);
    assert_eq!(store.count(Section::Research).unwrap(), 3);
}

#[test]
fn a_newer_version_updates_the_row() {
    let mut store = Store::in_memory().unwrap();
    let mut items = arxiv::parse(ARXIV).unwrap();
    store.upsert(&items, at("2026-10-27T06:00:00Z")).unwrap();
    items[0].version = Some(3);
    items[0].journal_ref = Some("Nature 1 (2026)".into());
    assert_eq!(
        store
            .upsert(&items[..1], at("2026-10-28T06:00:00Z"))
            .unwrap(),
        0
    );
    // An older version seen later (a late cross-listing) doesn't lower it.
    items[0].version = Some(1);
    items[0].journal_ref = None;
    store
        .upsert(&items[..1], at("2026-10-29T06:00:00Z"))
        .unwrap();
    store.retag(&topics()).unwrap();
    let row = store
        .list(Section::Research, None, 50)
        .unwrap()
        .into_iter()
        .find(|r| r.item.key == "arxiv:2610.91665")
        .unwrap();
    assert_eq!(row.item.version, Some(3));
    assert_eq!(row.item.journal_ref.as_deref(), Some("Nature 1 (2026)"));
}

#[test]
fn store_file_has_the_schema_version_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.sqlite");
    {
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version(), dispatch::store::SCHEMA_VERSION);
        s.set_meta("x", "1").unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.meta("x").unwrap().as_deref(), Some("1"));
}

#[test]
fn topics_file_is_seeded_once_and_a_broken_one_is_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("topics.json");
    let t = Topics::load_or_seed(&path).unwrap();
    assert_eq!(t, Topics::default());
    assert!(path.exists());
    std::fs::write(&path, "{ broken").unwrap();
    assert!(Topics::load_or_seed(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
}

#[test]
fn replacements_can_be_hidden_from_lists_and_counts() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-27T06:00:00Z");
    refresh::run(&mut store, &topics(), &fake, now, refresh::Scope::Research);
    let keys = |hide| -> Vec<String> {
        store
            .list_hiding(Section::Research, None, 50, None, hide)
            .unwrap()
            .into_iter()
            .map(|r| r.item.key)
            .collect()
    };
    assert!(keys(false).contains(&"arxiv:2609.01234".to_string()));
    let shown = keys(true);
    assert!(
        !shown.contains(&"arxiv:2609.01234".to_string()),
        "the replacement is hidden"
    );
    assert_eq!(shown.len(), 2, "new and cross-listed papers stay");

    let c = store.unread_counts_hiding(Section::Research, true).unwrap();
    assert_eq!(c[""], 2, "the counts match the list");
    assert!(
        !c.contains_key("adiabatic"),
        "its only paper was the replacement"
    );
    assert_eq!(store.unread_counts(Section::Research).unwrap()[""], 3);
}

#[test]
fn read_later_lasts_until_opened_or_the_next_day() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-27T06:00:00Z");
    refresh::run(&mut store, &topics(), &fake, now, refresh::Scope::Research);
    let (a, b) = (
        "arxiv:2610.91665".to_string(),
        "arxiv:2610.91660".to_string(),
    );
    let midnight = at("2026-10-26T16:00:00Z"); // 00:00 at UTC+8 on the 27th
    let queue = |store: &Store, since| -> Vec<String> {
        store
            .later(Section::Research, since)
            .unwrap()
            .into_iter()
            .map(|r| r.item.key)
            .collect()
    };

    store
        .set_later(std::slice::from_ref(&b), true, now)
        .unwrap();
    store
        .set_later(
            std::slice::from_ref(&a),
            true,
            now + chrono::Duration::minutes(1),
        )
        .unwrap();
    assert_eq!(
        queue(&store, midnight),
        vec![b.clone(), a.clone()],
        "in the order queued"
    );
    assert!(
        store
            .list(Section::Research, None, 50)
            .unwrap()
            .iter()
            .any(|r| r.later_at.is_some())
    );
    assert!(
        queue(&store, midnight)
            .iter()
            .all(|k| store.get(k).unwrap().is_some())
    );

    // Reading it (opening its page or PDF) takes it off; so does `l` again.
    store.set_opened(&a, now).unwrap();
    assert_eq!(queue(&store, midnight), vec![b.clone()]);
    store
        .set_later(std::slice::from_ref(&b), false, now)
        .unwrap();
    assert!(queue(&store, midnight).is_empty());

    // The next day the queue starts empty: yesterday's items are not shown.
    store
        .set_later(std::slice::from_ref(&b), true, now)
        .unwrap();
    let next_midnight = midnight + chrono::Duration::days(1);
    assert!(queue(&store, next_midnight).is_empty());
    // It is not "saved": nothing in Saved, nothing synced.
    assert!(
        !store
            .list(Section::Research, None, 50)
            .unwrap()
            .iter()
            .any(|r| r.saved)
    );
}

#[test]
fn read_state_and_unread_counts_count_each_item_once() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-27T06:00:00Z");
    refresh::run(&mut store, &topics(), &fake, now, refresh::Scope::Research);
    let c = store.unread_counts(Section::Research).unwrap();
    assert_eq!(
        c[""], 3,
        "the emitter paper is in two topics but counts once"
    );
    assert_eq!(c["quantum"], 3, "all three carry quant-ph");
    assert_eq!(c["optics"], 2, "Dicke and the adiabatic paper");

    store.set_read("arxiv:2610.91665", true, now).unwrap();
    let c = store.unread_counts(Section::Research).unwrap();
    assert_eq!((c[""], c["quantum"], c["optics"]), (2, 2, 1));
    assert!(
        store
            .list(Section::Research, None, 10)
            .unwrap()
            .iter()
            .any(|r| r.item.key == "arxiv:2610.91665" && r.read)
    );

    let undo = store
        .mark_all_read(Section::Research, Some("adiabatic"), now)
        .unwrap();
    assert_eq!(
        undo,
        ["arxiv:2609.01234"],
        "returns what it changed, for Undo"
    );
    assert_eq!(store.unread_counts(Section::Research).unwrap()[""], 1);
    store.set_read("arxiv:2610.91665", false, now).unwrap();
    assert_eq!(store.unread_counts(Section::Research).unwrap()[""], 2);

    // A refresh that sees the paper again keeps it read.
    store.set_read("arxiv:2610.91665", true, now).unwrap();
    refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-28T06:00:00Z"),
        refresh::Scope::Research,
    );
    assert_eq!(store.unread_counts(Section::Research).unwrap()[""], 1);
}

#[test]
fn news_and_arxiv_keep_separate_clocks() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Ok(FORUM)),
    ]);
    // Nothing fetched yet: both due.
    assert_eq!(
        refresh::due_scope(&store, &Settings::default(), at("2026-10-27T06:00:00Z")),
        Some(refresh::Scope::All)
    );
    refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-27T06:00:00Z"),
        refresh::Scope::All,
    );
    assert_eq!(
        refresh::due_scope(&store, &Settings::default(), at("2026-10-27T08:00:00Z")),
        None
    );
    // Three hours later only news is due.
    assert_eq!(
        refresh::due_scope(&store, &Settings::default(), at("2026-10-27T09:00:00Z")),
        Some(refresh::Scope::News)
    );
    let r = refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-27T09:00:00Z"),
        refresh::Scope::News,
    );
    assert!(r.sources.iter().all(|s| s.section == Section::News));
    assert_eq!(
        refresh::last_ok(&store, Section::Research),
        Some(at("2026-10-27T06:00:00Z")),
        "a news run doesn't move arXiv's clock"
    );
    // Next day after 05:30 UTC: arXiv due (news too, it's been hours).
    assert_eq!(
        refresh::due_scope(&store, &Settings::default(), at("2026-10-28T05:31:00Z")),
        Some(refresh::Scope::All)
    );
}

#[test]
fn test_feed_reports_count_and_first_title_or_the_error() {
    let fake = Fake::new(&[
        (SCIENCE_URL, Ok(SCIENCE)),
        ("https://example.org/page", Ok(b"<html>not a feed</html>")),
    ]);
    let (n, first) = refresh::test_feed(&fake, SCIENCE_URL).unwrap();
    assert!(n > 0 && first.is_some());
    assert!(
        refresh::test_feed(&fake, "https://example.org/page")
            .unwrap_err()
            .contains("not a feed")
    );
    assert!(refresh::test_feed(&fake, "ftp://x").is_err());
    assert!(refresh::test_feed(&fake, "https://gone.example/").is_err());
}

const API: &[u8] = include_bytes!("fixtures/arxiv_api.xml");
const API_DOI: &[u8] = include_bytes!("fixtures/arxiv_api_doi.xml");

#[test]
fn arxiv_api_entries_parse_with_doi_and_journal() {
    let items = dispatch::sources::arxiv_api::parse(API).unwrap();
    assert_eq!(items.len(), 3);
    assert!(
        items
            .iter()
            .all(|i| i.key.starts_with("arxiv:2610.") && !i.authors.is_empty())
    );
    assert_eq!(items[0].authors.len(), 7);
    let collab = &dispatch::sources::arxiv_api::parse(API_DOI).unwrap()[0];
    assert_eq!(collab.key, "arxiv:1602.90837");
    assert_eq!(collab.doi.as_deref(), Some("10.5555/demo.2016.061102"));
    assert_eq!(
        collab.journal_ref.as_deref(),
        Some("J. Demo Phys. Lett. 116, 061102 (2016)")
    );
    assert!(collab.categories.contains(&"gr-qc".to_string()));
}

#[test]
fn backfill_fills_new_categories_once_and_retries_failures() {
    use dispatch::sources::arxiv_api::recent_url;
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    assert_eq!(
        refresh::needs_backfill(&store, &topics),
        ["quant-ph", "physics.optics"]
    );
    let fake = Fake::new(&[
        (
            &recent_url("quant-ph", refresh::BACKFILL_PER_CATEGORY),
            Ok(API),
        ),
        (
            &recent_url("physics.optics", refresh::BACKFILL_PER_CATEGORY),
            Err("timed out"),
        ),
    ]);
    let pauses = std::cell::Cell::new(0);
    let cats = refresh::needs_backfill(&store, &topics);
    let r = refresh::backfill(
        &mut store,
        &topics,
        &cats,
        &fake,
        at("2026-10-30T01:00:00Z"),
        &|| pauses.set(pauses.get() + 1),
    );
    assert_eq!((r.fetched, r.new), (3, 3));
    assert_eq!(
        r.failed,
        [("physics.optics".to_string(), "timed out".to_string())]
    );
    assert_eq!(
        pauses.get(),
        2,
        "one between categories, one before the retry of the failure"
    );
    assert_eq!(
        store
            .list(Section::Research, Some("quantum"), 50)
            .unwrap()
            .len(),
        3,
        "tagged"
    );
    // Only the failed category is still owed.
    assert_eq!(refresh::needs_backfill(&store, &topics), ["physics.optics"]);
}

#[test]
fn citations_come_from_doi_then_arxiv_then_dispatch_and_are_cached() {
    use dispatch::cite::{self, Format};
    let mut store = Store::in_memory().unwrap();
    let items = arxiv::parse(ARXIV).unwrap();
    store.upsert(&items, at("2026-10-27T06:00:00Z")).unwrap();
    let now = at("2026-10-30T01:00:00Z");
    let emitter = &items[0];
    let with_doi = &items[2];
    let fake = Fake::new(&[
        (
            "https://arxiv.org/bibtex/2610.91665",
            Ok(b"@misc{okafor2026closed,\n title={Dicke}\n}".as_slice()),
        ),
        (
            "https://doi.org/10.5555/demo.2026.012345|application/x-bibtex",
            Ok(b" @article{Lovelace_2026, title={A}}".as_slice()),
        ),
        (
            "https://doi.org/10.5555/demo.2026.012345|application/x-research-info-systems",
            Ok(b"TY  - JOUR\nTI  - A\nER  - ".as_slice()),
        ),
    ]);

    let c = cite::citation(&store, emitter, Format::Bibtex, Some(&fake), now);
    assert_eq!(c.origin, "arXiv");
    assert!(c.text.starts_with("@misc{okafor2026closed"));
    let c = cite::citation(&store, with_doi, Format::Bibtex, Some(&fake), now);
    assert_eq!(
        (c.origin.as_str(), c.text.as_str()),
        ("doi.org", "@article{Lovelace_2026, title={A}}"),
        "trimmed"
    );
    assert_eq!(
        cite::citation(&store, with_doi, Format::Ris, Some(&fake), now).origin,
        "doi.org"
    );
    // arXiv RIS isn't offered by arXiv: written locally.
    assert_eq!(
        cite::citation(&store, emitter, Format::Ris, Some(&fake), now).origin,
        "Dispatch"
    );

    // Cached: answers with no network at all.
    let offline = Fake::new(&[]);
    assert_eq!(
        cite::citation(&store, emitter, Format::Bibtex, Some(&offline), now).origin,
        "arXiv"
    );
    // Never fetched + offline: written locally, says so, and isn't cached.
    let other = &items[1];
    let c = cite::citation(&store, other, Format::Bibtex, Some(&offline), now);
    assert_eq!(c.origin, "Dispatch (offline)");
    assert!(c.text.starts_with("@misc{doe2026telling,"), "{}", c.text);
    assert!(store.citation(&other.key, "bibtex").is_none());

    // Several at once: one text, origins counted.
    let (text, origins) = cite::citations(&store, &items, Format::Bibtex, Some(&offline), now);
    assert_eq!(text.matches('@').count(), 3);
    assert!(
        origins.contains(&("arXiv".to_string(), 1))
            && origins.contains(&("doi.org".to_string(), 1))
    );
}

#[test]
fn a_version_1_database_is_upgraded_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dispatch.sqlite");
    {
        let mut s = Store::open(&path).unwrap();
        let items = arxiv::parse(ARXIV).unwrap();
        s.upsert(&items, at("2026-10-27T06:00:00Z")).unwrap();
    }
    // Make it look like a file from the first release.
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TRIGGER search_item_new; DROP TRIGGER search_item_changed;
         DROP TRIGGER search_item_gone; DROP TRIGGER search_mine_new;
         DROP TRIGGER search_mine_changed; DROP TABLE search; DROP TABLE search_ids;
         DROP TABLE citations; DROP TABLE user_items; ALTER TABLE items DROP COLUMN read_stamp;
         PRAGMA user_version = 1;",
    )
    .unwrap();
    drop(conn);
    let s = Store::open(&path).unwrap();
    assert_eq!(
        s.schema_version(),
        dispatch::store::SCHEMA_VERSION,
        "1 → 2 → 3 → 4 in one go"
    );
    assert_eq!(
        s.indexed().unwrap(),
        3,
        "existing items are put in the search index"
    );
    assert!(s.list_saved(None, 10).unwrap().is_empty());
    assert_eq!(s.count(Section::Research).unwrap(), 3, "items kept");
    s.set_citation(
        "arxiv:2610.91665",
        "bibtex",
        "@misc{x}",
        "arXiv",
        at("2026-10-30T00:00:00Z"),
    )
    .unwrap();
}

#[test]
fn backfill_stops_and_waits_an_hour_when_arxiv_says_too_many_requests() {
    use dispatch::sources::arxiv_api::recent_url;
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[(
        &recent_url("quant-ph", refresh::BACKFILL_PER_CATEGORY),
        Err("HTTP 429"),
    )]);
    let now = at("2026-10-30T01:00:00Z");
    let cats = refresh::needs_backfill_at(&store, &topics, now);
    let calls = std::cell::Cell::new(0);
    let r = refresh::backfill(&mut store, &topics, &cats, &fake, now, &|| {
        calls.set(calls.get() + 1)
    });
    assert_eq!(
        calls.get(),
        0,
        "no retry and no second category after a 429"
    );
    assert_eq!(r.failed.len(), 2);
    assert!(refresh::needs_backfill_at(&store, &topics, at("2026-10-30T01:30:00Z")).is_empty());
    assert_eq!(
        refresh::needs_backfill_at(&store, &topics, at("2026-10-30T02:01:00Z")).len(),
        2
    );
}

#[test]
fn intervals_follow_the_settings() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Ok(FORUM)),
    ]);
    refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-27T06:00:00Z"),
        refresh::Scope::All,
    );
    let hourly = Settings {
        news_every_hours: 1,
        arxiv_every_days: 2,
        ..Settings::default()
    };
    assert_eq!(
        refresh::due_scope(&store, &hourly, at("2026-10-27T07:00:00Z")),
        Some(refresh::Scope::News)
    );
    // Next day after the slot: arXiv not due yet (every 2 days); news is.
    assert_eq!(
        refresh::due_scope(&store, &hourly, at("2026-10-28T05:31:00Z")),
        Some(refresh::Scope::News)
    );
    assert_eq!(
        refresh::next_due(
            &store,
            &hourly,
            Section::Research,
            at("2026-10-27T07:00:00Z")
        ),
        at("2026-10-29T05:30:00Z")
    );
    assert_eq!(
        refresh::next_due(&store, &hourly, Section::News, at("2026-10-27T06:30:00Z")),
        at("2026-10-27T07:00:00Z")
    );
}

#[test]
fn saved_notes_and_tags_show_in_lists_and_the_saved_view() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-30T01:00:00Z");
    refresh::run(&mut store, &topics(), &fake, now, refresh::Scope::Research);
    let keys = vec![
        "arxiv:2610.91665".to_string(),
        "arxiv:2609.01234".to_string(),
    ];
    store.set_saved(&keys, true, now).unwrap();
    store
        .set_note("arxiv:2610.91665", "Read section 3  ", now)
        .unwrap();
    store
        .set_tags(
            "arxiv:2610.91665",
            &[" Thesis".into(), "#emitter".into(), "thesis".into()],
            now,
        )
        .unwrap();
    store.add_tag(&keys, "to-read", now).unwrap();

    let saved = store.list_saved(None, 50).unwrap();
    assert_eq!(saved.len(), 2);
    let emitter = saved.iter().find(|r| r.item.key == keys[0]).unwrap();
    assert_eq!(emitter.note, "Read section 3");
    assert_eq!(
        emitter.tags,
        ["thesis", "emitter", "to-read"],
        "cleaned, no duplicates"
    );
    assert_eq!(store.list_saved(Some("thesis"), 50).unwrap().len(), 1);
    assert_eq!(store.tags().unwrap()[0], ("to-read".to_string(), 2));
    // Lists carry it too.
    let row = store
        .list(Section::Research, None, 50)
        .unwrap()
        .into_iter()
        .find(|r| r.item.key == keys[0])
        .unwrap();
    assert!(row.saved);
    // Saved items stay visible even when their topics are switched off.
    let mut off = topics();
    for t in &mut off.research {
        t.enabled = false;
    }
    store.retag(&off).unwrap();
    assert_eq!(store.list_saved(None, 50).unwrap().len(), 2);

    store
        .set_saved(&keys[..1], false, at("2026-10-30T02:00:00Z"))
        .unwrap();
    assert_eq!(store.list_saved(None, 50).unwrap().len(), 1);
    assert_eq!(
        store.user(&keys[0]).unwrap().unwrap().note,
        "Read section 3",
        "note kept when unsaved"
    );
}

#[test]
fn two_macs_sync_through_one_folder() {
    use dispatch::sync;
    let cloud = tempfile::tempdir().unwrap();
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (topics_a, topics_b) = (
        dir_a.path().join("topics.json"),
        dir_b.path().join("topics.json"),
    );
    let ta = topics();
    ta.save(&topics_a).unwrap();
    let mut tb = topics();
    tb.research.pop(); // B starts with different topics, older
    tb.save(&topics_b).unwrap();

    let mut a = Store::in_memory().unwrap();
    let mut b = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    refresh::run(
        &mut a,
        &ta,
        &fake,
        at("2026-10-30T01:00:00Z"),
        refresh::Scope::Research,
    );
    // B never fetched arXiv.
    let key = "arxiv:2610.91665".to_string();
    a.set_saved(std::slice::from_ref(&key), true, at("2026-10-30T02:00:00Z"))
        .unwrap();
    a.set_note(&key, "from A", at("2026-10-30T02:00:00Z"))
        .unwrap();
    a.set_read(&key, true, at("2026-10-30T02:00:00Z")).unwrap();
    a.set_meta(
        sync::TOPICS_STAMP,
        &at("2026-10-30T02:00:00Z").timestamp_millis().to_string(),
    )
    .unwrap();

    let r = sync::run(
        &mut a,
        &ta,
        &topics_a,
        cloud.path(),
        dir_a.path(),
        at("2026-10-30T02:01:00Z"),
    )
    .unwrap();
    assert_eq!(r.pulled, 0);
    assert!(cloud.path().join(sync::FILE).exists());

    // B picks it all up, including the paper it never fetched and A's topics (newer).
    let r = sync::run(
        &mut b,
        &tb,
        &topics_b,
        cloud.path(),
        dir_b.path(),
        at("2026-10-30T02:02:00Z"),
    )
    .unwrap();
    assert!(r.topics_updated);
    assert_eq!(Topics::load_or_seed(&topics_b).unwrap(), ta);
    let saved = b.list_saved(None, 10).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].note, "from A");
    assert!(saved[0].read, "read state came too");

    // Both change the same paper before syncing again: B the note (later), A the tags.
    a.set_tags(&key, &["thesis".into()], at("2026-10-30T03:00:00Z"))
        .unwrap();
    b.set_note(&key, "from B", at("2026-10-30T03:05:00Z"))
        .unwrap();
    sync::run(
        &mut a,
        &ta,
        &topics_a,
        cloud.path(),
        dir_a.path(),
        at("2026-10-30T03:10:00Z"),
    )
    .unwrap();
    sync::run(
        &mut b,
        &ta,
        &topics_b,
        cloud.path(),
        dir_b.path(),
        at("2026-10-30T03:11:00Z"),
    )
    .unwrap();
    // Field by field: B's note and A's tags both survive.
    let ub = b.user(&key).unwrap().unwrap();
    assert_eq!(
        (ub.note.as_str(), ub.tags.clone()),
        ("from B", vec!["thesis".to_string()])
    );
    sync::run(
        &mut a,
        &ta,
        &topics_a,
        cloud.path(),
        dir_a.path(),
        at("2026-10-30T03:12:00Z"),
    )
    .unwrap();
    let ua = a.user(&key).unwrap().unwrap();
    assert_eq!(
        (ua.note.as_str(), ua.tags.clone()),
        ("from B", vec!["thesis".to_string()]),
        "A catches up"
    );
}

#[test]
fn icloud_conflict_copies_are_merged_and_moved_aside() {
    use dispatch::sync;
    let cloud = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let topics_path = local.path().join("topics.json");
    topics().save(&topics_path).unwrap();
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-30T01:00:00Z"),
        refresh::Scope::Research,
    );
    // A conflict copy from another Mac that saved a paper.
    let mut other = sync::SyncFile {
        version: 1,
        ..Default::default()
    };
    other.items.insert(
        "arxiv:2609.01234".into(),
        sync::Entry {
            user: Some(dispatch::store::UserData {
                saved_at: Some(1),
                saved_stamp: 99,
                note: "conflict".into(),
                note_stamp: 99,
                ..Default::default()
            }),
            read: None,
            item: None,
        },
    );
    std::fs::write(
        cloud.path().join("dispatch-sync 2.json"),
        serde_json::to_string(&other).unwrap(),
    )
    .unwrap();
    let r = sync::run(
        &mut store,
        &topics(),
        &topics_path,
        cloud.path(),
        local.path(),
        at("2026-10-30T02:00:00Z"),
    )
    .unwrap();
    assert_eq!(r.conflicts, 1);
    assert_eq!(
        store.user("arxiv:2609.01234").unwrap().unwrap().note,
        "conflict"
    );
    assert!(
        !cloud.path().join("dispatch-sync 2.json").exists(),
        "moved out of iCloud"
    );
    assert_eq!(
        std::fs::read_dir(local.path().join("sync-conflicts"))
            .unwrap()
            .count(),
        1,
        "kept locally"
    );
}

/// The keys a search finds, best match first.
fn keys(store: &Store, q: &str, f: &dispatch::store::SearchFilter) -> Vec<String> {
    store
        .search(q, f, 50)
        .unwrap()
        .hits
        .into_iter()
        .map(|h| h.row.item.key)
        .collect()
}

#[test]
fn search_finds_everything_ever_fetched_and_stays_in_step() {
    use dispatch::search::{MARK_END, MARK_START, Sort};
    use dispatch::store::SearchFilter;
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Ok(FORUM)),
    ]);
    let now = at("2026-10-30T01:00:00Z");
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::All);
    let all = SearchFilter::default();
    let emitter = "arxiv:2610.91665".to_string();

    // Title words as prefixes, authors, and the arXiv id itself.
    assert_eq!(keys(&store, "emitt", &all), vec![emitter.clone()]);
    assert_eq!(keys(&store, "au:smith", &all).len(), 1);
    assert_eq!(keys(&store, "2610.91665", &all), vec![emitter.clone()]);
    assert!(keys(&store, "emitter -ensemble", &all).is_empty());
    assert!(keys(&store, "", &all).is_empty() && keys(&store, "$$", &all).is_empty());

    // The snippet comes from the abstract, with the match marked.
    let r = store.search("pump", &all, 50).unwrap();
    assert_eq!(r.total, 1);
    assert!(r.hits[0].snippet.contains(MARK_START) && r.hits[0].snippet.contains(MARK_END));

    // One section, or both.
    let news = SearchFilter {
        section: Some(Section::News),
        ..SearchFilter::default()
    };
    assert!(!keys(&store, "snowflakes", &news).is_empty());
    let research = SearchFilter {
        section: Some(Section::Research),
        ..SearchFilter::default()
    };
    assert!(keys(&store, "snowflakes", &research).is_empty());

    // The user's notes and tags are searchable (accents ignored) and follow edits.
    store
        .set_note(&emitter, "Compare with Schrödinger cats", now)
        .unwrap();
    assert_eq!(
        keys(&store, "note:schrodinger", &all),
        vec![emitter.clone()]
    );
    store
        .set_note(&emitter, "Not relevant after all", now)
        .unwrap();
    assert!(keys(&store, "schrodinger", &all).is_empty());
    store.set_tags(&emitter, &["thesis".into()], now).unwrap();
    assert_eq!(keys(&store, "tag:thesis", &all), vec![emitter.clone()]);

    // Only saved items.
    let saved = SearchFilter {
        saved_only: true,
        ..SearchFilter::default()
    };
    assert!(keys(&store, "quantum", &saved).is_empty());
    store
        .set_saved(std::slice::from_ref(&emitter), true, now)
        .unwrap();
    assert_eq!(keys(&store, "pumped", &saved), vec![emitter.clone()]);

    // One topic, unread only, since a date.
    let topic = store
        .list(Section::Research, None, 500)
        .unwrap()
        .into_iter()
        .find(|r| r.item.key == emitter)
        .unwrap()
        .topics[0]
        .clone();
    let in_topic = SearchFilter {
        topic: Some(topic),
        ..SearchFilter::default()
    };
    assert_eq!(keys(&store, "pumped", &in_topic), vec![emitter.clone()]);
    let elsewhere = SearchFilter {
        topic: Some("no-such-topic".into()),
        ..SearchFilter::default()
    };
    assert!(keys(&store, "pumped", &elsewhere).is_empty());
    let unread = SearchFilter {
        unread_only: true,
        ..SearchFilter::default()
    };
    assert_eq!(keys(&store, "pumped", &unread), vec![emitter.clone()]);
    store.set_read(&emitter, true, now).unwrap();
    assert!(keys(&store, "pumped", &unread).is_empty());
    store.set_read(&emitter, false, now).unwrap();
    let future = SearchFilter {
        since: Some(now.timestamp() + 86_400 * 365),
        ..SearchFilter::default()
    };
    assert!(keys(&store, "pumped", &future).is_empty());
    let past = SearchFilter {
        since: Some(0),
        ..SearchFilter::default()
    };
    assert_eq!(keys(&store, "pumped", &past), vec![emitter.clone()]);
    assert_eq!(store.search("pumped", &unread, 50).unwrap().total, 1);

    // A topic switched off hides its papers from the lists, not from the archive.
    let mut off = topics.clone();
    for t in &mut off.research {
        t.enabled = false;
    }
    store.retag(&off).unwrap();
    assert!(store.list(Section::Research, None, 50).unwrap().is_empty());
    assert_eq!(keys(&store, "ensemble", &all), vec![emitter.clone()]);

    // A new version with a new title is found by its new words only.
    let mut item = store.get(&emitter).unwrap().unwrap();
    item.title = "Collective pumping of two-level emitters".into();
    item.version = Some(2);
    store.upsert(&[item], now).unwrap();
    assert!(keys(&store, "ti:ensemble", &all).is_empty());
    assert_eq!(keys(&store, "ti:emitters", &all), vec![emitter.clone()]);

    // Newest first gives the same matches in date order.
    let newest = SearchFilter {
        sort: Sort::Newest,
        ..SearchFilter::default()
    };
    let mut a = keys(&store, "quantum OR optical", &all);
    let mut b = keys(&store, "quantum OR optical", &newest);
    assert!(!a.is_empty());
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert_eq!(
        store.indexed().unwrap(),
        store.count(Section::Research).unwrap() + store.count(Section::News).unwrap()
    );
}

#[test]
fn the_menu_bar_companion_offers_unread_new_papers_and_source_lights() {
    use dispatch::companion::Companion;
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Err("HTTP 503")),
    ]);
    let now = at("2026-10-30T06:00:00Z");
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::All);
    let settings = Settings::default();
    let c = Companion::build(&store, &topics, &settings, now).unwrap();
    let rows = store.list(Section::Research, None, 50).unwrap();
    let new_ones = rows
        .iter()
        .filter(|r| matches!(r.item.announce_type.as_deref(), None | Some("new")))
        .count();
    assert!(new_ones > 0);
    assert_eq!(c.top.len(), new_ones.min(dispatch::companion::TOP));
    assert_eq!(c.research_unread, 3);
    assert!(c.news_unread > 0);
    assert_eq!(c.failed, 1, "the forum failed");
    assert!(c.last_ok.is_some());
    // Each paper once, titles readable without KaTeX.
    let mut keys: Vec<_> = c.top.iter().map(|t| t.key.clone()).collect();
    keys.dedup();
    assert_eq!(keys.len(), c.top.len());
    assert!(c.top.iter().all(|t| !t.menu_title.contains('$')));

    // A paper marked read leaves the menu.
    let first = c.top[0].key.clone();
    store.set_read(&first, true, now).unwrap();
    let c = Companion::build(&store, &topics, &settings, now).unwrap();
    assert!(c.top.iter().all(|t| t.key != first));
    assert_eq!(c.research_unread, 2);
}

#[test]
fn followed_authors_keywords_and_saved_updates_alert_once() {
    use dispatch::alerts::{self, Why};
    use dispatch::follow::{self, Follow};
    let mut store = Store::in_memory().unwrap();
    let mut topics = topics();
    topics.follow = Follow {
        authors: vec!["H. Weller".into()],
        keywords: vec!["exceptional point".into()],
    };
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let on = Settings {
        notify_authors: true,
        notify_keywords: true,
        notify_updates: true,
        ..Settings::default()
    };
    let emitter = "arxiv:2610.91665".to_string();

    // First refresh: the emitter paper (Hanna J. Weller) and the exceptional-points paper.
    let t1 = at("2026-10-30T06:00:00Z");
    let before = alerts::snapshot(&store).unwrap();
    refresh::run(&mut store, &topics, &fake, t1, refresh::Scope::Research);
    let found = alerts::collect(&store, &topics.follow, &on, t1, &before).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    let by_weller = found.iter().find(|a| a.key == emitter).unwrap();
    assert_eq!(by_weller.why, Why::Author(vec!["H. Weller".into()]));
    assert!(
        found
            .iter()
            .any(|a| matches!(&a.why, Why::Keyword(k) if k == &["exceptional point"]))
    );
    let (title, _) = alerts::summary(&found).unwrap();
    assert_eq!(title, "2 new papers for you");
    // Switched off: nothing.
    assert!(
        alerts::collect(&store, &topics.follow, &Settings::default(), t1, &before)
            .unwrap()
            .is_empty()
    );

    // Lists mark the followed author's paper.
    let mut rows = store.list(Section::Research, None, 50).unwrap();
    follow::mark(&topics.follow, &mut rows);
    let row = rows.iter().find(|r| r.item.key == emitter).unwrap();
    assert_eq!(row.followed, vec!["H. Weller"]);

    // The same feed again: nothing new, so no alert.
    store
        .set_saved(std::slice::from_ref(&emitter), true, t1)
        .unwrap();
    let t2 = at("2026-10-31T06:00:00Z");
    let before = alerts::snapshot(&store).unwrap();
    refresh::run(&mut store, &topics, &fake, t2, refresh::Scope::Research);
    assert!(
        alerts::collect(&store, &topics.follow, &on, t2, &before)
            .unwrap()
            .is_empty()
    );

    // The saved paper gets a v2: one "updated" alert, not a "new" one.
    let t3 = at("2026-11-01T06:00:00Z");
    let before = alerts::snapshot(&store).unwrap();
    let mut item = store.get(&emitter).unwrap().unwrap();
    item.version = Some(item.version.unwrap_or(1) + 1);
    store.upsert(&[item], t3).unwrap();
    let found = alerts::collect(&store, &topics.follow, &on, t3, &before).unwrap();
    assert_eq!(found.len(), 1);
    assert!(matches!(&found[0].why, Why::Updated(v) if v.starts_with('v')));
    assert_eq!(alerts::summary(&found).unwrap().0, "Saved paper updated");
}

#[test]
fn an_author_card_and_a_polite_arxiv_author_search() {
    use dispatch::follow::{self, Follow};
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-30T06:00:00Z");
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::Research);
    let f = Follow {
        authors: vec!["Hanna J. Weller".into()],
        keywords: vec![],
    };

    let card = follow::card(&store, &f, "H. J. Weller").unwrap();
    assert!(card.followed);
    assert_eq!(card.papers.len(), 1);
    assert_eq!(card.papers[0].item.key, "arxiv:2610.91665");

    // The card's button: one arXiv API request, papers by this person stored.
    let url = follow::arxiv_url("H. J. Weller", follow::ARXIV_AUTHOR_MAX).unwrap();
    let api = Fake::new(&[(&url, Ok(API))]);
    let r = follow::search_arxiv(&mut store, &topics, "H. J. Weller", &api, now).unwrap();
    assert!(r.found >= 1, "{r:?}");
    assert!(
        !follow::card(&store, &f, "Weller")
            .unwrap()
            .papers
            .is_empty()
    );

    // arXiv says "too many requests": an error, then an hour without asking again.
    let busy = Fake::new(&[(&url, Err("HTTP 429"))]);
    assert_eq!(
        follow::search_arxiv(&mut store, &topics, "H. J. Weller", &busy, now).unwrap_err(),
        "HTTP 429"
    );
    let e = follow::search_arxiv(&mut store, &topics, "H. J. Weller", &api, now).unwrap_err();
    assert!(e.contains("slow down"), "{e}");
    assert!(
        refresh::needs_backfill_at(&store, &topics, now).is_empty(),
        "backfill waits too"
    );
}

#[test]
fn a_feed_given_a_new_address_drops_its_old_failure() {
    let mut store = Store::in_memory().unwrap();
    let mut topics = topics();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Ok(SCIENCE)),
        (FORUM_URL, Err("timed out")),
        ("https://forum.example.org/rss2", Ok(FORUM)),
    ]);
    let now = at("2026-10-30T06:00:00Z");
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::All);
    assert!(
        store
            .statuses()
            .unwrap()
            .iter()
            .any(|s| s.source == FORUM_URL && s.last_error.is_some())
    );

    // The user changes the forum's address: the old row goes at once (Save) and stays gone.
    topics.news[0].sources[1].url = "https://forum.example.org/rss2".into();
    refresh::forget_old_sources(&store, &topics).unwrap();
    assert!(
        store
            .statuses()
            .unwrap()
            .iter()
            .all(|s| s.source != FORUM_URL)
    );
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::All);
    let st = store.statuses().unwrap();
    assert!(st.iter().all(|s| s.last_error.is_none()), "{st:?}");
    assert_eq!(st.len(), 3, "arXiv, Science, the forum's new address");
    assert!(store.count(Section::News).unwrap() > 0, "items are kept");
}

#[test]
fn a_note_saves_old_read_papers_hide_and_the_archive_limit_keeps_the_users_items() {
    use dispatch::store::SearchFilter;
    let mut store = Store::in_memory().unwrap();
    let topics = topics();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV)), (SCIENCE_URL, Ok(SCIENCE))]);
    let t0 = at("2026-10-30T06:00:00Z");
    refresh::run(&mut store, &topics, &fake, t0, refresh::Scope::All);
    let research = store.list(Section::Research, None, 50).unwrap();
    let (a, b, c) = (
        research[0].item.key.clone(),
        research[1].item.key.clone(),
        research[2].item.key.clone(),
    );

    // A note saves the paper; removing it from Saved keeps the note.
    store.set_note(&a, "Check eq. 4", t0).unwrap();
    let u = store.user(&a).unwrap().unwrap();
    assert!(u.saved_at.is_some());
    store
        .set_saved(std::slice::from_ref(&a), false, t0)
        .unwrap();
    assert_eq!(store.user(&a).unwrap().unwrap().note, "Check eq. 4");
    assert_eq!(store.list_noted(10).unwrap().len(), 1);

    // Hide read papers older than a cutoff, but never saved ones.
    store
        .set_read_many(&[a.clone(), b.clone(), c.clone()], true, t0)
        .unwrap();
    store.set_saved(std::slice::from_ref(&b), true, t0).unwrap();
    let later = at("2027-01-01T00:00:00Z");
    let shown: Vec<String> = store
        .list_hiding(Section::Research, None, 50, Some(later), false)
        .unwrap()
        .into_iter()
        .map(|r| r.item.key)
        .collect();
    assert_eq!(shown, vec![b.clone()], "only the saved one stays");
    assert_eq!(
        store.list(Section::Research, None, 50).unwrap().len(),
        3,
        "off = all"
    );

    // The archive limit keeps saved, noted and tagged items; the rest go, index too.
    store.set_tags(&c, &["thesis".into()], t0).unwrap();
    let before = store.count(Section::Research).unwrap() + store.count(Section::News).unwrap();
    let gone = store.prune(later).unwrap();
    assert!(gone > 0);
    let left = store.count(Section::Research).unwrap() + store.count(Section::News).unwrap();
    assert_eq!(left as usize, before as usize - gone);
    assert_eq!(
        store.count(Section::Research).unwrap(),
        3,
        "noted, saved, tagged kept"
    );
    assert_eq!(store.count(Section::News).unwrap(), 0);
    assert_eq!(store.indexed().unwrap(), left);
    assert!(
        store
            .search("snowflakes", &SearchFilter::default(), 10)
            .unwrap()
            .hits
            .is_empty()
    );
    store.compact().unwrap();
}

#[test]
fn people_search_finds_authors_by_part_of_their_name() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    refresh::run(
        &mut store,
        &topics(),
        &fake,
        at("2026-10-30T06:00:00Z"),
        refresh::Scope::Research,
    );
    let found = dispatch::follow::find_authors(&store, "wel", 10).unwrap();
    assert_eq!(found, vec![("Hanna J. Weller".to_string(), 1)]);
    assert_eq!(
        dispatch::follow::find_authors(&store, "nico weiss", 10)
            .unwrap()
            .len(),
        1,
        "ß = ss"
    );
    assert!(
        dispatch::follow::find_authors(&store, "y", 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn the_menu_keeps_suggested_papers_once_everything_was_seen() {
    // A past bug: with every paper shown in the preview ("read"), the menu was empty even
    // though Suggested (not opened) had papers.
    use dispatch::companion::Companion;
    let mut store = Store::in_memory().unwrap();
    let mut topics = topics();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-30T06:00:00Z");
    refresh::run(&mut store, &topics, &fake, now, refresh::Scope::Research);
    let rows = store.list(Section::Research, None, 50).unwrap();
    let new = rows
        .iter()
        .find(|r| r.item.announce_type.as_deref() == Some("new"))
        .unwrap();
    // A watched word from a new paper's title, so Suggested has it.
    let word = new
        .item
        .title
        .split(|c: char| !c.is_alphanumeric())
        .rfind(|w| w.len() > 5)
        .unwrap()
        .to_lowercase();
    topics.follow.keywords = vec![word];
    let keys: Vec<String> = rows.iter().map(|r| r.item.key.clone()).collect();
    store.set_read_many(&keys, true, now).unwrap();
    let c = Companion::build(&store, &topics, &Settings::default(), now).unwrap();
    assert_eq!(c.research_unread, 0);
    assert!(c.top.iter().any(|t| t.key == new.item.key), "{:?}", c.top);
    assert!(c.top.iter().all(|t| t.read), "seen papers get no Mark read");
    // Opened papers leave it.
    store.set_opened(&new.item.key, now).unwrap();
    let c = Companion::build(&store, &topics, &Settings::default(), now).unwrap();
    assert!(c.top.iter().all(|t| t.key != new.item.key));
}

/// A muted feed's stories arrive read (dismissed), and its discussion links are read.
#[test]
fn muted_source_arrives_read_and_discussion_links_show() {
    use dispatch::news::{Mute, MuteKind};
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[(SCIENCE_URL, Ok(SCIENCE)), (FORUM_URL, Ok(FORUM))]);
    let mut t = topics();
    t.mute = vec![Mute {
        kind: MuteKind::Source,
        value: FORUM_URL.into(),
        label: "Example Forum".into(),
        until: None,
    }];
    refresh::run(
        &mut store,
        &t,
        &fake,
        at("2026-10-30T00:00:00Z"),
        refresh::Scope::News,
    );
    let rows = store
        .list_hiding(Section::News, None, 500, None, false)
        .unwrap();
    let forum: Vec<_> = rows.iter().filter(|r| r.item.source == FORUM_URL).collect();
    assert!(!forum.is_empty());
    assert!(forum.iter().all(|r| r.read), "muted stories arrive read");
    assert!(
        rows.iter()
            .filter(|r| r.item.source == SCIENCE_URL)
            .all(|r| !r.read)
    );
    let first = forum[0].discussion.as_ref().expect("discussion link");
    assert!(
        forum[0].item.summary.is_empty(),
        "a bare \"Comments\" link is no summary"
    );
    assert!(first.url.starts_with("https://forum.example.org/item?id="));
}

#[test]
fn retrying_one_failed_source_fetches_only_it_and_leaves_the_clocks() {
    let mut store = Store::in_memory().unwrap();
    let fake = Fake::new(&[
        (&arxiv_url(), Ok(ARXIV)),
        (SCIENCE_URL, Err("HTTP 503")),
        (FORUM_URL, Ok(FORUM)),
    ]);
    let first = at("2026-10-27T06:00:00Z");
    refresh::run(&mut store, &topics(), &fake, first, refresh::Scope::All);
    let science = |store: &Store| {
        store
            .statuses()
            .unwrap()
            .into_iter()
            .find(|s| s.source == SCIENCE_URL)
            .unwrap()
    };
    assert!(science(&store).last_error.is_some());

    // The feed is back; arXiv would now fail, but a retry of Science doesn't touch it.
    fake.set(SCIENCE_URL, Ok(SCIENCE));
    fake.set(&arxiv_url(), Err("HTTP 500"));
    let later = at("2026-10-27T07:00:00Z");
    let r = refresh::retry(&mut store, &topics(), &fake, later, SCIENCE_URL).unwrap();
    assert!(r.error.is_none() && r.new > 0);
    assert!(science(&store).last_error.is_none());
    let arxiv = store
        .statuses()
        .unwrap()
        .into_iter()
        .find(|s| s.source == arxiv::SOURCE_ID)
        .unwrap();
    assert!(arxiv.last_error.is_none(), "arXiv wasn't fetched again");
    assert_eq!(refresh::last_ok(&store, Section::News), Some(first));
    assert!(refresh::retry(&mut store, &topics(), &fake, later, "https://gone/rss").is_err());
}

#[test]
fn add_to_dispatch_saves_an_arxiv_paper_or_a_doi_with_one_request_each() {
    use dispatch::add;
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(&dir.path().join("t.sqlite")).unwrap();
    let t = topics();
    let now = at("2026-11-03T06:00:00Z");
    let csl = br#"{"title": "A journal paper", "author": [{"given": "B.", "family": "Jones"}],
        "issued": {"date-parts": [[2021]]}, "container-title": "Nature"}"#;
    let fetch = Fake::new(&[
        (
            &add::arxiv_url("2610.91660"),
            Ok(include_bytes!("fixtures/arxiv_api.xml")),
        ),
        (
            "https://doi.org/10.1038/nature12345|application/vnd.citationstyles.csl+json",
            Ok(csl),
        ),
    ]);

    let a = add::add(
        &mut store,
        &t,
        "https://arxiv.org/abs/2610.91660v1",
        &fetch,
        now,
    )
    .unwrap();
    assert_eq!((a.key.as_str(), a.new), ("arxiv:2610.91660", true));
    // The answer held other papers too: only the one asked for is stored.
    assert!(store.get("arxiv:2610.91665").unwrap().is_none());
    let d = add::add(&mut store, &t, "doi:10.1038/Nature12345", &fetch, now).unwrap();
    assert_eq!(
        (d.key.as_str(), d.title.as_str()),
        ("doi:10.1038/nature12345", "A journal paper")
    );
    let saved: Vec<String> = store
        .list_saved(None, 10)
        .unwrap()
        .into_iter()
        .map(|r| r.item.key)
        .collect();
    assert_eq!(saved.len(), 2);
    assert!(saved.contains(&"doi:10.1038/nature12345".to_string()));

    // A paper Dispatch already has: saved again, no request (the fake would fail it).
    let empty = Fake::new(&[]);
    let again = add::add(&mut store, &t, "2610.91660", &empty, now).unwrap();
    assert!(!again.new);
    assert!(add::add(&mut store, &t, "2610.99999", &empty, now).is_err());
}

#[test]
fn a_section_with_no_feeds_is_not_due_every_minute() {
    let mut store = Store::in_memory().unwrap();
    let mut t = topics();
    t.news.clear();
    let fake = Fake::new(&[(&arxiv_url(), Ok(ARXIV))]);
    let now = at("2026-10-27T06:00:00Z");
    refresh::run(&mut store, &t, &fake, now, refresh::Scope::All);
    let later = at("2026-10-27T06:01:00Z");
    assert_eq!(
        refresh::due_scope(&store, &Settings::default(), later),
        None
    );
}
