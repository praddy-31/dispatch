//! arXiv's search API (`export.arxiv.org/api/query`, Atom). Used to backfill recent papers
//! when a category is new to Dispatch (first run, or a category added in the picker), since
//! the daily RSS feed only holds the latest announcement and is empty on weekends and holidays.
//!
//! arXiv asks for at most one API request every 3 seconds; the caller pauses between
//! categories (`refresh::backfill`).

use crate::item::{Item, Section, plain_text};
use crate::sources::arxiv::{self, split_id};

/// Archives with sub-categories: `cs` is searched as `cat:cs.*`.
const ARCHIVES: [&str; 11] = [
    "cs", "math", "physics", "q-bio", "q-fin", "stat", "eess", "econ", "astro-ph", "cond-mat",
    "nlin",
];

/// The newest `max` papers announced in `category`.
pub fn recent_url(category: &str, max: u32) -> String {
    let query = if ARCHIVES.contains(&category) {
        format!("cat:{category}.*")
    } else {
        format!("cat:{category}")
    };
    format!(
        "https://export.arxiv.org/api/query?search_query={query}&sortBy=submittedDate&sortOrder=descending&max_results={max}"
    )
}

pub fn parse(xml: &[u8]) -> Result<Vec<Item>, String> {
    let feed = feed_rs::parser::parse(xml).map_err(|e| format!("not a feed: {e}"))?;
    let text = String::from_utf8_lossy(xml);
    // Fields feed-rs doesn't map, per <entry>, in the same order as feed.entries.
    let extras: Vec<(Option<String>, Option<String>)> = text
        .split("<entry>")
        .skip(1)
        .map(|chunk| {
            let chunk = chunk.split("</entry>").next().unwrap_or("");
            (
                arxiv::tag(chunk, "arxiv:doi"),
                arxiv::tag(chunk, "arxiv:journal_ref").map(|s| plain_text(&s)),
            )
        })
        .collect();
    let mut items = Vec::new();
    for (i, entry) in feed.entries.into_iter().enumerate() {
        let Some((id, version)) = split_id(&entry.id) else {
            continue;
        };
        let (doi, journal_ref) = extras.get(i).cloned().unwrap_or_default();
        items.push(Item {
            key: format!("arxiv:{id}"),
            section: Section::Research,
            source: arxiv::SOURCE_ID.into(),
            source_name: "arXiv".into(),
            title: plain_text(&entry.title.map(|t| t.content).unwrap_or_default()),
            authors: entry
                .authors
                .iter()
                .map(|a| crate::accents::decode(&a.name))
                .collect(),
            summary: plain_text(&entry.summary.map(|s| s.content).unwrap_or_default()),
            link: format!("https://arxiv.org/abs/{id}"),
            pdf: Some(format!("https://arxiv.org/pdf/{id}")),
            categories: entry.categories.iter().map(|c| c.term.clone()).collect(),
            published: entry.published.or(entry.updated),
            announce_type: None,
            version,
            doi,
            journal_ref,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_search_one_category_or_a_whole_archive() {
        assert!(recent_url("quant-ph", 40).contains("search_query=cat:quant-ph&"));
        assert!(recent_url("cs", 40).contains("search_query=cat:cs.*&"));
        assert!(recent_url("cs.LG", 40).contains("cat:cs.LG&"));
        assert!(recent_url("quant-ph", 40).ends_with("max_results=40"));
    }
}
