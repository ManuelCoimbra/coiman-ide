//! The Git Tools extension (`plugins/extensions/git-tools`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, on a small real repository: searching
//! the history (messages, a text that came in, the lines that match, a file's change), hooks
//! made, run and turned off, and what the `.gitignore` rules hide.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.git-tools";

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
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
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
            Block::Form(f) => {
                let ids: Vec<&str> = f.fields.iter().map(|f| f.id.as_str()).collect();
                out.push_str(&format!("form {} ({}) {:?}\n", f.id, ids.join(", "), f.buttons));
            }
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

#[test]
fn git_tools_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-gittools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    git(&ws, &["init", "-q", "-b", "main"]);
    // This repository's own settings only.
    git(&ws, &["config", "user.name", "Tester"]);
    git(&ws, &["config", "user.email", "tester@example.com"]);
    git(&ws, &["config", "commit.gpgsign", "false"]);
    w("a.rs", "fn alpha() {}\n");
    w(".gitignore", "*.log\nunused/\n");
    git(&ws, &["add", "."]);
    git(&ws, &["commit", "-q", "-m", "Add alpha"]);
    w("a.rs", "fn alpha() {}\nfn beta_test() {}\n");
    git(&ws, &["commit", "-q", "-am", "Add beta"]);
    w("x.log", "noise\n");
    w("a.tmp", "scratch\n");

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/git-tools");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::WorkspaceWrite].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // Search History: by message, then by a text that came in.
    host.run_command(ID, "search", "");
    let tab = panel(&host, "form search");
    assert!(tab.contains("[▸ Search History tab search]") && tab.contains("(kind, query, within, all)"), "{tab}");
    submit(&host, "search", "Search", &[("kind", "Message"), ("query", "ALPHA"), ("within", ""), ("all", "false")]);
    let found = panel(&host, "1 commit\n");
    assert!(found.contains("Add alpha | Tester") && !found.contains("Add beta"), "{found}");

    let kind = ("kind", "Added or removed text");
    submit(&host, "search", "Search", &[kind, ("query", "beta_test"), ("within", ""), ("all", "false")]);
    panel(&host, "Add beta | Tester");
    host.run_command(ID, "commit", "0");
    let lines = panel(&host, "The lines that match");
    assert!(lines.contains("+ fn beta_test() {}   a.rs"), "{lines}");
    let row = lines.lines().find(|l| l.contains("[Open change diff ")).expect("a file row");
    let arg = row.split("[Open change diff ").nth(1).unwrap().trim_end_matches(']');
    host.run_command(ID, "diff", arg);
    match wait(&host, |u| matches!(u, Update::CommitDiff { .. })) {
        Update::CommitDiff { commit, path, .. } => {
            assert_eq!(path, "a.rs");
            assert_eq!(commit.len(), 40);
        }
        _ => unreachable!(),
    }

    // Submodules: none here.
    host.run_command(ID, "tab", "submodules");
    panel(&host, "No submodules in this repository.");

    // Hooks: made from the template, run, turned off and on.
    host.run_command(ID, "tab", "hooks");
    let hooks = panel(&host, "[Create hook-create pre-commit]");
    assert!(hooks.contains("pre-commit | example only") || hooks.contains("pre-commit | "), "{hooks}");
    host.run_command(ID, "hook-create", "pre-commit");
    panel(&host, "[Run hook-run pre-commit]");
    let hook = ws.join(".git/hooks/pre-commit");
    assert!(std::fs::read_to_string(&hook).unwrap().contains("stopping (exit 1) stops the commit"));
    std::fs::write(&hook, "#!/bin/sh\necho hello from the hook\n").unwrap();
    host.run_command(ID, "hook-run", "pre-commit");
    let ran = panel(&host, "pre-commit ran:");
    assert!(ran.contains("hello from the hook"), "{ran}");
    host.run_command(ID, "hook-toggle", "pre-commit");
    panel(&host, "pre-commit | off [On hook-toggle pre-commit]");
    assert!(!hook.exists() && ws.join(".git/hooks/pre-commit.off").exists());
    host.run_command(ID, "hook-toggle", "pre-commit");
    panel(&host, "pre-commit | on");
    assert!(hook.exists());

    // .gitignore: each rule and what it hides, why a path is ignored, a rule tried then added.
    host.run_command(ID, "tab", "ignore");
    let rules = panel(&host, "hides nothing");
    assert!(rules.contains(".gitignore:1 | *.log | 1 entry"), "{rules}");
    assert!(rules.contains(".gitignore:2 | unused/ | hides nothing"), "{rules}");
    submit(&host, "why", "Check", &[("path", "x.log")]);
    let why = panel(&host, "the rule that hides it");
    assert!(why.contains("x.log: *.log") && why.contains("-> .gitignore:1"), "{why}");
    submit(&host, "why", "Check", &[("path", "a.rs")]);
    panel(&host, "a.rs isn't ignored");
    submit(&host, "rule", "What would it hide?", &[("pattern", "*.tmp")]);
    let preview = panel(&host, "*.tmp would hide");
    assert!(preview.contains("*.tmp would hide 1 untracked entry") && preview.contains("a.tmp"), "{preview}");
    submit(&host, "rule", "Add to .gitignore", &[("pattern", "*.tmp")]);
    panel(&host, ".gitignore:3 | *.tmp | 1 entry");
    assert_eq!(std::fs::read_to_string(ws.join(".gitignore")).unwrap(), "*.log\nunused/\n*.tmp\n");

    drop(host);
    let _ = std::fs::remove_dir_all(&ws);
}
