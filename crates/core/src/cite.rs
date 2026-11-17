//! Citations: BibTeX and RIS for papers (and, as a fallback, any item).
//!
//! Where a citation comes from, in order:
//! 1. the cache (`citations` table), so a second copy needs no network;
//! 2. a paper with a journal DOI: doi.org, asking for BibTeX or RIS (content negotiation);
//! 3. an arXiv paper, BibTeX: arXiv's own entry (`arxiv.org/bibtex/<id>`);
//! 4. otherwise, or when offline: written by Dispatch from what it knows (not cached, so the
//!    online version replaces it next time).

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

use crate::fetch::Fetch;
use crate::item::Item;
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Bibtex,
    Ris,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Bibtex => "bibtex",
            Format::Ris => "ris",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Bibtex => "bib",
            Format::Ris => "ris",
        }
    }

    fn accept(self) -> &'static str {
        match self {
            Format::Bibtex => "application/x-bibtex",
            Format::Ris => "application/x-research-info-systems",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Citation {
    pub text: String,
    /// "doi.org", "arXiv", "Dispatch" or "Dispatch (offline)".
    pub origin: String,
}

/// More uncached papers than this in one go are written by Dispatch instead of fetched one by
/// one (keeps a big export fast and polite).
pub const MAX_FETCHED: usize = 20;

pub fn arxiv_id(item: &Item) -> Option<&str> {
    item.key.strip_prefix("arxiv:")
}

/// One citation, from the cache, the network, or written locally (see the module notes).
pub fn citation(
    store: &Store,
    item: &Item,
    format: Format,
    fetch: Option<&dyn Fetch>,
    now: DateTime<Utc>,
) -> Citation {
    if let Some((text, origin)) = store.citation(&item.key, format.as_str()) {
        return Citation { text, origin };
    }
    let Some(fetch) = fetch else {
        return local(item, format, "Dispatch");
    };
    let online = if let Some(doi) = &item.doi {
        fetch
            .get_as(&format!("https://doi.org/{doi}"), format.accept())
            .map(|b| (b, "doi.org"))
    } else if let (Some(id), Format::Bibtex) = (arxiv_id(item), format) {
        fetch
            .get(&format!("https://arxiv.org/bibtex/{id}"))
            .map(|b| (b, "arXiv"))
    } else {
        return local(item, format, "Dispatch");
    };
    match online {
        Ok((bytes, origin)) => {
            let text = String::from_utf8_lossy(&bytes).trim().to_owned();
            if looks_right(&text, format) {
                let _ = store.set_citation(&item.key, format.as_str(), &text, origin, now);
                Citation {
                    text,
                    origin: origin.into(),
                }
            } else {
                local(item, format, "Dispatch")
            }
        }
        Err(_) => local(item, format, "Dispatch (offline)"),
    }
}

/// Citations for several items, joined into one file's worth of text. Returns the text and
/// how many came from each origin.
pub fn citations(
    store: &Store,
    items: &[Item],
    format: Format,
    fetch: Option<&dyn Fetch>,
    now: DateTime<Utc>,
) -> (String, Vec<(String, usize)>) {
    let mut fetched = 0;
    let mut parts = Vec::new();
    let mut origins: Vec<(String, usize)> = Vec::new();
    for item in items {
        let cached = store.citation(&item.key, format.as_str()).is_some();
        let allow = cached || fetched < MAX_FETCHED;
        if !cached && fetch.is_some() {
            fetched += 1;
        }
        let c = citation(store, item, format, if allow { fetch } else { None }, now);
        match origins.iter_mut().find(|(o, _)| *o == c.origin) {
            Some((_, n)) => *n += 1,
            None => origins.push((c.origin.clone(), 1)),
        }
        parts.push(c.text);
    }
    let sep = if format == Format::Bibtex {
        "\n\n"
    } else {
        "\n"
    };
    let mut text = parts.join(sep);
    text.push('\n');
    (text, origins)
}

fn looks_right(text: &str, format: Format) -> bool {
    match format {
        Format::Bibtex => text.starts_with('@') && text.contains('{'),
        Format::Ris => text.contains("TY  -") && text.contains("ER  -"),
    }
}

fn local(item: &Item, format: Format, origin: &str) -> Citation {
    Citation {
        text: match format {
            Format::Bibtex => bibtex(item),
            Format::Ris => ris(item),
        },
        origin: origin.into(),
    }
}

/// `okafor2026closed`: first author's surname, year, first long title word.
pub fn cite_key(item: &Item) -> String {
    let surname = item
        .authors
        .first()
        .and_then(|a| a.split_whitespace().last())
        .unwrap_or("anon");
    let year = item.published.map(|d| d.year()).unwrap_or(0);
    let word = item
        .title
        .split(|c: char| !c.is_alphanumeric())
        .find(|w| {
            w.len() > 3
                && !["with", "from", "that", "this", "into"].contains(&w.to_lowercase().as_str())
        })
        .unwrap_or("");
    let ascii = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
    };
    format!("{}{}{}", ascii(surname), year, ascii(word))
}

fn bib_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('&', "\\&")
        .replace('%', "\\%")
        .replace('#', "\\#")
}

/// BibTeX written from Dispatch's own data. Titles keep their LaTeX (`$…$`) as written;
/// other special characters are escaped.
pub fn bibtex(item: &Item) -> String {
    bibtex_with_key(item, &cite_key(item))
}

/// The same with a chosen key (the live `.bib` file's key style, `bibfile.rs`).
pub fn bibtex_with_key(item: &Item, key: &str) -> String {
    let year = item.published.map(|d| d.year().to_string());
    let mut fields: Vec<(&str, String)> = vec![
        ("title", format!("{{{}}}", keep_maths(&item.title))),
        ("author", format!("{{{}}}", item.authors.join(" and "))),
    ];
    if let Some(y) = year {
        fields.push(("year", format!("{{{y}}}")));
    }
    let kind = if let Some(id) = arxiv_id(item) {
        fields.push(("eprint", format!("{{{id}}}")));
        fields.push(("archivePrefix", "{arXiv}".into()));
        if let Some(c) = item.categories.first() {
            fields.push(("primaryClass", format!("{{{c}}}")));
        }
        if item.journal_ref.is_some() {
            "article"
        } else {
            "misc"
        }
    } else {
        fields.push((
            "howpublished",
            format!("{{{}}}", bib_escape(&item.source_name)),
        ));
        "misc"
    };
    if let Some(j) = &item.journal_ref {
        fields.push(("journal", format!("{{{}}}", bib_escape(j))));
    }
    if let Some(d) = &item.doi {
        fields.push(("doi", format!("{{{d}}}")));
    }
    fields.push(("url", format!("{{{}}}", item.link)));
    let body = fields
        .iter()
        .map(|(k, v)| format!("  {k} = {v}"))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("@{kind}{{{key},\n{body}\n}}")
}

/// Escapes text outside `$…$`, leaves maths alone.
fn keep_maths(s: &str) -> String {
    s.split('$')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                format!("${part}$")
            } else {
                bib_escape(part)
            }
        })
        .collect()
}

/// RIS written from Dispatch's own data. arXiv papers without a journal are `UNPB`
/// (unpublished work); with a journal reference, `JOUR`.
pub fn ris(item: &Item) -> String {
    let mut lines = Vec::new();
    let ty = match (arxiv_id(item), &item.journal_ref) {
        (Some(_), None) => "UNPB",
        (Some(_), Some(_)) => "JOUR",
        (None, _) => "ELEC",
    };
    lines.push(format!("TY  - {ty}"));
    lines.push(format!("TI  - {}", item.title));
    for a in &item.authors {
        lines.push(format!("AU  - {a}"));
    }
    if let Some(d) = item.published {
        lines.push(format!("PY  - {}", d.year()));
        lines.push(format!("DA  - {}", d.format("%Y/%m/%d")));
    }
    if let Some(j) = &item.journal_ref {
        lines.push(format!("JO  - {j}"));
    }
    if let Some(d) = &item.doi {
        lines.push(format!("DO  - {d}"));
    }
    if let Some(id) = arxiv_id(item) {
        lines.push(format!("N1  - arXiv:{id}"));
    } else {
        lines.push(format!("PB  - {}", item.source_name));
    }
    lines.push(format!("UR  - {}", item.link));
    if !item.summary.is_empty() {
        lines.push(format!("AB  - {}", item.summary));
    }
    lines.push("ER  - ".into());
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Section;

    fn paper() -> Item {
        Item {
            key: "arxiv:2609.01234".into(),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "Adiabatic transfer with $\\mathcal{O}(1)$ overhead & 50% loss".into(),
            authors: vec!["Ada Lovelace".into(), "Nico Weißler".into()],
            summary: "We show…".into(),
            link: "https://arxiv.org/abs/2609.01234".into(),
            pdf: None,
            categories: vec!["physics.optics".into()],
            published: "2026-10-27T04:00:00Z".parse().ok(),
            announce_type: None,
            version: Some(2),
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn local_bibtex_keeps_maths_and_escapes_the_rest() {
        let b = bibtex(&paper());
        assert!(b.starts_with("@misc{lovelace2026adiabatic,"), "{b}");
        assert!(b.contains("$\\mathcal{O}(1)$"), "{b}");
        assert!(b.contains("\\& 50\\% loss"), "{b}");
        assert!(b.contains("author = {Ada Lovelace and Nico Weißler}"));
        assert!(b.contains("eprint = {2609.01234}"));
        assert!(b.contains("primaryClass = {physics.optics}"));
        assert!(b.trim_end().ends_with('}'));
    }

    #[test]
    fn local_ris_has_the_required_tags() {
        let mut p = paper();
        let r = ris(&p);
        assert!(r.starts_with("TY  - UNPB\n"));
        assert!(r.contains("AU  - Ada Lovelace\nAU  - Nico Weißler\n"));
        assert!(r.contains("N1  - arXiv:2609.01234"));
        assert!(r.ends_with("ER  - \n"));
        p.journal_ref = Some("Phys. Rev. A 1 (2026)".into());
        assert!(ris(&p).starts_with("TY  - JOUR\n"));
    }
}
