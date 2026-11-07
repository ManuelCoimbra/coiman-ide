//! Coiman IDE's own line icons, inspired by VS Code's codicons and drawn as
//! vector shapes, so they look sharp at any size and need no icon font.

use coiman_core::project::project_tree::{ClassKind, FileRole};
use std::collections::HashMap;

use coiman_core::appearance::file_icons;
use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, epaint::CubicBezierShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Files,
    Search,
    SourceControl,
    Terminal,
    Folder,
    FolderOpen,
    Alert,
    Lines,
    Menu,
    Gear,
    CollapseAll,
    Locate,
    ChevronRight,
    ChevronDown,
    Refresh,
    ArrowUp,
    ArrowDown,
    Close,
    /// Unsaved-changes dot.
    Dot,
    Plus,
    Minus,
    /// Discard / revert (an undo arrow).
    Discard,
    ZoomIn,
    /// Run (a right-pointing triangle).
    Play,
    Stop,
    Check,
    /// A test flask, for the Testing view.
    Beaker,
    /// Skipped/ignored test.
    Skip,
    Bug,
    Pause,
    StepOver,
    StepInto,
    StepOut,
    /// Plugins (four blocks, one lifted out).
    Extensions,
    Trash,
    /// A light bulb: quick fixes are on offer.
    Bulb,
}

/// Icon-sized painter helpers: coordinates are fractions of the icon box.
struct Pen<'a> {
    painter: &'a egui::Painter,
    rect: Rect,
    stroke: Stroke,
}

impl Pen<'_> {
    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.rect.min + Vec2::new(x * self.rect.width(), y * self.rect.height())
    }

    fn line(&self, points: &[(f32, f32)]) {
        let pts: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::line(pts, self.stroke));
    }

    fn closed(&self, points: &[(f32, f32)]) {
        let pts: Vec<Pos2> = points.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(Shape::closed_line(pts, self.stroke));
    }

    fn circle(&self, x: f32, y: f32, r: f32) {
        self.painter.circle_stroke(self.p(x, y), r * self.rect.width(), self.stroke);
    }

    fn arc(&self, cx: f32, cy: f32, r: f32, from: f32, to: f32) -> Pos2 {
        let steps = 20;
        let pts: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let a = from + (to - from) * i as f32 / steps as f32;
                self.p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        let end = *pts.last().expect("steps > 0");
        self.painter.add(Shape::line(pts, self.stroke));
        end
    }
}

/// Paints `icon` inside `rect` in `color`.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let width = (rect.width() / 14.0).clamp(1.0, 2.0);
    let pen = Pen { painter, rect, stroke: Stroke::new(width, color) };
    use std::f32::consts::PI;
    match icon {
        Icon::Files => {
            // Two pages, the front one with a folded corner.
            pen.line(&[(0.36, 0.2), (0.36, 0.08), (0.7, 0.08), (0.88, 0.26), (0.88, 0.76), (0.76, 0.76)]);
            pen.closed(&[(0.14, 0.24), (0.5, 0.24), (0.68, 0.42), (0.68, 0.92), (0.14, 0.92)]);
            pen.line(&[(0.5, 0.24), (0.5, 0.42), (0.68, 0.42)]);
        }
        Icon::Search => {
            pen.circle(0.58, 0.4, 0.27);
            pen.line(&[(0.38, 0.6), (0.1, 0.88)]);
        }
        Icon::SourceControl => {
            pen.circle(0.3, 0.18, 0.1);
            pen.circle(0.3, 0.82, 0.1);
            pen.circle(0.72, 0.32, 0.1);
            pen.line(&[(0.3, 0.28), (0.3, 0.72)]);
            let curve = CubicBezierShape::from_points_stroke(
                [pen.p(0.72, 0.42), pen.p(0.72, 0.62), pen.p(0.3, 0.5), pen.p(0.3, 0.72)],
                false,
                Color32::TRANSPARENT,
                pen.stroke,
            );
            painter.add(curve);
        }
        Icon::Terminal => {
            painter.rect_stroke(
                Rect::from_min_max(pen.p(0.06, 0.16), pen.p(0.94, 0.84)),
                rect.width() * 0.08,
                pen.stroke,
                egui::StrokeKind::Middle,
            );
            pen.line(&[(0.24, 0.36), (0.4, 0.5), (0.24, 0.64)]);
            pen.line(&[(0.48, 0.66), (0.74, 0.66)]);
        }
        Icon::Folder => {
            pen.closed(&[(0.06, 0.2), (0.38, 0.2), (0.48, 0.32), (0.94, 0.32), (0.94, 0.84), (0.06, 0.84)]);
        }
        Icon::FolderOpen => {
            pen.line(&[(0.18, 0.84), (0.06, 0.84), (0.06, 0.2), (0.38, 0.2), (0.48, 0.32), (0.82, 0.32), (0.82, 0.46)]);
            pen.closed(&[(0.18, 0.46), (0.98, 0.46), (0.84, 0.84), (0.04, 0.84)]);
        }
        Icon::Alert => {
            pen.closed(&[(0.5, 0.12), (0.92, 0.86), (0.08, 0.86)]);
            pen.line(&[(0.5, 0.4), (0.5, 0.62)]);
            pen.line(&[(0.5, 0.71), (0.5, 0.74)]);
        }
        Icon::Lines => {
            for (y, end) in [(0.2, 0.86), (0.4, 0.6), (0.6, 0.86), (0.8, 0.5)] {
                pen.line(&[(0.14, y), (end, y)]);
            }
        }
        Icon::Menu => {
            for y in [0.25, 0.5, 0.75] {
                pen.line(&[(0.14, y), (0.86, y)]);
            }
        }
        Icon::Gear => {
            pen.circle(0.5, 0.5, 0.16);
            pen.circle(0.5, 0.5, 0.31);
            for tooth in 0..8 {
                let angle = tooth as f32 * PI / 4.0;
                let (x, y) = (angle.cos(), angle.sin());
                pen.line(&[(0.5 + 0.31 * x, 0.5 + 0.31 * y), (0.5 + 0.44 * x, 0.5 + 0.44 * y)]);
            }
        }
        Icon::CollapseAll => {
            pen.closed(&[(0.3, 0.1), (0.9, 0.1), (0.9, 0.7), (0.3, 0.7)]);
            pen.line(&[(0.1, 0.3), (0.1, 0.9), (0.7, 0.9)]);
            pen.line(&[(0.45, 0.4), (0.75, 0.4)]);
        }
        Icon::Locate => {
            pen.circle(0.5, 0.5, 0.28);
            pen.line(&[(0.5, 0.06), (0.5, 0.3)]);
            pen.line(&[(0.5, 0.7), (0.5, 0.94)]);
            pen.line(&[(0.06, 0.5), (0.3, 0.5)]);
            pen.line(&[(0.7, 0.5), (0.94, 0.5)]);
        }
        Icon::ChevronRight => pen.line(&[(0.38, 0.24), (0.64, 0.5), (0.38, 0.76)]),
        Icon::ChevronDown => pen.line(&[(0.24, 0.38), (0.5, 0.64), (0.76, 0.38)]),
        Icon::Refresh => {
            let end = pen.arc(0.5, 0.52, 0.32, -0.35 * PI, 1.35 * PI);
            let _ = end;
            // Arrowhead at the start of the arc (top right).
            pen.line(&[(0.62, 0.1), (0.76, 0.24), (0.58, 0.32)]);
        }
        Icon::ArrowUp => {
            pen.line(&[(0.5, 0.86), (0.5, 0.16)]);
            pen.line(&[(0.22, 0.42), (0.5, 0.14), (0.78, 0.42)]);
        }
        Icon::ArrowDown => {
            pen.line(&[(0.5, 0.14), (0.5, 0.84)]);
            pen.line(&[(0.22, 0.58), (0.5, 0.86), (0.78, 0.58)]);
        }
        Icon::Close => {
            pen.line(&[(0.24, 0.24), (0.76, 0.76)]);
            pen.line(&[(0.76, 0.24), (0.24, 0.76)]);
        }
        Icon::Dot => {
            painter.circle_filled(rect.center(), rect.width() * 0.22, color);
        }
        Icon::Plus => {
            pen.line(&[(0.5, 0.18), (0.5, 0.82)]);
            pen.line(&[(0.18, 0.5), (0.82, 0.5)]);
        }
        Icon::Minus => pen.line(&[(0.18, 0.5), (0.82, 0.5)]),
        Icon::Discard => {
            // An arrow curling back to the left.
            pen.arc(0.52, 0.56, 0.3, -0.5 * PI, 0.75 * PI);
            pen.line(&[(0.36, 0.1), (0.52, 0.26), (0.36, 0.42)]);
        }
        Icon::Play => {
            painter.add(Shape::convex_polygon(
                vec![pen.p(0.28, 0.16), pen.p(0.84, 0.5), pen.p(0.28, 0.84)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_min_max(pen.p(0.24, 0.24), pen.p(0.76, 0.76)), 1.0, color);
        }
        Icon::Check => pen.line(&[(0.16, 0.52), (0.4, 0.76), (0.86, 0.26)]),
        Icon::Beaker => {
            pen.line(&[(0.38, 0.08), (0.38, 0.4), (0.12, 0.86), (0.88, 0.86), (0.62, 0.4), (0.62, 0.08)]);
            pen.line(&[(0.3, 0.08), (0.7, 0.08)]);
            pen.line(&[(0.24, 0.64), (0.76, 0.64)]);
        }
        Icon::Skip => {
            pen.circle(0.5, 0.5, 0.3);
            pen.line(&[(0.29, 0.71), (0.71, 0.29)]);
        }
        Icon::Extensions => {
            pen.closed(&[(0.1, 0.42), (0.38, 0.42), (0.38, 0.7), (0.1, 0.7)]);
            pen.closed(&[(0.1, 0.7), (0.38, 0.7), (0.38, 0.94), (0.1, 0.94)]);
            pen.closed(&[(0.38, 0.7), (0.66, 0.7), (0.66, 0.94), (0.38, 0.94)]);
            pen.closed(&[(0.56, 0.1), (0.86, 0.1), (0.86, 0.4), (0.56, 0.4)]);
        }
        Icon::Trash => {
            pen.line(&[(0.16, 0.24), (0.84, 0.24)]);
            pen.line(&[(0.4, 0.24), (0.4, 0.12), (0.6, 0.12), (0.6, 0.24)]);
            pen.closed(&[(0.24, 0.24), (0.3, 0.9), (0.7, 0.9), (0.76, 0.24)]);
            pen.line(&[(0.43, 0.38), (0.43, 0.76)]);
            pen.line(&[(0.57, 0.38), (0.57, 0.76)]);
        }
        Icon::Bulb => {
            pen.arc(0.5, 0.38, 0.27, PI * 0.8, PI * 2.2);
            pen.line(&[(0.28, 0.54), (0.38, 0.72), (0.62, 0.72), (0.72, 0.54)]);
            pen.line(&[(0.4, 0.86), (0.6, 0.86)]);
        }
        Icon::Bug => {
            pen.circle(0.5, 0.58, 0.24);
            pen.circle(0.5, 0.26, 0.1);
            pen.line(&[(0.5, 0.36), (0.5, 0.82)]);
            for y in [0.46, 0.62, 0.78] {
                pen.line(&[(0.26, y), (0.1, y - 0.06)]);
                pen.line(&[(0.74, y), (0.9, y - 0.06)]);
            }
        }
        Icon::Pause => {
            painter.rect_filled(Rect::from_min_max(pen.p(0.26, 0.2), pen.p(0.42, 0.8)), 1.0, color);
            painter.rect_filled(Rect::from_min_max(pen.p(0.58, 0.2), pen.p(0.74, 0.8)), 1.0, color);
        }
        Icon::StepOver => {
            pen.arc(0.5, 0.52, 0.32, std::f32::consts::PI, 2.0 * std::f32::consts::PI);
            pen.line(&[(0.66, 0.42), (0.82, 0.54), (0.94, 0.38)]);
            painter.circle_filled(pen.p(0.5, 0.84), rect.width() * 0.08, color);
        }
        Icon::StepInto => {
            pen.line(&[(0.5, 0.1), (0.5, 0.6)]);
            pen.line(&[(0.3, 0.42), (0.5, 0.62), (0.7, 0.42)]);
            painter.circle_filled(pen.p(0.5, 0.84), rect.width() * 0.08, color);
        }
        Icon::StepOut => {
            pen.line(&[(0.5, 0.64), (0.5, 0.12)]);
            pen.line(&[(0.3, 0.32), (0.5, 0.12), (0.7, 0.32)]);
            painter.circle_filled(pen.p(0.5, 0.84), rect.width() * 0.08, color);
        }
        Icon::ZoomIn => {
            pen.circle(0.42, 0.42, 0.28);
            pen.line(&[(0.62, 0.62), (0.9, 0.9)]);
            pen.line(&[(0.42, 0.3), (0.42, 0.54)]);
            pen.line(&[(0.3, 0.42), (0.54, 0.42)]);
        }
    }
}

/// An icon of `size` points in the layout.
pub fn show(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint(ui.painter(), rect, icon, color);
    response
}

/// A small frameless icon button with a hover highlight.
pub fn button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let size = ui.text_style_height(&egui::TextStyle::Body) + 2.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let visuals = ui.style().interact(&response);
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.weak_bg_fill);
    }
    paint(ui.painter(), rect.shrink(size * 0.15), icon, visuals.text_color());
    response.on_hover_text(tooltip)
}

thread_local! {
    /// Icons drawn so far, by name and size in pixels.
    static FILE_ICONS: std::cell::RefCell<HashMap<(&'static str, u32), Option<egui::TextureHandle>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Draws an icon's SVG at this many pixels square.
fn draw_svg(svg: &str, pixels: u32) -> Option<egui::ColorImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let scale = pixels as f32 / size.width().max(size.height());
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let (x, y) = ((pixels as f32 - size.width() * scale) / 2.0, (pixels as f32 - size.height() * scale) / 2.0);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale).post_translate(x, y), &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([pixels as usize; 2], pixmap.data()))
}

fn file_icon_texture(ctx: &egui::Context, icon: &'static str, pixels: u32) -> Option<egui::TextureHandle> {
    FILE_ICONS.with_borrow_mut(|cache| {
        cache
            .entry((icon, pixels))
            .or_insert_with(|| {
                let image = draw_svg(file_icons::svg(icon)?, pixels)?;
                Some(ctx.load_texture(format!("file_icon_{icon}_{pixels}"), image, egui::TextureOptions::LINEAR))
            })
            .clone()
    })
}

/// Draws the icon for a file. `path` may be just its name; relative to the project, it
/// also tells a file in a test folder. Tests get a flask, except where their icon says so.
pub fn paint_file(painter: &egui::Painter, rect: Rect, path: &str) {
    paint_source(painter, rect, path, FileRole::of_name(path));
}

/// Draws a file's icon as JetBrains IDEs do once they've read it: its type in a ring if
/// it's one, with a mark where a program starts or tests are kept.
pub fn paint_source(painter: &egui::Painter, rect: Rect, path: &str, role: FileRole) {
    if let Some(kind) = role.class {
        paint_class(painter, rect, kind);
    } else {
        paint_theme_icon(painter, rect, path);
    }
    let own_flask = role.class.is_none() && file_icons::icon_for(path, false).starts_with("test");
    if role.test && !own_flask {
        paint_test_mark(painter, rect);
    } else if role.runs {
        paint_run_mark(painter, rect);
    }
}

fn paint_class(painter: &egui::Painter, rect: Rect, kind: ClassKind) {
    let (letter, color) = match kind {
        ClassKind::Class => ("C", Color32::from_rgb(0x54, 0x8A, 0xF7)),
        ClassKind::Abstract => ("A", Color32::from_rgb(0x7F, 0x9A, 0xC9)),
        ClassKind::Interface => ("I", Color32::from_rgb(0x57, 0x96, 0x5C)),
        ClassKind::Enum => ("E", Color32::from_rgb(0xB1, 0x89, 0xF5)),
        ClassKind::Record => ("R", Color32::from_rgb(0x54, 0x8A, 0xF7)),
        ClassKind::Annotation => ("@", Color32::from_rgb(0xC7, 0x7D, 0xBB)),
        ClassKind::Object => ("O", Color32::from_rgb(0xD0, 0x8F, 0x3E)),
        ClassKind::Trait => ("T", Color32::from_rgb(0x57, 0x96, 0x5C)),
        ClassKind::Component => ("<>", Color32::from_rgb(0x3F, 0xB5, 0xC4)),
        ClassKind::Hook => ("H", Color32::from_rgb(0xC9, 0xA0, 0x3C)),
        ClassKind::Page => ("P", Color32::from_rgb(0x57, 0xB0, 0x6A)),
        ClassKind::Route => ("R", Color32::from_rgb(0xD0, 0x8F, 0x3E)),
        ClassKind::CssModule => ("M", Color32::from_rgb(0x9C, 0x6F, 0xD0)),
    };
    paint_ring(painter, rect, letter, color);
}

/// A letter in a ring: what kind of thing a name is, in the tree and in completion.
pub fn paint_ring(painter: &egui::Painter, rect: Rect, letter: &str, color: Color32) {
    let radius = rect.width() * 0.46;
    painter.circle(rect.center(), radius, color.gamma_multiply(0.18), Stroke::new(1.2, color));
    let font = egui::FontId::proportional(rect.width() * if letter.len() > 1 { 0.5 } else { 0.7 });
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        font,
        color.lerp_to_gamma(crate::theme::colors().text, 0.45),
    );
}

fn paint_theme_icon(painter: &egui::Painter, rect: Rect, path: &str) {
    let ctx = painter.ctx().clone();
    let pal = crate::theme::colors();
    let light = pal.editor_bg.r() as u32 + pal.editor_bg.g() as u32 + pal.editor_bg.b() as u32 > 384;
    let icon = file_icons::icon_for(path, light);
    let pixels = (rect.width() * ctx.pixels_per_point()).round().max(1.0) as u32;
    if let Some(texture) = file_icon_texture(&ctx, icon, pixels) {
        let whole = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(texture.id(), rect, whole, Color32::WHITE);
    }
}

/// A green triangle at an icon's corner: a program starts here.
fn paint_run_mark(painter: &egui::Painter, rect: Rect) {
    let size = rect.width() * 0.48;
    let mark = Rect::from_center_size(rect.right_bottom() - Vec2::splat(size * 0.3), Vec2::splat(size));
    painter.circle_filled(mark.center(), size * 0.56, crate::theme::colors().editor_bg);
    let at = |x: f32, y: f32| mark.min + Vec2::new(x, y) * size;
    let points = vec![at(0.22, 0.08), at(0.92, 0.5), at(0.22, 0.92)];
    painter.add(egui::Shape::convex_polygon(points, Color32::from_rgb(0x57, 0xC0, 0x6A), Stroke::NONE));
}

/// A flask at an icon's corner: this holds tests.
pub fn paint_test_mark(painter: &egui::Painter, rect: Rect) {
    let size = rect.width() * 0.62;
    let flask = Rect::from_center_size(rect.right_bottom() - Vec2::splat(size * 0.3), Vec2::splat(size));
    painter.circle_filled(flask.center(), size * 0.56, crate::theme::colors().editor_bg);
    paint(painter, flask, Icon::Beaker, Color32::from_rgb(0x73, 0xC9, 0x91));
}

/// A file icon allocated in a layout, knowing what the file is.
pub fn show_source(ui: &mut Ui, path: &str, role: FileRole, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_source(ui.painter(), rect, path, role);
    response
}

/// A file icon allocated in a layout.
pub fn show_file(ui: &mut Ui, name: &str, size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_file(ui.painter(), rect, name);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_packed_icon_draws() {
        let mut count = 0;
        for (name, svg) in file_icons::all_svgs() {
            let image = draw_svg(svg, 16).unwrap_or_else(|| panic!("{name} didn't draw"));
            assert!(image.pixels.iter().any(|p| p.a() > 0), "{name} is empty");
            count += 1;
        }
        assert!(count > 500, "{count}");
    }
}
