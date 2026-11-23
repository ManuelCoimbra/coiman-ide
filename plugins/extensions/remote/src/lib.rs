//! Remote: a folder on another machine, brought here by `ssh … tar` into a mirror in the
//! extension's own folder that opens as the project; each file saved, deleted or renamed in it
//! goes back with ssh. Also terminals there, a command run there, and a terminal on any host of
//! `~/.ssh/config`.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod remote;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobResult, Pick, PickChoice, PickItem, Row, Span,
};
use remote::RemoteFolder;

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;
const DIM: u32 = 0x8B949E;

/// What a running program is for.
enum Then {
    /// The folder's tar stream is in the archive: unpack it.
    Downloaded(RemoteFolder),
    /// Unpacked into the fresh folder: put it in place of the mirror.
    Unpacked(RemoteFolder),
    /// Something sent there: what to say when it worked.
    Sent(String),
}

#[derive(Default)]
struct State {
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    /// The form as last filled in.
    form: RemoteFolder,
    hosts: Vec<String>,
    /// What went there lately, newest first: (it worked, what).
    log: Vec<(bool, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// The remote folder the open folder is a mirror of, if it is one.
fn mirror_of_open() -> Option<RemoteFolder> {
    RemoteFolder::from_marker(&host::read_file(remote::MARKER).ok()?)
}

fn job(program: &str, args: Vec<String>, stdin: &str, then: Then) -> Result<(), String> {
    let id = host::start_job(program, &args, stdin)?;
    with(|s| {
        s.jobs.insert(id, then);
    });
    Ok(())
}

fn note(ok: bool, what: String) {
    with(|s| {
        s.log.insert(0, (ok, what));
        s.log.truncate(12);
    });
}

// ----- the mirror ---------------------------------------------------------------------------

/// Brings the folder into its mirror: into a fresh folder beside it first, so a failed download
/// leaves the last mirror as it was.
fn download(r: RemoteFolder) -> Result<(), String> {
    if let Some(p) = r.problem() {
        return Err(p.to_owned());
    }
    if with(|s| s.busy.is_some()) {
        return Err("A download is on its way already".into());
    }
    let mirror = r.mirror();
    host::data_remove(&format!("{mirror}.download"))?;
    let id = host::start_job_into("ssh", &r.download_args(), "", &format!("{mirror}.tar"))?;
    with(|s| {
        s.jobs.insert(id, Then::Downloaded(r.clone()));
        s.busy = Some(format!("Downloading {}:{}…", r.host, r.path));
        s.error = None;
    });
    Ok(())
}

fn downloaded(r: RemoteFolder, job: &JobResult) -> Result<(), String> {
    let mirror = r.mirror();
    if job.exit_code != Some(0) {
        let _ = host::data_remove(&format!("{mirror}.tar"));
        return Err(said(job, "ssh"));
    }
    // Made, then unpacked into, run in the extension's own folder with relative paths: GNU tar
    // reads `C:\…` as another machine's.
    host::data_folder(&format!("{mirror}.download"))?;
    let args = vec!["-xf".to_owned(), format!("{mirror}.tar"), "-C".to_owned(), format!("{mirror}.download")];
    let id = host::start_data_job("tar", &args, "")?;
    with(|s| {
        s.jobs.insert(id, Then::Unpacked(r));
    });
    Ok(())
}

fn unpacked(r: RemoteFolder, job: &JobResult) -> Result<(), String> {
    let mirror = r.mirror();
    let _ = host::data_remove(&format!("{mirror}.tar"));
    if job.exit_code != Some(0) {
        let _ = host::data_remove(&format!("{mirror}.download"));
        return Err(format!("tar: {}", said(job, "tar")));
    }
    host::data_write(&format!("{mirror}.download/{}", remote::MARKER), &r.marker())?;
    host::data_rename(&format!("{mirror}.download"), &mirror)?;
    let place = host::data_folder(&mirror)?;
    let open = host::workspace_path().ok().map(|p| p.replace('\\', "/").to_lowercase());
    if open.as_deref() != Some(place.replace('\\', "/").to_lowercase().as_str()) {
        host::open_folder(&place)?;
    }
    host::show_message(&format!("{}:{} is here; saves go back to it", r.host, r.path));
    Ok(())
}

/// What a finished program said on failing, in a line.
fn said(job: &JobResult, program: &str) -> String {
    let t = job.stderr.trim();
    if t.is_empty() { format!("{program} stopped") } else { t.lines().last().unwrap_or(t).to_owned() }
}

/// A change in the mirror, sent there.
fn send(args: Vec<String>, stdin: &str, what: String) {
    with(|s| s.busy = Some(format!("{what}…")));
    if let Err(e) = job("ssh", args, stdin, Then::Sent(what.clone())) {
        note(false, format!("{what}: {e}"));
        with(|s| s.busy = None);
    }
    render();
}

fn saved(rel: &str) {
    let Some(r) = mirror_of_open() else { return };
    if !RemoteFolder::goes_back(rel) {
        return;
    }
    let Ok(text) = host::read_file(rel) else { return };
    send(r.upload_args(rel), &text, format!("Sent {rel}"));
}

fn deleted(rels: Vec<String>) {
    let Some(r) = mirror_of_open() else { return };
    let rels: Vec<String> = rels.into_iter().filter(|p| RemoteFolder::goes_back(p)).collect();
    if !rels.is_empty() {
        send(r.delete_args(&rels), "", format!("Deleted {} there", rels.join(", ")));
    }
}

fn renamed(a: &str, b: &str) {
    let Some(r) = mirror_of_open() else { return };
    if RemoteFolder::goes_back(a) && RemoteFolder::goes_back(b) {
        send(r.rename_args(a, b), "", format!("Renamed {a} to {b} there"));
    }
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let result = match then {
        Then::Downloaded(r) => {
            downloaded(r.clone(), &job).map_err(|e| format!("Couldn't download {}:{}: {e}", r.host, r.path))
        }
        Then::Unpacked(r) => {
            with(|s| s.busy = None);
            unpacked(r.clone(), &job).map_err(|e| format!("Couldn't download {}:{}: {e}", r.host, r.path))
        }
        Then::Sent(what) => {
            with(|s| s.busy = None);
            let ok = job.exit_code == Some(0);
            if ok {
                host::show_message(&what);
                note(true, what);
            } else {
                note(false, format!("Not done there ({what}): {}", said(&job, "ssh")));
            }
            Ok(())
        }
    };
    if let Err(e) = result {
        with(|s| {
            s.busy = None;
            s.error = Some(e);
        });
    }
    render();
}

// ----- terminals ----------------------------------------------------------------------------

fn terminal_there(command: Option<&str>) -> Result<(), String> {
    let r = mirror_of_open().ok_or("Open a remote folder first (Remote: Open SSH Folder...)")?;
    let title = match command {
        Some(c) => format!("{}: {c}", r.host),
        None => format!("ssh {}", r.host),
    };
    host::run_in_terminal(&title, "ssh", &r.terminal_args(command))
}

fn hosts() -> Vec<String> {
    remote::ssh_hosts(&host::ssh_config().unwrap_or_default())
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn text(id: &str, label: &str, value: &str, hint: &str) -> FormField {
    FormField {
        id: id.into(),
        label: label.into(),
        kind: FieldKind::Text,
        value: value.into(),
        options: vec![],
        hint: hint.into(),
    }
}

fn render() {
    let here = mirror_of_open();
    let blocks = with(|s| {
        let mut b = Vec::new();
        if let Some(busy) = &s.busy {
            b.push(Block::Spans(vec![span(busy, Some(DIM))]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        if let Some(r) = &here {
            b.push(Block::Spans(vec![
                span("This folder is a copy of ", Some(DIM)),
                Span { text: format!("{}:{}", r.host, r.path), color: None, bold: true, monospace: true },
                span(". Saves, deletes and renames go there.", Some(DIM)),
            ]));
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![button("Download again", "download", ""), button("Terminal there", "terminal", "")],
            }]));
            b.push(Block::Form(Form {
                id: "run".into(),
                fields: vec![text("command", "Run there", "", "cargo build, make, pytest")],
                buttons: vec!["Run".into()],
            }));
            b.push(Block::Separator);
        }
        b.push(Block::Heading("Open a folder over SSH".into()));
        let f = &s.form;
        b.push(Block::Form(Form {
            id: "open".into(),
            fields: vec![
                text("host", "Host", &f.host, "user@host, or a name in ~/.ssh/config"),
                text("path", "Folder", &f.path, "/home/me/project"),
                text("options", "ssh options", &f.ssh_options, "-i ~/.ssh/key -o Port=2222"),
            ],
            buttons: vec!["Open".into()],
        }));
        if !s.hosts.is_empty() {
            let mut buttons: Vec<Button> = s.hosts.iter().map(|h| button(h, "use-host", h)).collect();
            buttons.insert(0, button("New SSH terminal…", "ssh-terminal", ""));
            b.push(Block::Spans(vec![span("Hosts in ~/.ssh/config:", Some(DIM))]));
            b.push(Block::Rows(vec![Row { cells: vec![], buttons }]));
        }
        b.push(Block::Spans(vec![span(
            "The folder is copied here with ssh and tar (build folders like target and node_modules left out) and \
             opens as the project. Each file you save, delete or rename there goes back with ssh. ssh logs in with \
             a key: no password prompt.",
            Some(DIM),
        )]));
        if !s.log.is_empty() {
            b.push(Block::Heading("Lately".into()));
            for (ok, what) in &s.log {
                b.push(Block::Spans(vec![span(what, Some(if *ok { GREEN } else { RED }))]));
            }
        }
        b
    });
    host::set_panel("remote", &blocks);
}

// ----- commands -----------------------------------------------------------------------------

fn command(id: &str, arg: &str) -> Result<(), String> {
    let result = match id {
        "open" => {
            let found = hosts();
            let last = mirror_of_open();
            with(|s| {
                s.hosts = found;
                if let Some(r) = last {
                    s.form = r;
                }
            });
            Ok(())
        }
        "download" => match mirror_of_open() {
            Some(r) => download(r),
            None => Err("The open folder isn't a remote folder".into()),
        },
        "terminal" => terminal_there(None),
        "run" => {
            with(|s| s.hosts = hosts());
            Ok(())
        }
        "use-host" => {
            with(|s| s.form.host = arg.to_owned());
            Ok(())
        }
        "ssh-terminal" => {
            let list = hosts();
            if list.is_empty() {
                Err("No hosts in ~/.ssh/config: add one (Host name, HostName, User) to pick it here".into())
            } else {
                let items = list
                    .into_iter()
                    .map(|h| PickItem { text: h, kind: "ssh".into(), detail: String::new(), can_forget: false })
                    .collect();
                host::show_pick(&Pick {
                    id: "hosts".into(),
                    title: "Open a terminal on which host?".into(),
                    query: String::new(),
                    items,
                    at_terminal_cursor: false,
                    run_first: true,
                    filter: true,
                });
                Ok(())
            }
        }
        _ => return Err(format!("unknown command {id}")),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    } else if id != "use-host" {
        with(|s| s.error = None);
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    let result = match f.form.as_str() {
        "open" => {
            let r = RemoteFolder { host: get("host"), path: get("path"), ssh_options: get("options") };
            with(|s| s.form = r.clone());
            download(r)
        }
        "run" => {
            let command = get("command");
            if command.is_empty() { Ok(()) } else { terminal_there(Some(&command)) }
        }
        _ => Ok(()),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
    }
    render();
}

fn picked(c: PickChoice) {
    if c.pick == "hosts" {
        let title = format!("ssh {}", c.text);
        if let Err(e) = host::run_in_terminal(&title, "ssh", std::slice::from_ref(&c.text)) {
            with(|s| s.error = Some(e));
            render();
        }
    }
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
            Event::Picked(c) => picked(c),
            Event::Saved(rel) => saved(&rel),
            Event::Deleted(rels) => deleted(rels),
            Event::Renamed((a, b)) => renamed(&a, &b),
            _ => {}
        }
    }
}

export!(Plugin);
