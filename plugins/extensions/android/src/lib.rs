//! Android: the devices `adb` sees, each with a logcat view (filtered by level, text and app),
//! a shell and APK installs, in a panel. Stack frames in log lines that are this project's
//! files are links. Coiman talks to no device itself; `adb` does, as an approved program.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod adb;

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use adb::{Device, Filter, LogLine};
use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobLine, JobResult, LogLine as Line, LogView, Row, Span,
};

const RED: u32 = 0xE5534B;
const YELLOW: u32 = 0xCCA700;
const GREEN: u32 = 0x57C06A;
const ORANGE: u32 = 0xE2A93B;
const DIM: u32 = 0x8B949E;

/// Log lines kept; older ones go.
const MAX_LINES: usize = 50_000;
/// Lines shown, the newest that match.
const SHOWN: usize = 10_000;
/// How often the panel is drawn again while lines come, in milliseconds.
const DRAW_EVERY: i64 = 250;
/// Project files read for linking stack frames.
const MAX_FILES: u32 = 10_000;

fn missing() -> String {
    "adb isn't installed: install Android's SDK Platform-Tools (on PATH, in the Android SDK, or unpacked into the config folder's tools/platform-tools)".to_owned()
}

/// What a running adb is for.
enum Then {
    Devices,
    Logcat,
    Pids(String),
}

struct State {
    devices: Option<Result<Vec<Device>, String>>,
    /// The device being watched, and logcat's job.
    logcat: Option<(String, u64)>,
    lines: VecDeque<LogLine>,
    filter: Filter,
    package: String,
    paused: bool,
    /// The project's Java and Kotlin files, for linking stack frames.
    files: Vec<String>,
    /// APKs built in the project.
    apks: Vec<String>,
    jobs: HashMap<u64, Then>,
    /// A draw is waiting for the next frame.
    waiting: bool,
    drawn_at: i64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            devices: None,
            logcat: None,
            lines: VecDeque::new(),
            filter: Filter { min_level: adb::severity('I'), ..Filter::default() },
            package: String::new(),
            paused: false,
            files: Vec::new(),
            apks: Vec::new(),
            jobs: HashMap::new(),
            waiting: false,
            drawn_at: 0,
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Starts adb with `args`; its result comes back as `job-finished`.
fn start(args: &[&str], then: Then) -> Result<(), String> {
    let args = args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
    let id = host::start_job("adb", &args, "").map_err(|e| if e.contains("isn't on PATH") { missing() } else { e })?;
    with(|s| s.jobs.insert(id, then));
    Ok(())
}

fn refresh() {
    if let Err(e) = start(&["devices", "-l"], Then::Devices) {
        with(|s| s.devices = Some(Err(e)));
    }
    with(|s| s.apks = apks());
    render();
}

/// APKs under each module's `build/outputs/apk` (relative paths).
fn apks() -> Vec<String> {
    fn walk(dir: &str, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = host::read_dir(dir) else { return };
        for e in entries {
            let path = format!("{dir}/{}", e.name);
            if e.is_dir && depth < 3 {
                walk(&path, depth + 1, out);
            } else if !e.is_dir && adb::is_apk(&e.name) {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    let Ok(top) = host::read_dir("") else { return out };
    for module in top.iter().filter(|e| e.is_dir && !e.name.starts_with('.')) {
        let outputs = format!("{}/build/outputs/apk", module.name);
        if host::read_dir(&outputs).is_ok() {
            walk(&outputs, 0, &mut out);
        }
    }
    out.sort();
    out
}

fn start_logcat(serial: &str) {
    stop_logcat();
    // The last few thousand lines, then what comes.
    let args = ["-s", serial, "logcat", "-v", "threadtime", "-T", "2000"].map(str::to_owned);
    match host::start_job_lines("adb", &args, "") {
        Ok(id) => {
            // The project's files as they are now, for linking stack frames.
            let mut files = host::list_files(".kt", MAX_FILES).unwrap_or_default();
            files.extend(host::list_files(".java", MAX_FILES).unwrap_or_default());
            with(|s| {
                s.lines.clear();
                s.files = files;
                s.logcat = Some((serial.to_owned(), id));
                s.jobs.insert(id, Then::Logcat);
            });
        }
        Err(e) => host::show_message(&format!("Couldn't start logcat: {e}")),
    }
}

fn stop_logcat() {
    if let Some((_, id)) = with(|s| s.logcat.take()) {
        host::cancel_job(id);
    }
}

/// Only the app's lines: its processes, from `adb shell pidof`.
fn package(name: String) {
    with(|s| s.package = name.trim().to_owned());
    let (package, serial) = with(|s| (s.package.clone(), s.logcat.as_ref().map(|l| l.0.clone())));
    if package.is_empty() {
        with(|s| s.filter.pids.clear());
        return;
    }
    let Some(serial) = serial else { return };
    if let Err(e) = start(&["-s", &serial, "shell", "pidof", &package], Then::Pids(package.clone())) {
        host::show_message(&e);
    }
}

fn line(job: JobLine) {
    let wanted = with(|s| s.logcat.as_ref().is_some_and(|l| l.1 == job.id) && !s.paused);
    if !wanted {
        return;
    }
    let Some(mut l) = adb::parse_line(&job.line) else { return };
    with(|s| {
        l.place = adb::parse_frame(&l.message).and_then(|(path, n)| adb::locate(&path, &s.files).map(|p| (p, n)));
        s.lines.push_back(l);
        if s.lines.len() > MAX_LINES {
            s.lines.pop_front();
        }
    });
    later();
}

/// Draws again once the editor has drawn a frame, at most every `DRAW_EVERY` ms.
fn later() {
    let ask = with(|s| !std::mem::replace(&mut s.waiting, true));
    if ask {
        host::continue_later("draw");
    }
}

fn resumed() {
    let now = host::now().unix_millis;
    let due = with(|s| now - s.drawn_at >= DRAW_EVERY);
    if due {
        with(|s| s.waiting = false);
        render();
    } else {
        host::continue_later("draw");
    }
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let ok = job.exit_code == Some(0);
    let why = || {
        let e = job.stderr.trim();
        if e.is_empty() { format!("adb stopped (exit {})", job.exit_code.unwrap_or(-1)) } else { e.to_owned() }
    };
    match then {
        Then::Devices => {
            let found = if ok { Ok(adb::parse_devices(&job.stdout)) } else { Err(why()) };
            with(|s| s.devices = Some(found));
        }
        Then::Logcat => with(|s| {
            if s.logcat.as_ref().is_some_and(|l| l.1 == job.id) {
                s.logcat = None;
                host::show_message("logcat stopped");
            }
        }),
        Then::Pids(package) => {
            let pids = adb::parse_pids(&job.stdout);
            with(|s| s.filter.pids = pids.clone());
            if pids.is_empty() {
                host::show_message(&format!("{package} isn't running"));
            }
        }
    }
    render();
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn bold(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: true, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: hint.into() }
}

fn level_color(level: char) -> Option<u32> {
    match level {
        'E' | 'F' | 'A' => Some(RED),
        'W' => Some(YELLOW),
        'I' => None,
        _ => Some(DIM),
    }
}

fn render() {
    let now = host::now().unix_millis;
    let blocks = with(|s| {
        s.drawn_at = now;
        let mut b = Vec::new();
        devices_ui(s, &mut b);
        if s.logcat.is_some() {
            b.push(Block::Separator);
            logcat_ui(s, &mut b);
        }
        b
    });
    host::set_panel("android", &blocks);
}

fn devices_ui(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Rows(vec![Row { cells: Vec::new(), buttons: vec![button("Refresh", "refresh", "")] }]));
    let watching = s.logcat.as_ref().map(|l| l.0.as_str());
    match &s.devices {
        None => b.push(Block::Spans(vec![span("Asking adb…", Some(DIM))])),
        Some(Err(e)) => b.push(Block::Spans(vec![span(e.clone(), Some(RED))])),
        Some(Ok(list)) if list.is_empty() => b.push(Block::Spans(vec![span(
            "No devices: plug one in with USB debugging on (Developer options), or start an emulator.",
            Some(DIM),
        )])),
        Some(Ok(list)) => {
            let rows = list
                .iter()
                .map(|d| {
                    let mut cells = vec![
                        bold(d.title(), Some(if d.ready() { GREEN } else { ORANGE })),
                        span(format!("{}  {}", d.serial, d.state), Some(DIM)),
                    ];
                    let buttons = match d.advice() {
                        Some(advice) => {
                            cells.push(span(advice, Some(ORANGE)));
                            Vec::new()
                        }
                        None => {
                            let logcat = if watching == Some(d.serial.as_str()) {
                                button("Stop logcat", "stop", "")
                            } else {
                                button("Logcat", "logcat", &d.serial)
                            };
                            vec![logcat, button("Shell", "shell", &d.serial)]
                        }
                    };
                    Row { cells, buttons }
                })
                .collect();
            b.push(Block::Rows(rows));
            // Installing: a built APK or a path, onto a ready device.
            let ready: Vec<String> = list.iter().filter(|d| d.ready()).map(|d| d.serial.clone()).collect();
            if let Some(first) = ready.first() {
                let mut fields = vec![field("device", "Install", FieldKind::Choice, first, ready.clone(), "")];
                if !s.apks.is_empty() {
                    fields.push(field("apk", "", FieldKind::Choice, &s.apks[0], s.apks.clone(), ""));
                }
                fields.push(field("path", "", FieldKind::Text, "", Vec::new(), "or an APK in this folder"));
                b.push(Block::Form(Form { id: "install".into(), fields, buttons: vec!["Install APK".into()] }));
            }
        }
    }
}

fn logcat_ui(s: &State, b: &mut Vec<Block>) {
    let level = adb::LEVELS.iter().find(|(c, _)| adb::severity(*c) == s.filter.min_level).map_or("Info", |l| l.1);
    let fields = vec![
        field(
            "level",
            "Logcat",
            FieldKind::Choice,
            level,
            adb::LEVELS.iter().map(|(_, n)| (*n).to_owned()).collect(),
            "",
        ),
        field("text", "", FieldKind::Text, &s.filter.text, Vec::new(), "Filter tag or text"),
        field("package", "", FieldKind::Text, &s.package, Vec::new(), "App (com.example.app)"),
    ];
    b.push(Block::Form(Form { id: "filter".into(), fields, buttons: vec!["Filter".into()] }));
    let pause = if s.paused { "Go on" } else { "Pause" };
    b.push(Block::Rows(vec![Row {
        cells: Vec::new(),
        buttons: vec![button(pause, "pause", ""), button("Clear", "clear", "")],
    }]));
    let shown: Vec<&LogLine> = s.lines.iter().filter(|l| s.filter.keeps(l)).collect();
    let mut note = format!("{} of {} lines", shown.len(), s.lines.len());
    if shown.len() > SHOWN {
        note.push_str(&format!(", the newest {SHOWN} shown"));
    }
    b.push(Block::Spans(vec![span(note, Some(DIM))]));
    let lines = shown[shown.len().saturating_sub(SHOWN)..]
        .iter()
        .map(|l| {
            let (path, line) = l.place.clone().unwrap_or_default();
            Line { text: l.shown(), color: level_color(l.level), path, line }
        })
        .collect();
    b.push(Block::Log(LogView { lines, follow: !s.paused }));
}

/// `adb` with `args` in a terminal, in the open folder.
fn terminal(title: &str, args: &[String]) {
    if let Err(e) = host::run_in_terminal(title, "adb", args) {
        host::show_message(&if e.contains("isn't on PATH") { missing() } else { e });
    }
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" | "refresh" => {
            refresh();
            return Ok(());
        }
        "logcat" => start_logcat(arg),
        "stop" => stop_logcat(),
        "shell" => terminal(&format!("adb shell ({arg})"), &["-s".into(), arg.into(), "shell".into()]),
        "pause" => with(|s| s.paused = !s.paused),
        "clear" => with(|s| s.lines.clear()),
        _ => {}
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    let value = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    match f.form.as_str() {
        "filter" => {
            let level = value("level");
            let min = adb::LEVELS.iter().find(|(_, n)| *n == level).map_or(2, |(c, _)| adb::severity(*c));
            with(|s| {
                s.filter.min_level = min;
                s.filter.text = value("text");
            });
            let name = value("package");
            if name != with(|s| s.package.clone()) || name.is_empty() {
                package(name);
            }
        }
        "install" => {
            let typed = value("path");
            let apk = if typed.is_empty() { value("apk") } else { typed };
            if apk.is_empty() {
                host::show_message("Build the app, or give the path of an APK in this folder");
                return;
            }
            // The terminal runs in the open folder, so the relative path does.
            let name = apk.rsplit('/').next().unwrap_or(&apk).to_owned();
            let device = value("device");
            terminal(&format!("install {name}"), &["-s".into(), device, "install".into(), "-r".into(), apk]);
        }
        _ => {}
    }
    render();
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
            Event::JobLine(job) => line(job),
            Event::JobFinished(job) => finished(job),
            Event::Resumed(token) if token == "draw" => resumed(),
            Event::FormSubmitted(f) => submitted(f),
            _ => {}
        }
    }
}

export!(Plugin);
