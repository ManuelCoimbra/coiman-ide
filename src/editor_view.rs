//! The code editor widget: draws only the visible rows of a rope-backed
//! [`Document`], handles keyboard/mouse/IME/clipboard input itself, and keeps
//! its own scroll position. egui is only used for painting and raw input.

use egui::text::LayoutJob;
use egui::{Color32, Event, EventFilter, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, TextFormat, Vec2};

use crate::buffer::{Buffer, Change, Selection, char_width, tab_width, visual_width};
use crate::document::Document;
use crate::theme;

const SCROLLBAR: f32 = 12.0;
/// Width of the gutter strip (left of the text) where change markers sit.
const CHANGE_ZONE: f32 = 12.0;
/// Width of the strip at the gutter's left edge for test run buttons.
const RUN_ZONE: f32 = 14.0;
/// Width of the strip (left of the change markers) for fold chevrons.
const FOLD_ZONE: f32 = 14.0;
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
    /// Draw the cursor as a block (Vim/Helix normal mode).
    pub block_cursor: bool,
    pub smooth_scrolling: bool,
    /// Which pane this is, when the editor is split.
    pub pane: u8,
    /// Whether this is the focused pane; the other one leaves focus requests alone.
    pub active: bool,
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

/// By line: (char column, text), sorted by column.
pub type Inlays = std::sync::Arc<std::collections::HashMap<usize, Vec<(usize, String)>>>;

/// Coloring on part of a line (columns in characters).
#[derive(Clone, Debug, PartialEq)]
pub struct Highlight {
    pub start: usize,
    pub end: usize,
    pub foreground: Option<Color32>,
    pub background: Option<Color32>,
    pub italic: bool,
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
    /// Coloring by line, in layers: later ones paint over earlier ones.
    pub highlights: Vec<std::sync::Arc<std::collections::HashMap<usize, Vec<Highlight>>>>,
    /// Text shown between characters, like inferred types.
    pub inlays: Option<Inlays>,
    /// Columns by line of characters to draw as `?`, since they may not show.
    pub hidden: Option<std::sync::Arc<std::collections::HashMap<usize, Vec<usize>>>>,
    /// Sorted by line.
    pub changes: Vec<ChangeMark>,
    /// Blame text shown after the end of this line.
    pub blame: Option<(usize, String)>,
    /// Foldable regions (from the syntax tree or indentation).
    pub folds: Vec<rust_code_core::structure::FoldRange>,
    /// Breakpoints: (line, color, has a condition or log message).
    pub breakpoints: Vec<(usize, Color32, bool)>,
    /// The line the debugger is paused on.
    pub debug_line: Option<usize>,
    /// Bookmarked lines, sorted.
    pub bookmarks: Vec<usize>,
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
    /// The breakpoint margin was clicked on this line.
    pub breakpoint_click: Option<usize>,
    /// What Vim/Helix mode asked for this frame (`:w`, `:q`, clipboard).
    pub vim_effects: Vec<rust_code_core::vim::Effect>,
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

pub fn editor_id(uid: u64, pane: u8) -> egui::Id {
    egui::Id::unique(("editor", uid, pane))
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
    /// How far the painted view still is from `scroll.y`, while gliding.
    scroll_offset: f32,
    /// Fixed corner (line, visual column) of the column selection being dragged.
    column_anchor: Option<(usize, usize)>,
    /// Char clicked with Ctrl this frame.
    ctrl_click: Option<usize>,
    /// Gutter change marker clicked this frame.
    change_click: Option<(usize, Pos2)>,
    /// Gutter run button clicked this frame.
    run_click: Option<usize>,
    /// Breakpoint margin clicked this frame.
    breakpoint_click: Option<usize>,
    /// First lines of the folded regions.
    folded: std::collections::BTreeSet<usize>,
}

impl ViewState {
    pub fn folded(&self) -> &std::collections::BTreeSet<usize> {
        &self.folded
    }

    pub fn set_folded(&mut self, start: usize, folded: bool) {
        if folded {
            self.folded.insert(start);
        } else {
            self.folded.remove(&start);
        }
    }

    pub fn clear_folds(&mut self) {
        self.folded.clear();
    }
}

/// Hidden line ranges for the folded regions, merged and sorted.
fn hidden_lines(
    folded: &std::collections::BTreeSet<usize>,
    folds: &[rust_code_core::structure::FoldRange],
) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for f in folds.iter().filter(|f| folded.contains(&f.start) && f.end > f.start) {
        let (a, b) = (f.start + 1, f.end);
        match out.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Word-wrap layout: `first_row[line]` is the first visual row of `line`.
struct RowMap {
    version: u64,
    /// Wrap width (`usize::MAX` when not wrapping).
    cols: usize,
    /// Hidden (folded) line ranges the map was built for.
    hidden: Vec<(usize, usize)>,
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
    /// Hidden line ranges (inclusive), sorted and disjoint.
    hidden: Vec<(usize, usize)>,
}

/// The hidden range containing `line`, if any.
fn hidden_range(hidden: &[(usize, usize)], line: usize) -> Option<(usize, usize)> {
    let i = hidden.partition_point(|r| r.1 < line);
    hidden.get(i).copied().filter(|r| r.0 <= line)
}

impl Layout {
    fn new(doc: &Document, view: &mut ViewState, wrap_cols: Option<usize>, hidden: &[(usize, usize)]) -> Self {
        let buf = &doc.buffer;
        if wrap_cols.is_none() && hidden.is_empty() {
            view.rows = None;
            return Self { wrap_cols: None, total_rows: buf.len_lines(), hidden: Vec::new() };
        }
        let cols = wrap_cols.unwrap_or(usize::MAX);
        let stale =
            view.rows.as_ref().is_none_or(|r| r.version != buf.version() || r.cols != cols || r.hidden != hidden);
        if stale {
            let mut first_row = Vec::with_capacity(buf.len_lines() + 1);
            let mut row = 0;
            for line in 0..buf.len_lines() {
                first_row.push(row);
                if hidden_range(hidden, line).is_some() {
                    continue;
                }
                row += match wrap_cols {
                    Some(cols) => wrap_segments(&buf.line_text(line), cols).len(),
                    None => 1,
                };
            }
            first_row.push(row);
            view.rows = Some(RowMap { version: buf.version(), cols, hidden: hidden.to_vec(), first_row });
        }
        let total_rows = *view.rows.as_ref().unwrap().first_row.last().unwrap();
        Self { wrap_cols, total_rows, hidden: hidden.to_vec() }
    }

    fn row(&self, doc: &Document, view: &ViewState, row: usize) -> Row {
        let buf = &doc.buffer;
        let Some(map) = view.rows.as_ref() else {
            let line = row.min(buf.len_lines() - 1);
            return Row { line, start: buf.line_start(line), end: buf.line_end(line), first: true };
        };
        let line = map.first_row.partition_point(|&r| r <= row).saturating_sub(1).min(buf.len_lines() - 1);
        let (ls, le) = (buf.line_start(line), buf.line_end(line));
        match self.wrap_cols {
            Some(cols) => {
                let seg = row - map.first_row[line];
                let starts = wrap_segments(&buf.line_text(line), cols);
                let seg = seg.min(starts.len() - 1);
                let start = ls + starts[seg];
                let end = starts.get(seg + 1).map_or(le, |s| ls + s);
                Row { line, start, end, first: seg == 0 }
            }
            None => Row { line, start: ls, end: le, first: true },
        }
    }

    /// Visual row containing `pos` (a position inside a fold maps to its first line).
    fn row_of(&self, doc: &Document, view: &ViewState, pos: usize) -> usize {
        let buf = &doc.buffer;
        let mut line = buf.char_to_line(pos);
        let mut col = pos - buf.line_start(line);
        if let Some((first, _)) = hidden_range(&self.hidden, line) {
            line = first.saturating_sub(1);
            col = 0;
        }
        let Some(map) = view.rows.as_ref() else { return line };
        let seg = match self.wrap_cols {
            Some(cols) => wrap_segments(&buf.line_text(line), cols).partition_point(|&s| s <= col).saturating_sub(1),
            None => 0,
        };
        map.first_row[line] + seg
    }
}

struct Metrics {
    font_id: FontId,
    row_h: f32,
    char_w: f32,
    gutter_w: f32,
    inlays: Option<Inlays>,
    hidden: Option<std::sync::Arc<std::collections::HashMap<usize, Vec<usize>>>>,
}

/// Draws and runs the editor for `doc` in all remaining space of `ui`.
pub fn show(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    opts: Options,
    deco: &Decorations,
    vim: Option<&mut rust_code_core::vim::Vim>,
) -> Output {
    let mut vim_effects = Vec::new();
    let (font_size, word_wrap) = (opts.font_size, opts.word_wrap);
    let id = editor_id(doc.uid, opts.pane);
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());

    let font_id = FontId::monospace(font_size);
    let row_h = ui.fonts_mut(|f| f.row_height(&font_id));
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font_id, '0'));
    let digits = doc.buffer.len_lines().to_string().len().max(3);
    let m = Metrics {
        gutter_w: digits as f32 * char_w + 28.0 + RUN_ZONE + FOLD_ZONE,
        font_id,
        row_h,
        char_w,
        inlays: deco.inlays.clone(),
        hidden: deco.hidden.clone(),
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + m.gutter_w, rect.top()),
        Pos2::new(rect.right() - SCROLLBAR, rect.bottom()),
    );
    let wrap_cols = word_wrap.then(|| ((text_rect.width() - char_w) / char_w).floor().max(20.0) as usize);

    if let Some(line) = doc.buffer.take_edited_from_line() {
        doc.highlighter.invalidate_from(line);
    }

    // ----- focus ---------------------------------------------------------------
    if doc.request_focus && opts.active {
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
    // Forget folds whose region no longer exists (once the structure is known).
    let mut folds = deco.folds.clone();
    if !folds.is_empty() {
        view.folded.retain(|s| folds.iter().any(|f| f.start == *s));
    }
    let mut layout = Layout::new(doc, view, wrap_cols, &hidden_lines(&view.folded, &folds));
    let visible_rows = (text_rect.height() / row_h).floor().max(1.0) as usize;
    let mut reveal = std::mem::take(&mut doc.reveal);
    let line_before = doc.buffer.char_to_line(doc.buffer.primary().head);
    let lines_before = doc.buffer.len_lines();

    handle_scrolling(ui, view, &layout, &m, rect, text_rect, &response);
    let scroll_before = view.scroll.y;
    reveal |= handle_mouse(ui, doc, view, &layout, &m, text_rect, &response, deco);
    if has_focus {
        let version = doc.buffer.version();
        reveal |= match vim {
            Some(vim) => vim_keyboard(ui, doc, view, &m, visible_rows, vim, &mut vim_effects),
            None => handle_keyboard(ui, doc, view, &m, visible_rows),
        };
        if doc.buffer.version() != version {
            if let Some(line) = doc.buffer.take_edited_from_line() {
                doc.highlighter.invalidate_from(line);
            }
            // Lines were added or removed: move folds below the edit along.
            let delta = doc.buffer.len_lines() as isize - lines_before as isize;
            if delta != 0 {
                let shift = |l: usize| if l > line_before { (l as isize + delta).max(0) as usize } else { l };
                view.folded = view.folded.iter().map(|&l| shift(l)).collect();
                for f in &mut folds {
                    (f.start, f.end) = (shift(f.start), shift(f.end));
                }
            }
        }
    }
    // A folded region the cursor moved into: step over it when moving up or
    // down, otherwise (go to line, search) open it.
    let hidden = hidden_lines(&view.folded, &folds);
    let line = doc.buffer.char_to_line(doc.buffer.primary().head);
    if let Some((first, last)) = hidden_range(&hidden, line) {
        let vertical =
            ui.input(|i| [Key::ArrowUp, Key::ArrowDown, Key::PageUp, Key::PageDown].iter().any(|k| i.key_pressed(*k)));
        if vertical && hidden_range(&hidden, line_before).is_none() {
            let target = if line > line_before { (last + 1).min(doc.buffer.len_lines() - 1) } else { first - 1 };
            let vcol = doc.buffer.visual_col(doc.buffer.primary().head);
            let pos = doc.buffer.pos_at_visual_col(target, vcol);
            doc.buffer.set_single(Selection::cursor(pos));
        } else {
            view.folded.retain(|s| !(first..=last).contains(&(s + 1)) && *s + 1 != first);
        }
    }
    layout = Layout::new(doc, view, wrap_cols, &hidden_lines(&view.folded, &folds));

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
    glide(ui, view, opts.smooth_scrolling, scroll_before.clamp(0.0, max_y) - view.scroll.y, text_rect.height());

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
        breakpoint_click: view.breakpoint_click.take(),
        vim_effects,
        cursor_rect,
    }
}

/// X offset (in points, relative to the text area) of `pos` on the row starting at `row_start`.
fn col_x(buf: &Buffer, row_start: usize, pos: usize, m: &Metrics) -> f32 {
    let line = buf.char_to_line(row_start);
    let line_start = buf.line_start(line);
    let row_col = visual_width(buf.rope().slice(line_start..row_start).chars());
    let pos = pos.max(row_start);
    let pos_col = visual_width(buf.rope().slice(line_start..pos).chars());
    (pos_col - row_col + inlay_cols(m, line, row_start - line_start, pos - line_start)) as f32 * m.char_w
}

/// Columns taken by inlay hints in a row, up to and including char column `to`.
fn inlay_cols(m: &Metrics, line: usize, from: usize, to: usize) -> usize {
    let Some(hints) = m.inlays.as_ref().and_then(|i| i.get(&line)) else { return 0 };
    hints.iter().filter(|(col, _)| (from..=to).contains(col)).map(|(_, t)| t.chars().count()).sum()
}

/// Document position under a screen point.
fn pos_at(doc: &Document, view: &ViewState, layout: &Layout, m: &Metrics, text_rect: Rect, p: Pos2) -> usize {
    let (row_start, row_end, line, col) = cell_at(doc, view, layout, m, text_rect, p);
    doc.buffer.pos_at_visual_col(line, col).clamp(row_start, row_end)
}

/// The start and end of the row under `p`, its line, and the visual column
/// under `p`, which may lie past the end of the line.
fn cell_at(
    doc: &Document,
    view: &ViewState,
    layout: &Layout,
    m: &Metrics,
    text_rect: Rect,
    p: Pos2,
) -> (usize, usize, usize, usize) {
    let y = p.y - text_rect.top() + view.scroll.y;
    let row_idx = ((y / m.row_h).floor().max(0.0) as usize).min(layout.total_rows.saturating_sub(1));
    let row = layout.row(doc, view, row_idx);
    let buf = &doc.buffer;
    let line_start = buf.line_start(row.line);
    let row_col = visual_width(buf.rope().slice(line_start..row.start).chars());
    let x = p.x - text_rect.left() + view.scroll.x;
    let mut col = (x / m.char_w).round().max(0.0) as usize;
    // Screen column to text column: hints take room but aren't text.
    if let Some(hints) = m.inlays.as_ref().and_then(|i| i.get(&row.line)) {
        let in_row = (row.start - line_start)..=(row.end - line_start);
        let mut taken = 0;
        for (hint, text) in hints.iter().filter(|(c, _)| in_row.contains(c)) {
            let at = buf.visual_col(line_start + *hint) - row_col + taken;
            let len = text.chars().count();
            if col <= at {
                break;
            }
            if col < at + len {
                // Inside the hint: the place it sits.
                col = at;
                break;
            }
            taken += len;
        }
        col = col.saturating_sub(taken);
    }
    (row.start, row.end, row.line, row_col + col)
}

/// Eases the painted view toward `scroll.y` after a jump of `jump` from keys
/// or reveal. Wheel and scrollbar moves are outside `jump`. Long jumps cut.
fn glide(ui: &egui::Ui, view: &mut ViewState, smooth: bool, jump: f32, view_h: f32) {
    let offset = view.scroll_offset + jump;
    if !smooth || offset.abs() > view_h * 3.0 {
        view.scroll_offset = 0.0;
        return;
    }
    let dt = ui.input(|i| i.stable_dt);
    view.scroll_offset = offset * (-dt / 0.04).exp();
    if view.scroll_offset.abs() < 0.5 {
        view.scroll_offset = 0.0;
    } else {
        ui.ctx().request_repaint();
    }
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
    let (_, _, line, col) = cell_at(doc, view, layout, m, text_rect, p);
    let in_text = text_rect.x_range().contains(p.x);

    if !ui.input(|i| i.pointer.primary_down() || i.pointer.middle_down()) {
        view.column_anchor = None;
    }
    // The middle button drags a column selection from where it goes down.
    if in_text
        && response.is_pointer_button_down_on()
        && ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Middle))
    {
        view.column_anchor = Some((line, col));
        doc.buffer.select_column((line, col), (line, col));
        return false;
    }

    if pressed && p.x >= text_rect.left() - m.gutter_w && p.x < text_rect.right() {
        view.column_anchor = None;
        let cell = (line, col);
        let line = doc.buffer.char_to_line(pos);
        let gutter_left = text_rect.left() - m.gutter_w;
        // The left margin runs a test on test lines, and toggles a breakpoint elsewhere.
        if p.x < gutter_left + RUN_ZONE {
            if deco.runnables.iter().any(|r| r.0 == line) && !deco.breakpoints.iter().any(|b| b.0 == line) {
                view.run_click = Some(line);
            } else {
                view.breakpoint_click = Some(line);
            }
            return false;
        }
        let fold_x = text_rect.left() - CHANGE_ZONE - FOLD_ZONE;
        if (fold_x..fold_x + FOLD_ZONE).contains(&p.x) && deco.folds.iter().any(|f| f.start == line) {
            let folded = view.folded.contains(&line);
            view.set_folded(line, !folded);
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
        } else if mods.alt && mods.shift {
            // From the cursor to the pointer, like Shift+click but as a block.
            let head = doc.buffer.primary().head;
            let anchor = (doc.buffer.char_to_line(head), doc.buffer.visual_col(head));
            view.column_anchor = Some(anchor);
            doc.buffer.select_column(anchor, cell);
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

    if let Some(anchor) = view.column_anchor {
        if response.dragged() && !pressed {
            doc.buffer.select_column(anchor, (line, col));
            if p.y < text_rect.top() || p.y > text_rect.bottom() {
                ui.ctx().request_repaint();
                return true;
            }
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
/// The Vim/Helix key for an egui key press, if it means anything to them.
fn vim_key(key: Key, mods: Modifiers) -> Option<rust_code_core::vim::Key> {
    use rust_code_core::vim::Key as V;
    if mods.command || mods.ctrl {
        let name = key.name();
        let letter = name.chars().next().filter(|c| name.len() == 1 && c.is_ascii_alphabetic())?;
        return Some(V::Ctrl(letter.to_ascii_lowercase()));
    }
    Some(match key {
        Key::Escape => V::Esc,
        Key::Enter => V::Enter,
        Key::Backspace => V::Backspace,
        Key::Tab => V::Tab,
        Key::ArrowLeft => V::Left,
        Key::ArrowRight => V::Right,
        Key::ArrowUp => V::Up,
        Key::ArrowDown => V::Down,
        Key::Delete => V::Char('x'),
        _ => return None,
    })
}

/// Keyboard input in Vim/Helix mode: commands go to the engine; in insert
/// mode the editor types as usual and the engine just watches (for `.`).
fn vim_keyboard(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    m: &Metrics,
    visible_rows: usize,
    vim: &mut rust_code_core::vim::Vim,
    effects: &mut Vec<rust_code_core::vim::Effect>,
) -> bool {
    use rust_code_core::vim::{Key as V, Mode};
    let events = ui.input(|i| i.events.clone());
    let mut moved = false;
    for event in events {
        if vim.mode() == Mode::Insert {
            match &event {
                Event::Key { key: Key::Escape, pressed: true, .. } => {
                    effects.extend(vim.handle(&mut doc.buffer, V::Esc));
                    moved = true;
                    continue;
                }
                Event::Text(t) => t.chars().for_each(|c| vim.observe(V::Char(c))),
                Event::Paste(t) => t.chars().for_each(|c| vim.observe(if c == '\n' { V::Enter } else { V::Char(c) })),
                Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if modifiers.is_none() => {
                    vim.observe(V::Enter)
                }
                Event::Key { key: Key::Backspace, pressed: true, .. } => vim.observe(V::Backspace),
                Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if modifiers.is_none() => {
                    vim.observe(V::Tab)
                }
                _ => {}
            }
            // Type normally, as without Vim.
            moved |= handle_events(ui, doc, view, m, visible_rows, vec![event]);
            continue;
        }
        match event {
            Event::Text(text) => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    effects.extend(vim.handle(&mut doc.buffer, V::Char(c)));
                }
                moved = true;
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                // Printable keys arrive as text; only special keys here.
                if let Some(k) = vim_key(key, modifiers) {
                    effects.extend(vim.handle(&mut doc.buffer, k));
                    moved = true;
                }
            }
            Event::Copy => copy(ui, doc, view, false),
            _ => {}
        }
    }
    moved
}

fn handle_keyboard(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    m: &Metrics,
    visible_rows: usize,
) -> bool {
    let events = ui.input(|i| i.events.clone());
    handle_events(ui, doc, view, m, visible_rows, events)
}

fn handle_events(
    ui: &mut egui::Ui,
    doc: &mut Document,
    view: &mut ViewState,
    m: &Metrics,
    visible_rows: usize,
    events: Vec<Event>,
) -> bool {
    let mut moved = false;
    for event in events {
        match event {
            Event::Text(text) => {
                if !text.is_empty() && !text.chars().any(char::is_control) {
                    doc.buffer.type_text(&text);
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

    let scroll = Vec2::new(view.scroll.x, (view.scroll.y + view.scroll_offset).max(0.0));
    let first_row = (scroll.y / m.row_h).floor() as usize;
    let y_offset = first_row as f32 * m.row_h - scroll.y;
    let rows_on_screen = (rect.height() / m.row_h).ceil() as usize + 1;
    let last_row = (first_row + rows_on_screen).min(layout.total_rows);

    let primary = doc.buffer.primary();
    let primary_line = doc.buffer.char_to_line(primary.head);
    let primary_row = layout.row_of(doc, view, primary.head);
    let bracket = primary.is_empty().then(|| doc.buffer.matching_bracket(primary.head, BRACKET_SEARCH_LIMIT)).flatten();
    let x0 = text_rect.left() - scroll.x;
    let gutter_hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| p.x < text_rect.left() && p.x >= rect.left() && rect.contains(p));

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
        let guide = if buf.indent.use_tabs { tab_width() } else { buf.indent.size.max(1) };
        let levels = if opts.indent_guides { indent_cols(buf, row.line) / guide } else { 0 };
        for level in 0..levels {
            let x = (x0 + (level * guide) as f32 * m.char_w).round() + 0.5;
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

        // Plugin coloring: backgrounds under the text, the rest in the text itself.
        let line_highlights: Vec<Highlight> =
            deco.highlights.iter().filter_map(|h| h.get(&row.line)).flatten().cloned().collect();
        if !line_highlights.is_empty() {
            let highlights = &line_highlights;
            let line_start = buf.line_start(row.line);
            for h in highlights {
                let (a, b) = ((line_start + h.start).max(row.start), (line_start + h.end).min(row.end));
                if let (Some(color), true) = (h.background, b > a) {
                    let (xa, xb) = (x0 + col_x(buf, row.start, a, m), x0 + col_x(buf, row.start, b, m));
                    text_painter.rect_filled(
                        Rect::from_min_max(Pos2::new(xa, y), Pos2::new(xb, y + m.row_h)),
                        0.0,
                        color,
                    );
                }
            }
        }

        // Text.
        let job = row_job(doc, view, &row, m, Some(&line_highlights).filter(|h| !h.is_empty()).map(|h| h.as_slice()));
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

        // Fold chevron: always for folded regions, on gutter hover for open ones.
        if row.first
            && let Some(f) = deco.folds.iter().find(|f| f.start == row.line)
        {
            let folded = view.folded.contains(&f.start);
            if folded || gutter_hovered {
                let size = (m.row_h * 0.7).min(FOLD_ZONE);
                let center = Pos2::new(text_rect.left() - CHANGE_ZONE - FOLD_ZONE / 2.0, y + m.row_h / 2.0);
                let icon = if folded { crate::icons::Icon::ChevronRight } else { crate::icons::Icon::ChevronDown };
                crate::icons::paint(&painter, Rect::from_center_size(center, Vec2::splat(size)), icon, pal.line_number);
            }
            if folded && row.end == buf.line_end(row.line) {
                // "…" box after the line, like VS Code's folded-region marker.
                let x = x0 + col_x(buf, row.start, row.end, m) + m.char_w;
                let pill = Rect::from_min_size(Pos2::new(x, y + 2.0), Vec2::new(3.0 * m.char_w, m.row_h - 4.0));
                text_painter.rect_filled(pill, 3.0, pal.hover_bg);
                for k in 0..3 {
                    let dot = Pos2::new(pill.left() + (k as f32 + 0.5) * m.char_w, pill.center().y);
                    text_painter.circle_filled(dot, 1.3, pal.text_dim);
                }
            }
        }

        // Paused line: highlighted, with an arrow in the margin.
        if deco.debug_line == Some(row.line) {
            let band = Rect::from_min_max(Pos2::new(text_rect.left(), y), Pos2::new(text_rect.right(), y + m.row_h));
            text_painter.rect_filled(band, 0.0, Color32::from_rgba_unmultiplied(0xFF, 0xD7, 0x00, 28));
        }
        let margin = |size: f32| {
            Rect::from_center_size(
                Pos2::new(text_rect.left() - m.gutter_w + RUN_ZONE / 2.0 + 1.0, y + m.row_h / 2.0),
                Vec2::splat(size),
            )
        };
        if row.first && deco.bookmarks.binary_search(&row.line).is_ok() {
            let left = text_rect.left() - m.gutter_w + 1.0;
            let bar = Rect::from_min_max(Pos2::new(left, y + 1.0), Pos2::new(left + 4.0, y + m.row_h - 1.0));
            painter.rect_filled(bar, 1.0, pal.accent);
        }
        let breakpoint = deco.breakpoints.iter().find(|b| b.0 == row.line).filter(|_| row.first);
        if let Some((_, color, special)) = breakpoint {
            let r = margin(m.row_h * 0.55);
            painter.circle_filled(r.center(), r.width() / 2.0, *color);
            if *special {
                // Conditions and log points get a mark in the middle.
                painter.hline(r.x_range().shrink(r.width() * 0.25), r.center().y, Stroke::new(1.5, pal.editor_bg));
            }
        }
        if deco.debug_line == Some(row.line) && row.first {
            crate::icons::paint(
                &painter,
                margin(m.row_h * 0.8),
                crate::icons::Icon::Play,
                Color32::from_rgb(0xFF, 0xCC, 0x00),
            );
        }
        // Test run button (with the last result's icon and color).
        if row.first
            && breakpoint.is_none()
            && deco.debug_line != Some(row.line)
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
                Pos2::new(text_rect.left() - 18.0 - FOLD_ZONE, y),
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
            // A block cursor sits on the last selected char of a forward selection.
            let block_at = if opts.block_cursor && s.head > s.anchor { s.head - 1 } else { s.head };
            let row_idx = layout.row_of(doc, view, block_at);
            if row_idx < first_row || row_idx >= last_row {
                continue;
            }
            let row = layout.row(doc, view, row_idx);
            let y = rect.top() + y_offset + (row_idx - first_row) as f32 * m.row_h;
            let x = x0 + col_x(&doc.buffer, row.start, s.head, m);
            if opts.block_cursor {
                // A block on the char under the cursor (the last selected char
                // when a selection extends forward, as in Vim's visual mode).
                let x = x0 + col_x(&doc.buffer, row.start, block_at.max(row.start), m);
                let block = Rect::from_min_size(Pos2::new(x, y), Vec2::new(m.char_w, m.row_h));
                text_painter.rect_filled(block, 0.0, pal.cursor.gamma_multiply(0.55));
            } else {
                text_painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, m.row_h)),
                    0.0,
                    pal.cursor,
                );
            }
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
/// Syntax color runs with plugin colors on top.
fn overlay_runs(runs: &[(usize, Color32, bool)], highlights: &[Highlight], len: usize) -> Vec<(usize, Color32, bool)> {
    let mut styles = Vec::with_capacity(len);
    for &(end, color, italic) in runs {
        let end = end.min(len);
        if end > styles.len() {
            styles.resize(end, (color, italic));
        }
    }
    styles.resize(len, (theme::colors().editor_fg, false));
    for h in highlights {
        for style in styles.iter_mut().take(h.end.min(len)).skip(h.start) {
            style.0 = h.foreground.unwrap_or(style.0);
            style.1 |= h.italic;
        }
    }
    let mut out: Vec<(usize, Color32, bool)> = Vec::new();
    for (i, (color, italic)) in styles.into_iter().enumerate() {
        match out.last_mut() {
            Some(last) if last.1 == color && last.2 == italic => last.0 = i + 1,
            _ => out.push((i + 1, color, italic)),
        }
    }
    out
}

fn row_job(
    doc: &mut Document,
    view: &mut ViewState,
    row: &Row,
    m: &Metrics,
    highlights: Option<&[Highlight]>,
) -> LayoutJob {
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
    if let Some(highlights) = highlights.filter(|h| h.iter().any(|x| x.foreground.is_some() || x.italic)) {
        runs = overlay_runs(&runs, highlights, chars.len());
    }

    let mut job = LayoutJob::default();
    let hint_format =
        TextFormat { font_id: m.font_id.clone(), color: theme::colors().text_dim, italics: true, ..Default::default() };
    let hints: Vec<(usize, String)> = m
        .inlays
        .as_ref()
        .and_then(|i| i.get(&row.line))
        .map(|h| h.iter().filter(|(c, _)| (seg_a..=seg_b).contains(c)).cloned().collect())
        .unwrap_or_default();
    if !hints.is_empty() {
        // Runs also end at hints, so hints land between pieces.
        let mut split = Vec::with_capacity(runs.len() + hints.len());
        let mut from = 0;
        for &(end, color, italic) in &runs {
            for (col, _) in hints.iter().filter(|(c, _)| *c > from && *c < end) {
                split.push((*col, color, italic));
            }
            split.push((end, color, italic));
            from = end;
        }
        split.dedup_by_key(|r| r.0);
        runs = split;
    }
    let hidden = m.hidden.as_ref().and_then(|h| h.get(&row.line));
    let mut hint_at = 0;
    let mut idx = seg_a;
    for (end, color, italic) in runs {
        if end <= idx {
            continue;
        }
        let stop = end.min(seg_b);
        if stop <= idx {
            break;
        }
        while let Some((_, text)) = hints.get(hint_at).filter(|(c, _)| *c <= idx) {
            job.append(text, 0.0, hint_format.clone());
            hint_at += 1;
        }
        let mut piece = String::with_capacity(stop - idx);
        for (i, &c) in (idx..stop).zip(&chars[idx..stop]) {
            let w = char_width(c, col);
            if c == '\t' {
                piece.extend(std::iter::repeat_n(' ', w));
            } else if hidden.is_some_and(|h| h.contains(&i)) {
                piece.push('?');
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
    // Hints after the last character.
    for (_, text) in &hints[hint_at.min(hints.len())..] {
        job.append(text, 0.0, hint_format.clone());
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
