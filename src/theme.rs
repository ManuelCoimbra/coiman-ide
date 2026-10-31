//! The current color theme as egui colors and visuals. Theme data, VS Code
//! import and token matching live in `rust_code_core::theme`.

use egui::{Color32, CornerRadius, Stroke, Visuals};
pub use rust_code_core::theme::*;

/// Workbench colors as egui colors.
pub type Palette = rust_code_core::theme::Palette<Color32>;

/// Converts a core color to an egui color.
pub fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Colors of the current theme.
pub fn colors() -> Palette {
    current().palette.map(c32)
}

/// Makes `theme` current and restyles egui.
pub fn set(ctx: &egui::Context, theme: Theme) {
    apply_visuals(ctx, &theme.palette.map(c32));
    set_current(theme);
}

fn apply_visuals(ctx: &egui::Context, p: &Palette) {
    let mut visuals = if p.dark { Visuals::dark() } else { Visuals::light() };
    visuals.panel_fill = p.sidebar_bg;
    visuals.window_fill = p.sidebar_bg;
    visuals.extreme_bg_color = p.editor_bg;
    visuals.faint_bg_color = p.hover_bg;
    visuals.override_text_color = Some(p.text);
    visuals.selection.bg_fill = p.selection;
    visuals.selection.stroke = Stroke::new(1.0, p.text);
    visuals.hyperlink_color = p.accent;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(6);

    let w = &mut visuals.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = p.widget_bg;
    w.hovered.weak_bg_fill = p.hover_bg;
    w.hovered.bg_fill = p.hover_bg;
    w.hovered.bg_stroke = Stroke::NONE;
    w.active.weak_bg_fill = p.hover_bg;
    w.active.bg_fill = p.hover_bg;
    w.open.weak_bg_fill = p.hover_bg;
    ctx.set_visuals(visuals);
}

/// Spacing and the workbench (non-editor) font size.
pub fn apply_style(ctx: &egui::Context, ui_font_size: f32) {
    use egui::{FontId, TextStyle};
    let size = ui_font_size.clamp(8.0, 32.0);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(6.0, 3.0);
        style.spacing.button_padding = egui::vec2(6.0, 2.0);
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(size * 0.8)),
            (TextStyle::Body, FontId::proportional(size)),
            (TextStyle::Button, FontId::proportional(size)),
            (TextStyle::Heading, FontId::proportional(size * 1.4)),
            (TextStyle::Monospace, FontId::monospace(size)),
        ]
        .into();
    });
    // Keep egui's own Ctrl+=/- window zoom off: the app handles those keys and saves the result.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
}
