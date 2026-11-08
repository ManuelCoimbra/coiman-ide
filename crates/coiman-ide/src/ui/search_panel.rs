//! "Search" side bar: find in files (see `coiman_core::project::search`) with
//! search-as-you-type, streamed results grouped by file, and match highlights.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use coiman_core::editor::find::FindOptions;
use coiman_core::project::search::{FileResult, LineMatch, MAX_MATCHES, Msg, Query, SearchHandle};
use egui::text::LayoutJob;
use egui::{RichText, TextFormat};

use crate::ui::icons::{self, Icon};
use crate::ui::theme;

/// Wait this long after typing stops before searching.
const DEBOUNCE_SECS: f64 = 0.3;

/// Where a clicked result points: file, 1-based line and char columns.
pub struct Jump {
    pub path: PathBuf,
    pub line: usize,
    pub cols: (usize, usize),
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
    handle: Option<SearchHandle>,
    running: bool,
    files_searched: usize,
    truncated: bool,
    error: Option<String>,
    show_replace: bool,
    replacement: String,
    /// Asking before replacing in every file.
    confirming: bool,
    /// Confirmed: the app carries it out.
    pub replace_requested: bool,
}

/// What Replace All works on.
pub struct ProjectReplace {
    pub options: FindOptions,
    pub with: String,
    pub files: Vec<PathBuf>,
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
                egui::TextEdit::singleline(&mut self.query.text)
                    .hint_text("Search")
                    .desired_width(ui.available_width()),
            );
            if self.focus_requested {
                edit.request_focus();
                self.focus_requested = false;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run_now = true;
            }
        });
        let globs = ui.horizontal(|ui| {
            let size = ui.text_style_height(&egui::TextStyle::Small);
            let arrow = if self.show_globs { Icon::ChevronDown } else { Icon::ChevronRight };
            let icon = icons::show(ui, arrow, size, theme::colors().text_dim).interact(egui::Sense::click()).clicked();
            icon || ui.small_button("files to include/exclude").clicked()
        });
        if globs.inner {
            self.show_globs = !self.show_globs;
        }
        if self.show_globs {
            ui.add(
                egui::TextEdit::singleline(&mut self.query.include).hint_text("files to include, e.g. src/**, *.rs"),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.query.exclude).hint_text("files to exclude, e.g. *.md, tests/**"),
            );
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
            self.start(root, ui.ctx());
        } else if debounced {
            self.edited_at = None;
        }

        self.summary(ui);
        self.replace_ui(ui);
        ui.separator();
        self.results_ui(ui, root)
    }

    fn replace_ui(&mut self, ui: &mut egui::Ui) {
        let arrow = if self.show_replace { Icon::ChevronDown } else { Icon::ChevronRight };
        let size = ui.text_style_height(&egui::TextStyle::Small);
        let toggled = ui.horizontal(|ui| {
            let icon = icons::show(ui, arrow, size, theme::colors().text_dim).interact(egui::Sense::click()).clicked();
            icon || ui.small_button("replace").clicked()
        });
        if toggled.inner {
            self.show_replace = !self.show_replace;
            self.confirming = false;
        }
        if !self.show_replace {
            return;
        }
        let hint = if self.query.regex { "Replace ($1 for groups)" } else { "Replace" };
        ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text(hint));
        let matches: usize = self.results.iter().map(|f| f.matches.len()).sum();
        let files = self.results.len();
        if self.confirming {
            let more = if self.truncated { "at least " } else { "" };
            ui.label(format!(
                "Replace {more}{matches} occurrences in {files} files? Files that aren't open change on disk."
            ));
            ui.horizontal(|ui| {
                if ui.button("Replace All").clicked() {
                    self.confirming = false;
                    self.replace_requested = true;
                }
                if ui.button("Cancel").clicked() {
                    self.confirming = false;
                }
            });
        } else if ui.add_enabled(files > 0 && !self.running, egui::Button::new("Replace All...")).clicked() {
            self.confirming = true;
        }
    }

    /// What Replace All should do: the search that produced the results shown.
    pub fn project_replace(&self) -> Option<ProjectReplace> {
        let q = self.searched.as_ref()?;
        Some(ProjectReplace {
            options: FindOptions {
                query: q.text.clone(),
                match_case: q.match_case,
                whole_word: q.whole_word,
                regex: q.regex,
            },
            with: self.replacement.clone(),
            files: self.results.iter().map(|f| f.path.clone()).collect(),
        })
    }

    /// Runs the search again (files changed).
    pub fn search_again(&mut self, now: f64) {
        self.searched = None;
        self.edited_at = Some(now - DEBOUNCE_SECS);
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
                let header = format!("{}  ({})", rel.display(), file.matches.len());
                let clicked = ui
                    .horizontal(|ui| {
                        let size = ui.text_style_height(&egui::TextStyle::Body);
                        let arrow = if collapsed { Icon::ChevronRight } else { Icon::ChevronDown };
                        let icon = icons::show(ui, arrow, size, pal.text_dim).interact(egui::Sense::click()).clicked();
                        icon || ui.add(egui::Button::new(RichText::new(header).strong()).frame(false)).clicked()
                    })
                    .inner;
                if clicked {
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
        let Some(handle) = &self.handle else { return };
        while let Some(msg) = handle.try_recv() {
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
    fn start(&mut self, root: &Path, ctx: &egui::Context) {
        // Dropping the previous handle cancels that search.
        self.handle = None;
        self.results.clear();
        self.collapsed.clear();
        self.error = None;
        self.truncated = false;
        self.files_searched = 0;
        self.searched = Some(self.query.clone());
        if self.query.text.is_empty() {
            self.running = false;
            return;
        }
        self.running = true;
        let ctx = ctx.clone();
        self.handle = Some(SearchHandle::spawn(self.query.clone(), root.to_path_buf(), move || ctx.request_repaint()));
    }
}

/// Line text with matches highlighted.
fn highlighted(m: &LineMatch, pal: &theme::Palette) -> LayoutJob {
    let font = egui::FontId::monospace(12.0);
    let plain = TextFormat { font_id: font.clone(), color: pal.text, ..Default::default() };
    let hit = TextFormat { font_id: font.clone(), color: pal.text, background: pal.selection, ..Default::default() };
    let mut job = LayoutJob::default();
    job.append(
        &format!("{:>5}  ", m.line),
        0.0,
        TextFormat { font_id: font, color: pal.text_dim, ..Default::default() },
    );
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
