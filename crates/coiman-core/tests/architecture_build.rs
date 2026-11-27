//! The Architecture extension's Show Build Graph in the real plugin host: a Cargo workspace and
//! an npm one, and this repository itself.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Graph, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.architecture";

fn graph_and_panel(host: &PluginHost) -> (Graph, String) {
    let (mut graph, mut panel) = (None, None);
    let start = Instant::now();
    while (graph.is_none() || panel.is_none()) && start.elapsed() < Duration::from_secs(120) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Graph { graph: g, .. } => graph = Some(g),
                Update::Panel { blocks, .. } => panel = Some(text(&blocks)),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (graph.expect("a graph"), panel.expect("a report"))
}

fn text(blocks: &[Block]) -> String {
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
    host
}

#[test]
fn build_graph() {
    let ws = std::env::temp_dir().join(format!("coiman-arch-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let w = |p: &str, t: &str| {
        let path = ws.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, t).unwrap();
    };
    w("Cargo.toml", "[workspace]\nmembers = [\"core\", \"app\"]\n");
    w("core/Cargo.toml", "[package]\nname = \"core\"\n[dependencies]\napp = { path = \"../app\" }\n");
    w("app/Cargo.toml", "[package]\nname = \"app\"\n[dependencies]\ncore = { path = \"../core\" }\nserde = \"1\"\n");
    w("web/package.json", r#"{"name":"web","dependencies":{"ui":"workspace:*"}}"#);
    w("ui/package.json", r#"{"name":"ui"}"#);

    let host = load(&ws);
    host.run_command(ID, "build", "");
    let (g, report) = graph_and_panel(&host);
    assert_eq!(g.nodes.len(), 4, "{:?}", g.nodes.iter().map(|n| &n.label).collect::<Vec<_>>());
    let edge = |s: &str, t: &str| g.edges.iter().find(|e| e.source == s && e.target == t);
    assert!(edge("web/package.json", "ui/package.json").is_some());
    // core and app use each other: a cycle, in red.
    assert_eq!(edge("app/Cargo.toml", "core/Cargo.toml").unwrap().accent, Some(0xE5484D));
    assert!(report.contains("4 packages (Cargo, npm), 3 dependencies between them."), "{report}");
    assert!(report.contains("Cycles (1)") && report.contains("app (Cargo) → core (Cargo)"), "{report}");
    assert!(report.contains("ui | npm | 1 | 0 | ui/package.json"), "{report}");
    let _ = std::fs::remove_dir_all(&ws);

    // This repository: its three crates, the app on the core.
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let host = load(&repo);
    host.run_command(ID, "build", "");
    let (g, report) = graph_and_panel(&host);
    let id = |p: &str| g.nodes.iter().find(|n| n.id == p).map(|n| n.id.clone());
    let (ide, core) = (id("crates/coiman-ide/Cargo.toml").unwrap(), id("crates/coiman-core/Cargo.toml").unwrap());
    assert!(g.edges.iter().any(|e| e.source == ide && e.target == core), "{report}");
    assert!(!report.contains("Cycles"), "{report}");
}
