//! Main application state and the per-frame loop. The behaviour is split by
//! area into submodules that each add methods to [`CodeApp`].

mod autosave;
mod bookmarks;
mod bytes_view;
mod cargo;
mod config;
mod coverage;
mod cpp;
mod debug;
mod dispatch;
mod doctor;
mod environment;
mod files;
mod git;
mod hidden;
mod history;
mod keys_ui;
mod logs;
mod lsp;
mod lsp_extra;
mod lsp_room;
mod md_preview;
mod outline;
mod plugins;
mod preview;
mod prints;
mod profile;
mod repl;
mod settings_ui;
mod snippets_ui;
mod split;
mod tables;
mod test_panel;
mod traces;
mod web;
mod window;
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
    Prints,
    Terminal,
    /// What the live preview's page printed.
    PageConsole,
    /// Files by how much of them ran.
    Coverage,
    /// Stack Trace: Analyze.
    StackTrace,
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
    /// Scale of the whole window, ahead of the settings file while `zoom_save` is due.
    zoom: f32,
    /// Terminal font size; 0 follows `font_size`.
    terminal_font_size: f32,
    zoom_save: Option<std::time::Instant>,
    word_wrap: bool,
    zen_mode: bool,
    /// Paths of closed editors, most recent last, for Reopen Closed Editor.
    closed_editors: Vec<PathBuf>,
    /// The second editor pane, when split.
    split: Option<split::Split>,
    /// Tab being dragged.
    tab_drag: Option<u64>,
    bookmarks: bookmarks::BookmarkState,
    prints: prints::Prints,
    preview: preview::Preview,
    doctor: doctor::Doctor,
    repls: repl::Repls,
    environment: environment::Environment,
    snippets_ui: snippets_ui::SnippetsUi,
    md_preview: md_preview::MdPreview,
    web: web::Web,
    coverage: coverage::Coverage,
    bytes: bytes_view::BytesState,
    logs: logs::Logs,
    /// A file that asked for the log view, on its way there.
    log_to_open: Option<std::path::PathBuf>,
    tests: test_panel::Tests,
    tables: tables::Tables,
    traces: traces::Traces,
    cpp: cpp::Cpp,
    auto_save: autosave::AutoSave,
    /// A font being loaded in the background: what it is, or why it can't be.
    font_load: Option<std::sync::mpsc::Receiver<rust_code_core::fonts::Loaded>>,
    /// The chosen font for code, behind which the built-in fonts stay.
    editor_font: Option<std::sync::Arc<egui::FontData>>,
    /// Behind them all, for symbols, once a file has called for it.
    symbol_font: Option<std::sync::Arc<egui::FontData>>,
    symbol_font_tried: bool,
    settings_ui: settings_ui::SettingsUi,
    keys_ui: keys_ui::KeysUi,
    window: Option<rust_code_core::session::WindowState>,
    window_fitted: bool,
    /// Indentation picked from the status bar, by document.
    indent_chosen: std::collections::HashMap<u64, crate::buffer::Indent>,
    /// Whether the open folder or active file belongs to a Cargo project.
    cargo_project: bool,
    /// What `cargo_project` was worked out for, and when to check again.
    cargo_project_for: (Option<PathBuf>, f64),
    hidden: hidden::HiddenChars,
    /// Folders for Open Recent, newest first.
    recent_folders: Vec<PathBuf>,

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
    /// The palette just closed: give the editor focus unless something else took it.
    refocus_after_palette: bool,
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
            zoom: rust_code_core::settings::DEFAULT_ZOOM,
            terminal_font_size: 0.0,
            zoom_save: None,
            word_wrap: false,
            zen_mode: false,
            closed_editors: Vec::new(),
            split: None,
            tab_drag: None,
            bookmarks: Default::default(),
            prints: Default::default(),
            preview: Default::default(),
            doctor: Default::default(),
            repls: Default::default(),
            environment: Default::default(),
            snippets_ui: Default::default(),
            md_preview: Default::default(),
            web: Default::default(),
            coverage: Default::default(),
            bytes: Default::default(),
            logs: Default::default(),
            log_to_open: None,
            tests: Default::default(),
            tables: Default::default(),
            traces: Default::default(),
            cpp: Default::default(),
            auto_save: Default::default(),
            font_load: None,
            editor_font: None,
            symbol_font: None,
            symbol_font_tried: false,
            settings_ui: Default::default(),
            keys_ui: Default::default(),
            window: None,
            window_fitted: false,
            indent_chosen: Default::default(),
            cargo_project: false,
            cargo_project_for: (None, 0.0),
            hidden: Default::default(),
            recent_folders: rust_code_core::recent::load(),
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
            refocus_after_palette: false,
            path_target: None,
            delete_pending: None,
        };
        // Always restore the previous session first so unsaved work is never
        // dropped; a path given on the command line then opens on top of it.
        if let Some(saved) = session::load() {
            app.window = saved.window;
            app.cargo.run_pins = saved.run_pins.clone();
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
        self.track_window(&ctx);
        self.pump_environment(&ctx);
        self.pump_run_configs(&ctx);
        self.pump_run_problems(&ctx);
        self.pump_preview(&ctx);
        self.check_baseline();
        self.pump_cpp(&ctx);
        self.pump_repl(&ctx);
        self.pump_run_preview(&ctx);
        self.pump_zoom(&ctx);
        self.pump_bookmarks();
        self.pump_prints(&ctx);
        self.pump_auto_save(&ctx);
        self.pump_font_load(&ctx);
        self.pump_symbol_font(&ctx);
        self.refresh_cargo_project(&ctx);
        self.pump_tests(&ctx);
        self.pump_coverage(&ctx);
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
        if self.pending_close.is_none() && !self.palette.open && !self.keys_ui.capturing() {
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
                if self.git.diff.is_some() {
                    self.diff_view(ui);
                    return;
                }
                if let Some(path) = self.log_to_open.take() {
                    self.open_log_view(ui.ctx(), &path);
                }
                if self.log_view(ui) || self.bytes_view(ui) || self.image_view(ui) {
                    return;
                }
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
            self.hidden_bar(ui);
            self.lock_bar(ui);
            self.lsp_offer_bar(ui);
            self.find_bar(ui);
            self.glyph_bar(ui);
            let opts = editor_view::Options {
                font_size: self.font_size,
                word_wrap: self.word_wrap,
                indent_guides: self.settings.editor.indent_guides,
                rainbow_brackets: self.settings.editor.bracket_pair_colorization,
                block_cursor: false,
                smooth_scrolling: self.settings.editor.smooth_scrolling,
                pane: 0,
                active: true,
            };
            if self.git.diff.is_some() {
                self.diff_view(ui);
                return;
            }
            if let Some(path) = self.log_to_open.take() {
                self.open_log_view(ui.ctx(), &path);
            }
            if self.log_view(ui) || self.bytes_view(ui) || self.image_view(ui) || self.table_view(ui) {
                return;
            }
            self.markdown_preview(ui);
            let (active_rect, other_rect) = self.split_rects(ui);
            let mut active_ui = ui.new_child(egui::UiBuilder::new().max_rect(active_rect));
            let pane = self.active_pane();
            let deco = self.decorations(self.active);
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
                // Columns mean nothing once lines wrap.
                let wrap = opts.word_wrap && !doc.large && deco.dim_columns.is_empty();
                let opts = editor_view::Options { word_wrap: wrap, block_cursor, pane, ..opts };
                let mut out = editor_view::show(
                    &mut active_ui,
                    doc,
                    self.views.entry(doc.uid).or_default(),
                    opts,
                    &deco,
                    self.vim.as_mut(),
                );
                for effect in std::mem::take(&mut out.vim_effects) {
                    self.vim_effect(&ctx, effect);
                }
                if let Some((line, pos)) = out.change_click {
                    self.open_hunk_popup(line, pos);
                }
                if let Some(line) = out.run_click {
                    self.run_test_at_line(&ctx, line);
                }
                if let Some(line) = out.breakpoint_click.filter(|_| self.can_debug()) {
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
            if let Some(rect) = other_rect {
                self.other_pane(ui, rect, opts);
            }
        });
        self.lsp_popups(&ctx);
        self.lsp_room(&ctx);
        self.doctor_window(&ctx);
        self.settings_window(&ctx);
        self.keys_window(&ctx);
        self.snippets_window(&ctx);
        self.web_windows(&ctx);
        self.cpp_window(&ctx);
        self.prints_window(&ctx);
        self.git_popups(&ctx);
        self.terminal_window(&ctx);

        if self.palette.open {
            self.palette.plugin_commands = self.plugin_commands();
            self.palette.hidden_commands = if self.cargo_project {
                &[]
            } else if self.can_debug() {
                cargo::TESTING_ONLY
            } else {
                cargo::CARGO_ONLY
            };
        }
        self.pump_workspace_symbols();
        if std::mem::take(&mut self.refocus_after_palette)
            && ctx.memory(|m| m.focused().is_none())
            && let Some(doc) = self.docs.get_mut(self.active)
        {
            doc.request_focus = true;
            ctx.request_repaint();
        }
        let palette_was_open = self.palette.open;
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
        self.run_refusal_ui(&ctx);

        if let Some(c) = cmd {
            self.run_command(&ctx, c);
        }
        // After the command, which may have opened the palette again as a picker.
        if palette_was_open && !self.palette.open {
            ctx.memory_mut(|m| m.stop_text_input());
            self.refocus_after_palette = true;
            ctx.request_repaint();
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
