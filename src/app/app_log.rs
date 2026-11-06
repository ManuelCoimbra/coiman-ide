//! What goes in the regular log from the workbench, so that it can be read beside a screenshot.

use rust_code_core::applog;

use super::CodeApp;
use crate::theme;

impl CodeApp {
    /// Logs clicks, and each change to the status message or to what's on screen.
    pub(super) fn log_what_changed(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::PointerButton { pos, button, pressed: true, .. } = event {
                    applog::info(&format!("click {button:?} at {:.0},{:.0}", pos.x, pos.y));
                }
            }
        });
        // Waits for an empty status line, so as not to hide a message set this frame.
        if self.status.is_none()
            && let Some(text) = applog::take_last_crash()
        {
            self.status = Some(text);
        }
        if self.status != self.logged_status {
            self.logged_status.clone_from(&self.status);
            if let Some(status) = &self.status {
                applog::info(&format!("status: {status}"));
            }
        }
        let screen = self.describe_screen(ctx);
        if screen != self.logged_ui {
            applog::info(&format!("screen: {screen}"));
            self.logged_ui = screen;
        }
    }

    /// One line on what a screenshot should show now.
    fn describe_screen(&self, ctx: &egui::Context) -> String {
        let size = ctx.content_rect().size();
        let layout = if theme::islands() { "islands" } else { "classic" };
        let left = if self.sidebar_visible { format!("{:?}", self.sidebar_view) } else { "none".to_owned() };
        let right = self.rails.right_view.map_or_else(|| "none".to_owned(), |v| format!("{v:?}"));
        let panel = match (self.panel_visible, self.rails.panel_right) {
            (false, _) => "none".to_owned(),
            (true, false) => format!("{:?} at bottom", self.panel_tab),
            (true, true) => format!("{:?} at right", self.panel_tab),
        };
        let file = self.docs.get(self.active).map_or_else(|| "none".to_owned(), |d| d.title());
        format!(
            "{:.0}x{:.0} {layout} left={left} right={right} panel={panel} file={file} tabs={} palette={} settings={} zen={}",
            size.x,
            size.y,
            self.docs.len(),
            self.palette.describe(),
            self.settings_ui.open,
            self.zen_mode,
        )
    }
}
