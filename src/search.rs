//! "Search" side bar: find in files with the ripgrep crates. The search runs
//! on background threads (parallel walk that respects .gitignore) and streams
//! results to the panel; starting a new search cancels the previous one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::text::LayoutJob;
use egui::{RichText, TextFormat};
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkState;
use ignore::overrides::OverrideBuilder;

use crate::{finder, theme};

/// Stop after this many matches in total.
const MAX_MATCHES: usize = 20_000;
/// Longest line text kept for display.
const MAX_LINE_DISPLAY: usize = 240;
/// Wait this long after typing stops before searching.
const DEBOUNCE_SECS: f64 = 0.3;

pub struct LineMatch {
    /// 1-based line number.
    pub line: usize,
    /// Line text, leading whitespace trimmed and truncated for display.
    pub text: String,
    /// Byte ranges of matches within `text`.
    pub ranges: Vec<(usize, usize)>,
    /// Char column range of the first match within the original line.
    pub cols: (usize, usize),
}

pub struct FileResult {
    pub path: PathBuf,
    pub matches: Vec<LineMatch>,
}

enum Msg {
    File(FileResult),
    Done { files_searched: usize, truncated: bool },
    Error(String),
}

/// Where a clicked result points: file, 1-based line and char columns.
pub struct Jump {
    pub path: PathBuf,
    pub line: usize,
    pub cols: (usize, usize),
}

#[derive(Clone, PartialEq, Default)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include: String,
    pub exclude: String,
}

#[derive(Default)]
pub struct SearchPanel {
    pub query: Query,
    pub focus_requested: bool,
    show_globs: bool,
    /// Query of the running/finished search, and when the input last changed.
    searched: Option<Query>,
    edited_at: Option<f64>,
    results: Vec<FileResult>,
    collapsed: HashSet<PathBuf>,
    rx: Option<Receiver<Msg>>,
    cancel: Option<Arc<AtomicBool>>,
    running: bool,
    files_searched: usize,
    truncated: bool,
    error: Option<String>,
}

impl SearchPanel {
    /// Draws the panel. Returns where to jump when the user clicks a result.
    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<Jump> {
        let Some(root) = root else {
            ui.label(RichText::new("Open a folder to search in files.").color(theme::colors().text_dim));
            return None;
        };
        self.receive();
        if self.running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }

        let before = self.query.clone();
        let mut run_now = false;
        // Lay out right-to-left so the toggles take their width first and the
        // search box only fills what is left; asking for more than that makes
        // the side panel grow every frame and crush the editor area.
        let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let toggle = |ui: &mut egui::Ui, on: &mut bool, text: &str, tip: &str| {
                if ui.selectable_label(*on, RichText::new(text).monospace()).on_hover_text(tip).clicked() {
                    *on = !*on;
                }
            };
            toggle(ui, &mut self.query.regex, ".*", "Use Regular Expression");
            toggle(ui, &mut self.query.whole_word, "ab", "Match Whole Word");
            toggle(ui, &mut self.query.match_case, "Aa", "Match Case");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query.text).hint_text("Search").desired_width(ui.available_width()),
            );
            if self.focus_requested {
                edit.request_focus();
                self.focus_requested = false;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run_now = true;
            }
        });
        if ui.small_button(if self.show_globs { "▾ files to include/exclude" } else { "▸ files to include/exclude" }).clicked() {
            self.show_globs = !self.show_globs;
        }
        if self.show_globs {
            ui.add(egui::TextEdit::singleline(&mut self.query.include).hint_text("files to include, e.g. src/**, *.rs"));
            ui.add(egui::TextEdit::singleline(&mut self.query.exclude).hint_text("files to exclude, e.g. *.md, tests/**"));
        }

        let now = ui.input(|i| i.time);
        if self.query != before {
            self.edited_at = Some(now);
        }
        // Search as you type, after a short pause (VS Code behaviour).
        let debounced = self.edited_at.is_some_and(|t| now - t >= DEBOUNCE_SECS);
        if let Some(t) = self.edited_at
            && !debounced
        {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((DEBOUNCE_SECS - (now - t)).max(0.01)));
        }
        if (run_now || debounced) && self.searched.as_ref() != Some(&self.query) {
            self.edited_at = None;
            self.start(root);
        } else if debounced {
            self.edited_at = None;
        }

        self.summary(ui);
        ui.separator();
        self.results_ui(ui, root)
    }

    fn summary(&self, ui: &mut egui::Ui) {
        let dim = theme::colors().text_dim;
        if let Some(err) = &self.error {
            ui.label(RichText::new(err).color(egui::Color32::from_rgb(0xf1, 0x4c, 0x4c)));
            return;
        }
        if self.searched.as_ref().is_none_or(|q| q.text.is_empty()) {
            return;
        }
        let matches: usize = self.results.iter().map(|f| f.matches.len()).sum();
        let mut text = format!("{matches} results in {} files", self.results.len());
        if self.running {
            text.push_str(" — searching…");
        } else if self.truncated {
            text.push_str(&format!(" (stopped at {MAX_MATCHES}; refine the search)"));
        } else {
            text.push_str(&format!(" ({} files searched)", self.files_searched));
        }
        ui.label(RichText::new(text).small().color(dim));
    }

    fn results_ui(&mut self, ui: &mut egui::Ui, root: &Path) -> Option<Jump> {
        let mut jump = None;
        let mut toggle = None;
        let pal = theme::colors();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            // Long lines end in "…" instead of widening the side bar.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            for file in &self.results {
                let rel = file.path.strip_prefix(root).unwrap_or(&file.path);
                let collapsed = self.collapsed.contains(&file.path);
                let arrow = if collapsed { "▸" } else { "▾" };
                let header = format!("{arrow} {}  ({})", rel.display(), file.matches.len());
                if ui.add(egui::Button::new(RichText::new(header).strong()).frame(false)).clicked() {
                    toggle = Some(file.path.clone());
                }
                if collapsed {
                    continue;
                }
                for m in &file.matches {
                    let job = highlighted(m, &pal);
                    if ui.add(egui::Button::new(job).frame(false)).on_hover_text(format!("Line {}", m.line)).clicked() {
                        jump = Some(Jump { path: file.path.clone(), line: m.line, cols: m.cols });
                    }
                }
            }
        });
        if let Some(p) = toggle
            && !self.collapsed.remove(&p)
        {
            self.collapsed.insert(p);
        }
        jump
    }

    fn receive(&mut self) {
        let Some(rx) = &self.rx else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::File(f) => self.results.push(f),
                Msg::Done { files_searched, truncated } => {
                    self.files_searched = files_searched;
                    self.truncated = truncated;
                    self.running = false;
                }
                Msg::Error(e) => {
                    self.error = Some(e);
                    self.running = false;
                }
            }
        }
    }

    /// Cancels any running search and starts a new one for the current query.
    fn start(&mut self, root: &Path) {
        if let Some(c) = self.cancel.take() {
            c.store(true, Ordering::Relaxed);
        }
        self.results.clear();
        self.collapsed.clear();
        self.error = None;
        self.truncated = false;
        self.files_searched = 0;
        self.searched = Some(self.query.clone());
        if self.query.text.is_empty() {
            self.running = false;
            self.rx = None;
            return;
        }
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.cancel = Some(cancel.clone());
        self.running = true;
        let (query, root) = (self.query.clone(), root.to_path_buf());
        std::thread::spawn(move || run_search(&query, &root, &tx, &cancel));
    }
}

fn build_matcher(q: &Query) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!q.match_case)
        .word(q.whole_word)
        .fixed_strings(!q.regex)
        .build(&q.text)
        .map_err(|e| format!("Invalid search: {e}"))
}

fn run_search(q: &Query, root: &Path, tx: &Sender<Msg>, cancel: &Arc<AtomicBool>) {
    let matcher = match build_matcher(q) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(Msg::Error(e));
            return;
        }
    };
    let mut walk = finder::walker(root);
    let mut overrides = OverrideBuilder::new(root);
    let globs = |s: &str| s.split(',').map(str::trim).filter(|g| !g.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    for g in globs(&q.include) {
        if let Err(e) = overrides.add(&g) {
            let _ = tx.send(Msg::Error(format!("Bad include glob `{g}`: {e}")));
            return;
        }
    }
    for g in globs(&q.exclude) {
        if let Err(e) = overrides.add(&format!("!{g}")) {
            let _ = tx.send(Msg::Error(format!("Bad exclude glob `{g}`: {e}")));
            return;
        }
    }
    match overrides.build() {
        Ok(o) => {
            walk.overrides(o);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error(format!("Bad glob: {e}")));
            return;
        }
    }

    let total = Arc::new(AtomicUsize::new(0));
    let files = Arc::new(AtomicUsize::new(0));
    walk.build_parallel().run(|| {
        let (matcher, tx, cancel, total, files) = (matcher.clone(), tx.clone(), cancel.clone(), total.clone(), files.clone());
        let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) || total.load(Ordering::Relaxed) >= MAX_MATCHES {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            files.fetch_add(1, Ordering::Relaxed);
            let mut matches = Vec::new();
            let sink = UTF8(|line_number, line| {
                if let Some(m) = line_match(&matcher, line_number as usize, line) {
                    matches.push(m);
                }
                Ok(total.load(Ordering::Relaxed) + matches.len() < MAX_MATCHES)
            });
            let _ = searcher.search_path(&matcher, entry.path(), sink);
            if !matches.is_empty() {
                total.fetch_add(matches.len(), Ordering::Relaxed);
                if tx.send(Msg::File(FileResult { path: entry.into_path(), matches })).is_err() {
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    if !cancel.load(Ordering::Relaxed) {
        let _ = tx.send(Msg::Done {
            files_searched: files.load(Ordering::Relaxed),
            truncated: total.load(Ordering::Relaxed) >= MAX_MATCHES,
        });
    }
}

/// Matches of one line, trimmed for display.
fn line_match(matcher: &RegexMatcher, line: usize, raw: &str) -> Option<LineMatch> {
    let content = raw.trim_end_matches(['\n', '\r']);
    let mut found: Vec<(usize, usize)> = Vec::new();
    let _ = matcher.find_iter(content.as_bytes(), |m| {
        if m.end() > m.start() {
            found.push((m.start(), m.end()));
        }
        true
    });
    let &(first_start, first_end) = found.first()?;
    let cols = (content[..first_start].chars().count(), content[..first_end].chars().count());

    // Trim leading whitespace and cut very long lines around the first match.
    let lead = content.len() - content.trim_start().len();
    let mut start = lead;
    if first_start > start + MAX_LINE_DISPLAY / 2 {
        start = first_start - MAX_LINE_DISPLAY / 4;
        while !content.is_char_boundary(start) {
            start -= 1;
        }
    }
    let mut end = content.len().min(start + MAX_LINE_DISPLAY);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let text = content[start..end].trim_end().to_owned();
    let ranges = found
        .into_iter()
        .filter(|&(s, e)| s >= start && e <= start + text.len())
        .map(|(s, e)| (s - start, e - start))
        .collect();
    Some(LineMatch { line, text, ranges, cols })
}

/// Line text with matches highlighted.
fn highlighted(m: &LineMatch, pal: &theme::Palette) -> LayoutJob {
    let font = egui::FontId::monospace(12.0);
    let plain = TextFormat { font_id: font.clone(), color: pal.text, ..Default::default() };
    let hit = TextFormat { font_id: font.clone(), color: pal.text, background: pal.selection, ..Default::default() };
    let mut job = LayoutJob::default();
    job.append(&format!("{:>5}  ", m.line), 0.0, TextFormat { font_id: font, color: pal.text_dim, ..Default::default() });
    let mut at = 0;
    for &(s, e) in &m.ranges {
        if s < at {
            continue;
        }
        job.append(&m.text[at..s], 0.0, plain.clone());
        job.append(&m.text[s..e], 0.0, hit.clone());
        at = e;
    }
    job.append(&m.text[at..], 0.0, plain);
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn search(root: &Path, q: Query) -> (Vec<FileResult>, Option<String>) {
        let (tx, rx) = channel();
        run_search(&q, root, &tx, &Arc::new(AtomicBool::new(false)));
        drop(tx);
        let mut files = Vec::new();
        let mut err = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Ok(msg) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            match msg {
                Msg::File(f) => files.push(f),
                Msg::Error(e) => err = Some(e),
                Msg::Done { .. } => break,
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files, err)
    }

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust-code-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {\n    let Foo = foo_bar();\n}\n").unwrap();
        std::fs::write(dir.join("notes.md"), "foo in markdown\n").unwrap();
        std::fs::write(dir.join("target/out.rs"), "foo").unwrap();
        std::fs::write(dir.join("bin.dat"), b"foo\0\x01\x02").unwrap();
        dir
    }

    #[test]
    fn literal_case_word_regex_and_globs() {
        let dir = fixture();
        let q = |text: &str| Query { text: text.to_owned(), ..Default::default() };

        let (files, _) = search(&dir, q("foo"));
        let names: Vec<String> = files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        // Sorted by path: <dir>/notes.md before <dir>/src/a.rs. target/ and the binary file are skipped.
        assert_eq!(names, ["notes.md", "a.rs"]);
        let a = files.iter().find(|f| f.path.ends_with("a.rs")).unwrap();
        assert_eq!(a.matches[0].line, 2);
        assert_eq!(a.matches[0].text, "let Foo = foo_bar();", "leading whitespace trimmed");
        assert_eq!(a.matches[0].ranges.len(), 2, "case-insensitive by default");
        assert_eq!(a.matches[0].cols, (8, 11));

        let (files, _) = search(&dir, Query { match_case: true, ..q("Foo") });
        assert_eq!(files.iter().map(|f| f.matches.len()).sum::<usize>(), 1);

        let (files, _) = search(&dir, Query { whole_word: true, ..q("foo") });
        let a = files.iter().find(|f| f.path.ends_with("a.rs")).unwrap();
        assert_eq!(a.matches[0].ranges, vec![(4, 7)], "foo_bar is not the word foo");

        let (files, _) = search(&dir, Query { regex: true, ..q(r"foo_\w+") });
        assert_eq!(files.len(), 1);

        let (files, _) = search(&dir, Query { include: "*.md".to_owned(), ..q("foo") });
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("notes.md"));

        let (files, _) = search(&dir, Query { exclude: "*.md".to_owned(), ..q("foo") });
        assert!(files.iter().all(|f| !f.path.ends_with("notes.md")));

        let (_, err) = search(&dir, Query { regex: true, ..q("(") });
        assert!(err.is_some(), "invalid regex is reported");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let matcher = build_matcher(&Query { text: "needle".to_owned(), ..Default::default() }).unwrap();
        let line = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let m = line_match(&matcher, 1, &line).unwrap();
        assert!(m.text.len() <= MAX_LINE_DISPLAY);
        assert_eq!(&m.text[m.ranges[0].0..m.ranges[0].1], "needle");
        assert_eq!(m.cols, (1000, 1006));
    }
}
