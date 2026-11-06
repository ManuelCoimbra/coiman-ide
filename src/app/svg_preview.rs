//! Shows an SVG beside its text, following edits.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui::{self, RichText, Sense};
use resvg::{tiny_skia, usvg};

use super::{CodeApp, repaint};
use crate::theme;

/// Seconds typing must pause before drawing again.
const REST: f64 = 0.25;
/// The longest side drawn, so a huge declared size can't take all the memory.
const MAX_SIDE: f32 = 2048.0;

pub(super) type Drawn = Result<([usize; 2], Vec<u8>, [f32; 2]), String>;

#[derive(Default)]
pub(super) struct SvgPreview {
    /// The tab being shown.
    uid: Option<u64>,
    drawn_version: Option<u64>,
    waiting: Option<(u64, Receiver<Drawn>)>,
    changed_at: Option<(u64, f64)>,
    texture: Option<(egui::TextureHandle, [f32; 2])>,
    problem: Option<String>,
}

fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut fonts = usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        })
        .clone()
}

/// Draws SVG text. It reads nothing else: no files or addresses it names.
pub(super) fn draw(text: &str) -> Drawn {
    let options = usvg::Options { fontdb: fonts(), resources_dir: None, ..Default::default() };
    let tree = usvg::Tree::from_str(text, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let scale = (MAX_SIDE / size.width().max(size.height())).min(4.0);
    let (width, height) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(width.max(1), height.max(1)).ok_or("The picture has no size")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(([pixmap.width() as usize, pixmap.height() as usize], pixmap.take(), [size.width(), size.height()]))
}

impl CodeApp {
    /// Shows the active SVG beside its text, or stops showing it.
    pub(super) fn svg_preview_open(&self) -> bool {
        self.svg_preview.uid.is_some() && self.svg_preview.uid == self.docs.get(self.active).map(|d| d.uid)
    }

    pub(super) fn toggle_svg_preview(&mut self) {
        let uid = self.docs.get(self.active).map(|d| d.uid);
        let preview = &mut self.svg_preview;
        *preview = if preview.uid == uid { SvgPreview::default() } else { SvgPreview { uid, ..Default::default() } };
    }

    pub(super) fn svg_preview_ui(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = self.docs.get(self.active) else { return };
        if self.svg_preview.uid != Some(doc.uid) {
            return;
        }
        let version = doc.buffer.version();
        let now = ui.input(|i| i.time);
        let ctx = ui.ctx().clone();
        let preview = &mut self.svg_preview;
        if let Some((asked, rx)) = &preview.waiting
            && let Ok(answer) = rx.try_recv()
        {
            preview.drawn_version = Some(*asked);
            preview.waiting = None;
            match answer {
                Ok((size, pixels, natural)) => {
                    let image = egui::ColorImage::from_rgba_premultiplied(size, &pixels);
                    let texture = ctx.load_texture("svg_preview", image, egui::TextureOptions::LINEAR);
                    preview.texture = Some((texture, natural));
                    preview.problem = None;
                }
                // The last good picture stays: half-typed text is often invalid.
                Err(why) => preview.problem = Some(why),
            }
        }
        if preview.drawn_version != Some(version) && preview.waiting.is_none() {
            let since = match preview.changed_at {
                Some((seen, since)) if seen == version => since,
                _ => {
                    preview.changed_at = Some((version, now));
                    now
                }
            };
            let first = preview.drawn_version.is_none();
            if first || now - since >= REST {
                let text = doc.text();
                let (tx, rx) = mpsc::channel();
                let wake = repaint(&ctx);
                std::thread::spawn(move || {
                    let _ = tx.send(draw(&text));
                    wake();
                });
                preview.waiting = Some((version, rx));
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(REST));
            }
        }
        let half = ui.available_width() / 2.0;
        let pal = theme::colors();
        egui::Panel::right("svg_preview")
            .resizable(true)
            .default_size(half)
            .size_range(200.0..=(half * 2.0 - 200.0).max(200.0))
            .frame(egui::Frame::new().fill(pal.editor_bg).inner_margin(8.0))
            .show(ui, |ui| {
                if let Some(why) = &preview.problem {
                    ui.label(RichText::new(why).color(egui::Color32::from_rgb(0xF1, 0x4C, 0x4C)));
                }
                let Some((texture, natural)) = &preview.texture else {
                    ui.label(RichText::new("Drawing…").color(pal.text_dim));
                    return;
                };
                let room = ui.available_size();
                let scale = (room.x / natural[0].max(1.0)).min(room.y / natural[1].max(1.0));
                let size = egui::vec2(natural[0], natural[1]) * scale.max(0.01);
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                // A plain ground, so dark and light pictures both show.
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_gray(0xE8));
                let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(texture.id(), rect, whole, egui::Color32::WHITE);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_or_says_why_not() {
        let (size, pixels, natural) =
            draw(r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="20"><rect width="10" height="20" fill="#f00"/></svg>"##)
                .unwrap();
        assert_eq!(natural, [10.0, 20.0]);
        assert_eq!(pixels.len(), size[0] * size[1] * 4);
        assert_eq!(&pixels[..4], &[0xff, 0, 0, 0xff]);
        assert!(draw("<svg").is_err());
        assert!(draw("not svg").is_err());
        // A huge declared size is held to the limit.
        let huge = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"/>"#;
        let (size, ..) = draw(huge).unwrap();
        assert!(size[0] <= 2048 && size[1] <= 2048, "{size:?}");
    }
}
