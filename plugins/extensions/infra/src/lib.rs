//! Infrastructure Files: Kubernetes manifests, Helm charts, Terraform modules and Ansible
//! playbooks and roles, checked offline while typing (no cluster, no cloud, no downloads); a
//! panel listing them with their problems; a graph of what refers to what; and, when approved,
//! `helm lint`/`helm template`, `terraform fmt -check`/`terraform validate` and `ansible-lint`.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod ansible;
pub mod helm;
pub mod k8s;
pub mod project;
pub mod terraform;
pub mod tools;
pub mod yaml;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, EdgeKind, FileLink, Graph, GraphEdge, GraphNode, JobResult, Problem, Row, Severity, Span,
};
use project::{Analysis, Kind};

/// How bad a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Information,
    Hint,
}

/// A problem in a file: 1-based line, 0-based columns.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub path: String,
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub level: Level,
    pub message: String,
}

const MAX_FILES: u32 = 20_000;
/// Files read per step, and bytes of one at most.
const STEP: usize = 100;
const MAX_TEXT: usize = 1 << 20;
/// Lines of a program's output shown.
const MAX_OUTPUT: usize = 200;
const RED: u32 = 0xE5484D;
const AMBER: u32 = 0xE2A33D;
const GREEN: u32 = 0x3FB950;
const DIM: u32 = 0x8B949E;

enum Outcome {
    Running(u64),
    Done { code: Option<i32>, output: String },
    Failed(String),
}

struct ToolRun {
    run: tools::Run,
    outcome: Outcome,
}

#[derive(Default)]
struct State {
    all: BTreeSet<String>,
    texts: BTreeMap<String, String>,
    queue: Vec<String>,
    next: usize,
    reading: bool,
    scanned: bool,
    analysis: Analysis,
    /// The problems last given to the editor, by file.
    sent: BTreeMap<String, Vec<Finding>>,
    runs: Vec<ToolRun>,
    graph_wanted: bool,
    workspace: String,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn link(text: String, detail: String, path: &str, line: usize) -> FileLink {
    FileLink { text, detail, path: path.to_owned(), line: line.max(1) as u32 }
}

fn cut(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut at = MAX_TEXT;
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        text.truncate(at);
    }
    text
}

/// A file's text: the active editor's (unsaved changes too) when it's that file, else as saved.
fn text_of(path: &str) -> Option<String> {
    if let Ok(Some(doc)) = host::active_document()
        && doc.path == path
    {
        return Some(cut(doc.text));
    }
    host::read_file(path).ok().map(cut)
}

/// Whether a path could be an infrastructure file, before the folder is read.
fn maybe_infra(path: &str) -> bool {
    path.ends_with(".yaml") || path.ends_with(".yml") || path.ends_with(".tf") || path.ends_with(".tpl")
}

// ----- reading -------------------------------------------------------------------------------

fn scan(graph: bool) -> Result<(), String> {
    let all: BTreeSet<String> = host::list_files("", MAX_FILES)?.into_iter().collect();
    let charts = project::charts(&all);
    let queue: Vec<String> = all.iter().filter(|p| project::candidate(p, &charts)).cloned().collect();
    let workspace = host::workspace_path().unwrap_or_default();
    with(|s| {
        s.all = all;
        s.queue = queue;
        s.next = 0;
        s.texts.clear();
        s.reading = true;
        s.graph_wanted |= graph;
        s.workspace = workspace;
    });
    render();
    host::continue_later("scan");
    Ok(())
}

fn step() {
    let done = with(|s| {
        let end = (s.next + STEP).min(s.queue.len());
        for path in &s.queue[s.next..end] {
            if let Some(text) = text_of(path) {
                s.texts.insert(path.clone(), text);
            }
        }
        s.next = end;
        s.next >= s.queue.len()
    });
    if !done {
        host::continue_later("scan");
        return;
    }
    let graph = with(|s| {
        s.reading = false;
        s.scanned = true;
        std::mem::take(&mut s.graph_wanted)
    });
    check_all();
    if graph {
        show_graph();
    }
}

/// Checks everything read again, and gives the editor the problems that changed.
fn check_all() {
    let changed = with(|s| {
        s.analysis = project::analyse(&s.all, &s.texts);
        let now = project::by_file(&s.analysis.findings);
        let paths: BTreeSet<String> = s.sent.keys().chain(now.keys()).cloned().collect();
        let changed: Vec<(String, Vec<Finding>)> = paths
            .into_iter()
            .filter(|p| s.sent.get(p) != now.get(p))
            .map(|p| {
                let f = now.get(&p).cloned().unwrap_or_default();
                (p, f)
            })
            .collect();
        s.sent = now;
        changed
    });
    for (path, findings) in changed {
        let items: Vec<Problem> = findings
            .iter()
            .map(|f| Problem {
                line: f.line.max(1) as u32,
                start: f.start as u32,
                end: f.end.max(f.start + 1) as u32,
                severity: match f.level {
                    Level::Error => Severity::Error,
                    Level::Warning => Severity::Warning,
                    Level::Information => Severity::Information,
                    Level::Hint => Severity::Hint,
                },
                message: f.message.clone(),
            })
            .collect();
        host::set_problems(&path, &items);
    }
    render();
}

/// A file changed (opened, edited, saved): read it again and check.
fn changed(path: &str) {
    let (scanned, reading) = with(|s| (s.scanned, s.reading));
    if reading || path.is_empty() {
        return;
    }
    if !scanned {
        if maybe_infra(path) {
            let _ = scan(false);
        }
        return;
    }
    let charts = project::charts(&with(|s| s.all.clone()));
    let new_chart = path.ends_with("/Chart.yaml") || path == "Chart.yaml";
    if !project::candidate(path, &charts) && !new_chart {
        return;
    }
    if new_chart && !with(|s| s.all.contains(path)) {
        // A new chart: its templates are read too.
        let _ = scan(false);
        return;
    }
    let Some(text) = text_of(path) else { return };
    let same = with(|s| {
        s.all.insert(path.to_owned());
        s.texts.get(path) == Some(&text)
    });
    if same {
        return;
    }
    with(|s| s.texts.insert(path.to_owned(), text));
    check_all();
}

fn deleted(paths: &[String]) {
    if !with(|s| s.scanned) {
        return;
    }
    with(|s| {
        for p in paths {
            let prefix = format!("{p}/");
            s.all.retain(|f| f != p && !f.starts_with(&prefix));
            s.texts.retain(|f, _| f != p && !f.starts_with(&prefix));
        }
    });
    check_all();
}

fn renamed(from: &str, to: &str) {
    if !with(|s| s.scanned) {
        return;
    }
    with(|s| {
        let prefix = format!("{from}/");
        let moved: Vec<String> = s.all.iter().filter(|f| *f == from || f.starts_with(&prefix)).cloned().collect();
        for f in moved {
            let new = format!("{to}{}", &f[from.len()..]);
            s.all.remove(&f);
            s.all.insert(new.clone());
            if let Some(t) = s.texts.remove(&f) {
                s.texts.insert(new, t);
            }
        }
    });
    check_all();
}

// ----- programs ------------------------------------------------------------------------------

fn run_tools() {
    let runs = with(|s| {
        let a = &s.analysis;
        tools::runs(&a.charts, &tools::root_modules(&a.modules), a.ansible)
    });
    if runs.is_empty() {
        host::show_message("No charts, Terraform folders or playbooks to run tools on");
        return;
    }
    let mut started = Vec::new();
    for run in runs {
        let outcome = match host::start_job(run.program, &run.args, "") {
            Ok(id) => Outcome::Running(id),
            Err(e) => Outcome::Failed(format!("{} didn't start: {e}", run.program)),
        };
        started.push(ToolRun { run, outcome });
    }
    with(|s| s.runs = started);
    render();
}

fn job_done(job: JobResult) {
    let mine = with(|s| {
        let Some(r) = s.runs.iter_mut().find(|r| matches!(r.outcome, Outcome::Running(id) if id == job.id)) else {
            return false;
        };
        let mut output = job.stdout.clone();
        if !job.stderr.trim().is_empty() {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            output.push_str(&job.stderr);
        }
        r.outcome = Outcome::Done { code: job.exit_code, output };
        true
    });
    if mine {
        render();
    }
}

// ----- the panel -----------------------------------------------------------------------------

fn counts(findings: &[Finding]) -> String {
    let n = |l: Level| findings.iter().filter(|f| f.level == l).count();
    let mut parts = Vec::new();
    for (l, one) in [(Level::Error, "error"), (Level::Warning, "warning"), (Level::Hint, "hint")] {
        if n(l) > 0 {
            parts.push(plural(n(l), one));
        }
    }
    if parts.is_empty() { "no problems".into() } else { parts.join(", ") }
}

fn render() {
    let blocks = with(|s| {
        let buttons =
            vec![button("Check again", "rescan", ""), button("Graph", "graph", ""), button("Run tools", "tools", "")];
        let mut b = vec![Block::Rows(vec![Row { cells: vec![], buttons }])];
        if s.reading {
            b.push(Block::Text(format!("Reading the files: {} of {}…", s.next, s.queue.len())));
            return b;
        }
        if !s.scanned {
            b.push(Block::Text("Not read yet: Check again reads the open folder.".into()));
            return b;
        }
        let a = &s.analysis;
        if a.files.is_empty() {
            b.push(Block::Text(
                "No infrastructure files here: no Kubernetes manifests, Helm charts, Terraform (.tf) or Ansible \
                 playbooks."
                    .into(),
            ));
        } else {
            let mut kinds: BTreeMap<Kind, usize> = BTreeMap::new();
            for k in a.files.values() {
                *kinds.entry(*k).or_default() += 1;
            }
            let what: Vec<String> = kinds.iter().map(|(k, n)| format!("{n} {}", k.name())).collect();
            b.push(Block::Text(format!(
                "{}: {}. {}.",
                plural(a.files.len(), "file"),
                what.join(", "),
                counts(&a.findings)
            )));
            let by = project::by_file(&a.findings);
            let groups: [(&str, &[Kind]); 4] = [
                ("Kubernetes", &[Kind::Kubernetes]),
                ("Helm", &[Kind::Chart, Kind::Values, Kind::Template]),
                ("Terraform", &[Kind::Terraform]),
                ("Ansible", &[Kind::Playbook, Kind::Tasks, Kind::AnsibleVars, Kind::Inventory]),
            ];
            for (title, kinds) in groups {
                let files: Vec<(&String, &Kind)> = a.files.iter().filter(|(_, k)| kinds.contains(k)).collect();
                if files.is_empty() {
                    continue;
                }
                b.push(Block::Heading(format!("{title} ({})", files.len())));
                let mut links = Vec::new();
                for (path, kind) in files {
                    let found = by.get(path).map(Vec::as_slice).unwrap_or_default();
                    links.push(link(path.clone(), format!("{} · {}", kind.name(), counts(found)), path, 1));
                    for f in found {
                        let mark = match f.level {
                            Level::Error => "error",
                            Level::Warning => "warning",
                            _ => "hint",
                        };
                        links.push(link(format!("    {}: {}", f.line, f.message), mark.into(), path, f.line));
                    }
                }
                b.push(Block::Links(links));
            }
        }
        if !s.runs.is_empty() {
            b.push(Block::Separator);
            b.push(Block::Heading("Tools".into()));
            let all = &s.all;
            let exists = |p: &str| all.contains(p);
            for r in &s.runs {
                b.push(Block::Spans(vec![Span {
                    text: r.run.title.clone(),
                    color: None,
                    bold: true,
                    monospace: true,
                }]));
                match &r.outcome {
                    Outcome::Running(_) => b.push(Block::Spans(vec![span("Running…", Some(DIM))])),
                    Outcome::Failed(e) => b.push(Block::Spans(vec![span(e, Some(AMBER))])),
                    Outcome::Done { code, output } => {
                        let (text, color) = match code {
                            Some(0) => ("Passed".to_owned(), GREEN),
                            Some(c) => (format!("Exit code {c}"), RED),
                            None => ("Stopped".to_owned(), AMBER),
                        };
                        b.push(Block::Spans(vec![span(text, Some(color))]));
                        if let Some(why) = tools::explain(r.run.program, output) {
                            b.push(Block::Spans(vec![span(why, Some(AMBER))]));
                        }
                        let places = tools::places(output, &r.run.base, &s.workspace, &exists);
                        if !places.is_empty() && r.run.args.first().map(String::as_str) != Some("template") {
                            b.push(Block::Links(
                                places.into_iter().map(|(p, l, t)| link(t, format!("{p}:{l}"), &p, l)).collect(),
                            ));
                        }
                        let lines: Vec<&str> = output.lines().collect();
                        if !lines.is_empty() {
                            let mut shown = lines[..lines.len().min(MAX_OUTPUT)].join("\n");
                            if lines.len() > MAX_OUTPUT {
                                shown.push_str(&format!("\n… {} more lines", lines.len() - MAX_OUTPUT));
                            }
                            b.push(Block::Code(shown));
                        }
                    }
                }
            }
        }
        b
    });
    host::set_panel("infra", &blocks);
}

fn show_graph() {
    let graph = with(|s| {
        let (nodes, edges) = project::graph(&s.analysis);
        let n = nodes.len();
        Graph {
            id: "infra".into(),
            title: format!("Infrastructure: {}", plural(n, "object")),
            nodes: nodes
                .into_iter()
                .map(|n| GraphNode {
                    members: if n.path.is_empty() { Vec::new() } else { vec![format!("{}:{}", n.path, n.line)] },
                    id: n.id,
                    label: n.label,
                    kind: n.kind,
                    group: n.group,
                    path: n.path,
                    line: n.line as u32,
                    accent: n.outside.then_some(DIM),
                })
                .collect(),
            edges: edges
                .into_iter()
                .map(|e| GraphEdge { source: e.from, target: e.to, kind: EdgeKind::Uses, label: e.label, accent: None })
                .collect(),
            focus: None,
        }
    });
    if graph.nodes.is_empty() {
        host::show_message("No Kubernetes objects or Terraform blocks to draw");
        return;
    }
    host::show_graph(&graph);
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let read = with(|s| s.scanned || s.reading);
    match id {
        "show" => {
            if read {
                render()
            } else {
                scan(false)?
            }
        }
        "graph" => {
            if with(|s| s.scanned) {
                show_graph()
            } else if with(|s| s.reading) {
                with(|s| s.graph_wanted = true)
            } else {
                scan(true)?
            }
        }
        "rescan" => scan(false)?,
        "tools" => {
            if !with(|s| s.scanned) {
                return Err("Read the folder first: Infrastructure: Show Files".into());
            }
            run_tools();
        }
        "open" => {
            let (path, line) = arg.rsplit_once(':').ok_or("no place")?;
            host::open_location(path, line.parse().unwrap_or(1))?;
        }
        _ => return Err(format!("unknown command {id}")),
    }
    Ok(())
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
            Event::Resumed(t) if t == "scan" => step(),
            Event::Opened(p) | Event::Saved(p) | Event::Edited(p) | Event::ActiveChanged(p) => changed(&p),
            Event::Deleted(paths) => deleted(&paths),
            Event::Renamed((from, to)) => renamed(&from, &to),
            Event::JobFinished(job) => job_done(job),
            _ => {}
        }
    }
}

export!(Plugin);
