//! Recently opened folders, newest first, kept in `<config dir>/recent.json`.

use std::path::{Path, PathBuf};

const MAX: usize = 10;

fn recent_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("recent.json"))
}

pub fn load() -> Vec<PathBuf> {
    recent_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(folders: &[PathBuf]) -> std::io::Result<()> {
    let Some(path) = recent_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(folders).unwrap_or_default())
}

/// Puts `folder` first, once, and forgets the oldest past the limit.
pub fn add(folders: &mut Vec<PathBuf>, folder: &Path) {
    folders.retain(|f| f != folder);
    folders.insert(0, folder.to_path_buf());
    folders.truncate(MAX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_repeats_up_to_the_limit() {
        let mut folders = Vec::new();
        add(&mut folders, Path::new("a"));
        add(&mut folders, Path::new("b"));
        add(&mut folders, Path::new("a"));
        assert_eq!(folders, [PathBuf::from("a"), PathBuf::from("b")]);
        for i in 0..20 {
            add(&mut folders, Path::new(&i.to_string()));
        }
        assert_eq!(folders.len(), MAX);
        assert_eq!(folders[0], PathBuf::from("19"));
    }
}
