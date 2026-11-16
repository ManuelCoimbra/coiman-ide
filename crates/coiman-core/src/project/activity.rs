//! Time spent per project, kept on this computer only: seconds of active use (the window in
//! front and something typed or clicked lately) by day and project, in `activity.json` in the
//! config folder. Nothing leaves the machine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// Day (`2026-11-16`) to project folder to seconds.
    #[serde(default)]
    pub days: BTreeMap<String, BTreeMap<String, u64>>,
}

/// Today, in the computer's time zone.
pub fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn file() -> Option<PathBuf> {
    Some(crate::settings::config_dir()?.join("activity.json"))
}

/// `1h 05m`, `12m`, `40s`.
pub fn duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    match (h, m) {
        (0, 0) => format!("{secs}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

fn name(project: &str) -> String {
    let p = std::path::Path::new(project);
    p.file_name().map_or_else(|| project.to_owned(), |n| n.to_string_lossy().into_owned())
}

impl Activity {
    pub fn load() -> Self {
        file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = file().ok_or("No config folder")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, serde_json::to_string(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }

    /// Adds `secs` of use to `project` on `day`.
    pub fn add(&mut self, day: NaiveDate, project: &str, secs: u64) {
        if secs == 0 {
            return;
        }
        *self.days.entry(day.to_string()).or_default().entry(project.to_owned()).or_default() += secs;
    }

    fn day_total(&self, day: NaiveDate) -> u64 {
        self.days.get(&day.to_string()).map_or(0, |p| p.values().sum())
    }

    /// The statistics as Markdown, as of `today`.
    pub fn report(&self, today: NaiveDate) -> String {
        let mut md = String::from(
            "# Session statistics\n\nActive time: the window in front, with something typed or clicked in the last two minutes. Kept on this computer only.\n\n",
        );
        if self.days.is_empty() {
            md.push_str("Nothing yet: use Coiman for a while, then look again.\n");
            return md;
        }
        let back = |n: i64| today - chrono::Duration::days(n);
        let week: u64 = (0..7).map(|n| self.day_total(back(n))).sum();
        let month: u64 = (0..30).map(|n| self.day_total(back(n))).sum();
        md.push_str(&format!(
            "- Today: {}\n- Last 7 days: {}\n- Last 30 days: {}\n- Days with use: {}\n\n",
            duration(self.day_total(today)),
            duration(week),
            duration(month),
            self.days.len()
        ));
        if let Some(projects) = self.days.get(&today.to_string()) {
            md.push_str("## Today\n\n| Project | Time |\n|---|---:|\n");
            let mut list: Vec<_> = projects.iter().collect();
            list.sort_by_key(|p| std::cmp::Reverse(*p.1));
            for (project, secs) in list {
                md.push_str(&format!("| {} | {} |\n", name(project), duration(*secs)));
            }
            md.push('\n');
        }
        md.push_str("## Last 14 days\n\n| Day | Time |\n|---|---:|\n");
        for n in 0..14 {
            let day = back(n);
            let secs = self.day_total(day);
            if secs > 0 {
                md.push_str(&format!("| {} | {} |\n", day.format("%a %Y-%m-%d"), duration(secs)));
            }
        }
        md.push('\n');
        let mut all: BTreeMap<&str, u64> = BTreeMap::new();
        for projects in self.days.values() {
            for (project, secs) in projects {
                *all.entry(project.as_str()).or_default() += secs;
            }
        }
        md.push_str("## Per project, all time\n\n| Project | Time | Folder |\n|---|---:|---|\n");
        let mut all: Vec<_> = all.into_iter().collect();
        all.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (project, secs) in all {
            md.push_str(&format!("| {} | {} | `{project}` |\n", name(project), duration(secs)));
        }
        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_up_by_day_and_project() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let mut a = Activity::default();
        assert!(a.report(day).contains("Nothing yet"));
        a.add(day, "/w/coiman", 3600 + 300);
        a.add(day, "/w/site", 120);
        a.add(day - chrono::Duration::days(1), "/w/coiman", 600);
        a.add(day - chrono::Duration::days(40), "/w/old", 60);
        a.add(day, "/w/site", 0);
        let md = a.report(day);
        assert!(md.contains("- Today: 1h 07m"), "{md}");
        assert!(md.contains("- Last 7 days: 1h 17m"), "{md}");
        assert!(md.contains("- Days with use: 3"), "{md}");
        assert!(md.contains("| coiman | 1h 05m |"), "{md}");
        assert!(md.contains("| Sun 2026-11-15 | 10m |"), "{md}");
        assert!(md.contains("| old | 1m | `/w/old` |"), "{md}");
        assert_eq!(duration(40), "40s");
        let back: Activity = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
