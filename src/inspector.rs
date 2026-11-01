//! One row of a tree of values, the same in the debugger and a live session.

use eframe::egui::{self, Color32, RichText, Sense};

use crate::icons::{self, Icon};
use crate::theme;

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
            ui.add(egui::Label::new(name).selectable(false));
            ui.add(egui::Label::new(RichText::new("=").font(mono.clone()).color(pal.text_dim)).selectable(false));
            ui.add(egui::Label::new(RichText::new(row.value).font(mono.clone())).selectable(false));
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
    if row.type_name.is_empty() { response } else { response.on_hover_text(row.type_name) }
}
