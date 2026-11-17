//! Menu-bar companion: what the menu-bar icon's menu shows, computed here so it
//! can be tested without a window. The app turns a `Companion` into a native menu.
//!
//! "Top 5": unread new submissions, best "For you" score first (`rank`), taken in
//! turn from each topic in the user's topic order, so the first topic doesn't fill all five.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::item::Section;
use crate::settings::Settings;
use crate::store::{SourceStatus, Store};
use crate::topics::Topics;

/// How many papers the menu offers.
pub const TOP: usize = 5;
/// Longest title in the menu, in characters (macOS menus don't wrap).
pub const TITLE_CHARS: usize = 64;

/// What shows beside the menu-bar icon (Settings; off by default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconCount {
    #[default]
    Off,
    /// Unread papers only.
    Research,
    /// Unread papers and news together.
    All,
    /// No number; a dot while there are new papers since the window was last open.
    Dot,
    /// Suggested papers not yet opened.
    Suggested,
    /// Today's new papers by a followed author or with a watched keyword, not yet opened.
    ForYou,
    /// Papers read this week out of the weekly goal ("4/10"); nothing without a goal.
    Goal,
    /// The "Read later" list, papers and news.
    Later,
}

/// When the window was last open (`meta`), for the dot: papers first seen after it are new.
pub const WINDOW_SEEN: &str = "window_seen";

/// What the icon itself shows (the app draws it, `tray.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Badge {
    /// The plain icon.
    None,
    /// A small dot: something unread.
    Dot,
    /// A red dot: a source failed at its last fetch.
    Alert,
}

/// A paper in the menu's short lists (last opened, recently saved).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MenuPaper {
    pub key: String,
    pub menu_title: String,
}

/// How many recently saved papers the menu lists.
pub const RECENT_SAVED: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TopPaper {
    pub key: String,
    /// Plain text for a native menu (LaTeX turned into readable characters, shortened).
    pub menu_title: String,
    /// "Okafor, Weißler +5"
    pub authors: String,
    pub link: String,
    pub pdf: Option<String>,
    pub saved: bool,
    /// Already shown in the preview (no "Mark read" in its submenu).
    pub read: bool,
    /// Written by a followed author (shown with 👤).
    pub followed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Companion {
    pub research_unread: i64,
    pub news_unread: i64,
    pub top: Vec<TopPaper>,
    /// The top papers are all from arXiv's latest announcement ("Today's papers").
    pub top_is_latest: bool,
    pub sources: Vec<SourceStatus>,
    pub failed: usize,
    pub last_ok: Option<DateTime<Utc>>,
    pub research_next: DateTime<Utc>,
    pub news_next: DateTime<Utc>,
    /// Weekly goal (Stats): papers to read this week, 0 = none set (the menu then says nothing).
    pub weekly_goal: u32,
    /// Papers read since Monday, local time.
    pub week_read: i64,
    /// Coming deadlines from `deadlines.json`; filled in by the app, which knows
    /// the data folder.
    pub deadlines: Vec<crate::deadlines::Upcoming>,
    /// arXiv announces nothing for this slot (Saturday and Sunday, UTC).
    pub arxiv_weekend: bool,
    /// The paper whose page or PDF was opened last ("Copy BibTeX of …").
    pub last_opened: Option<MenuPaper>,
    /// The newest saved papers.
    pub recent_saved: Vec<MenuPaper>,
    /// Icon choices: Suggested papers not opened...
    pub suggested_left: usize,
    /// ...today's for-you hits (followed author or watched keyword), not opened...
    pub for_you_today: usize,
    /// ...the Read later list (kept `Settings::later_days`)...
    pub later_today: usize,
    /// ...and papers first seen since the window was last open (the dot).
    pub new_since_seen: usize,
}

/// The oldest "Read later" mark still on the list: `days` local days back from today's
/// midnight (1 = today only); 0 = no limit.
pub fn later_since(days: u32, now: DateTime<Utc>) -> DateTime<Utc> {
    if days == 0 {
        return DateTime::<Utc>::UNIX_EPOCH;
    }
    let day = crate::stats::local(now).date_naive() - chrono::Duration::days(i64::from(days) - 1);
    day.and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now)
}

/// arXiv announces Sunday to Thursday evenings, US Eastern; the daily slot after a Friday or
/// Saturday announcement-free night (Saturday or Sunday, UTC) has no new papers.
pub fn arxiv_weekend(now: DateTime<Utc>) -> bool {
    use chrono::{Datelike, Weekday};
    let slot = crate::schedule::latest_slot(now, crate::schedule::DEFAULT_SLOT_UTC);
    matches!(slot.weekday(), Weekday::Sat | Weekday::Sun)
}

fn menu_paper(r: &crate::store::Row) -> MenuPaper {
    MenuPaper {
        key: r.item.key.clone(),
        menu_title: menu_text(&r.item.title, TITLE_CHARS),
    }
}

impl Companion {
    pub fn build(
        store: &Store,
        topics: &Topics,
        settings: &Settings,
        now: DateTime<Utc>,
    ) -> Result<Companion, String> {
        let research_unread = store
            .unread_counts(Section::Research)?
            .get("")
            .copied()
            .unwrap_or(0);
        let news_unread = store
            .unread_counts(Section::News)?
            .get("")
            .copied()
            .unwrap_or(0);
        let picks = top_papers(store, topics, TOP, now)?;
        let (top, top_is_latest) = (picks.top, picks.latest);
        let later_since = later_since(settings.later_days, now);
        let later_today = store.later(Section::Research, later_since)?.len()
            + store.later(Section::News, later_since)?.len();
        let new_since_seen = match store.time_meta(WINDOW_SEEN) {
            Some(seen) => store.new_since(Section::Research, seen)?.len(),
            None => 0,
        };
        let sources = store.statuses()?;
        let week_read = if settings.weekly_goal > 0 {
            crate::stats::week_progress(store, crate::stats::local(now))?.1
        } else {
            0
        };
        Ok(Companion {
            suggested_left: picks.suggested_left,
            for_you_today: picks.for_you_today,
            later_today,
            new_since_seen,
            arxiv_weekend: arxiv_weekend(now),
            last_opened: store.last_opened()?.as_ref().map(menu_paper),
            recent_saved: store
                .list_saved(None, RECENT_SAVED)?
                .iter()
                .map(menu_paper)
                .collect(),
            weekly_goal: settings.weekly_goal,
            week_read,
            deadlines: vec![],
            research_unread,
            news_unread,
            top,
            top_is_latest,
            failed: sources.iter().filter(|s| s.last_error.is_some()).count(),
            sources,
            last_ok: store.time_meta(crate::refresh::LAST_OK),
            research_next: crate::refresh::next_due(store, settings, Section::Research, now),
            news_next: crate::refresh::next_due(store, settings, Section::News, now),
        })
    }

    /// The icon's own mark: red when a source failed (if `alert`), a dot for unread items
    /// when the count setting is `Dot`, else plain.
    pub fn badge(&self, count: IconCount, alert: bool) -> Badge {
        if alert && self.failed > 0 {
            Badge::Alert
        } else if count == IconCount::Dot && self.new_since_seen > 0 {
            Badge::Dot
        } else {
            Badge::None
        }
    }

    /// The text beside the icon, or None for the plain icon.
    pub fn icon_title(&self, count: IconCount) -> Option<String> {
        let n = match count {
            IconCount::Off | IconCount::Dot => 0,
            IconCount::Research => self.research_unread,
            IconCount::All => self.research_unread + self.news_unread,
            IconCount::Suggested => self.suggested_left as i64,
            IconCount::ForYou => self.for_you_today as i64,
            IconCount::Later => self.later_today as i64,
            IconCount::Goal => {
                return (self.weekly_goal > 0)
                    .then(|| format!("{}/{}", self.week_read, self.weekly_goal));
            }
        };
        (n > 0).then(|| {
            if n > 999 {
                "999+".into()
            } else {
                n.to_string()
            }
        })
    }
}

struct Picks {
    top: Vec<TopPaper>,
    latest: bool,
    suggested_left: usize,
    for_you_today: usize,
}

/// Up to `n` new submissions, the best scored first, one topic at a time in topic order.
/// Drawn from Suggested (the past 7 days, across topics, not opened) when it has
/// any; without a profile, keywords or followed authors, from all recent papers as before.
/// Unread papers come first; Suggested's seen-but-not-opened ones fill the rest, so the menu
/// isn't empty once every paper has been shown in the preview.
fn top_papers(
    store: &Store,
    topics: &Topics,
    n: usize,
    now: DateTime<Utc>,
) -> Result<Picks, String> {
    let week = (now - chrono::Duration::days(7)).timestamp();
    let mut rows = crate::rank::suggested(store, &topics.follow, week, now.timestamp() + 1)?;
    let from_suggested = !rows.is_empty();
    let suggested_left = rows.len();
    // Today = since this morning's arXiv slot (the refresh runs just after it).
    let slot = crate::schedule::latest_slot(now, crate::schedule::DEFAULT_SLOT_UTC);
    let for_you_today = rows
        .iter()
        .filter(|r| r.score.unwrap_or(0.0) >= crate::rank::STRONG && r.first_seen >= slot)
        .count();
    if !from_suggested {
        rows = store.list(Section::Research, None, 400)?;
        let profile = crate::rank::Profile::build(store)?;
        crate::rank::score_rows(store, &topics.follow, &profile, &mut rows)?;
        crate::rank::sort(&mut rows); // stable: newest first among equal scores
    }
    let latest = rows
        .iter()
        .filter_map(|r| r.item.published)
        .max()
        .map(|d| d.date_naive());
    let new_ones = |read: bool| -> Vec<&crate::store::Row> {
        rows.iter()
            .filter(|r| {
                r.read == read && matches!(r.item.announce_type.as_deref(), None | Some("new"))
            })
            .collect()
    };
    let order: Vec<&str> = topics
        .research
        .iter()
        .filter(|t| t.enabled)
        .map(|t| t.id.as_str())
        .collect();
    let mut picked: Vec<&crate::store::Row> = Vec::new();
    let mut pools = vec![new_ones(false)];
    if from_suggested {
        pools.push(new_ones(true));
    }
    for candidates in &pools {
        // Papers by followed authors first, then the rounds below.
        for r in candidates {
            if picked.len() < n && !topics.follow.authors_in(&r.item).is_empty() {
                picked.push(r);
            }
        }
        // Rounds: each topic gives its best paper not yet picked, until n or nothing left.
        loop {
            let before = picked.len();
            for t in &order {
                if picked.len() >= n {
                    break;
                }
                if let Some(r) = candidates.iter().find(|r| {
                    r.topics.iter().any(|x| x == t)
                        && !picked.iter().any(|p| p.item.key == r.item.key)
                }) {
                    picked.push(r);
                }
            }
            if picked.len() >= n || picked.len() == before {
                break;
            }
        }
    }
    // Shown newest first, like the list.
    picked.sort_by_key(|r| std::cmp::Reverse(r.item.published.unwrap_or(r.first_seen)));
    let is_latest = !picked.is_empty()
        && picked
            .iter()
            .all(|r| r.item.published.map(|d| d.date_naive()) == latest);
    let top = picked
        .into_iter()
        .map(|r| TopPaper {
            key: r.item.key.clone(),
            menu_title: menu_text(&r.item.title, TITLE_CHARS),
            authors: short_authors(&r.item.authors),
            link: r.item.link.clone(),
            pdf: r.item.pdf.clone(),
            saved: r.saved,
            read: r.read,
            followed: !topics.follow.authors_in(&r.item).is_empty(),
        })
        .collect();
    Ok(Picks {
        top,
        latest: is_latest,
        suggested_left,
        for_you_today,
    })
}

/// "Okafor, Weißler +5": surnames of the first two, then how many more.
pub fn short_authors(authors: &[String]) -> String {
    let surname = |a: &String| a.split_whitespace().last().unwrap_or(a).to_owned();
    let mut out = authors
        .iter()
        .take(2)
        .map(surname)
        .collect::<Vec<_>>()
        .join(", ");
    if authors.len() > 2 {
        out.push_str(&format!(" +{}", authors.len() - 2));
    }
    out
}

fn symbol(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "hbar" => "ħ",
        "ell" => "ℓ",
        "infty" => "∞",
        "times" => "×",
        "cdot" => "·",
        "pm" => "±",
        "to" | "rightarrow" => "→",
        "leftrightarrow" => "↔",
        "sim" => "~",
        "approx" => "≈",
        "neq" | "ne" => "≠",
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "partial" => "∂",
        "nabla" => "∇",
        "langle" => "⟨",
        "rangle" => "⟩",
        "dagger" => "†",
        "circ" => "°",
        "AA" => "Å",
        _ => return None,
    })
}

fn script(c: char, sup: bool) -> Option<char> {
    let (digits, signs) = if sup {
        ("⁰¹²³⁴⁵⁶⁷⁸⁹", ['⁺', '⁻'])
    } else {
        ("₀₁₂₃₄₅₆₇₈₉", ['₊', '₋'])
    };
    match c {
        '0'..='9' => digits.chars().nth(c as usize - '0' as usize),
        '+' => Some(signs[0]),
        '-' => Some(signs[1]),
        _ => None,
    }
}

/// LaTeX in a title made readable as plain text for a native menu: `$\mathcal{O}(1)$` →
/// `O(1)`, `$\alpha$-RuCl$_3$` → `α-RuCl₃`. Unknown commands keep their argument (or name).
/// Shortened to `max` characters with "…".
pub fn menu_text(title: &str, max: usize) -> String {
    let chars: Vec<char> = title.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '$' | '{' | '}' => i += 1,
            '\\' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j == start {
                    // `\(`, `\)`, `\,` and friends: spacing or delimiters; `\{` is a brace.
                    if let Some(&next) = chars.get(start) {
                        match next {
                            '{' | '}' | '%' | '&' | '#' | '_' => out.push(next),
                            ',' | ';' | ' ' => out.push(' '),
                            _ => {}
                        }
                    }
                    i = start + 1;
                    continue;
                }
                let cmd: String = chars[start..j].iter().collect();
                match symbol(&cmd) {
                    Some(s) => out.push_str(s),
                    // A command with an argument (\mathrm{X}) keeps the argument; a bare
                    // unknown one keeps its name.
                    None if chars.get(j) == Some(&'{') => {}
                    None => out.push_str(&cmd),
                }
                i = j;
            }
            '_' | '^' => {
                let sup = c == '^';
                // `_3`, `_{12}`, `^{-1}`: digits and signs become small characters.
                let (body, end) = if chars.get(i + 1) == Some(&'{') {
                    let close = chars[i + 2..]
                        .iter()
                        .position(|&x| x == '}')
                        .map(|p| i + 2 + p);
                    match close {
                        Some(e) => (chars[i + 2..e].to_vec(), e + 1),
                        None => (Vec::new(), i + 1),
                    }
                } else {
                    (
                        chars.get(i + 1).map(|&x| vec![x]).unwrap_or_default(),
                        i + 2,
                    )
                };
                let small: Option<String> = if body.is_empty() {
                    None
                } else {
                    body.iter().map(|&x| script(x, sup)).collect()
                };
                match small {
                    Some(s) => {
                        out.push_str(&s);
                        i = end;
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            c if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    let out = out.trim().to_owned();
    if out.chars().count() <= max {
        return out;
    }
    let cut: String = out.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latex_titles_read_well_in_a_menu() {
        let t = |s: &str| menu_text(s, 200);
        assert_eq!(
            t(r"Adiabatic state transfer with $\mathcal{O}(1)$ overhead"),
            "Adiabatic state transfer with O(1) overhead"
        );
        assert_eq!(t(r"$\alpha$-RuCl$_3$ under strain"), "α-RuCl₃ under strain");
        assert_eq!(t(r"$T^{-1}$ scaling of $\hbar\omega$"), "T⁻¹ scaling of ħω");
        assert_eq!(t(r"Spin-$\frac{1}{2}$ chains"), "Spin-12 chains");
        assert_eq!(t(r"$x_i$ and \{a\}  b"), "x_i and {a} b");
        assert_eq!(t("plain_text^ end"), "plain_text^ end");
    }

    #[test]
    fn long_titles_are_cut_with_an_ellipsis() {
        let s = menu_text(&"word ".repeat(30), 20);
        assert!(s.chars().count() <= 20 && s.ends_with('…'), "{s}");
        assert_eq!(menu_text("short", 20), "short");
    }

    #[test]
    fn authors_and_the_icon_count() {
        let a = |v: &[&str]| short_authors(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(a(&["Ada Lovelace"]), "Lovelace");
        assert_eq!(
            a(&["Jane Doe", "John Smith", "A B", "C D"]),
            "Doe, Smith +2"
        );
        let c = Companion {
            research_unread: 12,
            news_unread: 30,
            top: vec![],
            top_is_latest: false,
            sources: vec![],
            failed: 0,
            last_ok: None,
            research_next: Utc::now(),
            news_next: Utc::now(),
            weekly_goal: 0,
            week_read: 0,
            deadlines: vec![],
            arxiv_weekend: false,
            last_opened: None,
            recent_saved: vec![],
            suggested_left: 7,
            for_you_today: 0,
            later_today: 2,
            new_since_seen: 3,
        };
        assert_eq!(c.icon_title(IconCount::Suggested).as_deref(), Some("7"));
        assert_eq!(c.icon_title(IconCount::ForYou), None, "0 shows nothing");
        assert_eq!(c.icon_title(IconCount::Later).as_deref(), Some("2"));
        assert_eq!(c.icon_title(IconCount::Goal), None, "no goal set");
        let goal = Companion {
            weekly_goal: 10,
            week_read: 4,
            ..c.clone()
        };
        assert_eq!(goal.icon_title(IconCount::Goal).as_deref(), Some("4/10"));
        assert_eq!(c.icon_title(IconCount::Off), None);
        assert_eq!(c.icon_title(IconCount::Dot), None);
        assert_eq!(c.badge(IconCount::Dot, true), Badge::Dot);
        assert_eq!(c.badge(IconCount::Research, true), Badge::None);
        let failing = Companion {
            failed: 1,
            ..c.clone()
        };
        assert_eq!(failing.badge(IconCount::Off, true), Badge::Alert);
        assert_eq!(failing.badge(IconCount::Off, false), Badge::None);
        assert_eq!(c.icon_title(IconCount::Research).as_deref(), Some("12"));
        assert_eq!(c.icon_title(IconCount::All).as_deref(), Some("42"));
    }

    #[test]
    fn no_arxiv_at_weekends() {
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap();
        // Friday 13:30 UTC+8: Thursday night's announcement.
        assert!(!arxiv_weekend(at("2026-11-06T06:00:00Z")));
        // Saturday 13:30 UTC+8 until Monday 13:30 UTC+8: nothing new.
        assert!(arxiv_weekend(at("2026-11-07T06:00:00Z")));
        assert!(arxiv_weekend(at("2026-11-09T05:00:00Z")));
        assert!(!arxiv_weekend(at("2026-11-09T05:31:00Z")));
    }

    #[test]
    fn read_later_keeps_papers_for_the_chosen_days() {
        let now: DateTime<Utc> = "2026-11-03T06:00:00Z".parse().unwrap();
        let today = later_since(1, now);
        let week = later_since(7, now);
        assert!(
            today <= now && now - today < chrono::Duration::days(1),
            "today's midnight"
        );
        assert_eq!(
            today - week,
            chrono::Duration::days(6),
            "a week = today and the 6 days before"
        );
        assert_eq!(later_since(0, now), DateTime::<Utc>::UNIX_EPOCH, "no limit");
    }
}
