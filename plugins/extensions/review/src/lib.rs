//! Code Review: reviews the changes of the open folder (a Git repository) against a base,
//! all on this computer: the files changed with their diffs, comments on lines (shown above
//! them in the editor, following their line as the code moves), replies, resolving, and a
//! verdict with a summary, exported as Markdown for a pull request.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod model;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, CodeLens, FieldKind, Form, FormField, FormValues, JobResult, Row, Span};
use model::{Comment, Review};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x3FB950;
const AMBER: u32 = 0xE2A33D;
const DIM: u32 = 0x8B949E;

#[derive(Clone, Copy, PartialEq)]
enum Job {
    /// Which of main and master there is, for the base at first.
    Branches,
    Numstat,
    Untracked,
    Diff,
}

#[derive(Default)]
struct State {
    project: String,
    loaded: bool,
    review: Review,
    /// Changed files: path, lines added, removed; `None` counts for a file Git doesn't track.
    files: Vec<(String, Option<(u32, u32)>)>,
    untracked: Vec<String>,
    /// The file whose diff is shown, and the diff.
    diff: Option<(String, String)>,
    /// A comment being written: path, line, the line's text.
    adding: Option<(String, u32, String)>,
    replying: Option<u64>,
    deleting: Option<u64>,
    /// Comments whose line is gone from the file.
    outdated: Vec<u64>,
    /// The comment picked from the editor, shown first.
    focus: Option<u64>,
    jobs: Vec<(u64, Job, String)>,
    error: Option<String>,
    /// Forms get new ids once sent, so their boxes start empty again.
    sent: u32,
    /// Whether the changes have been asked of git yet (only once the panel is used).
    fetched: bool,
    /// A review was stored: its base is the one to use.
    base_known: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn bold(text: impl Into<String>) -> Span {
    Span { text: text.into(), color: None, bold: true, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: &[&str], hint: &str) -> FormField {
    FormField {
        id: id.into(),
        label: label.into(),
        kind,
        value: value.into(),
        options: options.iter().map(|s| (*s).to_owned()).collect(),
        hint: hint.into(),
    }
}

fn hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")
}

fn now() -> i64 {
    host::now().unix_millis / 1000
}

/// "2026-11-28 14:05", local time.
fn when(unix: i64) -> String {
    let t = unix + i64::from(host::now().utc_offset_minutes) * 60;
    let z = t.div_euclid(86_400) + 719_468;
    let secs = t.rem_euclid(86_400);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", secs / 3600, secs / 60 % 60)
}

// ----- storage -------------------------------------------------------------------------------

/// The stored review of the open folder (read once). No git.
fn read_stored() {
    if with(|s| s.loaded) {
        return;
    }
    // No folder open yet: read once there is one.
    let Ok(folder) = host::workspace_path() else { return };
    let project = hash(&folder);
    let review = model::read_all(&host::storage_read()).into_iter().find(|(p, _)| *p == project).map(|(_, r)| r);
    with(|s| {
        s.base_known = review.is_some();
        s.review = review.unwrap_or_default();
        s.project = project;
        s.loaded = true;
    });
}

/// The review, and the changes from git the first time the panel is used.
fn load() {
    read_stored();
    if with(|s| std::mem::replace(&mut s.fetched, true)) {
        return;
    }
    if with(|s| s.base_known) {
        refresh_files();
    } else {
        // The base at first: main or master, if the repository has one.
        start_git(Job::Branches, &["branch", "--list", "main", "master"], "");
    }
}

fn save() {
    let (project, review) = with(|s| (s.project.clone(), s.review.clone()));
    let mut all = model::read_all(&host::storage_read());
    all.retain(|(p, _)| *p != project);
    all.push((project, review));
    if let Err(e) = host::storage_write(&model::write_all(&all)) {
        with(|s| s.error = Some(format!("Couldn't keep the review: {e}")));
    }
}

// ----- git -------------------------------------------------------------------------------------

fn start_git(job: Job, args: &[&str], tag: &str) {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    match host::start_job("git", &args, "") {
        Ok(id) => with(|s| s.jobs.push((id, job, tag.to_owned()))),
        Err(e) => with(|s| {
            s.error =
                Some(if e.contains("isn't on PATH") { "Git isn't installed (or isn't on PATH)".into() } else { e })
        }),
    }
}

/// The base as git takes it ("" is the last commit).
fn base_rev() -> String {
    let base = with(|s| s.review.base.clone());
    if base.trim().is_empty() { "HEAD".into() } else { base.trim().to_owned() }
}

fn refresh_files() {
    let base = base_rev();
    if base.starts_with('-') {
        with(|s| s.error = Some("A base can't start with -".into()));
        return;
    }
    start_git(Job::Numstat, &["diff", "--numstat", "--no-renames", &base], "");
    start_git(Job::Untracked, &["ls-files", "--others", "--exclude-standard"], "");
}

fn job_done(job: JobResult) {
    let Some((kind, tag)) = with(|s| {
        let i = s.jobs.iter().position(|(id, ..)| *id == job.id)?;
        let (_, k, t) = s.jobs.remove(i);
        Some((k, t))
    }) else {
        return;
    };
    let ok = job.exit_code == Some(0);
    let why = || job.stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("git failed").to_owned();
    match kind {
        Job::Branches => {
            let names: Vec<String> =
                job.stdout.lines().map(|l| l.trim_start_matches(['*', ' ']).trim().to_owned()).collect();
            let base = ["main", "master"].into_iter().find(|b| names.iter().any(|n| n == b)).unwrap_or("");
            with(|s| s.review.base = base.to_owned());
            refresh_files();
        }
        Job::Numstat if ok => {
            let files = job
                .stdout
                .lines()
                .filter_map(|l| {
                    let mut p = l.splitn(3, '\t');
                    let (a, d, path) = (p.next()?, p.next()?, p.next()?);
                    Some((path.to_owned(), a.parse().ok().zip(d.parse().ok())))
                })
                .collect();
            with(|s| {
                s.files = files;
                s.error = None;
            });
        }
        Job::Numstat => with(|s| {
            s.files.clear();
            s.error = Some(format!("Couldn't compare with {}: {}", base_rev(), why()));
        }),
        Job::Untracked => {
            let list = if ok { job.stdout.lines().map(str::to_owned).collect() } else { Vec::new() };
            with(|s| s.untracked = list);
        }
        Job::Diff => {
            let text = if ok { job.stdout } else { format!("git: {}", why()) };
            with(|s| s.diff = Some((tag, text)));
        }
    }
    render();
}

// ----- comments in the editor ----------------------------------------------------------------

/// Moves the comments of `path` to where their lines are now and shows them above those lines.
fn refresh_file(path: &str) {
    if path.is_empty() {
        return;
    }
    let Ok(text) = host::read_file(path) else { return };
    let (lenses, changed) = with(|s| {
        let mut changed = false;
        let mut lenses = Vec::new();
        for c in s.review.comments.iter_mut().filter(|c| c.path == path) {
            match model::relocate(c, &text) {
                Some(line) => {
                    s.outdated.retain(|id| *id != c.id);
                    if line != c.line {
                        c.line = line;
                        changed = true;
                    }
                    if !c.resolved {
                        let first = c.text.lines().next().unwrap_or("").chars().take(80).collect::<String>();
                        let more = if c.replies.is_empty() {
                            String::new()
                        } else {
                            format!(" ({} repl{})", c.replies.len(), if c.replies.len() == 1 { "y" } else { "ies" })
                        };
                        lenses.push(CodeLens {
                            line,
                            title: format!("Review: {first}{more}"),
                            command: "focus".into(),
                            argument: c.id.to_string(),
                        });
                    }
                }
                None => {
                    if !s.outdated.contains(&c.id) {
                        s.outdated.push(c.id);
                    }
                }
            }
        }
        (lenses, changed)
    });
    host::set_code_lenses(path, &lenses);
    if changed {
        save();
    }
}

fn refresh_commented_files() {
    let mut paths: Vec<String> = with(|s| s.review.comments.iter().map(|c| c.path.clone()).collect());
    paths.sort();
    paths.dedup();
    for p in paths {
        refresh_file(&p);
    }
}

// ----- the panel -----------------------------------------------------------------------------

fn comment_rows(c: &Comment, outdated: bool, focus: bool) -> Vec<Block> {
    let mut b = Vec::new();
    let mut head = vec![bold(format!("{}:{}", c.path, c.line)), span(format!("  {}", when(c.time)), Some(DIM))];
    if c.resolved {
        head.push(span("  resolved", Some(GREEN)));
    }
    if outdated {
        head.push(span("  outdated: its line changed", Some(AMBER)));
    }
    if focus {
        head.push(span("  ◂", Some(AMBER)));
    }
    let id = c.id.to_string();
    let place = format!("{}:{}", c.path, c.line);
    let mut buttons = vec![button("Open", "open", &place), button("Reply", "reply", &id)];
    buttons.push(if c.resolved { button("Reopen", "reopen", &id) } else { button("Resolve", "resolve", &id) });
    buttons.push(button("Delete…", "delete", &id));
    b.push(Block::Rows(vec![Row { cells: head, buttons }]));
    if !c.anchor.is_empty() {
        b.push(Block::Spans(vec![Span { text: c.anchor.clone(), color: Some(DIM), bold: false, monospace: true }]));
    }
    b.push(Block::Text(c.text.clone()));
    for (t, text) in &c.replies {
        b.push(Block::Spans(vec![
            span("    ↳ ", Some(DIM)),
            span(text.clone(), None),
            span(format!("  {}", when(*t)), Some(DIM)),
        ]));
    }
    b
}

fn render() {
    let blocks = with(|s| {
        let r = &s.review;
        let mut b = Vec::new();
        b.push(Block::Form(Form {
            id: format!("base#{}", s.sent),
            fields: vec![field(
                "base",
                "Compare with",
                FieldKind::Text,
                &r.base,
                &[],
                "a branch or commit; empty: the last commit",
            )],
            buttons: vec!["Compare".into(), "Export as Markdown".into()],
        }));
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        if let Some(id) = s.deleting {
            b.push(Block::Spans(vec![span(
                "Delete this comment and its replies? It can't be brought back.",
                Some(RED),
            )]));
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![button("Delete", "delete-yes", &id.to_string()), button("Cancel", "cancel", "")],
            }]));
        }
        if let Some((path, line, anchor)) = &s.adding {
            b.push(Block::Heading(format!("Comment on {path}:{line}")));
            if !anchor.is_empty() {
                b.push(Block::Spans(vec![Span {
                    text: anchor.clone(),
                    color: Some(DIM),
                    bold: false,
                    monospace: true,
                }]));
            }
            b.push(Block::Form(Form {
                id: format!("add#{}", s.sent),
                fields: vec![field("text", "", FieldKind::Multiline, "", &[], "What should change, or a question")],
                buttons: vec!["Add comment".into(), "Cancel".into()],
            }));
        }
        if let Some(id) = s.replying {
            b.push(Block::Form(Form {
                id: format!("reply#{}#{id}", s.sent),
                fields: vec![field("text", "Reply", FieldKind::Multiline, "", &[], "")],
                buttons: vec!["Reply".into(), "Cancel".into()],
            }));
        }
        // Files changed.
        let (added, removed) = s.files.iter().filter_map(|f| f.1).fold((0, 0), |(a, d), (x, y)| (a + x, d + y));
        let base = if r.base.is_empty() { "the last commit".to_owned() } else { r.base.clone() };
        b.push(Block::Heading(format!("Changes against {base}")));
        if s.files.is_empty() && s.untracked.is_empty() {
            b.push(Block::Text("No changes.".into()));
        } else {
            b.push(Block::Spans(vec![
                span(
                    {
                        let n = s.files.len() + s.untracked.len();
                        format!("{n} file{} changed  ", if n == 1 { "" } else { "s" })
                    },
                    None,
                ),
                span(format!("+{added} "), Some(GREEN)),
                span(format!("−{removed}"), Some(RED)),
            ]));
            let count = |p: &str| r.comments.iter().filter(|c| c.path == p && !c.resolved).count();
            let mut rows: Vec<Row> = s
                .files
                .iter()
                .map(|(p, n)| {
                    let lines = n.map_or_else(|| "binary".to_owned(), |(a, d)| format!("+{a} −{d}"));
                    let shown = s.diff.as_ref().is_some_and(|(d, _)| d == p);
                    let mut cells = vec![span(p.clone(), None), span(lines, Some(DIM))];
                    if count(p) > 0 {
                        cells.push(span(
                            format!("{} open comment{}", count(p), if count(p) == 1 { "" } else { "s" }),
                            Some(AMBER),
                        ));
                    }
                    Row {
                        cells,
                        buttons: vec![
                            button(if shown { "Hide diff" } else { "Diff" }, "diff", p),
                            button("Open", "open", &format!("{p}:1")),
                        ],
                    }
                })
                .collect();
            rows.extend(s.untracked.iter().map(|p| Row {
                cells: vec![span(p.clone(), None), span("new, not in Git yet", Some(DIM))],
                buttons: vec![button("Open", "open", &format!("{p}:1"))],
            }));
            b.push(Block::Rows(rows));
        }
        if let Some((p, d)) = &s.diff {
            b.push(Block::Heading(p.clone()));
            b.push(Block::Diff(d.clone()));
        }
        // Comments, the one picked first, then open ones, then resolved.
        let open = r.comments.iter().filter(|c| !c.resolved).count();
        b.push(Block::Heading(format!("Comments ({open} open, {} resolved)", r.comments.len() - open)));
        if r.comments.is_empty() {
            b.push(Block::Text(
                "None yet. In a file, Review: Comment on This Line (or the editor's command palette) adds one at the cursor."
                    .into(),
            ));
        }
        let mut order: Vec<&Comment> = r.comments.iter().collect();
        order.sort_by_key(|c| (Some(c.id) != s.focus, c.resolved, c.path.clone(), c.line));
        for c in order {
            b.push(Block::Separator);
            b.extend(comment_rows(c, s.outdated.contains(&c.id), Some(c.id) == s.focus));
        }
        // The verdict.
        b.push(Block::Separator);
        b.push(Block::Heading("Verdict".into()));
        let verdict = match r.verdict.as_str() {
            "approve" => "Approve",
            "changes" => "Request changes",
            _ => "Comment",
        };
        b.push(Block::Form(Form {
            id: format!("verdict#{}", s.sent),
            fields: vec![
                field("verdict", "", FieldKind::Choice, verdict, &["Comment", "Approve", "Request changes"], ""),
                field("", "", FieldKind::LineBreak, "", &[], ""),
                field("summary", "", FieldKind::Multiline, &r.summary, &[], "A summary for the author"),
            ],
            buttons: vec!["Save verdict".into()],
        }));
        b
    });
    host::set_panel("review", &blocks);
}

// ----- commands --------------------------------------------------------------------------------

fn comment_here() -> Result<(), String> {
    let doc = host::active_document()?.ok_or("Open a file of the folder first")?;
    if doc.path.is_empty() {
        return Err("Save the file in the open folder first".into());
    }
    let anchor = doc.text.lines().nth(doc.line.saturating_sub(1) as usize).unwrap_or("").trim().to_owned();
    with(|s| {
        s.adding = Some((doc.path.clone(), doc.line, anchor));
        s.replying = None;
    });
    render();
    Ok(())
}

fn submitted(f: FormValues) -> Result<(), String> {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    let kind = f.form.split('#').next().unwrap_or("");
    with(|s| s.sent += 1);
    match kind {
        "base" if f.button == "Export as Markdown" => export(),
        "base" => {
            let base = get("base").trim().to_owned();
            if base.starts_with('-') || base.chars().any(char::is_whitespace) {
                return Err("A base is a branch or a commit, like main or a1b2c3d".into());
            }
            with(|s| {
                s.review.base = base;
                s.diff = None;
            });
            save();
            refresh_files();
        }
        "add" if f.button == "Cancel" => with(|s| s.adding = None),
        "add" => {
            let text = get("text").trim().to_owned();
            if text.is_empty() {
                return Err("Write the comment first".into());
            }
            let Some((path, line, anchor)) = with(|s| s.adding.take()) else { return Ok(()) };
            let id = host::random() >> 12;
            with(|s| {
                s.review.comments.push(Comment {
                    id,
                    path: path.clone(),
                    line,
                    anchor,
                    text,
                    time: now(),
                    resolved: false,
                    replies: vec![],
                });
                s.focus = Some(id);
            });
            save();
            refresh_file(&path);
        }
        "reply" if f.button == "Cancel" => with(|s| s.replying = None),
        "reply" => {
            let text = get("text").trim().to_owned();
            let id: u64 = f.form.rsplit('#').next().and_then(|n| n.parse().ok()).ok_or("no comment")?;
            if !text.is_empty() {
                with(|s| {
                    if let Some(c) = s.review.comments.iter_mut().find(|c| c.id == id) {
                        c.replies.push((now(), text));
                    }
                    s.replying = None;
                });
                save();
                refresh_commented_files();
            }
        }
        "verdict" => {
            let verdict = match get("verdict").as_str() {
                "Approve" => "approve",
                "Request changes" => "changes",
                _ => "comment",
            };
            with(|s| {
                s.review.verdict = verdict.to_owned();
                s.review.summary = get("summary").trim().to_owned();
            });
            save();
            host::show_message("Verdict saved");
        }
        _ => {}
    }
    Ok(())
}

fn export() {
    let text = with(|s| {
        let files: Vec<(String, u32, u32)> =
            s.files.iter().map(|(p, n)| (p.clone(), n.map_or(0, |x| x.0), n.map_or(0, |x| x.1))).collect();
        model::markdown(&s.review, &files, when)
    });
    host::open_text(&text);
}

fn set_resolved(id: u64, resolved: bool) {
    with(|s| {
        if let Some(c) = s.review.comments.iter_mut().find(|c| c.id == id) {
            c.resolved = resolved;
        }
    });
    save();
    refresh_commented_files();
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    load();
    let num = || arg.parse::<u64>().map_err(|_| "no comment".to_owned());
    match id {
        "show" => refresh_commented_files(),
        "comment" => return comment_here(),
        "export" => export(),
        "open" => {
            let (path, line) = arg.rsplit_once(':').ok_or("no place")?;
            host::open_location(path, line.parse().unwrap_or(1))?;
            return Ok(());
        }
        "diff" => {
            let shown = with(|s| s.diff.as_ref().is_some_and(|(p, _)| p == arg));
            if shown {
                with(|s| s.diff = None);
            } else {
                let base = base_rev();
                start_git(Job::Diff, &["diff", &base, "--", arg], arg);
            }
        }
        "focus" => with(|s| s.focus = num().ok()),
        "reply" => with(|s| {
            s.replying = num().ok();
            s.adding = None;
        }),
        "resolve" => set_resolved(num()?, true),
        "reopen" => set_resolved(num()?, false),
        "delete" => with(|s| s.deleting = num().ok()),
        "cancel" => with(|s| s.deleting = None),
        "delete-yes" => {
            let id = num()?;
            let path = with(|s| {
                s.deleting = None;
                let path = s.review.comments.iter().find(|c| c.id == id).map(|c| c.path.clone());
                s.review.comments.retain(|c| c.id != id);
                path
            });
            save();
            if let Some(p) = path {
                refresh_file(&p);
            }
        }
        _ => return Err(format!("unknown command {id}")),
    }
    render();
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        // Files reopened from the last session were open before this started: their comments
        // go above their lines now. Git isn't run until the panel is used.
        read_stored();
        if with(|s| !s.review.comments.is_empty()) {
            refresh_commented_files();
        }
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        let r = command(&id, &argument);
        if let Err(e) = &r {
            with(|s| s.error = Some(e.clone()));
            render();
        }
        r
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => job_done(job),
            Event::FormSubmitted(f) => {
                if let Err(e) = submitted(f) {
                    with(|s| s.error = Some(e));
                } else {
                    with(|s| s.error = None);
                }
                render();
            }
            // Comments show above their lines in each file as it opens or is saved.
            Event::Opened(path) | Event::Saved(path) | Event::ActiveChanged(path) if !path.is_empty() => {
                read_stored();
                refresh_file(&path);
            }
            _ => {}
        }
    }
}

export!(Plugin);
