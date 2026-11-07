//! Where each file dialog was last used, so each opens where it was left: Open File in
//! one place, a background picture's in another. Kept in `<config dir>/dialogs.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DialogDirs {
    #[serde(flatten)]
    dirs: BTreeMap<String, PathBuf>,
}

impl DialogDirs {
    /// Where dialog `kind` was last left, if that folder is still there.
    pub fn get(&self, kind: &str) -> Option<&Path> {
        self.dirs.get(kind).map(PathBuf::as_path).filter(|d| d.is_dir())
    }

    /// Remembers where dialog `kind` picked `chosen`: the folder holding it, so the next
    /// time it opens beside it.
    pub fn remember(&mut self, kind: &str, chosen: &Path) {
        if let Some(dir) = chosen.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.dirs.insert(kind.to_owned(), dir.to_path_buf());
        }
    }

    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }
}

fn file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("dialogs.json"))
}

pub fn load() -> DialogDirs {
    file().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| DialogDirs::parse(&t)).unwrap_or_default()
}

pub fn save(dirs: &DialogDirs) -> std::io::Result<()> {
    let Some(path) = file() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(dirs).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_dialog_keeps_its_own_folder() {
        let here = std::env::temp_dir();
        let mut dirs = DialogDirs::default();
        dirs.remember("open_file", &here.join("a.txt"));
        dirs.remember("background_picture", &here.join("pics").join("b.png"));
        assert_eq!(dirs.get("open_file"), Some(here.as_path()));
        // A folder that's gone isn't offered.
        assert_eq!(dirs.get("background_picture"), None);
        assert_eq!(dirs.get("save_as"), None);
        // Round trip.
        let text = serde_json::to_string(&dirs).unwrap();
        assert_eq!(DialogDirs::parse(&text), dirs);
        assert_eq!(DialogDirs::parse("nonsense"), DialogDirs::default());
    }
}
