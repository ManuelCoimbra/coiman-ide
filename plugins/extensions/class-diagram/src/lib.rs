//! Class Diagram: the classes, interfaces, enums, structs and traits of the project, read from
//! its source (Java, Kotlin, C#, TypeScript and JavaScript, C and C++, Swift, Dart, Scala,
//! PHP, Python, Rust, Go), drawn as a UML class diagram by the editor.
//!
//! - Class Diagram: Whole Project: every type (the most related ones when there are many).
//! - Class Diagram: Current File: the types of the file in front, their parents up to the
//!   top, their children, and what they hold or are held by.
//!
//! Files are read in steps (`continue-later`), so a large project never runs past a call's
//! time limit and the editor stays responsive.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod build;
mod parse;

use std::sync::Mutex;

use build::{Built, FileTypes, Kind, Scope};
use coiman::plugin::host;
use coiman::plugin::types::{EdgeKind, Graph, GraphEdge, GraphNode};

/// Files read per step at most, and the text read per step (a call has a time limit).
const STEP: usize = 120;
const STEP_BYTES: usize = 1 << 20;
/// Source files looked at, at most.
const MAX_FILES: u32 = 20_000;
/// Boxes drawn for the whole project, at most.
const MAX_TYPES: usize = 400;

enum Want {
    Project,
    File(String),
}

struct Scan {
    want: Want,
    files: Vec<String>,
    next: usize,
    found: Vec<FileTypes>,
}

static SCAN: Mutex<Option<Scan>> = Mutex::new(None);

fn graph_of(built: Built, title: String) -> Graph {
    Graph {
        id: "classes".to_owned(),
        title,
        nodes: built
            .nodes
            .into_iter()
            .map(|n| GraphNode {
                id: n.id,
                label: n.label,
                kind: n.kind,
                group: n.group,
                members: n.members,
                path: n.path,
                line: n.line,
            })
            .collect(),
        edges: built
            .edges
            .into_iter()
            .map(|e| GraphEdge {
                source: e.source,
                target: e.target,
                kind: match e.kind {
                    Kind::Inherits => EdgeKind::Inherits,
                    Kind::Implements => EdgeKind::Implements,
                    Kind::Has => EdgeKind::Has,
                },
                label: e.label,
            })
            .collect(),
        focus: built.focus,
    }
}

fn start(want: Want) -> Result<(), String> {
    let files: Vec<String> =
        host::list_files("", MAX_FILES)?.into_iter().filter(|f| parse::Lang::of(f).is_some()).collect();
    if files.is_empty() {
        return Err("No source files of a language it reads in this folder".to_owned());
    }
    host::show_message(&format!("Reading the classes of {} files…", files.len()));
    *SCAN.lock().unwrap_or_else(|e| e.into_inner()) = Some(Scan { want, files, next: 0, found: Vec::new() });
    host::continue_later("scan");
    Ok(())
}

/// Reads the next files; shows the diagram once all are read.
fn step() {
    let mut guard = SCAN.lock().unwrap_or_else(|e| e.into_inner());
    let Some(scan) = guard.as_mut() else { return };
    if scan.next < scan.files.len() {
        let mut read = 0;
        let mut count = 0;
        while scan.next < scan.files.len() && count < STEP && read < STEP_BYTES {
            let path = &scan.files[scan.next];
            scan.next += 1;
            count += 1;
            let Some(lang) = parse::Lang::of(path) else { continue };
            let Ok(text) = host::read_file(path) else { continue };
            read += text.len();
            let types = parse::parse(lang, &text);
            if !types.is_empty() {
                scan.found.push(FileTypes { path: path.clone(), types });
            }
        }
        host::show_message(&format!("Reading the classes: {} of {} files", scan.next, scan.files.len()));
        // The diagram is made in a step of its own.
        host::continue_later("scan");
        return;
    }
    let Some(scan) = guard.take() else { return };
    drop(guard);
    let (built, title) = match &scan.want {
        Want::Project => {
            let built = build::build(&scan.found, Scope::All(MAX_TYPES));
            let more = if built.dropped > 0 { format!(", the {MAX_TYPES} most related") } else { String::new() };
            let title = format!("Class Diagram: {} types{more}", built.nodes.len());
            (built, title)
        }
        Want::File(file) => {
            let built = build::build(&scan.found, Scope::File(file));
            let name = file.rsplit('/').next().unwrap_or(file);
            (built, format!("Class Diagram: {name}"))
        }
    };
    if built.nodes.is_empty() {
        host::show_message(match scan.want {
            Want::Project => "No classes found",
            Want::File(_) => "No classes in this file",
        });
        return;
    }
    host::show_message(&format!("{} types, {} relations", built.nodes.len(), built.edges.len()));
    host::show_graph(&graph_of(built, title));
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        match id.as_str() {
            "project" => start(Want::Project),
            "file" => {
                let doc = host::active_document()?.ok_or("Open a file first")?;
                if doc.path.is_empty() {
                    return Err("The file in front isn't in the open folder".to_owned());
                }
                if parse::Lang::of(&doc.path).is_none() {
                    return Err(format!("{} isn't a language Class Diagram reads", doc.path));
                }
                start(Want::File(doc.path))
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
