//! Editing by structure, searching by syntax tree, and a project's own lint rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText, TextEdit};
use rust_code_core::buffer::Selection;
use rust_code_core::lsp::types as lt;
use rust_code_core::syntax::Lang;
use rust_code_core::tree_edit::{self, Change, Move};
use rust_code_core::tree_rules::{self, FileHits, Hit, RULES_FILE, Rule, Search, Severity};
use rust_code_core::trust;

use super::lsp::path_key;
use super::{CodeApp, PanelTab, repaint};
use crate::theme;

const RULES_CHECK_SECONDS: f64 = 2.0;
const SCAN_REST: f64 = 0.3;

enum Job {
    Search,
    Rules(Arc<Vec<Rule>>),
}

/// A walk over the project: what it's for, its answer, and how to stop it.
type Running = (Job, Receiver<(Vec<FileHits>, bool)>, Arc<AtomicBool>);

#[derive(Default)]
pub(super) struct TreeTools {
    used: bool,
    query: String,
    lang: Option<Lang>,
    error: Option<String>,
    results: Vec<(PathBuf, lt::Range, String)>,
    capped: bool,
    searched: bool,
    job: Option<Running>,
    rules: Arc<Vec<Rule>>,
    rule_problems: Vec<String>,
    rules_modified: Option<std::time::SystemTime>,
    rules_root: Option<PathBuf>,
    next_rules_check: f64,
    /// (document, version) last looked at, and when it was first seen changed.
    scanned: Option<(u64, u64)>,
    changed_since: Option<f64>,
    /// What rules found, by file.
    found: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
}

fn range_of(hit: &Hit) -> lt::Range {
    lt::Range {
        start: lt::Position { line: hit.start.0, character: hit.start.1 },
        end: lt::Position { line: hit.end.0, character: hit.end.1 },
    }
}

fn diagnostic(rule: &Rule, hit: &Hit) -> lt::Diagnostic {
    let severity = match rule.severity {
        Severity::Error => lt::DiagnosticSeverity::ERROR,
        Severity::Warning => lt::DiagnosticSeverity::WARNING,
        Severity::Info => lt::DiagnosticSeverity::INFORMATION,
    };
    lt::Diagnostic {
        range: range_of(hit),
        severity: Some(severity),
        source: Some(rule.id.clone()),
        message: tree_rules::fill(&rule.message, &hit.captures),
        ..Default::default()
    }
}

impl CodeApp {
    fn apply_change(&mut self, change: Change) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        doc.buffer.apply_edits(change.edits);
        if let Some(cursor) = change.cursor {
            doc.buffer.set_selections(vec![Selection::new(cursor, cursor)], 0);
        }
        doc.reveal = true;
    }

    /// Slurp, barf, raise and splice.
    pub(super) fn tree_move(&mut self, what: Move) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let primary = doc.buffer.primary();
        let result = if tree_edit::is_lisp(&doc.extension()) {
            tree_edit::lisp_move(&doc.text(), primary.head, what)
        } else if what == Move::Raise {
            match doc.fresh_tree() {
                Some(tree) => tree_edit::tree_raise(&tree, doc.buffer.rope(), primary.start(), primary.end()),
                None => Err("Raise needs a syntax tree or a Lisp file".to_owned()),
            }
        } else {
            Err("Slurp, barf and splice are for Lisp files: other languages need separators moved too".to_owned())
        };
        match result {
            Ok(change) => self.apply_change(change),
            Err(why) => self.status = Some(why),
        }
    }

    pub(super) fn tree_wrap(&mut self, open: &str, close: &str) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let primary = doc.buffer.primary();
        let (start, end) = (primary.start(), primary.end());
        let result = if tree_edit::is_lisp(&doc.extension()) {
            tree_edit::lisp_wrap(&doc.text(), start, end, open, close)
        } else if start != end {
            Ok(tree_edit::wrap_range(start, end, open, close))
        } else {
            match doc.fresh_tree().and_then(|tree| tree_edit::node_range(&tree, doc.buffer.rope(), start, end)) {
                Some((a, b)) => Ok(tree_edit::wrap_range(a, b, open, close)),
                None => Err("Select what to wrap: there's no syntax tree here".to_owned()),
            }
        };
        match result {
            Ok(change) => self.apply_change(change),
            Err(why) => self.status = Some(why),
        }
    }

    // ----- search --------------------------------------------------------------------

    pub(super) fn tree_search_tab_shown(&self) -> bool {
        self.tree_tools.used
    }

    /// `Search: Find by Syntax Tree`.
    pub(super) fn show_tree_search(&mut self) {
        let from_file = self.docs.get(self.active).and_then(|d| Lang::from_extension(&d.extension()));
        let tools = &mut self.tree_tools;
        tools.used = true;
        tools.lang = tools.lang.or(from_file).or(Some(Lang::Rust));
        self.panel_tab = PanelTab::TreeSearch;
        self.panel_visible = true;
    }

    fn tree_search_start(&mut self, ctx: &egui::Context) {
        let tools = &mut self.tree_tools;
        tools.error = None;
        let Some(lang) = tools.lang else { return };
        let query = match tree_rules::compile(lang, &tools.query) {
            Ok(query) => query,
            Err(why) => {
                tools.error = Some(why);
                return;
            }
        };
        let Some(root) = self.explorer.root.clone() else {
            self.tree_tools.error = Some("Open a folder to search it".to_owned());
            return;
        };
        let searches = vec![Search { lang, query: Arc::new(query) }];
        self.tree_tools_run(ctx, root, searches, Job::Search);
    }

    fn tree_tools_run(&mut self, ctx: &egui::Context, root: PathBuf, searches: Vec<Search>, job: Job) {
        if let Some((_, _, cancel)) = &self.tree_tools.job {
            cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (wake, stop) = (repaint(ctx), cancel.clone());
        std::thread::spawn(move || {
            let _ = tx.send(tree_rules::scan_project(&root, &searches, &stop));
            wake();
        });
        self.tree_tools.job = Some((job, rx, cancel));
    }

    fn take_job(&mut self) {
        let tools = &mut self.tree_tools;
        let Some((_, rx, _)) = &tools.job else { return };
        let Ok((files, capped)) = rx.try_recv() else { return };
        let Some((job, _, _)) = tools.job.take() else { return };
        match job {
            Job::Search => {
                tools.results.clear();
                tools.capped = capped;
                tools.searched = true;
                for file in files {
                    for (_, hit) in file.hits {
                        tools.results.push((file.path.clone(), range_of(&hit), hit.line_text));
                    }
                }
            }
            Job::Rules(rules) => {
                let (mut count, mut in_files) = (0, 0);
                tools.found.retain(|_, (_, diagnostics)| diagnostics.iter().any(|d| d.source.is_none()));
                for file in files {
                    let diagnostics: Vec<lt::Diagnostic> =
                        file.hits.iter().map(|(at, hit)| diagnostic(&rules[*at], hit)).collect();
                    count += diagnostics.len();
                    in_files += 1;
                    tools.found.insert(path_key(&file.path), (file.path, diagnostics));
                }
                tools.scanned = None;
                let more = if capped { " (stopped early)" } else { "" };
                self.status = Some(format!("Rules found {count} in {in_files} file(s){more}"));
            }
        }
    }

    // ----- rules ---------------------------------------------------------------------

    fn load_rules(&mut self, root: &Path) {
        let path = root.join(RULES_FILE);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (rules, problems) = tree_rules::parse_rules(&text);
        let tools = &mut self.tree_tools;
        tools.rules = Arc::new(rules);
        tools.found.clear();
        tools.scanned = None;
        tools.rule_problems = problems;
        if !tools.rule_problems.is_empty() {
            let at = lt::Range::default();
            let make = |message: &String| lt::Diagnostic {
                range: at,
                severity: Some(lt::DiagnosticSeverity::ERROR),
                message: message.clone(),
                ..Default::default()
            };
            let diagnostics = tools.rule_problems.iter().map(make).collect();
            tools.found.insert(path_key(&path), (path, diagnostics));
        }
    }

    pub(super) fn pump_tree_tools(&mut self, ctx: &egui::Context) {
        self.take_job();
        let now = ctx.input(|i| i.time);
        // A project's rules count once it's trusted, as its other settings do.
        if now >= self.tree_tools.next_rules_check {
            self.tree_tools.next_rules_check = now + RULES_CHECK_SECONDS;
            let root = self.explorer.root.clone().filter(|r| trust::is_trusted(r));
            let modified = root.as_ref().and_then(|r| std::fs::metadata(r.join(RULES_FILE)).ok()?.modified().ok());
            if modified != self.tree_tools.rules_modified || root != self.tree_tools.rules_root {
                self.tree_tools.rules_modified = modified;
                self.tree_tools.rules_root = root.clone();
                match root {
                    Some(root) => self.load_rules(&root),
                    None => {
                        self.tree_tools.rules = Arc::default();
                        self.tree_tools.found.clear();
                    }
                }
            }
        }
        if self.tree_tools.rules.is_empty() {
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let current = (doc.uid, doc.buffer.version());
        if self.tree_tools.scanned == Some(current) || doc.large {
            return;
        }
        let since = *self.tree_tools.changed_since.get_or_insert(now);
        if now - since < SCAN_REST {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_REST));
            return;
        }
        self.tree_tools.changed_since = None;
        self.tree_tools.scanned = Some(current);
        self.scan_active_doc();
    }

    fn scan_active_doc(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.clone() else { return };
        let Some(lang) = Lang::from_extension(&doc.extension()) else { return };
        let rules = self.tree_tools.rules.clone();
        let ours: Vec<&Rule> = rules.iter().filter(|r| r.lang == lang).collect();
        if ours.is_empty() {
            return;
        }
        let Some(tree) = doc.fresh_tree() else { return };
        let source = doc.text();
        let mut diagnostics = Vec::new();
        for rule in ours {
            diagnostics.extend(tree_rules::scan(&rule.query, &tree, &source).iter().map(|h| diagnostic(rule, h)));
        }
        self.tree_tools.found.insert(path_key(&path), (path, diagnostics));
    }

    /// `Rules: Check Project`.
    pub(super) fn rules_check_project(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else {
            self.status = Some("Open a folder to check it".to_owned());
            return;
        };
        if !self.allowed_to_run(&root) {
            return;
        }
        let rules = self.tree_tools.rules.clone();
        if rules.is_empty() {
            self.status = Some(format!("No rules yet: add them to {RULES_FILE}"));
            return;
        }
        let searches = rules.iter().map(|r| Search { lang: r.lang, query: r.query.clone() }).collect();
        self.status = Some("Checking the project's rules…".to_owned());
        self.tree_tools_run(ctx, root, searches, Job::Rules(rules));
    }

    /// `Rules: Apply Fixes in File`.
    pub(super) fn rules_fix_file(&mut self) {
        let rules = self.tree_tools.rules.clone();
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let Some(lang) = Lang::from_extension(&doc.extension()) else { return };
        let Some(tree) = doc.fresh_tree() else { return };
        let source = doc.text();
        let rope = doc.buffer.rope();
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for rule in rules.iter().filter(|r| r.lang == lang) {
            let Some(fix) = &rule.fix else { continue };
            for hit in tree_rules::scan(&rule.query, &tree, &source) {
                let (a, b) = (rope.byte_to_char(hit.start_byte), rope.byte_to_char(hit.end_byte));
                edits.push((a, b, tree_rules::fill(fix, &hit.captures)));
            }
        }
        // Where fixes overlap, the outermost goes first; run again for the rest.
        edits.sort_by_key(|(a, b, _)| (*a, std::cmp::Reverse(*b)));
        let mut end = 0;
        edits.retain(|(a, b, _)| {
            let keep = *a >= end;
            if keep {
                end = *b;
            }
            keep
        });
        let count = edits.len();
        doc.buffer.apply_edits(edits);
        self.status =
            Some(if count == 0 { "No fixes to apply".to_owned() } else { format!("Applied {count} fix(es)") });
    }

    pub(super) fn tree_rule_problems(&self, file: &Path) -> Vec<lt::Diagnostic> {
        self.tree_tools.found.get(file).map(|(_, d)| d.clone()).unwrap_or_default()
    }

    pub(super) fn tree_rule_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.tree_tools.found.iter().map(|(k, (p, _))| (k.clone(), p.clone())).collect()
    }

    // ----- panel ---------------------------------------------------------------------

    pub(super) fn tree_search_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        let warn = egui::Color32::from_rgb(0xF1, 0x4C, 0x4C);
        let mono = egui::TextStyle::Monospace.resolve(ui.style());
        let (mut search, mut from_cursor, mut open) = (false, false, None);
        let running = self.tree_tools.job.as_ref().is_some_and(|(job, ..)| matches!(job, Job::Search));
        let tools = &mut self.tree_tools;
        ui.horizontal(|ui| {
            let shown = tools.lang.map_or_else(String::new, |l| format!("{l:?}"));
            egui::ComboBox::from_id_salt("tree_search_lang").selected_text(shown).show_ui(ui, |ui| {
                for lang in Lang::ALL {
                    ui.selectable_value(&mut tools.lang, Some(lang), format!("{lang:?}"));
                }
            });
            search = ui.button("Search").on_hover_text("Ctrl+Enter in the query").clicked();
            from_cursor = ui.button("Use Node at Cursor").clicked();
            if running {
                ui.label(RichText::new("Searching…").color(pal.text_dim));
            } else if tools.searched {
                let more = if tools.capped { " (stopped there)" } else { "" };
                ui.label(RichText::new(format!("{} found{more}", tools.results.len())).color(pal.text_dim));
            }
        });
        let edit = TextEdit::multiline(&mut tools.query)
            .font(mono.clone())
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("(call_expression function: (identifier) @name (#eq? @name \"print\")) @match");
        let response = ui.add(edit);
        if response.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter)) {
            search = true;
        }
        if let Some(why) = &tools.error {
            ui.label(RichText::new(why).color(warn));
        }
        egui::ScrollArea::vertical().id_salt("tree_search_results").auto_shrink([false, false]).show(ui, |ui| {
            for (at, (path, range, line)) in tools.results.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let label = format!("{name}:{}  {line}", range.start.line + 1);
                let row =
                    egui::Label::new(RichText::new(label).font(mono.clone())).truncate().sense(egui::Sense::click());
                if ui.add(row).on_hover_text(path.display().to_string()).clicked() {
                    open = Some(at);
                }
            }
        });
        if from_cursor {
            self.tree_query_from_cursor();
        }
        if search {
            self.tree_search_start(&ctx);
        }
        if let Some((path, range, _)) = open.and_then(|at| self.tree_tools.results.get(at).cloned()) {
            self.jump_to(&path, range, true);
        }
    }

    fn tree_query_from_cursor(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let primary = doc.buffer.primary();
        let kind = doc
            .fresh_tree()
            .and_then(|tree| tree_edit::node_kind(&tree, doc.buffer.rope(), primary.start(), primary.end()));
        let lang = Lang::from_extension(&doc.extension());
        match kind {
            Some(kind) => {
                self.tree_tools.query = format!("({kind}) @match");
                self.tree_tools.lang = lang.or(self.tree_tools.lang);
            }
            None => self.tree_tools.error = Some("No syntax tree at the cursor".to_owned()),
        }
    }
}
