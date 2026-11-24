//! PostgreSQL, MySQL (or MariaDB) and SQL Server databases through their own command-line
//! clients, `psql`, `mariadb`/`mysql` and `sqlcmd`: the same tables, columns, results and plans
//! as for SQLite.
//!
//! No database code is here and the editor opens no connections: the client does, as it would
//! in a terminal, run by the extension as an approved program. A password is never saved: it's
//! handed to the client in its environment (`PGPASSWORD`, `MYSQL_PWD`, `SQLCMDPASSWORD`) for that
//! run only. Each operation is the command to run ([`Call`]) and how to read what it printed.

use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::sqlite::{Column, ForeignKey, Index, MAX_ROWS, Outcome, PlanStep, ResultSet, Table, split_statements};
use crate::{Call, Out};

/// Which server it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Postgres,
    /// MySQL or MariaDB: one client speaks to both.
    MySql,
    /// Microsoft SQL Server (and LocalDB), through `sqlcmd`.
    SqlServer,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::MySql => "MySQL / MariaDB",
            Self::SqlServer => "SQL Server",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Postgres => 5432,
            Self::MySql => 3306,
            Self::SqlServer => 1433,
        }
    }

    pub fn default_user(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::MySql => "root",
            // Empty: Windows sign-in (integrated security).
            Self::SqlServer => "",
        }
    }

    /// A name as SQL for this server.
    pub fn quote_name(self, name: &str) -> String {
        match self {
            Self::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            Self::MySql => format!("`{}`", name.replace('`', "``")),
            Self::SqlServer => format!("[{}]", name.replace(']', "]]")),
        }
    }
}

/// A server to connect to. Everything but the password is kept for next time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    /// A name of the user's own for the connection ("Shop, staging"); empty: none.
    #[serde(default)]
    pub name: String,
    pub kind: Kind,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    /// Never saved, only held while the app runs.
    #[serde(skip)]
    pub password: Option<String>,
    /// What `sqlcmd -S` takes, once worked out (a LocalDB instance's pipe); not saved.
    #[serde(skip)]
    pub address: Option<String>,
}

impl Server {
    /// How it's shown: `postgres://user@host:5432/shop`.
    pub fn label(&self) -> String {
        let scheme = match self.kind {
            Kind::Postgres => "postgres",
            Kind::MySql => "mysql",
            Kind::SqlServer => "sqlserver",
        };
        // No user (Windows sign-in) and no port (LocalDB, a named instance) aren't shown.
        let user = if self.user.is_empty() { String::new() } else { format!("{}@", self.user) };
        let port = if self.port == 0 { String::new() } else { format!(":{}", self.port) };
        format!("{scheme}://{user}{}{port}/{}", self.host, self.database)
    }

    /// Its name when it has one, else its address.
    pub fn title(&self) -> String {
        if self.name.trim().is_empty() { self.label() } else { self.name.trim().to_owned() }
    }

    /// The client and how it's run for `script`; `program` is the client's name (`psql`,
    /// `mariadb` or `mysql`, `sqlcmd`). SQL Server's script goes in a file (`sqlcmd -i` reads
    /// batches split by `GO`), `file` in the extension's own folder.
    pub fn call(&self, program: &str, script: &str, file: &str) -> Call {
        let mut args: Vec<String> = Vec::new();
        let mut env = Vec::new();
        let port = self.port.to_string();
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<String>>();
        match self.kind {
            Kind::Postgres => {
                args.extend(s(&["-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", &self.host, "-p", &port, "-U", &self.user]));
                if !self.database.is_empty() {
                    args.extend(s(&["-d", &self.database]));
                }
                // Never a password prompt: there's no terminal to type it in.
                args.push("-w".into());
                if let Some(pw) = &self.password {
                    env.push(("PGPASSWORD".to_owned(), pw.clone()));
                }
                Call::new(program, args, script.to_owned(), env)
            }
            Kind::MySql => {
                args.extend(s(&["--xml", "--batch", "-h", &self.host, "-P", &port, "-u", &self.user]));
                if !self.database.is_empty() {
                    args.push(self.database.clone());
                }
                if let Some(pw) = &self.password {
                    env.push(("MYSQL_PWD".to_owned(), pw.clone()));
                }
                Call::new(program, args, script.to_owned(), env)
            }
            Kind::SqlServer => {
                // Stop at the first error, values up to 8000 characters, no padding, columns
                // split by a character no value holds, and a row a line: line breaks and tabs
                // in values come as spaces.
                args.extend(s(&["-b", "-W", "-y", "8000", "-k", "1", "-s", SEP]));
                if !self.database.is_empty() {
                    args.extend(s(&["-d", &self.database]));
                }
                if self.user.is_empty() {
                    args.push("-E".into());
                } else {
                    args.extend(s(&["-U", &self.user]));
                    if let Some(pw) = &self.password {
                        env.push(("SQLCMDPASSWORD".to_owned(), pw.clone()));
                    }
                }
                let address = self.address.clone().unwrap_or_else(|| self.host.trim().to_owned());
                args.extend(s(&["-S", &address, "-i", file]));
                let mut call = Call::new(program, args, String::new(), env);
                call.file = Some((file.to_owned(), script.to_owned()));
                call
            }
        }
    }

    /// A LocalDB instance (`(localdb)\MSSQLLocalDB`) to start and find the pipe of first.
    pub fn localdb_instance(&self) -> Option<String> {
        let host = self.host.trim();
        let instance = host.strip_prefix("(localdb)\\").or_else(|| host.strip_prefix("(LocalDB)\\"))?;
        Some(instance.to_owned())
    }

    /// What `sqlcmd -S` takes, when it needs no working out: `tcp:host,port`, or a named
    /// instance or a pipe as given.
    pub fn plain_address(&self) -> String {
        let host = self.host.trim();
        if host.contains('\\') || host.contains(':') || self.port == 0 {
            host.to_owned()
        } else {
            format!("tcp:{host},{}", self.port)
        }
    }

    /// What the client printed, or what its failure means.
    pub fn read(&self, out: &Out) -> Result<String, String> {
        let stdout = out.stdout.replace("\r\n", "\n");
        let stderr = out.stderr.replace("\r\n", "\n");
        if self.kind == Kind::SqlServer {
            if out.ok() {
                return Ok(stdout);
            }
            // "Msg 208, Level 16, State 1, Server …, Line 1" and the message on the next line.
            let both = format!("{stdout}\n{stderr}");
            let lines: Vec<&str> = both.lines().collect();
            let error = match lines.iter().position(|l| l.starts_with("Msg ")) {
                Some(i) => {
                    let head = lines[i].split(", Server").next().unwrap_or(lines[i]);
                    let message: Vec<&str> = lines[i + 1..]
                        .iter()
                        .take_while(|l| !l.trim().is_empty() && !l.starts_with("Msg "))
                        .copied()
                        .collect();
                    format!("{} ({head})", message.join(" ").trim())
                }
                None => {
                    let e = stderr.trim();
                    if e.is_empty() { out.stopped("sqlcmd") } else { e.to_owned() }
                }
            };
            return Err(name_statement(&error, &stdout));
        }
        // The client's own notes (MariaDB's about TLS on a passwordless login) aren't errors.
        let err: String =
            stderr.lines().filter(|l| !l.starts_with("WARNING: option --ssl")).collect::<Vec<_>>().join("\n");
        if !out.ok() {
            let err = err.trim();
            return Err(if err.is_empty() { out.stopped("the client") } else { name_statement(err, &stdout) });
        }
        Ok(stdout)
    }

    /// The script that runs SQL (any number of statements), and the line each statement starts
    /// on (MySQL's errors count lines).
    pub fn run_script(&self, sql: &str) -> (String, Vec<usize>) {
        let statements = split_statements(sql);
        let mut starts = Vec::new();
        let script = match self.kind {
            Kind::Postgres => {
                let mut script = format!("\\pset format csv\n\\pset null {NULL}\n");
                for (i, s) in statements.iter().enumerate() {
                    script.push_str(&format!("\\echo {MARK} {i}\n{s};\n\\echo {ROWS} :ROW_COUNT\n"));
                }
                script
            }
            Kind::MySql => {
                let mut script = String::new();
                for s in &statements {
                    starts.push(script.lines().count() + 1);
                    script.push_str(&format!("{s};\nSELECT ROW_COUNT() AS {CHANGES};\n"));
                }
                script
            }
            Kind::SqlServer => {
                // Each statement in a batch of its own (CREATE VIEW must start one), its mark
                // before it and its count of rows after.
                let mut script = String::from("SET NOCOUNT ON;\nGO\n");
                for (i, s) in statements.iter().enumerate() {
                    // A view, procedure, function or trigger is the whole of its batch: no count.
                    let count = if whole_batch(s) {
                        format!("GO\nPRINT '{ROWS} 0';\n")
                    } else {
                        format!(";PRINT '{ROWS} ' + CAST(@@ROWCOUNT AS varchar(20));\n")
                    };
                    script.push_str(&format!("PRINT '{MARK} {i}';\nGO\n{s}\n{count}GO\n"));
                }
                script
            }
        };
        (script, starts)
    }

    /// What a run's script gave: each statement's rows, and how many it changed.
    pub fn run_read(&self, out: &Out, starts: &[usize]) -> Result<Outcome, String> {
        match self.kind {
            Kind::Postgres => Ok(parse_psql(&self.read(out)?)),
            Kind::MySql => parse_mysql(&self.read(out).map_err(|e| mysql_error(&e, starts))?),
            Kind::SqlServer => Ok(parse_sqlcmd(&self.read(out)?)),
        }
    }

    /// The script that reads the tables and views (PostgreSQL: its current schema), with their
    /// columns, keys and indexes.
    pub fn schema_script(&self) -> String {
        let (tables, columns, keys, indexes) = match self.kind {
            Kind::Postgres => (PG_TABLES, PG_COLUMNS, PG_KEYS, PG_INDEXES),
            Kind::MySql => (MY_TABLES, MY_COLUMNS, MY_KEYS, MY_INDEXES),
            Kind::SqlServer => (MS_TABLES, MS_COLUMNS, MS_KEYS, MS_INDEXES),
        };
        self.run_script(&format!("{tables};\n{columns};\n{keys};\n{indexes};")).0
    }

    pub fn schema_read(&self, out: &Out) -> Result<Vec<Table>, String> {
        let sets = self.run_read(out, &[])?.sets;
        let [t, c, k, x] = sets.as_slice() else {
            return Err("the server's catalog didn't answer as expected".to_owned());
        };
        Ok(build_schema(t, c, k, x))
    }

    /// The script that asks how the server would carry out a statement (it doesn't run it), or
    /// none for a statement that has no plan.
    pub fn explain_script(&self, statement: &str) -> Option<String> {
        if !explainable(statement) {
            return None;
        }
        Some(if self.kind == Kind::SqlServer {
            format!("SET SHOWPLAN_TEXT ON;\nGO\n{statement}\nGO\nSET SHOWPLAN_TEXT OFF;\nGO\n")
        } else {
            self.run_script(&format!("EXPLAIN {statement}")).0
        })
    }

    pub fn explain_read(&self, out: &Out) -> Result<Vec<PlanStep>, String> {
        Ok(match self.kind {
            Kind::SqlServer => ms_plan(&sqlcmd_sets(&self.read(out)?)),
            Kind::Postgres => pg_plan(&self.run_read(out, &[])?.sets.into_iter().next().unwrap_or_default()),
            Kind::MySql => mysql_plan(&self.run_read(out, &[])?.sets.into_iter().next().unwrap_or_default()),
        })
    }
}

/// The client programs to try for a kind, in order.
pub fn programs(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Postgres => &["psql"],
        Kind::MySql => &["mariadb", "mysql"],
        Kind::SqlServer => &["sqlcmd"],
    }
}

/// A LocalDB instance's pipe (`np:\\.\pipe\LOCALDB#…\tsql\query`) from `SqlLocalDB info`: on a
/// line whose label is in the system's language.
pub fn localdb_pipe(info: &str, instance: &str) -> Result<String, String> {
    info.split_whitespace()
        .find(|w| w.starts_with("np:"))
        .map(str::to_owned)
        .ok_or_else(|| format!("LocalDB has no instance named {instance} (SqlLocalDB info lists them)"))
}

/// Connections used before, newest first, without passwords, from the extension's storage.
pub fn saved(text: &str) -> Vec<Server> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        server: Vec<Server>,
    }
    toml::from_str::<File>(text).map_or_else(|_| Vec::new(), |f| f.server)
}

/// The storage's text with `server` first (and only once), without its password.
pub fn remember(saved: &mut Vec<Server>, server: &Server) -> String {
    #[derive(Serialize)]
    struct File<'a> {
        server: &'a [Server],
    }
    let mut kept = server.clone();
    kept.password = None;
    kept.address = None;
    saved.retain(|s| s.label() != kept.label());
    saved.insert(0, kept);
    saved.truncate(20);
    toml::to_string(&File { server: saved }).unwrap_or_default()
}

/// Shown for SQL's NULL, so it isn't taken for an empty text.
const NULL: &str = "__coiman_null__";
/// Printed before each statement's output.
const MARK: &str = "__coiman_statement";
/// Printed after each statement: how many rows it touched.
const ROWS: &str = "__coiman_rows";
/// The column of the extra result giving a MySQL statement's changed rows.
const CHANGES: &str = "__coiman_changes";
/// Between `sqlcmd`'s columns (the ASCII unit separator, which no text holds).
const SEP: &str = "\u{1f}";

/// Whether SQL Server wants `statement` alone in its batch (`CREATE VIEW`, `ALTER PROCEDURE`,
/// `CREATE OR ALTER FUNCTION`, triggers, schemas).
fn whole_batch(statement: &str) -> bool {
    let words: Vec<String> = statement.split_whitespace().take(4).map(str::to_ascii_lowercase).collect();
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let object = match w.as_slice() {
        ["create", "or", "alter", o, ..] | ["create", o, ..] | ["alter", o, ..] => *o,
        _ => return false,
    };
    matches!(object, "view" | "procedure" | "proc" | "function" | "trigger" | "schema" | "default" | "rule")
}

pub fn missing_client(kind: Kind) -> String {
    match kind {
        Kind::Postgres => "The psql program isn't on this computer. Install PostgreSQL's command-line tools \
                           (with the PostgreSQL installer, your package manager, or MSYS2's \
                           mingw-w64-ucrt-x86_64-postgresql) and try again."
            .to_owned(),
        Kind::MySql => "Neither the mariadb nor the mysql program is on this computer. Install MariaDB's or MySQL's \
                        command-line client and try again."
            .to_owned(),
        Kind::SqlServer => "The sqlcmd program isn't on this computer. Install Microsoft's sqlcmd (the Go version \
                            from github.com/microsoft/go-sqlcmd, or the one with SQL Server's tools), or unpack it \
                            into the config folder's tools/sqlcmd, and try again."
            .to_owned(),
    }
}

/// psql's "psql:<stdin>:17: ERROR" counts lines of the script: says which statement instead,
/// from the last mark printed.
fn name_statement(error: &str, stdout: &str) -> String {
    let error =
        error.strip_prefix("psql:<stdin>:").map_or(error, |rest| rest.split_once(": ").map_or(rest, |(_, e)| e));
    let last = stdout.lines().rev().find_map(|l| l.strip_prefix(MARK)).and_then(|n| n.trim().parse::<usize>().ok());
    match last {
        Some(i) => format!("Statement {}: {error}", i + 1),
        None => error.to_owned(),
    }
}

/// psql's output: per statement a mark, then its rows as CSV (a header first) if it returned
/// any, then the count of rows it touched.
fn parse_psql(out: &str) -> Outcome {
    let mut outcome = Outcome::default();
    let mut body = String::new();
    let mut in_statement = false;
    for line in out.split_inclusive('\n') {
        let bare = line.trim_end_matches('\n');
        if bare.starts_with(MARK) {
            in_statement = true;
            body.clear();
            continue;
        }
        if let Some(count) = bare.strip_prefix(ROWS) {
            if in_statement {
                if body.trim().is_empty() {
                    // No rows came back: a change, with how many it touched.
                    outcome.changes += count.trim().parse::<u64>().unwrap_or(0);
                } else {
                    outcome.sets.push(csv_set(&body));
                }
            }
            in_statement = false;
            body.clear();
            continue;
        }
        if in_statement {
            body.push_str(line);
        }
    }
    outcome
}

/// A CSV result: the header, then rows, NULL marked as [`NULL`].
fn csv_set(text: &str) -> ResultSet {
    let mut records = parse_csv(text).into_iter();
    let columns = records.next().unwrap_or_default();
    let mut set = ResultSet { columns, ..ResultSet::default() };
    for record in records {
        if set.rows.len() >= MAX_ROWS {
            set.more += 1;
            continue;
        }
        set.rows.push(record.into_iter().map(|f| (f != NULL).then_some(f)).collect());
    }
    set
}

/// `sqlcmd`'s output, marked as [`Server::run`] marks it: per statement, its result sets (if
/// any), then the count of rows it touched.
fn parse_sqlcmd(out: &str) -> Outcome {
    let mut outcome = Outcome::default();
    let mut body = String::new();
    let mut in_statement = false;
    for line in out.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare.starts_with(MARK) {
            in_statement = true;
            body.clear();
            continue;
        }
        if let Some(count) = bare.strip_prefix(ROWS) {
            if in_statement {
                let sets = sqlcmd_sets(&body);
                if sets.is_empty() {
                    outcome.changes += count.trim().parse::<u64>().unwrap_or(0);
                } else {
                    outcome.sets.extend(sets);
                }
            }
            in_statement = false;
            body.clear();
            continue;
        }
        if in_statement {
            body.push_str(line);
        }
    }
    outcome
}

/// The result sets in `sqlcmd -W -s <SEP>` output: a header, a line of dashes, the rows. A
/// value with a line break goes on over the next lines: a row is whole once it has all its
/// columns. `NULL` is SQL's NULL.
fn sqlcmd_sets(text: &str) -> Vec<ResultSet> {
    let lines: Vec<&str> = text.lines().collect();
    let is_dashes = |l: &str, seps: usize| {
        !l.is_empty() && l.matches(SEP).count() == seps && l.chars().all(|c| c == '-' || c.to_string() == SEP)
    };
    let mut sets = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let seps = lines[i].matches(SEP).count();
        if !is_dashes(lines[i + 1], seps) {
            i += 1;
            continue;
        }
        let mut set = ResultSet { columns: lines[i].split(SEP).map(str::to_owned).collect(), ..ResultSet::default() };
        i += 2;
        let mut row = String::new();
        while i < lines.len() {
            let line = lines[i];
            // A blank line ends the set (unless a value is still open).
            if row.is_empty() && line.trim().is_empty() {
                break;
            }
            if !row.is_empty() {
                row.push('\n');
            }
            row.push_str(line);
            i += 1;
            if row.matches(SEP).count() >= seps {
                let cells = row.split(SEP).map(|v| (v != "NULL").then(|| v.to_owned())).collect();
                if set.rows.len() >= MAX_ROWS {
                    set.more += 1;
                } else {
                    set.rows.push(cells);
                }
                row.clear();
            }
        }
        sets.push(set);
    }
    sets
}

/// SQL Server's text plan (SHOWPLAN_TEXT): the statement, then a step per line, nested by
/// where its `|--` starts.
fn ms_plan(sets: &[ResultSet]) -> Vec<PlanStep> {
    sets.iter()
        .flat_map(|s| s.rows.iter())
        .filter_map(|r| r.first().cloned().flatten())
        .filter_map(|line| {
            let at = line.find("|--")?;
            let text = line[at + 3..].trim().to_owned();
            Some(PlanStep {
                depth: at / 5,
                full_scan: text.starts_with("Table Scan")
                    || text.starts_with("Clustered Index Scan")
                    || text.starts_with("Index Scan"),
                temp_sort: text.starts_with("Sort") || text.starts_with("Hash Match"),
                detail: text,
            })
        })
        .collect()
}

/// CSV as RFC 4180 writes it: fields in double quotes may hold commas, quotes (doubled) and
/// line breaks.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        any = true;
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => record.push(std::mem::take(&mut field)),
            (false, '\n') => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
                any = false;
            }
            (false, '\r') => {}
            (false, c) => field.push(c),
        }
    }
    if any {
        record.push(field);
        records.push(record);
    }
    records
}

/// The client's "ERROR 1054 (42S22) at line 3: ..." counts lines of the script sent: says which
/// statement instead, without the copy of the statement it prints around it.
fn mysql_error(error: &str, starts: &[usize]) -> String {
    let Some(line) = error.lines().find(|l| l.starts_with("ERROR ")) else { return error.to_owned() };
    let Some((head, rest)) = line.split_once(" at line ") else { return line.to_owned() };
    let Some((n, message)) = rest.split_once(": ") else { return line.to_owned() };
    match n.trim().parse::<usize>().ok().and_then(|n| starts.iter().rposition(|&s| s <= n)) {
        Some(k) => format!("Statement {}: {head}: {message}", k + 1),
        None => line.to_owned(),
    }
}

/// The client's XML: a `<resultset>` per statement that returned rows; each statement's count
/// of changed rows comes as one more result, taken off here.
fn parse_mysql(out: &str) -> Result<Outcome, String> {
    let mut outcome = Outcome::default();
    for set in parse_xml_sets(out)? {
        if set.columns == [CHANGES] {
            let n = set.rows.first().and_then(|r| r.first().cloned().flatten()).and_then(|v| v.parse::<i64>().ok());
            // -1 after a statement that returned rows.
            outcome.changes += n.filter(|n| *n > 0).unwrap_or(0) as u64;
        } else {
            outcome.sets.push(set);
        }
    }
    Ok(outcome)
}

/// `<resultset><row><field name="a">1</field><field name="b" xsi:nil="true" /></row>...`.
pub fn parse_xml_sets(out: &str) -> Result<Vec<ResultSet>, String> {
    let mut reader = quick_xml::Reader::from_str(out);
    let mut sets = Vec::new();
    let mut set: Option<ResultSet> = None;
    let mut row: Option<Vec<Option<String>>> = None;
    let mut field: Option<(String, String, bool)> = None;
    let attr = |e: &quick_xml::events::BytesStart, key: &[u8]| -> Option<String> {
        e.attributes()
            .flatten()
            .find(|a| a.key.as_ref() == key)
            .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
            .map(|v| v.into_owned())
    };
    loop {
        match reader.read_event().map_err(|e| format!("couldn't read the client's output: {e}"))? {
            Event::Eof => break,
            Event::Start(e) if e.name().as_ref() == b"resultset" => set = Some(ResultSet::default()),
            Event::Empty(e) if e.name().as_ref() == b"resultset" => sets.push(ResultSet::default()),
            Event::End(e) if e.name().as_ref() == b"resultset" => sets.extend(set.take()),
            Event::Start(e) if e.name().as_ref() == b"row" => row = Some(Vec::new()),
            Event::End(e) if e.name().as_ref() == b"row" => {
                if let (Some(s), Some(r)) = (set.as_mut(), row.take()) {
                    if s.rows.len() >= MAX_ROWS {
                        s.more += 1;
                    } else {
                        s.rows.push(r);
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == b"field" => {
                field = Some((attr(&e, b"name").unwrap_or_default(), String::new(), false));
            }
            Event::Empty(e) if e.name().as_ref() == b"field" => {
                let name = attr(&e, b"name").unwrap_or_default();
                let nil = attr(&e, b"xsi:nil").as_deref() == Some("true");
                push_field(set.as_mut(), row.as_mut(), name, if nil { None } else { Some(String::new()) });
            }
            Event::Text(t) => {
                if let Some(f) = field.as_mut() {
                    f.1.push_str(&t.decode().map_err(|e| e.to_string())?);
                }
            }
            Event::GeneralRef(r) => {
                if let Some(f) = field.as_mut() {
                    let name = String::from_utf8_lossy(r.as_ref()).into_owned();
                    f.1.push_str(&match name.as_str() {
                        "lt" => "<".to_owned(),
                        "gt" => ">".to_owned(),
                        "amp" => "&".to_owned(),
                        "quot" => "\"".to_owned(),
                        "apos" => "'".to_owned(),
                        n => n
                            .strip_prefix("#x")
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                            .and_then(char::from_u32)
                            .map_or_else(|| format!("&{n};"), String::from),
                    });
                }
            }
            Event::End(e) if e.name().as_ref() == b"field" => {
                if let Some((name, text, _)) = field.take() {
                    push_field(set.as_mut(), row.as_mut(), name, Some(text));
                }
            }
            _ => {}
        }
    }
    Ok(sets)
}

/// A field into its row; the first row names the columns.
fn push_field(set: Option<&mut ResultSet>, row: Option<&mut Vec<Option<String>>>, name: String, value: Option<String>) {
    let (Some(set), Some(row)) = (set, row) else { return };
    if set.rows.is_empty() && set.columns.len() == row.len() {
        set.columns.push(name);
    }
    row.push(value);
}

/// Statements EXPLAIN takes.
fn explainable(statement: &str) -> bool {
    let first = statement.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    matches!(first.as_str(), "select" | "insert" | "update" | "delete" | "with" | "values" | "table" | "replace")
}

/// PostgreSQL's text plan: one line per step, indented by `->`.
fn pg_plan(set: &ResultSet) -> Vec<PlanStep> {
    set.rows
        .iter()
        .filter_map(|r| r.first().cloned().flatten())
        .map(|line| {
            let indent = line.len() - line.trim_start().len();
            let text = line.trim().trim_start_matches("->").trim().to_owned();
            let node = line.trim_start().starts_with("->") || indent == 0;
            PlanStep {
                depth: indent / 6 + usize::from(line.trim_start().starts_with("->")),
                full_scan: node && text.starts_with("Seq Scan"),
                temp_sort: node && (text.starts_with("Sort ") || text.starts_with("HashAggregate")),
                detail: text,
            }
        })
        .collect()
}

/// MySQL's EXPLAIN table: a row per table read. Type `ALL` reads all of it.
fn mysql_plan(set: &ResultSet) -> Vec<PlanStep> {
    let col = |name: &str| set.columns.iter().position(|c| c.eq_ignore_ascii_case(name));
    let get = |row: &Vec<Option<String>>, name: &str| {
        col(name).and_then(|i| row.get(i).cloned().flatten()).unwrap_or_default()
    };
    set.rows
        .iter()
        .map(|row| {
            let (table, ty, key, rows, extra) =
                (get(row, "table"), get(row, "type"), get(row, "key"), get(row, "rows"), get(row, "Extra"));
            let mut detail = format!("{} {table}", get(row, "select_type"));
            if !ty.is_empty() {
                detail.push_str(&format!(", access {ty}"));
            }
            if !key.is_empty() {
                detail.push_str(&format!(", using {key}"));
            }
            if !rows.is_empty() {
                detail.push_str(&format!(", about {rows} rows"));
            }
            if !extra.is_empty() {
                detail.push_str(&format!(" ({extra})"));
            }
            PlanStep {
                depth: 0,
                full_scan: ty == "ALL",
                temp_sort: extra.contains("Using temporary") || extra.contains("Using filesort"),
                detail: detail.trim().to_owned(),
            }
        })
        .collect()
}

/// Columns of a catalog result, by name.
fn cell<'a>(set: &'a ResultSet, row: &'a [Option<String>], name: &str) -> &'a str {
    set.columns.iter().position(|c| c == name).and_then(|i| row.get(i)).and_then(|v| v.as_deref()).unwrap_or("")
}

fn truthy(v: &str) -> bool {
    matches!(v, "t" | "true" | "1" | "YES" | "yes")
}

/// Tables from the four catalog results: tables (t, k), columns (t, c, ty, pk, nn), foreign
/// keys (t, c, rt, rc), index columns (t, i, u, a, p, ic).
fn build_schema(t: &ResultSet, c: &ResultSet, k: &ResultSet, x: &ResultSet) -> Vec<Table> {
    let mut tables: Vec<Table> = t
        .rows
        .iter()
        .map(|r| Table {
            name: cell(t, r, "t").to_owned(),
            view: cell(t, r, "k").contains("VIEW"),
            columns: Vec::new(),
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
        })
        .collect();
    for r in &c.rows {
        if let Some(table) = tables.iter_mut().find(|tb| tb.name == cell(c, r, "t")) {
            table.columns.push(Column {
                name: cell(c, r, "c").to_owned(),
                ty: cell(c, r, "ty").to_owned(),
                primary_key: truthy(cell(c, r, "pk")),
                not_null: truthy(cell(c, r, "nn")),
            });
        }
    }
    for r in &k.rows {
        if let Some(table) = tables.iter_mut().find(|tb| tb.name == cell(k, r, "t")) {
            table.foreign_keys.push(ForeignKey {
                column: cell(k, r, "c").to_owned(),
                table: cell(k, r, "rt").to_owned(),
                to: cell(k, r, "rc").to_owned(),
            });
        }
    }
    for r in &x.rows {
        let Some(table) = tables.iter_mut().find(|tb| tb.name == cell(x, r, "t")) else { continue };
        let name = cell(x, r, "i").to_owned();
        let column = match cell(x, r, "ic") {
            "" => "(expression)".to_owned(),
            c => c.to_owned(),
        };
        match table.indexes.iter_mut().find(|i| i.name == name) {
            Some(i) => i.columns.push(column),
            None => table.indexes.push(Index {
                name,
                unique: truthy(cell(x, r, "u")),
                automatic: truthy(cell(x, r, "a")),
                partial: truthy(cell(x, r, "p")),
                columns: vec![column],
            }),
        }
    }
    tables.sort_by(|a, b| a.view.cmp(&b.view).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    tables
}

const PG_TABLES: &str = "SELECT table_name AS t, table_type AS k FROM information_schema.tables \
     WHERE table_schema = current_schema() ORDER BY 1";
const PG_COLUMNS: &str = "SELECT c.table_name AS t, c.column_name AS c, c.data_type AS ty, \
     EXISTS (SELECT 1 FROM information_schema.table_constraints tc JOIN information_schema.key_column_usage u \
       ON u.constraint_name = tc.constraint_name AND u.table_schema = tc.table_schema \
       WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema \
       AND u.table_name = c.table_name AND u.column_name = c.column_name) AS pk, \
     c.is_nullable = 'NO' AS nn \
     FROM information_schema.columns c WHERE c.table_schema = current_schema() \
     ORDER BY c.table_name, c.ordinal_position";
const PG_KEYS: &str = "SELECT cl.relname AS t, a.attname AS c, cf.relname AS rt, af.attname AS rc \
     FROM pg_constraint co JOIN pg_class cl ON cl.oid = co.conrelid \
     JOIN pg_namespace n ON n.oid = cl.relnamespace JOIN pg_class cf ON cf.oid = co.confrelid \
     CROSS JOIN LATERAL unnest(co.conkey, co.confkey) AS k(a1, a2) \
     JOIN pg_attribute a ON a.attrelid = co.conrelid AND a.attnum = k.a1 \
     JOIN pg_attribute af ON af.attrelid = co.confrelid AND af.attnum = k.a2 \
     WHERE co.contype = 'f' AND n.nspname = current_schema()";
const PG_INDEXES: &str = "SELECT t.relname AS t, i.relname AS i, ix.indisunique AS u, \
     EXISTS (SELECT 1 FROM pg_constraint co WHERE co.conindid = ix.indexrelid) AS a, \
     ix.indpred IS NOT NULL AS p, a.attname AS ic \
     FROM pg_index ix JOIN pg_class t ON t.oid = ix.indrelid JOIN pg_class i ON i.oid = ix.indexrelid \
     JOIN pg_namespace n ON n.oid = t.relnamespace \
     CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord) \
     LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum \
     WHERE n.nspname = current_schema() ORDER BY t.relname, i.relname, k.ord";

const MY_TABLES: &str = "SELECT table_name AS t, table_type AS k FROM information_schema.tables \
     WHERE table_schema = DATABASE() ORDER BY 1";
const MY_COLUMNS: &str = "SELECT table_name AS t, column_name AS c, column_type AS ty, \
     column_key = 'PRI' AS pk, is_nullable = 'NO' AS nn \
     FROM information_schema.columns WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position";
const MY_KEYS: &str = "SELECT table_name AS t, column_name AS c, referenced_table_name AS rt, \
     referenced_column_name AS rc FROM information_schema.key_column_usage \
     WHERE table_schema = DATABASE() AND referenced_table_name IS NOT NULL";
const MY_INDEXES: &str = "SELECT s.table_name AS t, s.index_name AS i, s.non_unique = 0 AS u, \
     (s.index_name = 'PRIMARY' OR EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
       WHERE tc.table_schema = s.table_schema AND tc.table_name = s.table_name \
       AND tc.constraint_name = s.index_name)) AS a, 0 AS p, s.column_name AS ic \
     FROM information_schema.statistics s WHERE s.table_schema = DATABASE() \
     ORDER BY s.table_name, s.index_name, s.seq_in_index";

const MS_TABLES: &str = "SELECT TABLE_NAME AS t, TABLE_TYPE AS k FROM INFORMATION_SCHEMA.TABLES \
     WHERE TABLE_SCHEMA = SCHEMA_NAME() ORDER BY 1";
const MS_COLUMNS: &str = "SELECT c.TABLE_NAME AS t, c.COLUMN_NAME AS c, c.DATA_TYPE + CASE WHEN c.DATA_TYPE IN ('char', 'varchar', \
     'nchar', 'nvarchar', 'binary', 'varbinary') THEN COALESCE('(' + CASE \
     WHEN c.CHARACTER_MAXIMUM_LENGTH = -1 THEN 'max' ELSE CAST(c.CHARACTER_MAXIMUM_LENGTH AS varchar(10)) END \
     + ')', '') ELSE '' END AS ty, CASE WHEN EXISTS (SELECT 1 FROM INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc \
       JOIN INFORMATION_SCHEMA.KEY_COLUMN_USAGE u ON u.CONSTRAINT_NAME = tc.CONSTRAINT_NAME \
       AND u.TABLE_SCHEMA = tc.TABLE_SCHEMA WHERE tc.CONSTRAINT_TYPE = 'PRIMARY KEY' \
       AND tc.TABLE_SCHEMA = c.TABLE_SCHEMA AND u.TABLE_NAME = c.TABLE_NAME AND u.COLUMN_NAME = c.COLUMN_NAME) \
     THEN 1 ELSE 0 END AS pk, CASE WHEN c.IS_NULLABLE = 'NO' THEN 1 ELSE 0 END AS nn \
     FROM INFORMATION_SCHEMA.COLUMNS c WHERE c.TABLE_SCHEMA = SCHEMA_NAME() \
     ORDER BY c.TABLE_NAME, c.ORDINAL_POSITION";
const MS_KEYS: &str = "SELECT tp.name AS t, cp.name AS c, tr.name AS rt, cr.name AS rc \
     FROM sys.foreign_key_columns fkc JOIN sys.tables tp ON tp.object_id = fkc.parent_object_id \
     JOIN sys.columns cp ON cp.object_id = fkc.parent_object_id AND cp.column_id = fkc.parent_column_id \
     JOIN sys.tables tr ON tr.object_id = fkc.referenced_object_id \
     JOIN sys.columns cr ON cr.object_id = fkc.referenced_object_id AND cr.column_id = fkc.referenced_column_id \
     WHERE tp.schema_id = SCHEMA_ID()";
const MS_INDEXES: &str = "SELECT t.name AS t, i.name AS i, CAST(i.is_unique AS int) AS u, \
     CASE WHEN i.is_primary_key = 1 OR i.is_unique_constraint = 1 THEN 1 ELSE 0 END AS a, \
     CAST(i.has_filter AS int) AS p, c.name AS ic \
     FROM sys.indexes i JOIN sys.tables t ON t.object_id = i.object_id \
     JOIN sys.index_columns ic ON ic.object_id = i.object_id AND ic.index_id = i.index_id AND ic.is_included_column = 0 \
     JOIN sys.columns c ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
     WHERE t.schema_id = SCHEMA_ID() AND i.name IS NOT NULL ORDER BY t.name, i.name, ic.key_ordinal";

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use crate::sqlite::split_statements;

    #[test]
    fn csv_with_quotes_commas_and_lines() {
        let rows = parse_csv("id,s,\"q,x\"\n1,\"x,\"\"y\",__coiman_null__\n3,,\"multi\nline\"\n");
        assert_eq!(rows[0], ["id", "s", "q,x"]);
        assert_eq!(rows[1], ["1", "x,\"y", NULL]);
        assert_eq!(rows[2], ["3", "", "multi\nline"]);
    }

    #[test]
    fn psql_output_by_statement() {
        let out = "__coiman_statement 0\n__coiman_rows 0\n__coiman_statement 1\n__coiman_rows 3\n\
                   __coiman_statement 2\nid,s\n1,__coiman_null__\n2,\n__coiman_rows 2\n\
                   __coiman_statement 3\nq\n__coiman_rows 0\n";
        let o = parse_psql(out);
        assert_eq!(o.changes, 3);
        assert_eq!(o.sets.len(), 2);
        assert_eq!(o.sets[0].rows, [vec![Some("1".to_owned()), None], vec![Some("2".to_owned()), Some(String::new())]]);
        assert_eq!(o.sets[1].columns, ["q"]);
        assert!(o.sets[1].rows.is_empty());
    }

    #[test]
    fn mysql_xml_with_nulls_and_counts() {
        let out = r#"<?xml version="1.0"?>
<resultset statement="select ROW_COUNT() as c" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <row><field name="__coiman_changes">3</field></row>
</resultset>
<?xml version="1.0"?>
<resultset statement="select * from a" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <row><field name="id">1</field><field name="s">a &lt;b&gt; &amp; c</field></row>
  <row><field name="id">2</field><field name="s" xsi:nil="true" /></row>
  <row><field name="id">3</field><field name="s">NULL</field></row>
</resultset>
<?xml version="1.0"?>
<resultset statement="x" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <row><field name="__coiman_changes">-1</field></row>
</resultset>"#;
        let o = parse_mysql(out).unwrap();
        assert_eq!(o.changes, 3);
        assert_eq!(o.sets.len(), 1);
        assert_eq!(o.sets[0].columns, ["id", "s"]);
        assert_eq!(o.sets[0].rows[0][1].as_deref(), Some("a <b> & c"));
        assert_eq!(o.sets[0].rows[1][1], None);
        assert_eq!(o.sets[0].rows[2][1].as_deref(), Some("NULL"));
    }

    #[test]
    fn sqlcmd_output_by_statement() {
        let u = SEP;
        let out = format!(
            "__coiman_statement 0\n__coiman_rows 2\n__coiman_statement 1\nid{u}name{u}note\n--{u}----{u}----\n\
             1{u}a,b{u}NULL\n2{u}two\nlines{u}x\n\n__coiman_rows 2\n__coiman_statement 2\nn\n-\n\n__coiman_rows 0\n"
        );
        let o = parse_sqlcmd(&out);
        assert_eq!(o.changes, 2);
        assert_eq!(o.sets.len(), 2);
        assert_eq!(o.sets[0].columns, ["id", "name", "note"]);
        assert_eq!(
            o.sets[0].rows,
            [
                vec![Some("1".to_owned()), Some("a,b".to_owned()), None],
                vec![Some("2".to_owned()), Some("two\nlines".to_owned()), Some("x".to_owned())]
            ]
        );
        assert!(o.sets[1].rows.is_empty() && o.sets[1].columns == ["n"]);
        let plan = ms_plan(&[ResultSet {
            columns: vec!["StmtText".into()],
            rows: vec![
                vec![Some("SELECT * FROM t WHERE a = 1".into())],
                vec![Some("  |--Sort(ORDER BY:([a] ASC))".into())],
                vec![Some("       |--Table Scan(OBJECT:([t]))".into())],
            ],
            ..ResultSet::default()
        }]);
        assert_eq!(plan.len(), 2);
        assert!(plan[0].temp_sort && plan[1].full_scan && plan[1].depth == plan[0].depth + 1);
        assert_eq!(Kind::SqlServer.quote_name("a]b"), "[a]]b]");
    }

    #[test]
    fn mysql_errors_name_the_statement() {
        let e = "--------------
SELECT nope
--------------

ERROR 1054 (42S22) at line 3: Unknown column 'nope' in 'SELECT'";
        assert_eq!(mysql_error(e, &[1, 3]), "Statement 2: ERROR 1054 (42S22): Unknown column 'nope' in 'SELECT'");
    }

    #[test]
    fn statement_named_in_errors() {
        let e = name_statement(
            "psql:<stdin>:17: ERROR:  column \"nope\" does not exist",
            "__coiman_statement 0\n__coiman_rows 0\n__coiman_statement 1\n",
        );
        assert_eq!(e, "Statement 2: ERROR:  column \"nope\" does not exist");
    }
}
