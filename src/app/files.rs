//! Documents and files: open/save/close, hot exit, external changes and syntax sync.

use crate::buffer::Indent;
use crate::document::Document;
use crate::explorer::ExplorerAction;
use crate::palette::InputKind;
use crate::session::{self, Session};
use crate::syntax::SyntaxWorker;
use crate::theme;
use crate::watcher::same_path;
use rust_code_core::recent;
use rust_code_core::{editorconfig, find};
use std::path::{Path, PathBuf};

use super::{BACKUP_INTERVAL, CodeApp, PendingClose, SidebarView, repaint};

impl CodeApp {
    /// Applies changes other programs made to files: reload clean tabs, flag
    /// conflicts and deletions, refresh the explorer and Quick Open index.
    pub(super) fn pump_watcher(&mut self, ctx: &egui::Context) {
        let changes = self.watcher.drain();
        if changes.git || changes.structure || !changes.paths.is_empty() {
            self.git_refresh_soon(ctx, 0.3);
        }
        if changes.structure {
            self.explorer.refresh();
            self.finder.mark_stale();
        }
        for path in &changes.paths {
            if self.is_config_file(path) {
                self.settings_reload = true;
            }
            for i in 0..self.docs.len() {
                let doc = &mut self.docs[i];
                if doc.path.as_deref().is_some_and(|p| same_path(p, path)) {
                    let version = doc.buffer.version();
                    doc.check_disk();
                    // Reloaded with another program's changes: keep that version too.
                    if doc.buffer.version() != version && !doc.is_dirty() {
                        self.history_record(i, rust_code_core::history::Source::External);
                    }
                }
            }
        }
    }

    /// Collects finished parses and sends the active document's latest text.
    pub(super) fn pump_syntax(&mut self, ctx: &egui::Context) {
        let worker = self.syntax.get_or_insert_with(|| SyntaxWorker::spawn(repaint(ctx)));
        while let Some(parsed) = worker.try_recv() {
            if let Some(doc) = self.docs.iter_mut().find(|d| d.uid == parsed.uid)
                && let Some(ts) = &mut doc.ts
            {
                ts.accept(parsed);
            }
        }
        if let Some(doc) = self.docs.get_mut(self.active)
            && let Some(ts) = &mut doc.ts
        {
            ts.sync(doc.uid, &mut doc.buffer, worker);
        }
    }

    pub(super) fn restore_session(&mut self, saved: Session) {
        if let Some(root) = saved.root.clone().filter(|r| r.is_dir()) {
            self.open_folder(root);
        }
        self.log_to_open = saved.log_view.clone().filter(|p| p.is_file());
        self.restore_log_views(saved.log_views.iter().filter(|p| p.is_file()).cloned().collect());
        self.at_cursor_restore(&saved.cursor_pins, saved.cursor_info_beside);
        let active = saved.active;
        for doc in saved.restore_docs() {
            self.untitled_counter = self.untitled_counter.max(doc.untitled_index());
            self.add_doc(doc);
        }
        self.active = active.min(self.docs.len().saturating_sub(1));
    }

    /// Writes the session if it changed. Returns false if writing failed.
    pub(super) fn save_session(&mut self) -> bool {
        let mut session =
            Session::capture(self.explorer.root.as_deref(), &self.docs, self.active, self.settings.files.hot_exit);
        session.window = self.window;
        session.run_pins = self.cargo.run_pins.clone();
        session.log_view = self.log_view_path();
        session.log_views =
            self.log_view_paths().into_iter().filter(|p| Some(p) != session.log_view.as_ref()).collect();
        (session.cursor_pins, session.cursor_info_beside) = self.at_cursor_saved();
        let json = session::to_json(&session);
        if json == self.last_session_json {
            return true;
        }
        match session::write(&json) {
            Ok(()) => {
                self.last_session_json = json;
                true
            }
            Err(e) => {
                self.status = Some(format!("Could not back up session: {e}"));
                false
            }
        }
    }

    pub(super) fn periodic_backup(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.last_backup_time >= BACKUP_INTERVAL {
            self.last_backup_time = now;
            self.save_session();
        }
        // Keep ticking while there is unsaved work, so idle edits still get backed up.
        if self.docs.iter().any(Document::is_dirty) {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(BACKUP_INTERVAL));
        }
    }

    /// Applies indentation and save rules to a document: `.editorconfig` first,
    /// then how the file is already indented, then the settings.
    pub(super) fn apply_file_rules(&self, doc: &mut Document) {
        let editor = &self.settings.editor;
        let config = doc.path.as_deref().map(editorconfig::lookup).unwrap_or_default();
        let detected =
            if editor.detect_indentation && !doc.large { editorconfig::detect_indent(&doc.text()) } else { None };
        let use_tabs = config.use_tabs.or(detected.map(|d| d.0)).unwrap_or(!editor.insert_spaces);
        let size = config.indent_size.or(detected.map(|d| d.1)).unwrap_or(editor.tab_size.max(1));
        doc.buffer.indent = Indent { use_tabs, size };
        doc.buffer.auto_close = editor.auto_close_brackets;
        // `'` starts Rust lifetimes and is an apostrophe in prose.
        doc.buffer.pair_single_quote = !matches!(doc.extension().as_str(), "rs" | "md" | "markdown" | "txt");
        doc.trim_trailing_whitespace = config.trim_trailing_whitespace.unwrap_or(editor.trim_trailing_whitespace);
        doc.insert_final_newline = config.insert_final_newline.unwrap_or(editor.insert_final_newline);
        // Picked from the status bar: stays until the user asks to detect again.
        if let Some(indent) = self.indent_chosen.get(&doc.uid) {
            doc.buffer.indent = *indent;
        }
    }

    pub(super) fn add_doc(&mut self, mut doc: Document) {
        self.apply_file_rules(&mut doc);
        doc.buffer.read_only = doc.path.as_deref().is_some_and(rust_code_core::lsp::convert::is_virtual_path);
        // Focus the editor when the tab is first shown, so typing goes straight into it.
        doc.request_focus = true;
        doc.uid = self.next_uid;
        self.next_uid += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }

    pub(super) fn active_doc(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    pub(super) fn new_file(&mut self) {
        self.untitled_counter += 1;
        let mut doc = Document::untitled(self.untitled_counter);
        doc.buffer.line_ending = self.settings.files.new_file_eol();
        if let Some((enc, bom)) = rust_code_core::encoding::from_setting(&self.settings.files.encoding) {
            (doc.encoding, doc.bom) = (enc, bom);
        }
        self.add_doc(doc);
    }

    pub(super) fn open_file(&mut self, path: &Path) {
        self.hide_view_tabs();
        if self.open_image_file(path) {
            return;
        }
        // Text only in name: it would come out garbled, and could be saved that way.
        if !self.docs.iter().any(|d| d.path.as_deref().is_some_and(|p| same_path(p, path)))
            && self.open_binary_file(path)
        {
            return;
        }
        // Too large for the editor to hold; reading it takes the view's own thread.
        let (log_mb, any_mb) = (self.settings.files.log_view_log_mb, self.settings.files.log_view_any_mb);
        const MIB: u64 = 1024 * 1024;
        if !self.logs.editor_anyway.contains(path)
            && rust_code_core::logfile::belongs_in_log_view_from(path, log_mb * MIB, any_mb * MIB)
        {
            self.log_to_open = Some(path.to_path_buf());
            return;
        }
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref().is_some_and(|p| same_path(p, path))) {
            self.active = i;
            return;
        }
        match Document::open(path) {
            Ok(doc) => {
                self.watcher.watch_file(path);
                self.add_doc(doc);
                self.plugins_opened(path);
            }
            Err(e) => self.status = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    /// Replace All in the Search view. Open files change in the editor, with
    /// undo, and stay unsaved; others change on disk (local history sees that).
    pub(super) fn replace_in_project(&mut self, now: f64) {
        let Some(job) = self.search.project_replace() else { return };
        let (mut count, mut files, mut failed) = (0, 0, Vec::new());
        for path in &job.files {
            let done = if let Some(doc) =
                self.docs.iter_mut().find(|d| d.path.as_deref().is_some_and(|p| same_path(p, path)))
            {
                doc.replace_all(&job.options, &job.with)
            } else {
                // Only UTF-8 text: anything else would be damaged by rewriting it.
                std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|text| {
                    let (new, n) = find::replace_in_text(&text, &job.options, &job.with)?;
                    if n > 0 {
                        std::fs::write(path, new).map_err(|e| e.to_string())?;
                    }
                    Ok(n)
                })
            };
            match done {
                Ok(n) => {
                    count += n;
                    files += usize::from(n > 0);
                }
                Err(_) => failed.push(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            }
        }
        self.status = Some(if failed.is_empty() {
            format!("Replaced {count} occurrences in {files} files")
        } else {
            format!("Replaced {count} occurrences in {files} files; couldn't change: {}", failed.join(", "))
        });
        self.search.search_again(now);
    }

    /// Starts a context menu or header action of the Explorer.
    pub(super) fn start_explorer_action(&mut self, action: ExplorerAction) {
        let root = self.explorer.root.clone();
        match action {
            ExplorerAction::NewFile(dir) => {
                self.path_target = dir.or(root);
                self.palette.show_input(InputKind::NewFile, "", "New file name, then Enter (a/b.rs makes folders too)");
            }
            ExplorerAction::NewFolder(dir) => {
                self.path_target = dir.or(root);
                self.palette.show_input(InputKind::NewFolder, "", "New folder name, then Enter");
            }
            ExplorerAction::Rename(path) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.path_target = Some(path);
                self.palette.show_input(InputKind::RenamePath, &name, "New name, then Enter");
            }
            ExplorerAction::Delete(path) => self.delete_pending = Some(path),
        }
    }

    pub(super) fn finish_new_path(&mut self, name: &str, folder: bool) {
        let Some(dir) = self.path_target.take() else { return };
        let Some(path) = child_path(&dir, name) else {
            self.status = Some(format!("\"{name}\" isn't a name inside this folder"));
            return;
        };
        if path.exists() {
            self.status = Some(format!("{name} already exists"));
            return;
        }
        let made = if folder {
            std::fs::create_dir_all(&path)
        } else {
            path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, ""))
        };
        match made {
            Ok(()) => {
                self.explorer.refresh();
                self.finder.mark_stale();
                if !folder {
                    self.open_file(&path);
                }
            }
            Err(e) => self.status = Some(format!("Could not create {name}: {e}")),
        }
    }

    pub(super) fn finish_rename_path(&mut self, name: &str) {
        let Some(old) = self.path_target.take() else { return };
        let Some(dir) = old.parent() else { return };
        // A plain name: moving elsewhere isn't what Rename says.
        let new = dir.join(name);
        if child_path(dir, name).is_none() || name.contains(['/', '\\']) {
            self.status = Some(format!("\"{name}\" isn't a valid name"));
            return;
        }
        if new == old {
            return;
        }
        if new.exists() && !same_path(&new, &old) {
            self.status = Some(format!("{name} already exists"));
            return;
        }
        match std::fs::rename(&old, &new) {
            Ok(()) => {
                // Open files follow, including those inside a renamed folder.
                for i in 0..self.docs.len() {
                    let moved =
                        self.docs[i].path.as_ref().and_then(|p| p.strip_prefix(&old).ok()).map(|rest| new.join(rest));
                    if let Some(path) = moved {
                        self.watcher.watch_file(&path);
                        self.docs[i].path = Some(path);
                    }
                }
                self.explorer.refresh();
                self.finder.mark_stale();
            }
            Err(e) => self.status = Some(format!("Could not rename: {e}")),
        }
    }

    /// Asks before moving a file or folder to the Recycle Bin.
    pub(super) fn delete_prompt(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.delete_pending.clone() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let unsaved = self.docs.iter().any(|d| d.is_dirty() && d.path.as_ref().is_some_and(|p| p.starts_with(&path)));
        let mut choice = None;
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
            let note = if unsaved { " It has unsaved changes." } else { "" };
            ui.label(format!("Move \"{name}\" to the Recycle Bin?{note}"));
            ui.horizontal(|ui| {
                if ui.button("Move to Recycle Bin").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.delete_pending = None;
                match trash::delete(&path) {
                    Ok(()) => {
                        self.explorer.refresh();
                        self.finder.mark_stale();
                    }
                    Err(e) => self.status = Some(format!("Could not delete {name}: {e}")),
                }
            }
            Some(false) => self.delete_pending = None,
            None => {}
        }
    }

    pub(super) fn open_folder(&mut self, path: PathBuf) {
        recent::add(&mut self.recent_folders, &path);
        if let Err(e) = recent::save(&self.recent_folders) {
            self.status = Some(format!("Could not save recent folders: {e}"));
        }
        self.finder.set_root(Some(path.clone()));
        self.watcher.set_root(Some(&path));
        // The workspace settings layer depends on the folder.
        self.settings_reload = true;
        self.terminal.set_cwd(&path);
        self.explorer.set_root(path);
        self.finder.mark_stale();
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Explorer;
    }

    pub(super) fn save_doc(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get_mut(index) else { return false };
        match doc.save() {
            Ok(true) => {
                let (title, path) = (doc.title(), doc.path.clone());
                self.status = Some(format!("Saved {title}"));
                if path.is_some_and(|p| self.is_config_file(&p)) {
                    self.settings_reload = true;
                }
                self.lsp_did_save(index);
                if !self.auto_save.skip_history {
                    self.history_record(index, rust_code_core::history::Source::Save);
                }
                self.plugins_saved(index);
                true
            }
            Ok(false) => self.save_doc_as(index),
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    pub(super) fn save_doc_as(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get_mut(index) else { return false };
        let mut dialog = rfd::FileDialog::new().set_file_name(doc.title());
        if let Some(dir) = doc.path.as_ref().and_then(|p| p.parent()).or(self.explorer.root.as_deref()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return false };
        match doc.save_as(path) {
            Ok(()) => {
                self.status = Some(format!("Saved {}", doc.title()));
                self.explorer.refresh();
                self.finder.mark_stale();
                self.lsp_did_save(index);
                self.history_record(index, rust_code_core::history::Source::Save);
                self.plugins_saved(index);
                true
            }
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    pub(super) fn request_close_tab(&mut self, index: usize) {
        if self.docs.get(index).is_some_and(Document::is_dirty) {
            self.pending_close = Some(PendingClose::Tab(index));
        } else {
            self.close_tab(index);
        }
    }

    pub(super) fn close_tab(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        let doc = self.docs.remove(index);
        self.views.remove(&doc.uid);
        self.hidden_forget(doc.uid);
        self.indent_chosen.remove(&doc.uid);
        self.lsp_did_close(&doc);
        if let Some(worker) = &self.syntax
            && doc.ts.is_some()
        {
            worker.close(doc.uid);
        }
        if let Some(path) = doc.path {
            self.closed_editors.retain(|p| p != &path);
            self.closed_editors.push(path);
        }
        let closed_active = index == self.active;
        if self.active > index || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
        }
        if closed_active {
            self.close_split_if_doubled();
            // Keep typing in the tab that took its place.
            if let Some(doc) = self.docs.get_mut(self.active) {
                doc.request_focus = true;
            }
        }
    }

    /// Hot exit: on close, unsaved buffers go into the session backup instead of a
    /// "save changes?" prompt. The prompt is only a fallback if the backup fails.
    pub(super) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        let saved = self.save_session();
        if (!saved || !self.settings.files.hot_exit) && self.docs.iter().any(Document::is_dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_close = Some(PendingClose::App);
        }
    }
}

/// `name` under `dir`, refusing anything that climbs out of it.
fn child_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let name = Path::new(name.trim());
    let plain = name.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    (plain && !name.as_os_str().is_empty()).then(|| dir.join(name))
}
