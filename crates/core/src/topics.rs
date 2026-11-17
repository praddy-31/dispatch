//! Topics: what the user follows. Kept in `topics.json` in the data folder (plain JSON, edited
//! by the topic picker or by hand), seeded once from `default_topics.json`. The defaults are a
//! small research starter set and no news feeds: each user adds the feeds they want.
//!
//! - A research topic is a group of arXiv categories; each can be switched off on its own.
//! - `include` (at least one must match, when any are given) and `exclude` (none may match)
//!   are checked case-insensitively in the title and abstract.
//! - `category_include` narrows one category inside a topic: "Language models" takes cs.LG
//!   papers only when they mention language models.
//! - A news topic is a list of feeds.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::item::{Item, Section};
use crate::paths::write_atomic;

pub const DEFAULT_JSON: &str = include_str!("default_topics.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Topics {
    pub version: u32,
    pub research: Vec<ResearchTopic>,
    pub news: Vec<NewsTopic>,
    /// Followed authors and watched keywords, edited in Settings, not the picker.
    #[serde(default, skip_serializing_if = "crate::follow::Follow::is_empty")]
    pub follow: crate::follow::Follow,
    /// Search queries pinned as chips in Research, in pin order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    /// Muted news sources and words, edited in Settings and the preview.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute: Vec<crate::news::Mute>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchTopic {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub categories: Vec<Category>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub category_include: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsTopic {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub sources: Vec<Feed>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

impl Default for Topics {
    fn default() -> Self {
        serde_json::from_str(DEFAULT_JSON).expect("default_topics.json is valid")
    }
}

impl Topics {
    /// Reads `topics.json`, writing the defaults first if it doesn't exist yet. A file that
    /// doesn't parse is an error, never silently replaced: it holds the user's choices.
    pub fn load_or_seed(path: &Path) -> Result<Topics, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| format!("{} is not valid: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let t = Topics::default();
                t.save(path)?;
                Ok(t)
            }
            Err(e) => Err(format!("can't read {}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        json.push('\n');
        write_atomic(path, json.as_bytes())
    }

    /// Every enabled arXiv category across enabled topics, once each, in order.
    pub fn arxiv_categories(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for t in self.research.iter().filter(|t| t.enabled) {
            for c in t.categories.iter().filter(|c| c.enabled) {
                if !out.contains(&c.id) {
                    out.push(c.id.clone());
                }
            }
        }
        out
    }

    /// Every enabled feed across enabled news topics, once per URL.
    pub fn news_feeds(&self) -> Vec<Feed> {
        let mut out: Vec<Feed> = Vec::new();
        for t in self.news.iter().filter(|t| t.enabled) {
            for f in t.sources.iter().filter(|f| f.enabled) {
                if !out.iter().any(|o| o.url == f.url) {
                    out.push(f.clone());
                }
            }
        }
        out
    }

    /// Checks a topics file before it is saved: unique, non-empty ids and names, arXiv
    /// category ids that look like ids, and web addresses for feeds.
    pub fn validate(&self) -> Result<(), String> {
        let mut ids: Vec<&str> = Vec::new();
        let research = self.research.iter().map(|t| (&t.id, &t.name));
        let news = self.news.iter().map(|t| (&t.id, &t.name));
        for (id, name) in research.chain(news) {
            if id.trim().is_empty() || name.trim().is_empty() {
                return Err("every topic needs a name".into());
            }
            if ids.contains(&id.as_str()) {
                return Err(format!("two topics share the id \"{id}\""));
            }
            ids.push(id);
        }
        for t in &self.research {
            for c in &t.categories {
                if !is_category_id(&c.id) {
                    return Err(format!(
                        "\"{}\" in {} isn't an arXiv category id",
                        c.id, t.name
                    ));
                }
            }
        }
        for t in &self.news {
            for f in &t.sources {
                if f.name.trim().is_empty() {
                    return Err(format!("a feed in {} has no name", t.name));
                }
                if !(f.url.starts_with("https://") || f.url.starts_with("http://")) {
                    return Err(format!("{}: the address must start with https://", f.name));
                }
            }
        }
        Ok(())
    }

    /// Whether `self` fetches anything `old` didn't: (new arXiv categories, new feeds).
    /// After a topic change, only new sources call for a refresh straight away.
    pub fn new_sources_since(&self, old: &Topics) -> (bool, bool) {
        let old_cats = old.arxiv_categories();
        let old_feeds: Vec<String> = old.news_feeds().into_iter().map(|f| f.url).collect();
        (
            self.arxiv_categories()
                .iter()
                .any(|c| !old_cats.contains(c)),
            self.news_feeds()
                .iter()
                .any(|f| !old_feeds.contains(&f.url)),
        )
    }

    /// The ids of the enabled topics `item` belongs to.
    pub fn topics_for(&self, item: &Item) -> Vec<String> {
        match item.section {
            Section::Research => self
                .research
                .iter()
                .filter(|t| t.enabled && t.matches(item))
                .map(|t| t.id.clone())
                .collect(),
            Section::News => self
                .news
                .iter()
                .filter(|t| t.enabled && t.matches(item))
                .map(|t| t.id.clone())
                .collect(),
        }
    }
}

impl ResearchTopic {
    pub fn matches(&self, item: &Item) -> bool {
        let text = searchable(item);
        let in_category = self.categories.iter().filter(|c| c.enabled).any(|c| {
            item.categories.iter().any(|ic| category_matches(&c.id, ic))
                && self
                    .category_include
                    .get(&c.id)
                    .is_none_or(|words| any_word(&text, words))
        });
        in_category && keywords_allow(&text, &self.include, &self.exclude)
    }
}

impl NewsTopic {
    pub fn matches(&self, item: &Item) -> bool {
        self.sources
            .iter()
            .any(|f| f.enabled && f.url == item.source)
            && keywords_allow(&searchable(item), &self.include, &self.exclude)
    }
}

/// `quant-ph`, `physics.optics`, `cs.LG`, `math-ph`, or a whole archive (`cs`).
pub fn is_category_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 24
        && id.starts_with(|c: char| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// `cs` (a whole archive) matches `cs.LG`; otherwise the ids must be equal.
fn category_matches(wanted: &str, actual: &str) -> bool {
    if wanted.contains('.') {
        wanted == actual
    } else {
        actual == wanted
            || actual
                .strip_prefix(wanted)
                .is_some_and(|rest| rest.starts_with('.'))
    }
}

fn searchable(item: &Item) -> String {
    format!("{} {}", item.title, item.summary).to_lowercase()
}

fn any_word(text: &str, words: &[String]) -> bool {
    words
        .iter()
        .any(|w| !w.trim().is_empty() && text.contains(&w.trim().to_lowercase()))
}

fn keywords_allow(text: &str, include: &[String], exclude: &[String]) -> bool {
    let wants = include.iter().any(|w| !w.trim().is_empty());
    (!wants || any_word(text, include)) && !any_word(text, exclude)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults have no news feeds (each user adds their own), so tests bring two.
    fn with_news() -> Topics {
        let feed = |name: &str, url: &str, enabled| Feed {
            name: name.into(),
            url: url.into(),
            enabled,
        };
        let topic = |id: &str, enabled, sources| NewsTopic {
            id: id.into(),
            name: id.into(),
            enabled,
            sources,
            include: vec![],
            exclude: vec![],
        };
        Topics {
            news: vec![
                topic(
                    "world",
                    true,
                    vec![
                        feed("Daily", "https://daily.example.org/rss", true),
                        feed("Weekly", "https://weekly.example.org/rss", false),
                    ],
                ),
                topic(
                    "business",
                    false,
                    vec![feed("Ledger", "https://ledger.example.org/feed", true)],
                ),
            ],
            ..Topics::default()
        }
    }

    #[test]
    fn defaults_parse_and_list_sources_once() {
        let t = Topics::default();
        assert!(t.news.is_empty(), "no news feeds out of the box");
        let cats = t.arxiv_categories();
        assert_eq!(cats.iter().filter(|c| *c == "cs.LG").count(), 1);
        assert!(cats.contains(&"astro-ph.CO".to_string()));
        assert!(!cats.contains(&"cs.AI".to_string()), "switched off");
        let feeds = with_news().news_feeds();
        assert_eq!(feeds.len(), 1);
        assert!(feeds.iter().all(|f| f.name != "Weekly"), "switched off");
        assert!(feeds.iter().all(|f| f.name != "Ledger"), "topic off");
    }

    #[test]
    fn validation_catches_bad_topics() {
        let mut t = with_news();
        assert_eq!(t.validate(), Ok(()));
        t.research[0].categories[0].id = "quant ph; drop".into();
        assert!(
            t.validate()
                .unwrap_err()
                .contains("isn't an arXiv category")
        );
        let mut t = with_news();
        t.news[1].id = t.news[0].id.clone();
        assert!(t.validate().unwrap_err().contains("share the id"));
        let mut t = with_news();
        t.news[0].sources[0].url = "file:///etc/passwd".into();
        assert!(t.validate().is_err());
    }

    #[test]
    fn new_sources_are_noticed_but_removals_are_not() {
        let old = with_news();
        let mut t = old.clone();
        assert_eq!(t.new_sources_since(&old), (false, false));
        t.research[0].enabled = false;
        assert_eq!(t.new_sources_since(&old), (false, false));
        t.research[4].categories[1].enabled = true; // cs.AI
        t.news[1].enabled = true; // business
        assert_eq!(t.new_sources_since(&old), (true, true));
    }

    #[test]
    fn archive_matches_its_categories() {
        assert!(category_matches("cs", "cs.LG"));
        assert!(!category_matches("cs", "csx.LG"));
        assert!(category_matches("quant-ph", "quant-ph"));
        assert!(!category_matches("physics.optics", "physics.optics2"));
    }
}
