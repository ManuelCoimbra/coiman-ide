//! Quick Open (Ctrl+P) and Command Palette (Ctrl+Shift+P) overlay.
//! As in VS Code, a query starting with '>' lists commands; otherwise files.

use std::path::{Path, PathBuf};

use egui::{Key, RichText};

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
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
}

pub const COMMANDS: &[(Command, &str, &str)] = &[
    (Command::NewFile, "File: New Untitled File", "Ctrl+N"),
    (Command::OpenFile, "File: Open File...", "Ctrl+O"),
    (Command::OpenFolder, "File: Open Folder...", "Ctrl+K Ctrl+O"),
    (Command::Save, "File: Save", "Ctrl+S"),
    (Command::SaveAs, "File: Save As...", "Ctrl+Shift+S"),
    (Command::SaveAll, "File: Save All", "Ctrl+Alt+S"),
    (Command::CloseTab, "View: Close Editor", "Ctrl+W"),
    (Command::ToggleSidebar, "View: Toggle Primary Side Bar Visibility", "Ctrl+B"),
    (Command::ToggleTerminal, "View: Toggle Terminal", "Ctrl+`"),
    (Command::ShowExplorer, "View: Show Explorer", "Ctrl+Shift+E"),
    (Command::ShowSearch, "View: Show Search", "Ctrl+Shift+F"),
    (Command::Find, "Find", "Ctrl+F"),
    (Command::GoToLine, "Go to Line...", "Ctrl+G"),
    (Command::ZoomIn, "View: Zoom In", "Ctrl+="),
    (Command::ZoomOut, "View: Zoom Out", "Ctrl+-"),
    (Command::ResetZoom, "View: Reset Zoom", "Ctrl+0"),
    (Command::RefreshExplorer, "File: Refresh Explorer", ""),
    (Command::Quit, "File: Exit", ""),
    (Command::ToggleLineComment, "Toggle Line Comment", "Ctrl+/"),
    (Command::MoveLinesUp, "Move Line Up", "Alt+Up"),
    (Command::MoveLinesDown, "Move Line Down", "Alt+Down"),
    (Command::CopyLinesUp, "Copy Line Up", "Shift+Alt+Up"),
    (Command::CopyLinesDown, "Copy Line Down", "Shift+Alt+Down"),
    (Command::DeleteLines, "Delete Line", "Ctrl+Shift+K"),
    (Command::IndentLines, "Indent Line", "Ctrl+]"),
    (Command::OutdentLines, "Outdent Line", "Ctrl+["),
    (Command::SelectNextOccurrence, "Add Selection to Next Find Match", "Ctrl+D"),
    (Command::AddCursorAbove, "Add Cursor Above", "Ctrl+Alt+Up"),
    (Command::AddCursorBelow, "Add Cursor Below", "Ctrl+Alt+Down"),
    (Command::JumpToBracket, "Go to Bracket", "Ctrl+Shift+\\"),
    (Command::JoinLines, "Join Lines", ""),
    (Command::DuplicateSelection, "Duplicate Selection", ""),
    (Command::SortLinesAscending, "Sort Lines Ascending", ""),
    (Command::SortLinesDescending, "Sort Lines Descending", ""),
    (Command::RemoveDuplicateLines, "Delete Duplicate Lines", ""),
    (Command::ReverseLines, "Reverse Lines", ""),
    (Command::TransformToUppercase, "Transform to Uppercase", ""),
    (Command::TransformToLowercase, "Transform to Lowercase", ""),
    (Command::TransformToTitlecase, "Transform to Title Case", ""),
    (Command::ToggleWordWrap, "View: Toggle Word Wrap", "Alt+Z"),
    (Command::ToggleZenMode, "View: Toggle Zen Mode", "Ctrl+K Z"),
    (Command::ReopenClosedEditor, "View: Reopen Closed Editor", "Ctrl+Shift+T"),
];

pub enum PaletteResult {
    OpenFile(PathBuf),
    Run(Command),
    GoToLine(usize),
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
}

impl Palette {
    pub fn show_with(&mut self, prefix: &str) {
        self.open = true;
        self.query = prefix.to_owned();
        self.selected = 0;
        self.just_opened = true;
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        root: Option<&Path>,
        files: &[PathBuf],
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
        if let Some(q) = self.query.strip_prefix('>') {
            let q = q.trim();
            for (cmd, name, keys) in COMMANDS {
                if fuzzy_match(name, q) {
                    items.push((name.to_string(), keys.to_string(), PaletteResult::Run(*cmd)));
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
                    .fill(egui::Color32::from_rgb(0x22, 0x22, 0x22))
                    .inner_margin(6.0)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.query)
                                .hint_text("Search files by name (> for commands, : for line)")
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
                                    ui.label(RichText::new(detail).small().color(theme::TEXT_DIM));
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
                                ui.label(RichText::new("No matching results").color(theme::TEXT_DIM));
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
