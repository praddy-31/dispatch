//! Following: authors and watched keywords, kept in `topics.json` under `follow`
//! (so iCloud sync carries them with the topics).
//!
//! Authors are recognised by name in papers Dispatch fetches anyway (no extra network):
//! surnames must be equal; first names must be equal when both are written out, or start with
//! the same letter when either is an initial. Accents are ignored ("Weißler" = "Weissler").
//! A common name can be narrowed by writing the full first name. For an author outside the
//! user's categories, the card offers a one-off arXiv search (`search_arxiv`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::fetch::Fetch;
use crate::item::Item;
use crate::sources::arxiv_api;
use crate::store::{Row, SearchFilter, Store};
use crate::topics::Topics;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Follow {
    /// As the user wrote them: "Hanna J. Weller", "Ondrej".
    pub authors: Vec<String>,
    /// Words or phrases looked for in new papers' titles and abstracts.
    pub keywords: Vec<String>,
}

impl Follow {
    pub fn is_empty(&self) -> bool {
        self.authors.is_empty() && self.keywords.is_empty()
    }

    /// Trimmed, without empties or duplicates (authors compared by person).
    pub fn cleaned(&self) -> Follow {
        let mut authors: Vec<String> = Vec::new();
        for a in &self.authors {
            let a = a.split_whitespace().collect::<Vec<_>>().join(" ");
            if Name::parse(&a).is_some() && !authors.iter().any(|b| fold(b) == fold(&a)) {
                authors.push(a);
            }
        }
        let mut keywords: Vec<String> = Vec::new();
        for k in &self.keywords {
            let k = k.trim().to_owned();
            if !k.is_empty() && !keywords.iter().any(|x| fold(x) == fold(&k)) {
                keywords.push(k);
            }
        }
        Follow { authors, keywords }
    }

    /// The followed authors (as the user wrote them) who wrote this item.
    pub fn authors_in(&self, item: &Item) -> Vec<String> {
        self.authors
            .iter()
            .filter(|f| item.authors.iter().any(|a| same_person(f, a)))
            .cloned()
            .collect()
    }

    /// The watched keywords in the item's title or abstract.
    pub fn keywords_in(&self, item: &Item) -> Vec<String> {
        let text = fold(&format!("{} {}", item.title, item.summary));
        self.keywords
            .iter()
            .filter(|k| text.contains(&fold(k)))
            .cloned()
            .collect()
    }

    pub fn is_followed(&self, name: &str) -> bool {
        self.authors.iter().any(|f| same_person(f, name))
    }
}

/// Sets `followed` on each row (for the ★-like mark in lists).
pub fn mark(follow: &Follow, rows: &mut [Row]) {
    if follow.authors.is_empty() {
        return;
    }
    for r in rows {
        r.followed = follow.authors_in(&r.item);
    }
}

/// Lower case, accents and other marks removed, ß → ss, so names compare as people type them.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let plain = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä'
            | 'Å' | 'Ā' => "a",
            'æ' | 'Æ' => "ae",
            'ç' | 'ć' | 'č' | 'Ç' | 'Ć' | 'Č' => "c",
            'ď' | 'đ' | 'Ď' | 'Đ' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' | 'È' | 'É' | 'Ê' | 'Ë' | 'Ě' => {
                "e"
            }
            'ğ' | 'Ğ' => "g",
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' | 'Ì' | 'Í' | 'Î' | 'Ï' | 'İ' => "i",
            'ł' | 'ľ' | 'Ł' | 'Ľ' => "l",
            'ñ' | 'ń' | 'ň' | 'Ñ' | 'Ń' | 'Ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø'
            | 'Ő' => "o",
            'œ' | 'Œ' => "oe",
            'ř' | 'Ř' => "r",
            'ś' | 'š' | 'ş' | 'Ś' | 'Š' | 'Ş' => "s",
            'ß' => "ss",
            'ť' | 'ţ' | 'Ť' | 'Ţ' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' | 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ů' | 'Ű' => {
                "u"
            }
            'ý' | 'ÿ' | 'Ý' => "y",
            'ź' | 'ż' | 'ž' | 'Ź' | 'Ż' | 'Ž' => "z",
            _ => {
                out.extend(c.to_lowercase());
                continue;
            }
        };
        out.push_str(plain);
    }
    out
}

/// A name split for comparing: folded surname and first name (or initial).
#[derive(Debug, PartialEq)]
struct Name {
    surname: String,
    first: Option<String>,
}

impl Name {
    /// "Hanna J. Weller", "H. Weller", "Weller, Hanna", "Weller"; a trailing Jr./III dropped.
    fn parse(s: &str) -> Option<Name> {
        let s = fold(s);
        let (surname, first) = match s.split_once(',') {
            Some((last, rest)) => (
                last.trim().to_owned(),
                rest.split_whitespace().next().map(str::to_owned),
            ),
            None => {
                let mut parts: Vec<&str> = s.split_whitespace().collect();
                while parts.len() > 1
                    && matches!(
                        parts.last().map(|p| p.trim_end_matches('.')),
                        Some("jr" | "sr" | "ii" | "iii" | "iv")
                    )
                {
                    parts.pop();
                }
                let last = (*parts.last()?).to_owned();
                let first = (parts.len() > 1).then(|| parts[0].to_owned());
                (last, first)
            }
        };
        let surname = surname
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_owned();
        if surname.is_empty() {
            return None;
        }
        let first = first
            .map(|f| f.trim_matches(|c: char| !c.is_alphanumeric()).to_owned())
            .filter(|f| !f.is_empty());
        Some(Name { surname, first })
    }

    fn initial(&self) -> Option<char> {
        self.first.as_ref().and_then(|f| f.chars().next())
    }
}

fn is_initial(first: &str) -> bool {
    // "s", "s.", "s.f", "j.-p": one letter before any dot or hyphen.
    first
        .split(['.', '-'])
        .next()
        .is_some_and(|p| p.chars().count() <= 1)
}

/// Whether `followed` (as the user wrote it) and `author` (as a paper lists it) are one person.
pub fn same_person(followed: &str, author: &str) -> bool {
    let (Some(f), Some(a)) = (Name::parse(followed), Name::parse(author)) else {
        return false;
    };
    if f.surname != a.surname {
        return false;
    }
    match (&f.first, &a.first) {
        (Some(x), Some(y)) if !is_initial(x) && !is_initial(y) => x == y,
        (Some(_), Some(_)) => f.initial() == a.initial(),
        // Only a surname on either side: the surname decides.
        _ => true,
    }
}

/// An author's card: their papers in the archive and who they write with most.
#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub name: String,
    pub followed: bool,
    pub papers: Vec<Row>,
    /// (name as papers spell it, papers together), most first.
    pub coauthors: Vec<(String, usize)>,
}

pub fn card(store: &Store, follow: &Follow, name: &str) -> Result<Card, String> {
    let n = Name::parse(name).ok_or("no name given")?;
    let query = format!("au:\"{}\"", n.surname);
    let filter = SearchFilter {
        sort: crate::search::Sort::Newest,
        ..SearchFilter::default()
    };
    let mut papers: Vec<Row> = store
        .search(&query, &filter, 2000)?
        .hits
        .into_iter()
        .map(|h| h.row)
        .filter(|r| r.item.authors.iter().any(|a| same_person(name, a)))
        .collect();
    let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for r in &papers {
        for a in &r.item.authors {
            if same_person(name, a) {
                continue;
            }
            let e = counts.entry(fold(a)).or_insert_with(|| (a.clone(), 0));
            e.1 += 1;
        }
    }
    let mut coauthors: Vec<(String, usize)> =
        counts.into_values().filter(|(_, n)| *n > 1).collect();
    coauthors.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    coauthors.truncate(10);
    mark(follow, &mut papers);
    papers.truncate(200);
    Ok(Card {
        name: name.to_owned(),
        followed: follow.is_followed(name),
        papers,
        coauthors,
    })
}

/// Authors in the archive whose name contains every word typed (Search → People), with how
/// many papers each has, most first. Spellings of one person are counted together.
pub fn find_authors(
    store: &Store,
    query: &str,
    limit: usize,
) -> Result<Vec<(String, usize)>, String> {
    let words: Vec<String> = query.split_whitespace().map(fold).collect();
    if words.is_empty() || words.iter().all(|w| w.chars().count() < 2) {
        return Ok(vec![]);
    }
    // A scan of the author lists with `fold`, not the search index: SQLite's tokenizer keeps
    // "ß", so "weiss" wouldn't find "Weißler" there.
    let mut people: Vec<(String, usize)> = Vec::new();
    for authors in store.author_lists()? {
        for a in &authors {
            let f = fold(a);
            if !words.iter().all(|w| f.contains(w.as_str())) {
                continue;
            }
            match people.iter_mut().find(|(p, _)| same_person(p, a)) {
                Some(p) => p.1 += 1,
                None => people.push((a.clone(), 1)),
            }
        }
    }
    people.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    people.truncate(limit);
    Ok(people)
}

/// arXiv's author search for `name`: `au:Weller_H` (surname and initial) or `au:Weller`.
pub fn arxiv_url(name: &str, max: u32) -> Option<String> {
    let n = Name::parse(name)?;
    let surname = n.surname.replace(' ', "_");
    let query = match n.initial() {
        Some(i) => format!("au:{surname}_{i}"),
        None => format!("au:{surname}"),
    };
    let mut url = reqwest::Url::parse("https://export.arxiv.org/api/query").ok()?;
    url.query_pairs_mut()
        .append_pair("search_query", &query)
        .append_pair("sortBy", "submittedDate")
        .append_pair("sortOrder", "descending")
        .append_pair("max_results", &max.to_string());
    Some(url.to_string())
}

/// Papers per author search.
pub const ARXIV_AUTHOR_MAX: u32 = 50;

#[derive(Debug, Clone, Serialize)]
pub struct ArxivSearch {
    /// Papers arXiv returned that are by this person (by name match).
    pub found: usize,
    pub new: usize,
}

/// One arXiv API request for an author's recent papers (the card's button). Stores the
/// papers by that person so the card, Search and (if they match a topic) the lists show them.
/// Honours the same "arXiv asked us to slow down" hour as backfill, and starts it on a 429.
pub fn search_arxiv(
    store: &mut Store,
    topics: &Topics,
    name: &str,
    fetch: &dyn Fetch,
    now: DateTime<Utc>,
) -> Result<ArxivSearch, String> {
    if let Some(until) = store
        .time_meta(crate::refresh::BACKFILL_PAUSED_UNTIL)
        .filter(|t| *t > now)
    {
        return Err(format!(
            "arXiv asked Dispatch to slow down; try again after {}",
            until.with_timezone(&chrono::Local).format("%H:%M")
        ));
    }
    let url = arxiv_url(name, ARXIV_AUTHOR_MAX).ok_or("no name given")?;
    let body = fetch.get_patient(&url).inspect_err(|e| {
        if e == "HTTP 429" || e == "HTTP 503" {
            let _ = store.set_time_meta(
                crate::refresh::BACKFILL_PAUSED_UNTIL,
                now + crate::refresh::BACKFILL_BACK_OFF,
            );
        }
    })?;
    let items: Vec<Item> = arxiv_api::parse(&body)?
        .into_iter()
        .filter(|it| it.authors.iter().any(|a| same_person(name, a)))
        .collect();
    let new = store.upsert(&items, now)?;
    store.retag(topics)?;
    Ok(ArxivSearch {
        found: items.len(),
        new,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_by_surname_and_first_name_or_initial() {
        assert!(same_person("Hanna J. Weller", "H. J. Weller"));
        assert!(same_person("H. Weller", "Hanna J. Weller"));
        assert!(same_person("Weller", "Hanna Weller"));
        assert!(same_person("Nico Weissler", "Nico Weißler"));
        assert!(same_person("Weller, Hanna", "Hanna Weller"));
        assert!(same_person("Jean-Pierre Dupont", "J.-P. Dupont"));
        assert!(same_person("Martin Luther King Jr.", "M. King"));
        assert!(
            !same_person("Hanna Weller", "Helga Weller"),
            "different first names"
        );
        assert!(!same_person("H. Weller", "T. Weller"));
        assert!(!same_person("Weller", "Ondrej"));
        assert!(!same_person("", "Ondrej"));
    }

    #[test]
    fn keywords_and_cleaning() {
        let f = Follow {
            authors: vec!["  Ada  Lovelace ".into(), "ada lovelace".into(), " ".into()],
            keywords: vec!["Superradiance".into(), "superradiance".into(), "".into()],
        }
        .cleaned();
        assert_eq!(f.authors, vec!["Ada Lovelace"]);
        assert_eq!(f.keywords, vec!["Superradiance"]);
        let item = Item {
            key: "arxiv:1".into(),
            section: crate::item::Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "Collective SUPERRADIANCE".into(),
            authors: vec!["A. Lovelace".into(), "C. Babbage".into()],
            summary: String::new(),
            link: String::new(),
            pdf: None,
            categories: vec![],
            published: None,
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        };
        assert_eq!(f.authors_in(&item), vec!["Ada Lovelace"]);
        assert_eq!(f.keywords_in(&item), vec!["Superradiance"]);
    }

    #[test]
    fn the_arxiv_author_query_uses_surname_and_initial() {
        let u = arxiv_url("Hanna J. Weller", 50).unwrap();
        assert!(u.contains("search_query=au%3Aweller_h"), "{u}");
        let u = arxiv_url("Weißler", 50).unwrap();
        assert!(u.contains("au%3Aweissler&"), "{u}");
        assert_eq!(arxiv_url("  ", 50), None);
    }
}
