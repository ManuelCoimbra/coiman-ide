//! Coverage: reports found around a project or left by a test run, shown beside
//! the code and in a list of files, least covered first.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use eframe::egui::{self, Color32, RichText};
use rust_code_core::coverage::{self, LineCoverage};

use super::lsp::path_key;
use super::{CodeApp, PanelTab, repaint};
use crate::editor_view::Decorations;
use crate::grid_view::{self, GridSource, GridState};
use crate::theme;

/// Seconds between looks for a new report.
const CHECK_EVERY: f64 = 5.0;

const COVERED: Color32 = Color32::from_rgb(0x4E, 0xC9, 0x6A);
const PARTIAL: Color32 = Color32::from_rgb(0xE2, 0xB9, 0x3C);
const MISSED: Color32 = Color32::from_rgb(0xF1, 0x4C, 0x4C);

struct Loaded {
    /// Where it came from, for saying so.
    label: String,
    written: Option<SystemTime>,
    files: Vec<(PathBuf, Vec<LineCoverage>)>,
    lost: Vec<String>,
    problem: Option<String>,
    branches: (u64, u64),
    functions: (u64, u64),
}

struct FileRow {
    path: PathBuf,
    name: String,
    covered: usize,
    total: usize,
}

pub(super) struct Coverage {
    pub(super) showing: bool,
    label: String,
    written: Option<SystemTime>,
    /// The report on disk last looked at: not looked at again until it changes.
    seen: Option<(PathBuf, SystemTime)>,
    branches: (u64, u64),
    functions: (u64, u64),
    /// A file's text as the report knew it, kept once it starts to change.
    bases: RefCell<HashMap<PathBuf, String>>,
    /// Marks moved to follow edits, for one version of one document.
    followed: RefCell<Option<(u64, u64, Vec<LineCoverage>)>>,
    /// By [`path_key`]: the path as it's written, and its lines.
    files: HashMap<PathBuf, (PathBuf, Vec<LineCoverage>)>,
    /// From plugins, by plugin then file.
    from_plugins: HashMap<(String, PathBuf), (PathBuf, Vec<LineCoverage>)>,
    rows: Vec<FileRow>,
    lost: Vec<String>,
    problem: Option<String>,
    loading: Option<Receiver<Loaded>>,
    next_check: f64,
    grid: GridState,
}

impl Default for Coverage {
    fn default() -> Self {
        Self {
            showing: true,
            label: String::new(),
            written: None,
            seen: None,
            branches: (0, 0),
            functions: (0, 0),
            bases: RefCell::default(),
            followed: RefCell::default(),
            files: HashMap::new(),
            from_plugins: HashMap::new(),
            rows: Vec::new(),
            lost: Vec::new(),
            problem: None,
            loading: None,
            next_check: 0.0,
            grid: GridState::default(),
        }
    }
}

fn load(path: &Path, root: &Path, label: String) -> Loaded {
    let written = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let empty = |problem: String| Loaded {
        label: label.clone(),
        written,
        files: Vec::new(),
        lost: Vec::new(),
        problem: Some(problem),
        branches: (0, 0),
        functions: (0, 0),
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => return empty(format!("{}: {e}", path.display())),
    };
    let report = match coverage::parse(&text) {
        Ok(report) => report,
        Err(why) => return empty(format!("{}: {why}", path.display())),
    };
    let (branches, functions) = (report.branches, report.functions);
    let files = rust_code_core::stacktrace::project_files(root);
    let (files, lost) = coverage::resolve(report, root, &files);
    Loaded { label, written, files, lost, problem: None, branches, functions }
}

struct Source<'a> {
    rows: &'a [FileRow],
}

impl GridSource for Source<'_> {
    fn rows(&self) -> usize {
        self.rows.len()
    }

    fn columns(&self) -> usize {
        4
    }

    fn header(&self, column: usize) -> String {
        ["File", "Covered", "Lines", "%"][column.min(3)].to_owned()
    }

    fn row(&self, row: usize) -> Vec<String> {
        let Some(r) = self.rows.get(row) else { return Vec::new() };
        let percent = coverage::percent(r.covered, r.total);
        vec![r.name.clone(), r.covered.to_string(), r.total.to_string(), format!("{percent:.1}")]
    }

    fn row_label(&self, row: usize) -> String {
        (row + 1).to_string()
    }

    fn cell_color(&self, row: usize, column: usize) -> Option<Color32> {
        if column != 3 {
            return None;
        }
        let r = self.rows.get(row)?;
        let percent = coverage::percent(r.covered, r.total);
        Some(if percent >= 80.0 {
            COVERED
        } else if percent >= 50.0 {
            PARTIAL
        } else {
            MISSED
        })
    }
}

impl CodeApp {
    pub(super) fn pump_coverage(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.coverage.loading
            && let Ok(done) = rx.try_recv()
        {
            self.coverage.loading = None;
            self.apply_coverage(done);
        }
        let now = ctx.input(|i| i.time);
        if now < self.coverage.next_check || self.coverage.loading.is_some() {
            return;
        }
        self.coverage.next_check = now + CHECK_EVERY;
        let Some(root) = self.explorer.root.clone() else { return };
        let newest = coverage::newest_report(&root);
        // The report shown from disk was deleted, and none has taken its place.
        if newest.is_none() && self.coverage.seen.take().is_some() {
            self.coverage.files.clear();
            self.coverage.written = None;
            self.coverage.label.clear();
            self.coverage.lost.clear();
            self.coverage.branches = (0, 0);
            self.coverage.functions = (0, 0);
            self.coverage.bases.borrow_mut().clear();
            self.rebuild_coverage_rows();
        }
        let Some((path, modified)) = newest else { return };
        let fresh = self.coverage.seen.as_ref().is_none_or(|(p, m)| *p != path || *m != modified);
        // A test run's report stands until something newer comes along. The one
        // that's showing can change to anything: a restored older copy is still it.
        let same = self.coverage.seen.as_ref().is_some_and(|(p, _)| *p == path);
        let newer = same || self.coverage.written.is_none_or(|w| modified > w);
        if fresh && newer {
            self.coverage.seen = Some((path.clone(), modified));
            let label = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            self.load_coverage(ctx, path, root, label, false);
        }
    }

    fn load_coverage(&mut self, ctx: &egui::Context, path: PathBuf, root: PathBuf, label: String, remove_after: bool) {
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let loaded = load(&path, &root, label);
            if remove_after {
                let _ = std::fs::remove_file(&path);
            }
            let _ = tx.send(loaded);
            wake();
        });
        self.coverage.loading = Some(rx);
    }

    fn apply_coverage(&mut self, done: Loaded) {
        let state = &mut self.coverage;
        state.problem = done.problem;
        if state.problem.is_some() {
            return;
        }
        state.label = done.label;
        state.written = done.written;
        state.lost = done.lost;
        state.branches = done.branches;
        state.functions = done.functions;
        state.bases.borrow_mut().clear();
        *state.followed.borrow_mut() = None;
        state.files = done.files.into_iter().map(|(path, lines)| (path_key(&path), (path, lines))).collect();
        self.rebuild_coverage_rows();
    }

    fn rebuild_coverage_rows(&mut self) {
        let root = self.explorer.root.clone();
        let state = &mut self.coverage;
        let named = state.files.values().chain(state.from_plugins.values());
        let mut rows: Vec<FileRow> = named
            .map(|(path, lines)| {
                let (covered, total) = coverage::counts(lines);
                let shown = root.as_ref().and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
                FileRow { name: shown.to_string_lossy().replace('\\', "/"), path: path.clone(), covered, total }
            })
            .collect();
        rows.sort_by(|a, b| {
            coverage::percent(a.covered, a.total)
                .total_cmp(&coverage::percent(b.covered, b.total))
                .then_with(|| a.name.cmp(&b.name))
        });
        state.rows = rows;
    }

    /// A test run left this folder: take its report, and clear up.
    pub(super) fn take_run_coverage(&mut self, ctx: &egui::Context, dir: &Path, name: &str) {
        let Some(root) = self.explorer.root.clone() else { return };
        match coverage::find_report_in(dir) {
            Some(report) => self.load_coverage(ctx, report, root, format!("{name} test run"), true),
            None => self.coverage.problem = Some(format!("{name} wrote no coverage report: see Output")),
        }
        self.coverage.showing = true;
        self.panel_tab = PanelTab::Coverage;
        self.panel_visible = true;
    }

    /// A plugin's lines for a file. None clears them.
    pub(super) fn set_plugin_coverage(&mut self, plugin: &str, file: &Path, lines: Vec<LineCoverage>) {
        let key = (plugin.to_owned(), path_key(file));
        if lines.is_empty() {
            self.coverage.from_plugins.remove(&key);
        } else {
            self.coverage.from_plugins.insert(key, (file.to_path_buf(), lines));
        }
        self.rebuild_coverage_rows();
    }

    fn coverage_lines(&self, file: &Path) -> Option<&Vec<LineCoverage>> {
        let key = path_key(file);
        self.coverage
            .from_plugins
            .iter()
            .find(|((_, k), _)| *k == key)
            .map(|(_, (_, lines))| lines)
            .or_else(|| self.coverage.files.get(&key).map(|(_, lines)| lines))
    }

    /// A document's marks where its lines are now, and whether they had to move. None
    /// if the file changed where it couldn't be watched, so its lines can't be trusted.
    fn coverage_now(&self, doc: &crate::document::Document) -> Option<(Vec<LineCoverage>, bool)> {
        let path = doc.path.as_ref()?;
        let lines = self.coverage_lines(path)?;
        let changed = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let newer = changed.zip(self.coverage.written).is_some_and(|(c, w)| c > w);
        if !doc.is_dirty() && !newer && !self.coverage.bases.borrow().contains_key(&path_key(path)) {
            return Some((lines.clone(), false));
        }
        let key = path_key(path);
        let mut bases = self.coverage.bases.borrow_mut();
        if !bases.contains_key(&key) {
            if newer {
                return None;
            }
            bases.insert(key.clone(), std::fs::read_to_string(path).ok()?);
        }
        let version = doc.buffer.version();
        let mut cache = self.coverage.followed.borrow_mut();
        if let Some((uid, at, moved)) = &*cache
            && *uid == doc.uid
            && *at == version
        {
            return Some((moved.clone(), true));
        }
        let moved = coverage::follow_lines(lines, bases.get(&key)?, &doc.text());
        *cache = Some((doc.uid, version, moved.clone()));
        Some((moved, true))
    }

    pub(super) fn coverage_decorate(&self, i: usize, deco: &mut Decorations) {
        if !self.coverage.showing {
            return;
        }
        let Some(doc) = self.docs.get(i) else { return };
        let Some((lines, _)) = self.coverage_now(doc) else { return };
        deco.coverage = lines
            .iter()
            .map(|l| {
                let color = match (l.hits > 0, l.partial) {
                    (false, _) => MISSED,
                    (true, true) => PARTIAL,
                    (true, false) => COVERED,
                };
                ((l.line as usize).saturating_sub(1), color)
            })
            .collect();
    }

    /// The status bar's label.
    pub(super) fn coverage_label(&self) -> Option<(String, String)> {
        let doc = self.docs.get(self.active)?;
        let path = doc.path.as_ref()?;
        let lines = self.coverage_lines(path)?;
        let (covered, total) = coverage::counts(lines);
        let percent = coverage::percent(covered, total);
        match self.coverage_now(doc) {
            None => {
                let tip =
                    "This file changed after the report was written, so its marks are hidden. Run the tests again.";
                return Some((format!("Coverage {percent:.0}% (old)"), tip.to_owned()));
            }
            Some((_, true)) => {
                let tip = "The marks follow your edits. Changed lines have none until the tests run again.";
                return Some((format!("Coverage {percent:.0}% (edited)"), tip.to_owned()));
            }
            Some(_) => {}
        }
        let tip = format!("{covered} of {total} lines ran, from {}. Click for all files.", self.coverage.label);
        Some((format!("Coverage {percent:.0}%"), tip))
    }

    pub(super) fn has_coverage(&self) -> bool {
        !self.coverage.rows.is_empty() || self.coverage.problem.is_some()
    }

    pub(super) fn toggle_coverage(&mut self) {
        self.coverage.showing = !self.coverage.showing;
    }

    pub(super) fn show_coverage_panel(&mut self) {
        if !self.has_coverage() {
            self.status = Some("No coverage report here yet: run tests with coverage, or load a report".to_owned());
            return;
        }
        self.panel_tab = PanelTab::Coverage;
        self.panel_visible = true;
    }

    /// `Coverage: Load Report`.
    pub(super) fn pick_coverage_report(&mut self, ctx: &egui::Context) {
        let Some(path) = self.file_dialog("coverage_report").pick_file() else { return };
        self.dialog_chose("coverage_report", &path);
        let root = self.explorer.root.clone().or_else(|| path.parent().map(Path::to_path_buf)).unwrap_or_default();
        let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.load_coverage(ctx, path, root, label, false);
        self.coverage.showing = true;
    }

    pub(super) fn coverage_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let (covered, total) = self.coverage.rows.iter().fold((0, 0), |(c, t), r| (c + r.covered, t + r.total));
        let mut hide = self.coverage.showing;
        ui.horizontal(|ui| {
            let percent = coverage::percent(covered, total);
            ui.label(RichText::new(format!("{percent:.1}%")).strong());
            let count = self.coverage.rows.len();
            ui.label(format!("{covered} of {total} lines, in {count} file{}", if count == 1 { "" } else { "s" }));
            if !self.coverage.label.is_empty() {
                ui.label(RichText::new(format!("from {}", self.coverage.label)).color(pal.text_dim));
            }
            for (name, (ran, all)) in [("branches", self.coverage.branches), ("functions", self.coverage.functions)] {
                if all > 0 {
                    ui.label(format!("{ran} of {all} {name}"));
                }
            }
            ui.checkbox(&mut hide, "Show beside the code");
        });
        self.coverage.showing = hide;
        let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
        if let Some(problem) = &self.coverage.problem {
            ui.label(RichText::new(problem).color(warn));
        }
        if !self.coverage.lost.is_empty() {
            let text = format!(
                "Not in this project, from the report: {}",
                self.coverage.lost.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
            );
            ui.add(egui::Label::new(RichText::new(text).color(warn)).truncate());
        }
        if self.coverage.grid.widths.is_empty() {
            let w = self.font_size * 0.65;
            self.coverage.grid.widths = vec![w * 48.0, w * 9.0, w * 7.0, w * 7.0];
        }
        let state = &mut self.coverage;
        let source = Source { rows: &state.rows };
        let id = ui.scope_id().with("coverage");
        let events = grid_view::show(ui, id, &mut state.grid, &source, self.font_size);
        let open = events.row_opened.and_then(|row| state.rows.get(row)).map(|r| r.path.clone());
        if let Some(path) = open {
            self.open_file(&path);
        }
    }

    /// Where a run with coverage should write.
    pub(super) fn coverage_folder() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join("rust-code-tests").join(format!(
            "cov-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }
}
