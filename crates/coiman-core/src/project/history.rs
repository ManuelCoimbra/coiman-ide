//! Local history: a copy of each file every time it is saved (or changed by
//! another program), kept in the config folder independently of any version
//! control, so earlier versions can be compared and restored.
//!
//! Layout: `<config>/rust-code/history/<path hash>/index.json` lists the
//! entries; each entry's text is stored next to it as `<id>.txt`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Why a version was kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    Save,
    /// Another program changed the file.
    External,
    /// The text before a version was restored over it.
    BeforeRestore,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Save => "Saved",
            Source::External => "Changed outside",
            Source::BeforeRestore => "Before restore",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Unique within the file; also the content file name.
    pub id: String,
    /// Unix time in milliseconds.
    pub time: u64,
    pub source: Source,
    pub size: usize,
    /// Content hash, to skip storing the same text twice in a row.
    hash: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Index {
    path: String,
    entries: Vec<Entry>,
}

/// How much history to keep.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_entries: usize,
    pub max_days: u64,
    pub max_file_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_entries: 50, max_days: 30, max_file_bytes: 5 * 1024 * 1024 }
    }
}

/// FNV-1a: a hash that stays the same across Rust versions (it names folders).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub struct History {
    root: PathBuf,
}

impl History {
    /// History in `<config>/rust-code/history`.
    pub fn open() -> Option<History> {
        crate::settings::config_dir().map(|d| History::at(d.join("history")))
    }

    pub fn at(root: PathBuf) -> History {
        History { root }
    }

    fn dir(&self, path: &Path) -> PathBuf {
        let key =
            if cfg!(windows) { path.to_string_lossy().to_lowercase() } else { path.to_string_lossy().into_owned() };
        self.root.join(format!("{:016x}", fnv(key.as_bytes())))
    }

    fn read_index(&self, path: &Path) -> Index {
        let file = self.dir(path).join("index.json");
        std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    /// Keeps `text` as a version of `path`. Returns false when nothing was
    /// stored (same as the latest version, or the file is too large).
    pub fn record(&self, path: &Path, text: &str, source: Source, limits: Limits) -> std::io::Result<bool> {
        if text.len() > limits.max_file_bytes {
            return Ok(false);
        }
        let dir = self.dir(path);
        let mut index = self.read_index(path);
        let hash = fnv(text.as_bytes());
        if index.entries.last().is_some_and(|e| e.hash == hash) {
            return Ok(false);
        }
        std::fs::create_dir_all(&dir)?;
        let time = now_ms();
        let mut id = time.to_string();
        while index.entries.iter().any(|e| e.id == id) {
            id.push('a');
        }
        std::fs::write(dir.join(format!("{id}.txt")), text)?;
        index.path = path.to_string_lossy().into_owned();
        index.entries.push(Entry { id, time, source, size: text.len(), hash });

        // Prune by age and count, oldest first.
        let cutoff = time.saturating_sub(limits.max_days * 86_400_000);
        let excess = index.entries.len().saturating_sub(limits.max_entries.max(1));
        let mut position = 0;
        index.entries.retain(|e| {
            let drop = position < excess || e.time < cutoff;
            position += 1;
            if drop {
                let _ = std::fs::remove_file(dir.join(format!("{}.txt", e.id)));
            }
            !drop
        });
        let json = serde_json::to_string_pretty(&index).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("index.json"), json)?;
        Ok(true)
    }

    /// Versions of `path`, newest first.
    pub fn entries(&self, path: &Path) -> Vec<Entry> {
        let mut entries = self.read_index(path).entries;
        entries.reverse();
        entries
    }

    /// The text of one version.
    pub fn content(&self, path: &Path, entry: &Entry) -> std::io::Result<String> {
        std::fs::read_to_string(self.dir(path).join(format!("{}.txt", entry.id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_lists_reads_and_prunes() {
        let root = std::env::temp_dir().join(format!("rust-code-history-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let history = History::at(root.clone());
        let file = Path::new("C:/work/src/main.rs");
        let limits = Limits { max_entries: 3, ..Limits::default() };

        assert!(history.record(file, "one", Source::Save, limits).unwrap());
        // The same text again is not stored twice.
        assert!(!history.record(file, "one", Source::Save, limits).unwrap());
        for text in ["two", "three", "four"] {
            assert!(history.record(file, text, Source::Save, limits).unwrap());
        }
        history.record(file, "five", Source::External, limits).unwrap();

        let entries = history.entries(file);
        assert_eq!(entries.len(), 3, "pruned to the limit");
        let texts: Vec<String> = entries.iter().map(|e| history.content(file, e).unwrap()).collect();
        assert_eq!(texts, vec!["five", "four", "three"]);
        assert_eq!(entries[0].source, Source::External);
        // Pruned versions' files are gone.
        let files = std::fs::read_dir(history.dir(file)).unwrap().count();
        assert_eq!(files, 4, "three versions plus the index");
        // Too large: skipped.
        let tiny = Limits { max_file_bytes: 2, ..limits };
        assert!(!history.record(file, "large", Source::Save, tiny).unwrap());
        let _ = std::fs::remove_dir_all(&root);
    }
}
