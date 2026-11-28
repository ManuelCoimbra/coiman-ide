//! Python Calls and Memory: runs the Python file in front under Python's own cProfile (how
//! many times each function was called, and its time) or tracemalloc (the memory each line
//! still holds at the end, and the peak), and shows the results with links to the code.
//! Nothing to install: both come with Python.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod report;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FieldKind, Form, FormField, FormValues, JobResult, Row, Span};
use report::{Calls, Held};

const RED: u32 = 0xE5484D;
const AMBER: u32 = 0xE2A33D;
const DIM: u32 = 0x8B949E;
/// Rows shown at most.
const ROWS: usize = 150;
/// A line holding this many objects at the end is flagged.
const MANY_BLOCKS: u64 = 100_000;
const MANY_BLOCKS_TEXT: &str = "100,000";

#[derive(Clone, Copy, PartialEq, Default)]
enum Kind {
    #[default]
    Calls,
    Memory,
}

#[derive(Default)]
struct State {
    kind: Kind,
    file: String,
    job: Option<(u64, Kind)>,
    calls: Vec<Calls>,
    peak: u64,
    held: Vec<Held>,
    output: String,
    error: Option<String>,
    note: Option<String>,
    /// Only functions and lines of the open folder's own files.
    project_only: bool,
    /// Source lines read for the memory table: (path, line) → text.
    sources: HashMap<(String, u32), String>,
    done: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(|| State { project_only: true, ..State::default() }))
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn root() -> String {
    host::workspace_path().unwrap_or_default()
}

fn start(kind: Kind) -> Result<(), String> {
    if with(|s| s.job.is_some()) {
        return Err("A run is still going".into());
    }
    let doc = host::active_document()?.ok_or("Open a Python file first")?;
    if doc.path.is_empty() {
        return Err("Save the file in the open folder first".into());
    }
    if !doc.path.ends_with(".py") {
        return Err("The file in front isn't a Python file (.py)".into());
    }
    let driver = match kind {
        Kind::Calls => report::CALLS,
        Kind::Memory => report::MEMORY,
    };
    // Its full path, so what Python reports links back to the project's files.
    let full = format!("{}/{}", root().trim_end_matches(['/', '\\']), doc.path);
    let args = vec!["-c".to_owned(), driver.to_owned(), full];
    let id = host::start_job("python", &args, "").map_err(|e| {
        if e.contains("isn't on PATH") { "Python isn't installed (or isn't on PATH)".to_owned() } else { e }
    })?;
    with(|s| {
        s.kind = kind;
        s.file = doc.path.clone();
        s.job = Some((id, kind));
        s.error = None;
        s.note = Some(format!("Running {}…", doc.path));
        s.done = false;
    });
    render();
    Ok(())
}

fn finished(job: JobResult) {
    let Some(kind) = with(|s| s.job.filter(|(id, _)| *id == job.id).map(|(_, k)| k)) else { return };
    let (own, results) = report::split(&job.stdout);
    let own = if job.stderr.trim().is_empty() { own } else { format!("{own}\n{}", job.stderr.trim_end()) };
    with(|s| {
        s.job = None;
        s.output = own;
        s.note = None;
        s.done = true;
    });
    let Some(results) = results else {
        let why = match job.exit_code {
            None => "Stopped".to_owned(),
            Some(c) => format!("Python ended (exit code {c}) before the results were printed: see its output"),
        };
        with(|s| s.error = Some(why));
        render();
        return;
    };
    match kind {
        Kind::Calls => {
            let (calls, err) = report::calls(results);
            with(|s| {
                s.calls = calls;
                s.error = err.map(|e| format!("The program raised {e} (the results are up to there)"));
            });
        }
        Kind::Memory => {
            let (peak, held, err) = report::memory(results);
            with(|s| {
                s.peak = peak;
                s.held = held;
                s.error = err.map(|e| format!("The program raised {e} (the results are up to there)"));
            });
        }
    }
    render();
}

/// The text of a line of a project file (read once).
fn source(path: &str, line: u32) -> String {
    if let Some(t) = with(|s| s.sources.get(&(path.to_owned(), line)).cloned()) {
        return t;
    }
    let text = host::read_file(path)
        .ok()
        .and_then(|t| t.lines().nth(line.saturating_sub(1) as usize).map(|l| l.trim().to_owned()))
        .unwrap_or_default();
    let cut: String = text.chars().take(70).collect();
    with(|s| s.sources.insert((path.to_owned(), line), cut.clone()));
    cut
}

fn render() {
    let root = root();
    let (kind, project_only) = with(|s| (s.kind, s.project_only));
    let mut b = vec![Block::Rows(vec![Row {
        cells: vec![],
        buttons: vec![
            button(if kind == Kind::Calls { "▸ Calls" } else { "Calls" }, "tab", "calls"),
            button(if kind == Kind::Memory { "▸ Memory" } else { "Memory" }, "tab", "memory"),
            button("Run again", "again", ""),
        ],
    }])];
    b.push(Block::Form(Form {
        id: "filter".into(),
        fields: vec![FormField {
            id: "project".into(),
            label: "Only the project's own code".into(),
            kind: FieldKind::Check,
            value: project_only.to_string(),
            options: vec![],
            hint: String::new(),
        }],
        buttons: vec!["Apply".into()],
    }));
    let (note, error, file, done) = with(|s| (s.note.clone(), s.error.clone(), s.file.clone(), s.done));
    if let Some(n) = note {
        b.push(Block::Spans(vec![span(n, Some(DIM))]));
    }
    if let Some(e) = error {
        b.push(Block::Spans(vec![span(e, Some(RED))]));
    }
    if !done {
        if file.is_empty() {
            b.push(Block::Text(
                "Open a Python file, then Python: Count Calls or Python: Memory by Line. It runs to its end \
                 (input() gets nothing)."
                    .into(),
            ));
        }
        host::set_panel("pycalls", &b);
        return;
    }
    match kind {
        Kind::Calls => calls_table(&root, project_only, &file, &mut b),
        Kind::Memory => memory_table(&root, project_only, &file, &mut b),
    }
    let output = with(|s| s.output.clone());
    if !output.trim().is_empty() {
        b.push(Block::Heading("What the program printed".into()));
        let lines: Vec<&str> = output.lines().collect();
        let from = lines.len().saturating_sub(40);
        b.push(Block::Code(lines[from..].join("\n")));
    }
    host::set_panel("pycalls", &b);
}

fn calls_table(root: &str, project_only: bool, file: &str, b: &mut Vec<Block>) {
    let all = with(|s| s.calls.clone());
    if all.is_empty() {
        b.push(Block::Text("Run Python: Count Calls on a file to see its functions here.".into()));
        return;
    }
    let shown: Vec<&Calls> =
        all.iter().filter(|c| !project_only || report::relative(&c.file, root).is_some()).collect();
    let total: u64 = all.iter().map(|c| c.calls).sum();
    b.push(Block::Heading(format!("{file}: {} functions called, {total} calls in all", all.len())));
    if shown.len() < all.len() {
        b.push(Block::Spans(vec![span(
            format!("{} of them are Python's or a library's, not shown", all.len() - shown.len()),
            Some(DIM),
        )]));
    }
    let rows = shown
        .iter()
        .take(ROWS)
        .map(|c| {
            let place = report::relative(&c.file, root);
            let calls = if c.primitive != c.calls {
                format!("{} calls ({} from outside, the rest recursive)", c.calls, c.primitive)
            } else {
                format!("{} call{}", c.calls, if c.calls == 1 { "" } else { "s" })
            };
            let per = if c.calls > 0 { c.total / c.primitive.max(1) as f64 } else { 0.0 };
            Row {
                cells: vec![
                    Span { text: c.name.clone(), color: None, bold: true, monospace: false },
                    span(calls, None),
                    span(format!("own {}", report::secs(c.own)), Some(DIM)),
                    span(format!("with what it calls {}", report::secs(c.total)), Some(DIM)),
                    span(format!("{} a call", report::secs(per)), Some(DIM)),
                    span(place.as_ref().map_or_else(|| c.file.clone(), |p| format!("{p}:{}", c.line)), Some(DIM)),
                ],
                buttons: place.map(|p| vec![button("Open", "open", &format!("{p}:{}", c.line))]).unwrap_or_default(),
            }
        })
        .collect();
    b.push(Block::Rows(rows));
}

fn memory_table(root: &str, project_only: bool, file: &str, b: &mut Vec<Block>) {
    let (peak, all) = with(|s| (s.peak, s.held.clone()));
    if all.is_empty() && peak == 0 {
        b.push(Block::Text("Run Python: Memory by Line on a file to see what holds memory here.".into()));
        return;
    }
    let held: u64 = all.iter().map(|h| h.bytes).sum();
    b.push(Block::Heading(format!(
        "{file}: {} at its peak, {} still held at the end",
        report::bytes(peak),
        report::bytes(held)
    )));
    let shown: Vec<&Held> = all.iter().filter(|h| !project_only || report::relative(&h.file, root).is_some()).collect();
    let flagged = |h: &Held| h.blocks >= MANY_BLOCKS || (h.bytes >= 1_000_000 && h.bytes * 10 >= peak.max(1));
    let n = shown.iter().filter(|h| flagged(h)).count();
    if n > 0 {
        b.push(Block::Spans(vec![span(
            format!(
                "{n} line{} hold{} a lot (in red): {MANY_BLOCKS_TEXT} objects or more, or a tenth of the peak",
                if n == 1 { "" } else { "s" },
                if n == 1 { "s" } else { "" }
            ),
            Some(AMBER),
        )]));
    }
    let rows = shown
        .iter()
        .take(ROWS)
        .map(|h| {
            let place = report::relative(&h.file, root);
            let text = place.as_ref().map(|p| source(p, h.line)).unwrap_or_default();
            let avg = h.bytes.checked_div(h.blocks).unwrap_or(0);
            Row {
                cells: vec![
                    span(report::bytes(h.bytes), flagged(h).then_some(RED)),
                    span(format!("{} object{}", h.blocks, if h.blocks == 1 { "" } else { "s" }), None),
                    span(format!("{} each", report::bytes(avg)), Some(DIM)),
                    span(
                        place.as_ref().map_or_else(|| format!("{}:{}", h.file, h.line), |p| format!("{p}:{}", h.line)),
                        Some(DIM),
                    ),
                    Span { text, color: None, bold: false, monospace: true },
                ],
                buttons: place.map(|p| vec![button("Open", "open", &format!("{p}:{}", h.line))]).unwrap_or_default(),
            }
        })
        .collect();
    b.push(Block::Rows(rows));
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "calls" => start(Kind::Calls)?,
        "memory" => start(Kind::Memory)?,
        "again" => start(with(|s| s.kind))?,
        "tab" => {
            with(|s| s.kind = if arg == "memory" { Kind::Memory } else { Kind::Calls });
            render();
        }
        "open" => {
            let (path, line) = arg.rsplit_once(':').ok_or("no place")?;
            host::open_location(path, line.parse().unwrap_or(1))?;
        }
        _ => return Err(format!("unknown command {id}")),
    }
    Ok(())
}

fn submitted(f: FormValues) {
    if f.form == "filter" {
        let on = f.values.iter().any(|(k, v)| k == "project" && v == "true");
        with(|s| s.project_only = on);
        render();
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        let r = command(&id, &argument);
        if let Err(e) = &r {
            with(|s| s.error = Some(e.clone()));
            render();
        }
        r
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
