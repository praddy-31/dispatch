//! PDF hand-off. Downloads a paper's PDF from arxiv.org (only when asked, one
//! request) into a folder with a tidy name, `2026 Smith - Adiabatic transfer.pdf`, so the
//! PDF app of your choice opens it. A file already there is reused, not fetched again.

use std::path::{Path, PathBuf};

use chrono::Datelike;

use crate::companion::menu_text;
use crate::fetch::Fetch;
use crate::item::Item;
use crate::paths::write_atomic;

/// `2026 Smith - Adiabatic transfer`: year, first author's surname, title (LaTeX made plain),
/// without characters Finder or Obsidian dislike. No extension.
pub fn tidy_name(item: &Item) -> String {
    let year = item
        .published
        .map(|d| format!("{} ", d.year()))
        .unwrap_or_default();
    let surname = item
        .authors
        .first()
        .and_then(|a| a.split_whitespace().last())
        .map(|s| format!("{s} - "))
        .unwrap_or_default();
    let title = menu_text(&item.title, 90);
    let raw = format!("{year}{surname}{title}");
    let clean: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let squeezed = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    squeezed
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_owned()
}

/// arXiv's PDF address for a paper (`None` for news).
pub fn url(item: &Item) -> Option<String> {
    if let Some(p) = &item.pdf {
        return Some(p.clone());
    }
    crate::cite::arxiv_id(item).map(|id| format!("https://arxiv.org/pdf/{id}"))
}

/// The PDF in `dir`, downloading it first if it isn't there yet. Returns its path.
pub fn fetch_to(item: &Item, dir: &Path, fetch: &dyn Fetch) -> Result<PathBuf, String> {
    let path = dir.join(format!("{}.pdf", tidy_name(item)));
    if path.is_file() {
        return Ok(path);
    }
    let url = url(item).ok_or("this item has no PDF")?;
    let bytes = fetch.get_patient(&url)?;
    if !bytes.starts_with(b"%PDF") {
        return Err("arXiv didn't send a PDF (it may still be processing); try again later".into());
    }
    write_atomic(&path, &bytes)?;
    Ok(path)
}

/// PDF apps Settings offers when they're installed, in this order. The name is
/// both the label and what `open -a` gets.
pub const KNOWN_APPS: &[&str] = &[
    "Preview",
    "Skim",
    "PDF Expert",
    "PDFgear",
    "LibreOffice",
    "OpenOffice",
    "ONLYOFFICE",
];

/// Where Mac apps live: `/Applications`, `/System/Applications` (Preview) and `~/Applications`.
pub fn app_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        home.join("Applications"),
    ]
}

/// The known PDF apps that are installed in one of `dirs` (a folder `<name>.app`).
pub fn installed_apps(dirs: &[PathBuf]) -> Vec<String> {
    KNOWN_APPS
        .iter()
        .filter(|name| dirs.iter().any(|d| d.join(format!("{name}.app")).is_dir()))
        .map(|name| (*name).to_owned())
        .collect()
}

/// What Settings shows for a chosen app: a name as it is, an app picked with "Other app…"
/// (a path to `Something.app`) by its name.
pub fn app_label(app: &str) -> String {
    let p = Path::new(app);
    match (p.is_absolute(), p.file_stem()) {
        (true, Some(stem)) => stem.to_string_lossy().into_owned(),
        _ => app.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::Section;

    struct Fake(Vec<u8>);
    impl Fetch for Fake {
        fn get(&self, _url: &str) -> Result<Vec<u8>, String> {
            Ok(self.0.clone())
        }
    }

    fn paper() -> Item {
        Item {
            key: "arxiv:2610.00001".into(),
            section: Section::Research,
            source: "arxiv".into(),
            source_name: "arXiv".into(),
            title: "Adiabatic transfer: $\\mathcal{O}(1)$ overhead / why?".into(),
            authors: vec!["Ada Smith".into()],
            summary: String::new(),
            link: "https://arxiv.org/abs/2610.00001".into(),
            pdf: None,
            categories: vec![],
            published: "2026-10-27T04:00:00Z".parse().ok(),
            announce_type: None,
            version: None,
            doi: None,
            journal_ref: None,
        }
    }

    #[test]
    fn tidy_names_and_one_download() {
        let p = paper();
        assert_eq!(
            tidy_name(&p),
            "2026 Smith - Adiabatic transfer O(1) overhead why"
        );
        assert_eq!(url(&p).unwrap(), "https://arxiv.org/pdf/2610.00001");
        let dir = tempfile::tempdir().unwrap();
        assert!(fetch_to(&p, dir.path(), &Fake(b"<html>".to_vec())).is_err());
        let path = fetch_to(&p, dir.path(), &Fake(b"%PDF-1.5 x".to_vec())).unwrap();
        assert!(path.ends_with("2026 Smith - Adiabatic transfer O(1) overhead why.pdf"));
        // Already there: reused (a fetch that fails proves no request was made).
        struct Fails;
        impl Fetch for Fails {
            fn get(&self, _url: &str) -> Result<Vec<u8>, String> {
                Err("no network".into())
            }
        }
        assert_eq!(fetch_to(&p, dir.path(), &Fails).unwrap(), path);
    }

    #[test]
    fn only_installed_apps_are_offered() {
        let system = tempfile::tempdir().unwrap();
        let mine = tempfile::tempdir().unwrap();
        std::fs::create_dir(system.path().join("Preview.app")).unwrap();
        std::fs::create_dir(mine.path().join("PDFgear.app")).unwrap();
        // A file of that name isn't an app bundle.
        std::fs::write(mine.path().join("Skim.app"), "").unwrap();
        let dirs = [mine.path().to_path_buf(), system.path().to_path_buf()];
        // Known order, not folder order.
        assert_eq!(installed_apps(&dirs), ["Preview", "PDFgear"]);
        assert_eq!(
            app_label("/Applications/Foxit PDF Reader.app"),
            "Foxit PDF Reader"
        );
        assert_eq!(app_label("Skim"), "Skim");
    }
}
