//! Markdown for Obsidian (or any folder of notes).
//!
//! - **One note per saved paper** in a folder you choose: front matter (title, authors, arXiv
//!   id, dates, tags), the abstract (Obsidian shows `$…$` maths) and your Dispatch note.
//!   Kept in sync, with one rule: **a note you edited is left alone.**
//!   Dispatch remembers a fingerprint of what it last wrote (`vault.json` in the data folder);
//!   a file that no longer matches is yours. A note you deleted isn't written again, and
//!   removing a paper from Saved never deletes its note.
//! - **The week's reading log**: every note you wrote this week, as one Markdown file (copied,
//!   and written into the folder as `Reading log 2026-W44.md` under the same rule).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::paths::write_atomic;
use crate::pdf::tidy_name;
use crate::store::Row;

/// In the data folder: which notes Dispatch wrote and their fingerprints.
pub const MANIFEST: &str = "vault.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The folder these notes are in; a different folder starts a fresh manifest.
    pub dir: String,
    /// Item key (or `log:2026-W44`) → the note.
    pub notes: BTreeMap<String, Written>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Written {
    pub file: String,
    /// FNV-1a of the text Dispatch last wrote.
    pub hash: String,
}

/// What a sync did, for Settings' status line.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    pub written: usize,
    pub unchanged: usize,
    /// Notes you edited (or deleted), left alone.
    pub yours: usize,
}

impl Manifest {
    pub fn load(data_dir: &Path, dir: &Path) -> Manifest {
        let here = dir.display().to_string();
        std::fs::read_to_string(data_dir.join(MANIFEST))
            .ok()
            .and_then(|t| serde_json::from_str::<Manifest>(&t).ok())
            .filter(|m| m.dir == here)
            .unwrap_or(Manifest {
                dir: here,
                notes: BTreeMap::new(),
            })
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        write_atomic(&data_dir.join(MANIFEST), json.as_bytes())
    }
}

/// FNV-1a, 64 bits: stable across Rust versions (unlike `DefaultHasher`), enough to tell
/// "the file is still what Dispatch wrote".
fn fingerprint(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// YAML string in double quotes.
fn yaml(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Obsidian tags can't hold spaces.
fn tag(s: &str) -> String {
    s.trim().replace(char::is_whitespace, "-")
}

/// One saved paper as a Markdown note.
pub fn note(row: &Row) -> String {
    let item = &row.item;
    let id = crate::cite::arxiv_id(item);
    let mut fm = vec![
        format!("title: {}", yaml(item.title.trim())),
        format!(
            "authors: [{}]",
            item.authors
                .iter()
                .map(|a| yaml(a))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ];
    if let Some(id) = id {
        fm.push(format!("arxiv: {}", yaml(id)));
    }
    if let Some(p) = item.published {
        fm.push(format!("published: {}", p.format("%Y-%m-%d")));
    }
    if let Some(j) = &item.journal_ref {
        fm.push(format!("journal: {}", yaml(j)));
    }
    if let Some(d) = &item.doi {
        fm.push(format!("doi: {}", yaml(d)));
    }
    if let Some(s) = row.saved_at {
        fm.push(format!("saved: {}", s.format("%Y-%m-%d")));
    }
    let mut tags = vec!["paper".to_owned()];
    tags.extend(row.tags.iter().map(|t| tag(t)).filter(|t| !t.is_empty()));
    fm.push(format!("tags: [{}]", tags.join(", ")));
    fm.push(format!("url: {}", item.link));

    let mut links = vec![format!("[arXiv]({})", item.link)];
    if let Some(pdf) = crate::pdf::url(item) {
        links.push(format!("[PDF]({pdf})"));
    }
    if let Some(id) = id {
        links.push(format!("[Open in Dispatch](dispatch://paper/{id})"));
    }
    let mut out = format!(
        "---\n{}\n---\n\n# {}\n\n{}\n\n{}\n\n## Abstract\n\n{}\n",
        fm.join("\n"),
        item.title.trim(),
        item.authors.join(", "),
        links.join(" · "),
        item.summary.trim()
    );
    if !row.note.trim().is_empty() {
        out.push_str(&format!("\n## My note\n\n{}\n", row.note.trim()));
    }
    out
}

/// Writes `text` as the note for `id` under the rule in the module notes. `name` is used
/// for a note Dispatch hasn't written before.
fn put(
    dir: &Path,
    m: &mut Manifest,
    id: &str,
    name: &str,
    text: &str,
    r: &mut Report,
) -> Result<(), String> {
    if let Some(w) = m.notes.get(id) {
        let path = dir.join(&w.file);
        match std::fs::read_to_string(&path) {
            Ok(now) if fingerprint(&now) != w.hash => r.yours += 1,
            Ok(now) if now == text => r.unchanged += 1,
            Ok(_) => {
                write_atomic(&path, text.as_bytes())?;
                m.notes.get_mut(id).expect("present").hash = fingerprint(text);
                r.written += 1;
            }
            // Deleted (or moved) in Obsidian: yours to decide, not written again.
            Err(_) => r.yours += 1,
        }
        return Ok(());
    }
    // A new note: a free file name (a file of yours with that name is never replaced).
    let mut file = format!("{name}.md");
    let mut n = 2;
    while dir.join(&file).exists() {
        file = format!("{name} ({n}).md");
        n += 1;
    }
    write_atomic(&dir.join(&file), text.as_bytes())?;
    m.notes.insert(
        id.to_owned(),
        Written {
            file,
            hash: fingerprint(text),
        },
    );
    r.written += 1;
    Ok(())
}

/// Brings the folder up to date with the saved papers.
pub fn sync(data_dir: &Path, dir: &Path, saved: &[Row]) -> Result<Report, String> {
    if !dir.is_dir() {
        return Err(format!(
            "{} isn't a folder (moved or renamed?)",
            dir.display()
        ));
    }
    let mut m = Manifest::load(data_dir, dir);
    let mut r = Report::default();
    for row in saved {
        put(
            dir,
            &mut m,
            &row.item.key,
            &tidy_name(&row.item),
            &note(row),
            &mut r,
        )?;
    }
    m.save(data_dir)?;
    Ok(r)
}

/// The reading log: `notes` are the rows whose note changed in the week, oldest first.
pub fn reading_log(label: &str, notes: &[Row]) -> String {
    let mut out = format!("# Reading log, {label}\n");
    if notes.is_empty() {
        out.push_str("\nNo notes this week.\n");
    }
    for row in notes {
        let item = &row.item;
        let who = match item.authors.len() {
            0 => String::new(),
            1 => item.authors[0].clone(),
            _ => format!("{} et al.", item.authors[0]),
        };
        out.push_str(&format!(
            "\n## {}\n\n{who} · [link]({})\n\n{}\n",
            item.title.trim(),
            item.link,
            row.note.trim()
        ));
    }
    out
}

/// Writes the week's log into the folder (same rule: left alone once you edit it).
pub fn write_log(data_dir: &Path, dir: &Path, week: &str, text: &str) -> Result<PathBuf, String> {
    let mut m = Manifest::load(data_dir, dir);
    let id = format!("log:{week}");
    put(
        dir,
        &mut m,
        &id,
        &format!("Reading log {week}"),
        text,
        &mut Report::default(),
    )?;
    m.save(data_dir)?;
    Ok(dir.join(&m.notes[&id].file))
}

/// ISO week label for a time (`2026-W44`), in the zone the caller converted it to.
pub fn week_label<Tz: chrono::TimeZone>(t: &DateTime<Tz>) -> String {
    use chrono::Datelike;
    let w = t.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, Section};
    use chrono::Utc;

    fn row(id: &str, note: &str) -> Row {
        Row {
            item: Item {
                key: format!("arxiv:{id}"),
                section: Section::Research,
                source: "arxiv".into(),
                source_name: "arXiv".into(),
                title: "Spin \"chains\" with $J_2$".into(),
                authors: vec!["Ada Smith".into(), "Bo Chen".into()],
                summary: "We find $\\Delta$.".into(),
                link: format!("https://arxiv.org/abs/{id}"),
                pdf: None,
                categories: vec![],
                published: "2026-10-27T04:00:00Z".parse().ok(),
                announce_type: None,
                version: None,
                doi: None,
                journal_ref: None,
            },
            topics: vec![],
            first_seen: Utc::now(),
            read: false,
            opened: false,
            later_at: None,
            saved: true,
            saved_at: "2026-10-30T04:00:00Z".parse().ok(),
            note: note.into(),
            tags: vec!["to read".into()],
            followed: vec![],
            score: None,
            why: vec![],
            code: vec![],
            hints: vec![],
            discussion: None,
            also: vec![],
        }
    }

    #[test]
    fn a_note_has_front_matter_abstract_and_your_note() {
        let n = note(&row("2610.00001", "Check eq. 3."));
        assert!(n.starts_with("---\ntitle: \"Spin \\\"chains\\\" with $J_2$\"\n"));
        assert!(n.contains("arxiv: \"2610.00001\""));
        assert!(n.contains("tags: [paper, to-read]"));
        assert!(n.contains("We find $\\Delta$."));
        assert!(n.contains("## My note\n\nCheck eq. 3."));
        assert!(n.contains("dispatch://paper/2610.00001"));
    }

    #[test]
    fn edited_and_deleted_notes_are_left_alone() {
        let data = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let (d, v) = (data.path(), vault.path());
        // A file of yours with the same name is never touched.
        let name = tidy_name(&row("2610.00001", "").item);
        std::fs::write(v.join(format!("{name}.md")), "mine").unwrap();

        let r = sync(d, v, &[row("2610.00001", "first")]).unwrap();
        assert_eq!((r.written, r.yours), (1, 0));
        assert_eq!(
            std::fs::read_to_string(v.join(format!("{name}.md"))).unwrap(),
            "mine"
        );
        let ours = v.join(format!("{name} (2).md"));
        assert!(std::fs::read_to_string(&ours).unwrap().contains("first"));

        // Untouched: updated with the new note; unchanged: not rewritten.
        let r = sync(d, v, &[row("2610.00001", "second")]).unwrap();
        assert_eq!(r.written, 1);
        assert_eq!(
            sync(d, v, &[row("2610.00001", "second")])
                .unwrap()
                .unchanged,
            1
        );

        // Edited in Obsidian: left alone from now on.
        std::fs::write(&ours, "my own words").unwrap();
        let r = sync(d, v, &[row("2610.00001", "third")]).unwrap();
        assert_eq!((r.written, r.yours), (0, 1));
        assert_eq!(std::fs::read_to_string(&ours).unwrap(), "my own words");

        // Deleted: not written again.
        std::fs::remove_file(&ours).unwrap();
        assert_eq!(sync(d, v, &[row("2610.00001", "4")]).unwrap().yours, 1);
        assert!(!ours.exists());
    }

    #[test]
    fn the_reading_log() {
        let log = reading_log("2026-W44", &[row("2610.00001", "Nice trick.")]);
        assert!(log.starts_with("# Reading log, 2026-W44\n"));
        assert!(log.contains("Ada Smith et al. · [link](https://arxiv.org/abs/2610.00001)"));
        assert!(log.contains("Nice trick."));
        let t: DateTime<Utc> = "2026-10-29T00:00:00Z".parse().unwrap();
        assert_eq!(week_label(&t), "2026-W44");
    }
}
