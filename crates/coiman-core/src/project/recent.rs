//! Recently opened projects (folders) and files, newest first, with when each was last
//! opened, kept in `<config dir>/recent.json`.
//!
//! Entries whose folder or file is gone stay listed, marked, until removed: a drive that
//! isn't connected right now shouldn't cost the list its projects.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const MAX_PROJECTS: usize = 20;
pub const MAX_FILES: usize = 30;

/// One recent folder or file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    /// When it was last opened, in seconds since 1970 (0: before this was kept).
    #[serde(default)]
    pub opened: u64,
}

impl Entry {
    /// What to call it: its folder or file name.
    pub fn name(&self) -> String {
        self.path.file_name().map_or_else(|| self.path.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// Whether it's still there to open.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    #[serde(default)]
    pub projects: Vec<Entry>,
    #[serde(default)]
    pub files: Vec<Entry>,
}

impl Recent {
    /// Puts `folder` first among projects, once, and forgets the oldest past the limit.
    pub fn add_project(&mut self, folder: &Path, now: u64) {
        put_first(&mut self.projects, folder, now, MAX_PROJECTS);
    }

    /// Puts `file` first among files, once, and forgets the oldest past the limit.
    pub fn add_file(&mut self, file: &Path, now: u64) {
        put_first(&mut self.files, file, now, MAX_FILES);
    }

    /// Forgets `path`, as a project or a file. Whether it was listed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.projects.len() + self.files.len();
        self.projects.retain(|e| e.path != path);
        self.files.retain(|e| e.path != path);
        before != self.projects.len() + self.files.len()
    }

    /// Forgets every entry whose folder or file is gone. How many went.
    pub fn remove_missing(&mut self) -> usize {
        let before = self.projects.len() + self.files.len();
        self.projects.retain(Entry::exists);
        self.files.retain(Entry::exists);
        before - self.projects.len() - self.files.len()
    }

    pub fn clear(&mut self) {
        self.projects.clear();
        self.files.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty() && self.files.is_empty()
    }
}

fn put_first(list: &mut Vec<Entry>, path: &Path, now: u64, max: usize) {
    list.retain(|e| e.path != path);
    list.insert(0, Entry { path: path.to_path_buf(), opened: now });
    list.truncate(max);
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// How long ago `opened` was, in words: "just now", "5 minutes ago", "yesterday"…
/// Empty when it isn't known.
pub fn ago(opened: u64, now: u64) -> String {
    if opened == 0 {
        return String::new();
    }
    let secs = now.saturating_sub(opened);
    let plural = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        0..60 => "just now".to_owned(),
        60..3_600 => plural(secs / 60, "minute"),
        3_600..86_400 => plural(secs / 3_600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..2_592_000 => plural(secs / 86_400, "day"),
        2_592_000..31_536_000 => plural(secs / 2_592_000, "month"),
        _ => plural(secs / 31_536_000, "year"),
    }
}

fn recent_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("recent.json"))
}

/// The list as saved; the old list of folders alone still reads, as projects.
pub fn parse(text: &str) -> Recent {
    if let Ok(recent) = serde_json::from_str::<Recent>(text) {
        return recent;
    }
    let folders: Vec<PathBuf> = serde_json::from_str(text).unwrap_or_default();
    Recent { projects: folders.into_iter().map(|path| Entry { path, opened: 0 }).collect(), files: Vec::new() }
}

pub fn load() -> Recent {
    recent_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| parse(&text)).unwrap_or_default()
}

pub fn save(recent: &Recent) -> std::io::Result<()> {
    let Some(path) = recent_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(recent).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_repeats_up_to_the_limit() {
        let mut recent = Recent::default();
        recent.add_project(Path::new("a"), 1);
        recent.add_project(Path::new("b"), 2);
        recent.add_project(Path::new("a"), 3);
        let paths: Vec<_> = recent.projects.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(recent.projects[0].opened, 3);
        for i in 0..40 {
            recent.add_project(Path::new(&i.to_string()), 10 + i);
            recent.add_file(Path::new(&format!("f{i}")), 10 + i);
        }
        assert_eq!(recent.projects.len(), MAX_PROJECTS);
        assert_eq!(recent.files.len(), MAX_FILES);
        assert_eq!(recent.projects[0].path, PathBuf::from("39"));
    }

    #[test]
    fn entries_are_removed_one_at_a_time_or_all() {
        let mut recent = Recent::default();
        recent.add_project(Path::new("proj"), 1);
        recent.add_file(Path::new("a.rs"), 1);
        recent.add_file(Path::new("b.rs"), 2);
        assert!(recent.remove(Path::new("a.rs")));
        assert!(!recent.remove(Path::new("a.rs")), "already gone");
        assert_eq!(recent.files.len(), 1);
        assert!(recent.remove(Path::new("proj")));
        assert!(recent.projects.is_empty());
        recent.clear();
        assert!(recent.is_empty());
    }

    #[test]
    fn missing_entries_stay_until_removed() {
        let here = std::env::temp_dir();
        let gone = here.join("coiman_recent_surely_not_here_42");
        let mut recent = Recent::default();
        recent.add_project(&here, 1);
        recent.add_project(&gone, 2);
        recent.add_file(&gone.join("x.rs"), 2);
        assert!(!recent.projects[0].exists());
        assert!(recent.projects[1].exists());
        assert_eq!(recent.remove_missing(), 2);
        assert_eq!(recent.projects.len(), 1);
        assert!(recent.files.is_empty());
    }

    #[test]
    fn the_old_list_of_folders_still_reads() {
        let old = parse(r#"["C:\\a", "C:\\b"]"#);
        assert_eq!(old.projects.len(), 2);
        assert_eq!(old.projects[0].path, PathBuf::from("C:\\a"));
        assert_eq!(old.projects[0].opened, 0);
        let mut new = Recent::default();
        new.add_project(Path::new("p"), 5);
        new.add_file(Path::new("f"), 6);
        assert_eq!(parse(&serde_json::to_string(&new).unwrap()), new);
        assert_eq!(parse("not json"), Recent::default());
    }

    #[test]
    fn times_in_words() {
        let now = 10_000_000;
        assert_eq!(ago(0, now), "");
        assert_eq!(ago(now - 5, now), "just now");
        assert_eq!(ago(now - 60, now), "1 minute ago");
        assert_eq!(ago(now - 5 * 3_600, now), "5 hours ago");
        assert_eq!(ago(now - 90_000, now), "yesterday");
        assert_eq!(ago(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(ago(now - 70 * 86_400, now), "2 months ago");
    }
}
