//! CSV and TSV files: colored columns in the text, and a table view that stays
//! quick at any length and follows the text as it changes.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use coiman_core::data::table::{self, Table};
use eframe::egui::{self, Color32, RichText};

use super::{CodeApp, repaint};
use crate::buffer::Selection;
use crate::editor_view::{Decorations, Highlight};
use crate::grid_view::{self, GridSource, GridState};
use crate::theme;

/// Larger files aren't colored: it would happen again on each keystroke.
const COLOR_LIMIT_BYTES: usize = 2 * 1024 * 1024;

const COLUMN_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xE0, 0xE0, 0xE0),
    Color32::from_rgb(0x5C, 0xB8, 0xF0),
    Color32::from_rgb(0xE5, 0xC0, 0x7B),
    Color32::from_rgb(0x98, 0xC3, 0x79),
    Color32::from_rgb(0xD1, 0x8C, 0xE0),
    Color32::from_rgb(0xF0, 0x8D, 0x70),
    Color32::from_rgb(0x56, 0xC8, 0xB6),
    Color32::from_rgb(0xE0, 0x7A, 0xA0),
];

type Lines = Arc<HashMap<usize, Vec<Highlight>>>;

/// What was asked of the rows on their way.
#[derive(Clone, PartialEq)]
struct Ask {
    first: usize,
    filter: String,
    sort: Option<(usize, bool)>,
}

#[derive(Default)]
struct View {
    version: u64,
    table: Option<Arc<Table>>,
    parsing: Option<(u64, Receiver<Table>)>,
    filter: String,
    sort: Option<(usize, bool)>,
    /// `None` until read from the table; then the choice stands.
    header: Option<bool>,
    order: Arc<Vec<u32>>,
    asked: Option<Ask>,
    ordering: Option<Receiver<Vec<u32>>>,
    grid: GridState,
    sized_for: usize,
    /// The record selected before the rows moved, to find it again after.
    keep: Option<(u32, usize)>,
    /// The cell being edited, and its text.
    cell: Option<(u32, usize)>,
    cell_text: String,
}

#[derive(Default)]
pub(super) struct Tables {
    /// Documents shown as tables.
    views: HashMap<u64, View>,
    showing: std::collections::HashSet<u64>,
    colors: HashMap<u64, (u64, Option<Lines>)>,
}

struct Source<'a> {
    table: &'a Table,
    order: &'a [u32],
    header: bool,
}

impl GridSource for Source<'_> {
    fn rows(&self) -> usize {
        self.order.len()
    }

    fn columns(&self) -> usize {
        self.table.columns
    }

    fn header(&self, column: usize) -> String {
        if self.header {
            self.table.cells(0).get(column).cloned().unwrap_or_default()
        } else {
            format!("Column {}", column + 1)
        }
    }

    fn row(&self, row: usize) -> Vec<String> {
        // Rows are one line high: a cell's line breaks show as what they are.
        let cells = self.order.get(row).map_or_else(Vec::new, |r| self.table.cells(*r as usize));
        cells.into_iter().map(|c| c.replace("\r\n", "\n").replace('\n', " \\n ")).collect()
    }

    fn row_label(&self, row: usize) -> String {
        // Numbered as the file has them, whatever the sort.
        self.order.get(row).map_or_else(String::new, |r| (*r + 1).to_string())
    }
}

impl CodeApp {
    pub(super) fn table_showing(&self, uid: u64) -> bool {
        self.tables.showing.contains(&uid)
    }

    pub(super) fn toggle_table_view(&mut self) {
        let Some(doc) = self.docs.get(self.active).filter(|d| table::is_table_file(&d.extension())) else {
            self.status = Some("Open a CSV or TSV file to see it as a table".to_owned());
            return;
        };
        let uid = doc.uid;
        if !self.tables.showing.remove(&uid) {
            self.tables.showing.insert(uid);
        }
    }

    /// The status bar's label.
    pub(super) fn table_label(&self) -> Option<String> {
        let doc = self.docs.get(self.active).filter(|d| table::is_table_file(&d.extension()))?;
        Some(if self.tables.showing.contains(&doc.uid) { "Show as Text" } else { "Show as Table" }.to_owned())
    }

    /// Colors each column in the text, so a row can be followed across.
    pub(super) fn table_decorate(&mut self, i: usize, deco: &mut Decorations) {
        if !self.settings.editor.csv_colors {
            return;
        }
        let Some(doc) = self.docs.get(i).filter(|d| table::is_table_file(&d.extension()) && !d.large) else { return };
        let (uid, version) = (doc.uid, doc.buffer.version());
        if self.tables.colors.get(&uid).is_none_or(|(v, _)| *v != version) {
            let text = doc.text();
            let lines = (text.len() <= COLOR_LIMIT_BYTES).then(|| {
                let delimiter = table::detect_delimiter(&text, &doc.extension());
                let colored: HashMap<usize, Vec<Highlight>> = table::colored_fields(&text, delimiter)
                    .into_iter()
                    .map(|(line, fields)| {
                        let marks = fields
                            .into_iter()
                            .map(|(start, end, n)| Highlight {
                                start,
                                end,
                                foreground: Some(COLUMN_COLORS[n % COLUMN_COLORS.len()]),
                                background: None,
                                italic: false,
                                strike: false,
                                frame: None,
                            })
                            .collect();
                        (line, marks)
                    })
                    .collect();
                Arc::new(colored)
            });
            self.tables.colors.insert(uid, (version, lines));
        }
        if let Some((_, Some(lines))) = self.tables.colors.get(&uid) {
            deco.highlights.push(lines.clone());
        }
    }

    /// Shows the active file as a table, if it's asked to. Says whether it did.
    pub(super) fn table_view(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(doc) = self.docs.get(self.active) else { return false };
        let uid = doc.uid;
        if !self.tables.showing.contains(&uid) {
            return false;
        }
        let version = doc.buffer.version();
        let view = self.tables.views.entry(uid).or_default();
        // The file changed, or this is the first look: read it, away from the UI.
        let current = view.table.is_some() && view.version == version;
        let on_the_way = view.parsing.as_ref().is_some_and(|(v, _)| *v == version);
        if !current && !on_the_way {
            let (text, ext) = (doc.text(), doc.extension());
            let (tx, rx) = mpsc::channel();
            let wake = repaint(ui.ctx());
            std::thread::spawn(move || {
                let delimiter = table::detect_delimiter(&text, &ext);
                let _ = tx.send(Table::parse(text, delimiter));
                wake();
            });
            view.parsing = Some((version, rx));
        }
        if let Some((v, rx)) = &view.parsing
            && let Ok(table) = rx.try_recv()
        {
            view.version = *v;
            view.table = Some(Arc::new(table));
            view.parsing = None;
            view.cell = None;
            view.asked = None;
        }
        let Some(table) = view.table.clone() else {
            ui.label(RichText::new("Reading…").color(theme::colors().text_dim));
            return true;
        };
        let header = *view.header.get_or_insert_with(|| table.first_row_is_header());

        // The rows to show, again away from the UI: sorting a long file takes a moment.
        let ask = Ask { first: usize::from(header), filter: view.filter.clone(), sort: view.sort };
        if let Some(rx) = &view.ordering
            && let Ok(order) = rx.try_recv()
        {
            view.order = Arc::new(order);
            view.ordering = None;
            // The row that was selected, wherever it went.
            if let Some((record, column)) = view.keep.take() {
                let row = view.order.iter().position(|r| *r == record);
                view.grid.selected = row.map(|row| (row, column));
                view.grid.reveal = row;
            }
        }
        if view.asked.as_ref() != Some(&ask) && view.ordering.is_none() {
            view.asked = Some(ask.clone());
            let (for_thread, wake) = (table.clone(), repaint(ui.ctx()));
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(for_thread.view(ask.first, &ask.filter, ask.sort));
                wake();
            });
            view.ordering = Some(rx);
        }
        let working = view.ordering.is_some() || view.parsing.is_some();

        let pal = theme::colors();
        let mut to_text = false;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.add(egui::TextEdit::singleline(&mut view.filter).hint_text("Filter rows").desired_width(200.0));
            let total = table.records() - usize::from(header);
            ui.label(format!(
                "{} of {total} rows, {} columns, {}",
                view.order.len(),
                table.columns,
                table::delimiter_name(table.delimiter)
            ));
            let mut first_is_header = header;
            if ui.checkbox(&mut first_is_header, "First row is a header").changed() {
                view.header = Some(first_is_header);
            }
            if working {
                ui.label(RichText::new("Working…").color(pal.text_dim));
            }
            to_text = ui.button("Show as Text").clicked();
        });
        let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
        if table.unclosed_quote {
            ui.label(
                RichText::new("  A quote is never closed, so the last row may have swallowed others.").color(warn),
            );
        }
        if table.uneven > 0 {
            let (rows, have) = if table.uneven == 1 { ("row", "has") } else { ("rows", "have") };
            let text = format!("  {} {rows} {have} a different number of cells than the first.", table.uneven);
            ui.label(RichText::new(text).color(warn));
        }
        // Sized by the text, once; dragging the header's edges changes it after that.
        if view.sized_for != table.columns {
            let char_w = self.font_size * 0.65;
            view.grid.widths = table.column_widths().into_iter().map(|w| w as f32 * char_w + 12.0).collect();
            view.sized_for = table.columns;
        }
        // The selected cell, to change.
        let chosen = view.grid.selected.and_then(|(row, column)| Some((*view.order.get(row)?, column)));
        let mut edited = None;
        if view.cell != chosen {
            view.cell = chosen;
            view.cell_text = chosen
                .and_then(|(record, column)| table.cells(record as usize).get(column).cloned())
                .unwrap_or_default();
        }
        if let Some((record, column)) = chosen {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if view.cell_text.contains('\n') {
                    ui.label(RichText::new("This cell has line breaks: edit it as text").color(pal.text_dim));
                    return;
                }
                ui.label(RichText::new("Cell").color(pal.text_dim));
                let edit = egui::TextEdit::singleline(&mut view.cell_text).desired_width(320.0);
                let response = ui.add(edit);
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && view.parsing.is_none()
                    && let Some(range) = table.cell_range(record as usize, column)
                {
                    edited = Some((range, table::quote_cell(&view.cell_text, table.delimiter), record, column));
                }
            });
        }
        let source = Source { table: &table, order: &view.order, header };
        let events = grid_view::show(ui, ui.scope_id().with(("table", uid)), &mut view.grid, &source, self.font_size);
        if let Some((_, _, record, column)) = &edited {
            view.keep = Some((*record, *column));
        }
        if let Some(column) = events.header_clicked {
            view.keep = chosen;
            // Up, down, then as the file has it.
            view.sort = match view.sort {
                Some((c, false)) if c == column => Some((c, true)),
                Some((c, true)) if c == column => None,
                _ => Some((column, false)),
            };
        }
        if let Some(((a, b), value, _, _)) = edited
            && let Some(doc) = self.docs.get_mut(self.active)
        {
            let rope = doc.buffer.rope();
            let (a, b) = (rope.byte_to_char(a), rope.byte_to_char(b));
            doc.buffer.apply_edits(vec![(a, b, value)]);
            let view = self.tables.views.entry(uid).or_default();
            view.cell = None;
            return true;
        }
        let view = self.tables.views.entry(uid).or_default();
        let jump = events.row_opened.and_then(|row| view.order.get(row)).and_then(|r| table.line_of(*r as usize));
        if to_text || jump.is_some() {
            self.tables.showing.remove(&uid);
        }
        if let (Some(line), Some(doc)) = (jump, self.docs.get_mut(self.active)) {
            let at = doc.buffer.line_start(line.min(doc.buffer.len_lines().saturating_sub(1)));
            doc.buffer.set_single(Selection::cursor(at));
            doc.reveal = true;
            doc.request_focus = true;
        }
        true
    }
}
