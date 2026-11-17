//! Alerts: after a refresh, what is worth a notification. One summary per refresh
//!, from three kinds, each switched on in Settings (all off by default):
//! a followed author's new paper, a watched keyword in a new paper, and a saved paper that got
//! a new version or a journal DOI.
//!
//! "New" = first stored by this refresh (`first_seen` = the refresh's time), so backfill and
//! sync never notify, and a cross-listing seen yesterday doesn't count again.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::companion::menu_text;
use crate::follow::Follow;
use crate::item::Section;
use crate::settings::Settings;
use crate::store::Store;

/// Saved papers' version, DOI and journal before a refresh, to see what it changed.
#[derive(Debug, Default)]
pub struct Snapshot(HashMap<String, Published>);

/// A saved paper's version, DOI and journal reference.
type Published = (Option<u32>, Option<String>, Option<String>);

pub fn snapshot(store: &Store) -> Result<Snapshot, String> {
    Ok(Snapshot(
        store
            .list_saved(None, 100_000)?
            .into_iter()
            .map(|r| (r.item.key, (r.item.version, r.item.doi, r.item.journal_ref)))
            .collect(),
    ))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Why {
    Author(Vec<String>),
    Keyword(Vec<String>),
    /// "v2", "DOI", "journal"
    Updated(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alert {
    pub key: String,
    pub title: String,
    pub why: Why,
}

/// What this refresh brought that the user asked to hear about. A paper by a followed author
/// that also matches a keyword counts once, as the author's.
pub fn collect(
    store: &Store,
    follow: &Follow,
    settings: &Settings,
    since: DateTime<Utc>,
    before: &Snapshot,
) -> Result<Vec<Alert>, String> {
    let mut out = Vec::new();
    if settings.notify_authors || settings.notify_keywords {
        for item in store.new_since(Section::Research, since)? {
            let authors = follow.authors_in(&item);
            let keywords = follow.keywords_in(&item);
            let why = if settings.notify_authors && !authors.is_empty() {
                Why::Author(authors)
            } else if settings.notify_keywords && !keywords.is_empty() {
                Why::Keyword(keywords)
            } else {
                continue;
            };
            out.push(Alert {
                key: item.key,
                title: item.title,
                why,
            });
        }
    }
    if settings.notify_updates {
        for r in store.list_saved(None, 100_000)? {
            let Some((version, doi, journal)) = before.0.get(&r.item.key) else {
                continue;
            };
            let what = if r.item.version.unwrap_or(0) > version.unwrap_or(0) {
                format!("v{}", r.item.version.unwrap_or(0))
            } else if doi.is_none() && r.item.doi.is_some() {
                "DOI".to_owned()
            } else if journal.is_none() && r.item.journal_ref.is_some() {
                "journal".to_owned()
            } else {
                continue;
            };
            out.push(Alert {
                key: r.item.key,
                title: r.item.title,
                why: Why::Updated(what),
            });
        }
    }
    Ok(out)
}

/// The one notification (title, body) for these alerts, or None if there are none.
pub fn summary(alerts: &[Alert]) -> Option<(String, String)> {
    if alerts.is_empty() {
        return None;
    }
    let new: Vec<&Alert> = alerts
        .iter()
        .filter(|a| !matches!(a.why, Why::Updated(_)))
        .collect();
    let updated: Vec<&Alert> = alerts
        .iter()
        .filter(|a| matches!(a.why, Why::Updated(_)))
        .collect();
    let short = |t: &str| menu_text(t, 70);
    let mut lines = Vec::new();

    let mut people: Vec<String> = Vec::new();
    let mut by_people = 0;
    let mut words: Vec<String> = Vec::new();
    let mut by_words = 0;
    for a in &new {
        match &a.why {
            Why::Author(names) => {
                by_people += 1;
                for n in names {
                    let surname = n.split_whitespace().last().unwrap_or(n).to_owned();
                    if !people.contains(&surname) {
                        people.push(surname);
                    }
                }
            }
            Why::Keyword(ks) => {
                by_words += 1;
                for k in ks {
                    if !words.contains(k) {
                        words.push(k.clone());
                    }
                }
            }
            Why::Updated(_) => {}
        }
    }
    let list = |v: &[String], quote: bool| {
        let shown: Vec<String> = v
            .iter()
            .take(3)
            .map(|s| {
                if quote {
                    format!("“{s}”")
                } else {
                    s.clone()
                }
            })
            .collect();
        let more = if v.len() > 3 {
            format!(" +{}", v.len() - 3)
        } else {
            String::new()
        };
        format!("{}{more}", shown.join(", "))
    };
    let papers = |n: usize| if n == 1 { "paper" } else { "papers" };
    if new.len() == 1 {
        lines.push(short(&new[0].title));
    }
    if by_people > 0 {
        lines.push(format!(
            "{by_people} {} by {}",
            papers(by_people),
            list(&people, false)
        ));
    }
    if by_words > 0 {
        lines.push(format!(
            "{by_words} {} mentioning {}",
            papers(by_words),
            list(&words, true)
        ));
    }
    match updated.as_slice() {
        [] => {}
        [one] => {
            let Why::Updated(what) = &one.why else {
                unreachable!()
            };
            let what = match what.as_str() {
                "DOI" => "now has a DOI".to_owned(),
                "journal" => "now in a journal".to_owned(),
                v => format!("new version ({v})"),
            };
            lines.push(format!("Saved: {} — {what}", short(&one.title)));
        }
        many => lines.push(format!("{} saved papers updated", many.len())),
    }
    let title = match (new.len(), updated.len()) {
        (0, 1) => "Saved paper updated".to_owned(),
        (0, n) => format!("{n} saved papers updated"),
        (n, _) => format!("{n} new {} for you", papers(n)),
    };
    Some((title, lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert(title: &str, why: Why) -> Alert {
        Alert {
            key: title.into(),
            title: title.into(),
            why,
        }
    }

    #[test]
    fn one_summary_says_who_what_and_which_saved_paper_changed() {
        assert_eq!(summary(&[]), None);
        let (t, b) = summary(&[
            alert("A", Why::Author(vec!["Hanna J. Weller".into()])),
            alert(
                "B",
                Why::Author(vec!["Pavel Ondrej".into(), "H. Weller".into()]),
            ),
            alert("C", Why::Keyword(vec!["superradiance".into()])),
            alert("D", Why::Updated("v2".into())),
        ])
        .unwrap();
        assert_eq!(t, "3 new papers for you");
        assert_eq!(
            b,
            "2 papers by Weller, Ondrej\n1 paper mentioning “superradiance”\nSaved: D — new version (v2)"
        );
        let (t, b) = summary(&[alert(
            r"Spin $\alpha$ chains",
            Why::Keyword(vec!["spin".into()]),
        )])
        .unwrap();
        assert_eq!(t, "1 new paper for you");
        assert_eq!(b, "Spin α chains\n1 paper mentioning “spin”");
        let (t, _) = summary(&[
            alert("D", Why::Updated("DOI".into())),
            alert("E", Why::Updated("v3".into())),
        ])
        .unwrap();
        assert_eq!(t, "2 saved papers updated");
    }
}
