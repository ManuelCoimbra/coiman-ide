//! GitHub: Show Actions, and the ACTIONS panel: the repository's workflow runs, a run's jobs
//! and steps, the log of what failed, and each failed step's place in its workflow file. All
//! through the `gh` program, on threads of their own.

use std::sync::mpsc::{self, Receiver};

use coiman_core::language::lsp::types as lt;
use coiman_core::project::actions::{self, Job, LogLine, Run};
use eframe::egui::{self, Color32, RichText, Sense};

use crate::app::{CodeApp, PanelTab, repaint};
use crate::ui::theme;

/// Runs listed.
const RUNS: usize = 30;

#[derive(Default)]
pub(crate) struct ActionsUi {
    /// The panel has been asked for: its tab stays until the app closes.
    pub(crate) used: bool,
    runs: Option<Result<Vec<Run>, String>>,
    loading: Option<Receiver<Result<Vec<Run>, String>>>,
    selected: Option<u64>,
    jobs: Option<Result<Vec<Job>, String>>,
    jobs_loading: Option<Receiver<Result<Vec<Job>, String>>>,
    log: Option<Result<Vec<LogLine>, String>>,
    log_loading: Option<Receiver<Result<Vec<LogLine>, String>>>,
    rerunning: Option<Receiver<Result<(), String>>>,
    note: Option<String>,
    /// List only the runs that failed.
    failed_only: bool,
    /// The whole log, not only its errors and the lines before them.
    whole_log: bool,
}

/// What the panel asks the app to do once drawn.
enum Ask {
    Refresh,
    Select(u64),
    Log(u64),
    Rerun(u64),
    /// Open the workflow file at a step: the workflow's name, and the step's.
    OpenStep(String, String),
}

/// A run's or step's state in a color: green done, red failed, amber going.
fn state_color(state: &str) -> Color32 {
    match state {
        "success" => Color32::from_rgb(0x4C, 0xB8, 0x6A),
        "failure" | "timed_out" | "startup_failure" => Color32::from_rgb(0xE5, 0x53, 0x4B),
        "in_progress" | "queued" | "waiting" | "pending" | "requested" => Color32::from_rgb(0xE0, 0x9A, 0x3E),
        _ => theme::colors().text_dim,
    }
}

fn spawn<T: Send + 'static>(ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    let (tx, rx) = mpsc::channel();
    let wake = repaint(ctx);
    std::thread::spawn(move || {
        let _ = tx.send(f());
        wake();
    });
    rx
}

impl CodeApp {
    /// GitHub: Show Actions.
    pub(crate) fn show_actions(&mut self, ctx: &egui::Context) {
        self.actions.used = true;
        self.panel_tab = PanelTab::Actions;
        self.panel_visible = true;
        if self.actions.runs.is_none() && self.actions.loading.is_none() {
            self.refresh_actions(ctx);
        }
    }

    fn refresh_actions(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else {
            self.actions.runs = Some(Err("Open a folder of a GitHub repository first".to_owned()));
            return;
        };
        let failed_only = self.actions.failed_only;
        self.actions.loading = Some(spawn(ctx, move || actions::runs(&root, RUNS, failed_only)));
    }

    fn pump_actions(&mut self) {
        let a = &mut self.actions;
        if let Some(r) = a.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            a.loading = None;
            a.runs = Some(r);
        }
        if let Some(r) = a.jobs_loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            a.jobs_loading = None;
            a.jobs = Some(r);
        }
        if let Some(r) = a.log_loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            a.log_loading = None;
            a.log = Some(r);
        }
        if let Some(r) = a.rerunning.as_ref().and_then(|rx| rx.try_recv().ok()) {
            a.rerunning = None;
            a.note = Some(match r {
                Ok(()) => "GitHub is running the failed jobs again: Refresh shows it".to_owned(),
                Err(e) => e,
            });
        }
    }

    pub(crate) fn actions_panel(&mut self, ui: &mut egui::Ui) {
        self.pump_actions();
        let ask = actions_ui(ui, &mut self.actions);
        let ctx = ui.ctx().clone();
        let Some(ask) = ask else { return };
        let Some(root) = self.explorer.root.clone() else { return };
        match ask {
            Ask::Refresh => self.refresh_actions(&ctx),
            Ask::Select(id) => {
                let a = &mut self.actions;
                a.selected = Some(id);
                a.jobs = None;
                a.log = None;
                let r = root.clone();
                a.jobs_loading = Some(spawn(&ctx, move || actions::jobs(&r, id)));
            }
            Ask::Log(id) => {
                self.actions.log = None;
                self.actions.log_loading = Some(spawn(&ctx, move || actions::failed_log(&root, id)));
            }
            Ask::Rerun(id) => {
                self.actions.rerunning = Some(spawn(&ctx, move || actions::rerun(&root, id, true)));
            }
            Ask::OpenStep(workflow, step) => match actions::workflow_file(&root, &workflow) {
                Some(file) => {
                    let line =
                        std::fs::read_to_string(&file).ok().and_then(|t| actions::step_line(&t, &step)).unwrap_or(1);
                    let at = lt::Position::new(line.saturating_sub(1) as u32, 0);
                    self.jump_to(&file, lt::Range::new(at, at), false);
                }
                None => self.status = Some(format!("No workflow file here is named {workflow}")),
            },
        }
    }
}

fn actions_ui(ui: &mut egui::Ui, a: &mut ActionsUi) -> Option<Ask> {
    let pal = theme::colors();
    let dim = |t: String| RichText::new(t).color(pal.text_dim);
    let mut ask = None;
    ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            ask = Some(Ask::Refresh);
        }
        if ui.checkbox(&mut a.failed_only, "Only failed").changed() {
            ask = Some(Ask::Refresh);
        }
        if a.loading.is_some() {
            ui.spinner();
        }
        ui.label(dim("Through gh, with your GitHub login".to_owned()));
        if let Some(note) = &a.note {
            ui.label(RichText::new(note).color(pal.modified));
        }
    });
    let runs = match &a.runs {
        None => return ask,
        Some(Err(e)) => {
            ui.colored_label(Color32::from_rgb(0xE5, 0x53, 0x4B), e);
            return ask;
        }
        Some(Ok(runs)) => runs,
    };
    if runs.is_empty() {
        ui.label("No workflow runs in this repository yet.");
        return ask;
    }
    let now = chrono_now();
    ui.horizontal_top(|ui| {
        // The runs, newest first.
        let list_w = (ui.available_width() * 0.42).clamp(260.0, 520.0);
        ui.vertical(|ui| {
            ui.set_width(list_w);
            egui::ScrollArea::vertical().id_salt("actions_runs").show(ui, |ui| {
                for r in runs {
                    let state = r.state();
                    let chosen = a.selected == Some(r.database_id);
                    let age = actions::age_seconds(&r.created_at, now)
                        .map(coiman_core::project::git::relative_time)
                        .unwrap_or_default();
                    let resp = ui
                        .horizontal(|ui| {
                            ui.label(RichText::new("●").color(state_color(state)));
                            let mut title = RichText::new(&r.display_title);
                            if chosen {
                                title = title.strong();
                            }
                            ui.add(egui::Label::new(title).truncate().selectable(false));
                        })
                        .response
                        .interact(Sense::click());
                    ui.label(dim(format!(
                        "    {} · {} · {} · {age}",
                        r.workflow_name,
                        r.head_branch,
                        state.replace('_', " ")
                    )));
                    if resp.on_hover_text(format!("{}\n{}", r.display_title, r.url)).clicked() {
                        ask = Some(Ask::Select(r.database_id));
                    }
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            let Some(run) = a.selected.and_then(|id| runs.iter().find(|r| r.database_id == id)) else {
                ui.label(dim("Click a run for its jobs and steps".to_owned()));
                return;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(&run.workflow_name).strong());
                ui.hyperlink_to("Open on GitHub", &run.url);
                if run.state() == "failure" {
                    if ui.button("Log of what failed").clicked() {
                        ask = Some(Ask::Log(run.database_id));
                    }
                    let rerun = ui
                        .add_enabled(a.rerunning.is_none(), egui::Button::new("Re-run failed jobs"))
                        .on_hover_text("GitHub runs the failed jobs again, on its minutes");
                    if rerun.clicked() {
                        ask = Some(Ask::Rerun(run.database_id));
                    }
                }
                if a.jobs_loading.is_some() || a.log_loading.is_some() {
                    ui.spinner();
                }
            });
            egui::ScrollArea::vertical().id_salt("actions_jobs").show(ui, |ui| {
                match &a.jobs {
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xE5, 0x53, 0x4B), e);
                    }
                    Some(Ok(jobs)) => {
                        for job in jobs {
                            let state = if job.status == "completed" { &job.conclusion } else { &job.status };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("●").color(state_color(state)));
                                ui.label(RichText::new(&job.name).strong());
                            });
                            // Steps that failed, and the one running; the rest only when it failed.
                            for step in &job.steps {
                                let s = if step.status == "completed" { &step.conclusion } else { &step.status };
                                let notable = s != "success" && s != "skipped";
                                if !notable && state != "failure" {
                                    continue;
                                }
                                ui.horizontal(|ui| {
                                    ui.add_space(18.0);
                                    ui.label(
                                        RichText::new(format!("{}. {}", step.number, step.name)).color(state_color(s)),
                                    );
                                    if s == "failure"
                                        && ui
                                            .small_button("Open in workflow")
                                            .on_hover_text("Where the step is in the workflow file")
                                            .clicked()
                                    {
                                        ask = Some(Ask::OpenStep(run.workflow_name.clone(), step.name.clone()));
                                    }
                                });
                            }
                        }
                    }
                    None => {}
                }
                match &a.log {
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xE5, 0x53, 0x4B), e);
                    }
                    Some(Ok(lines)) => {
                        ui.separator();
                        let errors = lines.iter().filter(|l| l.error).count();
                        ui.horizontal(|ui| {
                            ui.label(dim(format!("{} lines, {errors} errors (in red)", lines.len())));
                            ui.checkbox(&mut a.whole_log, "Whole log")
                                .on_hover_text("Else only the errors, each with the lines before it");
                        });
                        // The errors and the few lines before each, unless the whole log is asked for.
                        const BEFORE: usize = 4;
                        let shown: Vec<bool> = if a.whole_log || errors == 0 {
                            vec![true; lines.len()]
                        } else {
                            let mut keep = vec![false; lines.len()];
                            for (i, _) in lines.iter().enumerate().filter(|(_, l)| l.error) {
                                keep[i.saturating_sub(BEFORE)..=i].fill(true);
                            }
                            keep
                        };
                        let mono = egui::FontId::monospace(12.0);
                        let mut last_shown = None;
                        for (i, l) in lines.iter().enumerate().filter(|(i, _)| shown[*i]) {
                            if last_shown.is_some_and(|p: usize| p + 1 != i) {
                                ui.label(dim("⋯".to_owned()));
                            }
                            last_shown = Some(i);
                            let color = if l.error { Color32::from_rgb(0xE5, 0x53, 0x4B) } else { pal.text };
                            ui.label(RichText::new(&l.text).font(mono.clone()).color(color))
                                .on_hover_text(format!("{} · {}", l.job, l.step));
                        }
                    }
                    None => {}
                }
            });
        });
    });
    ask
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}
