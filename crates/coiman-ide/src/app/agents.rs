//! The Agents window: OpenCode installed on this computer, in a chat-like view.
//!
//! Coiman doesn't implement an AI model of its own. This view runs the `opencode`
//! command line in the open folder, asks it for its models, and shows its replies,
//! thinking and tool calls in a transcript. The window opens on the left and can be
//! collapsed like the Explorer.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

use coiman_core::language::highlight;
use coiman_core::language::markdown::Inline;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, FontId, Rect, RichText, ScrollArea, Sense, TextEdit};
use serde_json::Value;

use crate::app::CodeApp;
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

#[derive(Clone)]
struct ToolPart {
    id: String,
    name: String,
    title: String,
    input: String,
    output: String,
    status: String,
}

#[derive(Clone)]
enum Part {
    Text(String),
    Thinking(String),
    Tool(ToolPart),
}

#[derive(Clone)]
struct Message {
    role: Role,
    parts: Vec<Part>,
}

#[derive(Clone)]
struct SessionInfo {
    id: String,
    title: String,
    updated: u64,
    directory: Option<PathBuf>,
}

enum AgentMsg {
    Text(String),
    Thinking(String),
    Tool(ToolPart),
    Models(Vec<String>),
    Session(Vec<Message>),
    End(Option<i32>),
}

#[derive(Default)]
pub(crate) struct AgentsUi {
    open: bool,
    messages: Vec<Message>,
    input: String,
    session: String,
    /// The title of the current session, shown in the conversation bar.
    session_title: Option<String>,
    running: Option<Receiver<AgentMsg>>,
    child_pid: Option<u32>,
    running_models: Option<Receiver<AgentMsg>>,
    running_history: Option<Receiver<Vec<SessionInfo>>>,
    running_session: Option<Receiver<AgentMsg>>,
    running_delete: Option<Receiver<()>>,
    deleting: bool,
    /// The session waiting for a delete confirmation.
    delete_pending: Option<SessionInfo>,
    /// The session being deleted, remembered so the transcript can be cleared.
    deleting_id: Option<String>,
    session_loading: bool,
    /// Files attached as context; sent as `@path` references before the message.
    attached: Vec<String>,
    /// The file open in the editor, offered as context by "Add Context…".
    active_file: Option<PathBuf>,
    root: Option<PathBuf>,
    session_root: Option<PathBuf>,
    status: Option<String>,
    checked_opencode: bool,
    opencode: Option<PathBuf>,
    models: Vec<String>,
    models_loading: bool,
    models_loaded: bool,
    history: Vec<SessionInfo>,
    history_open: bool,
    history_loading: bool,
    session_search_open: bool,
    session_search: String,
    model: Option<String>,
    /// The agent mode: "build" edits files, "plan" only discusses.
    mode: String,
    /// "auto" approves tool use; "manual" doesn't pass --auto to OpenCode.
    permissions: String,
    prefs_loaded: bool,
    model_search: String,
    focus_input: bool,
    /// Messages typed while the agent runs, sent when it finishes.
    queue: Vec<String>,
    /// Sent prompts, cycled back with the up arrow in an empty ask box.
    prompt_history: Vec<String>,
    history_index: Option<usize>,
    /// Whether the ask box had keyboard focus last frame, for the composer border.
    input_focused: bool,
    /// Files edited in the last turn, for the working set.
    last_changed_files: Vec<String>,
    /// Tool calls in the last turn, for the "ran" status line.
    last_turn_tools: usize,
    working_set_open: bool,
    /// Ask the app to open this file in the editor.
    pub(crate) open_request: Option<PathBuf>,
    /// Ask the app to show this file's diff.
    pub(crate) diff_request: Option<PathBuf>,
    /// Ask the app to open Settings.
    pub(crate) requests_settings: bool,
    /// Set when Continue last session was chosen before the list was loaded.
    continue_requested: bool,
    /// True while the transcript is scrolled to the bottom, for the jump pill.
    at_bottom: bool,
    /// Set to scroll the transcript back to the bottom.
    jump_to_bottom: bool,
    /// Where the composer box sits on screen, for the floating jump pill.
    composer_rect: Option<egui::Rect>,
    /// The composer's measured height, so the transcript can take the rest.
    composer_height: f32,
    /// Focus the model search box on the frame its popup opens.
    focus_model_search: bool,
    /// Focus the sessions search box on the frame it opens.
    focus_session_search: bool,
    /// The panel fills the window, like a maximized terminal.
    pub(crate) maximized: bool,
}

impl AgentsUi {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the side-by-side sessions column is shown.
    /// Opens the panel and focuses the ask box.
    pub(crate) fn focus_chat(&mut self) {
        self.open = true;
        self.focus_input = true;
    }

    pub(crate) fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.focus_input = true;
        }
    }

    fn new_session() -> String {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        // OpenCode requires session ids to start with "ses".
        format!("ses_coiman_{}_{}", std::process::id(), nanos)
    }

    fn opencode_command(program: &Path) -> Command {
        let mut command = Command::new(program);
        // Coiman may itself run inside an OpenCode session. Those variables would make the
        // child join that session and its folder instead of the one open here.
        for key in [
            "OPENCODE",
            "OPENCODE_SESSION_ID",
            "OPENCODE_TERMINAL",
            "OPENCODE_SERVER",
            "OPENCODE_PORT",
            "OPENCODE_HOST",
        ] {
            command.env_remove(key);
        }
        command
    }

    fn opencode(&mut self) -> Option<PathBuf> {
        if !self.checked_opencode {
            self.checked_opencode = true;
            self.opencode = coiman_core::language::lsp::servers::which("opencode");
        }
        self.opencode.clone()
    }

    fn config_file() -> Option<PathBuf> {
        coiman_core::settings::config_dir().map(|d| d.join("agents.json"))
    }

    fn load_model(&mut self) {
        if self.prefs_loaded {
            return;
        }
        self.prefs_loaded = true;
        self.mode = "build".to_owned();
        self.permissions = "manual".to_owned();
        let Some(path) = Self::config_file() else { return };
        let Ok(text) = std::fs::read_to_string(path) else { return };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { return };
        if let Some(model) = value.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
            self.model = Some(model.to_owned());
        }
        if let Some(mode) = value.get("mode").and_then(Value::as_str).filter(|mode| *mode == "plan" || *mode == "build")
        {
            self.mode = mode.to_owned();
        }
        if let Some(permissions) = value
            .get("permissions")
            .and_then(Value::as_str)
            .filter(|permissions| *permissions == "auto" || *permissions == "manual")
        {
            self.permissions = permissions.to_owned();
        }
    }

    fn save_prefs(&self) {
        let Some(path) = Self::config_file() else { return };
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
        let value = serde_json::json!({ "model": self.model, "mode": self.mode, "permissions": self.permissions });
        let _ = std::fs::write(path, serde_json::to_string_pretty(&value).unwrap_or_default());
    }

    fn load_models(&mut self) {
        if self.models_loading || self.models_loaded {
            return;
        }
        let Some(program) = self.opencode() else { return };
        self.models_loading = true;
        let (tx, rx) = mpsc::channel();
        self.running_models = Some(rx);
        std::thread::spawn(move || {
            let output = Self::opencode_command(&program).arg("models").output();
            let models = match output {
                Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with("Error"))
                    .map(str::to_owned)
                    .collect(),
                _ => Vec::new(),
            };
            let _ = tx.send(AgentMsg::Models(models));
        });
    }

    fn load_history(&mut self) {
        if self.history_loading {
            return;
        }
        let Some(program) = self.opencode() else { return };
        self.history_loading = true;
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.running_history = Some(rx);
        std::thread::spawn(move || {
            let mut command = Self::opencode_command(&program);
            command.args(["--standalone", "session", "list", "--format", "json", "-n", "50"]).stdin(Stdio::null());
            if let Some(root) = &root {
                command.current_dir(root);
                command.env("PWD", root);
            }
            let sessions = command
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| serde_json::from_str::<Value>(&String::from_utf8_lossy(&o.stdout)).ok())
                .and_then(|v| v.as_array().cloned())
                .map(|items| {
                    let mut sessions: Vec<SessionInfo> = items
                        .iter()
                        .filter_map(|item| {
                            Some(SessionInfo {
                                id: item.get("id")?.as_str()?.to_owned(),
                                title: item.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_owned(),
                                updated: item.get("updated").and_then(Value::as_u64).unwrap_or(0),
                                directory: item.get("directory").and_then(Value::as_str).map(PathBuf::from),
                            })
                        })
                        .collect();
                    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
                    sessions
                })
                .unwrap_or_default();
            let _ = tx.send(sessions);
        });
    }

    fn poll_history(&mut self) {
        let Some(rx) = self.running_history.take() else { return };
        match rx.try_recv() {
            Ok(sessions) => {
                self.history_loading = false;
                if self.continue_requested {
                    self.continue_requested = false;
                    if let Some(latest) = sessions.first().cloned() {
                        self.history = sessions;
                        self.open_session(latest);
                        return;
                    }
                }
                // The session OpenCode named: its title is the generated one.
                if let Some(current) = sessions.iter().find(|session| session.id == self.session) {
                    self.session_title = Some(current.title.clone());
                }
                self.history = sessions;
            }
            Err(mpsc::TryRecvError::Empty) => self.running_history = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => self.history_loading = false,
        }
    }

    /// Deletes a session with OpenCode, then refreshes the list.
    fn delete_session(&mut self, session: SessionInfo) {
        let Some(program) = self.opencode() else { return };
        self.delete_pending = None;
        self.deleting = true;
        self.deleting_id = Some(session.id.clone());
        let root = self.root.clone();
        let (tx, rx) = mpsc::channel();
        self.running_delete = Some(rx);
        std::thread::spawn(move || {
            let mut command = Self::opencode_command(&program);
            command.args(["--standalone", "session", "delete", &session.id]).stdin(Stdio::null());
            if let Some(root) = &root {
                command.current_dir(root);
                command.env("PWD", root);
            }
            let _ = command.output();
            let _ = tx.send(());
        });
    }

    fn poll_delete(&mut self) {
        let Some(rx) = self.running_delete.take() else { return };
        match rx.try_recv() {
            Ok(()) => {
                self.deleting = false;
                if self.deleting_id.take().as_deref() == Some(self.session.as_str()) {
                    // The open session is gone; start fresh.
                    self.slash("clear");
                }
                self.history.clear();
                self.load_history();
                self.status = Some("Session deleted".to_owned());
            }
            Err(mpsc::TryRecvError::Empty) => self.running_delete = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => self.deleting = false,
        }
    }

    /// Opens a past session: its transcript comes from `opencode session export`.
    fn open_session(&mut self, session: SessionInfo) {
        if self.running_session.is_some() {
            return;
        }
        let Some(program) = self.opencode() else { return };
        self.session = session.id.clone();
        self.session_title = Some(session.title.clone());
        // A session belongs to the folder it was started in; export from there so OpenCode
        // finds its data.
        self.session_root = session.directory.clone().or_else(|| self.root.clone());
        self.messages.clear();
        self.last_changed_files.clear();
        self.last_turn_tools = 0;
        self.working_set_open = false;
        // The column is a picker: it closes once a chat is open, so the
        // conversation gets the panel's full width.
        self.history_open = false;
        self.status = Some(format!("Opening \"{}\"…", session.title));
        self.focus_input = true;
        self.session_loading = true;
        let (tx, rx) = mpsc::channel();
        self.running_session = Some(rx);
        let root = self.session_root.clone();
        std::thread::spawn(move || {
            let mut command = Self::opencode_command(&program);
            command.args(["--standalone", "session", "export", &session.id]).stdin(Stdio::null());
            if let Some(root) = &root {
                command.current_dir(root);
                command.env("PWD", root);
            }
            let value = command
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| serde_json::from_str::<Value>(&String::from_utf8_lossy(&output.stdout)).ok());
            let _ = tx.send(AgentMsg::Session(parse_export(&value)));
        });
    }

    fn poll_session(&mut self) {
        let Some(rx) = self.running_session.take() else { return };
        match rx.try_recv() {
            Ok(AgentMsg::Session(messages)) => {
                self.session_loading = false;
                self.focus_input = true;
                self.status = Some(if messages.is_empty() {
                    "This session has no messages".to_owned()
                } else {
                    "Resumed session".to_owned()
                });
                self.messages = messages;
                self.refresh_working_set();
            }
            Err(mpsc::TryRecvError::Empty) => self.running_session = Some(rx),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.session_loading = false;
                self.status = Some("Couldn't load the session".to_owned());
            }
            Ok(_) => {}
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = self.running_models.take() {
            match rx.try_recv() {
                Ok(AgentMsg::Models(models)) => {
                    self.models = models;
                    self.models_loading = false;
                    self.models_loaded = true;
                }
                Ok(_) => {}
                Err(mpsc::TryRecvError::Empty) => self.running_models = Some(rx),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.models_loading = false;
                    self.models_loaded = true;
                }
            }
        }

        let Some(rx) = &self.running else { return };
        let mut ended: Option<Option<i32>> = None;
        loop {
            match rx.try_recv() {
                Ok(AgentMsg::Text(text)) => {
                    // Errors are their own block: they must not be swallowed
                    // into the reply text, where their red would not show.
                    let is_error = text.starts_with("Error:");
                    let last_is_plain_text = self
                        .messages
                        .last()
                        .filter(|message| message.role == Role::Agent)
                        .and_then(|message| message.parts.last())
                        .is_some_and(|part| matches!(part, Part::Text(t) if !t.starts_with("Error:")));
                    if !is_error
                        && last_is_plain_text
                        && let Some(Part::Text(existing)) = self.messages.last_mut().and_then(|message| match message {
                            Message { role: Role::Agent, parts } => parts.last_mut(),
                            Message { role: Role::User, .. } => None,
                        })
                    {
                        existing.push_str(&text);
                        continue;
                    }
                    if let Some(Message { role: Role::Agent, parts }) =
                        self.messages.last_mut().filter(|m| m.role == Role::Agent)
                    {
                        parts.push(Part::Text(text));
                    } else {
                        self.messages.push(Message { role: Role::Agent, parts: vec![Part::Text(text)] });
                    }
                }
                Ok(AgentMsg::Thinking(text)) => {
                    if let Some(Message { role: Role::Agent, parts }) =
                        self.messages.last_mut().filter(|m| m.role == Role::Agent)
                    {
                        if let Some(Part::Thinking(existing)) = parts.last_mut() {
                            existing.push_str(&text);
                        } else {
                            parts.push(Part::Thinking(text));
                        }
                    } else {
                        self.messages.push(Message { role: Role::Agent, parts: vec![Part::Thinking(text)] });
                    }
                }
                Ok(AgentMsg::Tool(tool)) => {
                    if let Some(Message { role: Role::Agent, parts }) =
                        self.messages.last_mut().filter(|m| m.role == Role::Agent)
                    {
                        if let Some(Part::Tool(existing)) =
                            parts.iter_mut().find(|p| matches!(p, Part::Tool(t) if t.id == tool.id))
                        {
                            *existing = tool;
                        } else {
                            parts.push(Part::Tool(tool));
                        }
                    } else {
                        self.messages.push(Message { role: Role::Agent, parts: vec![Part::Tool(tool)] });
                    }
                    self.last_changed_files = files_touched(&self.messages);
                    self.last_turn_tools = tools_in_last_turn(&self.messages);
                }
                Ok(AgentMsg::Models(models)) => {
                    self.models = models;
                    self.models_loading = false;
                    self.models_loaded = true;
                }
                Ok(AgentMsg::Session(_)) => {}
                Ok(AgentMsg::End(code)) => {
                    // Don't break: the reader threads may still deliver lines that
                    // were buffered before the process exited.
                    ended = Some(code);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        if let Some(code) = ended {
            self.running = None;
            self.child_pid = None;
            self.last_changed_files = files_touched(&self.messages);
            self.last_turn_tools = tools_in_last_turn(&self.messages);
            // Keep the session list fresh: this run may have created a session.
            self.history.clear();
            self.load_history();
            self.status = Some(match code {
                Some(0) | None => "Ready".to_owned(),
                Some(code) => format!("OpenCode ended with exit code {code}"),
            });
            self.focus_input = true;
            // A message typed while the agent ran sends now.
            if !self.queue.is_empty() {
                let next = self.queue.remove(0);
                let root = self.root.clone();
                self.input = next;
                self.send(ctx, root.as_deref());
            }
        }
        if self.running.is_some() || self.running_models.is_some() || self.running_session.is_some() {
            ctx.request_repaint();
        }
    }

    fn send(&mut self, ctx: &egui::Context, root: Option<&Path>) {
        let mut text = self.input.trim().to_owned();
        if text.is_empty() || self.running.is_some() || self.session_loading {
            return;
        }
        if let Some(command) = text.strip_prefix('/') {
            self.slash(command);
            return;
        }
        let Some(program) = self.opencode() else {
            self.status = Some("OpenCode isn't installed: npm install -g opencode-ai".to_owned());
            return;
        };
        if !self.attached.is_empty() {
            // Attached context goes first, as @references the agent can look up.
            text = self.with_attachments(&text);
            self.attached.clear();
        }
        if self.session_root.as_deref() != root {
            // A session belongs to the folder it was started in; continuing it from another
            // folder would run the agent there.
            self.session.clear();
            self.messages.clear();
            self.session_root = root.map(Path::to_path_buf);
        }
        if self.session.is_empty() {
            self.session = Self::new_session();
            // OpenCode names the session itself; the title arrives with the
            // history refresh after the run.
            self.session_title = None;
        }
        self.messages.push(Message { role: Role::User, parts: vec![Part::Text(text.clone())] });
        self.prompt_history.push(text.clone());
        self.history_index = None;
        self.input.clear();
        self.status = Some(match root {
            Some(root) => format!("Working in {}", root.display()),
            None => "Working…".to_owned(),
        });
        self.load_models();

        let mut command = Self::opencode_command(&program);
        command
            .args(["run", "--standalone", "--thinking", "--format", "json", "--session", &self.session])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // The permission level: Auto approves the agent's tool use, Manual doesn't.
        if self.permissions != "manual" {
            command.arg("--auto");
        }
        if self.mode == "plan" {
            // The plan agent discusses; it doesn't edit files.
            command.args(["--agent", "plan"]);
        }
        if let Some(model) = &self.model {
            command.args(["--model", model]);
        }
        if let Some(root) = root {
            // The standalone server runs in this directory; the background service would
            // keep whatever directory it was started in.
            command.current_dir(root);
            command.env("PWD", root);
        }
        command.arg("--").arg(&text);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                self.status = Some(format!("Couldn't start OpenCode: {e}"));
                return;
            }
        };
        self.child_pid = Some(child.id());
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        if let Some(stdout) = child.stdout.take() {
            let tx = tx.clone();
            let wake = wake.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let Some(msg) = parse_event(&line) else { continue };
                    if tx.send(msg).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let tx = tx.clone();
            let wake = wake.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() && tx.send(AgentMsg::Text(format!("{line}\n"))).is_err() {
                        break;
                    }
                    wake.request_repaint();
                }
            });
        }
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|s| s.code());
            let _ = tx.send(AgentMsg::End(code));
            wake.request_repaint();
        });
        self.running = Some(rx);
    }

    fn stop(&mut self) {
        let Some(pid) = self.child_pid.take() else { return };
        if cfg!(windows) {
            let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).status();
        } else {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
        self.running = None;
        // A stopped run drops its queue: those messages must not fire after
        // some later run.
        self.queue.clear();
        self.refresh_working_set();
        self.status = Some("Stopped".to_owned());
    }

    /// Files edited in the last turn and how many tools it used.
    fn refresh_working_set(&mut self) {
        self.last_changed_files = files_touched(&self.messages);
        self.last_turn_tools = tools_in_last_turn(&self.messages);
    }

    fn slash(&mut self, command: &str) {
        match command.trim() {
            "clear" | "new" => {
                self.messages.clear();
                self.session.clear();
                self.session_title = None;
                self.session_root = None;
                self.queue.clear();
                self.attached.clear();
                self.last_changed_files.clear();
                self.last_turn_tools = 0;
                self.working_set_open = false;
                self.input.clear();
                self.history_index = None;
                self.status = Some("New session".to_owned());
            }
            "model" => {
                self.input.clear();
                self.model_search.clear();
                self.status = Some("Pick a model with the chip in the composer".to_owned());
            }
            "help" => {
                self.messages.push(Message {
                    role: Role::Agent,
                    parts: vec![Part::Text(
                        "Commands: /clear (new chat), /model (choose a model), /help (this text).\nEnter sends; Shift+Enter adds a newline.".to_owned(),
                    )],
                });
                self.input.clear();
            }
            other => {
                self.status = Some(format!("Unknown command /{other}"));
                self.input.clear();
            }
        }
    }

    pub(crate) fn panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.poll_history();
        self.poll_session();
        self.poll_delete();
        let pal = theme::colors();
        let running = self.running.is_some();
        let jump = std::mem::take(&mut self.jump_to_bottom);

        // The chat column, or the sessions list filling the window like Copilot Chat.
        let total = ui.available_width().max(320.0);
        let height = ui.available_height();
        // Read once: the Lines button flips `history_open` while the chat column
        // draws, so a live read here would disagree with the widths below.
        let sessions_open = self.history_open;
        // The sessions list fills the window; the chat returns when one is open.
        ui.allocate_ui_with_layout(egui::vec2(total, height), egui::Layout::top_down(Align::Min), |ui| {
            if sessions_open {
                self.sessions_column(ui, pal);
            } else {
                self.chat_column(ui, pal, running, jump);
            }
        });
        self.jump_pill(ui, pal);
        self.delete_modal(ui.ctx(), pal);
        // Files dropped onto the window attach as context.
        let dropped: Vec<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().filter_map(|file| file.path().to_path_buf().into()).collect());
        for path in dropped {
            self.attach(&path);
        }
    }

    /// The CHAT title bar, the conversation subbar, the transcript and the composer.
    fn chat_column(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool, jump: bool) {
        let width = ui.available_width();
        // Title bar: CHAT with a small underline, and the icon toolbar on the right.
        let left = ui.cursor().left();
        ui.allocate_ui_with_layout(egui::vec2(width, 36.0), egui::Layout::left_to_right(Align::Center), |ui| {
            // The label gives way to the toolbar when the column is narrow.
            if width >= 230.0 {
                ui.label(RichText::new("Chat").size(12.0).strong().color(pal.text));
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Close, "Close the Agents window").clicked() {
                    self.open = false;
                }
                let more = crate::ui::icons::button(ui, crate::ui::icons::Icon::Menu, "More actions");
                egui::Popup::menu(&more).show(|ui| {
                    if ui.button("Copy conversation").clicked() {
                        ui.ctx().copy_text(transcript_markdown(&self.messages));
                        self.status = Some("Conversation copied".to_owned());
                        ui.close();
                    }
                    if ui.button("Continue last session").clicked() {
                        self.continue_session();
                        ui.close();
                    }
                    if ui.button("New chat").clicked() {
                        self.slash("clear");
                        ui.close();
                    }
                });
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Gear, "Chat settings").clicked() {
                    self.requests_settings = true;
                }
                let sessions = crate::ui::icons::button(
                    ui,
                    crate::ui::icons::Icon::Lines,
                    if self.history_open { "Hide sessions" } else { "Show sessions" },
                );
                if sessions.clicked() {
                    self.history_open = !self.history_open;
                    if self.history_open {
                        self.load_history();
                    }
                }
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Plus, "New chat").clicked() {
                    self.slash("clear");
                }
                let (icon, tip) = if self.maximized {
                    (crate::ui::icons::Icon::Restore, "Restore the Agents window")
                } else {
                    (crate::ui::icons::Icon::Maximize, "Fill the window with the Agents panel")
                };
                if crate::ui::icons::button(ui, icon, tip).clicked() {
                    self.maximized = !self.maximized;
                }
            });
        });
        let bottom = ui.cursor().top();
        ui.painter().line_segment(
            [egui::pos2(left, bottom - 1.0), egui::pos2(left + width, bottom - 1.0)],
            egui::Stroke::new(1.0, pal.border),
        );
        // The small underline under CHAT, like the mockup's ::after.
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(left + 17.0, bottom - 1.0), egui::vec2(28.0, 1.0)),
            0.0,
            pal.text,
        );

        // Subbar: back to the sessions, the conversation title, copy.
        let left = ui.cursor().left();
        ui.allocate_ui_with_layout(egui::vec2(width, 36.0), egui::Layout::left_to_right(Align::Center), |ui| {
            if crate::ui::icons::button(ui, crate::ui::icons::Icon::ArrowLeft, "Back to sessions").clicked() {
                self.history_open = !self.history_open;
                if self.history_open {
                    self.load_history();
                }
            }
            let title = self.session_title.clone().unwrap_or_else(|| "New conversation".to_owned());
            ui.add(
                egui::Label::new(RichText::new(title.to_uppercase()).size(11.0).color(pal.text_dim))
                    .wrap_mode(egui::TextWrapMode::Truncate),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Copy, "Copy conversation").clicked() {
                    ui.ctx().copy_text(transcript_markdown(&self.messages));
                    self.status = Some("Conversation copied".to_owned());
                }
            });
        });
        let bottom = ui.cursor().top();
        ui.painter().line_segment(
            [egui::pos2(left, bottom - 1.0), egui::pos2(left + width, bottom - 1.0)],
            egui::Stroke::new(1.0, pal.border),
        );

        if self.opencode().is_none() {
            // OpenCode missing: a centered notice fills the column.
            let height = ui.available_height();
            ui.allocate_ui(egui::vec2(width, height), |ui| {
                ui.add_space(height * 0.3);
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 22.0, pal.accent.gamma_multiply(0.15));
                    crate::ui::icons::paint(ui.painter(), rect.shrink(10.0), crate::ui::icons::Icon::Agent, pal.accent);
                    ui.add_space(8.0);
                    ui.label(RichText::new("OpenCode isn't installed").strong());
                    ui.label(RichText::new("Install it with: npm install -g opencode-ai").small().color(pal.text_dim));
                    ui.label(
                        RichText::new("Coiman runs the program on this computer; it has no AI of its own.")
                            .small()
                            .color(pal.text_dim),
                    );
                });
            });
            return;
        }

        // The composer stays at the bottom while the transcript scrolls above it.
        // The composer's height comes from the previous frame; a one-frame lag is
        // invisible, and it keeps the two from fighting over the remaining space.
        let width = ui.available_width();
        let avail = ui.available_height();
        let composer_h = self.composer_height.clamp(60.0, (avail - 80.0).max(60.0));
        let rest = (avail - composer_h).max(60.0);
        ui.allocate_ui_with_layout(egui::vec2(width, rest), egui::Layout::top_down(Align::Min), |ui| {
            self.transcript(ui, pal, running, jump);
        });
        self.composer_rect = Some(self.composer(ui, pal, running));
    }

    /// The conversation above the composer: prompts, replies, tool cards, working set.
    fn transcript(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool, jump: bool) {
        let last_user = self.messages.iter().rposition(|message| message.role == Role::User);
        let output = ScrollArea::vertical()
            .id_salt("agents_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // The chat's padding matches the ask box's, so both line up.
                egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                    // Pin the content 1px inside the available: the labels'
                    // galleys are sub-pixel wider than the wrap width, and a
                    // content min above the row would creep the panel wider.
                    ui.set_width((ui.available_width() - 1.0).max(40.0));
                    // The mockup pads the transcript 14/16/12.
                    ui.add_space(2.0);
                    if self.session_loading {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Opening session…").small().color(pal.text_dim));
                        });
                        return;
                    }
                    if self.messages.is_empty() {
                        ui.add_space(40.0);
                        ui.vertical_centered(|ui| {
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
                            ui.painter().circle_filled(rect.center(), 22.0, pal.accent.gamma_multiply(0.15));
                            crate::ui::icons::paint(
                                ui.painter(),
                                rect.shrink(10.0),
                                crate::ui::icons::Icon::Agent,
                                pal.accent,
                            );
                            ui.add_space(10.0);
                            ui.label(RichText::new("What can I help you build?").strong());
                            ui.label(
                                RichText::new("Ask a question or describe a task for your project.")
                                    .small()
                                    .color(pal.text_dim),
                            );
                        });
                        return;
                    }
                    let messages = self.messages.clone();
                    for (index, message) in messages.iter().enumerate() {
                        match message.role {
                            Role::User => {
                                // Every prompt is a bubble on the right, the first one too.
                                // Hovering it offers copy, retry and edit.
                                let root = self.root.clone();
                                if let Some(action) = user_bubble(ui, message, pal, index) {
                                    let text = message_text(message);
                                    match action {
                                        UserAction::Retry => {
                                            self.input = text;
                                            self.send(ui.ctx(), root.as_deref());
                                        }
                                        UserAction::Edit => {
                                            self.input = text;
                                            self.focus_input = true;
                                        }
                                    }
                                }
                                if Some(index) == last_user {
                                    self.status_line(ui, pal, running);
                                }
                            }
                            Role::Agent => agent_message(ui, message, pal, running, index),
                        }
                    }
                    self.working_set_card(ui, pal);
                    if jump {
                        ui.scroll_to_rect(ui.min_rect(), Some(Align::BOTTOM));
                    }
                });
            });
        self.at_bottom = output.state.clamped_offset().y + output.inner_rect.height() >= output.content_size.y - 2.0;
    }

    /// The mockup's status line: what the last turn did, under its prompt.
    fn status_line(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool) {
        ui.add_space(4.0);
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Working…").small().color(pal.text_dim));
            });
            ui.add_space(10.0);
            return;
        }
        let tools = self.last_turn_tools;
        let changed = self.last_changed_files.len();
        let status = self.status.clone().filter(|text| !text.is_empty());
        if tools == 0 && changed == 0 && status.is_none() {
            ui.add_space(6.0);
            return;
        }
        ui.horizontal(|ui| {
            if tools > 0 || changed > 0 {
                ui.label(RichText::new("✓").small().color(Color32::from_rgb(0x89, 0xd1, 0x85)));
                if tools > 0 {
                    ui.label(RichText::new(format!("Ran {tools} tools")).small().color(pal.text_dim));
                }
                if changed > 0 {
                    ui.label(
                        RichText::new(format!("+{changed} files changed")).small().monospace().color(pal.text_dim),
                    );
                }
            }
            if let Some(status) = status {
                ui.label(RichText::new(status).small().color(pal.text_dim));
            }
        });
        ui.add_space(10.0);
    }

    /// The mockup's working-set card: the files the last turn edited.
    fn working_set_card(&mut self, ui: &mut egui::Ui, pal: theme::Palette) {
        if !self.working_set_open || self.last_changed_files.is_empty() {
            return;
        }
        ui.add_space(6.0);
        let files = self.last_changed_files.clone();
        egui::Frame::new()
            .fill(pal.editor_bg)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .corner_radius(5.0)
            .inner_margin(egui::Margin::ZERO)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Head: chevron, "Files changed", count.
                let (head, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), Sense::click());
                let y = head.center().y;
                let chevron = egui::Rect::from_min_size(egui::pos2(head.left() + 9.0, y - 5.0), egui::vec2(10.0, 10.0));
                crate::ui::icons::paint(ui.painter(), chevron, crate::ui::icons::Icon::ChevronDown, pal.text_dim);
                ui.painter().text(
                    egui::pos2(head.left() + 26.0, y),
                    Align2::LEFT_CENTER,
                    "Files changed",
                    FontId::proportional(11.0),
                    pal.text,
                );
                ui.painter().text(
                    egui::pos2(head.right() - 9.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{} files", files.len()),
                    FontId::proportional(11.0),
                    pal.text_dim,
                );
                if resp.clicked() {
                    self.working_set_open = false;
                }
                // Rows: open the file in the editor.
                for path in &files {
                    let (row, row_resp) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width(), 27.0), Sense::click());
                    if ui.rect_contains_pointer(row) {
                        ui.painter().rect_filled(row, 0.0, pal.hover_bg);
                    }
                    ui.painter().line_segment(
                        [egui::pos2(row.left() + 10.0, row.top()), egui::pos2(row.right() - 10.0, row.top())],
                        egui::Stroke::new(1.0, pal.border),
                    );
                    let name =
                        Path::new(path).file_name().map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                    let icon = egui::Rect::from_min_size(
                        egui::pos2(row.left() + 12.0, row.center().y - 6.5),
                        egui::vec2(13.0, 13.0),
                    );
                    crate::ui::icons::paint_file(ui.painter(), icon, path);
                    ui.painter().text(
                        egui::pos2(row.left() + 32.0, row.center().y),
                        Align2::LEFT_CENTER,
                        fit_text(ui, &name, FontId::proportional(11.0), pal.text, row.width() - 44.0),
                        FontId::proportional(11.0),
                        pal.text,
                    );
                    if row_resp.clicked() {
                        // Ctrl-click shows the file's diff; a plain click opens it.
                        if row_resp.ctx.input(|i| i.modifiers.ctrl) {
                            self.diff_request = Some(PathBuf::from(path));
                        } else {
                            self.open_request = Some(PathBuf::from(path));
                        }
                    }
                    row_resp.on_hover_text(path.clone());
                }
            });
    }

    /// The floating "Jump to bottom" pill, shown while the transcript is scrolled up.
    fn jump_pill(&mut self, ui: &mut egui::Ui, pal: theme::Palette) {
        if !self.at_bottom
            && let Some(composer) = self.composer_rect
        {
            let size = egui::vec2(122.0, 24.0);
            let pill = egui::Rect::from_min_size(
                egui::pos2(composer.right() - size.x - 8.0, composer.top() - size.y - 4.0),
                size,
            );
            let resp = ui.interact(pill, egui::Id::unique("agents_jump_pill"), Sense::click());
            ui.painter().rect_filled(pill, 12.0, if resp.hovered() { pal.hover_bg } else { pal.widget_bg });
            ui.painter().rect_stroke(pill, 12.0, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            let icon =
                egui::Rect::from_min_size(egui::pos2(pill.left() + 8.0, pill.center().y - 5.0), egui::vec2(10.0, 10.0));
            crate::ui::icons::paint(ui.painter(), icon, crate::ui::icons::Icon::ArrowDown, pal.text_dim);
            ui.painter().text(
                egui::pos2(pill.left() + 24.0, pill.center().y),
                Align2::LEFT_CENTER,
                "Jump to bottom",
                FontId::proportional(11.0),
                pal.text,
            );
            if resp.clicked() {
                self.jump_to_bottom = true;
            }
        }
    }

    /// The side-by-side ALL SESSIONS column.
    fn sessions_column(&mut self, ui: &mut egui::Ui, pal: theme::Palette) {
        let width = ui.available_width();
        let left = ui.cursor().left();
        ui.allocate_ui_with_layout(egui::vec2(width, 36.0), egui::Layout::left_to_right(Align::Center), |ui| {
            ui.add_space(13.0);
            ui.label(RichText::new("All sessions").size(11.0).color(pal.text));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Close, "Hide sessions").clicked() {
                    self.history_open = false;
                }
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Search, "Search sessions").clicked() {
                    self.session_search_open = !self.session_search_open;
                    if !self.session_search_open {
                        self.session_search.clear();
                    } else {
                        self.focus_session_search = true;
                    }
                }
                if crate::ui::icons::button(ui, crate::ui::icons::Icon::Refresh, "Refresh sessions").clicked() {
                    self.load_history();
                }
            });
        });
        let bottom = ui.cursor().top();
        ui.painter().line_segment(
            [egui::pos2(left, bottom - 1.0), egui::pos2(left + width, bottom - 1.0)],
            egui::Stroke::new(1.0, pal.border),
        );
        if self.session_search_open {
            let search = ui.add_sized(
                [width - 16.0, 0.0],
                TextEdit::singleline(&mut self.session_search)
                    .id(egui::Id::unique("agents_session_search"))
                    .hint_text("Search sessions"),
            );
            if std::mem::take(&mut self.focus_session_search) {
                search.request_focus();
            }
        }
        let sessions = self.matching_sessions();
        let current = self.session.clone();
        ScrollArea::vertical().id_salt("agents_sessions").auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.history_loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Loading sessions…").small().color(pal.text_dim));
                });
            } else if sessions.is_empty() {
                ui.add_space(14.0);
                ui.label(RichText::new("No sessions yet").small().color(pal.text_dim));
            }
            for session in &sessions {
                self.session_row(ui, session, pal, session.id == current);
            }
        });
        ui.add_space(12.0);
    }

    /// One session row: name, relative time, delete on hover.
    fn session_row(&mut self, ui: &mut egui::Ui, session: &SessionInfo, pal: theme::Palette, selected: bool) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 59.0), Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if selected {
            pal.accent.gamma_multiply(0.35)
        } else if hovered {
            pal.hover_bg
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 0.0, fill);
        if selected {
            ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
        }
        // Name, with the mockup's bullet before it when selected.
        let left = rect.left() + 13.0;
        let name_x = left + f32::from(selected) * 12.0;
        ui.painter().text(
            egui::pos2(name_x, rect.top() + 13.0),
            Align2::LEFT_TOP,
            fit_text(
                ui,
                &truncate(&session.title, 64),
                FontId::proportional(12.0),
                pal.text,
                rect.width() - (name_x - rect.left()) - 12.0,
            ),
            FontId::proportional(12.0),
            pal.text,
        );
        if selected {
            ui.painter().circle_filled(egui::pos2(left + 2.5, rect.top() + 16.0), 2.5, pal.text);
        }
        // Meta: relative time.
        ui.painter().text(
            egui::pos2(name_x, rect.bottom() - 15.0),
            Align2::LEFT_CENTER,
            relative_time(session.updated),
            FontId::proportional(10.0),
            pal.text_dim,
        );
        // Delete on hover.
        let del_rect =
            egui::Rect::from_min_size(egui::pos2(rect.right() - 26.0, rect.top() + 11.0), egui::vec2(20.0, 20.0));
        if hovered {
            crate::ui::icons::paint(ui.painter(), del_rect.shrink(3.0), crate::ui::icons::Icon::Close, pal.text_dim);
        }
        let deleting = self.deleting && self.deleting_id.as_deref() == Some(session.id.as_str());
        if deleting {
            // The delete is running: a spinner where the × was.
            ui.painter().circle_stroke(del_rect.center(), 5.0, egui::Stroke::new(1.5, pal.text_dim));
        }
        let del = ui.interact(del_rect, egui::Id::unique("agents_delete_row").with(&session.id), Sense::click());
        if del.clicked() && !deleting {
            self.delete_pending = Some(session.clone());
        }
        if resp.clicked() && !del.hovered() {
            self.open_session(session.clone());
        }
        resp.on_hover_text(format!("{}\n{}\n{}", session.title, session.id, time_label(session.updated)));
    }

    /// Confirmation before a session is deleted.
    fn delete_modal(&mut self, ctx: &egui::Context, pal: theme::Palette) {
        let Some(session) = self.delete_pending.clone() else { return };
        let modal = egui::Modal::new(egui::Id::unique("agents_delete_session")).show(ctx, |ui| {
            ui.label(RichText::new("Delete session?").strong());
            ui.label(RichText::new(&session.title).small().color(pal.text_dim));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    self.delete_session(session.clone());
                    ui.close();
                }
                if ui.button("Cancel").clicked() {
                    self.delete_pending = None;
                    ui.close();
                }
            });
        });
        if modal.should_close() {
            self.delete_pending = None;
        }
    }

    /// Opens the most recent session, loading the list first if needed.
    fn continue_session(&mut self) {
        if let Some(latest) = self.history.first().cloned() {
            self.open_session(latest);
        } else {
            // The click counts even while a load is in flight: poll_history
            // picks the flag up when the list arrives.
            self.continue_requested = true;
            if !self.history_loading {
                self.load_history();
            }
        }
    }

    /// Steps through sent prompts with the up/down arrows in an empty box.
    fn cycle_history(&mut self, step: i32) {
        if self.prompt_history.is_empty() {
            return;
        }
        let last = self.prompt_history.len() - 1;
        self.history_index = match self.history_index {
            None if step < 0 => Some(last),
            None => None,
            Some(index) if step < 0 => Some(index.saturating_sub(1)),
            Some(index) if index < last => Some(index + 1),
            Some(_) => None,
        };
        self.input = self.history_index.map(|index| self.prompt_history[index].clone()).unwrap_or_default();
    }

    fn composer(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool) -> Rect {
        let root = self.root.clone();
        let mut composer_rect = Rect::ZERO;
        let outer =
            egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 6, top: 0, bottom: 6 }).show(ui, |ui| {
                // The working-set toggle sits above the box, like the mockup.
                if !self.last_changed_files.is_empty() {
                    let (row, resp) =
                        ui.allocate_exact_size(egui::vec2(ui.available_width() - 12.0, 18.0), Sense::click());
                    let y = row.center().y;
                    ui.painter().circle_filled(egui::pos2(row.left() + 3.0, y), 3.0, pal.accent);
                    ui.painter().text(
                        egui::pos2(row.left() + 14.0, y),
                        Align2::LEFT_CENTER,
                        "Editor working set",
                        FontId::proportional(11.0),
                        pal.text_dim,
                    );
                    ui.painter().text(
                        egui::pos2(row.right(), y),
                        Align2::RIGHT_CENTER,
                        format!("{} files", self.last_changed_files.len()),
                        FontId::proportional(11.0),
                        pal.border,
                    );
                    if resp.clicked() {
                        self.working_set_open = !self.working_set_open;
                    }
                    ui.add_space(2.0);
                }
                // Messages typed while the agent runs, removable until they send.
                if !self.queue.is_empty() {
                    for text in self.queue.clone() {
                        egui::Frame::new()
                            .fill(pal.hover_bg)
                            .corner_radius(4.0)
                            .inner_margin(egui::Margin::symmetric(7, 3))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Queued · {}", truncate(&text, 28)))
                                            .small()
                                            .color(pal.text_dim),
                                    );
                                    if crate::ui::icons::button(ui, crate::ui::icons::Icon::Close, "Remove from queue")
                                        .clicked()
                                    {
                                        self.queue.retain(|kept| *kept != text);
                                    }
                                });
                            });
                    }
                    ui.add_space(2.0);
                }
                // Attached context, each removable until it sends.
                if !self.attached.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 4.0);
                        for path in self.attached.clone() {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                            egui::Frame::new()
                                .fill(pal.hover_bg)
                                .corner_radius(3.0)
                                .inner_margin(egui::Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(format!("@{name}")).size(10.5).color(pal.text_dim));
                                        if crate::ui::icons::button(
                                            ui,
                                            crate::ui::icons::Icon::Close,
                                            "Remove attached context",
                                        )
                                        .clicked()
                                        {
                                            self.attached.retain(|kept| kept != &path);
                                        }
                                    });
                                });
                        }
                    });
                    ui.add_space(2.0);
                }
                // The ask box: a border that brightens while it has focus.
                let border = if self.input_focused { mix(pal.border, pal.text, 0.35) } else { pal.border };
                let box_frame = egui::Frame::new()
                    .fill(pal.widget_bg)
                    .stroke(egui::Stroke::new(1.0, border))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin { left: 9, right: 9, top: 8, bottom: 7 });
                let response = box_frame.show(ui, |ui| {
                    // The box grows upward as you type, up to eight lines; past that the text
                    // scrolls inside it, and the caret is followed so it stays visible.
                    let row_h = ui.fonts_mut(|f| f.row_height(&egui::TextStyle::Body.resolve(ui.style())));
                    ScrollArea::vertical()
                        .id_salt("agents_input")
                        .auto_shrink([false, true])
                        .max_height(row_h * 8.0 + 10.0)
                        .show(ui, |ui| {
                            let avail = ui.available_width();
                            // A stable id: the queue and working-set rows above come
                            // and go, and their widget count must not shift this
                            // field's identity — that would drop focus mid-typing.
                            let output = TextEdit::multiline(&mut self.input)
                                .id(egui::Id::unique("agents_prompt"))
                                .desired_rows(1)
                                .frame(egui::Frame::NONE)
                                .hint_text("Ask opencode to run a task…")
                                .desired_width(avail)
                                .show(ui);
                            let focus_requested = std::mem::take(&mut self.focus_input);
                            let focused = output.response.response.has_focus();
                            // Only when focus is actually requested: calling it every
                            // frame would reset egui's focus-lock filter, and arrow
                            // keys would then move focus away from the box.
                            if focus_requested {
                                output.response.response.request_focus();
                            }
                            self.input_focused = focused || focus_requested;
                            if focused && let Some(cursor) = &output.cursor_range {
                                let caret = output
                                    .galley
                                    .pos_from_cursor(cursor.primary)
                                    .translate(output.galley_pos.to_vec2());
                                ui.scroll_to_rect(caret, None);
                            }
                        });
                    // The @ file reference: a popup over the box with the folder's entries.
                    if let Some((start, token)) = at_token(&self.input).map(|(at, token)| (at, token.to_owned())) {
                        let entries = at_entries(root.as_deref(), &token);
                        if !entries.is_empty() {
                            let anchor = egui::Rect::from_min_size(
                                ui.cursor().left_top() + egui::vec2(0.0, 4.0),
                                egui::vec2(1.0, 1.0),
                            );
                            let popup = egui::Popup::new(
                                egui::Id::unique("agents_at_popup"),
                                ui.ctx().clone(),
                                anchor,
                                egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("agents_at_popup")),
                            );
                            popup.show(|ui| {
                                ui.set_width(220.0);
                                for name in entries {
                                    if ui.selectable_label(false, &name).clicked() {
                                        let dir = token
                                            .rsplit_once('/')
                                            .map(|(dir, _)| format!("{dir}/"))
                                            .unwrap_or_default();
                                        self.input = format!("{}{}@{}{} ", &self.input[..start], "", dir, name);
                                        ui.close();
                                    }
                                }
                            });
                        }
                    }
                    // An image pasted from the clipboard attaches as a file.
                    if ui.input(|i| i.key_pressed(egui::Key::V) && i.modifiers.ctrl)
                        && let Ok(mut clipboard) = arboard::Clipboard::new()
                        && let Ok(image) = clipboard.get_image()
                    {
                        let nanos = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_nanos());
                        let path =
                            std::env::temp_dir().join(format!("coiman_paste_{}_{nanos}.png", std::process::id()));
                        let png = image::RgbaImage::from_raw(
                            image.width as u32,
                            image.height as u32,
                            image.bytes.into_owned(),
                        );
                        if let Some(png) = png
                            && png.save(&path).is_ok()
                        {
                            self.attach(&path);
                        }
                    }
                    // Enter sends, or queues while the agent runs. Only with the
                    // box focused and no modifiers — Shift and Ctrl Enter are
                    // newlines, and Enter elsewhere belongs to that widget.
                    if self.input_focused && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none()) {
                        let text = self.input.trim().to_owned();
                        self.input.clear();
                        if !text.is_empty() {
                            if running {
                                self.queue.push(text);
                                self.status = Some("Queued — it sends when the agent finishes".to_owned());
                            } else {
                                self.input = text;
                                self.send(ui.ctx(), root.as_deref());
                            }
                        }
                    }
                    // The up arrow in an empty box cycles sent prompts.
                    let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp) && i.modifiers.is_none());
                    let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown) && i.modifiers.is_none());
                    if self.input.is_empty() && self.input_focused {
                        if up {
                            self.cycle_history(-1);
                        } else if down {
                            self.cycle_history(1);
                        }
                    }
                    // The flat chips row: opencode, agent mode, model, permissions, path, send.
                    // It wraps on narrow panels so the composer's min width stays small.
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 5.0;
                        let open = chip(ui, pal, "opencode", true, Some(crate::ui::icons::Icon::Agent));
                        egui::Popup::menu(&open).show(|ui| {
                            if let Some(file) = self.active_file.clone() {
                                let name = file
                                    .file_name()
                                    .map_or_else(|| file.display().to_string(), |n| n.to_string_lossy().into_owned());
                                if ui.button(format!("Add context: {name}")).clicked() {
                                    self.attach(&file);
                                    ui.close();
                                }
                            }
                            if let Some(root) = self.root.clone() {
                                let folder = root
                                    .file_name()
                                    .map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
                                if ui.button(format!("Add context: {folder}")).clicked() {
                                    self.attach(&root);
                                    ui.close();
                                }
                            }
                        });
                        open.on_hover_text("Runs OpenCode on this computer; the menu adds context");
                        chip_divider(ui, pal);
                        let mode = if self.mode == "plan" { "Plan" } else { "Agent" };
                        let mode_resp = chip(ui, pal, mode, true, Some(crate::ui::icons::Icon::Agent));
                        egui::Popup::menu(&mode_resp).show(|ui| {
                            if ui.selectable_label(self.mode != "plan", "Agent — edits files").clicked() {
                                self.mode = "build".to_owned();
                                self.save_prefs();
                                ui.close();
                            }
                            if ui.selectable_label(self.mode == "plan", "Plan — discuss only").clicked() {
                                self.mode = "plan".to_owned();
                                self.save_prefs();
                                ui.close();
                            }
                        });
                        chip_divider(ui, pal);
                        let model =
                            self.model.as_deref().map(|name| truncate(name, 12)).unwrap_or_else(|| "Auto".to_owned());
                        let model_resp = chip(ui, pal, &model, true, None);
                        if model_resp.clicked() {
                            self.focus_model_search = true;
                            self.model_search.clear();
                        }
                        egui::Popup::menu(&model_resp).show(|ui| {
                            let search = ui.add(
                                TextEdit::singleline(&mut self.model_search)
                                    .id(egui::Id::unique("agents_model_search"))
                                    .hint_text("Search models"),
                            );
                            if std::mem::take(&mut self.focus_model_search) {
                                search.request_focus();
                            }
                            if ui.selectable_label(self.model.is_none(), "Auto — let OpenCode choose").clicked() {
                                self.model = None;
                                self.save_prefs();
                                ui.close();
                            }
                            if self.models_loading {
                                ui.label(RichText::new("Loading models…").small().color(pal.text_dim));
                            }
                            // At most 8 rows; the rest is scrolled through.
                            egui::ScrollArea::vertical()
                                .id_salt("agents_model_list")
                                .auto_shrink([false, false])
                                .max_height(8.0 * 22.0)
                                .show(ui, |ui| {
                                    for name in self.matching_models() {
                                        if ui
                                            .selectable_label(
                                                self.model.as_deref() == Some(name.as_str()),
                                                name.clone(),
                                            )
                                            .clicked()
                                        {
                                            self.model = Some(name);
                                            self.save_prefs();
                                            ui.close();
                                        }
                                    }
                                });
                        });
                        chip_divider(ui, pal);
                        let permissions = if self.permissions == "manual" { "Manual" } else { "Auto" };
                        let permissions_resp = chip(ui, pal, permissions, true, None);
                        egui::Popup::menu(&permissions_resp).show(|ui| {
                            if ui.selectable_label(self.permissions != "manual", "Auto — approve tool use").clicked()
                            {
                                self.permissions = "auto".to_owned();
                                self.save_prefs();
                                ui.close();
                            }
                            if ui.selectable_label(self.permissions == "manual", "Manual — ask first").clicked() {
                                self.permissions = "manual".to_owned();
                                self.save_prefs();
                                ui.close();
                            }
                        });
                        // The workspace path fills the middle, faint and truncated.
                        // `available_width` reports the row's full width here, so the
                        // remaining space comes from the cursor instead.
                        let path = root
                            .as_ref()
                            .map(|root| root.display().to_string())
                            .unwrap_or_else(|| "no folder open".to_owned());
                        let row_right = ui.max_rect().right();
                        // The path fills the middle but stays bounded: if it took
                        // every remaining pixel, the composer's min width would
                        // equal the panel's width, and opening the sessions column
                        // would ratchet the panel wider and wider.
                        let path_width = (row_right - ui.cursor().left() - 32.0).clamp(30.0, 120.0);
                        let (path_rect, _) = ui.allocate_exact_size(egui::vec2(path_width, 20.0), Sense::hover());
                        ui.put(
                            path_rect,
                            egui::Label::new(RichText::new(path).size(10.0).color(pal.text_dim))
                                .wrap_mode(egui::TextWrapMode::Truncate),
                        );
                        // The flat send (or stop) button.
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 25.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 4.0, pal.accent.gamma_multiply(0.35));
                        }
                        if running {
                            ui.painter().rect_filled(
                                rect.shrink(9.0),
                                1.5,
                                if resp.hovered() { pal.text } else { pal.text_dim },
                            );
                            if resp.clicked() {
                                self.stop();
                            }
                            resp.on_hover_text("Stop");
                        } else {
                            let can_send = !self.input.trim().is_empty() && !self.session_loading;
                            crate::ui::icons::paint(
                                ui.painter(),
                                rect.shrink(5.0),
                                crate::ui::icons::Icon::Send,
                                if can_send { pal.text } else { pal.text_dim },
                            );
                            if resp.clicked() && can_send {
                                self.send(ui.ctx(), root.as_deref());
                            }
                            resp.on_hover_text("Send (Enter)");
                        }
                    });
                });
                composer_rect = response.response.rect;
                // Slash commands, offered as quick buttons.
                if self.input.starts_with('/') {
                    let typed = self.input.clone();
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        for (command, label) in [("/clear", "Clear chat"), ("/model", "Model"), ("/help", "Help")] {
                            if command.starts_with(&typed) && ui.small_button(label).clicked() {
                                self.slash(command);
                            }
                        }
                    });
                }
            });
        self.composer_height = outer.response.rect.height();
        composer_rect
    }

    /// Adds a path to the attached context, once.
    /// The sessions that match the search text, in the list's order.
    fn matching_sessions(&self) -> Vec<SessionInfo> {
        let query = self.session_search.to_lowercase();
        self.history
            .iter()
            .filter(|session| {
                query.is_empty()
                    || session.title.to_lowercase().contains(&query)
                    || session.id.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    /// The models that match the popup's search text.
    fn matching_models(&self) -> Vec<String> {
        let query = self.model_search.to_lowercase();
        if query.is_empty() {
            return self.models.clone();
        }
        self.models.iter().filter(|name| name.to_lowercase().contains(&query)).cloned().collect()
    }

    /// The prompt with its attached context, as @references the agent can look up.
    fn with_attachments(&self, text: &str) -> String {
        if self.attached.is_empty() {
            return text.to_owned();
        }
        let references = self.attached.iter().map(|path| format!("@{path}")).collect::<Vec<_>>().join(" ");
        format!("{references}\n{text}")
    }

    fn attach(&mut self, path: &Path) {
        let text = path.display().to_string();
        if !self.attached.contains(&text) {
            self.attached.push(text);
        }
        self.focus_input = true;
    }
}

/// What a hover action on a user bubble asked for.
pub(crate) enum UserAction {
    Retry,
    Edit,
}

fn user_bubble(ui: &mut egui::Ui, message: &Message, pal: theme::Palette, index: usize) -> Option<UserAction> {
    ui.add_space(14.0);
    // The bubble sits on the right, rounded, no avatar, no border.
    let action = ui
        .with_layout(egui::Layout::right_to_left(Align::TOP), |ui| -> Option<UserAction> {
            ui.add_space(22.0);
            let bubble = egui::Frame::new()
                .fill(pal.widget_bg)
                .corner_radius(5.0)
                .inner_margin(egui::Margin::symmetric(11, 9))
                .show(ui, |ui| {
                    ui.set_max_width((ui.available_width() * 0.82).max(120.0));
                    for part in &message.parts {
                        if let Part::Text(text) = part {
                            markdown(ui, text, pal);
                        }
                    }
                });
            // Hover actions: copy, retry, edit.
            if ui.rect_contains_pointer(bubble.response.rect) {
                let rect = bubble.response.rect;
                let size = 16.0;
                let y = rect.top() + 2.0;
                let mut x = rect.right() - size - 2.0;
                let copy = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                let resp = ui.interact(copy, egui::Id::unique("agents_user_copy").with(index), Sense::click());
                ui.painter().rect_filled(copy, 3.0, pal.editor_bg);
                crate::ui::icons::paint(ui.painter(), copy.shrink(2.5), crate::ui::icons::Icon::Copy, pal.text_dim);
                if resp.clicked() {
                    ui.ctx().copy_text(message_text(message));
                }
                x -= size + 4.0;
                let retry = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                let resp = ui.interact(retry, egui::Id::unique("agents_user_retry").with(index), Sense::click());
                ui.painter().rect_filled(retry, 3.0, pal.editor_bg);
                crate::ui::icons::paint(ui.painter(), retry.shrink(2.5), crate::ui::icons::Icon::Refresh, pal.text_dim);
                if resp.clicked() {
                    return Some(UserAction::Retry);
                }
                x -= size + 4.0;
                let edit = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                let resp = ui.interact(edit, egui::Id::unique("agents_user_edit").with(index), Sense::click());
                ui.painter().rect_filled(edit, 3.0, pal.editor_bg);
                crate::ui::icons::paint(ui.painter(), edit.shrink(2.5), crate::ui::icons::Icon::Discard, pal.text_dim);
                if resp.clicked() {
                    return Some(UserAction::Edit);
                }
            }
            None
        })
        .inner;
    ui.add_space(8.0);
    action
}

fn agent_message(ui: &mut egui::Ui, message: &Message, pal: theme::Palette, live: bool, index: usize) {
    ui.add_space(6.0);
    // Replies are plain: no avatar, no name label.
    let top = ui.cursor().left_top();
    for (part_index, part) in message.parts.iter().enumerate() {
        match part {
            Part::Text(text) => {
                if text.starts_with("Error:") {
                    // A compact red line, like VS Code's error text: no bubble,
                    // so a history chat with failed runs is not a wall of red.
                    ui.add_space(3.0);
                    ui.label(RichText::new(truncate(text, 120)).small().color(Color32::from_rgb(0xF1, 0x4C, 0x4C)));
                    ui.add_space(6.0);
                } else {
                    markdown(ui, text, pal);
                }
            }
            Part::Thinking(text) => {
                // A unique salt: egui derives a CollapsingHeader's id from its
                // text, and every thinking block is called "Thinking".
                egui::CollapsingHeader::new(RichText::new("Thinking").small().strong().color(pal.text_dim))
                    .id_salt(format!("agents_thinking_{index}_{part_index}"))
                    .default_open(live)
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).italics().color(pal.text_dim).size(12.5)).wrap());
                    });
            }
            Part::Tool(tool) => tool_card(ui, tool, pal),
        }
    }
    let rect = Rect::from_min_max(top, egui::pos2(ui.min_rect().right(), ui.cursor().top()));
    copy_overlay(ui, rect, egui::Id::unique("agents_agent_copy").with(index), message_text(message), pal);
    ui.add_space(10.0);
}

/// A bordered tool card: chevron, "name · title", and a status mark on the right.
fn tool_card(ui: &mut egui::Ui, tool: &ToolPart, pal: theme::Palette) {
    ui.add_space(6.0);
    let id = egui::Id::unique("agents_tool_card").with(&tool.id);
    let open = ui.memory_mut(|memory| memory.data.get_temp::<bool>(id).unwrap_or(false));
    egui::Frame::new()
        .fill(pal.editor_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::ZERO)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (head, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 35.0), Sense::click());
            if resp.clicked() {
                ui.memory_mut(|memory| memory.data.insert_temp(id, !open));
            }
            let y = head.center().y;
            let chevron = egui::Rect::from_min_size(egui::pos2(head.left() + 10.0, y - 5.0), egui::vec2(10.0, 10.0));
            crate::ui::icons::paint(
                ui.painter(),
                chevron,
                if open { crate::ui::icons::Icon::ChevronDown } else { crate::ui::icons::Icon::ChevronRight },
                pal.text_dim,
            );
            ui.painter().text(
                egui::pos2(head.left() + 28.0, y),
                Align2::LEFT_CENTER,
                fit_text(
                    ui,
                    &format!("{} · {}", tool.name, tool.title),
                    FontId::proportional(12.0),
                    pal.text,
                    head.width() - 64.0,
                ),
                FontId::proportional(12.0),
                pal.text,
            );
            let (color, mark) = match tool.status.as_str() {
                "completed" => (Color32::from_rgb(0x89, 0xd1, 0x85), "✓"),
                "error" => (Color32::from_rgb(0xF1, 0x4C, 0x4C), "×"),
                _ => (pal.text_dim, "…"),
            };
            ui.painter().text(
                egui::pos2(head.right() - 10.0, y),
                Align2::RIGHT_CENTER,
                mark,
                FontId::proportional(12.0),
                color,
            );
            if open {
                ui.painter().line_segment(
                    [egui::pos2(head.left(), head.bottom()), egui::pos2(head.right(), head.bottom())],
                    egui::Stroke::new(1.0, pal.border),
                );
                // Detail: the tool's input and output, dim and indented like the mockup.
                ui.horizontal(|ui| {
                    ui.add_space(33.0);
                    ui.vertical(|ui| {
                        if !tool.input.is_empty() {
                            ui.label(RichText::new("Input").size(10.0).strong().color(pal.text_dim));
                            mono_text(ui, &tool.input, pal.text_dim, 10.5);
                            ui.add_space(4.0);
                        }
                        if !tool.output.is_empty() {
                            ui.label(RichText::new("Output").size(10.0).strong().color(pal.text_dim));
                            mono_text(ui, &truncate(&tool.output, 6_000), pal.text, 11.0);
                        }
                    });
                });
                ui.add_space(8.0);
            }
        });
}

/// Highlighted lines of a code block, shared across frames.
type HighlightLines = Option<std::rc::Rc<Vec<Vec<highlight::Span>>>>;

/// A code block with syntect highlighting, cached by language and text.
fn code_block(ui: &mut egui::Ui, text: &str, pal: theme::Palette, lang: &str) {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    thread_local! {
        static CACHE: RefCell<HashMap<(String, String), HighlightLines>> = RefCell::new(HashMap::new());
    }
    let text = truncate(text, 12_000);
    let key = (lang.to_lowercase(), text.clone());
    let lines = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 64 {
            cache.clear();
        }
        cache.entry(key).or_insert_with(|| highlight::snippet(lang, &text).map(Rc::new)).clone()
    });
    let top = ui.cursor().left_top();
    egui::Frame::new()
        .fill(pal.editor_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            match &lines {
                Some(lines) => {
                    let mut job = LayoutJob::default();
                    let format =
                        |color: Color32| TextFormat { font_id: FontId::monospace(12.0), color, ..Default::default() };
                    for line in &**lines {
                        let mut col = 0usize;
                        for span in line {
                            let end = col + span.len;
                            if end <= text.len() && text.is_char_boundary(col) && text.is_char_boundary(end) {
                                job.append(&text[col..end], 0.0, format(theme::c32(span.color)));
                            }
                            col = end;
                        }
                        job.append("\n", 0.0, format(pal.text));
                    }
                    ui.add(egui::Label::new(job).wrap());
                }
                None => {
                    ui.add(egui::Label::new(RichText::new(&text).monospace().size(12.0).color(pal.text)).wrap());
                }
            }
        });
    // Copy on hover, over the block's top-right corner.
    let rect =
        Rect::from_min_max(top, egui::pos2(ui.min_rect().right(), ui.cursor().top() - ui.spacing().item_spacing.y));
    let id = egui::Id::unique("agents_code_copy").with(top.x as u32).with(top.y as u32);
    if ui.rect_contains_pointer(rect) {
        let button = Rect::from_min_size(egui::pos2(rect.right() - 18.0, rect.top() + 4.0), egui::vec2(16.0, 16.0));
        let resp = ui.interact(button, id, Sense::click());
        ui.painter().rect_filled(button, 3.0, pal.widget_bg);
        crate::ui::icons::paint(ui.painter(), button.shrink(2.5), crate::ui::icons::Icon::Copy, pal.text_dim);
        if resp.clicked() {
            ui.ctx().copy_text(text);
        }
    }
}

/// A small copy button over the message's top-right corner, while it's hovered.
fn copy_overlay(ui: &mut egui::Ui, rect: Rect, id: egui::Id, text: String, pal: theme::Palette) {
    if rect.width() <= 0.0 || !ui.rect_contains_pointer(rect) {
        return;
    }
    let button = Rect::from_min_size(egui::pos2(rect.right() - 18.0, rect.top() + 2.0), egui::vec2(16.0, 16.0));
    let resp = ui.interact(button, id, Sense::click());
    ui.painter().rect_filled(button, 3.0, pal.editor_bg);
    // A quick press animation: the icon dips and springs back.
    let anim = ui.ctx().animate_bool_with_time(id.with("pressed"), resp.is_pointer_button_down_on(), 0.12);
    let scale = 1.0 - 0.18 * anim;
    let icon = egui::Rect::from_center_size(button.center(), egui::vec2(11.0, 11.0) * scale);
    crate::ui::icons::paint(ui.painter(), icon, crate::ui::icons::Icon::Copy, pal.text_dim);
    if resp.clicked() {
        ui.ctx().copy_text(text);
    }
}

/// A flat chip like the mockup's tool-select: label, hover fill, optional caret.
fn chip(
    ui: &mut egui::Ui,
    pal: theme::Palette,
    label: &str,
    caret: bool,
    icon: Option<crate::ui::icons::Icon>,
) -> egui::Response {
    let width = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(10.5), pal.text).size().x
        + f32::from(caret) * 9.0
        + if icon.is_some() { 12.0 } else { 0.0 }
        + 6.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 21.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, pal.hover_bg);
    }
    let mut x = rect.left() + 3.0;
    if let Some(icon) = icon {
        crate::ui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 5.0), egui::vec2(10.0, 10.0)),
            icon,
            pal.text_dim,
        );
        x += 12.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(10.5), pal.text);
    if caret {
        crate::ui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::pos2(rect.right() - 13.0, rect.center().y - 4.0), egui::vec2(8.0, 8.0)),
            crate::ui::icons::Icon::ChevronDown,
            pal.text_dim,
        );
    }
    resp
}

/// The 1px divider between chips.
fn chip_divider(ui: &mut egui::Ui, pal: theme::Palette) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 15.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, pal.border);
}

/// A copy of `text` that fits `max` pixels, with an ellipsis — for texts drawn
/// straight onto a painter, which never clip or wrap.
fn fit_text(ui: &egui::Ui, text: &str, font: FontId, color: Color32, max: f32) -> String {
    if max <= 0.0 {
        return String::new();
    }
    if ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color).size().x <= max {
        return text.to_owned();
    }
    let mut end = text.len();
    while end > 0 {
        // Step back to the previous character boundary.
        let mut start = end - 1;
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        end = start;
        let candidate = format!("{}…", &text[..end]);
        if ui.painter().layout_no_wrap(candidate.clone(), font.clone(), color).size().x <= max {
            return candidate;
        }
    }
    "…".to_owned()
}

/// The `@token` at the end of the ask box, if any: (start, token).
fn at_token(text: &str) -> Option<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'@' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let rest = &text[i + 1..];
            if rest.contains(char::is_whitespace) {
                return None;
            }
            return Some((i, rest));
        }
    }
    None
}

/// The folder's entries matching the `@token`: its directory and prefix.
fn at_entries(root: Option<&Path>, token: &str) -> Vec<String> {
    let Some(root) = root else { return Vec::new() };
    let (dir, prefix) = token.rsplit_once('/').map_or(("", token), |(dir, prefix)| (dir, prefix));
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(root.join(dir)) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(prefix) {
                entries.push(name);
            }
        }
    }
    entries.sort();
    entries
}

/// Mixes two colors, for the composer's focus border.
fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b()))
}

fn mono_text(ui: &mut egui::Ui, text: &str, color: Color32, size: f32) {
    ui.add(egui::Label::new(RichText::new(text).monospace().size(size).color(color)).wrap());
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn markdown(ui: &mut egui::Ui, text: &str, pal: theme::Palette) {
    use coiman_core::language::markdown::Block;
    for block in coiman_core::language::markdown::parse(text) {
        match block {
            Block::Heading { level, text, .. } => {
                let size = 15.0 + (3 - level.min(3)) as f32;
                ui.add(egui::Label::new(inline_text(&text, pal, size)).wrap());
                ui.add_space(2.0);
            }
            Block::Paragraph(text) => {
                ui.add(egui::Label::new(inline_text(&text, pal, 13.0)).wrap());
                ui.add_space(2.0);
            }
            Block::Code { lang, text } => {
                code_block(ui, &text, pal, &lang);
                ui.add_space(2.0);
            }
            Block::List { items, .. } => {
                for item in items {
                    ui.horizontal(|ui| {
                        ui.label("•");
                        ui.vertical(|ui| {
                            for block in item.blocks {
                                match block {
                                    Block::Paragraph(text) => {
                                        ui.add(egui::Label::new(inline_text(&text, pal, 13.0)).wrap());
                                    }
                                    Block::Code { lang, text } => code_block(ui, &text, pal, &lang),
                                    _ => {}
                                }
                            }
                        });
                    });
                }
            }
            Block::Quote(blocks) => {
                for block in blocks {
                    if let Block::Paragraph(text) = block {
                        ui.add(egui::Label::new(inline_text(&text, pal, 13.0)).wrap());
                    }
                }
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Table { header, rows, .. } => {
                for row in header.iter().chain(rows.iter().flatten()) {
                    ui.label(RichText::new(plain_inline(row)).monospace().small());
                }
            }
        }
    }
}

fn inline_text(inlines: &[Inline], pal: theme::Palette, size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for inline in inlines {
        match inline {
            Inline::Text(text, style) => {
                let font = if style.code { FontId::monospace(size) } else { FontId::proportional(size) };
                // Links and file references take the accent, like the mockup's file links.
                let color =
                    if style.link.is_some() || (style.code && is_file_like(text)) { pal.accent } else { pal.text };
                job.append(
                    text,
                    0.0,
                    TextFormat {
                        font_id: font,
                        color,
                        italics: style.italic,
                        strikethrough: if style.strike { egui::Stroke::new(1.0, color) } else { egui::Stroke::NONE },
                        ..Default::default()
                    },
                );
            }
            Inline::Image { alt, .. } => {
                job.append(
                    &format!("[{alt}]"),
                    0.0,
                    TextFormat {
                        font_id: FontId::proportional(size),
                        color: pal.text_dim,
                        italics: true,
                        ..Default::default()
                    },
                );
            }
            Inline::Break => job.append(
                "\n",
                0.0,
                TextFormat { font_id: FontId::proportional(size), color: pal.text, ..Default::default() },
            ),
        }
    }
    job
}

fn plain_inline(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text, _) => out.push_str(text),
            Inline::Image { alt, .. } => out.push_str(&format!("[{alt}]")),
            Inline::Break => out.push('\n'),
        }
    }
    out
}

fn parse_event(line: &str) -> Option<AgentMsg> {
    let value: Value = serde_json::from_str(line).ok()?;
    let kind = value.get("type").and_then(Value::as_str)?;
    let part = value.get("part");
    let part_type = part.and_then(|p| p.get("type")).and_then(Value::as_str);
    let text =
        part.and_then(|p| p.get("text")).and_then(Value::as_str).or_else(|| value.get("text").and_then(Value::as_str));
    match kind {
        "text" if part_type != Some("reasoning") => Some(AgentMsg::Text(text?.to_owned())),
        "reasoning" => Some(AgentMsg::Thinking(text?.to_owned())),
        "tool_use" => {
            let part = part?;
            let id = part.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
            let name = part.get("tool").and_then(Value::as_str).unwrap_or("tool").to_owned();
            let state = part.get("state")?;
            let title = state.get("title").and_then(Value::as_str).unwrap_or(&name).to_owned();
            let input = state.get("input").map(json_text).unwrap_or_default();
            let output = state.get("output").map(json_text).unwrap_or_default();
            let status = state.get("status").and_then(Value::as_str).unwrap_or("running").to_owned();
            Some(AgentMsg::Tool(ToolPart { id, name, title, input, output, status }))
        }
        "error" => {
            // One clean line, not the raw JSON: the provider errors carry a
            // message like "Model unavailable: …".
            let error = value.get("error").or_else(|| part.and_then(|p| p.get("error")));
            let message = error
                .and_then(|error| error.get("message").or_else(|| error.get("error")))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| error.map(json_text))
                .unwrap_or_else(|| "unknown error".to_owned());
            Some(AgentMsg::Text(format!("Error: {message}")))
        }
        _ if part_type == Some("reasoning") => Some(AgentMsg::Thinking(text?.to_owned())),
        _ => None,
    }
}

fn json_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

/// The transcript of a past session, from `opencode session export`.
fn parse_export(value: &Option<Value>) -> Vec<Message> {
    let Some(items) = value.as_ref().and_then(|value| value.get("messages")).and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut messages: Vec<Message> = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("user") => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                if !text.is_empty() {
                    messages.push(Message { role: Role::User, parts: vec![Part::Text(text.to_owned())] });
                }
            }
            Some("assistant") => {
                let mut parts: Vec<Part> = Vec::new();
                for part in item.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                            if !text.is_empty() {
                                parts.push(Part::Text(text.to_owned()));
                            }
                        }
                        Some("reasoning") => {
                            let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                            if !text.is_empty() {
                                parts.push(Part::Thinking(text.to_owned()));
                            }
                        }
                        Some("tool") => {
                            let state = part.get("state");
                            let name = part
                                .get("name")
                                .and_then(Value::as_str)
                                .or_else(|| part.get("tool").and_then(Value::as_str))
                                .unwrap_or("tool")
                                .to_owned();
                            let title = state
                                .and_then(|state| state.get("title"))
                                .and_then(Value::as_str)
                                .unwrap_or(&name)
                                .to_owned();
                            let input = state.and_then(|state| state.get("input")).map(json_text).unwrap_or_default();
                            let output = state
                                .and_then(|state| state.get("content"))
                                .and_then(Value::as_array)
                                .map(|blocks| {
                                    blocks
                                        .iter()
                                        .filter_map(|block| block.get("text").and_then(Value::as_str))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                })
                                .unwrap_or_default();
                            let status = state
                                .and_then(|state| state.get("status"))
                                .and_then(Value::as_str)
                                .unwrap_or("completed")
                                .to_owned();
                            let id = part.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
                            parts.push(Part::Tool(ToolPart { id, name, title, input, output, status }));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    // The steps of one turn arrive as separate assistant messages; show them
                    // as a single reply, like the live transcript does.
                    if let Some(Message { role: Role::Agent, parts: existing }) =
                        messages.last_mut().filter(|message| message.role == Role::Agent)
                    {
                        existing.append(&mut parts);
                    } else {
                        messages.push(Message { role: Role::Agent, parts });
                    }
                }
            }
            Some("idle") if item.get("outcome").and_then(Value::as_str) == Some("failed") => {
                // A run that failed: show its error, like the live view does.
                let error = item.get("error");
                let message = error
                    .and_then(|error| error.get("message").or_else(|| error.get("error")))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| error.map(json_text))
                    .unwrap_or_else(|| "unknown error".to_owned());
                let part = Part::Text(format!("Error: {message}"));
                if let Some(Message { role: Role::Agent, parts }) =
                    messages.last_mut().filter(|message| message.role == Role::Agent)
                {
                    parts.push(part);
                } else {
                    messages.push(Message { role: Role::Agent, parts: vec![part] });
                }
            }
            _ => {}
        }
    }
    messages
}

/// A short label for when a session was last used: the time today, else the date.
fn time_label(updated: u64) -> String {
    let date = local_date(updated);
    if date.date_naive() == chrono::Local::now().date_naive() {
        date.format("%H:%M").to_string()
    } else {
        date.format("%b %-d").to_string()
    }
}

/// How long ago a session was used: now, minutes, hours, days, else the date.
fn relative_time(updated: u64) -> String {
    let date = local_date(updated);
    let minutes = (chrono::Local::now() - date).num_minutes().max(0);
    if minutes < 1 {
        "now".to_owned()
    } else if minutes < 60 {
        format!("{minutes}m")
    } else if minutes < 60 * 24 {
        format!("{}h", minutes / 60)
    } else if minutes < 60 * 24 * 7 {
        format!("{}d", minutes / (60 * 24))
    } else {
        date.format("%b %-d").to_string()
    }
}

/// A message's plain text: its text parts, without thinking or tools.
fn message_text(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The whole conversation as markdown, for "Copy conversation".
fn transcript_markdown(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        match message.role {
            Role::User => out.push_str("## You\n"),
            Role::Agent => out.push_str("## opencode\n"),
        }
        let text = message_text(message);
        if !text.is_empty() {
            out.push_str(&text);
            out.push('\n');
        }
        for part in &message.parts {
            if let Part::Tool(tool) = part {
                out.push_str(&format!("- {}/{}\n", tool.name, tool.title));
            }
        }
        out.push('\n');
    }
    out
}

/// Files edited in the last turn, from the file-editing tools' inputs.
fn files_touched(messages: &[Message]) -> Vec<String> {
    let start = messages.iter().rposition(|message| message.role == Role::User).map_or(0, |index| index + 1);
    let mut files: Vec<String> = Vec::new();
    for message in &messages[start.min(messages.len())..] {
        for part in &message.parts {
            if let Part::Tool(tool) = part {
                if !is_edit_tool(&tool.name) || tool.status == "error" {
                    continue;
                }
                if let Some(path) = path_from_input(&tool.input)
                    && !files.contains(&path)
                {
                    files.push(path);
                }
            }
        }
    }
    files.truncate(20);
    files
}

/// How many tools the last turn used.
fn tools_in_last_turn(messages: &[Message]) -> usize {
    let start = messages.iter().rposition(|message| message.role == Role::User).map_or(0, |index| index + 1);
    messages[start.min(messages.len())..]
        .iter()
        .flat_map(|message| &message.parts)
        .filter(|part| matches!(part, Part::Tool(_)))
        .count()
}

fn is_edit_tool(name: &str) -> bool {
    matches!(
        name,
        "edit"
            | "write"
            | "str_replace"
            | "str_replace_editor"
            | "multiedit"
            | "multi_edit"
            | "apply_patch"
            | "applypatch"
            | "notebookedit"
            | "notebook_edit"
            | "create_file"
            | "create-file"
            | "save_file"
            | "patch_file"
    )
}

/// The first file path in a tool call's JSON input.
fn path_from_input(input: &str) -> Option<String> {
    let value: Value = serde_json::from_str(input).ok()?;
    fn walk(value: &Value, key: Option<&str>, out: &mut Option<String>) {
        if out.is_some() {
            return;
        }
        match value {
            Value::Object(map) => {
                for (name, item) in map {
                    walk(item, Some(name), out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, key, out);
                }
            }
            Value::String(text)
                if matches!(
                    key,
                    Some("file_path" | "filePath" | "path" | "file" | "filename" | "notebook_path" | "notebookPath")
                ) && is_file_like(text) =>
            {
                *out = Some(text.clone());
            }
            _ => {}
        }
    }
    let mut found = None;
    walk(&value, None, &mut found);
    found.or_else(|| {
        let mut strings = Vec::new();
        collect_strings(&value, &mut strings);
        strings.into_iter().find(|text| is_file_like(text))
    })
}

/// Collects every string in a JSON value, depth first.
fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// Whether a token looks like a file path: no spaces, with a directory or extension.
fn is_file_like(text: &str) -> bool {
    let trimmed = text.trim().trim_start_matches('@').trim_matches('`').trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) || trimmed.len() > 240 {
        return false;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return true;
    }
    // An extension at the end, like `README.md` but not `1.5`.
    match trimmed.rsplit_once('.') {
        Some((base, extension)) => {
            !base.is_empty()
                && base.chars().any(|c| c.is_ascii_alphabetic())
                && !extension.is_empty()
                && extension.len() <= 6
                && extension.chars().all(|c| c.is_ascii_alphanumeric())
                && extension.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

fn local_date(updated: u64) -> chrono::DateTime<chrono::Local> {
    let time = UNIX_EPOCH + std::time::Duration::from_millis(updated);
    chrono::DateTime::<chrono::Local>::from(time)
}

impl CodeApp {
    /// Prepares the Agents panel: the current folder and the model list.
    pub(crate) fn agents_prepare(&mut self) {
        if !self.settings.workbench.agents_enabled {
            self.agents.open = false;
            return;
        }
        self.agents.root = if self.file_manager.is_showing() {
            self.file_manager.current_dir()
        } else {
            self.explorer
                .root
                .clone()
                .or_else(|| self.docs.get(self.active).and_then(|d| d.path.as_deref()?.parent()).map(Path::to_path_buf))
        };
        self.agents.load_model();
        self.agents.load_models();
        self.agents.active_file = self.docs.get(self.active).and_then(|doc| doc.path.clone());
    }
}

#[cfg(test)]
// The tests build panels in many states, field by field.
#[allow(clippy::field_reassign_with_default, clippy::type_complexity)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_thinking_and_tools() {
        let text = r#"{"type":"text","part":{"type":"text","text":"hello"}}"#;
        assert!(matches!(parse_event(text), Some(AgentMsg::Text(t)) if t == "hello"));

        let thinking = r#"{"type":"reasoning","part":{"type":"reasoning","text":"hmm"}}"#;
        assert!(matches!(parse_event(thinking), Some(AgentMsg::Thinking(t)) if t == "hmm"));

        let tool = r#"{"type":"tool_use","part":{"type":"tool","id":"call_1","tool":"read","state":{"status":"completed","title":"read","input":{"path":"."},"output":"ok"}}}"#;
        match parse_event(tool) {
            Some(AgentMsg::Tool(tool)) => {
                assert_eq!(tool.name, "read");
                assert_eq!(tool.status, "completed");
                assert!(tool.input.contains("path"));
                assert_eq!(tool.output, "ok");
            }
            _ => panic!("tool event not parsed"),
        }
    }

    #[test]
    fn parses_an_exported_session() {
        // The shape of `opencode session export`: one user message, then the steps of one
        // assistant turn (reasoning, a tool call and the answer) as separate messages.
        let export = r#"{
            "info": {"id": "ses_1", "title": "Test"},
            "messages": [
                {"type": "user", "text": "What folder are we in?", "files": []},
                {"type": "assistant", "content": [
                    {"type": "reasoning", "text": "Let me check."},
                    {"type": "tool", "id": "call_1", "name": "shell", "state": {
                        "status": "completed",
                        "input": {"command": "pwd"},
                        "content": [{"type": "text", "text": "/home/me\n"}]
                    }}
                ]},
                {"type": "assistant", "content": [
                    {"type": "text", "text": "We're in `/home/me`."}
                ]},
                {"type": "idle", "outcome": "succeeded"}
            ]
        }"#;
        let value = serde_json::from_str::<Value>(export).ok();
        let messages = parse_export(&value);
        assert_eq!(messages.len(), 2);
        assert!(matches!(messages[0].parts.as_slice(), [Part::Text(text)] if text == "What folder are we in?"));
        assert!(
            matches!(messages[1].parts.as_slice(), [Part::Thinking(_), Part::Tool(tool), Part::Text(_)] if tool.name == "shell" && tool.output == "/home/me\n")
        );
    }

    #[test]
    fn empty_export_gives_no_messages() {
        assert!(parse_export(&None).is_empty());
        assert!(parse_export(&serde_json::from_str::<Value>("{}").ok()).is_empty());
    }

    #[test]
    fn working_set_uses_the_last_turns_edit_tools() {
        let edit = |path: &str, status: &str| {
            Part::Tool(ToolPart {
                id: "call".to_owned(),
                name: "edit".to_owned(),
                title: "edit".to_owned(),
                input: serde_json::to_string(&serde_json::json!({ "file_path": path })).unwrap(),
                output: String::new(),
                status: status.to_owned(),
            })
        };
        let read = Part::Tool(ToolPart {
            id: "call".to_owned(),
            name: "read".to_owned(),
            title: "read".to_owned(),
            input: serde_json::to_string(&serde_json::json!({ "path": "src/lib.rs" })).unwrap(),
            output: String::new(),
            status: "completed".to_owned(),
        });
        let messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("first".to_owned())] },
            Message { role: Role::Agent, parts: vec![read.clone(), edit("old.rs", "completed")] },
            Message { role: Role::User, parts: vec![Part::Text("second".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![edit("a.rs", "completed"), edit("b.rs", "completed"), edit("failed.rs", "error"), read],
            },
        ];
        // Only the last turn counts, and only its successful file edits.
        assert_eq!(files_touched(&messages), vec!["a.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(tools_in_last_turn(&messages), 4);
        assert_eq!(message_text(&messages[0]), "first");
    }

    #[test]
    fn paths_come_from_tool_inputs() {
        let json =
            serde_json::to_string(&serde_json::json!({ "command": "cat src/main.rs", "file_path": "src/app.rs" }))
                .unwrap();
        assert_eq!(path_from_input(&json).as_deref(), Some("src/app.rs"));

        let nested = serde_json::to_string(&serde_json::json!({ "edits": [{ "path": "lib/main.rs" }] })).unwrap();
        assert_eq!(path_from_input(&nested).as_deref(), Some("lib/main.rs"));

        let prose = serde_json::to_string(&serde_json::json!({ "command": "say hello" })).unwrap();
        assert_eq!(path_from_input(&prose), None);
    }

    #[test]
    fn file_like_tokens() {
        assert!(is_file_like("src/app.rs"));
        assert!(is_file_like("@README.md"));
        assert!(is_file_like("`Cargo.toml`"));
        assert!(is_file_like("main.c"));
        assert!(!is_file_like("hello world"));
        assert!(!is_file_like("1.5"));
        assert!(!is_file_like("3.14"));
    }

    #[test]
    fn relative_time_labels() {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        assert_eq!(relative_time(now), "now");
        assert_eq!(relative_time(now - 90 * 1000), "1m");
        assert_eq!(relative_time(now - 3 * 3600 * 1000), "3h");
    }

    // ---- The composer's input handling, driven headlessly ----------------

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        let modifiers = egui::Modifiers::default();
        vec![
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers },
        ]
    }

    /// Runs one composer frame and returns the box's rect.
    fn composer_frame(
        ctx: &egui::Context,
        agents: &mut AgentsUi,
        running: bool,
        modifiers: egui::Modifiers,
        mut events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(420.0, 320.0))),
            ..Default::default()
        };
        // egui derives the frame's modifiers from this event, like a real window.
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        raw.events = events;
        let mut box_rect = egui::Rect::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            box_rect = agents.composer(ui, theme::colors(), running);
        });
        // Headless: clear the font texture deltas instead of dropping them.
        output.textures_delta.clear();
        box_rect
    }

    #[test]
    fn composer_typing_needs_focus_and_enter_sends() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        let none = egui::Modifiers::default();

        let box_rect = composer_frame(&ctx, &mut agents, false, none, vec![]);
        assert!(!agents.input_focused, "the box starts unfocused");

        // Typing without focus goes nowhere.
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("hi".to_owned())]);
        assert_eq!(agents.input, "", "typing without focus must not land");
        // Enter without focus sends nothing.
        composer_frame(&ctx, &mut agents, false, none, vec![key_event(egui::Key::Enter, none)]);
        assert!(agents.queue.is_empty() && agents.input.is_empty());

        // Click the box: it takes focus.
        let at = egui::pos2(box_rect.center().x, box_rect.top() + 16.0);
        composer_frame(&ctx, &mut agents, false, none, click_events(at));
        assert!(agents.input_focused, "clicking the box should focus it");

        // Now typing lands.
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("hello".to_owned())]);
        assert_eq!(agents.input, "hello");

        // Shift+Enter is a newline, not a send.
        composer_frame(
            &ctx,
            &mut agents,
            false,
            egui::Modifiers::SHIFT,
            vec![key_event(egui::Key::Enter, egui::Modifiers::SHIFT)],
        );
        assert_eq!(agents.input, "hello\n", "shift-enter is a newline");
        assert!(agents.queue.is_empty());

        // Enter queues while the agent runs (no process is spawned).
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::Enter, none)]);
        assert_eq!(agents.queue, vec!["hello".to_owned()]);
        assert_eq!(agents.input, "", "enter clears the box");

        // The queue row appearing above the box must not steal focus: typing
        // still lands (this is the id-salt fix).
        composer_frame(&ctx, &mut agents, true, none, vec![egui::Event::Text("more".to_owned())]);
        assert_eq!(agents.input, "more", "focus must survive the queue row appearing");

        // Enter queues the second message too.
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::Enter, none)]);
        agents.prompt_history.push("first prompt".to_owned());
        // Up arrow in an empty box recalls the last prompt.
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::ArrowUp, none)]);
        assert_eq!(agents.input, "first prompt", "up arrow recalls history");

        // /clear clears the conversation without running anything.
        agents.messages = vec![Message { role: Role::Agent, parts: vec![Part::Text("old".to_owned())] }];
        agents.input.clear();
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("/clear".to_owned())]);
        composer_frame(&ctx, &mut agents, false, none, vec![key_event(egui::Key::Enter, none)]);
        assert!(agents.messages.is_empty(), "/clear empties the conversation");
        assert_eq!(agents.status.as_deref(), Some("New session"));
    }

    #[test]
    fn focus_request_lands_in_the_box() {
        let ctx = egui::Context::default();
        // toggle() / focus_chat() / attach() set this flag.
        let mut agents = AgentsUi { focus_input: true, ..Default::default() };
        composer_frame(&ctx, &mut agents, false, egui::Modifiers::default(), vec![]);
        assert!(agents.input_focused, "focus_input focuses the box");
    }

    fn panel_frame(ctx: &egui::Context, agents: &mut AgentsUi) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(520.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| agents.panel(ui));
        output.textures_delta.clear();
    }

    /// Runs the panel like `mod.rs` does — inside a right `Panel` — and returns
    /// the panel's rect, so the width ratchet can be measured.
    fn panel_with_chrome(ctx: &egui::Context, agents: &mut AgentsUi, screen: egui::Vec2) -> egui::Rect {
        // As if opencode were installed (it isn't on CI): otherwise the panel only says so.
        agents.checked_opencode = true;
        agents.opencode.get_or_insert_with(|| PathBuf::from("opencode"));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
            ..Default::default()
        };
        let mut rect = egui::Rect::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                rect = egui::Panel::right(egui::Id::unique("agents_panel_test"))
                    .default_size(430.0)
                    .size_range(360.0..=980.0)
                    .resizable(true)
                    .show_separator_line(true)
                    .frame(egui::Frame::new().fill(theme::colors().sidebar_bg).inner_margin(egui::Margin::ZERO))
                    .show(ui, |ui| agents.panel(ui))
                    .response
                    .rect;
            });
        });
        output.textures_delta.clear();
        rect
    }

    #[test]
    fn panel_width_stays_put_when_sessions_open() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
            Message { role: Role::Agent, parts: vec![Part::Text("Sure.".to_owned())] },
        ];
        // Closed: the panel keeps its default width.
        let closed = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        // Opening the sessions column must not inflate the panel.
        agents.history_open = true;
        agents.history =
            vec![SessionInfo { id: "ses_1".to_owned(), title: "A session".to_owned(), updated: 1, directory: None }];
        let mut open = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        for _ in 0..8 {
            open = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        }
        assert!(closed.width() <= 460.0, "closed panel stays ~430: {:.1}", closed.width());
        assert!(open.width() <= 470.0, "sessions column must not inflate the panel: {:.1}", open.width());
    }

    #[test]
    fn panel_width_is_stable_in_every_state() {
        let ctx = egui::Context::default();
        // Each state is rendered repeatedly: the panel's width must not drift.
        let states: Vec<(&str, Box<dyn Fn(&mut AgentsUi)>)> = vec![
            ("plain", Box::new(|_| {})),
            (
                "sessions",
                Box::new(|a: &mut AgentsUi| {
                    a.history_open = true;
                    a.history = vec![
                        SessionInfo { id: "ses_1".to_owned(), title: "One".to_owned(), updated: 3, directory: None },
                        SessionInfo { id: "ses_2".to_owned(), title: "Two".to_owned(), updated: 2, directory: None },
                    ];
                    a.session_search_open = true;
                }),
            ),
            (
                "working set",
                Box::new(|a: &mut AgentsUi| {
                    a.last_changed_files = vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned()];
                    a.working_set_open = true;
                }),
            ),
            (
                "attached and queued",
                Box::new(|a: &mut AgentsUi| {
                    a.attached = vec!["/home/mostafa/src/main.rs".to_owned()];
                    a.queue = vec!["a queued message that is fairly long".to_owned()];
                }),
            ),
            (
                "running",
                Box::new(|a: &mut AgentsUi| {
                    let (_tx, rx) = mpsc::channel();
                    a.running = Some(rx);
                    a.queue = vec!["a queued message".to_owned()];
                    a.last_changed_files = vec!["src/main.rs".to_owned()];
                }),
            ),
            (
                "searching sessions",
                Box::new(|a: &mut AgentsUi| {
                    a.history_open = true;
                    a.history = vec![
                        SessionInfo { id: "ses_1".to_owned(), title: "One".to_owned(), updated: 3, directory: None },
                        SessionInfo { id: "ses_2".to_owned(), title: "Two".to_owned(), updated: 2, directory: None },
                    ];
                    a.session_search_open = true;
                    a.session_search = "on".to_owned();
                }),
            ),
        ];
        for (name, setup) in &states {
            let mut agents = AgentsUi::default();
            agents.messages = vec![
                Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
                Message { role: Role::Agent, parts: vec![Part::Text("Working on it.".to_owned())] },
            ];
            setup(&mut agents);
            // The panel sizes itself (default 430 or the content's min); what
            // must not happen is drift between frames.
            let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
            for _ in 0..5 {
                rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
            }
            let first = rect.width();
            let mut drift = 0.0_f32;
            for _ in 0..10 {
                let next = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
                drift = drift.max((next.width() - rect.width()).abs());
                rect = next;
            }
            assert!(drift <= 1.5, "{name}: the panel jitters frame to frame ({drift:.1})");
            assert!(
                (rect.width() - first).abs() <= 1.5,
                "{name}: the panel creeps over time ({:.1} -> {:.1})",
                first,
                rect.width()
            );
        }
        // A narrow window: the panel gives way but must not jitter.
        for (name, setup) in &states {
            let mut agents = AgentsUi::default();
            agents.messages = vec![
                Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
                Message { role: Role::Agent, parts: vec![Part::Text("Working on it.".to_owned())] },
            ];
            setup(&mut agents);
            let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
            for _ in 0..5 {
                rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
            }
            let first = rect.width();
            for _ in 0..10 {
                let next = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
                assert!(
                    (next.width() - rect.width()).abs() <= 1.5,
                    "{name} narrow: the panel jitters ({:.1} -> {:.1})",
                    rect.width(),
                    next.width()
                );
                rect = next;
            }
            assert!(
                (rect.width() - first).abs() <= 1.5,
                "{name} narrow: the panel creeps ({:.1} -> {:.1})",
                first,
                rect.width()
            );
            if !agents.history_open {
                let composer = agents.composer_rect.expect("the composer was drawn");
                assert!(
                    (composer.bottom() - rect.bottom()).abs() < 30.0,
                    "{name} narrow: the composer stays at the bottom ({:?} vs {:?})",
                    composer,
                    rect
                );
            }
        }
    }

    #[test]
    fn stop_clears_the_queue() {
        let mut agents = AgentsUi { queue: vec!["a queued message".to_owned()], ..Default::default() };
        // A pid that does not exist: stop() must not touch a real process.
        agents.child_pid = Some(999_999);
        agents.stop();
        assert!(agents.queue.is_empty(), "a stopped run drops its queue");
        assert_eq!(agents.status.as_deref(), Some("Stopped"));
    }

    #[test]
    fn run_end_drains_one_queued_message() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        // No OpenCode program: send() returns early, so no process is spawned.
        agents.checked_opencode = true;
        agents.opencode = None;
        let (tx, rx) = mpsc::channel();
        tx.send(AgentMsg::End(Some(0))).unwrap();
        agents.running = Some(rx);
        agents.queue = vec!["one".to_owned(), "two".to_owned()];
        agents.poll(&ctx);
        assert_eq!(agents.input, "one", "the first queued message moves to the box");
        assert_eq!(agents.queue.len(), 1, "the rest wait for the next run");
        assert!(agents.running.is_none(), "the run ended");
    }

    #[test]
    fn resumed_session_loads_through_poll() {
        let mut agents = AgentsUi {
            messages: vec![Message { role: Role::Agent, parts: vec![Part::Text("old".to_owned())] }],
            ..Default::default()
        };
        agents.session_loading = true;
        let (tx, rx) = mpsc::channel();
        tx.send(AgentMsg::Session(vec![Message { role: Role::User, parts: vec![Part::Text("resumed".to_owned())] }]))
            .unwrap();
        agents.running_session = Some(rx);
        agents.poll_session();
        assert_eq!(agents.messages.len(), 1);
        assert_eq!(message_text(&agents.messages[0]), "resumed");
        assert!(!agents.session_loading);
        assert!(agents.focus_input, "the box regains focus after a resume");
    }

    #[test]
    fn fit_text_truncates_to_the_width() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0))),
            ..Default::default()
        };
        let mut result = String::new();
        let mut output = ctx.run_ui(raw, |ui| {
            result = fit_text(
                ui,
                "a very long title that will not fit in a small space at all",
                FontId::proportional(12.0),
                Color32::WHITE,
                100.0,
            );
        });
        output.textures_delta.clear();
        assert!(result.ends_with('…'), "long text is ellipsized: {result:?}");
        assert!(result.len() < 40, "the fit is short: {result:?}");
    }

    #[test]
    fn error_events_are_one_line() {
        let line =
            r#"{"type":"error","error":{"type":"provider.no-route","message":"Model unavailable: kilo/claude"}}"#;
        match parse_event(line) {
            Some(AgentMsg::Text(text)) => {
                assert_eq!(text, "Error: Model unavailable: kilo/claude");
                assert!(!text.contains('{'), "no raw JSON: {text:?}");
            }
            _ => panic!("error event not parsed"),
        }
    }

    #[test]
    fn thinking_headers_do_not_clash() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("hi".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![
                    Part::Thinking("first thought".to_owned()),
                    Part::Thinking("second thought".to_owned()),
                    Part::Thinking("third thought".to_owned()),
                ],
            },
        ];
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(520.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| agents.panel(ui));
        output.textures_delta.clear();
        // egui paints an error text for id clashes; none must appear.
        let clash = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => text.galley.text().contains("Widget is above"),
            _ => false,
        });
        assert!(!clash, "thinking headers must have unique ids");
    }

    #[test]
    fn long_code_lines_do_not_widen_the_panel() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        // A 400-char code line: the block must truncate, not widen the panel.
        let long_line = "let x = \"".to_owned() + &"y".repeat(390) + "\";";
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("show me".to_owned())] },
            Message { role: Role::Agent, parts: vec![Part::Text(format!("Here:\n\n```rust\n{long_line}\n```"))] },
        ];
        let rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        assert!(rect.width() <= 470.0, "long code lines must not widen the panel: {:.1}", rect.width());
    }

    #[test]
    fn failed_runs_show_clean_errors() {
        let export = r#"{
          "info": {},
          "messages": [
            {"type": "user", "text": "hi"},
            {"type": "idle", "outcome": "failed", "error": {"type": "provider.no-route", "message": "Model unavailable: kilo/claude-opus-latest"}},
            {"type": "user", "text": "hi again"},
            {"type": "idle", "outcome": "failed", "error": {"type": "provider.no-route", "message": "Model unavailable: kilo/claude-opus-latest"}}
          ]
        }"#;
        let value: Value = serde_json::from_str(export).unwrap();
        let messages = parse_export(&Some(value));
        // Two user prompts and two failed runs; the failures become one-line
        // error texts, not raw JSON blocks.
        assert_eq!(messages.len(), 4);
        let errors: Vec<&str> = messages
            .iter()
            .filter(|m| m.role == Role::Agent)
            .filter_map(|m| {
                m.parts.iter().find_map(|p| match p {
                    Part::Text(t) if t.starts_with("Error:") => Some(t.as_str()),
                    _ => None,
                })
            })
            .collect();
        assert_eq!(errors.len(), 2);
        for error in &errors {
            assert!(error.contains("Model unavailable"), "one-line message: {error:?}");
            assert!(!error.contains('{'), "no raw JSON: {error:?}");
        }
    }

    #[test]
    fn resumed_session_renders_sane() {
        let ctx = egui::Context::default();
        // The shape of `opencode session export`: user texts, assistant content
        // with text/tool parts, idle markers.
        let export = r#"{
          "info": {},
          "messages": [
            {"type": "user", "text": "hi hi"},
            {"type": "assistant", "content": [{"type": "text", "text": "Hi! How can I help you today?"}]},
            {"type": "idle"},
            {"type": "user", "text": "What can you see?"},
            {"type": "assistant", "content": [
                {"type": "tool", "id": "call_1", "name": "shell", "state": {"status": "completed", "input": {"command": "pwd"}, "content": [{"type": "text", "text": "/home/mostafa\n"}]}},
                {"type": "text", "text": "I can see the project files."}
            ]},
            {"type": "user", "text": "Show me the entry point."},
            {"type": "assistant", "content": [{"type": "text", "text": "Here it is:\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```"}]}
          ]
        }"#;
        let value: Value = serde_json::from_str(export).unwrap();
        let messages = parse_export(&Some(value));
        assert_eq!(messages.len(), 6, "idle markers are skipped (3 user + 3 assistant)");
        let mut agents = AgentsUi { messages, ..Default::default() };
        agents.session_title = Some("Greeting".to_owned());
        agents.last_changed_files = files_touched(&agents.messages);
        agents.last_turn_tools = tools_in_last_turn(&agents.messages);
        agents.working_set_open = true;
        let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        for _ in 0..6 {
            rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        }
        assert!(rect.width() <= 470.0, "a resumed session must not inflate the panel: {:.1}", rect.width());
        let composer = agents.composer_rect.expect("the composer was drawn");
        assert!(
            (composer.bottom() - rect.bottom()).abs() < 30.0,
            "the composer stays at the bottom: composer={:?} panel={:?}",
            composer,
            rect
        );
    }

    #[test]
    fn panel_renders_every_section() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![
                    Part::Thinking("hmm".to_owned()),
                    Part::Text("Sure, here is the plan:\n\n```rust\nfn main() {}\n```".to_owned()),
                    Part::Tool(ToolPart {
                        id: "call_1".to_owned(),
                        name: "edit".to_owned(),
                        title: "edit".to_owned(),
                        input: r#"{"file_path": "src/main.rs"}"#.to_owned(),
                        output: "ok".to_owned(),
                        status: "completed".to_owned(),
                    }),
                ],
            },
        ];
        agents.last_changed_files = files_touched(&agents.messages);
        agents.last_turn_tools = tools_in_last_turn(&agents.messages);
        agents.history = vec![
            SessionInfo { id: "ses_1".to_owned(), title: "A session".to_owned(), updated: 1, directory: None },
            SessionInfo { id: "ses_2".to_owned(), title: "Another".to_owned(), updated: 2, directory: None },
        ];
        agents.history_open = true;
        agents.working_set_open = true;
        agents.input = "a draft".to_owned();
        // Every section at once: sessions column, working set, tool cards,
        // code block, delete modal, jump pill.
        agents.delete_pending = Some(agents.history[0].clone());
        panel_frame(&ctx, &mut agents);
        // And with a run in flight (spinner, stop button, queue row).
        agents.delete_pending = None;
        agents.queue.push("queued".to_owned());
        let (_tx, rx) = mpsc::channel();
        agents.running = Some(rx);
        panel_frame(&ctx, &mut agents);
        agents.running = None;

        // Narrow windows must not panic (the negative-size crash class).
        let mut narrow = AgentsUi { history_open: true, ..Default::default() };
        narrow.messages = agents.messages.clone();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(240.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| narrow.panel(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn session_search_matches_titles_and_ids() {
        let mut agents = AgentsUi::default();
        agents.history = vec![
            SessionInfo { id: "ses_alpha".to_owned(), title: "Alpha plan".to_owned(), updated: 1, directory: None },
            SessionInfo { id: "ses_beta".to_owned(), title: "Beta work".to_owned(), updated: 2, directory: None },
        ];
        agents.session_search = "alp".to_owned();
        let matched = agents.matching_sessions();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "ses_alpha");
        agents.session_search = "beta".to_owned();
        assert_eq!(agents.matching_sessions().len(), 1, "ids match too");
        agents.session_search.clear();
        assert_eq!(agents.matching_sessions().len(), 2, "an empty query keeps all");
    }

    #[test]
    fn model_search_filters_the_list() {
        let mut agents = AgentsUi {
            models: vec!["google/gemini-3.5-flash".to_owned(), "kilo/claude".to_owned()],
            ..Default::default()
        };
        agents.model_search = "gem".to_owned();
        assert_eq!(agents.matching_models(), vec!["google/gemini-3.5-flash".to_owned()]);
        agents.model_search.clear();
        assert_eq!(agents.matching_models().len(), 2);
    }

    #[test]
    fn at_token_finds_the_reference() {
        assert_eq!(at_token("hello @"), Some((6, "")));
        assert_eq!(at_token("@src/main"), Some((0, "src/main")));
        assert_eq!(at_token("ask @src/ma"), Some((4, "src/ma")));
        assert_eq!(at_token("no reference"), None);
        assert_eq!(at_token("email@example.com"), None, "not a token mid-word");
        assert_eq!(at_token("done @file and more"), None, "space after the token ends it");
    }

    #[test]
    fn at_entries_match_the_prefix() {
        let dir = std::env::temp_dir().join("coiman_at_entries_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "x").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "x").unwrap();
        std::fs::write(dir.join("README.md"), "x").unwrap();
        let entries = at_entries(Some(&dir), "src/ma");
        assert_eq!(entries, vec!["main.rs".to_owned()]);
        let root_entries = at_entries(Some(&dir), "");
        assert!(root_entries.contains(&"README.md".to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attached_context_prefixes_the_prompt() {
        let mut agents =
            AgentsUi { attached: vec!["/tmp/a.rs".to_owned(), "/tmp/b.rs".to_owned()], ..Default::default() };
        assert_eq!(agents.with_attachments("hi"), "@/tmp/a.rs @/tmp/b.rs\nhi");
        agents.attached.clear();
        assert_eq!(agents.with_attachments("hi"), "hi");
    }
}
