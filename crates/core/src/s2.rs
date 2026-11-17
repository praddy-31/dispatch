//! Similar papers, references and cited-by from Semantic Scholar
//! (`api.semanticscholar.org`). Opt-in (Settings → Discovery), fetched only when asked from
//! the preview, then cached in the `citations` table (format `s2`), so it works offline after.
//! Works without a key (everyone without one shares a rate limit, so it's slower and
//! sometimes busy); the user's free key, if any, is kept in the Keychain by the app.
//!
//! What goes out: the paper's arXiv id (or DOI) and the key if there is one, nothing else.
//! Three requests per paper, at least `pause_for` apart: references, citations,
//! recommendations. Free of charge.

use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fetch::Fetch;
use crate::item::Item;
use crate::store::Store;

pub const BASE: &str = "https://api.semanticscholar.org";
/// Format name in the `citations` cache table.
pub const CACHE: &str = "s2";
/// Between two requests with a key (Semantic Scholar's free keys: 1 request per second).
pub const PAUSE: Duration = Duration::from_millis(1100);
/// Between two requests without a key: the shared pool is busy, so go gently.
pub const PAUSE_NO_KEY: Duration = Duration::from_secs(3);

/// The pause between requests for this key (or none).
pub fn pause_for(api_key: Option<&str>) -> Duration {
    if api_key.is_some() {
        PAUSE
    } else {
        PAUSE_NO_KEY
    }
}
/// Papers asked for per list.
pub const LIMIT: usize = 100;
/// Similar papers asked for.
pub const SIMILAR: usize = 20;

const FIELDS: &str = "title,year,authors,externalIds,citationCount";

/// One paper in a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Paper {
    pub title: String,
    pub year: Option<i32>,
    /// The first three authors; `more` if there are others.
    pub authors: Vec<String>,
    pub more: bool,
    /// arXiv id without version, if it has one.
    pub arxiv: Option<String>,
    pub doi: Option<String>,
    pub citations: i64,
    /// Filled when the lists are served, not cached: the key if Dispatch has the paper.
    #[serde(default)]
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lists {
    pub similar: Vec<Paper>,
    /// Why there are no similar papers, if the request failed (the other lists still count).
    pub similar_error: Option<String>,
    /// Most cited first.
    pub references: Vec<Paper>,
    pub cited_by: Vec<Paper>,
    /// More than `LIMIT` citing papers exist.
    pub cited_more: bool,
    /// Unix seconds.
    pub fetched: i64,
}

/// How Semantic Scholar names the paper: `arXiv:<id>`, else `DOI:<doi>`.
pub fn paper_id(item: &Item) -> Option<String> {
    crate::cite::arxiv_id(item)
        .map(|id| format!("arXiv:{id}"))
        .or_else(|| {
            item.doi
                .as_deref()
                .filter(|d| !d.trim().is_empty())
                .map(|d| format!("DOI:{}", d.trim()))
        })
}

/// The cached lists, if any (with `key` filled for papers Dispatch has).
pub fn cached(store: &Store, key: &str) -> Option<Lists> {
    let (text, _) = store.citation(key, CACHE)?;
    let mut lists: Lists = serde_json::from_str(&text).ok()?;
    mark_known(store, &mut lists);
    Some(lists)
}

/// Asks Semantic Scholar for the three lists, caches and returns them. `pause` goes between
/// requests (`PAUSE` in the app, zero in tests).
pub fn fetch(
    store: &Store,
    item: &Item,
    http: &dyn Fetch,
    api_key: Option<&str>,
    pause: Duration,
    now: DateTime<Utc>,
) -> Result<Lists, String> {
    let id = paper_id(item).ok_or("this item has no arXiv id or DOI")?;
    // Without a key the shared pool often answers 429: wait and ask again, twice at most
    //. The waits grow with `pause` (zero in tests).
    let get = |url: String| {
        let mut tries = 0;
        loop {
            match get_json(http, &url, api_key) {
                Err(e) if api_key.is_none() && e.contains("shares one limit") && tries < 2 => {
                    tries += 1;
                    std::thread::sleep(pause * (2 * tries));
                }
                other => break other,
            }
        }
    };
    let refs = get(format!(
        "{BASE}/graph/v1/paper/{id}/references?fields={FIELDS}&limit={LIMIT}"
    ))?;
    std::thread::sleep(pause);
    let cites = get(format!(
        "{BASE}/graph/v1/paper/{id}/citations?fields={FIELDS}&limit={LIMIT}"
    ))?;
    std::thread::sleep(pause);
    let (similar, similar_error) = match get(format!(
        "{BASE}/recommendations/v1/papers/forpaper/{id}?fields={FIELDS}&limit={SIMILAR}"
    )) {
        Ok(v) => (papers(&v["recommendedPapers"], None), None),
        Err(e) => (vec![], Some(e)),
    };
    let mut references = papers(&refs["data"], Some("citedPaper"));
    references.sort_by_key(|p| std::cmp::Reverse(p.citations));
    let mut cited_by = papers(&cites["data"], Some("citingPaper"));
    cited_by.sort_by_key(|p| std::cmp::Reverse(p.citations));
    let mut lists = Lists {
        similar,
        similar_error,
        references,
        cited_by,
        cited_more: cites.get("next").is_some_and(|n| !n.is_null()),
        fetched: now.timestamp(),
    };
    let text = serde_json::to_string(&lists).map_err(|e| e.to_string())?;
    store.set_citation(&item.key, CACHE, &text, "semanticscholar", now)?;
    mark_known(store, &mut lists);
    Ok(lists)
}

/// Settings → "Connect": one small request with the key (a famous paper's title).
pub fn check_key(http: &dyn Fetch, api_key: &str) -> Result<(), String> {
    get_json(
        http,
        &format!("{BASE}/graph/v1/paper/arXiv:1706.03762?fields=title"),
        Some(api_key),
    )
    .map(|_| ())
}

fn papers(list: &Value, inner: Option<&str>) -> Vec<Paper> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| paper(inner.map_or(v, |k| &v[k])))
        .collect()
}

fn paper(v: &Value) -> Option<Paper> {
    let title = v["title"].as_str()?.trim().to_owned();
    if title.is_empty() {
        return None;
    }
    let names: Vec<String> = v["authors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["name"].as_str().map(str::to_owned))
        .collect();
    let ids = &v["externalIds"];
    Some(Paper {
        title,
        year: v["year"].as_i64().map(|y| y as i32),
        more: names.len() > 3,
        authors: names.into_iter().take(3).collect(),
        arxiv: ids["ArXiv"].as_str().map(strip_version),
        doi: ids["DOI"]
            .as_str()
            .map(str::to_owned)
            .filter(|d| !d.to_ascii_lowercase().starts_with("10.48550/")),
        citations: v["citationCount"].as_i64().unwrap_or(0),
        key: None,
    })
}

fn strip_version(id: &str) -> String {
    match id.rfind('v') {
        Some(i) if i > 0 && i + 1 < id.len() && id[i + 1..].chars().all(|c| c.is_ascii_digit()) => {
            id[..i].to_owned()
        }
        _ => id.to_owned(),
    }
}

/// Papers in the lists that Dispatch has get their key, so a click shows them in Dispatch.
fn mark_known(store: &Store, lists: &mut Lists) {
    let all = lists
        .similar
        .iter_mut()
        .chain(lists.references.iter_mut())
        .chain(lists.cited_by.iter_mut());
    for p in all {
        p.key = p
            .arxiv
            .as_ref()
            .map(|id| format!("arxiv:{id}"))
            .filter(|k| matches!(store.get(k), Ok(Some(_))));
    }
}

fn get_json(http: &dyn Fetch, url: &str, api_key: Option<&str>) -> Result<Value, String> {
    let answer = match api_key {
        Some(k) => http.get_with(url, ("x-api-key", k)),
        None => http.get(url),
    };
    let bytes = answer.map_err(|e| {
        if e.contains("HTTP 429") && api_key.is_none() {
            "Semantic Scholar is busy (without a key, everyone shares one limit); try again in a minute".to_owned()
        } else if e.contains("HTTP 429") {
            "Semantic Scholar is busy (a free key allows about one request a second); try again in a moment".to_owned()
        } else if e.contains("HTTP 403") || e.contains("HTTP 401") {
            "Semantic Scholar didn't accept the API key; check it in Settings → Discovery".to_owned()
        } else if e.contains("HTTP 404") {
            "Semantic Scholar doesn't know this paper yet (new papers take a day or two)".to_owned()
        } else {
            format!("Semantic Scholar: {e}")
        }
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Semantic Scholar sent something unexpected ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Section;
    use std::sync::Mutex;

    /// Answers by URL fragment; records each URL and the key header.
    struct Fake {
        answers: Vec<(&'static str, Result<&'static str, &'static str>)>,
        asked: Mutex<Vec<(String, String)>>,
    }
    impl Fetch for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.get_with(url, ("no", "key"))
        }
        fn get_with(&self, url: &str, header: (&str, &str)) -> Result<Vec<u8>, String> {
            self.asked
                .lock()
                .unwrap()
                .push((url.into(), format!("{}={}", header.0, header.1)));
            self.answers
                .iter()
                .find(|(frag, _)| url.contains(frag))
                .map(|(_, a)| a.map(|t| t.as_bytes().to_vec()).map_err(String::from))
                .unwrap_or_else(|| Err("HTTP 404".into()))
        }
    }

    fn item(key: &str) -> Item {
        Item {
            key: key.into(),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "A paper".into(),
            authors: vec!["Jakob Runge".into()],
            summary: String::new(),
            link: String::new(),
            pdf: None,
            categories: vec![],
            published: None,
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        }
    }

    const REFS: &str = r#"{"offset":0,"data":[
        {"citedPaper":{"paperId":null,"title":null}},
        {"citedPaper":{"title":"PCMCI","year":2019,"citationCount":5,"authors":[],
          "externalIds":{"ArXiv":"1702.07007v2","DOI":"10.48550/arXiv.1702.07007"}}},
        {"citedPaper":{"title":"Granger causality","year":1969,"citationCount":900,
          "authors":[{"name":"C. Granger"}],"externalIds":{"DOI":"10.2307/1912791"}}}]}"#;
    const CITES: &str = r#"{"offset":0,"next":100,"data":[
        {"citingPaper":{"title":"Causal discovery review","year":2023,"citationCount":134,
          "authors":[{"name":"A One"},{"name":"B Two"},{"name":"C Three"},{"name":"D Four"}],
          "externalIds":{"ArXiv":"2305.13341"}}}]}"#;

    #[test]
    fn three_lists_cached_and_marked() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(&dir.path().join("t.sqlite")).unwrap();
        store
            .upsert(&[item("arxiv:2305.13341")], Utc::now())
            .unwrap();
        let http = Fake {
            answers: vec![
                ("paper/arXiv:2003.03685/references", Ok(REFS)),
                ("paper/arXiv:2003.03685/citations", Ok(CITES)),
                ("forpaper/arXiv:2003.03685", Err("HTTP 429")),
            ],
            asked: Mutex::new(vec![]),
        };
        let p = item("arxiv:2003.03685");
        assert!(cached(&store, &p.key).is_none());
        let l = fetch(&store, &p, &http, Some("k3y"), Duration::ZERO, Utc::now()).unwrap();
        // Untitled references are dropped; most cited first; arXiv DOIs become arXiv ids.
        assert_eq!(l.references.len(), 2);
        assert_eq!(l.references[0].title, "Granger causality");
        assert_eq!(l.references[1].arxiv.as_deref(), Some("1702.07007"));
        assert_eq!(l.references[1].doi, None);
        let c = &l.cited_by[0];
        assert!(l.cited_more && c.more);
        assert_eq!(c.authors.len(), 3);
        assert_eq!(c.key.as_deref(), Some("arxiv:2305.13341"));
        // Similar failed on its own; the rest still came.
        assert!(l.similar.is_empty());
        assert!(l.similar_error.as_deref().unwrap().contains("busy"));
        let asked = http.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 3);
        assert!(asked.iter().all(|(_, h)| h == "x-api-key=k3y"));
        assert_eq!(cached(&store, &p.key).unwrap(), l);
    }

    #[test]
    fn without_a_key_no_header_and_a_longer_pause() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite")).unwrap();
        let http = Fake {
            answers: vec![("references", Ok(REFS)), ("citations", Err("HTTP 429"))],
            asked: Mutex::new(vec![]),
        };
        let e = fetch(
            &store,
            &item("arxiv:2003.03685"),
            &http,
            None,
            Duration::ZERO,
            Utc::now(),
        )
        .unwrap_err();
        assert!(e.contains("shares one limit"), "{e}");
        assert!(
            http.asked
                .lock()
                .unwrap()
                .iter()
                .all(|(_, h)| h == "no=key")
        );
        assert!(pause_for(None) > pause_for(Some("k")));
    }

    #[test]
    fn unknown_paper_and_bad_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite")).unwrap();
        let none = Fake {
            answers: vec![],
            asked: Mutex::new(vec![]),
        };
        let e = fetch(
            &store,
            &item("arxiv:2611.00001"),
            &none,
            Some("k"),
            Duration::ZERO,
            Utc::now(),
        )
        .unwrap_err();
        assert!(e.contains("doesn't know"), "{e}");
        assert!(store.citation("arxiv:2611.00001", CACHE).is_none());
        let bad = Fake {
            answers: vec![("1706.03762", Err("HTTP 403"))],
            asked: Mutex::new(vec![]),
        };
        assert!(check_key(&bad, "x").unwrap_err().contains("didn't accept"));
        let mut news = item("news:https://x");
        assert_eq!(paper_id(&news), None);
        news.doi = Some("10.1103/X.1".into());
        assert_eq!(paper_id(&news).as_deref(), Some("DOI:10.1103/X.1"));
    }
}
