//! The code editor widget: draws only the visible rows of a rope-backed
//! [`Document`], handles keyboard/mouse/IME/clipboard input itself, and keeps
//! its own scroll position. egui is only used for painting and raw input.

use egui::text::LayoutJob;
use egui::{Color32, Event, EventFilter, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, TextFormat, Vec2};

use crate::buffer::{Buffer, Change, Selection, TAB_WIDTH, char_width, visual_width};
use crate::document::Document;
use crate::theme;

const SCROLLBAR: f32 = 12.0;
/// Width of the gutter strip (left of the text) where change markers sit.
const CHANGE_ZONE: f32 = 12.0;
/// Width of the strip at the gutter's left edge for test run buttons.
const RUN_ZONE: f32 = 14.0;
/// How far to look for a matching bracket around the cursor.
const BRACKET_SEARCH_LIMIT: usize = 20_000;
/// How far to look for a non-blank line when sizing indent guides on blank lines.
const GUIDE_SEARCH_LINES: usize = 100;

/// Display options from settings.
#[derive(Clone, Copy)]
pub struct Options {
    pub font_size: f32,
    pub word_wrap: bool,
    pub indent_guides: bool,
    pub rainbow_brackets: bool,
}

/// Problem severity, as in LSP: 1 error, 2 warning, 3 information, 4 hint.
pub type Severity = u8;

/// A diagnostic shown in the text: a squiggle, plus the message after the
/// line for errors and warnings.
pub struct Mark {
    pub start: usize,
    pub end: usize,
    /// Line of `start`.
    pub line: usize,
    pub severity: Severity,
    pub message: String,
}

/// How a run of lines differs from the version in Git's index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    /// Lines were removed just before `lines.start`.
    Deleted,
}

/// A gutter change marker.
pub struct ChangeMark {
    pub kind: ChangeKind,
    pub lines: std::ops::Range<usize>,
}

/// Extra things drawn over the text (language server and Git).
#[derive(Default)]
pub struct Decorations {
    /// Sorted by `start`.
    pub marks: Vec<Mark>,
    /// Sorted by line.
    pub changes: Vec<ChangeMark>,
    /// Blame text shown after the end of this line.
    pub blame: Option<(usize, String)>,
    /// Lines with a runnable test, and its last result (gutter run buttons).
    pub runnables: Vec<(usize, Option<crate::icons::Icon>, Color32)>,
}

impl Decorations {
    fn change_at(&self, line: usize) -> Option<&ChangeMark> {
        self.changes.iter().find(|c| c.lines.contains(&line) || (c.lines.is_empty() && c.lines.start == line))
    }
}

/// Gutter colors for changes (VS Code's defaults).
fn change_color(kind: ChangeKind) -> Color32 {
    match kind {
        ChangeKind::Added => Color32::from_rgb(0x2E, 0xA0, 0x43),
        ChangeKind::Modified => Color32::from_rgb(0x00, 0x78, 0xD4),
        ChangeKind::Deleted => Color32::from_rgb(0xF8, 0x51, 0x49),
    }
}

/// What happened in the editor this frame, for popups and language features.
pub struct Output {
    /// Char under the mouse pointer (only over text) and the pointer position.
    pub hover: Option<(usize, Pos2)>,
    /// Char clicked with Ctrl held (go to definition).
    pub ctrl_click: Option<usize>,
    /// A gutter change marker was clicked: its line and the pointer position.
    pub change_click: Option<(usize, Pos2)>,
    /// A gutter run button was clicked: its line.
    pub run_click: Option<usize>,
    /// Screen rectangle of the primary cursor.
    pub cursor_rect: Rect,
}

/// Color of a diagnostic severity (VS Code's defaults).
pub fn severity_color(severity: Severity) -> Color32 {
    match severity {
        1 => Color32::from_rgb(0xF1, 0x4C, 0x4C),
        2 => Color32::from_rgb(0xCC, 0xA7, 0x00),
        3 => Color32::from_rgb(0x37, 0x94, 0xFF),
        _ => Color32::from_gray(0x99),
    }
}

pub fn editor_id(uid: u64) -> egui::Id {
    egui::Id::unique(("editor", uid))
}

/// Per-document view state that survives between frames.
#[derive(Default)]
pub struct ViewState {
    /// Scroll offset in points (x: horizontal, y: vertical).
    scroll: Vec2,
    /// Widest line seen so far, in visual columns (for the horizontal scroll range).
    max_cols_seen: usize,
    /// Row layout when word wrap is on, rebuilt when the text or width changes.
    rows: Option<RowMap>,
    /// IME composition text, shown at the cursor until committed.
    preedit: String,
    /// Text last copied in "whole line" mode (copy with no selection), so
    /// pasting it inserts full lines above the cursor like VS Code.
    line_clipboard: Option<String>,
    /// Whether the current mouse drag started on the vertical/horizontal scrollbar.
    dragging_scrollbar: Option<bool>,
    /// Char clicked with Ctrl this frame.
    ctrl_click: Option<usize>,
    /// Gutter change marker clicked this frame.
    change_click: Option<(usize, Pos2)>,
    /// Gutter run button clicked this frame.
    run_click: Option<usize>,
}

/// Word-wrap layout: `first_row[line]` is the first visual row of `line`.
struct RowMap {
    version: u64,
    cols: usize,
    first_row: Vec<usize>,
}

/// One visual row: a whole line, or one wrapped segment of it.
#[derive(Clone, Copy)]
struct Row {
    line: usize,
    /// Char range of the row within the document.
    start: usize,
    end: usize,
    /// Whether this is the first row of its line (shows the line number).
    first: bool,
}

/// Char offsets (relative to the line) where each wrapped segment starts.
/// Breaks after the last space that fits, or mid-word if there is none.
fn wrap_segments(line: &str, cols: usize) -> Vec<usize> {
    let mut starts = vec![0];
    let mut col = 0;
    let mut seg_start = 0;
    let mut last_space: Option<usize> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let w = char_width(chars[i], col);
        if col + w > cols && i > seg_start {
            let brk = match last_space {
                Some(s) if s + 1 > seg_start && s + 1 < i + 1 => s + 1,
                _ => i,
            };
            starts.push(brk);
            seg_start = brk;
            last_space = None;
            col = visual_width(chars[seg_start..i].iter().copied());
            continue;
        }
        if chars[i] == ' ' {
            last_space = Some(i);
        }
        col += w;
        i += 1;
    }
    starts
}

struct Layout {
    wrap_cols: Option<usize>,
    total_rows: usize,
}

impl Layout {
    fn new(doc: &Document, view: &mut ViewState, wrap_cols: Option<usize>) -> Self {
        let buf = &doc.buffer;
        let Some(cols) = wrap_cols else {
            view.rows = None;
            return Self { wrap_cols: None, total_rows: buf.len_lines() };
        };
        let stale = view.rows.as_ref().is_none_or(|r| r.version != buf.version() || r.cols != cols);
        if stale {
            let mut first_row = Vec::with_capacity(buf.len_lines() + 1);
            let mut row = 0;
            for line in 0..buf.len_lines() {
                first_row.push(row);
                row += wrap_segments(&buf.line_text(line), cols).len();
            }
            first_row.push(row);
            view.rows = Some(RowMap { version: buf.version(), cols, first_row });
        }
        let total_rows = *view.rows.as_ref().unwrap().first_row.last().unwrap();
        Self { wrap_cols: Some(cols), total_rows }
    }

    fn row(&self, doc: &Document, view: &ViewState, row: usize) -> Row {
        let buf = &doc.buffer;
        match (self.wrap_cols, &view.rows) {
            (Some(cols), Some(map)) => {
                let line = map.first_row.partition_point(|&r| r <= row).saturating_sub(1).min(buf.len_lines() - 1);
                let seg = row - map.first_row[line];
                let starts = wrap_segments(&buf.line_text(line), cols);
                let ls = buf.line_start(line);
                let le = buf.line_end(line);
                let seg = seg.min(starts.len() - 1);
                let start = ls + starts[seg];
                let end = starts.get(seg + 1).map_or(le, |s| ls + s);
                Row { line, start, end, first: seg == 0 }
            }
            _ => {
                let line = row.min(buf.len_lines() - 1);
                Row { line, start: buf.line_start(line), end: buf.line_end(line), first: true }
            }
        }
    }

    /// Visual row containing `pos`.
    fn row_of(&self, doc: &Document, view: &ViewState, pos: usize) -> usize {
        let buf = &doc.buffer;
        let line = buf.char_to_line(pos);
        match (self.wrap_cols, &view.rows) {
            (Some(cols), Some(map)) => {
                let starts = wrap_segments(&buf.line_text(line), cols);
                let col = pos - buf.line_start(line);
                let seg = starts.partition_point(|&s| s <= col).saturating_sub(1);
                map.first_row[line] + seg
            }
            _ => line,
        }
    }
}

struct Metrics {
    font_id: FontId,
    row_h: f32,
    char_w: f32,
    gutter_w: f32,
}

/// Draws and runs the editor for `doc` in all remaining space of `ui`.
pub fn show(ui: &mut egui::Ui, doc: &mut Document, view: &mut ViewState, opts: Options, deco: &Decorations) -> Output {
    let (font_size, word_wrap) = (opts.font_size, opts.word_wrap);
    let id = editor_id(doc.uid);
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());

    let font_id = FontId::monospace(font_size);
    let row_h = ui.fonts_mut(|f| f.row_height(&font_id));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font_id, '0'));
    let digits = doc.buffer.len_lines().to_string().len().max(3);
    let m = Metrics { gutter_w: digits as f32 * char_w + 28.0 + RUN_ZONE, font_id, row_h, char_w };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + m.gutter_w, rect.top()),
        Pos2::new(rect.right() - SCROLLBAR, rect.bottom()),
    );
    let wrap_cols = word_wrap.then(|| ((text_rect.width() - char_w) / char_w).floor().max(20.0) as usize);

    if let Some(line) = doc.buffer.take_edited_from_line() {
        doc.highlighter.invalidate_from(line);
    }

    // ----- focus ---------------------------------------------------------------
    if doc.request_focus {
        response.request_focus();
        doc.request_focus = false;
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    let has_focus = response.has_focus();
    if has_focus {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
            );
        });
    }
    if response.hovered() && text_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }

    // ----- input ---------------------------------------------------------------
    let mut layout = Layout::new(doc, view, wrap_cols);
    let visible_rows = (text_rect.height() / row_h).floor().max(1.0) as usize;
    let mut reveal = std::mem::take(&mut doc.reveal);

    handle_scrolling(ui, view, &layout, &m, rect, text_rect, &response);
    reveal |= handle_mouse(ui, doc, view, &layout, &m, text_rect, &response, deco);
    if has_focus {
        let version = doc.buffer.version();
        reveal |= handle_keyboard(ui, doc, view, &m, visible_rows);
        if doc.buffer.version() != version {
            if let Some(line) = doc.buffer.take_edited_from_line() {
                doc.highlighter.invalidate_from(line);
            }
            layout = Layout::new(doc, view, wrap_cols);
        }
    }

    // ----- scroll limits and reveal ---------------------------------------------
    let content_h = layout.total_rows as f32 * row_h;
    let max_y = (content_h - row_h).max(0.0);
    let primary = doc.buffer.primary();
    let cursor_row = layout.row_of(doc, view, primary.head);
    let cursor_row_start = layout.row(doc, view, cursor_row).start;
    let cursor_x = col_x(&doc.buffer, cursor_row_start, primary.head, &m);
    if reveal {
        let top = cursor_row as f32 * row_h;
        let view_h = text_rect.height();
        if top < view.scroll.y {
            view.scroll.y = top;
        } else if top + row_h > view.scroll.y + view_h {
            view.scroll.y = top + row_h - view_h;
        }
        if wrap_cols.is_none() {
            let view_w = text_rect.width();
            let margin = 4.0 * char_w;
            if cursor_x < view.scroll.x + margin {
                view.scroll.x = (cursor_x - margin).max(0.0);
            } else if cursor_x > view.scroll.x + view_w - margin {
                view.scroll.x = cursor_x - view_w + margin;
            }
        }
    }
    let max_x = if wrap_cols.is_some() {
        0.0
    } else {
        (view.max_cols_seen as f32 * char_w - text_rect.width() + 4.0 * char_w)
            .max(cursor_x - text_rect.width() + 4.0 * char_w)
            .max(0.0)
    };
    view.scroll.y = view.scroll.y.clamp(0.0, max_y);
    view.scroll.x = view.scroll.x.clamp(0.0, max_x);

    // ----- paint ---------------------------------------------------------------
    paint(ui, doc, view, PaintCtx { layout: &layout, m: &m, rect, text_rect, has_focus, opts, deco });
    paint_scrollbars(ui, view, rect, text_rect, content_h, max_x);

    let row_y = text_rect.top() + cursor_row as f32 * row_h - view.scroll.y;
    let x = text_rect.left() + cursor_x - view.scroll.x;
    let cursor_rect = Rect::from_min_size(Pos2::new(x, row_y), Vec2::new(2.0, row_h));
    if has_focus {
        let to_global = ui.ctx().layer_transform_to_global(ui.layer_id()).unwrap_or_default();
        ui.output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: egui::IMEPurpose::Normal,
                rect: to_global * rect,
                cursor_rect: to_global * cursor_rect,
                should_interrupt_composition: false,
            });
        });
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Editor"));

    // The char under the pointer, only over actual text (not the space after a line).
    let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| text_rect.contains(*p));
    let idle = !ui.input(|i| i.pointer.any_down());
    let hover = pointer.filter(|_| idle && response.hovered()).and_then(|p| {
        let pos = pos_at(doc, view, &layout, &m, text_rect, p - Vec2::new(char_w / 2.0, 0.0));
        let row = layout.row(doc, view, layout.row_of(doc, view, pos));
        let end_x = text_rect.left() + col_x(&doc.buffer, row.start, row.end, &m) - view.scroll.x;
        (p.x < end_x).then_some((pos, p))
    });
    Output {
        hover,
        ctrl_click: view.ctrl_click.take(),
        change_click: view.change_click.take(),
        run_click: view.run_click.take(),
        cursor_rect,
    }
}

/// X offset (in points, relative to the text area) of `pos` on the row starting at `row_start`.
fn col_x(buf: &Buffer, row_start: usize, pos: usize, m: &Metrics) -> f32 {
    let line_start = buf.line_start(buf.char_to_line(row_start));
    let row_col = visual_width(buf.rope().slice(line_start..row_start).chars());
    let pos_col = visual_width(buf.rope().slice(line_start..pos.max(row_start)).chars());
    (pos_col - row_col) as f32 * m.char_w
}

/// Document position under a screen point.
fn pos_at(doc: &Document, view: &ViewState, layout: &Layout, m: &Metrics, text_rect: Rect, p: Pos2) -> usize {
    let y = p.y - text_rect.top() + view.scroll.y;
    let row_idx = ((y / m.row_h).floor().max(0.0) as usize).min(layout.total_rows.saturating_sub(1));
    let row = layout.row(doc, view, row_idx);
    let buf = &doc.buffer;
    let line_start = buf.line_start(row.line);
    let row_col = visual_width(buf.rope().slice(line_start..row.start).chars());
    let x = p.x - text_rect.left() + view.scroll.x;
    let col = (x / m.char_w).round().max(0.0) as usize;
    let pos = buf.pos_at_visual_col(row.line, row_col + col);
    pos.clamp(row.start, row.end)
}

fn handle_scrolling(
    ui: &mut egui::Ui,
    view: &mut ViewState,
    layout: &Layout,
    m: &Metrics,
    rect: Rect,
    text_rect: Rect,
    response: &egui::Response,
) {
    if response.contains_pointer() {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta != Vec2::ZERO {
            view.scroll -= delta;
            // Don't let parent scroll areas also react.
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }

    // Scrollbar dragging: decide on press whether the drag belongs to a scrollbar.
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if response.drag_started() {
        view.dragging_scrollbar = pointer.and_then(|p| {
            if p.x >= text_rect.right() {
                Some(true)
            } else if p.y >= rect.bottom() - SCROLLBAR && layout.wrap_cols.is_none() {
                Some(false)
            } else {
                None
            }
        });
    }
    if !response.dragged() {
        if !response.drag_started() {
            view.dragging_scrollbar = None;
        }
        return;
    }
    let content_h = layout.total_rows as f32 * m.row_h;
    let d = response.drag_delta();
    match view.dragging_scrollbar {
        Some(true) => view.scroll.y += d.y * content_h.max(text_rect.height()) / text_rect.height(),
        Some(false) => {
            let content_w = view.max_cols_seen as f32 * m.char_w;
            view.scroll.x += d.x * content_w.max(text_rect.width()) / text_rect.width();
        }
        None => {}
    }
}

/// Returns true if the cursor moved and should be kept in view.
#[allow(clippy::too_many_arguments)] // the editor's per-frame geometry, passed through
fn handle_mouse(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    layout: &Layout,
    m: &Metrics,
    text_rect: Rect,
    response: &egui::Response,
    deco: &Decorations,
) -> bool {
    if view.dragging_scrollbar.is_some() {
        return false;
    }
    let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return false };
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
    let mods = ui.input(|i| i.modifiers);
    let pos = pos_at(doc, view, layout, m, text_rect, p);

    if pressed && p.x >= text_rect.left() - m.gutter_w && p.x < text_rect.right() {
        let line = doc.buffer.char_to_line(pos);
        let gutter_left = text_rect.left() - m.gutter_w;
        if p.x < gutter_left + RUN_ZONE && deco.runnables.iter().any(|r| r.0 == line) {
            view.run_click = Some(line);
            return false;
        }
        if p.x < text_rect.left() && p.x >= text_rect.left() - CHANGE_ZONE && deco.change_at(line).is_some() {
            view.change_click = Some((line, p));
            return false;
        }
        if p.x < text_rect.left() {
            // Click in the gutter selects the whole line.
            let next = (line + 1 < doc.buffer.len_lines()).then(|| doc.buffer.line_start(line + 1));
            let end = next.unwrap_or_else(|| doc.buffer.len_chars());
            doc.buffer.set_single(Selection::new(doc.buffer.line_start(line), end));
        } else if mods.alt {
            doc.buffer.add_selection(Selection::cursor(pos));
        } else if mods.command {
            doc.buffer.set_single(Selection::cursor(pos));
            view.ctrl_click = Some(pos);
        } else if mods.shift {
            let anchor = doc.buffer.primary().anchor;
            doc.buffer.set_single(Selection::new(anchor, pos));
        } else {
            doc.buffer.set_single(Selection::cursor(pos));
        }
        return false;
    }

    if response.triple_clicked() {
        let line = doc.buffer.char_to_line(pos);
        let next = (line + 1 < doc.buffer.len_lines()).then(|| doc.buffer.line_start(line + 1));
        doc.buffer
            .set_single(Selection::new(doc.buffer.line_start(line), next.unwrap_or_else(|| doc.buffer.len_chars())));
        return false;
    }
    if response.double_clicked() {
        if let Some((s, e)) = doc.buffer.word_at(pos) {
            let mut sels = doc.buffer.selections().to_vec();
            let primary = sels.iter().position(|s| *s == doc.buffer.primary()).unwrap_or(0);
            sels[primary] = Selection::new(s, e);
            doc.buffer.set_selections(sels, primary);
        }
        return false;
    }

    if response.dragged() && !pressed && ui.input(|i| i.pointer.primary_down()) {
        // Extend the selection being dragged (the primary one) to the pointer.
        let mut sels = doc.buffer.selections().to_vec();
        let primary_sel = doc.buffer.primary();
        let idx = sels.iter().position(|s| *s == primary_sel).unwrap_or(0);
        sels[idx] = Selection::new(primary_sel.anchor, pos);
        doc.buffer.set_selections(sels, idx);
        // Auto-scroll when dragging past the top or bottom edge.
        if p.y < text_rect.top() || p.y > text_rect.bottom() {
            ui.ctx().request_repaint();
            return true;
        }
    }
    false
}

/// Returns true if the cursor moved or text changed (keep cursor in view).
fn handle_keyboard(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    m: &Metrics,
    visible_rows: usize,
) -> bool {
    let events = ui.input(|i| i.events.clone());
    let mut moved = false;
    for event in events {
        match event {
            Event::Text(text) => {
                if !text.is_empty() && !text.chars().any(char::is_control) {
                    doc.buffer.insert(&text);
                    moved = true;
                }
            }
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => view.preedit = text,
            Event::Ime(egui::ImeEvent::Commit(text)) => {
                view.preedit.clear();
                if !text.is_empty() {
                    doc.buffer.insert(&text);
                    moved = true;
                }
            }
            Event::Copy => copy(ui, doc, view, false),
            Event::Cut => {
                copy(ui, doc, view, true);
                moved = true;
            }
            Event::Paste(text) => {
                paste(doc, view, &text);
                moved = true;
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                moved |= handle_key(doc, view, key, modifiers, m, visible_rows);
            }
            _ => {}
        }
    }
    moved
}

fn handle_key(
    doc: &mut Document,
    view: &mut ViewState,
    key: Key,
    mods: Modifiers,
    m: &Metrics,
    visible_rows: usize,
) -> bool {
    let b = &mut doc.buffer;
    let (ctrl, shift) = (mods.command, mods.shift);
    match key {
        Key::ArrowLeft | Key::ArrowRight if !mods.alt => b.move_horizontal(key == Key::ArrowRight, ctrl, shift),
        Key::ArrowUp | Key::ArrowDown if !mods.alt => {
            let dir = if key == Key::ArrowDown { 1.0 } else { -1.0 };
            if ctrl {
                // Ctrl+Up/Down scrolls without moving the cursor.
                view.scroll.y += dir * m.row_h;
                return false;
            }
            b.move_vertical(dir as isize, shift);
        }
        Key::PageUp | Key::PageDown => {
            let dir: isize = if key == Key::PageDown { 1 } else { -1 };
            b.move_vertical(dir * visible_rows as isize, shift);
            view.scroll.y += dir as f32 * visible_rows as f32 * m.row_h;
        }
        Key::Home if ctrl => b.move_doc_edge(false, shift),
        Key::End if ctrl => b.move_doc_edge(true, shift),
        Key::Home => b.move_home(shift),
        Key::End => b.move_end(shift),
        Key::Backspace => b.delete_backward(ctrl),
        Key::Delete => b.delete_forward(ctrl),
        Key::Enter if ctrl => {
            // Ctrl+Enter: insert a line below without splitting the current one.
            b.move_end(false);
            b.insert_newline();
        }
        Key::Enter => b.insert_newline(),
        Key::Tab if mods.is_none() => b.insert_tab(),
        Key::A if ctrl => b.select_all(),
        Key::Z if ctrl && shift => {
            b.redo();
        }
        Key::Z if ctrl => {
            b.undo();
        }
        Key::Y if ctrl => {
            b.redo();
        }
        Key::L if ctrl => select_lines(b),
        Key::Escape => {
            if b.selections().len() > 1 {
                b.collapse_to_primary();
            } else if !b.primary().is_empty() {
                let head = b.primary().head;
                b.set_single(Selection::cursor(head));
            } else {
                return false;
            }
        }
        _ => return false,
    }
    true
}

/// Ctrl+L (`expandLineSelection`): selects the current line, then the next.
fn select_lines(b: &mut Buffer) {
    let p = b.primary();
    let first = b.char_to_line(p.start());
    let start = b.line_start(first);
    let mut last = b.char_to_line(p.end());
    // Already a full-line selection: extend by one more line.
    if p.start() == start && p.end() > p.start() && p.end() == b.line_start(last) {
        last = last.max(first);
    } else if p.end() == b.line_start(last) && last > first {
        last -= 1;
    }
    let end = if last + 1 < b.len_lines() { b.line_start(last + 1) } else { b.len_chars() };
    b.set_single(Selection::new(start, end));
}

/// Copy (or cut) the selections. With nothing selected the whole line is copied,
/// like VS Code.
fn copy(ui: &egui::Ui, doc: &mut Document, view: &mut ViewState, cut: bool) {
    let b = &doc.buffer;
    let nl = b.line_ending;
    let line_mode = b.selections().iter().all(Selection::is_empty);
    let text = if line_mode {
        let mut lines: Vec<usize> = b.selections().iter().map(|s| b.char_to_line(s.head)).collect();
        lines.dedup();
        lines.iter().map(|&l| format!("{}{nl}", b.line_text(l))).collect::<String>()
    } else {
        b.selected_texts().join(nl)
    };
    ui.ctx().copy_text(text.clone());
    view.line_clipboard = line_mode.then_some(text);

    if !cut {
        return;
    }
    if line_mode {
        let b = &mut doc.buffer;
        let mut lines: Vec<usize> = b.selections().iter().map(|s| b.char_to_line(s.head)).collect();
        lines.dedup();
        let changes = lines
            .into_iter()
            .map(|l| {
                let start = b.line_start(l);
                let end = if l + 1 < b.len_lines() { b.line_start(l + 1) } else { b.len_chars() };
                Change { start, end, text: String::new(), cursor_offset: None }
            })
            .collect();
        b.apply_changes(changes, false);
    } else {
        doc.buffer.delete_backward(false);
    }
}

fn paste(doc: &mut Document, view: &mut ViewState, text: &str) {
    let nl = doc.buffer.line_ending;
    let normalized = text.replace("\r\n", "\n").replace('\n', nl);
    let b = &mut doc.buffer;
    let all_empty = b.selections().iter().all(Selection::is_empty);
    if all_empty && view.line_clipboard.as_deref() == Some(text) {
        // Whole-line paste goes above the current line, keeping the cursor column.
        let inserted = normalized.chars().count();
        let changes = b
            .selections()
            .iter()
            .map(|s| {
                let start = b.line_start(b.char_to_line(s.head));
                Change { start, end: start, text: normalized.clone(), cursor_offset: Some(inserted + s.head - start) }
            })
            .collect();
        b.apply_changes(changes, false);
        return;
    }
    let pieces: Vec<String> = normalized.split(nl).map(str::to_owned).collect();
    if b.selections().len() > 1 && pieces.len() == b.selections().len() {
        b.insert_each(&pieces);
    } else {
        b.insert(&normalized);
    }
}

/// Everything [`paint`] needs besides the document and its view state.
#[derive(Clone, Copy)]
struct PaintCtx<'a> {
    layout: &'a Layout,
    m: &'a Metrics,
    rect: Rect,
    text_rect: Rect,
    has_focus: bool,
    opts: Options,
    deco: &'a Decorations,
}

fn paint(ui: &mut egui::Ui, doc: &mut Document, view: &mut ViewState, ctx: PaintCtx) {
    let PaintCtx { layout, m, rect, text_rect, has_focus, opts, deco } = ctx;
    let pal = theme::colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::colors().editor_bg);
    let text_painter = painter.with_clip_rect(text_rect);

    let scroll = view.scroll;
    let first_row = (scroll.y / m.row_h).floor() as usize;
    let y_offset = first_row as f32 * m.row_h - scroll.y;
    let rows_on_screen = (rect.height() / m.row_h).ceil() as usize + 1;
    let last_row = (first_row + rows_on_screen).min(layout.total_rows);

    let primary = doc.buffer.primary();
    let primary_line = doc.buffer.char_to_line(primary.head);
    let primary_row = layout.row_of(doc, view, primary.head);
    let bracket = primary.is_empty().then(|| doc.buffer.matching_bracket(primary.head, BRACKET_SEARCH_LIMIT)).flatten();
    let x0 = text_rect.left() - scroll.x;

    // Highlight the visible lines in one pass (tree-sitter documents).
    if last_row > first_row {
        let first_line = layout.row(doc, view, first_row).line;
        let last_line = layout.row(doc, view, last_row - 1).line;
        if let Some(ts) = &mut doc.ts {
            ts.prepare(&doc.buffer, first_line, last_line, opts.rainbow_brackets);
        }
    }

    for (i, row_idx) in (first_row..last_row).enumerate() {
        let row = layout.row(doc, view, row_idx);
        let y = rect.top() + y_offset + i as f32 * m.row_h;
        let buf = &doc.buffer;

        // Indent guides: one line per indentation level, also across blank lines.
        let levels = if opts.indent_guides { indent_cols(buf, row.line) / TAB_WIDTH } else { 0 };
        for level in 0..levels {
            let x = (x0 + (level * TAB_WIDTH) as f32 * m.char_w).round() + 0.5;
            text_painter.vline(x, y..=y + m.row_h, Stroke::new(1.0, pal.indent_guide));
        }

        // Current line highlight.
        if row_idx == primary_row {
            text_painter.rect_stroke(
                Rect::from_min_max(Pos2::new(text_rect.left(), y), Pos2::new(text_rect.right(), y + m.row_h)),
                0.0,
                Stroke::new(1.0, pal.current_line),
                egui::StrokeKind::Inside,
            );
        }

        // Selections on this row; a selection continuing past the line end
        // shows one extra cell for the newline.
        for s in buf.selections() {
            if s.is_empty() || s.end() < row.start || s.start() > row.end {
                continue;
            }
            let a = s.start().max(row.start);
            let b = s.end().min(row.end);
            let xa = x0 + col_x(buf, row.start, a, m);
            let mut xb = x0 + col_x(buf, row.start, b, m);
            if s.end() > row.end && row.end == buf.line_end(row.line) {
                xb += m.char_w * 0.6;
            }
            if xb > xa {
                text_painter.rect_filled(
                    Rect::from_min_max(Pos2::new(xa, y), Pos2::new(xb, y + m.row_h)),
                    0.0,
                    pal.selection,
                );
            }
        }

        // Matching brackets.
        if let Some((open, close)) = bracket {
            for p in [open, close] {
                if p >= row.start && p < row.end {
                    let x = x0 + col_x(buf, row.start, p, m);
                    text_painter.rect_stroke(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(m.char_w, m.row_h)),
                        0.0,
                        Stroke::new(1.0, pal.bracket_match),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        // Text.
        let job = row_job(doc, view, &row, m);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        text_painter.galley(Pos2::new(x0, y), galley, theme::colors().text);
        let buf = &doc.buffer;

        // Diagnostics: squiggles under their text, and the most severe
        // error/warning message after the end of its line.
        let mut inline: Option<&Mark> = None;
        for mark in deco.marks.iter().take_while(|k| k.start <= row.end.max(row.start + 1)) {
            if mark.severity <= 2 && mark.line == row.line && inline.is_none_or(|i| mark.severity < i.severity) {
                inline = Some(mark);
            }
            let before = mark.end < row.start || (mark.end == row.start && mark.start < row.start);
            let after = mark.start > row.end || (mark.start == row.end && mark.end > row.end);
            if mark.severity > 3 || before || after {
                continue;
            }
            let a = mark.start.max(row.start);
            let b = mark.end.min(row.end);
            let xa = x0 + col_x(buf, row.start, a, m);
            let xb = if b > a { x0 + col_x(buf, row.start, b, m) } else { xa + m.char_w };
            squiggle(&text_painter, xa, xb, y + m.row_h - 2.0, severity_color(mark.severity));
        }
        if inline.is_none()
            && row.end == buf.line_end(row.line)
            && let Some((_, blame)) = deco.blame.as_ref().filter(|(line, _)| *line == row.line)
        {
            let x = x0 + col_x(buf, row.start, row.end, m) + 3.0 * m.char_w;
            text_painter.text(Pos2::new(x, y), egui::Align2::LEFT_TOP, blame, m.font_id.clone(), pal.text_dim);
        }
        if let Some(mark) = inline
            && row.end == buf.line_end(row.line)
        {
            let x = x0 + col_x(buf, row.start, row.end, m) + 3.0 * m.char_w;
            let message = mark.message.lines().next().unwrap_or_default();
            text_painter.text(
                Pos2::new(x, y),
                egui::Align2::LEFT_TOP,
                message,
                m.font_id.clone(),
                severity_color(mark.severity).gamma_multiply(0.8),
            );
        }

        // Git change marker: a bar beside the line numbers, or a wedge where lines were deleted.
        if let Some(change) = deco.change_at(row.line) {
            let x = text_rect.left() - CHANGE_ZONE + 2.0;
            let color = change_color(change.kind);
            if change.kind == ChangeKind::Deleted {
                if row.first {
                    let tip = Pos2::new(x + 6.0, y);
                    painter.add(egui::Shape::convex_polygon(
                        vec![Pos2::new(x, y - 4.0), tip, Pos2::new(x, y + 4.0)],
                        color,
                        Stroke::NONE,
                    ));
                }
            } else {
                painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(3.0, m.row_h)), 0.0, color);
            }
        }

        // Test run button (with the last result's icon and color).
        if row.first
            && let Some((_, icon, color)) = deco.runnables.iter().find(|r| r.0 == row.line)
        {
            let size = (m.row_h * 0.8).min(RUN_ZONE);
            let at = Rect::from_center_size(
                Pos2::new(text_rect.left() - m.gutter_w + RUN_ZONE / 2.0 + 1.0, y + m.row_h / 2.0),
                Vec2::splat(size),
            );
            crate::icons::paint(&painter, at, icon.unwrap_or(crate::icons::Icon::Play), *color);
        }

        // Line number.
        if row.first {
            let color =
                if row.line == primary_line { theme::colors().line_number_active } else { theme::colors().line_number };
            painter.text(
                Pos2::new(text_rect.left() - 18.0, y),
                egui::Align2::RIGHT_TOP,
                (row.line + 1).to_string(),
                m.font_id.clone(),
                color,
            );
        }
    }

    // Cursors.
    if has_focus {
        for s in doc.buffer.selections() {
            let row_idx = layout.row_of(doc, view, s.head);
            if row_idx < first_row || row_idx >= last_row {
                continue;
            }
            let row = layout.row(doc, view, row_idx);
            let x = x0 + col_x(&doc.buffer, row.start, s.head, m);
            let y = rect.top() + y_offset + (row_idx - first_row) as f32 * m.row_h;
            text_painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, m.row_h)), 0.0, pal.cursor);
            if *s == primary && !view.preedit.is_empty() {
                let galley = ui.fonts_mut(|f| f.layout_no_wrap(view.preedit.clone(), m.font_id.clone(), pal.editor_fg));
                let w = galley.size().x;
                text_painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, m.row_h)),
                    0.0,
                    theme::colors().editor_bg,
                );
                text_painter.galley(Pos2::new(x, y), galley, pal.editor_fg);
                text_painter.hline(x..=x + w, y + m.row_h - 1.0, Stroke::new(1.0, pal.editor_fg));
            }
        }
    }
}

/// A wavy underline from `x0` to `x1` around height `y`.
fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 + 0.5 {
        points.push(Pos2::new(x, if up { y - 1.0 } else { y + 1.0 }));
        x += 2.0;
        up = !up;
    }
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.0, color)));
    }
}

/// Visual columns of leading whitespace; blank lines take the smaller indent of
/// the nearest non-blank lines above and below (as VS Code does for guides).
fn indent_cols(buf: &Buffer, line: usize) -> usize {
    let leading = |l: usize| -> Option<usize> {
        let text = buf.line_text(l);
        let ws: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        (ws.len() < text.len()).then(|| visual_width(ws.chars()))
    };
    if let Some(cols) = leading(line) {
        return cols;
    }
    let above = (line.saturating_sub(GUIDE_SEARCH_LINES)..line).rev().find_map(leading);
    let below = (line + 1..(line + GUIDE_SEARCH_LINES).min(buf.len_lines())).find_map(leading);
    above.unwrap_or(0).min(below.unwrap_or(0))
}

/// Builds the text for one row: syntax colors, tabs expanded to spaces.
fn row_job(doc: &mut Document, view: &mut ViewState, row: &Row, m: &Metrics) -> LayoutJob {
    let line_start = doc.buffer.line_start(row.line);
    let line_text = doc.buffer.line_text(row.line);
    let chars: Vec<char> = line_text.chars().collect();
    let (seg_a, seg_b) = (row.start - line_start, row.end - line_start);

    // Visual column of the row start (tab stops are relative to the line start).
    let mut col = visual_width(chars[..seg_a].iter().copied());
    let width = col + visual_width(chars[seg_a..seg_b].iter().copied());
    view.max_cols_seen = view.max_cols_seen.max(width);

    // (end char index within line, color, italic) runs.
    let mut runs: Vec<(usize, Color32, bool)> = Vec::new();
    let spans = match &doc.ts {
        Some(ts) => ts.lines.get(&row.line).map(Vec::as_slice),
        None => doc.highlighter.spans(&doc.buffer, row.line),
    };
    match spans {
        Some(spans) => {
            let mut end = 0;
            for s in spans {
                end += s.len;
                runs.push((end, theme::c32(s.color), s.italic));
            }
        }
        None => runs.push((chars.len(), theme::colors().editor_fg, false)),
    }

    let mut job = LayoutJob::default();
    let mut idx = seg_a;
    for (end, color, italic) in runs {
        if end <= idx {
            continue;
        }
        let stop = end.min(seg_b);
        if stop <= idx {
            break;
        }
        let mut piece = String::with_capacity(stop - idx);
        for &c in &chars[idx..stop] {
            let w = char_width(c, col);
            if c == '\t' {
                piece.extend(std::iter::repeat_n(' ', w));
            } else {
                piece.push(c);
            }
            col += w;
        }
        job.append(
            &piece,
            0.0,
            TextFormat { font_id: m.font_id.clone(), color, italics: italic, ..Default::default() },
        );
        idx = stop;
        if idx >= seg_b {
            break;
        }
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

fn paint_scrollbars(ui: &mut egui::Ui, view: &ViewState, rect: Rect, text_rect: Rect, content_h: f32, max_x: f32) {
    let painter = ui.painter_at(rect);
    let thumb = Color32::from_rgba_unmultiplied(0x79, 0x79, 0x79, 0x66);
    let view_h = text_rect.height();
    if content_h > view_h {
        let track = Rect::from_min_max(Pos2::new(text_rect.right(), rect.top()), rect.max);
        let h = (view_h / content_h * track.height()).max(20.0);
        let t = view.scroll.y / (content_h - view_h).max(1.0);
        let top = track.top() + t.clamp(0.0, 1.0) * (track.height() - h);
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(track.left() + 2.0, top), Vec2::new(SCROLLBAR - 4.0, h)),
            3.0,
            thumb,
        );
    }
    if max_x > 0.0 {
        let view_w = text_rect.width();
        let content_w = view_w + max_x;
        let w = (view_w / content_w * view_w).max(20.0);
        let left = text_rect.left() + (view.scroll.x / max_x).clamp(0.0, 1.0) * (view_w - w);
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(left, rect.bottom() - SCROLLBAR + 3.0), Vec2::new(w, SCROLLBAR - 6.0)),
            3.0,
            thumb,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_segments;

    #[test]
    fn wraps_at_last_space_or_mid_word() {
        assert_eq!(wrap_segments("aaaa bbbb cccc", 10), vec![0, 10]);
        assert_eq!(wrap_segments("abcdefghijkl", 5), vec![0, 5, 10]);
        assert_eq!(wrap_segments("short", 10), vec![0]);
        assert_eq!(wrap_segments("", 10), vec![0]);
    }
}
