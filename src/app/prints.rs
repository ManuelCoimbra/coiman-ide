//! Print debugging: what `dbg!` printed in the terminals and test output, shown
//! in a panel and next to the calls, and a way to take the calls out again.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use rust_code_core::dbgout::{self, Entry, Parser};
use rust_code_core::document::Document;
use rust_code_core::finder::walker;
use rust_code_core::watcher::same_path;

use super::CodeApp;
use crate::buffer::Selection;
use crate::editor_view::Decorations;
use crate::theme;

/// Seconds between reading the output again.
const SCAN_EVERY: f64 = 0.4;
/// Rows the panel lists at most, newest kept.
const SHOWN: usize = 1000;
/// Characters of a value shown beside the code.
const INLINE_CHARS: usize = 80;

struct Hit {
    entry: Entry,
    path: Option<PathBuf>,
}

/// A planned removal across the workspace, waiting for confirmation.
struct Planned {
    files: Vec<PathBuf>,
    calls: usize,
    skipped: usize,
}

#[derive(Default)]
pub(super) struct Prints {
    hits: Vec<Hit>,
    /// The latest value and how many were printed, by file and line (from 0).
    sites: HashMap<PathBuf, BTreeMap<usize, (String, usize)>>,
    /// Where each printed file name was found, for this folder.
    resolved: HashMap<String, Option<PathBuf>>,
    resolved_for: Option<PathBuf>,
    next_scan: f64,
    filter: String,
    planned: Option<Planned>,
}

/// A printed file name is relative to the workspace, which may be above the
/// package: tries each folder upward from the package.
pub(super) fn resolve(file: &str, bases: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(file);
    if path.is_absolute() {
        return path.exists().then(|| path.to_path_buf());
    }
    bases.iter().flat_map(|base| base.ancestors()).map(|dir| dir.join(path)).find(|p| p.exists())
}

impl CodeApp {
    /// Reads the output again now and then. It stays quiet with nothing new:
    /// a frame too early asks for just one more.
    pub(super) fn pump_prints(&mut self, ctx: &egui::Context) {
        // Nothing to read: no reason to look again.
        if self.terminal.is_empty() && self.job_output().is_empty() {
            self.prints.hits.clear();
            self.prints.sites.clear();
            return;
        }
        let now = ctx.input(|i| i.time);
        if now < self.prints.next_scan {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(self.prints.next_scan - now));
            return;
        }
        self.prints.next_scan = now + SCAN_EVERY;

        let mut entries = self.terminal.dbg_entries();
        let mut parser = Parser::default();
        for line in self.job_output() {
            parser.feed(line);
        }
        entries.extend(parser.finish());

        let root = self.explorer.root.clone();
        if self.prints.resolved_for != root {
            self.prints.resolved.clear();
            self.prints.resolved_for = root.clone();
        }
        let bases: Vec<PathBuf> = self.cargo_root_dir().into_iter().chain(root).collect();
        let mut sites: HashMap<PathBuf, BTreeMap<usize, (String, usize)>> = HashMap::new();
        let mut hits = Vec::with_capacity(entries.len());
        for entry in entries {
            let path = match self.prints.resolved.get(&entry.file) {
                Some(path) => path.clone(),
                None => {
                    let path = resolve(&entry.file, &bases);
                    self.prints.resolved.insert(entry.file.clone(), path.clone());
                    path
                }
            };
            if let Some(path) = &path {
                let site = sites.entry(path.clone()).or_default().entry(entry.line.saturating_sub(1)).or_default();
                site.0 = entry.short_value(INLINE_CHARS);
                site.1 += 1;
            }
            hits.push(Hit { entry, path });
        }
        self.prints.hits = hits;
        self.prints.sites = sites;
    }

    pub(super) fn prints_tab_label(&self) -> String {
        match self.prints.hits.len() {
            0 => "DBG".to_owned(),
            n => format!("DBG ({n})"),
        }
    }

    /// The printed values beside their calls. A line that no longer holds a
    /// call has moved since the run, and its value would mislead.
    pub(super) fn prints_decorate(&self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i) else { return };
        let Some(path) = &doc.path else { return };
        let Some((_, lines)) = self.prints.sites.iter().find(|(p, _)| same_path(p, path)) else { return };
        for (&line, (value, count)) in lines {
            if line < doc.buffer.len_lines() && doc.buffer.line_text(line).contains("dbg!") {
                let times = if *count > 1 { format!("  ×{count}") } else { String::new() };
                deco.values.push((line, format!("= {value}{times}")));
            }
        }
    }

    fn open_print(&mut self, path: &Path, line: usize, column: usize) {
        self.open_file(path);
        if let Some(doc) = self.docs.get_mut(self.active) {
            let line = line.saturating_sub(1).min(doc.buffer.len_lines() - 1);
            let at = doc.buffer.line_start(line) + column.saturating_sub(1);
            doc.buffer.set_single(Selection::cursor(at.min(doc.buffer.line_end(line))));
            doc.reveal = true;
            doc.request_focus = true;
        }
    }

    /// Takes the calls out of the active file, as one undo step.
    pub(super) fn remove_dbg_here(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else {
            self.status = Some("Open a file first".to_owned());
            return;
        };
        let removal = dbgout::remove(&doc.text());
        let calls = removal.edits.len();
        doc.buffer.apply_edits(removal.edits);
        self.status = Some(removal_message(calls, 1, removal.skipped));
    }

    /// Finds the calls in the workspace and asks before taking them out.
    pub(super) fn plan_remove_dbg(&mut self) {
        let Some(root) = self.cargo_root_dir().or_else(|| self.explorer.root.clone()) else {
            self.status = Some("Open a folder first".to_owned());
            return;
        };
        let mut planned = Planned { files: Vec::new(), calls: 0, skipped: 0 };
        for entry in walker(&root).build().flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) || entry.path().extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let path = entry.into_path();
            // An open file may hold unsaved edits.
            let text = match self.doc_index_by_path(&path) {
                Some(i) => self.docs[i].text(),
                None => std::fs::read_to_string(&path).unwrap_or_default(),
            };
            let removal = dbgout::remove(&text);
            planned.skipped += removal.skipped;
            if !removal.edits.is_empty() {
                planned.calls += removal.edits.len();
                planned.files.push(path);
            }
        }
        if planned.calls == 0 {
            self.status = Some(if planned.skipped == 0 {
                "No dbg! calls in this workspace".to_owned()
            } else {
                format!("Left {} dbg! call(s) that hold comments or aren't complete", planned.skipped)
            });
            return;
        }
        self.prints.planned = Some(planned);
    }

    /// Files that aren't open open in tabs and stay unsaved, so every change
    /// can be reviewed and undone, like a rename.
    fn remove_dbg_planned(&mut self, planned: Planned) {
        let active_uid = self.docs.get(self.active).map(|d| d.uid);
        let (mut calls, mut files) = (0, 0);
        for path in &planned.files {
            let i = match self.doc_index_by_path(path) {
                Some(i) => i,
                None => {
                    let doc = match Document::open(path) {
                        Ok(doc) => doc,
                        Err(e) => {
                            self.status = Some(format!("{}: {e}", path.display()));
                            continue;
                        }
                    };
                    self.watcher.watch_file(path);
                    self.add_doc(doc);
                    let last = self.docs.len() - 1;
                    self.docs[last].request_focus = false;
                    last
                }
            };
            let removal = dbgout::remove(&self.docs[i].text());
            calls += removal.edits.len();
            files += 1;
            self.docs[i].buffer.apply_edits(removal.edits);
        }
        if let Some(i) = active_uid.and_then(|uid| self.docs.iter().position(|d| d.uid == uid)) {
            self.active = i;
        }
        self.status = Some(format!("{}. Review and save.", removal_message(calls, files, planned.skipped)));
    }

    /// The DBG panel: every value printed, oldest first.
    pub(super) fn prints_panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let mut open = None;
        let (mut here, mut everywhere) = (false, false);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.prints.filter).hint_text("Filter").desired_width(180.0));
            if ui.small_button("Remove from This File").clicked() {
                here = true;
            }
            if ui.small_button("Remove from Workspace").clicked() {
                everywhere = true;
            }
        });
        let filter = self.prints.filter.to_lowercase();
        let matches = |h: &&Hit| {
            filter.is_empty()
                || h.entry.expr.to_lowercase().contains(&filter)
                || h.entry.value.to_lowercase().contains(&filter)
                || h.entry.file.to_lowercase().contains(&filter)
        };
        let shown: Vec<&Hit> = self.prints.hits.iter().filter(matches).collect();
        let font = egui::FontId::monospace(self.font_size * 0.9);
        if shown.is_empty() {
            let text = if self.prints.hits.is_empty() {
                "Values printed by dbg! in the terminal or test output show up here."
            } else {
                "Nothing matches the filter."
            };
            ui.label(RichText::new(text).color(pal.text_dim));
        }
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            let skip = shown.len().saturating_sub(SHOWN);
            if skip > 0 {
                ui.label(RichText::new(format!("{skip} older values not shown")).small().color(pal.text_dim));
            }
            for (n, hit) in shown.iter().enumerate().skip(skip) {
                let entry = &hit.entry;
                ui.horizontal(|ui| {
                    let place = RichText::new(format!("{}:{}", entry.file, entry.line)).font(font.clone());
                    match &hit.path {
                        Some(path) => {
                            if ui.link(place).on_hover_text("Go to this dbg! call").clicked() {
                                open = Some((path.clone(), entry.line, entry.column));
                            }
                        }
                        None => {
                            ui.label(place.color(pal.text_dim)).on_hover_text("This file isn't in the open folder");
                        }
                    }
                    if !entry.expr.is_empty() {
                        let expr = format!("{} =", entry.expr);
                        ui.label(RichText::new(expr).font(font.clone()).color(pal.text_dim));
                    }
                    if entry.value.contains('\n') {
                        let title = RichText::new(entry.short_value(INLINE_CHARS)).font(font.clone());
                        egui::CollapsingHeader::new(title).id_salt(("dbg", n)).show(ui, |ui| {
                            ui.label(RichText::new(&entry.value).font(font.clone()));
                        });
                    } else {
                        ui.label(RichText::new(&entry.value).font(font.clone()));
                    }
                });
            }
        });
        if let Some((path, line, column)) = open {
            self.open_print(&path, line, column);
        }
        if here {
            self.remove_dbg_here();
        }
        if everywhere {
            self.plan_remove_dbg();
        }
    }

    pub(super) fn prints_window(&mut self, ctx: &egui::Context) {
        let Some(planned) = &self.prints.planned else { return };
        let mut answer = None;
        egui::Window::new("Remove dbg!")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("Remove {} dbg! call(s) from {} file(s)?", planned.calls, planned.files.len()));
                ui.label(
                    RichText::new("They open in tabs, unsaved, so you can review and undo.")
                        .color(theme::colors().text_dim),
                );
                if planned.skipped > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} call(s) that hold comments or aren't complete stay.",
                            planned.skipped
                        ))
                        .color(theme::colors().text_dim),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if let Some(yes) = answer
            && let Some(planned) = self.prints.planned.take()
            && yes
        {
            self.remove_dbg_planned(planned);
        }
    }
}

fn removal_message(calls: usize, files: usize, skipped: usize) -> String {
    let left =
        if skipped > 0 { format!(", left {skipped} that hold comments or aren't complete") } else { String::new() };
    format!("Removed {calls} dbg! call(s) from {files} file(s){left}")
}
