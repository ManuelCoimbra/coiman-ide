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
mod deps;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, EdgeKind, Graph as Shown, GraphEdge, GraphNode, Table};
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
}

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
    }
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
    let unused = arch::isolated(graph);
    if !unused.is_empty() && !folders {
        blocks.push(Block::Heading(format!("Unused ({})", unused.len())));
        blocks.push(Block::Text("Nothing imports them and they import nothing (entry points and scripts too).".into()));
        blocks.push(Block::Bullets(unused.iter().take(ROWS * 2).map(|&i| name(i)).collect()));
    }
    host::set_panel("report", &blocks);
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
        if let Event::Resumed(token) = event
            && token == "scan"
        {
            step();
        }
    }
}

export!(Plugin);
