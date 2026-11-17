//! `dispatch://` links, for Raycast, Shortcuts, SketchyBar or a note
//! (`open "dispatch://search?q=spin chain"`). macOS hands the link to the running app (or
//! starts it); the app turns it into a [`Link`] here and acts on it.
//!
//! - `dispatch://` or `dispatch://open` — show the window
//! - `dispatch://open/research` (`news`, `saved`, `search`, `stats`) — show that tab
//! - `dispatch://search?q=WORDS` — Search for WORDS (same query language as the app)
//! - `dispatch://paper/2410.01234` — that paper (any version, `arXiv:` allowed)
//! - `dispatch://refresh` — refresh now (no window)
//! - `dispatch://add?q=LINK` — save an arXiv link/id or a DOI (`add::parse`)

use reqwest::Url;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Link {
    /// Show the window, on this tab if given.
    Open {
        view: Option<String>,
    },
    Search {
        query: String,
    },
    /// An item key (`arxiv:2410.01234`).
    Paper {
        key: String,
    },
    Refresh,
    /// "Add to Dispatch" (what was pasted, checked by `add::parse`).
    Add {
        input: String,
    },
    /// Made by the app, never parsed: the paper an `Add` saved, shown in Saved.
    Added {
        key: String,
        title: String,
    },
}

const VIEWS: [&str; 5] = ["research", "news", "saved", "search", "stats"];

/// Reads a `dispatch://` link; anything else (or a typo) is an error saying what works.
pub fn parse(link: &str) -> Result<Link, String> {
    let url = Url::parse(link.trim()).map_err(|_| format!("not a link: {link}"))?;
    if url.scheme() != "dispatch" {
        return Err(format!("not a dispatch:// link: {link}"));
    }
    // `dispatch://paper/2410.01234`: "paper" is the host, the rest the path.
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let rest: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(percent_decode).collect())
        .unwrap_or_default();
    match host.as_str() {
        "" | "open" | "show" => {
            let view = rest.first().map(|v| v.to_ascii_lowercase());
            match view {
                Some(v) if !VIEWS.contains(&v.as_str()) => Err(format!(
                    "dispatch://open/… takes {} (not {v})",
                    VIEWS.join(", ")
                )),
                view => Ok(Link::Open { view }),
            }
        }
        "search" => {
            let query = url
                .query_pairs()
                .find(|(k, _)| k == "q" || k == "query")
                .map(|(_, v)| v.into_owned())
                .or_else(|| (!rest.is_empty()).then(|| rest.join(" ")))
                .unwrap_or_default();
            Ok(Link::Search { query })
        }
        "paper" => {
            let id = rest.join("/");
            arxiv_key(&id)
                .map(|key| Link::Paper { key })
                .ok_or_else(|| format!("dispatch://paper/… needs an arXiv id (not `{id}`)"))
        }
        "refresh" => Ok(Link::Refresh),
        "add" => {
            let input = url
                .query_pairs()
                .find(|(k, _)| k == "q" || k == "url" || k == "doi" || k == "id")
                .map(|(_, v)| v.into_owned())
                .or_else(|| (!rest.is_empty()).then(|| rest.join("/")))
                .unwrap_or_default();
            crate::add::parse(&input)?;
            Ok(Link::Add { input })
        }
        other => Err(format!(
            "dispatch://{other} isn't known: use open, search?q=…, paper/ID, refresh or add?q=…"
        )),
    }
}

/// `2410.01234v2`, `arXiv:2410.01234`, `hep-th/9901001` → `arxiv:2410.01234` (no version).
pub fn arxiv_key(id: &str) -> Option<String> {
    let id = id.trim();
    let id = id
        .strip_prefix("arXiv:")
        .or_else(|| id.strip_prefix("arxiv:"))
        .unwrap_or(id);
    let base = match id.rfind('v') {
        Some(i) if i > 0 && id[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < id.len() => {
            &id[..i]
        }
        _ => id,
    };
    let new_style = base.split_once('.').is_some_and(|(a, b)| {
        a.len() == 4
            && a.chars().all(|c| c.is_ascii_digit())
            && (4..=5).contains(&b.len())
            && b.chars().all(|c| c.is_ascii_digit())
    });
    let old_style = base.split_once('/').is_some_and(|(a, b)| {
        !a.is_empty()
            && a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && b.len() == 7
            && b.chars().all(|c| c.is_ascii_digit())
    });
    (new_style || old_style).then(|| format!("arxiv:{base}"))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_kind_of_link() {
        assert_eq!(parse("dispatch://").unwrap(), Link::Open { view: None });
        assert_eq!(
            parse("dispatch://open/Saved").unwrap(),
            Link::Open {
                view: Some("saved".into())
            }
        );
        assert!(parse("dispatch://open/nowhere").is_err());
        assert_eq!(
            parse("dispatch://search?q=spin%20chain").unwrap(),
            Link::Search {
                query: "spin chain".into()
            }
        );
        assert_eq!(
            parse("dispatch://search?q=au:smith+qubit").unwrap(),
            Link::Search {
                query: "au:smith qubit".into()
            }
        );
        assert_eq!(
            parse("dispatch://search/quantum%20dot").unwrap(),
            Link::Search {
                query: "quantum dot".into()
            }
        );
        assert_eq!(
            parse("dispatch://paper/2410.01234v3").unwrap(),
            Link::Paper {
                key: "arxiv:2410.01234".into()
            }
        );
        assert_eq!(
            parse("dispatch://paper/arXiv:2410.01234").unwrap(),
            Link::Paper {
                key: "arxiv:2410.01234".into()
            }
        );
        assert_eq!(
            parse("dispatch://paper/hep-th/9901001v1").unwrap(),
            Link::Paper {
                key: "arxiv:hep-th/9901001".into()
            }
        );
        assert!(parse("dispatch://paper/hello").is_err());
        assert_eq!(parse("dispatch://refresh").unwrap(), Link::Refresh);
        assert_eq!(
            parse("dispatch://add?q=https%3A%2F%2Farxiv.org%2Fabs%2F2410.01234v2").unwrap(),
            Link::Add {
                input: "https://arxiv.org/abs/2410.01234v2".into()
            }
        );
        assert_eq!(
            parse("dispatch://add?doi=10.5555/demo.1.1").unwrap(),
            Link::Add {
                input: "10.5555/demo.1.1".into()
            }
        );
        assert!(
            parse("dispatch://add?q=hello").is_err(),
            "checked before the app acts"
        );
        assert!(parse("dispatch://delete-everything").is_err());
        assert!(parse("https://arxiv.org").is_err());
    }
}
