//! HTML and XML problems while typing, with no language server needed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use coiman_core::language::lsp::types as lt;
use coiman_core::language::markup::{self, Problem, Severity};
use eframe::egui;

use super::lsp::path_key;
use super::{CodeApp, repaint};

/// Seconds typing must pause before reading the file again.
const REST: f64 = 0.4;

type Checked = (u64, u64, PathBuf, Vec<Problem>);

#[derive(Default)]
pub(super) struct Markup {
    /// By file: what the last reading found.
    found: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    checked: HashMap<u64, u64>,
    waiting: Option<Receiver<Checked>>,
    changed_since: Option<f64>,
}

fn diagnostic(problem: &Problem) -> lt::Diagnostic {
    lt::Diagnostic {
        range: lt::Range::new(
            lt::Position::new(problem.line, problem.col),
            lt::Position::new(problem.end_line, problem.end_col),
        ),
        severity: Some(match problem.severity {
            Severity::Error => lt::DiagnosticSeverity::ERROR,
            Severity::Warning => lt::DiagnosticSeverity::WARNING,
            Severity::Information => lt::DiagnosticSeverity::INFORMATION,
        }),
        source: Some("markup".to_owned()),
        message: problem.message.clone(),
        ..Default::default()
    }
}

impl CodeApp {
    pub(super) fn pump_markup(&mut self, ctx: &egui::Context) {
        if !self.settings.editor.markup_problems {
            self.markup.found.clear();
            self.markup.checked.clear();
            return;
        }
        if let Some(waiting) = &self.markup.waiting {
            match waiting.try_recv() {
                Ok((uid, version, path, problems)) => {
                    self.markup.waiting = None;
                    self.markup.checked.insert(uid, version);
                    let found = problems.iter().map(diagnostic).collect();
                    self.markup.found.insert(path_key(&path), (path, found));
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => self.markup.waiting = None,
            }
        }
        // Closed files keep nothing.
        let open: Vec<PathBuf> = self.docs.iter().filter_map(|d| d.path.as_deref().map(path_key)).collect();
        self.markup.found.retain(|key, _| open.contains(key));
        let changed = self.docs.iter().find(|d| {
            d.path.is_some()
                && markup::checks(&d.extension())
                && self.markup.checked.get(&d.uid) != Some(&d.buffer.version())
        });
        let Some(doc) = changed else {
            self.markup.changed_since = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let since = *self.markup.changed_since.get_or_insert(now);
        // A file just opened is read at once; typing waits for a pause.
        if self.markup.checked.contains_key(&doc.uid) && now - since < REST {
            ctx.request_repaint_after(Duration::from_secs_f64(REST));
            return;
        }
        self.markup.changed_since = None;
        let (uid, version, extension, source) = (doc.uid, doc.buffer.version(), doc.extension(), doc.text());
        let Some(path) = doc.path.clone() else { return };
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send((uid, version, path, markup::check(&extension, &source)));
            wake();
        });
        self.markup.waiting = Some(rx);
    }

    /// What's wrong in the markup of `key` (a [`path_key`]).
    pub(super) fn markup_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        self.markup.found.get(key).map(|(_, d)| d.clone()).unwrap_or_default()
    }

    /// Files with markup problems: (key, path).
    pub(super) fn markup_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.markup.found.iter().filter(|(_, (_, d))| !d.is_empty()).map(|(k, (p, _))| (k.clone(), p.clone())).collect()
    }
}
