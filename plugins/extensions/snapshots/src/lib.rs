//! Project Snapshots: the whole open folder kept as it is now, under a name, then compared with
//! how it is later (or with another snapshot, or another folder) and restored, all of it or a
//! file. Snapshots are kept by `git` in a repository of their own in the plugin's data folder:
//! never in the project, and never in the project's own Git repository. What git would ignore
//! (`.gitignore`), and `target/` and `node_modules/`, isn't kept.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod parse;

use std::collections::VecDeque;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FieldKind, Form, FormField, FormValues, JobResult, JobSpec, Row, Span};
use parse::{Change, Snap};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x3FB950;
const AMBER: u32 = 0xE2A33D;
const DIM: u32 = 0x8B949E;
const NOW: &str = "Now";
const EXCLUDE: &str = "# Never kept in a snapshot.\ntarget/\nnode_modules/\n";

/// One side of a comparison.
#[derive(Clone, Debug, PartialEq)]
enum Side {
    Now,
    Snap(String),
    Folder(String),
}

/// A git run: its arguments (after the repository's own), and which tree it works on.
struct Job {
    args: Vec<String>,
    /// Another folder, read with an index of its own (so the open folder's stays as it is).
    folder: Option<String>,
}

/// What to do once every job of a run has ended.
enum Then {
    /// The snapshots, as job `n` listed them.
    List(usize),
    Taken(String),
    Compared {
        left: Side,
        right: Side,
        trees: (Tree, Tree),
    },
    FileDiff(String),
    Restored(String),
    RestoredFile(String),
}

/// A side's tree: a revision, or what job `n` printed.
#[derive(Clone, Copy)]
enum Tree {
    Rev,
    Out(usize),
}

struct Run {
    jobs: VecDeque<Job>,
    outs: Vec<String>,
    then: Then,
    current: Option<u64>,
}

struct Comparison {
    left: Side,
    right: Side,
    /// The revisions or trees compared, for each file's diff.
    revs: (String, String),
    changes: Vec<Change>,
    /// The file whose diff is shown, and the diff.
    open: Option<(String, String)>,
}

#[derive(Default)]
struct State {
    snaps: Vec<Snap>,
    listed: bool,
    run: Option<Run>,
    error: Option<String>,
    comparison: Option<Comparison>,
    /// A restore or a delete waiting to be confirmed (its snapshot's tag).
    confirm: Option<(bool, String)>,
    /// Snapshots taken since the panel opened.
    taken: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: hint.into() }
}

fn offset() -> i32 {
    host::now().utc_offset_minutes
}

fn label(s: &Snap) -> String {
    format!("{} · {}", if s.name.is_empty() { "Snapshot" } else { &s.name }, parse::when(s.time, offset()))
}

/// A side as the compare form's choice shows it.
fn choice(side: &Side, snaps: &[Snap]) -> String {
    match side {
        Side::Now => NOW.into(),
        _ => side_name(side, snaps),
    }
}

fn side_name(side: &Side, snaps: &[Snap]) -> String {
    match side {
        Side::Now => "now".into(),
        Side::Snap(tag) => snaps.iter().find(|s| &s.tag == tag).map(label).unwrap_or_else(|| tag.clone()),
        Side::Folder(p) => p.clone(),
    }
}

// ----- the repository --------------------------------------------------------------------------

/// The open folder's repository of snapshots (made if new), as its path in the data folder.
fn repo() -> Result<(String, String), String> {
    let folder = host::workspace_path().map_err(|_| "Open a folder first".to_owned())?;
    let sub = format!("{:016x}", parse::hash(&folder));
    let dir = host::data_folder(&sub)?;
    Ok((sub, dir))
}

fn git(args: &[&str]) -> Job {
    Job { args: args.iter().map(|s| (*s).to_owned()).collect(), folder: None }
}

/// Starts a run: makes the repository first (harmless when it's there).
fn start(mut jobs: Vec<Job>, then: Then) -> Result<(), String> {
    if with(|s| s.run.is_some()) {
        return Err("Wait for the snapshot being taken or compared".into());
    }
    let (sub, dir) = repo()?;
    jobs.insert(0, Job { args: vec!["init".into(), "-q".into(), "--bare".into(), dir.clone()], folder: None });
    with(|s| {
        s.error = None;
        s.run = Some(Run { jobs: jobs.into(), outs: Vec::new(), then, current: None });
    });
    // After init made `info/`: what is never kept.
    let _ = (sub, dir);
    next();
    Ok(())
}

/// Starts the run's next job, or finishes it.
fn next() {
    let dir = match repo() {
        Ok((_, d)) => d,
        Err(e) => return fail(e),
    };
    let job = with(|s| {
        let run = s.run.as_mut()?;
        let job = run.jobs.pop_front()?;
        Some((job, run.outs.clone()))
    });
    let Some((job, outs)) = job else { return finish() };
    // "{n}" (a whole argument) is what job n printed.
    let fill = |a: &str| {
        let n = a.strip_prefix('{').and_then(|r| r.strip_suffix('}')).and_then(|n| n.parse::<usize>().ok());
        match n.and_then(|n| outs.get(n)) {
            Some(o) => o.trim().to_owned(),
            None => a.to_owned(),
        }
    };
    let is_init = job.args.first().map(String::as_str) == Some("init");
    let mut args: Vec<String> = Vec::new();
    if !is_init {
        let work = job.folder.clone().unwrap_or_else(|| ".".into());
        for a in [
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.safecrlf=false",
            "-c",
            "core.quotepath=false",
            "-c",
            "core.longpaths=true",
            "-c",
            "user.name=Coiman",
            "-c",
            "user.email=snapshots@coiman.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "--git-dir",
            &dir,
            "--work-tree",
            &work,
        ] {
            args.push(a.to_owned());
        }
    }
    args.extend(job.args.iter().map(|a| fill(a)));
    let env = match &job.folder {
        Some(_) => vec![("GIT_INDEX_FILE".to_owned(), format!("{dir}/other-index"))],
        None => Vec::new(),
    };
    let spec = JobSpec { program: "git".into(), args, stdin: String::new(), env, in_own_folder: false };
    match host::start_job_with(&spec) {
        Ok(id) => with(|s| {
            if let Some(r) = s.run.as_mut() {
                r.current = Some(id);
            }
        }),
        Err(e) => fail(e),
    }
}

fn fail(e: String) {
    with(|s| {
        s.run = None;
        s.error = Some(e);
    });
    render();
}

fn job_done(job: JobResult) {
    let mine = with(|s| s.run.as_ref().and_then(|r| r.current) == Some(job.id));
    if !mine {
        return;
    }
    if job.exit_code != Some(0) {
        let why = job
            .stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("warning:"))
            .unwrap_or("git failed")
            .to_owned();
        return fail(why);
    }
    let first = with(|s| {
        let r = s.run.as_mut().expect("a run");
        r.current = None;
        r.outs.push(job.stdout);
        r.outs.len() == 1
    });
    if first {
        // The repository is there now.
        if let Ok((sub, _)) = repo() {
            let _ = host::data_write(&format!("{sub}/info/exclude"), EXCLUDE);
        }
    }
    next();
}

fn finish() {
    let Some(run) = with(|s| s.run.take()) else { return };
    // The init job's output comes first; each later job's is outs[n + 1].
    let out = |n: usize| run.outs.get(n + 1).map(|o| o.trim().to_owned()).unwrap_or_default();
    match run.then {
        Then::List(n) => {
            let snaps = parse::snapshots(&out(n));
            with(|s| {
                s.snaps = snaps;
                s.listed = true;
            });
            render();
        }
        Then::Taken(name) => {
            with(|s| s.taken += 1);
            host::show_message(&format!("Snapshot taken: {name}"));
            list();
        }
        Then::Compared { left, right, trees } => {
            let rev = |side: &Side, t: Tree| match (side, t) {
                (Side::Snap(tag), _) => tag.clone(),
                (_, Tree::Out(n)) => out(n),
                (_, Tree::Rev) => String::new(),
            };
            let revs = (rev(&left, trees.0), rev(&right, trees.1));
            let changes = parse::changes(&out(run.outs.len().saturating_sub(2)));
            with(|s| s.comparison = Some(Comparison { left, right, revs, changes, open: None }));
            render();
        }
        Then::FileDiff(path) => {
            let diff = run.outs.last().cloned().unwrap_or_default();
            with(|s| {
                if let Some(c) = s.comparison.as_mut() {
                    c.open = Some((path, diff));
                }
            });
            render();
        }
        Then::Restored(name) => {
            host::show_message(&format!(
                "Restored {name}; what was there before is in the snapshot \"Before restoring {name}\""
            ));
            with(|s| s.comparison = None);
            list();
        }
        Then::RestoredFile(path) => {
            host::show_message(&format!("{path} is as it was in the snapshot"));
            let again = with(|s| s.comparison.as_ref().map(|c| (c.left.clone(), c.right.clone())));
            if let Some((l, r)) = again {
                let _ = compare(l, r);
            }
        }
    }
}

// ----- what it does ------------------------------------------------------------------------------

fn list() {
    let fmt = "--format=%(refname:short)%09%(creatordate:unix)%09%(subject)";
    if let Err(e) = start(vec![git(&["for-each-ref", "refs/tags", "--sort=-creatordate", fmt])], Then::List(0)) {
        fail(e);
    }
}

/// The jobs that keep the open folder in a new snapshot named `name`.
fn take_jobs(name: &str, first: usize) -> Vec<Job> {
    let tag = format!("s{}-{:04x}", host::now().unix_millis / 1000, host::random() & 0xffff);
    vec![
        git(&["add", "-A", "."]),
        git(&["write-tree"]),
        git(&["commit-tree", &format!("{{{}}}", first + 2), "-m", name]),
        git(&["tag", &tag, &format!("{{{}}}", first + 3)]),
    ]
}

fn take(name: &str) -> Result<(), String> {
    let name = parse::clean_name(name);
    // outs[0] is init's: take's own jobs are outs[1..].
    start(take_jobs(&name, 0), Then::Taken(name.clone()))
}

/// The jobs that make a side's tree (after `done` jobs), and where its tree is then.
fn tree_jobs(side: &Side, done: usize, jobs: &mut Vec<Job>) -> Tree {
    match side {
        Side::Snap(_) => Tree::Rev,
        Side::Now => {
            jobs.push(git(&["add", "-A", "."]));
            jobs.push(git(&["write-tree"]));
            Tree::Out(done + 1)
        }
        Side::Folder(p) => {
            jobs.push(Job { args: vec!["add".into(), "-A".into(), ".".into()], folder: Some(p.clone()) });
            jobs.push(Job { args: vec!["write-tree".into()], folder: Some(p.clone()) });
            Tree::Out(done + 1)
        }
    }
}

fn compare(left: Side, right: Side) -> Result<(), String> {
    if left == right {
        return Err("Pick two different things to compare".into());
    }
    let mut jobs = Vec::new();
    let a = tree_jobs(&left, 0, &mut jobs);
    let b = tree_jobs(&right, jobs.len(), &mut jobs);
    // Outs are numbered after init's: job n's output is {n + 1}.
    let rev = |side: &Side, t: Tree| match (side, t) {
        (Side::Snap(tag), _) => tag.clone(),
        (_, Tree::Out(n)) => format!("{{{}}}", n + 1),
        (_, Tree::Rev) => String::new(),
    };
    let (ra, rb) = (rev(&left, a), rev(&right, b));
    jobs.push(git(&["diff", "--raw", "--numstat", "--no-renames", &ra, &rb]));
    with(|s| s.confirm = None);
    start(jobs, Then::Compared { left, right, trees: (a, b) })
}

fn file_diff(path: &str) -> Result<(), String> {
    let revs = with(|s| s.comparison.as_ref().map(|c| c.revs.clone())).ok_or("Compare first")?;
    if with(|s| s.comparison.as_ref().and_then(|c| c.open.as_ref()).is_some_and(|(p, _)| p == path)) {
        with(|s| s.comparison.as_mut().map(|c| c.open = None));
        render();
        return Ok(());
    }
    start(vec![git(&["diff", &revs.0, &revs.1, "--", path])], Then::FileDiff(path.to_owned()))
}

fn restore(tag: &str) -> Result<(), String> {
    let name = with(|s| {
        s.snaps
            .iter()
            .find(|x| x.tag == tag)
            .map(|x| if x.name.is_empty() { "Snapshot".to_owned() } else { x.name.clone() })
    })
    .ok_or("That snapshot isn't there any more")?;
    // What there is now is kept first, so a restore can be undone.
    let mut jobs = take_jobs(&parse::clean_name(&format!("Before restoring {name}")), 0);
    jobs.push(git(&["checkout", tag, "--", "."]));
    with(|s| s.confirm = None);
    start(jobs, Then::Restored(name))
}

fn restore_file(path: &str) -> Result<(), String> {
    let tag = with(|s| match s.comparison.as_ref().map(|c| (&c.left, &c.right)) {
        Some((Side::Snap(t), Side::Now)) => Some(t.clone()),
        _ => None,
    })
    .ok_or("Only a snapshot compared with now restores a file")?;
    start(vec![git(&["checkout", &tag, "--", path])], Then::RestoredFile(path.to_owned()))
}

fn delete(tag: &str) -> Result<(), String> {
    with(|s| {
        s.confirm = None;
        if s.comparison.as_ref().is_some_and(|c| c.left == Side::Snap(tag.into()) || c.right == Side::Snap(tag.into()))
        {
            s.comparison = None;
        }
    });
    let fmt = "--format=%(refname:short)%09%(creatordate:unix)%09%(subject)";
    start(
        vec![
            git(&["tag", "-d", tag]),
            // Frees what only that snapshot kept.
            git(&["gc", "-q", "--prune=now"]),
            git(&["for-each-ref", "refs/tags", "--sort=-creatordate", fmt]),
        ],
        Then::List(2),
    )
}

// ----- the panel -------------------------------------------------------------------------------

fn render() {
    let blocks = with(|s| {
        let mut b = vec![
            Block::Form(Form {
                // A new id once one is taken, so its name box starts empty again.
                id: format!("take-{}", s.taken),
                fields: vec![field("name", "", FieldKind::Text, "", vec![], "Name, e.g. Before the refactor")],
                buttons: vec!["Take snapshot".into()],
            }),
            Block::Spans(vec![span(
                "Kept in Coiman's data folder, not in the project or its Git repository. \
                 What Git would ignore, target/ and node_modules/ aren't kept.",
                Some(DIM),
            )]),
        ];
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        if s.run.is_some() {
            b.push(Block::Spans(vec![span("Working…", Some(DIM))]));
        }
        if let Some((is_restore, tag)) = &s.confirm {
            let name = s.snaps.iter().find(|x| &x.tag == tag).map(label).unwrap_or_else(|| tag.clone());
            let (text, yes, cmd) = if *is_restore {
                (
                    format!(
                        "Restore {name}? Every file it kept comes back as it was; files made since stay. \
                         What there is now is kept in a snapshot first."
                    ),
                    "Restore",
                    "restore-yes",
                )
            } else {
                (format!("Delete {name}? It can't be brought back."), "Delete", "delete-yes")
            };
            b.push(Block::Spans(vec![span(text, Some(AMBER))]));
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![button(yes, cmd, tag), button("Cancel", "cancel", "")],
            }]));
        }
        if !s.listed {
            return b;
        }
        b.push(Block::Heading(format!("Snapshots ({})", s.snaps.len())));
        if s.snaps.is_empty() {
            b.push(Block::Text("None yet: name one above and take it.".into()));
            return b;
        }
        b.push(Block::Rows(
            s.snaps
                .iter()
                .map(|x| Row {
                    cells: vec![
                        Span {
                            text: if x.name.is_empty() { "Snapshot".into() } else { x.name.clone() },
                            color: None,
                            bold: true,
                            monospace: false,
                        },
                        span(parse::when(x.time, offset()), Some(DIM)),
                    ],
                    buttons: vec![
                        button("Compare with now", "compare", &x.tag),
                        button("Restore…", "restore", &x.tag),
                        button("Delete…", "delete", &x.tag),
                    ],
                })
                .collect(),
        ));
        let mut options = vec![NOW.to_owned()];
        options.extend(s.snaps.iter().map(label));
        let (left, right) = match &s.comparison {
            Some(c) => (choice(&c.left, &s.snaps), choice(&c.right, &s.snaps)),
            None => (options.get(1).cloned().unwrap_or_default(), NOW.to_owned()),
        };
        // Another folder isn't a choice: then the newest snapshot against now.
        let (left, right) = if options.contains(&left) && options.contains(&right) && left != right {
            (left, right)
        } else {
            (options[1].clone(), NOW.to_owned())
        };
        b.push(Block::Form(Form {
            id: "compare".into(),
            fields: vec![
                field("left", "Compare", FieldKind::Choice, &left, options.clone(), ""),
                field("right", "with", FieldKind::Choice, &right, options.clone(), ""),
            ],
            buttons: vec!["Compare".into()],
        }));
        b.push(Block::Form(Form {
            id: "folder".into(),
            fields: vec![field("path", "Or with another folder", FieldKind::Text, "", vec![], "Its full path")],
            buttons: vec!["Compare with folder".into()],
        }));
        if let Some(c) = &s.comparison {
            comparison(c, &s.snaps, &mut b);
        }
        b
    });
    host::set_panel("snapshots", &blocks);
}

fn comparison(c: &Comparison, snaps: &[Snap], b: &mut Vec<Block>) {
    let (added, removed) = c.changes.iter().filter_map(|x| x.lines).fold((0, 0), |(a, r), (x, y)| (a + x, r + y));
    b.push(Block::Heading(format!("{} → {}", side_name(&c.left, snaps), side_name(&c.right, snaps))));
    if c.changes.is_empty() {
        b.push(Block::Text("No differences.".into()));
        return;
    }
    b.push(Block::Spans(vec![
        span(format!("{} changed  ", plural(c.changes.len(), "file")), None),
        span(format!("+{added} "), Some(GREEN)),
        span(format!("−{removed}"), Some(RED)),
    ]));
    let can_restore = matches!((&c.left, &c.right), (Side::Snap(_), Side::Now));
    let now_side = c.right == Side::Now;
    let rows = c
        .changes
        .iter()
        .map(|x| {
            let (what, color) = match x.status {
                'A' => ("added", GREEN),
                'D' => ("removed", RED),
                _ => ("changed", AMBER),
            };
            let lines = match x.lines {
                Some((a, r)) => format!("+{a} −{r}"),
                None => "binary".into(),
            };
            let mut buttons = vec![button(
                if c.open.as_ref().is_some_and(|(p, _)| p == &x.path) { "Hide diff" } else { "Diff" },
                "diff",
                &x.path,
            )];
            if now_side && x.status != 'D' {
                buttons.push(button("Open", "open", &x.path));
            }
            if can_restore && x.status != 'A' {
                buttons.push(button("Restore file", "restore-file", &x.path));
            }
            Row { cells: vec![span(what, Some(color)), span(&x.path, None), span(lines, Some(DIM))], buttons }
        })
        .collect();
    b.push(Block::Rows(rows));
    if let Some((path, diff)) = &c.open {
        b.push(Block::Heading(path.clone()));
        if diff.trim().is_empty() {
            b.push(Block::Text("No line differences (only its mode, or a binary file).".into()));
        } else {
            b.push(Block::Diff(diff.clone()));
        }
    }
}

fn side_of(value: &str) -> Option<Side> {
    if value == NOW {
        return Some(Side::Now);
    }
    with(|s| s.snaps.iter().find(|x| label(x) == value).map(|x| Side::Snap(x.tag.clone())))
}

fn submitted(f: FormValues) -> Result<(), String> {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    match f.form.as_str() {
        f if f.starts_with("take") => take(&get("name")),
        "compare" => {
            let (l, r) = (side_of(&get("left")), side_of(&get("right")));
            compare(l.ok_or("Pick what to compare")?, r.ok_or("Pick what to compare with")?)
        }
        "folder" => {
            let path = get("path").trim().trim_matches('"').to_owned();
            if path.is_empty() {
                return Err("Type the other folder's full path".into());
            }
            compare(Side::Now, Side::Folder(path))
        }
        _ => Ok(()),
    }
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" => {
            render();
            list();
        }
        "take" => {
            render();
            take(arg)?;
        }
        "compare" => compare(Side::Snap(arg.to_owned()), Side::Now)?,
        "restore" | "delete" => {
            with(|s| s.confirm = Some((id == "restore", arg.to_owned())));
            render();
        }
        "restore-yes" => restore(arg)?,
        "delete-yes" => delete(arg)?,
        "cancel" => {
            with(|s| s.confirm = None);
            render();
        }
        "diff" => file_diff(arg)?,
        "restore-file" => restore_file(arg)?,
        "open" => host::open_location(arg, 1)?,
        _ => return Err(format!("unknown command {id}")),
    }
    Ok(())
}

fn report(r: Result<(), String>) {
    if let Err(e) = r {
        with(|s| s.error = Some(e));
        render();
    }
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        let r = command(&id, &argument);
        if r.is_err() {
            render();
        }
        r
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => job_done(job),
            Event::FormSubmitted(f) => {
                let r = submitted(f);
                report(r);
            }
            _ => {}
        }
    }
}

export!(Plugin);
