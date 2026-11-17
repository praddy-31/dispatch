//! `status.json`, a small file other programs can read: SketchyBar (Palette's
//! "Dispatch" bar item), Raycast, a script of yours. Written by the app into the data folder
//! when Settings → Menu bar → "Status for SketchyBar" is on, and only when a number changed.
//!
//! `label` is the pill's text, already put together from `Settings::bar_metrics`, so the bar
//! script only shows it (one place decides what the numbers mean). `bar: false` (the switch
//! turned off) tells the script to hide the pill.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::companion::Companion;
use crate::settings::Settings;
use crate::stats;
use crate::store::Store;

pub const FILE: &str = "status.json";

/// The pill's choices: id and what Settings calls it.
pub const METRICS: [(&str, &str); 10] = [
    ("read_today", "Papers read today"),
    ("seen_today", "Papers seen today"),
    ("saved_today", "Papers saved today"),
    ("goal", "Weekly goal (read / goal)"),
    ("suggested", "Suggested papers left"),
    ("foryou", "Today's papers by people and keywords you follow"),
    ("later", "On the Read later list"),
    ("research_unread", "Unread papers"),
    ("news_unread", "Unread news"),
    ("next_refresh", "Next arXiv refresh"),
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub version: u32,
    /// When it was written (this Mac's time).
    pub updated: String,
    /// Today, this Mac's time ("2026-11-03"): a script can tell a stale file.
    pub date: String,
    /// The SketchyBar pill is wanted (false = hide it).
    pub bar: bool,
    /// The pill's text ("3 read · 1/10"); empty = nothing to show.
    pub label: String,
    pub research_unread: i64,
    pub news_unread: i64,
    pub suggested_left: usize,
    pub for_you_today: usize,
    pub later_today: usize,
    pub new_since_seen: usize,
    /// Papers read (page or PDF opened), seen, saved and noted today.
    pub read_today: i64,
    pub seen_today: i64,
    pub saved_today: i64,
    pub noted_today: i64,
    pub week_read: i64,
    /// 0 = no goal.
    pub weekly_goal: u32,
    /// Sources whose last fetch failed.
    pub failed_sources: usize,
    pub research_next: String,
    pub news_next: String,
}

impl Status {
    /// From the menu-bar data (already built every few seconds) plus today's counts.
    pub fn build(
        c: &Companion,
        store: &Store,
        settings: &Settings,
        now: DateTime<Utc>,
    ) -> Result<Status, String> {
        let local = stats::local(now);
        let today = stats::counts_per_day(store, local, 1)?
            .remove(&local.date_naive())
            .unwrap_or_default();
        let at = |t: DateTime<Utc>| stats::local(t).to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut s = Status {
            version: 1,
            updated: local.to_rfc3339_opts(SecondsFormat::Secs, true),
            date: local.format("%Y-%m-%d").to_string(),
            bar: true,
            label: String::new(),
            research_unread: c.research_unread,
            news_unread: c.news_unread,
            suggested_left: c.suggested_left,
            for_you_today: c.for_you_today,
            later_today: c.later_today,
            new_since_seen: c.new_since_seen,
            read_today: today.read,
            seen_today: today.seen,
            saved_today: today.saved,
            noted_today: today.noted,
            week_read: c.week_read,
            weekly_goal: c.weekly_goal,
            failed_sources: c.failed,
            research_next: at(c.research_next),
            news_next: at(c.news_next),
        };
        s.label = s.label_for(&settings.bar_metrics);
        Ok(s)
    }

    /// One metric's short text for the pill; `None` when it has nothing to say (no goal set).
    pub fn metric(&self, id: &str) -> Option<String> {
        Some(match id {
            "read_today" => format!("{} read", self.read_today),
            "seen_today" => format!("{} seen", self.seen_today),
            "saved_today" => format!("{} saved", self.saved_today),
            "goal" if self.weekly_goal > 0 => format!("{}/{}", self.week_read, self.weekly_goal),
            "suggested" => format!("{} suggested", self.suggested_left),
            "foryou" => format!("{} for you", self.for_you_today),
            "later" => format!("{} later", self.later_today),
            "research_unread" => format!("{} papers", self.research_unread),
            "news_unread" => format!("{} news", self.news_unread),
            // "13:30", from the stored local time.
            "next_refresh" => format!("next {}", self.research_next.get(11..16)?),
            _ => return None,
        })
    }

    pub fn label_for(&self, metrics: &[String]) -> String {
        metrics
            .iter()
            .filter_map(|m| self.metric(m))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// The same status with the moment it was written left out: two equal fingerprints mean
    /// nothing worth rewriting the file for.
    pub fn fingerprint(&self) -> String {
        let mut s = self.clone();
        s.updated.clear();
        serde_json::to_string(&s).unwrap_or_default()
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())? + "\n";
        crate::paths::write_atomic(path, json.as_bytes())
    }
}

/// What the file says once the switch is off: the pill hides itself.
pub fn turned_off(now: DateTime<Utc>) -> serde_json::Value {
    let local = stats::local(now);
    serde_json::json!({
        "version": 1,
        "updated": local.to_rfc3339_opts(SecondsFormat::Secs, true),
        "date": local.format("%Y-%m-%d").to_string(),
        "bar": false,
        "label": "",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            version: 1,
            updated: "2026-11-03T14:00:00+08:00".into(),
            date: "2026-11-03".into(),
            bar: true,
            label: String::new(),
            research_unread: 120,
            news_unread: 9,
            suggested_left: 12,
            for_you_today: 2,
            later_today: 1,
            new_since_seen: 0,
            read_today: 3,
            seen_today: 7,
            saved_today: 1,
            noted_today: 0,
            week_read: 4,
            weekly_goal: 10,
            failed_sources: 0,
            research_next: "2026-11-04T13:30:00+08:00".into(),
            news_next: "2026-11-03T17:00:00+08:00".into(),
        }
    }

    #[test]
    fn the_pill_shows_the_chosen_metrics_in_order() {
        let s = status();
        let ids = |v: &[&str]| v.iter().map(|m| m.to_string()).collect::<Vec<_>>();
        assert_eq!(s.label_for(&ids(&["read_today"])), "3 read");
        assert_eq!(
            s.label_for(&ids(&["goal", "suggested", "next_refresh"])),
            "4/10 · 12 suggested · next 13:30"
        );
        let no_goal = Status {
            weekly_goal: 0,
            ..status()
        };
        assert_eq!(no_goal.label_for(&ids(&["goal"])), "", "no goal: nothing");
        assert_eq!(s.label_for(&ids(&["nonsense"])), "");
        // Every choice in Settings has a text.
        for (id, _) in METRICS {
            assert!(s.metric(id).is_some(), "{id}");
        }
        // Only the write time differs: same fingerprint.
        let later = Status {
            updated: "2026-11-03T14:02:00+08:00".into(),
            ..status()
        };
        assert_eq!(s.fingerprint(), later.fingerprint());
    }
}
