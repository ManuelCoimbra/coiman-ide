//! Fits the window to the screen on launch and remembers its size. The position
//! isn't kept: a saved spot can be on a monitor that is no longer there. Also
//! scales the whole window, or only the code or terminal when Ctrl+wheel or pinch
//! happens over them.

use std::time::{Duration, Instant};

use crate::editor_view::ZoomPart;
use coiman_core::project::session::WindowState;
use eframe::egui::{self, ViewportCommand};

use super::CodeApp;

/// Share of the screen a first launch may take, leaving room for a taskbar.
const FIRST_LAUNCH_SHARE: f32 = 0.9;
/// Change per press of a zoom key.
pub(super) const ZOOM_STEP: f32 = 0.1;
const ZOOM_RANGE: (f32, f32) = (0.5, 3.0);
const FONT_RANGE: (f32, f32) = (6.0, 72.0);
/// Wait for Ctrl+wheel to settle, so one gesture is one write of the settings file.
const ZOOM_SAVE_DELAY: Duration = Duration::from_millis(500);

impl CodeApp {
    pub(super) fn track_window(&mut self, ctx: &egui::Context) {
        let (inner, monitor, maximized, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (v.inner_rect, v.monitor_size, v.maximized == Some(true), v.minimized == Some(true))
        });
        let Some(inner) = inner else { return };
        if !self.window_fitted {
            let Some(monitor) = monitor else {
                ctx.request_repaint();
                return;
            };
            self.window_fitted = true;
            if !maximized {
                let room = if self.window.is_some() { monitor } else { monitor * FIRST_LAUNCH_SHARE };
                let size = inner.size().min(room);
                if size != inner.size() {
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                }
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(((monitor - size) / 2.0).to_pos2()));
            }
            return;
        }
        if minimized {
            return;
        }
        // While maximized, keep the size to come back to.
        let [width, height] = match (&self.window, maximized) {
            (Some(w), true) => [w.width, w.height],
            // egui reports zoomed points; the size is restored before any zoom applies.
            _ => (inner.size() * ctx.zoom_factor()).into(),
        };
        self.window = Some(WindowState { width, height, maximized });
    }

    /// Applies a scale without saving it.
    pub(super) fn set_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        self.zoom = zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        ctx.set_zoom_factor(self.zoom);
    }

    /// Applies a scale the user asked for and saves it shortly after.
    pub(super) fn change_zoom(&mut self, ctx: &egui::Context, zoom: f32) {
        self.set_zoom(ctx, zoom);
        self.zoom_save = Some(Instant::now());
        ctx.request_repaint_after(ZOOM_SAVE_DELAY);
    }

    pub(super) fn pump_zoom(&mut self, ctx: &egui::Context) {
        let hovered = ZoomPart::take_hovered(ctx);
        let delta = ctx.input(|i| i.zoom_delta());
        if delta != 1.0 {
            match hovered {
                Some(ZoomPart::Code) => self.change_font_zoom(ctx, ZoomPart::Code, self.font_size * delta),
                Some(ZoomPart::Terminal) => {
                    self.change_font_zoom(ctx, ZoomPart::Terminal, self.terminal_font() * delta);
                }
                None => self.change_zoom(ctx, self.zoom * delta),
            }
        }
        let Some(changed) = self.zoom_save else { return };
        let left = ZOOM_SAVE_DELAY.saturating_sub(changed.elapsed());
        if !left.is_zero() {
            ctx.request_repaint_after(left);
            return;
        }
        self.zoom_save = None;
        // Whole 5% steps keep the settings file readable.
        let zoom = (self.zoom * 20.0).round() / 20.0;
        self.set_zoom(ctx, zoom);
        // Half points, for the same reason.
        self.font_size = (self.font_size * 2.0).round() / 2.0;
        self.terminal_font_size = (self.terminal_font_size * 2.0).round() / 2.0;
        let (font, terminal) = (self.font_size, self.terminal_font_size);
        let changes = [
            ("workbench", "zoom", zoom, self.settings.workbench.zoom),
            ("editor", "font_size", font, self.settings.editor.font_size),
            ("terminal", "font_size", terminal, self.settings.terminal.font_size),
        ];
        for (table, key, value, saved) in changes {
            if value == saved {
                continue;
            }
            if let Err(e) = self.write_user_setting(ctx, table, key, &format!("{value:?}")) {
                self.status = Some(e);
            }
        }
        let applied = (self.settings.workbench.zoom, self.settings.editor.font_size, self.settings.terminal.font_size);
        if applied != (zoom, font, terminal) {
            self.status = Some("Zoom saved, but workspace settings override it".to_owned());
        }
    }

    /// Scales one part's font and saves it shortly after.
    pub(super) fn change_font_zoom(&mut self, ctx: &egui::Context, part: ZoomPart, size: f32) {
        let size = size.clamp(FONT_RANGE.0, FONT_RANGE.1);
        match part {
            ZoomPart::Code => self.font_size = size,
            ZoomPart::Terminal => self.terminal_font_size = size,
        }
        self.zoom_save = Some(Instant::now());
        ctx.request_repaint_after(ZOOM_SAVE_DELAY);
    }

    /// The status bar's zoom controls, one row per part.
    pub(super) fn zoom_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        fn row(ui: &mut egui::Ui, name: &str, value: String) -> f32 {
            ui.label(name);
            let out = ui.button("\u{2212}").clicked();
            ui.vertical_centered(|ui| ui.label(value));
            let more = ui.button("+").clicked();
            ui.end_row();
            f32::from(i8::from(more) - i8::from(out))
        }
        egui::Grid::new("zoom_menu").min_col_width(24.0).show(ui, |ui| {
            let step = row(ui, "Window", format!("{:.0}%", self.zoom * 100.0));
            if step != 0.0 {
                self.change_zoom(&ctx, self.zoom + step * ZOOM_STEP);
            }
            let step = row(ui, "Code", format!("{}", self.font_size));
            if step != 0.0 {
                self.change_font_zoom(&ctx, ZoomPart::Code, self.font_size + step);
            }
            let step = row(ui, "Terminal", format!("{}", self.terminal_font()));
            if step != 0.0 {
                self.change_font_zoom(&ctx, ZoomPart::Terminal, self.terminal_font() + step);
            }
        });
        if ui.button("Reset All").clicked() {
            self.change_zoom(&ctx, coiman_core::settings::DEFAULT_ZOOM);
            self.font_size = coiman_core::settings::EditorSettings::default().font_size;
            self.terminal_font_size = 0.0;
        }
    }

    pub(super) fn terminal_font(&self) -> f32 {
        if self.terminal_font_size > 0.0 { self.terminal_font_size } else { self.font_size }
    }
}
