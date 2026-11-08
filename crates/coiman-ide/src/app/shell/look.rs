//! Themes by time of day, and pictures over the window or each part of it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use coiman_core::{appearance::theme_schedule, applog};

use crate::app::CodeApp;
use crate::ui::theme;

/// Seconds between looks at the clock.
const CLOCK_SECONDS: f64 = 20.0;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
/// Larger pictures are scaled down: it's a faint backdrop.
const LARGEST_SIDE: u32 = 2560;

/// A picture or folder of them, on its way to the screen.
#[derive(Default)]
struct Backdrop {
    /// The setting last read, and the pictures it means.
    source: String,
    pictures: Vec<PathBuf>,
    /// The last one shown.
    at: Option<usize>,
    next_picture: f64,
    loading: Option<Receiver<Result<egui::ColorImage, String>>>,
    texture: Option<egui::TextureHandle>,
}

/// Where each part of the window is this frame.
pub(crate) struct Places {
    pub(crate) editor: egui::Rect,
    pub(crate) sides: Vec<egui::Rect>,
    pub(crate) panels: Vec<egui::Rect>,
}

#[derive(Default)]
pub(crate) struct Look {
    /// The schedule last read, and the theme it last set.
    schedule: String,
    scheduled: Option<String>,
    next_check: f64,
    /// The main picture, then the editor's, the side bars' and the panel's own.
    backdrops: [Backdrop; 4],
}

/// The pictures a setting means: a file, or the images in a folder, in order.
fn pictures_of(source: &str) -> Vec<PathBuf> {
    let path = Path::new(source);
    if !path.is_dir() {
        return if path.is_file() { vec![path.to_path_buf()] } else { Vec::new() };
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str()))
        })
        .collect();
    found.sort();
    found
}

fn decode(path: &Path) -> Result<egui::ColorImage, String> {
    let reader = image::ImageReader::open(path).map_err(|e| e.to_string())?;
    let picture = reader.with_guessed_format().map_err(|e| e.to_string())?.decode().map_err(|e| e.to_string())?;
    let picture = if picture.width().max(picture.height()) > LARGEST_SIDE {
        picture.thumbnail(LARGEST_SIDE, LARGEST_SIDE)
    } else {
        picture
    };
    let rgba = picture.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

/// The part of a picture that covers a space of this shape, centered.
fn cover_uv(picture: egui::Vec2, space: egui::Vec2) -> egui::Rect {
    let (picture_wide, space_wide) = (picture.x / picture.y.max(1.0), space.x / space.y.max(1.0));
    let (width, height) =
        if picture_wide > space_wide { (space_wide / picture_wide, 1.0) } else { (1.0, picture_wide / space_wide) };
    egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(width, height))
}

/// Which picture comes next: the following one, or with `random` any but the current one.
/// `seed` is any changing number.
fn next_index(current: Option<usize>, count: usize, random: bool, seed: u64) -> usize {
    match current {
        None if !random => 0,
        Some(at) if !random => (at + 1) % count,
        Some(at) if count > 1 => {
            // Multiplying spreads out seeds that are close together, as clocks are.
            let step = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32;
            (at + 1 + step as usize % (count - 1)) % count
        }
        _ => (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as usize % count,
    }
}

impl Backdrop {
    /// Follows its setting, and loads the next picture when it's time. Returns a problem to tell.
    fn pump(&mut self, ctx: &egui::Context, source: &str, interval: f64, random: bool) -> Option<String> {
        let now = ctx.input(|i| i.time);
        let mut problem = None;
        if source != self.source {
            *self = Self { source: source.to_owned(), ..Self::default() };
            self.pictures = if source.is_empty() { Vec::new() } else { pictures_of(source) };
            if !source.is_empty() && self.pictures.is_empty() {
                problem = Some(format!("Background image: no picture found at {source}"));
            }
        }
        if let Some(rx) = &self.loading
            && let Ok(result) = rx.try_recv()
        {
            self.loading = None;
            match result {
                Ok(picture) => {
                    let name = format!("background_{source}");
                    self.texture = Some(ctx.load_texture(name, picture, egui::TextureOptions::LINEAR));
                }
                Err(why) => problem = Some(format!("Background image: {why}")),
            }
        }
        if self.pictures.is_empty() || self.loading.is_some() || now < self.next_picture {
            return problem;
        }
        let rotates = interval > 0.0 && self.pictures.len() > 1;
        // Once it's loaded, a single picture needs nothing more.
        if self.texture.is_some() && !rotates {
            return problem;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(now.to_bits(), |d| d.as_nanos() as u64);
        let at = next_index(self.at, self.pictures.len(), random, seed);
        self.at = Some(at);
        let path = self.pictures[at].clone();
        applog::info(&format!("background image {}", path.display()));
        self.next_picture = if rotates { now + interval } else { f64::INFINITY };
        if rotates {
            ctx.request_repaint_after(Duration::from_secs_f64(interval));
        }
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(decode(&path));
            wake.request_repaint();
        });
        self.loading = Some(rx);
        problem
    }

    fn paint(&self, ctx: &egui::Context, rect: egui::Rect, radius: f32, opacity: f32) {
        let Some(texture) = &self.texture else { return };
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("background_image"));
        let tint = egui::Color32::WHITE.gamma_multiply(opacity.clamp(0.0, 1.0));
        let shape = egui::epaint::RectShape::filled(rect, radius, tint)
            .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size()));
        ctx.layer_painter(layer).add(shape);
    }
}

impl CodeApp {
    /// Follows the schedule: a theme chosen by hand stays until the next time on it.
    pub(crate) fn pump_theme_schedule(&mut self, ctx: &egui::Context) {
        let text = &self.settings.workbench.theme_schedule;
        let now = ctx.input(|i| i.time);
        let settings_text = format!("{text}|{}", self.settings.workbench.theme_location);
        if settings_text != self.look.schedule {
            self.look.next_check = 0.0;
            if text.trim().is_empty() && self.look.scheduled.take().is_some() {
                // Back to the theme the settings name.
                let (back, _) = theme::resolve(&self.settings.workbench.color_theme, &self.themes);
                theme::set(ctx, back);
            }
            self.look.schedule = settings_text;
        }
        if self.settings.workbench.theme_schedule.trim().is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(CLOCK_SECONDS));
        if now < self.look.next_check {
            return;
        }
        self.look.next_check = now + CLOCK_SECONDS;
        let parsed = theme_schedule::parse_location(&self.settings.workbench.theme_location).and_then(|location| {
            theme_schedule::parse(&self.settings.workbench.theme_schedule, theme_schedule::sun_today(location))
        });
        let list = match parsed {
            Ok(list) => list,
            Err(why) => {
                self.status = Some(why);
                self.look.next_check = f64::INFINITY;
                return;
            }
        };
        let wanted = theme_schedule::theme_at(&list, theme_schedule::minute_now()).map(str::to_owned);
        if wanted.is_some() && wanted != self.look.scheduled {
            let name = wanted.clone().unwrap_or_default();
            if self.themes.is_empty() {
                self.themes = theme::discover();
            }
            let (chosen, problem) = theme::resolve(&name, &self.themes);
            match problem {
                Some(problem) => self.status = Some(format!("Theme schedule: {problem}")),
                None => {
                    theme::set(ctx, chosen);
                    applog::info(&format!("scheduled theme {name}"));
                }
            }
            self.look.scheduled = wanted;
        }
    }

    /// Follows the image settings, and loads the next pictures when it's time.
    pub(crate) fn pump_background(&mut self, ctx: &egui::Context) {
        let settings = &self.settings.workbench;
        let sources = [
            settings.background_image.trim().to_owned(),
            settings.background_editor.trim().to_owned(),
            settings.background_sidebar.trim().to_owned(),
            settings.background_panel.trim().to_owned(),
        ];
        let (interval, random) = (f64::from(settings.background_interval), settings.background_random);
        for (backdrop, source) in self.look.backdrops.iter_mut().zip(&sources) {
            if let Some(problem) = backdrop.pump(ctx, source, interval, random) {
                self.status = Some(problem);
            }
        }
    }

    /// Lays the pictures faintly over what's been drawn, under windows and popups.
    pub(crate) fn paint_background(&self, ctx: &egui::Context, places: &Places) {
        let settings = &self.settings.workbench;
        let opacity = settings.background_opacity;
        let radius = if theme::islands() { 10.0 } else { 0.0 };
        let [main, editor, side, panel] = &self.look.backdrops;
        if settings.background_area == "window" {
            main.paint(ctx, ctx.content_rect(), 0.0, opacity);
        } else {
            main.paint(ctx, places.editor, radius, opacity);
        }
        editor.paint(ctx, places.editor, radius, opacity);
        for rect in &places.sides {
            side.paint(ctx, *rect, radius, opacity);
        }
        for rect in &places.panels {
            panel.paint(ctx, *rect, radius, opacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_covers_its_space_without_stretching() {
        let wide = cover_uv(egui::vec2(200.0, 100.0), egui::vec2(100.0, 100.0));
        assert!((wide.width() - 0.5).abs() < 0.001 && (wide.height() - 1.0).abs() < 0.001);
        let tall = cover_uv(egui::vec2(100.0, 200.0), egui::vec2(100.0, 100.0));
        assert!((tall.width() - 1.0).abs() < 0.001 && (tall.height() - 0.5).abs() < 0.001);
        assert_eq!(wide.center(), egui::pos2(0.5, 0.5));
    }

    #[test]
    fn a_folder_means_its_images_in_order() {
        let dir = std::env::temp_dir().join(format!("rc_look_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.PNG", "a.jpg", "notes.txt"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let names: Vec<String> = pictures_of(&dir.to_string_lossy())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.jpg", "b.PNG"]);
        assert_eq!(pictures_of(&dir.join("a.jpg").to_string_lossy()).len(), 1);
        assert!(pictures_of(&dir.join("none").to_string_lossy()).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pictures_go_in_order_or_at_random_without_repeating() {
        assert_eq!(next_index(None, 3, false, 99), 0);
        assert_eq!(next_index(Some(0), 3, false, 99), 1);
        assert_eq!(next_index(Some(2), 3, false, 99), 0);
        let mut seen = std::collections::HashSet::new();
        for seed in 0..200 {
            let next = next_index(Some(1), 4, true, seed);
            assert!(next != 1 && next < 4, "{next}");
            seen.insert(next);
            assert!(next_index(None, 4, true, seed) < 4);
        }
        assert_eq!(seen.len(), 3, "reaches all the others");
        assert_eq!(next_index(Some(0), 1, true, 5), 0);
    }
}
