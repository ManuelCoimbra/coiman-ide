//! Git Graph: every branch's commits with lines for branches and merges, a commit's files, and
//! what's done from a commit: cherry-pick, revert, a branch, an interactive rebase, a merge,
//! a rebase onto it, a comparison with the current branch, worktrees. Everything goes through
//! `git`, run as approved jobs; conflicts are settled in the editor's Merge Conflicts window.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod graph;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, CommitGraph, CommitRow, FieldKind, Form, FormField, FormValues, GraphRef, JobResult, MenuItem, Row,
    Span,
};
use graph::{Commit, Operation, Step, Worktree};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;
const AMBER: u32 = 0xE09A3E;
const DIM: u32 = 0x8B949E;

/// What a running `git` is for.
enum Then {
    Log,
    Operation,
    Files(String),
    /// A change to the repository: what to say when it worked.
    Done(String),
    Plan(String, String),
    /// A comparison with `other`, read in steps: the merge base, then each side, then files.
    CompareBase(String),
    CompareTheirs(String, String),
    CompareOurs(String, String, Vec<Commit>),
    CompareFiles(Comparison),
    Worktrees(Option<String>),
}

#[derive(Clone, Default)]
struct Comparison {
    other: String,
    base: String,
    theirs: Vec<Commit>,
    ours: Vec<Commit>,
    files: Vec<(char, String)>,
}

struct Rebase {
    base: String,
    label: String,
    steps: Vec<Step>,
}

#[derive(Default)]
struct WorktreeForm {
    list: Vec<Worktree>,
    branch: String,
    confirm_remove: Option<String>,
}

#[derive(Default)]
struct State {
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    commits: Vec<Commit>,
    rows: Vec<graph::Row>,
    op: Option<Operation>,
    selected: Option<String>,
    files: Vec<(char, String)>,
    new_branch: Option<String>,
    rebase: Option<Rebase>,
    compare: Option<Option<Comparison>>,
    comparing: String,
    worktrees: Option<WorktreeForm>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn git(args: Vec<String>, stdin: &str, then: Then, busy: &str) {
    // Paths as they are (not quoted with octal escapes), and no colors in what is read.
    let mut all: Vec<String> = ["-c", "core.quotepath=off", "-c", "color.ui=false"].map(str::to_owned).into();
    all.extend(args);
    match host::start_job("git", &all, stdin) {
        Ok(id) => with(|s| {
            s.jobs.insert(id, then);
            if !busy.is_empty() {
                s.busy = Some(busy.to_owned());
            }
        }),
        Err(e) => with(|s| s.error = Some(e)),
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| (*a).to_owned()).collect()
}

fn load() {
    let limit = format!("-n{}", graph::LIMIT);
    git(args(&["log", "--all", "--topo-order", &limit, graph::LOG_FORMAT]), "", Then::Log, "Reading…");
}

fn commit(hash: &str) -> Option<Commit> {
    with(|s| s.commits.iter().find(|c| c.hash == hash).cloned())
}

/// Says what Git said, in short, and what to do when a conflict stopped it.
fn explain(said: &str) -> String {
    let first = said.lines().find(|l| !l.trim().is_empty() && !l.starts_with("hint:")).unwrap_or(said).trim();
    if said.contains("CONFLICT") || said.contains("could not apply") || said.contains("after resolving") {
        return format!("{first} Resolve the conflicts, then Continue.");
    }
    first.to_owned()
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let ok = job.exit_code == Some(0);
    let said = || {
        let t = format!("{}\n{}", job.stderr.trim(), job.stdout.trim());
        let t = t.trim();
        if t.is_empty() { "git failed".to_owned() } else { explain(t) }
    };
    let idle = with(|s| s.jobs.is_empty());
    if idle {
        with(|s| s.busy = None);
    }
    match then {
        Then::Log => {
            // A repository with no commits yet has nothing to draw.
            let commits = if ok { graph::parse_log(&job.stdout) } else { Vec::new() };
            if !ok && !job.stderr.contains("does not have any commits") {
                with(|s| s.error = Some(said()));
            }
            with(|s| {
                s.rows = graph::lanes(&commits);
                s.commits = commits;
                // The selection stays on its commit, wherever it moved.
                if s.selected.as_ref().is_some_and(|h| !s.commits.iter().any(|c| &c.hash == h)) {
                    s.selected = None;
                    s.files.clear();
                }
            });
            git(args(&["cat-file", "--batch-check"]), graph::OPERATION_REFS, Then::Operation, "");
            return;
        }
        Then::Operation => with(|s| s.op = graph::operation(&job.stdout)),
        Then::Files(hash) => with(|s| {
            if s.selected.as_deref() == Some(hash.as_str()) {
                s.files = graph::name_status(&job.stdout);
            }
        }),
        Then::Done(done) => {
            if ok {
                host::show_message(&done);
                with(|s| s.error = None);
            } else {
                with(|s| s.error = Some(said()));
            }
            load();
            if with(|s| s.worktrees.is_some()) {
                git(args(&["worktree", "list", "--porcelain"]), "", Then::Worktrees(None), "");
            }
        }
        Then::Plan(base, label) => with(|s| {
            if ok {
                s.rebase = Some(Rebase { base, label, steps: graph::parse_plan(&job.stdout) });
            } else {
                s.error = Some(said());
            }
        }),
        Then::CompareBase(other) => {
            if ok {
                let base = job.stdout.trim().to_owned();
                let a = vec!["log".into(), format!("HEAD..{other}"), graph::LOG_FORMAT.into()];
                git(a, "", Then::CompareTheirs(other, base), "Comparing…");
                return;
            }
            with(|s| {
                s.error = Some(said());
                s.compare = None;
            });
        }
        Then::CompareTheirs(other, base) => {
            let theirs = graph::parse_log(&job.stdout);
            let a = vec!["log".into(), format!("{other}..HEAD"), graph::LOG_FORMAT.into()];
            git(a, "", Then::CompareOurs(other, base, theirs), "Comparing…");
            return;
        }
        Then::CompareOurs(other, base, theirs) => {
            let ours = graph::parse_log(&job.stdout);
            let range = format!("{base}..{other}");
            let a = args(&["diff", "--name-status", "--no-renames", "-z", &range]);
            let c = Comparison { other, base, theirs, ours, files: Vec::new() };
            git(a, "", Then::CompareFiles(c), "Comparing…");
            return;
        }
        Then::CompareFiles(mut c) => with(|s| {
            c.files = graph::name_status(&job.stdout);
            if s.comparing == c.other {
                s.compare = Some(Some(c));
            }
        }),
        Then::Worktrees(branch) => with(|s| {
            if !ok {
                s.error = Some(said());
                return;
            }
            let w = s.worktrees.get_or_insert_with(WorktreeForm::default);
            w.list = graph::parse_worktrees(&job.stdout);
            if let Some(b) = branch {
                w.branch = b;
            }
        }),
    }
    render();
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: vec![], buttons }])
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, hint: &str) -> FormField {
    FormField {
        id: id.to_owned(),
        label: label.to_owned(),
        kind,
        value: value.to_owned(),
        options: vec![],
        hint: hint.to_owned(),
    }
}

fn kind_color(kind: char) -> u32 {
    match kind {
        'A' => GREEN,
        'D' => RED,
        _ => AMBER,
    }
}

fn commit_rows(list: &[Commit]) -> Block {
    Block::Rows(
        list.iter()
            .map(|c| Row {
                cells: vec![
                    Span { text: c.short.clone(), color: Some(DIM), bold: false, monospace: true },
                    span(&*c.subject, None),
                ],
                buttons: vec![],
            })
            .collect(),
    )
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let worktrees = if s.worktrees.is_some() { "Hide worktrees" } else { "Worktrees" };
        b.push(bar(vec![button("Refresh", "refresh", ""), button(worktrees, "worktrees", "")]));
        let mut line = vec![span(
            format!("{} commits, every branch · click one for its files, right-click for more", s.commits.len()),
            Some(DIM),
        )];
        if let Some(busy) = &s.busy {
            line.push(span(format!("   {busy}"), Some(DIM)));
        }
        b.push(Block::Spans(line));
        if let Some(op) = s.op {
            b.push(Block::Spans(vec![span(format!("A {} is in progress.", op.name()), Some(AMBER))]));
            b.push(bar(vec![
                button("Resolve conflicts…", "conflicts", ""),
                button("Continue", "continue", ""),
                button("Abort", "abort", ""),
            ]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        compare_ui(s, &mut b);
        rebase_ui(s, &mut b);
        worktrees_ui(s, &mut b);
        if let Some(hash) = &s.new_branch {
            let short = s.commits.iter().find(|c| &c.hash == hash).map_or("", |c| c.short.as_str());
            b.push(Block::Form(Form {
                id: "branch".into(),
                fields: vec![field("name", &format!("New branch at {short}:"), FieldKind::Text, "", "name")],
                buttons: vec!["Create".into(), "Cancel".into()],
            }));
        }
        let rows = s
            .commits
            .iter()
            .zip(&s.rows)
            .map(|(c, r)| CommitRow {
                id: c.hash.clone(),
                lane: r.lane as u32,
                top: r.top.iter().map(|&(a, b)| (a as u32, b as u32)).collect(),
                bottom: r.bottom.iter().map(|&(a, b)| (a as u32, b as u32)).collect(),
                merge: c.parents.len() > 1,
                head: c.head(),
                refs: c.ref_labels().into_iter().map(|(name, tag, current)| GraphRef { name, tag, current }).collect(),
                subject: c.subject.clone(),
                detail: format!("{} · {} · {}", c.short, c.author, c.when),
                target: c.target(),
                branch: c.branch().unwrap_or("").to_owned(),
            })
            .collect();
        let item = |label: &str, command: &str| MenuItem { label: label.into(), command: command.into() };
        b.push(Block::CommitGraph(CommitGraph {
            rows,
            selected: s.selected.clone().unwrap_or_default(),
            click: "select".into(),
            menu: vec![
                item("Cherry-pick onto the current branch", "cherry-pick"),
                item("Revert (a commit that undoes it)", "revert"),
                item("Create branch here…", "new-branch"),
                item("Rebase interactively from here…", "plan-rebase"),
                item("-", ""),
                item("New worktree for {branch}…", "worktree-for"),
                item("Compare {target} with the current branch", "compare"),
                item("Merge {target} into the current branch", "merge"),
                item("Rebase the current branch onto {target}", "rebase-onto"),
                item("-", ""),
                item("Copy hash", "copy"),
            ],
        }));
        if let Some(c) = s.selected.as_ref().and_then(|h| s.commits.iter().find(|c| &c.hash == h)) {
            b.push(Block::Heading(format!("{} {}", c.short, c.subject)));
            let rows = s
                .files
                .iter()
                .map(|(kind, path)| Row {
                    cells: vec![
                        Span { text: kind.to_string(), color: Some(kind_color(*kind)), bold: true, monospace: true },
                        span(path, None),
                    ],
                    buttons: vec![button("Show change", "file", path)],
                })
                .collect();
            b.push(Block::Rows(rows));
        }
        b
    });
    host::set_panel("graph", &blocks);
}

fn compare_ui(s: &State, b: &mut Vec<Block>) {
    let Some(found) = &s.compare else { return };
    b.push(Block::Heading(format!("{} against the current branch", s.comparing)));
    b.push(bar(vec![button("Close", "compare-close", "")]));
    let Some(c) = found else {
        b.push(Block::Spans(vec![span("Comparing…", Some(DIM))]));
        return;
    };
    b.push(Block::Spans(vec![span(format!("Only on {}: {}", c.other, c.theirs.len()), None)]));
    b.push(commit_rows(&c.theirs));
    b.push(Block::Spans(vec![span(format!("Only on the current branch: {}", c.ours.len()), None)]));
    b.push(commit_rows(&c.ours));
    b.push(Block::Spans(vec![span(format!("Files {} changed since they parted: {}", c.other, c.files.len()), None)]));
    let rows = c
        .files
        .iter()
        .map(|(kind, path)| Row {
            cells: vec![
                Span { text: kind.to_string(), color: Some(kind_color(*kind)), bold: true, monospace: true },
                span(path, None),
            ],
            buttons: vec![button("Show change", "compare-file", path)],
        })
        .collect();
    b.push(Block::Rows(rows));
    b.push(Block::Separator);
}

fn rebase_ui(s: &State, b: &mut Vec<Block>) {
    let Some(r) = &s.rebase else { return };
    b.push(Block::Heading(format!("Interactive rebase: the commits after {}", r.label)));
    if r.steps.is_empty() {
        b.push(Block::Text("No commits after this one on this branch.".into()));
        b.push(bar(vec![button("Close", "rebase-cancel", "")]));
        return;
    }
    b.push(Block::Spans(vec![span(
        "Oldest first. Move them, choose what's done to each (changing a message rewords it), then Start rebase.",
        Some(DIM),
    )]));
    let mut fields = Vec::new();
    for step in &r.steps {
        let h = &step.hash;
        fields.push(field(&format!("up-{h}"), "↑", FieldKind::Button, "", ""));
        fields.push(field(&format!("down-{h}"), "↓", FieldKind::Button, "", ""));
        let mut choice = field(&format!("act-{h}"), "", FieldKind::Choice, &step.action, "");
        choice.options = graph::ACTIONS.iter().map(|a| (*a).to_owned()).collect();
        fields.push(choice);
        fields.push(field(&format!("msg-{h}"), &step.short, FieldKind::Text, &step.message, "the message"));
        fields.push(field("", "", FieldKind::LineBreak, "", ""));
    }
    b.push(Block::Form(Form { id: "rebase".into(), fields, buttons: vec!["Start rebase".into(), "Cancel".into()] }));
    b.push(Block::Separator);
}

fn worktrees_ui(s: &State, b: &mut Vec<Block>) {
    let Some(w) = &s.worktrees else { return };
    b.push(Block::Heading("Worktrees".into()));
    b.push(Block::Spans(vec![span("This repository's other working folders, each on its own branch.", Some(DIM))]));
    let rows = w
        .list
        .iter()
        .map(|t| {
            let name = t.branch.clone().unwrap_or_else(|| format!("detached at {}", &t.head[..t.head.len().min(8)]));
            let mut place = t.path.clone();
            if t.main {
                place.push_str("  (the repository's own)");
            }
            if t.locked {
                place.push_str("  locked");
            }
            if t.prunable {
                place.push_str("  folder gone");
            }
            let mut buttons = Vec::new();
            if !t.prunable {
                buttons.push(button("Open", "worktree-open", &t.path));
            }
            if !t.main {
                if w.confirm_remove.as_deref() == Some(t.path.as_str()) {
                    buttons.push(button("Remove it", "worktree-remove", &t.path));
                    buttons.push(button("Even with changes", "worktree-force", &t.path));
                    buttons.push(button("Keep", "worktree-keep", ""));
                } else {
                    buttons.push(button("Remove…", "worktree-ask", &t.path));
                }
            }
            Row {
                cells: vec![
                    Span { text: name, color: if t.prunable { Some(DIM) } else { None }, bold: true, monospace: false },
                    span(place, Some(DIM)),
                ],
                buttons,
            }
        })
        .collect();
    b.push(Block::Rows(rows));
    if w.list.iter().any(|t| t.prunable) {
        b.push(bar(vec![button("Forget the ones whose folder is gone", "worktree-prune", "")]));
    }
    let main = w.list.iter().find(|t| t.main).map_or("", |t| t.path.as_str());
    let beside = graph::worktree_folder(main, if w.branch.is_empty() { "<branch>" } else { &w.branch });
    b.push(Block::Form(Form {
        id: "worktree".into(),
        fields: vec![
            field("branch", "New worktree on", FieldKind::Text, &w.branch, "branch"),
            field("create", "New branch", FieldKind::Check, "false", ""),
            field("folder", "in", FieldKind::Text, "", &format!("{beside} if left empty")),
        ],
        buttons: vec!["Add".into()],
    }));
    b.push(Block::Separator);
}

// ----- what the user does ---------------------------------------------------------------------

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    match f.form.as_str() {
        "branch" => {
            let hash = with(|s| s.new_branch.take());
            let name = get("name");
            if let (Some(hash), "Create", false) = (hash, f.button.as_str(), name.is_empty()) {
                let a = vec!["branch".into(), "--".into(), name.clone(), hash];
                git(a, "", Then::Done(format!("Branch {name} made")), "Creating the branch…");
            }
        }
        "rebase" => {
            // What was chosen and typed is kept, whichever button it was.
            with(|s| {
                if let Some(r) = &mut s.rebase {
                    for step in &mut r.steps {
                        step.action = get(&format!("act-{}", step.hash));
                        step.message = get(&format!("msg-{}", step.hash));
                    }
                }
            });
            let button = f.button.as_str();
            if button == "Cancel" {
                with(|s| s.rebase = None);
            } else if button == "Start rebase" {
                let plan = with(|s| s.rebase.as_ref().map(|r| (r.base.clone(), graph::todo(&r.steps))));
                match plan {
                    Some((base, Ok(todo))) => {
                        with(|s| s.rebase = None);
                        git(graph::rebase_args(&base, &todo), "", Then::Done("Rebased".into()), "Rebasing…");
                    }
                    Some((_, Err(e))) => with(|s| s.error = Some(e)),
                    None => {}
                }
            } else if let Some((dir, hash)) = button.split_once('-') {
                with(|s| {
                    let Some(r) = &mut s.rebase else { return };
                    let Some(i) = r.steps.iter().position(|st| st.hash == hash) else { return };
                    let j = if dir == "up" { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < r.steps.len()) };
                    if let Some(j) = j {
                        r.steps.swap(i, j);
                    }
                });
            }
        }
        "worktree" => {
            let (branch, create) = (get("branch"), get("create") == "true");
            if branch.is_empty() {
                with(|s| s.error = Some("Name the branch for the new worktree".into()));
            } else {
                let main =
                    with(|s| s.worktrees.as_ref().and_then(|w| w.list.iter().find(|t| t.main).map(|t| t.path.clone())));
                let folder = Some(get("folder"))
                    .filter(|f| !f.is_empty())
                    .unwrap_or_else(|| graph::worktree_folder(&main.unwrap_or_default(), &branch));
                let a = if create {
                    vec!["worktree".into(), "add".into(), "-b".into(), branch.clone(), "--".into(), folder.clone()]
                } else {
                    vec!["worktree".into(), "add".into(), "--".into(), folder.clone(), branch.clone()]
                };
                git(a, "", Then::Done(format!("Worktree for {branch} in {folder}")), "Adding the worktree…");
            }
        }
        _ => {}
    }
    render();
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" | "refresh" => {
            with(|s| s.error = None);
            load();
        }
        "select" => {
            with(|s| {
                s.selected = Some(arg.to_owned());
                s.files.clear();
            });
            let a = args(&["show", "--first-parent", "--format=", "--name-status", "--no-renames", "-z", arg]);
            git(a, "", Then::Files(arg.to_owned()), "");
        }
        "file" => {
            let hash = with(|s| s.selected.clone()).ok_or("no commit chosen")?;
            host::open_commit_diff(&hash, arg)?;
        }
        "cherry-pick" | "revert" => {
            let c = commit(arg).ok_or("that commit is gone: refresh")?;
            let a =
                if id == "revert" { args(&["revert", "--no-edit", &c.hash]) } else { args(&["cherry-pick", &c.hash]) };
            let done =
                if id == "revert" { format!("Reverted {}", c.short) } else { format!("Cherry-picked {}", c.short) };
            git(
                a,
                "",
                Then::Done(done),
                &format!("{} {}…", if id == "revert" { "Reverting" } else { "Cherry-picking" }, c.short),
            );
        }
        "new-branch" => with(|s| s.new_branch = Some(arg.to_owned())),
        "plan-rebase" => {
            let c = commit(arg).ok_or("that commit is gone: refresh")?;
            let range = format!("{}..HEAD", c.hash);
            let a = args(&["log", "--reverse", "--no-merges", "--format=%H%x1f%h%x1f%s", &range]);
            git(a, "", Then::Plan(c.hash.clone(), format!("{} {}", c.short, c.subject)), "Reading…");
        }
        "merge" | "rebase-onto" | "compare" => {
            let target = commit(arg).ok_or("that commit is gone: refresh")?.target();
            match id {
                "merge" => git(
                    args(&["merge", "--no-edit", &target]),
                    "",
                    Then::Done(format!("Merged {target}")),
                    &format!("Merging {target}…"),
                ),
                "rebase-onto" => git(
                    args(&["rebase", &target]),
                    "",
                    Then::Done(format!("Rebased onto {target}")),
                    &format!("Rebasing onto {target}…"),
                ),
                _ => {
                    with(|s| {
                        s.compare = Some(None);
                        s.comparing = target.clone();
                    });
                    let a = args(&["merge-base", "HEAD", &target]);
                    git(a, "", Then::CompareBase(target), "Comparing…");
                }
            }
        }
        "compare-close" => with(|s| s.compare = None),
        "compare-file" => {
            let (base, other) = with(|s| match &s.compare {
                Some(Some(c)) => Some((c.base.clone(), c.other.clone())),
                _ => None,
            })
            .ok_or("no comparison")?;
            host::open_compare_diff(&base, &other, arg)?;
        }
        "copy" => {
            host::copy_text(arg);
            host::show_message("Hash copied");
        }
        "conflicts" => host::show_merge_conflicts(),
        "continue" | "abort" => {
            let op = with(|s| s.op).ok_or("nothing is in progress")?;
            let (a, done, busy) = if id == "continue" {
                (op.continue_args(), format!("The {} went on", op.name()), format!("Continuing the {}…", op.name()))
            } else {
                let a = vec![op.name().into(), "--abort".into()];
                (a, format!("The {} was given up", op.name()), format!("Stopping the {}…", op.name()))
            };
            git(a, "", Then::Done(done), &busy);
        }
        "rebase-cancel" => with(|s| s.rebase = None),
        "worktrees" => {
            if with(|s| s.worktrees.take().is_none()) {
                git(args(&["worktree", "list", "--porcelain"]), "", Then::Worktrees(None), "Reading…");
            }
        }
        "worktree-for" => {
            let branch = commit(arg).and_then(|c| c.branch().map(str::to_owned));
            with(|s| {
                let w = s.worktrees.get_or_insert_with(WorktreeForm::default);
                if let Some(b) = &branch {
                    w.branch = b.clone();
                }
            });
            git(args(&["worktree", "list", "--porcelain"]), "", Then::Worktrees(branch), "Reading…");
        }
        "worktree-open" => host::open_folder(arg)?,
        "worktree-ask" => with(|s| {
            if let Some(w) = &mut s.worktrees {
                w.confirm_remove = Some(arg.to_owned());
            }
        }),
        "worktree-keep" => with(|s| {
            if let Some(w) = &mut s.worktrees {
                w.confirm_remove = None;
            }
        }),
        "worktree-remove" | "worktree-force" => {
            with(|s| {
                if let Some(w) = &mut s.worktrees {
                    w.confirm_remove = None;
                }
            });
            let mut a = args(&["worktree", "remove"]);
            if id == "worktree-force" {
                a.push("--force".into());
            }
            a.extend(["--".into(), arg.to_owned()]);
            git(a, "", Then::Done(format!("Removed the worktree in {arg}")), "Removing the worktree…");
        }
        "worktree-prune" => git(args(&["worktree", "prune"]), "", Then::Done("Pruned".into()), "Pruning…"),
        _ => return Err(format!("unknown command {id}")),
    }
    render();
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
            Event::JobFinished(job) => finished(job),
            Event::FormSubmitted(f) => submitted(f),
            _ => {}
        }
    }
}

export!(Plugin);
