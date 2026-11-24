//! Redis: the keys of a Redis server (or Valkey, Memurai, KeyDB), what each holds, its time to
//! live, and a console, in a panel. It runs `redis-cli` as an approved program; a password goes
//! to it in its environment for that run only and is never saved.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod redis;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobResult, JobSpec, Row, Span, Table as Grid,
};
use redis::{Key, Reply, Server};

/// A program to run: what the extension starts as a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: String,
    /// Environment variables for this run only (a password).
    pub env: Vec<(String, String)>,
}

impl Call {
    pub fn new(program: &str, args: Vec<String>, stdin: String, env: Vec<(String, String)>) -> Self {
        Self { program: program.to_owned(), args, stdin, env }
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

    /// "redis-cli stopped (exit 1)" when it said nothing.
    pub fn stopped(&self, program: &str) -> String {
        match self.code {
            Some(c) => format!("{program} stopped (exit {c})"),
            None => format!("{program} was stopped"),
        }
    }
}

const RED: u32 = 0xE5484D;
const DIM: u32 = 0x8B949E;
const ACCENT: u32 = 0x4C9AFF;

/// Keys one Scan gathers before it stops (Scan more goes on).
const SCAN_KEYS: usize = 500;
/// SCAN rounds one Scan makes at most: a round may find few keys.
const SCAN_ROUNDS: usize = 20;
/// Console lines kept.
const LOG: usize = 50;
/// The clients, in the order tried: Redis's, Valkey's, Memurai's.
const CLIENTS: [&str; 3] = ["redis-cli", "valkey-cli", "memurai-cli"];

/// A key's value as a table: its headers and rows.
type Value = (Vec<&'static str>, Vec<Vec<String>>);

/// What a running client is for.
enum Then {
    /// The first call after Connect: its error says the server can't be reached.
    Keyspace,
    Scan {
        more: bool,
        round: usize,
        keys: Vec<Key>,
        pattern: String,
    },
    Value(Key),
    Console(String),
    /// A change; the message shown when it's done.
    Change(String),
}

#[derive(Default)]
struct State {
    server: Option<Server>,
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    status: Option<String>,
    form: Option<Server>,
    saved: Vec<Server>,
    keyspace: Vec<(u32, u64)>,
    pattern: String,
    cursor: String,
    keys: Vec<Key>,
    selected: Option<Key>,
    value: Option<Result<Value, String>>,
    edit: String,
    confirm_delete: bool,
    log: Vec<(String, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

// ----- running the client -------------------------------------------------------------------

/// Starts a call with the first client that's here.
fn start(call: Call, then: Then, busy: &str) -> Result<(), String> {
    for name in CLIENTS {
        let spec = JobSpec {
            program: name.to_owned(),
            args: call.args.clone(),
            stdin: call.stdin.clone(),
            env: call.env.clone(),
            in_own_folder: false,
        };
        match host::start_job_with(&spec) {
            Ok(id) => {
                with(|s| {
                    s.jobs.insert(id, then);
                    s.busy = Some(busy.to_owned());
                    s.error = None;
                    s.status = None;
                });
                return Ok(());
            }
            // Not here: the next name.
            Err(e) if e.contains("isn't on PATH") => {}
            Err(e) => return Err(e),
        }
    }
    Err(redis::MISSING.to_owned())
}

fn server() -> Result<Server, String> {
    with(|s| s.server.clone()).ok_or_else(|| "Connect to a server first".to_owned())
}

fn scan(more: bool) -> Result<(), String> {
    let s = server()?;
    let (cursor, pattern) = with(|st| {
        let cursor = if more { st.cursor.clone() } else { "0".to_owned() };
        (cursor, st.pattern.clone())
    });
    let call = s.scan_call(&cursor, &pattern, SCAN_KEYS);
    start(call, Then::Scan { more, round: 1, keys: Vec::new(), pattern }, "Scanning…")
}

fn select(key: Key) -> Result<(), String> {
    let s = server()?;
    with(|st| {
        st.selected = Some(key.clone());
        st.value = None;
        st.confirm_delete = false;
    });
    match s.value_call(&key) {
        Ok(call) => start(call, Then::Value(key), "Reading…"),
        Err(e) => {
            with(|st| st.value = Some(Err(e)));
            Ok(())
        }
    }
}

fn change(words: Vec<String>, done: String) -> Result<(), String> {
    let s = server()?;
    start(s.call(&words), Then::Change(done), "Working…")
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let out = Out { code: job.exit_code, stdout: job.stdout, stderr: job.stderr };
    if with(|s| s.jobs.is_empty()) {
        with(|s| s.busy = None);
    }
    let result: Result<(), String> = match then {
        Then::Keyspace => match server().and_then(|s| s.keyspace_read(&out)) {
            Ok(k) => {
                with(|s| s.keyspace = k);
                Ok(())
            }
            Err(e) => {
                with(|s| {
                    s.server = None;
                    s.jobs.clear();
                    s.busy = None;
                });
                Err(e)
            }
        },
        Then::Scan { more, round, mut keys, pattern } => match server().and_then(|s| s.scan_read(&out)) {
            Ok((cursor, found)) => {
                keys.extend(found);
                if cursor != "0" && keys.len() < SCAN_KEYS && round < SCAN_ROUNDS {
                    // Another round from where this one stopped.
                    match server() {
                        Ok(s) => start(
                            s.scan_call(&cursor, &pattern, SCAN_KEYS),
                            Then::Scan { more, round: round + 1, keys, pattern },
                            "Scanning…",
                        ),
                        Err(e) => Err(e),
                    }
                } else {
                    with(|s| {
                        if !more {
                            s.keys.clear();
                        }
                        s.keys.extend(keys);
                        s.keys.sort_by(|a, b| a.name.cmp(&b.name));
                        s.keys.dedup_by(|a, b| a.name == b.name);
                        s.cursor = cursor;
                    });
                    Ok(())
                }
            }
            Err(e) => Err(e),
        },
        Then::Value(key) => {
            let value = server().and_then(|s| s.value_read(&key, &out));
            with(|s| {
                if s.selected.as_ref().is_some_and(|k| k.name == key.name) {
                    if let Ok((_, rows)) = &value
                        && key.kind == "string"
                    {
                        s.edit = rows.first().and_then(|r| r.first()).cloned().unwrap_or_default();
                    }
                    s.value = Some(value);
                }
            });
            Ok(())
        }
        Then::Console(line) => {
            let shown = match redis::read(&out) {
                Ok(r) => r.show(),
                Err(e) => format!("(error) {e}"),
            };
            with(|s| {
                s.log.push((line, shown));
                let over = s.log.len().saturating_sub(LOG);
                s.log.drain(..over);
            });
            Ok(())
        }
        Then::Change(done) => match redis::read(&out) {
            Ok(Reply::Error(e)) => Err(e),
            Ok(_) => {
                let key = with(|s| s.selected.clone());
                let scanned = scan(false);
                let read = match key {
                    Some(k) => scanned.and_then(|()| select(k)),
                    None => scanned,
                };
                // After the reads start: starting one clears what was said.
                with(|s| s.status = Some(done));
                read
            }
            Err(e) => Err(e),
        },
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
        match &s.server {
            None => connect_ui(s, &mut b),
            Some(server) => {
                b.push(Block::Spans(vec![Span {
                    text: server.title(),
                    color: None,
                    bold: true,
                    monospace: false,
                }]));
                b.push(bar(vec![button("Disconnect", "disconnect", "")]));
                status_ui(s, &mut b);
                db_ui(s, server, &mut b);
                keys_ui(s, &mut b);
                key_ui(s, &mut b);
                console_ui(s, &mut b);
            }
        }
        b
    });
    host::set_panel("redis", &blocks);
}

fn status_ui(s: &State, b: &mut Vec<Block>) {
    if let Some(busy) = &s.busy {
        b.push(Block::Spans(vec![span(busy, Some(DIM))]));
    }
    if let Some(e) = &s.error {
        b.push(Block::Spans(vec![span(e, Some(RED))]));
    }
    if let Some(done) = &s.status {
        b.push(Block::Spans(vec![span(done, Some(DIM))]));
    }
}

fn connect_ui(s: &State, b: &mut Vec<Block>) {
    let f = s.form.clone().unwrap_or_default();
    b.push(Block::Heading("Connect to Redis".into()));
    status_ui(s, b);
    let line = || field("", "", FieldKind::LineBreak, "", "");
    b.push(Block::Form(Form {
        id: "connect".into(),
        fields: vec![
            field("host", "Host", FieldKind::Text, &f.host, "127.0.0.1"),
            line(),
            field("port", "Port", FieldKind::Text, &f.port.to_string(), "6379"),
            line(),
            field("db", "Database", FieldKind::Text, &f.db.to_string(), "0"),
            line(),
            field("user", "User", FieldKind::Text, &f.user, "empty: the default user"),
            line(),
            field("name", "Name", FieldKind::Text, &f.name, "optional: Cache, staging"),
            line(),
            field("password", "Password", FieldKind::Password, "", "kept only while open"),
            line(),
            field("tls", "TLS", FieldKind::Check, if f.tls { "true" } else { "false" }, ""),
        ],
        buttons: vec!["Connect".into()],
    }));
    b.push(Block::Spans(vec![span(
        "Runs through redis-cli: the password goes to it for each run and is never saved.",
        Some(DIM),
    )]));
    if !s.saved.is_empty() {
        b.push(Block::Spans(vec![Span { text: "Recent".into(), color: None, bold: true, monospace: false }]));
        let rows = s
            .saved
            .iter()
            .enumerate()
            .map(|(i, v)| Row {
                cells: vec![span(v.title(), None), span(v.label(), Some(DIM))],
                buttons: vec![button("Use", "saved-use", &i.to_string()), button("Forget", "saved-forget", &i.to_string())],
            })
            .collect();
        b.push(Block::Rows(rows));
    }
}

fn db_ui(s: &State, server: &Server, b: &mut Vec<Block>) {
    let name = |n: u32| {
        let keys = s.keyspace.iter().find(|(d, _)| *d == n).map_or(String::new(), |(_, k)| format!("  {k} keys"));
        format!("db {n}{keys}")
    };
    let mut db = field("db", "", FieldKind::Choice, &name(server.db), "");
    db.options = (0..16).map(name).collect();
    b.push(Block::Form(Form {
        id: "db".into(),
        fields: vec![db, field("use", "Use", FieldKind::Button, "", "")],
        buttons: vec![],
    }));
}

fn keys_ui(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Form(Form {
        id: "scan".into(),
        fields: vec![field("pattern", "", FieldKind::Text, &s.pattern, "Pattern: user:*")],
        buttons: vec!["Scan".into()],
    }));
    let more = s.cursor != "0" && !s.cursor.is_empty();
    let mut counted = vec![Row {
        cells: vec![span(format!("{} keys{}", s.keys.len(), if more { ", more to scan" } else { "" }), Some(DIM))],
        buttons: if more { vec![button("Scan more", "scan-more", "")] } else { vec![] },
    }];
    counted.extend(s.keys.iter().enumerate().map(|(i, k)| {
        let chosen = s.selected.as_ref().is_some_and(|c| c.name == k.name);
        let mut cells = vec![
            mono(&k.kind, Some(ACCENT)),
            Span { text: k.name.clone(), color: None, bold: chosen, monospace: true },
        ];
        let ttl = redis::human_ttl(k.ttl_ms);
        if !ttl.is_empty() {
            cells.push(span(format!("⏱ {ttl}"), Some(DIM)));
        }
        Row { cells, buttons: vec![button("Open", "select", &i.to_string())] }
    }));
    b.push(Block::Rows(counted));
}

fn key_ui(s: &State, b: &mut Vec<Block>) {
    let Some(key) = &s.selected else { return };
    b.push(Block::Separator);
    let unit = match key.kind.as_str() {
        "string" => "characters",
        "hash" => "fields",
        "set" | "zset" => "members",
        "stream" => "entries",
        _ => "items",
    };
    b.push(Block::Spans(vec![
        Span { text: key.name.clone(), color: None, bold: true, monospace: true },
        span(format!("  {} · {} {unit}", key.kind, key.size.max(0)), Some(DIM)),
    ]));
    let ttl = if key.ttl_ms >= 0 { (key.ttl_ms / 1000).to_string() } else { String::new() };
    let mut fields = vec![field("ttl", "Expires in (s)", FieldKind::Text, &ttl, "never")];
    if key.ttl_ms >= 0 {
        fields.push(field("persist", "Never", FieldKind::Button, "", ""));
    }
    b.push(Block::Form(Form { id: "ttl".into(), fields, buttons: vec!["Set".into()] }));
    if s.confirm_delete {
        b.push(Block::Rows(vec![Row {
            cells: vec![span("Delete this key?", Some(RED))],
            buttons: vec![button("Delete", "delete", ""), button("Keep", "keep", "")],
        }]));
    } else {
        b.push(bar(vec![button("Delete…", "delete-ask", "")]));
    }
    match &s.value {
        None => b.push(Block::Spans(vec![span("Reading…", Some(DIM))])),
        Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
        Some(Ok(_)) if key.kind == "string" => b.push(Block::Form(Form {
            id: "value".into(),
            fields: vec![field("value", "", FieldKind::Multiline, &s.edit, "")],
            buttons: vec!["Save".into()],
        })),
        Some(Ok((headers, rows))) => {
            if rows.len() >= redis::SHOWN {
                b.push(Block::Spans(vec![span(
                    format!("The first {} items; the console shows the rest.", redis::SHOWN),
                    Some(DIM),
                )]));
            }
            b.push(Block::Table(Grid {
                headers: headers.iter().map(|h| (*h).to_owned()).collect(),
                rows: rows.clone(),
            }));
        }
    }
}

fn console_ui(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Separator);
    if !s.log.is_empty() {
        let text: Vec<String> = s.log.iter().map(|(line, out)| format!("> {line}\n{out}")).collect();
        b.push(Block::Code(text.join("\n")));
    }
    b.push(Block::Form(Form {
        id: "console".into(),
        fields: vec![field("line", "", FieldKind::Text, "", "A command: HGETALL user:1, INFO memory, …")],
        buttons: vec!["Run".into()],
    }));
}

// ----- what the user does ---------------------------------------------------------------------

fn save_saved(s: &State) {
    #[derive(serde::Serialize)]
    struct File<'a> {
        server: &'a [Server],
    }
    let _ = host::storage_write(&toml::to_string(&File { server: &s.saved }).unwrap_or_default());
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let index = arg.parse::<usize>().ok();
    let result = match id {
        "show" => {
            let saved = redis::saved(&host::storage_read());
            with(|s| {
                if s.server.is_none() && s.form.is_none() {
                    s.form = saved.first().cloned();
                }
                s.saved = saved;
            });
            Ok(())
        }
        "disconnect" => {
            with(|s| {
                s.server = None;
                s.keys.clear();
                s.selected = None;
                s.value = None;
                s.keyspace.clear();
                s.cursor.clear();
                s.error = None;
                s.status = None;
            });
            Ok(())
        }
        "scan-more" => scan(true),
        "select" => match with(|s| index.and_then(|i| s.keys.get(i)).cloned()) {
            Some(k) => select(k),
            None => Ok(()),
        },
        "delete-ask" => {
            with(|s| s.confirm_delete = true);
            Ok(())
        }
        "keep" => {
            with(|s| s.confirm_delete = false);
            Ok(())
        }
        "delete" => match with(|s| s.selected.take()) {
            Some(key) => {
                with(|s| {
                    s.value = None;
                    s.confirm_delete = false;
                });
                change(vec!["DEL".into(), key.name.clone()], format!("Deleted {}", key.name))
            }
            None => Ok(()),
        },
        "saved-use" => {
            with(|s| {
                if let Some(v) = index.and_then(|i| s.saved.get(i)).cloned() {
                    s.form = Some(v);
                }
            });
            Ok(())
        }
        "saved-forget" => {
            with(|s| {
                if let Some(i) = index.filter(|i| *i < s.saved.len()) {
                    s.saved.remove(i);
                    save_saved(s);
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

fn connect(server: Server) -> Result<(), String> {
    if server.host.is_empty() {
        return Err("Name the host (127.0.0.1, a name)".into());
    }
    with(|s| {
        let text = redis::remember(&mut s.saved, &server);
        let _ = host::storage_write(&text);
        let mut shown = server.clone();
        shown.password = None;
        s.form = Some(shown);
        s.server = Some(server.clone());
        s.keys.clear();
        s.selected = None;
        s.value = None;
        s.cursor.clear();
        s.error = None;
    });
    start(server.keyspace_call(), Then::Keyspace, "Connecting…")?;
    scan(false)
}

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    let result = match f.form.as_str() {
        "connect" => match get("port").trim().parse::<u16>() {
            Err(_) => Err("The port is a number, like 6379".to_owned()),
            Ok(port) => {
                let password = get("password");
                connect(Server {
                    name: get("name").trim().to_owned(),
                    host: get("host").trim().to_owned(),
                    port,
                    db: get("db").trim().parse().unwrap_or(0),
                    user: get("user").trim().to_owned(),
                    tls: get("tls") == "true",
                    password: (!password.is_empty()).then_some(password),
                })
            }
        },
        "db" => {
            let db = get("db").strip_prefix("db ").and_then(|r| r.split_whitespace().next()?.parse::<u32>().ok());
            match db {
                Some(db) => {
                    with(|s| {
                        if let Some(server) = s.server.as_mut() {
                            server.db = db;
                        }
                        if let Some(form) = s.form.as_mut() {
                            form.db = db;
                        }
                        s.selected = None;
                        s.value = None;
                    });
                    scan(false)
                }
                None => Ok(()),
            }
        }
        "scan" => {
            with(|s| s.pattern = get("pattern"));
            scan(false)
        }
        "ttl" => match with(|s| s.selected.clone()) {
            None => Ok(()),
            Some(key) if f.button == "persist" => {
                change(vec!["PERSIST".into(), key.name.clone()], format!("{} no longer expires", key.name))
            }
            Some(key) => match get("ttl").trim().parse::<u64>() {
                Ok(seconds) => change(
                    vec!["EXPIRE".into(), key.name.clone(), seconds.to_string()],
                    format!("{} expires in {seconds} s", key.name),
                ),
                Err(_) => Err("The time to live is a number of seconds".to_owned()),
            },
        },
        "value" => match with(|s| s.selected.clone()) {
            Some(key) => {
                let value = get("value");
                with(|s| s.edit = value.clone());
                let mut words = vec!["SET".to_owned(), key.name.clone(), value];
                // SET drops the time to live: keep it.
                if key.ttl_ms > 0 {
                    words.extend(["PX".to_owned(), key.ttl_ms.to_string()]);
                }
                change(words, format!("Saved {}", key.name))
            }
            None => Ok(()),
        },
        "console" => {
            let line = get("line");
            match redis::split_words(&line) {
                Ok(words) if !words.is_empty() => {
                    server().and_then(|s| start(s.call(&words), Then::Console(line), "Running…"))
                }
                Ok(_) => Ok(()),
                Err(e) => {
                    with(|s| s.log.push((line, format!("(error) {e}"))));
                    Ok(())
                }
            }
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    }
    render();
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
            _ => {}
        }
    }
}

export!(Plugin);
