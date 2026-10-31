//! A virtual grid: any number of rows, of which only those in view are read or
//! drawn. It knows nothing about where its cells come from.

use eframe::egui::{self, Align2, Color32, CursorIcon, Key, Pos2, Rect, Sense, Stroke, Vec2};

use crate::theme;

/// What a grid shows.
pub trait GridSource {
    fn rows(&self) -> usize;
    fn columns(&self) -> usize;
    fn header(&self, column: usize) -> String;
    /// A row's cells, as many as it has.
    fn row(&self, row: usize) -> Vec<String>;
    /// What the row's number in the margin says.
    fn row_label(&self, row: usize) -> String;
    /// A cell's own text color, when it has something to say.
    fn cell_color(&self, _row: usize, _column: usize) -> Option<Color32> {
        None
    }
}

#[derive(Default)]
pub struct GridState {
    /// Column widths, in points.
    pub widths: Vec<f32>,
    /// Selected (row, column).
    pub selected: Option<(usize, usize)>,
    /// A row to bring into view next frame.
    pub reveal: Option<usize>,
    /// A place to scroll to, from the keyboard.
    scroll_to: Option<Vec2>,
    resizing: Option<usize>,
}

/// What happened this frame.
#[derive(Default)]
pub struct GridEvents {
    pub header_clicked: Option<usize>,
    pub row_opened: Option<usize>,
}

const MIN_WIDTH: f32 = 30.0;
const EDGE: f32 = 4.0;

/// One line of a cell.
fn one_line(text: &str) -> String {
    text.replace(['\n', '\r'], " ")
}

fn is_number(text: &str) -> bool {
    text.trim().parse::<f64>().is_ok()
}

fn paint_cell(painter: &egui::Painter, rect: Rect, text: &str, font: &egui::FontId, color: Color32, right: bool) {
    // Cut off on the left, a number would read as a smaller one.
    let fits = text.chars().count() as f32 * font.size * 0.62 + 8.0 <= rect.width();
    let right = right && fits;
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect.shrink2(Vec2::new(4.0, 0.0))));
    let (at, align) = if right {
        (Pos2::new(rect.right() - 4.0, rect.center().y), Align2::RIGHT_CENTER)
    } else {
        (Pos2::new(rect.left() + 4.0, rect.center().y), Align2::LEFT_CENTER)
    };
    painter.text(at, align, one_line(text), font.clone(), color);
}

/// Draws the grid in all the space there is.
pub fn show(
    ui: &mut egui::Ui,
    id: egui::Id,
    state: &mut GridState,
    source: &dyn GridSource,
    font_size: f32,
) -> GridEvents {
    let mut events = GridEvents::default();
    let pal = theme::colors();
    let font = egui::FontId::monospace(font_size);
    let row_h = (font_size * 1.5).round();
    let (rows, columns) = (source.rows(), source.columns());
    state.widths.resize(columns, 100.0);
    let digits = source.row_label(rows.saturating_sub(1)).chars().count().max(3);
    let margin = digits as f32 * font_size * 0.65 + 10.0;
    let width: f32 = margin + state.widths.iter().sum::<f32>();
    // Rows sit in doubles: a float loses whole pixels this far down long files.
    let height = (rows as f64 + 1.0) * f64::from(row_h);

    let focus_id = id.with("body");
    let focused = ui.memory(|m| m.has_focus(focus_id));
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                focus_id,
                egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() },
            )
        });
        keyboard(ui, state, source, row_h, margin, &mut events);
    }

    let mut area = egui::ScrollArea::both().id_salt(id).auto_shrink([false, false]);
    if let Some(row) = state.reveal.take() {
        let y = (row as f32 * row_h - ui.available_height() / 2.0).max(0.0);
        state.scroll_to = Some(Vec2::new(0.0, y));
    }
    if let Some(to) = state.scroll_to.take() {
        area = area.scroll_offset(to);
    }
    area.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(Vec2::new(width, height as f32));
        let screen = ui.clip_rect();
        let painter = ui.painter_at(screen);
        let widths = state.widths.clone();
        let x_of = |column: usize| screen.left() - viewport.left() + margin + widths[..column].iter().sum::<f32>();
        let top = f64::from(viewport.top());
        let first = (top / f64::from(row_h)) as usize;
        let last = (((top + f64::from(screen.height())) / f64::from(row_h)) as usize + 1).min(rows);
        let y_of = |row: usize| screen.top() + ((row as f64 + 1.0) * f64::from(row_h) - top) as f32;

        // What's clicked: the whole visible area takes clicks; the header's edges drag.
        let body = ui.interact(screen, id.with("body"), Sense::click());
        for row in first..last {
            let y = y_of(row);
            let row_rect = Rect::from_min_size(Pos2::new(screen.left(), y), Vec2::new(screen.width(), row_h));
            if row % 2 == 1 {
                painter.rect_filled(row_rect, 0.0, pal.hover_bg.gamma_multiply(0.35));
            }
            let cells = source.row(row);
            for column in 0..columns {
                let x = x_of(column);
                if x > screen.right() || x + state.widths[column] < screen.left() + margin {
                    continue;
                }
                let cell = Rect::from_min_size(Pos2::new(x, y), Vec2::new(state.widths[column], row_h));
                if state.selected == Some((row, column)) {
                    painter.rect_stroke(cell, 0.0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                }
                if let Some(text) = cells.get(column).filter(|t| !t.is_empty()) {
                    let inner = painter.with_clip_rect(screen.with_min_x(screen.left() + margin));
                    let color = source.cell_color(row, column).unwrap_or(pal.editor_fg);
                    paint_cell(&inner, cell, text, &font, color, is_number(text));
                }
            }
        }
        // The margin, over anything scrolled beneath it.
        let gutter = Rect::from_min_size(screen.min, Vec2::new(margin, screen.height()));
        painter.rect_filled(gutter, 0.0, pal.editor_bg);
        for row in first..last {
            let rect = Rect::from_min_size(Pos2::new(screen.left(), y_of(row)), Vec2::new(margin - 4.0, row_h));
            let text = source.row_label(row);
            painter.with_clip_rect(rect).text(
                rect.right_center(),
                Align2::RIGHT_CENTER,
                text,
                font.clone(),
                pal.line_number,
            );
        }
        // The header stays at the top.
        let bar = Rect::from_min_size(screen.min, Vec2::new(screen.width(), row_h));
        painter.rect_filled(bar, 0.0, pal.sidebar_bg);
        for column in 0..columns {
            let x = x_of(column);
            if x > screen.right() {
                break;
            }
            let cell = Rect::from_min_size(Pos2::new(x, screen.top()), Vec2::new(state.widths[column], row_h));
            if cell.right() < screen.left() + margin {
                continue;
            }
            let inner = painter.with_clip_rect(bar.with_min_x(screen.left() + margin));
            paint_cell(&inner, cell, &source.header(column), &font, pal.tab_active_fg, false);
            inner.vline(cell.right(), bar.y_range(), Stroke::new(1.0, pal.indent_guide));
            let edge = Rect::from_center_size(cell.right_center(), Vec2::new(EDGE * 2.0, row_h));
            let handle = ui.interact(edge, id.with(("edge", column)), Sense::drag());
            if handle.hovered() || handle.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            if handle.drag_started() {
                state.resizing = Some(column);
            }
            if handle.drag_stopped() {
                state.resizing = None;
            }
        }
        if let Some(column) = state.resizing {
            let dx = ui.input(|i| i.pointer.delta().x);
            state.widths[column] = (state.widths[column] + dx).max(MIN_WIDTH);
        }

        // Where a click landed.
        if let Some(p) = body.interact_pointer_pos().filter(|_| body.clicked() || body.double_clicked()) {
            ui.memory_mut(|m| m.request_focus(focus_id));
            let column = (0..columns).find(|c| (x_of(*c)..x_of(*c) + state.widths[*c]).contains(&p.x));
            if p.y < screen.top() + row_h {
                if p.x >= screen.left() + margin
                    && let Some(column) = column
                {
                    events.header_clicked = Some(column);
                }
            } else {
                let row = ((f64::from(p.y - screen.top()) + top) / f64::from(row_h)) as usize;
                let row = row.saturating_sub(1);
                if row < rows {
                    state.selected = Some((row, column.unwrap_or(0)));
                    if body.double_clicked() {
                        events.row_opened = Some(row);
                    }
                }
            }
        }
    });
    events
}

fn keyboard(
    ui: &egui::Ui,
    state: &mut GridState,
    source: &dyn GridSource,
    row_h: f32,
    margin: f32,
    events: &mut GridEvents,
) {
    let (rows, columns) = (source.rows(), source.columns());
    if rows == 0 || columns == 0 {
        return;
    }
    let (row, column) = state.selected.unwrap_or((0, 0));
    let page = (ui.available_height() / row_h) as usize;
    let (mut r, mut c) = (row, column);
    let copy = ui.input_mut(|i| {
        let pressed = |i: &mut egui::InputState, key| i.consume_key(egui::Modifiers::NONE, key);
        if pressed(i, Key::ArrowDown) {
            r = (r + 1).min(rows - 1);
        }
        if pressed(i, Key::ArrowUp) {
            r = r.saturating_sub(1);
        }
        if pressed(i, Key::ArrowRight) {
            c = (c + 1).min(columns - 1);
        }
        if pressed(i, Key::ArrowLeft) {
            c = c.saturating_sub(1);
        }
        if pressed(i, Key::PageDown) {
            r = (r + page).min(rows - 1);
        }
        if pressed(i, Key::PageUp) {
            r = r.saturating_sub(page);
        }
        if pressed(i, Key::Home) {
            c = 0;
        }
        if pressed(i, Key::End) {
            c = columns - 1;
        }
        if pressed(i, Key::Enter) {
            events.row_opened = Some(r);
        }
        i.events.iter().any(|e| matches!(e, egui::Event::Copy))
    });
    if (r, c) != (row, column) || state.selected.is_none() {
        state.selected = Some((r, c));
        // Enough to bring it into view; a page's worth of slack isn't known here.
        let x = margin + state.widths[..c].iter().sum::<f32>() - margin;
        let y = (r.saturating_sub(page / 2)) as f32 * row_h;
        state.scroll_to = Some(Vec2::new(x.max(0.0), y));
    }
    if copy && let Some((r, c)) = state.selected {
        let text = source.row(r).get(c).cloned().unwrap_or_default();
        ui.ctx().copy_text(text);
    }
}
