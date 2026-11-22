//! HTTP Client: `.http` files get "▶ Send" and "Copy as curl" above each request; the panel
//! shows the response, the environment used, the file's history of responses, and two
//! compared. Requests go out through `curl`, run as an approved job.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod request;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, CodeLens, JobResult, Row, Span};
use request::{Entry, History, Response};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;
const AMBER: u32 = 0xF29E4C;
const DIM: u32 = 0x8B949E;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Body,
    Headers,
    History,
    Compare,
}

/// A request on its way.
struct Sending {
    job: u64,
    file: String,
    name: Option<String>,
    label: String,
    method: String,
    url: String,
}

#[derive(Default)]
struct State {
    /// The `.http` file the panel is about.
    file: Option<String>,
    /// The environment chosen, by the file's folder.
    env: HashMap<String, String>,
    envs: request::Environments,
    /// Responses of named requests this session, by file: what `{{name.response…}}` reads.
    named: HashMap<String, HashMap<String, Response>>,
    sending: Option<Sending>,
    shown: Option<Entry>,
    error: Option<String>,
    history: Vec<Entry>,
    tab: Tab,
    /// Two history entries to compare (indexes).
    pick: (Option<usize>, Option<usize>),
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

struct Editor;

impl request::Outside for Editor {
    fn now_millis(&self) -> i64 {
        host::now().unix_millis
    }

    fn random(&self) -> u64 {
        host::random()
    }

    fn env_var(&self, name: &str) -> Result<Option<String>, String> {
        host::environment_variable(name)
    }
}

fn read(path: &str) -> Option<String> {
    host::read_file(path).ok()
}

// ----- lenses -------------------------------------------------------------------------------

/// "▶ Send" and "Copy as curl" above each request of the active `.http` file.
fn lenses() {
    let Ok(Some(doc)) = host::active_document() else { return };
    if doc.path.is_empty() || !request::is_http_file(&doc.path) {
        return;
    }
    let file = request::parse(&doc.text);
    let lenses: Vec<CodeLens> = file
        .requests
        .iter()
        .flat_map(|r| {
            let line = r.line as u32 + 1;
            [
                CodeLens { line, title: "▶ Send".into(), command: "send".into(), argument: r.line.to_string() },
                CodeLens { line, title: "Copy as curl".into(), command: "curl".into(), argument: r.line.to_string() },
            ]
        })
        .collect();
    host::set_code_lenses(&doc.path, &lenses);
}

// ----- sending ------------------------------------------------------------------------------

/// The active `.http` file's path, parsed text and the request at `line` (or the cursor).
fn request_at(line: &str) -> Result<(String, request::File, request::Request), String> {
    let doc = host::active_document()?.ok_or("Open a .http file first")?;
    if !request::is_http_file(&doc.path) {
        return Err("Put the cursor in a request of a .http file".into());
    }
    let line = line.parse::<usize>().unwrap_or(doc.line.saturating_sub(1) as usize);
    let file = request::parse(&doc.text);
    let req = request::request_at(&file, line).ok_or("There's no request in this file")?.clone();
    Ok((doc.path, file, req))
}

fn ready(path: &str, file: &request::File, req: &request::Request) -> Result<request::Ready, String> {
    let envs = request::environments(path, &read);
    let env_name = with(|s| {
        s.envs = envs.clone();
        s.env.get(&request::folder_of(path)).cloned()
    });
    let env = env_name.as_ref().and_then(|n| envs.iter().find(|(e, _)| e == n)).map(|(_, v)| v);
    let named = with(|s| s.named.get(path).cloned().unwrap_or_default());
    let values = request::Values {
        file_vars: &file.vars,
        env,
        responses: &named,
        dotenv: request::dotenv(path, &read),
        outside: &Editor,
    };
    request::prepare(req, &values, path, &|p| host::read_file(p))
}

fn show_file(path: &str) {
    let history = History::read(&host::storage_read()).of(path);
    with(|s| {
        if s.file.as_deref() != Some(path) {
            s.pick = (None, None);
        }
        s.file = Some(path.to_owned());
        s.history = history;
    });
}

fn send(line: &str) -> Result<(), String> {
    let (path, file, req) = request_at(line)?;
    show_file(&path);
    let ready = match ready(&path, &file, &req) {
        Ok(r) => r,
        Err(e) => {
            with(|s| s.error = Some(e));
            return Ok(());
        }
    };
    let (args, stdin) = request::curl_args(&ready);
    let job = host::start_job("curl", &args, &stdin)?;
    with(|s| {
        s.error = None;
        s.sending = Some(Sending {
            job,
            file: path,
            name: req.name.clone(),
            label: req.label(),
            method: ready.method,
            url: ready.url,
        });
    });
    Ok(())
}

fn finished(job: JobResult) {
    let Some(sent) =
        with(|s| if s.sending.as_ref().is_some_and(|x| x.job == job.id) { s.sending.take() } else { None })
    else {
        return;
    };
    match request::read_curl(job.exit_code, &job.stdout, &job.stderr, &sent.url) {
        Ok(resp) => {
            let entry = Entry {
                when: host::now().unix_millis,
                request: sent.label,
                method: sent.method,
                url: sent.url,
                response: resp,
            };
            let mut all = History::read(&host::storage_read());
            let text = all.remember(&sent.file, entry.clone());
            if let Err(e) = host::storage_write(&text) {
                host::show_message(&format!("Couldn't keep the response: {e}"));
            }
            let r = &entry.response;
            host::show_message(&format!("{} {} in {:.0} ms", r.status, r.reason, r.millis));
            with(|s| {
                if let Some(name) = &sent.name {
                    s.named.entry(sent.file.clone()).or_default().insert(name.clone(), entry.response.clone());
                }
                if s.file.as_deref() == Some(sent.file.as_str()) {
                    s.history = all.of(&sent.file);
                    // Older picks now point one further down.
                    let shift = |p: Option<usize>| p.map(|i| i + 1);
                    s.pick = (shift(s.pick.0), shift(s.pick.1));
                }
                s.shown = Some(entry);
                if matches!(s.tab, Tab::History | Tab::Compare) {
                    s.tab = Tab::Body;
                }
            });
        }
        Err(e) => with(|s| s.error = Some(e)),
    }
    render();
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: vec![], buttons }])
}

fn status_color(status: u16) -> u32 {
    match status {
        200..=299 => GREEN,
        300..=399 => AMBER,
        _ => RED,
    }
}

fn size(n: usize) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// The local time of day of a moment.
fn time_of_day(millis: i64) -> String {
    let offset = i64::from(host::now().utc_offset_minutes) * 60;
    let (_, _, _, h, m, s, _) = request::civil(millis.div_euclid(1000) + offset);
    format!("{h:02}:{m:02}:{s:02}")
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let name = s.file.clone().unwrap_or_else(|| "No .http file yet".into());
        let mut top = vec![button("Send", "send", "")];
        if let Some(file) = &s.file {
            // The environments, the chosen one marked.
            let current = s.env.get(&request::folder_of(file)).cloned();
            let mark = |on: bool, label: &str| if on { format!("▸ {label}") } else { label.to_owned() };
            top.push(button(&mark(current.is_none(), "No environment"), "env", ""));
            for (env, _) in &s.envs {
                top.push(button(&mark(current.as_ref() == Some(env), env), "env", env));
            }
        }
        b.push(Block::Spans(vec![Span { text: name, color: None, bold: true, monospace: false }]));
        b.push(bar(top));
        if let Some(sending) = &s.sending {
            b.push(Block::Spans(vec![span(format!("Sending {} {}…", sending.method, sending.url), Some(DIM))]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        let tabs = [
            (Tab::Body, "Body".to_owned(), "body"),
            (Tab::Headers, "Headers".to_owned(), "headers"),
            (Tab::History, format!("History  {}", s.history.len()), "history"),
            (Tab::Compare, "Compare".to_owned(), "compare"),
        ];
        b.push(bar(tabs
            .iter()
            .map(|(t, label, id)| button(&if *t == s.tab { format!("▸ {label}") } else { label.clone() }, "tab", id))
            .collect()));
        if let Some(entry) = &s.shown
            && matches!(s.tab, Tab::Body | Tab::Headers)
        {
            let r = &entry.response;
            b.push(Block::Spans(vec![
                Span {
                    text: format!("{} {}", r.status, r.reason),
                    color: Some(status_color(r.status)),
                    bold: true,
                    monospace: false,
                },
                span(
                    format!(
                        "   {:.0} ms · {} · {}   {} {}",
                        r.millis,
                        size(r.body.len()),
                        r.version,
                        entry.method,
                        r.url
                    ),
                    Some(DIM),
                ),
            ]));
        }
        match s.tab {
            Tab::Body => match &s.shown {
                None => b.push(Block::Spans(vec![span(
                    "Send a request: ▶ Send above it in a .http file, or Send here with the cursor in it.",
                    Some(DIM),
                )])),
                Some(entry) => {
                    b.push(bar(vec![button("Copy", "copy-body", ""), button("Open in a tab", "open-body", "")]));
                    let text = entry.response.pretty();
                    b.push(Block::Code(if text.is_empty() { "(no body)".to_owned() } else { text }));
                }
            },
            Tab::Headers => {
                if let Some(entry) = &s.shown {
                    let rows = entry
                        .response
                        .headers
                        .iter()
                        .map(|(k, v)| Row {
                            cells: vec![
                                Span { text: k.clone(), color: None, bold: true, monospace: true },
                                Span { text: v.clone(), color: None, bold: false, monospace: true },
                            ],
                            buttons: vec![],
                        })
                        .collect();
                    b.push(Block::Rows(rows));
                }
            }
            Tab::History => {
                if s.history.is_empty() {
                    b.push(Block::Spans(vec![span(
                        "Responses to this file's requests are kept here (the last 30).",
                        Some(DIM),
                    )]));
                } else {
                    b.push(Block::Spans(vec![span("Show one; A and B choose two to compare.", Some(DIM))]));
                    let rows = s
                        .history
                        .iter()
                        .enumerate()
                        .map(|(i, e)| {
                            let a = if s.pick.0 == Some(i) { "▸ A" } else { "A" };
                            let bb = if s.pick.1 == Some(i) { "▸ B" } else { "B" };
                            Row {
                                cells: vec![
                                    span(time_of_day(e.when), Some(DIM)),
                                    span(e.response.status.to_string(), Some(status_color(e.response.status))),
                                    Span {
                                        text: format!("{} {}", e.method, e.url),
                                        color: None,
                                        bold: false,
                                        monospace: true,
                                    },
                                    span(format!("{:.0} ms", e.response.millis), Some(DIM)),
                                ],
                                buttons: vec![
                                    button("Show", "history-show", &i.to_string()),
                                    button(a, "pick-a", &i.to_string()),
                                    button(bb, "pick-b", &i.to_string()),
                                ],
                            }
                        })
                        .collect();
                    b.push(Block::Rows(rows));
                    b.push(bar(vec![button("Forget this file's history", "forget", "")]));
                }
            }
            Tab::Compare => {
                let get = |p: Option<usize>| p.and_then(|i| s.history.get(i));
                match (get(s.pick.0), get(s.pick.1)) {
                    (Some(a), Some(bb)) => {
                        b.push(Block::Spans(vec![span(
                            format!(
                                "A: {} {} at {}   B: {} {} at {}",
                                a.method,
                                a.url,
                                time_of_day(a.when),
                                bb.method,
                                bb.url,
                                time_of_day(bb.when)
                            ),
                            Some(DIM),
                        )]));
                        let lines = request::compare(&a.response, &bb.response);
                        if lines.iter().all(|(m, _)| *m == ' ') {
                            b.push(Block::Spans(vec![span("The same, but for dates and request ids.", Some(GREEN))]));
                        }
                        let text: Vec<String> = lines.iter().map(|(m, l)| format!("{m}{l}")).collect();
                        b.push(Block::Diff(text.join("\n")));
                    }
                    _ => b.push(Block::Spans(vec![span("Choose A and B in History.", Some(DIM))])),
                }
            }
        }
        b
    });
    host::set_panel("http", &blocks);
}

// ----- commands -----------------------------------------------------------------------------

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "send" => send(arg)?,
        "curl" => {
            let (path, file, req) = request_at(arg)?;
            let ready = ready(&path, &file, &req)?;
            host::copy_text(&request::curl_line(&ready));
            host::show_message("Copied the request as a curl command");
            return Ok(());
        }
        "show" => {
            if let Ok(Some(doc)) = host::active_document()
                && request::is_http_file(&doc.path)
            {
                show_file(&doc.path);
                let envs = request::environments(&doc.path, &read);
                with(|s| s.envs = envs);
            }
        }
        "env" => with(|s| {
            if let Some(file) = &s.file {
                let key = request::folder_of(file);
                if arg.is_empty() {
                    s.env.remove(&key);
                } else {
                    s.env.insert(key, arg.to_owned());
                }
            }
        }),
        "tab" => with(|s| {
            s.tab = match arg {
                "headers" => Tab::Headers,
                "history" => Tab::History,
                "compare" => Tab::Compare,
                _ => Tab::Body,
            }
        }),
        "copy-body" => {
            if let Some(text) = with(|s| s.shown.as_ref().map(|e| e.response.pretty())) {
                host::copy_text(&text);
            }
            return Ok(());
        }
        "open-body" => {
            if let Some(text) = with(|s| s.shown.as_ref().map(|e| e.response.pretty())) {
                host::open_text(&text);
            }
            return Ok(());
        }
        "history-show" => with(|s| {
            if let Some(e) = arg.parse::<usize>().ok().and_then(|i| s.history.get(i)) {
                s.shown = Some(e.clone());
                s.tab = Tab::Body;
            }
        }),
        "pick-a" => with(|s| s.pick.0 = arg.parse().ok()),
        "pick-b" => with(|s| s.pick.1 = arg.parse().ok()),
        "forget" => {
            let file = with(|s| s.file.clone()).ok_or("no file")?;
            let mut all = History::read(&host::storage_read());
            all.files.remove(&file);
            host::storage_write(&serde_json::to_string(&all).unwrap_or_default())?;
            with(|s| {
                s.history.clear();
                s.pick = (None, None);
            });
        }
        _ => return Err(format!("unknown command {id}")),
    }
    render();
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        lenses();
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        command(&id, &argument)
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => finished(job),
            Event::Opened(_) | Event::ActiveChanged(_) | Event::Edited(_) => lenses(),
            _ => {}
        }
    }
}

export!(Plugin);
