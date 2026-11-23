//! GitHub Actions: the repository's latest workflow runs, a run's jobs and steps, the log of
//! what failed, re-running the failed jobs, and each failed step's place in its workflow file.
//! Everything goes through the `gh` program, run as an approved job, with the user's own login.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod actions;

use std::collections::HashMap;
use std::sync::Mutex;

use actions::{Job, LogLine, Run};
use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, JobResult, Row, Span};

const RED: u32 = 0xE5534B;
const GREEN: u32 = 0x4CB86A;
const AMBER: u32 = 0xE09A3E;
const DIM: u32 = 0x8B949E;

/// Lines of a whole log shown at most.
const LOG_MAX: usize = 3000;

enum Then {
    Runs,
    Jobs(u64),
    Log(u64),
    Rerun,
}

#[derive(Default)]
struct State {
    jobs_running: HashMap<u64, Then>,
    busy: Option<String>,
    note: Option<String>,
    runs: Option<Result<Vec<Run>, String>>,
    selected: Option<u64>,
    jobs: Option<Result<Vec<Job>, String>>,
    log: Option<Result<Vec<LogLine>, String>>,
    failed_only: bool,
    whole_log: bool,
    confirm_rerun: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn gh(args: Vec<String>, then: Then, busy: &str) -> Result<(), String> {
    let id = host::start_job("gh", &args, "").map_err(|e| {
        if e.contains("isn't on PATH") {
            "The gh program isn't installed: get GitHub CLI (cli.github.com), then gh auth login".to_owned()
        } else {
            e
        }
    })?;
    with(|s| {
        s.jobs_running.insert(id, then);
        s.busy = Some(busy.to_owned());
    });
    Ok(())
}

fn refresh() {
    let failed_only = with(|s| s.failed_only);
    if let Err(e) = gh(actions::runs_args(failed_only), Then::Runs, "Asking GitHub…") {
        with(|s| s.runs = Some(Err(e)));
    }
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs_running.remove(&job.id)) else { return };
    let ok = job.exit_code == Some(0);
    let said = || actions::explain(&job.stderr);
    if with(|s| s.jobs_running.is_empty()) {
        with(|s| s.busy = None);
    }
    with(|s| match then {
        Then::Runs => s.runs = Some(if ok { actions::parse_runs(&job.stdout) } else { Err(said()) }),
        Then::Jobs(id) if s.selected == Some(id) => {
            s.jobs = Some(if ok { actions::parse_jobs(&job.stdout) } else { Err(said()) });
        }
        Then::Log(id) if s.selected == Some(id) => {
            s.log = Some(if ok { Ok(actions::parse_failed_log(&job.stdout)) } else { Err(said()) });
        }
        Then::Rerun => {
            s.note =
                Some(if ok { "GitHub is running the failed jobs again: Refresh shows it".to_owned() } else { said() });
        }
        _ => {}
    });
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

/// A run's or step's state in a color: green done, red failed, amber going.
fn state_color(state: &str) -> Option<u32> {
    Some(match state {
        "success" => GREEN,
        "failure" | "timed_out" | "startup_failure" => RED,
        "in_progress" | "queued" | "waiting" | "pending" | "requested" => AMBER,
        _ => DIM,
    })
}

fn render() {
    let now = host::now().unix_millis / 1000;
    let blocks = with(|s| {
        let mut b = Vec::new();
        let only = if s.failed_only { "▸ Only failed" } else { "Only failed" };
        b.push(bar(vec![button("Refresh", "refresh", ""), button(only, "only-failed", "")]));
        let mut line = vec![span("Through gh, with your GitHub login", Some(DIM))];
        if let Some(busy) = &s.busy {
            line.push(span(format!("   {busy}"), Some(DIM)));
        }
        b.push(Block::Spans(line));
        if let Some(note) = &s.note {
            b.push(Block::Spans(vec![span(note, Some(AMBER))]));
        }
        let runs = match &s.runs {
            None => return b,
            Some(Err(e)) => {
                b.push(Block::Spans(vec![span(e, Some(RED))]));
                return b;
            }
            Some(Ok(runs)) => runs,
        };
        if runs.is_empty() {
            b.push(Block::Text("No workflow runs in this repository yet.".into()));
            return b;
        }
        if let Some(run) = s.selected.and_then(|id| runs.iter().find(|r| r.database_id == id)) {
            details(s, run, &mut b);
        }
        let rows = runs
            .iter()
            .map(|r| {
                let state = r.state();
                let age = actions::unix_seconds(&r.created_at)
                    .map(|t| actions::relative_time((now - t).max(0)))
                    .unwrap_or_default();
                let chosen = s.selected == Some(r.database_id);
                Row {
                    cells: vec![
                        span("●", state_color(state)),
                        Span { text: r.display_title.clone(), color: None, bold: chosen, monospace: false },
                        span(
                            format!("{} · {} · {} · {age}", r.workflow_name, r.head_branch, state.replace('_', " ")),
                            Some(DIM),
                        ),
                    ],
                    buttons: vec![button(if chosen { "▸ Jobs" } else { "Jobs" }, "select", &r.database_id.to_string())],
                }
            })
            .collect();
        b.push(Block::Rows(rows));
        b
    });
    host::set_panel("actions", &blocks);
}

fn details(s: &State, run: &Run, b: &mut Vec<Block>) {
    let id = run.database_id.to_string();
    b.push(Block::Heading(format!("{}: {}", run.workflow_name, run.display_title)));
    let mut buttons = vec![button("Open on GitHub", "open", &run.url)];
    if run.state() == "failure" {
        buttons.push(button("Log of what failed", "log", &id));
        if s.confirm_rerun {
            buttons.push(button("Run them again", "rerun", &id));
            buttons.push(button("Keep", "rerun-keep", ""));
        } else {
            buttons.push(button("Re-run failed jobs…", "rerun-ask", ""));
        }
    }
    buttons.push(button("Close", "select", ""));
    b.push(bar(buttons));
    if s.confirm_rerun {
        b.push(Block::Spans(vec![span("GitHub runs the failed jobs again, on its minutes.", Some(AMBER))]));
    }
    match &s.jobs {
        None => {}
        Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
        Some(Ok(jobs)) => {
            for job in jobs {
                let state = if job.status == "completed" { &job.conclusion } else { &job.status };
                b.push(Block::Spans(vec![
                    span("● ", state_color(state)),
                    Span { text: job.name.clone(), color: None, bold: true, monospace: false },
                ]));
                // Steps that failed, and the one running; the rest only when the job failed.
                let rows: Vec<Row> = job
                    .steps
                    .iter()
                    .filter_map(|step| {
                        let st = if step.status == "completed" { &step.conclusion } else { &step.status };
                        let notable = st != "success" && st != "skipped";
                        if !notable && state != "failure" {
                            return None;
                        }
                        let buttons = if st == "failure" {
                            vec![button(
                                "Open in workflow",
                                "open-step",
                                &format!("{}\n{}", run.workflow_name, step.name),
                            )]
                        } else {
                            vec![]
                        };
                        Some(Row {
                            cells: vec![span(format!("    {}. {}", step.number, step.name), state_color(st))],
                            buttons,
                        })
                    })
                    .collect();
                if !rows.is_empty() {
                    b.push(Block::Rows(rows));
                }
            }
        }
    }
    match &s.log {
        None => {}
        Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
        Some(Ok(lines)) => {
            let errors = lines.iter().filter(|l| l.error).count();
            b.push(Block::Spans(vec![span(format!("{} lines, {errors} errors (in red)", lines.len()), Some(DIM))]));
            let whole = if s.whole_log { "▸ Whole log" } else { "Whole log" };
            b.push(bar(vec![button(whole, "whole-log", "")]));
            let shown = actions::shown_lines(lines, s.whole_log);
            let mut last = None;
            for (count, (i, l)) in lines.iter().enumerate().filter(|(i, _)| shown[*i]).enumerate() {
                if count == LOG_MAX {
                    b.push(Block::Spans(vec![span(format!("… {} more lines", lines.len() - i), Some(DIM))]));
                    break;
                }
                if last.is_some_and(|p: usize| p + 1 != i) {
                    b.push(Block::Spans(vec![span("⋯", Some(DIM))]));
                }
                last = Some(i);
                let color = if l.error { Some(RED) } else { None };
                b.push(Block::Spans(vec![Span { text: l.text.clone(), color, bold: false, monospace: true }]));
            }
        }
    }
    b.push(Block::Separator);
}

// ----- commands -----------------------------------------------------------------------------

/// The workflow file named `workflow` (by its `name:`, or its file name), and the step's line.
fn open_step(workflow: &str, step: &str) -> Result<(), String> {
    let dir = ".github/workflows";
    let files: Vec<String> = host::read_dir(dir)?
        .into_iter()
        .map(|e| e.name)
        .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        .map(|n| format!("{dir}/{n}"))
        .collect();
    let named =
        files.iter().find(|f| host::read_file(f).ok().and_then(|t| actions::top_name(&t)).as_deref() == Some(workflow));
    let file = named
        .or_else(|| files.iter().find(|f| f.rsplit('/').next() == Some(workflow)))
        .ok_or(format!("No workflow file here is named {workflow}"))?;
    let line = host::read_file(file).ok().and_then(|t| actions::step_line(&t, step)).unwrap_or(1);
    host::open_location(file, line as u32)
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let result = match id {
        "show" | "refresh" => {
            refresh();
            Ok(())
        }
        "only-failed" => {
            with(|s| s.failed_only = !s.failed_only);
            refresh();
            Ok(())
        }
        "select" => {
            let run = arg.parse::<u64>().ok();
            with(|s| {
                s.selected = run;
                s.jobs = None;
                s.log = None;
                s.confirm_rerun = false;
            });
            match run {
                Some(run) => {
                    let a = vec!["run".into(), "view".into(), run.to_string(), "--json".into(), "jobs".into()];
                    gh(a, Then::Jobs(run), "Reading the run…")
                }
                None => Ok(()),
            }
        }
        "log" => {
            let run: u64 = arg.parse().map_err(|_| "no run")?;
            with(|s| s.log = None);
            let a = vec!["run".into(), "view".into(), run.to_string(), "--log-failed".into()];
            gh(a, Then::Log(run), "Reading the log…")
        }
        "whole-log" => {
            with(|s| s.whole_log = !s.whole_log);
            Ok(())
        }
        "rerun-ask" => {
            with(|s| s.confirm_rerun = true);
            Ok(())
        }
        "rerun-keep" => {
            with(|s| s.confirm_rerun = false);
            Ok(())
        }
        "rerun" => {
            with(|s| s.confirm_rerun = false);
            let a = vec!["run".into(), "rerun".into(), arg.to_owned(), "--failed".into()];
            gh(a, Then::Rerun, "Asking GitHub to run them again…")
        }
        "open" => host::open_url(arg),
        "open-step" => {
            let (workflow, step) = arg.split_once('\n').ok_or("no step")?;
            open_step(workflow, step)
        }
        _ => return Err(format!("unknown command {id}")),
    };
    if let Err(e) = result {
        with(|s| s.note = Some(e));
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
        if let Event::JobFinished(job) = event {
            finished(job);
        }
    }
}

export!(Plugin);
