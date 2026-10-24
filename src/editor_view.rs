//! The code editor widget: draws only the visible rows of a rope-backed
//! [`Document`], handles keyboard/mouse/IME/clipboard input itself, and keeps
//! its own scroll position. egui is only used for painting and raw input.

use egui::text::LayoutJob;
use egui::{Color32, Event, EventFilter, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, TextFormat, Vec2};

use crate::buffer::{Buffer, Change, Selection, TAB_WIDTH, char_width, visual_width};
use crate::document::Document;
use crate::theme;

const SCROLLBAR: f32 = 12.0;
/// How far to look for a matching bracket around the cursor.
const BRACKET_SEARCH_LIMIT: usize = 20_000;
const CURSOR_COLOR: Color32 = Color32::from_rgb(0xae, 0xaf, 0xad);
const SELECTION_COLOR: Color32 = Color32::from_rgb(0x26, 0x4f, 0x78);
const BRACKET_COLOR: Color32 = Color32::from_rgb(0x88, 0x88, 0x88);
const INDENT_GUIDE_COLOR: Color32 = Color32::from_rgb(0x40, 0x40, 0x40);
/// How far to look for a non-blank line when sizing indent guides on blank lines.
const GUIDE_SEARCH_LINES: usize = 100;

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
    fn new(doc: &mut Document, wrap_cols: Option<usize>) -> Self {
        let buf = &doc.buffer;
        let Some(cols) = wrap_cols else {
            doc.view.rows = None;
            return Self { wrap_cols: None, total_rows: buf.len_lines() };
        };
        let stale = doc.view.rows.as_ref().is_none_or(|r| r.version != buf.version() || r.cols != cols);
        if stale {
            let mut first_row = Vec::with_capacity(buf.len_lines() + 1);
            let mut row = 0;
            for line in 0..buf.len_lines() {
                first_row.push(row);
                row += wrap_segments(&buf.line_text(line), cols).len();
            }
            first_row.push(row);
            doc.view.rows = Some(RowMap { version: buf.version(), cols, first_row });
        }
        let total_rows = *doc.view.rows.as_ref().unwrap().first_row.last().unwrap();
        Self { wrap_cols: Some(cols), total_rows }
    }

    fn row(&self, doc: &Document, row: usize) -> Row {
        let buf = &doc.buffer;
        match (self.wrap_cols, &doc.view.rows) {
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
    fn row_of(&self, doc: &Document, pos: usize) -> usize {
        let buf = &doc.buffer;
        let line = buf.char_to_line(pos);
        match (self.wrap_cols, &doc.view.rows) {
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
pub fn show(ui: &mut egui::Ui, doc: &mut Document, font_size: f32, word_wrap: bool) {
    let id = editor_id(doc.uid);
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());

    let font_id = FontId::monospace(font_size);
    let row_h = ui.fonts_mut(|f| f.row_height(&font_id));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font_id, '0'));
    let digits = doc.buffer.len_lines().to_string().len().max(3);
    let m = Metrics { gutter_w: digits as f32 * char_w + 28.0, font_id, row_h, char_w };
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
    let mut layout = Layout::new(doc, wrap_cols);
    let visible_rows = (text_rect.height() / row_h).floor().max(1.0) as usize;
    let mut reveal = std::mem::take(&mut doc.reveal);

    handle_scrolling(ui, doc, &layout, &m, rect, text_rect, &response);
    reveal |= handle_mouse(ui, doc, &layout, &m, text_rect, &response);
    if has_focus {
        let version = doc.buffer.version();
        reveal |= handle_keyboard(ui, doc, &m, visible_rows);
        if doc.buffer.version() != version {
            if let Some(line) = doc.buffer.take_edited_from_line() {
                doc.highlighter.invalidate_from(line);
            }
            layout = Layout::new(doc, wrap_cols);
        }
    }

    // ----- scroll limits and reveal ---------------------------------------------
    let content_h = layout.total_rows as f32 * row_h;
    let max_y = (content_h - row_h).max(0.0);
    let primary = doc.buffer.primary();
    let cursor_row = layout.row_of(doc, primary.head);
    let cursor_row_start = layout.row(doc, cursor_row).start;
    let cursor_x = col_x(&doc.buffer, cursor_row_start, primary.head, &m);
    if reveal {
        let top = cursor_row as f32 * row_h;
        let view_h = text_rect.height();
        if top < doc.view.scroll.y {
            doc.view.scroll.y = top;
        } else if top + row_h > doc.view.scroll.y + view_h {
            doc.view.scroll.y = top + row_h - view_h;
        }
        if wrap_cols.is_none() {
            let view_w = text_rect.width();
            let margin = 4.0 * char_w;
            if cursor_x < doc.view.scroll.x + margin {
                doc.view.scroll.x = (cursor_x - margin).max(0.0);
            } else if cursor_x > doc.view.scroll.x + view_w - margin {
                doc.view.scroll.x = cursor_x - view_w + margin;
            }
        }
    }
    let max_x = if wrap_cols.is_some() {
        0.0
    } else {
        (doc.view.max_cols_seen as f32 * char_w - text_rect.width() + 4.0 * char_w).max(cursor_x - text_rect.width() + 4.0 * char_w).max(0.0)
    };
    doc.view.scroll.y = doc.view.scroll.y.clamp(0.0, max_y);
    doc.view.scroll.x = doc.view.scroll.x.clamp(0.0, max_x);

    // ----- paint ---------------------------------------------------------------
    paint(ui, doc, &layout, &m, rect, text_rect, has_focus);
    paint_scrollbars(ui, doc, rect, text_rect, content_h, max_x);

    if has_focus {
        let row_y = text_rect.top() + cursor_row as f32 * row_h - doc.view.scroll.y;
        let x = text_rect.left() + cursor_x - doc.view.scroll.x;
        let cursor_rect = Rect::from_min_size(Pos2::new(x, row_y), Vec2::new(2.0, row_h));
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
}

/// X offset (in points, relative to the text area) of `pos` on the row starting at `row_start`.
fn col_x(buf: &Buffer, row_start: usize, pos: usize, m: &Metrics) -> f32 {
    let line_start = buf.line_start(buf.char_to_line(row_start));
    let row_col = visual_width(buf.rope().slice(line_start..row_start).chars());
    let pos_col = visual_width(buf.rope().slice(line_start..pos.max(row_start)).chars());
    (pos_col - row_col) as f32 * m.char_w
}

/// Document position under a screen point.
fn pos_at(doc: &Document, layout: &Layout, m: &Metrics, text_rect: Rect, p: Pos2) -> usize {
    let y = p.y - text_rect.top() + doc.view.scroll.y;
    let row_idx = ((y / m.row_h).floor().max(0.0) as usize).min(layout.total_rows.saturating_sub(1));
    let row = layout.row(doc, row_idx);
    let buf = &doc.buffer;
    let line_start = buf.line_start(row.line);
    let row_col = visual_width(buf.rope().slice(line_start..row.start).chars());
    let x = p.x - text_rect.left() + doc.view.scroll.x;
    let col = (x / m.char_w).round().max(0.0) as usize;
    let pos = buf.pos_at_visual_col(row.line, row_col + col);
    pos.clamp(row.start, row.end)
}

fn handle_scrolling(
    ui: &mut egui::Ui,
    doc: &mut Document,
    layout: &Layout,
    m: &Metrics,
    rect: Rect,
    text_rect: Rect,
    response: &egui::Response,
) {
    if response.contains_pointer() {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta != Vec2::ZERO {
            doc.view.scroll -= delta;
            // Don't let parent scroll areas also react.
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }

    // Scrollbar dragging: decide on press whether the drag belongs to a scrollbar.
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if response.drag_started() {
        doc.view.dragging_scrollbar = pointer.and_then(|p| {
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
            doc.view.dragging_scrollbar = None;
        }
        return;
    }
    let content_h = layout.total_rows as f32 * m.row_h;
    let d = response.drag_delta();
    match doc.view.dragging_scrollbar {
        Some(true) => doc.view.scroll.y += d.y * content_h.max(text_rect.height()) / text_rect.height(),
        Some(false) => {
            let content_w = doc.view.max_cols_seen as f32 * m.char_w;
            doc.view.scroll.x += d.x * content_w.max(text_rect.width()) / text_rect.width();
        }
        None => {}
    }
}

/// Returns true if the cursor moved and should be kept in view.
fn handle_mouse(
    ui: &mut egui::Ui,
    doc: &mut Document,
    layout: &Layout,
    m: &Metrics,
    text_rect: Rect,
    response: &egui::Response,
) -> bool {
    if doc.view.dragging_scrollbar.is_some() {
        return false;
    }
    let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return false };
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed());
    let mods = ui.input(|i| i.modifiers);
    let pos = pos_at(doc, layout, m, text_rect, p);

    if pressed && p.x >= text_rect.left() - m.gutter_w && p.x < text_rect.right() {
        if p.x < text_rect.left() {
            // Click in the gutter selects the whole line.
            let line = doc.buffer.char_to_line(pos);
            let next = (line + 1 < doc.buffer.len_lines()).then(|| doc.buffer.line_start(line + 1));
            let end = next.unwrap_or_else(|| doc.buffer.len_chars());
            doc.buffer.set_single(Selection::new(doc.buffer.line_start(line), end));
        } else if mods.alt {
            doc.buffer.add_selection(Selection::cursor(pos));
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
        doc.buffer.set_single(Selection::new(doc.buffer.line_start(line), next.unwrap_or_else(|| doc.buffer.len_chars())));
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
fn handle_keyboard(ui: &mut egui::Ui, doc: &mut Document, m: &Metrics, visible_rows: usize) -> bool {
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
            Event::Ime(egui::ImeEvent::Preedit { text, .. }) => doc.view.preedit = text,
            Event::Ime(egui::ImeEvent::Commit(text)) => {
                doc.view.preedit.clear();
                if !text.is_empty() {
                    doc.buffer.insert(&text);
                    moved = true;
                }
            }
            Event::Copy => copy(ui, doc, false),
            Event::Cut => {
                copy(ui, doc, true);
                moved = true;
            }
            Event::Paste(text) => {
                paste(doc, &text);
                moved = true;
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                moved |= handle_key(doc, key, modifiers, m, visible_rows);
            }
            _ => {}
        }
    }
    moved
}

fn handle_key(doc: &mut Document, key: Key, mods: Modifiers, m: &Metrics, visible_rows: usize) -> bool {
    let b = &mut doc.buffer;
    let (ctrl, shift) = (mods.command, mods.shift);
    match key {
        Key::ArrowLeft | Key::ArrowRight if !mods.alt => b.move_horizontal(key == Key::ArrowRight, ctrl, shift),
        Key::ArrowUp | Key::ArrowDown if !mods.alt => {
            let dir = if key == Key::ArrowDown { 1.0 } else { -1.0 };
            if ctrl {
                // Ctrl+Up/Down scrolls without moving the cursor.
                doc.view.scroll.y += dir * m.row_h;
                return false;
            }
            b.move_vertical(dir as isize, shift);
        }
        Key::PageUp | Key::PageDown => {
            let dir: isize = if key == Key::PageDown { 1 } else { -1 };
            b.move_vertical(dir * visible_rows as isize, shift);
            doc.view.scroll.y += dir as f32 * visible_rows as f32 * m.row_h;
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
fn copy(ui: &egui::Ui, doc: &mut Document, cut: bool) {
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
    doc.view.line_clipboard = line_mode.then_some(text);

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

fn paste(doc: &mut Document, text: &str) {
    let nl = doc.buffer.line_ending;
    let normalized = text.replace("\r\n", "\n").replace('\n', nl);
    let b = &mut doc.buffer;
    let all_empty = b.selections().iter().all(Selection::is_empty);
    if all_empty && doc.view.line_clipboard.as_deref() == Some(text) {
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

fn paint(
    ui: &mut egui::Ui,
    doc: &mut Document,
    layout: &Layout,
    m: &Metrics,
    rect: Rect,
    text_rect: Rect,
    has_focus: bool,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::EDITOR_BG);
    let text_painter = painter.with_clip_rect(text_rect);

    let scroll = doc.view.scroll;
    let first_row = (scroll.y / m.row_h).floor() as usize;
    let y_offset = first_row as f32 * m.row_h - scroll.y;
    let rows_on_screen = (rect.height() / m.row_h).ceil() as usize + 1;
    let last_row = (first_row + rows_on_screen).min(layout.total_rows);

    let primary = doc.buffer.primary();
    let primary_line = doc.buffer.char_to_line(primary.head);
    let primary_row = layout.row_of(doc, primary.head);
    let bracket = primary
        .is_empty()
        .then(|| doc.buffer.matching_bracket(primary.head, BRACKET_SEARCH_LIMIT))
        .flatten();
    let x0 = text_rect.left() - scroll.x;

    // Highlight the visible lines in one pass (tree-sitter documents).
    if last_row > first_row {
        let first_line = layout.row(doc, first_row).line;
        let last_line = layout.row(doc, last_row - 1).line;
        if let Some(ts) = &mut doc.ts {
            ts.prepare(&doc.buffer, first_line, last_line);
        }
    }

    for (i, row_idx) in (first_row..last_row).enumerate() {
        let row = layout.row(doc, row_idx);
        let y = rect.top() + y_offset + i as f32 * m.row_h;
        let buf = &doc.buffer;

        // Indent guides: one line per indentation level, also across blank lines.
        let levels = indent_cols(buf, row.line) / TAB_WIDTH;
        for level in 0..levels {
            let x = (x0 + (level * TAB_WIDTH) as f32 * m.char_w).round() + 0.5;
            text_painter.vline(x, y..=y + m.row_h, Stroke::new(1.0, INDENT_GUIDE_COLOR));
        }

        // Current line highlight.
        if row_idx == primary_row {
            text_painter.rect_stroke(
                Rect::from_min_max(Pos2::new(text_rect.left(), y), Pos2::new(text_rect.right(), y + m.row_h)),
                0.0,
                Stroke::new(1.0, Color32::from_rgb(0x28, 0x28, 0x28)),
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
                text_painter.rect_filled(Rect::from_min_max(Pos2::new(xa, y), Pos2::new(xb, y + m.row_h)), 0.0, SELECTION_COLOR);
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
                        Stroke::new(1.0, BRACKET_COLOR),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        // Text.
        let job = row_job(doc, &row, m);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        text_painter.galley(Pos2::new(x0, y), galley, theme::TEXT);

        // Line number.
        if row.first {
            let color = if row.line == primary_line { theme::LINE_NUMBER_ACTIVE } else { theme::LINE_NUMBER };
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
            let row_idx = layout.row_of(doc, s.head);
            if row_idx < first_row || row_idx >= last_row {
                continue;
            }
            let row = layout.row(doc, row_idx);
            let x = x0 + col_x(&doc.buffer, row.start, s.head, m);
            let y = rect.top() + y_offset + (row_idx - first_row) as f32 * m.row_h;
            text_painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, m.row_h)), 0.0, CURSOR_COLOR);
            if *s == primary && !doc.view.preedit.is_empty() {
                let galley = ui.fonts_mut(|f| f.layout_no_wrap(doc.view.preedit.clone(), m.font_id.clone(), Color32::WHITE));
                let w = galley.size().x;
                text_painter.rect_filled(Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, m.row_h)), 0.0, theme::EDITOR_BG);
                text_painter.galley(Pos2::new(x, y), galley, Color32::WHITE);
                text_painter.hline(x..=x + w, y + m.row_h - 1.0, Stroke::new(1.0, Color32::WHITE));
            }
        }
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
fn row_job(doc: &mut Document, row: &Row, m: &Metrics) -> LayoutJob {
    let line_start = doc.buffer.line_start(row.line);
    let line_text = doc.buffer.line_text(row.line);
    let chars: Vec<char> = line_text.chars().collect();
    let (seg_a, seg_b) = (row.start - line_start, row.end - line_start);

    // Visual column of the row start (tab stops are relative to the line start).
    let mut col = visual_width(chars[..seg_a].iter().copied());
    let width = col + visual_width(chars[seg_a..seg_b].iter().copied());
    doc.view.max_cols_seen = doc.view.max_cols_seen.max(width);

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
                runs.push((end, s.color, s.italic));
            }
        }
        None => runs.push((chars.len(), theme::TEXT, false)),
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
        job.append(&piece, 0.0, TextFormat { font_id: m.font_id.clone(), color, italics: italic, ..Default::default() });
        idx = stop;
        if idx >= seg_b {
            break;
        }
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

fn paint_scrollbars(ui: &mut egui::Ui, doc: &Document, rect: Rect, text_rect: Rect, content_h: f32, max_x: f32) {
    let painter = ui.painter_at(rect);
    let thumb = Color32::from_rgba_unmultiplied(0x79, 0x79, 0x79, 0x66);
    let view_h = text_rect.height();
    if content_h > view_h {
        let track = Rect::from_min_max(Pos2::new(text_rect.right(), rect.top()), rect.max);
        let h = (view_h / content_h * track.height()).max(20.0);
        let t = doc.view.scroll.y / (content_h - view_h).max(1.0);
        let top = track.top() + t.clamp(0.0, 1.0) * (track.height() - h);
        painter.rect_filled(Rect::from_min_size(Pos2::new(track.left() + 2.0, top), Vec2::new(SCROLLBAR - 4.0, h)), 3.0, thumb);
    }
    if max_x > 0.0 {
        let view_w = text_rect.width();
        let content_w = view_w + max_x;
        let w = (view_w / content_w * view_w).max(20.0);
        let left = text_rect.left() + (doc.view.scroll.x / max_x).clamp(0.0, 1.0) * (view_w - w);
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
