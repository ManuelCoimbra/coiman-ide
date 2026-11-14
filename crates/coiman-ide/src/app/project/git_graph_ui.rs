//! The Git Graph window: every branch's commits with lines for branches and merges, a
//! commit's files, and what's done from a commit (cherry-pick, revert, a branch, an
//! interactive rebase). Also the Merge Conflicts window, where each conflict of a file is
//! settled by taking one side, the other, or both.

use std::sync::mpsc::{self, Receiver};

use coiman_core::project::git;
use coiman_core::project::git_graph::{self, Action, Choice, GraphCommit, Operation, Row, Step};
use egui::{Color32, Pos2, RichText, Sense, Stroke, Vec2};

use crate::app::project::git::GitWork;
use crate::app::{CodeApp, repaint};
use crate::ui::theme;

/// Commits read for the graph.
const LIMIT: usize = 2000;
const ROW_H: f32 = 24.0;
const LANE_W: f32 = 14.0;

type Loaded = Result<(Vec<GraphCommit>, Option<Operation>), String>;
/// A commit's files: (kind, path).
type CommitFiles = Result<Vec<(char, String)>, String>;

#[derive(Default)]
pub(crate) struct GitGraphUi {
    pub(crate) open: bool,
    /// Read the graph again: something changed it.
    pub(crate) stale: bool,
    loading: Option<Receiver<Loaded>>,
    commits: Vec<GraphCommit>,
    rows: Vec<Row>,
    error: Option<String>,
    op: Option<Operation>,
    selected: Option<usize>,
    files: Option<(String, Receiver<CommitFiles>)>,
    shown_files: Vec<(char, String)>,
    /// A new branch's name, being typed for this commit.
    new_branch: Option<(String, String)>,
    rebase: Option<Rebase>,
    pub(crate) conflicts_open: bool,
    conflict_file: Option<String>,
}

/// An interactive rebase being planned: the commits after `base`, in their new order.
struct Rebase {
    base: String,
    base_label: String,
    steps: Vec<Step>,
    loading: Option<Receiver<Result<Vec<Step>, String>>>,
}

/// What the window asks the app to do.
enum Ask {
    Reload,
    Select(usize),
    OpenFile(String, String, String),
    CherryPick(String, String),
    Revert(String, String),
    Branch(String, String),
    PlanRebase(String, String),
    Rebase(String, Vec<Step>),
    Continue(Operation),
    Abort(Operation),
    ShowConflicts,
}

/// A color for a lane, from the theme's bracket colors and a few more.
fn lane_color(lane: usize) -> Color32 {
    let pal = theme::colors();
    let more = [
        pal.accent,
        Color32::from_rgb(0x57, 0xC0, 0x6A),
        Color32::from_rgb(0xE0, 0x9A, 0x3E),
        Color32::from_rgb(0xC7, 0x7D, 0xBB),
    ];
    let all: Vec<Color32> = more.into_iter().chain(pal.rainbow).collect();
    all[lane % all.len()]
}

impl CodeApp {
    pub(crate) fn show_git_graph(&mut self) {
        if self.git_repo().cloned().is_none() {
            self.status = Some("Open a folder in a Git repository first".to_owned());
            return;
        }
        self.git_graph.open = true;
        self.git_graph.stale = true;
    }

    pub(crate) fn show_merge_conflicts(&mut self) {
        self.git_graph.conflicts_open = true;
        // The operation in progress is read with the graph.
        self.git_graph.stale = true;
    }

    fn load_git_graph(&mut self, ctx: &egui::Context) {
        let Some(repo) = self.git_repo().cloned() else { return };
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(repo.graph_log(LIMIT).map(|c| (c, repo.operation())));
            wake();
        });
        self.git_graph.loading = Some(rx);
        self.git_graph.stale = false;
    }

    pub(crate) fn git_graph_window(&mut self, ctx: &egui::Context) {
        if self.git_graph.conflicts_open {
            self.merge_conflicts_window(ctx);
        }
        if self.git_graph.stale && (self.git_graph.open || self.git_graph.conflicts_open) {
            self.load_git_graph(ctx);
        }
        if !self.git_graph.open {
            self.take_git_graph();
            return;
        }
        self.take_git_graph();
        let mut open = true;
        let mut ask = None;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.86).min(1300.0), (room.y * 0.82).max(400.0));
        egui::Window::new(RichText::new("Git Graph").size(16.0).strong())
            .id(egui::Id::unique("git_graph_window"))
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
            .show(ctx, |ui| ask = graph_ui(ui, &mut self.git_graph));
        self.git_graph.open = open;
        self.git_graph_ask(ctx, ask);
    }

    /// Takes in what the background reads have brought.
    fn take_git_graph(&mut self) {
        let g = &mut self.git_graph;
        if let Some(r) = g.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            g.loading = None;
            match r {
                Ok((commits, op)) => {
                    // The selection stays on its commit, wherever it moved.
                    let was = g.selected.and_then(|i| g.commits.get(i)).map(|c| c.hash.clone());
                    g.rows = git_graph::lanes(&commits);
                    g.commits = commits;
                    g.op = op;
                    g.error = None;
                    g.selected = was.and_then(|h| g.commits.iter().position(|c| c.hash == h));
                    if g.selected.is_none() {
                        g.shown_files.clear();
                    }
                }
                Err(e) => g.error = Some(git::explain(&e)),
            }
        }
        if let Some((hash, rx)) = &g.files
            && let Ok(r) = rx.try_recv()
        {
            let _ = hash;
            g.shown_files = r.unwrap_or_default();
            g.files = None;
        }
        if let Some(rebase) = &mut g.rebase
            && let Some(r) = rebase.loading.as_ref().and_then(|rx| rx.try_recv().ok())
        {
            rebase.loading = None;
            match r {
                Ok(steps) => rebase.steps = steps,
                Err(e) => g.error = Some(git::explain(&e)),
            }
        }
    }

    fn git_graph_ask(&mut self, ctx: &egui::Context, ask: Option<Ask>) {
        let Some(ask) = ask else { return };
        let run = |app: &mut CodeApp, busy: String, f: GitWork| {
            app.git_run(ctx, busy, f);
            app.git_graph.stale = true;
        };
        match ask {
            Ask::Reload => self.git_graph.stale = true,
            Ask::Select(i) => {
                let g = &mut self.git_graph;
                g.selected = Some(i);
                g.shown_files.clear();
                let (Some(repo), Some(commit)) = (self.git_repo().cloned(), self.git_graph.commits.get(i)) else {
                    return;
                };
                let hash = commit.hash.clone();
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                let h = hash.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(repo.commit_files(&h));
                    wake();
                });
                self.git_graph.files = Some((hash, rx));
            }
            Ask::OpenFile(hash, short, rel) => self.open_commit_file(ctx, hash, short, rel),
            Ask::CherryPick(hash, short) => run(
                self,
                format!("Cherry-picking {short}…"),
                Box::new(move |repo| match repo.cherry_pick(&hash) {
                    Ok(()) => Ok(format!("Cherry-picked {short}")),
                    Err(e) if repo.operation().is_some() => {
                        Err(format!("{e} — resolve the conflicts (Git: Resolve Merge Conflicts), then Continue"))
                    }
                    Err(e) => Err(e),
                }),
            ),
            Ask::Revert(hash, short) => run(
                self,
                format!("Reverting {short}…"),
                Box::new(move |repo| repo.revert(&hash).map(|()| format!("Reverted {short}"))),
            ),
            Ask::Branch(name, hash) => run(
                self,
                format!("Creating {name}…"),
                Box::new(move |repo| repo.branch_at(&name, &hash).map(|()| format!("Branch {name} made"))),
            ),
            Ask::PlanRebase(hash, label) => {
                let Some(repo) = self.git_repo().cloned() else { return };
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                let base = hash.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(repo.rebase_plan(&base));
                    wake();
                });
                self.git_graph.rebase =
                    Some(Rebase { base: hash, base_label: label, steps: Vec::new(), loading: Some(rx) });
            }
            Ask::Rebase(base, steps) => {
                self.git_graph.rebase = None;
                run(
                    self,
                    "Rebasing…".to_owned(),
                    Box::new(move |repo| match repo.rebase(&base, &steps) {
                        Ok(()) => Ok("Rebased".to_owned()),
                        Err(e) if repo.operation() == Some(Operation::Rebase) => {
                            Err(format!("{e} — the rebase stopped at a conflict: resolve it, then Continue"))
                        }
                        Err(e) => Err(e),
                    }),
                );
            }
            Ask::Continue(op) => run(
                self,
                format!("Continuing the {}…", op.name()),
                Box::new(move |repo| repo.continue_operation(op).map(|()| format!("The {} went on", op.name()))),
            ),
            Ask::Abort(op) => run(
                self,
                format!("Stopping the {}…", op.name()),
                Box::new(move |repo| repo.abort_operation(op).map(|()| format!("The {} was given up", op.name()))),
            ),
            Ask::ShowConflicts => self.show_merge_conflicts(),
        }
    }

    fn merge_conflicts_window(&mut self, ctx: &egui::Context) {
        let pal = theme::colors();
        let files = self.git_conflicts();
        let root = self.git_repo().cloned().map(|r| r.root);
        let mut open = true;
        let mut choose: Option<(String, usize, Choice)> = None;
        let mut resolved: Option<String> = None;
        let mut open_file: Option<std::path::PathBuf> = None;
        let mut op_ask = None;
        let op = self.git_graph.op;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.8).min(1100.0), (room.y * 0.78).max(400.0));
        let g = &mut self.git_graph;
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
                            op_ask = Some(Ask::Continue(op));
                        }
                        if ui.button("Abort").on_hover_text(format!("Give the {} up", op.name())).clicked() {
                            op_ask = Some(Ask::Abort(op));
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
                        if let Some(base) = &c.base {
                            ui.label(
                                RichText::new(format!("Before both: {}", base.join(" ⏎ "))).small().color(pal.text_dim),
                            );
                        }
                    }
                });
            });
        self.git_graph.conflicts_open = open;
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
        self.git_graph_ask(ctx, op_ask);
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

/// The window's contents. Returns what to do.
fn graph_ui(ui: &mut egui::Ui, g: &mut GitGraphUi) -> Option<Ask> {
    let pal = theme::colors();
    let mut ask = None;
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            ask = Some(Ask::Reload);
        }
        if g.loading.is_some() {
            ui.spinner();
        }
        ui.label(RichText::new(format!("{} commits, every branch", g.commits.len())).small().color(pal.text_dim));
        ui.label(
            RichText::new("Click a commit for its files · right click for cherry-pick, revert, a branch, a rebase")
                .small()
                .color(pal.text_dim),
        );
    });
    if let Some(op) = g.op {
        ui.horizontal(|ui| {
            ui.colored_label(Color32::from_rgb(0xE0, 0x9A, 0x3E), format!("A {} is in progress.", op.name()));
            if ui.button("Resolve Conflicts…").clicked() {
                ask = Some(Ask::ShowConflicts);
            }
            if ui.button("Continue").clicked() {
                ask = Some(Ask::Continue(op));
            }
            if ui.button("Abort").clicked() {
                ask = Some(Ask::Abort(op));
            }
        });
    }
    if let Some(e) = &g.error {
        ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
    }
    if let Some(rebase) = &mut g.rebase {
        let mut close = false;
        egui::Frame::new()
            .stroke(Stroke::new(1.0, pal.accent.gamma_multiply(0.6)))
            .corner_radius(8.0)
            .inner_margin(8.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(format!("Interactive rebase: the commits after {}", rebase.base_label)).strong(),
                );
                ui.label(
                    RichText::new("Oldest first. Move them, change what's done to each, then Start.")
                        .small()
                        .color(pal.text_dim),
                );
                if rebase.loading.is_some() {
                    ui.spinner();
                }
                let mut swap = None;
                let count = rebase.steps.len();
                for (i, step) in rebase.steps.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(i > 0, egui::Button::new("↑").small()).clicked() {
                            swap = Some((i, i - 1));
                        }
                        if ui.add_enabled(i + 1 < count, egui::Button::new("↓").small()).clicked() {
                            swap = Some((i, i + 1));
                        }
                        let current = match step.action {
                            Action::Pick => "pick",
                            Action::Reword(_) => "reword",
                            Action::Squash => "squash",
                            Action::Fixup => "fixup",
                            Action::Drop => "drop",
                        };
                        egui::ComboBox::from_id_salt(("rebase_action", i)).selected_text(current).width(80.0).show_ui(
                            ui,
                            |ui| {
                                for (name, action) in [
                                    ("pick", Action::Pick),
                                    ("reword", Action::Reword(step.subject.clone())),
                                    ("squash", Action::Squash),
                                    ("fixup", Action::Fixup),
                                    ("drop", Action::Drop),
                                ] {
                                    if ui.selectable_label(current == name, name).clicked() {
                                        step.action = action;
                                    }
                                }
                            },
                        );
                        ui.label(RichText::new(&step.short).monospace().color(pal.text_dim));
                        match &mut step.action {
                            Action::Reword(message) => {
                                ui.add(egui::TextEdit::singleline(message).desired_width(f32::INFINITY));
                            }
                            Action::Drop => {
                                ui.label(RichText::new(&step.subject).strikethrough().color(pal.text_dim));
                            }
                            _ => {
                                ui.label(&step.subject);
                            }
                        }
                    });
                }
                if let Some((a, b)) = swap {
                    rebase.steps.swap(a, b);
                }
                ui.horizontal(|ui| {
                    let ready = !rebase.steps.is_empty() && rebase.loading.is_none();
                    if ui.add_enabled(ready, egui::Button::new("Start Rebase")).clicked() {
                        ask = Some(Ask::Rebase(rebase.base.clone(), rebase.steps.clone()));
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                    if !ready && rebase.loading.is_none() {
                        ui.label(
                            RichText::new("No commits after this one on this branch.").small().color(pal.text_dim),
                        );
                    }
                });
            });
        if close {
            g.rebase = None;
        }
        ui.add_space(6.0);
    }
    // A new branch's name, asked for above the graph.
    if let Some((hash, name)) = &mut g.new_branch {
        let mut done = false;
        ui.horizontal(|ui| {
            ui.label("New branch at this commit:");
            let edit = ui.add(egui::TextEdit::singleline(name).hint_text("name").desired_width(200.0));
            if name.is_empty() && !edit.has_focus() {
                edit.request_focus();
            }
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Create").clicked() || enter) && !name.trim().is_empty() {
                ask = Some(Ask::Branch(name.trim().to_owned(), hash.clone()));
                done = true;
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = true;
            }
        });
        if done {
            g.new_branch = None;
        }
    }
    let width = ui.available_width();
    let files_h = if g.selected.is_some() { (ui.available_height() * 0.3).clamp(80.0, 220.0) } else { 0.0 };
    let graph_h = (ui.available_height() - files_h).max(100.0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let max_lanes = g.rows.iter().map(|r| r.width).max().unwrap_or(1).min(24);
    let graph_w = max_lanes as f32 * LANE_W + 8.0;
    egui::ScrollArea::vertical().id_salt("git_graph_rows").max_height(graph_h).auto_shrink([false, false]).show_rows(
        ui,
        ROW_H,
        g.commits.len(),
        |ui, range| {
            for i in range {
                let (commit, row) = (&g.commits[i], &g.rows[i]);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                let selected = g.selected == Some(i);
                if selected {
                    ui.painter().rect_filled(rect, 4.0, pal.accent.gamma_multiply(0.18));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, pal.hover_bg);
                }
                let x = |lane: usize| rect.left() + 6.0 + lane as f32 * LANE_W;
                let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
                let painter = ui.painter();
                for &(a, b) in &row.top {
                    if a < 24 && b < 24 {
                        painter.line_segment(
                            [Pos2::new(x(a), top), Pos2::new(x(b), mid)],
                            Stroke::new(1.6, lane_color(a)),
                        );
                    }
                }
                for &(a, b) in &row.bottom {
                    if a < 24 && b < 24 {
                        painter.line_segment(
                            [Pos2::new(x(a), mid), Pos2::new(x(b), bottom)],
                            Stroke::new(1.6, lane_color(b)),
                        );
                    }
                }
                let head = commit.refs.contains("HEAD");
                let node = Pos2::new(x(row.lane.min(23)), mid);
                painter.circle(
                    node,
                    if head { 5.0 } else { 4.0 },
                    pal.editor_bg,
                    Stroke::new(2.0, lane_color(row.lane)),
                );
                if commit.parents.len() > 1 {
                    painter.circle_filled(node, 2.0, lane_color(row.lane));
                }
                // Refs, then the subject, then who and when.
                let mut text_x = rect.left() + graph_w;
                for r in commit.refs.split(", ").filter(|r| !r.is_empty()) {
                    let label = r.trim_start_matches("HEAD -> ").trim_start_matches("tag: ");
                    let is_tag = r.starts_with("tag: ");
                    let current = r.starts_with("HEAD -> ") || r == "HEAD";
                    let galley = painter.layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.5), pal.text);
                    let chip = egui::Rect::from_min_size(
                        Pos2::new(text_x, mid - galley.size().y / 2.0 - 2.0),
                        galley.size() + Vec2::new(10.0, 4.0),
                    );
                    let color = if is_tag { Color32::from_rgb(0xE0, 0x9A, 0x3E) } else { lane_color(row.lane) };
                    painter.rect(
                        chip,
                        8.0,
                        color.gamma_multiply(if current { 0.45 } else { 0.18 }),
                        Stroke::new(1.0, color.gamma_multiply(0.8)),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(chip.min + Vec2::new(5.0, 2.0), galley, pal.text);
                    text_x = chip.right() + 6.0;
                }
                let right = format!("{} · {} · {}", commit.short, commit.author, git::relative_time(now - commit.time));
                let right_galley = painter.layout_no_wrap(right, egui::FontId::proportional(11.5), pal.text_dim);
                let right_x = rect.right() - right_galley.size().x - 6.0;
                let subject = painter.layout(
                    commit.subject.clone(),
                    egui::FontId::proportional(13.0),
                    if head { pal.text } else { pal.text.gamma_multiply(0.92) },
                    (right_x - text_x - 12.0).max(40.0),
                );
                let subject_y = mid - subject.size().y.min(16.0) / 2.0;
                painter
                    .with_clip_rect(egui::Rect::from_min_max(
                        Pos2::new(text_x, rect.top()),
                        Pos2::new(right_x - 8.0, rect.bottom()),
                    ))
                    .galley(Pos2::new(text_x, subject_y), subject, pal.text);
                painter.galley(Pos2::new(right_x, mid - right_galley.size().y / 2.0), right_galley, pal.text_dim);
                if resp.clicked() {
                    ask = Some(Ask::Select(i));
                }
                let (hash, short) = (commit.hash.clone(), commit.short.clone());
                let subject_text = commit.subject.clone();
                resp.context_menu(|ui| {
                    if ui.button("Cherry-pick onto the current branch").clicked() {
                        ask = Some(Ask::CherryPick(hash.clone(), short.clone()));
                        ui.close();
                    }
                    if ui.button("Revert (a commit that undoes it)").clicked() {
                        ask = Some(Ask::Revert(hash.clone(), short.clone()));
                        ui.close();
                    }
                    if ui.button("Create Branch Here…").clicked() {
                        g.new_branch = Some((hash.clone(), String::new()));
                        ui.close();
                    }
                    if ui
                        .button("Rebase Interactively From Here…")
                        .on_hover_text("Reorder, reword, squash or drop the commits after this one")
                        .clicked()
                    {
                        ask = Some(Ask::PlanRebase(hash.clone(), format!("{short} {subject_text}")));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Copy Hash").clicked() {
                        ui.ctx().copy_text(hash.clone());
                        ui.close();
                    }
                });
            }
        },
    );
    if let Some(commit) = g.selected.and_then(|i| g.commits.get(i)) {
        ui.separator();
        ui.label(RichText::new(format!("{} {}", commit.short, commit.subject)).strong());
        if g.files.is_some() {
            ui.spinner();
        }
        egui::ScrollArea::vertical().id_salt("git_graph_files").auto_shrink([false, false]).show(ui, |ui| {
            for (kind, rel) in &g.shown_files {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(kind.to_string()).strong().color(pal.text_dim));
                    if ui.add(egui::Label::new(rel).sense(Sense::click())).on_hover_text("Show what changed").clicked()
                    {
                        ask = Some(Ask::OpenFile(commit.hash.clone(), commit.short.clone(), rel.clone()));
                    }
                });
            }
        });
    }
    ask
}
