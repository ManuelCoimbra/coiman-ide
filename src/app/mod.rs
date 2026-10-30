//! Main application state and the per-frame loop. The behaviour is split by
//! area into submodules that each add methods to [`CodeApp`].

mod cargo;
mod config;
mod debug;
mod dispatch;
mod files;
mod git;
mod history;
mod lsp;
mod lsp_extra;
mod outline;
mod plugins;
mod workbench;

use std::path::PathBuf;

use egui::KeyboardShortcut;

use crate::document::Document;
use crate::editor_view;
use crate::explorer::Explorer;
use crate::finder::FileFinder;
use crate::keymap::Keymap;
use crate::palette::{InputKind, Palette, PaletteResult};
use crate::search_panel::SearchPanel;
use crate::session;
use crate::settings::Settings;
use crate::syntax::SyntaxWorker;
use crate::terminal::{Terminals, strip_verbatim};
use crate::theme;
use crate::watcher::FileWatcher;

const DEFAULT_FONT_SIZE: f32 = 14.0;
/// Seconds between crash-recovery session backups.
const BACKUP_INTERVAL: f64 = 1.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarView {
    Explorer,
    Search,
    Git,
    Testing,
    Debug,
    Extensions,
}

/// Tabs of the bottom panel.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PanelTab {
    Problems,
    References,
    Output,
    DebugConsole,
    Terminal,
    /// A panel contributed by a plugin (index into its panels).
    Plugin(usize),
}

enum PendingClose {
    Tab(usize),
    App,
}

/// Find-bar matches and the inputs they were computed for.
struct FindCache {
    uid: u64,
    version: u64,
    options: rust_code_core::find::FindOptions,
    matches: Vec<(usize, usize)>,
    /// The regular expression doesn't compile.
    error: Option<String>,
}

#[derive(Default)]
struct FindBar {
    open: bool,
    options: rust_code_core::find::FindOptions,
    show_replace: bool,
    replacement: String,
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
    terminal: Terminals,
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
    /// Bottom panel (Problems, References, Terminal).
    panel_visible: bool,
    panel_tab: PanelTab,
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
    /// Set by `--measure-startup`: process start, time the window was ready,
    /// and where to write the timings.
    pub(crate) startup_probe: Option<(std::time::Instant, std::time::Duration, PathBuf)>,
    /// Background tree-sitter parser, started on the first frame.
    syntax: Option<SyntaxWorker>,
    /// Language servers and the state of their features.
    lsp: lsp::LspState,
    /// Git status, gutter, blame, diff and Source Control state.
    git: git::GitState,
    /// Run configurations, tests and cargo check results.
    cargo: cargo::CargoState,
    /// Labels and lines offered by Go to Symbol.
    symbol_pick: Vec<(String, usize)>,
    /// Selections to return to with Shrink Selection.
    selection_stack: outline::SelectionStack,
    /// Whether the Outline section of the Explorer is open.
    outline_open: bool,
    /// Local history: versions of files kept on save.
    history: history::HistoryState,
    /// Whether the Timeline section of the Explorer is open.
    timeline_open: bool,
    /// Vim or Helix key handling (`editor.keyboard_mode`), and the document it last saw.
    vim: Option<rust_code_core::vim::Vim>,
    vim_doc: Option<u64>,
    /// Debugger sessions, breakpoints, watches.
    debug: debug::DebugState,
    /// Installed plugins and what they contribute.
    plugins: plugins::PluginState,
    /// "Stay Restricted" was chosen: no bar until something needs trust again.
    trust_prompt_hidden: bool,
    /// Where the Explorer's next new file, folder or rename applies.
    path_target: Option<PathBuf>,
    /// Asking before moving this to the Recycle Bin.
    delete_pending: Option<PathBuf>,
}
impl CodeApp {
    pub fn new(start_path: Option<PathBuf>, ctx: &egui::Context) -> Self {
        // Before anything reads a folder's settings or runs a tool for it.
        rust_code_core::trust::enforce();
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let mut app = Self {
            docs: Vec::new(),
            active: 0,
            next_uid: 1,
            untitled_counter: 0,
            explorer: Explorer::default(),
            search: SearchPanel::default(),
            terminal: Terminals::new(cwd),
            palette: Palette::default(),
            find: FindBar::default(),
            views: std::collections::HashMap::new(),
            finder: FileFinder::new(repaint(ctx)),
            watcher: FileWatcher::new(repaint(ctx)),
            sidebar_visible: true,
            sidebar_view: SidebarView::Explorer,
            panel_visible: false,
            panel_tab: PanelTab::Terminal,
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
            startup_probe: None,
            syntax: None,
            lsp: lsp::LspState::default(),
            git: git::GitState::default(),
            cargo: cargo::CargoState::default(),
            symbol_pick: Vec::new(),
            selection_stack: None,
            outline_open: true,
            history: history::HistoryState::default(),
            timeline_open: false,
            vim: None,
            vim_doc: None,
            debug: debug::DebugState::default(),
            plugins: plugins::PluginState::default(),
            trust_prompt_hidden: false,
            path_target: None,
            delete_pending: None,
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
        self.pump_watcher(&ctx);
        self.pump_lsp(&ctx);
        self.pump_git(&ctx);
        self.pump_cargo(&ctx);
        self.pump_debug(&ctx);
        self.pump_plugins(&ctx);
        // The last terminal's shell exited: close the panel, as VS Code does.
        if self.terminal.pump(&ctx) {
            if self.panel_tab == PanelTab::Terminal {
                self.panel_visible = false;
            }
            self.terminal.detached = false;
        }
        if self.pending_close.is_none() && !self.palette.open {
            if self.editor_focused(&ctx) {
                self.lsp_keys(&ctx);
            }
            self.handle_shortcuts(&ctx);
        }

        let mut cmd = None;

        let zen = self.zen_mode;
        self.window_resize_edges(&ctx);
        // The title bar stays in Zen mode: it holds the window buttons.
        egui::Panel::top("title_bar").frame(egui::Frame::new().fill(theme::colors().activity_bg)).show(ui, |ui| {
            cmd = cmd.or(self.title_bar(ui));
        });
        if !self.trust_prompt_hidden && self.restricted_folder().is_some() {
            egui::Panel::top("trust_bar").frame(egui::Frame::NONE).show(ui, |ui| self.trust_bar(ui));
        }
        // Zen mode hides all workbench chrome around the editor.
        if !zen {
            egui::Panel::bottom("status_bar")
                .frame(egui::Frame::new().fill(theme::colors().status_bg).inner_margin(egui::Margin::symmetric(8, 2)))
                .show_separator_line(false)
                .show(ui, |ui| {
                    cmd = cmd.or(self.status_bar(ui));
                });

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

        if self.panel_visible && !zen {
            egui::Panel::bottom("panel")
                .default_size(220.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(egui::Margin::symmetric(12, 6)))
                .show(ui, |ui| self.panel(ui));
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
            self.lsp_offer_bar(ui);
            self.find_bar(ui);
            let opts = editor_view::Options {
                font_size: self.font_size,
                word_wrap: self.word_wrap,
                indent_guides: self.settings.editor.indent_guides,
                rainbow_brackets: self.settings.editor.bracket_pair_colorization,
                block_cursor: false,
            };
            if self.git.diff.is_some() {
                self.diff_view(ui);
                return;
            }
            let mut deco = self.lsp_decorations(self.active);
            self.git_decorate(self.active, &mut deco);
            self.cargo_decorate(self.active, &mut deco);
            self.debug_decorate(self.active, &mut deco);
            // Plugins last: their colors go over the server's.
            self.lsp_views_decorate(self.active, &mut deco);
            self.plugin_decorate(self.active, &mut deco);
            if let Some(doc) = self.docs.get_mut(self.active) {
                deco.folds = doc.structure().folds.clone();
            }
            // Text typed this frame (the editor reads events without consuming them).
            let typed: String = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| if let egui::Event::Text(t) = e { Some(t.as_str()) } else { None })
                    .collect()
            });
            if let Some(doc) = self.docs.get_mut(self.active) {
                // Switching files starts Vim afresh in normal mode on the new cursor.
                if let Some(vim) = &mut self.vim
                    && self.vim_doc != Some(doc.uid)
                {
                    vim.reset(&mut doc.buffer);
                    self.vim_doc = Some(doc.uid);
                }
                let block_cursor = self.vim.as_ref().is_some_and(rust_code_core::vim::Vim::block_cursor);
                let was_typing = self.vim.as_ref().is_none_or(|v| v.mode() == rust_code_core::vim::Mode::Insert);
                let opts = editor_view::Options { word_wrap: opts.word_wrap && !doc.large, block_cursor, ..opts };
                let mut out =
                    editor_view::show(ui, doc, self.views.entry(doc.uid).or_default(), opts, &deco, self.vim.as_mut());
                for effect in std::mem::take(&mut out.vim_effects) {
                    self.vim_effect(&ctx, effect);
                }
                if let Some((line, pos)) = out.change_click {
                    self.open_hunk_popup(line, pos);
                }
                if let Some(line) = out.run_click {
                    self.run_test_at_line(&ctx, line);
                }
                if let Some(line) = out.breakpoint_click {
                    self.toggle_breakpoint(line);
                }
                // Vim commands arrive as text too; only insert-mode typing drives completion.
                let typing =
                    was_typing && self.vim.as_ref().is_none_or(|v| v.mode() == rust_code_core::vim::Mode::Insert);
                if !typing {
                    self.close_completion();
                }
                self.after_editor(&ctx, out, if typing { &typed } else { "" });
            }
        });
        self.lsp_popups(&ctx);
        self.git_popups(&ctx);
        self.terminal_window(&ctx);

        if self.palette.open {
            self.palette.plugin_commands = self.plugin_commands();
        }
        self.pump_workspace_symbols();
        match self.palette.ui(&ctx, &mut self.finder, &self.keymap) {
            Some(PaletteResult::OpenFile(path)) => self.open_file(&path),
            Some(PaletteResult::Run(c)) => cmd = Some(c),
            Some(PaletteResult::WorkspaceSymbol(i)) => self.picked_workspace_symbol(i),
            Some(PaletteResult::RunPlugin(key)) => self.run_plugin_command(&key),
            Some(PaletteResult::Picked(kind, label)) => self.handle_pick(&ctx, kind, &label),
            Some(PaletteResult::Input(InputKind::Rename, name)) => self.finish_rename(&name),
            Some(PaletteResult::Input(InputKind::NewFile, name)) => self.finish_new_path(&name, false),
            Some(PaletteResult::Input(InputKind::NewFolder, name)) => self.finish_new_path(&name, true),
            Some(PaletteResult::Input(InputKind::RenamePath, name)) => self.finish_rename_path(&name),
            Some(PaletteResult::Input(InputKind::NewBranch, name)) => self.switch_branch(&ctx, name, true),
            Some(PaletteResult::Input(InputKind::BreakpointCondition, text)) => {
                self.finish_breakpoint_edit(false, &text)
            }
            Some(PaletteResult::Input(InputKind::Logpoint, text)) => self.finish_breakpoint_edit(true, &text),
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
        if let Some((started, window_ready, file)) = self.startup_probe.take() {
            // "<ms until window ready> <ms until first frame>"
            let _ = std::fs::write(file, format!("{} {}", window_ready.as_millis(), started.elapsed().as_millis()));
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

// ----- editor widget -----------------------------------------------------------

/// A callback that wakes the UI from a background thread.
fn repaint(ctx: &egui::Context) -> impl Fn() + Send + Sync + 'static {
    let ctx = ctx.clone();
    move || ctx.request_repaint()
}
