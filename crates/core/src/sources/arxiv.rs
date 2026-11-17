//! arXiv's daily RSS feeds (`rss.arxiv.org`). One request covers several categories:
//! `https://rss.arxiv.org/rss/quant-ph+physics.optics`. Feeds are rebuilt at midnight US
//! Eastern and are empty at weekends and on holidays; an empty feed is a success.

use crate::item::{Item, Section, plain_text};

pub const SOURCE_ID: &str = "arxiv";

pub fn feed_url(categories: &[String]) -> String {
    format!("https://rss.arxiv.org/rss/{}", categories.join("+"))
}

/// `https://arxiv.org/abs/2610.91665v2` or `oai:arXiv.org:2610.91665v2` → (`2610.91665`, 2).
/// Old-style ids (`hep-th/9901001v1`) work the same way.
pub fn split_id(s: &str) -> Option<(String, Option<u32>)> {
    let tail = s
        .rsplit_once("/abs/")
        .map(|(_, t)| t)
        .or_else(|| s.strip_prefix("oai:arXiv.org:"))
        .unwrap_or(s)
        .trim();
    if tail.is_empty() {
        return None;
    }
    if let Some((base, v)) = tail.rsplit_once('v')
        && !base.is_empty()
        && !v.is_empty()
        && v.chars().all(|c| c.is_ascii_digit())
        && base.ends_with(|c: char| c.is_ascii_digit())
    {
        return Some((base.to_owned(), v.parse().ok()));
    }
    Some((tail.to_owned(), None))
}

/// Parses an arXiv RSS document. Repeated papers (a cross-listing reached through two of
/// the requested categories) come back once.
pub fn parse(xml: &[u8]) -> Result<Vec<Item>, String> {
    let feed = feed_rs::parser::parse(xml).map_err(|e| format!("not a feed: {e}"))?;
    let extra = extensions(xml);
    let mut items: Vec<Item> = Vec::new();
    for entry in feed.entries {
        let link = entry
            .links
            .first()
            .map(|l| l.href.clone())
            .unwrap_or_default();
        let Some((id, guid_version)) = split_id(&entry.id).or_else(|| split_id(&link)) else {
            continue;
        };
        let key = format!("arxiv:{id}");
        if items.iter().any(|i| i.key == key) {
            continue;
        }
        let description = entry.summary.map(|s| s.content).unwrap_or_default();
        let (announce, abstract_text) = split_description(&description);
        let ext = extra.iter().find(|e| e.id == id);
        let authors: Vec<String> = match ext.map(|e| e.creators.clone()) {
            Some(c) if !c.is_empty() => c,
            _ => entry.authors.iter().map(|a| a.name.clone()).collect(),
        };
        let authors = authors.iter().map(|a| crate::accents::decode(a)).collect();
        items.push(Item {
            key,
            section: Section::Research,
            source: SOURCE_ID.into(),
            source_name: "arXiv".into(),
            title: plain_text(&entry.title.map(|t| t.content).unwrap_or_default()),
            authors,
            summary: plain_text(&abstract_text),
            link: format!("https://arxiv.org/abs/{id}"),
            pdf: Some(format!("https://arxiv.org/pdf/{id}")),
            categories: entry.categories.iter().map(|c| c.term.clone()).collect(),
            published: entry.published.or(entry.updated),
            announce_type: ext.and_then(|e| e.announce_type.clone()).or(announce),
            version: guid_version,
            doi: ext.and_then(|e| e.doi.clone()),
            journal_ref: ext.and_then(|e| e.journal_ref.clone()),
        });
    }
    Ok(items)
}

/// `arXiv:2610.91665v1 Announce Type: new \nAbstract: …` → (`new`, `…`).
fn split_description(d: &str) -> (Option<String>, String) {
    let announce = d.split("Announce Type:").nth(1).and_then(|rest| {
        rest.split_whitespace()
            .next()
            .map(|s| s.trim_end_matches(',').to_owned())
    });
    let body = match d.split_once("Abstract:") {
        Some((_, abs)) => abs.trim().to_owned(),
        None => d.trim().to_owned(),
    };
    (announce, body)
}

/// Fields feed-rs doesn't map: `dc:creator` (one comma-separated author list),
/// `arxiv:announce_type`, `arxiv:DOI`, `arxiv:journal_reference`.
#[derive(Debug, Default)]
struct Extra {
    id: String,
    creators: Vec<String>,
    announce_type: Option<String>,
    doi: Option<String>,
    journal_ref: Option<String>,
}

fn extensions(xml: &[u8]) -> Vec<Extra> {
    let text = String::from_utf8_lossy(xml);
    let mut out = Vec::new();
    for chunk in text.split("<item>").skip(1) {
        let chunk = chunk.split("</item>").next().unwrap_or("");
        let Some(id) = tag(chunk, "guid")
            .and_then(|g| split_id(&g))
            .map(|(id, _)| id)
        else {
            continue;
        };
        let creators = tag(chunk, "dc:creator")
            .map(|c| {
                plain_text(&c)
                    .split(", ")
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        out.push(Extra {
            id,
            creators,
            announce_type: tag(chunk, "arxiv:announce_type"),
            doi: tag(chunk, "arxiv:DOI"),
            journal_ref: tag(chunk, "arxiv:journal_reference").map(|s| plain_text(&s)),
        });
    }
    out
}

/// Text of the first `<name …>…</name>` in `s`, trimmed; None if missing or empty.
pub(crate) fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let start = s.find(&open)?;
    let after = &s[start + open.len()..];
    // The next character must end the tag name (`>` or a space before attributes).
    if !after.starts_with('>') && !after.starts_with(' ') {
        return None;
    }
    let body = &after[after.find('>')? + 1..];
    let end = body.find(&format!("</{name}>"))?;
    let v = body[..end].trim();
    (!v.is_empty()).then(|| v.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_ids_and_versions() {
        assert_eq!(
            split_id("oai:arXiv.org:2610.91665v12"),
            Some(("2610.91665".into(), Some(12)))
        );
        assert_eq!(
            split_id("https://arxiv.org/abs/2610.91665"),
            Some(("2610.91665".into(), None))
        );
        assert_eq!(
            split_id("http://arxiv.org/abs/hep-th/9901001v1"),
            Some(("hep-th/9901001".into(), Some(1)))
        );
    }

    #[test]
    fn feed_url_joins_categories() {
        assert_eq!(
            feed_url(&["quant-ph".into(), "physics.optics".into()]),
            "https://rss.arxiv.org/rss/quant-ph+physics.optics"
        );
    }
}
