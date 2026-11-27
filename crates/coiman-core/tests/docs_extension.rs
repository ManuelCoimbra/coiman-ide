//! The Project Docs extension (`plugins/extensions/docs`) in the real plugin host, with the real
//! `git`: a repository whose guide names a file changed after it, a broken link and a
//! document nothing links to.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.docs";

fn wait_panel(host: &PluginHost, until: &str) -> String {
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
            Block::Spans(s) => {
                out.push_str(&s.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} ({}) -> {}:{}\n", l.text, l.detail, l.path, l.line));
                }
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {}]", b.label, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            _ => {}
        }
    }
    out
}

/// git in `dir`, its name and dates given for this repository only.
fn git(dir: &Path, date: &str, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

#[test]
fn docs_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-docs-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(ws.join("docs")).unwrap();
    std::fs::create_dir_all(ws.join("src")).unwrap();
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    git(&ws, "2024-01-01T10:00:00", &["init", "-q"]);
    w("README.md", "# Demo\n\nRead the [guide](docs/guide.md).\n");
    w(
        "docs/guide.md",
        "# Guide\n\n## Running\nStart with `src/main.rs`.\n\nSee [setup](setup.md#install), [gone](old.md) and [here](#running).\n",
    );
    w("docs/setup.md", "# Setup\n## Install\n");
    w("docs/lonely.md", "# Lonely\n");
    w("src/main.rs", "fn main() {}\n");
    git(&ws, "2024-01-01T10:00:00", &["add", "-A"]);
    git(&ws, "2024-01-01T10:00:00", &["commit", "-q", "-m", "first"]);
    // The code changes later; the guide doesn't.
    w("src/main.rs", "fn main() { println!(\"hi\"); }\n");
    git(&ws, "2025-06-01T10:00:00", &["commit", "-q", "-am", "second"]);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/docs");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());

    host.run_command(ID, "index", "");
    let index = wait_panel(&host, "4 documents");
    assert!(
        index.contains("Guide | docs/guide.md | 2 headings · 12 words")
            && index.contains("Lonely | docs/lonely.md | 1 heading · 2 words"),
        "{index}"
    );
    assert!(index.contains("[Headings docs/guide.md]"), "{index}");
    host.run_command(ID, "doc", "docs/guide.md");
    let heads = wait_panel(&host, "docs/guide.md: headings");
    assert!(heads.contains("  Running (#running) -> docs/guide.md:3"), "{heads}");

    host.run_command(ID, "tab", "links");
    let links = wait_panel(&host, "Broken links (1)");
    assert!(links.contains("docs/guide.md:6  old.md (docs/old.md isn't there) -> docs/guide.md:6"), "{links}");
    assert!(links.contains("Not linked from any document (1)") && links.contains("docs/lonely.md"), "{links}");

    host.run_command(ID, "tab", "outdated");
    let old = wait_panel(&host, "src/main.rs");
    assert!(old.contains("docs/guide.md:4  src/main.rs (the document 2024-01-01, the file 2025-06-01)"), "{old}");
    let _ = std::fs::remove_dir_all(&ws);
}
