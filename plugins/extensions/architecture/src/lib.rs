//! Architecture: which file of the project imports which (Rust, Python, JavaScript and
//! TypeScript, Java, C#, Go, C and C++), read from the import lines without a build, drawn
//! by the editor; with the cycles, the most used files, the unused ones, and what a change to
//! a file reaches, in a panel.
//!
//! - Architecture: Show Dependency Graph (files), Show Folder Graph.
//! - Architecture: Impact of Current File: the files that import it, near and far, and the
//!   tests among them.
//!
//! Files are read in steps (`continue-later`), within each call's time limit.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod arch;
mod build;
mod changes;
mod deps;
mod rules;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, EdgeKind, FieldKind, FileLink, Form, FormField, FormValues, Graph as Shown, GraphEdge, GraphNode, JobResult,
    Table,
};
use deps::{Graph, Lang};

/// Source files looked at, at most.
const MAX_FILES: u32 = 20_000;
/// Text read per step, and files per step at most.
const STEP_BYTES: usize = 1 << 20;
const STEP: usize = 200;
/// Files whose imports are resolved per step.
const RESOLVE_STEP: usize = 40;
/// Only a file's start is read: imports are at the top.
const MAX_READ: usize = 512 * 1024;
/// Rows listed in the panel's tables.
const ROWS: usize = 25;

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;

#[derive(Clone, PartialEq)]
enum Want {
    Files,
    Folders,
    Impact(String),
    /// How the graph changed since this revision.
    Since(String),
}

/// The graph as it was at another revision, made while it's asked for.
struct Since {
    rev: String,
    now: Graph,
    files: Vec<(PathBuf, Lang)>,
    texts: Vec<String>,
    /// `git diff --name-status` running, then each changed file's `git show`, one at a time.
    job: Option<u64>,
    queue: Vec<String>,
    showing: Option<String>,
    added: Vec<String>,
    deleted: Vec<String>,
    old: std::collections::HashMap<String, String>,
    /// The old graph's files and texts, and its imports made a part at a time.
    old_files: Vec<(PathBuf, Lang)>,
    old_texts: Vec<String>,
    old_edges: Vec<deps::Edge>,
    resolved: usize,
}

static SINCE: Mutex<Option<Since>> = Mutex::new(None);

struct Scan {
    want: Want,
    files: Vec<(PathBuf, Lang)>,
    texts: Vec<String>,
    /// Files read, then files whose imports are resolved.
    next: usize,
    resolved: usize,
    edges: Vec<deps::Edge>,
}

static SCAN: Mutex<Option<Scan>> = Mutex::new(None);

/// The open folder's files sit under `/` here.
fn under_root(rel: &str) -> PathBuf {
    PathBuf::from(format!("/{rel}"))
}

fn read(path: &Path) -> Option<String> {
    let rel = path.to_string_lossy();
    host::read_file(rel.trim_start_matches('/')).ok()
}

fn start(want: Want) -> Result<(), String> {
    let mut files: Vec<(PathBuf, Lang)> = host::list_files("", MAX_FILES)?
        .into_iter()
        .filter_map(|rel| {
            let p = under_root(&rel);
            Lang::of(&p).map(|l| (p, l))
        })
        .collect();
    if files.is_empty() {
        return Err("No source files of a language it reads in this folder".to_owned());
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    host::show_message(&format!("Reading the imports of {} files…", files.len()));
    let texts = Vec::with_capacity(files.len());
    *SCAN.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Scan { want, files, texts, next: 0, resolved: 0, edges: Vec::new() });
    host::continue_later("scan");
    Ok(())
}

fn step() {
    let mut guard = SCAN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(scan) = guard.as_mut() else { return };
    if scan.next < scan.files.len() {
        let (mut bytes, mut count) = (0, 0);
        while scan.next < scan.files.len() && count < STEP && bytes < STEP_BYTES {
            let mut text = read(&scan.files[scan.next].0).unwrap_or_default();
            if text.len() > MAX_READ {
                let mut end = MAX_READ;
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                text.truncate(end);
            }
            bytes += text.len();
            scan.texts.push(text);
            scan.next += 1;
            count += 1;
        }
        host::show_message(&format!("Reading the imports: {} of {} files", scan.next, scan.files.len()));
        // The graph is made in a step of its own.
        host::continue_later("scan");
        return;
    }
    if scan.resolved < scan.files.len() {
        let end = (scan.resolved + RESOLVE_STEP).min(scan.files.len());
        let found = deps::edges(Path::new("/"), &scan.files, &scan.texts, &read, scan.resolved..end);
        scan.edges.extend(found);
        scan.resolved = end;
        host::show_message(&format!("Following the imports: {} of {} files", scan.resolved, scan.files.len()));
        host::continue_later("scan");
        return;
    }
    let Some(scan) = guard.take() else { return };
    drop(guard);
    let mut graph = deps::nodes(Path::new("/"), &scan.files, &scan.texts);
    graph.edges = scan.edges;
    match scan.want {
        Want::Files => show(&graph, false),
        Want::Folders => show(&arch::by_folder(&graph), true),
        Want::Impact(file) => impact(&graph, &file),
        Want::Since(rev) => since_start(graph, scan.files, scan.texts, rev),
    }
}

/// The form asking which revision to compare with.
fn since_form(note: Option<String>) {
    let mut blocks = vec![
        Block::Heading("Changes since".into()),
        Block::Text(
            "How the imports changed since a commit, tag or branch: the files git says changed are read as \
             they were there."
                .into(),
        ),
        Block::Form(Form {
            id: "since".into(),
            fields: vec![FormField {
                id: "rev".into(),
                label: "Since".into(),
                kind: FieldKind::Text,
                value: "HEAD~1".into(),
                options: vec![],
                hint: "a commit, tag or branch".into(),
            }],
            buttons: vec!["Compare".into()],
        }),
    ];
    if let Some(n) = note {
        blocks.push(Block::Text(n));
    }
    host::set_panel("report", &blocks);
}

fn since_start(now: Graph, files: Vec<(PathBuf, Lang)>, texts: Vec<String>, rev: String) {
    let args = ["diff", "--name-status", "--no-renames", rev.as_str(), "--"].map(str::to_owned);
    match host::start_job("git", &args, "") {
        Ok(id) => {
            host::show_message(&format!("Asking git what changed since {rev}…"));
            *SINCE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Since {
                rev,
                now,
                files,
                texts,
                job: Some(id),
                queue: Vec::new(),
                showing: None,
                added: Vec::new(),
                deleted: Vec::new(),
                old: Default::default(),
                old_files: Vec::new(),
                old_texts: Vec::new(),
                old_edges: Vec::new(),
                resolved: 0,
            });
        }
        Err(e) => since_form(Some(format!("git: {e}"))),
    }
}

/// The next changed file's text at the revision, or, with none left, the old graph is made.
fn since_next(s: &mut Since) {
    while let Some(path) = s.queue.pop() {
        let args = ["show".to_owned(), format!("{}:{path}", s.rev)];
        match host::start_job("git", &args, "") {
            Ok(id) => {
                s.job = Some(id);
                s.showing = Some(path);
                return;
            }
            Err(_) => continue,
        }
    }
    // Every file as it was: the changed ones from git, the rest as they are now; the added
    // ones weren't there, the deleted ones were.
    let mut files: Vec<(PathBuf, Lang, String)> = Vec::new();
    for ((p, lang), text) in s.files.iter().zip(&s.texts) {
        let rel = p.to_string_lossy().trim_start_matches('/').to_owned();
        if s.added.contains(&rel) {
            continue;
        }
        let text = s.old.get(&rel).cloned().unwrap_or_else(|| text.clone());
        files.push((p.clone(), *lang, text));
    }
    for rel in &s.deleted {
        let p = under_root(rel);
        if let (Some(lang), Some(text)) = (Lang::of(&p), s.old.get(rel)) {
            files.push((p, lang, text.clone()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    s.old_files = files.iter().map(|(p, l, _)| (p.clone(), *l)).collect();
    s.old_texts = files.into_iter().map(|(_, _, t)| t).collect();
    host::continue_later("since");
}

fn since_finished(job: JobResult) {
    let mut guard = SINCE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return };
    if s.job != Some(job.id) {
        return;
    }
    s.job = None;
    match s.showing.take() {
        None => {
            if job.exit_code != Some(0) {
                let err = job.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("git diff failed").to_owned();
                *guard = None;
                drop(guard);
                since_form(Some(err));
                return;
            }
            let d = changes::parse_name_status(&job.stdout);
            let source = |p: &String| Lang::of(&under_root(p)).is_some();
            s.added = d.added.into_iter().filter(source).collect();
            s.deleted = d.deleted.iter().filter(|p| source(p)).cloned().collect();
            s.queue = d.modified.into_iter().chain(d.deleted).filter(source).collect();
            host::show_message(&format!("Reading {} changed files as they were…", s.queue.len()));
        }
        Some(path) => {
            if job.exit_code == Some(0) {
                s.old.insert(path, job.stdout);
            }
        }
    }
    since_next(s);
}

/// Makes the old graph's imports a part at a time, then compares.
fn since_step() {
    let mut guard = SINCE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = guard.as_mut() else { return };
    if s.resolved < s.old_files.len() {
        let end = (s.resolved + RESOLVE_STEP).min(s.old_files.len());
        let found = deps::edges(Path::new("/"), &s.old_files, &s.old_texts, &read, s.resolved..end);
        s.old_edges.extend(found);
        s.resolved = end;
        host::show_message(&format!("Making the graph at {}: {} of {} files", s.rev, s.resolved, s.old_files.len()));
        host::continue_later("since");
        return;
    }
    let Some(s) = guard.take() else { return };
    drop(guard);
    let mut old = deps::nodes(Path::new("/"), &s.old_files, &s.old_texts);
    old.edges = s.old_edges;
    since_report(&s.rev, &changes::compare(&old, &s.now));
}

fn since_report(rev: &str, c: &changes::Changes) {
    let mut blocks = vec![Block::Heading(format!("Changes since {rev}"))];
    if c.is_empty() {
        blocks.push(Block::Text("The imports are the same as there.".into()));
    } else {
        blocks.push(Block::Text(format!(
            "{} imports added, {} removed; {} new cycles, {} gone; {} files added, {} removed.",
            c.imports_added.len(),
            c.imports_removed.len(),
            c.cycles_new.len(),
            c.cycles_gone.len(),
            c.files_added.len(),
            c.files_removed.len()
        )));
    }
    let pairs =
        |list: &[(String, String)]| list.iter().take(ROWS * 4).map(|(a, b)| vec![a.clone(), b.clone()]).collect();
    if !c.cycles_new.is_empty() {
        blocks.push(Block::Heading(format!("New cycles ({})", c.cycles_new.len())));
        blocks.push(Block::Bullets(c.cycles_new.iter().map(|v| v.join(" ⇄ ")).collect()));
    }
    if !c.imports_added.is_empty() {
        blocks.push(Block::Heading(format!("Imports added ({})", c.imports_added.len())));
        blocks.push(table(&["File", "Now imports"], pairs(&c.imports_added)));
    }
    if !c.imports_removed.is_empty() {
        blocks.push(Block::Heading(format!("Imports removed ({})", c.imports_removed.len())));
        blocks.push(table(&["File", "No longer imports"], pairs(&c.imports_removed)));
    }
    if !c.cycles_gone.is_empty() {
        blocks.push(Block::Heading(format!("Cycles gone ({})", c.cycles_gone.len())));
        blocks.push(Block::Bullets(c.cycles_gone.iter().map(|v| v.join(" ⇄ ")).collect()));
    }
    if !c.files_added.is_empty() {
        blocks.push(Block::Heading(format!("Files added ({})", c.files_added.len())));
        blocks.push(Block::Bullets(c.files_added.iter().take(ROWS * 2).cloned().collect()));
    }
    if !c.files_removed.is_empty() {
        blocks.push(Block::Heading(format!("Files removed ({})", c.files_removed.len())));
        blocks.push(Block::Bullets(c.files_removed.iter().take(ROWS * 2).cloned().collect()));
    }
    host::set_panel("report", &blocks);
    host::show_message(&format!("Compared with {rev}"));
}

/// The graph in the editor's window, and the report in the panel.
fn show(graph: &Graph, folders: bool) {
    let cycle_edges = arch::cycle_edges(graph);
    let in_cycle: std::collections::HashSet<usize> = cycle_edges.iter().flat_map(|&(a, b)| [a, b]).collect();
    let nodes = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| GraphNode {
            id: n.path.clone(),
            label: if folders { n.path.clone() } else { arch::label(&n.path) },
            kind: String::new(),
            group: if folders { String::new() } else { n.path.rsplit_once('/').map_or("", |(d, _)| d).to_owned() },
            members: Vec::new(),
            path: if folders { String::new() } else { n.path.clone() },
            line: 1,
            accent: if in_cycle.contains(&i) {
                Some(RED)
            } else if n.test {
                Some(GREEN)
            } else {
                None
            },
        })
        .collect();
    let edges = graph
        .edges
        .iter()
        .map(|e| GraphEdge {
            source: graph.nodes[e.from].path.clone(),
            target: graph.nodes[e.to].path.clone(),
            kind: EdgeKind::Uses,
            // By folder, `line` counts the imports it stands for.
            label: if folders { e.line.to_string() } else { String::new() },
            accent: cycle_edges.contains(&(e.from, e.to)).then_some(RED),
        })
        .collect();
    let what = if folders { "folders" } else { "files" };
    let more = if graph.truncated { format!(" (the first {MAX_FILES})") } else { String::new() };
    let title = format!("Architecture: {} {what}{more}, {} imports", graph.nodes.len(), graph.edges.len());
    host::show_graph(&Shown { id: "architecture".to_owned(), title, nodes, edges, focus: None });
    report(graph, folders);
}

fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Block {
    Block::Table(Table { headers: headers.iter().map(|h| (*h).to_owned()).collect(), rows })
}

/// Cycles, the most used, the unused.
fn report(graph: &Graph, folders: bool) {
    let name = |i: usize| graph.nodes[i].path.clone();
    let cycles = arch::cycles(graph);
    let coupling = arch::coupling(graph);
    let mut blocks = vec![
        Block::Heading(if folders { "Folders".into() } else { "Files".into() }),
        Block::Text(format!(
            "{} {}, {} imports, {} in cycles.",
            graph.nodes.len(),
            if folders { "folders" } else { "files" },
            graph.edges.len(),
            cycles.iter().map(Vec::len).sum::<usize>()
        )),
    ];
    if !cycles.is_empty() {
        blocks.push(Block::Heading(format!("Cycles ({})", cycles.len())));
        blocks.push(Block::Text("Files that import each other round in a circle, largest first.".into()));
        let rows = cycles
            .iter()
            .take(ROWS)
            .map(|c| vec![c.len().to_string(), c.iter().map(|&i| name(i)).collect::<Vec<_>>().join(" → ")])
            .collect();
        blocks.push(table(&["Files", "Cycle"], rows));
    }
    let mut used: Vec<usize> = (0..graph.nodes.len()).filter(|&i| coupling[i].fan_in > 0).collect();
    used.sort_by_key(|&i| std::cmp::Reverse(coupling[i].fan_in));
    if !used.is_empty() {
        blocks.push(Block::Heading("Most used".into()));
        let rows = used
            .iter()
            .take(ROWS)
            .map(|&i| vec![name(i), coupling[i].fan_in.to_string(), coupling[i].fan_out.to_string()])
            .collect();
        blocks.push(table(&["File", "Used by", "Uses"], rows));
    }
    if !folders {
        rules_report(graph, &mut blocks);
    }
    let unused = arch::isolated(graph);
    if !unused.is_empty() && !folders {
        blocks.push(Block::Heading(format!("Unused ({})", unused.len())));
        blocks.push(Block::Text("Nothing imports them and they import nothing (entry points and scripts too).".into()));
        blocks.push(Block::Bullets(unused.iter().take(ROWS * 2).map(|&i| name(i)).collect()));
    }
    host::set_panel("report", &blocks);
}

/// The imports that break the project's rules (`.coiman/architecture.toml`).
fn rules_report(graph: &Graph, blocks: &mut Vec<Block>) {
    let Ok(text) = host::read_file(rules::FILE) else {
        blocks.push(Block::Heading("Rules".into()));
        blocks.push(Block::Text(format!(
            "No rules for this project. Put them in {}: `layers`, from the top down (a file may not \
             import a layer above its own), and `[[rule]]`s with `from`, `not` and `why`.",
            rules::FILE
        )));
        return;
    };
    match rules::parse(&text) {
        Err(e) => {
            blocks.push(Block::Heading("Rules".into()));
            blocks.push(Block::Text(format!("{}: {e}", rules::FILE)));
        }
        Ok(r) => {
            let found = rules::check(graph, &r);
            blocks.push(Block::Heading(format!("Rules broken ({})", found.len())));
            if found.is_empty() {
                blocks.push(Block::Text(format!(
                    "Every import keeps the rules in {} ({} layers, {} rules).",
                    rules::FILE,
                    r.layers.len(),
                    r.rules.len()
                )));
                return;
            }
            blocks.push(Block::Text(format!("Imports that break the rules in {}:", rules::FILE)));
            blocks.push(Block::Links(
                found
                    .iter()
                    .map(|v| FileLink {
                        text: format!("{}:{} → {}", v.from, v.line, v.to),
                        detail: v.why.clone(),
                        path: v.from.clone(),
                        line: v.line as u32,
                    })
                    .collect(),
            ));
        }
    }
}

/// The project's own packages and their dependencies on each other, from the build files.
fn build_graph() -> Result<(), String> {
    let mut files = std::collections::BTreeMap::new();
    for path in host::list_files("", MAX_FILES)?.into_iter().filter(|p| build::is_build_file(p)).take(2000) {
        if let Ok(text) = host::read_file(&path) {
            files.insert(path, text);
        }
    }
    let g = build::read(&files);
    if g.packages.is_empty() {
        return Err("No workspace here: no Cargo.toml with [package], package.json with a name, Gradle settings file \
             or pom.xml"
            .into());
    }
    // The same graph as files, for the cycles and counts the file graph has.
    let as_files = Graph {
        nodes: g
            .packages
            .iter()
            .map(|p| deps::Node { path: format!("{} ({})", p.name, p.tool), lang: Lang::Rust, test: false })
            .collect(),
        edges: g.deps.iter().map(|d| deps::Edge { from: d.from, to: d.to, line: 1 }).collect(),
        ..Graph::default()
    };
    let cycle_edges = arch::cycle_edges(&as_files);
    let nodes = g
        .packages
        .iter()
        .enumerate()
        .map(|(i, p)| GraphNode {
            id: p.manifest.clone(),
            label: p.name.clone(),
            kind: p.tool.to_lowercase(),
            group: p.tool.to_owned(),
            members: Vec::new(),
            path: p.manifest.clone(),
            line: 1,
            accent: cycle_edges.iter().any(|&(a, b)| a == i || b == i).then_some(RED),
        })
        .collect();
    let edges = g
        .deps
        .iter()
        .map(|d| GraphEdge {
            source: g.packages[d.from].manifest.clone(),
            target: g.packages[d.to].manifest.clone(),
            kind: EdgeKind::Uses,
            label: d.kind.clone(),
            accent: cycle_edges.contains(&(d.from, d.to)).then_some(RED),
        })
        .collect();
    let title = format!("Build: {} packages, {} dependencies", g.packages.len(), g.deps.len());
    host::show_graph(&Shown { id: "build".to_owned(), title, nodes, edges, focus: None });

    let label = |i: usize| format!("{} ({})", g.packages[i].name, g.packages[i].tool);
    let mut tools: Vec<&str> = g.packages.iter().map(|p| p.tool).collect();
    tools.dedup();
    let mut blocks = vec![
        Block::Heading("Build".into()),
        Block::Text(format!(
            "{} packages ({}), {} dependencies between them. Read from the build files; a dependency \
             on something outside the project isn't shown.",
            g.packages.len(),
            tools.join(", "),
            g.deps.len()
        )),
    ];
    let cycles = arch::cycles(&as_files);
    if !cycles.is_empty() {
        blocks.push(Block::Heading(format!("Cycles ({})", cycles.len())));
        blocks.push(Block::Bullets(
            cycles.iter().map(|c| c.iter().map(|&i| label(i)).collect::<Vec<_>>().join(" → ")).collect(),
        ));
    }
    let coupling = arch::coupling(&as_files);
    let mut order: Vec<usize> = (0..g.packages.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(coupling[i].fan_in), label(i)));
    let rows = order
        .iter()
        .map(|&i| {
            vec![
                g.packages[i].name.clone(),
                g.packages[i].tool.to_owned(),
                coupling[i].fan_in.to_string(),
                coupling[i].fan_out.to_string(),
                g.packages[i].manifest.clone(),
            ]
        })
        .collect();
    blocks.push(table(&["Package", "Tool", "Used by", "Uses", "Build file"], rows));
    host::set_panel("report", &blocks);
    host::show_message(&format!("{} packages", g.packages.len()));
    Ok(())
}

/// What a change to `file` reaches.
fn impact(graph: &Graph, file: &str) {
    let Some(i) = graph.index_of(file) else {
        host::show_message(&format!("{file} isn't a source file of a language Architecture reads"));
        return;
    };
    let found = arch::impact(graph, i);
    // Who imports it, near and far, with how many steps away.
    let far: Vec<(usize, usize)> = arch::reach(graph, i, false);
    let mut blocks = vec![
        Block::Heading(format!("Impact of {file}")),
        Block::Text(format!(
            "{} files import it, {} of them directly; {} tests among them.",
            far.len(),
            found.direct.len(),
            found.tests.len()
        )),
    ];
    if !far.is_empty() {
        let rows = far.iter().map(|&(f, d)| vec![graph.nodes[f].path.clone(), d.to_string()]).collect();
        blocks.push(table(&["File", "Steps away"], rows));
    }
    if !found.tests.is_empty() {
        blocks.push(Block::Heading("Tests to run".into()));
        blocks.push(Block::Bullets(found.tests.iter().map(|&t| graph.nodes[t].path.clone()).collect()));
    }
    host::set_panel("report", &blocks);
    // The graph of the file and what reaches it.
    let mut keep: Vec<usize> = far.iter().map(|&(f, _)| f).collect();
    keep.push(i);
    let sub = Graph {
        root: graph.root.clone(),
        nodes: keep.iter().map(|&k| graph.nodes[k].clone()).collect(),
        edges: graph
            .edges
            .iter()
            .filter_map(|e| {
                let from = keep.iter().position(|&k| k == e.from)?;
                let to = keep.iter().position(|&k| k == e.to)?;
                Some(deps::Edge { from, to, line: e.line })
            })
            .collect(),
        truncated: false,
    };
    show_impact(&sub, file);
}

fn show_impact(sub: &Graph, file: &str) {
    let nodes = sub
        .nodes
        .iter()
        .map(|n| GraphNode {
            id: n.path.clone(),
            label: arch::label(&n.path),
            kind: String::new(),
            group: n.path.rsplit_once('/').map_or("", |(d, _)| d).to_owned(),
            members: Vec::new(),
            path: n.path.clone(),
            line: 1,
            accent: if n.path == file {
                Some(0x3B8EEA)
            } else if n.test {
                Some(GREEN)
            } else {
                None
            },
        })
        .collect();
    let edges = sub
        .edges
        .iter()
        .map(|e| GraphEdge {
            source: sub.nodes[e.from].path.clone(),
            target: sub.nodes[e.to].path.clone(),
            kind: EdgeKind::Uses,
            label: String::new(),
            accent: None,
        })
        .collect();
    let name = file.rsplit('/').next().unwrap_or(file);
    let title = format!("Architecture: impact of {name}");
    host::show_graph(&Shown { id: "architecture".to_owned(), title, nodes, edges, focus: Some(file.to_owned()) });
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        match id.as_str() {
            "files" => start(Want::Files),
            "folders" => start(Want::Folders),
            "build" => build_graph(),
            // The file graph's report has the rules.
            "rules" => start(Want::Files),
            "since" => {
                since_form(None);
                Ok(())
            }
            "impact" => {
                let doc = host::active_document()?.ok_or("Open a file first")?;
                if doc.path.is_empty() {
                    return Err("The file in front isn't in the open folder".to_owned());
                }
                start(Want::Impact(doc.path))
            }
            _ => Err(format!("unknown command {id}")),
        }
    }

    fn on_event(event: Event) {
        match event {
            Event::Resumed(token) if token == "scan" => step(),
            Event::Resumed(token) if token == "since" => since_step(),
            Event::JobFinished(job) => since_finished(job),
            Event::FormSubmitted(FormValues { form, values, .. }) if form == "since" => {
                let rev =
                    values.into_iter().find(|(k, _)| k == "rev").map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
                if !rev.is_empty()
                    && let Err(e) = start(Want::Since(rev))
                {
                    since_form(Some(e));
                }
            }
            _ => {}
        }
    }
}

export!(Plugin);
