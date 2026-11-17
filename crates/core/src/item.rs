//! The one shape every adapter produces: a paper or a news article.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Research,
    News,
}

impl Section {
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Research => "research",
            Section::News => "news",
        }
    }

    pub fn parse(s: &str) -> Option<Section> {
        match s {
            "research" => Some(Section::Research),
            "news" => Some(Section::News),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Stable identity: `arxiv:2610.91665` (no version, so cross-listings and new versions
    /// land on the same row) or `news:<guid or link>`.
    pub key: String,
    pub section: Section,
    /// The source that delivered it: `arxiv` or a feed URL.
    pub source: String,
    /// Shown next to news items ("Science Weekly"); "arXiv" for papers.
    pub source_name: String,
    pub title: String,
    pub authors: Vec<String>,
    /// Plain text (HTML removed). LaTeX is left as written, for KaTeX in the UI.
    pub summary: String,
    pub link: String,
    pub pdf: Option<String>,
    /// arXiv categories (`quant-ph`), or a feed's own categories.
    pub categories: Vec<String>,
    pub published: Option<DateTime<Utc>>,
    /// arXiv only: `new`, `cross`, `replace` or `replace-cross`.
    pub announce_type: Option<String>,
    /// arXiv only: the version number in the feed (2 for `v2`).
    pub version: Option<u32>,
    pub doi: Option<String>,
    pub journal_ref: Option<String>,
}

/// Removes HTML tags and decodes the common entities, collapsing whitespace.
/// Feeds put markup in summaries; the list and preview show plain text.
pub fn plain_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        // A `<` only opens a tag when a letter, `/` or `!` follows, so maths like
        // `$x<y$` survives.
        let opens_tag = c == '<'
            && chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphabetic() || *n == '/' || *n == '!');
        match c {
            '<' if !in_tag && opens_tag => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#039;", "'")
        .replace("&#8217;", "’")
        .replace("&#8216;", "‘")
        .replace("&#8220;", "“")
        .replace("&#8221;", "”")
        .replace("&#8211;", "–")
        .replace("&#8212;", "—")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_strips_tags_and_entities() {
        assert_eq!(
            plain_text("<p>Hello&nbsp;<b>world</b> &amp; more</p>\n\n<img src=x>"),
            "Hello world & more"
        );
    }

    #[test]
    fn plain_text_keeps_latex() {
        assert_eq!(plain_text("rate $Γ$ and $x < y$"), "rate $Γ$ and $x < y$");
        assert_eq!(plain_text("$x<1$ and $x&lt;y$"), "$x<1$ and $x<y$");
    }
}
