//! SQL Databases: a SQLite file (through `sqlite3`) or a PostgreSQL, MySQL/MariaDB or SQL Server
//! database (through `psql`, `mariadb`/`mysql`, `sqlcmd`), in a panel: its tables and views with
//! their columns, keys and indexes, SQL run with each result as a table, plans, and the tables
//! as a diagram; two databases' tables compared, with the SQL that makes one like the other; and
//! the project's migrations (sqlx, Diesel, Alembic, Flyway) with which have run. The clients
//! run as approved programs; a password goes to them in their
//! environment for that run only and is never saved.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod compare;
pub mod migrations;
pub mod server;
pub mod sqlite;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, EdgeKind, FieldKind, Form, FormField, FormValues, Graph, GraphEdge, GraphNode, JobResult, JobSpec,
    Pick, PickChoice, PickItem, Row, Span, Table as Grid,
};
use compare::{Change, Dialect};
use migrations::{Migration, Status, Tool};
use server::{Kind, Server};
use sqlite::{Outcome, Plan, Table};

/// A program to run: what the extension starts as a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: String,
    /// Environment variables for this run only (a password).
    pub env: Vec<(String, String)>,
    /// A file to write in the extension's own folder first (a script), relative to it: the
    /// program then runs there.
    pub file: Option<(String, String)>,
}

impl Call {
    pub fn new(program: &str, args: Vec<String>, stdin: String, env: Vec<(String, String)>) -> Self {
        Self { program: program.to_owned(), args, stdin, env, file: None }
    }
}

/// What a program printed, and how it ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Out {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// "sqlite3 stopped (exit 1)" when it said nothing.
    pub fn stopped(&self, program: &str) -> String {
        match self.code {
            Some(c) => format!("{program} stopped (exit {c})"),
            None => format!("{program} was stopped"),
        }
    }
}

const RED: u32 = 0xE5484D;
const AMBER: u32 = 0xE09A3E;
const DIM: u32 = 0x8B949E;
const GREEN: u32 = 0x3FB950;

/// Statements kept in the history.
const HISTORY: usize = 20;
/// Rows a table's Rows button reads.
const PEEK: usize = 200;
const KINDS: [(Kind, &str); 3] =
    [(Kind::Postgres, "PostgreSQL"), (Kind::MySql, "MySQL or MariaDB"), (Kind::SqlServer, "SQL Server")];

#[derive(Clone)]
enum Target {
    /// A SQLite file's absolute path.
    Sqlite(String),
    Server(Server),
}

impl Target {
    fn title(&self) -> String {
        match self {
            Target::Sqlite(path) => path.rsplit(['/', '\\']).next().unwrap_or(path).to_owned(),
            Target::Server(s) => s.title(),
        }
    }

    fn quote(&self, name: &str) -> String {
        match self {
            Target::Sqlite(_) => sqlite::quote_name(name),
            Target::Server(s) => s.kind.quote_name(name),
        }
    }

    fn dialect(&self) -> Dialect {
        match self {
            Target::Sqlite(_) => Dialect::Sqlite,
            Target::Server(s) => match s.kind {
                Kind::Postgres => Dialect::Postgres,
                Kind::MySql => Dialect::MySql,
                Kind::SqlServer => Dialect::SqlServer,
            },
        }
    }
}

/// What a running client is for.
enum Then {
    Run {
        starts: Vec<usize>,
        started: i64,
    },
    Schema,
    SqliteExplain {
        statements: Vec<String>,
        starts: Vec<usize>,
    },
    /// A server's plans, one statement at a time: what's left, and what's done.
    ServerExplain {
        left: Vec<String>,
        done: Vec<Plan>,
    },
    /// LocalDB started; its pipe is asked for next.
    LocalDbStarted(Server, String),
    LocalDbInfo(Server, String),
    /// The tables of the database compared with the open one.
    OtherSchema(Target),
    /// What a migration tool's table says ran.
    History(Tool),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Results,
    Plan,
    Compare,
    Migrations,
}

#[derive(Default)]
struct State {
    target: Option<Target>,
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    tables: Option<Vec<Table>>,
    open_tables: HashSet<String>,
    sql: String,
    history: Vec<String>,
    results: Option<Outcome>,
    plans: Option<Vec<Plan>>,
    view: View,
    connecting: bool,
    form: Option<Server>,
    saved: Vec<Server>,
    /// Scripts written for `sqlcmd`, to number the next.
    scripts: u64,
    /// What Compare… offers, by the text shown.
    compare_choices: Vec<(String, Target)>,
    /// The database compared with, its tables, and what differs.
    other: Option<(Target, Vec<Table>)>,
    changes: Vec<Change>,
    compare_sql: String,
    migrations: Option<Vec<Migration>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

// ----- running the clients ------------------------------------------------------------------

/// Starts a call on the open database; for a server, the first of its client programs here.
fn start(call: Call, then: Then, busy: &str) -> Result<(), String> {
    start_on(with(|s| s.target.clone()), call, then, busy)
}

/// Starts a call on `target`.
fn start_on(target: Option<Target>, call: Call, then: Then, busy: &str) -> Result<(), String> {
    let names: Vec<String> = match target.clone() {
        Some(Target::Server(s)) if call.program != "SqlLocalDB" => {
            server::programs(s.kind).iter().map(|p| (*p).to_owned()).collect()
        }
        _ => vec![call.program.clone()],
    };
    let own = call.file.is_some();
    if let Some((file, text)) = &call.file {
        host::data_write(file, text)?;
    }
    let mut last = String::new();
    for name in &names {
        let spec = JobSpec {
            program: name.clone(),
            args: call.args.clone(),
            stdin: call.stdin.clone(),
            env: call.env.clone(),
            in_own_folder: own,
        };
        match host::start_job_with(&spec) {
            Ok(id) => {
                with(|s| {
                    s.jobs.insert(id, then);
                    s.busy = Some(busy.to_owned());
                    s.error = None;
                });
                return Ok(());
            }
            // Not here: the next name (mysql after mariadb).
            Err(e) if e.contains("isn't on PATH") => last = e,
            Err(e) => return Err(e),
        }
    }
    match target {
        Some(Target::Server(s)) if call.program != "SqlLocalDB" => Err(server::missing_client(s.kind)),
        Some(Target::Sqlite(_)) => Err("The sqlite3 program isn't on this computer. Install SQLite's command-line \
             shell (sqlite.org, your package manager, or MSYS2's sqlite3) and try again."
            .to_owned()),
        _ => Err(last),
    }
}

/// The call for `script` on the server, its script file numbered.
fn server_call(s: &Server, script: &str) -> Call {
    let n = with(|st| {
        st.scripts += 1;
        st.scripts
    });
    let program = server::programs(s.kind)[0];
    s.call(program, script, &format!("scripts/{n}.sql"))
}

fn read_schema() -> Result<(), String> {
    match with(|s| s.target.clone()).ok_or("Open a database first")? {
        Target::Sqlite(db) => start(sqlite::schema_call(&db), Then::Schema, "Reading the tables…"),
        Target::Server(s) => start(server_call(&s, &s.schema_script()), Then::Schema, "Reading the tables…"),
    }
}

fn run_sql(sql: String, explain: bool) -> Result<(), String> {
    let sql = sql.trim().to_owned();
    if sql.is_empty() {
        return Ok(());
    }
    let target = with(|s| s.target.clone()).ok_or("Open a database first (open a SQLite file, or connect)")?;
    with(|s| {
        s.sql = sql.clone();
        s.history.retain(|h| h != &sql);
        s.history.insert(0, sql.clone());
        s.history.truncate(HISTORY);
        s.view = if explain { View::Plan } else { View::Results };
    });
    let started = host::now().unix_millis;
    match (target, explain) {
        (Target::Sqlite(db), false) => {
            start(sqlite::run_call(&db, &sql), Then::Run { starts: Vec::new(), started }, "Running…")
        }
        (Target::Sqlite(db), true) => {
            let (call, statements, starts) = sqlite::explain_call(&db, &sql);
            start(call, Then::SqliteExplain { statements, starts }, "Asking for the plan…")
        }
        (Target::Server(s), false) => {
            let (script, starts) = s.run_script(&sql);
            start(server_call(&s, &script), Then::Run { starts, started }, "Running…")
        }
        (Target::Server(_), true) => {
            let left = sqlite::split_statements(&sql);
            explain_next(left, Vec::new())
        }
    }
}

/// The next statement's plan, or the plans when none is left.
fn explain_next(mut left: Vec<String>, mut done: Vec<Plan>) -> Result<(), String> {
    let Some(Target::Server(s)) = with(|st| st.target.clone()) else { return Ok(()) };
    while !left.is_empty() {
        let statement = left.remove(0);
        match s.explain_script(&statement) {
            Some(script) => {
                done.push(Plan { statement, steps: Vec::new() });
                return start(server_call(&s, &script), Then::ServerExplain { left, done }, "Asking for the plan…");
            }
            // No plan for this one (CREATE TABLE and the like).
            None => done.push(Plan { statement, steps: Vec::new() }),
        }
    }
    with(|st| st.plans = Some(done));
    Ok(())
}

/// Opens a server: LocalDB is started and its pipe found first; then its tables are read.
fn connect(mut s: Server) -> Result<(), String> {
    if s.host.trim().is_empty() {
        return Err("Name the host (localhost, a name, or (localdb)\\MSSQLLocalDB)".into());
    }
    if s.kind == Kind::SqlServer && s.address.is_none() {
        if let Some(instance) = s.localdb_instance() {
            with(|st| st.target = Some(Target::Server(s.clone())));
            let call = Call::new("SqlLocalDB", vec!["start".into(), instance.clone()], String::new(), Vec::new());
            return start(call, Then::LocalDbStarted(s, instance), "Starting LocalDB…");
        }
        s.address = Some(s.plain_address());
    }
    with(|st| {
        st.target = Some(Target::Server(s.clone()));
        st.tables = None;
        st.results = None;
        st.plans = None;
        st.connecting = false;
        let text = server::remember(&mut st.saved, &s);
        let _ = host::storage_write(&text);
    });
    read_schema()
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let out = Out { code: job.exit_code, stdout: job.stdout, stderr: job.stderr };
    if with(|s| s.jobs.is_empty()) {
        with(|s| s.busy = None);
    }
    let target = with(|s| s.target.clone());
    let result: Result<(), String> = match (then, target) {
        (Then::Run { starts, started }, Some(t)) => {
            let outcome = match &t {
                Target::Sqlite(_) => sqlite::run_read(&out),
                Target::Server(s) => s.run_read(&out, &starts),
            };
            outcome.map(|mut o| {
                o.millis = (host::now().unix_millis - started).max(0) as u128;
                // What it changed may change the tables too.
                let changed = o.changes > 0 || o.sets.is_empty();
                with(|s| s.results = Some(o));
                if changed {
                    let _ = read_schema();
                }
            })
        }
        (Then::Schema, Some(t)) => {
            let tables = match &t {
                Target::Sqlite(_) => sqlite::schema_read(&out),
                Target::Server(s) => s.schema_read(&out),
            };
            tables.map(|tables| {
                let other = with(|s| {
                    s.tables = Some(tables);
                    s.other.clone().filter(|_| s.view == View::Compare)
                });
                // Read again while compared: what differs, again.
                if let Some((other, theirs)) = other {
                    compared(&t, other, theirs);
                }
            })
        }
        (Then::SqliteExplain { statements, starts }, _) => {
            sqlite::explain_read(&out, statements, &starts).map(|p| with(|s| s.plans = Some(p)))
        }
        (Then::ServerExplain { left, mut done }, Some(Target::Server(s))) => match s.explain_read(&out) {
            Ok(steps) => {
                if let Some(last) = done.last_mut() {
                    last.steps = steps;
                }
                explain_next(left, done)
            }
            Err(e) => Err(e),
        },
        (Then::OtherSchema(other), Some(t)) => {
            let tables = match &other {
                Target::Sqlite(_) => sqlite::schema_read(&out),
                Target::Server(s) => s.schema_read(&out),
            };
            tables.map(|theirs| compared(&t, other, theirs))
        }
        (Then::History(tool), Some(t)) => {
            let outcome = match &t {
                Target::Sqlite(_) => sqlite::run_read(&out),
                Target::Server(s) => s.run_read(&out, &[0]),
            };
            outcome.map(|o| {
                with(|s| {
                    if let Some(m) = &mut s.migrations {
                        migrations::mark(m, tool, o.sets.first());
                    }
                })
            })
        }
        (Then::LocalDbStarted(s, instance), _) => {
            let call = Call::new("SqlLocalDB", vec!["info".into(), instance.clone()], String::new(), Vec::new());
            start(call, Then::LocalDbInfo(s, instance), "Starting LocalDB…")
        }
        (Then::LocalDbInfo(mut s, instance), _) => match server::localdb_pipe(&out.stdout, &instance) {
            Ok(pipe) => {
                s.address = Some(pipe);
                connect(s)
            }
            Err(e) => Err(e),
        },
        _ => Ok(()),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    }
    render();
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn mono(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: true }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: vec![], buttons }])
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options: vec![], hint: hint.into() }
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let mut top = vec![button("Open SQLite file…", "open-sqlite", ""), button("Connect…", "connect", "")];
        if s.target.is_some() {
            top.extend([
                button("Refresh", "refresh", ""),
                button("Diagram", "diagram", ""),
                button("Compare…", "compare", ""),
            ]);
        }
        top.push(button("Migrations", "migrations", ""));
        match &s.target {
            Some(t) => b.push(Block::Spans(vec![Span { text: t.title(), color: None, bold: true, monospace: false }])),
            None => b.push(Block::Spans(vec![span("No database open.", Some(DIM))])),
        }
        b.push(bar(top));
        if let Some(busy) = &s.busy {
            b.push(Block::Spans(vec![span(busy, Some(DIM))]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        if s.connecting {
            connect_ui(s, &mut b);
        }
        if s.target.is_none() {
            if s.view == View::Migrations {
                migrations_ui(s, &mut b);
            }
            return b;
        }
        if s.view == View::Compare {
            compare_ui(s, &mut b);
            return b;
        }
        tables_ui(s, &mut b);
        b.push(Block::Form(Form {
            id: "sql".into(),
            fields: vec![field(
                "sql",
                "",
                FieldKind::Multiline,
                &s.sql,
                "SQL: one or more statements (Ctrl+Enter runs them)",
            )],
            buttons: vec!["Run".into(), "Explain".into()],
        }));
        if !s.history.is_empty() {
            let rows = s
                .history
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, h)| Row {
                    cells: vec![mono(h.lines().next().unwrap_or("").chars().take(90).collect::<String>(), Some(DIM))],
                    buttons: vec![button("Again", "history", &i.to_string())],
                })
                .collect();
            b.push(Block::Rows(rows));
        }
        match s.view {
            View::Results => results_ui(s, &mut b),
            View::Plan => plan_ui(s, &mut b),
            View::Migrations => migrations_ui(s, &mut b),
            View::Compare => {}
        }
        b
    });
    host::set_panel("sql", &blocks);
}

fn connect_ui(s: &State, b: &mut Vec<Block>) {
    let f = s.form.clone().unwrap_or_default();
    let kind = KINDS.iter().find(|(k, _)| *k == f.kind).map_or("PostgreSQL", |(_, n)| n);
    let mut choice = field("kind", "", FieldKind::Choice, kind, "");
    choice.options = KINDS.iter().map(|(_, n)| (*n).to_owned()).collect();
    let port = if f.port == 0 { String::new() } else { f.port.to_string() };
    b.push(Block::Heading("Connect to a server".into()));
    b.push(Block::Form(Form {
        id: "connect".into(),
        fields: vec![
            choice,
            field("name", "Name", FieldKind::Text, &f.name, "shop, staging (optional)"),
            field("", "", FieldKind::LineBreak, "", ""),
            field("host", "Host", FieldKind::Text, &f.host, "localhost, or (localdb)\\MSSQLLocalDB"),
            field("port", "Port", FieldKind::Text, &port, "the usual one"),
            field("", "", FieldKind::LineBreak, "", ""),
            field("user", "User", FieldKind::Text, &f.user, "SQL Server: empty for Windows sign-in"),
            field("password", "Password", FieldKind::Password, "", "never saved"),
            field("database", "Database", FieldKind::Text, &f.database, ""),
        ],
        buttons: vec!["Connect".into(), "Cancel".into()],
    }));
    if !s.saved.is_empty() {
        b.push(Block::Spans(vec![span("Used before (passwords aren't kept):", Some(DIM))]));
        let rows = s
            .saved
            .iter()
            .enumerate()
            .map(|(i, v)| Row {
                cells: vec![span(v.title(), None), mono(v.label(), Some(DIM))],
                buttons: vec![
                    button("Use", "saved-use", &i.to_string()),
                    button("Forget", "saved-forget", &i.to_string()),
                ],
            })
            .collect();
        b.push(Block::Rows(rows));
    }
    b.push(Block::Separator);
}

fn tables_ui(s: &State, b: &mut Vec<Block>) {
    let Some(tables) = &s.tables else { return };
    if tables.is_empty() {
        b.push(Block::Text("No tables yet.".into()));
        return;
    }
    let mut rows = Vec::new();
    for t in tables {
        let open = s.open_tables.contains(&t.name);
        rows.push(Row {
            cells: vec![
                Span { text: t.name.clone(), color: None, bold: true, monospace: true },
                span(format!("{}{} columns", if t.view { "view · " } else { "" }, t.columns.len()), Some(DIM)),
            ],
            buttons: vec![
                button("Rows", "rows", &t.name),
                button(if open { "Hide" } else { "Columns" }, "table", &t.name),
            ],
        });
        if !open {
            continue;
        }
        for c in &t.columns {
            let mut notes = Vec::new();
            if c.primary_key {
                notes.push("primary key".to_owned());
            }
            if c.not_null {
                notes.push("not null".to_owned());
            }
            if let Some(fk) = t.foreign_keys.iter().find(|f| f.column == c.name) {
                notes.push(format!("→ {}.{}", fk.table, fk.to));
            }
            rows.push(Row {
                cells: vec![
                    mono(format!("   {}", c.name), None),
                    mono(&*c.ty, Some(DIM)),
                    span(notes.join(", "), Some(DIM)),
                ],
                buttons: vec![],
            });
        }
        for x in &t.indexes {
            let how = format!(
                "{}{}{}",
                if x.unique { "unique " } else { "" },
                if x.partial { "partial " } else { "" },
                if x.automatic { "(from a constraint)" } else { "index" }
            );
            rows.push(Row {
                cells: vec![
                    mono(format!("   {}", x.name), Some(DIM)),
                    mono(x.columns.join(", "), Some(DIM)),
                    span(how, Some(DIM)),
                ],
                buttons: vec![],
            });
        }
    }
    b.push(Block::Rows(rows));
}

fn results_ui(s: &State, b: &mut Vec<Block>) {
    let Some(o) = &s.results else { return };
    b.push(Block::Spans(vec![span(
        format!(
            "{} {} · {} rows changed · {} ms",
            o.sets.len(),
            if o.sets.len() == 1 { "result" } else { "results" },
            o.changes,
            o.millis
        ),
        Some(DIM),
    )]));
    for set in &o.sets {
        let rows =
            set.rows.iter().map(|r| r.iter().map(|c| c.clone().unwrap_or_else(|| "NULL".into())).collect()).collect();
        b.push(Block::Table(Grid { headers: set.columns.clone(), rows }));
        if set.more > 0 {
            b.push(Block::Spans(vec![span(
                format!("… and {} more rows, not kept ({} at most)", set.more, sqlite::MAX_ROWS),
                Some(DIM),
            )]));
        }
    }
}

fn plan_ui(s: &State, b: &mut Vec<Block>) {
    let Some(plans) = &s.plans else { return };
    for p in plans {
        b.push(Block::Spans(vec![mono(p.statement.lines().next().unwrap_or(""), Some(DIM))]));
        if p.steps.is_empty() {
            b.push(Block::Spans(vec![span("   (no plan: it isn't a query)", Some(DIM))]));
            continue;
        }
        for step in &p.steps {
            let warn = step.full_scan || step.temp_sort;
            let mut line =
                vec![mono(format!("{}{}", "   ".repeat(step.depth + 1), step.detail), warn.then_some(AMBER))];
            if step.full_scan {
                line.push(span("   reads every row: an index may help", Some(AMBER)));
            } else if step.temp_sort {
                line.push(span("   sorts in a temporary table: an index in that order would skip it", Some(AMBER)));
            }
            b.push(Block::Spans(line));
        }
    }
}

fn compare_ui(s: &State, b: &mut Vec<Block>) {
    let Some((other, _)) = &s.other else { return };
    let this = s.target.as_ref().map(Target::title).unwrap_or_default();
    b.push(Block::Spans(vec![
        span("Compared with ", Some(DIM)),
        Span { text: other.title(), color: None, bold: true, monospace: false },
        span(format!(": {} differences", s.changes.len()), Some(DIM)),
    ]));
    b.push(bar(vec![
        button("Compare again", "compare-again", ""),
        button("Compare with…", "compare", ""),
        button("Back to the tables", "compare-close", ""),
    ]));
    if s.changes.is_empty() {
        b.push(Block::Spans(vec![span("Same tables, columns, keys and indexes.", Some(GREEN))]));
        return;
    }
    let rows = s
        .changes
        .iter()
        .map(|c| {
            let color = match c.sign() {
                "+" => GREEN,
                "−" => RED,
                _ => AMBER,
            };
            Row { cells: vec![mono(c.sign(), Some(color)), mono(c.describe(), None)], buttons: vec![] }
        })
        .collect();
    b.push(Block::Rows(rows));
    b.push(Block::Spans(vec![span(format!("The SQL that makes {this} like {}:", other.title()), Some(DIM))]));
    b.push(Block::Code(s.compare_sql.clone()));
    b.push(bar(vec![button("Open as SQL", "compare-open", ""), button("Put in the SQL box", "compare-box", "")]));
}

fn migrations_ui(s: &State, b: &mut Vec<Block>) {
    let Some(list) = &s.migrations else { return };
    for tool in Tool::ALL {
        let mine: Vec<&Migration> = list.iter().filter(|m| m.tool == tool).collect();
        if mine.is_empty() {
            continue;
        }
        let count = |st: Status| mine.iter().filter(|m| m.status == st).count();
        let summary = if mine.iter().all(|m| m.status == Status::Unknown) {
            "open the database to see which have run".to_owned()
        } else {
            let mut parts = vec![format!("{} ran", count(Status::Ran)), format!("{} not run", count(Status::NotRun))];
            if count(Status::Failed) > 0 {
                parts.push(format!("{} failed", count(Status::Failed)));
            }
            parts.join(", ")
        };
        b.push(Block::Spans(vec![
            Span { text: tool.label().into(), color: None, bold: true, monospace: false },
            span(format!("  {} · {summary}", tool.table()), Some(DIM)),
        ]));
        let rows = mine
            .iter()
            .map(|m| {
                let (status, color) = match m.status {
                    Status::Ran => ("ran", GREEN),
                    Status::NotRun => ("not run", AMBER),
                    Status::Failed => ("failed", RED),
                    Status::Unknown => ("?", DIM),
                };
                Row {
                    cells: vec![mono(&m.version, Some(DIM)), span(&m.name, None), span(status, Some(color))],
                    buttons: vec![button("Open", "open-file", &m.path)],
                }
            })
            .collect();
        b.push(Block::Rows(rows));
    }
}

/// What differs between the open database and `other`, and the SQL that makes it like `other`.
fn compared(this: &Target, other: Target, theirs: Vec<Table>) {
    with(|s| {
        let mine = s.tables.clone().unwrap_or_default();
        s.changes = compare::diff(&mine, &theirs);
        s.compare_sql = compare::script(&s.changes, &mine, &theirs, this.dialect());
        s.other = Some((other, theirs));
        s.view = View::Compare;
    });
}

/// Offers the SQLite files of the folder and the saved connections to compare with.
fn compare_pick() -> Result<(), String> {
    let (target, saved) = with(|s| {
        s.saved = server::saved(&host::storage_read());
        (s.target.clone(), s.saved.clone())
    });
    let target = target.ok_or("Open a database first")?;
    if with(|s| s.tables.is_none()) {
        return Err("The open database's tables aren't read yet".into());
    }
    let root = host::workspace_path().unwrap_or_default();
    let mut choices: Vec<(String, String, Target)> = Vec::new();
    for ext in ["db", "sqlite", "sqlite3", "db3", "s3db", "sl3"] {
        for f in host::list_files(&format!(".{ext}"), 200).unwrap_or_default() {
            let path = format!("{}/{}", root.trim_end_matches(['/', '\\']), f);
            if matches!(&target, Target::Sqlite(open) if open.replace('\\', "/") == path.replace('\\', "/")) {
                continue;
            }
            choices.push((f, "SQLite".into(), Target::Sqlite(path)));
        }
    }
    for v in saved {
        if matches!(&target, Target::Server(open) if open.label() == v.label()) {
            continue;
        }
        choices.push((v.title(), v.kind.label().into(), Target::Server(v)));
    }
    if choices.is_empty() {
        return Err("Nothing to compare with: open another SQLite file in the folder, or connect to another \
             database once so it's kept"
            .into());
    }
    let items = choices
        .iter()
        .map(|(text, kind, _)| PickItem {
            text: text.clone(),
            kind: kind.clone(),
            detail: String::new(),
            can_forget: false,
        })
        .collect();
    with(|s| s.compare_choices = choices.into_iter().map(|(t, _, target)| (t, target)).collect());
    host::show_pick(&Pick {
        id: "compare".into(),
        title: "Compare the open database with…".into(),
        query: String::new(),
        items,
        at_terminal_cursor: false,
        run_first: true,
        filter: true,
    });
    Ok(())
}

/// Reads `other`'s tables to compare them with the open database's.
fn compare_with(mut other: Target) -> Result<(), String> {
    let open = with(|s| s.target.clone()).ok_or("Open a database first")?;
    if let (Target::Server(o), Target::Server(this)) = (&mut other, &open) {
        // The same server: its password and address (LocalDB's pipe) serve both.
        if o.kind == this.kind && o.host == this.host && o.port == this.port && o.user == this.user {
            o.password = this.password.clone();
            o.address = this.address.clone();
        }
        if o.address.is_none() {
            if o.kind == Kind::SqlServer && o.localdb_instance().is_some() {
                return Err("Connect to that LocalDB database once first, then compare".into());
            }
            o.address = Some(o.plain_address());
        }
    }
    let call = match &other {
        Target::Sqlite(db) => sqlite::schema_call(db),
        Target::Server(o) => server_call(o, &o.schema_script()),
    };
    start_on(Some(other.clone()), call, Then::OtherSchema(other), "Reading the other database's tables…")
}

/// Finds the project's migrations, then asks the open database which have run.
fn show_migrations() -> Result<(), String> {
    let mut paths = host::list_files(".sql", 10_000).unwrap_or_default();
    paths.extend(host::list_files(".py", 10_000).unwrap_or_default());
    let found = migrations::find(&paths, |p| host::read_file(p).ok());
    if found.is_empty() {
        with(|s| s.migrations = None);
        return Err("No migrations in the open folder (sqlx, Diesel, Alembic or Flyway)".into());
    }
    let tools: Vec<Tool> = Tool::ALL.into_iter().filter(|t| found.iter().any(|m| m.tool == *t)).collect();
    let (target, tables) = with(|s| {
        s.migrations = Some(found);
        s.view = View::Migrations;
        (s.target.clone(), s.tables.clone())
    });
    let (Some(target), Some(tables)) = (target, tables) else { return Ok(()) };
    for tool in tools {
        if !tables.iter().any(|t| t.name.eq_ignore_ascii_case(tool.table())) {
            // No table yet: nothing of it ran here.
            with(|s| {
                if let Some(m) = &mut s.migrations {
                    migrations::mark(m, tool, None);
                }
            });
            continue;
        }
        let call = match &target {
            Target::Sqlite(db) => sqlite::run_call(db, &tool.query()),
            Target::Server(s) => server_call(s, &s.run_script(&tool.query()).0),
        };
        start(call, Then::History(tool), "Asking which migrations ran…")?;
    }
    Ok(())
}

fn diagram() -> Result<(), String> {
    let (title, tables) = with(|s| (s.target.as_ref().map(Target::title), s.tables.clone()));
    let tables = tables.ok_or("Open a database first")?;
    let nodes = tables
        .iter()
        .map(|t| GraphNode {
            id: t.name.clone(),
            label: t.name.clone(),
            kind: if t.view { "view".into() } else { "table".into() },
            group: String::new(),
            members: t
                .columns
                .iter()
                .map(|c| format!("{}{}: {}", if c.primary_key { "# " } else { "+ " }, c.name, c.ty))
                .collect(),
            path: String::new(),
            line: 0,
            accent: None,
        })
        .collect();
    let edges = tables
        .iter()
        .flat_map(|t| {
            t.foreign_keys.iter().map(|fk| GraphEdge {
                source: t.name.clone(),
                target: fk.table.clone(),
                kind: EdgeKind::Uses,
                label: format!("{} → {}", fk.column, fk.to),
                accent: None,
            })
        })
        .collect();
    host::show_graph(&Graph {
        id: "er".into(),
        title: format!("{}: tables", title.unwrap_or_default()),
        nodes,
        edges,
        focus: None,
    });
    Ok(())
}

// ----- what the user does ---------------------------------------------------------------------

fn open_sqlite(path: &str) -> Result<(), String> {
    if path.is_empty() {
        let mut found = Vec::new();
        for ext in ["db", "sqlite", "sqlite3", "db3", "s3db", "sl3"] {
            found.extend(host::list_files(&format!(".{ext}"), 200).unwrap_or_default());
        }
        if found.is_empty() {
            return Err(
                "No SQLite files in the open folder (.db, .sqlite, .sqlite3): open one from the Explorer".into()
            );
        }
        let items = found
            .into_iter()
            .map(|f| PickItem { text: f, kind: "SQLite".into(), detail: String::new(), can_forget: false })
            .collect();
        host::show_pick(&Pick {
            id: "sqlite".into(),
            title: "Open which SQLite file?".into(),
            query: String::new(),
            items,
            at_terminal_cursor: false,
            run_first: true,
            filter: true,
        });
        return Ok(());
    }
    with(|s| {
        s.target = Some(Target::Sqlite(path.to_owned()));
        s.tables = None;
        s.results = None;
        s.plans = None;
        s.open_tables.clear();
    });
    read_schema()
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let result = match id {
        "show" => Ok(()),
        "open-sqlite" => open_sqlite(arg),
        "connect" => {
            let saved = server::saved(&host::storage_read());
            with(|s| {
                s.saved = saved;
                s.connecting = true;
                if s.form.is_none() {
                    s.form = Some(Server {
                        kind: Kind::Postgres,
                        host: "localhost".into(),
                        port: Kind::Postgres.default_port(),
                        user: Kind::Postgres.default_user().into(),
                        ..Server::default()
                    });
                }
            });
            Ok(())
        }
        "refresh" => read_schema(),
        "compare" => compare_pick(),
        "compare-again" => match with(|s| s.other.clone()) {
            Some((other, _)) => {
                // Both read again: the open one first.
                read_schema().and_then(|()| compare_with(other))
            }
            None => compare_pick(),
        },
        "compare-close" => {
            with(|s| s.view = View::Results);
            Ok(())
        }
        "compare-open" => {
            host::open_text(&with(|s| s.compare_sql.clone()));
            Ok(())
        }
        "compare-box" => {
            with(|s| {
                s.sql = s.compare_sql.clone();
                s.view = View::Results;
            });
            Ok(())
        }
        "migrations" => show_migrations(),
        "open-file" => host::open_location(arg, 1),
        "diagram" => diagram(),
        "run" | "explain" => {
            let doc = host::active_document()?.ok_or("Open a .sql file, or type the SQL in the panel")?;
            let sql = if doc.selection.trim().is_empty() { doc.text } else { doc.selection };
            run_sql(sql, id == "explain")
        }
        "rows" => {
            let sql = with(|s| s.target.as_ref().map(|t| format!("SELECT * FROM {} LIMIT {PEEK}", t.quote(arg))));
            match sql {
                Some(sql) => run_sql(sql, false),
                None => Ok(()),
            }
        }
        "table" => {
            with(|s| {
                if !s.open_tables.remove(arg) {
                    s.open_tables.insert(arg.to_owned());
                }
            });
            Ok(())
        }
        "history" => {
            with(|s| {
                if let Some(h) = arg.parse::<usize>().ok().and_then(|i| s.history.get(i)).cloned() {
                    s.sql = h;
                }
            });
            Ok(())
        }
        "saved-use" => {
            with(|s| {
                if let Some(v) = arg.parse::<usize>().ok().and_then(|i| s.saved.get(i)).cloned() {
                    s.form = Some(v);
                }
            });
            Ok(())
        }
        "saved-forget" => {
            with(|s| {
                if let Some(i) = arg.parse::<usize>().ok().filter(|i| *i < s.saved.len()) {
                    let gone = s.saved.remove(i);
                    // Kept again without it.
                    let rest = std::mem::take(&mut s.saved);
                    for v in rest.iter().rev() {
                        let _ = server::remember(&mut s.saved, v);
                    }
                    let _ = gone;
                    #[derive(serde::Serialize)]
                    struct File<'a> {
                        server: &'a [Server],
                    }
                    let _ = host::storage_write(&toml::to_string(&File { server: &s.saved }).unwrap_or_default());
                }
            });
            Ok(())
        }
        _ => return Err(format!("unknown command {id}")),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    let result = match (f.form.as_str(), f.button.as_str()) {
        ("sql", button) => run_sql(get("sql"), button == "Explain"),
        ("connect", "Cancel") => {
            with(|s| s.connecting = false);
            Ok(())
        }
        ("connect", _) => {
            let kind = KINDS.iter().find(|(_, n)| *n == get("kind")).map_or(Kind::Postgres, |(k, _)| *k);
            let port = get("port").trim().parse().unwrap_or(if kind == Kind::SqlServer && get("host").contains('\\') {
                0
            } else {
                kind.default_port()
            });
            let password = get("password");
            let s = Server {
                name: get("name").trim().to_owned(),
                kind,
                host: get("host").trim().to_owned(),
                port,
                user: get("user").trim().to_owned(),
                database: get("database").trim().to_owned(),
                password: (!password.is_empty()).then_some(password),
                address: None,
            };
            with(|st| {
                let mut shown = s.clone();
                shown.password = None;
                st.form = Some(shown);
            });
            connect(s)
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    }
    render();
}

fn picked(c: PickChoice) {
    if c.pick == "compare" {
        let choice = with(|s| s.compare_choices.iter().find(|(t, _)| *t == c.text).map(|(_, t)| t.clone()));
        if let Some(Err(e)) = choice.map(compare_with) {
            with(|s| s.error = Some(e));
        }
        render();
        return;
    }
    if c.pick == "sqlite" {
        let root = host::workspace_path().unwrap_or_default();
        let path = format!("{}/{}", root.trim_end_matches(['/', '\\']), c.text);
        if let Err(e) = open_sqlite(&path) {
            with(|s| s.error = Some(e));
        }
        render();
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        command(&id, &argument)
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => finished(job),
            Event::FormSubmitted(f) => submitted(f),
            Event::Picked(c) => picked(c),
            _ => {}
        }
    }
}

export!(Plugin);
