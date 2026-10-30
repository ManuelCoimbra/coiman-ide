//! Warns about characters that change how code reads without showing up.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use rust_code_core::unicode::{self, Kind};

use super::CodeApp;
use crate::buffer::Selection;
use crate::editor_view::{Decorations, Highlight};
use crate::theme;

const RED: Color32 = Color32::from_rgb(0xC5, 0x30, 0x30);

/// What a scan of one version of a document found.
struct Scan {
    version: u64,
    /// (char position, character, kind), in order.
    found: Vec<(usize, char, Kind)>,
    marks: Arc<HashMap<usize, Vec<Highlight>>>,
    cols: Arc<HashMap<usize, Vec<usize>>>,
}

#[derive(Default)]
pub(super) struct HiddenChars {
    scans: HashMap<u64, Scan>,
    /// Documents whose warning was dismissed; the marks stay.
    dismissed: HashSet<u64>,
}

fn scan(rope: &ropey::Rope, version: u64) -> Scan {
    let mut found = Vec::new();
    let mut marks: HashMap<usize, Vec<Highlight>> = HashMap::new();
    let mut cols: HashMap<usize, Vec<usize>> = HashMap::new();
    for (line, slice) in rope.lines().enumerate() {
        // Plain ASCII, the usual case, has nothing to find.
        if slice.len_bytes() == slice.len_chars() {
            continue;
        }
        let start = rope.line_to_char(line);
        for (col, c, kind) in unicode::scan_line(&slice.to_string()) {
            // A byte order mark at the very start is ordinary.
            if c == '\u{FEFF}' && start + col == 0 {
                continue;
            }
            found.push((start + col, c, kind));
            marks.entry(line).or_default().push(Highlight {
                start: col,
                end: col + 1,
                foreground: Some(Color32::WHITE),
                background: Some(RED),
                italic: false,
            });
            cols.entry(line).or_default().push(col);
        }
    }
    Scan { version, found, marks: Arc::new(marks), cols: Arc::new(cols) }
}

impl CodeApp {
    /// Marks the suspicious characters of document `i`, scanning it if it changed.
    pub(super) fn hidden_decorate(&mut self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i).filter(|d| !d.large && self.settings.editor.unicode_warnings) else { return };
        let version = doc.buffer.version();
        let scans = &mut self.hidden.scans;
        if scans.get(&doc.uid).is_none_or(|s| s.version != version) {
            scans.insert(doc.uid, scan(doc.buffer.rope(), version));
        }
        let scan = &scans[&doc.uid];
        if !scan.found.is_empty() {
            deco.highlights.push(scan.marks.clone());
            deco.hidden = Some(scan.cols.clone());
        }
    }

    pub(super) fn hidden_forget(&mut self, uid: u64) {
        self.hidden.scans.remove(&uid);
        self.hidden.dismissed.remove(&uid);
    }

    /// The warning above the active editor.
    pub(super) fn hidden_bar(&mut self, ui: &mut egui::Ui) {
        let Some(uid) = self.docs.get(self.active).map(|d| d.uid) else { return };
        let count = self.hidden.scans.get(&uid).map_or(0, |s| s.found.len());
        if count == 0 || self.hidden.dismissed.contains(&uid) || !self.settings.editor.unicode_warnings {
            return;
        }
        let (mut next, mut dismiss) = (false, false);
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal(|ui| {
                let what = if count == 1 { "1 hidden or unusual character" } else { "hidden or unusual characters" };
                let message = if count == 1 {
                    format!("This file has {what} that can change how the code reads.")
                } else {
                    format!("This file has {count} {what} that can change how the code reads.")
                };
                ui.label(RichText::new(message).color(theme::colors().modified));
                next = ui.button("Next").clicked();
                dismiss = ui.button("Dismiss").clicked();
            });
        });
        if next {
            self.next_hidden_char();
        }
        if dismiss {
            self.hidden.dismissed.insert(uid);
        }
    }

    /// Selects the next suspicious character after the cursor and names it.
    pub(super) fn next_hidden_char(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let found = self.hidden.scans.get(&doc.uid).map(|s| s.found.as_slice()).unwrap_or_default();
        let head = doc.buffer.primary().end();
        let Some(&(pos, c, kind)) = found.iter().find(|f| f.0 >= head).or(found.first()) else {
            self.status = Some("No hidden characters in this file".to_owned());
            return;
        };
        doc.buffer.set_single(Selection::new(pos, pos + 1));
        doc.reveal = true;
        doc.request_focus = true;
        self.status = Some(format!("U+{:04X}: {}", c as u32, kind.label()));
    }
}
