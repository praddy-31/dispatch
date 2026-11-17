//! Colours. Dispatch follows Palette's current theme by reading `dispatch.json`, the file
//! Palette writes for it (`~/.config/palette/current/`). An older Palette without that target
//! only writes `habit-tracker.json`, which has the same keys, so that is the fallback.
//!
//! `dispatch.json` may also carry the window's glass (`palette glass dispatch`):
//! `"glass": {"kind": "blurred", "opacity": 0.80, "blur": 20}`. Kind `own` (or no block)
//! leaves the look to Dispatch's Settings.
//!
//! Checking for a change is one `stat` of the file every couple of seconds; the file is only
//! read when its modification time changes. Palette swaps its `current` folder atomically
//! (a symlink), so the path always points at a complete file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;

pub const FILES: [&str; 2] = ["dispatch.json", "habit-tracker.json"];

/// The colour names the UI uses (Palette's names). The UI turns each into a CSS variable
/// (`--background`, …).
pub const KEYS: [&str; 17] = [
    "background",
    "dark_background",
    "lighter_background",
    "selection",
    "muted",
    "foreground",
    "dark_foreground",
    "light_foreground",
    "bright_foreground",
    "accent",
    "red",
    "orange",
    "yellow",
    "green",
    "cyan",
    "blue",
    "magenta",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Theme {
    /// "Palette: tokyo-night" or "Built in: Tokyo Night".
    pub source: String,
    pub mode: String,
    pub colours: BTreeMap<String, String>,
    /// The file it came from (`dispatch.json`, or `habit-tracker.json` as a fallback); empty
    /// for the built-in theme. Shown in Settings → Look.
    pub file: String,
    /// The window glass chosen in Palette, if any (`None` = Dispatch's Settings decide).
    pub glass: Option<Glass>,
}

/// Palette's glass for Dispatch's window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Glass {
    pub kind: GlassKind,
    /// How much of the desktop shows through, 0–100 % (Palette's "strength", as set there).
    pub transparency: u32,
    /// Blur, 0–100 (only for `Blurred`).
    pub blur: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GlassKind {
    Solid,
    Blurred,
    Clear,
}

/// Reads the `glass` block; `own`, a missing block or an unknown kind give `None`.
fn glass_from(v: Option<&serde_json::Value>) -> Option<Glass> {
    let g = v?.as_object()?;
    let kind = match g.get("kind")?.as_str()? {
        "solid" => GlassKind::Solid,
        "blurred" => GlassKind::Blurred,
        "clear" => GlassKind::Clear,
        _ => return None,
    };
    let opacity = g
        .get("opacity")
        .and_then(|o| o.as_f64().or_else(|| o.as_str()?.parse().ok()))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let transparency = if kind == GlassKind::Solid {
        0
    } else {
        ((1.0 - opacity) * 100.0).round() as u32
    };
    let blur = if kind == GlassKind::Blurred {
        g.get("blur").and_then(|b| b.as_u64()).unwrap_or(0).min(100) as u32
    } else {
        0
    };
    Some(Glass {
        kind,
        transparency,
        blur,
    })
}

/// Tokyo Night (the old home page's default), used when Palette isn't installed.
pub fn builtin() -> Theme {
    let pairs = [
        ("background", "#1a1b26"),
        ("dark_background", "#16161e"),
        ("lighter_background", "#24283b"),
        ("selection", "#283457"),
        ("muted", "#565f89"),
        ("foreground", "#c0caf5"),
        ("dark_foreground", "#a9b1d6"),
        ("light_foreground", "#cfd6f7"),
        ("bright_foreground", "#e0e5fb"),
        ("accent", "#7aa2f7"),
        ("red", "#f7768e"),
        ("orange", "#ff9e64"),
        ("yellow", "#e0af68"),
        ("green", "#9ece6a"),
        ("cyan", "#7dcfff"),
        ("blue", "#7aa2f7"),
        ("magenta", "#bb9af7"),
    ];
    Theme {
        source: "Built in: Tokyo Night".into(),
        mode: "dark".into(),
        colours: pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        file: String::new(),
        glass: None,
    }
}

/// Parses a Palette colour file. Only `#rrggbb` values are taken; a file without a valid
/// background and foreground is ignored. Missing names fall back to the built-in theme's.
pub fn from_palette_json(text: &str) -> Option<Theme> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = v.as_object()?;
    let mut colours = builtin().colours;
    let mut found = 0;
    for key in KEYS {
        if let Some(c) = obj.get(key).and_then(|c| c.as_str()).filter(|c| is_hex(c)) {
            colours.insert(key.to_owned(), c.to_lowercase());
            found += 1;
        }
    }
    let has = |k: &str| obj.get(k).and_then(|c| c.as_str()).is_some_and(is_hex);
    if found == 0 || !has("background") || !has("foreground") {
        return None;
    }
    let name = obj
        .get("theme")
        .and_then(|t| t.as_str())
        .unwrap_or("current");
    let mode = obj
        .get("mode")
        .and_then(|m| m.as_str())
        .filter(|m| *m == "light" || *m == "dark")
        .unwrap_or("dark");
    Some(Theme {
        source: format!("Palette: {name}"),
        mode: mode.to_owned(),
        colours,
        file: String::new(),
        glass: glass_from(obj.get("glass")),
    })
}

impl Theme {
    /// The background colour as RGB, for a solid window (`glass.rs`).
    pub fn background_rgb(&self) -> (u8, u8, u8) {
        self.colours
            .get("background")
            .and_then(|h| rgb(h))
            .unwrap_or((0x1a, 0x1b, 0x26))
    }
}

/// `#rrggbb` → (r, g, b).
pub fn rgb(hex: &str) -> Option<(u8, u8, u8)> {
    if !is_hex(hex) {
        return None;
    }
    let n = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((n(1)?, n(3)?, n(5)?))
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Remembers which file was read and when it changed, so `poll` is cheap.
pub struct Watch {
    dir: PathBuf,
    seen: Option<(PathBuf, Option<SystemTime>)>,
    current: Theme,
}

impl Watch {
    pub fn new(dir: &Path) -> Watch {
        let mut w = Watch {
            dir: dir.to_owned(),
            seen: None,
            current: builtin(),
        };
        w.poll();
        w
    }

    pub fn current(&self) -> &Theme {
        &self.current
    }

    /// Returns the new theme if it changed since the last call.
    pub fn poll(&mut self) -> Option<Theme> {
        let found = FILES.iter().map(|f| self.dir.join(f)).find_map(|p| {
            let modified = std::fs::metadata(&p).ok()?.modified().ok();
            Some((p, modified))
        });
        if found == self.seen && self.seen.is_some() {
            return None;
        }
        let theme = found
            .as_ref()
            .and_then(|(p, _)| std::fs::read_to_string(p).ok())
            .and_then(|t| from_palette_json(&t))
            .map(|mut t| {
                t.file = found
                    .as_ref()
                    .and_then(|(p, _)| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                t
            })
            .unwrap_or_else(builtin);
        self.seen = found.or(Some((PathBuf::new(), None)));
        if theme == self.current {
            return None;
        }
        self.current = theme.clone();
        Some(theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_palette_colours_and_rejects_bad_values() {
        let t = from_palette_json(
            r##"{"palette":1,"theme":"ash","mode":"dark","background":"#121212",
                "foreground":"#E0E0E0","accent":"red","green":"#767676"}"##,
        )
        .unwrap();
        assert_eq!(t.source, "Palette: ash");
        assert_eq!(t.colours["background"], "#121212");
        assert_eq!(t.colours["foreground"], "#e0e0e0");
        assert_eq!(t.colours["green"], "#767676");
        assert_eq!(
            t.colours["accent"],
            builtin().colours["accent"],
            "not #rrggbb"
        );
        assert!(from_palette_json(r##"{"background":"#121212"}"##).is_none());
        assert!(from_palette_json("not json").is_none());
    }

    #[test]
    fn watch_notices_a_theme_switch_and_a_removed_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = Watch::new(dir.path());
        assert_eq!(w.current().source, builtin().source);
        assert!(w.poll().is_none(), "nothing changed");

        let file = dir.path().join("habit-tracker.json");
        std::fs::write(
            &file,
            r##"{"theme":"a","background":"#000000","foreground":"#ffffff"}"##,
        )
        .unwrap();
        assert_eq!(w.poll().unwrap().source, "Palette: a");
        assert!(w.poll().is_none());

        assert_eq!(w.current().file, "habit-tracker.json");

        // Palette's own file for Dispatch wins over the Habit Tracker's.
        std::fs::write(
            dir.path().join("dispatch.json"),
            r##"{"theme":"b","background":"#000000","foreground":"#ffffff"}"##,
        )
        .unwrap();
        let t = w.poll().unwrap();
        assert_eq!(
            (t.source.as_str(), t.file.as_str()),
            ("Palette: b", "dispatch.json")
        );

        std::fs::remove_file(dir.path().join("dispatch.json")).unwrap();
        std::fs::remove_file(&file).unwrap();
        assert_eq!(w.poll().unwrap().source, builtin().source);
    }

    #[test]
    fn background_colour_as_rgb() {
        assert_eq!(rgb("#1a1b26"), Some((0x1a, 0x1b, 0x26)));
        assert_eq!(rgb("1a1b26"), None);
        assert_eq!(
            builtin().background_rgb(),
            rgb(&builtin().colours["background"]).unwrap()
        );
    }

    #[test]
    fn reads_palettes_glass_for_the_window() {
        let base = r##"{"background":"#000000","foreground":"#ffffff""##;
        let with = |glass: &str| from_palette_json(&format!("{base},\"glass\":{glass}}}")).unwrap();
        assert_eq!(from_palette_json(&format!("{base}}}")).unwrap().glass, None);
        assert_eq!(
            with(r#"{"kind":"own","opacity":0.5,"blur":50}"#).glass,
            None
        );
        assert_eq!(
            with(r#"{"kind":"blurred","opacity":0.80,"blur":30}"#).glass,
            Some(Glass {
                kind: GlassKind::Blurred,
                transparency: 20,
                blur: 30
            })
        );
        // What Palette asks for is used as it is (no cap). Clear has no blur.
        assert_eq!(
            with(r#"{"kind":"clear","opacity":0.1,"blur":30}"#).glass,
            Some(Glass {
                kind: GlassKind::Clear,
                transparency: 90,
                blur: 0
            })
        );
        assert_eq!(
            with(r#"{"kind":"solid","opacity":0.5,"blur":30}"#).glass,
            Some(Glass {
                kind: GlassKind::Solid,
                transparency: 0,
                blur: 0
            })
        );
    }
}
