//! The Settings window: every setting as a control, saved to the settings file
//! when changed. The file stays the source of truth and can still be edited.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};
use toml::Value;

use super::CodeApp;
use crate::commands::Command;
use crate::settings::{self, Settings};
use crate::theme;

enum Kind {
    Toggle,
    Whole(i64, i64),
    Decimal(f64, f64),
    Choice(&'static [(&'static str, &'static str)]),
    Text(&'static str),
    Font,
}

struct Row {
    table: &'static str,
    key: &'static str,
    label: &'static str,
    help: &'static str,
    kind: Kind,
}

const fn row(table: &'static str, key: &'static str, label: &'static str, help: &'static str, kind: Kind) -> Row {
    Row { table, key, label, help, kind }
}

const LINE_ENDINGS: &[(&str, &str)] = &[("auto", "Automatic (CRLF on Windows)"), ("lf", "LF"), ("crlf", "CRLF")];
const ENCODINGS: &[(&str, &str)] =
    &[("utf8", "UTF-8"), ("utf8bom", "UTF-8 with BOM"), ("utf16le", "UTF-16 LE"), ("windows1252", "Windows 1252")];

const SECTIONS: &[(&str, &[Row])] = &[
    (
        "Editor",
        &[
            row("editor", "font_family", "Font", "Fixed-width fonts installed on this computer.", Kind::Font),
            row("editor", "font_size", "Font size", "Size of the code.", Kind::Decimal(6.0, 72.0)),
            row(
                "editor",
                "tab_size",
                "Tab size",
                "Columns per indent level, and how wide tabs display.",
                Kind::Whole(1, 16),
            ),
            row("editor", "insert_spaces", "Indent with spaces", "Otherwise with tabs.", Kind::Toggle),
            row(
                "editor",
                "detect_indentation",
                "Detect indentation",
                "Follow how each file is already indented.",
                Kind::Toggle,
            ),
            row("editor", "word_wrap", "Word wrap", "Wrap long lines at the window's edge.", Kind::Toggle),
            row(
                "editor",
                "auto_close_brackets",
                "Auto-close brackets",
                "Type brackets and quotes in pairs.",
                Kind::Toggle,
            ),
            row("editor", "indent_guides", "Indent guides", "Lines showing indentation levels.", Kind::Toggle),
            row(
                "editor",
                "bracket_pair_colorization",
                "Rainbow brackets",
                "Color matching brackets by depth.",
                Kind::Toggle,
            ),
            row(
                "editor",
                "semantic_highlighting",
                "Semantic highlighting",
                "Color symbols by what the language server says they are.",
                Kind::Toggle,
            ),
            row(
                "editor",
                "inlay_hints",
                "Inlay hints",
                "Show inferred types and parameter names in the text.",
                Kind::Toggle,
            ),
            row("editor", "code_lens", "Code lens", "Show what servers report about a line at its end.", Kind::Toggle),
            row("editor", "smooth_scrolling", "Smooth scrolling", "Glide when keys move the view.", Kind::Toggle),
            row(
                "editor",
                "unicode_warnings",
                "Hidden character warnings",
                "Mark bidi controls and invisible or private use characters.",
                Kind::Toggle,
            ),
            row("editor", "glyph_bar", "Glyph bar", "Characters to click, for languages with their own.", Kind::Toggle),
            row("editor", "csv_colors", "CSV column colors", "Color each column of CSV and TSV files.", Kind::Toggle),
            row("editor", "emmet", "Emmet", "Tab grows abbreviations in HTML and CSS.", Kind::Toggle),
            row(
                "editor",
                "format_on_save",
                "Format on save",
                "Format with the language server when saving.",
                Kind::Toggle,
            ),
            row("editor", "trim_trailing_whitespace", "Trim trailing whitespace", "When saving.", Kind::Toggle),
            row("editor", "insert_final_newline", "Insert final newline", "When saving.", Kind::Toggle),
            row(
                "editor",
                "keyboard_mode",
                "Keyboard mode",
                "Modal editing, if you want it.",
                Kind::Choice(&[("default", "Default"), ("vim", "Vim"), ("helix", "Helix")]),
            ),
        ],
    ),
    (
        "Appearance",
        &[
            row("workbench", "zoom", "Zoom", "Scale of the whole window. 1.25 is 125%.", Kind::Decimal(0.5, 3.0)),
            row(
                "workbench",
                "ui_font_size",
                "Interface font size",
                "Menus, side bar, tabs and status bar.",
                Kind::Decimal(8.0, 32.0),
            ),
        ],
    ),
    (
        "Files",
        &[
            row(
                "files",
                "hot_exit",
                "Restore unsaved changes",
                "Bring them back on the next launch instead of asking on exit.",
                Kind::Toggle,
            ),
            row(
                "files",
                "auto_save",
                "Auto save",
                "Save without asking, after you stop typing or when you leave the file or window.",
                Kind::Choice(&[("off", "Off"), ("after_delay", "After a delay"), ("focus_change", "On focus change")]),
            ),
            row(
                "files",
                "auto_save_delay_ms",
                "Auto save delay (ms)",
                "For \"After a delay\".",
                Kind::Whole(100, 60_000),
            ),
            row("files", "eol", "Line ending for new files", "", Kind::Choice(LINE_ENDINGS)),
            row("files", "encoding", "Encoding for new files", "", Kind::Choice(ENCODINGS)),
        ],
    ),
    (
        "Terminal",
        &[
            row(
                "terminal",
                "shell",
                "Shell",
                "Program and arguments. Empty uses the system's default.",
                Kind::Text("pwsh -NoLogo"),
            ),
            row("terminal", "scrollback", "Scrollback", "Lines kept per terminal.", Kind::Whole(100, 1_000_000)),
            row("terminal", "font_size", "Font size", "0 follows the code's size.", Kind::Decimal(0.0, 72.0)),
        ],
    ),
    (
        "Language servers",
        &[
            row("lsp", "enabled", "Language servers", "Errors, completion, hover and go to definition.", Kind::Toggle),
            row(
                "lsp",
                "ask_to_install",
                "Offer to install",
                "Offer to install a missing language server.",
                Kind::Toggle,
            ),
            row("lsp.paths", "python", "Python server", "Program and arguments. Empty finds it.", Kind::Text("")),
            row(
                "lsp.paths",
                "typescript",
                "TypeScript server",
                "Program and arguments. Empty finds it.",
                Kind::Text(""),
            ),
            row("lsp.paths", "clangd", "C and C++ server", "Program and arguments. Empty finds it.", Kind::Text("")),
            row("lsp.paths", "java", "Java server", "Program and arguments. Empty finds it.", Kind::Text("")),
            row("lsp.paths", "csharp", "C# server", "Program and arguments. Empty finds it.", Kind::Text("")),
            row("lsp.paths", "go", "Go server", "Program and arguments. Empty finds it.", Kind::Text("")),
            row(
                "cpp",
                "compile_commands_dir",
                "C and C++ database folder",
                "Where compile_commands.json is, from the project. Empty leaves it to clangd.",
                Kind::Text(""),
            ),
            row(
                "cpp",
                "config",
                "C and C++ configuration",
                "In a database of several, such as Ninja Multi-Config's: Debug, Release...",
                Kind::Text(""),
            ),
            row(
                "web",
                "baseline",
                "Baseline target",
                "widely, newly or a year such as 2023. Flags web features below it. Empty flags nothing.",
                Kind::Text(""),
            ),
        ],
    ),
    (
        "Rust",
        &[
            row(
                "lsp",
                "rust_analyzer_path",
                "rust-analyzer path",
                "Empty finds it on PATH or in ~/.cargo/bin.",
                Kind::Text(""),
            ),
            row(
                "lsp",
                "rust_analyzer_memory",
                "rust-analyzer memory",
                "Minimal also skips build scripts and proc macros, so code depending on them shows false errors.",
                Kind::Choice(&[("default", "Default"), ("low", "Low"), ("minimal", "Minimal")]),
            ),
            row(
                "lsp",
                "rust_analyzer_separate_target",
                "rust-analyzer's own build folder",
                "Cargo no longer waits for its checks, but build output is duplicated.",
                Kind::Toggle,
            ),
            row(
                "cargo",
                "check_command",
                "Check on save",
                "",
                Kind::Choice(&[("check", "cargo check"), ("clippy", "cargo clippy")]),
            ),
            row(
                "cargo",
                "test_runner",
                "Test runner",
                "",
                Kind::Choice(&[
                    ("auto", "Automatic (nextest when installed)"),
                    ("cargo", "cargo test"),
                    ("nextest", "nextest"),
                ]),
            ),
            row(
                "debug",
                "codelldb_path",
                "CodeLLDB path",
                "Empty finds Rust Code's own install or VS Code's extension.",
                Kind::Text(""),
            ),
            row("debug", "step_into_std", "Step into the standard library", "Instead of over it.", Kind::Toggle),
        ],
    ),
    (
        "Git",
        &[
            row("git", "enabled", "Source control", "Needs git on PATH.", Kind::Toggle),
            row("git", "blame_inline", "Inline blame", "Show who last changed the cursor line.", Kind::Toggle),
        ],
    ),
    (
        "Local history",
        &[
            row(
                "history",
                "enabled",
                "Keep local history",
                "A copy of a file each time it is saved or changed outside.",
                Kind::Toggle,
            ),
            row("history", "max_entries", "Versions per file", "", Kind::Whole(1, 10_000)),
            row("history", "max_days", "Days to keep", "", Kind::Whole(1, 3650)),
            row("history", "max_file_kb", "Largest file (KB)", "Larger files aren't kept.", Kind::Whole(1, 1_048_576)),
        ],
    ),
    (
        "Plugins",
        &[
            row("plugins", "enabled", "Load plugins", "", Kind::Toggle),
            row(
                "plugins",
                "registry",
                "Registry",
                "A folder or an http(s)/file URL with a registry.json.",
                Kind::Text(""),
            ),
        ],
    ),
];

#[derive(Default)]
pub(super) struct SettingsUi {
    pub(super) open: bool,
    filter: String,
    /// Change the open folder's settings instead of your own.
    pub(super) workspace: bool,
    /// Text being typed and numbers being dragged, saved when finished.
    texts: HashMap<&'static str, String>,
    numbers: HashMap<&'static str, f64>,
    /// Fixed-width fonts, once looked up.
    fonts: Vec<String>,
    fonts_loading: Option<Receiver<Vec<String>>>,
    fonts_loaded: bool,
}

enum Action {
    Set(&'static str, &'static str, String),
    Reset(&'static str, &'static str),
    Theme,
    OpenFile,
    KeyboardShortcuts,
}

fn lookup<'a>(root: &'a Value, table: &str, key: &str) -> Option<&'a Value> {
    table.split('.').try_fold(root, |v, part| v.get(part))?.get(key)
}

fn literal(value: Value) -> String {
    value.to_string()
}

/// Draws the control for one setting.
fn control(ui: &mut egui::Ui, state: &mut SettingsUi, row: &Row, value: Option<&Value>, out: &mut Option<Action>) {
    let (table, key) = (row.table, row.key);
    match row.kind {
        Kind::Toggle => {
            let mut on = value.and_then(Value::as_bool).unwrap_or(false);
            if ui.checkbox(&mut on, "").changed() {
                *out = Some(Action::Set(table, key, literal(Value::Boolean(on))));
            }
        }
        Kind::Choice(options) => {
            let current = value.and_then(Value::as_str).unwrap_or("");
            let shown = options.iter().find(|o| o.0 == current).map_or(current, |o| o.1);
            egui::ComboBox::new(("settings", table, key), "").selected_text(shown).width(220.0).show_ui(ui, |ui| {
                for (v, name) in options {
                    if ui.selectable_label(*v == current, *name).clicked() {
                        *out = Some(Action::Set(table, key, literal(Value::String((*v).to_owned()))));
                    }
                }
            });
        }
        Kind::Text(hint) => {
            let saved = value.and_then(Value::as_str).unwrap_or("");
            let text = state.texts.entry(key).or_insert_with(|| saved.to_owned());
            let edit = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(260.0));
            if edit.lost_focus() {
                if text != saved {
                    *out = Some(Action::Set(table, key, literal(Value::String(text.clone()))));
                }
                state.texts.remove(key);
            } else if !edit.has_focus() {
                // Not being edited: follow the file.
                saved.clone_into(text);
            }
        }
        Kind::Font => {
            let current = value.and_then(Value::as_str).unwrap_or("");
            let shown = if current.is_empty() { "Built-in" } else { current };
            egui::ComboBox::new(("settings", table, key), "").selected_text(shown).width(220.0).height(320.0).show_ui(
                ui,
                |ui| {
                    if ui.selectable_label(current.is_empty(), "Built-in").clicked() {
                        *out = Some(Action::Set(table, key, literal(Value::String(String::new()))));
                    }
                    for name in &state.fonts {
                        if ui.selectable_label(name == current, name).clicked() {
                            *out = Some(Action::Set(table, key, literal(Value::String(name.clone()))));
                        }
                    }
                    if state.fonts_loading.is_some() {
                        ui.label("Looking for fonts…");
                    }
                },
            );
        }
        Kind::Whole(low, high) => {
            let saved = value.and_then(Value::as_integer).unwrap_or(0) as f64;
            if let Some(v) = number(ui, state, key, saved, low as f64, high as f64, 0) {
                *out = Some(Action::Set(table, key, literal(Value::Integer(v.round() as i64))));
            }
        }
        Kind::Decimal(low, high) => {
            let saved = value.and_then(Value::as_float).unwrap_or(0.0);
            if let Some(v) = number(ui, state, key, saved, low, high, 1) {
                *out = Some(Action::Set(table, key, literal(Value::Float(v))));
            }
        }
    }
}

/// A number field. Gives the value when the user finishes changing it.
fn number(
    ui: &mut egui::Ui,
    state: &mut SettingsUi,
    key: &'static str,
    saved: f64,
    low: f64,
    high: f64,
    decimals: usize,
) -> Option<f64> {
    let mut value = state.numbers.get(key).copied().unwrap_or(saved);
    let field = ui.add(egui::DragValue::new(&mut value).range(low..=high).fixed_decimals(decimals).speed(0.2));
    if field.changed() {
        state.numbers.insert(key, value);
    }
    if field.drag_stopped() || field.lost_focus() {
        return state.numbers.remove(key).filter(|v| *v != saved);
    }
    None
}

impl CodeApp {
    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_ui.open {
            return;
        }
        let current = Value::try_from(&self.settings).unwrap_or(Value::Boolean(false));
        let defaults = Value::try_from(Settings::default()).unwrap_or(Value::Boolean(false));
        let workspace_allowed = self.explorer.root.is_some() && self.restricted_folder().is_none();
        if !workspace_allowed {
            self.settings_ui.workspace = false;
        }
        let mut open = true;
        let mut action = None;
        let state = &mut self.settings_ui;
        if !state.fonts_loaded {
            state.fonts_loaded = true;
            let (tx, rx) = mpsc::channel();
            state.fonts_loading = Some(rx);
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(rust_code_core::fonts::monospace_families(&rust_code_core::fonts::installed()));
                wake.request_repaint();
            });
        }
        if let Some(fonts) = state.fonts_loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.fonts = fonts;
            state.fonts_loading = None;
        }
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_size([760.0, 560.0])
            .default_pos(ctx.content_rect().center() - egui::vec2(380.0, 280.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.filter).hint_text("Search settings").desired_width(240.0),
                    );
                    ui.selectable_value(&mut state.workspace, false, "User");
                    ui.add_enabled_ui(workspace_allowed, |ui| {
                        ui.selectable_value(&mut state.workspace, true, "This folder")
                            .on_disabled_hover_text("Open a trusted folder to change its settings");
                    });
                    if ui.button("Keyboard Shortcuts").clicked() {
                        action = Some(Action::KeyboardShortcuts);
                    }
                    if ui.button("Edit as File").clicked() {
                        action = Some(Action::OpenFile);
                    }
                });
                ui.separator();
                let filter = state.filter.to_lowercase();
                let matches = |r: &Row| {
                    filter.is_empty()
                        || r.label.to_lowercase().contains(&filter)
                        || r.help.to_lowercase().contains(&filter)
                        || r.key.contains(&filter)
                };
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if filter.is_empty() || "color theme".contains(&filter) {
                        ui.heading("Color theme");
                        if ui.button(&self.settings.workbench.color_theme).clicked() {
                            action = Some(Action::Theme);
                        }
                        ui.add_space(10.0);
                    }
                    for (title, rows) in SECTIONS {
                        if !rows.iter().any(matches) {
                            continue;
                        }
                        ui.heading(*title);
                        for row in rows.iter().filter(|r| matches(r)) {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width() - 340.0);
                                    ui.label(row.label);
                                    if !row.help.is_empty() {
                                        let help = RichText::new(row.help).small().color(theme::colors().text_dim);
                                        ui.add(egui::Label::new(help).wrap());
                                    }
                                });
                                let value = lookup(&current, row.table, row.key);
                                let default = lookup(&defaults, row.table, row.key);
                                let changed = match (value, default) {
                                    (Some(v), Some(d)) => v != d,
                                    (Some(v), None) => v.as_str() != Some(""),
                                    _ => false,
                                };
                                control(ui, state, row, value, &mut action);
                                if changed && ui.small_button("Reset").clicked() {
                                    action = Some(Action::Reset(row.table, row.key));
                                }
                            });
                            ui.add_space(4.0);
                        }
                        ui.add_space(10.0);
                    }
                });
            });
        self.settings_ui.open = open;
        let workspace = self.settings_ui.workspace;
        let result = match action {
            Some(Action::Set(table, key, value)) => self.change_setting(ctx, workspace, table, key, Some(&value)),
            Some(Action::Reset(table, key)) => self.change_setting(ctx, workspace, table, key, None),
            Some(Action::Theme) => {
                self.run_command(ctx, Command::SelectTheme);
                Ok(())
            }
            Some(Action::KeyboardShortcuts) => {
                self.keys_ui.open = true;
                Ok(())
            }
            Some(Action::OpenFile) => {
                let path = match (&self.explorer.root, workspace) {
                    (Some(root), true) => Some(settings::workspace_settings_path(root)),
                    _ => settings::user_settings_path(),
                };
                self.open_config_file(path, settings::default_file_contents());
                self.settings_ui.open = false;
                Ok(())
            }
            None => Ok(()),
        };
        if let Err(e) = result {
            self.status = Some(e);
        }
    }
}
