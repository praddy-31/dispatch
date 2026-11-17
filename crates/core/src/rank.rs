//! Ranking: the "For you" order of Research, and the menu's Top 5.
//!
//! A paper's score has two layers (keyword scores first, then similarity):
//! - **Strong signals:** each watched keyword in its title or abstract and each followed author
//!   adds [`STRONG`], so any such paper comes before every paper without one.
//! - **Similarity to your papers:** the [`Profile`] is the words that recur across the papers you
//!   saved, noted or tagged. They become one FTS5 `OR` query over title and abstract, and SQLite's
//!   BM25 score (rare words count more, long abstracts less) says how close each paper is.
//!   Scaled to 0–[`SIMILAR`] within the list, so it only orders papers among themselves.
//!
//! It also adds the papers you *opened* to the profile as a weaker signal (half a vote each),
//! [`suggested`] (the best unread papers of a stretch of days, across all topics; also the
//! menu's Top 5) and [`related`] (papers like one paper, over the whole archive).
//!
//! No neural network and nothing downloaded: it is the same index Search uses.

use std::collections::{HashMap, HashSet};

use rusqlite::params;
use serde::Serialize;

use crate::follow::{Follow, fold};
use crate::store::{Row, Store};

/// Per watched keyword or followed author.
pub const STRONG: f64 = 100.0;
/// The most similarity can add.
pub const SIMILAR: f64 = 10.0;
/// Words in the profile query.
pub const PROFILE_TERMS: usize = 40;
/// Your most recent papers the profile is built from.
pub const PROFILE_PAPERS: i64 = 300;
/// Similarity (0–1 within the list) from which a paper's reason names the shared words.
pub const WHY_SIMILAR: f64 = 0.3;
/// An opened paper's vote in the profile, against 1 for a saved, noted or tagged one.
pub const OPENED_WEIGHT: f64 = 0.5;
/// Suggested: similarity (0–1 of the week's best) a paper needs without a keyword or author.
pub const SUGGEST_MIN: f64 = 0.4;
/// Suggested: at most this many papers.
pub const SUGGEST_MAX: usize = 30;
/// Related: papers shown under a paper.
pub const RELATED: usize = 6;
/// Related: a paper must score this share of the closest one's score.
pub const RELATED_MIN: f64 = 0.75;
/// Related: candidates the index hands over for re-scoring.
pub const RELATED_POOL: usize = 300;

/// Words too common in abstracts to say anything about a paper's subject.
const STOPWORDS: &[&str] = &[
    "about",
    "above",
    "across",
    "after",
    "again",
    "against",
    "allow",
    "allows",
    "along",
    "also",
    "although",
    "among",
    "analysis",
    "analyze",
    "another",
    "approach",
    "approaches",
    "based",
    "because",
    "been",
    "before",
    "being",
    "below",
    "between",
    "both",
    "case",
    "cases",
    "could",
    "data",
    "demonstrate",
    "demonstrated",
    "derive",
    "derived",
    "describe",
    "different",
    "does",
    "done",
    "due",
    "during",
    "each",
    "effect",
    "effects",
    "either",
    "enables",
    "even",
    "every",
    "existing",
    "experimental",
    "explore",
    "find",
    "finding",
    "first",
    "found",
    "framework",
    "from",
    "further",
    "general",
    "give",
    "given",
    "gives",
    "have",
    "having",
    "here",
    "high",
    "however",
    "important",
    "including",
    "into",
    "investigate",
    "itself",
    "known",
    "large",
    "leads",
    "less",
    "like",
    "low",
    "made",
    "make",
    "many",
    "method",
    "methods",
    "model",
    "models",
    "more",
    "most",
    "much",
    "must",
    "near",
    "need",
    "new",
    "novel",
    "number",
    "observed",
    "obtain",
    "obtained",
    "only",
    "other",
    "otherwise",
    "over",
    "paper",
    "part",
    "particular",
    "perform",
    "performance",
    "present",
    "presented",
    "previous",
    "problem",
    "propose",
    "proposed",
    "provide",
    "provides",
    "range",
    "recent",
    "related",
    "report",
    "reported",
    "respectively",
    "result",
    "results",
    "same",
    "second",
    "several",
    "should",
    "show",
    "showed",
    "shown",
    "shows",
    "significant",
    "significantly",
    "similar",
    "since",
    "small",
    "some",
    "such",
    "suggest",
    "system",
    "systems",
    "than",
    "that",
    "their",
    "them",
    "then",
    "theory",
    "there",
    "these",
    "they",
    "this",
    "those",
    "three",
    "through",
    "thus",
    "type",
    "under",
    "upon",
    "used",
    "using",
    "various",
    "very",
    "well",
    "were",
    "what",
    "when",
    "where",
    "whether",
    "which",
    "while",
    "whose",
    "will",
    "with",
    "within",
    "without",
    "work",
    "would",
    "yield",
    "yields",
];

/// The words that describe the user's own papers.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Profile {
    /// Most shared first.
    pub terms: Vec<String>,
    /// Saved, noted or tagged papers it was built from.
    pub papers: usize,
    /// Opened papers it was also built from (weaker, [`OPENED_WEIGHT`]).
    pub opened: usize,
}

impl Profile {
    /// From your saved, noted or tagged papers (newest [`PROFILE_PAPERS`]), plus the papers
    /// you opened at half weight. A word needs two votes once there are five or more papers'
    /// worth, so one odd paper doesn't steer the list.
    pub fn build(store: &Store) -> Result<Profile, String> {
        let query = |sql: &str| -> Result<Vec<(String, String)>, String> {
            let mut st = store
                .conn()
                .prepare_cached(sql)
                .map_err(|e| format!("database: {e}"))?;
            st.query_map([PROFILE_PAPERS], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| format!("database: {e}"))?
                .collect::<Result<_, _>>()
                .map_err(|e| format!("database: {e}"))
        };
        let mine = query(
            "SELECT i.title, i.summary FROM items i JOIN user_items u ON u.key = i.key
             WHERE i.section = 'research'
               AND (u.saved_at IS NOT NULL OR u.note != '' OR u.tags != '[]')
             ORDER BY MAX(COALESCE(u.saved_at, 0), u.note_stamp / 1000, u.tags_stamp / 1000) DESC
             LIMIT ?1",
        )?;
        let opened = query(
            "SELECT i.title, i.summary FROM items i LEFT JOIN user_items u ON u.key = i.key
             WHERE i.section = 'research' AND i.opened_at IS NOT NULL
               AND NOT (u.saved_at IS NOT NULL OR COALESCE(u.note, '') != '' OR COALESCE(u.tags, '[]') != '[]')
             ORDER BY i.opened_at DESC
             LIMIT ?1",
        )?;
        // `votes` order the words; `papers` (no title bonus) decide whether a word is shared.
        let mut votes: HashMap<String, f64> = HashMap::new();
        let mut papers: HashMap<String, f64> = HashMap::new();
        for (texts, weight) in [(&mine, 1.0), (&opened, OPENED_WEIGHT)] {
            for (title, summary) in texts {
                // A title word is the paper's subject: it counts double (and breaks the many
                // ties a small profile has, which otherwise fall to alphabetical order).
                let title = words(title);
                for w in words(summary).union(&title) {
                    let bonus = if title.contains(w) { 2.0 } else { 1.0 };
                    *votes.entry(w.clone()).or_default() += weight * bonus;
                    *papers.entry(w.clone()).or_default() += weight;
                }
            }
        }
        let worth = mine.len() as f64 + OPENED_WEIGHT * opened.len() as f64;
        let min = if worth >= 5.0 { 2.0 } else { 0.5 };
        let mut terms: Vec<(String, f64)> = votes
            .into_iter()
            .filter(|(w, _)| papers.get(w).copied().unwrap_or(0.0) >= min)
            .collect();
        terms.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        terms.truncate(PROFILE_TERMS);
        Ok(Profile {
            terms: terms.into_iter().map(|(w, _)| w).collect(),
            papers: mine.len(),
            opened: opened.len(),
        })
    }

    /// The FTS5 query: any profile word in the title or abstract (None without a profile).
    /// Each word is a prefix, so "chain" also finds "chains".
    pub fn fts_query(&self) -> Option<String> {
        (!self.terms.is_empty()).then(|| {
            let any: Vec<String> = self.terms.iter().map(|t| format!("\"{t}\"*")).collect();
            format!("{{ftitle fabstract}} : ({})", any.join(" OR "))
        })
    }
}

/// The subject words of a text: folded to plain lower case, LaTeX maths and commands left out,
/// at least four letters, no stopwords or numbers, a plural "s" dropped ("chains" → "chain";
/// not "-ss", "-us", "-is": "glass", "modulus", "analysis").
pub fn words(text: &str) -> HashSet<String> {
    let mut plain = String::with_capacity(text.len());
    let mut in_math = false;
    let mut in_command = false;
    for c in text.chars() {
        match c {
            '$' => in_math = !in_math,
            '\\' => in_command = true,
            _ if in_math => {}
            _ if in_command && c.is_alphabetic() => {}
            _ => {
                in_command = false;
                plain.push(c);
            }
        }
    }
    fold(&plain)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 4 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOPWORDS.contains(w))
        .map(|w| {
            let plural = w.len() > 4
                && w.ends_with('s')
                && !["ss", "us", "is"].iter().any(|e| w.ends_with(e));
            w[..w.len() - plural as usize].to_owned()
        })
        .collect()
}

/// Scores `rows` in place (`score`, `why`). The list order is the caller's to change.
pub fn score_rows(
    store: &Store,
    follow: &Follow,
    profile: &Profile,
    rows: &mut [Row],
) -> Result<(), String> {
    let sim = similarity(store, profile, rows)?;
    let best = sim.values().copied().fold(0.0_f64, f64::max);
    let profile_words: Vec<&str> = profile.terms.iter().map(String::as_str).collect();
    for r in rows.iter_mut() {
        let keywords = follow.keywords_in(&r.item);
        let authors = follow.authors_in(&r.item);
        let s = if best > 0.0 {
            sim.get(&r.item.key).copied().unwrap_or(0.0) / best
        } else {
            0.0
        };
        let mut why: Vec<String> = keywords.iter().map(|k| format!("keyword “{k}”")).collect();
        why.extend(authors.iter().map(|a| format!("you follow {a}")));
        if s >= WHY_SIMILAR {
            let mine = words(&format!("{} {}", r.item.title, r.item.summary));
            let shared: Vec<&str> = profile_words
                .iter()
                .filter(|w| mine.contains(**w))
                .take(3)
                .copied()
                .collect();
            if !shared.is_empty() {
                why.push(format!("like your papers: {}", shared.join(", ")));
            }
        }
        r.score = Some(STRONG * (keywords.len() + authors.len()) as f64 + SIMILAR * s);
        r.why = why;
    }
    Ok(())
}

/// BM25 closeness of each row to the profile (higher = closer; missing = no shared word).
fn similarity(
    store: &Store,
    profile: &Profile,
    rows: &[Row],
) -> Result<HashMap<String, f64>, String> {
    let Some(query) = profile.fts_query() else {
        return Ok(HashMap::new());
    };
    let keys: Vec<&str> = rows.iter().map(|r| r.item.key.as_str()).collect();
    let keys = serde_json::to_string(&keys).map_err(|e| e.to_string())?;
    // bm25() is lower for a better match; the weights count the title ten times the abstract.
    let mut st = store
        .conn()
        .prepare_cached(
            "SELECT s.key, -bm25(search, 10.0, 0.0, 1.0, 0.0, 0.0) FROM search
             JOIN search_ids s ON s.id = search.rowid
             WHERE search MATCH ?1 AND s.key IN (SELECT value FROM json_each(?2))",
        )
        .map_err(|e| format!("database: {e}"))?;
    let out = st
        .query_map(params![query, keys], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(|e| format!("database: {e}"))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("database: {e}"))?;
    Ok(out)
}

/// Suggested: new papers you haven't opened, first seen in `[from, to)` (Unix seconds), across
/// all topics, best first. A paper needs a watched keyword, a followed author, or
/// [`SUGGEST_MIN`] of the best similarity; at most [`SUGGEST_MAX`].
pub fn suggested(store: &Store, follow: &Follow, from: i64, to: i64) -> Result<Vec<Row>, String> {
    let profile = Profile::build(store)?;
    let mut rows = store.unopened_research_between(from, to)?;
    // Papers you already saved, noted or tagged aren't news to you.
    rows.retain(|r| !r.saved && r.note.is_empty() && r.tags.is_empty());
    score_rows(store, follow, &profile, &mut rows)?;
    rows.retain(|r| r.score.unwrap_or(0.0) >= SIMILAR * SUGGEST_MIN);
    sort(&mut rows);
    rows.truncate(SUGGEST_MAX);
    Ok(rows)
}

/// Related: the papers in the archive that share the most *rare* subject words
/// with `key`, itself left out. The index finds up to [`RELATED_POOL`] candidates (any shared
/// word); each is then scored by the words it shares, weighted by rarity (log N/df, so "dot"
/// counts more than "control"), doubled for words in `key`'s title, and divided by the
/// square root of its own word count (at least 60) so long abstracts don't win by size. One stray word in a
/// title (a "gate" in an LSTM paper) can't carry a paper this way.
pub fn related(store: &Store, key: &str, limit: usize) -> Result<Vec<Row>, String> {
    let Some(item) = store.get(key)? else {
        return Ok(vec![]);
    };
    let title = words(&item.title);
    let mut terms: Vec<String> = words(&format!("{} {}", item.title, item.summary))
        .into_iter()
        .collect();
    terms.sort();
    if terms.is_empty() {
        return Ok(vec![]);
    }
    let db = |e: rusqlite::Error| format!("database: {e}");
    let conn = store.conn();
    let total: f64 = conn
        .query_row("SELECT COUNT(*) FROM search_ids", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(db)?
        .max(1) as f64;
    // Rarity of each word over the archive (one small index count per word).
    let mut idf: HashMap<String, f64> = HashMap::new();
    {
        let mut st = conn
            .prepare_cached("SELECT COUNT(*) FROM search WHERE search MATCH ?1")
            .map_err(db)?;
        for t in &terms {
            let q = format!("{{ftitle fabstract}} : \"{t}\"*");
            let df: i64 = st.query_row([&q], |r| r.get(0)).map_err(db)?;
            idf.insert(t.clone(), (total / df.max(1) as f64).ln().max(0.0));
        }
    }
    // Keep the rarest words for the candidate query (common ones only add noise).
    let mut by_rarity = terms.clone();
    by_rarity.sort_by(|a, b| idf[b].total_cmp(&idf[a]).then(a.cmp(b)));
    by_rarity.truncate(PROFILE_TERMS);
    let any: Vec<String> = by_rarity.iter().map(|t| format!("\"{t}\"*")).collect();
    let query = format!("{{ftitle fabstract}} : ({})", any.join(" OR "));
    let mut st = conn
        .prepare_cached(
            "SELECT s.key, i.title, i.summary FROM search JOIN search_ids s ON s.id = search.rowid
             JOIN items i ON i.key = s.key
             WHERE search MATCH ?1 AND s.key != ?2 AND i.section = 'research'
             ORDER BY bm25(search, 10.0, 0.0, 1.0, 0.0, 0.0) LIMIT ?3",
        )
        .map_err(db)?;
    let pool: Vec<(String, String, String)> = st
        .query_map(params![query, key, RELATED_POOL as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(db)?
        .collect::<Result<_, _>>()
        .map_err(db)?;
    let mut scored: Vec<(f64, String)> = pool
        .into_iter()
        .map(|(k, t, sm)| {
            let their_title = words(&t);
            let theirs = words(&format!("{t} {sm}"));
            let shared: f64 = by_rarity
                .iter()
                .filter(|w| theirs.contains(*w))
                .map(|w| {
                    let mine = if title.contains(w) { 2.0 } else { 1.0 };
                    let their = if their_title.contains(w) { 1.5 } else { 1.0 };
                    idf[w] * mine * their
                })
                .sum();
            // A floor of 60 words: a two-line abstract mustn't win by being short.
            (shared / (theirs.len().max(60) as f64).sqrt(), k)
        })
        .filter(|(score, _)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    // Only papers about as close as the closest: a thin archive gives fewer, not worse.
    let best = scored.first().map(|x| x.0).unwrap_or(0.0);
    scored.retain(|x| x.0 >= RELATED_MIN * best);
    scored.truncate(limit);
    let keys: Vec<String> = scored.into_iter().map(|(_, k)| k).collect();
    store.rows(&keys)
}

/// Highest score first; equal scores keep their order (newest first, as listed).
pub fn sort(rows: &mut [Row]) {
    rows.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_skip_maths_commands_stopwords_and_short_ones() {
        let w =
            words(r"We show that $\alpha$-stable Floquet qubits in a \emph{Kagomé} lattice: 2026");
        let mut w: Vec<_> = w.into_iter().collect();
        w.sort();
        assert_eq!(w, vec!["floquet", "kagome", "lattice", "qubit", "stable"]);
        let w = words("glass modulus synthesis chains physics");
        let mut w: Vec<_> = w.into_iter().collect();
        w.sort();
        assert_eq!(w, vec!["chain", "glass", "modulus", "physic", "synthesis"]);
    }
}
