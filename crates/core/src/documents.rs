//! "Dispatch documents", one folder for the live `.bib` and the PDFs Dispatch
//! downloads. Choosing another folder moves them along: the `.bib` (only if Dispatch wrote it)
//! and the PDFs named as Dispatch names them (`pdf::tidy_name` of a paper in the archive).
//! Nothing else in the folder is touched, and nothing at the destination is replaced.

use std::collections::HashSet;
use std::path::Path;

use crate::bibfile;
use crate::item::Item;
use crate::pdf;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Moved {
    pub bib: bool,
    pub pdfs: usize,
    /// Files left where they were because the new folder already has one of that name.
    pub kept: Vec<String>,
}

impl Moved {
    /// "Moved dispatch.bib and 12 PDFs" for a toast.
    pub fn summary(&self, bib_name: &str) -> String {
        let pdfs = match self.pdfs {
            1 => "1 PDF".to_owned(),
            n => format!("{n} PDFs"),
        };
        let mut out = match (self.bib, self.pdfs) {
            (false, 0) => "Nothing to move".to_owned(),
            (true, 0) => format!("Moved {bib_name}"),
            (false, _) => format!("Moved {pdfs}"),
            (true, _) => format!("Moved {bib_name} and {pdfs}"),
        };
        if !self.kept.is_empty() {
            out.push_str(&format!(
                "; {} left in the old folder (the new one has a file of that name)",
                self.kept.len()
            ));
        }
        out
    }
}

/// The file names Dispatch gives the PDFs of these papers.
pub fn pdf_names(papers: &[Item]) -> HashSet<String> {
    papers
        .iter()
        .map(|p| format!("{}.pdf", pdf::tidy_name(p)))
        .collect()
}

/// Moves Dispatch's files from `from` to `to` (made if missing).
pub fn move_all(
    from: &Path,
    to: &Path,
    bib_name: &str,
    pdf_names: &HashSet<String>,
) -> Result<Moved, String> {
    let mut moved = Moved::default();
    if from == to || !from.is_dir() {
        return Ok(moved);
    }
    std::fs::create_dir_all(to).map_err(|e| format!("can't make {}: {e}", to.display()))?;
    let bib = from.join(bib_name);
    let ours = std::fs::read_to_string(&bib).is_ok_and(|t| t.starts_with(bibfile::HEADER));
    if ours {
        let target = to.join(bib_name);
        let theirs =
            std::fs::read_to_string(&target).is_ok_and(|t| !t.starts_with(bibfile::HEADER));
        if theirs || (target.exists() && std::fs::read_to_string(&target).is_err()) {
            moved.kept.push(bib_name.to_owned());
        } else {
            // Ours on both sides: the new one is rewritten from the database anyway.
            move_file(&bib, &target)?;
            moved.bib = true;
        }
    }
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("can't read {}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !pdf_names.contains(&name) || !entry.path().is_file() {
            continue;
        }
        let target = to.join(&name);
        if target.exists() {
            moved.kept.push(name);
            continue;
        }
        move_file(&entry.path(), &target)?;
        moved.pdfs += 1;
    }
    Ok(moved)
}

/// `rename`, or copy then remove when the folders are on different disks.
fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("can't copy {}: {e}", from.display()))?;
    std::fs::remove_file(from)
        .map_err(|e| format!("copied, but can't remove {}: {e}", from.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_only_dispatch_files_and_never_replaces() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let to = new.path().join("Dispatch");
        std::fs::write(
            old.path().join("dispatch.bib"),
            format!("{}\n@x", bibfile::HEADER),
        )
        .unwrap();
        std::fs::write(old.path().join("2026 Smith - A.pdf"), "%PDF a").unwrap();
        std::fs::write(old.path().join("2026 Li - B.pdf"), "%PDF b").unwrap();
        std::fs::write(old.path().join("my own.pdf"), "%PDF mine").unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(to.join("2026 Li - B.pdf"), "%PDF other").unwrap();
        let names: HashSet<String> = ["2026 Smith - A.pdf", "2026 Li - B.pdf"]
            .map(String::from)
            .into();
        let m = move_all(old.path(), &to, "dispatch.bib", &names).unwrap();
        assert!(m.bib);
        assert_eq!(m.pdfs, 1);
        assert_eq!(m.kept, ["2026 Li - B.pdf"]);
        assert!(to.join("dispatch.bib").exists() && to.join("2026 Smith - A.pdf").exists());
        // Not Dispatch's, or already there: left alone, nothing replaced.
        assert!(old.path().join("my own.pdf").exists());
        assert_eq!(
            std::fs::read_to_string(to.join("2026 Li - B.pdf")).unwrap(),
            "%PDF other"
        );
        assert!(
            m.summary("dispatch.bib")
                .starts_with("Moved dispatch.bib and 1 PDF; 1 left")
        );
        // A .bib Dispatch didn't write stays put.
        std::fs::write(old.path().join("refs.bib"), "@mine").unwrap();
        let m = move_all(old.path(), &to, "refs.bib", &HashSet::new()).unwrap();
        assert_eq!(m, Moved::default());
        assert!(old.path().join("refs.bib").exists());
    }
}
