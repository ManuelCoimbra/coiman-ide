//! The Code Health extension (`plugins/extensions/code-health`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: the checks of a small project read in
//! steps, its tabs, complexity from the editor's parser, the overview page, and programs it may
//! only run once approved.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.code-health";

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
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

/// The panel's text, when it shows `until`.
fn panel(host: &PluginHost, until: &str) -> String {
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            if last.contains(until) {
                return last;
            }
        }
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Bullets(items) => out.push_str(&format!("{}\n", items.join("\n"))),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} {} -> {}:{}\n", l.text, l.detail, l.path, l.line));
                }
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn code_health_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-healthext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let w = |p: &str, t: &str| {
        let path = ws.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, t).unwrap();
    };
    // Put together at run time: a key-shaped literal here would trip push protection.
    let stripe = ["sk", "live", "abcdefghijklmnopqrstuvwx"].join("_");
    w(".env", &format!("STRIPE={stripe}\n"));
    w("README.md", "# Demo\n\nA small project.\n");
    w("Cargo.toml", "[package]\nname = \"demo\"\n");
    let busy = "pub fn busy(x: i32) -> i32 {\n    if x > 1 { 1 } else if x > 2 { 2 } else { 3 }\n}\n";
    let block = "    let a = 1;\n    let b = a + 2;\n    let c = b * 3;\n    let d = c - 4;\n    let e = d / 5;\n    let f = e % 6;\n    let g = f + 7;\n";
    w("src/lib.rs", &format!("/// Said.\npub fn said() {{}}\n\n{busy}\nfn one() {{\n{block}}}\n"));
    w("src/other.rs", &format!("fn two() {{\n{block}}}\n"));
    w(
        "Cargo.lock",
        "version = 3\n\n[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\ndependencies = [\"serde\"]\n\n\
         [[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+x\"\n",
    );
    w("node_modules/left-pad/package.json", r#"{"name":"left-pad","version":"1.3.0","license":"WTFPL"}"#);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/code-health");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    // Reads files; no program approved.
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load(pkg.manifest, pkg.wasm, granted);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    host.run_command(ID, "check", "");
    let secrets = panel(&host, "1 secret that look");
    assert!(secrets.contains(".env:1 Stripe secret key") && secrets.contains("-> .env:1"), "{secrets}");
    assert!(secrets.contains("[▸ Secrets  1 tab secrets]"), "{secrets}");

    host.run_command(ID, "tab", "complexity");
    let complexity = panel(&host, "Cyclomatic");
    assert!(complexity.contains("3 | 3 | busy | src/lib.rs:4 [Open open src/lib.rs:4]"), "{complexity}");

    host.run_command(ID, "tab", "duplicates");
    let dups = panel(&host, "written more than once");
    assert!(dups.contains("src/lib.rs:") && dups.contains("src/other.rs:"), "{dups}");

    host.run_command(ID, "tab", "undocumented");
    let docs = panel(&host, "doc comment");
    assert!(docs.contains("busy") && !docs.contains("function said"), "{docs}");

    host.run_command(ID, "tab", "dependencies");
    let deps = panel(&host, "Cargo.lock: 2 packages");
    assert!(deps.contains("[Check for updates updates ]"), "{deps}");

    host.run_command(ID, "tab", "licences");
    let lic = panel(&host, "left-pad 1.3.0");
    // cargo isn't approved: said, and npm's are still listed.
    assert!(lic.contains("program not approved: cargo"), "{lic}");

    // Updating runs only an approved program.
    host.run_command(ID, "update", "Cargo|serde");
    match wait(&host, |u| matches!(u, Update::Message { text, .. } if text.contains("approved"))) {
        Update::Message { text, .. } => assert!(text.contains("program not approved: cargo"), "{text}"),
        _ => unreachable!(),
    }

    host.run_command(ID, "overview", "");
    match wait(&host, |u| matches!(u, Update::Markdown { .. })) {
        Update::Markdown { title, text, .. } => {
            assert!(title.ends_with(": overview"), "{title}");
            assert!(text.contains("**Demo**: A small project."), "{text}");
            assert!(text.contains("| Rust | 2 |") && text.contains("Cargo (`Cargo.toml`)"), "{text}");
            assert!(!text.contains("## Git"), "git isn't approved: {text}");
        }
        _ => unreachable!(),
    }
    let _ = std::fs::remove_dir_all(&ws);
}
