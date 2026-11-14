//! The editor's right-click menu: undo and redo, clipboard, the language server's go-to and refactoring
//! commands, and the edits most often wanted on a selection.

use coiman_core::editor::commands::Command;
use egui::{Pos2, RichText};

use crate::app::CodeApp;
use crate::ui::theme;

/// The open menu: where, for which document, and whether it opened this frame (so the click
/// that opened it doesn't close it).
pub(crate) struct EditorMenu {
    at: Pos2,
    uid: u64,
    fresh: bool,
}

/// One row of the menu.
enum Item {
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    Run(Command, &'static str),
    Line,
}

impl CodeApp {
    pub(crate) fn open_editor_menu(&mut self, at: Pos2) {
        let Some(uid) = self.docs.get(self.active).map(|d| d.uid) else { return };
        self.close_completion();
        self.editor_menu = Some(EditorMenu { at, uid, fresh: true });
    }

    pub(crate) fn editor_menu_ui(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.editor_menu.as_mut() else { return };
        let fresh = std::mem::take(&mut menu.fresh);
        let at = menu.at;
        if self.docs.get(self.active).map(|d| d.uid) != Some(menu.uid) {
            self.editor_menu = None;
            return;
        }
        let doc = self.docs.get(self.active);
        let selected = doc.is_some_and(|d| d.buffer.selections().iter().any(|s| !s.is_empty()));
        let (can_undo, can_redo) =
            doc.map_or((false, false), |d| (d.buffer.undo_depth() > 0, d.buffer.redo_depth() > 0));
        let items = [
            Item::Run(Command::GoToDefinition, "Go to Definition"),
            Item::Run(Command::GoToTypeDefinition, "Go to Type Definition"),
            Item::Run(Command::GoToImplementation, "Go to Implementations"),
            Item::Run(Command::FindReferences, "Go to References"),
            Item::Line,
            Item::Run(Command::RenameSymbol, "Rename Symbol"),
            Item::Run(Command::QuickFix, "Quick Fix..."),
            Item::Run(Command::FormatDocument, "Format Document"),
            Item::Line,
            Item::Undo,
            Item::Redo,
            Item::Line,
            Item::Cut,
            Item::Copy,
            Item::Paste,
            Item::Line,
            Item::Run(Command::ToggleLineComment, "Toggle Line Comment"),
            Item::Run(Command::SelectAllOccurrences, "Select All Occurrences"),
            Item::Line,
            Item::Run(Command::CommandPalette, "Command Palette..."),
        ];
        let pal = theme::colors();
        let mut chosen = None;
        let mut close = false;
        let area = egui::Area::new(egui::Id::unique("editor_context_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_min_width(260.0);
                    ui.set_max_width(260.0);
                    ui.style_mut().interaction.selectable_labels = false;
                    for (i, item) in items.iter().enumerate() {
                        let (label, command, enabled) = match item {
                            Item::Line => {
                                ui.separator();
                                continue;
                            }
                            Item::Undo => ("Undo", None, can_undo),
                            Item::Redo => ("Redo", None, can_redo),
                            Item::Cut => ("Cut", None, selected),
                            Item::Copy => ("Copy", None, selected),
                            Item::Paste => ("Paste", None, true),
                            Item::Run(c, label) => (*label, Some(*c), true),
                        };
                        let keys = match (item, command) {
                            (Item::Undo, _) => "Ctrl+Z".to_owned(),
                            (Item::Redo, _) => "Ctrl+Y".to_owned(),
                            (Item::Cut, _) => "Ctrl+X".to_owned(),
                            (Item::Copy, _) => "Ctrl+C".to_owned(),
                            (Item::Paste, _) => "Ctrl+V".to_owned(),
                            (_, Some(c)) => self.keymap.shortcut_text(c),
                            _ => String::new(),
                        };
                        let button = egui::Button::new(label)
                            .right_text(RichText::new(keys).color(pal.text_dim))
                            .frame(false)
                            .min_size(egui::vec2(ui.available_width(), 0.0));
                        if ui.add_enabled(enabled, button).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            });
        let outside = ui_clicked_outside(ctx, area.response.rect);
        if !fresh && (outside || ctx.input(|i| i.key_pressed(egui::Key::Escape))) {
            close = true;
        }
        let Some(i) = chosen else {
            if close {
                self.editor_menu = None;
            }
            return;
        };
        let uid = self.editor_menu.take().map(|m| m.uid).unwrap_or_default();
        // The click on the menu took the keyboard; the editor gets it back to act on it.
        ctx.memory_mut(|m| m.request_focus(crate::ui::editor_view::editor_id(uid, 0)));
        match &items[i] {
            // The editor handles these as it does the keys, next frame.
            Item::Cut => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCut),
            Item::Copy => ctx.send_viewport_cmd(egui::ViewportCommand::RequestCopy),
            Item::Paste => ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste),
            Item::Undo => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.buffer.undo();
                }
            }
            Item::Redo => {
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.buffer.redo();
                }
            }
            Item::Run(c, _) => self.run_command(ctx, *c),
            Item::Line => {}
        }
    }
}

/// Whether a mouse button went down this frame outside `rect`.
fn ui_clicked_outside(ctx: &egui::Context, rect: egui::Rect) -> bool {
    ctx.input(|i| (i.pointer.any_pressed()) && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)))
}
