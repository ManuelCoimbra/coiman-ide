//! Main application: window layout, tabs, editor, shortcuts and dialogs.

use std::path::{Path, PathBuf};

use egui::{Align, Color32, FontId, Key, KeyboardShortcut, Modifiers, RichText, Sense, Stroke};

use crate::document::Document;
use crate::edit_ops::{self, CaseTransform, Edit, Sel};
use crate::editor_view::{self, editor_id};
use crate::explorer::{self, Explorer};
use crate::palette::{Command, Palette, PaletteResult};
use crate::search::SearchPanel;
use crate::session::{self, Session};
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
    /// Cached file list for Quick Open, rebuilt when the palette opens.
    file_index: Vec<PathBuf>,

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
    /// Set after Ctrl+K, for the Ctrl+K Ctrl+O chord.
    chord_k: bool,
    status: Option<String>,
    /// Last session JSON written, to skip unchanged backups.
    last_session_json: String,
    last_backup_time: f64,
    /// Window title last sent to the OS.
    last_title: String,
}

impl CodeApp {
    pub fn new(start_path: Option<PathBuf>) -> Self {
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
            file_index: Vec::new(),
            sidebar_visible: true,
            sidebar_view: SidebarView::Explorer,
            terminal_visible: false,
            font_size: DEFAULT_FONT_SIZE,
            word_wrap: false,
            zen_mode: false,
            closed_editors: Vec::new(),
            pending_close: None,
            allow_close: false,
            chord_k: false,
            status: None,
            last_session_json: String::new(),
            last_backup_time: 0.0,
            last_title: String::new(),
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
        app
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
        let json = session::to_json(&Session::capture(self.explorer.root.as_deref(), &self.docs, self.active));
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
            Ok(doc) => self.add_doc(doc),
            Err(e) => self.status = Some(format!("Could not open {}: {e}", path.display())),
        }
    }

    fn open_folder(&mut self, path: PathBuf) {
        self.terminal.set_cwd(&path);
        self.explorer.set_root(path);
        self.file_index.clear();
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Explorer;
    }

    fn save_doc(&mut self, index: usize) -> bool {
        let Some(doc) = self.docs.get_mut(index) else { return false };
        match doc.save() {
            Ok(true) => {
                self.status = Some(format!("Saved {}", doc.title()));
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
                self.file_index.clear();
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
            Command::ResetZoom => self.font_size = DEFAULT_FONT_SIZE,
            Command::RefreshExplorer => {
                self.explorer.refresh();
                self.file_index.clear();
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
            Command::ReopenClosedEditor => {
                if let Some(path) = self.closed_editors.pop() {
                    self.open_file(&path);
                }
            }
        }
    }

    fn open_palette(&mut self, prefix: &str) {
        if self.file_index.is_empty()
            && let Some(root) = &self.explorer.root
        {
            self.file_index = explorer::walk_files(root, 50_000);
        }
        self.palette.show_with(prefix);
    }

    // ----- input ---------------------------------------------------------------

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let ctrl = Modifiers::COMMAND;
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let ctrl_alt = Modifiers::COMMAND | Modifiers::ALT;
        let sc = |m, k| KeyboardShortcut::new(m, k);

        // Shortcuts with more modifiers must be checked first.
        let table: &[(KeyboardShortcut, Command)] = &[
            (sc(ctrl_shift, Key::S), Command::SaveAs),
            (sc(ctrl_alt, Key::S), Command::SaveAll),
            (sc(ctrl_shift, Key::E), Command::ShowExplorer),
            (sc(ctrl_shift, Key::F), Command::ShowSearch),
            (sc(ctrl_shift, Key::T), Command::ReopenClosedEditor),
            (sc(Modifiers::ALT, Key::Z), Command::ToggleWordWrap),
            (sc(ctrl, Key::S), Command::Save),
            (sc(ctrl, Key::N), Command::NewFile),
            (sc(ctrl, Key::W), Command::CloseTab),
            (sc(ctrl, Key::B), Command::ToggleSidebar),
            (sc(ctrl, Key::Backtick), Command::ToggleTerminal),
            (sc(ctrl, Key::F), Command::Find),
            (sc(ctrl, Key::G), Command::GoToLine),
            (sc(ctrl, Key::Equals), Command::ZoomIn),
            (sc(ctrl, Key::Plus), Command::ZoomIn),
            (sc(ctrl, Key::Minus), Command::ZoomOut),
            (sc(ctrl, Key::Num0), Command::ResetZoom),
        ];

        if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl_shift, Key::P))) {
            self.run_command(ctx, Command::CommandPalette);
            return;
        }
        // The chord stays armed across idle frames until the next key press.
        // egui reports modifiers separately, so any Key press is a real key.
        let key_pressed = ctx.input(|i| {
            i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
        });
        if self.chord_k && key_pressed {
            self.chord_k = false;
            let chord_cmd = if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl, Key::O))) {
                Some(Command::OpenFolder)
            } else if ctx.input_mut(|i| i.consume_shortcut(&sc(Modifiers::NONE, Key::Z))) {
                Some(Command::ToggleZenMode)
            } else {
                None
            };
            if let Some(c) = chord_cmd {
                // Drop the typed character so the chord key isn't inserted into the editor.
                ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Text(_))));
                self.run_command(ctx, c);
                return;
            }
        }
        if self.zen_mode
            && !self.find.open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.run_command(ctx, Command::ToggleZenMode);
            return;
        }

        // Editor actions only fire while the editor has focus ("editorTextFocus" in VS Code).
        // Variants with Shift come first because shortcut matching ignores extra Shift.
        if self.editor_focused(ctx) {
            let alt = Modifiers::ALT;
            let shift_alt = Modifiers::SHIFT | Modifiers::ALT;
            let editor_table: &[(KeyboardShortcut, Command)] = &[
                (sc(ctrl_alt, Key::ArrowUp), Command::AddCursorAbove),
                (sc(ctrl_alt, Key::ArrowDown), Command::AddCursorBelow),
                (sc(shift_alt, Key::ArrowUp), Command::CopyLinesUp),
                (sc(shift_alt, Key::ArrowDown), Command::CopyLinesDown),
                (sc(alt, Key::ArrowUp), Command::MoveLinesUp),
                (sc(alt, Key::ArrowDown), Command::MoveLinesDown),
                (sc(ctrl_shift, Key::K), Command::DeleteLines),
                (sc(ctrl_shift, Key::Backslash), Command::JumpToBracket),
                (sc(ctrl, Key::Slash), Command::ToggleLineComment),
                (sc(ctrl, Key::CloseBracket), Command::IndentLines),
                (sc(ctrl, Key::OpenBracket), Command::OutdentLines),
                (sc(ctrl, Key::D), Command::SelectNextOccurrence),
                (sc(Modifiers::SHIFT, Key::Tab), Command::OutdentLines),
            ];
            for (shortcut, cmd) in editor_table {
                if ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                    self.run_command(ctx, *cmd);
                    return;
                }
            }
            // Tab with a multi-line selection indents the lines instead of replacing them.
            let multi_line = self.docs.get(self.active).is_some_and(|d| {
                let p = d.buffer.primary();
                d.buffer.char_to_line(p.start()) != d.buffer.char_to_line(p.end())
            });
            if multi_line && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                self.run_command(ctx, Command::IndentLines);
                return;
            }
        }
        if !ctx.input(|i| i.modifiers.shift) && ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl, Key::K))) {
            self.chord_k = true;
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl, Key::O))) {
            self.run_command(ctx, Command::OpenFile);
            return;
        }
        if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl, Key::P))) {
            self.run_command(ctx, Command::QuickOpen);
            return;
        }
        // Ctrl+Tab / Ctrl+Shift+Tab cycle through tabs.
        if !self.docs.is_empty() {
            let n = self.docs.len();
            if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl_shift, Key::Tab))) {
                self.active = (self.active + n - 1) % n;
            } else if ctx.input_mut(|i| i.consume_shortcut(&sc(ctrl, Key::Tab))) {
                self.active = (self.active + 1) % n;
            }
        }
        for (shortcut, cmd) in table {
            if ctx.input_mut(|i| i.consume_shortcut(shortcut)) {
                self.run_command(ctx, *cmd);
                return;
            }
        }
    }

    /// Hot exit: on close, unsaved buffers go into the session backup instead of a
    /// "save changes?" prompt. The prompt is only a fallback if the backup fails.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        if !self.save_session() && self.docs.iter().any(Document::is_dirty) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending_close = Some(PendingClose::App);
        }
    }

    // ----- UI pieces -----------------------------------------------------------

    fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        egui::MenuBar::new().ui(ui, |ui| {
            let mut item = |ui: &mut egui::Ui, label: &str, keys: &str, c: Command| {
                if ui.add(egui::Button::new(label).shortcut_text(keys)).clicked() {
                    cmd = Some(c);
                }
            };
            ui.menu_button("File", |ui| {
                item(ui, "New Text File", "Ctrl+N", Command::NewFile);
                ui.separator();
                item(ui, "Open File...", "Ctrl+O", Command::OpenFile);
                item(ui, "Open Folder...", "Ctrl+K Ctrl+O", Command::OpenFolder);
                ui.separator();
                item(ui, "Save", "Ctrl+S", Command::Save);
                item(ui, "Save As...", "Ctrl+Shift+S", Command::SaveAs);
                item(ui, "Save All", "Ctrl+Alt+S", Command::SaveAll);
                ui.separator();
                item(ui, "Close Editor", "Ctrl+W", Command::CloseTab);
                item(ui, "Reopen Closed Editor", "Ctrl+Shift+T", Command::ReopenClosedEditor);
                item(ui, "Exit", "", Command::Quit);
            });
            ui.menu_button("Edit", |ui| {
                item(ui, "Find", "Ctrl+F", Command::Find);
                item(ui, "Find in Files", "Ctrl+Shift+F", Command::ShowSearch);
                ui.separator();
                item(ui, "Toggle Line Comment", "Ctrl+/", Command::ToggleLineComment);
            });
            ui.menu_button("Selection", |ui| {
                item(ui, "Copy Line Up", "Shift+Alt+Up", Command::CopyLinesUp);
                item(ui, "Copy Line Down", "Shift+Alt+Down", Command::CopyLinesDown);
                item(ui, "Move Line Up", "Alt+Up", Command::MoveLinesUp);
                item(ui, "Move Line Down", "Alt+Down", Command::MoveLinesDown);
                item(ui, "Duplicate Selection", "", Command::DuplicateSelection);
                ui.separator();
                item(ui, "Add Next Occurrence", "Ctrl+D", Command::SelectNextOccurrence);
                ui.separator();
                item(ui, "Sort Lines Ascending", "", Command::SortLinesAscending);
                item(ui, "Sort Lines Descending", "", Command::SortLinesDescending);
                item(ui, "Join Lines", "", Command::JoinLines);
            });
            ui.menu_button("View", |ui| {
                item(ui, "Command Palette...", "Ctrl+Shift+P", Command::CommandPalette);
                ui.separator();
                item(ui, "Explorer", "Ctrl+Shift+E", Command::ShowExplorer);
                item(ui, "Search", "Ctrl+Shift+F", Command::ShowSearch);
                item(ui, "Terminal", "Ctrl+`", Command::ToggleTerminal);
                item(ui, "Toggle Side Bar", "Ctrl+B", Command::ToggleSidebar);
                item(ui, "Zen Mode", "Ctrl+K Z", Command::ToggleZenMode);
                item(ui, "Word Wrap", "Alt+Z", Command::ToggleWordWrap);
                ui.separator();
                item(ui, "Zoom In", "Ctrl+=", Command::ZoomIn);
                item(ui, "Zoom Out", "Ctrl+-", Command::ZoomOut);
                item(ui, "Reset Zoom", "Ctrl+0", Command::ResetZoom);
            });
            ui.menu_button("Go", |ui| {
                item(ui, "Go to File...", "Ctrl+P", Command::QuickOpen);
                item(ui, "Go to Bracket", "Ctrl+Shift+\\", Command::JumpToBracket);
                item(ui, "Go to Line...", "Ctrl+G", Command::GoToLine);
            });
            ui.menu_button("Terminal", |ui| {
                item(ui, "Toggle Terminal", "Ctrl+`", Command::ToggleTerminal);
            });
        });
        cmd
    }

    fn activity_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        ui.add_space(6.0);
        let mut icon = |ui: &mut egui::Ui, glyph: &str, tip: &str, active: bool, c: Command| {
            let color = if active { Color32::WHITE } else { theme::TEXT_DIM };
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(48.0, 44.0), Sense::click());
            if active {
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                    0.0,
                    theme::ACCENT,
                );
            }
            let color = if resp.hovered() { Color32::WHITE } else { color };
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
                            self.file_index.clear();
                        }
                    });
                });
                if let Some(root) = &self.explorer.root {
                    let name = root.file_name().map_or_else(
                        || root.display().to_string(),
                        |n| n.to_string_lossy().to_uppercase(),
                    );
                    ui.label(RichText::new(name).strong().size(12.0));
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
                if let Some((path, line)) = self.search.ui(ui, self.explorer.root.as_deref()) {
                    self.open_file(&path);
                    if let Some(doc) = self.active_doc() {
                        doc.goto_line(line);
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
                        let fill = if active { theme::TAB_ACTIVE_BG } else { theme::TAB_INACTIVE_BG };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .inner_margin(egui::Margin::symmetric(10, 7))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let color = if active { Color32::WHITE } else { theme::TEXT_DIM };
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
                                        if doc.is_dirty() { theme::MODIFIED } else { theme::TEXT_DIM };
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
                            ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, theme::ACCENT));
                        } else {
                            let r = resp.rect;
                            ui.painter().vline(r.right(), r.y_range(), Stroke::new(1.0, theme::BORDER));
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
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("  ›  "),
            (Some(p), None) => p.display().to_string(),
            (None, _) => doc.title(),
        };
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(text).size(12.0).color(theme::TEXT_DIM));
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
                ui.label(RichText::new(count).small().color(theme::TEXT_DIM));
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
            ui.label(RichText::new("Rust Code").size(44.0).color(theme::TEXT_DIM));
            ui.label(RichText::new("Editing evolved, in Rust").size(16.0).color(theme::TEXT_DIM));
            ui.add_space(24.0);
            let rows: &[(&str, &str, Command)] = &[
                ("New File", "Ctrl+N", Command::NewFile),
                ("Open File...", "Ctrl+O", Command::OpenFile),
                ("Open Folder...", "Ctrl+K Ctrl+O", Command::OpenFolder),
                ("Show All Commands", "Ctrl+Shift+P", Command::CommandPalette),
                ("Go to File...", "Ctrl+P", Command::QuickOpen),
                ("Toggle Terminal", "Ctrl+`", Command::ToggleTerminal),
            ];
            // Grids don't center themselves, so pad to roughly center a ~300px block.
            ui.horizontal(|ui| {
                ui.add_space(((ui.available_width() - 300.0) / 2.0).max(0.0));
                egui::Grid::new("welcome").num_columns(2).spacing([40.0, 10.0]).show(ui, |ui| {
                    for (label, keys, c) in rows {
                        if ui.link(*label).clicked() {
                            cmd = Some(*c);
                        }
                        ui.label(RichText::new(*keys).monospace().color(theme::TEXT_DIM));
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
            let white = |s: String| RichText::new(s).size(12.0).color(Color32::WHITE);
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
                if (self.font_size - DEFAULT_FONT_SIZE).abs() > f32::EPSILON {
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
                ui.label(RichText::new("Your changes will be lost if you don't save them.").color(theme::TEXT_DIM));
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
        self.periodic_backup(&ctx);
        if self.pending_close.is_none() && !self.palette.open {
            self.handle_shortcuts(&ctx);
        }

        let mut cmd = None;

        let zen = self.zen_mode;
        // Zen mode hides all workbench chrome around the editor.
        if !zen {
            egui::Panel::top("menu_bar")
                .frame(egui::Frame::new().fill(theme::ACTIVITY_BG).inner_margin(egui::Margin::symmetric(6, 3)))
                .show(ui, |ui| {
                    cmd = cmd.or(self.menu_bar(ui));
                });

            egui::Panel::bottom("status_bar")
                .frame(egui::Frame::new().fill(theme::STATUS_BG).inner_margin(egui::Margin::symmetric(8, 2)))
                .show_separator_line(false)
                .show(ui, |ui| self.status_bar(ui));

            egui::Panel::left("activity_bar")
                .exact_size(48.0)
                .resizable(false)
                .frame(egui::Frame::new().fill(theme::ACTIVITY_BG))
                .show(ui, |ui| {
                    cmd = cmd.or(self.activity_bar(ui));
                });
        }

        if self.sidebar_visible && !zen {
            egui::Panel::left("side_bar")
                .default_size(260.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(theme::SIDEBAR_BG).inner_margin(egui::Margin::symmetric(8, 6)))
                .show(ui, |ui| self.sidebar(ui));
        }

        if self.terminal_visible && !zen {
            egui::Panel::bottom("terminal")
                .default_size(220.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(theme::EDITOR_BG).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ui, |ui| self.terminal.ui(ui));
        }

        egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(theme::EDITOR_BG))
            .show(ui, |ui| {
                if self.docs.is_empty() {
                    cmd = cmd.or(self.welcome(ui));
                    return;
                }
                egui::Frame::new().fill(theme::TAB_INACTIVE_BG).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.tab_bar(ui);
                });
                if !zen {
                    self.breadcrumbs(ui);
                }
                self.find_bar(ui);
                let (font_size, word_wrap) = (self.font_size, self.word_wrap);
                if let Some(doc) = self.docs.get_mut(self.active) {
                    editor_view::show(ui, doc, font_size, word_wrap && !doc.large);
                }
            });

        let root = self.explorer.root.clone();
        match self.palette.ui(&ctx, root.as_deref(), &self.file_index) {
            Some(PaletteResult::OpenFile(path)) => self.open_file(&path),
            Some(PaletteResult::Run(c)) => cmd = Some(c),
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
