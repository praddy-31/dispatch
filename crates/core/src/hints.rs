//! Calmer reading: what an abstract tells you before you open the paper.
//!
//! - **Code and data links**: GitHub, GitLab, Zenodo, Figshare, Hugging Face, OSF, Dryad,
//!   Software Heritage and Codeberg links in the abstract (authors often write them without
//!   `https://`), shown as badges.
//! - **Reading hints**: "long abstract" (over [`LONG_WORDS`] words) and "maths-heavy" (many
//!   formulas, or a large share of the text inside `$…$`).
//!
//! Computed when rows are listed (cheap, nothing stored), so older papers get them too.

use serde::Serialize;

use crate::store::Row;

/// Words above which an abstract counts as long (arXiv abstracts are mostly 120–220).
pub const LONG_WORDS: usize = 250;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodeLink {
    /// "GitHub", "Zenodo", … (what the badge says).
    pub kind: &'static str,
    /// Always `https://…`.
    pub url: String,
}

/// Hosts that mean code or data, and the badge's name.
const HOSTS: [(&str, &str); 11] = [
    ("github.com", "GitHub"),
    ("gitlab.com", "GitLab"),
    ("codeberg.org", "Codeberg"),
    ("bitbucket.org", "Bitbucket"),
    ("zenodo.org", "Zenodo"),
    ("doi.org/10.5281/zenodo", "Zenodo"),
    ("figshare.com", "Figshare"),
    ("huggingface.co", "Hugging Face"),
    ("osf.io", "OSF"),
    ("datadryad.org", "Dryad"),
    ("softwareheritage.org", "Software Heritage"),
];

/// Code and data links in a text, in order, without repeats.
pub fn code_links(text: &str) -> Vec<CodeLink> {
    let mut out: Vec<CodeLink> = Vec::new();
    for raw in text.split(|c: char| {
        c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>' | '"' | '{' | '}' | '\\')
    }) {
        let token =
            raw.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | ':' | '\'' | ']' | '[' | '`'));
        let bare = token
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.");
        let lower = bare.to_ascii_lowercase();
        let Some((_, kind)) = HOSTS
            .iter()
            .find(|(host, _)| lower.starts_with(host) || lower.contains(&format!(".{host}")))
        else {
            continue;
        };
        // A bare host ("github.com") is not a link to anything in particular.
        let Some((_, path)) = bare.split_once('/') else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let url = format!("https://{bare}");
        if !out.iter().any(|l| l.url.eq_ignore_ascii_case(&url)) {
            out.push(CodeLink { kind, url });
        }
    }
    out
}

/// "long abstract", "maths-heavy", or both (in that order), from the text alone.
pub fn reading_hints(text: &str) -> Vec<&'static str> {
    let mut out = Vec::new();
    let words = text.split_whitespace().count();
    if words > LONG_WORDS {
        out.push("long abstract");
    }
    let (formulas, inside) = maths(text);
    let share = if text.is_empty() {
        0.0
    } else {
        inside as f64 / text.chars().count() as f64
    };
    if formulas >= 6 || (formulas >= 2 && share >= 0.15) {
        out.push("maths-heavy");
    }
    out
}

/// How many `$…$` formulas, and how many characters sit inside them.
fn maths(text: &str) -> (usize, usize) {
    let mut count = 0;
    let mut inside = 0;
    let mut open = false;
    let mut chars = text.chars().peekable();
    let mut prev = ' ';
    while let Some(c) = chars.next() {
        if c == '$' && prev != '\\' {
            if chars.peek() == Some(&'$') {
                chars.next(); // $$ … $$ counts like $ … $
            }
            if open {
                count += 1;
            }
            open = !open;
        } else if open {
            inside += 1;
        }
        prev = c;
    }
    (count, inside)
}

/// Fills each row's `code` and `hints` (research papers only; news summaries are teasers).
pub fn annotate(rows: &mut [Row]) {
    for r in rows {
        if r.item.section != crate::Section::Research {
            continue;
        }
        r.code = code_links(&r.item.summary);
        r.hints = reading_hints(&r.item.summary)
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_code_and_data_links_written_any_way() {
        let text = "Code: github.com/ada/spin-chains. Data at https://zenodo.org/records/123 and \
                    (https://doi.org/10.5281/zenodo.456); see GitHub.com/ada/spin-chains again, \
                    github.com alone, and https://arxiv.org/abs/1.";
        let links = code_links(text);
        let got: Vec<_> = links.iter().map(|l| (l.kind, l.url.as_str())).collect();
        assert_eq!(
            got,
            vec![
                ("GitHub", "https://github.com/ada/spin-chains"),
                ("Zenodo", "https://zenodo.org/records/123"),
                ("Zenodo", "https://doi.org/10.5281/zenodo.456"),
            ]
        );
        assert!(code_links("No links here.").is_empty());
    }

    #[test]
    fn hints_for_long_and_maths_heavy_abstracts() {
        assert!(reading_hints("A short abstract.").is_empty());
        let long = "word ".repeat(LONG_WORDS + 1);
        assert_eq!(reading_hints(&long), vec!["long abstract"]);
        let maths = "We find $a$, $b$, $c$, $d$, $e$ and $$f = g$$ in it.";
        assert_eq!(reading_hints(maths), vec!["maths-heavy"]);
        let dense =
            "Let $H = \\sum_i J S_i \\cdot S_{i+1}$ and $Z = \\mathrm{Tr}\\, e^{-\\beta H}$.";
        assert_eq!(reading_hints(dense), vec!["maths-heavy"]);
        assert!(reading_hints("It costs \\$5 and \\$6.").is_empty());
    }
}
