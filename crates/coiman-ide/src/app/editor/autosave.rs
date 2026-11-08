//! Auto save (`files.auto_save`): shortly after typing stops, or when the window
//! or the active tab loses focus. Only files that have a path and no conflict
//! with the disk. Unsaved text is safe either way: hot exit keeps it.

use std::collections::HashMap;

use eframe::egui;

use crate::app::CodeApp;
use crate::document::DiskState;

#[derive(Default)]
pub(crate) struct AutoSave {
    /// By document: version last seen, when it changed, whether a save failed.
    seen: HashMap<u64, (u64, f64, bool)>,
    window_focused: bool,
    active: Option<u64>,
    /// Saves after a delay happen too often to each be a history entry.
    pub(crate) skip_history: bool,
}

impl CodeApp {
    fn can_auto_save(&self, i: usize) -> bool {
        let doc = &self.docs[i];
        doc.is_dirty() && doc.path.is_some() && doc.disk == DiskState::InSync
    }

    pub(crate) fn pump_auto_save(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
        let active = self.docs.get(self.active).map(|d| d.uid);
        let left_window = std::mem::replace(&mut self.auto_save.window_focused, focused) && !focused;
        let left_tab = std::mem::replace(&mut self.auto_save.active, active) != active;
        match self.settings.files.auto_save.as_str() {
            "after_delay" => self.auto_save_after_delay(ctx),
            "focus_change" if left_window || left_tab => {
                for i in 0..self.docs.len() {
                    if self.can_auto_save(i) {
                        self.save_doc(i);
                    }
                }
            }
            _ => {}
        }
    }

    fn auto_save_after_delay(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let delay = self.settings.files.auto_save_delay_ms as f64 / 1000.0;
        for i in 0..self.docs.len() {
            if !self.can_auto_save(i) {
                continue;
            }
            let (uid, version) = (self.docs[i].uid, self.docs[i].buffer.version());
            let entry = self.auto_save.seen.entry(uid).or_insert((version, now, false));
            if entry.0 != version {
                *entry = (version, now, false);
            }
            // A failed save waits for the next edit rather than retrying forever.
            if entry.2 {
                continue;
            }
            let waited = now - entry.1;
            if waited < delay {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(delay - waited));
                continue;
            }
            entry.2 = true;
            self.auto_save.skip_history = true;
            self.save_doc(i);
            self.auto_save.skip_history = false;
        }
    }
}
