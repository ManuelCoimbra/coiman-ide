//! Hot exit: remembers the open folder and tabs, including unsaved text, so
//! nothing is lost on exit or crash (VS Code's `files.hotExit: "onExit"`).
//!
//! The session is written to `<config dir>/rust-code/session.json` on exit and
//! periodically while editing, so a crash loses at most a second or two of typing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Document;

const VERSION: u32 = 1;
/// Unsaved buffers larger than this are not backed up, to keep saves cheap.
const MAX_BACKUP_BYTES: usize = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize, Default, PartialEq, Debug)]
pub struct Session {
    pub version: u32,
    pub root: Option<PathBuf>,
    pub active: usize,
    pub docs: Vec<SessionDoc>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct SessionDoc {
    pub path: Option<PathBuf>,
    pub untitled_index: usize,
    /// Unsaved buffer content; `None` means the document matches the file on disk.
    pub backup: Option<String>,
}

impl Session {
    /// `include_unsaved: false` (hot exit off) remembers tabs but not unsaved text.
    pub fn capture(root: Option<&Path>, docs: &[Document], active: usize, include_unsaved: bool) -> Self {
        let docs = docs
            .iter()
            // An empty, never-saved untitled tab is not worth restoring.
            .filter(|d| d.path.is_some() || d.buffer.len_chars() > 0)
            .map(|d| SessionDoc {
                path: d.path.clone(),
                untitled_index: d.untitled_index(),
                backup: (include_unsaved && d.is_dirty() && d.buffer.rope().len_bytes() <= MAX_BACKUP_BYTES)
                    .then(|| d.text()),
            })
            .collect();
        Self { version: VERSION, root: root.map(Path::to_path_buf), active, docs }
    }

    /// Rebuilds documents. Files that no longer exist are dropped unless they have
    /// unsaved content, which is kept so it can be saved elsewhere.
    pub fn restore_docs(self) -> Vec<Document> {
        self.docs
            .into_iter()
            .filter_map(|d| {
                let on_disk = d.path.as_ref().and_then(|p| std::fs::read(p).ok());
                let saved = on_disk.map(|b| String::from_utf8_lossy(&b).into_owned());
                match (d.path, saved, d.backup) {
                    (Some(path), Some(saved), backup) => {
                        let text = backup.unwrap_or_else(|| saved.clone());
                        Some(Document::restored(Some(path), 0, saved, text))
                    }
                    (Some(path), None, Some(backup)) => Some(Document::restored(Some(path), 0, String::new(), backup)),
                    (Some(_), None, None) => None,
                    (None, _, backup) => Some(Document::restored(
                        None,
                        d.untitled_index,
                        String::new(),
                        backup.unwrap_or_default(),
                    )),
                }
            })
            .collect()
    }
}

fn session_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("session.json"))
}

pub fn load() -> Option<Session> {
    let text = std::fs::read_to_string(session_path()?).ok()?;
    let session: Session = serde_json::from_str(&text).ok()?;
    (session.version == VERSION).then_some(session)
}

/// Serializes the session. Kept separate from [`write`] so callers can skip
/// writing when nothing changed.
pub fn to_json(session: &Session) -> String {
    serde_json::to_string(session).unwrap_or_default()
}

/// Writes atomically (temp file + rename) so a crash mid-write can't corrupt the session.
pub fn write(json: &str) -> std::io::Result<()> {
    let Some(path) = session_path() else { return Ok(()) };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_keeps_unsaved_text() {
        let dir = std::env::temp_dir().join(format!("rust-code-session-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "on disk").unwrap();

        let mut clean = Document::open(&file).unwrap();
        let mut dirty_file = Document::open(&file).unwrap();
        let end = dirty_file.buffer.len_chars();
        dirty_file.buffer.set_single(crate::buffer::Selection::cursor(end));
        dirty_file.buffer.insert(" + edits");
        let mut untitled = Document::untitled(3);
        untitled.buffer.insert("scratch");
        let empty_untitled = Document::untitled(4);
        clean.uid = 1;

        let docs = vec![clean, dirty_file, untitled, empty_untitled];
        let session = Session::capture(Some(&dir), &docs, 1, true);
        assert_eq!(session.docs.len(), 3, "empty untitled tab is dropped");

        let json = to_json(&session);
        let back: Session = serde_json::from_str(&json).unwrap();
        assert_eq!(back, session);

        let restored = back.restore_docs();
        assert_eq!(restored.len(), 3);
        assert!(!restored[0].is_dirty());
        assert_eq!(restored[1].text(), "on disk + edits");
        assert!(restored[1].is_dirty());
        assert_eq!(restored[2].text(), "scratch");
        assert_eq!(restored[2].untitled_index(), 3);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_clean_file_is_dropped_but_unsaved_one_is_kept() {
        let session = Session {
            version: VERSION,
            root: None,
            active: 0,
            docs: vec![
                SessionDoc { path: Some("Z:/does/not/exist.txt".into()), untitled_index: 0, backup: None },
                SessionDoc {
                    path: Some("Z:/does/not/exist2.txt".into()),
                    untitled_index: 0,
                    backup: Some("keep me".into()),
                },
            ],
        };
        let docs = session.restore_docs();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].text(), "keep me");
    }
}
