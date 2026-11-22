//! The Class Diagram extension (`plugins/extensions/class-diagram`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: reading a project in steps
//! (`continue-later`), and the graph it shows (`show-graph`).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Document, EdgeKind, Graph, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.class-diagram";

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            if want(&u) {
                return u;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

fn graph(host: &PluginHost, messages: &mut Vec<String>) -> Graph {
    match wait(host, |u| {
        if let Update::Message { text, .. } = u {
            messages.push(text.clone());
        }
        matches!(u, Update::Graph { .. })
    }) {
        Update::Graph { graph, .. } => graph,
        _ => unreachable!(),
    }
}

#[test]
fn class_diagram_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-classdiag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("src/shop")).unwrap();
    std::fs::create_dir_all(ws.join("py")).unwrap();
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    w(
        "src/shop/Animal.java",
        "package shop;\npublic abstract class Animal implements Comparable<Animal> {\n  protected String name;\n  abstract void speak();\n}\n",
    );
    w(
        "src/shop/Dog.java",
        "package shop;\npublic class Dog extends Animal {\n  private Collar collar;\n  void speak() {}\n}\n",
    );
    w("src/shop/Collar.java", "package shop;\npublic class Collar {}\n");
    w(
        "py/models.py",
        "class Base:\n    pass\nclass User(Base):\n    def __init__(self, name):\n        self.name = name\n",
    );
    // Enough files that reading them takes several steps.
    for i in 0..300 {
        w(&format!("src/shop/Gen{i}.java"), &format!("package shop;\nclass Gen{i} extends Animal {{}}\n"));
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/class-diagram");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load(pkg.manifest, pkg.wasm, granted);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    let mut messages = Vec::new();
    host.run_command(ID, "project", "");
    let g = graph(&host, &mut messages);
    assert!(messages.iter().any(|m| m.starts_with("Reading the classes: 120 of 304")), "{messages:?}");
    // 303 Java classes, 2 Python ones and the external Comparable.
    assert_eq!(g.nodes.len(), 306, "{}", g.title);
    let node = |label: &str| g.nodes.iter().find(|n| n.label == label).unwrap_or_else(|| panic!("no {label}"));
    let dog = node("Dog");
    assert_eq!((dog.path.as_str(), dog.line, dog.group.as_str()), ("src/shop/Dog.java", 2, "shop"));
    assert_eq!(dog.members, ["- collar: Collar", "~ speak()"]);
    assert_eq!(node("Animal").kind, "abstract");
    assert_eq!(node("Comparable").kind, "external");
    let edge = |s: &str, t: &str, k: EdgeKind| {
        let (s, t) = (&node(s).id, &node(t).id);
        g.edges.iter().any(|e| &e.source == s && &e.target == t && e.kind == k)
    };
    assert!(edge("Dog", "Animal", EdgeKind::Inherits));
    assert!(edge("Animal", "Comparable", EdgeKind::Implements));
    assert!(edge("Dog", "Collar", EdgeKind::Has));
    assert!(edge("User", "Base", EdgeKind::Inherits));
    assert!(edge("Gen7", "Animal", EdgeKind::Inherits));

    // The file in front: its classes and those related to them.
    host.set_document(Some(Document {
        path: "py/models.py".into(),
        language: "Python".into(),
        text: String::new(),
        line: 1,
        column: 1,
        dirty: false,
    }));
    host.run_command(ID, "file", "");
    let g = graph(&host, &mut messages);
    let labels: BTreeSet<&str> = g.nodes.iter().map(|n| n.label.as_str()).collect();
    assert_eq!(labels, ["Base", "User"].into());
    assert_eq!(g.title, "Class Diagram: models.py");
    assert!(g.focus.as_deref().is_some_and(|f| f.ends_with("#Base")));
    let _ = std::fs::remove_dir_all(&ws);
}
