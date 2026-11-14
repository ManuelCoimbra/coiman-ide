//! What the File Manager lists: a folder's entries with their sizes and dates, sorted and
//! filtered, and the places to start from (home, its usual folders, the drives).

use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// One file or folder in a listing.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    /// Bytes; 0 for a folder.
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// A dot file, or marked hidden on Windows.
    pub hidden: bool,
    pub readonly: bool,
}

/// What a listing is sorted by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SortBy {
    #[default]
    Name,
    Size,
    Modified,
    Kind,
}

/// The entries of `dir`, in no order. Entries that can't be read are left out.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        // Through a link, what it points at (a link to a folder opens as one).
        let Ok(meta) = std::fs::metadata(&path).or_else(|_| item.metadata()) else { continue };
        let is_dir = meta.is_dir();
        out.push(Entry {
            hidden: name.starts_with('.') || hidden_attribute(&meta),
            readonly: meta.permissions().readonly(),
            size: if is_dir { 0 } else { meta.len() },
            modified: meta.modified().ok(),
            is_dir,
            name,
            path,
        });
    }
    Ok(out)
}

#[cfg(windows)]
fn hidden_attribute(meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn hidden_attribute(_meta: &std::fs::Metadata) -> bool {
    false
}

/// What kind of file a name is, as a column shows it: "Folder", the extension ("RS"), or "File".
pub fn kind(entry: &Entry) -> String {
    if entry.is_dir {
        return "Folder".to_owned();
    }
    match entry.name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_uppercase(),
        _ => "File".to_owned(),
    }
}

/// Names compared as people read them: case aside, and "file2" before "file10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let by_number = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if by_number != Ordering::Equal {
                    return by_number;
                }
            }
            (Some(x), Some(y)) => {
                let by_letter = x.to_lowercase().cmp(y.to_lowercase());
                if by_letter != Ordering::Equal {
                    return by_letter;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Sorts folders first, then by `by` (names breaking ties), reversed when `descending`.
pub fn sort(entries: &mut [Entry], by: SortBy, descending: bool) {
    entries.sort_by(|a, b| {
        let within = match by {
            SortBy::Name => Ordering::Equal,
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Modified => a.modified.cmp(&b.modified),
            SortBy::Kind => kind(a).cmp(&kind(b)),
        }
        .then_with(|| natural_cmp(&a.name, &b.name));
        b.is_dir.cmp(&a.is_dir).then(if descending { within.reverse() } else { within })
    });
}

/// Whether `entry` passes the filter: hidden ones only when shown, and `text` found in the
/// name (case aside), or matched as a pattern when it has `*` or `?` ("*.rs").
pub fn passes(entry: &Entry, text: &str, show_hidden: bool) -> bool {
    if entry.hidden && !show_hidden {
        return false;
    }
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return true;
    }
    let name = entry.name.to_lowercase();
    if text.contains(['*', '?']) { wildcard(&text, &name) } else { name.contains(&text) }
}

/// `*` for any run of characters, `?` for one.
fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi, mut ni, mut star, mut back) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            back = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            back += 1;
            ni = back;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// A size as people read it: "0 B", "12 KB", "3.4 MB".
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 || size >= 100.0 { format!("{size:.0} {}", UNITS[unit]) } else { format!("{size:.1} {}", UNITS[unit]) }
}

/// A date as the Modified column shows it, in local time: "2026-11-14 09:05".
pub fn date_text(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
}

/// Where to start from: the home folder and its usual folders that exist, then the drives
/// (Windows) or the root (elsewhere). Each with the name to show.
pub fn places() -> Vec<(String, PathBuf)> {
    // On a phone: its storage and Termux's home come first.
    let mut out = crate::mobile::phone().places();
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    if let Some(home) = home.filter(|h| h.is_dir()) {
        out.push(("Home".to_owned(), home.clone()));
        for name in ["Desktop", "Documents", "Downloads", "Pictures"] {
            let dir = home.join(name);
            if dir.is_dir() {
                out.push((name.to_owned(), dir));
            }
        }
    }
    if cfg!(windows) {
        for letter in b'A'..=b'Z' {
            let root = PathBuf::from(format!("{}:\\", letter as char));
            // Floppy letters are skipped: asking an empty drive can stall.
            if letter >= b'C' && root.is_dir() {
                out.push((format!("{}:", letter as char), root));
            }
        }
    } else {
        out.push(("Computer".to_owned(), PathBuf::from("/")));
    }
    out
}

/// The File Manager's lasting state: its bookmarks and where its panes were.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq, Clone)]
pub struct Saved {
    #[serde(default)]
    pub bookmarks: Vec<PathBuf>,
    /// Each pane's tabs (folders) and which tab is in front.
    #[serde(default)]
    pub panes: Vec<(Vec<PathBuf>, usize)>,
    #[serde(default)]
    pub dual: bool,
    #[serde(default)]
    pub show_hidden: bool,
}

impl Saved {
    fn file() -> Option<PathBuf> {
        crate::settings::config_dir().map(|d| d.join("file_manager.json"))
    }

    pub fn load() -> Self {
        Self::file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn store(&self) -> io::Result<()> {
        let Some(file) = Self::file() else { return Ok(()) };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(file, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool, size: u64) -> Entry {
        Entry {
            path: PathBuf::from(name),
            name: name.to_owned(),
            is_dir,
            size,
            modified: None,
            hidden: name.starts_with('.'),
            readonly: false,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn folders_come_first_and_names_sort_as_read() {
        let mut list = vec![
            entry("file10.rs", false, 5),
            entry("B", true, 0),
            entry("file2.rs", false, 50),
            entry("a", true, 0),
            entry("Readme.md", false, 1),
        ];
        sort(&mut list, SortBy::Name, false);
        assert_eq!(names(&list), ["a", "B", "file2.rs", "file10.rs", "Readme.md"]);
        sort(&mut list, SortBy::Name, true);
        assert_eq!(names(&list), ["B", "a", "Readme.md", "file10.rs", "file2.rs"]);
        sort(&mut list, SortBy::Size, true);
        assert_eq!(names(&list), ["B", "a", "file2.rs", "file10.rs", "Readme.md"]);
        sort(&mut list, SortBy::Kind, false);
        assert_eq!(names(&list), ["a", "B", "Readme.md", "file2.rs", "file10.rs"]);
    }

    #[test]
    fn filter_by_words_patterns_and_hidden() {
        let rs = entry("Main.rs", false, 1);
        let dot = entry(".env", false, 1);
        assert!(passes(&rs, "", false));
        assert!(passes(&rs, "MAIN", false));
        assert!(passes(&rs, "*.rs", false));
        assert!(passes(&rs, "m?in*", false));
        assert!(!passes(&rs, "*.md", false));
        assert!(!passes(&dot, "", false));
        assert!(passes(&dot, "env", true));
    }

    #[test]
    fn sizes_and_kinds_read_plainly() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1536), "1.5 KB");
        assert_eq!(human_size(250 * 1024 * 1024), "250 MB");
        assert_eq!(kind(&entry("x.tar.gz", false, 0)), "GZ");
        assert_eq!(kind(&entry(".env", false, 0)), "File");
        assert_eq!(kind(&entry("src", true, 0)), "Folder");
    }

    #[test]
    fn lists_a_folder() {
        let dir = std::env::temp_dir().join(format!("coiman_listing_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        let mut list = list(&dir).unwrap();
        sort(&mut list, SortBy::Name, false);
        assert_eq!(names(&list), ["sub", ".hidden", "a.txt"]);
        let a = list.iter().find(|e| e.name == "a.txt").unwrap();
        assert_eq!((a.size, a.is_dir, a.hidden), (5, false, false));
        assert!(list.iter().any(|e| e.name == "sub" && e.is_dir));
        assert!(list.iter().any(|e| e.name == ".hidden" && e.hidden));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saved_state_reads_back() {
        let saved = Saved {
            bookmarks: vec![PathBuf::from("/x")],
            panes: vec![(vec![PathBuf::from("/a"), PathBuf::from("/b")], 1)],
            dual: true,
            show_hidden: false,
        };
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
        assert_eq!(serde_json::from_str::<Saved>("{}").unwrap(), Saved::default());
    }
}
