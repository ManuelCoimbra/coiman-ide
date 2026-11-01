//! Constant preview: keeps showing what a file does. Pages and documents have views
//! of their own; a program runs again after each pause in typing.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui::{self, RichText};
use rust_code_core::constant_run::{self, Ending, Job, RunEvent};
use rust_code_core::lsp::types as lt;
use rust_code_core::{matchers, run};

use super::cargo::{problem_diagnostic, shell};
use super::{CodeApp, PanelTab, repaint};
use crate::theme;

/// Seconds typing must pause before the program runs again.
const SETTLE: f64 = 1.0;
/// Lines kept from one run.
const MAX_LINES: usize = 5000;
/// The time limits on offer: a server or a window wants none, and restarts on each change.
const LIMITS: [(&str, Option<u64>); 3] = [("10 seconds", Some(10)), ("1 minute", Some(60)), ("No limit", None)];

#[derive(Default)]
pub(super) struct Constant {
    used: bool,
    /// What's being kept running, and the folder whose edits start it again.
    target: Option<(PathBuf, PathBuf)>,
    planning: Option<Receiver<Result<Vec<run::RunPlan>, String>>>,
    plans: Vec<run::RunPlan>,
    chosen: Option<String>,
    job: Option<Job>,
    lines: Vec<(String, bool)>,
    /// What the last finished run printed, to compare the next one with.
    last_output: Option<String>,
    changed: Vec<bool>,
    state: String,
    failed: bool,
    began: f64,
    limit: usize,
    /// The edits last run, and the ones waiting for a pause.
    ran: u64,
    seen: (u64, f64),
    problems: Vec<(PathBuf, lt::Diagnostic)>,
}

/// Whether a script belongs to a page in its folder, so the page is what shows it.
fn script_of_a_page(file: &Path) -> bool {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let page = file.parent().map(|d| d.join("index.html")).and_then(|p| std::fs::read_to_string(p).ok());
    page.is_some_and(|text| text.contains(&name))
}

impl CodeApp {
    pub(super) fn constant_tab_shown(&self) -> bool {
        self.constant.used
    }

    /// `Preview: Constant Preview`: starts what suits the active file, or stops the run.
    pub(super) fn toggle_constant_preview(&mut self, ctx: &egui::Context) {
        let active = self.docs.get(self.active).and_then(|d| d.path.clone());
        if self.constant.target.as_ref().is_some_and(|(file, _)| Some(file) == active.as_ref()) {
            self.stop_constant();
            return;
        }
        let Some(file) = active else {
            self.refuse_run(ctx, "Save this file first: a preview follows a file".to_owned());
            return;
        };
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "html" | "htm" | "css" => return self.open_live_preview(ctx),
            "js" | "mjs" if script_of_a_page(&file) => return self.open_live_preview(ctx),
            "md" | "markdown" => return self.toggle_markdown_preview(),
            "svg" => return self.toggle_svg_preview(),
            _ => {}
        }
        let folder = self
            .explorer
            .root
            .clone()
            .filter(|r| file.starts_with(r))
            .or_else(|| file.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| file.clone());
        if !self.allowed_to_run(&folder) {
            return;
        }
        self.stop_constant();
        self.constant.used = true;
        self.constant.target = Some((file, folder));
        self.constant.last_output = None;
        self.constant.chosen = None;
        self.constant.plans.clear();
        self.constant.lines.clear();
        self.constant.changed.clear();
        self.panel_tab = PanelTab::Constant;
        self.panel_visible = true;
        self.constant_run_now(ctx);
    }

    fn stop_constant(&mut self) {
        let state = &mut self.constant;
        state.target = None;
        state.planning = None;
        state.job = None;
        state.state = "Stopped".to_owned();
        state.failed = false;
        state.problems.clear();
    }

    /// Everything open in the followed folder, as one number that moves on any edit.
    fn constant_edits(&self, folder: &Path) -> u64 {
        let open = self.docs.iter().filter(|d| d.path.as_deref().is_some_and(|p| p.starts_with(folder)));
        open.fold(0u64, |sum, d| sum.wrapping_add(d.buffer.version()).wrapping_add(d.uid))
    }

    /// Saves what's being edited, and runs what's on screen.
    fn constant_run_now(&mut self, ctx: &egui::Context) {
        let Some((file, folder)) = self.constant.target.clone() else { return };
        for i in 0..self.docs.len() {
            let inside = self.docs[i].path.as_deref().is_some_and(|p| p.starts_with(&folder));
            if inside && self.docs[i].is_dirty() && !self.save_doc(i) {
                self.constant.state = format!("{} couldn't be saved", self.docs[i].title());
                self.constant.failed = true;
                return;
            }
        }
        self.constant.ran = self.constant_edits(&folder);
        self.constant.seen = (self.constant.ran, ctx.input(|i| i.time));
        self.constant.job = None;
        self.constant.state = "Starting".to_owned();
        self.constant.failed = false;
        let source = std::fs::read_to_string(&file).unwrap_or_default();
        let root = self.explorer.root.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(run::preview_plans(&file, &source, root.as_deref()));
            wake();
        });
        self.constant.planning = Some(rx);
    }

    fn constant_start(&mut self, ctx: &egui::Context) {
        let chosen = self.constant.chosen.clone();
        let plan =
            self.constant.plans.iter().find(|p| Some(&p.title) == chosen.as_ref()).or(self.constant.plans.first());
        let Some(plan) = plan.cloned() else { return };
        let (program, args) = shell(&plan.command);
        let limit = LIMITS[self.constant.limit].1.map(Duration::from_secs);
        let env = self.environment_variables();
        self.constant.lines.clear();
        self.constant.changed.clear();
        self.constant.state = format!("Running {}", plan.title);
        self.constant.began = ctx.input(|i| i.time);
        self.constant.job = Some(constant_run::start(&program, &args, &plan, &env, limit, repaint(ctx)));
    }

    pub(super) fn pump_constant(&mut self, ctx: &egui::Context) {
        if let Some(planning) = &self.constant.planning
            && let Ok(answer) = planning.try_recv()
        {
            self.constant.planning = None;
            match answer {
                Ok(plans) => {
                    self.constant.plans = plans;
                    self.constant_start(ctx);
                }
                Err(why) => {
                    // Mid-edit it may run again; a file that never ran isn't followed.
                    if self.constant.plans.is_empty() {
                        self.constant.target = None;
                    }
                    self.constant.lines.clear();
                    self.constant.changed.clear();
                    self.constant.state = why;
                    self.constant.failed = true;
                }
            }
        }
        let mut finished = None;
        if let Some(job) = &self.constant.job {
            while let Ok(event) = job.events.try_recv() {
                match event {
                    RunEvent::Line { text, error } => {
                        if self.constant.lines.len() < MAX_LINES {
                            self.constant.lines.push((text, error));
                        }
                    }
                    RunEvent::Finished(ending) => finished = Some(ending),
                }
            }
        }
        if let Some(ending) = finished {
            self.constant_finished(ctx, ending);
        }
        let Some((_, folder)) = self.constant.target.clone() else { return };
        let edits = self.constant_edits(&folder);
        if edits == self.constant.ran {
            return;
        }
        let now = ctx.input(|i| i.time);
        if self.constant.seen.0 != edits {
            self.constant.seen = (edits, now);
        }
        let left = SETTLE - (now - self.constant.seen.1);
        if left > 0.0 {
            ctx.request_repaint_after(Duration::from_secs_f64(left));
        } else {
            self.constant_run_now(ctx);
        }
    }

    fn constant_finished(&mut self, ctx: &egui::Context, ending: Ending) {
        let took = ctx.input(|i| i.time) - self.constant.began;
        let (state, failed) = match &ending {
            Ending::Exit(Some(0)) => (format!("Finished in {took:.1} s"), false),
            Ending::Exit(Some(code)) => (format!("Exited with code {code} after {took:.1} s"), true),
            Ending::Exit(None) => ("Ended by a signal".to_owned(), true),
            Ending::TimedOut => (format!("Stopped at the {} limit", LIMITS[self.constant.limit].0), true),
            Ending::TooMuchOutput => ("Stopped: it printed too much".to_owned(), true),
            Ending::CouldNotStart(why) => (format!("Could not start: {why}"), true),
        };
        let state_now = &mut self.constant;
        state_now.state = state;
        state_now.failed = failed;
        state_now.job = None;
        let text: String = state_now.lines.iter().map(|(l, _)| format!("{l}\n")).collect();
        state_now.changed = match &state_now.last_output {
            Some(old) => constant_run::changed_lines(old, &text),
            None => Vec::new(),
        };
        state_now.last_output = Some(text);
        let lines: Vec<&str> = state_now.lines.iter().map(|(l, _)| l.as_str()).collect();
        let found = matchers::scan(&lines);
        let bases: Vec<PathBuf> = state_now.plans.first().map(|p| p.cwd.clone()).into_iter().collect();
        state_now.problems = found
            .into_iter()
            .filter_map(|p| Some((super::prints::resolve(&p.file, &bases)?, problem_diagnostic(p))))
            .collect();
    }

    /// What the last run reported about `key` (a [`super::lsp::path_key`]).
    pub(super) fn constant_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        let found = self.constant.problems.iter().filter(|(p, _)| super::lsp::path_key(p) == key);
        found.map(|(_, d)| d.clone()).collect()
    }

    /// Files the last run reported about: (key, path).
    pub(super) fn constant_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.constant.problems.iter().map(|(p, _)| (super::lsp::path_key(p), p.clone())).collect()
    }

    pub(super) fn constant_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let red = egui::Color32::from_rgb(0xF1, 0x4C, 0x4C);
        let (mut stop, mut again, mut restart) = (false, false, false);
        ui.horizontal(|ui| {
            match &self.constant.target {
                Some((file, _)) => {
                    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(RichText::new(name).strong());
                }
                None => {
                    ui.label(RichText::new("Off").color(pal.text_dim));
                }
            }
            let color = if self.constant.failed { red } else { pal.text_dim };
            ui.label(RichText::new(&self.constant.state).color(color));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.constant.target.is_some() {
                    stop = ui.button("Stop").clicked();
                    again = ui.button("Run Again").clicked();
                    let shown = LIMITS[self.constant.limit].0;
                    egui::ComboBox::from_id_salt("constant_limit").selected_text(shown).show_ui(ui, |ui| {
                        for (i, (name, _)) in LIMITS.iter().enumerate() {
                            restart |= ui.selectable_value(&mut self.constant.limit, i, *name).changed();
                        }
                    });
                    ui.label(RichText::new("Stop after").color(pal.text_dim));
                    if self.constant.plans.len() > 1 {
                        let shown =
                            self.constant.chosen.clone().or(self.constant.plans.first().map(|p| p.title.clone()));
                        egui::ComboBox::from_id_salt("constant_program")
                            .selected_text(shown.unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for plan in &self.constant.plans {
                                    let this = Some(plan.title.clone());
                                    restart |= ui
                                        .selectable_value(&mut self.constant.chosen, this, plan.title.as_str())
                                        .changed();
                                }
                            });
                    }
                }
            });
        });
        if self.constant.target.is_some() {
            let warning = "Saves and runs this again after each pause in typing, so anything it does happens again. \
                           It can't be typed into. For a server or a window, choose No limit.";
            ui.label(RichText::new(warning).small().color(egui::Color32::from_rgb(0xCC, 0xA7, 0x00)));
        }
        let font = egui::FontId::monospace(self.font_size * 0.9);
        let highlight = pal.accent.gamma_multiply(0.25);
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if self.constant.lines.is_empty() && self.constant.job.is_none() && self.constant.planning.is_none() {
                ui.label(RichText::new("Nothing was printed.").color(pal.text_dim));
            }
            for (i, (line, error)) in self.constant.lines.iter().enumerate() {
                let mut text = RichText::new(line).font(font.clone()).color(match *error {
                    // Tools write their progress there too: it's only alarming if the run failed.
                    true if self.constant.failed => red,
                    true => pal.text_dim,
                    false => pal.text,
                });
                if self.constant.changed.get(i).copied().unwrap_or(false) {
                    text = text.background_color(highlight);
                }
                ui.add(egui::Label::new(text).wrap());
            }
        });
        if stop {
            self.stop_constant();
        } else if again || restart {
            self.constant_run_now(ui.ctx());
        }
    }
}
