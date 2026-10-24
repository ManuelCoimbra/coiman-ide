//! VS Code "Dark Modern"-inspired colors.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const EDITOR_BG: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const ACTIVITY_BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const TAB_INACTIVE_BG: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const TAB_ACTIVE_BG: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x1f);
pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0x78, 0xd4);
pub const STATUS_BG: Color32 = Color32::from_rgb(0x00, 0x7a, 0xcc);
pub const BORDER: Color32 = Color32::from_rgb(0x2b, 0x2b, 0x2b);
pub const TEXT: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x85, 0x85, 0x85);
pub const LINE_NUMBER: Color32 = Color32::from_rgb(0x6e, 0x76, 0x81);
pub const LINE_NUMBER_ACTIVE: Color32 = Color32::from_rgb(0xcc, 0xcc, 0xcc);
pub const MODIFIED: Color32 = Color32::from_rgb(0xe2, 0xc0, 0x8d);

pub fn apply(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = SIDEBAR_BG;
    visuals.window_fill = Color32::from_rgb(0x25, 0x25, 0x26);
    visuals.extreme_bg_color = EDITOR_BG;
    visuals.faint_bg_color = Color32::from_rgb(0x2a, 0x2d, 0x2e);
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = Color32::from_rgb(0x26, 0x4f, 0x78);
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.hyperlink_color = ACCENT;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.window_corner_radius = CornerRadius::same(6);

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    widgets.inactive.bg_fill = Color32::from_rgb(0x31, 0x31, 0x31);
    widgets.hovered.weak_bg_fill = Color32::from_rgb(0x2a, 0x2d, 0x2e);
    widgets.hovered.bg_fill = Color32::from_rgb(0x2a, 0x2d, 0x2e);
    widgets.hovered.bg_stroke = Stroke::NONE;
    widgets.active.weak_bg_fill = Color32::from_rgb(0x37, 0x37, 0x3d);
    widgets.active.bg_fill = Color32::from_rgb(0x37, 0x37, 0x3d);
    widgets.open.weak_bg_fill = Color32::from_rgb(0x37, 0x37, 0x3d);

    ctx.set_visuals(visuals);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
    });
}
