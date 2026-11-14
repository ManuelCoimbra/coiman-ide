//! The Run History window: every test run, build and check of the project, kept between
//! sessions. A test run shows its failures and what changed since the run before (broken,
//! fixed, slower); Flaky lists tests that pass and fail by turns; Builds lists builds, checks
//! and runs with their first error. Failed Cargo tests can be run again on their own.

use std::path::PathBuf;

use coiman_core::launch::run_history::{self, BuildRun, History, TestRun};
use coiman_core::launch::testing::Outcome;
use coiman_core::project::recent;
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Tests,
    Flaky,
    Builds,
}

#[derive(Default)]
pub(crate) struct RunHistory {
    /// The project the history is of, and the history.
    loaded: Option<(PathBuf, History)>,
    open: bool,
    tab: Tab,
    /// The test run picked, as its index in the history.
    picked: Option<usize>,
}

enum Action {
    Open(PathBuf, usize),
    RerunFailed(Vec<String>),
}

fn red() -> Color32 {
    Color32::from_rgb(0xE5, 0x53, 0x4B)
}

fn green() -> Color32 {
    Color32::from_rgb(0x57, 0xC0, 0x6A)
}

fn outcome_color(o: Outcome) -> Color32 {
    match o {
        Outcome::Passed => green(),
        Outcome::Failed => red(),
        Outcome::Skipped => theme::colors().text_dim,
    }
}

fn plural(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

fn seconds_text(s: f32) -> String {
    if s >= 60.0 { format!("{}m {:02}s", (s / 60.0) as u32, (s % 60.0) as u32) } else { format!("{s:.1} s") }
}

impl CodeApp {
    /// The open project's history, loaded when the project changed.
    fn history_mut(&mut self) -> Option<&mut History> {
        let root = self.explorer.root.clone()?;
        if self.run_history.loaded.as_ref().is_none_or(|(r, _)| *r != root) {
            let history = History::load(&root);
            self.run_history.loaded = Some((root, history));
            self.run_history.picked = None;
        }
        self.run_history.loaded.as_mut().map(|(_, h)| h)
    }

    fn save_history(&mut self) {
        if let Some((root, history)) = &self.run_history.loaded
            && let Err(e) = history.save(root)
        {
            self.status = Some(format!("Could not save the run history: {e}"));
        }
    }

    pub(crate) fn record_test_run(&mut self, run: TestRun) {
        if run.tests.is_empty() {
            return;
        }
        let Some(history) = self.history_mut() else { return };
        history.add_tests(run);
        self.run_history.picked = None;
        self.save_history();
    }

    pub(crate) fn record_build(&mut self, run: BuildRun) {
        let Some(history) = self.history_mut() else { return };
        history.add_build(run);
        self.save_history();
    }

    pub(crate) fn show_run_history(&mut self) {
        if self.history_mut().is_none() {
            self.status = Some("Open a folder to see its run history".to_owned());
            return;
        }
        self.run_history.open = true;
    }

    pub(crate) fn run_history_window(&mut self, ctx: &egui::Context) {
        if !self.run_history.open {
            return;
        }
        if self.history_mut().is_none() {
            self.run_history.open = false;
            return;
        }
        let mut open = true;
        let mut action = None;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.8).min(1100.0), (room.y * 0.78).max(400.0));
        egui::Window::new(RichText::new("Run History").size(16.0).strong())
            .id(egui::Id::unique("run_history_window"))
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
            .show(ctx, |ui| action = self.run_history_ui(ui));
        self.run_history.open = open;
        match action {
            Some(Action::Open(path, line)) => {
                self.open_file(&path);
                self.jump_to_symbol_line(line.saturating_sub(1));
            }
            Some(Action::RerunFailed(paths)) => self.rerun_cargo_tests(ctx, paths),
            None => {}
        }
    }

    fn run_history_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let dim = |text: String| RichText::new(text).color(pal.text_dim);
        let mut action = None;
        let state = &mut self.run_history;
        let (_, history) = state.loaded.as_ref()?;
        let flaky = history.flaky();
        ui.horizontal(|ui| {
            let tabs = [
                (Tab::Tests, format!("Test runs  {}", history.tests.len())),
                (Tab::Flaky, if flaky.is_empty() { "Flaky".to_owned() } else { format!("Flaky  {}", flaky.len()) }),
                (Tab::Builds, format!("Builds  {}", history.builds.len())),
            ];
            for (tab, label) in tabs {
                if ui.selectable_label(state.tab == tab, label).clicked() {
                    state.tab = tab;
                }
            }
        });
        ui.separator();
        let now = recent::now();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match state.tab {
            Tab::Tests => {
                if history.tests.is_empty() {
                    ui.label("No test runs yet: run the tests from the Testing view or the gutter.");
                    return;
                }
                let picked = state.picked.unwrap_or(history.tests.len() - 1).min(history.tests.len() - 1);
                ui.columns(2, |cols| {
                    let ui = &mut cols[0];
                    ui.label(dim("Runs, newest first".to_owned()));
                    for (i, run) in history.tests.iter().enumerate().rev() {
                        let (p, f, s) =
                            (run.count(Outcome::Passed), run.count(Outcome::Failed), run.count(Outcome::Skipped));
                        let scope = if run.whole { "" } else { " (some)" };
                        let text = format!(
                            "{}  ·  {}{scope}  ·  {p} passed, {f} failed{}  ·  {}",
                            recent::ago(run.at, now),
                            run.runner,
                            if s > 0 { format!(", {s} skipped") } else { String::new() },
                            seconds_text(run.seconds)
                        );
                        let color = if f > 0 { red() } else { pal.text };
                        if ui.selectable_label(i == picked, RichText::new(text).color(color)).clicked() {
                            state.picked = Some(i);
                        }
                    }
                    let ui = &mut cols[1];
                    let run = &history.tests[picked];
                    // The run before, of the same runner, to compare with.
                    let before = history.tests[..picked].iter().rev().find(|r| r.runner == run.runner);
                    if let Some(before) = before {
                        let mut c = run_history::compare(before, run);
                        // A run of a few tests says nothing about the others.
                        if !run.whole {
                            c.removed.clear();
                        }
                        let mut said = false;
                        let mut list = |ui: &mut egui::Ui, title: &str, ids: &[String], color: Color32| {
                            if ids.is_empty() {
                                return;
                            }
                            said = true;
                            ui.label(RichText::new(format!("{title} ({})", ids.len())).color(color).strong());
                            for id in ids.iter().take(30) {
                                ui.label(RichText::new(format!("    {id}")).monospace().size(12.0));
                            }
                        };
                        ui.label(dim("Since the run before".to_owned()));
                        list(ui, "Broke", &c.broke, red());
                        list(ui, "Fixed", &c.fixed, green());
                        let slower: Vec<String> =
                            c.slower.iter().map(|(id, a, b)| format!("{id}  {a} → {b} ms")).collect();
                        list(ui, "Slower", &slower, pal.modified);
                        list(ui, "New", &c.added, pal.text);
                        list(ui, "Gone", &c.removed, pal.text_dim);
                        if !said {
                            ui.label("Nothing changed.");
                        }
                        ui.add_space(8.0);
                    }
                    let failed = run.failed();
                    if failed.is_empty() {
                        ui.label(RichText::new("Every test passed.").color(green()));
                        return;
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Failed ({})", failed.len())).color(red()).strong());
                        let cargo = run.runner.starts_with("cargo");
                        let rerun = ui
                            .add_enabled(cargo, egui::Button::new("Run the failed ones again"))
                            .on_disabled_hover_text("For Cargo's tests; run other runners' tests again from Testing");
                        if rerun.clicked() {
                            action = Some(Action::RerunFailed(failed.iter().map(|t| t.id.clone()).collect()));
                        }
                    });
                    for t in failed {
                        let past = history.results_of(&t.id);
                        let streak = past.iter().rev().take_while(|o| **o == Outcome::Failed).count();
                        let note =
                            if streak > 1 { format!("  (failing {streak} runs in a row)") } else { String::new() };
                        ui.label(RichText::new(format!("{}{note}", t.id)).monospace().size(12.0));
                        if !t.message.is_empty() {
                            ui.label(dim(format!("    {}", t.message)));
                        }
                    }
                });
            }
            Tab::Flaky => {
                if flaky.is_empty() {
                    ui.label(format!(
                        "No flaky tests: none both passed and failed in its last {} runs.",
                        run_history::FLAKY_WINDOW
                    ));
                    return;
                }
                ui.label(dim(format!(
                    "Tests that both passed and failed in their last {} runs, those that flip most first. Each dot \
                     is a run, oldest first.",
                    run_history::FLAKY_WINDOW
                )));
                ui.add_space(6.0);
                egui::Grid::new("flaky_tests").striped(true).num_columns(3).show(ui, |ui| {
                    for f in &flaky {
                        ui.label(RichText::new(&f.id).monospace().size(12.0));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for o in &f.recent {
                                ui.label(RichText::new("●").color(outcome_color(*o)));
                            }
                        });
                        ui.label(dim(format!("{} passed, {} failed", f.passed, f.failed)));
                        ui.end_row();
                    }
                });
            }
            Tab::Builds => {
                if history.builds.is_empty() {
                    ui.label("No builds yet: Cargo checks, and builds and programs started with Run, are kept here.");
                    return;
                }
                let failed = history.builds.iter().filter(|b| !b.success).count();
                ui.label(dim(format!("{} kept, {failed} failed. Newest first.", history.builds.len())));
                ui.add_space(6.0);
                egui::Grid::new("build_history").striped(true).num_columns(5).show(ui, |ui| {
                    for b in history.builds.iter().rev() {
                        let (mark, color) = if b.success { ("✔", green()) } else { ("✖", red()) };
                        ui.label(RichText::new(mark).color(color));
                        ui.label(&b.what);
                        ui.label(dim(format!(
                            "{}{}",
                            recent::ago(b.at, now),
                            b.seconds.map(|s| format!(" · {}", seconds_text(s))).unwrap_or_default()
                        )));
                        let mut counts = Vec::new();
                        if b.errors > 0 {
                            counts.push(plural(b.errors, "error"));
                        }
                        if b.warnings > 0 {
                            counts.push(plural(b.warnings, "warning"));
                        }
                        if let Some(code) = b.exit.filter(|c| *c != 0) {
                            counts.push(format!("exit {code}"));
                        }
                        ui.label(counts.join(", "));
                        match &b.first_error {
                            Some((path, line, message)) => {
                                let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
                                let first = message.lines().next().unwrap_or("");
                                if ui.link(format!("{name}:{line}  {first}")).clicked() {
                                    action = Some(Action::Open(path.clone(), *line as usize));
                                }
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
            }
        });
        action
    }
}

/// The failure's first line, for the history (a whole backtrace would bloat it).
pub(crate) fn first_line(text: &str) -> String {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(200).collect()
}

/// A test's id as the history keeps it: `suite::name`, or the name alone.
pub(crate) fn test_id(suite: &str, name: &str) -> String {
    if suite.is_empty() || name.starts_with(suite) { name.to_owned() } else { format!("{suite}::{name}") }
}

/// The build's first error, from what a run's output was found to say.
pub(crate) fn first_error(
    found: &[(PathBuf, coiman_core::language::lsp::types::Diagnostic)],
) -> Option<(PathBuf, u32, String)> {
    found
        .iter()
        .find(|(_, d)| d.severity == Some(coiman_core::language::lsp::types::DiagnosticSeverity::ERROR))
        .map(|(p, d)| (p.clone(), d.range.start.line + 1, first_line(&d.message)))
}
