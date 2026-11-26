//! The Architecture extension (`plugins/extensions/architecture`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: the dependency graph, by file and by
//! folder, its report panel, the impact of a file, and a whole real project (this one) read
//! in steps within the time limit of each call.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Document, Graph, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.architecture";

thread_local! {
    /// Updates polled but not waited for yet.
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(120) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
                }
                if let Update::Message { text, .. } = &u {
                    eprintln!("extension: {text}");
                }
                if want(&u) {
                    return Some(u);
                }
            }
            None
        });
        if let Some(u) = found {
            return u;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

/// The next graph, and the report panel shown with it.
fn graph_and_panel(host: &PluginHost) -> (Graph, Vec<Block>) {
    let mut graph = None;
    let mut panel = None;
    while graph.is_none() || panel.is_none() {
        match wait(host, |u| matches!(u, Update::Graph { .. } | Update::Panel { .. })) {
            Update::Graph { graph: g, .. } => graph = Some(g),
            Update::Panel { blocks, .. } => panel = Some(blocks),
            _ => unreachable!(),
        }
    }
    (graph.unwrap(), panel.unwrap())
}

fn texts(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Bullets(items) => out.push_str(&format!("{}\n", items.join("\n"))),
            Block::Table(t) => {
                for row in &t.rows {
                    out.push_str(&format!("{}\n", row.join(" | ")));
                }
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} ({}) -> {}:{}\n", l.text, l.detail, l.path, l.line));
                }
            }
            _ => {}
        }
    }
    out
}

fn load(ws: &Path) -> PluginHost {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/architecture");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.to_path_buf()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load(pkg.manifest, pkg.wasm, granted);
    wait(&host, |u| matches!(u, Update::Activated { .. }));
    host
}

#[test]
fn architecture_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-archext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let w = |p: &str, t: &str| {
        let path = ws.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, t).unwrap();
    };
    w("Cargo.toml", "[package]\nname = \"demo\"\n");
    w("src/main.rs", "mod parser;\nmod util;\nuse crate::parser::parse;\nfn main() { parse(); }\n");
    w("src/parser.rs", "use crate::util::helper;\npub fn parse() { helper(); }\n");
    w("src/util.rs", "use crate::parser::parse;\npub fn helper() {}\n");
    w("src/lonely.rs", "pub fn alone() {}\n");
    w("tests/it.rs", "use demo::parser;\n#[test]\nfn t() {}\n");
    let host = load(&ws);

    host.run_command(ID, "files", "");
    let (g, panel) = graph_and_panel(&host);
    let ids: BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains("src/main.rs") && ids.contains("src/parser.rs") && ids.contains("src/util.rs"), "{ids:?}");
    let edge = |s: &str, t: &str| g.edges.iter().find(|e| e.source == s && e.target == t);
    assert!(edge("src/main.rs", "src/parser.rs").is_some());
    // parser and util import each other: a cycle, in red.
    let cyc = edge("src/parser.rs", "src/util.rs").expect("parser uses util");
    assert_eq!(cyc.accent, Some(0xE5484D));
    assert!(edge("src/main.rs", "src/parser.rs").unwrap().accent.is_none());
    let report = texts(&panel);
    assert!(report.contains("Cycles (1)"), "{report}");
    assert!(
        report.contains("src/parser.rs → src/util.rs") || report.contains("src/util.rs → src/parser.rs"),
        "{report}"
    );
    assert!(report.contains("src/lonely.rs"), "{report}");
    assert!(report.contains("No rules for this project."), "{report}");

    // The project's rules: util stays at the bottom, so its import of parser breaks one.
    w(
        ".coiman/architecture.toml",
        "[[rule]]\nfrom = \"src/util.rs\"\nnot = [\"src/parser.rs\"]\nwhy = \"util stays at the bottom\"\n",
    );
    host.run_command(ID, "rules", "");
    let (_, panel) = graph_and_panel(&host);
    let report = texts(&panel);
    assert!(report.contains("Rules broken (1)"), "{report}");
    assert!(report.contains("src/util.rs:1 → src/parser.rs (util stays at the bottom) -> src/util.rs:1"), "{report}");
    // Layers, top down: main may use parser, but parser may not import main's layer.
    w(".coiman/architecture.toml", "layers = [\"src/main.rs\", \"src/*.rs\"]\n");
    host.run_command(ID, "rules", "");
    let (_, panel) = graph_and_panel(&host);
    let report = texts(&panel);
    assert!(report.contains("Rules broken (0)") && report.contains("(2 layers, 0 rules)"), "{report}");
    std::fs::remove_file(ws.join(".coiman/architecture.toml")).unwrap();

    host.run_command(ID, "folders", "");
    let (g, _) = graph_and_panel(&host);
    assert!(g.nodes.iter().any(|n| n.id == "src"), "{:?}", g.nodes.iter().map(|n| &n.id).collect::<Vec<_>>());
    assert!(g.title.contains("folders"));

    host.set_document(Some(Document {
        path: "src/util.rs".into(),
        language: "Rust".into(),
        text: String::new(),
        line: 1,
        column: 1,
        dirty: false,
        selection: String::new(),
    }));
    host.run_command(ID, "impact", "");
    let (g, panel) = graph_and_panel(&host);
    assert_eq!(g.focus.as_deref(), Some("src/util.rs"));
    let report = texts(&panel);
    assert!(report.contains("Impact of src/util.rs"), "{report}");
    assert!(report.contains("src/parser.rs | 1") && report.contains("src/main.rs | 2"), "{report}");
    let _ = std::fs::remove_dir_all(&ws);

    // A real project: this repository, read and followed in steps, no call past its limit.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    host.set_workspace(Some(repo));
    let start = Instant::now();
    host.run_command(ID, "files", "");
    let (g, _) = graph_and_panel(&host);
    assert!(g.nodes.len() > 200 && g.edges.len() > 500, "{}", g.title);
    println!("this repository: {} in {:?}", g.title, start.elapsed());
}
