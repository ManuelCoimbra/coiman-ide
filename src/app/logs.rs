//! The log view: files of any size, read a screen at a time, with a regex filter
//! and following. It takes the editor's place, and stays running behind other tabs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, Key, Pos2, Rect, RichText, Sense, Vec2};
use rust_code_core::logfile::{self, FilterPlace, Level, LineIndex};

use super::{CodeApp, repaint};
use crate::theme;

/// How often to look for new lines.
const POLL: Duration = Duration::from_millis(400);
/// The most lines one copy takes.
const COPY_LINES: usize = 1_000_000;
/// What the index reads between showing progress.
const INDEX_STEP: u64 = 64 * 1024 * 1024;

/// One filter's results, added to as the file is read.
#[derive(Default)]
struct Matches {
    lines: Vec<u64>,
    place: FilterPlace,
    /// It went through everything there was, last it looked.
    caught_up: bool,
}

struct Filter {
    pattern: String,
    match_case: bool,
    invert: bool,
    matches: Arc<Mutex<Matches>>,
    stop: Arc<AtomicBool>,
}

impl Drop for Filter {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(super) struct LogView {
    path: PathBuf,
    index: Arc<RwLock<LineIndex>>,
    stop: Arc<AtomicBool>,
    problem: Arc<Mutex<Option<String>>>,
    /// In front, rather than behind a tab.
    showing: bool,
    /// The document that was active when it came to the front.
    over: Option<u64>,
    filter_text: String,
    match_case: bool,
    invert: bool,
    filter: Option<Filter>,
    filter_problem: Option<String>,
    follow: bool,
    /// The view's row selected, as a line of the file.
    selected: Option<u64>,
    /// Where a selection of several lines started.
    anchor: Option<u64>,
    goto_text: String,
    /// Scroll here next frame: a row.
    scroll_to_row: Option<u64>,
    /// The lines read last: (first line asked for, generation, how many lines there were, text).
    cache: Option<(u64, usize, u32, u64, Vec<String>)>,
    widest: f32,
    /// A generation the view has been told about.
    seen_generation: u32,
    focus: bool,
}

impl Drop for LogView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
pub(super) struct Logs {
    views: Vec<LogView>,
    /// Saved last time, and not started yet.
    restoring: Vec<PathBuf>,
    /// Files someone chose to open in the editor, large or not.
    pub(super) editor_anyway: std::collections::HashSet<PathBuf>,
}

fn start_index(path: &Path, ctx: &egui::Context) -> LogView {
    let index = Arc::new(RwLock::new(LineIndex::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let problem = Arc::new(Mutex::new(None));
    let (file, shared, halt, trouble, wake) =
        (path.to_path_buf(), index.clone(), stop.clone(), problem.clone(), repaint(ctx));
    std::thread::spawn(move || {
        while !halt.load(Ordering::Relaxed) {
            // Read outside the lock: the UI wants it every frame.
            let mut copy = shared.read().map(|i| i.clone()).unwrap_or_default();
            match copy.update(&file, INDEX_STEP) {
                Ok((changed, more)) => {
                    if let Ok(mut slot) = trouble.lock() {
                        *slot = None;
                    }
                    if changed {
                        if let Ok(mut live) = shared.write() {
                            *live = copy;
                        }
                        wake();
                    }
                    if !more {
                        std::thread::sleep(POLL);
                    }
                }
                Err(e) => {
                    if let Ok(mut slot) = trouble.lock() {
                        *slot = Some(e.to_string());
                    }
                    wake();
                    std::thread::sleep(POLL);
                }
            }
        }
    });
    LogView {
        path: path.to_path_buf(),
        index,
        stop,
        problem,
        showing: true,
        over: None,
        filter_text: String::new(),
        match_case: false,
        invert: false,
        filter: None,
        filter_problem: None,
        follow: false,
        selected: None,
        anchor: None,
        goto_text: String::new(),
        scroll_to_row: None,
        cache: None,
        widest: 0.0,
        seen_generation: 0,
        focus: true,
    }
}

/// Starts a filter, which keeps up with the file until it's replaced.
fn start_filter(view: &LogView, ctx: &egui::Context) -> Option<Filter> {
    let regex = match logfile::compile_filter(&view.filter_text, view.match_case) {
        Ok(regex) => regex?,
        Err(_) => return None,
    };
    let matches = Arc::new(Mutex::new(Matches::default()));
    let stop = Arc::new(AtomicBool::new(false));
    let (shared, halt, file, invert, wake) =
        (matches.clone(), stop.clone(), view.path.clone(), view.invert, repaint(ctx));
    std::thread::spawn(move || {
        let mut place = FilterPlace::default();
        let mut size = 0;
        while !halt.load(Ordering::Relaxed) {
            let now = std::fs::metadata(&file).map_or(0, |m| m.len());
            if now < size {
                // Another file now: start over.
                place = FilterPlace::default();
                if let Ok(mut m) = shared.lock() {
                    *m = Matches::default();
                }
            }
            size = now;
            if place.offset >= now {
                if let Ok(mut m) = shared.lock() {
                    m.caught_up = true;
                }
                wake();
                std::thread::sleep(POLL);
                continue;
            }
            let mut batch = Vec::new();
            let step = logfile::scan_matches(&file, &regex, invert, place, INDEX_STEP, &halt, |l| {
                batch.push(l);
            });
            let Ok(next) = step else {
                std::thread::sleep(POLL);
                continue;
            };
            if let Ok(mut m) = shared.lock() {
                m.lines.extend(batch);
                m.place = next;
                m.caught_up = false;
            }
            wake();
            // Only part of a line is left: nothing to do until it ends.
            if next.offset == place.offset {
                std::thread::sleep(POLL);
            }
            place = next;
        }
    });
    Some(Filter { pattern: view.filter_text.clone(), match_case: view.match_case, invert: view.invert, matches, stop })
}

fn level_color(level: Level, fg: Color32, dim: Color32) -> Color32 {
    match level {
        Level::Error => Color32::from_rgb(0xF1, 0x4C, 0x4C),
        Level::Warning => Color32::from_rgb(0xCC, 0xA7, 0x00),
        Level::Quiet => dim,
        Level::Plain => fg,
    }
}

impl CodeApp {
    pub(super) fn open_log_view(&mut self, ctx: &egui::Context, path: &Path) {
        self.hide_view_tabs();
        let over = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = self.logs.views.iter_mut().find(|v| v.path == path) {
            view.showing = true;
            view.over = over;
            return;
        }
        let mut view = start_index(path, ctx);
        view.over = over;
        self.logs.views.push(view);
    }

    pub(super) fn restore_log_views(&mut self, paths: Vec<PathBuf>) {
        self.logs.restoring = paths;
    }

    pub(super) fn log_tabs(&self) -> Vec<(String, bool)> {
        let name = |v: &LogView| v.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.logs.views.iter().map(|v| (name(v), v.showing)).collect()
    }

    pub(super) fn hide_logs(&mut self) {
        self.logs.views.iter_mut().for_each(|v| v.showing = false);
    }

    pub(super) fn show_log_tab(&mut self, i: usize) {
        let over = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = self.logs.views.get_mut(i) {
            view.showing = true;
            view.over = over;
        }
    }

    pub(super) fn close_log_tab(&mut self, i: usize) {
        if i < self.logs.views.len() {
            self.logs.views.remove(i);
        }
    }

    pub(super) fn log_view_paths(&self) -> Vec<PathBuf> {
        let open = self.logs.views.iter().map(|v| v.path.clone());
        open.chain(self.logs.restoring.iter().cloned()).collect()
    }

    /// `Log: Open in Log View`: the active file, or one to choose.
    pub(super) fn log_view_command(&mut self, ctx: &egui::Context) {
        let current = self.docs.get(self.active).and_then(|d| d.path.clone());
        let picked = || self.file_dialog("log_file").pick_file();
        if let Some(path) = current.or_else(picked) {
            self.dialog_chose("log_file", &path);
            self.open_log_view(ctx, &path);
        }
    }

    pub(super) fn log_view_path(&self) -> Option<PathBuf> {
        self.logs.views.iter().find(|v| v.showing).map(|v| v.path.clone())
    }

    /// The status bar's way back to it, from behind a tab.
    pub(super) fn log_label(&self) -> Option<String> {
        let view = self.logs.views.first().filter(|_| !self.logs.views.iter().any(|v| v.showing))?;
        let name = view.path.file_name()?.to_string_lossy().into_owned();
        Some(format!("Log: {name}"))
    }

    pub(super) fn show_log_view(&mut self) {
        self.hide_view_tabs();
        self.show_log_tab(0);
    }

    /// Shows the log view in the editor's place. Says whether it did.
    pub(super) fn log_view(&mut self, ui: &mut egui::Ui) -> bool {
        let active = self.docs.get(self.active).map(|d| d.uid);
        let ctx = ui.ctx().clone();
        let font_size = self.font_size;
        for path in std::mem::take(&mut self.logs.restoring) {
            if !self.logs.views.iter().any(|v| v.path == path) {
                let mut view = start_index(&path, &ctx);
                view.showing = false;
                view.focus = false;
                self.logs.views.push(view);
            }
        }
        // Another tab was chosen. These go on reading, and their tabs bring them back.
        for view in self.logs.views.iter_mut().filter(|v| v.over != active) {
            view.showing = false;
        }
        let Some(shown) = self.logs.views.iter().position(|v| v.showing) else { return false };
        let view = &mut self.logs.views[shown];
        let pal = theme::colors();
        let (index_lines, scanned, generation) =
            view.index.read().map(|i| (i.lines(), i.scanned, i.generation)).unwrap_or_default();
        let size = std::fs::metadata(&view.path).map_or(0, |m| m.len());
        if generation != view.seen_generation {
            // Rotated or truncated: what the view knew no longer holds.
            view.seen_generation = generation;
            view.selected = None;
            view.anchor = None;
            view.cache = None;
            // Its matches were of the file that's gone.
            view.filter = None;
        }

        let mut close = false;
        let mut analyze = None;
        let mut goto = None;
        let mut in_editor = false;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let name = view.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            ui.label(RichText::new(name).strong());
            let edit =
                egui::TextEdit::singleline(&mut view.filter_text).hint_text("Filter: a regex").desired_width(240.0);
            let response = ui.add(edit);
            if std::mem::take(&mut view.focus) {
                response.request_focus();
            }
            ui.toggle_value(&mut view.match_case, "Aa").on_hover_text("Match case");
            ui.toggle_value(&mut view.invert, "Not").on_hover_text("Lines that don't match");
            let was = view.follow;
            ui.toggle_value(&mut view.follow, "Follow").on_hover_text("Stay at the end as lines arrive");
            if view.follow && !was {
                view.scroll_to_row = Some(u64::MAX);
            }
            if ui
                .button("Trace")
                .on_hover_text("Analyze the selected line and those after it as a stack trace")
                .clicked()
            {
                analyze = view.selected;
            }
            let edit = egui::TextEdit::singleline(&mut view.goto_text).hint_text("Go to line").desired_width(80.0);
            let response = ui.add(edit);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                goto = view.goto_text.trim().parse::<u64>().ok();
            }
            in_editor = ui
                .button("Open in Editor")
                .on_hover_text("Load all of it into the editor, which can take a while and a lot of memory")
                .clicked();
            close = ui.button("Close").clicked();
        });

        // The filter follows what's typed, once the pattern is valid.
        let wanted = !view.filter_text.is_empty();
        let changed = match &view.filter {
            Some(f) => f.pattern != view.filter_text || f.match_case != view.match_case || f.invert != view.invert,
            None => wanted,
        };
        if changed {
            view.filter_problem = logfile::compile_filter(&view.filter_text, view.match_case).err();
            if view.filter_problem.is_none() {
                view.filter = start_filter(view, &ctx);
                view.cache = None;
            }
        }
        let (matched, caught_up) = view
            .filter
            .as_ref()
            .and_then(|f| f.matches.lock().ok().map(|m| (m.lines.len() as u64, m.caught_up)))
            .unwrap_or((0, true));

        ui.horizontal(|ui| {
            ui.add_space(8.0);
            let mut text = match &view.filter {
                Some(_) => format!("{matched} of {index_lines} lines"),
                None if index_lines == 1 => "1 line".to_owned(),
                None => format!("{index_lines} lines"),
            };
            text.push_str(&format!(", {:.1} MB", size as f64 / (1024.0 * 1024.0)));
            ui.label(RichText::new(text).small().color(pal.text_dim));
            if scanned < size {
                let done = scanned as f64 / size.max(1) as f64 * 100.0;
                ui.label(RichText::new(format!("indexing {done:.0}%")).small().color(pal.text_dim));
            } else if view.filter.is_some() && !caught_up {
                ui.label(RichText::new("filtering…").small().color(pal.text_dim));
            }
            let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
            if let Some(problem) = &view.filter_problem {
                ui.label(RichText::new(format!("Filter: {problem}")).small().color(warn));
            }
            if let Some(problem) = view.problem.lock().ok().and_then(|p| p.clone()) {
                ui.label(RichText::new(problem).small().color(warn));
            }
            if generation > 0 {
                ui.label(
                    RichText::new(format!("Started over {generation}×: the file was replaced")).small().color(warn),
                );
            }
        });

        // The list.
        let font = egui::FontId::monospace(font_size);
        let row_h = (font_size * 1.4).round();
        let rows = if view.filter.is_some() { matched } else { index_lines };
        let height = rows as f64 * f64::from(row_h);
        let digits = index_lines.max(1).to_string().len().max(3);
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, 'x'));
        let margin = digits as f32 * char_w + 14.0;
        let mut area = egui::ScrollArea::both().id_salt(("log_view", &view.path)).auto_shrink([false, false]);
        if view.follow {
            view.scroll_to_row = Some(u64::MAX);
        }
        if let Some(row) = view.scroll_to_row.take() {
            let room = ui.available_height();
            // Past the end a little: the scroll bar along the bottom would cover the last line.
            let y = if row == u64::MAX {
                height - f64::from(room) + f64::from(row_h) * 2.0
            } else {
                row as f64 * f64::from(row_h) - f64::from(room / 2.0)
            };
            area = area.vertical_scroll_offset(y.max(0.0) as f32);
        }
        let mut copy = None;
        let mut key_move = None;
        let mut reveal = None;
        let focus_id = ui.make_persistent_id("log_rows");
        let output = area.show_viewport(ui, |ui, viewport| {
            ui.set_min_size(Vec2::new((margin + view.widest).max(viewport.width()), height as f32));
            let screen = ui.clip_rect();
            let top = f64::from(viewport.top());
            let first = (top / f64::from(row_h)) as u64;
            let count = (screen.height() / row_h) as usize + 2;
            let last = (first + count as u64).min(rows);
            let body = ui.interact(screen, focus_id, Sense::click());
            let row_of = |p: Pos2| ((f64::from(p.y - screen.top()) + top) / f64::from(row_h)) as u64;
            // Filtered, a row is a matching line; otherwise the line itself.
            let line_of_row = |row: u64| -> Option<u64> {
                match &view.filter {
                    Some(f) => f.matches.lock().ok()?.lines.get(row as usize).copied(),
                    None => (row < index_lines).then_some(row),
                }
            };
            let wanted_lines: Vec<u64> = (first..last).filter_map(line_of_row).collect();
            // Reading the same lines again each frame would hit the disk at 60 Hz.
            let key = (first, wanted_lines.len(), generation, index_lines);
            let current = view.cache.as_ref().is_some_and(|c| (c.0, c.1, c.2, c.3) == key);
            if !current {
                let index = view.index.read().map(|i| i.clone()).unwrap_or_default();
                let texts: Vec<String> = if view.filter.is_some() {
                    wanted_lines
                        .iter()
                        .map(|l| index.read_lines(&view.path, *l, 1).ok().and_then(|mut v| v.pop()).unwrap_or_default())
                        .collect()
                } else {
                    index.read_lines(&view.path, first, wanted_lines.len()).unwrap_or_default()
                };
                view.cache = Some((key.0, key.1, key.2, key.3, texts));
            }
            let painter = ui.painter_at(screen);
            let left = screen.left() - viewport.left();
            let texts = view.cache.as_ref().map_or(&[][..], |c| c.4.as_slice());
            for (at, line) in wanted_lines.iter().enumerate() {
                let y = screen.top() + ((first + at as u64) as f64 * f64::from(row_h) - top) as f32;
                let text = texts.get(at).map_or("", String::as_str);
                let bar = Rect::from_min_size(Pos2::new(screen.left(), y), Vec2::new(screen.width(), row_h));
                let lo = view.selected.map(|s| view.anchor.map_or(s, |a| a.min(s)));
                let hi = view.selected.map(|s| view.anchor.map_or(s, |a| a.max(s)));
                if lo.zip(hi).is_some_and(|(lo, hi)| lo <= *line && *line <= hi) {
                    painter.rect_filled(bar, 0.0, pal.selection);
                }
                let color = level_color(logfile::level_of(text), pal.editor_fg, pal.text_dim);
                let shown: std::borrow::Cow<str> =
                    if text.contains('\t') { text.replace('\t', "    ").into() } else { text.into() };
                let at_x = Pos2::new(left + margin, bar.center().y);
                let body_painter = painter.with_clip_rect(screen.with_min_x(screen.left() + margin));
                let rect = body_painter.text(at_x, Align2::LEFT_CENTER, shown.as_ref(), font.clone(), color);
                view.widest = view.widest.max(rect.width() + 20.0);
            }
            let gutter = Rect::from_min_size(screen.min, Vec2::new(margin, screen.height()));
            painter.rect_filled(gutter, 0.0, pal.editor_bg);
            for (at, line) in wanted_lines.iter().enumerate() {
                let y = screen.top() + ((first + at as u64) as f64 * f64::from(row_h) - top) as f32;
                let at = Pos2::new(screen.left() + margin - 8.0, y + row_h / 2.0);
                painter.text(at, Align2::RIGHT_CENTER, (line + 1).to_string(), font.clone(), pal.line_number);
            }
            if let Some(p) = body.interact_pointer_pos().filter(|_| body.clicked() || body.double_clicked()) {
                ui.memory_mut(|m| m.request_focus(focus_id));
                if let Some(line) = line_of_row(row_of(p)) {
                    view.anchor = if ui.input(|i| i.modifiers.shift) { view.anchor.or(view.selected) } else { None };
                    view.selected = Some(line);
                    // From a filtered row to the lines around it.
                    if body.double_clicked() && view.filter.is_some() {
                        reveal = Some(line);
                    }
                }
            }
            if ui.memory(|m| m.has_focus(focus_id)) {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(
                        focus_id,
                        egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
                    );
                });
                let page = i64::from(count as i32 - 2);
                key_move = ui.input_mut(|i| {
                    let mut step = None;
                    for (key, by) in [
                        (Key::ArrowDown, 1),
                        (Key::ArrowUp, -1),
                        (Key::PageDown, page),
                        (Key::PageUp, -page),
                        (Key::Home, i64::MIN / 2),
                        (Key::End, i64::MAX / 2),
                    ] {
                        // Shift first: a plain key's match accepts Shift too.
                        if i.consume_key(egui::Modifiers::SHIFT, key) {
                            step = Some((by, true));
                        }
                        if i.consume_key(egui::Modifiers::NONE, key) {
                            step = Some((by, false));
                        }
                    }
                    step
                });
                if ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
                    copy = view.selected.map(|s| {
                        let a = view.anchor.unwrap_or(s);
                        (a.min(s), a.max(s))
                    });
                }
            }
            viewport.top()
        });
        // Scrolling away from the end stops following.
        let at_end = output.inner + output.inner_rect.height() >= height as f32 - row_h * 2.0;
        if view.follow && !at_end && ui.input(|i| i.smooth_scroll_delta.y > 0.0) {
            view.follow = false;
        }
        if let Some((by, extend)) = key_move {
            view.anchor = if extend { view.anchor.or(view.selected) } else { None };
            let matches = view.filter.as_ref().and_then(|f| f.matches.lock().ok().map(|m| m.lines.clone()));
            let row_now = match (&matches, view.selected) {
                (Some(lines), Some(line)) => lines.partition_point(|l| *l < line) as i64,
                (None, Some(line)) => line as i64,
                _ => (output.inner / row_h) as i64,
            };
            let row = row_now.saturating_add(by).clamp(0, (rows as i64 - 1).max(0));
            view.selected = match &matches {
                Some(lines) => lines.get(row as usize).copied(),
                None => Some(row as u64),
            };
            view.scroll_to_row = Some(row as u64);
            view.follow = false;
        }
        if let Some(line) = goto.map(|l| l.saturating_sub(1).min(index_lines.saturating_sub(1))) {
            let row = match view.filter.as_ref().and_then(|f| f.matches.lock().ok()) {
                Some(matches) => matches.lines.partition_point(|l| *l < line) as u64,
                None => line,
            };
            view.selected = Some(line);
            view.anchor = None;
            view.scroll_to_row = Some(row);
            view.follow = false;
        }
        if let Some(line) = reveal {
            view.filter_text.clear();
            view.filter = None;
            view.cache = None;
            view.selected = Some(line);
            view.scroll_to_row = Some(line);
            view.follow = false;
        }
        if let Some((lo, hi)) = copy {
            let index = view.index.read().map(|i| i.clone()).unwrap_or_default();
            let count = ((hi - lo + 1) as usize).min(COPY_LINES);
            let mut texts = index.read_lines(&view.path, lo, count).unwrap_or_default();
            // Filtered, what's selected is the lines that show.
            if let Some(matches) = view.filter.as_ref().and_then(|f| f.matches.lock().ok()) {
                let mut at = lo;
                texts.retain(|_| {
                    let keep = matches.lines.binary_search(&at).is_ok();
                    at += 1;
                    keep
                });
            }
            ctx.copy_text(texts.join("\n"));
        }
        if let Some(line) = analyze {
            let index = view.index.read().map(|i| i.clone()).unwrap_or_default();
            let text = index.read_lines(&view.path, line, 200).unwrap_or_default().join("\n");
            self.analyze_stack_trace_text(text);
        }
        if close {
            self.logs.views.remove(shown);
        } else if in_editor {
            let path = self.logs.views.remove(shown).path.clone();
            self.logs.editor_anyway.insert(path.clone());
            self.open_file(&path);
        }
        true
    }
}
