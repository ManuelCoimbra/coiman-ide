//! Agents: opencode, the program installed on this computer, in the editor's chat view (the
//! Agents panel). Coiman has no AI of its own: this extension runs `opencode` as an approved
//! program in the open folder, asks it for its models and past sessions, and fills the chat
//! view with its replies, thinking and tool calls as they come. The editor keeps the box you
//! type in and draws the conversation.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod opencode;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    ChatAction, ChatMessage, ChatPart, ChatSession, ChatTool, ChatView, JobLine, JobResult, JobSpec,
};
use opencode::{Event as Line, Message, NEW_SESSION, Part, Prefs, Session};

const PROGRAM: &str = "opencode";
const INSTALL: &str = "npm install -g opencode-ai";

/// What a job is for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Job {
    /// `opencode run --help`: whether it has `--standalone`.
    Help,
    Run,
    Models,
    Sessions,
    Export,
    Delete(String),
}

#[derive(Default)]
struct State {
    prefs: Prefs,
    /// None until `opencode run --help` has said.
    standalone: Option<bool>,
    /// Set when opencode can't be started: how to install it.
    missing: Option<String>,
    /// Actions waiting for the help check.
    waiting: Vec<ChatAction>,
    jobs: Vec<(u64, Job)>,
    messages: Vec<Message>,
    /// The first message the chat view hasn't been sent since it changed.
    changed_from: Option<usize>,
    session: String,
    title: String,
    status: String,
    sessions: Vec<Session>,
    sessions_loaded: bool,
    sessions_changed: bool,
    models: Vec<String>,
    models_loaded: bool,
    models_changed: bool,
    /// Open the newest session once the list is in.
    continue_asked: bool,
    turn_started: Option<i64>,
    turn_seconds: Option<u64>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn now_millis() -> i64 {
    host::now().unix_millis
}

impl State {
    fn job(&self, kind: &Job) -> Option<u64> {
        self.jobs.iter().find(|(_, j)| j == kind).map(|(id, _)| *id)
    }

    fn running(&self) -> bool {
        self.job(&Job::Run).is_some()
    }

    fn loading(&self) -> bool {
        self.job(&Job::Export).is_some()
    }

    fn changed(&mut self, from: usize) {
        self.changed_from = Some(self.changed_from.map_or(from, |f| f.min(from)));
    }

    fn clear(&mut self) {
        self.messages.clear();
        self.changed(0);
    }

    /// Starts opencode; when it can't be started, says why (and how to install it, if it isn't
    /// there).
    fn start(&mut self, kind: Job, args: Vec<String>, stdin: String, env: Vec<(String, String)>) -> bool {
        let lines = kind == Job::Run;
        let spec = JobSpec { program: PROGRAM.to_owned(), args, stdin, env, in_own_folder: false };
        let started = if lines { host::start_job_lines_with(&spec) } else { host::start_job_with(&spec) };
        match started {
            Ok(id) => {
                self.jobs.push((id, kind));
                true
            }
            Err(e) => {
                if e.contains("isn't on PATH") {
                    self.missing = Some(INSTALL.to_owned());
                } else if e.contains("isn't trusted") {
                    self.status = "The working folder is restricted. Trust it before starting an agent.".to_owned();
                } else {
                    self.status = format!("Couldn't start opencode: {e}");
                }
                false
            }
        }
    }

    fn standalone_args(&self, args: &[&str]) -> Vec<String> {
        opencode::with_standalone(self.standalone.unwrap_or(false), args)
    }

    fn act(&mut self, action: ChatAction) {
        let needs_opencode = !matches!(action.action.as_str(), "new" | "model" | "mode" | "permissions" | "stop");
        if needs_opencode && self.standalone.is_none() {
            self.waiting.push(action);
            if self.job(&Job::Help).is_none() && self.missing.is_none() {
                self.start(Job::Help, vec!["run".into(), "--help".into()], String::new(), Vec::new());
            }
            return;
        }
        let arg = action.argument.clone();
        match action.action.as_str() {
            "send" => self.send(&arg, &action.attachments),
            "stop" => {
                if let Some(id) = self.job(&Job::Run) {
                    host::cancel_job(id);
                    self.jobs.retain(|(j, _)| *j != id);
                    self.end_turn();
                    self.status = "Stopped".to_owned();
                }
            }
            "new" => {
                if self.running() || self.loading() {
                    self.status = "Stop the active task before starting a new session".to_owned();
                } else {
                    self.clear();
                    self.session.clear();
                    self.title.clear();
                    self.turn_seconds = None;
                    self.status = "New session".to_owned();
                }
            }
            "open" => {
                let title = self.sessions.iter().find(|s| s.id == arg).map(|s| s.title.clone());
                self.open(&arg, title.unwrap_or_else(|| arg.clone()));
            }
            "delete" => {
                if self.running() || self.loading() {
                    self.status = "Stop the active task before deleting a session".to_owned();
                } else {
                    let args = self.standalone_args(&["session", "delete", &arg]);
                    self.start(Job::Delete(arg), args, String::new(), Vec::new());
                }
            }
            "continue" => match self.sessions.first().cloned() {
                Some(latest) if self.sessions_loaded => self.open(&latest.id, latest.title),
                _ => {
                    self.continue_asked = true;
                    self.load_sessions();
                }
            },
            "sessions" => self.load_sessions(),
            "models" => {
                if !self.models_loaded && self.job(&Job::Models).is_none() {
                    self.start(Job::Models, vec!["models".into()], String::new(), Vec::new());
                }
            }
            "model" => {
                self.prefs.model = Some(arg).filter(|m| !m.is_empty());
                self.save();
            }
            "mode" if arg == "build" || arg == "plan" => {
                self.prefs.mode = arg;
                self.save();
            }
            "permissions" if (arg == "auto" || arg == "manual") && arg != self.prefs.permissions => {
                self.status = if arg == "manual" {
                    "Manual: tool calls needing approval are turned down by opencode run, and it says so".to_owned()
                } else {
                    "Auto: the agent's tool calls are approved".to_owned()
                };
                self.prefs.permissions = arg;
                self.save();
            }
            _ => {}
        }
    }

    fn save(&self) {
        let _ = host::storage_write(&self.prefs.to_json());
    }

    fn load_sessions(&mut self) {
        if self.job(&Job::Sessions).is_none() {
            let args = self.standalone_args(&["session", "list", "--format", "json", "-n", "50"]);
            self.start(Job::Sessions, args, String::new(), Vec::new());
        }
    }

    fn open(&mut self, id: &str, title: String) {
        if self.running() || self.loading() {
            self.status = "Stop the active task before switching sessions".to_owned();
            return;
        }
        self.session = id.to_owned();
        self.status = format!("Opening \"{title}\"…");
        self.title = title;
        self.turn_seconds = None;
        self.clear();
        let args = if self.standalone == Some(true) {
            vec!["--standalone".into(), "session".into(), "export".into(), id.to_owned()]
        } else {
            vec!["export".into(), id.to_owned()]
        };
        if !self.start(Job::Export, args, String::new(), Vec::new()) && self.status.starts_with("Opening") {
            self.status = "Couldn't load the session".to_owned();
        }
    }

    fn send(&mut self, text: &str, attachments: &[String]) {
        let text = text.trim();
        if text.is_empty() || self.running() || self.loading() {
            return;
        }
        let standalone = self.standalone.unwrap_or(false);
        if self.session.is_empty() {
            self.session = format!("{NEW_SESSION}{}_{}", now_millis(), host::random() % 1_000_000);
            // opencode names the session; the title comes with the list after the run.
            self.title.clear();
        }
        let run = opencode::run(standalone, &self.session, text, attachments, &self.prefs);
        self.messages.push(Message::user(&opencode::with_attachments(text, attachments)));
        self.changed(self.messages.len() - 1);
        if self.start(Job::Run, run.args, run.stdin, run.env) {
            self.status = "Working…".to_owned();
            self.turn_started = Some(now_millis());
            self.turn_seconds = None;
        } else {
            self.messages.pop();
        }
    }

    fn end_turn(&mut self) {
        self.turn_seconds = self.turn_started.take().map(|t| ((now_millis() - t).max(0) / 1000) as u64);
    }

    fn line(&mut self, id: u64, line: &str) {
        if self.job(&Job::Run) != Some(id) {
            return;
        }
        match opencode::event(line) {
            Some(Line::SessionId(sid)) => {
                if self.session.starts_with(NEW_SESSION) && !sid.is_empty() {
                    self.session = sid;
                }
            }
            Some(event) => {
                if let Some(from) = opencode::apply(&mut self.messages, event) {
                    self.changed(from);
                }
            }
            None => {}
        }
    }

    fn finished(&mut self, r: JobResult) {
        let Some(at) = self.jobs.iter().position(|(id, _)| *id == r.id) else { return };
        let (_, kind) = self.jobs.remove(at);
        let ok = r.exit_code == Some(0);
        match kind {
            Job::Help => {
                // An opencode that can't say is taken as a newer one.
                self.standalone =
                    Some(opencode::has_standalone(&r.stdout) || opencode::has_standalone(&r.stderr) || !ok);
                for action in std::mem::take(&mut self.waiting) {
                    self.act(action);
                }
            }
            Job::Run => {
                self.end_turn();
                self.status = match r.exit_code {
                    Some(0) | None => "Ready".to_owned(),
                    Some(code) => format!("opencode ended with exit code {code}"),
                };
                // This run may have made a session: the list says its title.
                self.load_sessions();
            }
            Job::Models => {
                self.models = if ok { opencode::models(&r.stdout) } else { Vec::new() };
                self.models_loaded = true;
                self.models_changed = true;
            }
            Job::Sessions => {
                self.sessions = if ok { opencode::sessions(&r.stdout) } else { Vec::new() };
                self.sessions_loaded = true;
                self.sessions_changed = true;
                if let Some(current) = self.sessions.iter().find(|s| s.id == self.session) {
                    self.title = current.title.clone();
                }
                if std::mem::take(&mut self.continue_asked) {
                    match self.sessions.first().cloned() {
                        Some(latest) => self.open(&latest.id, latest.title),
                        None => self.status = "No sessions yet".to_owned(),
                    }
                }
            }
            Job::Export => {
                self.messages = if ok { opencode::export(&r.stdout) } else { Vec::new() };
                self.changed(0);
                self.status = if !ok {
                    "Couldn't load the session".to_owned()
                } else if self.messages.is_empty() {
                    "This session has no messages".to_owned()
                } else {
                    "Resumed session".to_owned()
                };
            }
            Job::Delete(id) => {
                if id == self.session {
                    // The open session is gone: start afresh.
                    self.clear();
                    self.session.clear();
                    self.title.clear();
                }
                self.status = if ok { "Session deleted".to_owned() } else { "Couldn't delete the session".to_owned() };
                self.load_sessions();
            }
        }
    }

    /// What changed since the last time, for the chat view.
    fn view(&mut self) -> ChatView {
        let from = self.changed_from.take().unwrap_or(self.messages.len()).min(self.messages.len());
        ChatView {
            name: "OpenCode".to_owned(),
            missing: self.missing.clone(),
            session: if self.session.starts_with(NEW_SESSION) { String::new() } else { self.session.clone() },
            title: self.title.clone(),
            messages_from: from as u32,
            messages: self.messages[from..].iter().map(to_chat).collect(),
            running: self.running(),
            loading: self.loading(),
            status: self.status.clone(),
            sessions: std::mem::take(&mut self.sessions_changed).then(|| {
                self.sessions
                    .iter()
                    .map(|s| ChatSession { id: s.id.clone(), title: s.title.clone(), updated: s.updated })
                    .collect()
            }),
            sessions_loading: self.job(&Job::Sessions).is_some()
                || (self.continue_asked && self.job(&Job::Help).is_some()),
            models: std::mem::take(&mut self.models_changed).then(|| self.models.clone()),
            models_loading: self.job(&Job::Models).is_some(),
            model: self.prefs.model.clone().unwrap_or_default(),
            mode: self.prefs.mode.clone(),
            permissions: self.prefs.permissions.clone(),
            turn_seconds: self.turn_seconds,
        }
    }
}

fn to_chat(m: &Message) -> ChatMessage {
    ChatMessage {
        from_user: m.user,
        parts: m
            .parts
            .iter()
            .map(|p| match p {
                Part::Text(t) => ChatPart::Text(t.clone()),
                Part::Thinking(t) => ChatPart::Thinking(t.clone()),
                Part::Tool(t) => ChatPart::Tool(ChatTool {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    title: t.title.clone(),
                    input: t.input.clone(),
                    output: t.output.clone(),
                    status: t.status.clone(),
                }),
            })
            .collect(),
    }
}

fn show() {
    let view = with(State::view);
    host::set_chat(&view);
}

struct Agents;

impl Guest for Agents {
    fn activate() -> Result<(), String> {
        // Nothing runs until the panel asks: the saved choices only.
        let prefs = Prefs::from_json(&host::storage_read());
        with(|s| s.prefs = prefs);
        show();
        Ok(())
    }

    fn run_command(_id: String, _argument: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(event: Event) {
        let shown = match event {
            Event::ChatAction(action) => {
                with(|s| s.act(action));
                true
            }
            Event::JobLine(JobLine { id, line }) => {
                with(|s| s.line(id, &line));
                true
            }
            Event::JobFinished(r) => {
                with(|s| s.finished(r));
                true
            }
            _ => false,
        };
        if shown {
            show();
        }
    }
}

export!(Agents);
