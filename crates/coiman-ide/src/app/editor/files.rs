//! Documents and files: open/save/close, hot exit, external changes and syntax sync.

use crate::buffer::Indent;
use crate::document::Document;
use crate::session::{self, Session};
use crate::syntax::SyntaxWorker;
use crate::ui::explorer::ExplorerAction;
use crate::ui::palette::InputKind;
use crate::ui::theme;
use crate::watcher::same_path;
use coiman_core::{editor::editorconfig, editor::find};
use coiman_core::{project::project_tree, project::recent};
use std::path::{Path, PathBuf};

use crate::app::{BACKUP_INTERVAL, CodeApp, PendingClose, SidebarView, repaint};

impl CodeApp {
    /// Takes in a deletion that finished in the background: the tree, then the status.
    pub(crate) fn pump_deleting(&mut self) {
        let Some(rx) = &self.deleting else { return };
        let Ok((gone, message)) = rx.try_recv() else { return };
        self.deleting = None;
        self.explorer.forget(&gone);
        self.explorer.refresh();
        self.finder.mark_stale();
        self.status = Some(message);
    }

    /// Applies changes other programs made to files: reload clean tabs, flag
    /// conflicts and deletions, refresh the explorer and Quick Open index.
    pub(crate) fn pump_watcher(&mut self, ctx: &egui::Context) {
        self.pump_deleting();
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
                        self.history_record(i, coiman_core::project::history::Source::External);
                    }
                }
            }
        }
    }

    /// Collects finished parses and sends the active document's latest text.
    pub(crate) fn pump_syntax(&mut self, ctx: &egui::Context) {
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

    pub(crate) fn restore_session(&mut self, saved: Session) {
        if let Some(root) = saved.root.clone().filter(|r| r.is_dir()) {
            self.open_folder(root);
        }
        self.log_to_open = saved.log_view.clone().filter(|p| p.is_file());
        self.restore_log_views(saved.log_views.iter().filter(|p| p.is_file()).cloned().collect());
        self.at_cursor_restore(&saved.cursor_pins, saved.cursor_info_beside);
        self.rails.restore(&saved.right_tools, saved.panel_right);
        // Opening the folder shows the side bar; a session that had it hidden keeps it so.
        if saved.sidebar_hidden {
            self.sidebar_visible = false;
        }
        self.layout.saved = saved.layout.clone();
        self.sheet_pinned = saved.layout.side_pinned;
        let active = saved.active;
        for doc in saved.restore_docs() {
            self.untitled_counter = self.untitled_counter.max(doc.untitled_index());
            self.add_doc(doc);
        }
        self.active = active.min(self.docs.len().saturating_sub(1));
    }

    /// Writes the session if it changed. Returns false if writing failed.
    pub(crate) fn save_session(&mut self) -> bool {
        let mut session =
            Session::capture(self.explorer.root.as_deref(), &self.docs, self.active, self.settings.files.hot_exit);
        session.window = self.window;
        session.run_pins = self.cargo.run_pins.clone();
        session.log_view = self.log_view_path();
        session.log_views =
            self.log_view_paths().into_iter().filter(|p| Some(p) != session.log_view.as_ref()).collect();
        (session.cursor_pins, session.cursor_info_beside) = self.at_cursor_saved();
        session.right_tools = self.rails.saved();
        session.panel_right = self.rails.panel_right;
        session.sidebar_hidden = !self.sidebar_visible;
        session.layout = self.layout.saved.clone();
        session.layout.side_pinned = self.sheet_pinned;
        session.breakpoints = self.breakpoints_by_project();
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

    pub(crate) fn periodic_backup(&mut self, ctx: &egui::Context) {
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
    pub(crate) fn apply_file_rules(&self, doc: &mut Document) {
        let editor = &self.settings.editor;
        let config = doc.path.as_deref().map(editorconfig::lookup).unwrap_or_default();
        let detected =
            if editor.detect_indentation && !doc.large { editorconfig::detect_indent(&doc.text()) } else { None };
        let use_tabs = config.use_tabs.or(detected.map(|d| d.0)).unwrap_or(!editor.insert_spaces);
        let size = config.indent_size.or(detected.map(|d| d.1)).unwrap_or(editor.tab_size.max(1));
        doc.buffer.indent = Indent { use_tabs, size };
        doc.buffer.auto_close = editor.auto_close_brackets;
        doc.buffer.indent_rules = coiman_core::editor::indent_rules::IndentRules::for_extension(&doc.extension());
        let tags = coiman_core::editor::auto_tags::TagMode::for_extension(&doc.extension());
        doc.buffer.auto_tags = if editor.auto_close_tags { tags } else { coiman_core::editor::auto_tags::TagMode::Off };
        doc.buffer.auto_quotes = editor.auto_create_quotes;
        // Languages whose doc comments are `/** ... */` with `@param` and `@return`.
        let javadoc_like = matches!(
            doc.extension().as_str(),
            "c" | "h"
                | "cpp"
                | "hpp"
                | "cc"
                | "cxx"
                | "hh"
                | "hxx"
                | "java"
                | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "mjs"
                | "cs"
                | "php"
                | "kt"
                | "swift"
                | "scala"
                | "groovy"
                | "dart"
        );
        doc.buffer.doc_comments = editor.doc_comment_templates && javadoc_like;
        // `'` starts Rust lifetimes and is an apostrophe in prose.
        doc.buffer.pair_single_quote = !matches!(doc.extension().as_str(), "rs" | "md" | "markdown" | "txt");
        doc.trim_trailing_whitespace = config.trim_trailing_whitespace.unwrap_or(editor.trim_trailing_whitespace);
        doc.insert_final_newline = config.insert_final_newline.unwrap_or(editor.insert_final_newline);
        // Picked from the status bar: stays until the user asks to detect again.
        if let Some(indent) = self.indent_chosen.get(&doc.uid) {
            doc.buffer.indent = *indent;
        }
    }

    pub(crate) fn add_doc(&mut self, mut doc: Document) {
        self.apply_file_rules(&mut doc);
        doc.buffer.read_only = doc.path.as_deref().is_some_and(coiman_core::language::lsp::convert::is_virtual_path);
        // Focus the editor when the tab is first shown, so typing goes straight into it.
        doc.request_focus = true;
        doc.uid = self.next_uid;
        self.next_uid += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }

    pub(crate) fn active_doc(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    pub(crate) fn new_file(&mut self) {
        self.untitled_counter += 1;
        let mut doc = Document::untitled(self.untitled_counter);
        doc.buffer.line_ending = self.settings.files.new_file_eol();
        if let Some((enc, bom)) = coiman_core::editor::encoding::from_setting(&self.settings.files.encoding) {
            (doc.encoding, doc.bom) = (enc, bom);
        }
        self.add_doc(doc);
    }

    pub(crate) fn open_file(&mut self, path: &Path) {
        coiman_core::applog::info(&format!("open file {}", path.display()));
        if path.is_file() {
            self.recent.add_file(path, recent::now());
            self.remember_recent();
        }
        self.hide_view_tabs();
        // A SQLite database opens in the Database window, not as bytes.
        if coiman_core::data::database::is_sqlite_file(path) {
            self.open_database(path);
            return;
        }
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
            && coiman_core::data::logfile::belongs_in_log_view_from(path, log_mb * MIB, any_mb * MIB)
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
    pub(crate) fn replace_in_project(&mut self, now: f64) {
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
    pub(crate) fn start_explorer_action(&mut self, action: ExplorerAction) {
        let root = self.explorer.root.clone();
        match action {
            ExplorerAction::NewFile(dir) => {
                self.path_target = dir.or(root);
                self.palette.show_input(InputKind::NewFile, "", "New file name, then Enter (a/b.rs makes folders too)");
            }
            ExplorerAction::NewFromTemplate(dir, template) => {
                self.path_target = dir.or(root);
                self.path_template = Some(template);
                if let Some(name) = project_tree::TEMPLATES[template].fixed_name() {
                    self.finish_new_path(name, false);
                    return;
                }
                self.palette.show_input(InputKind::NewFile, "", "New file's name, then Enter (its extension is added)");
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
            ExplorerAction::Delete(paths, permanent) => self.delete_pending = Some((paths, permanent)),
            ExplorerAction::Paste(dir) => self.paste_into(&dir),
            ExplorerAction::Move(paths, dir) => self.move_into(&paths, &dir),
            ExplorerAction::OpenInFileManager(dir) => self.open_file_manager(Some(dir)),
        }
    }

    pub(crate) fn finish_new_path(&mut self, name: &str, folder: bool) {
        let Some(dir) = self.path_target.take() else { return };
        let template = self.path_template.take().map(|t| &project_tree::TEMPLATES[t]);
        let typed =
            template.filter(|t| t.fixed_name().is_none()).map_or_else(|| name.to_owned(), |t| t.file_name(name));
        let Some(path) = child_path(&dir, &typed) else {
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
            let text = template.map_or_else(String::new, |t| {
                let root = self.explorer.root.as_deref();
                let folder = path.parent().unwrap_or(&dir);
                let inside = root.and_then(|r| folder.strip_prefix(r).ok()).unwrap_or(folder);
                let file_name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                t.text(t.stem_of(&file_name), &inside.to_string_lossy())
            });
            path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, text))
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

    pub(crate) fn finish_rename_path(&mut self, name: &str) {
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
                    let moved = self.docs[i]
                        .path
                        .as_ref()
                        .and_then(|p| coiman_core::project::file_ops::moved_path(p, &old, &new));
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

    /// Asks before moving files and folders to the Recycle Bin, or deleting them for good
    /// (Shift+Delete). Enter confirms and Escape cancels, as in VS Code's dialog.
    pub(crate) fn delete_prompt(&mut self, ui: &mut egui::Ui) {
        let Some((paths, permanent)) = self.delete_pending.clone() else { return };
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.delete_pending = None;
            return;
        }
        let name_of = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let what = match paths.as_slice() {
            [one] => format!("\"{}\"", name_of(one)),
            many => format!("these {} items", many.len()),
        };
        let unsaved = self
            .docs
            .iter()
            .any(|d| d.is_dirty() && d.path.as_ref().is_some_and(|p| paths.iter().any(|gone| p.starts_with(gone))));
        let mut choice = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)).then_some(true);
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
            let note = if unsaved { " Their unsaved changes are saved first." } else { "" };
            if permanent {
                ui.label(format!("Delete {what} for good? This can't be undone.{note}"));
            } else {
                ui.label(format!("Move {what} to the Recycle Bin?{note}"));
            }
            ui.horizontal(|ui| {
                let yes = if permanent { "Delete" } else { "Move to Recycle Bin" };
                if ui.button(yes).clicked() {
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
                self.start_delete(ui.ctx(), paths, permanent);
            }
            Some(false) => self.delete_pending = None,
            None => {}
        }
    }

    /// Moves `paths` to the Recycle Bin (or deletes them for good) on a thread, saving unsaved
    /// work in them first. The Explorer's prompt and the File Manager both end here.
    pub(crate) fn start_delete(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, permanent: bool) {
        // Unsaved work goes to disk first, so the Recycle Bin holds what was shown.
        for i in 0..self.docs.len() {
            let covered = self.docs[i].path.as_ref().is_some_and(|p| paths.iter().any(|gone| p.starts_with(gone)));
            if covered && self.docs[i].is_dirty() {
                self.save_doc(i);
            }
        }
        // Large trees take a while: the work goes to a thread, and the frame stays live.
        let count = paths.len();
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = repaint(ctx);
        let job = paths.clone();
        std::thread::spawn(move || {
            let mut failed = Vec::new();
            for path in &job {
                let done = if permanent {
                    coiman_core::project::file_ops::remove_all(path).map_err(|e| e.to_string())
                } else {
                    trash::delete(path).map_err(|e| e.to_string())
                };
                if let Err(e) = done {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    failed.push(format!("{name}: {e}"));
                }
            }
            let message = if failed.is_empty() {
                let items = if count == 1 { "1 item".to_owned() } else { format!("{count} items") };
                if permanent { format!("Deleted {items}") } else { format!("Moved {items} to the Recycle Bin") }
            } else {
                format!("Could not delete {}", failed.join("; "))
            };
            let _ = tx.send((job, message));
            wake();
        });
        self.deleting = Some(rx);
        self.status =
            Some(if count == 1 { "Deleting 1 item…".to_owned() } else { format!("Deleting {count} items…") });
    }

    /// Pastes the Explorer's copied or cut files into `dir`. Copies take a free name
    /// ("a copy.rs"); cut files move, and open files follow them.
    pub(crate) fn paste_into(&mut self, dir: &Path) {
        let Some((files, cut)) = self.explorer.clipboard.clone() else { return };
        let failed = if cut {
            let (moved, why) = self.move_into_paths(&files, dir);
            self.explorer.clipboard = None;
            self.explorer.forget(&moved);
            why
        } else {
            Self::copy_into_paths(&files, dir)
        };
        self.explorer.refresh();
        self.finder.mark_stale();
        self.status = Some(if failed.is_empty() {
            format!(
                "{} {} item{}",
                if cut { "Moved" } else { "Pasted" },
                files.len(),
                if files.len() == 1 { "" } else { "s" }
            )
        } else {
            format!("Could not paste {}", failed.join("; "))
        });
    }

    /// Copies `files` into `dir`, each under a free name ("a copy.rs" when "a.rs" is taken).
    /// Returns what couldn't be copied, with why.
    pub(crate) fn copy_into_paths(files: &[PathBuf], dir: &Path) -> Vec<String> {
        use coiman_core::project::file_ops;
        let mut failed = Vec::new();
        for from in files {
            let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if from.is_dir() && file_ops::is_within(dir, from) {
                failed.push(format!("{name}: a folder can't go inside itself"));
                continue;
            }
            if let Err(e) = file_ops::copy_all(from, &file_ops::free_copy_name(dir, &name)) {
                failed.push(format!("{name}: {e}"));
            }
        }
        failed
    }

    /// Moves what was dragged onto a folder there, with open files following it.
    pub(crate) fn move_into(&mut self, files: &[PathBuf], dir: &Path) {
        let (moved, failed) = self.move_into_paths(files, dir);
        self.explorer.forget(&moved);
        self.explorer.refresh();
        self.finder.mark_stale();
        self.status = Some(if !failed.is_empty() {
            format!("Could not move: {}", failed.join("; "))
        } else if moved.is_empty() {
            "Already in that folder".to_owned()
        } else {
            let to =
                dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string());
            format!("Moved {} item{} to {to}", moved.len(), if moved.len() == 1 { "" } else { "s" })
        });
    }

    /// Moves `files` into `dir`; returns what moved and what couldn't be, with why.
    fn move_into_paths(&mut self, files: &[PathBuf], dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
        use coiman_core::project::file_ops;
        let (mut moved, mut failed) = (Vec::new(), Vec::new());
        for from in files {
            let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if from.parent().is_some_and(|parent| same_path(parent, dir)) {
                continue;
            }
            if from.is_dir() && file_ops::is_within(dir, from) {
                failed.push(format!("{name}: a folder can't go inside itself"));
                continue;
            }
            let to = dir.join(&name);
            if to.exists() {
                failed.push(format!("{name} already exists there"));
                continue;
            }
            match file_ops::move_to(from, &to) {
                Ok(()) => {
                    for doc in &mut self.docs {
                        let follows = doc.path.as_ref().and_then(|p| file_ops::moved_path(p, from, &to));
                        if let Some(path) = follows {
                            self.watcher.watch_file(&path);
                            doc.path = Some(path);
                        }
                    }
                    moved.push(from.clone());
                }
                Err(e) => failed.push(format!("{name}: {e}")),
            }
        }
        (moved, failed)
    }

    pub(crate) fn open_folder(&mut self, path: PathBuf) {
        self.restore_breakpoints(&path);
        self.recent.add_project(&path, recent::now());
        self.remember_recent();
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

    pub(crate) fn save_doc(&mut self, index: usize) -> bool {
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
                    self.history_record(index, coiman_core::project::history::Source::Save);
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

    pub(crate) fn save_doc_as(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get_mut(index) else { return false };
        // A saved file's own folder; an untitled one where Save As was last left.
        let own = doc.path.as_ref().and_then(|p| p.parent()).map(Path::to_path_buf);
        let title = doc.title();
        let mut dialog = self.file_dialog("save_as").set_file_name(title);
        if let Some(dir) = own {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else { return false };
        self.dialog_chose("save_as", &path);
        let Some(doc) = self.docs.get_mut(index) else { return false };
        match doc.save_as(path) {
            Ok(()) => {
                self.status = Some(format!("Saved {}", doc.title()));
                self.explorer.refresh();
                self.finder.mark_stale();
                self.lsp_did_save(index);
                self.history_record(index, coiman_core::project::history::Source::Save);
                self.plugins_saved(index);
                true
            }
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    pub(crate) fn request_close_tab(&mut self, index: usize) {
        if self.docs.get(index).is_some_and(Document::is_dirty) {
            self.pending_close = Some(PendingClose::Tab(index));
        } else {
            self.close_tab(index);
        }
    }

    pub(crate) fn close_tab(&mut self, index: usize) {
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
    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
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
