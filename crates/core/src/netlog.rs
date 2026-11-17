//! The network log. Which hosts Dispatch contacted in the last `DAYS` days, how
//! often, and why, so the user can check the privacy promise himself. Local only: one small
//! file in the data folder (`network-log.json`), never sent anywhere.
//!
//! Only the host is kept, not the address (a search's words or an API key never land in the
//! log). Requests are counted per day and host, so the file stays a few kilobytes.
//!
//! The app (and the CLI) call `init` with the data folder once; until then `record` does
//! nothing, which keeps the tests (fake fetchers, no network) from writing files.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::paths::write_atomic;

pub const FILE: &str = "network-log.json";
/// How long requests are remembered.
pub const DAYS: i64 = 7;

/// One host on one day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// "2026-11-03" (UTC).
    pub day: String,
    pub host: String,
    pub count: u32,
    pub failed: u32,
    pub last: DateTime<Utc>,
}

/// What Settings shows: a host over the last week.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Host {
    pub host: String,
    pub why: String,
    pub count: u32,
    pub failed: u32,
    pub last: DateTime<Utc>,
}

struct Log {
    path: PathBuf,
    entries: Vec<Entry>,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

/// Starts logging into `data_dir/network-log.json` (keeps what an earlier run wrote).
pub fn init(data_dir: &Path) {
    let path = data_dir.join(FILE);
    let entries = load(&path);
    let _ = LOG.set(Mutex::new(Log { path, entries }));
}

fn load(path: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// The host of a web address, without the port ("export.arxiv.org").
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    let host = host.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Counts one request (call it after the answer, `ok` = it succeeded).
pub fn record(url: &str, ok: bool) {
    let (Some(log), Some(host)) = (LOG.get(), host_of(url)) else {
        return;
    };
    let Ok(mut log) = log.lock() else { return };
    let now = Utc::now();
    add(&mut log.entries, &host, ok, now);
    let text = serde_json::to_string_pretty(&log.entries).unwrap_or_default() + "\n";
    let _ = write_atomic(&log.path, text.as_bytes());
}

fn add(entries: &mut Vec<Entry>, host: &str, ok: bool, now: DateTime<Utc>) {
    let day = now.format("%Y-%m-%d").to_string();
    let cutoff = (now - Duration::days(DAYS)).format("%Y-%m-%d").to_string();
    entries.retain(|e| e.day > cutoff);
    match entries.iter_mut().find(|e| e.day == day && e.host == host) {
        Some(e) => {
            e.count += 1;
            e.failed += u32::from(!ok);
            e.last = now;
        }
        None => entries.push(Entry {
            day,
            host: host.to_owned(),
            count: 1,
            failed: u32::from(!ok),
            last: now,
        }),
    }
}

/// Why Dispatch talks to a host. Feeds the user added are "a news feed" unless they're one
/// of the known services.
pub fn why(host: &str) -> &'static str {
    match host {
        "rss.arxiv.org" => "arXiv's daily feed",
        "export.arxiv.org" => "arXiv search: new categories and author searches",
        "arxiv.org" | "www.arxiv.org" => "arXiv: BibTeX for a citation, PDFs you open",
        "doi.org" | "dx.doi.org" | "api.crossref.org" | "data.crossref.org" => {
            "Citations for papers with a DOI"
        }
        "api.semanticscholar.org" => "Semantic Scholar lists you asked for",
        "api.zotero.org" => "Zotero online (Send to Zotero, collections)",
        "127.0.0.1" | "localhost" => "The Zotero app on this Mac",
        _ => "A news feed you follow",
    }
}

/// The last week by host, most requests first.
pub fn summary(entries: &[Entry], now: DateTime<Utc>) -> Vec<Host> {
    let cutoff = (now - Duration::days(DAYS)).format("%Y-%m-%d").to_string();
    let mut hosts: Vec<Host> = Vec::new();
    for e in entries.iter().filter(|e| e.day > cutoff) {
        match hosts.iter_mut().find(|h| h.host == e.host) {
            Some(h) => {
                h.count += e.count;
                h.failed += e.failed;
                h.last = h.last.max(e.last);
            }
            None => hosts.push(Host {
                host: e.host.clone(),
                why: why(&e.host).into(),
                count: e.count,
                failed: e.failed,
                last: e.last,
            }),
        }
    }
    hosts.sort_by(|a, b| b.count.cmp(&a.count).then(a.host.cmp(&b.host)));
    hosts
}

/// The last week, read from the file (so the CLI sees what the app logged).
pub fn recent(data_dir: &Path, now: DateTime<Utc>) -> Vec<Host> {
    match LOG.get().and_then(|l| l.lock().ok()) {
        Some(log) => summary(&log.entries, now),
        None => summary(&load(&data_dir.join(FILE)), now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_only_counted_per_day_and_kept_a_week() {
        assert_eq!(
            host_of("https://api.semanticscholar.org/graph/v1/paper/x?fields=a").as_deref(),
            Some("api.semanticscholar.org")
        );
        assert_eq!(
            host_of("http://127.0.0.1:23119/connector/ping").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(host_of("not a url"), None);

        let now: DateTime<Utc> = "2026-11-03T05:30:00Z".parse().unwrap();
        let mut log = vec![];
        add(&mut log, "rss.arxiv.org", true, now - Duration::days(9));
        add(&mut log, "rss.arxiv.org", true, now);
        add(&mut log, "rss.arxiv.org", false, now);
        add(&mut log, "feeds.example.org", true, now);
        // The nine-day-old entry is dropped on the next write.
        assert_eq!(log.len(), 2);
        let s = summary(&log, now);
        assert_eq!(s[0].host, "rss.arxiv.org");
        assert_eq!((s[0].count, s[0].failed), (2, 1));
        assert_eq!(s[1].why, "A news feed you follow");
    }
}
