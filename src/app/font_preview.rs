//! A card showing the fonts a `font-family` list names, each in itself: what a browser on
//! this computer would use, and what it falls back to for those that aren't installed.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};
use resvg::usvg::fontdb::{Database, Family, Query};
use resvg::{tiny_skia, usvg};
use rust_code_core::css::{self, FontList};

use super::{CodeApp, repaint};
use crate::editor_view::{Decorations, WordImage};
use crate::theme;

/// What the samples say.
const SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";
/// Sample size in points.
const SAMPLE_SIZE: f32 = 18.0;
/// Room the text is laid out in: wide enough for any font's sample, then cropped to it.
const LAYOUT_WIDTH: f32 = 2000.0;
/// The card's widest sample.
const SAMPLE_WIDTH: f32 = 640.0;

/// One family of the list, as this computer has it.
struct Row {
    family: String,
    /// The face drawn: the family itself, what a generic family stands for here, or
    /// for a missing one, the fallback.
    face: Option<String>,
    installed: bool,
    generic: bool,
    image: Option<egui::ColorImage>,
}

struct Card {
    uid: u64,
    version: u64,
    list: FontList,
    anchor: egui::Pos2,
    rows: Vec<(Row, Option<egui::TextureHandle>)>,
    waiting: Option<Receiver<Vec<Row>>>,
    rect: Option<egui::Rect>,
    /// Opened by command: stays until the cursor leaves the list.
    sticky: bool,
}

/// Font names as pictures, by line.
type Pictures = Arc<HashMap<usize, Vec<WordImage>>>;
/// Names drawn off the UI thread, for a look: each as written, and its picture if installed.
type Drawn = (u64, Vec<(String, Option<egui::ColorImage>)>);

#[derive(Default)]
pub(super) struct FontPreview {
    card: Option<Card>,
    /// Font names drawn in their own font, by tab: (version, look) and the pictures by line.
    inline: HashMap<u64, ((u64, u64), Pictures)>,
    /// Each family's name drawn in itself, for a look; `None` when it isn't installed.
    names: HashMap<(String, u64), Option<(egui::TextureHandle, egui::Vec2)>>,
    /// Names being drawn, off the UI thread.
    drawing: Option<Receiver<Drawn>>,
}

/// Where the text's baseline sits in a picture from `draw_text`, as a share of its height.
const BASELINE: f32 = 1.15 / 1.6;

/// A family's name as `written` (quotes included) in the face it stands for here (a
/// generic one in what it maps to), in white so it can take the syntax color when painted;
/// `None` when the font isn't installed: a missing font is left as written.
fn draw_name(db: &Database, family: &str, written: &str, size: f32, scale: f32) -> Option<egui::ColorImage> {
    let lower = family.to_ascii_lowercase();
    let face = if css::GENERIC_FAMILIES.contains(&lower.as_str()) {
        generic_face(db, &lower)?
    } else {
        let id = db.query(&Query { families: &[Family::Name(family)], ..Default::default() })?;
        db.face(id)?.families.first()?.0.clone()
    };
    draw_text(written, &face, [255, 255, 255], size, scale)
}

/// What a generic family is on this computer, as browsers on Windows, macOS and Linux
/// map them, else fontdb's choice.
fn generic_face(db: &Database, name: &str) -> Option<String> {
    let first = |names: &[&str]| {
        names.iter().find_map(|n| {
            db.query(&Query { families: &[Family::Name(n)], ..Default::default() }).map(|_| (*n).to_owned())
        })
    };
    let generic = |family: Family| {
        let id = db.query(&Query { families: &[family], ..Default::default() })?;
        db.face(id).and_then(|f| f.families.first()).map(|(n, _)| n.clone())
    };
    match name {
        "system-ui" | "ui-sans-serif" | "ui-rounded" => {
            first(&["Segoe UI", "SF Pro Text", "Cantarell", "Ubuntu"]).or_else(|| generic(Family::SansSerif))
        }
        "ui-monospace" => {
            first(&["Cascadia Mono", "SF Mono", "Consolas", "DejaVu Sans Mono"]).or_else(|| generic(Family::Monospace))
        }
        "ui-serif" | "serif" => generic(Family::Serif),
        "sans-serif" => generic(Family::SansSerif),
        "monospace" => generic(Family::Monospace),
        "cursive" => generic(Family::Cursive),
        "fantasy" => generic(Family::Fantasy),
        "emoji" => first(&["Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji"]),
        "math" => first(&["Cambria Math", "STIX Two Math", "Latin Modern Math"]).or_else(|| generic(Family::Serif)),
        "fangsong" => first(&["FangSong", "STFangsong"]).or_else(|| generic(Family::Serif)),
        _ => None,
    }
}

/// The sample drawn in `face`, at `scale` pixels per point.
fn draw(face: &str, color: [u8; 3], scale: f32) -> Option<egui::ColorImage> {
    draw_text(SAMPLE, face, color, SAMPLE_SIZE, scale)
}

/// `text` drawn in `face` at `size` points, `scale` pixels per point, cropped to its width.
fn draw_text(text: &str, face: &str, color: [u8; 3], size: f32, scale: f32) -> Option<egui::ColorImage> {
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;").replace('\'', "&apos;");
    let (w, h) = (LAYOUT_WIDTH, size * 1.6);
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\"><text x=\"0\" y=\"{}\" font-family=\"'{}'\" font-size=\"{size}\" fill=\"#{:02x}{:02x}{:02x}\">{}</text></svg>",
        size * 1.15,
        escape(face),
        color[0],
        color[1],
        color[2],
        escape(text),
    );
    let options = usvg::Options { fontdb: super::svg_preview::fonts(), resources_dir: None, ..Default::default() };
    let tree = usvg::Tree::from_str(&svg, &options).ok()?;
    // As wide as this font sets the sample, so a wide one isn't cut off.
    let width = (tree.root().abs_bounding_box().right() + crate::editor_view::WORD_IMAGE_MARGIN).clamp(1.0, w);
    let (pw, ph) = ((width * scale).ceil() as u32, (h * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph)?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Some(egui::ColorImage::from_rgba_premultiplied([pw as usize, ph as usize], pixmap.data()))
}

/// Each family of the list, found and drawn. Missing ones are drawn in the fallback a
/// browser would reach: the next family in the list that's here, else sans-serif.
fn rows_for(families: &[String], color: [u8; 3], scale: f32) -> Vec<Row> {
    let db = super::svg_preview::fonts();
    let found: Vec<(bool, Option<String>)> = families
        .iter()
        .map(|family| {
            let lower = family.to_ascii_lowercase();
            if css::GENERIC_FAMILIES.contains(&lower.as_str()) {
                return (true, generic_face(&db, &lower));
            }
            let id = db.query(&Query { families: &[Family::Name(family)], ..Default::default() });
            let face = id.and_then(|id| db.face(id)).and_then(|f| f.families.first()).map(|(n, _)| n.clone());
            (false, face)
        })
        .collect();
    let fallback_after =
        |i: usize| found[i + 1..].iter().find_map(|(_, face)| face.clone()).or_else(|| generic_face(&db, "sans-serif"));
    families
        .iter()
        .enumerate()
        .map(|(i, family)| {
            let (generic, face) = found[i].clone();
            let installed = face.is_some();
            let drawn = face.clone().or_else(|| fallback_after(i));
            let image = drawn.as_deref().and_then(|f| draw(f, color, scale));
            Row { family: family.clone(), face: drawn, installed, generic, image }
        })
        .collect()
}

impl CodeApp {
    /// Font names in a style sheet or page drawn in their own font, over their characters.
    pub(super) fn font_names_decorate(&mut self, ctx: &egui::Context, i: usize, deco: &mut Decorations) {
        if !self.settings.editor.font_previews {
            return;
        }
        let Some(doc) = self.docs.get(i) else { return };
        if doc.large {
            return;
        }
        // What the pictures depend on: theme, code font size and screen scale.
        let look = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (theme::generation(), self.font_size.to_bits(), ctx.pixels_per_point().to_bits()).hash(&mut h);
            h.finish()
        };
        // Pictures drawn since: take them in.
        if let Some(rx) = &self.font_preview.drawing
            && let Ok((drawn_look, drawn)) = rx.try_recv()
        {
            self.font_preview.drawing = None;
            for (family, image) in drawn {
                let texture = image.map(|image| {
                    let size = egui::vec2(image.size[0] as f32, image.size[1] as f32) / ctx.pixels_per_point();
                    (ctx.load_texture(format!("font_name_{family}"), image, egui::TextureOptions::LINEAR), size)
                });
                self.font_preview.names.insert((family, drawn_look), texture);
            }
            self.font_preview.inline.clear();
        }
        let (uid, version) = (doc.uid, doc.buffer.version());
        if let Some((key, lines)) = self.font_preview.inline.get(&uid)
            && *key == (version, look)
        {
            deco.word_images = Some(lines.clone());
            return;
        }
        let text = doc.text();
        let chars: Vec<char> = text.chars().collect();
        // With their quotes, which are drawn in the same font: the room a narrower font
        // leaves is then after the name and its quotes, before the comma.
        let found: Vec<(usize, usize, String, String)> = css::font_names(&doc.extension(), &text)
            .into_iter()
            .map(|(start, end, family)| {
                let quote = start.checked_sub(1).and_then(|i| chars.get(i)).copied();
                let (start, end) = match quote {
                    Some(q @ ('"' | '\'')) if chars.get(end) == Some(&q) => (start - 1, end + 1),
                    _ => (start, end),
                };
                (start, end, chars[start..end].iter().collect(), family)
            })
            .collect();
        if found.is_empty() {
            return;
        }
        // Names not drawn yet: drawn off to the side, then this runs again.
        let missing: Vec<(String, String)> = {
            let mut m: Vec<(String, String)> = found
                .iter()
                .map(|(_, _, written, family)| (written.clone(), family.clone()))
                .filter(|(written, _)| !self.font_preview.names.contains_key(&(written.clone(), look)))
                .collect();
            m.sort();
            m.dedup();
            m
        };
        if !missing.is_empty() {
            if self.font_preview.drawing.is_none() {
                let (size, scale) = (self.font_size, ctx.pixels_per_point());
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                std::thread::spawn(move || {
                    let db = super::svg_preview::fonts();
                    let drawn = missing.into_iter().map(|(written, family)| {
                        let image = draw_name(&db, &family, &written, size, scale);
                        (written, image)
                    });
                    let _ = tx.send((look, drawn.collect()));
                    wake();
                });
                self.font_preview.drawing = Some(rx);
            }
            return;
        }
        // Char offsets to line and column.
        let mut starts = vec![0];
        starts.extend(chars.iter().enumerate().filter(|(_, c)| **c == '\n').map(|(i, _)| i + 1));
        let mut lines: HashMap<usize, Vec<WordImage>> = HashMap::new();
        for (start, end, written, _) in found {
            let Some(Some((texture, size))) = self.font_preview.names.get(&(written, look)) else { continue };
            let line = starts.partition_point(|&s| s <= start) - 1;
            lines.entry(line).or_default().push(WordImage {
                start: start - starts[line],
                end: end - starts[line],
                texture: texture.id(),
                size: *size,
                baseline: size.y * BASELINE,
            });
        }
        let lines = Arc::new(lines);
        self.font_preview.inline.insert(uid, ((version, look), lines.clone()));
        deco.word_images = Some(lines);
    }

    /// Opens the card when the pointer rests on a font list; closes it when it leaves.
    pub(super) fn font_preview_hover(&mut self, ctx: &egui::Context, hover: Option<(usize, egui::Pos2)>) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let pointer = ctx.pointer_hover_pos();
        if let Some(card) = &self.font_preview.card {
            let over_card = card.rect.zip(pointer).is_some_and(|(r, p)| r.expand(12.0).contains(p));
            let on_list =
                hover.is_some_and(|(pos, _)| card.uid == uid && (card.list.start..=card.list.end).contains(&pos));
            let cursor_in = (card.list.start..=card.list.end).contains(&doc.buffer.primary().head);
            let stays =
                card.uid == uid && card.version == version && (over_card || on_list || (card.sticky && cursor_in));
            if !stays {
                self.font_preview.card = None;
            }
        }
        if self.font_preview.card.is_some() || doc.large {
            return;
        }
        let Some((pos, at)) = hover else { return };
        if let Some(list) = css::font_families_at(&doc.extension(), &doc.text(), pos) {
            self.open_font_card(ctx, list, at + egui::vec2(0.0, 18.0), false);
        }
    }

    /// `CSS: Preview Font Family`: the card for the list under the cursor.
    pub(super) fn preview_font_at_cursor(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let head = doc.buffer.primary().head;
        match css::font_families_at(&doc.extension(), &doc.text(), head) {
            Some(list) => {
                let at = self
                    .cursor_on_screen()
                    .map_or(egui::pos2(200.0, 120.0), |r| r.left_bottom() + egui::vec2(0.0, 4.0));
                self.open_font_card(ctx, list, at, true);
            }
            None => self.status = Some("Put the cursor in a font-family list to preview its fonts".to_owned()),
        }
    }

    fn open_font_card(&mut self, ctx: &egui::Context, list: FontList, anchor: egui::Pos2, sticky: bool) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let text = theme::colors().text;
        let (color, scale) = ([text.r(), text.g(), text.b()], ctx.pixels_per_point());
        let families = list.families.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        // The system's fonts are read the first time: off the UI thread.
        std::thread::spawn(move || {
            let _ = tx.send(rows_for(&families, color, scale));
            wake();
        });
        self.font_preview.card = Some(Card {
            uid: doc.uid,
            version: doc.buffer.version(),
            list,
            anchor,
            rows: Vec::new(),
            waiting: Some(rx),
            rect: None,
            sticky,
        });
    }

    pub(super) fn font_preview_ui(&mut self, ctx: &egui::Context) {
        // Under the hover box, if the server or the editor shows one there too.
        let below = self.hover_rect();
        let Some(card) = &mut self.font_preview.card else { return };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.font_preview.card = None;
            return;
        }
        if let Some(rx) = &card.waiting
            && let Ok(rows) = rx.try_recv()
        {
            card.waiting = None;
            card.rows = rows
                .into_iter()
                .enumerate()
                .map(|(i, mut row)| {
                    let texture = row.image.take().map(|image| {
                        ctx.load_texture(format!("font_sample_{i}_{}", row.family), image, egui::TextureOptions::LINEAR)
                    });
                    (row, texture)
                })
                .collect();
        }
        let pal = theme::colors();
        // What a browser here would use: the first family that's installed.
        let used = card.rows.iter().position(|(row, _)| row.installed);
        let anchor = match below {
            Some(rect) if rect.top() <= card.anchor.y + 4.0 => egui::pos2(card.anchor.x, rect.bottom() + 6.0),
            _ => card.anchor,
        };
        let shown = egui::Area::new(egui::Id::unique("font_preview"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(pal.sidebar_bg).inner_margin(10.0).show(ui, |ui| {
                    ui.set_max_width(SAMPLE_WIDTH + 20.0);
                    if card.waiting.is_some() {
                        ui.label(RichText::new("Reading this computer's fonts…").color(pal.text_dim));
                        return;
                    }
                    for (i, (row, texture)) in card.rows.iter().enumerate() {
                        if i > 0 {
                            ui.add_space(6.0);
                        }
                        let state = match (row.installed, row.generic, &row.face) {
                            (true, true, Some(face)) => format!("generic: {face} here"),
                            (true, _, _) => "installed".to_owned(),
                            (false, _, Some(face)) => format!("not installed: shown in {face}, its fallback"),
                            (false, _, None) => "not installed".to_owned(),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&row.family).strong());
                            let color = if row.installed { pal.text_dim } else { pal.modified };
                            ui.label(RichText::new(state).small().color(color));
                            if used == Some(i) {
                                ui.label(RichText::new("used").small().color(pal.accent)).on_hover_text(
                                    "A browser on this computer would use this one: the first that's there",
                                );
                            }
                        });
                        match texture {
                            Some(texture) => {
                                // Scaled down only if wider than the card.
                                let natural = texture.size_vec2() / ctx.pixels_per_point();
                                let size = natural * (SAMPLE_WIDTH / natural.x).min(1.0);
                                let tint = if row.installed {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::from_white_alpha(140)
                                };
                                ui.add(egui::Image::new((texture.id(), size)).tint(tint));
                            }
                            None => {
                                ui.label(RichText::new("(no font to draw it with)").color(pal.text_dim));
                            }
                        }
                    }
                });
            });
        card.rect = Some(shown.response.rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_say_what_is_installed_and_what_stands_in() {
        let families: Vec<String> =
            ["No Such Font Anywhere", "Arial", "sans-serif"].iter().map(|s| (*s).to_owned()).collect();
        let rows = rows_for(&families, [255, 255, 255], 1.0);
        assert!(!rows[0].installed, "a missing font");
        // A missing font is drawn in the next one that's here.
        if rows[1].installed {
            assert_eq!(rows[0].face.as_deref(), rows[1].face.as_deref());
            assert!(rows[0].image.is_some());
        }
        assert!(rows[2].generic && rows[2].installed);
    }
}
