//! Themes by time of day, and an image over the window.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use rust_code_core::{applog, theme_schedule};

use super::CodeApp;
use crate::theme;

/// Seconds between looks at the clock.
const CLOCK_SECONDS: f64 = 20.0;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
/// Larger pictures are scaled down: it's a faint backdrop.
const LARGEST_SIDE: u32 = 2560;

#[derive(Default)]
pub(super) struct Look {
    /// The schedule last read, and the theme it last set.
    schedule: String,
    scheduled: Option<String>,
    next_check: f64,
    /// The image setting last read, and the pictures it means.
    source: String,
    pictures: Vec<PathBuf>,
    at: usize,
    next_picture: f64,
    loading: Option<Receiver<Result<egui::ColorImage, String>>>,
    texture: Option<egui::TextureHandle>,
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

impl CodeApp {
    /// Follows the schedule: a theme chosen by hand stays until the next time on it.
    pub(super) fn pump_theme_schedule(&mut self, ctx: &egui::Context) {
        let text = &self.settings.workbench.theme_schedule;
        let now = ctx.input(|i| i.time);
        if *text != self.look.schedule {
            self.look.schedule.clone_from(text);
            self.look.next_check = 0.0;
            if text.trim().is_empty() && self.look.scheduled.take().is_some() {
                // Back to the theme the settings name.
                let (back, _) = theme::resolve(&self.settings.workbench.color_theme, &self.themes);
                theme::set(ctx, back);
            }
        }
        if self.look.schedule.trim().is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_secs_f64(CLOCK_SECONDS));
        if now < self.look.next_check {
            return;
        }
        self.look.next_check = now + CLOCK_SECONDS;
        let list = match theme_schedule::parse(&self.look.schedule) {
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

    /// Follows the image setting, and loads the next picture when it's time.
    pub(super) fn pump_background(&mut self, ctx: &egui::Context) {
        let source = self.settings.workbench.background_image.trim().to_owned();
        let now = ctx.input(|i| i.time);
        if source != self.look.source {
            self.look.source.clone_from(&source);
            self.look.pictures = if source.is_empty() { Vec::new() } else { pictures_of(&source) };
            self.look.at = 0;
            self.look.texture = None;
            self.look.next_picture = 0.0;
            if !source.is_empty() && self.look.pictures.is_empty() {
                self.status = Some(format!("Background image: no picture found at {source}"));
            }
        }
        if let Some(rx) = &self.look.loading
            && let Ok(result) = rx.try_recv()
        {
            self.look.loading = None;
            match result {
                Ok(picture) => {
                    let options = egui::TextureOptions::LINEAR;
                    self.look.texture = Some(ctx.load_texture("background_image", picture, options));
                }
                Err(why) => self.status = Some(format!("Background image: {why}")),
            }
        }
        if self.look.pictures.is_empty() || self.look.loading.is_some() || now < self.look.next_picture {
            return;
        }
        let interval = f64::from(self.settings.workbench.background_interval);
        let rotates = interval > 0.0 && self.look.pictures.len() > 1;
        // Once it's loaded, a single picture needs nothing more.
        if self.look.texture.is_some() && !rotates {
            return;
        }
        let path = self.look.pictures[self.look.at % self.look.pictures.len()].clone();
        applog::info(&format!("background image {}", path.display()));
        self.look.at += 1;
        self.look.next_picture = if rotates { now + interval } else { f64::INFINITY };
        if rotates {
            ctx.request_repaint_after(Duration::from_secs_f64(interval));
        }
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(decode(&path));
            wake.request_repaint();
        });
        self.look.loading = Some(rx);
    }

    /// Lays the picture faintly over what's been drawn, under windows and popups.
    pub(super) fn paint_background(&self, ctx: &egui::Context, editor: egui::Rect) {
        let Some(texture) = &self.look.texture else { return };
        let settings = &self.settings.workbench;
        let whole = settings.background_area == "window";
        let rect = if whole { ctx.content_rect() } else { editor };
        let layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("background_image"));
        let tint = egui::Color32::WHITE.gamma_multiply(settings.background_opacity.clamp(0.0, 1.0));
        let radius = if whole || !theme::islands() { 0.0 } else { 10.0 };
        let shape = egui::epaint::RectShape::filled(rect, radius, tint)
            .with_texture(texture.id(), cover_uv(texture.size_vec2(), rect.size()));
        ctx.layer_painter(layer).add(shape);
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
}
