//! Send to Zotero, local: hands papers to the Zotero app running on this Mac
//! through the same local port its browser connector uses (`127.0.0.1:23119`). No account,
//! no API key, nothing leaves the Mac. Zotero must be open. The port's interface is Zotero's
//! own and not formally documented, so a failure says so and points to "Open in Zotero"
//! (the BibTeX hand-over), which always works.
//!
//! **Web API:** with an API key from zotero.org (kept in the macOS
//! Keychain by the app, never in a file), papers go to `api.zotero.org` when the Zotero app
//! isn't open, into a collection chosen in Settings. The local app is always tried first
//!.

use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Item;

pub const BASE: &str = "http://127.0.0.1:23119/connector";

/// Zotero's item JSON for one arXiv paper: a preprint with its authors, abstract, date,
/// link, arXiv id and DOI (and the journal, once published).
pub fn item_json(item: &Item) -> Value {
    let id = item.key.strip_prefix("arxiv:").unwrap_or(&item.key);
    let creators: Vec<Value> = item
        .authors
        .iter()
        .map(|name| {
            let name = name.trim();
            match name.rsplit_once(' ') {
                Some((first, last)) => {
                    json!({ "firstName": first, "lastName": last, "creatorType": "author" })
                }
                None => json!({ "lastName": name, "creatorType": "author", "fieldMode": 1 }),
            }
        })
        .collect();
    let mut v = json!({
        "itemType": "preprint",
        "title": item.title,
        "creators": creators,
        "abstractNote": item.summary,
        "url": item.link,
        "repository": "arXiv",
        "archiveID": format!("arXiv:{id}"),
        "libraryCatalog": "arXiv.org",
        "accessDate": Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        "tags": item.categories.iter().map(|c| json!({ "tag": c })).collect::<Vec<_>>(),
    });
    if let Some(p) = item.published {
        v["date"] = json!(p.format("%Y-%m-%d").to_string());
    }
    if let Some(doi) = &item.doi {
        v["DOI"] = json!(doi);
    }
    if let Some(j) = &item.journal_ref {
        v["extra"] = json!(format!("Published in: {j}"));
    }
    v
}

/// The request body for `saveItems`.
pub fn payload(items: &[Item]) -> Value {
    json!({
        "sessionID": format!("dispatch-{}", Utc::now().timestamp_millis()),
        "uri": items.first().map(|i| i.link.as_str()).unwrap_or("https://arxiv.org"),
        "items": items.iter().enumerate().map(|(n, i)| {
            let mut v = item_json(i);
            v["id"] = json!(format!("dispatch-{n}"));
            v
        }).collect::<Vec<_>>(),
    })
}

/// Is Zotero open and listening? (A quick local request; false when it's closed.)
pub fn running() -> bool {
    client()
        .and_then(|c| {
            c.get(format!("{BASE}/ping"))
                .send()
                .map_err(|e| e.to_string())
        })
        .is_ok_and(|r| r.status().is_success())
}

/// Sends the papers to the Zotero app; they land in the collection selected there.
pub fn send(items: &[Item]) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }
    let c = client()?;
    let resp = c
        .post(format!("{BASE}/saveItems"))
        .header("Content-Type", "application/json")
        .header("X-Zotero-Connector-API-Version", "3")
        .body(payload(items).to_string())
        .send()
        .inspect(|_| crate::netlog::record(BASE, true))
        .map_err(|e| {
            if e.is_connect() {
                "Zotero isn't open (start it and try again)".to_owned()
            } else {
                e.to_string()
            }
        })?;
    match resp.status().as_u16() {
        200 | 201 => Ok(()),
        // Zotero is busy with a dialog, or a newer Zotero changed its local interface.
        code => Err(format!(
            "Zotero answered HTTP {code}; use Open in Zotero (BibTeX) instead"
        )),
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(crate::fetch::USER_AGENT)
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(2))
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())
}

// ---- web API ----------------------------------------------------------

pub const WEB: &str = "https://api.zotero.org";
/// Items per write request (the API's limit).
pub const WEB_BATCH: usize = 50;

/// Who a key belongs to and whether it may add items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebKey {
    pub user_id: u64,
    pub username: String,
    pub can_write: bool,
}

/// One collection, with its full path ("Thesis / Chapter 2") for a menu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Collection {
    pub key: String,
    pub path: String,
}

/// `GET /keys/current`'s answer.
pub fn parse_key(json: &str) -> Result<WebKey, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Zotero's answer: {e}"))?;
    let user_id = v["userID"]
        .as_u64()
        .ok_or("Zotero didn't say whose key this is")?;
    Ok(WebKey {
        user_id,
        username: v["username"].as_str().unwrap_or_default().to_owned(),
        can_write: v["access"]["user"]["library"].as_bool() == Some(true)
            && v["access"]["user"]["write"].as_bool() == Some(true),
    })
}

/// `GET /users/<id>/collections`'s answer, as paths sorted by name.
pub fn parse_collections(json: &str) -> Result<Vec<Collection>, String> {
    let v: Vec<Value> = serde_json::from_str(json).map_err(|e| format!("Zotero's answer: {e}"))?;
    let mut by_key = std::collections::HashMap::new();
    for c in &v {
        let d = &c["data"];
        let key = d["key"].as_str().unwrap_or_default().to_owned();
        let name = d["name"].as_str().unwrap_or_default().to_owned();
        let parent = d["parentCollection"].as_str().map(str::to_owned);
        by_key.insert(key, (name, parent));
    }
    let path = |key: &str| {
        let mut parts = vec![];
        let mut at = Some(key.to_owned());
        while let Some(k) = at {
            let Some((name, parent)) = by_key.get(&k) else {
                break;
            };
            parts.insert(0, name.clone());
            at = parent.clone();
            if parts.len() > 20 {
                break; // a loop in the data; never seen, but don't hang
            }
        }
        parts.join(" / ")
    };
    let mut out: Vec<Collection> = by_key
        .keys()
        .map(|k| Collection {
            key: k.clone(),
            path: path(k),
        })
        .collect();
    out.sort_by_key(|c| c.path.to_lowercase());
    Ok(out)
}

/// The items for a write request: the same preprints, filed in `collection` if given.
pub fn web_items(items: &[Item], collection: Option<&str>) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| {
                let mut v = item_json(i);
                v["accessDate"] = json!(Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
                if let Some(c) = collection {
                    v["collections"] = json!([c]);
                }
                v
            })
            .collect(),
    )
}

/// How many of a write's items Zotero refused, with its first reason.
pub fn parse_write(json: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("Zotero's answer: {e}"))?;
    match v["failed"].as_object() {
        Some(f) if !f.is_empty() => {
            let why = f
                .values()
                .next()
                .and_then(|x| x["message"].as_str())
                .unwrap_or("no reason given");
            Err(format!("Zotero refused {} of them: {why}", f.len()))
        }
        _ => Ok(()),
    }
}

fn web_get(key: &str, path: &str) -> Result<String, String> {
    let resp = web_client()?
        .get(format!("{WEB}{path}"))
        .header("Zotero-API-Key", key)
        .header("Zotero-API-Version", "3")
        .send();
    crate::netlog::record(WEB, resp.is_ok());
    web_answer(resp.map_err(|e| format!("can't reach api.zotero.org: {e}"))?)
}

fn web_answer(resp: reqwest::blocking::Response) -> Result<String, String> {
    let code = resp.status().as_u16();
    let text = resp.text().map_err(|e| e.to_string())?;
    match code {
        200..=299 => Ok(text),
        403 => Err("Zotero refused the key (wrong key, or it can't write to your library)".into()),
        429 | 503 => Err("Zotero is busy; try again in a minute".into()),
        _ => Err(format!("Zotero answered HTTP {code}: {}", text.trim())),
    }
}

/// Checks an API key: whose it is and whether it may write.
pub fn check_key(key: &str) -> Result<WebKey, String> {
    parse_key(&web_get(key, "/keys/current")?)
}

/// The library's collections (up to 100, which covers most libraries).
pub fn collections(user_id: u64, key: &str) -> Result<Vec<Collection>, String> {
    parse_collections(&web_get(
        key,
        &format!("/users/{user_id}/collections?limit=100"),
    )?)
}

/// Adds the papers to the web library, 50 at a time.
pub fn web_send(
    user_id: u64,
    key: &str,
    items: &[Item],
    collection: Option<&str>,
) -> Result<(), String> {
    let c = web_client()?;
    for chunk in items.chunks(WEB_BATCH) {
        let resp = c
            .post(format!("{WEB}/users/{user_id}/items"))
            .header("Zotero-API-Key", key)
            .header("Zotero-API-Version", "3")
            .header("Content-Type", "application/json")
            .body(web_items(chunk, collection).to_string())
            .send();
        crate::netlog::record(WEB, resp.is_ok());
        let resp = resp.map_err(|e| format!("can't reach api.zotero.org: {e}"))?;
        parse_write(&web_answer(resp)?)?;
    }
    Ok(())
}

fn web_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(crate::fetch::USER_AGENT)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Section;

    #[test]
    fn a_paper_becomes_a_zotero_preprint() {
        let item = Item {
            key: "arxiv:2410.01234".into(),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "Spin chains".into(),
            authors: vec!["Ada M. Lovelace".into(), "Plato".into()],
            summary: "We study $H$.".into(),
            link: "https://arxiv.org/abs/2410.01234".into(),
            pdf: None,
            categories: vec!["quant-ph".into()],
            published: Some("2024-10-02T00:00:00Z".parse().unwrap()),
            announce_type: Some("new".into()),
            version: Some(2),
            doi: Some("10.1000/x".into()),
            journal_ref: Some("Phys. Rev. B 1, 2 (2025)".into()),
        };
        let v = item_json(&item);
        assert_eq!(v["itemType"], "preprint");
        assert_eq!(v["archiveID"], "arXiv:2410.01234");
        assert_eq!(v["creators"][0]["lastName"], "Lovelace");
        assert_eq!(v["creators"][0]["firstName"], "Ada M.");
        assert_eq!(v["creators"][1]["fieldMode"], 1);
        assert_eq!(v["date"], "2024-10-02");
        assert_eq!(v["DOI"], "10.1000/x");
        let p = payload(&[item]);
        assert_eq!(p["items"].as_array().unwrap().len(), 1);
        assert_eq!(p["items"][0]["id"], "dispatch-0");
    }

    #[test]
    fn web_api_answers_are_read() {
        let k = parse_key(
            r#"{"key":"x","userID":1234567,"username":"demo-user","access":{"user":{"library":true,"files":true,"notes":true,"write":true}}}"#,
        )
        .unwrap();
        assert_eq!(
            (k.user_id, k.username.as_str(), k.can_write),
            (1234567, "demo-user", true)
        );
        let read_only = parse_key(r#"{"userID":1,"access":{"user":{"library":true}}}"#).unwrap();
        assert!(!read_only.can_write);

        let cols = parse_collections(
            r#"[{"key":"B","data":{"key":"B","name":"chapter 2","parentCollection":"A"}},
                {"key":"A","data":{"key":"A","name":"Thesis","parentCollection":false}},
                {"key":"C","data":{"key":"C","name":"Reading","parentCollection":false}}]"#,
        )
        .unwrap();
        let paths: Vec<_> = cols
            .iter()
            .map(|c| (c.key.as_str(), c.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![
                ("C", "Reading"),
                ("A", "Thesis"),
                ("B", "Thesis / chapter 2")
            ]
        );

        assert!(parse_write(r#"{"successful":{"0":{}},"failed":{}}"#).is_ok());
        let err =
            parse_write(r#"{"failed":{"0":{"code":400,"message":"bad field"}}}"#).unwrap_err();
        assert!(err.contains("bad field"));
    }

    #[test]
    fn web_items_are_filed_in_the_collection() {
        let item = Item {
            key: "arxiv:2410.01234".into(),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "Spin chains".into(),
            authors: vec!["Ada Lovelace".into()],
            summary: String::new(),
            link: "https://arxiv.org/abs/2410.01234".into(),
            pdf: None,
            categories: vec![],
            published: None,
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        };
        let v = web_items(std::slice::from_ref(&item), Some("ABCD"));
        assert_eq!(v[0]["collections"][0], "ABCD");
        assert_eq!(v[0]["itemType"], "preprint");
        assert!(v[0]["accessDate"].as_str().unwrap().ends_with('Z'));
        assert!(web_items(&[item], None)[0].get("collections").is_none());
    }
}
