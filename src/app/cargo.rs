//! Cargo: run configurations, the test explorer (run tests from the Testing
//! view, the gutter or at the cursor, with `cargo test` or `cargo nextest`),
//! and `cargo check`/`clippy` results in Problems. Cargo runs in the
//! background and its output streams into the Output panel.

use crate::commands::Command;
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use egui::{Align, Color32, RichText, Sense};
use rust_code_core::cargo::{
    self, CargoDiagnostic, FileTarget, Outcome, OutputParser, RunConfig, Runner, TestItem, TestScope,
};
use rust_code_core::lsp::discovery::quiet_command;
use rust_code_core::lsp::status::waits_for_build_lock;
use rust_code_core::lsp::types as lt;
use rust_code_core::run;

use crate::icons::{self, Icon};
use crate::palette::PickerKind;
use crate::terminal::Task;
use crate::theme;

use super::{CodeApp, PanelTab, SidebarView, repaint};

/// Output panel lines kept.
const MAX_OUTPUT: usize = 20_000;
/// Seconds between re-reading tests from the active document while typing.
const PARSE_INTERVAL: f64 = 0.5;

/// Tests found in one file.
pub(super) struct FileTests {
    path: PathBuf,
    target: FileTarget,
    items: Vec<TestItem>,
}

enum JobKind {
    Tests(Runner),
    Check { tool: &'static str, workspace: PathBuf },
}

/// A cargo process running in the background.
struct Job {
    kind: JobKind,
    child: Child,
    /// (from stdout, line)
    lines: Receiver<(bool, String)>,
    parser: OutputParser,
    found: Vec<CargoDiagnostic>,
    /// Results seen in this run: (passed, failed).
    counts: (usize, usize),
}

enum Msg {
    /// Run configurations; `pick` opens the picker when they arrive.
    Configs(Vec<RunConfig>, Option<String>, bool),
    Scanned(PathBuf, Vec<(PathBuf, FileTarget, Vec<TestItem>)>),
    Nextest(bool),
}

/// Commands that only work in a Cargo project: hidden everywhere else.
pub(super) const CARGO_ONLY: &[Command] = &[
    Command::StartDebugging,
    Command::StopDebugging,
    Command::RestartDebugging,
    Command::StepOver,
    Command::StepInto,
    Command::StepOut,
    Command::PauseDebugging,
    Command::ToggleBreakpoint,
    Command::ConditionalBreakpoint,
    Command::AddLogpoint,
    Command::ShowDbgOutput,
    Command::RemoveDbgFromFile,
    Command::RemoveDbgFromWorkspace,
    Command::ShowDebug,
    Command::ShowDebugConsole,
    Command::DebugTestAtCursor,
    Command::ShowTesting,
    Command::TestAtCursor,
    Command::TestCurrentFile,
    Command::TestAll,
    Command::CancelTests,
    Command::CargoCheck,
    Command::CargoClippy,
];

pub(super) struct CargoState {
    /// Programs offered by Run Program when several fit.
    run_targets: Vec<run::RunPlan>,
    /// Tests by file key.
    tests: HashMap<PathBuf, FileTests>,
    scanned_for: Option<PathBuf>,
    /// (doc uid, buffer version, time) the active document's tests were read at.
    parsed: Option<(u64, u64, f64)>,
    /// Last outcome by (test binary, test path).
    results: HashMap<(String, String), Outcome>,
    job: Option<Job>,
    output: Vec<String>,
    output_title: String,
    /// The running job said it is waiting for the build folder's lock.
    pub(super) lock_wait: bool,
    /// Diagnostics from the last `Cargo: Check`/`Clippy`, by file key.
    pub(super) diagnostics: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    configs: Vec<RunConfig>,
    selected: Option<String>,
    nextest: Option<bool>,
    collapsed: HashSet<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for CargoState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            run_targets: Vec::new(),
            tests: HashMap::new(),
            scanned_for: None,
            parsed: None,
            results: HashMap::new(),
            job: None,
            output: Vec::new(),
            output_title: String::new(),
            lock_wait: false,
            diagnostics: HashMap::new(),
            configs: Vec::new(),
            selected: None,
            nextest: None,
            collapsed: HashSet::new(),
            tx,
            rx,
        }
    }
}

fn key(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().to_lowercase()) } else { path.to_path_buf() }
}

fn outcome_icon(outcome: Option<Outcome>) -> (Icon, Color32) {
    match outcome {
        None => (Icon::Play, Color32::from_rgb(0x89, 0xD1, 0x85)),
        Some(Outcome::Running) => (Icon::Dot, Color32::from_rgb(0xE2, 0xC0, 0x8D)),
        Some(Outcome::Passed) => (Icon::Check, Color32::from_rgb(0x73, 0xC9, 0x91)),
        Some(Outcome::Failed) => (Icon::Close, Color32::from_rgb(0xF1, 0x4C, 0x4C)),
        Some(Outcome::Ignored) => (Icon::Skip, Color32::from_gray(0x99)),
    }
}

/// A shell running `command`, so run configurations can use pipes and `&&`.
fn shell(command: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("powershell.exe".into(), vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), command.into()])
    } else {
        ("sh".into(), vec!["-c".into(), command.into()])
    }
}

impl CodeApp {
    /// The Cargo project for run/test/check: the open folder if it has a
    /// Cargo.toml, else the package of the active file.
    fn cargo_root(&self) -> Option<PathBuf> {
        if let Some(root) = self.explorer.root.as_ref().filter(|r| r.join("Cargo.toml").exists()) {
            return Some(root.clone());
        }
        let path = self.docs.get(self.active)?.path.as_ref()?;
        path.ancestors().skip(1).find(|d| d.join("Cargo.toml").exists()).map(cargo::workspace_dir)
    }

    fn runner(&mut self, ctx: &egui::Context) -> Runner {
        match self.settings.cargo.test_runner.as_str() {
            "cargo" => Runner::Cargo,
            "nextest" => Runner::Nextest,
            _ => {
                if self.cargo.nextest.is_none() {
                    // Find out once, in the background; use cargo test meanwhile.
                    self.cargo.nextest = Some(false);
                    let (tx, wake) = (self.cargo.tx.clone(), repaint(ctx));
                    std::thread::spawn(move || {
                        let _ = tx.send(Msg::Nextest(cargo::has_nextest()));
                        wake();
                    });
                }
                if self.cargo.nextest == Some(true) { Runner::Nextest } else { Runner::Cargo }
            }
        }
    }

    // ----- background ------------------------------------------------------------------

    /// Runs every frame: messages, the running job, tests of the active file.
    pub(super) fn pump_cargo(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.cargo.rx.try_recv() {
            match msg {
                Msg::Configs(configs, error, pick) => {
                    if let Some(e) = error {
                        self.status = Some(e);
                    }
                    self.cargo.configs = configs;
                    if pick {
                        self.pick_run_config();
                    }
                }
                Msg::Scanned(root, found) => {
                    if self.cargo.scanned_for.as_ref() == Some(&root) {
                        self.cargo.tests = found
                            .into_iter()
                            .map(|(path, target, items)| (key(&path), FileTests { path, target, items }))
                            .collect();
                        self.cargo.parsed = None;
                    }
                }
                Msg::Nextest(found) => self.cargo.nextest = Some(found),
            }
        }
        self.pump_job(ctx);

        // Keep the active Rust file's tests current while it is edited.
        let now = ctx.input(|i| i.time);
        if let Some(doc) = self.docs.get(self.active)
            && let Some(path) = doc.path.clone()
            && doc.extension() == "rs"
        {
            let (uid, version) = (doc.uid, doc.buffer.version());
            let stale =
                self.cargo.parsed.is_none_or(|(u, v, t)| u != uid || (v != version && now - t > PARSE_INTERVAL));
            if stale {
                self.cargo.parsed = Some((uid, version, now));
                let k = key(&path);
                let target = self.cargo.tests.get(&k).map(|f| f.target.clone()).or_else(|| cargo::file_target(&path));
                if let Some(target) = target {
                    let items = cargo::discover_tests(&doc.text(), &target.modules);
                    if items.is_empty() {
                        self.cargo.tests.remove(&k);
                    } else {
                        self.cargo.tests.insert(k, FileTests { path, target, items });
                    }
                }
            } else if self.cargo.parsed.is_some_and(|(u, v, _)| u == uid && v != version) {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(PARSE_INTERVAL));
            }
        }
    }

    /// Re-reads every test in the project (Testing view, refresh).
    fn scan_tests(&mut self, ctx: &egui::Context) {
        let Some(root) = self.cargo_root() else { return };
        self.cargo.scanned_for = Some(root.clone());
        let (tx, wake) = (self.cargo.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let found = cargo::scan_workspace(&root);
            let _ = tx.send(Msg::Scanned(root, found));
            wake();
        });
    }

    fn push_output(&mut self, line: String) {
        self.cargo.output.push(line);
        if self.cargo.output.len() > MAX_OUTPUT {
            self.cargo.output.drain(..self.cargo.output.len() - MAX_OUTPUT);
        }
    }

    /// Keeps `cargo_project` current: when the folder or file changes, and every
    /// few seconds in case a `Cargo.toml` appeared or went away.
    pub(super) fn refresh_cargo_project(&mut self, ctx: &egui::Context) {
        let key = self.explorer.root.clone().or_else(|| self.docs.get(self.active)?.path.clone());
        let now = ctx.input(|i| i.time);
        if key != self.cargo_project_for.0 || now >= self.cargo_project_for.1 {
            self.cargo_project_for = (key, now + 3.0);
            self.cargo_project = self.cargo_root_dir().is_some();
        }
        if !self.cargo_project && matches!(self.sidebar_view, SidebarView::Testing | SidebarView::Debug) {
            self.sidebar_view = SidebarView::Explorer;
            self.status = Some("Testing and debugging work in Cargo projects".to_owned());
        }
    }

    /// Starts `cargo <args>` in `cwd` as the background job (replacing any).
    fn start_job(&mut self, ctx: &egui::Context, kind: JobKind, args: Vec<String>, cwd: &Path) {
        self.cancel_job();
        if !self.allowed_to_run(cwd) {
            return;
        }
        let title = format!("cargo {}", args.join(" "));
        let spawned = quiet_command(Path::new("cargo"))
            .args(&args)
            .current_dir(cwd)
            .env("CARGO_TERM_COLOR", "never")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(e) => {
                self.status = Some(format!("Could not run cargo: {e}"));
                return;
            }
        };
        let (tx, lines) = mpsc::channel();
        for (pipe, stdout) in [
            (child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), true),
            (child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), false),
        ] {
            let Some(pipe) = pipe else { continue };
            let (tx, wake) = (tx.clone(), repaint(ctx));
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if tx.send((stdout, line)).is_err() {
                        break;
                    }
                    wake();
                }
            });
        }
        self.cargo.output.clear();
        self.cargo.output.push(format!("> {title}"));
        self.cargo.output_title = title;
        self.cargo.job =
            Some(Job { kind, child, lines, parser: OutputParser::default(), found: Vec::new(), counts: (0, 0) });
        self.panel_visible = true;
        self.panel_tab = PanelTab::Output;
    }

    pub(super) fn cancel_job(&mut self) {
        self.cargo.lock_wait = false;
        if let Some(mut job) = self.cargo.job.take() {
            let _ = job.child.kill();
            self.push_output("[cancelled]".to_owned());
            self.cargo.results.retain(|_, o| *o != Outcome::Running);
        }
    }

    fn pump_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &mut self.cargo.job else { return };
        let mut new_lines = Vec::new();
        while let Ok((stdout, line)) = job.lines.try_recv() {
            // Any later output means the wait is over.
            if !stdout {
                self.cargo.lock_wait = waits_for_build_lock(&line);
            }
            match &job.kind {
                JobKind::Tests(runner) => {
                    if let Some((binary, path, outcome)) = job.parser.line(*runner, &line) {
                        match outcome {
                            Outcome::Passed => job.counts.0 += 1,
                            Outcome::Failed => job.counts.1 += 1,
                            _ => {}
                        }
                        self.cargo.results.insert((binary, path), outcome);
                    }
                    new_lines.push(line);
                }
                JobKind::Check { workspace, .. } => {
                    // JSON on stdout is parsed, not shown; progress is on stderr.
                    if stdout {
                        job.found.extend(cargo::parse_message(&line, workspace));
                    } else {
                        new_lines.push(line);
                    }
                }
            }
        }
        let finished = job.child.try_wait().ok().flatten();
        for line in new_lines {
            self.push_output(line);
        }
        let Some(exit) = finished else {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
            return;
        };
        let job = self.cargo.job.take().expect("checked above");
        self.cargo.lock_wait = false;
        // Let the reader threads hand over the last lines.
        for (_, line) in job.lines.try_iter().collect::<Vec<_>>() {
            self.push_output(line);
        }
        match job.kind {
            JobKind::Tests(_) => {
                self.cargo.results.retain(|_, o| *o != Outcome::Running);
                let (passed, failed) = job.counts;
                let summary = format!("Tests finished: {passed} passed, {failed} failed");
                self.push_output(summary.clone());
                self.status = Some(summary);
            }
            JobKind::Check { tool, .. } => {
                self.set_cargo_diagnostics(tool, job.found);
                let all = self.cargo.diagnostics.values().flat_map(|(_, d)| d);
                let count = |s| all.clone().filter(|d| d.severity == Some(s)).count();
                let (errors, warnings) = (count(lt::DiagnosticSeverity::ERROR), count(lt::DiagnosticSeverity::WARNING));
                let summary = format!("cargo {tool}: {errors} error(s), {warnings} warning(s)");
                self.push_output(summary.clone());
                self.status = Some(summary);
                if errors + warnings > 0 {
                    self.panel_tab = PanelTab::Problems;
                    self.panel_visible = true;
                }
            }
        }
        if !exit.success() && self.cargo.output.iter().all(|l| !l.contains("error")) {
            self.push_output(format!("[cargo exited with {exit}]"));
        }
    }

    // ----- check / clippy ----------------------------------------------------------------

    pub(super) fn run_cargo_check(&mut self, ctx: &egui::Context, tool: &'static str) {
        let Some(root) = self.cargo_root() else {
            self.status = Some("Open a Cargo project first".to_owned());
            return;
        };
        let args = [tool, "--workspace", "--all-targets", "--message-format=json"].map(str::to_owned).to_vec();
        self.start_job(ctx, JobKind::Check { tool, workspace: root.clone() }, args, &root);
    }

    fn set_cargo_diagnostics(&mut self, tool: &str, found: Vec<CargoDiagnostic>) {
        self.cargo.diagnostics.clear();
        for d in found {
            let severity = match d.severity {
                1 => lt::DiagnosticSeverity::ERROR,
                2 => lt::DiagnosticSeverity::WARNING,
                _ => lt::DiagnosticSeverity::INFORMATION,
            };
            let diag = lt::Diagnostic {
                range: lt::Range::new(lt::Position::new(d.start.0, d.start.1), lt::Position::new(d.end.0, d.end.1)),
                severity: Some(severity),
                code: d.code.map(lt::NumberOrString::String),
                source: Some(format!("cargo {tool}")),
                message: d.message,
                ..Default::default()
            };
            let entry = self.cargo.diagnostics.entry(key(&d.path)).or_insert_with(|| (d.path.clone(), Vec::new()));
            if !entry.1.contains(&diag) {
                entry.1.push(diag);
            }
        }
    }

    // ----- tests -------------------------------------------------------------------------

    fn outcome_of(&self, target: &FileTarget, item: &TestItem) -> Option<Outcome> {
        let binary = target.binary();
        if !item.is_module {
            return self.cargo.results.get(&(binary, item.path.clone())).copied();
        }
        // A module sums up its tests: running, else failed, else passed.
        let prefix = format!("{}::", item.path);
        let inner: Vec<Outcome> = self
            .cargo
            .results
            .iter()
            .filter(|((b, p), _)| *b == binary && p.starts_with(&prefix))
            .map(|(_, o)| *o)
            .collect();
        [Outcome::Running, Outcome::Failed, Outcome::Passed].into_iter().find(|o| inner.contains(o))
    }

    /// Runs the tests in `scope`, marking them as running.
    fn run_tests(&mut self, ctx: &egui::Context, scope: TestScope, cwd: PathBuf) {
        let runner = self.runner(ctx);
        let marks: Vec<(String, String)> = self
            .cargo
            .tests
            .values()
            .flat_map(|f| f.items.iter().filter(|i| !i.is_module).map(move |i| (f, i)))
            .filter(|(f, i)| match &scope {
                TestScope::Workspace => true,
                TestScope::Package(p) => &f.target.package == p,
                TestScope::Filter { package, target, path, exact } => {
                    &f.target.package == package
                        && &f.target.target == target
                        && if *exact {
                            &i.path == path
                        } else {
                            path.is_empty() || i.path.starts_with(&format!("{path}::"))
                        }
                }
            })
            .map(|(f, i)| (f.target.binary(), i.path.clone()))
            .collect();
        for m in marks {
            self.cargo.results.insert(m, Outcome::Running);
        }
        let args = cargo::test_args(runner, &scope);
        self.start_job(ctx, JobKind::Tests(runner), args, &cwd);
    }

    fn run_item(&mut self, ctx: &egui::Context, target: FileTarget, item: TestItem) {
        let scope = TestScope::Filter {
            package: target.package.clone(),
            target: target.target.clone(),
            path: item.path.clone(),
            exact: !item.is_module,
        };
        self.run_tests(ctx, scope, target.dir.clone());
    }

    fn active_tests(&self) -> Option<&FileTests> {
        let path = self.docs.get(self.active)?.path.as_ref()?;
        self.cargo.tests.get(&key(path))
    }

    /// Test: Run Test at Cursor — the innermost test or test module around it.
    /// The innermost test or test module around the cursor.
    pub(super) fn test_item_at_cursor(&self) -> Option<(FileTarget, TestItem)> {
        let doc = self.docs.get(self.active)?;
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let f = self.active_tests()?;
        let item = f.items.iter().filter(|i| i.line <= line && line <= i.end_line).max_by_key(|i| i.line)?;
        Some((f.target.clone(), item.clone()))
    }

    pub(super) fn test_at_cursor(&mut self, ctx: &egui::Context) {
        match self.test_item_at_cursor() {
            Some((target, item)) => self.run_item(ctx, target, item),
            None => self.status = Some("No test at the cursor".to_owned()),
        }
    }

    pub(super) fn test_current_file(&mut self, ctx: &egui::Context) {
        let Some(f) = self.active_tests() else {
            self.status = Some("No tests in this file".to_owned());
            return;
        };
        let (target, path) = (f.target.clone(), f.target.modules.join("::"));
        let scope =
            TestScope::Filter { package: target.package.clone(), target: target.target.clone(), path, exact: false };
        self.run_tests(ctx, scope, target.dir);
    }

    pub(super) fn test_all(&mut self, ctx: &egui::Context) {
        let Some(root) = self.cargo_root() else {
            self.status = Some("Open a Cargo project first".to_owned());
            return;
        };
        self.run_tests(ctx, TestScope::Workspace, root);
    }

    /// A gutter run button was clicked.
    pub(super) fn run_test_at_line(&mut self, ctx: &egui::Context, line: usize) {
        let found = self
            .active_tests()
            .and_then(|f| Some((f.target.clone(), f.items.iter().find(|i| i.line == line)?.clone())));
        if let Some((target, item)) = found {
            self.run_item(ctx, target, item);
        }
    }

    /// Gutter run buttons for document `i`.
    pub(super) fn cargo_decorate(&self, i: usize, deco: &mut crate::editor_view::Decorations) {
        if i != self.active {
            return;
        }
        let Some(f) = self.active_tests() else { return };
        deco.runnables = f
            .items
            .iter()
            .map(|item| {
                let (icon, color) = outcome_icon(self.outcome_of(&f.target, item));
                (item.line, Some(icon), color)
            })
            .collect();
    }

    // ----- views -------------------------------------------------------------------------

    pub(super) fn show_testing(&mut self, ctx: &egui::Context) {
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Testing;
        if self.cargo.scanned_for != self.cargo_root() {
            self.scan_tests(ctx);
        }
    }

    /// The Testing side bar view: every test, its last result, run buttons.
    pub(super) fn testing_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        let running = matches!(self.cargo.job, Some(Job { kind: JobKind::Tests(_), .. }));
        let mut run: Option<(FileTarget, TestItem)> = None;
        let mut debug: Option<(FileTarget, TestItem)> = None;
        let mut open: Option<(PathBuf, usize)> = None;
        let mut action = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("TESTING").small().strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, Icon::Refresh, "Refresh Tests").clicked() {
                    action = Some(0);
                }
                if running && icons::button(ui, Icon::Stop, "Cancel Test Run").clicked() {
                    action = Some(1);
                }
                if icons::button(ui, Icon::Play, "Run All Tests (Ctrl+; A)").clicked() {
                    action = Some(2);
                }
            });
        });
        let runner = match self.settings.cargo.test_runner.as_str() {
            "nextest" => "cargo nextest",
            "auto" if self.cargo.nextest == Some(true) => "cargo nextest",
            _ => "cargo test",
        };
        ui.label(RichText::new(format!("Runner: {runner}")).small().color(pal.text_dim));
        let root = self.cargo_root();
        let mut files: Vec<&FileTests> = self.cargo.tests.values().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut toggle = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            if files.is_empty() {
                let text = if root.is_none() { "Open a Cargo project to see its tests." } else { "No tests found." };
                ui.label(RichText::new(text).color(pal.text_dim));
            }
            let size = ui.text_style_height(&egui::TextStyle::Body);
            for f in files {
                let rel = root
                    .as_ref()
                    .and_then(|r| f.path.strip_prefix(r).ok())
                    .map_or_else(|| f.path.display().to_string(), |p| p.display().to_string());
                let collapsed = self.cargo.collapsed.contains(&f.path);
                ui.horizontal(|ui| {
                    let chevron = if collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
                    if icons::show(ui, chevron, size, pal.text_dim).interact(Sense::click()).clicked()
                        || ui.add(egui::Label::new(RichText::new(rel).strong()).sense(Sense::click())).clicked()
                    {
                        toggle = Some(f.path.clone());
                    }
                });
                if collapsed {
                    continue;
                }
                for item in &f.items {
                    let depth = item.path.matches("::").count().saturating_sub(f.target.modules.len());
                    ui.horizontal(|ui| {
                        ui.add_space(12.0 + depth as f32 * 12.0);
                        let (icon, color) = outcome_icon(self.outcome_of(&f.target, item));
                        let icon = if icon == Icon::Play { Icon::Beaker } else { icon };
                        let color = if icon == Icon::Beaker { pal.text_dim } else { color };
                        icons::show(ui, icon, size, color);
                        let name = if item.is_module {
                            RichText::new(&item.name).italics()
                        } else {
                            RichText::new(&item.name)
                        };
                        if ui.add(egui::Label::new(name).sense(Sense::click())).on_hover_text(&item.path).clicked() {
                            open = Some((f.path.clone(), item.line));
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if icons::button(ui, Icon::Bug, "Debug").clicked() {
                                debug = Some((f.target.clone(), item.clone()));
                            }
                            if icons::button(ui, Icon::Play, "Run").clicked() {
                                run = Some((f.target.clone(), item.clone()));
                            }
                        });
                    });
                }
            }
        });
        if let Some(path) = toggle
            && !self.cargo.collapsed.remove(&path)
        {
            self.cargo.collapsed.insert(path);
        }
        match action {
            Some(0) => self.scan_tests(&ctx),
            Some(1) => self.cancel_job(),
            Some(2) => self.test_all(&ctx),
            _ => {}
        }
        if let Some((target, item)) = run {
            self.run_item(&ctx, target, item);
        }
        if let Some((target, item)) = debug {
            self.debug_test(&ctx, target, item);
        }
        if let Some((path, line)) = open {
            self.open_file(&path);
            if let Some(doc) = self.active_doc() {
                doc.goto_line(line + 1);
            }
        }
    }

    /// What the last cargo run printed.
    pub(super) fn job_output(&self) -> &[String] {
        &self.cargo.output
    }

    /// The Output panel: what the last cargo run printed.
    pub(super) fn output_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let mut cancel = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&self.cargo.output_title).small().color(pal.text_dim));
            if self.cargo.job.is_some() {
                ui.label(RichText::new("running…").small().color(pal.accent));
                if icons::button(ui, Icon::Stop, "Stop").clicked() {
                    cancel = true;
                }
            }
        });
        let font = egui::FontId::monospace(self.font_size * 0.9);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let (red, green, yellow) = (
            Color32::from_rgb(0xF1, 0x4C, 0x4C),
            Color32::from_rgb(0x73, 0xC9, 0x91),
            Color32::from_rgb(0xCC, 0xA7, 0x00),
        );
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show_rows(
            ui,
            row_h,
            self.cargo.output.len(),
            |ui, range| {
                for line in &self.cargo.output[range] {
                    let color = if line.contains("FAILED") || line.contains("FAIL [") || line.starts_with("error") {
                        red
                    } else if line.ends_with(" ok") || line.contains("PASS [") {
                        green
                    } else if line.starts_with("warning") {
                        yellow
                    } else {
                        pal.editor_fg
                    };
                    ui.label(RichText::new(line).font(font.clone()).color(color));
                }
            },
        );
        if cancel {
            self.cancel_job();
        }
    }

    // ----- run configurations ------------------------------------------------------------

    /// Loads run configurations in the background (the launch file, else
    /// what Cargo implies), then opens the picker if `pick`.
    fn load_run_configs(&mut self, ctx: &egui::Context, pick: bool) {
        let Some(root) = self.cargo_root().or_else(|| self.explorer.root.clone()) else {
            self.status = Some("Open a folder first".to_owned());
            return;
        };
        let (tx, wake) = (self.cargo.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let (configs, error) = match cargo::load_configs(&root) {
                Ok(Some(configs)) => (configs, None),
                Ok(None) => (cargo::default_configs(&root), None),
                Err(e) => (cargo::default_configs(&root), Some(e)),
            };
            let _ = tx.send(Msg::Configs(configs, error, pick));
            wake();
        });
    }

    fn pick_run_config(&mut self) {
        if self.cargo.configs.is_empty() {
            self.status = Some("No run configurations (Run: Open Configurations to add some)".to_owned());
            return;
        }
        let names = self.cargo.configs.iter().map(|c| c.name.clone()).collect();
        let current = self.cargo.selected.clone().unwrap_or_default();
        self.palette.show_picker(PickerKind::RunConfig, names, &current, "Select a configuration to run");
    }

    /// Run Program: runs the active file the way its language runs. With no
    /// file open, the folder's Cargo project.
    pub(super) fn run_program(&mut self, ctx: &egui::Context) {
        // What runs is what's on screen.
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() && self.docs[i].path.is_some() {
                self.save_doc(i);
            }
        }
        let root = self.explorer.root.clone();
        let active = self.docs.get(self.active);
        let outcome = match active.and_then(|d| d.path.clone().map(|p| (p, d.text()))) {
            Some((file, source)) => run::plan(&file, &source, root.as_deref()),
            None if active.is_some() => Err("Save this file first".to_owned()),
            None => match self.cargo_root() {
                Some(dir) => Ok(run::Outcome::Run(run::RunPlan {
                    title: "cargo run".to_owned(),
                    command: "cargo run".to_owned(),
                    cwd: dir,
                })),
                None => Err("Open a file to run".to_owned()),
            },
        };
        match outcome {
            Ok(run::Outcome::Run(plan)) => self.start_run(ctx, &plan),
            Ok(run::Outcome::Choose(plans)) => {
                let names = plans.iter().map(|p| p.title.clone()).collect();
                self.cargo.run_targets = plans;
                self.palette.show_picker(PickerKind::RunTarget, names, "", "Select the program to run");
            }
            Err(e) => self.status = Some(e),
        }
    }

    pub(super) fn picked_run_target(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(plan) = self.cargo.run_targets.iter().find(|p| p.title == name).cloned() {
            self.start_run(ctx, &plan);
        }
    }

    /// Runs a plan in its own terminal tab.
    fn start_run(&mut self, ctx: &egui::Context, plan: &run::RunPlan) {
        if !self.allowed_to_run(&plan.cwd) {
            return;
        }
        let (program, args) = shell(&plan.command);
        let scrollback = self.settings.terminal.scrollback;
        // Each run replaces the last one's finished tab.
        self.terminal.close_finished("Run: ");
        let title = format!("Run: {}", plan.title);
        let task = Task { title: &title, program: &program, args: &args, env: &[], cwd: &plan.cwd };
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = PanelTab::Terminal;
                self.terminal.detached = false;
            }
            Err(e) => self.status = Some(format!("Could not run {}: {e}", plan.title)),
        }
    }

    /// Ctrl+F5: run the selected configuration (asks which the first time).
    pub(super) fn run_selected_config(&mut self, ctx: &egui::Context) {
        let selected = self.cargo.selected.clone();
        match selected.and_then(|s| self.cargo.configs.iter().find(|c| c.name == s).cloned()) {
            Some(config) => self.run_config(ctx, &config),
            None => self.load_run_configs(ctx, true),
        }
    }

    pub(super) fn select_run_config(&mut self, ctx: &egui::Context) {
        self.load_run_configs(ctx, true);
    }

    pub(super) fn picked_run_config(&mut self, ctx: &egui::Context, name: &str) {
        let Some(config) = self.cargo.configs.iter().find(|c| c.name == name).cloned() else { return };
        self.cargo.selected = Some(config.name.clone());
        self.run_config(ctx, &config);
    }

    fn run_config(&mut self, ctx: &egui::Context, config: &RunConfig) {
        let Some(root) = self.cargo_root().or_else(|| self.explorer.root.clone()) else { return };
        if !self.allowed_to_run(&root) {
            return;
        }
        let cwd = config.cwd.as_ref().map_or(root.clone(), |c| root.join(c));
        let (program, args) = shell(&config.command);
        let env: Vec<(String, String)> = config.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let scrollback = self.settings.terminal.scrollback;
        let task = Task { title: &config.name, program: &program, args: &args, env: &env, cwd: &cwd };
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = PanelTab::Terminal;
                self.terminal.detached = false;
            }
            Err(e) => self.status = Some(format!("Could not run {}: {e}", config.name)),
        }
    }

    /// Run: Open Configurations — creates the launch file from the defaults if needed.
    pub(super) fn open_run_configs(&mut self) {
        let Some(root) = self.cargo_root().or_else(|| self.explorer.root.clone()) else {
            self.status = Some("Open a folder first".to_owned());
            return;
        };
        let path = cargo::launch_file(&root);
        if !path.exists() {
            let contents = cargo::launch_file_contents(&cargo::default_configs(&root));
            let written =
                path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, contents));
            if let Err(e) = written {
                self.status = Some(format!("Could not create {}: {e}", path.display()));
                return;
            }
        }
        self.open_file(&path);
        // Saved edits take effect on the next run.
        self.cargo.configs.clear();
        self.cargo.selected = None;
    }
}
