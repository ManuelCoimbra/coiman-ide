//! Warns about endless loops and leaks while typing. C and C++ also get the Clang
//! analyzer when it's installed, on saved files in trusted folders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use coiman_core::language::likely::{self, Certainty, Finding};
use coiman_core::language::lsp::servers::which;
use coiman_core::language::lsp::types as lt;
use coiman_core::launch::constant_run::{self, Ending, Job, RunEvent};
use coiman_core::{launch::run, project::trust};
use eframe::egui;

use super::lsp::path_key;
use super::{CodeApp, repaint};

/// Seconds typing must pause before reading the file again.
const REST: f64 = 0.5;
/// The analyzer can take a while on a big file, but not forever.
const ANALYZER_LIMIT: Duration = Duration::from_secs(60);

type Scanned = (u64, u64, PathBuf, Vec<Finding>);

struct Analyzing {
    uid: u64,
    version: u64,
    path: PathBuf,
    job: Job,
    lines: Vec<String>,
}

#[derive(Default)]
pub(super) struct Likely {
    /// By file: what the reading found, for this version.
    found: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    /// By file: what the analyzer found, for this version of the document.
    analyzed: HashMap<PathBuf, (PathBuf, u64, Vec<lt::Diagnostic>)>,
    scanned: HashMap<u64, u64>,
    analyzer_ran: HashMap<u64, u64>,
    waiting: Option<Receiver<Scanned>>,
    analyzing: Option<Analyzing>,
    changed_since: Option<f64>,
}

fn diagnostic(finding: &Finding, source: &str) -> lt::Diagnostic {
    lt::Diagnostic {
        range: lt::Range::new(
            lt::Position::new(finding.line, finding.col),
            lt::Position::new(finding.line, finding.end_col),
        ),
        severity: Some(match finding.certainty {
            Certainty::Likely => lt::DiagnosticSeverity::WARNING,
            Certainty::Possible => lt::DiagnosticSeverity::INFORMATION,
        }),
        source: Some(source.to_owned()),
        message: finding.message.clone(),
        ..Default::default()
    }
}

impl CodeApp {
    pub(super) fn pump_likely(&mut self, ctx: &egui::Context) {
        if !self.settings.editor.likely_problems {
            self.likely.found.clear();
            self.likely.analyzed.clear();
            self.likely.analyzing = None;
            return;
        }
        if let Some(waiting) = &self.likely.waiting {
            match waiting.try_recv() {
                Ok((uid, version, path, findings)) => {
                    self.likely.waiting = None;
                    self.likely.scanned.insert(uid, version);
                    let found = findings.iter().map(|f| diagnostic(f, "likely")).collect();
                    self.likely.found.insert(path_key(&path), (path, found));
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.likely.waiting = None,
            }
        }
        self.pump_analyzer();
        self.likely_scan_next(ctx);
        self.likely_analyze_next(ctx);
    }

    /// Reads one file whose text changed, once typing has paused.
    fn likely_scan_next(&mut self, ctx: &egui::Context) {
        if self.likely.waiting.is_some() {
            return;
        }
        let changed = self.docs.iter().find(|d| {
            d.path.is_some()
                && likely::checks(&d.extension())
                && self.likely.scanned.get(&d.uid) != Some(&d.buffer.version())
        });
        let Some(doc) = changed else {
            self.likely.changed_since = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let since = *self.likely.changed_since.get_or_insert(now);
        if now - since < REST {
            ctx.request_repaint_after(Duration::from_secs_f64(REST));
            return;
        }
        self.likely.changed_since = None;
        let (uid, version, extension, source) = (doc.uid, doc.buffer.version(), doc.extension(), doc.text());
        let Some(path) = doc.path.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send((uid, version, path, likely::check(&extension, &source)));
            wake();
        });
        self.likely.waiting = Some(rx);
    }

    fn pump_analyzer(&mut self) {
        let mut ending = None;
        if let Some(running) = &mut self.likely.analyzing {
            while let Ok(event) = running.job.events.try_recv() {
                match event {
                    RunEvent::Line { text, .. } => running.lines.push(text),
                    RunEvent::Finished(end) => ending = Some(end),
                }
            }
        }
        let Some(end) = ending else { return };
        let Some(done) = self.likely.analyzing.take() else { return };
        // An analyzer that failed knows nothing, so it says nothing.
        if matches!(end, Ending::Exit(_)) {
            let findings = likely::parse_analyzer(&done.lines, &done.path);
            let found = findings.iter().map(|f| diagnostic(f, "clang analyzer")).collect();
            self.likely.analyzed.insert(path_key(&done.path), (done.path, done.version, found));
        }
        self.likely.analyzer_ran.insert(done.uid, done.version);
    }

    /// Starts the Clang analyzer on one saved C or C++ file it hasn't seen.
    fn likely_analyze_next(&mut self, ctx: &egui::Context) {
        if self.likely.analyzing.is_some() {
            return;
        }
        // Lines move as soon as the text does.
        let versions: HashMap<PathBuf, u64> =
            self.docs.iter().filter_map(|d| Some((path_key(d.path.as_deref()?), d.buffer.version()))).collect();
        self.likely.analyzed.retain(|k, (_, version, _)| versions.get(k) == Some(version));
        let next = self.docs.iter().find(|d| {
            likely::analyzer_reads(&d.extension())
                && !d.is_dirty()
                && self.likely.analyzer_ran.get(&d.uid) != Some(&d.buffer.version())
        });
        let Some(doc) = next else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let Some(path) = doc.path.clone() else { return };
        let Some(program) = which("clang-tidy") else {
            self.likely.analyzer_ran.insert(uid, version);
            return;
        };
        let folder = self.explorer.root.clone().or_else(|| path.parent().map(Path::to_path_buf));
        // It reads the project's compile flags, which can name programs to run.
        if !folder.as_deref().is_some_and(trust::is_trusted) {
            self.likely.analyzer_ran.insert(uid, version);
            return;
        }
        let root = self.explorer.root.as_deref();
        let has_database = run::has_compile_database(&path, root);
        let plan = run::RunPlan {
            title: "clang-tidy".to_owned(),
            command: String::new(),
            cwd: path.parent().map_or_else(|| path.clone(), Path::to_path_buf),
        };
        let args = likely::analyzer_args(&path, has_database);
        let job =
            constant_run::start(&program.to_string_lossy(), &args, &plan, &[], Some(ANALYZER_LIMIT), repaint(ctx));
        self.likely.analyzing = Some(Analyzing { uid, version, path, job, lines: Vec::new() });
    }

    /// What's likely wrong in `key` (a [`path_key`]). The analyzer follows every path
    /// through the code, so on a line it reports, its word is the one kept.
    pub(super) fn likely_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        let analyzed = self.likely.analyzed.get(key).map_or(&[][..], |(_, _, d)| d.as_slice());
        let read = self.likely.found.get(key).map_or(&[][..], |(_, d)| d.as_slice());
        let mut all = analyzed.to_vec();
        // It reports the same leak further on, where the memory is finally lost: by name.
        let same_leak = |d: &lt::Diagnostic| {
            d.message.contains("leak")
                && analyzed.iter().any(|a| {
                    a.range.start.line == d.range.start.line
                        || d.message
                            .split_whitespace()
                            .any(|word| a.message.contains(&format!("'{}'", word.trim_end_matches(','))))
                })
        };
        all.extend(read.iter().filter(|d| !same_leak(d)).cloned());
        all
    }

    /// Files with something likely wrong: (key, path).
    pub(super) fn likely_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        let read = self.likely.found.iter().map(|(k, (p, _))| (k.clone(), p.clone()));
        read.chain(self.likely.analyzed.iter().map(|(k, (p, _, _))| (k.clone(), p.clone()))).collect()
    }
}
