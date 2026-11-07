//! Editing by structure, searching by syntax tree, and a project's own lint rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use coiman_core::editor::buffer::Selection;
use coiman_core::editor::tree_edit::{self, Change, Move};
use coiman_core::language::lsp::types as lt;
use coiman_core::language::syntax::Lang;
use coiman_core::language::tree_rules::{self, FileHits, Hit, RULES_FILE, Rule, Search, Severity};
use coiman_core::project::trust;
use eframe::egui::{self, RichText, TextEdit};

use super::Document;
use super::lsp::path_key;
use super::{CodeApp, PanelTab, repaint};
use crate::theme;

const RULES_CHECK_SECONDS: f64 = 2.0;
const SCAN_REST: f64 = 0.3;

enum Job {
    Search,
    Rules(Arc<Vec<Rule>>),
}

/// A place found, its line, and what the query captured there.
type Found = (PathBuf, lt::Range, String, Vec<(String, String)>);

/// A walk over the project: what it's for, its answer, and how to stop it.
type Running = (Job, Receiver<(Vec<FileHits>, bool, usize)>, Arc<AtomicBool>);

#[derive(Default)]
pub(super) struct TreeTools {
    used: bool,
    query: String,
    lang: Option<Lang>,
    error: Option<String>,
    /// Place, line, and what the query captured there.
    results: Vec<Found>,
    /// What the results came from.
    searched_with: Option<(String, Lang, Arc<tree_rules::CompiledQuery>)>,
    replacement: String,
    capped: bool,
    skipped: usize,
    searched: bool,
    job: Option<Running>,
    rules: Arc<Vec<Rule>>,
    rule_problems: Vec<String>,
    rules_modified: Option<std::time::SystemTime>,
    rules_root: Option<PathBuf>,
    next_rules_check: f64,
    /// Each document's version last looked at, and when one was first seen changed.
    scanned: HashMap<u64, u64>,
    changed_since: Option<f64>,
    /// Open files' text, handed to the next walk.
    open_text: HashMap<PathBuf, String>,
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
        let result = if tree_edit::is_lisp(&doc.extension()) && what != Move::Raise {
            let source = doc.text();
            let heads: Vec<usize> = doc.buffer.selections().iter().map(|s| s.head).collect();
            let results: Vec<_> = heads.iter().map(|head| tree_edit::lisp_move(&source, *head, what)).collect();
            let why = results.iter().find_map(|r| r.as_ref().err().cloned());
            let mut edits: Vec<_> = results.into_iter().flatten().flat_map(|c| c.edits).collect();
            // Several cursors in one list move it once.
            edits.sort_by_key(|(a, b, _)| (*a, *b));
            edits.dedup();
            let mut end = 0;
            edits.retain(|(a, b, _)| {
                let keep = *a >= end;
                if keep {
                    end = *b;
                }
                keep
            });
            match (edits.is_empty(), why) {
                (true, Some(why)) => Err(why),
                _ => Ok(Change { edits, cursor: None }),
            }
        } else if tree_edit::is_lisp(&doc.extension()) {
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
        let open: HashMap<PathBuf, String> = self
            .docs
            .iter()
            .filter(|d| !d.large)
            .filter_map(|d| Some((path_key(d.path.as_ref()?), d.text())))
            .collect();
        self.tree_tools.open_text = open;
        let tools = &mut self.tree_tools;
        tools.error = None;
        let Some(lang) = tools.lang else { return };
        let query = match tree_rules::compile(lang, &tools.query) {
            Ok(query) => query,
            Err(why) => {
                tools.error = Some(why);
                tools.results.clear();
                tools.searched = false;
                tools.searched_with = None;
                return;
            }
        };
        let Some(root) = self.explorer.root.clone() else {
            self.tree_tools.error = Some("Open a folder to search it".to_owned());
            return;
        };
        let query = Arc::new(query);
        self.tree_tools.searched_with = Some((self.tree_tools.query.clone(), lang, query.clone()));
        let searches = vec![Search { lang, query }];
        self.tree_tools_run(ctx, root, searches, Job::Search);
    }

    fn tree_tools_run(&mut self, ctx: &egui::Context, root: PathBuf, searches: Vec<Search>, job: Job) {
        if let Some((_, _, cancel)) = &self.tree_tools.job {
            cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (wake, stop) = (repaint(ctx), cancel.clone());
        let open_text = std::mem::take(&mut self.tree_tools.open_text);
        std::thread::spawn(move || {
            let open = |path: &Path| open_text.get(&path_key(path)).cloned();
            let _ = tx.send(tree_rules::scan_project(&root, &open, &searches, &stop));
            wake();
        });
        self.tree_tools.job = Some((job, rx, cancel));
    }

    fn take_job(&mut self) {
        let tools = &mut self.tree_tools;
        let Some((_, rx, _)) = &tools.job else { return };
        let Ok((files, capped, skipped)) = rx.try_recv() else { return };
        let Some((job, _, _)) = tools.job.take() else { return };
        match job {
            Job::Search => {
                tools.results.clear();
                tools.capped = capped;
                tools.skipped = skipped;
                tools.searched = true;
                for file in files {
                    for (_, hit) in file.hits {
                        tools.results.push((file.path.clone(), range_of(&hit), hit.line_text, hit.captures));
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
                tools.scanned.clear();
                let more = if capped { " (stopped early)" } else { "" };
                let large = if skipped > 0 { format!(", {skipped} too large to read") } else { String::new() };
                self.status = Some(format!("Rules found {count} in {in_files} file(s){more}{large}"));
            }
        }
    }

    /// Replaces what was found, reading each file again for where things are now. The
    /// changes stay unsaved in tabs, so they can be looked over and undone.
    fn tree_replace_all(&mut self, ctx: &egui::Context) {
        let Some((_, lang, query)) = self.tree_tools.searched_with.clone() else { return };
        let template = self.tree_tools.replacement.clone();
        let mut files: Vec<PathBuf> = self.tree_tools.results.iter().map(|r| r.0.clone()).collect();
        files.sort();
        files.dedup();
        let active = self.docs.get(self.active).map(|d| d.uid);
        let (mut count, mut changed) = (0, 0);
        let mut problems = Vec::new();
        for path in files {
            let i = match self.doc_index_by_path(&path) {
                Some(i) => i,
                None => match Document::open(&path) {
                    Ok(doc) => {
                        self.watcher.watch_file(&path);
                        self.add_doc(doc);
                        let last = self.docs.len() - 1;
                        self.docs[last].request_focus = false;
                        last
                    }
                    Err(e) => {
                        problems.push(format!("{}: {e}", path.display()));
                        continue;
                    }
                },
            };
            let doc = &mut self.docs[i];
            if doc.buffer.read_only {
                problems.push(format!("{} is read-only", path.display()));
                continue;
            }
            let source = doc.text();
            let Some(tree) = tree_rules::parse(lang, &source) else { continue };
            let edits = tree_rules::replacements(&query, &tree, &source, &template);
            if edits.is_empty() {
                continue;
            }
            count += edits.len();
            changed += 1;
            let rope = doc.buffer.rope();
            let edits =
                edits.into_iter().map(|(a, b, text)| (rope.byte_to_char(a), rope.byte_to_char(b), text)).collect();
            doc.buffer.apply_edits(edits);
        }
        if let Some(i) = active.and_then(|uid| self.doc_index(uid)) {
            self.active = i;
        }
        let skipped = match problems.first() {
            Some(first) => format!(". Skipped {}: {first}", problems.len()),
            None => String::new(),
        };
        self.status = Some(format!("Replaced {count} in {changed} file(s), not saved yet{skipped}"));
        self.tree_search_start(ctx);
    }

    // ----- rules ---------------------------------------------------------------------

    fn load_rules(&mut self, root: &Path) {
        let path = tree_rules::rules_path(root);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (rules, problems) = tree_rules::parse_rules(&text);
        let tools = &mut self.tree_tools;
        tools.rules = Arc::new(rules);
        tools.found.clear();
        tools.scanned.clear();
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
            let modified =
                root.as_ref().and_then(|r| std::fs::metadata(tree_rules::rules_path(r)).ok()?.modified().ok());
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
        let scanned = &self.tree_tools.scanned;
        let changed: Vec<usize> = (0..self.docs.len())
            .filter(|i| {
                let doc = &self.docs[*i];
                !doc.large && scanned.get(&doc.uid) != Some(&doc.buffer.version())
            })
            .collect();
        if changed.is_empty() {
            return;
        }
        let since = *self.tree_tools.changed_since.get_or_insert(now);
        if now - since < SCAN_REST {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SCAN_REST));
            return;
        }
        self.tree_tools.changed_since = None;
        for i in changed {
            let doc = &self.docs[i];
            self.tree_tools.scanned.insert(doc.uid, doc.buffer.version());
            self.scan_doc(i);
        }
    }

    fn scan_doc(&mut self, i: usize) {
        let Some(doc) = self.docs.get(i) else { return };
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
        self.rules_fix(false);
    }

    /// `Rules: Apply Fix at Cursor`.
    pub(super) fn rules_fix_here(&mut self) {
        self.rules_fix(true);
    }

    fn rules_fix(&mut self, only_here: bool) {
        let rules = self.tree_tools.rules.clone();
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let Some(lang) = Lang::from_extension(&doc.extension()) else { return };
        let Some(tree) = doc.fresh_tree() else { return };
        let source = doc.text();
        let rope = doc.buffer.rope();
        let here = rope.char_to_byte(doc.buffer.primary().head);
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for rule in rules.iter().filter(|r| r.lang == lang) {
            let Some(fix) = &rule.fix else { continue };
            let hits = tree_rules::scan(&rule.query, &tree, &source);
            for hit in hits.into_iter().filter(|h| !only_here || (h.start_byte <= here && here <= h.end_byte)) {
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
        let (mut search, mut from_cursor, mut open, mut replace) = (false, false, None, false);
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
                let large = match tools.skipped {
                    0 => String::new(),
                    n => format!(", {n} files too large to read"),
                };
                ui.label(RichText::new(format!("{} found{more}{large}", tools.results.len())).color(pal.text_dim));
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
        // Replacing goes by the search that was run, so the two can't drift apart.
        let current = tools.searched_with.as_ref().is_some_and(|(query, ..)| *query == tools.query) && !running;
        ui.horizontal(|ui| {
            let edit = TextEdit::singleline(&mut tools.replacement)
                .font(mono.clone())
                .desired_width(ui.available_width() - 110.0)
                .hint_text("Replace with: captures by name, such as $name");
            ui.add(edit);
            let can = current && !tools.results.is_empty();
            let tip = if can { "Changes stay unsaved, so they can be undone" } else { "Search first" };
            replace = ui.add_enabled(can, egui::Button::new("Replace All")).on_hover_text(tip).clicked();
        });
        if let Some(why) = &tools.error {
            ui.label(RichText::new(why).color(warn));
        }
        egui::ScrollArea::vertical().id_salt("tree_search_results").auto_shrink([false, false]).show(ui, |ui| {
            for (at, (path, range, line, captures)) in tools.results.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let mut label = format!("{name}:{}:{}  {line}", range.start.line + 1, range.start.character + 1);
                if current && !tools.replacement.is_empty() {
                    let becomes = tree_rules::fill(&tools.replacement, captures);
                    label.push_str(&format!("   →   {}", becomes.lines().next().unwrap_or_default()));
                }
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
        if replace {
            self.tree_replace_all(&ctx);
        }
        if let Some((path, range, ..)) = open.and_then(|at| self.tree_tools.results.get(at).cloned()) {
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
