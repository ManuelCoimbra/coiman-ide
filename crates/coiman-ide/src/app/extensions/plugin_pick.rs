//! A plugin's pick list (`show-pick`): a box to type in and rows to choose from, below the
//! terminal's cursor or at the top of the window. Typing narrows the rows here when the
//! plugin asked for that, else the plugin hears of it and sends the rows again.

use coiman_core::extensions::plugin::{Event, Pick, PickAction, PickChoice, PickQuery};
use egui::{Key, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::theme;

/// The pick list on screen.
pub(super) struct Shown {
    plugin: String,
    pick: Pick,
    /// What the box holds: ahead of the plugin's own copy while it catches up.
    query: String,
    /// What the plugin last heard.
    told: String,
    selected: usize,
    focus: bool,
    at: Option<egui::Pos2>,
}

impl Shown {
    /// The rows shown: all of them, or those holding every word typed when the editor narrows.
    fn rows(&self) -> Vec<usize> {
        let words: Vec<String> = self.query.split_whitespace().map(str::to_lowercase).collect();
        (0..self.pick.items.len())
            .filter(|&i| {
                !self.pick.filter || {
                    let text = self.pick.items[i].text.to_lowercase();
                    words.iter().all(|w| text.contains(w.as_str()))
                }
            })
            .collect()
    }
}

impl CodeApp {
    /// A plugin shows, changes or (with an empty title) closes its pick list.
    pub(super) fn show_plugin_pick(&mut self, plugin: String, pick: Pick) {
        let same = self.plugins.pick.as_ref().filter(|s| s.plugin == plugin && s.pick.id == pick.id);
        if pick.title.is_empty() {
            if same.is_some() {
                self.plugins.pick = None;
            }
            return;
        }
        let shown = match same {
            // New rows for the same list: what is typed stays.
            Some(s) => Shown {
                query: s.query.clone(),
                told: s.told.clone(),
                at: s.at,
                plugin,
                pick,
                selected: 0,
                focus: false,
            },
            None => Shown {
                query: pick.query.clone(),
                told: pick.query.clone(),
                at: if pick.at_terminal_cursor { self.plugins.terminal_at } else { None },
                plugin,
                pick,
                selected: 0,
                focus: true,
            },
        };
        self.plugins.pick = Some(shown);
    }

    fn plugin_picked(&mut self, plugin: &str, choice: PickChoice) {
        if let Some(host) = &self.plugins.host {
            host.deliver(plugin, Event::Picked(choice));
        }
    }

    pub(crate) fn plugin_pick_window(&mut self, ctx: &egui::Context) {
        let Some(mut shown) = self.plugins.pick.take() else { return };
        let pal = theme::colors();
        let mut open = true;
        let mut chosen: Option<(String, PickAction)> = None;
        let (down, up, enter, shift, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.key_pressed(Key::Enter),
                i.modifiers.shift || i.modifiers.command,
                i.consume_key(egui::Modifiers::NONE, Key::Escape),
            )
        });
        let rows = shown.rows();
        if down && shown.selected + 1 < rows.len() {
            shown.selected += 1;
        }
        if up {
            shown.selected = shown.selected.saturating_sub(1);
        }
        shown.selected = shown.selected.min(rows.len().saturating_sub(1));
        let room = ctx.content_rect();
        let width = (room.width() * 0.6).clamp(360.0, 760.0);
        let mut window = egui::Window::new(RichText::new(&shown.pick.title).strong())
            .id(egui::Id::unique("plugin_pick"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(shown.at.is_none())
            .min_width(width)
            .max_width(width)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(10.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(10, 8)),
            );
        let x = |at: egui::Pos2| at.x.min(room.right() - width - 8.0);
        window = match shown.at {
            // Above the cursor when there is no room below.
            Some(at) if at.y + 330.0 > room.bottom() => {
                window.pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(egui::pos2(x(at), at.y - 22.0))
            }
            Some(at) => window.fixed_pos(egui::pos2(x(at), at.y + 2.0)),
            None => window.anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0)),
        };
        let run_first = shown.pick.run_first;
        let action = |flip: bool| if run_first != flip { PickAction::Run } else { PickAction::Put };
        window.show(ctx, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut shown.query)
                    .hint_text("Type to narrow the list")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            if std::mem::take(&mut shown.focus) {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                    let end = egui::text::CCursor::new(shown.query.chars().count());
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                    state.store(ui.ctx(), r.id);
                }
            }
            if enter && r.lost_focus() {
                match rows.get(shown.selected) {
                    Some(&i) => chosen = Some((shown.pick.items[i].text.clone(), action(shift))),
                    None if !shown.query.trim().is_empty() => chosen = Some((shown.query.clone(), action(shift))),
                    None => {}
                }
            }
            if rows.is_empty() {
                ui.label(RichText::new("Nothing fits").color(pal.text_dim));
            }
            egui::ScrollArea::vertical().max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
                for (n, &i) in rows.iter().enumerate() {
                    let item = &shown.pick.items[i];
                    ui.horizontal(|ui| {
                        let selected = n == shown.selected;
                        let r = ui
                            .add(egui::Button::selectable(selected, RichText::new(&item.text).monospace()).truncate());
                        if selected && (down || up) {
                            r.scroll_to_me(None);
                        }
                        if r.clicked() {
                            chosen = Some((item.text.clone(), action(ui.input(|i| i.modifiers.shift))));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if item.can_forget && ui.small_button("✕").on_hover_text("Forget it").clicked() {
                                chosen = Some((item.text.clone(), PickAction::Forget));
                            }
                            ui.label(RichText::new(&item.kind).small().color(pal.text_dim));
                            if !item.detail.is_empty() {
                                ui.add(
                                    egui::Label::new(RichText::new(&item.detail).small().color(pal.text_dim))
                                        .truncate(),
                                );
                            }
                        });
                    });
                }
            });
            let keys = if run_first {
                "Enter runs it · Shift+Enter puts it on the command line · Esc closes"
            } else {
                "Enter puts it on the command line · Shift+Enter runs it · Esc closes"
            };
            ui.label(RichText::new(keys).small().color(pal.text_dim));
        });
        let plugin = shown.plugin.clone();
        let id = shown.pick.id.clone();
        if let Some((text, action)) = chosen {
            let forget = action == PickAction::Forget;
            self.plugin_picked(&plugin, PickChoice { pick: id, text, action });
            if forget {
                self.plugins.pick = Some(shown);
            } else {
                self.terminal.focus_requested = true;
            }
            return;
        }
        if !open || escape {
            self.terminal.focus_requested = true;
            return;
        }
        // The plugin narrows the list itself: it hears what is typed.
        if !shown.pick.filter && shown.query != shown.told {
            shown.told = shown.query.clone();
            if let Some(host) = &self.plugins.host {
                host.deliver(&plugin, Event::PickQueryChanged(PickQuery { pick: id, query: shown.query.clone() }));
            }
        }
        self.plugins.pick = Some(shown);
    }
}
