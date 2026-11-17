//! "Add to Dispatch". A pasted arXiv link or id, or a DOI, becomes a saved paper,
//! from a Shortcut, `dispatch://add?q=…`, Settings → Links or `dispatch-cli add`.
//!
//! - arXiv (`2410.01234`, `arXiv:2410.01234v2`, `https://arxiv.org/abs/…` or `/pdf/…`, and
//!   arXiv's own DOIs `10.48550/arXiv.2410.01234`): one arXiv API request (`id_list`), the
//!   same politeness as the author search (the 1-hour pause after a 429 is honoured).
//! - Any other DOI (`10.5555/demo.1.1`, `doi:…`, `https://doi.org/…`): one doi.org
//!   request for its CSL JSON. Stored as `doi:<doi, lower case>`, in Research with no arXiv
//!   category, so it shows in Saved and Search but in no topic list.
//!
//! A paper Dispatch already has is only saved: no request.

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;

use crate::fetch::Fetch;
use crate::item::{Item, Section, plain_text};
use crate::sources::arxiv_api;
use crate::store::Store;
use crate::topics::Topics;

/// `source` of papers added by DOI (Data health lists them under this name).
pub const DOI_SOURCE: &str = "doi";
pub const DOI_SOURCE_NAME: &str = "Added by DOI";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// An arXiv id without version (`2410.01234`, `hep-th/9901001`).
    Arxiv(String),
    /// A DOI, lower case (`10.1103/physrevlett.1.1`).
    Doi(String),
}

impl Wanted {
    pub fn key(&self) -> String {
        match self {
            Wanted::Arxiv(id) => format!("arxiv:{id}"),
            Wanted::Doi(doi) => format!("doi:{doi}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Added {
    pub key: String,
    pub title: String,
    /// Dispatch didn't have it before.
    pub new: bool,
}

/// Reads what was pasted; an error says what works.
pub fn parse(input: &str) -> Result<Wanted, String> {
    let s = pick(input);
    let s = s.as_str();
    let lower = s.to_ascii_lowercase();
    // DOIs first: arXiv's own become arXiv papers.
    let doi = lower
        .split_once("doi.org/")
        .map(|(_, d)| d)
        .or_else(|| lower.strip_prefix("doi:"))
        .or_else(|| lower.starts_with("10.").then_some(lower.as_str()))
        .map(|d| d.trim().trim_end_matches(['.', ',', ';']));
    if let Some(d) = doi {
        let d = percent_decoded(d);
        if let Some(id) = d.strip_prefix("10.48550/arxiv.") {
            return arxiv_id(id).map(Wanted::Arxiv).ok_or_else(|| bad(input));
        }
        let valid = d.split_once('/').is_some_and(|(prefix, rest)| {
            prefix.len() >= 7
                && prefix[3..].chars().all(|c| c.is_ascii_digit() || c == '.')
                && !rest.is_empty()
                && !rest.contains(char::is_whitespace)
        });
        return if valid {
            Ok(Wanted::Doi(d))
        } else {
            Err(bad(input))
        };
    }
    // arXiv links: abs/ID, pdf/ID(.pdf), html/ID.
    let id = match lower.find("arxiv.org/") {
        Some(i) => {
            let path = &s[i + "arxiv.org/".len()..];
            let path = path.split(['?', '#']).next().unwrap_or("");
            let rest = ["abs/", "pdf/", "html/", "format/"]
                .iter()
                .find_map(|p| path.strip_prefix(p))
                .ok_or_else(|| bad(input))?;
            rest.trim_end_matches('/')
                .trim_end_matches(".pdf")
                .to_owned()
        }
        None => s.to_owned(),
    };
    arxiv_id(&id).map(Wanted::Arxiv).ok_or_else(|| bad(input))
}

/// What a share sheet hands over can be more than the link ("Title — https://…",
/// several lines), and journals put the DOI in their own links
/// (`journals.aps.org/prl/abstract/10.1103/…`). Picks the part that names a paper.
fn pick(input: &str) -> String {
    let clean = |t: &str| {
        t.trim()
            .trim_matches(['<', '>', '"', '\'', '(', ')'])
            .to_owned()
    };
    let tokens: Vec<String> = input.split_whitespace().map(clean).collect();
    let named = |t: &String| {
        let l = t.to_ascii_lowercase();
        l.contains("arxiv.org/") || l.contains("doi.org/") || l.starts_with("doi:")
    };
    if let Some(t) = tokens.iter().find(|t| named(t)) {
        return t.clone();
    }
    // A journal's link with a DOI in its path: from "10." on, without the query.
    for t in tokens.iter().filter(|t| t.starts_with("http")) {
        let path = t.split(['?', '#']).next().unwrap_or("");
        if let Some(i) = path.find("/10.") {
            let doi = &path[i + 1..];
            let looks = doi.split_once('/').is_some_and(|(p, rest)| {
                p.len() >= 7 && p[3..].chars().all(|c| c.is_ascii_digit()) && !rest.is_empty()
            });
            if looks {
                return doi.trim_end_matches('/').to_owned();
            }
        }
    }
    if tokens.len() > 1 {
        let id = tokens.iter().find(|t| {
            let l = t.to_ascii_lowercase();
            l.starts_with("10.") || crate::links::arxiv_key(&l).is_some()
        });
        if let Some(t) = id {
            return t.clone();
        }
    }
    clean(input)
}

fn bad(input: &str) -> String {
    format!(
        "`{}` isn't an arXiv link, arXiv id or DOI",
        input.trim().chars().take(80).collect::<String>()
    )
}

fn percent_decoded(s: &str) -> String {
    s.replace("%2f", "/").replace("%3a", ":")
}

/// `2410.01234v2`, `arXiv:2410.01234`, `hep-th/9901001` → the id without version.
fn arxiv_id(id: &str) -> Option<String> {
    crate::links::arxiv_key(id).map(|k| k.trim_start_matches("arxiv:").to_owned())
}

/// The arXiv API address for one paper.
pub fn arxiv_url(id: &str) -> String {
    format!("https://export.arxiv.org/api/query?id_list={id}&max_results=1")
}

/// CSL JSON (what doi.org answers for `application/vnd.citationstyles.csl+json`) → an item.
pub fn from_csl(doi: &str, json: &[u8]) -> Result<Item, String> {
    let v: serde_json::Value =
        serde_json::from_slice(json).map_err(|_| "doi.org didn't send the paper's details")?;
    // Some fields are a string, some a list of strings.
    let text = |key: &str| -> Option<String> {
        match &v[key] {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).map(String::from),
            _ => None,
        }
        .map(|s| plain_text(&s))
        .filter(|s| !s.is_empty())
    };
    let title = text("title").ok_or("doi.org has no title for this DOI")?;
    let authors = v["author"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let given = p["given"].as_str().unwrap_or("");
                    let family = p["family"].as_str().or(p["literal"].as_str())?;
                    Some(format!("{given} {family}").trim().to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let published = ["published", "issued", "published-print", "published-online"]
        .iter()
        .find_map(|k| {
            let parts = v[*k]["date-parts"].get(0)?.as_array()?;
            let n = |i: usize, or: u32| {
                parts
                    .get(i)
                    .and_then(|x| x.as_u64())
                    .map_or(or, |x| x as u32)
            };
            let year = parts.first()?.as_i64()? as i32;
            Utc.with_ymd_and_hms(year, n(1, 1), n(2, 1), 0, 0, 0)
                .single()
        });
    let journal = text("container-title").map(|j| {
        let mut j = j;
        if let Some(vol) = v["volume"].as_str() {
            j += &format!(" {vol}");
        }
        if let Some(page) = v["page"].as_str().or(v["article-number"].as_str()) {
            j += &format!(", {page}");
        }
        if let Some(y) = published.map(|t| t.format("%Y").to_string()) {
            j += &format!(" ({y})");
        }
        j
    });
    Ok(Item {
        key: format!("doi:{doi}"),
        section: Section::Research,
        source: DOI_SOURCE.into(),
        source_name: DOI_SOURCE_NAME.into(),
        title,
        authors,
        summary: text("abstract").unwrap_or_default(),
        link: format!("https://doi.org/{doi}"),
        pdf: None,
        categories: vec![],
        published,
        announce_type: None,
        version: None,
        doi: Some(doi.to_owned()),
        journal_ref: journal,
    })
}

/// Adds and saves what `input` names (see the module notes). `now` stamps the save.
pub fn add(
    store: &mut Store,
    topics: &Topics,
    input: &str,
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
) -> Result<Added, String> {
    let wanted = parse(input)?;
    let key = wanted.key();
    let existing = store.get(&key)?;
    let new = existing.is_none();
    let item = match existing {
        Some(item) => item,
        None => {
            let item = match &wanted {
                Wanted::Arxiv(id) => fetch_arxiv(store, id, fetch, now)?,
                Wanted::Doi(doi) => from_csl(
                    doi,
                    &fetch.get_as(
                        &format!("https://doi.org/{doi}"),
                        "application/vnd.citationstyles.csl+json",
                    )?,
                )?,
            };
            store.upsert(std::slice::from_ref(&item), now)?;
            store.retag(topics)?;
            item
        }
    };
    store.set_saved(std::slice::from_ref(&key), true, now)?;
    Ok(Added {
        key,
        title: item.title,
        new,
    })
}

fn fetch_arxiv(
    store: &Store,
    id: &str,
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
) -> Result<Item, String> {
    use crate::refresh::{BACKFILL_BACK_OFF, BACKFILL_PAUSED_UNTIL};
    if let Some(until) = store.time_meta(BACKFILL_PAUSED_UNTIL).filter(|t| *t > now) {
        return Err(format!(
            "arXiv asked Dispatch to slow down; try again after {}",
            until.with_timezone(&chrono::Local).format("%H:%M")
        ));
    }
    let body = fetch.get_patient(&arxiv_url(id)).inspect_err(|e| {
        if e == "HTTP 429" || e == "HTTP 503" {
            let _ = store.set_time_meta(BACKFILL_PAUSED_UNTIL, now + BACKFILL_BACK_OFF);
        }
    })?;
    arxiv_api::parse(&body)?
        .into_iter()
        .find(|it| it.key == format!("arxiv:{id}"))
        .ok_or_else(|| format!("arXiv doesn't know {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_share_sheet_hands_over() {
        let arxiv = Ok(Wanted::Arxiv("2410.01234".into()));
        assert_eq!(
            parse("Some title https://arxiv.org/abs/2410.01234v2"),
            arxiv
        );
        assert_eq!(
            parse("https://arxiv.org/pdf/2410.01234v2\nSome title"),
            arxiv
        );
        assert_eq!(parse("https://www.arxiv.org/pdf/2410.01234v2.pdf"), arxiv);
        assert_eq!(
            parse("https://journal.example.org/abstract/10.5555/demo.2016.061102?ft=1"),
            Ok(Wanted::Doi("10.5555/demo.2016.061102".into()))
        );
        assert_eq!(
            parse("A paper (10.5555/demo.2016.061102)"),
            Ok(Wanted::Doi("10.5555/demo.2016.061102".into()))
        );
        assert!(parse("https://publisher.example.com/articles/s1-0001").is_err());
    }

    #[test]
    fn what_can_be_pasted() {
        let arxiv = |id: &str| Ok(Wanted::Arxiv(id.into()));
        assert_eq!(parse("2410.01234"), arxiv("2410.01234"));
        assert_eq!(parse(" arXiv:2410.01234v3 "), arxiv("2410.01234"));
        assert_eq!(
            parse("https://arxiv.org/abs/2410.01234v2"),
            arxiv("2410.01234")
        );
        assert_eq!(
            parse("https://arxiv.org/pdf/2410.01234v1.pdf"),
            arxiv("2410.01234")
        );
        assert_eq!(
            parse("arxiv.org/abs/hep-th/9901001"),
            arxiv("hep-th/9901001")
        );
        assert_eq!(
            parse("https://doi.org/10.48550/arXiv.2410.01234"),
            arxiv("2410.01234")
        );
        assert_eq!(
            parse("https://doi.org/10.5555/demo.2023.010801"),
            Ok(Wanted::Doi("10.5555/demo.2023.010801".into()))
        );
        assert_eq!(
            parse("doi:10.1038/s41586-023-06096-3."),
            Ok(Wanted::Doi("10.1038/s41586-023-06096-3".into()))
        );
        assert!(parse("hello world").is_err());
        assert!(parse("https://example.com/paper").is_err());
        assert!(
            parse("10.12/x").is_err(),
            "a DOI prefix has four digits or more"
        );
    }

    #[test]
    fn a_doi_paper_from_csl_json() {
        let json = br#"{
            "title": "Squeezed light for <i>everyone</i>",
            "author": [{"given": "Ada", "family": "Lovelace"}, {"literal": "The Example Team"}],
            "issued": {"date-parts": [[2023, 5]]},
            "container-title": ["Physical Review Letters"],
            "volume": "130", "article-number": "010801",
            "abstract": "<jats:p>We squeeze.</jats:p>"
        }"#;
        let item = from_csl("10.5555/demo.2023.010801", json).unwrap();
        assert_eq!(item.key, "doi:10.5555/demo.2023.010801");
        assert_eq!(item.title, "Squeezed light for everyone");
        assert_eq!(item.authors, ["Ada Lovelace", "The Example Team"]);
        assert_eq!(item.summary, "We squeeze.");
        assert_eq!(
            item.journal_ref.as_deref(),
            Some("Physical Review Letters 130, 010801 (2023)")
        );
        assert_eq!(
            item.published.map(|t| t.format("%Y-%m-%d").to_string()),
            Some("2023-05-01".into())
        );
        assert!(item.categories.is_empty(), "no topic list shows it");
        assert!(from_csl("10.1/x", b"<html>").is_err());
    }
}
