//! The Code Review extension (`plugins/extensions/review`) in the real plugin host with the real
//! `git`: the changes against main, a comment on a line shown above it and following it when
//! lines are added, a reply, the verdict, the Markdown export, resolving and deleting.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{
    Block, Document, Event, EventKind, FormValues, Permission, PluginHost, Update, registry,
};

const ID: &str = "coiman.review";

/// What the host sent since: the panel's text, the last code lenses (line, title), opened text.
#[derive(Default)]
struct Seen {
    panel: String,
    lenses: Vec<(u32, String)>,
    opened: String,
}

fn wait(host: &PluginHost, seen: &mut Seen, what: &str, until: impl Fn(&Seen) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => seen.panel = text(&blocks),
                Update::CodeLenses { lenses, .. } => {
                    seen.lenses = lenses.into_iter().map(|l| (l.line, l.title)).collect();
                }
                Update::OpenText(t) => seen.opened = t,
                _ => {}
            }
        }
        if until(seen) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("never: {what}\npanel:\n{}\nlenses: {:?}", seen.panel, seen.lenses);
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Diff(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(s) => {
                out.push_str(&s.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {}]", b.label, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => out.push_str(&format!("<form {}>\n", f.id)),
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

fn form_id(panel: &str, kind: &str) -> String {
    let at = panel.find(&format!("<form {kind}#")).expect("the form") + "<form ".len();
    panel[at..].split('>').next().unwrap().to_owned()
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

fn document(path: &str, text: &str, line: u32) -> Document {
    Document {
        path: path.into(),
        language: "rust".into(),
        text: text.into(),
        line,
        column: 1,
        dirty: false,
        selection: String::new(),
    }
}

#[test]
fn review_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-review-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("project");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("APPDATA", tmp.join("config"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
    }
    let main_rs = "fn main() {\n    let x = 41;\n    println!(\"{x}\");\n}\n";
    std::fs::write(ws.join("src/main.rs"), main_rs).unwrap();
    git(&ws, &["init", "-q", "-b", "main"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "first"]);
    git(&ws, &["checkout", "-q", "-b", "feature"]);
    let changed = "fn main() {\n    let x = 42;\n    println!(\"{x}\");\n}\n";
    std::fs::write(ws.join("src/main.rs"), changed).unwrap();
    git(&ws, &["commit", "-q", "-am", "answer"]);
    std::fs::write(ws.join("notes.txt"), "todo\n").unwrap();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/review");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());
    let mut seen = Seen::default();

    host.run_command(ID, "show", "");
    wait(&host, &mut seen, "changes against main", |s| {
        s.panel.contains("Changes against main") && s.panel.contains("+1") && s.panel.contains("notes.txt")
    });
    assert!(seen.panel.contains("2 files changed  +1 −1"), "{}", seen.panel);
    assert!(seen.panel.contains("src/main.rs | +1 −1 [Diff src/main.rs] [Open src/main.rs:1]"), "{}", seen.panel);
    assert!(seen.panel.contains("notes.txt | new, not in Git yet"), "{}", seen.panel);

    host.run_command(ID, "diff", "src/main.rs");
    wait(&host, &mut seen, "the diff", |s| s.panel.contains("+    let x = 42;"));

    // A comment on line 2, from the editor.
    host.set_document(Some(document("src/main.rs", changed, 2)));
    host.run_command(ID, "comment", "");
    wait(&host, &mut seen, "the comment form", |s| s.panel.contains("Comment on src/main.rs:2"));
    let add = form_id(&seen.panel, "add");
    submit(&host, &add, "Add comment", &[("text", "Why 42?")]);
    wait(&host, &mut seen, "the lens", |s| s.lenses == vec![(2, "Review: Why 42?".to_owned())]);
    assert!(seen.panel.contains("Comments (1 open, 0 resolved)"), "{}", seen.panel);

    // Lines added above: the comment follows its line.
    std::fs::write(ws.join("src/main.rs"), format!("// The answer.\n// Really.\n{changed}")).unwrap();
    host.event(EventKind::Saved, Event::Saved("src/main.rs".into()));
    wait(&host, &mut seen, "the lens moved", |s| s.lenses.first().is_some_and(|l| l.0 == 4));

    // A reply, then the verdict and the export.
    let cid = {
        let at = seen.panel.find("[Reply ").unwrap() + "[Reply ".len();
        seen.panel[at..].split(']').next().unwrap().to_owned()
    };
    host.run_command(ID, "reply", &cid);
    wait(&host, &mut seen, "the reply form", |s| s.panel.contains("<form reply#"));
    let reply = form_id(&seen.panel, "reply");
    submit(&host, &reply, "Reply", &[("text", "It's the answer.")]);
    wait(&host, &mut seen, "the reply", |s| s.panel.contains("↳ It's the answer."));
    let verdict = form_id(&seen.panel, "verdict");
    submit(&host, &verdict, "Save verdict", &[("verdict", "Request changes"), ("summary", "One question.")]);
    host.run_command(ID, "export", "");
    wait(&host, &mut seen, "the Markdown", |s| !s.opened.is_empty());
    assert!(
        seen.opened.starts_with("## Review: Changes requested\n\nCompared with `main`.\n\nOne question.\n"),
        "{}",
        seen.opened
    );
    assert!(
        seen.opened.contains("- **`src/main.rs:4`** `let x = 42;`  \n  Why 42?\n  - It's the answer."),
        "{}",
        seen.opened
    );

    // The line changes: outdated. Resolved: no lens. Kept for the next start.
    std::fs::write(ws.join("src/main.rs"), "fn main() {}\n").unwrap();
    host.event(EventKind::Saved, Event::Saved("src/main.rs".into()));
    host.run_command(ID, "show", "");
    wait(&host, &mut seen, "outdated", |s| s.panel.contains("outdated: its line changed"));
    host.run_command(ID, "resolve", &cid);
    wait(&host, &mut seen, "resolved", |s| s.panel.contains("Comments (0 open, 1 resolved)"));
    drop(host);

    let pkg = registry::read_folder(&dir).unwrap();
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());
    let mut seen = Seen::default();
    host.run_command(ID, "show", "");
    wait(&host, &mut seen, "kept", |s| {
        s.panel.contains("Comments (0 open, 1 resolved)") && s.panel.contains("Why 42?")
    });
    host.run_command(ID, "delete", &cid);
    wait(&host, &mut seen, "asked", |s| s.panel.contains("It can't be brought back"));
    host.run_command(ID, "delete-yes", &cid);
    wait(&host, &mut seen, "deleted", |s| s.panel.contains("Comments (0 open, 0 resolved)"));
    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
