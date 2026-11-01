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
use rust_code_core::build_tools::{self, BuildTool};
use rust_code_core::cargo::{
    self, CargoDiagnostic, FileTarget, Outcome, OutputParser, RunConfig, Runner, TestItem, TestScope,
};
use rust_code_core::lsp::discovery::quiet_command;
use rust_code_core::lsp::status::waits_for_build_lock;
use rust_code_core::lsp::types as lt;
use rust_code_core::matchers;
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
    Configs(PathBuf, Vec<RunConfig>, Option<String>, bool),
    Packages(PathBuf, Vec<cargo::CargoPackage>),
    Scanned(PathBuf, Vec<(PathBuf, FileTarget, Vec<TestItem>)>),
    Nextest(bool),
}

/// Commands that only work in a Cargo project: hidden everywhere else.
pub(super) const TESTING_ONLY: &[Command] = &[
    Command::ShowDbgOutput,
    Command::RemoveDbgFromFile,
    Command::RemoveDbgFromWorkspace,
    Command::DebugTestAtCursor,
    Command::TestAtCursor,
    Command::TestCurrentFile,
    Command::CargoCheck,
    Command::CargoClippy,
];

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
    Command::TestAtCursor,
    Command::TestCurrentFile,
    Command::CargoCheck,
    Command::CargoClippy,
];

pub(super) struct CargoState {
    /// Programs offered by Run Program when several fit.
    run_targets: Vec<run::RunPlan>,
    /// Cargo's account of the project, for telling which target a file starts.
    packages: Vec<cargo::CargoPackage>,
    /// The tests of the active file, when not Rust: (doc uid, version, line and id).
    other_tests: Option<OtherTests>,
    /// The colors written in the active stylesheet: (doc uid, version, line and color).
    gutter_colors: Option<GutterColors>,
    /// What dev servers in Run's tabs said their address was, by tab.
    dev_urls: std::collections::BTreeMap<u64, String>,
    /// An address to open once a dev server just started says where it is.
    pending_route: Option<String>,
    /// What Run would do for a tab: (doc uid, buffer version, answer), worked
    /// out away from the UI because it can ask Cargo or walk a project.
    run_preview: Option<RunPreview>,
    /// When `run_preview` was last worked out.
    run_preview_at: f64,
    run_preview_job: Option<std::sync::mpsc::Receiver<RunPreview>>,
    /// The (doc uid, buffer version) on screen, and since when.
    run_preview_seen: Option<(u64, u64, f64)>,
    /// What the programs run from here reported, by terminal tab.
    tab_problems: HashMap<u64, Vec<(PathBuf, lt::Diagnostic)>>,
    next_problem_scan: f64,
    /// Why Run did nothing, when it was refused, and for which tab.
    run_refusal: Option<(String, f64, Option<u64>)>,
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
    /// The configuration each folder's Run button is set to; none means the
    /// current file. Kept in the session.
    pub(super) run_pins: std::collections::BTreeMap<PathBuf, String>,
    /// A tool the build needs, on offer to download.
    tool_offer: Option<BuildTool>,
    tool_declined: bool,
    tool_install: Option<(BuildTool, std::sync::mpsc::Receiver<Result<(), String>>)>,
    /// What `configs` were read for: the folder and its files' change times.
    configs_for: Option<ConfigsKey>,
    configs_checked: f64,
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
            packages: Vec::new(),
            other_tests: None,
            gutter_colors: None,
            dev_urls: Default::default(),
            pending_route: None,
            run_preview: None,
            run_preview_at: 0.0,
            run_preview_job: None,
            run_preview_seen: None,
            tab_problems: HashMap::new(),
            next_problem_scan: 0.0,
            run_refusal: None,
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
            run_pins: std::collections::BTreeMap::new(),
            tool_offer: None,
            tool_declined: false,
            tool_install: None,
            configs_for: None,
            configs_checked: f64::NEG_INFINITY,
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

pub(super) fn outcome_icon(outcome: Option<Outcome>) -> (Icon, Color32) {
    match outcome {
        None => (Icon::Play, Color32::from_rgb(0x89, 0xD1, 0x85)),
        Some(Outcome::Running) => (Icon::Dot, Color32::from_rgb(0xE2, 0xC0, 0x8D)),
        Some(Outcome::Passed) => (Icon::Check, Color32::from_rgb(0x73, 0xC9, 0x91)),
        Some(Outcome::Failed) => (Icon::Close, Color32::from_rgb(0xF1, 0x4C, 0x4C)),
        Some(Outcome::Ignored) => (Icon::Skip, Color32::from_gray(0x99)),
    }
}

/// A shell running `command`, so run configurations can use pipes and `&&`.
pub(super) fn shell(command: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("powershell.exe".into(), vec!["-NoLogo".into(), "-NoProfile".into(), "-Command".into(), command.into()])
    } else {
        ("sh".into(), vec!["-c".into(), command.into()])
    }
}

/// Colors written in a stylesheet: (doc uid, version, line and color).
type GutterColors = (u64, u64, Vec<(usize, Color32)>);

/// The choice that makes Run follow the active file.
const CURRENT_FILE: &str = "Current File";
/// Seconds between looks at whether the configurations changed on disk.
const CONFIGS_CHECK_SECONDS: f64 = 2.0;

/// A folder, with the change times of the files its configurations come from.
type ConfigsKey = (PathBuf, Option<std::time::SystemTime>, Option<std::time::SystemTime>);

fn configs_key(root: &Path) -> ConfigsKey {
    let changed = |path: PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    (root.to_path_buf(), changed(cargo::launch_file(root)), changed(root.join("Cargo.toml")))
}

/// Tests of a file in another language: (doc uid, buffer version, line and id).
type OtherTests = (u64, u64, Vec<(usize, String)>);

/// Seconds between looks at what running programs printed.
const PROBLEM_SCAN_SECONDS: f64 = 0.5;

pub(super) fn problem_diagnostic(problem: matchers::Problem) -> lt::Diagnostic {
    let line = problem.line.saturating_sub(1);
    let start = problem.col.saturating_sub(1);
    lt::Diagnostic {
        range: lt::Range::new(lt::Position::new(line, start), lt::Position::new(line, start + 1)),
        severity: Some(match problem.severity {
            matchers::Severity::Error => lt::DiagnosticSeverity::ERROR,
            matchers::Severity::Warning => lt::DiagnosticSeverity::WARNING,
            matchers::Severity::Info => lt::DiagnosticSeverity::INFORMATION,
        }),
        source: Some("run".to_owned()),
        message: problem.message,
        ..Default::default()
    }
}

/// What Run would do for a tab: (doc uid, buffer version, answer).
type RunPreview = (u64, u64, Result<run::Outcome, String>);

/// Seconds before asking Cargo again about a file that hasn't changed.
const RUN_PREVIEW_CARGO_AGE: f64 = 5.0;
/// Seconds typing must pause before asking again what Run would do.
const RUN_PREVIEW_SETTLE: f64 = 1.0;
/// Seconds a refusal stays beside the Run button.
const RUN_REFUSAL_SECONDS: f64 = 10.0;

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
                Msg::Configs(root, configs, error, pick) => {
                    // The folder changed while these were read.
                    if self.run_scope().as_ref() != Some(&root) {
                        continue;
                    }
                    if let Some(e) = error {
                        self.status = Some(e);
                    }
                    self.cargo.configs = configs;
                    // Unreadable is not the same as removed: keep the choice for the fix.
                    let removed = self.cargo.run_pins.get(&root).filter(|name| {
                        self.status.as_deref().is_none_or(|s| !s.contains("launch.toml"))
                            && !self.cargo.configs.iter().any(|c| &c.name == *name)
                    });
                    if let Some(name) = removed.cloned() {
                        self.cargo.run_pins.remove(&root);
                        self.status = Some(format!("{name} no longer exists: Run follows the current file again"));
                    }
                    if pick {
                        self.pick_run_config();
                    }
                }
                Msg::Packages(root, packages) => {
                    if self.run_scope().as_ref() == Some(&root) {
                        self.cargo.packages = packages;
                        // Files may start different targets now.
                        self.cargo.parsed = None;
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
                let target = cargo::file_target_in(&path, &self.cargo.packages);
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
        self.refresh_other_tests();
    }

    /// Keeps the gutter's tests current for a file in another language.
    fn refresh_other_tests(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.cargo.other_tests = None;
            return;
        };
        let (uid, version) = (doc.uid, doc.buffer.version());
        if self.cargo.other_tests.as_ref().is_some_and(|(u, v, _)| (*u, *v) == (uid, version)) {
            return;
        }
        let text = doc.text();
        let root = self.explorer.root.as_deref();
        let found = doc.path.as_deref().map_or_else(Vec::new, |p| {
            let relative = root.and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p).to_string_lossy().into_owned();
            let mut found = run::test_lines(p, &text);
            found.extend(run::program_lines(p, &text).into_iter().map(|line| (line, run::RUN_PROGRAM.to_owned())));
            found.extend(run::web_lines(p, &relative, &text));
            found
        });
        let stylesheet =
            doc.path.as_deref().and_then(|p| p.extension()).is_some_and(|e| {
                matches!(e.to_string_lossy().to_lowercase().as_str(), "css" | "scss" | "sass" | "less")
            });
        let colors = if stylesheet { rust_code_core::css::color_lines(&text) } else { Vec::new() };
        let colors = colors.into_iter().map(|(line, [r, g, b, a])| (line, Color32::from_rgba_unmultiplied(r, g, b, a)));
        self.cargo.gutter_colors = Some((uid, version, colors.collect()));
        self.cargo.other_tests = Some((uid, version, found));
    }

    /// The test on `line` of the active file, when it isn't a Rust one.
    fn other_test_at(&self, line: usize) -> Option<String> {
        let (uid, _, found) = self.cargo.other_tests.as_ref()?;
        let doc = self.docs.get(self.active).filter(|d| d.uid == *uid)?;
        doc.path.as_ref()?;
        found.iter().find(|(l, _)| *l == line).map(|(_, id)| id.clone())
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

    /// Starts the Output panel afresh on a new run.
    pub(super) fn begin_output(&mut self, title: String) {
        self.cargo.output.clear();
        self.cargo.output.push(format!("> {title}"));
        self.cargo.output_title = title;
        self.panel_visible = true;
        self.panel_tab = PanelTab::Output;
    }

    pub(super) fn push_output(&mut self, line: String) {
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
        let testing_gone = !self.testing_available() && self.sidebar_view == SidebarView::Testing;
        if testing_gone || (self.sidebar_view == SidebarView::Debug && !self.can_debug()) {
            self.sidebar_view = SidebarView::Explorer;
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
        self.cancel_test_run();
        self.cargo.lock_wait = false;
        if let Some(mut job) = self.cargo.job.take() {
            rust_code_core::lsp::discovery::kill_tree(&mut job.child);
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
        if !self.cargo_project && self.run_all_other_tests(ctx) {
            return;
        }
        let Some(root) = self.cargo_root() else {
            self.status = Some("Open a Cargo project first".to_owned());
            return;
        };
        self.run_tests(ctx, TestScope::Workspace, root);
    }

    /// A gutter run button was clicked.
    pub(super) fn run_test_at_line(&mut self, ctx: &egui::Context, line: usize) {
        if self.run_cell_at_line(ctx, line) {
            return;
        }
        if let Some(id) = self.other_test_at(line) {
            if id == run::RUN_PROGRAM {
                self.run_program(ctx);
                return;
            }
            let file = self.docs[self.active].path.clone();
            let web = id.starts_with(run::RUN_SCRIPT) || id.starts_with(run::RUN_PAGE);
            if !web && file.is_some_and(|f| self.run_one_test(ctx, &f, &id, line)) {
                return;
            }
            for i in 0..self.docs.len() {
                if self.docs[i].is_dirty() && self.docs[i].path.is_some() && !self.save_doc(i) {
                    let why = format!("Nothing was run: {} couldn't be saved", self.docs[i].title());
                    self.refuse_run(ctx, why);
                    return;
                }
            }
            let doc = &self.docs[self.active];
            let Some(file) = doc.path.clone() else { return };
            if let Some(script) = id.strip_prefix(run::RUN_SCRIPT) {
                match run::package_script_plan(&file, script) {
                    Ok(plan) => self.start_run(ctx, &plan),
                    Err(why) => self.refuse_run(ctx, why),
                }
                return;
            }
            if let Some(route) = id.strip_prefix(run::RUN_PAGE) {
                match (self.dev_url(), run::dev_script_plan(&file, self.explorer.root.as_deref())) {
                    (Some(url), _) => ctx.open_url(egui::OpenUrl::new_tab(format!("{url}{route}"))),
                    (None, Ok(plan)) => {
                        self.cargo.pending_route = Some(route.to_owned());
                        self.start_run(ctx, &plan);
                    }
                    (None, Err(why)) => self.refuse_run(ctx, why),
                }
                return;
            }
            match run::test_plan(&file, &doc.text(), &id, self.explorer.root.as_deref()) {
                Ok(plan) => self.start_run(ctx, &plan),
                Err(why) => self.refuse_run(ctx, why),
            }
            return;
        }
        let found = self
            .active_tests()
            .and_then(|f| Some((f.target.clone(), f.items.iter().find(|i| i.line == line)?.clone())));
        if let Some((target, item)) = found {
            self.run_item(ctx, target, item);
        }
    }

    /// Where the latest dev server running in a tab is.
    pub(super) fn dev_url(&self) -> Option<String> {
        self.cargo.dev_urls.values().next_back().cloned()
    }

    /// Gutter run buttons for document `i`.
    pub(super) fn cargo_decorate(&self, i: usize, deco: &mut crate::editor_view::Decorations) {
        if i != self.active {
            return;
        }
        if let Some((uid, _, colors)) = &self.cargo.gutter_colors
            && self.docs.get(i).is_some_and(|d| d.uid == *uid)
        {
            deco.color_marks.clone_from(colors);
        }
        if let Some((uid, _, found)) = &self.cargo.other_tests
            && self.docs.get(i).is_some_and(|d| d.uid == *uid)
            && !found.is_empty()
        {
            let color = Color32::from_rgb(0x89, 0xD1, 0x85);
            let file = self.docs.get(i).and_then(|d| d.path.clone());
            deco.runnables = found
                .iter()
                .map(|(line, _)| match file.as_deref().and_then(|f| self.test_outcome_at(f, *line)) {
                    Some((icon, color)) => (*line, Some(icon), color),
                    None => (*line, None, color),
                })
                .collect();
        }
        let Some(f) = self.active_tests() else { return };
        let tests = f
            .items
            .iter()
            .map(|item| {
                let (icon, color) = outcome_icon(self.outcome_of(&f.target, item));
                (item.line, Some(icon), color)
            })
            .collect::<Vec<_>>();
        deco.runnables.extend(tests);
    }

    // ----- views -------------------------------------------------------------------------

    pub(super) fn show_testing(&mut self, ctx: &egui::Context) {
        if !self.testing_available() {
            self.status = Some("No tests or test runner found in this folder".to_owned());
            return;
        }
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Testing;
        if self.cargo.scanned_for != self.cargo_root() {
            self.scan_tests(ctx);
        }
    }

    /// The Testing side bar view: every test, its last result, run buttons.
    pub(super) fn testing_view(&mut self, ui: &mut egui::Ui) {
        if self.showing_other_tests(ui) {
            self.other_tests_view(ui);
            return;
        }
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
        let Some(root) = self.run_scope() else {
            self.status = Some("Open a folder first".to_owned());
            return;
        };
        self.cargo.configs_for = Some(configs_key(&root));
        let (tx, wake) = (self.cargo.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let (configs, error) = match cargo::load_configs(&root) {
                Ok(Some(configs)) => (configs, None),
                Ok(None) => (cargo::default_configs(&root), None),
                Err(e) => (cargo::default_configs(&root), Some(e)),
            };
            let _ = tx.send(Msg::Packages(root.clone(), cargo::packages(&root).unwrap_or_default()));
            let _ = tx.send(Msg::Configs(root, configs, error, pick));
            wake();
        });
    }

    /// The folder whose configurations apply.
    fn run_scope(&self) -> Option<PathBuf> {
        self.cargo_root().or_else(|| self.explorer.root.clone())
    }

    /// Keeps the configurations current, so the title bar can offer them.
    pub(super) fn pump_run_configs(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.cargo.configs_checked < CONFIGS_CHECK_SECONDS {
            return;
        }
        self.cargo.configs_checked = now;
        let Some(root) = self.run_scope() else {
            self.cargo.configs.clear();
            self.cargo.configs_for = None;
            return;
        };
        if self.cargo.configs_for.as_ref() != Some(&configs_key(&root)) {
            self.load_run_configs(ctx, false);
        }
    }

    /// The configuration Run is set to, if any.
    fn pinned_config(&self) -> Option<&RunConfig> {
        let name = self.cargo.run_pins.get(&self.run_scope()?)?;
        self.cargo.configs.iter().find(|c| &c.name == name)
    }

    /// The title bar's label for what Run starts, when there's a choice to make.
    pub(super) fn run_target_label(&self) -> Option<String> {
        if self.cargo.configs.is_empty() {
            return None;
        }
        Some(self.pinned_config().map_or_else(|| CURRENT_FILE.to_owned(), |c| c.name.clone()))
    }

    /// Lets the user set what Run starts, without starting it.
    pub(super) fn choose_run_target(&mut self) {
        if self.cargo.configs.is_empty() {
            self.status = Some("No run configurations (Run: Open Configurations to add some)".to_owned());
            return;
        }
        let mut names = vec![CURRENT_FILE.to_owned()];
        names.extend(self.cargo.configs.iter().map(|c| c.name.clone()));
        let current = self.run_target_label().unwrap_or_default();
        self.palette.show_picker(PickerKind::RunMode, names, &current, "Choose what Run starts");
    }

    pub(super) fn picked_run_mode(&mut self, name: &str) {
        let Some(root) = self.run_scope() else { return };
        if name == CURRENT_FILE {
            self.cargo.run_pins.remove(&root);
        } else {
            self.cargo.run_pins.insert(root, name.to_owned());
        }
    }

    fn pick_run_config(&mut self) {
        if self.cargo.configs.is_empty() {
            self.status = Some("No run configurations (Run: Open Configurations to add some)".to_owned());
            return;
        }
        let names = self.cargo.configs.iter().map(|c| c.name.clone()).collect();
        let current = self.pinned_config().map(|c| c.name.clone()).unwrap_or_default();
        self.palette.show_picker(PickerKind::RunConfig, names, &current, "Select a configuration to run");
    }

    /// Whether the title bar offers Run: for a file that could run, or with
    /// nothing open, for the folder's Cargo project.
    pub(super) fn run_button_visible(&self) -> bool {
        if self.pinned_config().is_some() {
            return true;
        }
        let Some(doc) = self.docs.get(self.active) else { return self.cargo_root().is_some() };
        if !doc.path.as_deref().is_some_and(run::may_run) {
            return false;
        }
        match &self.cargo.run_preview {
            Some((uid, _, Err(why))) if *uid == doc.uid => !run::refusal_is_final(why),
            _ => true,
        }
    }

    /// What the Run button says it will do.
    pub(super) fn run_button_tip(&self) -> String {
        if let Some(config) = self.pinned_config() {
            return format!("Run {} (Shift+F10)\n{}", config.name, config.command);
        }
        if self.active_is_page() {
            return "Open Live Preview (Shift+F10)".to_owned();
        }
        let known = self.docs.get(self.active).and_then(|doc| {
            self.cargo.run_preview.as_ref().filter(|(uid, _, _)| *uid == doc.uid).map(|(_, _, answer)| answer)
        });
        match known {
            Some(Ok(run::Outcome::Run(plan))) => format!("Run {} (Shift+F10)\n{}", plan.title, plan.command),
            Some(Ok(run::Outcome::Choose(plans))) => format!("Run: choose from {} programs (Shift+F10)", plans.len()),
            Some(Err(why)) => format!("Run Program (Shift+F10)\n{why}"),
            None => "Run Program (Shift+F10)".to_owned(),
        }
    }

    /// Keeps the answer for the active tab current, once its text has settled.
    pub(super) fn pump_run_preview(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.cargo.run_preview_job {
            match job.try_recv() {
                Ok(answer) => {
                    self.cargo.run_preview = Some(answer);
                    self.cargo.run_preview_at = ctx.input(|i| i.time);
                    self.cargo.run_preview_job = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.cargo.run_preview_job = None,
            }
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(file) = doc.path.clone().filter(|p| run::may_run(p)) else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let now = ctx.input(|i| i.time);
        if self.cargo.run_preview.as_ref().is_some_and(|(u, v, _)| (*u, *v) == (uid, version)) {
            // A Cargo project can change on disk with no edit here.
            let cargo = matches!(doc.extension().as_str(), "rs" | "toml");
            let left = RUN_PREVIEW_CARGO_AGE - (now - self.cargo.run_preview_at);
            if !cargo {
                return;
            }
            if left > 0.0 {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
                return;
            }
        }
        let since = match self.cargo.run_preview_seen {
            Some((u, v, since)) if (u, v) == (uid, version) => since,
            _ => {
                self.cargo.run_preview_seen = Some((uid, version, now));
                now
            }
        };
        // A newly shown tab is answered at once; typing waits for a pause.
        let waits = self.cargo.run_preview.as_ref().is_some_and(|(u, v, _)| *u == uid && *v != version);
        if waits && now - since < RUN_PREVIEW_SETTLE {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(RUN_PREVIEW_SETTLE));
            return;
        }
        let source = doc.text();
        let root = self.explorer.root.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = super::repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send((uid, version, run::plan(&file, &source, root.as_deref())));
            wake();
        });
        self.cargo.run_preview_job = Some(rx);
    }

    /// Looks through what running programs printed, for the Problems panel.
    pub(super) fn pump_run_problems(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now < self.cargo.next_problem_scan {
            return;
        }
        self.cargo.next_problem_scan = now + PROBLEM_SCAN_SECONDS;
        let (outputs, live) = self.terminal.scan_outputs();
        for output in outputs {
            match run::server_url(&output.lines).filter(|_| !output.ended) {
                Some(url) => {
                    self.cargo.dev_urls.insert(output.id, url);
                }
                None => {
                    self.cargo.dev_urls.remove(&output.id);
                }
            }
            let found = match output.matcher {
                Some(pattern) => matchers::scan_custom(pattern, &output.lines),
                None => matchers::scan(&output.lines),
            };
            let bases = [output.cwd.to_path_buf()];
            let placed = found
                .into_iter()
                .filter_map(|p| Some((super::prints::resolve(&p.file, &bases)?, problem_diagnostic(p))))
                .collect();
            self.cargo.tab_problems.insert(output.id, placed);
        }
        self.cargo.tab_problems.retain(|id, _| live.contains(id));
        self.cargo.dev_urls.retain(|id, _| live.contains(id));
        if let Some(url) = self.dev_url()
            && let Some(route) = self.cargo.pending_route.take()
        {
            ctx.open_url(egui::OpenUrl::new_tab(format!("{url}{route}")));
        }
    }

    /// What running programs reported about `file` (by [`key`]).
    pub(super) fn run_problems(&self, file: &Path) -> Vec<lt::Diagnostic> {
        let found = self.cargo.tab_problems.values().flatten();
        found.filter(|(path, _)| key(path) == file).map(|(_, d)| d.clone()).collect()
    }

    /// Files running programs reported about: (key, path).
    pub(super) fn run_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        let found = self.cargo.tab_problems.values().flatten();
        found.map(|(path, _)| (key(path), path.clone())).collect()
    }

    /// Says why Run did nothing, beside the button: the status bar may be hidden.
    pub(super) fn refuse_run(&mut self, ctx: &egui::Context, why: String) {
        self.status = Some(why.clone());
        let tab = self.docs.get(self.active).map(|d| d.uid);
        self.cargo.run_refusal = Some((why, ctx.input(|i| i.time), tab));
    }

    /// Asks whether to fetch a tool.
    pub(super) fn offer_build_tool(&mut self, tool: BuildTool) {
        self.cargo.tool_offer = Some(tool);
    }

    /// Offers a tool the build lacks, and fetches it on a yes. Nothing is downloaded
    /// without one. Once it's here, Run goes ahead as it was asked to.
    fn tool_offer_ui(&mut self, ctx: &egui::Context) {
        if let Some((tool, rx)) = &self.cargo.tool_install
            && let Ok(result) = rx.try_recv()
        {
            let name = tool.name();
            let tool = *tool;
            self.cargo.tool_install = None;
            match result {
                Ok(()) => {
                    self.status = Some(format!("{name} is ready"));
                    if tool == BuildTool::JsDebug {
                        self.start_debugging(ctx);
                    } else {
                        self.run_program(ctx);
                    }
                }
                Err(why) => self.status = Some(format!("Could not get {name}: {why}")),
            }
        }
        if let Some((tool, _)) = &self.cargo.tool_install {
            let name = tool.name();
            egui::Window::new("Getting a tool")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Downloading and unpacking {name}…"));
                    });
                });
            return;
        }
        let Some(tool) = self.cargo.tool_offer else { return };
        let (mut download, mut later) = (false, false);
        egui::Window::new("A tool is missing")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 60.0))
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(tool.offer());
                ui.label(RichText::new("It uses the system's curl and tar, and checks the file's hash.").small());
                ui.horizontal(|ui| {
                    download = ui.button("Download").clicked();
                    later = ui.button("Not Now").clicked();
                });
            });
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            later = true;
        }
        if later {
            self.cargo.tool_offer = None;
            if tool == BuildTool::JsDebug {
                self.status = Some("Not fetched: JavaScript can still run, without debugging".to_owned());
            } else {
                self.cargo.tool_declined = true;
                self.status = Some("Run again to try without it: CMake will say what it can't find".to_owned());
            }
        }
        if download {
            self.cargo.tool_offer = None;
            let (tx, rx) = mpsc::channel();
            let wake = repaint(ctx);
            std::thread::spawn(move || {
                let _ = tx.send(build_tools::install(tool));
                wake();
            });
            self.cargo.tool_install = Some((tool, rx));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    pub(super) fn run_refusal_ui(&mut self, ctx: &egui::Context) {
        self.tool_offer_ui(ctx);
        let Some((why, since, tab)) = &self.cargo.run_refusal else { return };
        let left = RUN_REFUSAL_SECONDS - (ctx.input(|i| i.time) - since);
        // It's about that tab: on another it would read as being about this one.
        if left <= 0.0 || *tab != self.docs.get(self.active).map(|d| d.uid) {
            if self.status.as_deref() == Some(why.as_str()) {
                self.status = None;
            }
            self.cargo.run_refusal = None;
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        let shown = egui::Area::new(egui::Id::unique("run_refusal"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 40.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.add(egui::Label::new(why.as_str()).wrap().selectable(false));
                });
            });
        if shown.response.interact(egui::Sense::click()).clicked() {
            self.cargo.run_refusal = None;
        }
    }

    /// Run Program: runs the active file the way its language runs. With no
    /// file open, the folder's Cargo project.
    pub(super) fn run_program(&mut self, ctx: &egui::Context) {
        self.cargo.run_refusal = None;
        // What runs is what's on screen.
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() && self.docs[i].path.is_some() && !self.save_doc(i) {
                let why = format!("Nothing was run: {} couldn't be saved", self.docs[i].title());
                self.refuse_run(ctx, why);
                return;
            }
        }
        if let Some(config) = self.pinned_config().cloned() {
            self.run_config(ctx, &config);
            return;
        }
        if self.active_is_page() {
            self.open_live_preview(ctx);
            return;
        }
        let root = self.explorer.root.clone();
        let active = self.docs.get(self.active);
        let outcome = match active.and_then(|d| d.path.clone().map(|p| (p, d.text()))) {
            Some((file, source)) => run::plan(&file, &source, root.as_deref()),
            None if active.is_some() => Err("Save this file first".to_owned()),
            None => match self.cargo_root() {
                Some(dir) => run::plan(&dir.join("Cargo.toml"), "", root.as_deref()),
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
            Err(e) => self.refuse_run(ctx, e),
        }
    }

    pub(super) fn picked_run_target(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(plan) = self.cargo.run_targets.iter().find(|p| p.title == name).cloned() {
            self.start_run(ctx, &plan);
        }
    }

    /// Runs a plan in its own terminal tab.
    pub(super) fn start_run(&mut self, ctx: &egui::Context, plan: &run::RunPlan) {
        if !self.allowed_to_run(&plan.cwd) {
            return;
        }
        // A build that can't work is better stopped here, with a way forward.
        if plan.command.starts_with("cmake -S")
            && !self.cargo.tool_declined
            && let Some(tool) = build_tools::needed_for_cmake(&plan.cwd)
        {
            self.cargo.tool_offer = Some(tool);
            return;
        }
        let (program, args) = shell(&plan.command);
        let scrollback = self.settings.terminal.scrollback;
        // Each run replaces the last one's finished tab.
        self.terminal.close_finished("Run: ");
        let title = format!("Run: {}", plan.title);
        // A redrawn progress bar would leave its pieces in the output we read.
        let mut env = vec![("CARGO_TERM_PROGRESS_WHEN".to_owned(), "never".to_owned())];
        env.extend(self.environment_variables());
        let task = Task { title: &title, program: &program, args: &args, env: &env, cwd: &plan.cwd, scan: Some(None) };
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
        match self.pinned_config().cloned() {
            Some(config) => self.run_config(ctx, &config),
            None => self.load_run_configs(ctx, true),
        }
    }

    pub(super) fn select_run_config(&mut self, ctx: &egui::Context) {
        self.load_run_configs(ctx, true);
    }

    pub(super) fn picked_run_config(&mut self, ctx: &egui::Context, name: &str) {
        let Some(config) = self.cargo.configs.iter().find(|c| c.name == name).cloned() else { return };
        if let Some(root) = self.run_scope() {
            self.cargo.run_pins.insert(root, config.name.clone());
        }
        self.run_config(ctx, &config);
    }

    fn run_config(&mut self, ctx: &egui::Context, config: &RunConfig) {
        let Some(root) = self.run_scope() else { return };
        if !self.allowed_to_run(&root) {
            return;
        }
        let cwd = config.cwd.as_ref().map_or(root.clone(), |c| root.join(c));
        let (program, args) = shell(&config.command);
        // The configuration's own variables come last, so they win.
        let mut env = self.environment_variables();
        env.extend(config.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        let scrollback = self.settings.terminal.scrollback;
        let matcher = match config.matcher.as_deref().map(matchers::compile_custom).transpose() {
            Ok(matcher) => matcher,
            Err(why) => {
                self.refuse_run(ctx, format!("{}: {why}", config.name));
                return;
            }
        };
        // Each run replaces the last one's finished tab, and its problems.
        self.terminal.close_finished_named(&config.name);
        let task =
            Task { title: &config.name, program: &program, args: &args, env: &env, cwd: &cwd, scan: Some(matcher) };
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
        let Some(root) = self.run_scope() else {
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
    }
}
