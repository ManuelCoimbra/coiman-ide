//! OpenAPI: a spec's endpoints by tag and its models, in a panel; an endpoint opens as a
//! request in a `.http` file beside the spec, ready for the HTTP Client's ▶ Send. A spec gets a
//! lens on its first line saying how many endpoints and models it has.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod spec;

use std::collections::HashSet;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, CodeLens, FieldKind, Form, FormField, FormValues, Row, Span};
use spec::{Endpoint, Spec};

const RED: u32 = 0xE5484D;
const DIM: u32 = 0x8B949E;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Endpoints,
    Models,
}

#[derive(Default)]
struct State {
    /// The spec's path (relative to the open folder) and text, as last shown.
    file: Option<String>,
    text: String,
    spec: Option<Result<Spec, String>>,
    tab: Tab,
    filter: String,
    selected: Option<usize>,
    open_models: HashSet<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn extension(path: &str) -> String {
    path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// A lens on a spec's first line: how many endpoints, and a way to see them.
fn lens() {
    let Ok(Some(doc)) = host::active_document() else { return };
    if doc.path.is_empty() || !matches!(extension(&doc.path).as_str(), "json" | "yaml" | "yml") {
        return;
    }
    if !spec::looks_like_spec(&extension(&doc.path), &doc.text) {
        host::set_code_lenses(&doc.path, &[]);
        return;
    }
    let title = match spec::parse(&doc.text) {
        Ok(s) => format!("▸ {} endpoints, {} models: show them", s.endpoints.len(), s.models.len()),
        Err(_) => "▸ OpenAPI: show what can be read".to_owned(),
    };
    let lens = CodeLens { line: 1, title, command: "show".into(), argument: String::new() };
    host::set_code_lenses(&doc.path, &[lens]);
}

fn show() -> Result<(), String> {
    let doc = host::active_document()?.ok_or("Open an OpenAPI or Swagger file (JSON or YAML) first")?;
    if !spec::looks_like_spec(&extension(&doc.path), &doc.text) {
        return Err("Open an OpenAPI or Swagger file (JSON or YAML) first".into());
    }
    let parsed = spec::parse(&doc.text);
    with(|s| {
        s.file = Some(doc.path.clone());
        s.spec = Some(parsed);
        s.text = doc.text;
        s.selected = None;
    });
    Ok(())
}

/// The `.http` file beside the spec: `api/orders.yaml` → `api/orders.http`.
fn http_path(spec: &str) -> String {
    match spec.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.http"),
        None => format!("{spec}.http"),
    }
}

/// Adds the endpoint's request to the `.http` file (made with `@baseUrl` if new), unless it's
/// there already, and shows it.
fn open_request(index: usize) -> Result<(), String> {
    let (file, request, what, base) = with(|s| {
        let file = s.file.clone()?;
        let spec = s.spec.as_ref()?.as_ref().ok()?;
        let e = spec.endpoints.get(index)?;
        let base = spec.servers.first().cloned().unwrap_or_else(|| "http://localhost:8080".into());
        Some((file, spec.request(e), format!("{} {}", e.method, e.path), base))
    })
    .ok_or("Show a spec first")?;
    let path = http_path(&file);
    let text = host::read_file(&path).unwrap_or_else(|_| format!("@baseUrl = {base}\n"));
    // Already there (by its name, or its first line): just show it.
    let first = request.lines().find(|l| !l.starts_with('#')).unwrap_or("").to_owned();
    let named = request.lines().find(|l| l.starts_with("# @name ")).map(str::to_owned);
    let found = text.lines().position(|l| named.as_deref() == Some(l) || l == first);
    let line = match found {
        Some(at) => at,
        None => {
            let sep = if text.ends_with("\n\n") || text.is_empty() {
                ""
            } else if text.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            let now = format!("{text}{sep}{request}");
            host::write_file(&path, &now)?;
            now.lines().enumerate().filter(|(_, l)| *l == first).last().map_or(0, |(i, _)| i)
        }
    };
    host::open_location(&path, line as u32 + 1)?;
    host::show_message(&format!("{what} is in {path}: ▶ Send above it"));
    Ok(())
}

fn whole_file() -> Result<(), String> {
    let (file, http, count) = with(|s| {
        let spec = s.spec.as_ref()?.as_ref().ok()?;
        Some((s.file.clone()?, spec.http_file(), spec.endpoints.len()))
    })
    .ok_or("Show a spec first")?;
    let path = http_path(&file);
    if host::read_file(&path).is_ok() {
        host::open_location(&path, 1)?;
        host::show_message(&format!("{path} is there already: Open as request adds endpoints to it one by one"));
        return Ok(());
    }
    host::write_file(&path, &http)?;
    host::open_location(&path, 1)?;
    host::show_message(&format!("Wrote {count} endpoints to {path}"));
    Ok(())
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn mono(text: impl Into<String>, color: Option<u32>, bold: bool) -> Span {
    Span { text: text.into(), color, bold, monospace: true }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: vec![], buttons }])
}

fn method_color(m: &str) -> u32 {
    match m {
        "GET" => 0x57C06A,
        "POST" => 0x4F9DFF,
        "PUT" => 0xF29E4C,
        "PATCH" => 0xB07CF7,
        "DELETE" => 0xE5484D,
        _ => DIM,
    }
}

fn matches(e: &Endpoint, needle: &str) -> bool {
    needle.is_empty()
        || e.path.to_lowercase().contains(needle)
        || e.summary.to_lowercase().contains(needle)
        || e.method.to_lowercase() == needle
        || e.operation_id.as_ref().is_some_and(|o| o.to_lowercase().contains(needle))
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let spec = match &s.spec {
            None => {
                b.push(Block::Text("Open an OpenAPI or Swagger file and run OpenAPI: Show Endpoints.".into()));
                return b;
            }
            Some(Err(e)) => {
                b.push(Block::Spans(vec![span(e, Some(RED))]));
                return b;
            }
            Some(Ok(spec)) => spec,
        };
        let mut head = vec![
            Span { text: format!("{} {}", spec.title, spec.version), color: None, bold: true, monospace: false },
            span(format!("   {}", spec.kind), Some(DIM)),
        ];
        for server in &spec.servers {
            head.push(mono(format!("   {server}"), Some(DIM), false));
        }
        b.push(Block::Spans(head));
        let tab =
            |t: Tab, label: String, id: &str| button(&if s.tab == t { format!("▸ {label}") } else { label }, "tab", id);
        b.push(bar(vec![
            tab(Tab::Endpoints, format!("Endpoints  {}", spec.endpoints.len()), "endpoints"),
            tab(Tab::Models, format!("Models  {}", spec.models.len()), "models"),
            button("All as a .http file", "whole-file", ""),
        ]));
        b.push(Block::Form(Form {
            id: "filter".into(),
            fields: vec![FormField {
                id: "filter".into(),
                label: String::new(),
                kind: FieldKind::Text,
                value: s.filter.clone(),
                options: vec![],
                hint: "Filter".into(),
            }],
            buttons: vec!["Filter".into()],
        }));
        let needle = s.filter.to_lowercase();
        match s.tab {
            Tab::Endpoints => {
                if let Some(i) = s.selected
                    && let Some(e) = spec.endpoints.get(i)
                {
                    details(spec, e, i, &mut b);
                }
                for (tag, list) in spec.by_tag() {
                    let shown: Vec<usize> =
                        list.into_iter().filter(|&i| matches(&spec.endpoints[i], &needle)).collect();
                    if shown.is_empty() {
                        continue;
                    }
                    b.push(Block::Heading(tag));
                    let rows = shown
                        .into_iter()
                        .map(|i| {
                            let e = &spec.endpoints[i];
                            let path_color = if e.deprecated { Some(DIM) } else { None };
                            let mark = if s.selected == Some(i) { "▸ Details" } else { "Details" };
                            Row {
                                cells: vec![
                                    mono(format!("{:<6}", e.method), Some(method_color(&e.method)), true),
                                    mono(&*e.path, path_color, false),
                                    span(&*e.summary, Some(DIM)),
                                ],
                                buttons: vec![button(mark, "select", &i.to_string())],
                            }
                        })
                        .collect();
                    b.push(Block::Rows(rows));
                }
            }
            Tab::Models => {
                for m in spec.models.iter().filter(|m| needle.is_empty() || m.name.to_lowercase().contains(&needle)) {
                    let open = s.open_models.contains(&m.name);
                    b.push(Block::Rows(vec![Row {
                        cells: vec![mono(&*m.name, None, true), span(format!("{} fields", m.fields.len()), Some(DIM))],
                        buttons: vec![button(if open { "Hide" } else { "Show" }, "model", &m.name)],
                    }]));
                    if open {
                        if !m.description.is_empty() {
                            b.push(Block::Spans(vec![span(&*m.description, Some(DIM))]));
                        }
                        let rows = m
                            .fields
                            .iter()
                            .map(|(name, ty, required, desc)| Row {
                                cells: vec![
                                    mono(format!("{name}{}", if *required { " *" } else { "" }), None, false),
                                    mono(&**ty, Some(DIM), false),
                                    span(&**desc, Some(DIM)),
                                ],
                                buttons: vec![],
                            })
                            .collect();
                        b.push(Block::Rows(rows));
                    }
                }
            }
        }
        b
    });
    host::set_panel("openapi", &blocks);
}

fn details(spec: &Spec, e: &Endpoint, i: usize, b: &mut Vec<Block>) {
    let mut title =
        vec![mono(&*e.method, Some(method_color(&e.method)), true), mono(format!(" {}", e.path), None, true)];
    if e.deprecated {
        title.push(span("  (deprecated)", Some(DIM)));
    }
    b.push(Block::Spans(title));
    if !e.summary.is_empty() {
        b.push(Block::Text(e.summary.clone()));
    }
    if !e.description.is_empty() {
        b.push(Block::Spans(vec![span(&*e.description, Some(DIM))]));
    }
    let n = i.to_string();
    b.push(bar(vec![
        button("Open as request", "open-request", &n),
        button("Copy request", "copy-request", &n),
        button("Go to it", "go-to", &n),
        button("Close", "select", ""),
    ]));
    if !e.security.is_empty() {
        b.push(Block::Spans(vec![span(format!("Needs: {}", e.security.join(", ")), Some(DIM))]));
    }
    if !e.params.is_empty() {
        b.push(Block::Heading("Parameters".into()));
        let rows = e
            .params
            .iter()
            .map(|p| Row {
                cells: vec![
                    mono(&*p.name, None, true),
                    span(&*p.place, Some(DIM)),
                    mono(&*p.ty, None, false),
                    span(format!("{}{}", if p.required { "required. " } else { "" }, p.description), Some(DIM)),
                ],
                buttons: vec![],
            })
            .collect();
        b.push(Block::Rows(rows));
    }
    if let (Some((media, schema)), Some(example)) = (&e.body, spec.body_example(e)) {
        b.push(Block::Heading(format!(
            "Body: {} ({media}){}",
            spec::type_name(schema),
            if e.body_required { ", required" } else { "" }
        )));
        b.push(Block::Code(example));
    }
    if !e.responses.is_empty() {
        b.push(Block::Heading("Responses".into()));
        let rows = e
            .responses
            .iter()
            .map(|(code, desc, ty)| {
                let color = if code.starts_with('2') {
                    Some(method_color("GET"))
                } else if code.starts_with('4') || code.starts_with('5') {
                    Some(RED)
                } else {
                    None
                };
                Row {
                    cells: vec![mono(&**code, color, false), mono(&**ty, None, false), span(&**desc, Some(DIM))],
                    buttons: vec![],
                }
            })
            .collect();
        b.push(Block::Rows(rows));
    }
    b.push(Block::Separator);
}

// ----- commands -----------------------------------------------------------------------------

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" => show()?,
        "tab" => with(|s| s.tab = if arg == "models" { Tab::Models } else { Tab::Endpoints }),
        "select" => with(|s| s.selected = arg.parse().ok()),
        "model" => with(|s| {
            if !s.open_models.remove(arg) {
                s.open_models.insert(arg.to_owned());
            }
        }),
        "open-request" => open_request(arg.parse().map_err(|_| "no endpoint")?)?,
        "whole-file" => whole_file()?,
        "copy-request" => {
            let request = with(|s| {
                let spec = s.spec.as_ref()?.as_ref().ok()?;
                Some(spec.request(spec.endpoints.get(arg.parse::<usize>().ok()?)?))
            });
            if let Some(r) = request {
                host::copy_text(&r);
                host::show_message("Copied the request");
            }
        }
        "go-to" => {
            let at = with(|s| {
                let spec = s.spec.as_ref()?.as_ref().ok()?;
                let e = spec.endpoints.get(arg.parse::<usize>().ok()?)?;
                Some((s.file.clone()?, spec::line_of_path(&s.text, &e.path).unwrap_or(0)))
            });
            if let Some((file, line)) = at {
                host::open_location(&file, line as u32 + 1)?;
            }
        }
        _ => return Err(format!("unknown command {id}")),
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    if f.form == "filter" {
        let value = f.values.iter().find(|(k, _)| k == "filter").map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
        with(|s| s.filter = value);
        render();
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        lens();
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        command(&id, &argument)
    }

    fn on_event(event: Event) {
        match event {
            Event::Opened(_) | Event::ActiveChanged(_) | Event::Edited(_) => lens(),
            Event::FormSubmitted(f) => submitted(f),
            _ => {}
        }
    }
}

export!(Plugin);
