//! Benchmarks: runs the project's benchmarks (`cargo bench`, Criterion's or libtest's, or
//! `go test -bench`), keeps each run with the commit it was made at, and shows how each
//! benchmark changed: the latest run against the one before (slower than the threshold: a
//! regression, in red), a plot over the runs, and any two runs side by side.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod bench;

use std::sync::Mutex;

use bench::Run;
use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobLine, JobResult, LogLine, LogView, PlotSeries, Row, Span,
};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x3FB950;
const DIM: u32 = 0x8B949E;
/// Runs kept per project.
const KEEP: usize = 100;
/// Output lines kept on screen while a run goes.
const LOG_LINES: usize = 400;

#[derive(Default)]
struct State {
    project: String,
    runs: Vec<Run>,
    loaded: bool,
    threshold: f64,
    /// The run under way: its job, the commit, and what it printed.
    job: Option<u64>,
    stage: Stage,
    commit: String,
    output: Vec<String>,
    note: Option<(String, u32)>,
    /// The two runs compared (by time), when picked.
    compare: Option<(i64, i64)>,
    /// Forgetting the runs waits for a yes.
    confirm: bool,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Stage {
    #[default]
    Idle,
    Commit,
    Dirty,
    Bench,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: String::new() }
}

fn hash(s: &str) -> String {
    let h = s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{h:016x}")
}

/// "2026-11-28 10:15", local time.
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

fn label(r: &Run) -> String {
    let commit = if r.commit.is_empty() { "no commit".to_owned() } else { r.commit.clone() };
    format!("{commit} · {}", when(r.time))
}

// ----- storage -------------------------------------------------------------------------------

fn load() {
    let project = host::workspace_path().map(|p| hash(&p)).unwrap_or_default();
    let text = host::storage_read();
    let threshold =
        text.lines().find_map(|l| l.strip_prefix("threshold\t")).and_then(|v| v.trim().parse().ok()).unwrap_or(10.0);
    let all = bench::read_history(&text);
    with(|s| {
        s.runs = all.into_iter().filter(|r| r.project == project).collect();
        s.project = project;
        s.threshold = threshold;
        s.loaded = true;
    });
}

fn save(new: Option<Run>) -> Result<(), String> {
    let text = host::storage_read();
    let mut all = bench::read_history(&text);
    all.extend(new);
    let threshold = with(|s| s.threshold);
    host::storage_write(&format!("threshold\t{threshold}\n{}", bench::write_history(&all, KEEP)))
}

// ----- running -------------------------------------------------------------------------------

/// The program and arguments that run this folder's benchmarks.
fn runner() -> Result<(&'static str, Vec<String>), String> {
    if host::read_file("Cargo.toml").is_ok() {
        // Every member of a workspace, not only its default ones.
        return Ok(("cargo", vec!["bench".into(), "--workspace".into()]));
    }
    if host::read_file("go.mod").is_ok() {
        return Ok(("go", ["test", "-run", "^$", "-bench", ".", "./..."].map(str::to_owned).to_vec()));
    }
    Err("No Cargo.toml or go.mod at the top of the open folder: nothing to run benchmarks with".into())
}

fn start_run() -> Result<(), String> {
    if with(|s| s.job.is_some()) {
        return Err("Benchmarks are already running".into());
    }
    runner()?;
    with(|s| {
        s.output.clear();
        s.note = None;
        s.commit.clear();
    });
    // The commit first; without git (or a repository) the run is kept with none.
    match host::start_job("git", &["rev-parse".into(), "--short".into(), "HEAD".into()], "") {
        Ok(id) => with(|s| {
            s.job = Some(id);
            s.stage = Stage::Commit;
        }),
        Err(_) => start_bench()?,
    }
    render();
    Ok(())
}

fn start_bench() -> Result<(), String> {
    let (program, args) = runner()?;
    let id = host::start_job_lines(program, &args, "").map_err(|e| {
        if e.contains("isn't on PATH") { format!("{program} isn't installed (or isn't on PATH)") } else { e }
    })?;
    with(|s| {
        s.job = Some(id);
        s.stage = Stage::Bench;
        s.output.push(format!("$ {program} {}", args.join(" ")));
    });
    Ok(())
}

fn job_done(job: JobResult) {
    let stage = with(|s| if s.job == Some(job.id) { s.stage } else { Stage::Idle });
    match stage {
        Stage::Idle => {}
        Stage::Commit => {
            let ok = job.exit_code == Some(0);
            with(|s| s.commit = if ok { job.stdout.trim().to_owned() } else { String::new() });
            let next = if ok {
                host::start_job("git", &["status".into(), "--porcelain".into(), "--untracked-files=no".into()], "").map(
                    |id| {
                        with(|s| {
                            s.job = Some(id);
                            s.stage = Stage::Dirty;
                        })
                    },
                )
            } else {
                start_bench()
            };
            if let Err(e) = next.or_else(|_| start_bench()) {
                fail(e);
            }
        }
        Stage::Dirty => {
            if job.exit_code == Some(0) && !job.stdout.trim().is_empty() {
                with(|s| s.commit.push('+'));
            }
            if let Err(e) = start_bench() {
                fail(e);
            }
        }
        Stage::Bench => finished(job.exit_code),
    }
    render();
}

fn fail(e: String) {
    with(|s| {
        s.job = None;
        s.stage = Stage::Idle;
        s.note = Some((e, RED));
    });
    render();
}

fn finished(code: Option<i32>) {
    let (output, project, commit) = with(|s| {
        s.job = None;
        s.stage = Stage::Idle;
        (s.output.join("\n"), s.project.clone(), s.commit.clone())
    });
    let results = bench::parse(&output);
    if results.is_empty() {
        let why = match code {
            None => "Stopped".to_owned(),
            Some(0) => "The run printed no benchmark results (does the project have benchmarks?)".to_owned(),
            Some(c) => format!("The benchmarks failed (exit code {c}): see the output below"),
        };
        with(|s| s.note = Some((why, RED)));
        return;
    }
    let run = Run { project, commit, time: host::now().unix_millis / 1000, results };
    let previous = with(|s| s.runs.last().cloned());
    let threshold = with(|s| s.threshold);
    if let Err(e) = save(Some(run.clone())) {
        with(|s| s.note = Some((format!("Couldn't keep the run: {e}"), RED)));
    }
    let note = match previous.as_ref().map(|p| bench::regressions(p, &run, threshold)) {
        Some(slow) if !slow.is_empty() => {
            let n = slow.len();
            host::show_message(&format!(
                "{n} benchmark{} slower than the run before by more than {threshold}%",
                if n == 1 { "" } else { "s" }
            ));
            (format!("{n} slower than the run before by more than {threshold}%"), RED)
        }
        Some(_) => (format!("{} benchmarks recorded; none slower than the run before", run.results.len()), GREEN),
        None => (format!("{} benchmarks recorded: the next run is compared with this one", run.results.len()), GREEN),
    };
    with(|s| {
        s.runs.push(run);
        s.compare = None;
        s.note = Some(note);
    });
}

fn job_line(line: JobLine) {
    let mine = with(|s| s.job == Some(line.id) && s.stage == Stage::Bench);
    if mine {
        with(|s| s.output.push(line.line));
        render();
    }
}

// ----- the panel -----------------------------------------------------------------------------

/// The table of `new` against `old`.
fn table(old: Option<&Run>, new: &Run, threshold: f64) -> Block {
    Block::Rows(
        new.results
            .iter()
            .map(|(name, ns)| {
                let before = old.and_then(|o| o.results.iter().find(|(n, _)| n == name)).map(|(_, v)| *v);
                let mut cells = vec![
                    Span { text: name.clone(), color: None, bold: true, monospace: false },
                    span(bench::human(*ns), None),
                ];
                if let Some(b) = before {
                    let c = bench::change(b, *ns);
                    let color = if c > threshold {
                        Some(RED)
                    } else if c < -threshold {
                        Some(GREEN)
                    } else {
                        Some(DIM)
                    };
                    cells.push(span(format!("was {}", bench::human(b)), Some(DIM)));
                    cells.push(span(format!("{c:+.1}%"), color));
                } else if old.is_some() {
                    cells.push(span("new", Some(DIM)));
                }
                Row { cells, buttons: vec![] }
            })
            .collect(),
    )
}

fn render() {
    let blocks = with(|s| {
        let running = s.job.is_some();
        let mut b = vec![Block::Rows(vec![Row {
            cells: vec![],
            buttons: if running {
                vec![button("Stop", "stop", "")]
            } else {
                vec![button("Run benchmarks", "run", ""), button("Forget runs…", "forget", "")]
            },
        }])];
        b.push(Block::Form(Form {
            id: "threshold".into(),
            fields: vec![field(
                "percent",
                "Slower by more than this % is a regression",
                FieldKind::Text,
                &format!("{}", s.threshold),
                vec![],
            )],
            buttons: vec!["Set".into()],
        }));
        if s.confirm {
            b.push(Block::Spans(vec![span("Forget every run of this project? It can't be undone.", Some(RED))]));
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![button("Forget them", "forget-all", ""), button("Cancel", "cancel", "")],
            }]));
        }
        if let Some((text, color)) = &s.note {
            b.push(Block::Spans(vec![span(text, Some(*color))]));
        }
        if running || (!s.output.is_empty() && s.note.as_ref().is_some_and(|(_, c)| *c == RED)) {
            let from = s.output.len().saturating_sub(LOG_LINES);
            let lines = s.output[from..]
                .iter()
                .map(|t| LogLine { text: t.clone(), color: None, path: String::new(), line: 0 })
                .collect();
            b.push(Block::Log(LogView { lines, follow: true }));
        }
        let Some(last) = s.runs.last() else {
            b.push(Block::Text(
                "No runs yet. Run benchmarks runs cargo bench (Criterion or #[bench]) or go test -bench, \
                 and keeps the results with the commit."
                    .into(),
            ));
            return b;
        };
        let previous = s.runs.len().checked_sub(2).map(|i| &s.runs[i]);
        b.push(Block::Heading(match previous {
            Some(p) => format!("Latest run, {} (against {})", label(last), label(p)),
            None => format!("Latest run, {}", label(last)),
        }));
        b.push(table(previous, last, s.threshold));
        if s.runs.len() >= 2 {
            b.push(Block::Heading(format!("Over the last {} runs, as % of the first", s.runs.len())));
            let mut names: Vec<&str> = last.results.iter().map(|(n, _)| n.as_str()).collect();
            names.truncate(8);
            let series = names
                .iter()
                .map(|name| {
                    let values: Vec<f64> = s
                        .runs
                        .iter()
                        .filter_map(|r| r.results.iter().find(|(n, _)| n == name).map(|(_, v)| *v))
                        .collect();
                    let first = values.first().copied().unwrap_or(1.0).max(f64::MIN_POSITIVE);
                    PlotSeries { name: (*name).to_owned(), values: values.iter().map(|v| v / first * 100.0).collect() }
                })
                .collect();
            b.push(Block::Plot(series));
            let options: Vec<String> = s.runs.iter().rev().map(label).collect();
            let (a, z) = s.compare.unwrap_or((s.runs[s.runs.len() - 2].time, last.time));
            let name_of = |t: i64| s.runs.iter().find(|r| r.time == t).map(label).unwrap_or_default();
            b.push(Block::Form(Form {
                id: "compare".into(),
                fields: vec![
                    field("old", "Compare", FieldKind::Choice, &name_of(a), options.clone()),
                    field("new", "with", FieldKind::Choice, &name_of(z), options),
                ],
                buttons: vec!["Compare".into()],
            }));
            if let Some((a, z)) = s.compare {
                let old = s.runs.iter().find(|r| r.time == a);
                if let (Some(old), Some(new)) = (old, s.runs.iter().find(|r| r.time == z)) {
                    b.push(Block::Heading(format!("{} → {}", label(old), label(new))));
                    b.push(table(Some(old), new, s.threshold));
                }
            }
        }
        b
    });
    host::set_panel("benchmarks", &blocks);
}

fn submitted(f: FormValues) -> Result<(), String> {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    match f.form.as_str() {
        "threshold" => {
            let v: f64 = get("percent").trim().trim_end_matches('%').parse().map_err(|_| "Type a number, like 10")?;
            if !(0.0..=1000.0).contains(&v) {
                return Err("Type a number from 0 to 1000".into());
            }
            with(|s| s.threshold = v);
            save(None)?;
        }
        "compare" => {
            let time_of = |l: &str| with(|s| s.runs.iter().find(|r| label(r) == l).map(|r| r.time));
            let (a, z) = (time_of(&get("old")).ok_or("Pick a run")?, time_of(&get("new")).ok_or("Pick a run")?);
            with(|s| s.compare = Some((a, z)));
        }
        _ => {}
    }
    Ok(())
}

fn command(id: &str) -> Result<(), String> {
    if !with(|s| s.loaded) {
        load();
    }
    match id {
        "show" => {}
        "run" => start_run()?,
        "stop" => {
            if let Some(job) = with(|s| s.job) {
                host::cancel_job(job);
            }
        }
        "forget" => with(|s| s.confirm = true),
        "cancel" => with(|s| s.confirm = false),
        _ => return Err(format!("unknown command {id}")),
    }
    render();
    Ok(())
}

fn forget_all() -> Result<(), String> {
    let project = with(|s| s.project.clone());
    let text = host::storage_read();
    let kept: Vec<Run> = bench::read_history(&text).into_iter().filter(|r| r.project != project).collect();
    let threshold = with(|s| s.threshold);
    host::storage_write(&format!("threshold\t{threshold}\n{}", bench::write_history(&kept, KEEP)))?;
    with(|s| {
        s.runs.clear();
        s.compare = None;
        s.confirm = false;
        s.note = Some(("Runs forgotten".into(), DIM));
    });
    render();
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        if id == "forget-all" {
            return forget_all();
        }
        let r = command(&id);
        if let Err(e) = &r {
            with(|s| s.note = Some((e.clone(), RED)));
            render();
        }
        r
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => job_done(job),
            Event::JobLine(line) => job_line(line),
            Event::FormSubmitted(f) => {
                if let Err(e) = submitted(f) {
                    with(|s| s.note = Some((e, RED)));
                }
                render();
            }
            _ => {}
        }
    }
}

export!(Plugin);
