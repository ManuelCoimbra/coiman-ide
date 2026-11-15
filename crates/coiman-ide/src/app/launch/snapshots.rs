//! The Review Snapshots window: the snapshots insta left to review (`.snap.new`), each shown as
//! what changed against the accepted one, to accept or reject one by one or all at once. In a
//! JavaScript project, Update Snapshots runs Jest's or Vitest's `-u` in a terminal.

use std::path::{Path, PathBuf};

use coiman_core::launch::snapshots::{self, DiffLine, Pending};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::theme;

#[derive(Default)]
pub(crate) struct Snapshots {
    open: bool,
    /// The folder looked in, and what it has waiting.
    root: Option<PathBuf>,
    pending: Vec<Pending>,
    /// The snapshot shown, and its change (computed when picked).
    picked: usize,
    shown: Option<(PathBuf, Vec<DiffLine>)>,
}

enum Action {
    Accept(Vec<Pending>),
    Reject(Vec<Pending>),
    Refresh,
    Open(PathBuf),
    UpdateJs(Vec<String>),
}

fn red() -> Color32 {
    Color32::from_rgb(0xE5, 0x53, 0x4B)
}

fn green() -> Color32 {
    Color32::from_rgb(0x57, 0xC0, 0x6A)
}

/// The change for a pending snapshot; a new snapshot (no old one) is all added lines.
fn change_of(p: &Pending) -> Vec<DiffLine> {
    let new = std::fs::read_to_string(&p.new).unwrap_or_default();
    let old = std::fs::read_to_string(&p.old).unwrap_or_default();
    snapshots::diff(&old, &new)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

impl CodeApp {
    pub(crate) fn show_snapshots(&mut self) {
        let Some(root) = self.explorer.root.clone() else {
            self.status = Some("Open a folder to review its snapshots".to_owned());
            return;
        };
        self.snapshots.root = Some(root);
        self.refresh_snapshots();
        self.snapshots.open = true;
    }

    fn refresh_snapshots(&mut self) {
        let Some(root) = self.snapshots.root.clone() else { return };
        self.snapshots.pending = snapshots::find_pending(&root);
        self.snapshots.picked = self.snapshots.picked.min(self.snapshots.pending.len().saturating_sub(1));
        self.snapshots.shown = None;
    }

    pub(crate) fn snapshots_window(&mut self, ctx: &egui::Context) {
        if !self.snapshots.open {
            return;
        }
        let mut open = true;
        let mut action = None;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.75).min(1000.0), (room.y * 0.7).max(360.0));
        egui::Window::new(RichText::new("Review Snapshots").size(16.0).strong())
            .id(egui::Id::unique("snapshots_window"))
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
            .show(ctx, |ui| action = self.snapshots_ui(ui));
        self.snapshots.open = open;
        match action {
            Some(Action::Accept(list)) => self.settle_snapshots(&list, true),
            Some(Action::Reject(list)) => self.settle_snapshots(&list, false),
            Some(Action::Refresh) => self.refresh_snapshots(),
            Some(Action::Open(path)) => self.open_file(&path),
            Some(Action::UpdateJs(args)) => self.update_js_snapshots(ctx, &args),
            None => {}
        }
    }

    /// Accepts or rejects snapshots, then looks again.
    fn settle_snapshots(&mut self, list: &[Pending], accept: bool) {
        let mut done = 0;
        for p in list {
            let result = if accept { snapshots::accept(p) } else { snapshots::reject(p) };
            match result {
                Ok(()) => done += 1,
                Err(e) => self.status = Some(e),
            }
        }
        if done == list.len() {
            let what = if accept { "Accepted" } else { "Rejected" };
            let n = if done == 1 { "1 snapshot".to_owned() } else { format!("{done} snapshots") };
            self.status = Some(format!("{what} {n}"));
        }
        self.refresh_snapshots();
    }

    /// Runs Jest's or Vitest's update in a terminal.
    fn update_js_snapshots(&mut self, ctx: &egui::Context, args: &[String]) {
        let Some(root) = self.snapshots.root.clone() else { return };
        if !self.allowed_to_run(&root) {
            return;
        }
        let Some((program, rest)) = args.split_first() else { return };
        // `npx` is `npx.cmd` on Windows: the terminal needs the file itself.
        let program = coiman_core::language::lsp::servers::which(program)
            .map_or_else(|| program.clone(), |p| p.to_string_lossy().into_owned());
        let env = self.environment_variables();
        let title = "Update Snapshots";
        let task =
            crate::ui::terminal::Task { title, program: &program, args: rest, env: &env, cwd: &root, scan: None };
        let scrollback = self.settings.terminal.scrollback;
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = crate::app::PanelTab::Terminal;
                self.status = Some(format!("Updating snapshots: {}", args.join(" ")));
            }
            Err(e) => self.status = Some(format!("Couldn't start {program}: {e}")),
        }
    }

    fn snapshots_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let mut action = None;
        let state = &mut self.snapshots;
        let root = state.root.clone()?;
        let js = snapshots::js_update_command(&root);
        ui.horizontal(|ui| {
            if ui.button("Refresh").on_hover_text("Look for pending snapshots again").clicked() {
                action = Some(Action::Refresh);
            }
            if !state.pending.is_empty() {
                if ui.button(format!("Accept all {}", state.pending.len())).clicked() {
                    action = Some(Action::Accept(state.pending.clone()));
                }
                if ui.button("Reject all").clicked() {
                    action = Some(Action::Reject(state.pending.clone()));
                }
            }
            if let Some(args) = &js {
                let tip = format!("Runs {} in a terminal: every snapshot that fails is rewritten", args.join(" "));
                if ui.button("Update Snapshots").on_hover_text(tip).clicked() {
                    action = Some(Action::UpdateJs(args.clone()));
                }
            }
        });
        ui.separator();
        if state.pending.is_empty() {
            ui.label("No snapshots waiting for review.");
            let hint = if js.is_some() {
                "Jest and Vitest don't leave snapshots to review: Update Snapshots rewrites the ones that fail."
            } else {
                "insta leaves a .snap.new beside the snapshot when a test's output changes: run the tests, then look again."
            };
            ui.label(RichText::new(hint).color(pal.text_dim));
            return action;
        }
        let picked = state.picked.min(state.pending.len() - 1);
        if state.shown.as_ref().is_none_or(|(p, _)| *p != state.pending[picked].new) {
            let p = &state.pending[picked];
            state.shown = Some((p.new.clone(), change_of(p)));
        }
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            ui.label(RichText::new(format!("{} waiting", state.pending.len())).color(pal.text_dim));
            egui::ScrollArea::vertical().id_salt("snap_list").auto_shrink([false, false]).show(ui, |ui| {
                for (i, p) in state.pending.iter().enumerate() {
                    let fresh = !p.old.exists();
                    let label = if fresh { format!("{}  (new)", p.name()) } else { p.name() };
                    let folder = p.new.parent().map(|d| relative(&root, d)).unwrap_or_default();
                    if ui.selectable_label(i == picked, label).on_hover_text(folder).clicked() {
                        state.picked = i;
                    }
                }
            });
            let ui = &mut cols[1];
            let p = state.pending[picked].clone();
            ui.horizontal(|ui| {
                if ui.button("Accept").on_hover_text("The new snapshot replaces the old").clicked() {
                    action = Some(Action::Accept(vec![p.clone()]));
                }
                if ui.button("Reject").on_hover_text("The new snapshot is deleted; the old stays").clicked() {
                    action = Some(Action::Reject(vec![p.clone()]));
                }
                if ui.button("Open").on_hover_text("Open the new snapshot in the editor").clicked() {
                    action = Some(Action::Open(p.new.clone()));
                }
            });
            ui.label(RichText::new(relative(&root, &p.new)).small().color(pal.text_dim));
            let Some((_, lines)) = &state.shown else { return };
            egui::ScrollArea::both().id_salt("snap_diff").auto_shrink([false, false]).show(ui, |ui| {
                for (tag, text) in lines {
                    let color = match tag {
                        '-' => red(),
                        '+' => green(),
                        _ => pal.text,
                    };
                    let text = RichText::new(format!("{tag} {text}")).monospace().color(color);
                    ui.add(egui::Label::new(text).extend());
                }
            });
        });
        action
    }
}
