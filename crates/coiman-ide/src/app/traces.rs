//! Stack Trace: Analyze. Paste a trace from anywhere, and its frames in this
//! project become links.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use coiman_core::language::lsp::types as lt;
use coiman_core::launch::stacktrace::{self, Analysis, Frame, Place};
use eframe::egui::{self, RichText, Sense};

use super::{CodeApp, PanelTab, repaint};
use crate::theme;

#[derive(Default)]
pub(super) struct Traces {
    /// The panel has been asked for: its tab stays until the app closes.
    pub(super) used: bool,
    text: String,
    analysis: Option<Analysis>,
    /// The text the analysis in hand or on its way is of.
    analyzed: String,
    working: Option<Receiver<Analysis>>,
    focus: bool,
    only_project: bool,
    /// The box stays open though a trace was read.
    editing: bool,
    paste: bool,
}

/// What the row's frame jumps to, with a word about it.
fn target(frame: &Frame, place: &Place) -> Option<(PathBuf, String)> {
    let at = |path: &PathBuf| format!("{}:{}", path.display(), frame.line);
    match place {
        Place::Project(path) => Some((path.clone(), at(path))),
        Place::Maybe(path, others) => {
            let more = if *others > 0 { format!(", and {others} more with this name") } else { String::new() };
            Some((path.clone(), format!("Only the name matches{more}, so this may be another file: {}", at(path))))
        }
        Place::Elsewhere => None,
    }
}

impl CodeApp {
    /// Opens the panel, starting from the selected text if there is any.
    pub(super) fn analyze_stack_trace(&mut self) {
        if let Some(doc) = self.docs.get(self.active) {
            let p = doc.buffer.primary();
            if !p.is_empty() {
                self.traces.text = doc.buffer.slice_string(p.start(), p.end());
            }
        }
        self.traces.used = true;
        self.traces.focus = self.traces.text.is_empty();
        self.panel_tab = PanelTab::StackTrace;
        self.panel_visible = true;
    }

    /// Opens the panel on this text.
    pub(super) fn analyze_stack_trace_text(&mut self, text: String) {
        self.traces.text = text;
        self.traces.used = true;
        self.panel_tab = PanelTab::StackTrace;
        self.panel_visible = true;
    }

    fn pump_traces(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.traces.working
            && let Ok(done) = rx.try_recv()
        {
            self.traces.analysis = Some(done);
            self.traces.working = None;
        }
        if self.traces.working.is_some() || self.traces.text == self.traces.analyzed {
            return;
        }
        self.traces.analyzed = self.traces.text.clone();
        let text = self.traces.text.clone();
        let root = self.explorer.root.clone().unwrap_or_default();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let files = if root.as_os_str().is_empty() { Vec::new() } else { stacktrace::project_files(&root) };
            let _ = tx.send(stacktrace::analyze(&text, &root, &files));
            wake();
        });
        self.traces.working = Some(rx);
    }

    pub(super) fn stack_trace_panel(&mut self, ui: &mut egui::Ui) {
        self.pump_traces(ui.ctx());
        let pal = theme::colors();
        ui.horizontal(|ui| {
            match &self.traces.analysis {
                Some(a) if !self.traces.text.trim().is_empty() => {
                    let plural = if a.frames() == 1 { "" } else { "s" };
                    let mut said = format!("{} frame{plural}, {} in this project", a.frames(), a.in_project());
                    if a.repaired > 0 {
                        said.push_str(&format!(". Repaired {} wrapped or flattened", a.repaired));
                    }
                    ui.label(said);
                }
                _ => {
                    ui.label(RichText::new("Paste a stack trace below, or a log around one").color(pal.text_dim));
                }
            }
            ui.checkbox(&mut self.traces.only_project, "Only this project");
            if ui.button("Clear").clicked() {
                self.traces.text.clear();
                self.traces.focus = true;
                self.traces.editing = false;
            }
            if ui.button("Paste").on_hover_text("Replace the text with what's on the clipboard").clicked() {
                self.traces.text.clear();
                self.traces.editing = true;
                self.traces.focus = true;
                self.traces.paste = true;
            }
            let read = self.traces.analysis.as_ref().is_some_and(|a| a.frames() > 0);
            if read {
                ui.toggle_value(&mut self.traces.editing, "Edit Text");
            }
        });
        // With a trace read, its frames need the room more than the box does.
        let read = self.traces.analysis.as_ref().is_some_and(|a| a.frames() > 0);
        if !read || self.traces.editing {
            let input = egui::TextEdit::multiline(&mut self.traces.text)
                .hint_text("Stack trace")
                .code_editor()
                .desired_rows(3)
                .desired_width(f32::INFINITY);
            let box_area = egui::ScrollArea::vertical().id_salt("trace_input").max_height(70.0);
            let response = box_area.show(ui, |ui| ui.add(input)).inner;
            if std::mem::take(&mut self.traces.focus) {
                response.request_focus();
            }
            // The box has the keyboard by now, so the paste lands in it.
            if response.has_focus() && std::mem::take(&mut self.traces.paste) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            } else if self.traces.paste {
                ui.ctx().request_repaint();
            }
        }
        ui.separator();
        let mut jump = None;
        let only_project = self.traces.only_project;
        egui::ScrollArea::both().id_salt("trace_rows").auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let Some(analysis) = &self.traces.analysis else { return };
            for row in &analysis.rows {
                let found = row.frame.as_ref().and_then(|(f, place)| target(f, place).map(|t| (f, place, t)));
                let text = RichText::new(row.text.trim_end()).monospace();
                match found {
                    Some((frame, place, (path, tip))) => {
                        let color = if matches!(place, Place::Project(_)) { pal.accent } else { pal.text_dim };
                        let label = egui::Label::new(text.color(color).underline()).extend().sense(Sense::click());
                        let response = ui.add(label).on_hover_text(tip);
                        if response.clicked() {
                            jump = Some((path, frame.line, frame.column));
                        }
                    }
                    None if only_project => {}
                    None if row.frame.is_some() => {
                        ui.add(egui::Label::new(text.color(pal.text_dim)).extend());
                    }
                    None => {
                        ui.add(egui::Label::new(text).extend());
                    }
                }
            }
        });
        if let Some((path, line, column)) = jump {
            let at = lt::Position::new(line.saturating_sub(1), column.unwrap_or(1).saturating_sub(1));
            self.jump_to(&path, lt::Range::new(at, at), false);
        }
    }
}
