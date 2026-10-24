//! Main application: window layout, tabs, editor, shortcuts and dialogs.

use std::path::{Path, PathBuf};

use egui::{Align, FontId, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke};

use crate::document::{DiskState, Document};
use crate::edit_ops::{self, CaseTransform, Edit, Sel};
use crate::editor_view::{self, editor_id};
use crate::explorer::Explorer;
use crate::palette::{Command, Palette, PaletteResult};
use crate::search::SearchPanel;
use crate::finder::FileFinder;
use crate::keymap::Keymap;
use crate::session::{self, Session};
use crate::settings::{self, Settings};
use crate::syntax::SyntaxWorker;
use crate::watcher::{FileWatcher, same_path};
use crate::terminal::{Terminal, strip_verbatim};
use crate::theme;

const DEFAULT_FONT_SIZE: f32 = 14.0;
const INDENT: &str = "    ";
/// Seconds between crash-recovery session backups.
const BACKUP_INTERVAL: f64 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Explorer,
    Search,
}

enum PendingClose {
    Tab(usize),
    App,
}

#[derive(Default)]
struct FindBar {
    open: bool,
    query: String,
    match_case: bool,
    focus_requested: bool,
    /// Matches for (doc uid, buffer version, query, match case), recomputed only on change.
    cache: Option<(u64, u64, String, bool, Vec<(usize, usize)>)>,
}

pub struct CodeApp {
    docs: Vec<Document>,
    active: usize,
    next_uid: u64,
    untitled_counter: usize,

    explorer: Explorer,
    search: SearchPanel,
    terminal: Terminal,
    palette: Palette,
    find: FindBar,
    /// Quick Open index of the open folder.
    finder: FileFinder,
    /// Reports changes made to files by other programs.
    watcher: FileWatcher,

    sidebar_visible: bool,
    sidebar_view: SidebarView,
    terminal_visible: bool,
    font_size: f32,
    word_wrap: bool,
    zen_mode: bool,
    /// Paths of closed editors, most recent last, for Reopen Closed Editor.
    closed_editors: Vec<PathBuf>,

    pending_close: Option<PendingClose>,
    allow_close: bool,
    /// First stroke of a pending two-key chord (e.g. Ctrl+K).
    chord: Option<KeyboardShortcut>,
    settings: Settings,
    keymap: Keymap,
    /// Themes found for the Color Theme picker (scanned on first use).
    themes: Vec<theme::ThemeEntry>,
    /// Reload settings at the start of the next frame (a config file was saved).
    settings_reload: bool,
    /// Whether settings have been applied once (first load applies everything).
    settings_applied: bool,
    status: Option<String>,
    /// Last session JSON written, to skip unchanged backups.
    last_session_json: String,
    last_backup_time: f64,
    /// Window title last sent to the OS.
    last_title: String,
    /// Background tree-sitter parser, started on the first frame.
    syntax: Option<SyntaxWorker>,
}

impl CodeApp {
    pub fn new(start_path: Option<PathBuf>, ctx: &egui::Context) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            next_uid: 1,
            untitled_counter: 0,
            explorer: Explorer::default(),
            search: SearchPanel::default(),
            terminal: Terminal::new(cwd),
            palette: Palette::default(),
            find: FindBar::default(),
            finder: FileFinder::new(ctx.clone()),
            watcher: FileWatcher::new(ctx.clone()),
            sidebar_visible: true,
            sidebar_view: SidebarView::Explorer,
            terminal_visible: false,
            font_size: DEFAULT_FONT_SIZE,
            word_wrap: false,
            zen_mode: false,
            closed_editors: Vec::new(),
            pending_close: None,
            allow_close: false,
            chord: None,
            settings: Settings::default(),
            keymap: Keymap::defaults(),
            themes: Vec::new(),
            settings_reload: false,
            settings_applied: false,
            status: None,
            last_session_json: String::new(),
            last_backup_time: 0.0,
            last_title: String::new(),
            syntax: None,
        };
        // Always restore the previous session first so unsaved work is never
        // dropped; a path given on the command line then opens on top of it.
        if let Some(saved) = session::load() {
            app.restore_session(saved);
        }
        if let Some(path) = start_path {
            let path = path.canonicalize().map(strip_verbatim).unwrap_or(path);
            if path.is_dir() {
                app.open_folder(path);
            } else {
                app.open_file(&path);
            }
        }
        app.reload_settings(ctx);
        app
    }

    // ----- settings ------------------------------------------------------------

    /// Opens a settings/keybindings file, creating it with `initial` contents first.
    fn open_config_file(&mut self, path: Option<PathBuf>, initial: String) {
        let Some(path) = path else {
            self.status = Some("No config directory available".to_owned());
            return;
        };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, initial) {
                self.status = Some(format!("Could not create {}: {e}", path.display()));
                return;
            }
        }
        self.open_file(&path);
    }

    fn is_config_file(&self, path: &Path) -> bool {
        let mut files: Vec<PathBuf> =
            [settings::user_settings_path(), settings::user_keybindings_path()].into_iter().flatten().collect();
        if let Some(root) = &self.explorer.root {
            files.push(settings::workspace_settings_path(root));
        }
        files.iter().any(|f| f == path)
    }

    /// Reloads settings, keybindings and the color theme, applying what changed.
    fn reload_settings(&mut self, ctx: &egui::Context) {
        let loaded = settings::load(self.explorer.root.as_deref());
        let (keymap, key_problems) = Keymap::load(settings::user_keybindings_path().as_deref());
        let mut problems = loaded.problems;
        problems.extend(key_problems);
        let first = !self.settings_applied;
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        let new = self.settings.clone();
        if first || new.editor.font_size != old.editor.font_size {
            self.font_size = new.editor.font_size.clamp(6.0, 72.0);
        }
        if first || new.editor.word_wrap != old.editor.word_wrap {
            self.word_wrap = new.editor.word_wrap;
        }
        if first || new.workbench.color_theme != old.workbench.color_theme {
            // The built-in theme needs no scan of installed VS Code themes.
            if self.themes.is_empty() && !new.workbench.color_theme.eq_ignore_ascii_case("Dark Modern") {
                self.themes = theme::discover();
            }
            let (t, err) = theme::resolve(&new.workbench.color_theme, &self.themes);
            theme::set(ctx, t);
            problems.extend(err);
        }
        theme::apply_style(ctx, new.workbench.ui_font_size);
        self.keymap = keymap;
        self.settings_applied = true;
        if !problems.is_empty() {
            self.status = Some(format!("Settings: {}", problems.join("; ")));
        }
    }

    /// Saves the picked theme into the user settings file (keeping its comments) and applies it.
    fn set_color_theme(&mut self, ctx: &egui::Context, label: &str) {
        let Some(path) = settings::user_settings_path() else { return };
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| settings::default_file_contents());
        let value = toml::Value::String(label.to_owned()).to_string();
        let updated = settings::set_value_in_text(&text, "workbench", "color_theme", &value);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, updated) {
            Ok(()) => {
                // An open, unmodified tab of the file would otherwise show stale text
                // (and saving it would undo the change).
                if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path.as_path()) && !d.is_dirty())
                    && let Ok(fresh) = Document::open(&path)
                {
                    let uid = self.docs[i].uid;
                    self.docs[i] = fresh;
                    self.docs[i].uid = uid;
                }
                self.reload_settings(ctx);
                if self.settings.workbench.color_theme != label {
                    self.status = Some("Theme saved, but workspace settings override it".to_owned());
                }
            }
            Err(e) => self.status = Some(format!("Could not save theme: {e}")),
        }
    }

    // ----- external changes ----------------------------------------------------

    /// Applies changes other programs made to files: reload clean tabs, flag
    /// conflicts and deletions, refresh the explorer and Quick Open index.
    fn pump_watcher(&mut self) {
        let changes = self.watcher.drain();
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

    /// Bar above the editor when the file changed or vanished on disk.
    fn disk_conflict_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let message = match doc.disk {
            DiskState::InSync => return,
            DiskState::Changed => "This file was changed on disk, and you have unsaved changes.",
            DiskState::Deleted => "This file was deleted on disk. Save to recreate it.",
        };
        let deleted = doc.disk == DiskState::Deleted;
        let mut action = None;
        egui::Frame::new()
            .fill(theme::colors().hover_bg)
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(message).color(theme::colors().modified));
                    if !deleted && ui.button("Reload (discard my changes)").clicked() {
                        action = Some(true);
                    }
                    if ui.button(if deleted { "Close" } else { "Keep mine" }).clicked() {
                        action = Some(false);
                    }
                });
            });
        match (action, deleted) {
            (Some(true), _) => {
                let doc = &mut self.docs[self.active];
                if let Err(e) = doc.reload_from_disk() {
                    self.status = Some(format!("Reload failed: {e}"));
                }
            }
            (Some(false), true) => self.close_tab(self.active),
            // Keep mine: saving will overwrite the file on disk.
            (Some(false), false) => self.docs[self.active].disk = DiskState::InSync,
            (None, _) => {}
        }
    }

    // ----- syntax --------------------------------------------------------------

    /// Collects finished parses and sends the active document's latest text.
    fn pump_syntax(&mut self, ctx: &egui::Context) {
        let worker = self.syntax.get_or_insert_with(|| SyntaxWorker::spawn(ctx.clone()));
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

    // ----- hot exit --------------------------------------------------------------

    fn restore_session(&mut self, saved: Session) {
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
    fn save_session(&mut self) -> bool {
        let session = Session::capture(self.explorer.root.as_deref(), &self.docs, self.active, self.settings.files.hot_exit);
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

    fn periodic_backup(&mut self, ctx: &egui::Context) {
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

    // ----- document management -------------------------------------------------

    fn add_doc(&mut self, mut doc: Document) {
        // Focus the editor when the tab is first shown, so typing goes straight into it.
        doc.request_focus = true;
        doc.uid = self.next_uid;
        self.next_uid += 1;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }

    fn active_doc(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    fn new_file(&mut self) {
        self.untitled_counter += 1;
        self.add_doc(Document::untitled(self.untitled_counter));
    }

    fn open_file(&mut self, path: &Path) {
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path)) {
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

    fn open_folder(&mut self, path: PathBuf) {
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

    fn save_doc(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get_mut(index) else { return false };
        match doc.save() {
            Ok(true) => {
                let (title, path) = (doc.title(), doc.path.clone());
                self.status = Some(format!("Saved {title}"));
                if path.is_some_and(|p| self.is_config_file(&p)) {
                    self.settings_reload = true;
                }
                true
            }
            Ok(false) => self.save_doc_as(index),
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    fn save_doc_as(&mut self, index: usize) -> bool {
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
                true
            }
            Err(e) => {
                self.status = Some(format!("Save failed: {e}"));
                false
            }
        }
    }

    fn request_close_tab(&mut self, index: usize) {
        if self.docs.get(index).is_some_and(Document::is_dirty) {
            self.pending_close = Some(PendingClose::Tab(index));
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.docs.len() {
            return;
        }
        let doc = self.docs.remove(index);
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

    /// Applies a text edit to the active document and moves its selection.
    fn apply_edit(&mut self, f: impl FnOnce(&str, Sel) -> Option<Edit>) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let text = doc.text();
        if let Some(edit) = f(&text, doc.selection()) {
            doc.replace_all(&edit.text, edit.sel);
        }
    }

    fn editor_focused(&self, ctx: &egui::Context) -> bool {
        self.docs
            .get(self.active)
            .is_some_and(|d| ctx.memory(|m| m.has_focus(editor_id(d.uid))))
    }

    fn run_command(&mut self, ctx: &egui::Context, cmd: Command) {
        match cmd {
            Command::CommandPalette => self.open_palette(">"),
            Command::QuickOpen => self.open_palette(""),
            Command::NewFile => self.new_file(),
            Command::OpenFile => {
                let mut dialog = rfd::FileDialog::new();
                if let Some(root) = &self.explorer.root {
                    dialog = dialog.set_directory(root);
                }
                if let Some(path) = dialog.pick_file() {
                    self.open_file(&path);
                }
            }
            Command::OpenFolder => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.open_folder(path);
                }
            }
            Command::Save => {
                self.save_doc(self.active);
            }
            Command::SaveAs => {
                self.save_doc_as(self.active);
            }
            Command::SaveAll => {
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        self.save_doc(i);
                    }
                }
            }
            Command::CloseTab => self.request_close_tab(self.active),
            Command::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
            Command::ToggleTerminal => {
                self.terminal_visible = !self.terminal_visible;
                self.terminal.focus_requested = self.terminal_visible;
            }
            Command::ShowExplorer => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Explorer;
            }
            Command::ShowSearch => {
                self.sidebar_visible = true;
                self.sidebar_view = SidebarView::Search;
                self.search.focus_requested = true;
            }
            Command::Find => {
                self.find.open = true;
                self.find.focus_requested = true;
            }
            Command::GoToLine => self.palette.show_with(":"),
            Command::ZoomIn => self.font_size = (self.font_size + 1.0).min(40.0),
            Command::ZoomOut => self.font_size = (self.font_size - 1.0).max(8.0),
            Command::ResetZoom => self.font_size = self.settings.editor.font_size,
            Command::RefreshExplorer => {
                self.explorer.refresh();
                self.finder.mark_stale();
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),

            Command::ToggleLineComment => {
                let ext = self.docs.get(self.active).map(Document::extension).unwrap_or_default();
                match edit_ops::comment_token(&ext) {
                    Some(token) => self.apply_edit(|t, s| Some(edit_ops::toggle_line_comment(t, s, token))),
                    None => self.status = Some("No line comment syntax for this language".to_owned()),
                }
            }
            Command::MoveLinesUp => self.apply_edit(|t, s| edit_ops::move_lines(t, s, false)),
            Command::MoveLinesDown => self.apply_edit(|t, s| edit_ops::move_lines(t, s, true)),
            Command::CopyLinesUp => self.apply_edit(|t, s| Some(edit_ops::copy_lines(t, s, false))),
            Command::CopyLinesDown => self.apply_edit(|t, s| Some(edit_ops::copy_lines(t, s, true))),
            Command::DeleteLines => self.apply_edit(|t, s| Some(edit_ops::delete_lines(t, s))),
            Command::IndentLines => self.apply_edit(|t, s| Some(edit_ops::indent_lines(t, s, false, INDENT))),
            Command::OutdentLines => self.apply_edit(|t, s| Some(edit_ops::indent_lines(t, s, true, INDENT))),
            Command::SelectNextOccurrence => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.add_next_occurrence();
                    doc.reveal = true;
                }
            }
            Command::AddCursorAbove | Command::AddCursorBelow => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.add_cursor_vertical(cmd == Command::AddCursorBelow);
                    doc.reveal = true;
                }
            }
            Command::JumpToBracket => {
                // Jump to the partner of the bracket next to the cursor.
                if let Some(doc) = self.active_doc() {
                    let head = doc.buffer.primary().head;
                    if let Some((open, close)) = doc.buffer.matching_bracket(head, 1_000_000) {
                        let target = if head == open || head == open + 1 { close } else { open };
                        doc.select(target, target);
                    }
                }
            }
            Command::JoinLines => self.apply_edit(edit_ops::join_lines),
            Command::DuplicateSelection => self.apply_edit(|t, s| Some(edit_ops::duplicate_selection(t, s))),
            Command::SortLinesAscending => self.apply_edit(|t, s| Some(edit_ops::sort_lines(t, s, false))),
            Command::SortLinesDescending => self.apply_edit(|t, s| Some(edit_ops::sort_lines(t, s, true))),
            Command::RemoveDuplicateLines => self.apply_edit(|t, s| Some(edit_ops::remove_duplicate_lines(t, s))),
            Command::ReverseLines => self.apply_edit(|t, s| Some(edit_ops::reverse_lines(t, s))),
            Command::TransformToUppercase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Upper)));
            }
            Command::TransformToLowercase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Lower)));
            }
            Command::TransformToTitlecase => {
                self.apply_edit(|t, s| Some(edit_ops::transform_case(t, s, CaseTransform::Title)));
            }

            Command::ToggleWordWrap => self.word_wrap = !self.word_wrap,
            Command::ToggleZenMode => {
                self.zen_mode = !self.zen_mode;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.zen_mode));
            }
            Command::NextEditor | Command::PreviousEditor => {
                let n = self.docs.len();
                if n > 0 {
                    let step = if cmd == Command::NextEditor { 1 } else { n - 1 };
                    self.active = (self.active + step) % n;
                    self.docs[self.active].request_focus = true;
                }
            }
            Command::OpenUserSettings => {
                self.open_config_file(settings::user_settings_path(), settings::default_file_contents());
            }
            Command::OpenWorkspaceSettings => match self.explorer.root.clone() {
                Some(root) => {
                    self.open_config_file(Some(settings::workspace_settings_path(&root)), settings::default_file_contents());
                }
                None => self.status = Some("Open a folder to use workspace settings".to_owned()),
            },
            Command::OpenKeybindings => {
                let contents = Keymap::defaults().default_file_contents();
                self.open_config_file(settings::user_keybindings_path(), contents);
            }
            Command::SelectTheme => {
                if self.themes.is_empty() {
                    self.themes = theme::discover();
                }
                let mut labels: Vec<String> = self.themes.iter().map(|t| t.label.clone()).collect();
                if !labels.iter().any(|l| l == "Dark Modern") {
                    labels.insert(0, "Dark Modern".to_owned());
                }
                self.palette.show_themes(labels, &self.settings.workbench.color_theme);
            }
            Command::ReopenClosedEditor => {
                if let Some(path) = self.closed_editors.pop() {
                    self.open_file(&path);
                }
            }
        }
    }

    fn open_palette(&mut self, prefix: &str) {
        self.finder.refresh_if_stale();
        self.palette.show_with(prefix);
    }

    // ----- input ---------------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.zen_mode
            && !self.find.open
            && self.chord.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.run_command(ctx, Command::ToggleZenMode);
            return;
        }
        let editor_focused = self.editor_focused(ctx);
        // Tab with a multi-line selection indents the lines instead of replacing them.
        if editor_focused && self.chord.is_none() {
            let multi_line = self.docs.get(self.active).is_some_and(|d| {
                let p = d.buffer.primary();
                d.buffer.char_to_line(p.start()) != d.buffer.char_to_line(p.end())
            });
            if multi_line && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                self.run_command(ctx, Command::IndentLines);
                return;
            }
        }
        if let Some(cmd) = self.keymap.process(ctx, &mut self.chord, editor_focused) {
            self.run_command(ctx, cmd);
        }
    }

    /// Hot exit: on close, unsaved buffers go into the session backup instead of a
    /// "save changes?" prompt. The prompt is only a fallback if the backup fails.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        let saved = self.save_session();
        if (!saved || !self.settings.files.hot_exit) && self.docs.iter().any(Document::is_dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_close = Some(PendingClose::App);
        }
    }

    // ----- UI pieces -----------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        let keymap = &self.keymap;
        egui::MenuBar::new().ui(ui, |ui| {
            let mut item = |ui: &mut egui::Ui, label: &str, c: Command| {
                if ui.add(egui::Button::new(label).shortcut_text(keymap.shortcut_text(c))).clicked() {
                    cmd = Some(c);
                }
            };
            ui.menu_button("File", |ui| {
                item(ui, "New Text File", Command::NewFile);
                ui.separator();
                item(ui, "Open File...", Command::OpenFile);
                item(ui, "Open Folder...", Command::OpenFolder);
                ui.separator();
                item(ui, "Save", Command::Save);
                item(ui, "Save As...", Command::SaveAs);
                item(ui, "Save All", Command::SaveAll);
                ui.separator();
                ui.menu_button("Preferences", |ui| {
                    item(ui, "Settings", Command::OpenUserSettings);
                    item(ui, "Workspace Settings", Command::OpenWorkspaceSettings);
                    item(ui, "Keyboard Shortcuts", Command::OpenKeybindings);
                    item(ui, "Color Theme", Command::SelectTheme);
                });
                ui.separator();
                item(ui, "Close Editor", Command::CloseTab);
                item(ui, "Reopen Closed Editor", Command::ReopenClosedEditor);
                item(ui, "Exit", Command::Quit);
            });
            ui.menu_button("Edit", |ui| {
                item(ui, "Find", Command::Find);
                item(ui, "Find in Files", Command::ShowSearch);
                ui.separator();
                item(ui, "Toggle Line Comment", Command::ToggleLineComment);
            });
            ui.menu_button("Selection", |ui| {
                item(ui, "Copy Line Up", Command::CopyLinesUp);
                item(ui, "Copy Line Down", Command::CopyLinesDown);
                item(ui, "Move Line Up", Command::MoveLinesUp);
                item(ui, "Move Line Down", Command::MoveLinesDown);
                item(ui, "Duplicate Selection", Command::DuplicateSelection);
                ui.separator();
                item(ui, "Add Next Occurrence", Command::SelectNextOccurrence);
                ui.separator();
                item(ui, "Sort Lines Ascending", Command::SortLinesAscending);
                item(ui, "Sort Lines Descending", Command::SortLinesDescending);
                item(ui, "Join Lines", Command::JoinLines);
            });
            ui.menu_button("View", |ui| {
                item(ui, "Command Palette...", Command::CommandPalette);
                ui.separator();
                item(ui, "Explorer", Command::ShowExplorer);
                item(ui, "Search", Command::ShowSearch);
                item(ui, "Terminal", Command::ToggleTerminal);
                item(ui, "Toggle Side Bar", Command::ToggleSidebar);
                item(ui, "Zen Mode", Command::ToggleZenMode);
                item(ui, "Word Wrap", Command::ToggleWordWrap);
                ui.separator();
                item(ui, "Zoom In", Command::ZoomIn);
                item(ui, "Zoom Out", Command::ZoomOut);
                item(ui, "Reset Zoom", Command::ResetZoom);
            });
            ui.menu_button("Go", |ui| {
                item(ui, "Go to File...", Command::QuickOpen);
                item(ui, "Go to Bracket", Command::JumpToBracket);
                item(ui, "Go to Line...", Command::GoToLine);
            });
            ui.menu_button("Terminal", |ui| {
                item(ui, "Toggle Terminal", Command::ToggleTerminal);
            });
        });
        cmd
    }

    fn activity_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        ui.add_space(6.0);
        let mut icon = |ui: &mut egui::Ui, glyph: &str, tip: &str, active: bool, c: Command| {
            let color = if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(48.0, 44.0), Sense::click());
            if active {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                    0.0,
                    theme::colors().accent,
                );
            }
            let color = if resp.hovered() { theme::colors().tab_active_fg } else { color };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(22.0),
                color,
            );
            if resp.on_hover_text(tip).clicked() {
                cmd = Some(c);
            }
        };
        let side = self.sidebar_visible.then_some(self.sidebar_view);
        icon(ui, "📁", "Explorer (Ctrl+Shift+E)", side == Some(SidebarView::Explorer), Command::ShowExplorer);
        icon(ui, "🔍", "Search (Ctrl+Shift+F)", side == Some(SidebarView::Search), Command::ShowSearch);
        icon(ui, "⌨", "Terminal (Ctrl+`)", self.terminal_visible, Command::ToggleTerminal);
        cmd
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let active_path = self.docs.get(self.active).and_then(|d| d.path.clone());
        match self.sidebar_view {
            SidebarView::Explorer => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("EXPLORER").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if self.explorer.root.is_some()
                            && ui.small_button("⟳").on_hover_text("Refresh Explorer").clicked()
                        {
                            self.explorer.refresh();
                            self.finder.mark_stale();
                        }
                    });
                });
                if let Some(root) = &self.explorer.root {
                    let name = root.file_name().map_or_else(
                        || root.display().to_string(),
                        |n| n.to_string_lossy().to_uppercase(),
                    );
                    ui.label(RichText::new(name).strong().size(small_size(ui)));
                }
                if self.explorer.root.is_none() && ui.button("Open Folder").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        self.open_folder(path);
                    }
                }
                if let Some(path) = self.explorer.ui(ui, active_path.as_deref()) {
                    self.open_file(&path);
                }
            }
            SidebarView::Search => {
                ui.label(RichText::new("SEARCH").small().strong());
                if let Some(jump) = self.search.ui(ui, self.explorer.root.as_deref()) {
                    self.open_file(&jump.path);
                    if let Some(doc) = self.active_doc() {
                        // Select the match itself, not just the line.
                        let line = jump.line.saturating_sub(1).min(doc.buffer.len_lines() - 1);
                        let start = doc.buffer.line_start(line);
                        let end = doc.buffer.line_end(line);
                        doc.select((start + jump.cols.0).min(end), (start + jump.cols.1).min(end));
                    }
                }
            }
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .auto_shrink([false, true])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (i, doc) in self.docs.iter().enumerate() {
                        let active = i == self.active;
                        let fill = if active { theme::colors().tab_active_bg } else { theme::colors().tab_inactive_bg };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let color = if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
                                    let label = ui.add(
                                        egui::Label::new(RichText::new(doc.title()).color(color))
                                            .sense(Sense::click())
                                            .selectable(false),
                                    );
                                    if label.clicked() {
                                        select = Some(i);
                                    }
                                    if label.middle_clicked() {
                                        close = Some(i);
                                    }
                                    let close_glyph = if doc.is_dirty() { "●" } else { "×" };
                                    let close_color =
                                        if doc.is_dirty() { theme::colors().modified } else { theme::colors().text_dim };
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new(close_glyph).color(close_color))
                                                .frame(false),
                                        )
                                        .on_hover_text("Close (Ctrl+W)")
                                        .clicked()
                                    {
                                        close = Some(i);
                                    }
                                });
                            })
                            .response;
                        if active {
                            let r = resp.rect;
                            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, theme::colors().accent));
                        } else {
                            let r = resp.rect;
                            ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0, theme::colors().border));
                        }
                    }
                });
            });
        if let Some(i) = select {
            self.active = i;
        }
        if let Some(i) = close {
            self.request_close_tab(i);
        }
    }

    fn breadcrumbs(&self, ui: &mut egui::Ui) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let text = match (&doc.path, &self.explorer.root) {
            (Some(p), Some(root)) => p
                .strip_prefix(root)
                .unwrap_or(p)
                .components()
                // Skip the root "\" component so absolute paths read "C: › Users › …".
                .filter(|c| !matches!(c, std::path::Component::RootDir))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("  ›  "),
            (Some(p), None) => p.display().to_string(),
            (None, _) => doc.title(),
        };
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(text).size(small_size(ui)).color(theme::colors().text_dim));
        });
    }

    fn find_bar(&mut self, ui: &mut egui::Ui) {
        if !self.find.open {
            return;
        }
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let key = (doc.uid, doc.buffer.version(), self.find.query.clone(), self.find.match_case);
        let fresh = self
            .find
            .cache
            .as_ref()
            .is_some_and(|(u, v, q, c, _)| (*u, *v, q, *c) == (key.0, key.1, &key.2, key.3));
        if !fresh {
            let matches = find_matches(&doc.text(), &self.find.query, self.find.match_case);
            self.find.cache = Some((key.0, key.1, key.2, key.3, matches));
        }
        let matches = self.find.cache.as_ref().map(|c| c.4.clone()).unwrap_or_default();
        let sel_start = doc.buffer.primary().start();
        let current = matches.iter().position(|&(s, _)| s >= sel_start);

        let mut go_next = false;
        let mut go_prev = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                if ui.small_button("×").on_hover_text("Close (Escape)").clicked() {
                    self.find.open = false;
                }
                if ui.small_button("↓").on_hover_text("Next Match (Enter)").clicked() {
                    go_next = true;
                }
                if ui.small_button("↑").on_hover_text("Previous Match (Shift+Enter)").clicked() {
                    go_prev = true;
                }
                let count = if self.find.query.is_empty() {
                    String::new()
                } else if matches.is_empty() {
                    "No results".to_owned()
                } else {
                    let shown = current.map_or(matches.len(), |c| c + 1);
                    format!("{shown} of {}", matches.len())
                };
                ui.label(RichText::new(count).small().color(theme::colors().text_dim));
                if ui
                    .selectable_label(self.find.match_case, "Aa")
                    .on_hover_text("Match Case")
                    .clicked()
                {
                    self.find.match_case = !self.find.match_case;
                }
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.find.query)
                        .hint_text("Find")
                        .desired_width(220.0),
                );
                if self.find.focus_requested {
                    edit.request_focus();
                    self.find.focus_requested = false;
                }
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    if ui.input(|i| i.modifiers.shift) {
                        go_prev = true;
                    } else {
                        go_next = true;
                    }
                    edit.request_focus();
                }
                if edit.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.find.open = false;
                }
            });
        });

        if matches.is_empty() {
            return;
        }
        let target = if go_next {
            // First match strictly after the current selection start.
            Some(matches.iter().find(|&&(s, _)| s > sel_start).unwrap_or(&matches[0]))
        } else if go_prev {
            Some(matches.iter().rev().find(|&&(s, _)| s < sel_start).unwrap_or(&matches[matches.len() - 1]))
        } else {
            None
        };
        if let Some(&(s, e)) = target {
            // Keep focus in the find box (like VS Code); just move the selection.
            doc.buffer.set_single(crate::buffer::Selection::new(s, e));
            doc.reveal = true;
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.label(RichText::new("Rust Code").size(44.0).color(theme::colors().text_dim));
            ui.label(RichText::new("Editing evolved, in Rust").size(16.0).color(theme::colors().text_dim));
            ui.add_space(24.0);
            let keymap = &self.keymap;
            let rows: &[(&str, Command)] = &[
                ("New File", Command::NewFile),
                ("Open File...", Command::OpenFile),
                ("Open Folder...", Command::OpenFolder),
                ("Show All Commands", Command::CommandPalette),
                ("Go to File...", Command::QuickOpen),
                ("Toggle Terminal", Command::ToggleTerminal),
            ];
            // Grids don't center themselves, so pad to roughly center a ~300px block.
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 300.0) / 2.0).max(0.0));
                egui::Grid::new("welcome").num_columns(2).spacing([40.0, 10.0]).show(ui, |ui| {
                    for (label, c) in rows {
                        if ui.link(*label).clicked() {
                            cmd = Some(*c);
                        }
                        ui.label(RichText::new(keymap.shortcut_text(*c)).monospace().color(theme::colors().text_dim));
                        ui.end_row();
                    }
                });
            });
        });
        cmd
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let size = small_size(ui);
            let white = |s: String| RichText::new(s).size(size).color(theme::colors().status_fg);
            if let Some(root) = &self.explorer.root {
                let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
                ui.label(white(format!("📁 {name}")));
            }
            if let Some(msg) = &self.status {
                ui.label(white(msg.clone()));
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if let Some(doc) = self.docs.get(self.active) {
                    ui.label(white(doc.language_name().to_owned()));
                    if doc.large {
                        ui.label(white("Large file mode".to_owned()))
                            .on_hover_text("Syntax highlighting and word wrap are off for very large files");
                    }
                    ui.label(white(doc.line_ending().to_owned()));
                    ui.label(white("UTF-8".to_owned()));
                    ui.label(white("Spaces: 4".to_owned()));
                    let (line, col) = doc.cursor();
                    let count = doc.buffer.selections().len();
                    let sel = match (count, doc.selection_len()) {
                        (1, 0) => String::new(),
                        (1, n) => format!(" ({n} selected)"),
                        (n, _) => format!(" ({n} selections)"),
                    };
                    ui.label(white(format!("Ln {line}, Col {col}{sel}")));
                }
                if (self.font_size - self.settings.editor.font_size).abs() > f32::EPSILON {
                    ui.label(white(format!("🔍 {}px", self.font_size)));
                }
            });
        });
    }

    fn close_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending_close else { return };
        let (message, index) = match pending {
            PendingClose::Tab(i) => {
                let title = self.docs.get(*i).map(Document::title).unwrap_or_default();
                (format!("Do you want to save the changes you made to {title}?"), Some(*i))
            }
            PendingClose::App => {
                let n = self.docs.iter().filter(|d| d.is_dirty()).count();
                (format!("Do you want to save the changes to {n} file(s) before exiting?"), None)
            }
        };

        let mut choice = None;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(&message);
                ui.label(RichText::new("Your changes will be lost if you don't save them.").color(theme::colors().text_dim));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                        choice = Some(2);
                    }
                });
            });

        match (choice, index) {
            (Some(0), Some(i)) => {
                if self.save_doc(i) {
                    self.close_tab(i);
                }
                self.pending_close = None;
            }
            (Some(1), Some(i)) => {
                self.close_tab(i);
                self.pending_close = None;
            }
            (Some(0), None) => {
                let mut all_saved = true;
                for i in 0..self.docs.len() {
                    if self.docs[i].is_dirty() {
                        all_saved &= self.save_doc(i);
                    }
                }
                self.pending_close = None;
                if all_saved {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            (Some(1), None) => {
                self.pending_close = None;
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Some(_), _) => self.pending_close = None,
            (None, _) => {}
        }
    }

    fn update_title(&mut self, ctx: &egui::Context) {
        let mut title = String::new();
        if let Some(doc) = self.docs.get(self.active) {
            if doc.is_dirty() {
                title.push_str("● ");
            }
            title.push_str(&doc.title());
            title.push_str(" - ");
        }
        if let Some(root) = &self.explorer.root
            && let Some(name) = root.file_name()
        {
            title.push_str(&name.to_string_lossy());
            title.push_str(" - ");
        }
        title.push_str("Rust Code");
        // Sending a viewport command schedules another frame, so only send changes;
        // otherwise the app repaints continuously.
        if title != self.last_title {
            self.last_title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}

impl eframe::App for CodeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_close_request(&ctx);
        if std::mem::take(&mut self.settings_reload) {
            self.reload_settings(&ctx);
        }
        self.periodic_backup(&ctx);
        self.pump_syntax(&ctx);
        self.pump_watcher();
        if self.pending_close.is_none() && !self.palette.open {
            self.handle_shortcuts(&ctx);
        }

        let mut cmd = None;

        let zen = self.zen_mode;
        // Zen mode hides all workbench chrome around the editor.
        if !zen {
            egui::Panel::top("menu_bar")
                .frame(egui::Frame::new().fill(theme::colors().activity_bg).inner_margin(egui::Margin::symmetric(6, 3)))
                .show(ui, |ui| {
                    cmd = cmd.or(self.menu_bar(ui));
                });

            egui::Panel::bottom("status_bar")
                .frame(egui::Frame::new().fill(theme::colors().status_bg).inner_margin(egui::Margin::symmetric(8, 2)))
                .show_separator_line(false)
                .show(ui, |ui| self.status_bar(ui));

            egui::Panel::left("activity_bar")
                .exact_size(48.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::colors().activity_bg))
                .show(ui, |ui| {
                    cmd = cmd.or(self.activity_bar(ui));
                });
        }

        if self.sidebar_visible && !zen {
            // Never let the side bar take more than half the window, whatever it contains.
            let max_side = (ui.available_width() * 0.5).max(200.0);
            egui::Panel::left("side_bar")
                .default_size(260.0)
                .size_range(160.0..=max_side)
                .resizable(true)
                .frame(egui::Frame::new().fill(theme::colors().sidebar_bg).inner_margin(egui::Margin::symmetric(8, 6)))
                .show(ui, |ui| self.sidebar(ui));
        }

        if self.terminal_visible && !zen {
            egui::Panel::bottom("terminal")
                .default_size(220.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ui, |ui| self.terminal.ui(ui));
        }

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(theme::colors().editor_bg))
            .show(ui, |ui| {
                if self.docs.is_empty() {
                    cmd = cmd.or(self.welcome(ui));
                    return;
                }
                egui::Frame::new().fill(theme::colors().tab_inactive_bg).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.tab_bar(ui);
                });
                if !zen {
                    self.breadcrumbs(ui);
                }
                self.disk_conflict_bar(ui);
                self.find_bar(ui);
                let opts = editor_view::Options {
                    font_size: self.font_size,
                    word_wrap: self.word_wrap,
                    indent_guides: self.settings.editor.indent_guides,
                    rainbow_brackets: self.settings.editor.bracket_pair_colorization,
                };
                if let Some(doc) = self.docs.get_mut(self.active) {
                    let opts = editor_view::Options { word_wrap: opts.word_wrap && !doc.large, ..opts };
                    editor_view::show(ui, doc, opts);
                }
            });

        match self.palette.ui(&ctx, &mut self.finder, &self.keymap) {
            Some(PaletteResult::OpenFile(path)) => self.open_file(&path),
            Some(PaletteResult::Run(c)) => cmd = Some(c),
            Some(PaletteResult::Theme(label)) => self.set_color_theme(&ctx, &label),
            Some(PaletteResult::GoToLine(line)) if line > 0 => {
                if let Some(doc) = self.active_doc() {
                    doc.goto_line(line);
                }
            }
            _ => {}
        }

        self.close_dialog(&ctx);

        if let Some(c) = cmd {
            self.run_command(&ctx, c);
        }
        self.update_title(&ctx);
    }
}

// ----- editor widget -----------------------------------------------------------

/// Size for secondary workbench text (breadcrumbs, status bar): 12/13 of the UI font.
fn small_size(ui: &egui::Ui) -> f32 {
    egui::TextStyle::Body.resolve(ui.style()).size * 12.0 / 13.0
}

/// All matches of `query` in `text`, as `[start, end)` char index ranges.
fn find_matches(text: &str, query: &str, match_case: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, needle) = if match_case {
        (text.to_owned(), query.to_owned())
    } else {
        (text.to_lowercase(), query.to_lowercase())
    };
    // Lowercasing can change byte lengths for some scripts; fall back to exact matching then.
    let (hay, needle) = if hay.len() == text.len() { (hay, needle) } else { (text.to_owned(), query.to_owned()) };
    let needle_chars = needle.chars().count();
    let mut out = Vec::new();
    let mut last_byte = 0;
    let mut last_char = 0;
    for (byte, _) in hay.match_indices(&needle) {
        last_char += hay[last_byte..byte].chars().count();
        last_byte = byte;
        out.push((last_char, last_char + needle_chars));
        if out.len() >= 10_000 {
            break;
        }
    }
    out
}
