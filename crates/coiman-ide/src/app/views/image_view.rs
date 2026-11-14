//! Image files shown in place of the editor: fit or any zoom, panned by dragging,
//! turned the way their camera says, and animated when they're GIFs or WebPs.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText, Sense};
use image::{AnimationDecoder, DynamicImage, ImageDecoder};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

/// The largest image file to read.
const MAX_IMAGE_BYTES: u64 = 256 * 1024 * 1024;
/// Memory a decoded picture may take, all frames together.
const MAX_DECODED_BYTES: u64 = 1024 * 1024 * 1024;
/// Frames of an animation kept; past this it stops on the last.
const MAX_FRAMES: usize = 1000;
const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 64.0;

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "jfif", "gif", "webp", "bmp", "ico", "tif", "tiff"];
/// Pictures this build can't decode, which still deserve better than a hex dump.
const UNSUPPORTED: &[&str] = &["avif", "heic", "heif", "jxl", "psd"];

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub(crate) fn is_image(path: &Path) -> bool {
    let ext = extension(path);
    IMAGE_EXTENSIONS.contains(&ext.as_str()) || UNSUPPORTED.contains(&ext.as_str())
}

pub(crate) struct Decoded {
    pub(crate) size: [usize; 2],
    pub(crate) rgba: Vec<u8>,
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

/// A still picture, turned upright by its EXIF orientation.
fn decode_still(path: &Path) -> Result<DynamicImage, String> {
    let mut reader =
        image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    picture.apply_orientation(orientation);
    Ok(picture)
}

/// A picture's first frame, upright, as RGBA no larger than `max_side` a side: for
/// the Markdown preview.
pub(crate) fn decode(path: &Path, max_side: usize) -> Result<Decoded, String> {
    let picture = fit_texture(decode_still(path)?.into_rgba8(), max_side, &mut None);
    Ok(Decoded { size: [picture.width() as usize, picture.height() as usize], rgba: picture.into_raw() })
}

/// A frame of a picture, and the seconds it stays.
type Frame = (egui::ColorImage, f64);

/// What the viewer shows: frames with how long each stays, the picture's own size, and
/// anything worth saying about how it's shown.
struct Loaded {
    frames: Vec<Frame>,
    size: [usize; 2],
    format: String,
    note: Option<String>,
}

fn color_image(picture: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([picture.width() as usize, picture.height() as usize], picture.as_raw())
}

/// Smaller, when a side is more than the graphics card can hold in one texture.
fn fit_texture(picture: image::RgbaImage, max_side: usize, note: &mut Option<String>) -> image::RgbaImage {
    let (w, h) = (picture.width() as usize, picture.height() as usize);
    if w.max(h) <= max_side {
        return picture;
    }
    let scale = max_side as f64 / w.max(h) as f64;
    let (nw, nh) = (((w as f64 * scale) as u32).max(1), ((h as f64 * scale) as u32).max(1));
    *note = Some(format!("shown at {nw} × {nh}: larger than this graphics card holds at once"));
    image::imageops::resize(&picture, nw, nh, image::imageops::FilterType::Triangle)
}

fn frames_of<'a>(
    animation: impl AnimationDecoder<'a>,
    max_side: usize,
    note: &mut Option<String>,
) -> Result<(Vec<Frame>, [usize; 2]), String> {
    let mut frames = Vec::new();
    let mut size = [0, 0];
    let mut bytes = 0u64;
    for frame in animation.into_frames() {
        let frame = frame.map_err(|e| e.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        // Browsers treat a delay this short as 0.1 s, and so does this.
        let delay = f64::from(numerator) / f64::from(denominator.max(1)) / 1000.0;
        let delay = if delay < 0.02 { 0.1 } else { delay };
        let picture = frame.into_buffer();
        size = [picture.width() as usize, picture.height() as usize];
        bytes += picture.as_raw().len() as u64;
        if frames.len() >= MAX_FRAMES || bytes > MAX_DECODED_BYTES {
            *note = Some(format!("only the first {} frames are shown", frames.len()));
            break;
        }
        frames.push((color_image(&fit_texture(picture, max_side, note)), delay));
    }
    if frames.is_empty() {
        return Err("it has no frames".to_owned());
    }
    Ok((frames, size))
}

fn load(path: &Path, max_side: usize) -> Result<Loaded, String> {
    let ext = extension(path);
    if UNSUPPORTED.contains(&ext.as_str()) {
        return Err(format!("{} pictures can't be shown here: no decoder for them is built in", ext.to_uppercase()));
    }
    let mut note = None;
    let open = || std::fs::File::open(path).map(BufReader::new).map_err(|e| e.to_string());
    let format = image::ImageFormat::from_path(path).ok();
    let animated = match format {
        Some(image::ImageFormat::Gif) => {
            let mut decoder = image::codecs::gif::GifDecoder::new(open()?).map_err(|e| e.to_string())?;
            // Made with no limits: a frame's declared size would be allocated as it says.
            decoder.set_limits(limits()).map_err(|e| e.to_string())?;
            Some(frames_of(decoder, max_side, &mut note)?)
        }
        Some(image::ImageFormat::WebP) => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(open()?).map_err(|e| e.to_string())?;
            decoder.set_limits(limits()).map_err(|e| e.to_string())?;
            if decoder.has_animation() { Some(frames_of(decoder, max_side, &mut note)?) } else { None }
        }
        _ => None,
    };
    let format = format.map_or_else(|| ext.to_uppercase(), |f| format!("{f:?}").to_uppercase());
    if let Some((frames, size)) = animated {
        if frames.len() > 1 {
            let count = format!("{} frames", frames.len());
            note = Some(note.map_or(count.clone(), |n| format!("{count}, {n}")));
        }
        return Ok(Loaded { frames, size, format, note });
    }
    let picture = decode_still(path)?.into_rgba8();
    let size = [picture.width() as usize, picture.height() as usize];
    let picture = fit_texture(picture, max_side, &mut note);
    Ok(Loaded { frames: vec![(color_image(&picture), 0.0)], size, format, note })
}

enum State {
    Loading(Receiver<Result<Loaded, String>>),
    Ready {
        frames: Vec<(egui::TextureHandle, f64)>,
        size: [usize; 2],
        format: String,
        note: Option<String>,
        since: f64,
    },
    Failed(String),
}

struct View {
    path: PathBuf,
    bytes: u64,
    state: State,
    /// `None` fits the picture to the space; otherwise screen points per pixel.
    zoom: Option<f32>,
    /// How far the picture's center is from the space's center, in points.
    offset: egui::Vec2,
    /// The document that was active: switching tabs leaves the picture.
    over: Option<u64>,
    showing: bool,
}

#[derive(Default)]
pub(crate) struct Viewer {
    view: Option<View>,
    /// Opened since the last frame, and not started yet.
    to_open: Option<PathBuf>,
}

/// The zoom that fits `natural` in `room`, never larger than the picture itself.
fn fit_zoom(natural: egui::Vec2, room: egui::Vec2) -> f32 {
    (room.x / natural.x.max(1.0)).min(room.y / natural.y.max(1.0)).min(1.0)
}

/// The offset that keeps the picture's point under `pointer` there through a zoom.
fn zoom_about(center: egui::Pos2, offset: egui::Vec2, pointer: egui::Pos2, from: f32, to: f32) -> egui::Vec2 {
    let picture_center = center + offset;
    let under = (pointer - picture_center) / from;
    pointer - under * to - center
}

/// Keeps part of the picture in view however far it's dragged.
fn clamp_offset(offset: egui::Vec2, shown: egui::Vec2, room: egui::Vec2) -> egui::Vec2 {
    let limit = |picture: f32, space: f32| ((picture + space) / 2.0 - 40.0).max(0.0);
    egui::vec2(
        offset.x.clamp(-limit(shown.x, room.x), limit(shown.x, room.x)),
        offset.y.clamp(-limit(shown.y, room.y), limit(shown.y, room.y)),
    )
}

/// Which frame shows `elapsed` seconds into an animation, and the seconds until the next.
fn frame_at(delays: &[f64], elapsed: f64) -> (usize, f64) {
    let total: f64 = delays.iter().sum();
    if delays.len() < 2 || total <= 0.0 {
        return (0, f64::INFINITY);
    }
    let mut t = elapsed % total;
    for (i, delay) in delays.iter().enumerate() {
        if t < *delay {
            return (i, delay - t);
        }
        t -= delay;
    }
    (delays.len() - 1, 0.0)
}

/// What shows through a transparent picture.
fn checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    const CELL: f32 = 10.0;
    let (dark, light) = (egui::Color32::from_gray(60), egui::Color32::from_gray(90));
    painter.rect_filled(rect, 0.0, dark);
    let clip = painter.clip_rect().intersect(rect);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    let (first_x, first_y) =
        (((clip.left() - rect.left()) / CELL).floor() as i32, ((clip.top() - rect.top()) / CELL).floor() as i32);
    let count_x = (clip.width() / CELL).ceil() as i32 + 1;
    let count_y = (clip.height() / CELL).ceil() as i32 + 1;
    for y in first_y..first_y + count_y {
        for x in first_x..first_x + count_x {
            if (x + y) % 2 == 0 {
                let min = rect.min + egui::vec2(x as f32 * CELL, y as f32 * CELL);
                let cell = egui::Rect::from_min_size(min, egui::vec2(CELL, CELL)).intersect(rect);
                painter.rect_filled(cell, 0.0, light);
            }
        }
    }
}

impl CodeApp {
    /// Asks for an image file to be shown. Any other file closes the one showing.
    pub(crate) fn open_image_file(&mut self, path: &Path) -> bool {
        let image = is_image(path);
        self.images.to_open = image.then(|| path.to_path_buf());
        image
    }

    fn open_image(&mut self, ctx: &egui::Context, path: &Path) {
        let bytes = std::fs::metadata(path).map_or(0, |m| m.len());
        let state = if bytes > MAX_IMAGE_BYTES {
            State::Failed(format!("too large to show ({} MB)", bytes / (1024 * 1024)))
        } else {
            let (tx, rx) = mpsc::channel();
            let (file, wake) = (path.to_path_buf(), repaint(ctx));
            let max_side = ctx.input(|i| i.max_texture_side).max(1024);
            std::thread::spawn(move || {
                let _ = tx.send(load(&file, max_side));
                wake();
            });
            State::Loading(rx)
        };
        let over = self.docs.get(self.active).map(|d| d.uid);
        self.images.view = Some(View {
            path: path.to_path_buf(),
            bytes,
            state,
            zoom: None,
            offset: egui::Vec2::ZERO,
            over,
            showing: true,
        });
    }

    pub(crate) fn image_tab(&self) -> Option<(String, bool)> {
        let view = self.images.view.as_ref()?;
        Some((view.path.file_name()?.to_string_lossy().into_owned(), view.showing))
    }

    /// The picture in place of the editor, if one is.
    pub(crate) fn shown_image(&self) -> Option<&Path> {
        self.images.view.as_ref().filter(|v| v.showing).map(|v| v.path.as_path())
    }

    pub(crate) fn hide_image(&mut self) {
        if let Some(view) = &mut self.images.view {
            view.showing = false;
        }
    }

    pub(crate) fn show_image_tab(&mut self) {
        let over = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = &mut self.images.view {
            view.showing = true;
            view.over = over;
        }
    }

    pub(crate) fn close_image_tab(&mut self) {
        self.images.view = None;
    }

    /// Shows the open picture in place of the editor. Says whether it did.
    pub(crate) fn image_view(&mut self, ui: &mut egui::Ui) -> bool {
        if let Some(path) = self.images.to_open.take() {
            self.hide_logs();
            self.hide_bytes();
            self.hide_file_manager();
            self.open_image(ui.ctx(), &path);
            self.explorer.reveal(&path);
        }
        let active = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = self.images.view.as_mut().filter(|v| v.over != active) {
            view.showing = false;
        }
        let Some(view) = self.images.view.as_mut().filter(|v| v.showing) else { return false };
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if let State::Loading(rx) = &view.state
            && let Ok(result) = rx.try_recv()
        {
            view.state = match result {
                Ok(loaded) => {
                    // Smooth when smaller than itself, crisp pixels when zoomed in.
                    let options = egui::TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Linear,
                        mipmap_mode: Some(egui::TextureFilter::Linear),
                        ..Default::default()
                    };
                    let name = view.path.display().to_string();
                    let frames = loaded
                        .frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (image, delay))| (ctx.load_texture(format!("{name}#{i}"), image, options), delay))
                        .collect();
                    State::Ready { frames, size: loaded.size, format: loaded.format, note: loaded.note, since: now }
                }
                Err(why) => State::Failed(why),
            };
        }
        let pal = theme::colors();
        let mut close = false;
        let room_size = ui.available_size() - egui::vec2(0.0, 36.0);
        let natural = match &view.state {
            State::Ready { size: [w, h], .. } => Some(egui::vec2(*w as f32, *h as f32)),
            _ => None,
        };
        let fit = natural.map_or(1.0, |n| fit_zoom(n, room_size));
        let zoom = view.zoom.unwrap_or(fit);
        let mut new_zoom = None;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let name = view.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(RichText::new(name).strong());
            let kib = view.bytes as f64 / 1024.0;
            let weight = if kib < 1024.0 { format!("{kib:.1} KB") } else { format!("{:.1} MB", kib / 1024.0) };
            let details = match &view.state {
                State::Ready { size: [w, h], format, note, .. } => {
                    let note = note.as_ref().map(|n| format!(", {n}")).unwrap_or_default();
                    format!("{format}, {w} × {h}, {weight}{note}")
                }
                _ => weight,
            };
            ui.label(RichText::new(details).color(pal.text_dim));
            if natural.is_some() {
                ui.add_space(8.0);
                if ui.button("−").on_hover_text("Zoom out (-)").clicked() {
                    new_zoom = Some(zoom / 1.25);
                }
                ui.label(RichText::new(format!("{:.0}%", zoom * 100.0)).monospace());
                if ui.button("+").on_hover_text("Zoom in (+)").clicked() {
                    new_zoom = Some(zoom * 1.25);
                }
                if ui.selectable_label(view.zoom.is_none(), "Fit").on_hover_text("Fit to the space (0)").clicked() {
                    view.zoom = None;
                    view.offset = egui::Vec2::ZERO;
                }
                if ui.selectable_label(view.zoom == Some(1.0), "100%").on_hover_text("Actual size (1)").clicked() {
                    new_zoom = Some(1.0);
                }
            }
            close = ui.button("Close").clicked();
        });
        ui.separator();
        match &view.state {
            State::Loading(_) => {
                ui.label("Loading…");
            }
            State::Failed(why) => {
                ui.label(format!("Can't show this image: {why}"));
            }
            State::Ready { frames, since, .. } => {
                let (room, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                let natural = natural.unwrap_or(egui::Vec2::splat(1.0));
                let center = room.center();
                // Keys work while the pointer is over the picture.
                if response.hovered() {
                    let (plus, minus, zero, one) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
                            i.key_pressed(egui::Key::Minus),
                            i.key_pressed(egui::Key::Num0),
                            i.key_pressed(egui::Key::Num1),
                        )
                    });
                    if plus {
                        new_zoom = Some(zoom * 1.25);
                    }
                    if minus {
                        new_zoom = Some(zoom / 1.25);
                    }
                    if one {
                        new_zoom = Some(1.0);
                    }
                    if zero {
                        view.zoom = None;
                        view.offset = egui::Vec2::ZERO;
                    }
                    // Ctrl and the wheel (or a pinch) zoom about the pointer; the wheel alone pans.
                    let (pinch, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
                    if (pinch - 1.0).abs() > f32::EPSILON {
                        new_zoom = Some(zoom * pinch);
                    } else if scroll != egui::Vec2::ZERO {
                        view.zoom.get_or_insert(zoom);
                        view.offset += scroll;
                    }
                }
                if let Some(to) = new_zoom {
                    let to = to.clamp(MIN_ZOOM, MAX_ZOOM);
                    let pointer = response.hover_pos().unwrap_or(center);
                    view.offset = zoom_about(center, view.offset, pointer, zoom, to);
                    view.zoom = Some(to);
                }
                if response.dragged() {
                    view.zoom.get_or_insert(zoom);
                    view.offset += response.drag_delta();
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                } else if response.hovered() && view.zoom.is_some() {
                    ctx.set_cursor_icon(egui::CursorIcon::Grab);
                }
                if response.double_clicked() {
                    view.zoom = None;
                    view.offset = egui::Vec2::ZERO;
                }
                let zoom = view.zoom.unwrap_or(fit);
                let shown = natural * zoom;
                view.offset = clamp_offset(view.offset, shown, room.size());
                let rect = egui::Rect::from_center_size(center + view.offset, shown);
                let painter = ui.painter_at(room);
                checkerboard(&painter, rect);
                let delays: Vec<f64> = frames.iter().map(|f| f.1).collect();
                let (index, next) = frame_at(&delays, now - since);
                if next.is_finite() {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.01)));
                }
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(frames[index].0.id(), rect, uv, egui::Color32::WHITE);
            }
        }
        if close {
            self.images.view = None;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_never_enlarges() {
        assert_eq!(fit_zoom(egui::vec2(100.0, 50.0), egui::vec2(400.0, 400.0)), 1.0);
        assert_eq!(fit_zoom(egui::vec2(1000.0, 500.0), egui::vec2(500.0, 500.0)), 0.5);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let center = egui::pos2(500.0, 300.0);
        let pointer = egui::pos2(600.0, 350.0);
        let offset = zoom_about(center, egui::Vec2::ZERO, pointer, 1.0, 2.0);
        // The picture's point 100,50 from its center was under the pointer: at twice the size it's 200,100.
        assert_eq!(center + offset + egui::vec2(200.0, 100.0), pointer);
    }

    #[test]
    fn a_dragged_picture_stays_partly_in_view() {
        let clamped = clamp_offset(egui::vec2(5000.0, -5000.0), egui::vec2(200.0, 100.0), egui::vec2(800.0, 600.0));
        assert_eq!(clamped, egui::vec2(460.0, -310.0));
    }

    #[test]
    fn animation_frames_follow_their_delays() {
        let delays = [0.1, 0.2, 0.3];
        assert_eq!(frame_at(&delays, 0.05).0, 0);
        assert_eq!(frame_at(&delays, 0.15).0, 1);
        assert_eq!(frame_at(&delays, 0.45).0, 2);
        assert_eq!(frame_at(&delays, 0.65).0, 0, "it loops");
        assert_eq!(frame_at(&[0.1], 5.0), (0, f64::INFINITY));
    }

    #[test]
    fn reads_turns_and_refuses_pictures() {
        let dir = std::env::temp_dir().join(format!("coiman-images-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // 3 wide, 2 high, then saved as a TIFF the test can mark as turned.
        let picture = image::RgbaImage::from_fn(3, 2, |x, _| image::Rgba([x as u8 * 100, 0, 0, 255]));
        let png = dir.join("a.png");
        picture.save(&png).unwrap();
        let loaded = load(&png, 4096).unwrap();
        assert_eq!(loaded.size, [3, 2]);
        assert_eq!(loaded.format, "PNG");
        // Too large for the texture: smaller, and said so.
        let wide = dir.join("wide.png");
        image::RgbaImage::new(5000, 10).save(&wide).unwrap();
        let loaded = load(&wide, 2048).unwrap();
        assert_eq!(loaded.size, [5000, 10]);
        assert_eq!(loaded.frames[0].0.size, [2048, 4]);
        assert!(loaded.note.is_some_and(|n| n.contains("2048")));
        // Cut short, and a kind with no decoder.
        let broken = dir.join("broken.png");
        std::fs::write(&broken, &std::fs::read(&png).unwrap()[..20]).unwrap();
        assert!(load(&broken, 4096).is_err());
        let avif = dir.join("x.avif");
        std::fs::write(&avif, b"....").unwrap();
        assert!(load(&avif, 4096).err().is_some_and(|why| why.contains("AVIF")));
        assert!(is_image(&avif) && is_image(Path::new("a.TIFF")) && !is_image(Path::new("a.svg")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
