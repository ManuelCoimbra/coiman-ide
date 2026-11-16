//! Counts active time per project for Project: Show Session Statistics: while the window is in
//! front and something was typed or clicked in the last two minutes. Kept on this computer.

use std::path::PathBuf;

use coiman_core::project::activity::{self, Activity};
use eframe::egui;

use crate::app::CodeApp;

/// Without input for this long, time stops counting.
const IDLE_SECS: f64 = 120.0;
/// Time is written to disk about this often.
const FLUSH_SECS: f64 = 60.0;

#[derive(Default)]
pub(crate) struct Tracker {
    log: Option<Activity>,
    last_input: f64,
    last_tick: Option<f64>,
    /// Whether the window was in front at the last frame: the gap since counts only then.
    was_focused: bool,
    /// Seconds counted and not yet written, and for which project.
    pending: f64,
    project: Option<PathBuf>,
    last_flush: f64,
}

impl CodeApp {
    pub(crate) fn track_activity(&mut self, ctx: &egui::Context) {
        let (time, input, focused) =
            ctx.input(|i| (i.time, !i.events.is_empty(), i.viewport().focused.unwrap_or(true)));
        let t = &mut self.activity;
        // egui draws only when something happens: the time since the last frame counts up to
        // where the idle window that the last input opened ends.
        let dt = t.last_tick.map_or(0.0, |last| (time.min(t.last_input + IDLE_SECS) - last).max(0.0));
        t.last_tick = Some(time);
        if input {
            t.last_input = time;
        }
        let root = self.explorer.root.clone();
        if t.project != root {
            self.flush_activity();
            self.activity.project = root;
        }
        let t = &mut self.activity;
        if t.was_focused && t.project.is_some() {
            t.pending += dt;
        }
        t.was_focused = focused;
        if time - t.last_flush >= FLUSH_SECS {
            t.last_flush = time;
            self.flush_activity();
        }
    }

    /// Writes the time counted so far.
    pub(crate) fn flush_activity(&mut self) {
        let t = &mut self.activity;
        let secs = t.pending.floor();
        let Some(project) = t.project.clone() else { return };
        if secs < 1.0 {
            return;
        }
        t.pending -= secs;
        let log = t.log.get_or_insert_with(Activity::load);
        log.add(activity::today(), &project.to_string_lossy(), secs as u64);
        if let Err(e) = log.save() {
            coiman_core::applog::warn(&format!("activity: {e}"));
        }
    }

    /// Project: Show Session Statistics.
    pub(crate) fn show_session_statistics(&mut self) {
        self.flush_activity();
        let log = self.activity.log.get_or_insert_with(Activity::load);
        let markdown = log.report(activity::today());
        self.open_markdown_page(&markdown);
    }

    /// Opens Markdown in a new tab with its preview beside it.
    pub(crate) fn open_markdown_page(&mut self, markdown: &str) {
        self.untitled_counter += 1;
        let doc = coiman_core::editor::document::Document::untitled_with(self.untitled_counter, markdown);
        self.add_doc(doc);
        self.md_preview.open = true;
        self.md_preview.forced = Some(self.docs[self.active].uid);
    }
}
