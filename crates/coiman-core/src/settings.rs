//! Layered TOML settings.
//!
//! Layers, lowest to highest priority:
//! 1. built-in defaults,
//! 2. user settings: `<config dir>/settings.toml`,
//! 3. workspace settings: `<folder>/.coiman/settings.toml` (or `.rust-code`, from before).
//!
//! Each file may contain `extends = "other.toml"` (relative to the file) to
//! inherit from another file, and `[windows]` / `[macos]` / `[linux]` tables
//! whose contents apply only on that platform.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::{Table, Value};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct Settings {
    pub editor: EditorSettings,
    pub workbench: WorkbenchSettings,
    pub files: FilesSettings,
    pub lsp: LspSettings,
    pub terminal: TerminalSettings,
    pub git: GitSettings,
    pub cargo: CargoSettings,
    pub history: HistorySettings,
    pub debug: DebugSettings,
    pub plugins: PluginSettings,
    pub python: PythonSettings,
    pub web: WebSettings,
    pub cpp: CppSettings,
    /// More backslash names: `to = "⟶"`. Yours win over the shipped ones.
    pub symbols: std::collections::HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct CppSettings {
    /// The folder of the `compile_commands.json` clangd reads, relative to the
    /// project. Empty leaves it to clangd.
    pub compile_commands_dir: String,
    /// In a database of several configurations, the one to read.
    pub config: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct WebSettings {
    /// Which browsers a project promises: "widely", "newly" or a year such as "2023".
    /// Empty: only say what's available, and don't flag anything.
    pub baseline: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct PythonSettings {
    /// An environment's folder or a Python, relative to the folder; empty works it out.
    pub interpreter: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct EditorSettings {
    /// A fixed-width font installed on the system, or a path to a font file.
    /// Empty uses the built-in font.
    pub font_family: String,
    pub font_size: f32,
    pub word_wrap: bool,
    pub indent_guides: bool,
    /// Brighten the indent guide of the block holding the cursor
    /// (`editor.guides.highlightActiveIndentation`).
    pub highlight_active_indentation: bool,
    pub bracket_pair_colorization: bool,
    /// Mark the matching start or end tag when the cursor is in a tag's name.
    pub matching_tags: bool,
    /// A line in the bracket's color from the pair around the cursor to its closer
    /// (`editor.guides.bracketPairs: active`).
    pub bracket_pair_guides: bool,
    /// Format the file with its language server when saving.
    pub format_on_save: bool,
    /// Warn about loops that look endless and memory that looks leaked.
    pub likely_problems: bool,
    /// Check HTML and XML for tags left open, misnested or broken.
    pub markup_problems: bool,
    /// Show colors written in a file on themselves (`editor.colorDecorators`).
    pub color_decorators: bool,
    /// Show font names in style sheets in their own font, with a card of samples on hover.
    pub font_previews: bool,
    /// Enter after `/**` above a function writes a Doxygen comment for it.
    pub doc_comment_templates: bool,
    /// Type a tag's end for it in HTML, XML and JSX (`html.autoClosingTags`).
    pub auto_close_tags: bool,
    /// `=` after an attribute's name adds `""` (`html.autoCreateQuotes`).
    pub auto_create_quotes: bool,
    /// Renaming a tag renames its partner (`editor.linkedEditing`, Auto Rename Tag).
    pub rename_tags: bool,
    /// File and folder names while a path is typed in a string (Path Intellisense).
    pub path_completion: bool,
    /// Columns per indent level (`.editorconfig` and detection take priority).
    pub tab_size: usize,
    /// Indent with spaces rather than tabs.
    pub insert_spaces: bool,
    /// Follow how each file is already indented.
    pub detect_indentation: bool,
    /// Type brackets and quotes in pairs.
    pub auto_close_brackets: bool,
    /// Color symbols by what the language server says they are.
    pub semantic_highlighting: bool,
    /// Show inferred types and parameter names in the text.
    pub inlay_hints: bool,
    /// Columns to draw a line after, in languages whose profile names none.
    pub rulers: Vec<usize>,
    /// Show a bar of glyphs for languages whose profile has them.
    pub glyph_bar: bool,
    /// More folders to look for included files in, from the project's folder.
    pub include_paths: Vec<String>,
    /// Color each column of CSV and TSV files in the text.
    pub csv_colors: bool,
    /// Tab grows Emmet abbreviations in HTML and CSS.
    pub emmet: bool,
    /// Show what servers report about a line at its end, such as reference counts.
    pub code_lens: bool,
    /// Glide to the new position when keys move the view, instead of jumping.
    pub smooth_scrolling: bool,
    /// Mark bidi controls and invisible or private use characters, and warn about them.
    pub unicode_warnings: bool,
    pub trim_trailing_whitespace: bool,
    pub insert_final_newline: bool,
    /// Key handling: "default", "vim" (modal, Vim keys) or "helix" (select, then act).
    pub keyboard_mode: String,
}

/// Window scale when none is set. At 100% the interface reads small on a 1080p screen.
pub const DEFAULT_ZOOM: f32 = 1.25;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct WorkbenchSettings {
    /// Font size of menus, side bar, tabs and status bar (independent of the editor).
    pub ui_font_size: f32,
    /// Scale of the whole window: text, icons, panels and spacing. 1.0 is 100%.
    pub zoom: f32,
    /// Port of the live preview on this computer; 0 picks a free one.
    pub preview_port: u16,
    /// A theme name from the Color Theme picker, or a path to a VS Code theme `.json`.
    pub color_theme: String,
    /// A picture, or a folder of them, laid faintly over the window. Empty: none.
    pub background_image: String,
    /// Pictures for one part of the window, over the main one.
    pub background_editor: String,
    pub background_sidebar: String,
    pub background_panel: String,
    /// Go through a folder in no order.
    pub background_random: bool,
    /// How strongly it shows, from 0 to 1.
    pub background_opacity: f32,
    /// Over the "window", or only the "editor".
    pub background_area: String,
    /// Seconds between pictures of a folder; 0 keeps the first.
    pub background_interval: u32,
    /// Where this computer is, `38.7, -9.1`, for sunrise and sunset in the schedule.
    pub theme_location: String,
    /// Themes by time of day: `07:00 Light Modern; sunset Dark Modern`.
    pub theme_schedule: String,
    /// Panels, tabs and popups change at once instead of moving.
    pub reduced_motion: bool,
    /// Pressing Shift twice opens Search Everywhere.
    pub double_shift: bool,
    /// Search Everywhere's box in the middle of the top bar; off, a magnifier on the right.
    pub search_in_top_bar: bool,
    /// Most frames a second while something moves (scrolling output, animations); 0
    /// follows the display. The window draws nothing while nothing changes either way.
    pub max_fps: u32,
    /// The window's layout: "dock" (tools in a dock at the bottom, opening as sheets over the
    /// code), "islands", or "classic" for square panels edge to edge.
    pub ui: String,
    /// Explorer: show a chain of packages holding only each other as one row, `org.example`.
    pub compact_folders: bool,
    /// Explorer: show `a.test.ts`, `a.js` and `a.css` under `a.ts`.
    pub nest_files: bool,
    /// Explorer: how a folder's entries are ordered (`explorer.sortOrder`): "default"
    /// (folders first), "mixed", "files_first", "type" or "modified".
    pub explorer_sort_order: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct FilesSettings {
    /// Restore unsaved changes on the next launch instead of asking on exit.
    pub hot_exit: bool,
    /// Save without asking: "off", "after_delay" or "focus_change".
    pub auto_save: String,
    /// How long after the last edit "after_delay" saves.
    pub auto_save_delay_ms: u64,
    /// Line ending for new files: "auto" (CRLF on Windows, LF elsewhere), "lf" or "crlf".
    pub eol: String,
    /// Encoding for new files (VS Code ids: "utf8", "utf8bom", "utf16le", "windows1252", ...).
    pub encoding: String,
    /// `.log` files from this many MiB open in the log view. 0 never does.
    pub log_view_log_mb: u64,
    /// Any file from this many MiB opens in the log view. 0 never does.
    pub log_view_any_mb: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct LspSettings {
    /// Language features from language servers (rust-analyzer for Rust).
    pub enabled: bool,
    /// Path to rust-analyzer; empty finds it on PATH or in ~/.cargo/bin.
    pub rust_analyzer_path: String,
    /// Offer to install a missing language server (rust-analyzer, pyright…).
    pub ask_to_install: bool,
    /// How much rust-analyzer may keep in memory: "default", "low" (fewer
    /// cached trees, no warm-up on load) or "minimal" (also no build scripts or
    /// proc macros, so code that depends on them shows false errors).
    pub rust_analyzer_memory: String,
    /// Give rust-analyzer its own build folder inside the target folder, so its
    /// checks don't make cargo wait for the lock. It duplicates build output.
    pub rust_analyzer_separate_target: bool,
    /// Commands for language servers, by language: `python`, `typescript`,
    /// `clangd`, `java`, `csharp`, `go` (and `rust`). A program path,
    /// optionally followed by arguments; empty finds the server itself.
    pub paths: std::collections::BTreeMap<String, String>,
    /// Run the language servers that come in VS Code extensions (ESLint, Tailwind CSS)
    /// beside a file's own server, when the extension is installed and the project uses
    /// the tool.
    pub extension_servers: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TerminalSettings {
    /// Shell for new terminals, with arguments (e.g. "pwsh -NoLogo"); empty:
    /// PowerShell on Windows, $SHELL elsewhere.
    pub shell: String,
    /// Lines of scrollback kept per terminal.
    pub scrollback: usize,
    /// Font size of the terminal; 0 follows `editor.font_size`.
    pub font_size: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GitSettings {
    /// Source control features (needs `git` on PATH).
    pub enabled: bool,
    /// Show who last changed the cursor line, after its end.
    pub blame_inline: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct CargoSettings {
    /// What rust-analyzer runs on save for errors and warnings: "check" or "clippy".
    pub check_command: String,
    /// Test runner: "auto" (nextest when installed), "cargo" or "nextest".
    pub test_runner: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct HistorySettings {
    /// Keep a copy of a file every time it is saved or changed outside.
    pub enabled: bool,
    /// Versions kept per file.
    pub max_entries: usize,
    /// Versions older than this are dropped.
    pub max_days: u64,
    /// Larger files are not kept.
    pub max_file_kb: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct DebugSettings {
    /// Path to CodeLLDB (the adapter executable or its extension folder);
    /// empty finds Coiman IDE's own install or VS Code's extension.
    pub codelldb_path: String,
    /// Step into the standard library (`std`, `core`, `alloc`) instead of over it.
    pub step_into_std: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PluginSettings {
    /// Load installed plugins.
    pub enabled: bool,
    /// Plugin registry: a folder or an http(s)/file URL with a `registry.json`.
    /// Only read when you check it from the Extensions view.
    pub registry: String,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self { enabled: true, registry: String::new() }
    }
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { enabled: true, max_entries: 50, max_days: 30, max_file_kb: 5120 }
    }
}

impl Default for CargoSettings {
    fn default() -> Self {
        Self { check_command: "check".to_owned(), test_runner: "auto".to_owned() }
    }
}

impl Default for GitSettings {
    fn default() -> Self {
        Self { enabled: true, blame_inline: true }
    }
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self { shell: String::new(), scrollback: 10_000, font_size: 0.0 }
    }
}

impl Default for LspSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            rust_analyzer_path: String::new(),
            ask_to_install: true,
            rust_analyzer_memory: "default".to_owned(),
            rust_analyzer_separate_target: false,
            paths: std::collections::BTreeMap::new(),
            extension_servers: true,
        }
    }
}

impl FilesSettings {
    /// Line ending for new files from `files.eol`.
    pub fn new_file_eol(&self) -> &'static str {
        match self.eol.to_ascii_lowercase().as_str() {
            "lf" => "\n",
            "crlf" => "\r\n",
            _ if cfg!(windows) => "\r\n",
            _ => "\n",
        }
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_family: String::new(),
            font_size: 14.0,
            word_wrap: false,
            indent_guides: true,
            highlight_active_indentation: true,
            bracket_pair_colorization: true,
            matching_tags: true,
            bracket_pair_guides: true,
            format_on_save: false,
            likely_problems: true,
            markup_problems: true,
            color_decorators: true,
            font_previews: true,
            doc_comment_templates: true,
            auto_close_tags: true,
            auto_create_quotes: true,
            rename_tags: true,
            path_completion: true,
            tab_size: 4,
            insert_spaces: true,
            detect_indentation: true,
            auto_close_brackets: true,
            semantic_highlighting: true,
            inlay_hints: true,
            emmet: true,
            csv_colors: true,
            rulers: Vec::new(),
            glyph_bar: true,
            include_paths: Vec::new(),
            code_lens: true,
            smooth_scrolling: true,
            unicode_warnings: true,
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            keyboard_mode: "default".to_owned(),
        }
    }
}

impl Default for WorkbenchSettings {
    fn default() -> Self {
        Self {
            ui_font_size: 13.0,
            zoom: DEFAULT_ZOOM,
            preview_port: 0,
            color_theme: crate::appearance::theme::DEFAULT_THEME.to_owned(),
            background_image: String::new(),
            background_editor: String::new(),
            background_sidebar: String::new(),
            background_panel: String::new(),
            background_random: false,
            theme_location: String::new(),
            background_opacity: 0.1,
            background_area: "window".to_owned(),
            background_interval: 0,
            theme_schedule: String::new(),
            reduced_motion: false,
            double_shift: true,
            search_in_top_bar: true,
            max_fps: 0,
            ui: "dock".to_owned(),
            compact_folders: true,
            explorer_sort_order: "default".to_owned(),
            nest_files: true,
        }
    }
}

impl Default for FilesSettings {
    fn default() -> Self {
        Self {
            log_view_log_mb: 16,
            log_view_any_mb: 128,
            hot_exit: true,
            auto_save: "off".to_owned(),
            auto_save_delay_ms: 1000,
            eol: "auto".to_owned(),
            encoding: "utf8".to_owned(),
        }
    }
}

const PLATFORM_KEYS: [&str; 3] = ["windows", "macos", "linux"];
const MAX_EXTENDS_DEPTH: usize = 8;

fn current_platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn config_base() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// `<config dir>/coiman-ide`, e.g. `%APPDATA%\coiman-ide` on Windows.
pub fn config_dir() -> Option<PathBuf> {
    config_base().map(|b| b.join("coiman-ide"))
}

/// The folder before the rename to Coiman IDE: `<config dir>/rust-code`.
pub fn old_config_dir() -> Option<PathBuf> {
    config_base().map(|b| b.join("rust-code"))
}

/// Brings the settings, plugins, history and downloaded tools from before the rename:
/// the old folder is moved to the new name, once, when there's no new one yet. Returns
/// what happened, for the log.
pub fn adopt_old_config() -> Option<String> {
    let (old, new) = (old_config_dir()?, config_dir()?);
    if new.exists() || !old.is_dir() {
        return None;
    }
    Some(match std::fs::rename(&old, &new) {
        Ok(()) => format!("settings moved from {} to {}", old.display(), new.display()),
        Err(e) => format!("could not move {} to {}: {e}", old.display(), new.display()),
    })
}

/// A project's own folder of settings: `.coiman`, or `.rust-code` from before the rename
/// when that's the one there.
pub fn project_dir(root: &Path) -> PathBuf {
    let new = root.join(".coiman");
    let old = root.join(".rust-code");
    if !new.exists() && old.is_dir() { old } else { new }
}

pub fn user_settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("settings.toml"))
}

pub fn user_keybindings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("keybindings.toml"))
}

pub fn workspace_settings_path(root: &Path) -> PathBuf {
    project_dir(root).join("settings.toml")
}

/// Result of loading all layers: the settings plus anything worth telling the user.
pub struct Loaded {
    pub settings: Settings,
    pub problems: Vec<String>,
}

/// Loads and merges defaults, user and workspace settings. Problems (parse
/// errors, unknown keys, bad `extends`) are reported, never fatal: a broken
/// file is skipped and the other layers still apply.
pub fn load(workspace_root: Option<&Path>) -> Loaded {
    let mut problems = Vec::new();
    let mut merged = Table::new();
    // An untrusted folder's settings are ignored: they can name programs to run.
    let workspace_root = workspace_root.filter(|r| crate::project::trust::is_trusted(r));
    let files = user_settings_path().into_iter().chain(workspace_root.map(workspace_settings_path));
    for path in files {
        if path.exists() {
            match load_file(&path, 0) {
                Ok(table) => deep_merge(&mut merged, table),
                Err(e) => problems.push(e),
            }
        }
    }
    let (settings, mut more) = from_table(merged);
    problems.append(&mut more);
    Loaded { settings, problems }
}

/// Reads one file, following `extends` and applying its platform table.
fn load_file(path: &Path, depth: usize) -> Result<Table, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut table: Table = text.parse().map_err(|e: toml::de::Error| format!("{}: {}", path.display(), e.message()))?;

    let mut result = Table::new();
    if let Some(parent) = table.remove("extends") {
        let Value::String(rel) = parent else {
            return Err(format!("{}: `extends` must be a string path", path.display()));
        };
        if depth >= MAX_EXTENDS_DEPTH {
            return Err(format!("{}: `extends` nests too deeply (cycle?)", path.display()));
        }
        let parent_path = path.parent().unwrap_or(Path::new(".")).join(rel);
        result = load_file(&parent_path, depth + 1)?;
    }
    let platform = table.remove(current_platform());
    for key in PLATFORM_KEYS {
        table.remove(key);
    }
    deep_merge(&mut result, table);
    if let Some(Value::Table(p)) = platform {
        deep_merge(&mut result, p);
    }
    Ok(result)
}

/// Merges `over` into `base`; nested tables merge, everything else replaces.
fn deep_merge(base: &mut Table, over: Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(b)), Value::Table(o)) => deep_merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

/// Turns a merged table into settings, reporting unknown keys and type errors.
fn from_table(table: Table) -> (Settings, Vec<String>) {
    let mut problems = Vec::new();
    let defaults = Value::try_from(Settings::default()).expect("defaults serialize");
    unknown_keys(&Value::Table(table.clone()), &defaults, "", &mut problems);
    match Value::Table(table).try_into::<Settings>() {
        Ok(s) => (s, problems),
        Err(e) => {
            problems.push(format!("settings: {}", e.message()));
            (Settings::default(), problems)
        }
    }
}

fn unknown_keys(value: &Value, known: &Value, prefix: &str, out: &mut Vec<String>) {
    let (Value::Table(t), Value::Table(k)) = (value, known) else { return };
    for (key, v) in t {
        let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
        match k.get(key) {
            // Named by the user: one entry per language server.
            Some(_) if path == "lsp.paths" => {}
            Some(kv) => unknown_keys(v, kv, &path, out),
            None => out.push(format!("Unknown setting `{path}`")),
        }
    }
}

/// Sets `[table] key = value` in TOML text without disturbing comments: replaces
/// (and uncomments) an existing `key = ...` line in that table, or inserts one.
/// `value` must already be a TOML literal (e.g. `"Monokai"`).
pub fn set_value_in_text(text: &str, table: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = format!("[{table}]");
    let line = format!("{key} = {value}");
    let mut in_table = false;
    let mut header_at = None;
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.starts_with('[') {
            in_table = t == header;
            if in_table {
                header_at = Some(i);
            }
            continue;
        }
        let uncommented = t.trim_start_matches('#').trim_start();
        if in_table && uncommented.split('=').next().is_some_and(|k| k.trim() == key) && uncommented.contains('=') {
            lines[i] = line;
            return lines.join("\n") + "\n";
        }
    }
    match header_at {
        Some(i) => lines.insert(i + 1, line),
        None => {
            lines.push(String::new());
            lines.push(header);
            lines.push(line);
        }
    }
    lines.join("\n") + "\n"
}

/// Takes `[table] key = ...` out of a settings file, keeping everything else.
pub fn remove_value_in_text(text: &str, table: &str, key: &str) -> String {
    let header = format!("[{table}]");
    let mut in_table = false;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_table = t == header;
        }
        if in_table && !t.starts_with('#') && t.split('=').next().is_some_and(|k| k.trim() == key) && t.contains('=') {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Contents for a new user settings file: every setting, commented, with its default.
pub fn default_file_contents() -> String {
    let defaults = toml::to_string_pretty(&Settings::default()).unwrap_or_default();
    let commented: String = defaults
        .lines()
        .map(|l| if l.is_empty() || l.starts_with('[') { format!("{l}\n") } else { format!("# {l}\n") })
        .collect();
    format!(
        "# Coiman IDE user settings. Uncomment a line to change it; save to apply.\n\
         # Workspace settings in <folder>/.coiman/settings.toml override these.\n\
         # `extends = \"base.toml\"` inherits from another file; [windows]/[macos]/[linux]\n\
         # tables apply only on that platform, e.g. [windows.editor] font_size = 15\n\
         #\n\
         # workbench.color_theme: a name from \"Preferences: Color Theme\" or a path to a\n\
         # VS Code theme .json file.\n\n{commented}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_settings_folder_old_and_new() {
        let root = std::env::temp_dir().join(format!("rc-project-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Nothing yet: the new name.
        assert_eq!(project_dir(&root), root.join(".coiman"));
        // A project from before the rename keeps its folder.
        std::fs::create_dir_all(root.join(".rust-code")).unwrap();
        assert_eq!(project_dir(&root), root.join(".rust-code"));
        assert_eq!(workspace_settings_path(&root), root.join(".rust-code").join("settings.toml"));
        // With both, the new one.
        std::fs::create_dir_all(root.join(".coiman")).unwrap();
        assert_eq!(project_dir(&root), root.join(".coiman"));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn table(s: &str) -> Table {
        s.parse().unwrap()
    }

    #[test]
    fn later_layers_override_and_tables_merge() {
        let mut base = table("[editor]\nfont_size = 12\nword_wrap = true");
        deep_merge(&mut base, table("[editor]\nfont_size = 16"));
        let (s, problems) = from_table(base);
        assert!(problems.is_empty());
        assert_eq!(s.editor.font_size, 16.0);
        assert!(s.editor.word_wrap, "untouched keys survive the merge");
        assert_eq!(s.workbench.ui_font_size, 13.0, "missing keys use defaults");
    }

    #[test]
    fn extends_and_platform_tables() {
        let dir = std::env::temp_dir().join(format!("coiman-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base.toml"), "[editor]\nfont_size = 11\nword_wrap = true").unwrap();
        let platform = current_platform();
        std::fs::write(
            dir.join("settings.toml"),
            format!("extends = \"base.toml\"\n[editor]\nfont_size = 12\n[{platform}.editor]\nfont_size = 20\n[other_os.editor]\nfont_size = 99"),
        )
        .unwrap();
        let t = load_file(&dir.join("settings.toml"), 0).unwrap();
        let (s, problems) = from_table(t);
        assert_eq!(s.editor.font_size, 20.0, "platform table wins");
        assert!(s.editor.word_wrap, "inherited from base.toml");
        assert_eq!(problems, vec!["Unknown setting `other_os`".to_owned()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removes_only_the_named_setting() {
        let text = "# note\n[editor]\nfont_size = 15.0\n# tab_size = 4\ntab_size = 2\n[files]\ntab_size = 9\n";
        assert_eq!(
            remove_value_in_text(text, "editor", "tab_size"),
            "# note\n[editor]\nfont_size = 15.0\n# tab_size = 4\n[files]\ntab_size = 9\n"
        );
        assert_eq!(remove_value_in_text(text, "git", "enabled"), text);
    }

    #[test]
    fn unknown_keys_and_type_errors_are_reported() {
        let (s, problems) = from_table(table("[editor]\nfont_sise = 3"));
        assert_eq!(problems, vec!["Unknown setting `editor.font_sise`".to_owned()]);
        let (with_paths, problems) = from_table(table("[lsp.paths]\ngo = \"my-gopls\""));
        assert!(problems.is_empty(), "server names aren't settings: {problems:?}");
        assert_eq!(with_paths.lsp.paths["go"], "my-gopls");
        assert_eq!(s, Settings::default());
        let (_, problems) = from_table(table("[editor]\nfont_size = \"big\""));
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn extends_cycle_is_an_error_not_a_hang() {
        let dir = std::env::temp_dir().join(format!("coiman-cycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.toml"), "extends = \"a.toml\"").unwrap();
        assert!(load_file(&dir.join("a.toml"), 0).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_value_keeps_comments_and_uncomments() {
        let text = default_file_contents();
        let out = set_value_in_text(&text, "workbench", "color_theme", "\"Monokai\"");
        assert!(out.contains("# Coiman IDE user settings"), "comments survive");
        let (s, problems) = from_table(table(&out));
        assert!(problems.is_empty());
        assert_eq!(s.workbench.color_theme, "Monokai");
        // A file without the table gets one appended.
        let out = set_value_in_text("[editor]\nfont_size = 12\n", "workbench", "color_theme", "\"X\"");
        let (s, _) = from_table(table(&out));
        assert_eq!((s.editor.font_size, s.workbench.color_theme.as_str()), (12.0, "X"));
        // A key with the same name in another table is left alone.
        let out = set_value_in_text("[other]\ncolor_theme = 1\n[workbench]\n", "workbench", "color_theme", "\"Y\"");
        assert!(out.contains("color_theme = 1") && out.contains("color_theme = \"Y\""));
    }

    #[test]
    fn default_file_parses_to_defaults() {
        let (s, problems) = from_table(table(&default_file_contents()));
        assert!(problems.is_empty());
        assert_eq!(s, Settings::default());
    }
}
