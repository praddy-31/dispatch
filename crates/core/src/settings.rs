//! App settings: `settings.json` in the data folder, next to `topics.json`. Separate from
//! topics because they are preferences for this Mac, not what the user follows.
//!
//! Limits: news at most every hour, arXiv at most once a day, so the app
//! stays light on the machine and polite to the feeds.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths::write_atomic;

pub const FILE: &str = "settings.json";
/// The live `.bib`'s default name in the documents folder.
pub const BIB_NAME: &str = "dispatch.bib";
/// ⌃⌥S opens Search (Control+Option, clear of AeroSpace's Option keys;
/// Palette has ⌥⇧T, the Habit Tracker ⌃⌥Space).
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+s";

/// Highest weekly goal kept (Settings offers up to 50; a hand-edited file may say more).
pub const MAX_WEEKLY_GOAL: u32 = 500;

/// Most of the desktop that may show through the window: 100, like Palette's slider (no
/// hidden cap).
pub const MAX_TRANSPARENCY: u32 = 100;

/// The frosted material behind a transparent window, named after Apple's
/// `NSVisualEffectView` materials. The app maps each to Tauri's window effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frost {
    /// The window background material (the original look): light blur, close to solid.
    #[default]
    Window,
    /// The sidebar material: more of the desktop's colour comes through.
    Sidebar,
    /// The pop-over material: a brighter, milkier blur.
    Popover,
    /// The HUD material: the heaviest, darkest blur.
    Hud,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    /// News refresh interval, 1–24 hours.
    pub news_every_hours: u32,
    /// arXiv refresh interval, 1–7 days (always at the 05:30 UTC slot).
    pub arxiv_every_days: u32,
    /// Show the week's digest automatically on days arXiv announced nothing.
    pub digest_auto: bool,
    /// The login item exists (kept in sync with the LaunchAgent by the app).
    pub launch_at_login: bool,
    /// Sync topics, saved items, notes, tags and read state through iCloud Drive.
    pub icloud_sync: bool,
    /// How much of the desktop shows through the window, 0–100 % (0 = solid).
    pub transparency: u32,
    /// Which macOS frosted material sits behind the window. Apps can't set a blur
    /// radius; each material has its own blur and tint.
    pub frost: Frost,
    /// Blur radius behind the window, 0–100.
    /// 0 = use the Frost material; above 0 replaces it, through a private macOS call.
    pub blur: u32,
    /// The window turns solid (no glass, no blur) while another app is in front, so
    /// a Dispatch window left open in a tile costs no GPU for its glass. Off = always glass.
    pub solid_in_background: bool,
    /// Grey out unread items that arrived before today (leftovers).
    pub dim_earlier: bool,
    /// Unread count beside the menu-bar icon: off, papers, or papers and news;
    /// Or a dot on the icon.
    pub menu_bar_count: crate::companion::IconCount,
    /// A red dot on the icon while a source's last fetch failed.
    pub menu_bar_alert: bool,
    /// The global shortcut that opens Search from anywhere ("ctrl+alt+s", the
    /// plugin's names joined by `+`); empty = none.
    pub hotkey: String,
    /// Notifications held back until then (the menu's Pause). macOS Focus is
    /// respected by macOS itself.
    pub notify_paused_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Days a paper stays on the Read later list (1 = until midnight, as
    /// before); 0 = until you open it or take it off.
    pub later_days: u32,
    /// Write `status.json` for SketchyBar (Palette's Dispatch item) and others.
    pub status_file: bool,
    /// What the SketchyBar pill shows, in order (`status::METRICS` ids).
    pub bar_metrics: Vec<String>,
    /// Notifications, one summary per refresh: a followed author's new paper...
    pub notify_authors: bool,
    /// ...a watched keyword in a new paper...
    pub notify_keywords: bool,
    /// ...a saved paper's new version, DOI or journal.
    pub notify_updates: bool,
    /// Research and News hide read papers older than this many days, unless saved; 0 = off.
    pub hide_read_after_days: u32,
    /// Archive limit: remove items older than this many months that aren't saved, noted or
    /// tagged; 0 = keep everything (default).
    pub archive_months: u32,
    /// Papers to read each week (Stats), Monday to Sunday; 0 = no goal.
    pub weekly_goal: u32,
    /// Write `reading-log.json` (papers read per day) for the Habit Tracker to pick up.
    pub habit_export: bool,
    /// "Dispatch documents", one folder for the live `.bib` and downloaded PDFs;
    /// empty = not chosen (no `.bib`, PDFs go to ~/Downloads).
    pub docs_dir: String,
    /// Keep the live `.bib` in `docs_dir`.
    pub bib_on: bool,
    /// Its file name (an older Dispatch's chosen name is kept), default `dispatch.bib`.
    pub bib_name: String,
    /// From older versions: the `.bib` path. Read once and folded into `docs_dir`; never written.
    #[serde(skip_serializing)]
    pub bib_file: String,
    /// Only saved papers with this tag go in the `.bib`; empty = every saved paper.
    pub bib_tag: String,
    /// Citation keys in the `.bib`, `\cite{}` and `\bibitem` copies.
    pub cite_keys: crate::bibfile::KeyStyle,
    /// Folder for one Markdown note per saved paper (an Obsidian vault folder); empty = off.
    pub notes_dir: String,
    /// App that opens PDFs: empty = the browser (as before), else an app name `open -a` knows
    /// (`pdf::KNOWN_APPS`) or the path of one picked with "Other app…".
    pub pdf_app: String,
    /// From older versions: the PDF folder. Read once and folded into `docs_dir`; never written.
    #[serde(skip_serializing)]
    pub pdf_dir: String,
    /// Semantic Scholar connected (its API key is in the Keychain, never here).
    pub s2_connected: bool,
    /// Semantic Scholar in the preview, with or without a key (off by default).
    pub s2_on: bool,
    /// Zotero web API: the library's user ID, 0 = not set up. The API key itself is
    /// in the Keychain, never here.
    pub zotero_user: u64,
    /// The zotero.org user name, shown in Settings.
    pub zotero_name: String,
    /// Collection key papers go into through the web API; empty = the library's root.
    pub zotero_collection: String,
    pub zotero_collection_name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 1,
            news_every_hours: 3,
            arxiv_every_days: 1,
            digest_auto: true,
            launch_at_login: false,
            icloud_sync: false,
            transparency: 0,
            frost: Frost::default(),
            blur: 0,
            solid_in_background: false,
            dim_earlier: true,
            menu_bar_count: crate::companion::IconCount::Off,
            menu_bar_alert: true,
            hotkey: DEFAULT_HOTKEY.into(),
            notify_paused_until: None,
            later_days: 7,
            status_file: false,
            bar_metrics: vec!["read_today".into()],
            notify_authors: false,
            notify_keywords: false,
            notify_updates: false,
            hide_read_after_days: 0,
            archive_months: 0,
            weekly_goal: 0,
            habit_export: false,
            bib_file: String::new(),
            bib_tag: String::new(),
            cite_keys: crate::bibfile::KeyStyle::Author,
            notes_dir: String::new(),
            pdf_app: String::new(),
            docs_dir: String::new(),
            bib_on: false,
            bib_name: BIB_NAME.into(),
            s2_connected: false,
            s2_on: false,
            pdf_dir: String::new(),
            zotero_user: 0,
            zotero_name: String::new(),
            zotero_collection: String::new(),
            zotero_collection_name: String::new(),
        }
    }
}

impl Settings {
    /// Notifications are paused (the menu's Pause) at `now`.
    pub fn notifications_paused(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.notify_paused_until.is_some_and(|t| t > now)
    }

    /// Reads `settings.json`; a missing file gives the defaults. A broken file also gives the
    /// defaults (settings are easy to set again, unlike topics), with the reason.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Settings>(&text) {
                Ok(s) => (s.migrated().clamped(), None),
                Err(e) => (
                    Settings::default(),
                    Some(format!(
                        "{} is not valid ({e}); using defaults",
                        path.display()
                    )),
                ),
            },
            Err(_) => (Settings::default(), None),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut json =
            serde_json::to_string_pretty(&self.clone().clamped()).map_err(|e| e.to_string())?;
        json.push('\n');
        write_atomic(path, json.as_bytes())
    }

    /// An older `bib_file` / `pdf_dir` becomes the documents folder (the `.bib`'s
    /// folder wins when both were set; PDFs already in the old PDF folder stay there).
    pub fn migrated(mut self) -> Settings {
        if self.docs_dir.is_empty() {
            let bib = Path::new(&self.bib_file);
            if let (false, Some(dir), Some(name)) =
                (self.bib_file.is_empty(), bib.parent(), bib.file_name())
            {
                self.docs_dir = dir.display().to_string();
                self.bib_name = name.to_string_lossy().into_owned();
                self.bib_on = true;
            }
            if self.docs_dir.is_empty() {
                self.docs_dir = self.pdf_dir.clone();
            }
        }
        self.bib_file.clear();
        self.pdf_dir.clear();
        self
    }

    /// The live `.bib`, when it's switched on and there's a documents folder.
    pub fn bib_path(&self) -> Option<PathBuf> {
        (self.bib_on && !self.docs_dir.is_empty())
            .then(|| Path::new(&self.docs_dir).join(&self.bib_name))
    }

    /// Where PDFs are downloaded: the documents folder, else `~/Downloads` under `home`.
    pub fn pdf_folder(&self, home: &Path) -> PathBuf {
        if self.docs_dir.is_empty() {
            home.join("Downloads")
        } else {
            PathBuf::from(&self.docs_dir)
        }
    }

    /// Keeps every value inside its limits.
    pub fn clamped(mut self) -> Settings {
        self.news_every_hours = self.news_every_hours.clamp(1, 24);
        self.arxiv_every_days = self.arxiv_every_days.clamp(1, 7);
        self.transparency = self.transparency.min(MAX_TRANSPARENCY);
        self.blur = self.blur.min(100);
        self.hide_read_after_days = self.hide_read_after_days.min(365);
        // The archive limit never goes below 3 months, so a slip can't empty the archive.
        if self.archive_months != 0 {
            self.archive_months = self.archive_months.clamp(3, 120);
        }
        self.weekly_goal = self.weekly_goal.min(MAX_WEEKLY_GOAL);
        self.later_days = self.later_days.min(365);
        let mut metrics = Vec::new();
        for m in &self.bar_metrics {
            if crate::status::METRICS.iter().any(|(id, _)| id == m) && !metrics.contains(m) {
                metrics.push(m.clone());
            }
        }
        self.bar_metrics = metrics;
        // A hand-edited shortcut that can't be used falls back to the default.
        self.hotkey = normalise_hotkey(&self.hotkey).unwrap_or_else(|_| DEFAULT_HOTKEY.into());
        // A name, not a path: the `.bib` always lives in the documents folder.
        let name = Path::new(self.bib_name.trim())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.bib_name = if name.is_empty() {
            BIB_NAME.into()
        } else {
            name
        };
        self
    }

    /// The window's look: Palette's glass when it chose one (`palette glass dispatch`),
    /// otherwise these settings.
    pub fn look(&self, glass: Option<&crate::theme::Glass>) -> Look {
        use crate::theme::GlassKind;
        let mut look = match glass {
            None => Look {
                transparency: self.transparency.min(MAX_TRANSPARENCY),
                blur: self.blur.min(100),
                material: Some(self.frost),
                by_palette: false,
                kind: None,
                paused: false,
            },
            Some(g) => Look {
                transparency: g.transparency,
                blur: g.blur,
                // Clear: see-through with no blur at all, so no material either.
                material: (g.kind != GlassKind::Clear).then_some(self.frost),
                by_palette: true,
                kind: Some(g.kind),
                paused: false,
            },
        };
        look.material = look.material.filter(|_| look.transparency > 0);
        look
    }

    /// The look while another app is in front: solid if `solid_in_background` is on.
    pub fn look_unfocused(&self, glass: Option<&crate::theme::Glass>) -> Look {
        let look = self.look(glass);
        if !self.solid_in_background || look.transparency == 0 {
            return look;
        }
        Look {
            transparency: 0,
            blur: 0,
            material: None,
            paused: true,
            ..look
        }
    }
}

/// What the window looks like right now (settings or Palette).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Look {
    /// 0–100 % of the desktop showing through.
    pub transparency: u32,
    /// 0 = the material's own blur; above 0 = this radius instead (private macOS call).
    pub blur: u32,
    /// The frosted material when the radius isn't used; `None` = none (Palette's "clear").
    pub material: Option<Frost>,
    /// Palette chose it; Settings shows its controls greyed out.
    pub by_palette: bool,
    /// Palette's kind (solid, blurred, clear) when it chose one.
    pub kind: Option<crate::theme::GlassKind>,
    /// Solid for now because another app is in front (`solid_in_background`).
    pub paused: bool,
}

// ---- the global shortcut -------------------

const MODIFIERS: [(&str, &[&str]); 4] = [
    ("ctrl", &["ctrl", "control"]),
    ("alt", &["alt", "option", "opt"]),
    ("shift", &["shift"]),
    ("cmd", &["cmd", "command", "super", "meta"]),
];

/// `Control-Option-S`, `ctrl+alt+s`, `alt + ctrl + S` → `ctrl+alt+s`. At least one modifier
/// and exactly one key: a letter, digit, F1–F20, `space`, or an arrow. Empty stays empty (off).
pub fn normalise_hotkey(input: &str) -> Result<String, String> {
    if input.trim().is_empty() {
        return Ok(String::new());
    }
    let parts: Vec<String> = input
        .split(['+', '-'])
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    let mut mods = Vec::new();
    let mut key = None;
    for part in &parts {
        match MODIFIERS
            .iter()
            .find(|(_, names)| names.contains(&part.as_str()))
        {
            Some((name, _)) if !mods.contains(name) => mods.push(*name),
            Some(_) => return Err(format!("{input}: {part} appears twice")),
            None if key.is_none() => key = Some(part.clone()),
            None => return Err(format!("{input}: only one key besides the modifiers")),
        }
    }
    let Some(key) = key else {
        return Err(format!("{input}: needs a key, e.g. ctrl+alt+s"));
    };
    let valid_key = (key.len() == 1 && key.chars().all(|c| c.is_ascii_alphanumeric()))
        || ["space", "up", "down", "left", "right", "tab", "enter"].contains(&key.as_str())
        || key
            .strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=20).contains(&n));
    if !valid_key {
        return Err(format!(
            "{input}: {key} can't be used (letters, digits, F1–F20, space, arrows)"
        ));
    }
    if mods.is_empty() {
        return Err(format!(
            "{input}: needs at least one of Control, Option, Shift, Command"
        ));
    }
    mods.sort_by_key(|m| MODIFIERS.iter().position(|(n, _)| n == m));
    Ok(format!("{}+{key}", mods.join("+")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_like_palettes() {
        assert_eq!(normalise_hotkey("Control-Option-S").unwrap(), "ctrl+alt+s");
        assert_eq!(normalise_hotkey("alt + ctrl + S").unwrap(), "ctrl+alt+s");
        assert_eq!(
            normalise_hotkey("cmd+ctrl+space").unwrap(),
            "ctrl+cmd+space"
        );
        assert_eq!(normalise_hotkey("").unwrap(), "");
        assert!(normalise_hotkey("s").is_err());
        assert!(normalise_hotkey("ctrl+alt").is_err());
        assert!(normalise_hotkey("ctrl+a+b").is_err());
        assert!(normalise_hotkey("alt+é").is_err());
        assert_eq!(Settings::default().hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn old_bib_and_pdf_paths_become_the_documents_folder() {
        let old =
            r#"{"bib_file": "/Volumes/Work/Papers/refs.bib", "pdf_dir": "/Volumes/Work/PDFs"}"#;
        let s = serde_json::from_str::<Settings>(old)
            .unwrap()
            .migrated()
            .clamped();
        assert_eq!(s.docs_dir, "/Volumes/Work/Papers");
        assert_eq!(s.bib_name, "refs.bib");
        assert!(s.bib_on);
        assert_eq!(
            s.bib_path(),
            Some(PathBuf::from("/Volumes/Work/Papers/refs.bib"))
        );
        // Old fields are never written back.
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains("bib_file") && !json.contains("pdf_dir"),
            "{json}"
        );
        // Only a PDF folder: that's the documents folder, no .bib.
        let s = serde_json::from_str::<Settings>(r#"{"pdf_dir": "/x/PDFs"}"#)
            .unwrap()
            .migrated();
        assert_eq!((s.docs_dir.as_str(), s.bib_path()), ("/x/PDFs", None));
        assert_eq!(s.pdf_folder(Path::new("/h")), PathBuf::from("/x/PDFs"));
        // Nothing set: Downloads, default name; a path typed as the name is cut to a name.
        let mut s = Settings::default().migrated();
        assert_eq!(s.pdf_folder(Path::new("/h")), PathBuf::from("/h/Downloads"));
        s.bib_name = "../../etc/x.bib".into();
        assert_eq!(s.clamped().bib_name, "x.bib");
    }

    #[test]
    fn limits_hold_and_missing_fields_get_defaults() {
        let s: Settings =
            serde_json::from_str(r#"{"news_every_hours": 0, "arxiv_every_days": 30}"#).unwrap();
        let s = s.clamped();
        assert_eq!((s.news_every_hours, s.arxiv_every_days), (1, 7));
        assert!(s.digest_auto, "default kept for a missing field");
    }

    #[test]
    fn weekly_goal_and_habit_export_default_off_and_are_clamped() {
        let s = Settings::default();
        assert_eq!((s.weekly_goal, s.habit_export), (0, false));
        let s: Settings =
            serde_json::from_str(r#"{"weekly_goal": 100000, "habit_export": true}"#).unwrap();
        let s = s.clamped();
        assert_eq!((s.weekly_goal, s.habit_export), (MAX_WEEKLY_GOAL, true));
    }

    #[test]
    fn frost_defaults_to_the_old_material_and_round_trips() {
        let s: Settings = serde_json::from_str(r#"{"transparency": 20}"#).unwrap();
        assert_eq!(
            s.frost,
            Frost::Window,
            "older settings files keep their look"
        );
        let s: Settings = serde_json::from_str(r#"{"frost": "hud"}"#).unwrap();
        assert_eq!(s.frost, Frost::Hud);
    }

    #[test]
    fn palettes_glass_wins_over_settings_unless_own() {
        use crate::theme::{Glass, GlassKind};
        let s = Settings {
            transparency: 30,
            blur: 40,
            frost: Frost::Hud,
            ..Settings::default()
        };
        let own = s.look(None);
        assert_eq!(
            (own.transparency, own.blur, own.material, own.by_palette),
            (30, 40, Some(Frost::Hud), false)
        );
        let clear = s.look(Some(&Glass {
            kind: GlassKind::Clear,
            transparency: 50,
            blur: 0,
        }));
        assert_eq!(
            (
                clear.transparency,
                clear.blur,
                clear.material,
                clear.by_palette
            ),
            (50, 0, None, true)
        );
        let solid = s.look(Some(&Glass {
            kind: GlassKind::Solid,
            transparency: 0,
            blur: 0,
        }));
        assert_eq!(
            (solid.transparency, solid.material),
            (0, None),
            "no material behind solid"
        );
        // Solid in the background only when switched on.
        assert_eq!(s.look_unfocused(None), own);
        let s = Settings {
            solid_in_background: true,
            ..s
        };
        let away = s.look_unfocused(None);
        assert_eq!(
            (away.transparency, away.blur, away.material, away.paused),
            (0, 0, None, true)
        );
    }

    #[test]
    fn a_broken_file_gives_defaults_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(Settings::load(&path), (Settings::default(), None));
        std::fs::write(&path, "{ nope").unwrap();
        let (s, why) = Settings::load(&path);
        assert_eq!(s, Settings::default());
        assert!(why.unwrap().contains("not valid"));
        let s = Settings {
            news_every_hours: 6,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).0.news_every_hours, 6);
    }
}
