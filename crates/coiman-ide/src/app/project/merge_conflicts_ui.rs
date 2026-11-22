//! The Merge Conflicts window, where each conflict of a file is settled by taking one side, the
//! other, both, or a result written by hand, and the merge, cherry-pick, revert or rebase in
//! progress goes on or is given up. (The commit graph is the Git Graph extension.)

use std::sync::mpsc::{self, Receiver};

use coiman_core::project::git_graph::{self, Choice, Operation};
use egui::{Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Default)]
pub(crate) struct MergeConflictsUi {
    pub(crate) open: bool,
    /// Read the operation in progress again: something may have changed it.
    pub(crate) stale: bool,
    loading: Option<Receiver<Option<Operation>>>,
    op: Option<Operation>,
    conflict_file: Option<String>,
    /// Results written by hand for conflicts: by (file, conflict).
    results: std::collections::HashMap<(String, usize), String>,
}

/// Going on with the operation in progress, or giving it up.
enum OpAsk {
    Continue(Operation),
    Abort(Operation),
}

impl CodeApp {
    pub(crate) fn show_merge_conflicts(&mut self) {
        self.merge_conflicts.open = true;
        self.merge_conflicts.stale = true;
    }

    pub(crate) fn merge_conflicts_window(&mut self, ctx: &egui::Context) {
        if !self.merge_conflicts.open {
            return;
        }
        let m = &mut self.merge_conflicts;
        if let Some(op) = m.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            m.op = op;
            m.loading = None;
        }
        if m.stale && m.loading.is_none() {
            m.stale = false;
            if let Some(repo) = self.git_repo().cloned() {
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                std::thread::spawn(move || {
                    let _ = tx.send(repo.operation());
                    wake();
                });
                self.merge_conflicts.loading = Some(rx);
            }
        }
        let pal = theme::colors();
        let files = self.git_conflicts();
        let root = self.git_repo().cloned().map(|r| r.root);
        let mut open = true;
        let mut choose: Option<(String, usize, Choice)> = None;
        let mut custom: Option<(String, usize, Vec<String>)> = None;
        let mut resolved: Option<String> = None;
        let mut open_file: Option<std::path::PathBuf> = None;
        let mut op_ask = None;
        let op = self.merge_conflicts.op;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.8).min(1100.0), (room.y * 0.78).max(400.0));
        let g = &mut self.merge_conflicts;
        if g.conflict_file.as_ref().is_none_or(|f| !files.iter().any(|c| &c.rel == f)) {
            g.conflict_file = files.first().map(|f| f.rel.clone());
        }
        egui::Window::new(RichText::new("Merge Conflicts").size(16.0).strong())
            .id(egui::Id::unique("merge_conflicts_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                if let Some(op) = op {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("A {} is in progress.", op.name())).strong());
                        if ui
                            .add_enabled(files.is_empty(), egui::Button::new("Continue"))
                            .on_disabled_hover_text("Resolve every file first")
                            .clicked()
                        {
                            op_ask = Some(OpAsk::Continue(op));
                        }
                        if ui.button("Abort").on_hover_text(format!("Give the {} up", op.name())).clicked() {
                            op_ask = Some(OpAsk::Abort(op));
                        }
                    });
                    ui.separator();
                }
                if files.is_empty() {
                    ui.label(RichText::new("No conflicts.").color(pal.text_dim));
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for f in &files {
                        let on = g.conflict_file.as_deref() == Some(f.rel.as_str());
                        if ui.selectable_label(on, &f.rel).clicked() {
                            g.conflict_file = Some(f.rel.clone());
                        }
                    }
                });
                ui.separator();
                let (Some(rel), Some(root)) = (g.conflict_file.clone(), root.as_ref()) else { return };
                let path = root.join(&rel);
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let found = git_graph::conflicts(&text);
                ui.horizontal(|ui| {
                    let left = match found.len() {
                        1 => "1 conflict left".to_owned(),
                        n => format!("{n} conflicts left"),
                    };
                    ui.label(RichText::new(format!("{rel}: {left}")).strong());
                    if ui.button("Open in Editor").clicked() {
                        open_file = Some(path.clone());
                    }
                    if ui
                        .add_enabled(found.is_empty(), egui::Button::new("Mark as Resolved"))
                        .on_hover_text("Stage the file as it is now")
                        .on_disabled_hover_text("Settle each conflict first, or edit the file")
                        .clicked()
                    {
                        resolved = Some(rel.clone());
                    }
                });
                let mono = egui::FontId::monospace(12.5);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, c) in found.iter().enumerate() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(format!("Conflict {} at line {}", i + 1, c.start + 1))
                                .small()
                                .color(pal.text_dim),
                        );
                        ui.columns(2, |cols| {
                            for (col, title, lines, color) in [
                                (
                                    0,
                                    format!("Current ({})", c.ours_label),
                                    &c.ours,
                                    Color32::from_rgb(0x57, 0xC0, 0x6A),
                                ),
                                (1, format!("Incoming ({})", c.theirs_label), &c.theirs, pal.accent),
                            ] {
                                let ui = &mut cols[col];
                                egui::Frame::new()
                                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
                                    .corner_radius(6.0)
                                    .inner_margin(6.0)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.label(RichText::new(title).small().color(color));
                                        let body =
                                            if lines.is_empty() { "(nothing)".to_owned() } else { lines.join("\n") };
                                        ui.add(egui::Label::new(RichText::new(body).font(mono.clone())).wrap());
                                    });
                            }
                        });
                        ui.horizontal(|ui| {
                            for (label, choice) in [
                                ("Accept Current", Choice::Ours),
                                ("Accept Incoming", Choice::Theirs),
                                ("Accept Both", Choice::Both),
                                ("Both, Incoming First", Choice::BothTheirsFirst),
                            ] {
                                if ui.button(label).clicked() {
                                    choose = Some((rel.clone(), i, choice));
                                }
                            }
                        });
                        // The result, to write by hand: starts as both sides, current first.
                        let key = (rel.clone(), i);
                        let result = g
                            .results
                            .entry(key.clone())
                            .or_insert_with(|| git_graph::chosen(c, Choice::Both).join("\n"));
                        ui.label(RichText::new("Result (edit it, then Use This Result)").small().color(pal.text_dim));
                        ui.add(
                            egui::TextEdit::multiline(result)
                                .font(mono.clone())
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        );
                        if ui.button("Use This Result").clicked() {
                            custom = Some((rel.clone(), i, result.lines().map(str::to_owned).collect()));
                        }
                        if let Some(base) = &c.base {
                            ui.label(
                                RichText::new(format!("Before both: {}", base.join(" ⏎ "))).small().color(pal.text_dim),
                            );
                        }
                    }
                });
            });
        self.merge_conflicts.open = open;
        if let Some((rel, index, choice)) = choose
            && let Some(root) = &root
        {
            let path = root.join(&rel);
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some(new) = git_graph::resolve(&text, index, choice) {
                match std::fs::write(&path, new) {
                    Ok(()) => self.reload_open_file(&path),
                    Err(e) => self.status = Some(format!("Couldn't write {rel}: {e}")),
                }
            }
        }
        if let Some((rel, index, lines)) = custom
            && let Some(root) = &root
        {
            let path = root.join(&rel);
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Some(new) = git_graph::resolve_with(&text, index, &lines) {
                match std::fs::write(&path, new) {
                    Ok(()) => self.reload_open_file(&path),
                    Err(e) => self.status = Some(format!("Couldn't write {rel}: {e}")),
                }
            }
            // The rest move up a place: their results are written afresh.
            self.merge_conflicts.results.retain(|(r, _), _| r != &rel);
        }
        if let Some(rel) = resolved {
            self.git_run(
                ctx,
                format!("Marking {rel} resolved…"),
                Box::new(move |repo| repo.mark_resolved(&rel).map(|()| format!("{rel} resolved"))),
            );
        }
        if let Some(path) = open_file {
            self.open_file(&path);
        }
        match op_ask {
            Some(OpAsk::Continue(op)) => self.git_run(
                ctx,
                format!("Continuing the {}…", op.name()),
                Box::new(move |repo| repo.continue_operation(op).map(|()| format!("The {} went on", op.name()))),
            ),
            Some(OpAsk::Abort(op)) => self.git_run(
                ctx,
                format!("Stopping the {}…", op.name()),
                Box::new(move |repo| repo.abort_operation(op).map(|()| format!("The {} was given up", op.name()))),
            ),
            None => {}
        }
    }

    /// A file changed on disk here: an open copy without unsaved edits shows the new text.
    fn reload_open_file(&mut self, path: &std::path::Path) {
        if let Some(doc) = self
            .docs
            .iter_mut()
            .find(|d| d.path.as_deref().is_some_and(|p| coiman_core::project::watcher::same_path(p, path)))
            && !doc.is_dirty()
        {
            let _ = doc.reload_from_disk();
        }
    }
}
