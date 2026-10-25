//! Workbench UI around the editor: menus, side bar, tabs, find bar, status bar and dialogs.

use crate::commands::Command;
use crate::document::{DiskState, Document};
use crate::theme;
use egui::{Align, FontId, Key, RichText, Sense, Stroke};

use super::{CodeApp, FindCache, PendingClose, SidebarView};

impl CodeApp {
    /// Bar above the editor when the file changed or vanished on disk.
    pub(super) fn disk_conflict_bar(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let message = match doc.disk {
            DiskState::InSync => return,
            DiskState::Changed => "This file was changed on disk, and you have unsaved changes.",
            DiskState::Deleted => "This file was deleted on disk. Save to recreate it.",
        };
        let deleted = doc.disk == DiskState::Deleted;
        let mut action = None;
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
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

    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
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

    pub(super) fn activity_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
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
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, FontId::proportional(22.0), color);
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

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
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
                    let name = root
                        .file_name()
                        .map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().to_uppercase());
                    ui.label(RichText::new(name).strong().size(small_size(ui)));
                }
                if self.explorer.root.is_none()
                    && ui.button("Open Folder").clicked()
                    && let Some(path) = rfd::FileDialog::new().pick_folder()
                {
                    self.open_folder(path);
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

    pub(super) fn tab_bar(&mut self, ui: &mut egui::Ui) {
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
                                    let color =
                                        if active { theme::colors().tab_active_fg } else { theme::colors().text_dim };
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
                                    let close_color = if doc.is_dirty() {
                                        theme::colors().modified
                                    } else {
                                        theme::colors().text_dim
                                    };
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

    pub(super) fn breadcrumbs(&self, ui: &mut egui::Ui) {
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

    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui) {
        if !self.find.open {
            return;
        }
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let fresh = self.find.cache.as_ref().is_some_and(|c| {
            (c.uid, c.version, c.match_case) == (uid, version, self.find.match_case) && c.query == self.find.query
        });
        if !fresh {
            let matches = find_matches(&doc.text(), &self.find.query, self.find.match_case);
            let (query, match_case) = (self.find.query.clone(), self.find.match_case);
            self.find.cache = Some(FindCache { uid, version, query, match_case, matches });
        }
        let matches = self.find.cache.as_ref().map(|c| c.matches.clone()).unwrap_or_default();
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
                if ui.selectable_label(self.find.match_case, "Aa").on_hover_text("Match Case").clicked() {
                    self.find.match_case = !self.find.match_case;
                }
                let edit =
                    ui.add(egui::TextEdit::singleline(&mut self.find.query).hint_text("Find").desired_width(220.0));
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

    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) -> Option<Command> {
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

    /// Draws the status bar. Returns a command when a clickable item was clicked.
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut status_cmd = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            let size = small_size(ui);
            let white = |s: String| RichText::new(s).size(size).color(theme::colors().status_fg);
            if let Some(root) = &self.explorer.root {
                let name =
                    root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
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
                    let clickable = |ui: &mut egui::Ui, text: String, tip: &str| {
                        ui.add(egui::Label::new(white(text)).sense(Sense::click()).selectable(false))
                            .on_hover_text(tip)
                            .clicked()
                    };
                    if clickable(ui, doc.line_ending().to_owned(), "Select End of Line Sequence") {
                        status_cmd = Some(Command::ChangeLineEnding);
                    }
                    if clickable(ui, doc.encoding_name(), "Select Encoding") {
                        status_cmd = Some(Command::ChangeEncoding);
                    }
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
        status_cmd
    }

    pub(super) fn close_dialog(&mut self, ctx: &egui::Context) {
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
                ui.label(
                    RichText::new("Your changes will be lost if you don't save them.").color(theme::colors().text_dim),
                );
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

    pub(super) fn update_title(&mut self, ctx: &egui::Context) {
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

/// Size for secondary workbench text (breadcrumbs, status bar): 12/13 of the UI font.
fn small_size(ui: &egui::Ui) -> f32 {
    egui::TextStyle::Body.resolve(ui.style()).size * 12.0 / 13.0
}

/// All matches of `query` in `text`, as `[start, end)` char index ranges.
fn find_matches(text: &str, query: &str, match_case: bool) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let (hay, needle) =
        if match_case { (text.to_owned(), query.to_owned()) } else { (text.to_lowercase(), query.to_lowercase()) };
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
