//! News-only extras, all local.
//!
//! - **Mutes:** a source or a word, for a day, a week, a month or until unmuted. Kept in
//!   `topics.json` (`mute`), so they sync with topics. Muted stories are left out of the lists
//!   and marked read (as "dismissed", so Stats doesn't count them), so the counts agree.
//! - **Discussion:** a story's discussion page, when its feed names one (`<comments>`).
//! - **Same story:** stories from different feeds whose titles share most of their words,
//!   published within `SAME_STORY_HOURS`, are shown once with "also in" the others.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::item::Item;
use crate::store::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MuteKind {
    /// `value` is the feed URL (`Item::source`); `label` its name.
    Source,
    /// `value` is a word or phrase, matched as whole words in titles, any case.
    Word,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mute {
    pub kind: MuteKind,
    pub value: String,
    /// What Settings shows (the feed's name; the word itself).
    #[serde(default)]
    pub label: String,
    /// Muted until then; `None` = until unmuted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime<Utc>>,
}

impl Mute {
    pub fn active(&self, now: DateTime<Utc>) -> bool {
        self.until.is_none_or(|u| u > now)
    }

    pub fn matches(&self, item: &Item) -> bool {
        match self.kind {
            MuteKind::Source => item.source == self.value,
            MuteKind::Word => {
                let phrase = words(&self.value).join(" ");
                !phrase.is_empty()
                    && format!(" {} ", words(&item.title).join(" "))
                        .contains(&format!(" {phrase} "))
            }
        }
    }
}

/// "1d", "7d", "30d" or "" (until unmuted) → when the mute ends.
pub fn until(span: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let days: i64 = span.trim_end_matches('d').parse().ok()?;
    (days > 0).then(|| now + Duration::days(days))
}

/// Whether any active mute hides this item (news only).
pub fn muted(mutes: &[Mute], item: &Item, now: DateTime<Utc>) -> bool {
    item.section == crate::Section::News && mutes.iter().any(|m| m.active(now) && m.matches(item))
}

/// The keys among `unread` that an active mute hides (to be marked read).
pub fn to_dismiss(mutes: &[Mute], unread: &[Item], now: DateTime<Utc>) -> Vec<String> {
    unread
        .iter()
        .filter(|i| muted(mutes, i, now))
        .map(|i| i.key.clone())
        .collect()
}

/// Drops mutes that have run out (kept tidy whenever the list is saved).
pub fn prune(mutes: &mut Vec<Mute>, now: DateTime<Utc>) {
    mutes.retain(|m| m.active(now));
}

// ---- discussion pages ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Discussion {
    /// The story's discussion page, as its feed names it.
    pub url: String,
}

/// The discussion link `rss::parse` kept at the end of a story's summary, and the summary
/// without it. A summary left with at most one word (a bare "Comments" link) counts as none.
pub fn discussion(summary: &str) -> Option<(String, Discussion)> {
    let marker = crate::sources::rss::DISCUSSION_MARKER;
    let start = summary.find(marker)?;
    let url: String = summary[start + marker.len()..]
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '<')
        .collect();
    if !url.starts_with("http") {
        return None;
    }
    let rest = summary[..start].trim();
    let rest = if rest.split_whitespace().count() <= 1 {
        String::new()
    } else {
        rest.to_owned()
    };
    Some((rest, Discussion { url }))
}

/// Fills `discussion` on news rows (called with `hints::annotate`) and takes the link out of
/// the summary shown.
pub fn annotate(rows: &mut [Row]) {
    for r in rows {
        if r.item.section == crate::Section::News
            && let Some((rest, d)) = discussion(&r.item.summary)
        {
            r.item.summary = rest;
            r.discussion = Some(d);
        }
    }
}

// ---- the same story from several feeds -------------------------------------------------

/// Stories further apart than this aren't the same story.
pub const SAME_STORY_HOURS: i64 = 36;

/// Another feed's copy of a story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Also {
    pub key: String,
    pub source_name: String,
    pub link: String,
    pub read: bool,
}

const STOP: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "into", "over", "after", "about", "new",
    "says", "say", "its", "are", "was", "has", "have", "will", "how", "why", "what", "who", "you",
    "your", "our", "but", "not", "out", "more", "than", "his", "her", "their", "they", "can",
];

/// Title words that carry meaning: lower case, letters and digits, 3+ characters, no stop words.
fn key_words(title: &str) -> HashSet<String> {
    words(title)
        .into_iter()
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .collect()
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Same story: at least 3 meaningful words in common, and they make up at least half of the
/// shorter title's words.
pub fn same_story(a: &str, b: &str) -> bool {
    let (a, b) = (key_words(a), key_words(b));
    let shared = a.intersection(&b).count();
    let smaller = a.len().min(b.len());
    shared >= 3 && smaller > 0 && shared * 2 >= smaller
}

/// Folds copies of a story into the first row that has it (the list's order is kept): each
/// kept row lists the others in `also`. Only rows from different feeds are folded.
pub fn merge(rows: Vec<Row>) -> Vec<Row> {
    let when = |r: &Row| r.item.published.unwrap_or(r.first_seen);
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    'rows: for r in rows {
        if r.item.section != crate::Section::News {
            out.push(r);
            continue;
        }
        for kept in out
            .iter_mut()
            .filter(|k| k.item.section == crate::Section::News)
        {
            let feeds_differ = kept.item.source != r.item.source
                && kept
                    .also
                    .iter()
                    .all(|a| a.source_name != r.item.source_name);
            if feeds_differ
                && (when(kept) - when(&r)).num_hours().abs() <= SAME_STORY_HOURS
                && same_story(&kept.item.title, &r.item.title)
            {
                kept.also.push(Also {
                    key: r.item.key.clone(),
                    source_name: r.item.source_name.clone(),
                    link: r.item.link.clone(),
                    read: r.read,
                });
                continue 'rows;
            }
        }
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Section;

    fn story(key: &str, source: &str, title: &str, hour: u32) -> Item {
        Item {
            key: key.into(),
            section: Section::News,
            source: format!("https://{source}/rss"),
            source_name: source.into(),
            title: title.into(),
            authors: vec![],
            summary: String::new(),
            link: format!("https://{source}/{key}"),
            pdf: None,
            categories: vec![],
            published: format!("2026-11-02T{hour:02}:00:00Z").parse().ok(),
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn mutes_by_source_and_whole_word_until_they_run_out() {
        let now: DateTime<Utc> = "2026-11-03T00:00:00Z".parse().unwrap();
        let a = story("news:a", "daily", "Crypto exchange collapses overnight", 1);
        let b = story("news:b", "forum", "Cryptography after quantum computers", 2);
        let word = Mute {
            kind: MuteKind::Word,
            value: "Crypto".into(),
            label: "crypto".into(),
            until: until("7d", now),
        };
        assert!(word.matches(&a) && !word.matches(&b), "whole words only");
        let src = Mute {
            kind: MuteKind::Source,
            value: "https://forum/rss".into(),
            label: "forum".into(),
            until: None,
        };
        let mut mutes = vec![word, src];
        assert_eq!(
            to_dismiss(&mutes, &[a.clone(), b.clone()], now),
            ["news:a", "news:b"]
        );
        // A week later the word mute has run out; the source mute stays.
        let later = now + Duration::days(8);
        assert_eq!(to_dismiss(&mutes, &[a.clone(), b], later), ["news:b"]);
        prune(&mut mutes, later);
        assert_eq!(mutes.len(), 1);
        assert_eq!(until("", now), None);
        // Phrases match as a run of words.
        let phrase = Mute {
            kind: MuteKind::Word,
            value: "exchange collapses".into(),
            label: String::new(),
            until: None,
        };
        assert!(phrase.matches(&a));
    }

    #[test]
    fn discussion_link_is_taken_out_of_the_summary() {
        assert_eq!(discussion("A story with no discussion"), None);
        let (rest, d) =
            discussion("Comments\nComments URL: https://forum.example.org/t/5").unwrap();
        assert_eq!(d.url, "https://forum.example.org/t/5");
        assert_eq!(rest, "", "a bare link word is no summary");
        let (rest, _) =
            discussion("Two real sentences here.\nComments URL: https://forum.example.org/t/6")
                .unwrap();
        assert_eq!(rest, "Two real sentences here.");
    }

    #[test]
    fn same_story_merges_across_feeds_only() {
        assert!(same_story(
            "Japan's prime minister resigns after election defeat",
            "Japanese PM resigns: prime minister quits after election defeat"
        ));
        assert!(!same_story(
            "Apple announces new iPhone",
            "Apple announces new iPad"
        ));
        // Half the shorter title's words is enough (5 of 10 here).
        assert!(same_story(
            "Cat A COE premium hits all-time high of $139,001; Cat B price also rises",
            "Category A COE premium hits record S$139,001 in latest bidding exercise"
        ));
        let row = |i: Item| {
            let dir = tempfile::tempdir().unwrap();
            let mut store = crate::store::Store::open(&dir.path().join("t.sqlite")).unwrap();
            store.upsert(std::slice::from_ref(&i), Utc::now()).unwrap();
            store.rows(std::slice::from_ref(&i.key)).unwrap().remove(0)
        };
        let rows = vec![
            row(story(
                "news:1",
                "daily",
                "Prime minister resigns after election defeat",
                9,
            )),
            row(story(
                "news:2",
                "herald",
                "Prime minister resigns after crushing election defeat",
                8,
            )),
            row(story(
                "news:3",
                "daily",
                "Prime minister resigns after election defeat, again",
                7,
            )),
            row(story("news:4", "herald", "Rain expected all week", 6)),
        ];
        let merged = merge(rows);
        let keys: Vec<_> = merged.iter().map(|r| r.item.key.as_str()).collect();
        // Same feed (daily twice) isn't folded; another feed's copy is.
        assert_eq!(keys, ["news:1", "news:3", "news:4"]);
        assert_eq!(merged[0].also.len(), 1);
        assert_eq!(merged[0].also[0].source_name, "herald");
    }
}
