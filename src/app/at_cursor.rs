//! A panel that follows the cursor: goals in a proof, the type at point elsewhere,
//! and problems. It can be paused, and a place can be pinned to keep watching it.
//! Nothing is asked of a server while the panel isn't showing.

use eframe::egui::{self, Color32, RichText};
use rust_code_core::goals::{self, Prover};
use serde_json::Value;

use super::lsp::{Pending, hover_markdown};
use super::{CodeApp, PanelTab};
use crate::document::Document;
use crate::{lsp_ui, theme};

/// How long the cursor rests before asking.
const REST: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Part {
    Goals,
    ExpectedType,
    Hover,
}

impl Part {
    fn title(self) -> &'static str {
        match self {
            Self::Goals => "GOALS",
            Self::ExpectedType => "EXPECTED TYPE",
            Self::Hover => "AT POINT",
        }
    }
}

#[derive(Clone)]
struct Section {
    part: Part,
    text: String,
    /// Per line, whether it's new since the last answer.
    changed: Vec<bool>,
    /// Lines the last answer had and this one doesn't.
    removed: Vec<String>,
}

/// What's known about one place.
#[derive(Clone)]
struct View {
    uid: u64,
    /// To pick a pin up again if its file is closed and reopened.
    path: Option<std::path::PathBuf>,
    pos: usize,
    version: u64,
    label: String,
    /// The line it's on, to find it again once lines above it change.
    line: usize,
    line_text: String,
    column: usize,
    sections: Vec<Section>,
    /// Answers to anything older than this are dropped.
    ask: u64,
    waiting: usize,
    /// Same place as the last ask: on an error, what's shown still applies.
    same_place: bool,
    has_server: bool,
}

#[derive(Default)]
pub(super) struct AtCursor {
    used: bool,
    /// Beside the code, rather than in the panel.
    beside: bool,
    /// Closed by hand, so a proof doesn't open it again.
    closed: bool,
    paused: bool,
    current: Option<View>,
    pins: Vec<(u64, View)>,
    next: u64,
    /// The place the cursor came to, and when.
    rest: Option<((u64, usize, u64), f64)>,
    /// When pins were last asked about.
    pins_asked: f64,
    /// A server was working last time: what it said then may have been too early.
    server_busy: bool,
}

fn place_label(doc: &Document, pos: usize) -> String {
    let rope = doc.buffer.rope();
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let name = doc.path.as_ref().and_then(|p| p.file_name()).map_or("untitled".into(), |n| n.to_string_lossy());
    format!("{name}:{}:{}", line + 1, pos - rope.line_to_char(line) + 1)
}

/// The line nearest to `line` that has this text.
fn find_line(doc: &Document, line: usize, text: &str) -> Option<usize> {
    let count = doc.buffer.rope().len_lines();
    (0..count).filter(|l| doc.buffer.line_text(*l) == text).min_by_key(|l| l.abs_diff(line))
}

impl CodeApp {
    pub(super) fn at_cursor_tab_shown(&self) -> bool {
        self.at_cursor.used && !self.at_cursor.beside
    }

    /// `View: Toggle Cursor Info Beside Code`.
    pub(super) fn at_cursor_toggle_beside(&mut self) {
        let showing = self.at_cursor.used && self.at_cursor.beside;
        self.at_cursor.used = true;
        self.at_cursor.closed = false;
        self.at_cursor.beside = !showing;
        if self.at_cursor.beside {
            if self.panel_tab == PanelTab::AtCursor {
                self.panel_tab = PanelTab::Terminal;
                self.panel_visible = false;
            }
        } else {
            self.show_at_cursor();
        }
    }

    /// Draws it beside the code, where a proof's goals are easiest to read.
    pub(super) fn at_cursor_side(&mut self, ui: &mut egui::Ui) {
        if !self.at_cursor.used || !self.at_cursor.beside {
            return;
        }
        let half = ui.available_width() / 2.0;
        egui::Panel::right("at_cursor_side")
            .resizable(true)
            .default_size((half * 0.8).max(240.0))
            .size_range(200.0..=(half * 2.0 - 200.0).max(200.0))
            .frame(egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(8.0))
            .show(ui, |ui| self.at_cursor_panel(ui));
    }

    /// `View: Show Cursor Info`.
    pub(super) fn show_at_cursor(&mut self) {
        self.at_cursor.used = true;
        self.at_cursor.closed = false;
        if !self.at_cursor.beside {
            self.panel_tab = PanelTab::AtCursor;
            self.panel_visible = true;
        }
    }

    pub(super) fn at_cursor_toggle_pause(&mut self) {
        self.show_at_cursor();
        self.at_cursor.paused = !self.at_cursor.paused;
        self.at_cursor.rest = None;
    }

    /// Keeps watching the place shown now, as well as following the cursor.
    pub(super) fn at_cursor_pin(&mut self) {
        self.show_at_cursor();
        let Some(mut view) = self.at_cursor.current.clone() else { return };
        // Answers on their way are for the view it was copied from.
        view.waiting = 0;
        let state = &mut self.at_cursor;
        if state.pins.iter().all(|(_, p)| p.uid != view.uid || p.pos != view.pos) {
            state.next += 1;
            state.pins.push((state.next, view));
        }
    }

    pub(super) fn pump_at_cursor(&mut self, ctx: &egui::Context) {
        // In a proof, what's at the cursor is the point: it opens the first time.
        if !self.at_cursor.used
            && !self.at_cursor.closed
            && self.docs.get(self.active).is_some_and(|d| Prover::for_extension(&d.extension()).is_some())
        {
            self.at_cursor.beside = true;
            self.show_at_cursor();
        }
        let shown = self.at_cursor.used && self.at_cursor.beside;
        if !shown && (!self.panel_visible || self.panel_tab != PanelTab::AtCursor) {
            return;
        }
        let now = ctx.input(|i| i.time);
        let busy = !self.lsp.progress.is_empty();
        if std::mem::replace(&mut self.at_cursor.server_busy, busy) && !busy {
            let state = &mut self.at_cursor;
            let views = state.current.iter_mut().chain(state.pins.iter_mut().map(|(_, v)| v));
            // No version is this old, so each is asked about again.
            views.for_each(|v| v.version = u64::MAX);
        }
        let wanted = self.docs.get(self.active).map(|d| (d.uid, d.buffer.primary().head, d.buffer.version()));
        let shown = self.at_cursor.current.as_ref().map(|v| (v.uid, v.pos, v.version));
        if let Some(wanted) = wanted.filter(|w| !self.at_cursor.paused && Some(*w) != shown) {
            let since = match self.at_cursor.rest {
                Some((place, since)) if place == wanted => since,
                _ => {
                    self.at_cursor.rest = Some((wanted, now));
                    now
                }
            };
            if now - since < REST {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REST));
            } else {
                self.at_cursor.rest = None;
                let (uid, pos, _) = wanted;
                self.at_cursor_ask(None, uid, pos);
            }
        }
        for at in 0..self.at_cursor.pins.len() {
            let (uid, path) = (self.at_cursor.pins[at].1.uid, self.at_cursor.pins[at].1.path.clone());
            let reopened = path.filter(|_| self.doc_index(uid).is_none()).and_then(|p| self.doc_index_by_path(&p));
            if let Some(i) = reopened {
                let view = &mut self.at_cursor.pins[at].1;
                view.uid = self.docs[i].uid;
                view.version = u64::MAX;
            }
        }
        // A pinned place's answers change with its file.
        let outdated: Vec<(u64, u64, usize)> = {
            let versions = |uid| self.doc_index(uid).map(|i| self.docs[i].buffer.version());
            let pins = self.at_cursor.pins.iter();
            pins.filter(|(_, v)| versions(v.uid).is_some_and(|version| version != v.version))
                .map(|(id, v)| (*id, v.uid, v.pos))
                .collect()
        };
        if !outdated.is_empty() {
            if now - self.at_cursor.pins_asked < REST {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REST));
            } else {
                self.at_cursor.pins_asked = now;
                for (id, uid, pos) in outdated {
                    self.at_cursor_ask(Some(id), uid, pos);
                }
            }
        }
    }

    fn at_cursor_view(&mut self, pin: Option<u64>) -> Option<&mut View> {
        let state = &mut self.at_cursor;
        match pin {
            None => state.current.as_mut(),
            Some(id) => state.pins.iter_mut().find(|(p, _)| *p == id).map(|(_, v)| v),
        }
    }

    fn at_cursor_ask(&mut self, pin: Option<u64>, uid: u64, pos: usize) {
        let Some(i) = self.doc_index(uid) else { return };
        let was = pin.and_then(|_| self.at_cursor_view(pin)).map(|v| (v.line, v.line_text.clone(), v.column));
        let doc = &self.docs[i];
        let rope = doc.buffer.rope();
        // A pin stays with its line, unless that line itself changed.
        let found = was.and_then(|(line, text, column)| Some(rope.line_to_char(find_line(doc, line, &text)?) + column));
        let pos = found.unwrap_or(pos).min(rope.len_chars());
        let (version, label, path) = (doc.buffer.version(), place_label(doc, pos), doc.path.clone());
        let line = rope.char_to_line(pos);
        let (line_text, column) = (doc.buffer.line_text(line), pos - rope.line_to_char(line));
        let prover = Prover::for_extension(&doc.extension());
        let has_server = self.has_ready_server(i);
        self.at_cursor.next += 1;
        let ask = self.at_cursor.next;
        let mut parts = Vec::new();
        if has_server {
            if let Some(prover) = prover {
                parts.push((Part::Goals, prover.goals_method().to_owned()));
                if let Some(method) = prover.expected_type_method() {
                    parts.push((Part::ExpectedType, method.to_owned()));
                }
            }
            parts.push((Part::Hover, "textDocument/hover".to_owned()));
        }
        let waiting = parts.len();
        let old = self.at_cursor_view(pin).filter(|v| v.uid == uid);
        let same_place = old.as_ref().is_some_and(|v| v.pos == pos);
        let sections = old.map(|v| std::mem::take(&mut v.sections)).unwrap_or_default();
        let view = View {
            uid,
            path,
            pos,
            version,
            label,
            line,
            line_text,
            column,
            sections,
            ask,
            waiting,
            same_place,
            has_server,
        };
        match pin {
            None => self.at_cursor.current = Some(view),
            Some(_) => *self.at_cursor_view(pin).expect("asked about an existing pin") = view,
        }
        for (part, method) in parts {
            let pending = Pending::AtCursor { pin, ask, part, prover };
            self.request_for(i, &method, pending, |doc, td| Self::position_params(doc, td, pos));
        }
    }

    pub(super) fn on_at_cursor(
        &mut self,
        pin: Option<u64>,
        ask: u64,
        part: Part,
        prover: Option<Prover>,
        result: Result<Value, String>,
    ) {
        let Some(view) = self.at_cursor_view(pin).filter(|v| v.ask == ask) else { return };
        view.waiting = view.waiting.saturating_sub(1);
        let at = view.sections.iter().position(|s| s.part == part);
        let value = match result {
            Ok(value) => value,
            // Routine while a file is changing: what's shown stays if it still applies.
            Err(_) => {
                if !view.same_place
                    && let Some(at) = at
                {
                    view.sections.remove(at);
                }
                return;
            }
        };
        let text = match (part, prover) {
            (Part::Goals, Some(prover)) => prover.goals_text(&value),
            (Part::ExpectedType, _) => goals::lean_expected_type(&value),
            _ => hover_markdown(value),
        };
        if text.trim().is_empty() {
            if let Some(at) = at {
                view.sections.remove(at);
            }
            return;
        }
        let changed = match (part, at) {
            (Part::Hover, _) | (_, None) => vec![false; text.lines().count()],
            (_, Some(at)) => goals::changed_lines(&view.sections[at].text, &text),
        };
        let removed = match (part, at) {
            (Part::Hover, _) | (_, None) => Vec::new(),
            (_, Some(at)) => goals::removed_lines(&view.sections[at].text, &text),
        };
        let section = Section { part, text, changed, removed };
        match at {
            Some(at) => view.sections[at] = section,
            None => view.sections.push(section),
        }
        view.sections.sort_by_key(|s| s.part);
    }

    fn at_cursor_view_ui(ui: &mut egui::Ui, view: &View, problems: &[(u8, String)], size: f32) {
        let pal = theme::colors();
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        for (severity, message) in problems {
            let color = match severity {
                1 => Color32::from_rgb(0xF1, 0x4C, 0x4C),
                2 => Color32::from_rgb(0xCC, 0xA7, 0x00),
                _ => pal.accent,
            };
            ui.add(egui::Label::new(RichText::new(message).color(color)).wrap());
        }
        for section in &view.sections {
            ui.label(RichText::new(section.part.title()).small().strong().color(pal.text_dim));
            if section.part == Part::Hover {
                lsp_ui::markdown(ui, &section.text, size);
                continue;
            }
            for (at, line) in section.text.lines().enumerate() {
                let changed = section.changed.get(at).copied().unwrap_or(false);
                let color = if changed { Color32::from_rgb(0x89, 0xD1, 0x85) } else { pal.editor_fg };
                ui.add(egui::Label::new(RichText::new(line).font(mono.clone()).color(color)).wrap());
            }
            for line in &section.removed {
                let gone = RichText::new(line).font(mono.clone()).color(pal.text_dim).strikethrough();
                ui.add(egui::Label::new(gone).wrap());
            }
        }
        if view.sections.is_empty() && problems.is_empty() && view.waiting == 0 {
            let note =
                if view.has_server { "Nothing known here." } else { "No language server here: only problems show." };
            ui.label(RichText::new(note).color(pal.text_dim));
        }
    }

    pub(super) fn at_cursor_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let size = self.settings.workbench.ui_font_size;
        let problems = |app: &Self, view: &View| app.problems_at(view.uid, view.pos);
        let current = self.at_cursor.current.as_ref().map(|v| problems(self, v)).unwrap_or_default();
        let pinned: Vec<_> = self.at_cursor.pins.iter().map(|(_, v)| problems(self, v)).collect();
        let open: Vec<bool> = self.at_cursor.pins.iter().map(|(_, v)| self.doc_index(v.uid).is_some()).collect();
        let state = &self.at_cursor;
        let (mut pause, mut pin, mut unpin, mut go) = (false, false, None, None);
        let (mut move_it, mut close) = (false, false);
        ui.horizontal(|ui| {
            let label = state.current.as_ref().map_or("", |v| v.label.as_str());
            ui.label(RichText::new(label).strong());
            if state.paused {
                ui.label(RichText::new("Paused").color(Color32::from_rgb(0xCC, 0xA7, 0x00)));
            }
            if state.current.as_ref().is_some_and(|v| v.waiting > 0) {
                ui.label(RichText::new("…").color(pal.text_dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.beside {
                    close = ui.button("Close").clicked();
                }
                move_it = ui.button(if state.beside { "To Panel" } else { "Beside Code" }).clicked();
                pause = ui.button(if state.paused { "Resume" } else { "Pause" }).clicked();
                pin = ui.add_enabled(state.current.is_some(), egui::Button::new("Pin")).clicked();
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().id_salt("at_cursor").auto_shrink([false, false]).show(ui, |ui| {
            match &state.current {
                Some(view) => Self::at_cursor_view_ui(ui, view, &current, size),
                None => {
                    ui.label(RichText::new("Open a file: this follows its cursor.").color(pal.text_dim));
                }
            }
            for (at, (id, view)) in state.pins.iter().enumerate() {
                ui.separator();
                ui.horizontal(|ui| {
                    let title = RichText::new(format!("Pinned: {}", view.label)).strong();
                    let link = egui::Label::new(title).sense(egui::Sense::click()).selectable(false);
                    if ui.add(link).on_hover_text("Go there").clicked() {
                        go = Some((view.uid, view.pos));
                    }
                    if !open[at] {
                        ui.label(RichText::new("(file closed)").color(pal.text_dim));
                    } else if view.waiting > 0 {
                        ui.label(RichText::new("…").color(pal.text_dim));
                    }
                    if ui.small_button("×").on_hover_text("Unpin").clicked() {
                        unpin = Some(*id);
                    }
                });
                Self::at_cursor_view_ui(ui, view, &pinned[at], size);
            }
        });
        if let Some((uid, pos)) = go
            && let Some(i) = self.doc_index(uid)
        {
            self.active = i;
            let doc = &mut self.docs[i];
            let pos = pos.min(doc.buffer.rope().len_chars());
            doc.buffer.set_selections(vec![rust_code_core::buffer::Selection::new(pos, pos)], 0);
            doc.reveal = true;
            doc.request_focus = true;
        }
        if move_it {
            self.at_cursor_toggle_beside();
        }
        if close {
            self.at_cursor.used = false;
            self.at_cursor.closed = true;
        }
        if pause {
            self.at_cursor_toggle_pause();
        }
        if pin {
            self.at_cursor_pin();
        }
        // These are used while editing, so the keyboard goes back there.
        if (pause || pin)
            && let Some(doc) = self.docs.get_mut(self.active)
        {
            doc.request_focus = true;
        }
        if let Some(id) = unpin {
            self.at_cursor.pins.retain(|(p, _)| *p != id);
        }
    }
}
