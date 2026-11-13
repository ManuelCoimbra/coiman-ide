//! The Testing view for everything but Cargo: finds a project's runners, runs them,
//! and shows the results the same way whichever runner it was.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use coiman_core::build::cargo::Outcome as CargoOutcome;
use coiman_core::language::lsp::discovery::{kill_tree, quiet_command};
use coiman_core::language::lsp::types as lt;
use coiman_core::launch::testing::{self, Adapter, Collect, Format, Outcome, TestResult};
use coiman_core::{launch::stacktrace, project::trust};
use eframe::egui::{self, Align, Color32, RichText, Sense};

use crate::app::cargo::{outcome_icon, shell};
use crate::app::lsp::path_key;
use crate::app::{CodeApp, repaint};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

/// Seconds between looks for new tests, while the view is open.
const RESCAN_EVERY: f64 = 30.0;

/// A test to run alone: its file, id and line.
type OneTest = (PathBuf, String, usize);

struct Scan {
    root: PathBuf,
    adapters: Vec<Adapter>,
    tests: Vec<(PathBuf, Vec<(usize, String)>)>,
    files: Vec<PathBuf>,
    problem: Option<String>,
}

struct Run {
    adapter: usize,
    /// One test, or all of them.
    single: bool,
    child: Child,
    lines: Receiver<String>,
    captured: String,
    report: PathBuf,
    format: Format,
    report_dir: Option<PathBuf>,
    started: SystemTime,
    /// Where it was told to write coverage.
    coverage: Option<PathBuf>,
}

enum Row {
    Heading { adapter: usize, name: String },
    Group(String),
    Test { adapter: usize, file: Option<PathBuf>, id: String, line: usize, result: Option<usize> },
}

#[derive(Default)]
pub(crate) struct Tests {
    root: Option<PathBuf>,
    scanning: Option<Receiver<Scan>>,
    next_scan: f64,
    pub(crate) adapters: Vec<Adapter>,
    tests: Vec<(PathBuf, Vec<(usize, String)>)>,
    files: Vec<PathBuf>,
    results: Vec<Vec<TestResult>>,
    run: Option<Run>,
    rows: Vec<Row>,
    /// What the gutter shows: (file, line) to its outcome.
    outcomes: HashMap<(PathBuf, usize), Outcome>,
    selected: Option<(usize, usize)>,
    only_failed: bool,
    notice: Option<String>,
    problem: Option<String>,
    /// Runners still to go, when all of them were asked for.
    queue: Vec<usize>,
    /// Failures, as problems: by [`path_key`], the path, and each one.
    problems: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    /// Results from a past session have been looked for.
    restored: bool,
    /// With Cargo in the same project, which view shows.
    pub(crate) show_others: bool,
}

/// Where a project's last results are kept between sessions.
fn results_file(root: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path_key(root).hash(&mut hash);
    let name = format!("{:016x}.json", hash.finish());
    Some(coiman_core::settings::config_dir()?.join("test_results").join(name))
}

fn scan(root: PathBuf) -> Scan {
    let files = coiman_core::launch::stacktrace::project_files(&root);
    let (adapters, problem) = match testing::detect(&root, &files) {
        Ok(found) => (found, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let wanted: Vec<&str> = adapters.iter().flat_map(|a| a.extensions.iter().map(String::as_str)).collect();
    let mut tests = Vec::new();
    for file in &files {
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !wanted.contains(&ext.as_str()) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(file) else { continue };
        let found = testing::discover_in_file(file, &source);
        if !found.is_empty() {
            tests.push((file.clone(), found));
        }
    }
    tests.sort();
    Scan { root, adapters, tests, files, problem }
}

fn to_cargo(outcome: Option<Outcome>) -> Option<CargoOutcome> {
    outcome.map(|o| match o {
        Outcome::Passed => CargoOutcome::Passed,
        Outcome::Failed => CargoOutcome::Failed,
        Outcome::Skipped => CargoOutcome::Ignored,
    })
}

fn duration(millis: Option<u64>) -> String {
    match millis {
        Some(ms) if ms >= 1000 => format!("{:.1} s", ms as f64 / 1000.0),
        Some(ms) => format!("{ms} ms"),
        None => String::new(),
    }
}

impl Tests {
    /// Sets out the view again, after new tests or new results.
    fn rebuild(&mut self) {
        self.rows.clear();
        self.outcomes.clear();
        for (a, adapter) in self.adapters.iter().enumerate() {
            let results = self.results.get(a).map_or(&[][..], Vec::as_slice);
            let mut used = vec![false; results.len()];
            self.rows.push(Row::Heading { adapter: a, name: adapter.name.clone() });
            for (file, found) in &self.tests {
                let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                if !adapter.extensions.contains(&ext) || !file.starts_with(&adapter.cwd) {
                    continue;
                }
                let name = self.root.as_ref().and_then(|r| file.strip_prefix(r).ok()).unwrap_or(file);
                self.rows.push(Row::Group(name.to_string_lossy().replace('\\', "/")));
                for (line, id) in found {
                    // With several results, parameters say, a failure speaks for them.
                    let result = results
                        .iter()
                        .position(|r| r.outcome == Outcome::Failed && testing::matches(r, file, id))
                        .or_else(|| results.iter().position(|r| testing::matches(r, file, id)));
                    if let Some(at) = result {
                        used[at] = true;
                        for (i, r) in results.iter().enumerate() {
                            if testing::matches(r, file, id) {
                                used[i] = true;
                            }
                        }
                        self.outcomes.insert((file.clone(), *line), results[at].outcome);
                    }
                    self.rows.push(Row::Test {
                        adapter: a,
                        file: Some(file.clone()),
                        id: id.clone(),
                        line: *line,
                        result,
                    });
                }
            }
            // What the runner reported that reading the text didn't find.
            let mut last_suite = None;
            for (i, result) in results.iter().enumerate().filter(|(i, _)| !used[*i]) {
                if last_suite != Some(result.suite.as_str()) {
                    last_suite = Some(result.suite.as_str());
                    let title = if result.suite.is_empty() { "Other results" } else { result.suite.as_str() };
                    self.rows.push(Row::Group(title.to_owned()));
                }
                self.rows.push(Row::Test {
                    adapter: a,
                    file: None,
                    id: result.name.clone(),
                    line: result.line.map_or(0, |l| l as usize),
                    result: Some(i),
                });
            }
        }
    }

    fn counts(&self) -> (usize, usize, usize) {
        let mut totals = (0, 0, 0);
        for result in self.results.iter().flatten() {
            match result.outcome {
                Outcome::Passed => totals.0 += 1,
                Outcome::Failed => totals.1 += 1,
                Outcome::Skipped => totals.2 += 1,
            }
        }
        totals
    }
}

impl CodeApp {
    fn tests_root(&self) -> Option<PathBuf> {
        self.explorer.root.clone()
    }

    /// Whether there's a Testing view to show.
    pub(crate) fn testing_available(&self) -> bool {
        self.cargo_project || !self.tests.adapters.is_empty()
    }

    pub(crate) fn pump_tests(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.tests.scanning
            && let Ok(done) = rx.try_recv()
        {
            self.tests.scanning = None;
            if self.tests.root.as_ref() == Some(&done.root) {
                // Results stay with their adapter when the list comes back the same.
                if self.tests.adapters != done.adapters {
                    self.tests.results = vec![Vec::new(); done.adapters.len()];
                }
                self.tests.adapters = done.adapters;
                self.tests.tests = done.tests;
                self.tests.files = done.files;
                self.tests.problem = done.problem;
                if !std::mem::replace(&mut self.tests.restored, true) {
                    self.restore_test_results();
                }
                self.tests.rebuild();
            }
        }
        self.pump_test_run(ctx);
        let root = self.tests_root();
        let now = ctx.input(|i| i.time);
        let moved = root != self.tests.root;
        let open = (self.sidebar_visible && self.sidebar_view == crate::app::SidebarView::Testing)
            || self.rails.right_view == Some(crate::app::SidebarView::Testing);
        let due = open && now >= self.tests.next_scan;
        if !moved && !due {
            return;
        }
        if moved {
            self.tests = crate::app::test_panel::Tests { root: root.clone(), ..Default::default() };
        }
        self.tests.next_scan = now + RESCAN_EVERY;
        // Finding out which runners work means starting the project's own Python.
        let Some(root) = root.filter(|r| trust::is_trusted(r)) else { return };
        if self.tests.scanning.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(scan(root));
            wake();
        });
        self.tests.scanning = Some(rx);
    }

    fn restore_test_results(&mut self) {
        let Some(path) = self.tests.root.as_deref().and_then(results_file) else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(saved) = serde_json::from_str::<Vec<Vec<TestResult>>>(&text) else { return };
        if saved.len() == self.tests.adapters.len() && saved.iter().any(|r| !r.is_empty()) {
            self.tests.results = saved;
            self.tests.notice = Some("Results from the last run, in an earlier session".to_owned());
            self.rebuild_test_problems();
        }
    }

    fn save_test_results(&self) {
        let Some(path) = self.tests.root.as_deref().and_then(results_file) else { return };
        let Ok(text) = serde_json::to_string(&self.tests.results) else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }

    /// Failures show in Problems too, where the code went wrong.
    fn rebuild_test_problems(&mut self) {
        let root = self.tests.root.clone().unwrap_or_default();
        let mut problems: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)> = HashMap::new();
        for (adapter, results) in self.tests.results.iter().enumerate() {
            let source = self.tests.adapters.get(adapter).map(|a| a.name.clone());
            for result in results.iter().filter(|r| r.outcome == Outcome::Failed) {
                let Some((path, line)) = testing::failure_place(result, &root, &self.tests.files) else { continue };
                let first = result.message.lines().next().unwrap_or("failed");
                let at = lt::Position::new(line.saturating_sub(1), 0);
                let problem = lt::Diagnostic {
                    range: lt::Range::new(at, lt::Position::new(at.line, 1000)),
                    severity: Some(lt::DiagnosticSeverity::ERROR),
                    source: source.clone(),
                    message: format!("{}: {first}", result.name),
                    ..Default::default()
                };
                problems.entry(path_key(&path)).or_insert_with(|| (path, Vec::new())).1.push(problem);
            }
        }
        for k in problems.keys() {
            self.cargo.stale_problems.remove(k);
        }
        self.tests.problems = problems;
    }

    pub(crate) fn test_problems(&self, file: &Path) -> Vec<lt::Diagnostic> {
        if self.cargo.stale_problems.contains(file) {
            return Vec::new();
        }
        self.tests.problems.get(file).map(|(_, d)| d.clone()).unwrap_or_default()
    }

    pub(crate) fn test_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        let fresh = self.tests.problems.iter().filter(|(k, _)| !self.cargo.stale_problems.contains(*k));
        fresh.map(|(k, (p, _))| (k.clone(), p.clone())).collect()
    }

    /// Runs a project's tests in full, or one of them.
    fn start_test_run(&mut self, ctx: &egui::Context, adapter: usize, one: Option<OneTest>) {
        self.start_tests(ctx, adapter, one, false);
    }

    fn start_tests(&mut self, ctx: &egui::Context, adapter: usize, one: Option<OneTest>, with_coverage: bool) {
        self.cancel_test_run();
        self.cargo.stale_problems.clear();
        let Some(spec) = self.tests.adapters.get(adapter).cloned() else { return };
        if !self.allowed_to_run(&spec.cwd) {
            return;
        }
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() && self.docs[i].path.is_some() && !self.save_doc(i) {
                self.status = Some(format!("Nothing was run: {} couldn't be saved", self.docs[i].title()));
                return;
            }
        }
        let report = testing::report_file(spec.format);
        let root = self.tests.root.clone().unwrap_or_else(|| spec.cwd.clone());
        let single = one.is_some();
        let command = match &one {
            Some((file, id, line)) => {
                let Some(command) = spec.command_for_one(&report, &root, file, id, *line) else {
                    self.status = Some(format!("{} can't run a single test", spec.name));
                    return;
                };
                command
            }
            None => spec.command_for_all(&report),
        };
        let folder = with_coverage.then(CodeApp::coverage_folder);
        let command = match &folder {
            Some(folder) => spec.command_with_coverage(&report, folder).unwrap_or(command),
            None => command,
        };
        let (program, args) = shell(&command);
        let spawned = quiet_command(Path::new(&program))
            .args(&args)
            .current_dir(&spec.cwd)
            .envs(self.environment_variables())
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                self.status = Some(format!("Could not start {}: {e}", spec.name));
                return;
            }
        };
        let (tx, lines) = mpsc::channel();
        let pipes: Vec<Box<dyn std::io::Read + Send>> = [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        .collect();
        for pipe in pipes {
            let (tx, wake) = (tx.clone(), repaint(ctx));
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                    wake();
                }
            });
        }
        self.begin_output(command);
        self.tests.notice = None;
        self.tests.run = Some(Run {
            adapter,
            single,
            child,
            lines,
            captured: String::new(),
            report,
            format: spec.format,
            report_dir: spec.report_dir.as_ref().map(|d| spec.cwd.join(d)),
            started: SystemTime::now(),
            coverage: folder,
        });
    }

    fn pump_test_run(&mut self, ctx: &egui::Context) {
        let Some(run) = &mut self.tests.run else { return };
        let mut got = Vec::new();
        let ended = loop {
            match run.lines.try_recv() {
                Ok(line) => got.push(line),
                Err(mpsc::TryRecvError::Empty) => break false,
                Err(mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        for line in &got {
            run.captured.push_str(line);
            run.captured.push('\n');
        }
        let readable = run.format == Format::GoJson;
        for line in got {
            // Go's events carry what was printed; the rest of them are for the view.
            let text = if readable {
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(event) => match event["Output"].as_str() {
                        Some(output) => output.trim_end().to_owned(),
                        None => continue,
                    },
                    Err(_) => line,
                }
            } else {
                line
            };
            self.push_output(text);
        }
        if !ended {
            return;
        }
        let Some(mut run) = self.tests.run.take() else { return };
        let code = run.child.wait().ok().and_then(|s| s.code());
        let found = testing::collect(&Collect {
            format: run.format,
            stdout: &run.captured,
            report: &run.report,
            report_dir: run.report_dir.clone(),
            started: run.started,
        });
        let _ = std::fs::remove_file(&run.report);
        let name = self.tests.adapters.get(run.adapter).map_or(String::new(), |a| a.name.clone());
        if found.is_empty() {
            // A run that can't start or build still has exit codes to say so with.
            let code = code.map_or("no exit code".to_owned(), |c| format!("exit {c}"));
            self.tests.notice = Some(format!("{name} reported no results ({code}): see Output."));
        } else if let Some(slot) = self.tests.results.get_mut(run.adapter) {
            if run.single {
                slot.retain(|old| !found.iter().any(|new| new.name == old.name && new.suite == old.suite));
                slot.extend(found);
            } else {
                *slot = found;
            }
        }
        if let Some(folder) = &run.coverage {
            self.take_run_coverage(ctx, folder, &name);
        }
        self.tests.selected = None;
        self.tests.rebuild();
        self.rebuild_test_problems();
        self.save_test_results();
        let (passed, failed, skipped) = self.tests.counts();
        self.status = Some(format!("Tests: {passed} passed, {failed} failed, {skipped} skipped"));
        if !self.tests.queue.is_empty() {
            let next = self.tests.queue.remove(0);
            self.start_test_run(ctx, next, None);
        }
    }

    pub(crate) fn cancel_test_run(&mut self) {
        if let Some(mut run) = self.tests.run.take() {
            kill_tree(&mut run.child);
            let _ = std::fs::remove_file(&run.report);
            self.tests.queue.clear();
            self.tests.notice = Some("Test run stopped".to_owned());
        }
    }

    /// `Test: Run All Tests` when this view is the one that applies.
    pub(crate) fn run_all_other_tests(&mut self, ctx: &egui::Context) -> bool {
        if self.tests.adapters.is_empty() {
            return false;
        }
        self.start_test_run(ctx, 0, None);
        // Each runner in turn, so "all" means all of them.
        self.tests.queue = (1..self.tests.adapters.len()).collect();
        true
    }

    /// A gutter button on a test: through its runner, if it has one.
    pub(crate) fn run_one_test(&mut self, ctx: &egui::Context, file: &Path, id: &str, line: usize) -> bool {
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(adapter) = self
            .tests
            .adapters
            .iter()
            .position(|a| a.single.is_some() && a.extensions.contains(&ext) && file.starts_with(&a.cwd))
        else {
            return false;
        };
        self.start_test_run(ctx, adapter, Some((file.to_path_buf(), id.to_owned(), line)));
        true
    }

    /// The last result of the test on this line of a file.
    pub(crate) fn test_outcome_at(&self, file: &Path, line: usize) -> Option<(Icon, Color32)> {
        let outcome = self.tests.outcomes.get(&(file.to_path_buf(), line)).copied()?;
        Some(outcome_icon(to_cargo(Some(outcome))))
    }

    /// With Cargo too, a choice between the two lists. Says whether this one shows.
    pub(crate) fn showing_other_tests(&mut self, ui: &mut egui::Ui) -> bool {
        if !self.cargo_project {
            return true;
        }
        if self.tests.adapters.is_empty() {
            return false;
        }
        ui.horizontal(|ui| {
            if ui.selectable_label(!self.tests.show_others, "Cargo").clicked() {
                self.tests.show_others = false;
            }
            let others: Vec<&str> = self.tests.adapters.iter().map(|a| a.name.as_str()).collect();
            if ui.selectable_label(self.tests.show_others, others.join(", ")).clicked() {
                self.tests.show_others = true;
            }
        });
        self.tests.show_others
    }

    pub(crate) fn other_tests_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        let running = self.tests.run.as_ref().map(|r| r.adapter);
        let mut start: Option<(usize, Option<OneTest>)> = None;
        let mut open: Option<(PathBuf, usize)> = None;
        let mut select = None;
        let mut rescan = false;
        let mut cover = None;
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("TESTING").small().strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                rescan = icons::button(ui, Icon::Refresh, "Find Tests Again").clicked();
                if running.is_some() {
                    cancel = icons::button(ui, Icon::Stop, "Cancel Test Run").clicked();
                }
            });
        });
        let (passed, failed, skipped) = self.tests.counts();
        if passed + failed + skipped > 0 {
            ui.label(format!("{passed} passed, {failed} failed, {skipped} skipped"));
            ui.checkbox(&mut self.tests.only_failed, "Only failures");
        }
        let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
        for line in [&self.tests.notice, &self.tests.problem].into_iter().flatten() {
            ui.add(egui::Label::new(RichText::new(line).color(warn)).wrap());
        }
        if self.tests.adapters.is_empty() {
            let text =
                if self.tests.scanning.is_some() { "Looking for tests…" } else { "No test runner found here." };
            ui.label(RichText::new(text).color(pal.text_dim));
        }
        // A floating bar would cover the Run buttons when it widens.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        let detail_height = if self.tests.selected.is_some() { 170.0 } else { 0.0 };
        let list_height = (ui.available_height() - detail_height).max(60.0);
        let only_failed = self.tests.only_failed;
        let size = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical().id_salt("other_tests").max_height(list_height).auto_shrink([false, false]).show(
            ui,
            |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                let results = &self.tests.results;
                for row in &self.tests.rows {
                    match row {
                        Row::Heading { adapter, name } => {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(name).strong());
                                if running == Some(*adapter) {
                                    ui.label(RichText::new("running…").small().color(pal.text_dim));
                                }
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if icons::button(ui, Icon::Play, "Run All").clicked() {
                                        start = Some((*adapter, None));
                                    }
                                    let can = self.tests.adapters.get(*adapter).is_some_and(|a| {
                                        a.command_with_coverage(Path::new("r"), Path::new("c")).is_some()
                                    });
                                    if can
                                        && ui.small_button("Coverage").on_hover_text("Run all, with coverage").clicked()
                                    {
                                        cover = Some(*adapter);
                                    }
                                });
                            });
                        }
                        Row::Group(title) => {
                            if !only_failed {
                                ui.label(RichText::new(title).small().color(pal.text_dim));
                            }
                        }
                        Row::Test { adapter, file, id, line, result } => {
                            let found = result.and_then(|r| results.get(*adapter)?.get(r));
                            if only_failed && !found.is_some_and(|r| r.outcome == Outcome::Failed) {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                let (icon, color) = outcome_icon(to_cargo(found.map(|r| r.outcome)));
                                let (icon, color) =
                                    if icon == Icon::Play { (Icon::Beaker, pal.text_dim) } else { (icon, color) };
                                icons::show(ui, icon, size, color);
                                if ui.add(egui::Label::new(id.as_str()).sense(Sense::click())).clicked() {
                                    if let Some(r) = result {
                                        select = Some((*adapter, *r));
                                    }
                                    if let Some(file) = file {
                                        open = Some((file.clone(), *line));
                                    }
                                }
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if let Some(file) = file
                                        && icons::button(ui, Icon::Play, "Run").clicked()
                                    {
                                        start = Some((*adapter, Some((file.clone(), id.clone(), *line))));
                                    }
                                    let time = duration(found.and_then(|r| r.millis));
                                    ui.label(RichText::new(time).small().color(pal.text_dim));
                                });
                            });
                        }
                    }
                }
            },
        );
        // What a selected test printed, and a way to where it went wrong.
        let mut jump = None;
        if let Some((a, r)) = self.tests.selected {
            let result = self.tests.results.get(a).and_then(|list| list.get(r));
            if let Some(result) = result {
                ui.separator();
                ui.label(RichText::new(&result.name).strong());
                let root = self.tests.root.clone().unwrap_or_default();
                if let Some(place) = testing::failure_place(result, &root, &self.tests.files) {
                    let name = place.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.link(format!("{name}:{}", place.1)).clicked() {
                        jump = Some(place);
                    }
                }
                egui::ScrollArea::vertical().id_salt("test_message").max_height(120.0).show(ui, |ui| {
                    let text = if result.message.is_empty() { "No message." } else { result.message.trim_end() };
                    for line in text.lines() {
                        let framed = stacktrace::parse_frame(line).and_then(|f| {
                            match stacktrace::locate(&f, &root, &self.tests.files) {
                                stacktrace::Place::Project(path) => Some((path, f.line)),
                                _ => None,
                            }
                        });
                        let rich = RichText::new(line).monospace().small();
                        match framed {
                            Some(place) => {
                                if ui.add(egui::Label::new(rich.color(pal.accent)).sense(Sense::click())).clicked() {
                                    jump = Some(place);
                                }
                            }
                            None => {
                                ui.label(rich);
                            }
                        }
                    }
                });
            }
        }
        if let Some(chosen) = select {
            self.tests.selected = Some(chosen);
        }
        if cancel {
            self.cancel_test_run();
        }
        if rescan {
            self.tests.next_scan = 0.0;
        }
        if let Some((adapter, one)) = start {
            self.start_test_run(&ctx, adapter, one);
        }
        if let Some(adapter) = cover {
            self.start_tests(&ctx, adapter, None, true);
        }
        if let Some((path, line)) = jump {
            let at = lt::Position::new(line.saturating_sub(1), 0);
            self.jump_to(&path, lt::Range::new(at, at), false);
        } else if let Some((path, line)) = open {
            self.open_file(&path);
            if let Some(doc) = self.active_doc() {
                doc.goto_line(line + 1);
            }
        }
    }
}
