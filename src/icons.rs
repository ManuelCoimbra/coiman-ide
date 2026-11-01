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

/// What a file is, for its icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Rust,
    CargoManifest,
    Lock,
    Toml,
    Markdown,
    Json,
    Yaml,
    JavaScript,
    TypeScript,
    Python,
    Java,
    C,
    Cpp,
    CSharp,
    Go,
    Html,
    Css,
    Shell,
    Git,
    Image,
    License,
    Wasm,
    Text,
    Other,
}

/// A file's kind from its name (`Cargo.toml`, `.gitignore`, `main.rs`…).
pub fn file_kind(name: &str) -> FileKind {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "cargo.toml" => return FileKind::CargoManifest,
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" => return FileKind::Git,
        _ => {}
    }
    if lower.starts_with("license") || lower == "copying" {
        return FileKind::License;
    }
    let ext = lower.rsplit_once('.').map_or("", |(_, e)| e);
    match ext {
        "rs" => FileKind::Rust,
        "lock" => FileKind::Lock,
        "toml" => FileKind::Toml,
        "md" | "markdown" => FileKind::Markdown,
        "json" | "jsonc" => FileKind::Json,
        "yaml" | "yml" => FileKind::Yaml,
        "js" | "mjs" | "cjs" | "jsx" => FileKind::JavaScript,
        "ts" | "mts" | "cts" | "tsx" => FileKind::TypeScript,
        "py" | "pyw" | "pyi" => FileKind::Python,
        "java" | "jar" => FileKind::Java,
        "c" | "h" => FileKind::C,
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "inl" => FileKind::Cpp,
        "cs" => FileKind::CSharp,
        "go" => FileKind::Go,
        "html" | "htm" | "xml" => FileKind::Html,
        "css" | "scss" => FileKind::Css,
        "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => FileKind::Shell,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "bmp" | "webp" => FileKind::Image,
        "wasm" | "wit" | "wat" => FileKind::Wasm,
        "txt" | "log" | "csv" => FileKind::Text,
        _ => FileKind::Other,
    }
}

fn kind_color(kind: FileKind) -> Color32 {
    match kind {
        FileKind::Rust | FileKind::CargoManifest => Color32::from_rgb(0xDE, 0x8F, 0x5A),
        FileKind::Lock | FileKind::Toml | FileKind::Text | FileKind::Other => Color32::from_rgb(0x9C, 0x9C, 0x9C),
        FileKind::Markdown => Color32::from_rgb(0x51, 0x9A, 0xBA),
        FileKind::Json | FileKind::License => Color32::from_rgb(0xCB, 0xCB, 0x41),
        FileKind::Yaml | FileKind::Image => Color32::from_rgb(0xA0, 0x74, 0xC4),
        FileKind::JavaScript => Color32::from_rgb(0xE8, 0xC8, 0x4A),
        FileKind::TypeScript => Color32::from_rgb(0x31, 0x78, 0xC6),
        FileKind::Python => Color32::from_rgb(0x4B, 0x8B, 0xBE),
        FileKind::Java => Color32::from_rgb(0xE7, 0x6F, 0x00),
        FileKind::C => Color32::from_rgb(0x5C, 0x8D, 0xBC),
        FileKind::Cpp => Color32::from_rgb(0x00, 0x86, 0xD4),
        FileKind::CSharp => Color32::from_rgb(0x9B, 0x4F, 0x96),
        FileKind::Go => Color32::from_rgb(0x00, 0xAD, 0xD8),
        FileKind::Html => Color32::from_rgb(0xE3, 0x7B, 0x3C),
        FileKind::Css => Color32::from_rgb(0x56, 0x9C, 0xD6),
        FileKind::Shell => Color32::from_rgb(0x89, 0xD1, 0x85),
        FileKind::Git => Color32::from_rgb(0xF0, 0x50, 0x33),
        FileKind::Wasm => Color32::from_rgb(0x65, 0x4F, 0xF0),
    }
}

/// Draws the icon for file `name`: a glyph per file kind in the kind's
/// color (Seti-style), drawn as shapes like the other icons.
pub fn paint_file(painter: &egui::Painter, rect: Rect, name: &str) {
    let kind = file_kind(name);
    let color = kind_color(kind);
    let width = (rect.width() / 12.0).clamp(1.0, 2.0);
    let pen = Pen { painter, rect, stroke: Stroke::new(width, color) };
    let dot = |x: f32, y: f32, r: f32| painter.circle_filled(pen.p(x, y), rect.width() * r, color);
    // Short text in the icon box (badges, braces).
    let text = |s: &str, scale: f32, c: Color32| {
        let font = egui::FontId::monospace(rect.height() * scale);
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, s, font, c);
    };
    let badge = |s: &str| {
        painter.rect_filled(rect.shrink(rect.width() * 0.08), rect.width() * 0.12, color);
        text(s, 0.5, Color32::from_rgb(0x1E, 0x1E, 0x1E));
    };
    let page = || {
        pen.closed(&[(0.2, 0.08), (0.6, 0.08), (0.82, 0.3), (0.82, 0.92), (0.2, 0.92)]);
        pen.line(&[(0.6, 0.08), (0.6, 0.3), (0.82, 0.3)]);
    };
    match kind {
        FileKind::Rust => {
            // A cog: a ring with eight teeth.
            pen.circle(0.5, 0.5, 0.24);
            for i in 0..8 {
                let (s, c) = (i as f32 * std::f32::consts::FRAC_PI_4).sin_cos();
                pen.line(&[(0.5 + c * 0.24, 0.5 + s * 0.24), (0.5 + c * 0.4, 0.5 + s * 0.4)]);
            }
            dot(0.5, 0.5, 0.08);
        }
        FileKind::CargoManifest => {
            // A crate.
            pen.closed(&[(0.5, 0.1), (0.88, 0.3), (0.5, 0.5), (0.12, 0.3)]);
            pen.line(&[(0.12, 0.3), (0.12, 0.72), (0.5, 0.92), (0.88, 0.72), (0.88, 0.3)]);
            pen.line(&[(0.5, 0.5), (0.5, 0.92)]);
        }
        FileKind::Lock => {
            painter.rect_filled(Rect::from_min_max(pen.p(0.18, 0.44), pen.p(0.82, 0.9)), rect.width() * 0.08, color);
            pen.arc(0.5, 0.4, 0.2, std::f32::consts::PI, 2.0 * std::f32::consts::PI);
            pen.line(&[(0.3, 0.4), (0.3, 0.46)]);
            pen.line(&[(0.7, 0.4), (0.7, 0.46)]);
        }
        FileKind::Toml => {
            // Settings sliders.
            for (y, knob) in [(0.24, 0.65), (0.5, 0.35), (0.76, 0.58)] {
                pen.line(&[(0.12, y), (0.88, y)]);
                dot(knob, y, 0.1);
            }
        }
        FileKind::Markdown => {
            pen.closed(&[(0.06, 0.24), (0.94, 0.24), (0.94, 0.76), (0.06, 0.76)]);
            pen.line(&[(0.2, 0.64), (0.2, 0.36), (0.33, 0.5), (0.46, 0.36), (0.46, 0.64)]);
            pen.line(&[(0.7, 0.34), (0.7, 0.64)]);
            pen.line(&[(0.6, 0.54), (0.7, 0.66), (0.8, 0.54)]);
        }
        FileKind::Json => text("{}", 0.8, color),
        FileKind::Yaml => {
            for (y, x0) in [(0.22, 0.12), (0.4, 0.3), (0.58, 0.3), (0.76, 0.12)] {
                pen.line(&[(x0, y), (0.88, y)]);
            }
        }
        FileKind::JavaScript => badge("JS"),
        FileKind::TypeScript => badge("TS"),
        FileKind::Python => badge("PY"),
        FileKind::C => badge("C"),
        FileKind::Cpp => badge("C+"),
        FileKind::CSharp => badge("C#"),
        FileKind::Go => badge("GO"),
        FileKind::Java => {
            // A coffee cup with steam.
            pen.closed(&[(0.18, 0.46), (0.74, 0.46), (0.68, 0.86), (0.24, 0.86)]);
            pen.arc(0.76, 0.6, 0.1, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
            pen.line(&[(0.38, 0.38), (0.34, 0.26), (0.4, 0.12)]);
            pen.line(&[(0.54, 0.38), (0.5, 0.26), (0.56, 0.12)]);
        }
        FileKind::Wasm => badge("WA"),
        FileKind::Html => {
            pen.line(&[(0.36, 0.26), (0.12, 0.5), (0.36, 0.74)]);
            pen.line(&[(0.64, 0.26), (0.88, 0.5), (0.64, 0.74)]);
        }
        FileKind::Css => text("#", 0.9, color),
        FileKind::Shell => {
            pen.line(&[(0.14, 0.28), (0.4, 0.5), (0.14, 0.72)]);
            pen.line(&[(0.48, 0.74), (0.86, 0.74)]);
        }
        FileKind::Git => {
            // The Git diamond with a branch.
            pen.closed(&[(0.5, 0.06), (0.94, 0.5), (0.5, 0.94), (0.06, 0.5)]);
            pen.line(&[(0.4, 0.34), (0.4, 0.68)]);
            pen.line(&[(0.4, 0.38), (0.63, 0.52)]);
            for (x, y) in [(0.4, 0.34), (0.4, 0.68), (0.63, 0.52)] {
                dot(x, y, 0.07);
            }
        }
        FileKind::Image => {
            pen.closed(&[(0.1, 0.18), (0.9, 0.18), (0.9, 0.82), (0.1, 0.82)]);
            pen.line(&[(0.14, 0.78), (0.4, 0.48), (0.58, 0.66), (0.7, 0.54), (0.86, 0.72)]);
            dot(0.68, 0.36, 0.07);
        }
        FileKind::License => {
            page();
            pen.circle(0.5, 0.56, 0.14);
            pen.line(&[(0.42, 0.68), (0.38, 0.86)]);
            pen.line(&[(0.58, 0.68), (0.62, 0.86)]);
        }
        FileKind::Text => {
            page();
            for y in [0.46, 0.6, 0.74] {
                pen.line(&[(0.32, y), (0.7, y)]);
            }
        }
        FileKind::Other => page(),
    }
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
    fn file_kinds_by_name_and_extension() {
        assert_eq!(file_kind("main.rs"), FileKind::Rust);
        assert_eq!(file_kind("Cargo.toml"), FileKind::CargoManifest);
        assert_eq!(file_kind("rustfmt.toml"), FileKind::Toml);
        assert_eq!(file_kind("Cargo.lock"), FileKind::Lock);
        assert_eq!(file_kind(".gitignore"), FileKind::Git);
        assert_eq!(file_kind("LICENSE-MIT"), FileKind::License);
        assert_eq!(file_kind("README.MD"), FileKind::Markdown);
        assert_eq!(file_kind("Makefile"), FileKind::Other);
        assert_eq!(file_kind("Main.java"), FileKind::Java);
        assert_eq!(file_kind("vec.hpp"), FileKind::Cpp);
        assert_eq!(file_kind("Program.cs"), FileKind::CSharp);
    }
}
