//! Font ligatures in the editor (`->`, `!=`, `=>` drawn as the font draws them), for fonts that
//! have them: Fira Code, JetBrains Mono, Cascadia Code and the like.
//!
//! egui lays text out a character at a time, so each line is shaped here with rustybuzz using
//! the editor's font. Where the font puts a different glyph than a character's own (a
//! ligature, or one of its parts), that character is drawn transparent and the font's glyph is
//! drawn in its place, rasterized once and kept.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use egui::{Color32, Pos2, Rect};
use resvg::tiny_skia;
use rustybuzz::ttf_parser;

/// What the font draws for a character instead of its own glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swap {
    /// The character's index in its line.
    pub index: usize,
    /// The glyph to draw; `None` for a character a ligature before it already covers.
    pub glyph: Option<u16>,
}

struct Font {
    data: Arc<egui::FontData>,
    /// Shaped lines: their text, and what's swapped.
    lines: HashMap<String, Arc<Vec<Swap>>>,
}

fn font() -> &'static Mutex<Option<Font>> {
    static FONT: OnceLock<Mutex<Option<Font>>> = OnceLock::new();
    FONT.get_or_init(|| Mutex::new(None))
}

/// Uses `data` for ligatures from now on; `None` (the built-in font, which has none) turns them off.
pub fn set_font(data: Option<Arc<egui::FontData>>) {
    let mut f = font().lock().unwrap_or_else(|e| e.into_inner());
    *f = data.map(|data| Font { data, lines: HashMap::new() });
    GLYPHS.with_borrow_mut(HashMap::clear);
}

/// The characters of `line` the font draws otherwise. Empty when there's no font set, or it
/// has no ligatures there.
pub fn swaps(line: &str) -> Arc<Vec<Swap>> {
    let mut guard = font().lock().unwrap_or_else(|e| e.into_inner());
    let Some(f) = guard.as_mut() else { return Arc::default() };
    // Ligatures join punctuation; a line with no two marks side by side has none.
    let chars: Vec<char> = line.chars().collect();
    if !chars.windows(2).any(|w| w[0].is_ascii_punctuation() && w[1].is_ascii_punctuation()) {
        return Arc::default();
    }
    if let Some(found) = f.lines.get(line) {
        return found.clone();
    }
    let found = Arc::new(shape(&f.data, line));
    if f.lines.len() > 20_000 {
        f.lines.clear();
    }
    f.lines.insert(line.to_owned(), found.clone());
    found
}

fn shape(data: &egui::FontData, line: &str) -> Vec<Swap> {
    let Some(face) = rustybuzz::Face::from_slice((*data.font).as_ref(), data.index) else { return Vec::new() };
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(line);
    let shaped = rustybuzz::shape(&face, &[], buffer);
    // Each character by its byte offset, which is what clusters are.
    let starts: Vec<(usize, char)> = line.char_indices().collect();
    let index_of = |byte: usize| starts.iter().position(|(b, _)| *b == byte);
    let mut covered = vec![false; starts.len()];
    let mut out = Vec::new();
    for info in shaped.glyph_infos() {
        let Some(i) = index_of(info.cluster as usize) else { continue };
        if covered[i] {
            continue;
        }
        covered[i] = true;
        let own = face.glyph_index(starts[i].1).map(|g| g.0);
        let glyph = info.glyph_id as u16;
        if own != Some(glyph) {
            out.push(Swap { index: i, glyph: Some(glyph) });
        }
    }
    // Characters joined into a glyph before them have none of their own.
    for (i, seen) in covered.iter().enumerate() {
        if !seen && !starts[i].1.is_whitespace() {
            out.push(Swap { index: i, glyph: None });
        }
    }
    out.sort_by_key(|s| s.index);
    out
}

/// A drawn glyph: its texture, and its left and top from the cell's left and the baseline.
type Drawn = (egui::TextureHandle, f32, f32);

thread_local! {
    /// Drawn glyphs: by (glyph, pixel size, cell width in pixels), the texture and where its
    /// left edge and top sit from the cell's left and the baseline, in pixels.
    static GLYPHS: std::cell::RefCell<HashMap<(u16, u32, u32), Option<Drawn>>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Draws `glyph` for the cell whose left edge is `x` on `baseline`, `size` points high.
pub fn paint(painter: &egui::Painter, glyph: u16, x: f32, baseline: f32, size: f32, cell_w: f32, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (size * ppp).round().max(1.0) as u32;
    let cell_px = (cell_w * ppp).round().max(1.0) as u32;
    let drawn = GLYPHS.with_borrow_mut(|cache| {
        cache
            .entry((glyph, px, cell_px))
            .or_insert_with(|| {
                let data = font().lock().unwrap_or_else(|e| e.into_inner()).as_ref()?.data.clone();
                let (image, left, top) = rasterize(&data, glyph, px as f32, cell_px as f32)?;
                let texture = ctx.load_texture(format!("ligature_{glyph}_{px}"), image, egui::TextureOptions::LINEAR);
                Some((texture, left, top))
            })
            .clone()
    });
    let Some((texture, left, top)) = drawn else { return };
    let size_pts = texture.size_vec2() / ppp;
    let rect = Rect::from_min_size(Pos2::new(x + left / ppp, baseline - top / ppp), size_pts);
    painter.image(texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), color);
}

/// The glyph as a white mask, and its left and top from the cell's left and the baseline.
/// The canvas spans four cells back and two ahead: a ligature's last glyph draws over the
/// characters before it (`<!--`, `...`).
fn rasterize(data: &egui::FontData, glyph: u16, px: f32, cell_px: f32) -> Option<(egui::ColorImage, f32, f32)> {
    let face = ttf_parser::Face::parse((*data.font).as_ref(), data.index).ok()?;
    let scale = px / face.units_per_em() as f32;
    let ascent = face.ascender() as f32 * scale;
    let descent = -(face.descender() as f32) * scale;
    const BACK: f32 = 4.0;
    let (w, h) = ((cell_px * (BACK + 3.0)).ceil() as u32, (ascent + descent).ceil().max(1.0) as u32 + 2);
    struct Outline(tiny_skia::PathBuilder);
    impl ttf_parser::OutlineBuilder for Outline {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to(x, y);
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to(x, y);
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0.quad_to(x1, y1, x, y);
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.cubic_to(x1, y1, x2, y2, x, y);
        }
        fn close(&mut self) {
            self.0.close();
        }
    }
    let mut outline = Outline(tiny_skia::PathBuilder::new());
    face.outline_glyph(ttf_parser::GlyphId(glyph), &mut outline)?;
    let path = outline.0.finish()?;
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color(tiny_skia::Color::WHITE);
    paint.anti_alias = true;
    // Font units to pixels, y up to y down, the cell's left `BACK` cells in, the baseline at `ascent`.
    let transform = tiny_skia::Transform::from_row(scale, 0.0, 0.0, -scale, cell_px * BACK, ascent + 1.0);
    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
    let image = egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data());
    Some((image, -cell_px * BACK, ascent + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With JetBrains Mono when it's at hand (set COIMAN_LIGATURE_FONT to its .ttf).
    #[test]
    fn a_ligature_font_swaps_arrows_and_not_letters() {
        let Some(path) = std::env::var_os("COIMAN_LIGATURE_FONT") else {
            eprintln!("COIMAN_LIGATURE_FONT not set: skipped");
            return;
        };
        let bytes = std::fs::read(path).unwrap();
        let data = egui::FontData::from_owned(bytes);
        let found = shape(&data, "a -> b != c");
        let at: Vec<usize> = found.iter().map(|s| s.index).collect();
        assert!(at.contains(&2) && at.contains(&3), "-> is swapped: {found:?}");
        assert!(at.contains(&7) && at.contains(&8), "!= is swapped: {found:?}");
        assert!(!at.contains(&0) && !at.contains(&5), "letters keep their glyphs");
        assert!(shape(&data, "plain words").is_empty());
        // Some parts are empty spacers (the next glyph reaches back over them); some have ink.
        let drawn = found.iter().filter_map(|s| s.glyph).filter_map(|g| rasterize(&data, g, 28.0, 17.0)).count();
        assert!(drawn >= 2, "the arrow and the not-equal sign draw");
    }
}
