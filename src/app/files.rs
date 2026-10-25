//! Documents and files: open/save/close, hot exit, external changes and syntax sync.

use crate::document::Document;
use crate::session::{self, Session};
use crate::syntax::SyntaxWorker;
use crate::watcher::same_path;
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
            for doc in &mut self.docs {
                if doc.path.as_deref().is_some_and(|p| same_path(p, path)) {
                    doc.check_disk();
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
        let active = saved.active;
        for doc in saved.restore_docs() {
            self.untitled_counter = self.untitled_counter.max(doc.untitled_index());
            self.add_doc(doc);
        }
        self.active = active.min(self.docs.len().saturating_sub(1));
    }

    /// Writes the session if it changed. Returns false if writing failed.
    pub(super) fn save_session(&mut self) -> bool {
        let session =
            Session::capture(self.explorer.root.as_deref(), &self.docs, self.active, self.settings.files.hot_exit);
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

    pub(super) fn add_doc(&mut self, mut doc: Document) {
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
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref().is_some_and(|p| same_path(p, path))) {
            self.active = i;
            return;
        }
        match Document::open(path) {
            Ok(doc) => {
                self.watcher.watch_file(path);
                self.add_doc(doc);
            }
            Err(e) => self.status = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    pub(super) fn open_folder(&mut self, path: PathBuf) {
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
        if self.active > index || self.active >= self.docs.len() {
            self.active = self.active.saturating_sub(1);
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
