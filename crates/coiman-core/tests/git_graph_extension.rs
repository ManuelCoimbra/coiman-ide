//! The Git Graph extension (`plugins/extensions/git-graph`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, on a real repository: the graph and its
//! lanes, a commit's files, a cherry-pick that conflicts and is continued, revert, a branch, an
//! interactive rebase that reorders, rewords, squashes and drops, a comparison, a merge, a rebase
//! onto a branch, and worktrees.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.git-graph";

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

/// The panel's text and blocks, when it shows `until`.
fn panel(host: &PluginHost, until: &str) -> (String, Vec<Block>) {
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            if last.contains(until) {
                return (last, blocks);
            }
        }
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn message(host: &PluginHost, part: &str) -> String {
    match wait(host, |u| matches!(u, Update::Message { text, .. } if text.contains(part))) {
        Update::Message { text, .. } => text,
        _ => unreachable!(),
    }
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
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => {
                let fields: Vec<String> = f.fields.iter().map(|f| format!("{}={}", f.id, f.value)).collect();
                out.push_str(&format!("form {} ({}) {:?}\n", f.id, fields.join(", "), f.buttons));
            }
            Block::CommitGraph(g) => {
                for r in &g.rows {
                    let refs: Vec<&str> = r.refs.iter().map(|r| r.name.as_str()).collect();
                    out.push_str(&format!("@{} [{}] {} | {}\n", r.lane, refs.join(","), r.subject, r.detail));
                }
            }
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(String, String)]) {
    let values = values.to_vec();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

fn pair(k: &str, v: &str) -> (String, String) {
    (k.to_owned(), v.to_owned())
}

#[test]
fn git_graph_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-gitgraph-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("repo");
    std::fs::create_dir_all(&ws).unwrap();
    let w = |p: &str, t: &str| std::fs::write(ws.join(p), t).unwrap();
    let commit = |file: &str, t: &str, msg: &str| {
        w(file, t);
        git(&ws, &["add", "."]);
        git(&ws, &["commit", "-q", "-m", msg]);
        git(&ws, &["rev-parse", "HEAD"])
    };
    git(&ws, &["init", "-q", "-b", "main"]);
    // This repository's own settings only.
    git(&ws, &["config", "user.name", "Tester"]);
    git(&ws, &["config", "user.email", "tester@example.com"]);
    git(&ws, &["config", "commit.gpgsign", "false"]);
    git(&ws, &["config", "core.autocrlf", "false"]);
    commit("a.txt", "one\n", "first");
    git(&ws, &["switch", "-q", "-c", "feature"]);
    let feature = commit("a.txt", "feature\n", "on feature");
    git(&ws, &["switch", "-q", "main"]);
    commit("a.txt", "main\n", "on main");

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/git-graph");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["git".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The graph: two branches, two lanes.
    host.run_command(ID, "show", "");
    let (graph, _) = panel(&host, "3 commits, every branch");
    // Which tip comes first depends on the clock: one in each lane.
    let main_row = graph.lines().find(|l| l.contains("[main] on main | ")).expect("main's tip");
    let feature_row = graph.lines().find(|l| l.contains("[feature] on feature | ")).expect("feature's tip");
    let mut lanes = [&main_row[..2], &feature_row[..2]];
    lanes.sort();
    assert_eq!(lanes, ["@0", "@1"], "{graph}");
    assert!(graph.contains("first | ") && graph.contains("Tester"), "{graph}");

    // A commit's files.
    host.run_command(ID, "select", &feature);
    let (files, _) = panel(&host, "[Show change file a.txt]");
    assert!(files.contains("on feature\nM | a.txt"), "{files}");
    host.run_command(ID, "file", "a.txt");
    wait(&host, |u| matches!(u, Update::CommitDiff { path, .. } if path == "a.txt"));

    // A cherry-pick that conflicts: in progress until its file is settled and it goes on.
    host.run_command(ID, "cherry-pick", &feature);
    let (stopped, _) = panel(&host, "A cherry-pick is in progress.");
    assert!(stopped.contains("Resolve the conflicts, then Continue."), "{stopped}");
    assert!(stopped.contains("[Resolve conflicts… conflicts ]"), "{stopped}");
    host.run_command(ID, "conflicts", "");
    wait(&host, |u| matches!(u, Update::MergeConflicts));
    w("a.txt", "main\nfeature\n");
    git(&ws, &["add", "a.txt"]);
    host.run_command(ID, "continue", "");
    message(&host, "The cherry-pick went on");
    let (after, _) = panel(&host, "4 commits");
    assert!(!after.contains("in progress"), "{after}");
    assert_eq!(git(&ws, &["log", "-1", "--format=%s"]), "on feature");

    // Revert, and a branch made at a commit.
    let b1 = commit("b.txt", "1\n", "b one");
    host.run_command(ID, "refresh", "");
    panel(&host, "5 commits");
    host.run_command(ID, "revert", &b1);
    message(&host, "Reverted");
    assert!(!ws.join("b.txt").exists(), "the revert undid it");
    host.run_command(ID, "new-branch", &b1);
    panel(&host, "form branch (name=)");
    submit(&host, "branch", "Create", &[pair("name", "kept")]);
    message(&host, "Branch kept made");
    assert_eq!(git(&ws, &["rev-parse", "kept"]), b1);

    // An interactive rebase: d moved first and reworded, e squashed into c, f dropped.
    let base = git(&ws, &["rev-parse", "HEAD"]);
    let c = commit("c.txt", "c\n", "add c");
    let d = commit("d.txt", "d\n", "add d");
    let e = commit("e.txt", "e\n", "add e");
    let f = commit("f.txt", "f\n", "add f");
    host.run_command(ID, "refresh", "");
    panel(&host, "add f");
    host.run_command(ID, "plan-rebase", &base);
    let (plan, _) = panel(&host, "Interactive rebase: the commits after");
    let order = |t: &str| -> Vec<usize> {
        [&c, &d, &e, &f].iter().map(|h| t.find(&format!("act-{h}=")).expect("a step")).collect()
    };
    let at = order(&plan);
    assert!(at[0] < at[1] && at[1] < at[2] && at[2] < at[3], "oldest first: {plan}");
    let values = |acts: [(&str, &str, &str); 4]| -> Vec<(String, String)> {
        acts.iter().flat_map(|(h, act, msg)| [pair(&format!("act-{h}"), act), pair(&format!("msg-{h}"), msg)]).collect()
    };
    let changed = values([
        (&c, "pick", "add c"),
        (&d, "pick", "add d (it's reworded)"),
        (&e, "squash", "add e"),
        (&f, "drop", "add f"),
    ]);
    submit(&host, "rebase", &format!("up-{d}"), &changed);
    let (moved, _) = panel(&host, &format!("msg-{d}=add d (it's reworded)"));
    let at = order(&moved);
    assert!(at[1] < at[0] && at[0] < at[2], "d moved up, the typed values kept: {moved}");
    submit(&host, "rebase", "Start rebase", &changed);
    message(&host, "Rebased");
    let log = git(&ws, &["log", "-2", "--format=%s"]);
    assert_eq!(log, "add c\nadd d (it's reworded)");
    assert!(ws.join("e.txt").exists() && !ws.join("f.txt").exists());

    // A branch compared with the current one, then merged.
    let main_tip = git(&ws, &["rev-parse", "HEAD"]);
    git(&ws, &["switch", "-q", "-c", "side"]);
    let side = commit("side.txt", "s\n", "side work");
    git(&ws, &["switch", "-q", "main"]);
    commit("main2.txt", "m\n", "main work");
    host.run_command(ID, "refresh", "");
    panel(&host, "side work");
    host.run_command(ID, "compare", &side);
    let (cmp, _) = panel(&host, "Files side changed since they parted: 1");
    assert!(cmp.contains("Only on side: 1\n") && cmp.contains("side work"), "{cmp}");
    assert!(cmp.contains("Only on the current branch: 1\n") && cmp.contains("main work"), "{cmp}");
    host.run_command(ID, "compare-file", "side.txt");
    match wait(&host, |u| matches!(u, Update::CompareDiff { .. })) {
        Update::CompareDiff { base, other, path, .. } => {
            assert_eq!((base.as_str(), other.as_str(), path.as_str()), (main_tip.as_str(), "side", "side.txt"));
        }
        _ => unreachable!(),
    }
    host.run_command(ID, "merge", &side);
    message(&host, "Merged side");
    assert!(ws.join("side.txt").exists());

    // Another branch rebased onto main.
    git(&ws, &["switch", "-q", "-c", "later"]);
    commit("later.txt", "l\n", "later work");
    git(&ws, &["switch", "-q", "main"]);
    let main3 = commit("main3.txt", "m3\n", "more main");
    git(&ws, &["switch", "-q", "later"]);
    host.run_command(ID, "refresh", "");
    panel(&host, "more main");
    host.run_command(ID, "rebase-onto", &main3);
    message(&host, "Rebased onto main");
    assert!(ws.join("main3.txt").exists() && ws.join("later.txt").exists(), "later now sits on main");

    // Worktrees: one made for a branch, listed, then removed.
    host.run_command(ID, "worktree-for", &side);
    let (form, _) = panel(&host, "(the repository's own)");
    assert!(form.contains("form worktree (branch=side,"), "{form}");
    let folder = tmp.join("wt").display().to_string();
    submit(&host, "worktree", "Add", &[pair("branch", "wt-branch"), pair("create", "true"), pair("folder", &folder)]);
    message(&host, "Worktree for wt-branch");
    assert!(tmp.join("wt/a.txt").is_file(), "the new tree has the files");
    let (list, _) = panel(&host, "wt-branch |");
    assert!(list.contains("[Remove… worktree-ask "), "{list}");
    let listed = git(&ws, &["worktree", "list", "--porcelain"]);
    let path = listed.lines().filter_map(|l| l.strip_prefix("worktree ")).nth(1).unwrap().to_owned();
    host.run_command(ID, "worktree-ask", &path);
    panel(&host, "[Remove it worktree-remove ");
    host.run_command(ID, "worktree-remove", &path);
    message(&host, "Removed the worktree");
    assert!(!tmp.join("wt").exists());

    host.run_command(ID, "copy", &side);
    wait(&host, |u| matches!(u, Update::CopyText(t) if *t == side));

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
