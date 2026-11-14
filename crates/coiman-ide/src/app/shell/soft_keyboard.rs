//! The on-screen keyboard, for a phone where Termux:X11's own keyboard comes and goes with the
//! Back button: letters, a layer of the symbols code needs, and Esc, Tab, Ctrl, Alt and the
//! arrows. Its button cycles Off, On and Auto (`workbench.soft_keyboard`). Keys are taken as
//! the next frame's input, as a real keyboard's would be, and what had the keyboard keeps it.

use coiman_core::mobile::KeyboardMode;
use eframe::egui::{self, Event, Key, Modifiers, Sense};

use crate::app::CodeApp;
use crate::ui::theme;

/// A hardware keyboard used this recently keeps Auto's keyboard away.
const HARDWARE_SECS: f64 = 30.0;
/// Most keys in a row: a key is this fraction of the width.
const ROW_UNITS: f32 = 11.0;

#[derive(Clone, Copy)]
enum Cap {
    /// Typed as it is (letters follow Shift).
    Char(char),
    Key(Key, &'static str),
    Shift,
    Ctrl,
    Alt,
    Layer,
    Space,
}

const LETTERS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm,."];
const SYMBOLS: [&str; 4] = ["1234567890`", "(){}[]<>'\"^", "+-*/=_&|\\~", "!?:;@#$%"];

#[derive(Default)]
pub(crate) struct SoftKeyboard {
    symbols: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    /// Keys pressed, for the next frame.
    pending: Vec<Event>,
    paste: bool,
    /// What had the keyboard before a key was pressed: it gets it back.
    last_focus: Option<egui::Id>,
    /// When a hardware key was last pressed.
    hardware_at: Option<f64>,
    shown: bool,
}

impl SoftKeyboard {
    fn mods(&self) -> Modifiers {
        Modifiers { shift: self.shift, ctrl: self.ctrl, command: self.ctrl, alt: self.alt, ..Default::default() }
    }

    /// One press: a key, with the held modifiers, which then let go (as phone keyboards do).
    fn press(&mut self, cap: Cap) {
        match cap {
            Cap::Shift => {
                self.shift = !self.shift;
                return;
            }
            Cap::Ctrl => {
                self.ctrl = !self.ctrl;
                return;
            }
            Cap::Alt => {
                self.alt = !self.alt;
                return;
            }
            Cap::Layer => {
                self.symbols = !self.symbols;
                return;
            }
            Cap::Space => self.text(" "),
            Cap::Key(key, _) => self.key(key),
            Cap::Char(c) if self.ctrl || self.alt => {
                // Shortcuts: the clipboard's through the window, the rest as keys.
                match (self.ctrl, c.to_ascii_lowercase()) {
                    (true, 'c') => self.pending.push(Event::Copy),
                    (true, 'x') => self.pending.push(Event::Cut),
                    (true, 'v') => self.paste = true,
                    (_, c) => match Key::from_name(&c.to_string()) {
                        Some(key) => self.key(key),
                        None => self.text(&c.to_string()),
                    },
                }
            }
            Cap::Char(c) => {
                let c = if self.shift { c.to_uppercase().next().unwrap_or(c) } else { c };
                self.text(&c.to_string());
            }
        }
        self.shift = false;
        self.ctrl = false;
        self.alt = false;
    }

    fn text(&mut self, text: &str) {
        self.pending.push(Event::Text(text.to_owned()));
    }

    fn key(&mut self, key: Key) {
        let modifiers = self.mods();
        for pressed in [true, false] {
            self.pending.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        }
    }

    fn rows(&self) -> Vec<Vec<(Cap, f32)>> {
        let chars = |s: &str| s.chars().map(|c| (Cap::Char(c), 1.0)).collect::<Vec<_>>();
        let top = vec![
            (Cap::Key(Key::Escape, "Esc"), 1.0),
            (Cap::Key(Key::Tab, "Tab"), 1.0),
            (Cap::Ctrl, 1.0),
            (Cap::Alt, 1.0),
            (Cap::Key(Key::Home, "Home"), 1.0),
            (Cap::Key(Key::End, "End"), 1.0),
            (Cap::Key(Key::ArrowLeft, "←"), 1.0),
            (Cap::Key(Key::ArrowDown, "↓"), 1.0),
            (Cap::Key(Key::ArrowUp, "↑"), 1.0),
            (Cap::Key(Key::ArrowRight, "→"), 1.0),
        ];
        let backspace = (Cap::Key(Key::Backspace, "⌫"), 1.5);
        let enter = (Cap::Key(Key::Enter, "⏎"), 1.5);
        let mut rows = vec![top];
        if self.symbols {
            rows.push(chars(SYMBOLS[0]));
            rows.push(chars(SYMBOLS[1]));
            let mut third = chars(SYMBOLS[2]);
            third.push(backspace);
            rows.push(third);
            let mut last = vec![(Cap::Layer, 1.5)];
            last.extend(chars(SYMBOLS[3]));
            last.push((Cap::Space, 1.5));
            last.push(enter);
            rows.push(last);
        } else {
            rows.push(chars(LETTERS[0]));
            let mut second = chars(LETTERS[1]);
            second.push(backspace);
            rows.push(second);
            let mut third = vec![(Cap::Shift, 1.5)];
            third.extend(chars(LETTERS[2]));
            rows.push(third);
            rows.push(vec![(Cap::Layer, 1.5), (Cap::Char('-'), 1.0), (Cap::Space, 5.0), (Cap::Char(';'), 1.0), enter]);
        }
        rows
    }

    fn label(&self, cap: Cap) -> String {
        match cap {
            Cap::Char(c) if self.shift => c.to_uppercase().collect(),
            Cap::Char(c) => c.to_string(),
            Cap::Key(_, name) => name.to_owned(),
            Cap::Shift => "⇧".to_owned(),
            Cap::Ctrl => "Ctrl".to_owned(),
            Cap::Alt => "Alt".to_owned(),
            Cap::Layer => if self.symbols { "ABC" } else { "?123" }.to_owned(),
            Cap::Space => "space".to_owned(),
        }
    }

    fn held(&self, cap: Cap) -> bool {
        match cap {
            Cap::Shift => self.shift,
            Cap::Ctrl => self.ctrl,
            Cap::Alt => self.alt,
            _ => false,
        }
    }
}

impl CodeApp {
    pub(crate) fn soft_keyboard_mode(&self) -> KeyboardMode {
        KeyboardMode::parse(&self.settings.workbench.soft_keyboard)
    }

    /// The keyboard button and its command: Off, On, Auto, and round again (kept in settings).
    pub(crate) fn cycle_soft_keyboard(&mut self, ctx: &egui::Context) {
        let next = self.soft_keyboard_mode().next();
        let literal = format!("\"{}\"", next.setting());
        match self.write_user_setting(ctx, "workbench", "soft_keyboard", &literal) {
            Ok(()) => self.status = Some(format!("On-screen keyboard: {}", next.label())),
            Err(e) => self.status = Some(format!("Could not keep the keyboard mode: {e}")),
        }
    }

    /// Before the frame's keys are read: notes a hardware keyboard, then hands over what the
    /// on-screen one typed, giving the keyboard back to what had it.
    pub(crate) fn soft_keyboard_input(&mut self, ctx: &egui::Context) {
        let typed = ctx.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { pressed: true, .. })));
        if typed {
            self.soft_keyboard.hardware_at = Some(ctx.input(|i| i.time));
        }
        let keyboard = &mut self.soft_keyboard;
        if keyboard.pending.is_empty() && !keyboard.paste {
            return;
        }
        if let Some(id) = keyboard.last_focus {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        let events = std::mem::take(&mut keyboard.pending);
        ctx.input_mut(|i| i.events.extend(events));
        if std::mem::take(&mut keyboard.paste) {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        ctx.request_repaint();
    }

    /// The keyboard at the bottom of the window, when its mode says so.
    pub(crate) fn soft_keyboard_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let focused = ctx.memory(|m| m.focused());
        let hardware = self.soft_keyboard.hardware_at.is_some_and(|t| now - t < HARDWARE_SECS);
        // A press on the keyboard can take the focus for a moment: it's kept showing meanwhile.
        let typing = focused.is_some() || (self.soft_keyboard.shown && self.soft_keyboard.last_focus.is_some());
        let shows = self.soft_keyboard_mode().shows(typing, self.phone.android, hardware);
        self.soft_keyboard.shown = shows;
        if !shows {
            self.soft_keyboard.last_focus = None;
            return;
        }
        if focused.is_some() {
            self.soft_keyboard.last_focus = focused;
        }
        let pal = theme::colors();
        let rows = self.soft_keyboard.rows();
        let row_height = if theme::touch() { 44.0 } else { 34.0 };
        let height = rows.len() as f32 * (row_height + 4.0) + 8.0;
        egui::Panel::bottom("soft_keyboard")
            .exact_size(height)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(pal.sidebar_bg).inner_margin(egui::Margin::symmetric(6, 4)))
            .show(ui, |ui| {
                let width = ui.available_width();
                // Keys are sized by the widest row, so every row fits.
                let widest = rows
                    .iter()
                    .map(|row| (row.iter().map(|(_, w)| w).sum::<f32>(), row.len() as f32))
                    .fold((ROW_UNITS, ROW_UNITS), |a, b| if b.0 > a.0 { b } else { a });
                let unit = (width - 4.0 * widest.1) / widest.0;
                let mut pressed = None;
                for row in &rows {
                    let row_width: f32 = row.iter().map(|(_, w)| w * unit + 4.0).sum::<f32>() - 4.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add_space(((width - row_width) / 2.0).max(0.0));
                        for (cap, w) in row {
                            // Not focusable: the editor keeps the keyboard focus.
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(w * unit, row_height), Sense::CLICK);
                            let held = self.soft_keyboard.held(*cap);
                            let fill = if resp.is_pointer_button_down_on() || held {
                                pal.accent.gamma_multiply(0.45)
                            } else if matches!(cap, Cap::Char(_) | Cap::Space) {
                                pal.widget_bg
                            } else {
                                pal.hover_bg
                            };
                            ui.painter().rect_filled(rect, 6.0, fill);
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                self.soft_keyboard.label(*cap),
                                egui::FontId::proportional(if row_height > 40.0 { 18.0 } else { 15.0 }),
                                pal.text,
                            );
                            if resp.clicked() {
                                pressed = Some(*cap);
                            }
                        }
                    });
                    ui.add_space(4.0 - ui.spacing().item_spacing.y);
                }
                if let Some(cap) = pressed {
                    self.soft_keyboard.press(cap);
                    ctx.request_repaint();
                }
            });
    }

    /// The button that cycles the keyboard's mode, with the mode on it. Shown on a phone, or
    /// once the keyboard was turned on.
    pub(crate) fn soft_keyboard_button(&mut self, ui: &mut egui::Ui) -> bool {
        let mode = self.soft_keyboard_mode();
        if !self.phone.android && mode != KeyboardMode::On {
            return false;
        }
        let tip = "On-screen keyboard: OFF, ON or AUTO (on a phone, while typing, without a hardware keyboard)";
        ui.button(format!("Keys: {}", mode.label())).on_hover_text(tip).clicked()
    }
}
