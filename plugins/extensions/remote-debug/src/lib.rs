//! Remote Debugging: a program on another machine debugged here with gdb. Over ssh, the
//! extension copies the program there if asked (scp), starts gdbserver (its port carried here by
//! `ssh -L`) and, once gdbserver listens, has the editor attach gdb with the local program's
//! symbols; or gdb connects to a gdbserver already running. The target is kept per project.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod target;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, FieldKind, Form, FormField, FormValues, JobLine, JobResult, RemoteAttach, Span};
use target::RemoteTarget;

const RED: u32 = 0xE5484D;
const DIM: u32 = 0x8B949E;

const OVER_SSH: &str = "Start gdbserver over ssh";
const CONNECT: &str = "Connect to a gdbserver running";

#[derive(Default)]
struct State {
    /// The target as last filled in (read from the project at first).
    target: Option<RemoteTarget>,
    /// The copy with scp, while it runs.
    copying: Option<u64>,
    /// ssh running gdbserver there.
    ssh: Option<u64>,
    /// gdbserver isn't listening yet.
    waiting: bool,
    note: Option<String>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn current() -> RemoteTarget {
    if let Some(t) = with(|s| s.target.clone()) {
        return t;
    }
    let t = RemoteTarget::read(&host::read_file(target::FILE).unwrap_or_default());
    with(|s| s.target = Some(t.clone()));
    t
}

fn say(text: &str) {
    with(|s| s.note = Some(text.to_owned()));
    host::show_message(text);
}

/// Ends the ssh run, if there is one.
fn stop() {
    let ssh = with(|s| {
        s.waiting = false;
        s.ssh.take()
    });
    if let Some(id) = ssh {
        host::cancel_job(id);
    }
}

/// Keeps the target for the project and starts: copy, ssh and gdbserver, then gdb.
fn start(t: RemoteTarget) -> Result<(), String> {
    if let Some(p) = t.problem() {
        return Err(p.to_owned());
    }
    stop();
    with(|s| s.target = Some(t.clone()));
    if let Err(e) = host::write_file(target::FILE, &t.text()) {
        host::debug_console(&format!("Couldn't keep the remote target: {e}"));
    }
    if !t.over_ssh {
        return attach(&t);
    }
    if t.copy_first {
        host::debug_console(&format!("scp {}", t.scp_args().join(" ")));
        let id = host::start_job("scp", &t.scp_args(), "")?;
        with(|s| s.copying = Some(id));
        say("Copying the program there…");
        return Ok(());
    }
    gdbserver(&t)
}

/// ssh runs gdbserver there; its lines come back one by one.
fn gdbserver(t: &RemoteTarget) -> Result<(), String> {
    let args = t.ssh_args();
    host::debug_console(&format!("ssh {}", args.join(" ")));
    let id = host::start_job_lines("ssh", &args, "")?;
    with(|s| {
        s.ssh = Some(id);
        s.waiting = true;
    });
    say("Starting gdbserver over ssh…");
    Ok(())
}

/// gdb attaches to the target, with the local program's symbols.
fn attach(t: &RemoteTarget) -> Result<(), String> {
    let program = t.local_program.trim();
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    host::debug_attach(&RemoteAttach {
        name: format!("{file} on {}", t.target()),
        program: program.to_owned(),
        target: t.target(),
    })
}

fn line(l: JobLine) {
    if with(|s| s.ssh) != Some(l.id) {
        return;
    }
    host::debug_console(&l.line);
    if target::listening(&l.line) && with(|s| std::mem::take(&mut s.waiting)) {
        if let Err(e) = attach(&current()) {
            say(&e);
        }
    } else if target::failed(&l.line) && with(|s| s.waiting) {
        say("Remote debugging couldn't start (see the debug console)");
    }
    render();
}

fn finished(job: JobResult) {
    if with(|s| s.copying) == Some(job.id) {
        with(|s| s.copying = None);
        if job.exit_code == Some(0) {
            if let Err(e) = gdbserver(&current()) {
                say(&e);
            }
        } else {
            host::debug_console(job.stderr.trim());
            say("Couldn't copy the program there (see the debug console)");
        }
    } else if with(|s| s.ssh) == Some(job.id) {
        let was_waiting = with(|s| {
            s.ssh = None;
            std::mem::take(&mut s.waiting)
        });
        if was_waiting {
            let code = job.exit_code.map_or(String::new(), |c| format!(" (exit {c})"));
            say(&format!("ssh ended before gdbserver was ready{code}: see the debug console"));
        }
    }
    render();
}

// ----- the panel ----------------------------------------------------------------------------

fn field(id: &str, label: &str, kind: FieldKind, value: &str, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options: vec![], hint: hint.into() }
}

fn line_break() -> FormField {
    field("", "", FieldKind::LineBreak, "", "")
}

fn render() {
    let t = current();
    let (note, waiting) = with(|s| (s.note.clone(), s.waiting || s.copying.is_some()));
    let mut b = Vec::new();
    b.push(Block::Spans(vec![Span {
        text: "A program on another machine, run under gdbserver, debugged here with gdb.".into(),
        color: Some(DIM),
        bold: false,
        monospace: false,
    }]));
    let mut mode = field("mode", "", FieldKind::Choice, if t.over_ssh { OVER_SSH } else { CONNECT }, "");
    mode.options = vec![OVER_SSH.into(), CONNECT.into()];
    let mut fields = vec![
        mode,
        line_break(),
        field("host", "Host", FieldKind::Text, &t.host, "user@host, or a name in ~/.ssh/config"),
        field("port", "gdbserver's port", FieldKind::Text, &t.port.to_string(), "2345"),
        line_break(),
        field("local", "Program here (its symbols)", FieldKind::Text, &t.local_program, "target/debug/app"),
        line_break(),
    ];
    // Over ssh only: said in their labels.
    fields.extend([
        field("remote", "Over ssh: program there", FieldKind::Text, &t.remote_program, "/home/pi/app"),
        field("args", "its arguments", FieldKind::Text, &t.args, ""),
        line_break(),
        field("copy", "Copy the program there first (scp)", FieldKind::Check, &t.copy_first.to_string(), ""),
        field("gdbserver", "gdbserver there", FieldKind::Text, &t.gdbserver, "gdbserver"),
        field("local_port", "port here", FieldKind::Text, &t.local_port.to_string(), "0: the same"),
        line_break(),
        field("ssh", "ssh options", FieldKind::Text, &t.ssh_options, "-i ~/.ssh/key -o Port=2222"),
        line_break(),
    ]);
    let buttons = if waiting { vec!["Stop".into()] } else { vec!["Start".into()] };
    b.push(Block::Form(Form { id: "target".into(), fields, buttons }));
    b.push(Block::Spans(vec![Span {
        text: if t.over_ssh {
            "ssh logs in with a key (no password prompt). Kept in .coiman/remote_debug.toml.".into()
        } else {
            "gdb connects to host:port. Kept in .coiman/remote_debug.toml.".into()
        },
        color: Some(DIM),
        bold: false,
        monospace: false,
    }]));
    if let Some(n) = note {
        b.push(Block::Spans(vec![Span { text: n, color: Some(RED), bold: false, monospace: false }]));
    }
    host::set_panel("remote-debug", &b);
}

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned());
    let mut t = current();
    if let Some(m) = get("mode") {
        t.over_ssh = m != CONNECT;
    }
    if let Some(v) = get("host") {
        t.host = v;
    }
    if let Some(v) = get("port") {
        t.port = v.parse().unwrap_or(0);
    }
    if let Some(v) = get("local") {
        t.local_program = v;
    }
    if let Some(v) = get("remote") {
        t.remote_program = v;
    }
    if let Some(v) = get("args") {
        t.args = v;
    }
    if let Some(v) = get("copy") {
        t.copy_first = v == "true";
    }
    if let Some(v) = get("gdbserver") {
        t.gdbserver = v;
    }
    if let Some(v) = get("local_port") {
        t.local_port = v.parse().unwrap_or(0);
    }
    if let Some(v) = get("ssh") {
        t.ssh_options = v;
    }
    with(|s| {
        s.target = Some(t.clone());
        s.note = None;
    });
    if f.button == "Stop" {
        stop();
        with(|s| s.copying = None);
    } else if let Err(e) = start(t) {
        with(|s| s.note = Some(e));
    }
    render();
}

fn command(id: &str) -> Result<(), String> {
    match id {
        "show" => {}
        // The editor's Restart: the way it started, again.
        "restart" => {
            if let Err(e) = start(current()) {
                say(&e);
            }
        }
        // The session ended: gdbserver's ssh run goes too.
        "debug-ended" => stop(),
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

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        command(&id)
    }

    fn on_event(event: Event) {
        match event {
            Event::JobLine(l) => line(l),
            Event::JobFinished(job) => finished(job),
            Event::FormSubmitted(f) => submitted(f),
            _ => {}
        }
    }
}

export!(Plugin);
