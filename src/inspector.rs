//! One row of a tree of values, the same in the debugger and a live session.

use eframe::egui::{self, Color32, RichText, Sense};

use crate::icons::{self, Icon};
use crate::theme;

/// The least room a name or value gets, however narrow the panel.
const NARROWEST: f32 = 40.0;
/// Room kept at the end of a row for what's added there.
const END_ROOM: f32 = 56.0;

/// What a row shows.
pub struct Row<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub type_name: &'a str,
    pub depth: usize,
    pub expandable: bool,
    pub open: bool,
}

/// Draws a row. `extra` adds to its end. The response is for clicks to open it.
pub fn row(ui: &mut egui::Ui, row: &Row, mono: &egui::FontId, extra: impl FnOnce(&mut egui::Ui)) -> egui::Response {
    let pal = theme::colors();
    let response = ui
        .horizontal(|ui| {
            ui.add_space(row.depth as f32 * 12.0);
            let size = ui.text_style_height(&egui::TextStyle::Body);
            if row.expandable {
                let chevron = if row.open { Icon::ChevronDown } else { Icon::ChevronRight };
                icons::show(ui, chevron, size, pal.text_dim);
            } else {
                ui.add_space(size);
            }
            // Not selectable, so a click anywhere on the row reaches it. Copy is on its menu.
            let name = RichText::new(row.name).font(mono.clone()).color(Color32::from_rgb(0x9C, 0xDC, 0xFE));
            // Long ones give way, or they'd widen the whole side bar. Hovering shows all.
            let height = ui.text_style_height(&egui::TextStyle::Body);
            let room = ui.available_width();
            let name_room = egui::vec2((room * 0.5).max(NARROWEST), height);
            ui.allocate_ui(name_room, |ui| ui.add(egui::Label::new(name).selectable(false).truncate()));
            ui.add(egui::Label::new(RichText::new("=").font(mono.clone()).color(pal.text_dim)).selectable(false));
            let value_room = egui::vec2((ui.available_width() - END_ROOM).max(NARROWEST), height);
            let value = RichText::new(row.value).font(mono.clone());
            ui.allocate_ui(value_room, |ui| ui.add(egui::Label::new(value).selectable(false).truncate()));
            extra(ui);
        })
        .response
        .interact(Sense::click());
    response.context_menu(|ui| {
        if ui.button("Copy Value").clicked() {
            ui.ctx().copy_text(row.value.to_owned());
            ui.close();
        }
    });
    let full = format!("{} = {}", row.name, row.value);
    let tip = if row.type_name.is_empty() { full } else { format!("{}\n{full}", row.type_name) };
    response.on_hover_text(tip)
}
