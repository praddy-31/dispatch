//! Search archive: what the user types becomes an SQLite FTS5 query.
//!
//! The index (`search`, built in `store.rs` migration 4) holds every item ever fetched:
//! title, authors, abstract, the user's note and tags, and a "meta" column with the key
//! (so an arXiv id finds its paper), categories, source name, DOI and journal.
//!
//! Query language, kept small:
//! - words are all required, and each matches as a prefix (`qubit` finds `qubits`);
//! - `"exact phrase"` matches those words in a row, without prefixes;
//! - `-word` leaves out items with that word; `OR` between two words takes either;
//! - `ti:` `au:` `abs:` `note:` `cat:` (and `title:` `author:` `tag:` `source:` …) look in
//!   one field only, like arXiv's own search.
//!
//! Every word is quoted before it reaches FTS5, so no input can be a syntax error.

use serde::{Deserialize, Serialize};

/// The index's columns, in the order `bm25` weights are given.
pub const COLUMNS: [&str; 5] = ["ftitle", "fauthors", "fabstract", "fmine", "fmeta"];
/// A title match counts most, then authors, then the user's notes, then the abstract.
pub const WEIGHTS: [f64; 5] = [10.0, 6.0, 1.0, 4.0, 2.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// FTS5's BM25 score with the weights above, newest first among equals.
    #[default]
    Best,
    Newest,
}

fn column(field: &str) -> Option<&'static str> {
    Some(match field.to_lowercase().as_str() {
        "ti" | "title" => "ftitle",
        "au" | "author" | "authors" | "by" => "fauthors",
        "abs" | "abstract" | "summary" => "fabstract",
        "note" | "notes" | "tag" | "tags" | "mine" => "fmine",
        "cat" | "category" | "source" | "src" | "id" | "doi" | "journal" => "fmeta",
        _ => return None,
    })
}

/// One piece of the query before it is written out.
#[derive(Debug, PartialEq)]
struct Term {
    text: String,
    phrase: bool,
    not: bool,
    or_before: bool,
    col: Option<&'static str>,
}

/// Splits on spaces, keeping `"…"` together (an unclosed quote runs to the end).
fn pieces(q: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for ch in q.chars() {
        match ch {
            '"' => {
                if quoted {
                    out.push((std::mem::take(&mut cur), true));
                    quoted = false;
                    continue;
                }
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
                quoted = true;
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push((std::mem::take(&mut cur), false));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push((cur, quoted));
    }
    out
}

fn terms(q: &str) -> Vec<Term> {
    let mut out: Vec<Term> = Vec::new();
    let mut or_next = false;
    let mut pending_col: Option<&'static str> = None;
    let mut pending_not = false;
    for (raw, phrase) in pieces(q) {
        let mut text = raw.as_str();
        if !phrase && text == "OR" {
            or_next = !out.is_empty();
            continue;
        }
        let mut not = pending_not;
        let mut col = pending_col;
        pending_not = false;
        pending_col = None;
        if !phrase {
            if let Some(rest) = text.strip_prefix('-') {
                not = true;
                text = rest;
            }
            if let Some((field, rest)) = text.split_once(':')
                && let Some(c) = column(field)
            {
                col = Some(c);
                text = rest;
                if text.is_empty() {
                    // `au: "Smith"` or `-cat: x`: the field applies to the next piece.
                    pending_col = Some(c);
                    pending_not = not;
                    continue;
                }
            }
        }
        // A piece with no letters or digits has nothing for the index to match.
        if !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        out.push(Term {
            text: text.to_owned(),
            phrase,
            not,
            or_before: std::mem::take(&mut or_next),
            col,
        });
    }
    out
}

fn write(t: &Term) -> String {
    let quoted = format!("\"{}\"", t.text.replace('"', "\"\""));
    let body = if t.phrase {
        quoted
    } else {
        format!("{quoted}*")
    };
    match t.col {
        Some(c) => format!("{c} : {body}"),
        None => body,
    }
}

/// The FTS5 MATCH expression for what was typed, or None if there is nothing to look for
/// (empty, only punctuation, or only `-words`).
pub fn fts_query(q: &str) -> Option<String> {
    let terms = terms(q);
    let (wanted, unwanted): (Vec<&Term>, Vec<&Term>) = terms.iter().partition(|t| !t.not);
    if wanted.is_empty() {
        return None;
    }
    // FTS5 binds AND tighter than OR, so `a OR b c` is written `(a OR b) AND c`.
    let mut groups: Vec<Vec<String>> = Vec::new();
    for t in wanted {
        match groups.last_mut() {
            Some(g) if t.or_before => g.push(write(t)),
            _ => groups.push(vec![write(t)]),
        }
    }
    let mut out = groups
        .iter()
        .map(|g| {
            if g.len() > 1 {
                format!("({})", g.join(" OR "))
            } else {
                g[0].clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    for t in unwanted {
        out.push_str(" NOT ");
        out.push_str(&write(t));
    }
    Some(out)
}

/// Snippet markers: the UI splits on these to highlight matches without parsing any HTML.
pub const MARK_START: char = '\u{2}';
pub const MARK_END: char = '\u{3}';

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_required_prefixes_and_phrases_are_exact() {
        assert_eq!(
            fts_query("quantum dot").unwrap(),
            r#""quantum"* AND "dot"*"#
        );
        assert_eq!(
            fts_query(r#""quantum dot" laser"#).unwrap(),
            r#""quantum dot" AND "laser"*"#
        );
        // An unclosed quote still searches (as you type).
        assert_eq!(fts_query(r#""spin ch"#).unwrap(), r#""spin ch""#);
    }

    #[test]
    fn fields_not_and_or() {
        assert_eq!(
            fts_query("au:smith ti:graphene").unwrap(),
            r#"fauthors : "smith"* AND ftitle : "graphene"*"#
        );
        assert_eq!(
            fts_query(r#"au: "de Vries""#).unwrap(),
            r#"fauthors : "de Vries""#
        );
        assert_eq!(
            fts_query("cat:quant-ph -review").unwrap(),
            r#"fmeta : "quant-ph"* NOT "review"*"#
        );
        assert_eq!(
            fts_query("qubit OR qudit noise").unwrap(),
            r#"("qubit"* OR "qudit"*) AND "noise"*"#
        );
        // An unknown field is just a word with a colon in it.
        assert_eq!(fts_query("ratio:2").unwrap(), r#""ratio:2"*"#);
    }

    #[test]
    fn nothing_to_look_for_gives_none() {
        for q in ["", "   ", "$ ^ {}", "-noise", "OR", "au:"] {
            assert_eq!(fts_query(q), None, "{q:?}");
        }
        // Quotes inside a word can't break out of the string.
        assert_eq!(fts_query(r#"a"b"#).unwrap(), r#""a"* AND "b""#);
        assert_eq!(fts_query(r#"x"y""z"#).unwrap(), r#""x"* AND "y" AND "z""#);
    }
}
