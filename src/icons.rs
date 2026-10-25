//! Rust Code's own line icons, inspired by VS Code's codicons and drawn as
//! vector shapes, so they look sharp at any size and need no icon font.

use egui::{Color32, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, epaint::CubicBezierShape};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Files,
    Search,
    SourceControl,
    Terminal,
    Folder,
    FolderOpen,
    File,
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
        Icon::File => {
            pen.closed(&[(0.2, 0.08), (0.6, 0.08), (0.82, 0.3), (0.82, 0.92), (0.2, 0.92)]);
            pen.line(&[(0.6, 0.08), (0.6, 0.3), (0.82, 0.3)]);
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

/// Icon tint for a file in the explorer, by extension.
pub fn file_color(name: &str) -> Color32 {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Color32::from_rgb(0xDE, 0x8F, 0x5A),
        "toml" | "lock" => Color32::from_rgb(0x9C, 0x9C, 0x9C),
        "md" => Color32::from_rgb(0x51, 0x9A, 0xBA),
        "json" => Color32::from_rgb(0xCB, 0xCB, 0x41),
        "js" | "ts" | "tsx" | "jsx" => Color32::from_rgb(0xE8, 0xC8, 0x4A),
        "py" => Color32::from_rgb(0x4B, 0x8B, 0xBE),
        "html" | "htm" | "xml" => Color32::from_rgb(0xE3, 0x7B, 0x3C),
        "css" => Color32::from_rgb(0x56, 0x9C, 0xD6),
        "sii" | "sui" => Color32::from_rgb(0x8F, 0xBC, 0x5A),
        _ => Color32::from_rgb(0xB0, 0xB0, 0xB0),
    }
}
