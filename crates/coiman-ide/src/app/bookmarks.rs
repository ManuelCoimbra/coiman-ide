//! Bookmarks: mark lines, jump between them, and find them again after a restart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use coiman_core::bookmarks::{self, Bookmarks};
use coiman_core::git;

use super::CodeApp;
use crate::buffer::Selection;
use crate::editor_view::Decorations;
use crate::palette::PickerKind;

#[derive(Default)]
pub(super) struct BookmarkState {
    lines: Bookmarks,
    loaded: bool,
    /// The text last seen of each bookmarked document, to follow its lines.
    seen: HashMap<u64, (u64, String)>,
    /// What the open picker lists: its label, file and line.
    listed: Vec<(String, PathBuf, usize)>,
}

fn key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl CodeApp {
    fn save_bookmarks(&mut self) {
        if let Err(e) = bookmarks::save(&self.bookmarks.lines) {
            self.status = Some(format!("Could not save bookmarks: {e}"));
        }
    }

    /// Keeps marks on their lines as text is added and removed above them.
    pub(super) fn pump_bookmarks(&mut self) {
        if !self.bookmarks.loaded {
            self.bookmarks.loaded = true;
            self.bookmarks.lines = bookmarks::load();
        }
        if self.bookmarks.lines.is_empty() {
            return;
        }
        let mut changed = false;
        for doc in &self.docs {
            let Some(path) = &doc.path else { continue };
            let Some(lines) = self.bookmarks.lines.get_mut(&key(path)) else { continue };
            let version = doc.buffer.version();
            let Some((seen, old)) = self.bookmarks.seen.get_mut(&doc.uid) else {
                self.bookmarks.seen.insert(doc.uid, (version, doc.text()));
                continue;
            };
            if *seen == version {
                continue;
            }
            let new = doc.text();
            let (before, after) = (git::lines(old), git::lines(&new));
            if before.len() != after.len() {
                let moved = bookmarks::shift(lines, &git::diff_lines(&before, &after, false));
                if moved != *lines {
                    *lines = moved;
                    changed = true;
                }
            }
            (*seen, *old) = (version, new);
        }
        if changed {
            self.save_bookmarks();
        }
    }

    pub(super) fn bookmarks_decorate(&self, i: usize, deco: &mut Decorations) {
        if let Some(lines) =
            self.docs.get(i).and_then(|d| d.path.as_deref()).and_then(|p| self.bookmarks.lines.get(&key(p)))
        {
            deco.bookmarks.clone_from(lines);
        }
    }

    pub(super) fn toggle_bookmark(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.as_deref() else {
            self.status = Some("Save the file to bookmark it".to_owned());
            return;
        };
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let (name, uid, version, text) = (key(path), doc.uid, doc.buffer.version(), doc.text());
        self.bookmarks.seen.insert(uid, (version, text));
        let lines = self.bookmarks.lines.entry(name.clone()).or_default();
        match lines.binary_search(&line) {
            Ok(at) => {
                lines.remove(at);
            }
            Err(at) => lines.insert(at, line),
        }
        if lines.is_empty() {
            self.bookmarks.lines.remove(&name);
        }
        self.save_bookmarks();
    }

    /// To the next or previous mark in this file, going round.
    pub(super) fn next_bookmark(&mut self, forward: bool) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let lines = doc.path.as_deref().and_then(|p| self.bookmarks.lines.get(&key(p)));
        let Some(lines) = lines.filter(|l| !l.is_empty()) else {
            self.status = Some("No bookmarks in this file".to_owned());
            return;
        };
        let here = doc.buffer.char_to_line(doc.buffer.primary().head);
        let target = if forward {
            lines.iter().find(|l| **l > here).or(lines.first())
        } else {
            lines.iter().rev().find(|l| **l < here).or(lines.last())
        };
        if let Some(line) = target {
            let at = doc.buffer.line_start((*line).min(doc.buffer.len_lines() - 1));
            doc.buffer.set_single(Selection::cursor(at));
            doc.reveal = true;
            doc.request_focus = true;
        }
    }

    /// A picker over every bookmark, in all files.
    pub(super) fn list_bookmarks(&mut self) {
        let mut listed = Vec::new();
        for (file, lines) in &self.bookmarks.lines {
            let path = PathBuf::from(file);
            // An open document may have unsaved edits the marks already follow.
            let open = self.docs.iter().find(|d| d.path.as_deref() == Some(path.as_path()));
            let text = open.map_or_else(|| std::fs::read_to_string(&path).unwrap_or_default(), |d| d.text());
            let content = git::lines(&text);
            let name = path.file_name().map_or(file.clone(), |n| n.to_string_lossy().into_owned());
            for line in lines {
                let preview = content.get(*line).map_or("", |l| l.trim());
                listed.push((format!("{name}:{}  {preview}", line + 1), path.clone(), *line));
            }
        }
        if listed.is_empty() {
            self.status = Some("No bookmarks yet".to_owned());
            return;
        }
        let items = listed.iter().map(|l| l.0.clone()).collect();
        self.bookmarks.listed = listed;
        self.palette.show_picker(PickerKind::Bookmark, items, "", "Go to a bookmark");
    }

    pub(super) fn picked_bookmark(&mut self, label: &str) {
        let Some((_, path, line)) = self.bookmarks.listed.iter().find(|l| l.0 == label).cloned() else { return };
        self.open_file(&path);
        if let Some(doc) = self.docs.get_mut(self.active) {
            let at = doc.buffer.line_start(line.min(doc.buffer.len_lines() - 1));
            doc.buffer.set_single(Selection::cursor(at));
            doc.reveal = true;
            doc.request_focus = true;
        }
    }
}
