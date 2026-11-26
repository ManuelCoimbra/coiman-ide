//! The Architecture extension's Changes Since in the real plugin host, with the real `git`: a
//! repository of two commits, and how its imports changed between them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.architecture";

/// The panel's text once it holds `until`.
fn panel(host: &PluginHost, until: &str) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(120) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => last = text(&blocks),
                _ => {}
            }
        }
        if last.contains(until) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the panel never said {until:?}; last:\n{last}");
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

/// git in `dir`, with a name of its own for this repository only.
fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

#[test]
fn architecture_changes_since() {
    let ws = std::env::temp_dir().join(format!("coiman-arch-since-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("src")).unwrap();
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    git(&ws, &["init", "-q"]);
    // Before: main → parser → util.
    w("Cargo.toml", "[package]\nname = \"demo\"\n");
    w("src/main.rs", "mod parser;\nmod util;\nmod old;\nuse crate::parser::parse;\nfn main() { parse(); }\n");
    w("src/parser.rs", "use crate::util::helper;\npub fn parse() { helper(); }\n");
    w("src/util.rs", "pub fn helper() {}\n");
    w("src/old.rs", "use crate::util::helper;\npub fn o() { helper(); }\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "first"]);
    // Now: util imports parser (a cycle), old.rs is gone, fresh.rs is new and imports util.
    w("src/main.rs", "mod parser;\nmod util;\nmod fresh;\nuse crate::parser::parse;\nfn main() { parse(); }\n");
    w("src/util.rs", "use crate::parser::parse;\npub fn helper() {}\n");
    std::fs::remove_file(ws.join("src/old.rs")).unwrap();
    w("src/fresh.rs", "use crate::util::helper;\npub fn f() { helper(); }\n");
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "second"]);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/architecture");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    assert_eq!(pkg.manifest.programs.iter().collect::<Vec<_>>(), ["git"]);
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());

    host.run_command(ID, "since", "");
    panel(&host, "Changes since");
    let values = vec![("rev".to_owned(), "HEAD~1".to_owned())];
    host.deliver(ID, Event::FormSubmitted(FormValues { form: "since".into(), button: "Compare".into(), values }));
    let report = panel(&host, "Changes since HEAD~1");
    assert!(report.contains("2 imports added, 1 removed; 1 new cycles, 0 gone; 1 files added, 1 removed."), "{report}");
    assert!(report.contains("src/parser.rs ⇄ src/util.rs"), "{report}");
    assert!(
        report.contains("src/util.rs | src/parser.rs") && report.contains("src/fresh.rs | src/util.rs"),
        "{report}"
    );
    assert!(report.contains("src/old.rs | src/util.rs"), "{report}");

    // A revision git doesn't know: it says so.
    let values = vec![("rev".to_owned(), "no-such-branch".to_owned())];
    host.deliver(ID, Event::FormSubmitted(FormValues { form: "since".into(), button: "Compare".into(), values }));
    panel(&host, "no-such-branch");
    let _ = std::fs::remove_dir_all(&ws);
}
