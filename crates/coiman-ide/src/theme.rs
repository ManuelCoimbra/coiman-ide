//! The current color theme as egui colors and visuals. Theme data, VS Code
//! import and token matching live in `coiman_core::theme`.

use std::sync::atomic::{AtomicBool, Ordering};

pub use coiman_core::theme::*;
use egui::{Color32, CornerRadius, Margin, Rect, Stroke, Visuals};

/// Workbench colors as egui colors.
pub type Palette = coiman_core::theme::Palette<Color32>;

/// Converts a core color to an egui color.
pub fn c32(c: Color) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// The gap around islands, and how round they are.
const GAP: i8 = 6;
const RADIUS: u8 = 10;
const MOTION_SECONDS: f32 = 0.14;

static ISLANDS: AtomicBool = AtomicBool::new(true);

pub fn islands() -> bool {
    ISLANDS.load(Ordering::Relaxed)
}

/// Chooses between the island layout and the classic one, and restyles egui.
pub fn set_islands(ctx: &egui::Context, on: bool) {
    ISLANDS.store(on, Ordering::Relaxed);
    apply_visuals(ctx, &colors());
}

/// Colors of the current theme.
pub fn colors() -> Palette {
    let palette = current().palette;
    if islands() { palette.islands().map(c32) } else { palette.map(c32) }
}

/// Makes `theme` current and restyles egui.
pub fn set(ctx: &egui::Context, theme: Theme) {
    set_current(theme);
    apply_visuals(ctx, &colors());
}

/// How long panels, tabs and popups take to change: not at all with reduced motion.
pub fn set_motion(ctx: &egui::Context, reduced: bool) {
    let seconds = if reduced { 0.0 } else { MOTION_SECONDS };
    ctx.global_style_mut(|style| style.animation_time = seconds);
}

/// From 0 to 1 as `on` turns true, and back, over the current motion time.
pub fn eased(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    ctx.animate_bool(id, on)
}

/// Where a tool window of this width first opens: centered, clear of the top bar and its
/// window buttons.
pub fn window_spot(ctx: &egui::Context, width: f32) -> egui::Pos2 {
    let room = ctx.content_rect();
    egui::pos2(room.center().x - width / 2.0, room.top() + 60.0)
}

/// The gaps an island keeps on these sides, or none in the classic layout.
pub fn island_gaps(left: bool, right: bool, top: bool, bottom: bool) -> Margin {
    if !islands() {
        return Margin::ZERO;
    }
    let gap = |on: bool| if on { GAP } else { 0 };
    Margin { left: gap(left), right: gap(right), top: gap(top), bottom: gap(bottom) }
}

/// A panel's frame: filled and rounded inside its gaps.
pub fn island_frame(fill: Color32, inner: Margin, gaps: Margin) -> egui::Frame {
    let frame = egui::Frame::new().fill(fill).inner_margin(inner);
    if islands() { frame.outer_margin(gaps).corner_radius(CornerRadius::same(RADIUS)) } else { frame }
}

/// Rounds off what panels drew square, right to the corners of their islands: tab bars and
/// editors don't know they're in one.
pub fn round_island_corners(ctx: &egui::Context, islands: &[Rect]) {
    let id = egui::LayerId::new(egui::Order::Background, egui::Id::unique("island_corners"));
    let painter = ctx.layer_painter(id);
    let canvas = colors().activity_bg;
    let radius = f32::from(RADIUS);
    let mut mesh = egui::Mesh::default();
    for rect in islands {
        for (corner, x, y) in [
            (rect.left_top(), 1.0, 1.0),
            (rect.right_top(), -1.0, 1.0),
            (rect.left_bottom(), 1.0, -1.0),
            (rect.right_bottom(), -1.0, -1.0),
        ] {
            let center = corner + egui::vec2(x, y) * radius;
            let apex = mesh.vertices.len() as u32;
            mesh.colored_vertex(corner, canvas);
            const STEPS: u32 = 8;
            for step in 0..=STEPS {
                let angle = step as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
                mesh.colored_vertex(center - egui::vec2(x * angle.cos(), y * angle.sin()) * radius, canvas);
                if step > 0 {
                    mesh.add_triangle(apex, apex + step, apex + step + 1);
                }
            }
        }
    }
    painter.add(mesh);
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
    visuals.window_corner_radius = CornerRadius::same(if islands() { RADIUS } else { 6 });

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
    if islands() {
        visuals.menu_corner_radius = CornerRadius::same(8);
        for widget in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            widget.corner_radius = CornerRadius::same(5);
        }
    }
    ctx.set_visuals(visuals);
}

/// Spacing and the workbench (non-editor) font size.
pub fn apply_style(ctx: &egui::Context, ui_font_size: f32) {
    use egui::{FontId, TextStyle};
    let size = ui_font_size.clamp(8.0, 32.0);
    ctx.global_style_mut(|style| {
        // The calm layout breathes a little more.
        let roomy = islands();
        style.spacing.item_spacing = if roomy { egui::vec2(8.0, 4.0) } else { egui::vec2(6.0, 3.0) };
        style.spacing.button_padding = if roomy { egui::vec2(8.0, 3.0) } else { egui::vec2(6.0, 2.0) };
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
