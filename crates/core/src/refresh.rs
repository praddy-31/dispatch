//! One refresh run: fetch every enabled source at the same time, store what arrived,
//! record each source's status, then retag items by topic.
//!
//! A source that fails keeps its old items and gets a red status; it never stops the others.
//! News and arXiv run on different clocks (`Scope`, `due_scope`).

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::fetch::Fetch;
use crate::item::{Item, Section};
use crate::schedule;
use crate::settings::Settings;
use crate::sources::{arxiv, arxiv_api, rss};
use crate::store::Store;
use crate::topics::Topics;

/// Last refresh where any source worked, whatever the section (shown in the tray).
pub const LAST_OK: &str = "last_refresh_ok";
pub const LAST_ATTEMPT: &str = "last_refresh_attempt";

/// Which sections a run covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Scope {
    All,
    Research,
    News,
}

impl Scope {
    fn covers(self, s: Section) -> bool {
        match self {
            Scope::All => true,
            Scope::Research => s == Section::Research,
            Scope::News => s == Section::News,
        }
    }

    fn join(a: bool, b: bool) -> Option<Scope> {
        match (a, b) {
            (true, true) => Some(Scope::All),
            (true, false) => Some(Scope::Research),
            (false, true) => Some(Scope::News),
            (false, false) => None,
        }
    }
}

fn ok_key(s: Section) -> String {
    format!("last_ok:{}", s.as_str())
}

fn attempt_key(s: Section) -> String {
    format!("last_attempt:{}", s.as_str())
}

/// The last refresh where a source of `section` worked. Falls back to the overall time
/// for data from before the two clocks were split.
pub fn last_ok(store: &Store, section: Section) -> Option<DateTime<Utc>> {
    store
        .time_meta(&ok_key(section))
        .or_else(|| store.time_meta(LAST_OK))
}

fn last_attempt(store: &Store, section: Section) -> Option<DateTime<Utc>> {
    store
        .time_meta(&attempt_key(section))
        .or_else(|| store.time_meta(LAST_ATTEMPT))
}

/// What an automatic refresh should cover now, if anything: arXiv after the daily slot,
/// news every `NEWS_EVERY`.
pub fn due_scope(store: &Store, settings: &Settings, now: DateTime<Utc>) -> Option<Scope> {
    let research = schedule::due_days(
        last_ok(store, Section::Research),
        last_attempt(store, Section::Research),
        now,
        schedule::DEFAULT_SLOT_UTC,
        settings.arxiv_every_days,
    );
    let news = schedule::due_every(
        last_ok(store, Section::News),
        last_attempt(store, Section::News),
        now,
        news_every(settings),
    );
    Scope::join(research, news)
}

fn news_every(settings: &Settings) -> chrono::Duration {
    chrono::Duration::hours(i64::from(settings.news_every_hours.clamp(1, 24)))
}

/// When the next automatic refresh of `section` is planned.
pub fn next_due(
    store: &Store,
    settings: &Settings,
    section: Section,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    match section {
        Section::Research => {
            let slot = schedule::DEFAULT_SLOT_UTC;
            last_ok(store, Section::Research)
                .map(|t| {
                    schedule::latest_slot(t, slot)
                        + chrono::Duration::days(i64::from(settings.arxiv_every_days.clamp(1, 7)))
                })
                .filter(|t| *t > now)
                .unwrap_or_else(|| schedule::next_slot(now, slot))
        }
        Section::News => last_ok(store, Section::News)
            .map(|t| t + news_every(settings))
            .filter(|t| *t > now)
            .unwrap_or(now),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceReport {
    pub source: String,
    pub name: String,
    pub section: Section,
    /// Items in the feed this time (0 is fine: arXiv at weekends).
    pub fetched: usize,
    pub new: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub at: DateTime<Utc>,
    pub sources: Vec<SourceReport>,
}

impl Report {
    pub fn any_ok(&self) -> bool {
        self.sources.iter().any(|s| s.error.is_none())
    }

    pub fn new_items(&self) -> usize {
        self.sources.iter().map(|s| s.new).sum()
    }
}

struct Job {
    source: String,
    name: String,
    section: Section,
    url: String,
}

struct Fetched {
    items: Vec<Item>,
    build_date: Option<String>,
}

fn jobs(topics: &Topics, scope: Scope) -> Vec<Job> {
    let mut jobs = Vec::new();
    let cats = topics.arxiv_categories();
    if !cats.is_empty() && scope.covers(Section::Research) {
        jobs.push(Job {
            source: arxiv::SOURCE_ID.into(),
            name: "arXiv".into(),
            section: Section::Research,
            url: arxiv::feed_url(&cats),
        });
    }
    for f in topics
        .news_feeds()
        .into_iter()
        .filter(|_| scope.covers(Section::News))
    {
        jobs.push(Job {
            source: f.url.clone(),
            name: f.name,
            section: Section::News,
            url: f.url,
        });
    }
    jobs
}

/// Downloads and parses in parallel (a dozen or so threads, each mostly waiting on the
/// network); stores afterwards, one transaction per source, and records each source's status.
fn fetch_and_store(
    store: &mut Store,
    jobs: &[Job],
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
) -> Vec<SourceReport> {
    let results: Vec<Result<Fetched, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|job| s.spawn(move || fetch_one(job, fetch)))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("crashed while parsing".into()))
            })
            .collect()
    });

    let mut reports = Vec::new();
    for (job, result) in jobs.iter().zip(results) {
        let (stored, build_date) = match result {
            Ok(f) => {
                let n = f.items.len();
                (
                    store.upsert(&f.items, now).map(|new| (n, new)),
                    f.build_date,
                )
            }
            Err(e) => (Err(e), None),
        };
        let counted = stored.as_ref().map(|(n, _)| *n).map_err(Clone::clone);
        let _ = store.record(
            &job.source,
            &job.name,
            job.section,
            now,
            &counted,
            build_date.as_deref(),
        );
        reports.push(SourceReport {
            source: job.source.clone(),
            name: job.name.clone(),
            section: job.section,
            fetched: stored.as_ref().map(|(n, _)| *n).unwrap_or(0),
            new: stored.as_ref().map(|(_, new)| *new).unwrap_or(0),
            error: stored.err(),
        });
    }
    reports
}

/// After new items: topics, muted news read, statuses of dropped sources forgotten.
fn tidy_after(store: &mut Store, topics: &Topics, now: DateTime<Utc>) {
    let _ = store.retag(topics);
    // Stories from a muted source or with a muted word arrive already read.
    if let Ok(unread) = store.unread_news() {
        let keys = crate::news::to_dismiss(&topics.mute, &unread, now);
        if !keys.is_empty() {
            let _ = store.set_read_many(&keys, true, now);
        }
    }
    let _ = forget_old_sources(store, topics);
}

/// Fetches one source again (the menu's "Retry"), by its source id (a feed's URL,
/// or `arxiv::SOURCE_ID`). Leaves the refresh clocks alone, so the next scheduled refresh
/// comes at its usual time. arXiv's feed is fetched at most once per daily slot elsewhere;
/// a retry after a failure is the one extra request.
pub fn retry(
    store: &mut Store,
    topics: &Topics,
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
    source: &str,
) -> Result<SourceReport, String> {
    let job: Vec<Job> = jobs(topics, Scope::All)
        .into_iter()
        .filter(|j| j.source == source)
        .collect();
    if job.is_empty() {
        return Err("that source isn't in your topics any more".into());
    }
    let report = fetch_and_store(store, &job, fetch, now)
        .pop()
        .ok_or("nothing fetched")?;
    tidy_after(store, topics, now);
    Ok(report)
}

pub fn run(
    store: &mut Store,
    topics: &Topics,
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
    scope: Scope,
) -> Report {
    let jobs = jobs(topics, scope);
    let reports = fetch_and_store(store, &jobs, fetch, now);

    let report = Report {
        at: now,
        sources: reports,
    };
    tidy_after(store, topics, now);
    let _ = store.set_time_meta(LAST_ATTEMPT, now);
    if report.any_ok() {
        let _ = store.set_time_meta(LAST_OK, now);
    }
    for section in [Section::Research, Section::News] {
        let ran: Vec<_> = report
            .sources
            .iter()
            .filter(|s| s.section == section)
            .collect();
        if ran.is_empty() {
            // Nothing to fetch in this section (no feeds yet): its clock moves on anyway, so
            // the scheduler doesn't ask again every minute. Saving a new feed refreshes at once.
            if scope.covers(section) {
                let _ = store.set_time_meta(&attempt_key(section), now);
                let _ = store.set_time_meta(&ok_key(section), now);
            }
            continue;
        }
        let _ = store.set_time_meta(&attempt_key(section), now);
        if ran.iter().any(|s| s.error.is_none()) {
            let _ = store.set_time_meta(&ok_key(section), now);
        }
    }
    report
}

/// Drops the status of sources no longer in the topics (a feed removed, switched off or given
/// a new address), so an old failure doesn't stay red in the footer and the menu. Their items
/// stay.
pub fn forget_old_sources(store: &Store, topics: &Topics) -> Result<(), String> {
    let mut keep: Vec<String> = topics.news_feeds().into_iter().map(|f| f.url).collect();
    if !topics.arxiv_categories().is_empty() {
        keep.push(arxiv::SOURCE_ID.into());
    }
    store.keep_statuses(&keep)
}

fn fetch_one(job: &Job, fetch: &dyn Fetch) -> Result<Fetched, String> {
    let bytes = fetch.get(&job.url)?;
    let items = match job.section {
        Section::Research => arxiv::parse(&bytes)?,
        Section::News => rss::parse(&bytes, &job.source, &job.name)?,
    };
    let build_date = (job.section == Section::Research)
        .then(|| arxiv::tag(&String::from_utf8_lossy(&bytes), "lastBuildDate"))
        .flatten();
    Ok(Fetched { items, build_date })
}

/// Fetches and parses one news feed without storing anything: the topic picker's "Test feed".
/// Returns the number of items and the first title.
pub fn test_feed(fetch: &dyn Fetch, url: &str) -> Result<(usize, Option<String>), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("a feed address starts with https://".into());
    }
    let bytes = fetch.get(url)?;
    let items = rss::parse(&bytes, url, "test")?;
    Ok((items.len(), items.first().map(|i| i.title.clone())))
}

/// Papers fetched per category when backfilling.
pub const BACKFILL_PER_CATEGORY: u32 = 40;
/// arXiv asks for at most one API request every 3 seconds.
pub const API_PAUSE: std::time::Duration = std::time::Duration::from_secs(3);

fn backfill_key(category: &str) -> String {
    format!("backfilled:{category}")
}

/// When arXiv says "too many requests" (HTTP 429), no backfill until this time.
pub const BACKFILL_PAUSED_UNTIL: &str = "backfill_paused_until";
pub const BACKFILL_BACK_OFF: chrono::Duration = chrono::Duration::hours(1);

/// Enabled arXiv categories that have never been backfilled (first run, or newly added).
/// Empty while arXiv has asked us to slow down.
pub fn needs_backfill(store: &Store, topics: &Topics) -> Vec<String> {
    needs_backfill_at(store, topics, Utc::now())
}

pub fn needs_backfill_at(store: &Store, topics: &Topics, now: DateTime<Utc>) -> Vec<String> {
    if store
        .time_meta(BACKFILL_PAUSED_UNTIL)
        .is_some_and(|t| t > now)
    {
        return vec![];
    }
    topics
        .arxiv_categories()
        .into_iter()
        .filter(|c| store.meta(&backfill_key(c)).ok().flatten().is_none())
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct BackfillReport {
    pub fetched: usize,
    pub new: usize,
    /// (category, reason) for categories that failed; they are tried again next time.
    pub failed: Vec<(String, String)>,
}

/// Fetches the newest papers of each category from the arXiv API and stores them, so a new
/// category isn't empty until arXiv's next announcement. `pause` runs between requests
/// (3 s in the app, nothing in tests). A category is marked done only when its fetch worked.
pub fn backfill(
    store: &mut Store,
    topics: &Topics,
    categories: &[String],
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
    pause: &dyn Fn(),
) -> BackfillReport {
    let mut report = BackfillReport {
        fetched: 0,
        new: 0,
        failed: vec![],
    };
    for (i, cat) in categories.iter().enumerate() {
        if i > 0 {
            pause();
        }
        let url = arxiv_api::recent_url(cat, BACKFILL_PER_CATEGORY);
        // The API is often slow: one more try after a pause, but only after a time-out.
        let result = fetch
            .get_patient(&url)
            .or_else(|e| {
                if e == "timed out" {
                    pause();
                    fetch.get_patient(&url)
                } else {
                    Err(e)
                }
            })
            .and_then(|b| arxiv_api::parse(&b))
            .and_then(|items| {
                let n = items.len();
                store.upsert(&items, now).map(|new| (n, new))
            });
        match result {
            Ok((n, new)) => {
                report.fetched += n;
                report.new += new;
                let _ = store.set_time_meta(&backfill_key(cat), now);
            }
            Err(e) => {
                let refused = e == "HTTP 429" || e == "HTTP 503";
                report.failed.push((cat.clone(), e));
                if refused {
                    // arXiv asked us to slow down: stop here and leave it alone for an hour.
                    let _ = store.set_time_meta(BACKFILL_PAUSED_UNTIL, now + BACKFILL_BACK_OFF);
                    for rest in &categories[i + 1..] {
                        report
                            .failed
                            .push((rest.clone(), "not tried: arXiv asked to slow down".into()));
                    }
                    break;
                }
            }
        }
    }
    let _ = store.retag(topics);
    report
}
