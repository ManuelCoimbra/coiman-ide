//! Quick Open (Ctrl+P) and Command Palette (Ctrl+Shift+P) overlay.
//! As in VS Code, a query starting with '>' lists commands; otherwise files.

use std::path::{Path, PathBuf};

use egui::{Key, RichText};

use crate::keymap::Keymap;
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    CommandPalette,
    QuickOpen,
    NewFile,
    OpenFile,
    OpenFolder,
    Save,
    SaveAs,
    SaveAll,
    CloseTab,
    ToggleSidebar,
    ToggleTerminal,
    ShowExplorer,
    ShowSearch,
    Find,
    GoToLine,
    ZoomIn,
    ZoomOut,
    ResetZoom,
    RefreshExplorer,
    Quit,
    // Editor actions (editor.action.* in VS Code).
    ToggleLineComment,
    MoveLinesUp,
    MoveLinesDown,
    CopyLinesUp,
    CopyLinesDown,
    DeleteLines,
    IndentLines,
    OutdentLines,
    SelectNextOccurrence,
    AddCursorAbove,
    AddCursorBelow,
    JumpToBracket,
    JoinLines,
    DuplicateSelection,
    SortLinesAscending,
    SortLinesDescending,
    RemoveDuplicateLines,
    ReverseLines,
    TransformToUppercase,
    TransformToLowercase,
    TransformToTitlecase,
    // Workbench actions.
    ToggleWordWrap,
    ToggleZenMode,
    ReopenClosedEditor,
    NextEditor,
    PreviousEditor,
    // Preferences.
    OpenUserSettings,
    OpenWorkspaceSettings,
    OpenKeybindings,
    SelectTheme,
}

pub const COMMANDS: &[(Command, &str)] = &[
    (Command::NewFile, "File: New Untitled File"),
    (Command::OpenFile, "File: Open File..."),
    (Command::OpenFolder, "File: Open Folder..."),
    (Command::Save, "File: Save"),
    (Command::SaveAs, "File: Save As..."),
    (Command::SaveAll, "File: Save All"),
    (Command::CloseTab, "View: Close Editor"),
    (Command::ToggleSidebar, "View: Toggle Primary Side Bar Visibility"),
    (Command::ToggleTerminal, "View: Toggle Terminal"),
    (Command::ShowExplorer, "View: Show Explorer"),
    (Command::ShowSearch, "View: Show Search"),
    (Command::Find, "Find"),
    (Command::GoToLine, "Go to Line..."),
    (Command::ZoomIn, "View: Zoom In"),
    (Command::ZoomOut, "View: Zoom Out"),
    (Command::ResetZoom, "View: Reset Zoom"),
    (Command::RefreshExplorer, "File: Refresh Explorer"),
    (Command::Quit, "File: Exit"),
    (Command::ToggleLineComment, "Toggle Line Comment"),
    (Command::MoveLinesUp, "Move Line Up"),
    (Command::MoveLinesDown, "Move Line Down"),
    (Command::CopyLinesUp, "Copy Line Up"),
    (Command::CopyLinesDown, "Copy Line Down"),
    (Command::DeleteLines, "Delete Line"),
    (Command::IndentLines, "Indent Line"),
    (Command::OutdentLines, "Outdent Line"),
    (Command::SelectNextOccurrence, "Add Selection to Next Find Match"),
    (Command::AddCursorAbove, "Add Cursor Above"),
    (Command::AddCursorBelow, "Add Cursor Below"),
    (Command::JumpToBracket, "Go to Bracket"),
    (Command::JoinLines, "Join Lines"),
    (Command::DuplicateSelection, "Duplicate Selection"),
    (Command::SortLinesAscending, "Sort Lines Ascending"),
    (Command::SortLinesDescending, "Sort Lines Descending"),
    (Command::RemoveDuplicateLines, "Delete Duplicate Lines"),
    (Command::ReverseLines, "Reverse Lines"),
    (Command::TransformToUppercase, "Transform to Uppercase"),
    (Command::TransformToLowercase, "Transform to Lowercase"),
    (Command::TransformToTitlecase, "Transform to Title Case"),
    (Command::ToggleWordWrap, "View: Toggle Word Wrap"),
    (Command::ToggleZenMode, "View: Toggle Zen Mode"),
    (Command::ReopenClosedEditor, "View: Reopen Closed Editor"),
    (Command::NextEditor, "View: Open Next Editor"),
    (Command::PreviousEditor, "View: Open Previous Editor"),
    (Command::OpenUserSettings, "Preferences: Open User Settings (TOML)"),
    (Command::OpenWorkspaceSettings, "Preferences: Open Workspace Settings (TOML)"),
    (Command::OpenKeybindings, "Preferences: Open Keyboard Shortcuts (TOML)"),
    (Command::SelectTheme, "Preferences: Color Theme"),
];

pub enum PaletteResult {
    OpenFile(PathBuf),
    Run(Command),
    GoToLine(usize),
    Theme(String),
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
    /// When set, the palette is the Color Theme picker over these labels.
    themes: Option<Vec<String>>,
}

impl Palette {
    pub fn show_with(&mut self, prefix: &str) {
        self.open = true;
        self.query = prefix.to_owned();
        self.selected = 0;
        self.just_opened = true;
        self.themes = None;
    }

    /// Opens the Color Theme picker with `current` preselected.
    pub fn show_themes(&mut self, labels: Vec<String>, current: &str) {
        self.show_with("");
        self.selected = labels.iter().position(|l| l.eq_ignore_ascii_case(current)).unwrap_or(0);
        self.themes = Some(labels);
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        root: Option<&Path>,
        files: &[PathBuf],
        keymap: &Keymap,
    ) -> Option<PaletteResult> {
        if !self.open {
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return None;
        }

        // Build the candidate list: (label, detail, result).
        let mut items: Vec<(String, String, PaletteResult)> = Vec::new();
        if let Some(themes) = &self.themes {
            let q = self.query.trim();
            for t in themes.iter().filter(|t| fuzzy_match(t, q)) {
                items.push((t.clone(), String::new(), PaletteResult::Theme(t.clone())));
            }
        } else if let Some(q) = self.query.strip_prefix('>') {
            let q = q.trim();
            for (cmd, name) in COMMANDS {
                if fuzzy_match(name, q) {
                    items.push((name.to_string(), keymap.shortcut_text(*cmd), PaletteResult::Run(*cmd)));
                }
            }
        } else if let Some(q) = self.query.strip_prefix(':') {
            let line = q.trim().parse::<usize>().ok();
            let label = match line {
                Some(n) => format!("Go to line {n}"),
                None => "Type a line number to go to".to_owned(),
            };
            items.push((label, String::new(), PaletteResult::GoToLine(line.unwrap_or(0))));
        } else {
            let q = self.query.trim();
            for path in files {
                let rel = root.and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
                let rel_s = rel.to_string_lossy();
                if fuzzy_match(&rel_s, q) {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let dir = rel.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    items.push((name, dir, PaletteResult::OpenFile(path.clone())));
                    if items.len() >= 200 {
                        break;
                    }
                }
            }
        }

        let (up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.key_pressed(Key::Enter),
            )
        });
        if !items.is_empty() {
            if up {
                self.selected = self.selected.checked_sub(1).unwrap_or(items.len() - 1);
            }
            if down {
                self.selected = (self.selected + 1) % items.len();
            }
            self.selected = self.selected.min(items.len() - 1);
        }

        let mut result = None;
        let width = (ctx.content_rect().width() * 0.5).clamp(400.0, 700.0);
        egui::Area::new(egui::Id::unique("command_palette"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(theme::colors().sidebar_bg)
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text(if self.themes.is_some() {
                                    "Select Color Theme"
                                } else {
                                    "Search files by name (> for commands, : for line)"
                                })
                                .desired_width(f32::INFINITY),
                        );
                        if self.just_opened {
                            edit.request_focus();
                            // Put the cursor at the end of the prefilled text.
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                                let end = egui::text::CCursor::new(self.query.chars().count());
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                                state.store(ui.ctx(), edit.id);
                            }
                            self.just_opened = false;
                        }
                        if edit.changed() {
                            self.selected = 0;
                        }
                        ui.add_space(4.0);

                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            for (i, (label, detail, _)) in items.iter().enumerate() {
                                let selected = i == self.selected;
                                let resp = ui.horizontal(|ui| {
                                    let r = ui.selectable_label(selected, label);
                                    ui.label(RichText::new(detail).small().color(theme::colors().text_dim));
                                    r
                                });
                                if selected && (up || down) {
                                    resp.inner.scroll_to_me(None);
                                }
                                if resp.inner.clicked() {
                                    self.selected = i;
                                    result = Some(i);
                                }
                            }
                            if items.is_empty() {
                                ui.label(RichText::new("No matching results").color(theme::colors().text_dim));
                            }
                        });
                    });
            });

        if enter && !items.is_empty() {
            result = Some(self.selected);
        }
        let picked = result.map(|i| items.swap_remove(i).2);
        if picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// Case-insensitive subsequence match ("mnrs" matches "main.rs").
fn fuzzy_match(text: &str, query: &str) -> bool {
    let mut chars = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .all(|q| chars.any(|c| c == q))
}
