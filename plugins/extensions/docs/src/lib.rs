//! Project Docs: the project's Markdown documents, read without a build, in a panel: an index
//! with their titles and headings; the links between them, the broken ones and the documents
//! nothing links to, and their graph; and the documents older than the code they mention,
//! from `git log`.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod md;

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, EdgeKind, FileLink, Graph, GraphEdge, GraphNode, JobResult, Row, Span};
use md::Doc;

const MAX_FILES: u32 = 20_000;
/// Documents read per step, and bytes of one read at most.
const STEP: usize = 100;
const MAX_DOC: usize = 1 << 20;
const RED: u32 = 0xE5484D;
const AMBER: u32 = 0xE2A33D;
const DIM: u32 = 0x8B949E;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Index,
    Links,
    Outdated,
}

const TABS: [(Tab, &str, &str); 3] =
    [(Tab::Index, "index", "Index"), (Tab::Links, "links", "Links"), (Tab::Outdated, "outdated", "Outdated")];

#[derive(Default)]
struct State {
    files: BTreeSet<String>,
    paths: Vec<String>,
    next: usize,
    reading: bool,
    docs: Vec<Doc>,
    /// When each path last changed, from git; why not, when it couldn't say.
    changed: Option<HashMap<String, i64>>,
    git_note: Option<String>,
    git_job: Option<u64>,
    tab: Tab,
    /// The document whose headings are open in the index.
    open: Option<String>,
    /// Show the graph once read.
    graph_wanted: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn link(text: String, detail: String, path: &str, line: usize) -> FileLink {
    FileLink { text, detail, path: path.to_owned(), line: line as u32 }
}

fn date(t: i64) -> String {
    // Civil from days (Howard Hinnant's algorithm).
    let z = t.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

// ----- reading -------------------------------------------------------------------------------

fn scan(graph: bool) -> Result<(), String> {
    let files: BTreeSet<String> = host::list_files("", MAX_FILES)?.into_iter().collect();
    let paths: Vec<String> = files.iter().filter(|f| md::is_doc(f)).cloned().collect();
    if paths.is_empty() {
        return Err("No Markdown documents (.md) in the open folder".into());
    }
    with(|s| {
        let (tab, open) = (s.tab, s.open.take());
        *s = State { files, paths, reading: true, tab, open, graph_wanted: graph, ..State::default() };
    });
    host::show_message("Reading the documents…");
    render();
    host::continue_later("scan");
    Ok(())
}

fn step() {
    let done = with(|s| {
        let end = (s.next + STEP).min(s.paths.len());
        for path in &s.paths[s.next..end] {
            let Ok(mut text) = host::read_file(path) else { continue };
            if text.len() > MAX_DOC {
                let mut cut = MAX_DOC;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                text.truncate(cut);
            }
            s.docs.push(md::read(path, &text));
        }
        s.next = end;
        s.next >= s.paths.len()
    });
    if !done {
        host::continue_later("scan");
        return;
    }
    // When each file last changed: one git log, if git is approved and this is a repository.
    let args = ["log", "--name-only", "--no-renames", "--format=%x00%ct"].map(str::to_owned);
    let started = host::start_job("git", &args, "");
    let graph = with(|s| {
        s.reading = false;
        match started {
            Ok(id) => s.git_job = Some(id),
            Err(e) => s.git_note = Some(format!("Outdated documents need git's history: {e}")),
        }
        std::mem::take(&mut s.graph_wanted)
    });
    if graph {
        show_graph();
    }
    host::show_message("Documents read");
    render();
}

fn git_done(job: JobResult) {
    let mine = with(|s| s.git_job == Some(job.id));
    if !mine {
        return;
    }
    with(|s| {
        s.git_job = None;
        if job.exit_code == Some(0) {
            s.changed = Some(md::last_changes(&job.stdout));
        } else {
            let err = job.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("git log failed").to_owned();
            s.git_note = Some(format!("Outdated documents need git's history: {err}"));
        }
    });
    render();
}

// ----- the panel -----------------------------------------------------------------------------

fn render() {
    let blocks = with(|s| {
        let broken = md::broken(&s.docs, &s.files);
        let stale = s.changed.as_ref().map(|c| md::outdated(&s.docs, c)).unwrap_or_default();
        let mut tabs: Vec<Button> = TABS
            .iter()
            .map(|(tab, id, name)| {
                let n = match tab {
                    Tab::Index => s.docs.len(),
                    Tab::Links => broken.len(),
                    Tab::Outdated => stale.len(),
                };
                let mark = if *tab == s.tab { "▸ " } else { "" };
                button(&if n > 0 { format!("{mark}{name}  {n}") } else { format!("{mark}{name}") }, "tab", id)
            })
            .collect();
        tabs.push(button("Graph", "graph", ""));
        tabs.push(button("Read again", "rescan", ""));
        let mut b = vec![Block::Rows(vec![Row { cells: vec![], buttons: tabs }])];
        if s.reading {
            b.push(Block::Text(format!("Reading the documents: {} of {}…", s.next, s.paths.len())));
            return b;
        }
        match s.tab {
            Tab::Index => index(s, &mut b),
            Tab::Links => links(s, &broken, &mut b),
            Tab::Outdated => outdated(s, &stale, &mut b),
        }
        b
    });
    host::set_panel("docs", &blocks);
}

fn index(s: &State, b: &mut Vec<Block>) {
    let words: usize = s.docs.iter().map(|d| d.words).sum();
    b.push(Block::Text(format!(
        "{} documents, {words} words. A title opens the document; Headings lists its headings.",
        s.docs.len()
    )));
    let rows = s
        .docs
        .iter()
        .map(|d| {
            let changed = s.changed.as_ref().and_then(|c| c.get(&d.path)).map(|t| date(*t)).unwrap_or_default();
            let open = s.open.as_deref() == Some(d.path.as_str());
            Row {
                cells: vec![
                    Span { text: d.title.clone(), color: None, bold: true, monospace: false },
                    span(&d.path, Some(DIM)),
                    span(format!("{} headings · {} words", d.headings.len(), d.words), Some(DIM)),
                    span(changed, Some(DIM)),
                ],
                buttons: vec![
                    button("Open", "open", &format!("{}:1", d.path)),
                    button(if open { "Hide headings" } else { "Headings" }, "doc", &d.path),
                ],
            }
        })
        .collect();
    b.push(Block::Rows(rows));
    if let Some(d) = s.open.as_ref().and_then(|p| s.docs.iter().find(|d| &d.path == p)) {
        b.push(Block::Heading(format!("{}: headings", d.path)));
        b.push(Block::Links(
            d.headings
                .iter()
                .map(|h| {
                    link(format!("{}{}", "  ".repeat(h.level - 1), h.text), format!("#{}", h.slug), &d.path, h.line)
                })
                .collect(),
        ));
    }
}

fn links(s: &State, broken: &[md::Broken], b: &mut Vec<Block>) {
    let between = md::doc_links(&s.docs).len();
    let web = s.docs.iter().flat_map(|d| &d.links).filter(|l| md::resolve("", &l.target).is_none()).count();
    b.push(Block::Text(format!(
        "{between} links between documents, {web} to the web (not checked: Coiman opens no connections). Graph draws them."
    )));
    if broken.is_empty() {
        b.push(Block::Text("No broken links.".into()));
    } else {
        b.push(Block::Heading(format!("Broken links ({})", broken.len())));
        b.push(Block::Links(
            broken
                .iter()
                .map(|x| link(format!("{}:{}  {}", x.doc, x.line, x.target), x.why.clone(), &x.doc, x.line))
                .collect(),
        ));
    }
    let orphans = md::orphans(&s.docs);
    if !orphans.is_empty() {
        b.push(Block::Heading(format!("Not linked from any document ({})", orphans.len())));
        b.push(Block::Links(orphans.iter().map(|p| link(p.clone(), String::new(), p, 1)).collect()));
    }
}

fn outdated(s: &State, stale: &[md::Stale], b: &mut Vec<Block>) {
    if let Some(note) = &s.git_note {
        b.push(Block::Spans(vec![span(note, Some(AMBER))]));
        return;
    }
    if s.changed.is_none() {
        b.push(Block::Spans(vec![span("Reading git's history…", Some(DIM))]));
        return;
    }
    b.push(Block::Text(
        "Documents that mention a file (in `code`, or by a link) changed in git after the document last was: \
         they may describe it as it was."
            .into(),
    ));
    if stale.is_empty() {
        b.push(Block::Text("None: every document changed after the files it names.".into()));
        return;
    }
    b.push(Block::Links(
        stale
            .iter()
            .map(|x| {
                link(
                    format!("{}:{}  {}", x.doc, x.line, x.file),
                    format!("the document {}, the file {}", date(x.doc_changed), date(x.file_changed)),
                    &x.doc,
                    x.line,
                )
            })
            .collect(),
    ));
}

fn show_graph() {
    let graph = with(|s| {
        let broken: BTreeSet<String> = md::broken(&s.docs, &s.files).into_iter().map(|b| b.doc).collect();
        let nodes = s
            .docs
            .iter()
            .map(|d| GraphNode {
                id: d.path.clone(),
                label: d.title.clone(),
                kind: "doc".into(),
                group: d.path.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned(),
                members: d.headings.iter().filter(|h| h.level <= 2).take(12).map(|h| h.text.clone()).collect(),
                path: d.path.clone(),
                line: 1,
                accent: broken.contains(&d.path).then_some(RED),
            })
            .collect();
        let edges = md::doc_links(&s.docs)
            .into_iter()
            .map(|(from, to)| GraphEdge {
                source: from,
                target: to,
                kind: EdgeKind::Uses,
                label: String::new(),
                accent: None,
            })
            .collect();
        Graph { id: "docs".into(), title: format!("Documents: {}", s.docs.len()), nodes, edges, focus: None }
    });
    host::show_graph(&graph);
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let read = with(|s| !s.docs.is_empty() || s.reading);
    let set_tab = |tab: Tab| with(|s| s.tab = tab);
    match id {
        "index" | "links" | "outdated" => {
            set_tab(match id {
                "links" => Tab::Links,
                "outdated" => Tab::Outdated,
                _ => Tab::Index,
            });
            if read { render() } else { scan(false)? }
        }
        "graph" => {
            if read {
                show_graph()
            } else {
                scan(true)?
            }
        }
        "rescan" => scan(false)?,
        "tab" => {
            if let Some((tab, ..)) = TABS.iter().find(|(_, i, _)| *i == arg) {
                set_tab(*tab);
            }
            render();
        }
        "doc" => {
            with(|s| s.open = if s.open.as_deref() == Some(arg) { None } else { Some(arg.to_owned()) });
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
            Event::JobFinished(job) => git_done(job),
            _ => {}
        }
    }
}

export!(Plugin);
