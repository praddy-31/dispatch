//! Conference and journal deadlines: a hand-edited `deadlines.json` in the data
//! folder, shown in the menu-bar menu and in Stats ("APS abstract in 12 days"). No network.
//!
//! ```json
//! [
//!   { "name": "APS March Meeting abstract", "date": "2026-11-20" },
//!   { "name": "PRL resubmission", "date": "2026-12-01", "url": "https://journals.aps.org" }
//! ]
//! ```

use std::path::Path;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::paths::write_atomic;

pub const FILE: &str = "deadlines.json";

/// Deadlines further away than this aren't shown.
pub const HORIZON_DAYS: i64 = 90;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deadline {
    pub name: String,
    /// `YYYY-MM-DD`; the deadline is the end of that day.
    pub date: NaiveDate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Upcoming {
    pub name: String,
    pub date: NaiveDate,
    pub url: Option<String>,
    /// 0 = today.
    pub days: i64,
    /// "today", "tomorrow", "in 12 days".
    pub when: String,
}

/// The file's deadlines; a missing file is an empty list, a broken one says why.
pub fn load(data_dir: &Path) -> Result<Vec<Deadline>, String> {
    let path = data_dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{FILE} is not valid: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(format!("can't read {FILE}: {e}")),
    }
}

/// Deadlines from `today` to [`HORIZON_DAYS`] ahead, soonest first, at most `n`.
pub fn upcoming(list: &[Deadline], today: NaiveDate, n: usize) -> Vec<Upcoming> {
    let mut out: Vec<Upcoming> = list
        .iter()
        .filter_map(|d| {
            let days = (d.date - today).num_days();
            (0..=HORIZON_DAYS).contains(&days).then(|| Upcoming {
                name: d.name.clone(),
                date: d.date,
                url: d.url.clone(),
                days,
                when: match days {
                    0 => "today".into(),
                    1 => "tomorrow".into(),
                    n => format!("in {n} days"),
                },
            })
        })
        .collect();
    out.sort_by_key(|u| (u.days, u.name.clone()));
    out.truncate(n);
    out
}

/// Makes the file with one example if it doesn't exist (Settings' "Edit deadlines…").
pub fn ensure(data_dir: &Path, today: NaiveDate) -> Result<std::path::PathBuf, String> {
    let path = data_dir.join(FILE);
    if !path.exists() {
        let example = [Deadline {
            name: "Example: conference abstract (edit or remove me)".into(),
            date: today + chrono::Days::new(30),
            url: None,
        }];
        let mut json = serde_json::to_string_pretty(&example).map_err(|e| e.to_string())?;
        json.push('\n');
        write_atomic(&path, json.as_bytes())?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upcoming_deadlines_soonest_first_within_the_horizon() {
        let today: NaiveDate = "2026-11-01".parse().unwrap();
        let d = |name: &str, date: &str| Deadline {
            name: name.into(),
            date: date.parse().unwrap(),
            url: None,
        };
        let list = vec![
            d("past", "2026-10-31"),
            d("far", "2027-06-01"),
            d("abstract", "2026-11-13"),
            d("today", "2026-11-01"),
            d("tomorrow", "2026-11-02"),
        ];
        let got = upcoming(&list, today, 3);
        let names: Vec<_> = got
            .iter()
            .map(|u| (u.name.as_str(), u.when.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("today", "today"),
                ("tomorrow", "tomorrow"),
                ("abstract", "in 12 days")
            ]
        );
    }

    #[test]
    fn the_file_is_made_once_and_errors_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()).unwrap(), vec![]);
        let today: NaiveDate = "2026-11-01".parse().unwrap();
        let path = ensure(dir.path(), today).unwrap();
        assert_eq!(load(dir.path()).unwrap().len(), 1);
        std::fs::write(&path, "[{\"name\": \"x\"}]").unwrap();
        assert!(load(dir.path()).unwrap_err().contains("not valid"));
        ensure(dir.path(), today).unwrap();
        assert!(
            load(dir.path()).is_err(),
            "an existing file is never replaced"
        );
    }
}
