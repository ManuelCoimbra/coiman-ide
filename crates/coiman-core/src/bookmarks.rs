//! Bookmarks: marked lines by file, kept in `<config dir>/bookmarks.json`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::git::Hunk;

pub type Bookmarks = BTreeMap<String, Vec<usize>>;

fn file_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("bookmarks.json"))
}

pub fn load() -> Bookmarks {
    file_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(bookmarks: &Bookmarks) -> std::io::Result<()> {
    let Some(path) = file_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(bookmarks).unwrap_or_default())
}

/// Where marked lines end up after an edit. A marked line that was changed stays
/// with the text at its place; one that was deleted moves to the line after.
pub fn shift(lines: &[usize], hunks: &[Hunk]) -> Vec<usize> {
    let mut out: Vec<usize> = lines
        .iter()
        .map(|&line| {
            let mut delta = 0isize;
            for h in hunks {
                if line < h.old.start {
                    break;
                }
                if line < h.old.end {
                    let inside = line - h.old.start;
                    return if h.new.is_empty() { h.new.start } else { h.new.start + inside.min(h.new.len() - 1) };
                }
                delta = h.new.end as isize - h.old.end as isize;
            }
            (line as isize + delta) as usize
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff_lines;

    fn after(old: &[&str], new: &[&str], lines: &[usize]) -> Vec<usize> {
        shift(lines, &diff_lines(old, new, false))
    }

    #[test]
    fn marks_follow_their_lines() {
        let old = ["a", "b", "c", "d"];
        // Lines added above push a mark down; below leave it.
        assert_eq!(after(&old, &["x", "y", "a", "b", "c", "d"], &[1]), [3]);
        assert_eq!(after(&old, &["a", "b", "x", "c", "d"], &[1, 2]), [1, 3]);
        // Lines deleted above pull it up.
        assert_eq!(after(&old, &["c", "d"], &[3]), [1]);
        // A deleted marked line moves to the next one, and two marks can merge.
        assert_eq!(after(&old, &["a", "c", "d"], &[1, 2]), [1]);
        // A changed line keeps its mark.
        assert_eq!(after(&old, &["a", "B", "c", "d"], &[1]), [1]);
        // Several edits add up.
        assert_eq!(after(&old, &["n", "a", "b", "d"], &[0, 3]), [1, 3]);
    }
}
