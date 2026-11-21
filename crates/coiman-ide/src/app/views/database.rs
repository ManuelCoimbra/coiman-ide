//! The Database window: a SQLite file's, or a PostgreSQL or MySQL server's, tables and views
//! with their columns, a box to write SQL in, and each statement's rows in a grid. It runs
//! through the `sqlite3`, `psql` and `mysql` programs (see `coiman_core::data::database` and
//! `server_db`), in the background so a slow query never stops the editor.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::data::database::{self, Outcome, Plan, Sqlite, Table};
use coiman_core::data::server_db::{self, Kind, Server};
use egui::{Color32, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::grid_view::{self, GridSource, GridState};
use crate::ui::theme;

const HISTORY: usize = 30;
const WARN: Color32 = Color32::from_rgb(0xE0, 0x9A, 0x3E);

/// What the window's lower half shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum View {
    #[default]
    Results,
    Plan,
    Diagram,
}

/// What the window works on: a SQLite file, or a server.
#[derive(Clone, Debug, PartialEq)]
enum Target {
    File(PathBuf),
    Server(Server),
}

impl Target {
    fn label(&self) -> String {
        match self {
            Self::File(path) => path.display().to_string(),
            Self::Server(server) => server.label(),
        }
    }

    fn short(&self) -> String {
        match self {
            Self::File(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Self::Server(server) if !server.name.trim().is_empty() => server.name.trim().to_owned(),
            Self::Server(server) => format!("{} on {}", server.database, server.host),
        }
    }

    fn engine(&self) -> &'static str {
        match self {
            Self::File(_) => "SQLite",
            Self::Server(s) => match s.kind {
                Kind::Postgres => "PostgreSQL",
                Kind::MySql => "MySQL",
                Kind::SqlServer => "SQL Server",
            },
        }
    }

    fn quote(&self, name: &str) -> String {
        match self {
            Self::File(_) => database::quote_name(name),
            Self::Server(s) => s.kind.quote_name(name),
        }
    }

    fn run(&self, sql: &str) -> Result<Outcome, String> {
        match self {
            Self::File(path) => Sqlite::find().ok_or_else(missing_sqlite)?.run(path, sql),
            Self::Server(server) => server.run(sql),
        }
    }

    fn schema(&self) -> Result<Vec<Table>, String> {
        match self {
            Self::File(path) => Sqlite::find().ok_or_else(missing_sqlite)?.schema(path),
            Self::Server(server) => server.schema(),
        }
    }

    fn explain(&self, sql: &str) -> Result<Vec<Plan>, String> {
        match self {
            Self::File(path) => Sqlite::find().ok_or_else(missing_sqlite)?.explain(path, sql),
            Self::Server(server) => server.explain(sql),
        }
    }
}

/// The Connect form: everything but the password is remembered.
#[derive(Default)]
struct ConnectForm {
    server: Server,
    port: String,
    password: String,
    error: Option<String>,
}

impl ConnectForm {
    fn for_kind(kind: Kind) -> Self {
        let server = Server {
            kind,
            host: "localhost".to_owned(),
            port: kind.default_port(),
            user: kind.default_user().to_owned(),
            database: if kind == Kind::Postgres { "postgres".to_owned() } else { String::new() },
            ..Server::default()
        };
        Self { port: server.port.to_string(), server, ..Self::default() }
    }
}

/// What the window asks the app to do after drawing.
#[derive(Default)]
struct Asks {
    run: bool,
    explain: bool,
    choose: bool,
    connect: bool,
    copy_mermaid: bool,
    preview_mermaid: bool,
}

#[derive(Default)]
pub(crate) struct DatabaseUi {
    view: View,
    explaining: Option<Receiver<Result<Vec<Plan>, String>>>,
    plans: Option<Result<Vec<Plan>, String>>,
    pub(crate) open: bool,
    target: Option<Target>,
    /// The Connect form, while it's open.
    connect: Option<ConnectForm>,
    /// Connections used before (no passwords), read when first wanted.
    saved: Option<Vec<Server>>,
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
        self.open_target(Target::File(path.to_path_buf()));
    }

    fn open_target(&mut self, target: Target) {
        let ui = &mut self.database;
        ui.open = true;
        ui.connect = None;
        if ui.target.as_ref() != Some(&target) {
            let first_sql = match &target {
                Target::File(_) => "SELECT name FROM sqlite_schema WHERE type = 'table';",
                Target::Server(s) if s.kind == Kind::Postgres => {
                    "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema();"
                }
                Target::Server(s) if s.kind == Kind::SqlServer => {
                    "SELECT TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_SCHEMA = SCHEMA_NAME();"
                }
                Target::Server(_) => "SHOW TABLES;",
            };
            let was_first = ui.sql.trim().is_empty()
                || ui.sql.starts_with("SELECT name FROM sqlite_schema")
                || ui.sql.starts_with("SELECT table_name FROM information_schema")
                || ui.sql.starts_with("SELECT TABLE_NAME FROM INFORMATION_SCHEMA")
                || ui.sql == "SHOW TABLES;";
            ui.target = Some(target);
            ui.tables.clear();
            ui.outcome = None;
            ui.plans = None;
            ui.expanded.clear();
            if was_first {
                ui.sql = first_sql.to_owned();
            }
        }
        self.read_schema();
    }

    /// Database: Connect to PostgreSQL or MySQL: the window, on its Connect form.
    pub(crate) fn show_database_connect(&mut self) {
        let ui = &mut self.database;
        ui.open = true;
        let saved = ui.saved.get_or_insert_with(server_db::load_saved);
        let form = match saved.first() {
            Some(last) => ConnectForm { port: last.port.to_string(), server: last.clone(), ..ConnectForm::default() },
            None => ConnectForm::for_kind(Kind::Postgres),
        };
        ui.connect = Some(form);
    }

    /// The form's Connect: remembers the server (not its password) and reads its tables.
    fn connect_database(&mut self) {
        let Some(form) = self.database.connect.as_mut() else { return };
        // SQL Server's LocalDB, named instances and pipes need no port.
        let no_port = form.server.kind == Kind::SqlServer && form.port.trim().is_empty();
        let Ok(port) = (if no_port { Ok(0) } else { form.port.trim().parse::<u16>() }) else {
            form.error = Some("The port is a number, like 5432".to_owned());
            return;
        };
        let mut server = form.server.clone();
        server.port = port;
        server.host = server.host.trim().to_owned();
        server.password = (!form.password.is_empty()).then(|| form.password.clone());
        let saved = self.database.saved.get_or_insert_with(server_db::load_saved);
        if let Err(e) = server_db::remember(saved, &server) {
            self.status = Some(format!("Couldn't remember the connection: {e}"));
        }
        self.open_target(Target::Server(server));
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
        let Some(target) = self.database.target.clone() else { return };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(target.schema());
        });
        self.database.schema_loading = Some(rx);
    }

    /// Runs SQL: the selection or whole `.sql` file in front when asked from the editor, else
    /// what's in the window's box.
    pub(crate) fn run_database_sql(&mut self, from_editor: bool) {
        self.database_sql(from_editor, false);
    }

    /// Opens the Database window on its diagram of the tables.
    pub(crate) fn show_database_diagram(&mut self) {
        self.database.open = true;
        self.database.view = View::Diagram;
        if self.database.target.is_none() {
            self.status = Some("Open a database first (Database: Open SQLite File, or Connect)".to_owned());
        }
    }

    /// Shows how SQLite would carry out the SQL (its query plan), without running it.
    pub(crate) fn explain_database_sql(&mut self, from_editor: bool) {
        self.database_sql(from_editor, true);
    }

    fn database_sql(&mut self, from_editor: bool, explain: bool) {
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
        if self.database.target.is_none() {
            self.database.open = true;
            self.status = Some("Open a database first (Database: Open SQLite File, or Connect)".to_owned());
            return;
        }
        self.database.open = true;
        let (Some(target), sql) = (self.database.target.clone(), self.database.sql.clone()) else { return };
        if sql.trim().is_empty() {
            return;
        }
        let history = &mut self.database.history;
        history.retain(|h| h != &sql);
        history.insert(0, sql.clone());
        history.truncate(HISTORY);
        if explain {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(target.explain(&sql));
            });
            self.database.explaining = Some(rx);
            self.database.view = View::Plan;
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(target.run(&sql));
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
            state.view = View::Results;
        }
        if let Some(r) = state.explaining.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.explaining = None;
            state.plans = Some(r);
        }
        if state.running.is_some() || state.schema_loading.is_some() || state.explaining.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        if std::mem::take(&mut state.reload_schema) {
            self.read_schema();
        }
        let mut open = true;
        let mut asks = Asks::default();
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.84).min(1240.0), (room.y * 0.82).max(420.0));
        let title = match &self.database.target {
            Some(t) => format!("Database · {}", t.short()),
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
                if state.connect.is_some() {
                    connect_ui(&mut side_ui, state, &mut asks);
                } else if let Some(table) = tables_ui(&mut side_ui, state, &mut asks) {
                    state.sql = rows_of(state.target.as_ref(), &table);
                    asks.run = true;
                }
                let mut main_ui = ui.new_child(egui::UiBuilder::new().max_rect(main));
                sql_box(&mut main_ui, state, font_size, &mut asks);
                view_tabs(&mut main_ui, state);
                match state.view {
                    View::Results => results(&mut main_ui, state, font_size),
                    View::Plan => plan_view(&mut main_ui, state, font_size),
                    View::Diagram => {
                        if let Some(table) = diagram_view(&mut main_ui, state, &mut asks) {
                            state.sql = rows_of(state.target.as_ref(), &table);
                            asks.run = true;
                        }
                    }
                }
            });
        self.database.open = open;
        if asks.choose {
            self.choose_database();
        }
        if asks.connect {
            self.connect_database();
        }
        if asks.run {
            self.run_database_sql(false);
        } else if asks.explain {
            self.explain_database_sql(false);
        }
        if asks.copy_mermaid {
            ctx.copy_text(database::er_mermaid(&self.database.tables));
            self.status = Some("Copied the tables as a Mermaid diagram".to_owned());
        }
        if asks.preview_mermaid {
            let text = database::er_mermaid(&self.database.tables);
            self.preview_mermaid(&text);
            self.status = Some("Previewing the tables as a Mermaid diagram".to_owned());
        }
    }
}

/// SQL showing a table's rows.
fn rows_of(target: Option<&Target>, table: &str) -> String {
    let name = target.map_or_else(|| database::quote_name(table), |t| t.quote(table));
    match target {
        // SQL Server has TOP, not LIMIT.
        Some(Target::Server(s)) if s.kind == Kind::SqlServer => format!("SELECT TOP 500 * FROM {name};"),
        _ => format!("SELECT * FROM {name} LIMIT 500;"),
    }
}

/// The Connect form, in place of the tables.
fn connect_ui(ui: &mut egui::Ui, state: &mut DatabaseUi, asks: &mut Asks) {
    let pal = theme::colors();
    let saved = state.saved.clone().unwrap_or_default();
    let Some(form) = state.connect.as_mut() else { return };
    ui.label(RichText::new("Connect to a server").strong());
    ui.add_space(4.0);
    let mut kind = form.server.kind;
    egui::ComboBox::from_id_salt("db_kind").selected_text(kind.label()).show_ui(ui, |ui| {
        for k in [Kind::Postgres, Kind::MySql, Kind::SqlServer] {
            ui.selectable_value(&mut kind, k, k.label());
        }
    });
    if kind != form.server.kind {
        *form = ConnectForm::for_kind(kind);
    }
    let mut enter = false;
    egui::Grid::new("db_connect").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
        let sql_server = form.server.kind == Kind::SqlServer;
        for (label, text, hint) in [
            (
                "Host",
                &mut form.server.host,
                if sql_server { "localhost, or (localdb)\\MSSQLLocalDB" } else { "localhost" },
            ),
            ("Port", &mut form.port, if sql_server { "empty for LocalDB or an instance" } else { "" }),
            ("User", &mut form.server.user, if sql_server { "empty: Windows sign-in" } else { "" }),
            ("Database", &mut form.server.database, if sql_server { "optional" } else { "optional for MySQL" }),
            ("Name", &mut form.server.name, "optional: Shop, staging"),
        ] {
            ui.label(label);
            let r = ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY));
            enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.end_row();
        }
        ui.label("Password");
        let r = ui.add(
            egui::TextEdit::singleline(&mut form.password)
                .password(true)
                .hint_text("kept only while open")
                .desired_width(f32::INFINITY),
        );
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.end_row();
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Connect").clicked() || enter {
            asks.connect = true;
        }
        if ui.button("Cancel").clicked() {
            state.connect = None;
        }
    });
    let Some(form) = state.connect.as_mut() else { return };
    if let Some(e) = &form.error {
        ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
    }
    ui.label(
        RichText::new(format!(
            "Runs through {}: the password goes to it for each run and is never saved.",
            match form.server.kind {
                Kind::Postgres => "psql",
                Kind::MySql => "the mysql or mariadb client",
                Kind::SqlServer => "sqlcmd",
            }
        ))
        .small()
        .color(pal.text_dim),
    );
    if !saved.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new("Recent").small().strong());
        for s in saved {
            if ui.selectable_label(false, s.title()).on_hover_text(s.label()).clicked() {
                form.port = s.port.to_string();
                form.server = s;
                form.password.clear();
                form.error = None;
            }
        }
    }
}

fn missing_sqlite() -> String {
    "The sqlite3 program isn't on this computer. Install it (winget install SQLite.SQLite, or \
     MSYS2's mingw-w64-ucrt-x86_64-sqlite3) and try again."
        .to_owned()
}

/// The tables beside the SQL. Returns the table whose rows to show, when one is opened.
fn tables_ui(ui: &mut egui::Ui, state: &mut DatabaseUi, asks: &mut Asks) -> Option<String> {
    let pal = theme::colors();
    let mut run = None;
    // Rows take the clicks, not their text.
    ui.style_mut().interaction.selectable_labels = false;
    ui.horizontal(|ui| {
        if ui.button("Open...").on_hover_text("Open another SQLite file").clicked() {
            asks.choose = true;
        }
        if ui.button("Connect...").on_hover_text("A PostgreSQL or MySQL (MariaDB) server").clicked() {
            state.connect = Some(match &state.target {
                Some(Target::Server(s)) => {
                    ConnectForm { port: s.port.to_string(), server: s.clone(), ..ConnectForm::default() }
                }
                _ => ConnectForm::for_kind(Kind::Postgres),
            });
            state.saved.get_or_insert_with(server_db::load_saved);
        }
        if state.schema_loading.is_some() {
            ui.spinner();
        }
    });
    if let Some(t) = &state.target {
        ui.label(RichText::new(t.label()).small().color(pal.text_dim)).on_hover_text(t.label());
    }
    if let Some(e) = &state.schema_error {
        ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical().id_salt("db_tables").auto_shrink([false, false]).show(ui, |ui| {
        if state.target.is_none() {
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
                run = Some(table.name.clone());
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
                for index in &table.indexes {
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        let mut note = String::from(if index.unique { "unique index" } else { "index" });
                        if index.partial {
                            note.push_str(", some rows");
                        }
                        if index.automatic {
                            note.push_str(", from a constraint");
                        }
                        ui.label(RichText::new(format!("({})", index.columns.join(", "))).color(pal.text_dim));
                        ui.label(RichText::new(note).small().color(pal.text_dim))
                            .on_hover_text(format!("Index {}", index.name));
                    });
                }
                ui.add_space(4.0);
            }
        }
    });
    run
}

/// The SQL box and its buttons.
fn sql_box(ui: &mut egui::Ui, state: &mut DatabaseUi, font_size: f32, asks: &mut Asks) {
    let pal = theme::colors();
    ui.horizontal(|ui| {
        let running = state.running.is_some();
        let label = if running { "Running..." } else { "Run  Ctrl+Enter" };
        if ui.add_enabled(!running && state.target.is_some(), egui::Button::new(label)).clicked() {
            asks.run = true;
        }
        let explaining = state.explaining.is_some();
        if ui
            .add_enabled(!explaining && state.target.is_some(), egui::Button::new("Explain  Ctrl+Shift+Enter"))
            .on_hover_text(
                "How the database would carry the SQL out: which indexes it uses, which tables it reads whole. \
                 Nothing runs.",
            )
            .clicked()
        {
            asks.explain = true;
        }
        if running || explaining {
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
    if resp.has_focus() {
        // Shift first: the plain shortcut would match it too.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Enter)) {
            asks.explain = true;
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            asks.run = true;
        }
    }
    ui.add_space(6.0);
}

/// Results, the query plan or the diagram.
fn view_tabs(ui: &mut egui::Ui, state: &mut DatabaseUi) {
    ui.horizontal(|ui| {
        for (view, label, hint) in [
            (View::Results, "Results", "What the SQL returned"),
            (View::Plan, "Query plan", "How the database would carry the SQL out (Explain)"),
            (View::Diagram, "Diagram", "The tables and how their foreign keys link them"),
        ] {
            if ui.selectable_label(state.view == view, label).on_hover_text(hint).clicked() {
                state.view = view;
            }
        }
    });
    ui.separator();
}

/// Each statement's plan, as a tree, with full table scans and temporary sorts marked.
fn plan_view(ui: &mut egui::Ui, state: &mut DatabaseUi, font_size: f32) {
    let pal = theme::colors();
    let plans = match &state.plans {
        None => {
            ui.label(
                RichText::new("Explain (Ctrl+Shift+Enter) shows how the database would carry out the SQL above.")
                    .color(pal.text_dim),
            );
            return;
        }
        Some(Err(e)) => {
            ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
            return;
        }
        Some(Ok(plans)) => plans,
    };
    let scans = plans.iter().flat_map(|p| &p.steps).filter(|s| s.full_scan).count();
    let sorts = plans.iter().flat_map(|p| &p.steps).filter(|s| s.temp_sort).count();
    if scans + sorts > 0 {
        let mut parts = Vec::new();
        if scans > 0 {
            parts.push(format!("{scans} full table {}", if scans == 1 { "scan" } else { "scans" }));
        }
        if sorts > 0 {
            parts.push(format!("{sorts} {}", if sorts == 1 { "temporary sort" } else { "temporary sorts" }));
        }
        ui.label(RichText::new(parts.join(" · ")).color(WARN)).on_hover_text(
            "A full scan reads every row; an index on the columns in WHERE or JOIN lets the database search instead. \
             A temporary sort could use an index in the ORDER BY or GROUP BY order. Small tables don't need either.",
        );
    } else if !plans.is_empty() {
        ui.label(RichText::new("No full table scans.").color(pal.text_dim));
    }
    let engine = state.target.as_ref().map_or("SQLite", Target::engine);
    let mono = egui::FontId::monospace(font_size);
    egui::ScrollArea::vertical().id_salt("db_plan").auto_shrink([false, false]).show(ui, |ui| {
        for plan in plans {
            ui.add_space(6.0);
            let one_line: String = plan.statement.split_whitespace().collect::<Vec<_>>().join(" ");
            let short: String = one_line.chars().take(120).collect();
            ui.label(RichText::new(short).font(mono.clone()).strong()).on_hover_text(&plan.statement);
            if plan.steps.is_empty() {
                ui.label(
                    RichText::new(format!("    No plan: {engine} doesn't search for this kind of statement."))
                        .color(pal.text_dim),
                );
            }
            for step in &plan.steps {
                ui.horizontal(|ui| {
                    ui.add_space(14.0 + step.depth as f32 * 22.0);
                    let color = if step.full_scan || step.temp_sort { WARN } else { pal.text };
                    ui.label(RichText::new(&step.detail).font(mono.clone()).color(color));
                    if step.full_scan {
                        ui.label(RichText::new("reads every row").small().color(pal.text_dim));
                    } else if step.temp_sort {
                        ui.label(RichText::new("sorts in a temporary table").small().color(pal.text_dim));
                    }
                });
            }
        }
    });
}

/// The tables as boxes, with a line from each foreign key to the table it points to. Returns
/// the table whose rows to show, when one is double-clicked.
fn diagram_view(ui: &mut egui::Ui, state: &mut DatabaseUi, asks: &mut Asks) -> Option<String> {
    let pal = theme::colors();
    let tables = &state.tables;
    let places = database::er_layout(tables);
    ui.horizontal(|ui| {
        let has = !places.is_empty();
        if ui.add_enabled(has, egui::Button::new("Copy as Mermaid")).clicked() {
            asks.copy_mermaid = true;
        }
        if ui.add_enabled(has, egui::Button::new("Preview as Mermaid")).clicked() {
            asks.preview_mermaid = true;
        }
        ui.label(RichText::new("Double click a table to see its rows").small().color(pal.text_dim));
    });
    if places.is_empty() {
        ui.label(RichText::new("No tables.").color(pal.text_dim));
        return None;
    }
    const PAD: f32 = 10.0;
    const GAP_X: f32 = 90.0;
    const GAP_Y: f32 = 28.0;
    let body = egui::FontId::proportional(13.0);
    let head = egui::FontId::proportional(14.0);
    let row_h = 20.0;
    let head_h = 28.0;
    let painter = ui.painter().clone();
    let width =
        |text: &str, font: &egui::FontId| painter.layout_no_wrap(text.to_owned(), font.clone(), pal.text).size().x;
    // Each box's size, then each layout column's width.
    let sizes: Vec<egui::Vec2> = places
        .iter()
        .map(|&(i, ..)| {
            let t = &tables[i];
            let rows = t
                .columns
                .iter()
                .map(|c| width(&c.name, &body) + width(&c.ty.to_lowercase(), &body) + 56.0)
                .fold(width(&t.name, &head) + 2.0 * PAD, f32::max);
            egui::vec2(rows.max(150.0), head_h + t.columns.len().max(1) as f32 * row_h + 6.0)
        })
        .collect();
    let columns = places.iter().map(|p| p.1).max().unwrap_or(0) + 1;
    let mut col_w = vec![0.0f32; columns];
    for (k, p) in places.iter().enumerate() {
        col_w[p.1] = col_w[p.1].max(sizes[k].x);
    }
    let col_x: Vec<f32> = col_w
        .iter()
        .scan(0.0, |x, w| {
            let at = *x;
            *x += w + GAP_X;
            Some(at)
        })
        .collect();
    let mut col_y = vec![0.0f32; columns];
    let mut boxes = vec![egui::Rect::NOTHING; tables.len()];
    for (k, p) in places.iter().enumerate() {
        boxes[p.0] = egui::Rect::from_min_size(egui::pos2(col_x[p.1], col_y[p.1]), sizes[k]);
        col_y[p.1] += sizes[k].y + GAP_Y;
    }
    let total = egui::vec2(
        col_x.last().copied().unwrap_or(0.0) + col_w.last().copied().unwrap_or(0.0) + GAP_X,
        col_y.iter().copied().fold(0.0, f32::max) + GAP_Y,
    );
    let mut open = None;
    egui::ScrollArea::both().id_salt("db_diagram").auto_shrink([false, false]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(total, egui::Sense::hover());
        let origin = area.min.to_vec2() + egui::vec2(GAP_X / 2.0 + 6.0, 8.0);
        let painter = ui.painter_at(ui.clip_rect());
        let rect_of = |i: usize| boxes[i].translate(origin);
        // Which box the pointer is over, to bring its links forward.
        let mut hovered = None;
        for &(i, ..) in &places {
            let resp = ui.interact(rect_of(i), egui::Id::unique(("db_er", i)), egui::Sense::click());
            if resp.hovered() {
                hovered = Some(i);
            }
            if resp.double_clicked() {
                open = Some(tables[i].name.clone());
            }
            resp.on_hover_text(format!("{}: double click to see its rows", tables[i].name));
        }
        let row_y = |i: usize, column: &str| {
            let r = rect_of(i);
            match tables[i].columns.iter().position(|c| c.name.eq_ignore_ascii_case(column)) {
                Some(k) => r.top() + head_h + (k as f32 + 0.5) * row_h,
                None => r.top() + head_h / 2.0,
            }
        };
        // Links first, so boxes cover their ends.
        for &(i, ..) in &places {
            for fk in &tables[i].foreign_keys {
                let Some(&(to, ..)) = places.iter().find(|p| tables[p.0].name.eq_ignore_ascii_case(&fk.table)) else {
                    continue;
                };
                let (a, b) = (rect_of(i), rect_of(to));
                let (ya, yb) = (row_y(i, &fk.column), row_y(to, &fk.to));
                // From the side facing the other box; a box in the same column loops round its left.
                let (from, from_dir, end, end_dir) = if b.right() <= a.left() {
                    (egui::pos2(a.left(), ya), -1.0, egui::pos2(b.right(), yb), 1.0)
                } else if b.left() >= a.right() {
                    (egui::pos2(a.right(), ya), 1.0, egui::pos2(b.left(), yb), -1.0)
                } else {
                    (egui::pos2(a.left(), ya), -1.0, egui::pos2(b.left(), yb), -1.0)
                };
                let reach = ((end.x - from.x).abs() / 2.0).max(40.0);
                let lit = hovered == Some(i) || hovered == Some(to);
                let color = if lit { pal.accent } else { pal.text_dim.gamma_multiply(0.7) };
                let stroke = Stroke::new(if lit { 2.0 } else { 1.2 }, color);
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [from, from + egui::vec2(from_dir * reach, 0.0), end + egui::vec2(end_dir * reach, 0.0), end],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
                // An arrowhead at the table pointed to; a dot where the key is.
                let tip = end;
                let back = tip + egui::vec2(end_dir * 8.0, 0.0);
                painter.add(egui::Shape::convex_polygon(
                    vec![tip, back + egui::vec2(0.0, 4.5), back - egui::vec2(0.0, 4.5)],
                    color,
                    Stroke::NONE,
                ));
                painter.circle_filled(from, 3.0, color);
            }
        }
        for &(i, ..) in &places {
            let t = &tables[i];
            let r = rect_of(i);
            let lit = hovered == Some(i);
            painter.rect(
                r,
                8.0,
                pal.widget_bg,
                Stroke::new(if lit { 2.0 } else { 1.0 }, if lit { pal.accent } else { pal.border }),
                egui::StrokeKind::Inside,
            );
            let head_rect = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), head_h));
            painter.line_segment([head_rect.left_bottom(), head_rect.right_bottom()], Stroke::new(1.0, pal.border));
            painter.text(
                head_rect.left_center() + egui::vec2(PAD, 0.0),
                egui::Align2::LEFT_CENTER,
                &t.name,
                head.clone(),
                pal.text,
            );
            for (k, c) in t.columns.iter().enumerate() {
                let y = r.top() + head_h + (k as f32 + 0.5) * row_h;
                let fk = t.foreign_keys.iter().any(|f| f.column == c.name);
                let mark = match (c.primary_key, fk) {
                    (true, _) => "PK",
                    (false, true) => "FK",
                    _ => "",
                };
                painter.text(
                    egui::pos2(r.left() + PAD, y),
                    egui::Align2::LEFT_CENTER,
                    mark,
                    egui::FontId::proportional(10.0),
                    if c.primary_key { WARN } else { pal.accent },
                );
                painter.text(
                    egui::pos2(r.left() + PAD + 24.0, y),
                    egui::Align2::LEFT_CENTER,
                    &c.name,
                    body.clone(),
                    pal.text,
                );
                painter.text(
                    egui::pos2(r.right() - PAD, y),
                    egui::Align2::RIGHT_CENTER,
                    c.ty.to_lowercase(),
                    body.clone(),
                    pal.text_dim,
                );
            }
        }
    });
    open
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
