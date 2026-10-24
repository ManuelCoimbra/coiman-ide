//! Main application state and the per-frame loop. The behaviour is split by
//! area into submodules that each add methods to [`CodeApp`].

mod config;
mod dispatch;
mod files;
mod workbench;

use std::path::PathBuf;

use egui::KeyboardShortcut;

use crate::document::Document;
use crate::editor_view;
use crate::explorer::Explorer;
use crate::finder::FileFinder;
use crate::keymap::Keymap;
use crate::palette::{Palette, PaletteResult};
use crate::search_panel::SearchPanel;
use crate::session;
use crate::settings::Settings;
use crate::syntax::SyntaxWorker;
use crate::terminal::{Terminal, strip_verbatim};
use crate::theme;
use crate::watcher::FileWatcher;

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

/// Find-bar matches and the inputs they were computed for.
struct FindCache {
    uid: u64,
    version: u64,
    query: String,
    match_case: bool,
    matches: Vec<(usize, usize)>,
}

#[derive(Default)]
struct FindBar {
    open: bool,
    query: String,
    match_case: bool,
    focus_requested: bool,
    /// Matches for (doc uid, buffer version, query, match case), recomputed only on change.
    cache: Option<FindCache>,
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
    /// Editor view state (scroll, wrap layout, IME) per document uid.
    views: std::collections::HashMap<u64, editor_view::ViewState>,
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
            views: std::collections::HashMap::new(),
            finder: FileFinder::new(repaint(ctx)),
            watcher: FileWatcher::new(repaint(ctx)),
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

        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(theme::colors().editor_bg)).show(ui, |ui| {
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
                editor_view::show(ui, doc, self.views.entry(doc.uid).or_default(), opts);
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

/// A callback that wakes the UI from a background thread.
fn repaint(ctx: &egui::Context) -> impl Fn() + Send + Sync + 'static {
    let ctx = ctx.clone();
    move || ctx.request_repaint()
}
