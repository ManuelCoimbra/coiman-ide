//! MongoDB: a server's databases and collections, documents found by a filter (read, edited,
//! added, deleted as Extended JSON), and a script box, in a panel. It runs `mongosh` as an
//! approved program; a password goes to it for that run only and is never saved.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod mongo;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobResult, JobSpec, Row, Span, Table as Grid,
};
use mongo::Server;
use serde_json::Value;

/// A program to run: what the extension starts as a job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub program: String,
    pub args: Vec<String>,
    pub stdin: String,
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

    /// "mongosh stopped (exit 1)" when it said nothing.
    pub fn stopped(&self, program: &str) -> String {
        match self.code {
            Some(c) => format!("{program} stopped (exit {c})"),
            None => format!("{program} was stopped"),
        }
    }
}

const RED: u32 = 0xE5484D;
const DIM: u32 = 0x8B949E;
/// Between a database's name and a collection's in a button's argument: neither may hold it.
const SEP: char = '\u{1f}';

/// What a running client is for.
enum Then {
    /// The first call after Connect: its error says the server can't be reached.
    Databases,
    Collections(String),
    Found,
    Script,
    /// A change; the message shown when it's done ({n}: the count it answered).
    Change(String),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Documents,
    Script,
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
    databases: Vec<(String, u64)>,
    collections: HashMap<String, Vec<(String, u64, bool)>>,
    expanded: Vec<String>,
    /// (database, collection)
    chosen: Option<(String, String)>,
    tab: Tab,
    filter: String,
    sort: String,
    projection: String,
    skip: usize,
    docs: Vec<Value>,
    count: u64,
    /// The document open for editing (its index; None: a new one), and its text.
    editing: Option<(Option<usize>, String)>,
    confirm_delete: bool,
    script: Option<String>,
    script_out: Option<Result<String, String>>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

// ----- running the client -------------------------------------------------------------------

fn start(call: Call, then: Then, busy: &str) -> Result<(), String> {
    let spec =
        JobSpec { program: call.program, args: call.args, stdin: call.stdin, env: call.env, in_own_folder: false };
    match host::start_job_with(&spec) {
        Ok(id) => {
            with(|s| {
                s.jobs.insert(id, then);
                s.busy = Some(busy.to_owned());
                s.error = None;
                s.status = None;
            });
            Ok(())
        }
        Err(e) if e.contains("isn't on PATH") => Err(mongo::MISSING.to_owned()),
        Err(e) => Err(e),
    }
}

fn server() -> Result<Server, String> {
    with(|s| s.server.clone()).ok_or_else(|| "Connect to a server first".to_owned())
}

fn find() -> Result<(), String> {
    let s = server()?;
    let Some((db, coll, filter, sort, projection, skip)) = with(|st| {
        let (db, coll) = st.chosen.clone()?;
        Some((db, coll, st.filter.clone(), st.sort.clone(), st.projection.clone(), st.skip))
    }) else {
        return Ok(());
    };
    start(s.find_call(&db, &coll, &filter, &sort, &projection, skip), Then::Found, "Finding…")
}

fn read_collections(db: String) -> Result<(), String> {
    let s = server()?;
    start(s.collections_call(&db), Then::Collections(db), "Reading…")
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let out = Out { code: job.exit_code, stdout: job.stdout, stderr: job.stderr };
    if with(|s| s.jobs.is_empty()) {
        with(|s| s.busy = None);
    }
    let result: Result<(), String> = match then {
        Then::Databases => match mongo::databases_read(&out) {
            Ok(d) => {
                with(|s| s.databases = d);
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
        Then::Collections(db) => mongo::collections_read(&out).map(|c| {
            with(|s| {
                s.collections.insert(db, c);
            })
        }),
        Then::Found => mongo::find_read(&out).map(|(docs, count)| {
            with(|s| {
                s.docs = docs;
                s.count = count;
            })
        }),
        Then::Script => {
            let shown = mongo::read(&out).map(|v| mongo::pretty(&v));
            with(|s| s.script_out = Some(shown));
            Ok(())
        }
        Then::Change(done) => match mongo::count_read(&out) {
            Ok(n) => {
                with(|s| {
                    s.editing = None;
                    s.confirm_delete = false;
                });
                // The documents, and the tree's count of them, again.
                let db = with(|s| s.chosen.clone().map(|(d, _)| d));
                let read = find().and_then(|()| db.map_or(Ok(()), read_collections));
                // After the reads start: starting one clears what was said.
                with(|s| s.status = Some(done.replace("{n}", &n.to_string())));
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

fn bold(text: impl Into<String>) -> Span {
    Span { text: text.into(), color: None, bold: true, monospace: false }
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

fn line() -> FormField {
    field("", "", FieldKind::LineBreak, "", "")
}

/// Bytes as "8.0 KB", "1.2 MB".
fn human(bytes: u64) -> String {
    let mut n = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 {
            return if unit == "B" { format!("{bytes} B") } else { format!("{n:.1} {unit}") };
        }
        n /= 1024.0;
    }
    format!("{n:.1} TB")
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        match &s.server {
            None => connect_ui(s, &mut b),
            Some(server) => {
                b.push(Block::Spans(vec![bold(server.title())]));
                b.push(bar(vec![button("Refresh", "refresh", ""), button("Disconnect", "disconnect", "")]));
                status_ui(s, &mut b);
                tree_ui(s, &mut b);
                b.push(Block::Separator);
                let mut tabs = vec![button("Documents", "tab", "documents"), button("Script", "tab", "script")];
                if s.tab == Tab::Script {
                    tabs.reverse();
                }
                let mut cells = vec![bold(if s.tab == Tab::Documents { "Documents" } else { "Script" })];
                if let Some((db, coll)) = &s.chosen {
                    cells.push(span(format!("  {db}.{coll}"), Some(DIM)));
                }
                b.push(Block::Rows(vec![Row { cells, buttons: vec![tabs.pop().expect("two tabs")] }]));
                match s.tab {
                    Tab::Documents => documents_ui(s, &mut b),
                    Tab::Script => script_ui(s, &mut b),
                }
            }
        }
        b
    });
    host::set_panel("mongodb", &blocks);
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
    b.push(Block::Heading("Connect to MongoDB".into()));
    status_ui(s, b);
    b.push(Block::Form(Form {
        id: "connect".into(),
        fields: vec![
            field("uri", "Address", FieldKind::Text, &f.uri, "mongodb://127.0.0.1:27017"),
            line(),
            field("user", "User", FieldKind::Text, &f.user, "empty: none"),
            line(),
            field("auth_db", "User's database", FieldKind::Text, &f.auth_db, "admin"),
            line(),
            field("name", "Name", FieldKind::Text, &f.name, "optional: Shop, staging"),
            line(),
            field("password", "Password", FieldKind::Password, "", "kept only while open"),
        ],
        buttons: vec!["Connect".into()],
    }));
    b.push(Block::Spans(vec![span(
        "Runs through mongosh: the password is given to it for each run and never saved.",
        Some(DIM),
    )]));
    if !s.saved.is_empty() {
        b.push(Block::Spans(vec![bold("Recent")]));
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

fn tree_ui(s: &State, b: &mut Vec<Block>) {
    let mut rows = Vec::new();
    for (db, size) in &s.databases {
        let open = s.expanded.contains(db);
        rows.push(Row {
            cells: vec![bold(format!("{} {db}", if open { "▾" } else { "▸" })), span(human(*size), Some(DIM))],
            buttons: vec![button(if open { "Hide" } else { "Show" }, "expand", db)],
        });
        if !open {
            continue;
        }
        match s.collections.get(db) {
            None => rows.push(Row { cells: vec![span("    …", Some(DIM))], buttons: vec![] }),
            Some(list) if list.is_empty() => {
                rows.push(Row { cells: vec![span("    no collections", Some(DIM))], buttons: vec![] })
            }
            Some(list) => {
                for (name, count, view) in list {
                    let chosen = s.chosen.as_ref().is_some_and(|(d, c)| d == db && c == name);
                    let label = if *view { format!("    {name}  (view)") } else { format!("    {name}") };
                    let mut cells = vec![Span { text: label, color: None, bold: chosen, monospace: false }];
                    if !view {
                        cells.push(span(count.to_string(), Some(DIM)));
                    }
                    rows.push(Row { cells, buttons: vec![button("Open", "choose", &format!("{db}{SEP}{name}"))] });
                }
            }
        }
    }
    if !rows.is_empty() {
        b.push(Block::Rows(rows));
    }
}

fn documents_ui(s: &State, b: &mut Vec<Block>) {
    if s.chosen.is_none() {
        b.push(Block::Spans(vec![span("Choose a collection.", Some(DIM))]));
        return;
    }
    b.push(Block::Form(Form {
        id: "find".into(),
        fields: vec![
            field("filter", "Filter", FieldKind::Text, &s.filter, "{ price: { $gt: 2 } }"),
            line(),
            field("sort", "Sort", FieldKind::Text, &s.sort, "{ price: -1 }"),
            line(),
            field("projection", "Fields", FieldKind::Text, &s.projection, "{ name: 1, price: 1 }"),
        ],
        buttons: vec!["Find".into()],
    }));
    let shown = s.docs.len();
    let counted = if shown > 0 {
        format!("{}–{} of {}", s.skip + 1, s.skip + shown, s.count)
    } else {
        format!("{} documents", s.count)
    };
    let mut buttons = vec![button("Add a document", "add", "")];
    if s.skip > 0 {
        buttons.push(button("Previous", "page", &s.skip.saturating_sub(mongo::LIMIT).to_string()));
    }
    if (s.skip + shown) < s.count as usize {
        buttons.push(button("Next", "page", &(s.skip + mongo::LIMIT).to_string()));
    }
    b.push(Block::Rows(vec![Row { cells: vec![span(counted, Some(DIM))], buttons }]));
    if let Some((index, text)) = &s.editing {
        b.push(Block::Separator);
        b.push(Block::Spans(vec![span(
            if index.is_some() {
                "Edit the document (Extended JSON); its _id says which it replaces."
            } else {
                "A new document (Extended JSON)."
            },
            Some(DIM),
        )]));
        b.push(Block::Form(Form {
            id: "edit".into(),
            fields: vec![field("doc", "", FieldKind::Multiline, text, "")],
            buttons: vec![if index.is_some() { "Save".into() } else { "Add".into() }, "Close".into()],
        }));
        if index.is_some() {
            if s.confirm_delete {
                b.push(Block::Rows(vec![Row {
                    cells: vec![span("Delete this document?", Some(RED))],
                    buttons: vec![button("Delete", "delete", ""), button("Keep", "keep", "")],
                }]));
            } else {
                b.push(bar(vec![button("Delete…", "delete-ask", "")]));
            }
        }
        b.push(Block::Separator);
    }
    if s.docs.is_empty() {
        return;
    }
    let columns = mongo::columns(&s.docs);
    let rows: Vec<Vec<String>> =
        s.docs.iter().map(|d| columns.iter().map(|c| mongo::cell(d.get(c)).unwrap_or_default()).collect()).collect();
    b.push(Block::Table(Grid { headers: columns.clone(), rows: rows.clone() }));
    // A row per document to open it, by its first cell (the _id).
    b.push(Block::Spans(vec![bold("Open a document")]));
    let open = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let first = r.first().cloned().unwrap_or_default();
            let rest: Vec<&str> = r.iter().skip(1).take(3).map(String::as_str).filter(|c| !c.is_empty()).collect();
            Row {
                cells: vec![mono(first, None), span(rest.join(" · ").chars().take(80).collect::<String>(), Some(DIM))],
                buttons: vec![button("Open", "open", &i.to_string())],
            }
        })
        .collect();
    b.push(Block::Rows(open));
}

fn script_ui(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Spans(vec![span(
        "A mongosh script; db is the chosen database. What its last line is shows below.",
        Some(DIM),
    )]));
    b.push(Block::Form(Form {
        id: "script".into(),
        fields: vec![field(
            "script",
            "",
            FieldKind::Multiline,
            s.script.as_deref().unwrap_or("db.getCollectionNames()"),
            "",
        )],
        buttons: vec!["Run".into()],
    }));
    match &s.script_out {
        Some(Ok(out)) => b.push(Block::Code(out.clone())),
        Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
        None => {}
    }
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
            let saved = mongo::saved(&host::storage_read());
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
                s.databases.clear();
                s.collections.clear();
                s.chosen = None;
                s.docs.clear();
                s.editing = None;
                s.error = None;
                s.status = None;
            });
            Ok(())
        }
        "refresh" => {
            let expanded = with(|s| {
                s.collections.clear();
                s.expanded.clone()
            });
            server().and_then(|srv| {
                start(srv.databases_call(), Then::Databases, "Reading…")?;
                expanded.into_iter().try_for_each(read_collections)
            })
        }
        "expand" => {
            let fetch = with(|s| {
                if let Some(i) = s.expanded.iter().position(|d| d == arg) {
                    s.expanded.remove(i);
                    false
                } else {
                    s.expanded.push(arg.to_owned());
                    !s.collections.contains_key(arg)
                }
            });
            if fetch { read_collections(arg.to_owned()) } else { Ok(()) }
        }
        "choose" => match arg.split_once(SEP) {
            Some((db, coll)) => {
                with(|s| {
                    s.chosen = Some((db.to_owned(), coll.to_owned()));
                    s.tab = Tab::Documents;
                    s.skip = 0;
                    s.editing = None;
                    s.docs.clear();
                    s.count = 0;
                });
                find()
            }
            None => Ok(()),
        },
        "tab" => {
            with(|s| s.tab = if arg == "script" { Tab::Script } else { Tab::Documents });
            Ok(())
        }
        "page" => {
            with(|s| s.skip = index.unwrap_or(0));
            find()
        }
        "add" => {
            with(|s| {
                s.editing = Some((None, "{\n  \n}".to_owned()));
                s.confirm_delete = false;
            });
            Ok(())
        }
        "open" => {
            with(|s| {
                if let Some((i, doc)) = index.and_then(|i| s.docs.get(i).map(|d| (i, d))) {
                    s.editing = Some((Some(i), mongo::pretty(doc)));
                    s.confirm_delete = false;
                }
            });
            Ok(())
        }
        "delete-ask" => {
            with(|s| s.confirm_delete = true);
            Ok(())
        }
        "keep" => {
            with(|s| s.confirm_delete = false);
            Ok(())
        }
        "delete" => {
            let target = with(|s| {
                let (db, coll) = s.chosen.clone()?;
                let i = s.editing.as_ref()?.0?;
                Some((db, coll, s.docs.get(i)?.get("_id")?.clone()))
            });
            match target {
                Some((db, coll, id)) => server().and_then(|srv| {
                    start(srv.delete_call(&db, &coll, &id), Then::Change("Deleted {n} document".into()), "Deleting…")
                }),
                None => Ok(()),
            }
        }
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
    if server.uri.is_empty() {
        return Err("Give the address (mongodb://127.0.0.1:27017)".into());
    }
    with(|s| {
        let text = mongo::remember(&mut s.saved, &server);
        let _ = host::storage_write(&text);
        let mut shown = server.clone();
        shown.password = None;
        s.form = Some(shown);
        s.server = Some(server.clone());
        s.databases.clear();
        s.collections.clear();
        s.expanded.clear();
        s.chosen = None;
        s.docs.clear();
        s.editing = None;
        s.error = None;
    });
    start(server.databases_call(), Then::Databases, "Connecting…")
}

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    let result = match f.form.as_str() {
        "connect" => {
            let password = get("password");
            connect(Server {
                name: get("name").trim().to_owned(),
                uri: get("uri").trim().to_owned(),
                user: get("user").trim().to_owned(),
                auth_db: get("auth_db").trim().to_owned(),
                password: (!password.is_empty()).then_some(password),
            })
        }
        "find" => {
            with(|s| {
                s.filter = get("filter");
                s.sort = get("sort");
                s.projection = get("projection");
                s.skip = 0;
            });
            find()
        }
        "edit" if f.button == "Close" => {
            with(|s| {
                s.editing = None;
                s.confirm_delete = false;
            });
            Ok(())
        }
        "edit" => {
            let text = get("doc");
            let target = with(|s| {
                if let Some(e) = s.editing.as_mut() {
                    e.1 = text.clone();
                }
                s.chosen.clone().zip(s.editing.as_ref().map(|e| e.0))
            });
            match target {
                None => Ok(()),
                Some(_) if serde_json::from_str::<Value>(&text).is_err() => {
                    Err(format!("Not JSON: {}", serde_json::from_str::<Value>(&text).unwrap_err()))
                }
                Some(((db, coll), index)) => server().and_then(|srv| match index {
                    Some(_) => {
                        start(srv.replace_call(&db, &coll, &text), Then::Change("Saved ({n} changed)".into()), "Saving…")
                    }
                    None => start(srv.insert_call(&db, &coll, &text), Then::Change("Added the document".into()), "Saving…"),
                }),
            }
        }
        "script" => {
            let script = get("script");
            let db = with(|s| {
                s.script = Some(script.clone());
                s.script_out = None;
                s.chosen.as_ref().map_or_else(|| "test".to_owned(), |(d, _)| d.clone())
            });
            server().and_then(|srv| start(srv.run_call(&db, &script), Then::Script, "Running…"))
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
