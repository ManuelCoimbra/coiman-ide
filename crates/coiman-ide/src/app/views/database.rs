//! The Database window: a SQLite file's tables and views with their columns, a box to write
//! SQL in, and each statement's rows in a grid. It runs through the `sqlite3` program (see
//! `coiman_core::data::database`), in the background so a slow query never stops the editor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::data::database::{self, Outcome, Sqlite, Table};
use egui::{Color32, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::grid_view::{self, GridSource, GridState};
use crate::ui::theme;

const HISTORY: usize = 30;

#[derive(Default)]
pub(crate) struct DatabaseUi {
    pub(crate) open: bool,
    db: Option<PathBuf>,
    tables: Vec<Table>,
    schema_error: Option<String>,
    schema_loading: Option<Receiver<Result<Vec<Table>, String>>>,
    sql: String,
    running: Option<Receiver<Result<Outcome, String>>>,
    outcome: Option<Result<Outcome, String>>,
    shown: usize,
    grid: GridState,
    expanded: HashSet<String>,
    history: Vec<String>,
    /// Re-read the tables after a run that changed them.
    reload_schema: bool,
}

struct Rows<'a>(&'a database::ResultSet);

impl GridSource for Rows<'_> {
    fn rows(&self) -> usize {
        self.0.rows.len()
    }
    fn columns(&self) -> usize {
        self.0.columns.len()
    }
    fn header(&self, column: usize) -> String {
        self.0.columns[column].clone()
    }
    fn row(&self, row: usize) -> Vec<String> {
        self.0.rows[row].iter().map(|c| c.clone().unwrap_or_else(|| "NULL".to_owned())).collect()
    }
    fn row_label(&self, row: usize) -> String {
        (row + 1).to_string()
    }
    fn cell_color(&self, row: usize, column: usize) -> Option<Color32> {
        self.0.rows.get(row)?.get(column)?.is_none().then(|| theme::colors().text_dim)
    }
}

impl CodeApp {
    /// Opens a SQLite file in the Database window.
    pub(crate) fn open_database(&mut self, path: &Path) {
        let ui = &mut self.database;
        ui.open = true;
        if ui.db.as_deref() != Some(path) {
            ui.db = Some(path.to_path_buf());
            ui.outcome = None;
            ui.expanded.clear();
            if ui.sql.trim().is_empty() {
                ui.sql = "SELECT name FROM sqlite_schema WHERE type = 'table';".to_owned();
            }
        }
        self.read_schema();
    }

    /// File > Open a database, through the file dialog.
    pub(crate) fn choose_database(&mut self) {
        let dialog = self
            .file_dialog("database")
            .add_filter("SQLite databases", &["db", "sqlite", "sqlite3", "db3", "s3db", "sl3"])
            .add_filter("All files", &["*"]);
        if let Some(path) = dialog.pick_file() {
            self.dialog_chose("database", &path);
            self.open_database(&path);
        }
    }

    fn read_schema(&mut self) {
        let Some(db) = self.database.db.clone() else { return };
        let Some(sqlite) = Sqlite::find() else {
            self.database.schema_error = Some(missing_sqlite());
            return;
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(sqlite.schema(&db));
        });
        self.database.schema_loading = Some(rx);
    }

    /// Runs SQL: the selection or whole `.sql` file in front when asked from the editor, else
    /// what's in the window's box.
    pub(crate) fn run_database_sql(&mut self, from_editor: bool) {
        if from_editor && let Some(doc) = self.docs.get(self.active) {
            let text = doc.buffer.selected_texts().join(
                "
",
            );
            let sql = if text.trim().is_empty() { doc.buffer.text() } else { text };
            if !sql.trim().is_empty() {
                self.database.sql = sql;
            }
        }
        if self.database.db.is_none() {
            self.database.open = true;
            self.status = Some("Open a database first (Database: Open SQLite File)".to_owned());
            return;
        }
        self.database.open = true;
        let (Some(db), sql) = (self.database.db.clone(), self.database.sql.clone()) else { return };
        if sql.trim().is_empty() {
            return;
        }
        let Some(sqlite) = Sqlite::find() else {
            self.database.outcome = Some(Err(missing_sqlite()));
            return;
        };
        let history = &mut self.database.history;
        history.retain(|h| h != &sql);
        history.insert(0, sql.clone());
        history.truncate(HISTORY);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(sqlite.run(&db, &sql));
        });
        self.database.running = Some(rx);
    }

    pub(crate) fn database_window(&mut self, ctx: &egui::Context) {
        let state = &mut self.database;
        if !state.open {
            return;
        }
        if let Some(r) = state.schema_loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.schema_loading = None;
            match r {
                Ok(tables) => {
                    state.tables = tables;
                    state.schema_error = None;
                }
                Err(e) => state.schema_error = Some(e),
            }
        }
        if let Some(r) = state.running.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.running = None;
            state.reload_schema = r.as_ref().is_ok_and(|o| o.changes > 0)
                || state.sql.to_lowercase().contains("create ")
                || state.sql.to_lowercase().contains("drop ")
                || state.sql.to_lowercase().contains("alter ");
            state.outcome = Some(r);
            state.shown = 0;
            state.grid = GridState::default();
        }
        if state.running.is_some() || state.schema_loading.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        if std::mem::take(&mut state.reload_schema) {
            self.read_schema();
        }
        let mut open = true;
        let mut run = false;
        let mut choose = false;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.84).min(1240.0), (room.y * 0.82).max(420.0));
        let title = match &self.database.db {
            Some(db) => format!("Database · {}", db.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
            None => "Database".to_owned(),
        };
        let font_size = self.font_size;
        egui::Window::new(RichText::new(title).size(16.0).strong())
            .id(egui::Id::unique("database_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                let state = &mut self.database;
                let area = ui.available_rect_before_wrap();
                let side =
                    egui::Rect::from_min_size(area.min, egui::vec2(250.0_f32.min(area.width() * 0.3), area.height()));
                let main = egui::Rect::from_min_max(egui::pos2(side.right() + 12.0, area.top()), area.max);
                ui.allocate_rect(area, egui::Sense::hover());
                let mut side_ui = ui.new_child(egui::UiBuilder::new().max_rect(side));
                let insert = tables_ui(&mut side_ui, state, &mut choose);
                if let Some(sql) = insert {
                    state.sql = sql;
                    run = true;
                }
                let mut main_ui = ui.new_child(egui::UiBuilder::new().max_rect(main));
                run |= sql_box(&mut main_ui, state, font_size);
                results(&mut main_ui, state, font_size);
            });
        self.database.open = open;
        if choose {
            self.choose_database();
        }
        if run {
            self.run_database_sql(false);
        }
    }
}

fn missing_sqlite() -> String {
    "The sqlite3 program isn't on this computer. Install it (winget install SQLite.SQLite, or \
     MSYS2's mingw-w64-ucrt-x86_64-sqlite3) and try again."
        .to_owned()
}

/// The tables beside the SQL. Returns SQL to run when a table is opened.
fn tables_ui(ui: &mut egui::Ui, state: &mut DatabaseUi, choose: &mut bool) -> Option<String> {
    let pal = theme::colors();
    let mut run = None;
    // Rows take the clicks, not their text.
    ui.style_mut().interaction.selectable_labels = false;
    ui.horizontal(|ui| {
        if ui.button("Open...").on_hover_text("Open another SQLite file").clicked() {
            *choose = true;
        }
        if state.schema_loading.is_some() {
            ui.spinner();
        }
    });
    if let Some(db) = &state.db {
        ui.label(RichText::new(db.display().to_string()).small().color(pal.text_dim))
            .on_hover_text(db.display().to_string());
    }
    if let Some(e) = &state.schema_error {
        ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("db_tables").auto_shrink([false, false]).show(ui, |ui| {
        if state.db.is_none() {
            ui.label(RichText::new("No database open.").color(pal.text_dim));
        }
        for table in &state.tables {
            let open = state.expanded.contains(&table.name);
            let resp = ui
                .horizontal(|ui| {
                    let arrow = if open { "▾" } else { "▸" };
                    ui.label(RichText::new(arrow).color(pal.text_dim));
                    let kind = if table.view { "view" } else { "table" };
                    ui.label(RichText::new(&table.name).strong());
                    ui.label(RichText::new(kind).small().color(pal.text_dim));
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_text("Click: columns · Double click: show its rows");
            if resp.double_clicked() {
                run = Some(format!("SELECT * FROM {} LIMIT 500;", database::quote_name(&table.name)));
            } else if resp.clicked() {
                if open {
                    state.expanded.remove(&table.name);
                } else {
                    state.expanded.insert(table.name.clone());
                }
            }
            if open {
                for c in &table.columns {
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        let key = if c.primary_key { "🔑 " } else { "" };
                        let fk = table.foreign_keys.iter().find(|f| f.column == c.name);
                        ui.label(RichText::new(format!("{key}{}", c.name)));
                        let mut note = c.ty.to_lowercase();
                        if c.not_null {
                            note.push_str(" not null");
                        }
                        if let Some(f) = fk {
                            note.push_str(&format!(" → {}.{}", f.table, f.to));
                        }
                        ui.label(RichText::new(note).small().color(pal.text_dim));
                    });
                }
                ui.add_space(4.0);
            }
        }
    });
    run
}

/// The SQL box and its buttons. Returns whether to run.
fn sql_box(ui: &mut egui::Ui, state: &mut DatabaseUi, font_size: f32) -> bool {
    let pal = theme::colors();
    let mut run = false;
    ui.horizontal(|ui| {
        let running = state.running.is_some();
        let label = if running { "Running..." } else { "Run  Ctrl+Enter" };
        if ui.add_enabled(!running && state.db.is_some(), egui::Button::new(label)).clicked() {
            run = true;
        }
        if running {
            ui.spinner();
        }
        egui::ComboBox::from_id_salt("db_history").selected_text("History").width(120.0).show_ui(ui, |ui| {
            if state.history.is_empty() {
                ui.label(RichText::new("Nothing run yet").color(pal.text_dim));
            }
            for h in state.history.clone() {
                let one_line: String = h.split_whitespace().collect::<Vec<_>>().join(" ");
                let short: String = one_line.chars().take(70).collect();
                if ui.selectable_label(false, short).on_hover_text(&h).clicked() {
                    state.sql = h;
                }
            }
        });
    });
    let edit = egui::TextEdit::multiline(&mut state.sql)
        .font(egui::FontId::monospace(font_size))
        .desired_rows(6)
        .desired_width(f32::INFINITY)
        .hint_text("SQL: one statement or several, each ending with ;");
    let height = (ui.available_height() * 0.32).clamp(90.0, 260.0);
    let resp = egui::ScrollArea::vertical().id_salt("db_sql").max_height(height).show(ui, |ui| ui.add(edit)).inner;
    if resp.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
        run = true;
    }
    ui.add_space(6.0);
    run
}

fn results(ui: &mut egui::Ui, state: &mut DatabaseUi, font_size: f32) {
    let pal = theme::colors();
    match &state.outcome {
        None => {
            ui.label(RichText::new("Results show here. Double click a table to see its rows.").color(pal.text_dim));
        }
        Some(Err(e)) => {
            ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
        }
        Some(Ok(outcome)) => {
            let mut shown = state.shown.min(outcome.sets.len().saturating_sub(1));
            ui.horizontal(|ui| {
                for (i, set) in outcome.sets.iter().enumerate() {
                    let label = format!("Result {} · {} rows", i + 1, set.rows.len() + set.more);
                    if ui.selectable_label(shown == i, label).clicked() {
                        shown = i;
                    }
                }
                let changed =
                    if outcome.changes > 0 { format!(" · {} rows changed", outcome.changes) } else { String::new() };
                ui.label(RichText::new(format!("{} ms{changed}", outcome.millis)).small().color(pal.text_dim));
            });
            if shown != state.shown {
                state.shown = shown;
                state.grid = GridState::default();
            }
            let Some(set) = outcome.sets.get(shown) else {
                ui.label(RichText::new("Done. No rows came back.").color(pal.text_dim));
                return;
            };
            if set.more > 0 {
                ui.label(
                    RichText::new(format!("Showing the first {} rows; {} more left out.", set.rows.len(), set.more))
                        .small()
                        .color(pal.text_dim),
                );
            }
            if set.columns.is_empty() {
                ui.label(RichText::new("No rows.").color(pal.text_dim));
                return;
            }
            let rows = Rows(set);
            grid_view::show(ui, egui::Id::unique("db_grid"), &mut state.grid, &rows, font_size);
        }
    }
}
