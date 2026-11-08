//! Local history in the workbench: versions are kept on every save and when
//! another program changes a file; the Timeline section lists them, and any
//! version can be compared with the current text, reverted change by change,
//! or restored whole (as an unsaved, undoable edit).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use coiman_core::project::git;
use coiman_core::project::history::{Entry, History, Limits, Source};
use egui::{Align, RichText, Sense};

use crate::buffer::Selection;
use crate::document::Document;
use crate::ui::icons::{self, Icon};
use crate::ui::palette::PickerKind;
use crate::ui::theme;
use crate::watcher::same_path;

use crate::app::CodeApp;

/// Seconds a file's version list is reused before reading it again.
const CACHE_SECONDS: f64 = 2.0;

pub(crate) struct HistoryState {
    store: Option<History>,
    /// Version lists by file key: (time read, newest first).
    cache: HashMap<PathBuf, (f64, Vec<Entry>)>,
    /// Labels offered by the picker, with their file and version.
    pick: Vec<(String, PathBuf, Entry)>,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self { store: History::open(), cache: HashMap::new(), pick: Vec::new() }
    }
}

fn key(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().to_lowercase()) } else { path.to_path_buf() }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// "Saved · 3 minutes ago".
fn describe(e: &Entry) -> String {
    let ago = git::relative_time(((now_ms().saturating_sub(e.time)) / 1000) as i64);
    format!("{} · {ago}", e.source.label())
}

impl CodeApp {
    fn history_limits(&self) -> Limits {
        let h = &self.settings.history;
        Limits { max_entries: h.max_entries, max_days: h.max_days, max_file_bytes: h.max_file_kb * 1024 }
    }

    /// Keeps the current text of document `i` as a version (in the background).
    pub(crate) fn history_record(&mut self, i: usize, source: Source) {
        if !self.settings.history.enabled {
            return;
        }
        let Some(doc) = self.docs.get(i) else { return };
        let (Some(path), Some(config)) = (doc.path.clone(), crate::settings::config_dir()) else { return };
        let (text, limits) = (doc.text(), self.history_limits());
        self.history.cache.remove(&key(&path));
        std::thread::spawn(move || {
            let _ = History::at(config.join("history")).record(&path, &text, source, limits);
        });
    }

    /// Versions of `path`, newest first (cached briefly).
    fn history_entries(&mut self, path: &Path, now: f64) -> Vec<Entry> {
        let k = key(path);
        if let Some((read, entries)) = self.history.cache.get(&k)
            && now - read < CACHE_SECONDS
        {
            return entries.clone();
        }
        let entries = self.history.store.as_ref().map(|h| h.entries(path)).unwrap_or_default();
        self.history.cache.insert(k, (now, entries.clone()));
        entries
    }

    /// The Timeline section of the Explorer: versions of the active file.
    pub(crate) fn timeline_view(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let now = ui.input(|i| i.time);
        let Some(path) = self.docs.get(self.active).and_then(|d| d.path.clone()) else {
            ui.label(RichText::new("Open a saved file to see its history.").color(pal.text_dim));
            return;
        };
        if !self.settings.history.enabled {
            ui.label(RichText::new("Local history is off (history.enabled).").color(pal.text_dim));
            return;
        }
        let entries = self.history_entries(&path, now);
        if entries.is_empty() {
            ui.label(RichText::new("No saved versions yet.").color(pal.text_dim));
            return;
        }
        let mut compare = None;
        let mut restore = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for e in &entries {
                ui.horizontal(|ui| {
                    let resp = ui
                        .add(egui::Label::new(describe(e)).sense(Sense::click()).truncate())
                        .on_hover_text(format!("{} bytes\nClick to compare with the current text", e.size));
                    if resp.clicked() {
                        compare = Some(e.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if icons::button(ui, Icon::Discard, "Restore this version").clicked() {
                            restore = Some(e.clone());
                        }
                    });
                });
            }
        });
        if let Some(e) = compare {
            self.compare_with_history(&path, &e);
        }
        if let Some(e) = restore {
            self.restore_history(&path, &e);
        }
        // Relative times change: refresh now and then.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
    }

    /// Local History: Find Entry to Restore… — versions of the active file.
    pub(crate) fn history_picker(&mut self, ctx: &egui::Context) {
        let Some(path) = self.docs.get(self.active).and_then(|d| d.path.clone()) else {
            self.status = Some("Open a saved file first".to_owned());
            return;
        };
        let entries = self.history_entries(&path, ctx.input(|i| i.time));
        if entries.is_empty() {
            self.status = Some("No local history for this file yet".to_owned());
            return;
        }
        self.history.pick = entries
            .into_iter()
            .enumerate()
            .map(|(n, e)| (format!("{}  ·  #{}", describe(&e), n + 1), path.clone(), e))
            .collect();
        let labels = self.history.pick.iter().map(|p| p.0.clone()).collect();
        self.palette.show_picker(PickerKind::History, labels, "", "Select a version to compare with");
    }

    pub(crate) fn picked_history(&mut self, label: &str) {
        if let Some((_, path, entry)) = self.history.pick.iter().find(|p| p.0 == label).cloned() {
            self.compare_with_history(&path, &entry);
        }
    }

    /// Opens `path` (if needed) and returns its document index.
    fn ensure_open(&mut self, path: &Path) -> Option<usize> {
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref().is_some_and(|p| same_path(p, path))) {
            return Some(i);
        }
        let doc = Document::open(path).ok()?;
        self.watcher.watch_file(path);
        self.add_doc(doc);
        Some(self.docs.len() - 1)
    }

    /// Side-by-side diff: the version on the left, the current text on the right.
    fn compare_with_history(&mut self, path: &Path, entry: &Entry) {
        let Some(old) = self.history.store.as_ref().and_then(|h| h.content(path, entry).ok()) else {
            self.status = Some("That version could not be read".to_owned());
            return;
        };
        let Some(i) = self.ensure_open(path) else { return };
        self.active = i;
        let name = self
            .explorer
            .root
            .as_ref()
            .and_then(|r| path.strip_prefix(r).ok())
            .map_or_else(|| path.display().to_string(), |p| p.display().to_string());
        self.show_history_diff(path.to_path_buf(), name, old, entry.clone(), i);
    }

    /// Replaces the file's text with a version (unsaved; Ctrl+Z undoes it).
    pub(crate) fn restore_history(&mut self, path: &Path, entry: &Entry) {
        let Some(text) = self.history.store.as_ref().and_then(|h| h.content(path, entry).ok()) else {
            self.status = Some("That version could not be read".to_owned());
            return;
        };
        let Some(i) = self.ensure_open(path) else { return };
        // Keep what is being replaced, so the restore itself can be undone later.
        self.history_record(i, Source::BeforeRestore);
        let doc = &mut self.docs[i];
        let head = doc.buffer.primary().head;
        let text = text.replace("\r\n", "\n").replace('\n', doc.buffer.line_ending);
        doc.buffer.replace_all(&text, Selection::cursor(head.min(text.chars().count())));
        doc.reveal = true;
        self.active = i;
        self.git.diff = None;
        self.status = Some(format!("Restored the version from {} (not saved yet)", describe(entry)));
    }
}
