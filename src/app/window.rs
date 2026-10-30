//! Fits the window to the screen on launch and remembers its size. The position
//! isn't kept: a saved spot can be on a monitor that is no longer there.

use eframe::egui::{self, ViewportCommand};
use rust_code_core::session::WindowState;

use super::CodeApp;

/// Share of the screen a first launch may take, leaving room for a taskbar.
const FIRST_LAUNCH_SHARE: f32 = 0.9;

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
            _ => inner.size().into(),
        };
        self.window = Some(WindowState { width, height, maximized });
    }
}
