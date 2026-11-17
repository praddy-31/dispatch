//! Any RSS or Atom news feed, through feed-rs.

use std::collections::HashMap;

use crate::item::{Item, Section, plain_text};

pub fn parse(xml: &[u8], url: &str, name: &str) -> Result<Vec<Item>, String> {
    let feed = feed_rs::parser::parse(xml).map_err(|e| format!("not a feed: {e}"))?;
    let comments = rss_comments(xml);
    let mut items: Vec<Item> = Vec::new();
    for entry in feed.entries {
        let replies = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref() == Some("replies"))
            .map(|l| l.href.clone());
        let link = entry
            .links
            .iter()
            .find(|l| l.rel.as_deref().is_none_or(|r| r == "alternate"))
            .or(entry.links.first())
            .map(|l| l.href.clone())
            .unwrap_or_default();
        // The article's link is its identity. feed-rs makes up an id (a hash of the
        // contents) when a feed has no <guid>, which changes if a headline is edited.
        let id = if link.is_empty() {
            entry.id.clone()
        } else {
            link.clone()
        };
        if id.is_empty() {
            continue;
        }
        let key = format!("news:{id}");
        if items.iter().any(|i| i.key == key) {
            continue;
        }
        let summary = entry
            .summary
            .map(|s| s.content)
            .or_else(|| entry.content.and_then(|c| c.body))
            .unwrap_or_default();
        items.push(Item {
            key,
            section: Section::News,
            source: url.to_owned(),
            source_name: name.to_owned(),
            title: plain_text(&entry.title.map(|t| t.content).unwrap_or_default()),
            authors: entry.authors.iter().map(|a| a.name.clone()).collect(),
            summary: with_discussion(
                plain_text(&summary),
                comments.get(&link).or(replies.as_ref()).map(String::as_str),
            ),
            link,
            pdf: None,
            categories: entry.categories.iter().map(|c| c.term.clone()).collect(),
            published: entry.published.or(entry.updated),
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        });
    }
    Ok(items)
}

/// The marker a story's discussion link is kept under, at the end of its summary
/// (`news::discussion` reads it back). Older stored stories use the same words.
pub const DISCUSSION_MARKER: &str = "Comments URL:";

/// Keeps a story's discussion page, if its feed names one, at the end of the summary.
/// RSS 2.0 has `<comments>` and Atom has `<link rel="replies">`; feed-rs doesn't keep the
/// first, so it comes from `rss_comments` (keyed by the story's link).
fn with_discussion(text: String, discussion: Option<&str>) -> String {
    match discussion {
        Some(url)
            if (url.starts_with("https://") || url.starts_with("http://"))
                && !text.contains(DISCUSSION_MARKER) =>
        {
            format!("{text}\n{DISCUSSION_MARKER} {url}")
        }
        _ => text,
    }
}

/// Each RSS `<item>`'s `<link>` → its `<comments>` URL, read straight from the XML.
fn rss_comments(xml: &[u8]) -> HashMap<String, String> {
    let xml = String::from_utf8_lossy(xml);
    let mut out = HashMap::new();
    for block in xml.split("<item").skip(1) {
        let block = block.split("</item>").next().unwrap_or("");
        if let (Some(link), Some(comments)) = (tag_text(block, "link"), tag_text(block, "comments"))
        {
            out.insert(link, comments);
        }
    }
    out
}

/// The text of the first `<tag>…</tag>` in `xml` (CDATA and `&amp;` undone), if not empty.
fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let len = xml[start..].find(&format!("</{tag}>"))?;
    let raw = xml[start..start + len].trim();
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw)
        .trim()
        .replace("&amp;", "&");
    (!raw.is_empty()).then_some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discussion_links_from_rss_comments_and_atom_replies() {
        let rss = br#"<rss version="2.0"><channel><title>T</title>
            <item><title>A</title><link>https://a.example.org/1</link>
              <description>One story with a summary.</description>
              <comments><![CDATA[https://talk.example.org/1?x=1&amp;y=2]]></comments></item>
            <item><title>B</title><link>https://a.example.org/2</link>
              <description>No discussion here.</description></item>
            </channel></rss>"#;
        let items = parse(rss, "https://a.example.org/rss", "A").unwrap();
        assert_eq!(
            items[0].summary,
            "One story with a summary.\nComments URL: https://talk.example.org/1?x=1&y=2"
        );
        assert_eq!(items[1].summary, "No discussion here.");

        let atom = br#"<feed xmlns="http://www.w3.org/2005/Atom"><title>T</title><id>t</id>
            <updated>2026-10-29T00:00:00Z</updated>
            <entry><title>C</title><id>c</id><updated>2026-10-29T00:00:00Z</updated>
              <link href="https://a.example.org/3"/>
              <link rel="replies" href="https://talk.example.org/3"/>
              <summary>Atom story.</summary></entry></feed>"#;
        let items = parse(atom, "https://a.example.org/atom", "A").unwrap();
        assert_eq!(
            items[0].summary,
            "Atom story.\nComments URL: https://talk.example.org/3"
        );
    }
}
