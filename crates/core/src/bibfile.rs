//! A live `.bib` file and LaTeX snippets.
//!
//! The `.bib` file holds every saved paper (or the saved papers with one tag), written by
//! Dispatch from its own data (no network) whenever saved papers change. Keys follow the
//! chosen style and stay unique (`smith2024adiabatic`, then `…a`, `…b`). Papers are ordered
//! by arXiv id, so a newly saved paper usually lands at the end and older keys don't move.
//!
//! Dispatch only ever replaces a file it wrote itself (it starts with [`HEADER`]); an
//! existing `.bib` of yours is never overwritten.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::Datelike;
use serde::{Deserialize, Serialize};

use crate::cite;
use crate::item::Item;
use crate::paths::write_atomic;

/// First line of every `.bib` Dispatch writes; also how it recognises its own file.
pub const HEADER: &str = "% Written by Dispatch from your saved papers. Edits here are replaced.";

/// How citation keys are made (Settings → Research workflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStyle {
    /// `smith2024adiabatic`: first author's surname, year, first long title word.
    #[default]
    Author,
    /// `2410.01234`: the arXiv id (papers without one fall back to the author style).
    Arxiv,
}

/// The key before making it unique.
pub fn base_key(item: &Item, style: KeyStyle) -> String {
    match (style, cite::arxiv_id(item)) {
        (KeyStyle::Arxiv, Some(id)) => id.to_owned(),
        _ => cite::cite_key(item),
    }
}

/// Unique keys for a set of items (item key → citation key), in arXiv-id order: the first
/// of a clash keeps the plain key, later ones get `a`, `b`, …
pub fn keys(items: &[Item], style: KeyStyle) -> HashMap<String, String> {
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let mut used: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = HashMap::new();
    for item in sorted {
        let base = base_key(item, style);
        let n = used.entry(base.clone()).or_insert(0);
        let key = if *n == 0 {
            base.clone()
        } else {
            // 1 → a, 2 → b, … 26 → z, then aa (never reached in practice).
            let mut suffix = String::new();
            let mut k = *n;
            while k > 0 {
                k -= 1;
                suffix.insert(0, (b'a' + (k % 26) as u8) as char);
                k /= 26;
            }
            format!("{base}{suffix}")
        };
        *n += 1;
        out.insert(item.key.clone(), key);
    }
    out
}

/// The whole `.bib` file for these items.
pub fn render(items: &[Item], style: KeyStyle) -> String {
    let keys = keys(items, style);
    let mut sorted: Vec<&Item> = items.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let mut out = format!("{HEADER}\n% {} papers.\n", sorted.len());
    for item in sorted {
        out.push('\n');
        out.push_str(&cite::bibtex_with_key(item, &keys[&item.key]));
        out.push('\n');
    }
    out
}

/// Writes `text` to `path` if it differs from what's there. Refuses to replace a file that
/// Dispatch didn't write. Returns whether the file changed.
pub fn write(path: &Path, text: &str) -> Result<bool, String> {
    match std::fs::read_to_string(path) {
        Ok(old) if old == text => return Ok(false),
        Ok(old) if !old.starts_with(HEADER) => {
            return Err(format!(
                "{} already exists and wasn't written by Dispatch; choose another file name",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("can't read {}: {e}", path.display())),
    }
    write_atomic(path, text.as_bytes())?;
    Ok(true)
}

/// `\cite{key}`.
pub fn cite_command(key: &str) -> String {
    format!("\\cite{{{key}}}")
}

/// A `\bibitem` for a `thebibliography` list: authors, title, journal or arXiv id, year.
pub fn bibitem(item: &Item, key: &str) -> String {
    let authors = match item.authors.len() {
        0 => String::new(),
        1 => item.authors[0].clone(),
        2 => format!("{} and {}", item.authors[0], item.authors[1]),
        _ => format!("{} \\emph{{et al.}}", item.authors[0]),
    };
    let year = item.published.map(|d| d.year().to_string());
    let mut parts = vec![];
    if !authors.is_empty() {
        parts.push(authors);
    }
    parts.push(format!("``{},''", item.title.trim()));
    let mut source = match (&item.journal_ref, cite::arxiv_id(item)) {
        (Some(j), _) => j.clone(),
        (None, Some(id)) => format!("arXiv:{id}"),
        (None, None) => item.source_name.clone(),
    };
    if let Some(y) = year.filter(|y| !source.contains(y.as_str())) {
        source = format!("{source} ({y})");
    }
    parts.push(source);
    format!(
        "\\bibitem{{{key}}} {}.",
        parts.join(", ").replace(",'', ", ",'' ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Section;

    fn paper(id: &str, first: &str, title: &str) -> Item {
        Item {
            key: format!("arxiv:{id}"),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: title.into(),
            authors: vec![format!("Ada {first}"), "Bo Chen".into(), "Cy Diaz".into()],
            summary: "We show.".into(),
            link: format!("https://arxiv.org/abs/{id}"),
            pdf: None,
            categories: vec!["quant-ph".into()],
            published: "2026-10-27T04:00:00Z".parse().ok(),
            announce_type: None,
            version: Some(1),
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn keys_are_unique_and_stable_in_arxiv_order() {
        let a = paper("2610.00002", "Smith", "Adiabatic pumping");
        let b = paper("2610.00001", "Smith", "Adiabatic transfer");
        let c = paper("2610.00003", "Jones", "Spin chains");
        let k = keys(&[a.clone(), b.clone(), c.clone()], KeyStyle::Author);
        assert_eq!(k[&b.key], "smith2026adiabatic");
        assert_eq!(k[&a.key], "smith2026adiabatica");
        assert_eq!(k[&c.key], "jones2026spin");
        let k = keys(&[a.clone(), c], KeyStyle::Arxiv);
        assert_eq!(k[&a.key], "2610.00002");
    }

    #[test]
    fn the_file_is_dispatchs_own_and_never_replaces_yours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("refs.bib");
        let text = render(
            &[paper("2610.00001", "Smith", "Adiabatic transfer")],
            KeyStyle::Author,
        );
        assert!(text.starts_with(HEADER));
        assert!(text.contains("@misc{smith2026adiabatic,"));
        assert!(write(&path, &text).unwrap());
        assert!(!write(&path, &text).unwrap(), "unchanged: not rewritten");
        let mine = dir.path().join("mine.bib");
        std::fs::write(&mine, "@article{x, title={Mine}}").unwrap();
        assert!(write(&mine, &text).is_err());
        assert_eq!(
            std::fs::read_to_string(&mine).unwrap(),
            "@article{x, title={Mine}}"
        );
    }

    #[test]
    fn latex_snippets() {
        let p = paper("2610.00001", "Smith", "Adiabatic transfer");
        assert_eq!(
            cite_command("smith2026adiabatic"),
            "\\cite{smith2026adiabatic}"
        );
        assert_eq!(
            bibitem(&p, "k"),
            "\\bibitem{k} Ada Smith \\emph{et al.}, ``Adiabatic transfer,'' arXiv:2610.00001 (2026)."
        );
    }
}
