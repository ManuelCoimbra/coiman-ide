//! The Keyboard Shortcuts window: search commands, press new keys, done. Changes
//! go to the keybindings file, which can still be edited by hand.

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText};

use super::CodeApp;
use crate::commands::{COMMANDS, Command, command_id};
use crate::keymap::{BindingChange, Keymap, format_keys, is_modifier, parse_keys, set_binding_in_text};
use crate::settings;
use crate::theme;

#[derive(Default)]
pub(super) struct KeysUi {
    pub(super) open: bool,
    filter: String,
    /// The command getting new keys, and the strokes pressed so far.
    capture: Option<(Command, Vec<KeyboardShortcut>)>,
}

impl KeysUi {
    /// Keys pressed now are for this window, not for running commands.
    pub(super) fn capturing(&self) -> bool {
        self.open && self.capture.is_some()
    }
}

enum Action {
    Change(Command, BindingChange),
    OpenFile,
}

/// Adds this frame's key presses to `strokes`. Returns true on Escape.
fn read_strokes(ctx: &egui::Context, strokes: &mut Vec<KeyboardShortcut>) -> bool {
    let events = ctx.input(|i| i.events.clone());
    for event in events {
        if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
            if key == Key::Escape {
                return true;
            }
            if !is_modifier(key) && strokes.len() < 2 {
                // The same form as keys read from the file: Ctrl is `command` there.
                let mut plain = Modifiers::NONE;
                if modifiers.command || modifiers.ctrl {
                    plain |= Modifiers::COMMAND;
                }
                if modifiers.shift {
                    plain |= Modifiers::SHIFT;
                }
                if modifiers.alt {
                    plain |= Modifiers::ALT;
                }
                strokes.push(KeyboardShortcut::new(plain, key));
            }
        }
    }
    false
}

impl CodeApp {
    pub(super) fn keys_window(&mut self, ctx: &egui::Context) {
        if !self.keys_ui.open {
            return;
        }
        // While a key is being captured, Esc cancels that instead.
        let mut open = self.keys_ui.capture.is_some()
            || !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut action = None;
        let keymap = &self.keymap;
        let state = &mut self.keys_ui;
        if let Some((_, strokes)) = &mut state.capture
            && read_strokes(ctx, strokes)
        {
            state.capture = None;
        }
        egui::Window::new("Keyboard Shortcuts")
            .open(&mut open)
            .collapsible(false)
            .default_size([760.0, 560.0])
            .default_pos(ctx.content_rect().center() - egui::vec2(360.0, 260.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let search = egui::TextEdit::singleline(&mut state.filter).hint_text("Search commands or keys");
                    ui.add_enabled(state.capture.is_none(), search.desired_width(320.0));
                    if ui.button("Edit as File").clicked() {
                        action = Some(Action::OpenFile);
                    }
                });
                if let Some((command, strokes)) = &state.capture {
                    capture_bar(ui, keymap, *command, strokes, &mut action);
                }
                ui.separator();
                let filter = state.filter.to_lowercase();
                let rows: Vec<(Command, &str, String)> = COMMANDS
                    .iter()
                    .map(|(c, name)| (*c, *name, keymap.shortcut_text(*c)))
                    .filter(|(_, name, keys)| {
                        filter.is_empty()
                            || name.to_lowercase().contains(&filter)
                            || keys.to_lowercase().contains(&filter)
                    })
                    .collect();
                let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                let mut change = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(
                    ui,
                    row_h,
                    rows.len(),
                    |ui, range| {
                        for (command, name, keys) in &rows[range] {
                            ui.horizontal(|ui| {
                                let left = egui::Layout::left_to_right(egui::Align::Center);
                                ui.allocate_ui_with_layout(egui::vec2(360.0, row_h), left, |ui| {
                                    ui.set_width(360.0);
                                    ui.add(egui::Label::new(*name).truncate());
                                });
                                let text = if keys.is_empty() { "—" } else { keys.as_str() };
                                ui.allocate_ui_with_layout(egui::vec2(170.0, row_h), left, |ui| {
                                    ui.set_width(170.0);
                                    ui.add(egui::Label::new(RichText::new(text).monospace()).truncate());
                                });
                                if ui.small_button("Change").clicked() {
                                    change = Some(*command);
                                }
                                if !keys.is_empty() && ui.small_button("Remove").clicked() {
                                    action = Some(Action::Change(*command, BindingChange::Remove));
                                }
                                if ui.small_button("Reset").clicked() {
                                    action = Some(Action::Change(*command, BindingChange::Reset));
                                }
                            });
                        }
                    },
                );
                if let Some(command) = change {
                    state.capture = Some((command, Vec::new()));
                    // Keys go to the capture, not to a text box.
                    ui.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                }
            });
        self.keys_ui.open = open;
        if !open {
            self.keys_ui.capture = None;
        }
        match action {
            Some(Action::Change(command, change)) => {
                self.keys_ui.capture = None;
                if let Err(e) = self.change_binding(ctx, command, change) {
                    self.status = Some(e);
                }
            }
            Some(Action::OpenFile) => {
                self.keys_ui.open = false;
                self.open_config_file(settings::user_keybindings_path(), Keymap::defaults().default_file_contents());
            }
            None => {}
        }
    }

    fn change_binding(&mut self, ctx: &egui::Context, command: Command, change: BindingChange) -> Result<(), String> {
        let path = settings::user_keybindings_path().ok_or("No config directory available")?;
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| Keymap::defaults().default_file_contents());
        let updated = set_binding_in_text(&text, command_id(command), &change)?;
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, updated).map_err(|e| format!("Could not save keybindings: {e}"))?;
        self.reload_settings(ctx);
        Ok(())
    }
}

fn capture_bar(
    ui: &mut egui::Ui,
    keymap: &Keymap,
    command: Command,
    strokes: &[KeyboardShortcut],
    action: &mut Option<Action>,
) {
    let name = COMMANDS.iter().find(|(c, _)| *c == command).map_or("", |(_, n)| *n);
    egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(8.0).show(ui, |ui| {
        ui.label(RichText::new(format!("New keys for \"{name}\"")).strong());
        if strokes.is_empty() {
            ui.label("Press the keys, one stroke or two for a chord. Esc cancels.");
            return;
        }
        let keys = format_keys(strokes);
        ui.label(RichText::new(&keys).monospace());
        let others = keymap.commands_using(strokes);
        let others: Vec<&str> =
            COMMANDS.iter().filter(|(c, _)| *c != command && others.contains(c)).map(|(_, n)| *n).collect();
        if !others.is_empty() {
            ui.label(RichText::new(format!("Also used by: {}", others.join(", "))).color(theme::colors().modified));
        }
        // What is saved must read back as the same keys.
        let valid = parse_keys(&keys).is_ok_and(|k| k == strokes);
        if !valid {
            ui.label(RichText::new("These keys can't be saved.").color(theme::colors().modified));
        }
        if ui.add_enabled(valid, egui::Button::new("Save")).clicked() {
            let editor_only = keymap.is_editor_only(command);
            *action = Some(Action::Change(command, BindingChange::Set { keys, editor_only }));
        }
    });
}
