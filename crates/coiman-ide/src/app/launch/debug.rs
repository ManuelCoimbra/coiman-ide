//! The debugger: CodeLLDB through the Debug Adapter Protocol. Breakpoints
//! (with conditions and log messages) live in the gutter; the Debug view
//! shows the call stack, variables (with CodeLLDB's Rust formatters) and
//! watch expressions; the program runs in a terminal tab, and the debug console
//! shows log points and evaluates expressions. Binaries and single tests are
//! built with cargo first.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::build::cargo::{self, FileTarget, TestItem};
use coiman_core::data::hexdata;
use coiman_core::language::lsp::registry;
use coiman_core::launch::breakpoints::{
    self as bpx, DataBreakpoint, DataInfo, ExceptionFilter, FunctionBreakpoint, Process, Support,
};
use coiman_core::launch::dap::{self, Adapter, DapClient, DapEvent, DebugKind};
use coiman_core::launch::run;
use egui::{Align, Color32, RichText, Sense};
use serde_json::{Value, json};

use crate::ui::editor_view::Decorations;
use crate::ui::icons::{self, Icon};
use crate::ui::palette::{InputKind, PickerKind};
use crate::ui::terminal::Task;
use crate::ui::theme;
use crate::watcher::same_path;

use crate::app::{CodeApp, PanelTab, SidebarView, repaint};

/// Debug console lines kept.
const MAX_CONSOLE: usize = 10_000;

#[derive(Clone, Debug)]
pub(crate) struct Breakpoint {
    /// 0-based line.
    line: usize,
    condition: Option<String>,
    log: Option<String>,
    enabled: bool,
    /// Whether the debugger could place it (None: no session yet).
    verified: Option<bool>,
}

/// What to debug.
#[derive(Clone, Debug)]
struct Launch {
    kind: DebugKind,
    name: String,
    program: PathBuf,
    args: Vec<String>,
    cwd: PathBuf,
    /// A running process to attach to, in place of starting `program`.
    attach: Option<u32>,
}

enum AdapterState {
    Unknown,
    Found(Adapter),
    Missing,
    Installing(Receiver<Result<Adapter, String>>),
}

enum Pending {
    Initialize,
    Launch,
    SetBreakpoints(PathBuf),
    SetFunctionBreakpoints,
    SetExceptionBreakpoints,
    SetDataBreakpoints,
    /// Whether a variable can be watched: its description.
    DataInfo(String),
    /// gdb's own watchpoint, for a variable: its description.
    GdbWatch(String),
    ConfigurationDone,
    Threads,
    StackTrace,
    Scopes,
    Variables(i64),
    Watch(usize, u64),
    Repl,
    /// A page of memory, for this stop.
    Memory(u64),
    MemoryAddress,
    Disassembly,
    Other,
}

struct Frame {
    id: i64,
    name: String,
    path: Option<PathBuf>,
    /// 1-based, as the adapter reports it.
    line: usize,
    /// Where the frame is executing, for disassembly.
    pointer: Option<String>,
}

struct Var {
    name: String,
    value: String,
    ty: String,
    reference: i64,
    /// Where it is, if the adapter says.
    memory: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RunState {
    Starting,
    Running,
    Stopped { thread: i64, reason: String },
    Ended,
}

struct Session {
    client: DapClient,
    /// Sessions the current one was started from, kept open while it runs.
    parents: Vec<DapClient>,
    /// What the adapter asked this session to launch, in place of our own.
    asked_launch: Option<(String, Value)>,
    launch: Launch,
    state: RunState,
    threads: Vec<(i64, String)>,
    frames: Vec<Frame>,
    selected: usize,
    scopes: Vec<(String, i64)>,
    /// Scopes read only when asked for.
    lazy_scopes: HashSet<i64>,
    vars: HashMap<i64, Vec<Var>>,
    /// Expanded variables by path (`Local/names/[0]`): references change on
    /// every stop, paths don't, so expansion survives stepping.
    expanded: HashSet<String>,
    watch_values: Vec<Option<String>>,
    pending: HashMap<i64, Pending>,
    /// Bumped on every stop, so late answers from an earlier stop are ignored.
    stop_gen: u64,
    capabilities: hexdata::Capabilities,
    support: Support,
}

/// Results of background work (finding binaries, building, installing).
enum Msg {
    Targets(Result<Vec<run::CargoAction>, String>),
    Built(Result<Launch, String>),
    /// A Python file's session, and whether debugpy is there.
    PythonChecked(Launch, bool),
    DebugpyInstalled(Launch, Result<(), String>),
    /// The processes running here, to attach to.
    Processes(Result<Vec<Process>, String>),
}

pub(crate) struct DebugState {
    /// What the Debug picker is choosing from.
    targets: Vec<run::CargoAction>,
    adapter: AdapterState,
    offer_dismissed: bool,
    /// Breakpoints by file key: (path, breakpoints).
    breakpoints: BTreeMap<PathBuf, (PathBuf, Vec<Breakpoint>)>,
    session: Option<Session>,
    console: Vec<String>,
    console_input: String,
    watches: Vec<String>,
    watch_input: String,
    building: Option<String>,
    last_launch: Option<Launch>,
    /// A launch waiting for the adapter to be installed.
    queued: Option<Launch>,
    /// A Python session waiting for the user's yes to installing debugpy.
    debugpy_offer: Option<Launch>,
    installing_debugpy: bool,
    /// Breakpoint being given a condition or log message: (path, line).
    editing: Option<(PathBuf, usize)>,
    /// Function breakpoints, and whether the debugger placed each.
    functions: Vec<(FunctionBreakpoint, Option<bool>)>,
    function_input: String,
    /// Exception filters the user turned on or off, by id.
    exceptions: HashMap<String, bool>,
    /// The exception filters the last debugger offered.
    filters: Vec<ExceptionFilter>,
    /// Data breakpoints of this session.
    data: Vec<DataBreakpoint>,
    /// Processes listed for attaching, and the one chosen.
    processes: Vec<Process>,
    attach_to: Option<Process>,
    /// Breakpoints of every project, as kept in the session (read when first needed).
    kept: Option<BTreeMap<PathBuf, bpx::Saved>>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for DebugState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            adapter: AdapterState::Unknown,
            offer_dismissed: false,
            breakpoints: BTreeMap::new(),
            session: None,
            console: Vec::new(),
            console_input: String::new(),
            watches: Vec::new(),
            watch_input: String::new(),
            targets: Vec::new(),
            building: None,
            last_launch: None,
            queued: None,
            debugpy_offer: None,
            installing_debugpy: false,
            editing: None,
            functions: Vec::new(),
            function_input: String::new(),
            exceptions: HashMap::new(),
            filters: Vec::new(),
            data: Vec::new(),
            processes: Vec::new(),
            attach_to: None,
            kept: None,
            tx,
            rx,
        }
    }
}

/// A path as an adapter wrote it, as this system writes it: Delve uses forward slashes.
fn native_path(path: &str) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.replace('/', "\\")) } else { PathBuf::from(path) }
}

fn key(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().to_lowercase()) } else { path.to_path_buf() }
}

impl CodeApp {
    fn push_console(&mut self, text: &str) {
        for line in text.trim_end_matches('\n').split('\n') {
            self.debug.console.push(line.trim_end_matches('\r').to_owned());
        }
        if self.debug.console.len() > MAX_CONSOLE {
            self.debug.console.drain(..self.debug.console.len() - MAX_CONSOLE);
        }
    }

    pub(crate) fn debugging(&self) -> bool {
        self.debug.session.as_ref().is_some_and(|s| s.state != RunState::Ended)
    }

    fn adapter(&mut self) -> Option<Adapter> {
        if matches!(self.debug.adapter, AdapterState::Unknown) {
            self.debug.adapter = match dap::find_codelldb(&self.settings.debug.codelldb_path) {
                Some(a) => AdapterState::Found(a),
                None => AdapterState::Missing,
            };
        }
        match &self.debug.adapter {
            AdapterState::Found(a) => Some(a.clone()),
            _ => None,
        }
    }

    // ----- frame loop ---------------------------------------------------------------------

    pub(crate) fn pump_debug(&mut self, ctx: &egui::Context) {
        if let AdapterState::Installing(rx) = &self.debug.adapter
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok(adapter) => {
                    self.status = Some("CodeLLDB installed".to_owned());
                    self.debug.adapter = AdapterState::Found(adapter);
                    if let Some(launch) = self.debug.queued.take() {
                        self.start_session(ctx, launch);
                    }
                }
                Err(e) => {
                    self.status = Some(format!("Could not install CodeLLDB: {e}"));
                    self.debug.adapter = AdapterState::Missing;
                }
            }
        }
        while let Ok(msg) = self.debug.rx.try_recv() {
            match msg {
                Msg::Targets(found) => match found {
                    Err(why) => self.refuse_run(ctx, why),
                    Ok(mut targets) if targets.len() == 1 => {
                        let target = targets.remove(0);
                        self.debug_target(ctx, target);
                    }
                    Ok(targets) => {
                        let names = targets.iter().map(|t| t.title.clone()).collect();
                        self.debug.targets = targets;
                        self.palette.show_picker(PickerKind::DebugTarget, names, "", "Select a program to debug");
                    }
                },
                Msg::PythonChecked(launch, true) => self.start_session(ctx, launch),
                Msg::PythonChecked(launch, false) => {
                    self.debug.debugpy_offer = Some(launch);
                    self.show_debug_view();
                    self.status = Some("Debugging Python needs debugpy".to_owned());
                }
                Msg::DebugpyInstalled(launch, result) => {
                    self.debug.installing_debugpy = false;
                    match result {
                        Ok(()) => self.start_session(ctx, launch),
                        Err(e) => self.refuse_run(ctx, format!("Could not install debugpy: {e}")),
                    }
                }
                Msg::Processes(Err(e)) => self.status = Some(format!("Could not list processes: {e}")),
                Msg::Processes(Ok(list)) => {
                    self.status = None;
                    let names = list.iter().map(|p| format!("{} ({})", p.name, p.pid)).collect();
                    self.debug.processes = list;
                    self.palette.show_picker(PickerKind::AttachProcess, names, "", "Select a process to attach to");
                }
                Msg::Built(result) => {
                    self.debug.building = None;
                    match result {
                        Ok(launch) => self.start_session(ctx, launch),
                        Err(e) => {
                            self.status = Some("Build failed (see the debug console)".to_owned());
                            self.push_console(&e);
                            self.panel_visible = true;
                            self.panel_tab = PanelTab::DebugConsole;
                        }
                    }
                }
            }
        }
        let Some(session) = &mut self.debug.session else { return };
        // Only the newest session is being debugged; the rest just stay connected.
        session.parents.iter_mut().for_each(|p| drop(p.poll()));
        let events = session.client.poll();
        for ev in events {
            self.on_dap_event(ctx, ev);
        }
        if self.debug.session.as_ref().is_some_and(|s| s.state == RunState::Ended) {
            self.debug.session = None;
            self.status = Some("Debugging stopped".to_owned());
        }
    }

    fn request(&mut self, command: &str, args: Value, pending: Pending) {
        if let Some(s) = &mut self.debug.session {
            let seq = s.client.request(command, args);
            s.pending.insert(seq, pending);
        }
    }

    fn on_dap_event(&mut self, ctx: &egui::Context, ev: DapEvent) {
        match ev {
            DapEvent::Response { request_seq, success, message, body } => {
                let Some(pending) = self.debug.session.as_mut().and_then(|s| s.pending.remove(&request_seq)) else {
                    return;
                };
                if !success {
                    let msg = message.unwrap_or_else(|| "request failed".to_owned());
                    match pending {
                        Pending::Watch(i, generation) => {
                            if let Some(s) = &mut self.debug.session
                                && s.stop_gen == generation
                                && let Some(v) = s.watch_values.get_mut(i)
                            {
                                *v = Some(format!("<{msg}>"));
                            }
                        }
                        // Memory that can't be read is an answer, not an error.
                        Pending::Memory(start) => self.on_memory_page(start, None),
                        Pending::MemoryAddress => self.on_memory_address(None, Some(msg)),
                        Pending::Launch | Pending::Initialize => {
                            self.push_console(&format!("Could not start debugging: {msg}"));
                            self.end_session();
                        }
                        Pending::DataInfo(what) | Pending::GdbWatch(what) => {
                            self.status = Some(format!("Can't break when {what} changes: {msg}"));
                        }
                        _ => self.push_console(&format!("[{msg}]")),
                    }
                    return;
                }
                self.on_dap_response(ctx, pending, body);
            }
            DapEvent::Event { event, body } => self.on_dap_notification(ctx, &event, body),
            DapEvent::Request { seq, command, arguments } if command == "runInTerminal" => {
                let ok = self.run_in_terminal(ctx, &arguments);
                if let Some(s) = &mut self.debug.session {
                    s.client.respond(seq, &command, ok, json!({}));
                }
            }
            DapEvent::Request { seq, command, arguments } if command == "startDebugging" => {
                self.start_child_session(ctx, seq, &arguments);
            }
            DapEvent::Request { seq, command, .. } => {
                if let Some(s) = &mut self.debug.session {
                    s.client.respond(seq, &command, false, Value::Null);
                }
            }
            DapEvent::Exited => {
                if self.debug.session.as_ref().is_some_and(|s| s.state == RunState::Starting) {
                    let name = self.debug.session.as_ref().map(|s| s.launch.kind.name()).unwrap_or_default();
                    self.push_console(&format!("{name} ended before debugging started"));
                    self.panel_visible = true;
                    self.panel_tab = PanelTab::DebugConsole;
                }
                self.end_session();
            }
        }
    }

    fn on_dap_response(&mut self, ctx: &egui::Context, pending: Pending, body: Value) {
        match pending {
            Pending::Memory(start) => self.on_memory_page(start, hexdata::parse_read_memory(&body)),
            Pending::MemoryAddress => {
                let found = body["memoryReference"].as_str().map(str::to_owned);
                let found = found.or_else(|| {
                    let result = body["result"].as_str()?;
                    result.split_whitespace().find(|w| w.starts_with("0x")).map(str::to_owned)
                });
                self.on_memory_address(found, None);
            }
            Pending::Disassembly => self.on_disassembly(hexdata::parse_disassembly(&body)),
            Pending::Initialize => {
                if let Some(s) = &mut self.debug.session {
                    s.capabilities = hexdata::Capabilities::parse(&body);
                    s.support = Support::parse(&body);
                    if !s.support.exceptions.is_empty() {
                        self.debug.filters = s.support.exceptions.clone();
                    }
                }
                // A session the adapter asked for runs what it was told to.
                let asked = self.debug.session.as_mut().and_then(|s| s.asked_launch.take());
                if let Some((request, configuration)) = asked {
                    self.request(&request, configuration, Pending::Launch);
                    return;
                }
                let Some(s) = &self.debug.session else { return };
                let l = &s.launch;
                let step_into_std = self.settings.debug.step_into_std;
                if let Some(pid) = l.attach {
                    let args = l.kind.attach_args(pid, step_into_std);
                    self.request("attach", args, Pending::Launch);
                    return;
                }
                let args = l.kind.launch_args(&l.program, &l.args, &l.cwd, step_into_std);
                self.request("launch", args, Pending::Launch);
            }
            Pending::SetBreakpoints(path) => {
                let verified: Vec<bool> = body["breakpoints"]
                    .as_array()
                    .map(|a| a.iter().map(|b| b["verified"].as_bool().unwrap_or(false)).collect())
                    .unwrap_or_default();
                if let Some((_, bps)) = self.debug.breakpoints.get_mut(&key(&path)) {
                    for (bp, v) in bps.iter_mut().filter(|b| b.enabled).zip(verified) {
                        bp.verified = Some(v);
                    }
                }
            }
            Pending::SetFunctionBreakpoints => {
                let enabled = self.debug.functions.iter_mut().filter(|(f, _)| f.enabled);
                for ((_, v), placed) in enabled.zip(bpx::verified(&body)) {
                    *v = placed;
                }
            }
            Pending::SetDataBreakpoints => {
                let sent = self.debug.data.iter_mut().filter(|d| d.enabled && !d.id.starts_with("gdb:"));
                for (d, placed) in sent.zip(bpx::verified(&body)) {
                    d.verified = placed;
                }
            }
            Pending::SetExceptionBreakpoints => {}
            Pending::DataInfo(what) => match bpx::parse_data_info(&body) {
                DataInfo::Watchable { id, description, access } => {
                    if !self.debug.data.iter().any(|d| d.id == id) {
                        let description = if description.is_empty() { what } else { description };
                        let access = bpx::pick_access(&access);
                        self.debug.data.push(DataBreakpoint { id, description, access, enabled: true, verified: None });
                    }
                    self.send_data_breakpoints();
                }
                DataInfo::No(why) => self.status = Some(format!("Can't break when {what} changes: {why}")),
            },
            Pending::GdbWatch(what) => {
                // "Hardware watchpoint 2: -location n"
                let said = body["result"].as_str().unwrap_or("");
                let number = said
                    .split(|c: char| !c.is_ascii_digit())
                    .find(|w| !w.is_empty())
                    .filter(|_| said.to_lowercase().contains("watchpoint"));
                match number {
                    Some(n) => self.debug.data.push(DataBreakpoint {
                        id: format!("gdb:{n}"),
                        description: what,
                        access: "write".to_owned(),
                        enabled: true,
                        verified: Some(true),
                    }),
                    None => self.status = Some(format!("Can't break when {what} changes: {}", said.trim())),
                }
            }
            Pending::ConfigurationDone => {
                if let Some(s) = &mut self.debug.session
                    && s.state == RunState::Starting
                {
                    s.state = RunState::Running;
                }
            }
            Pending::Threads => {
                if let Some(s) = &mut self.debug.session {
                    s.threads = body["threads"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|t| Some((t["id"].as_i64()?, t["name"].as_str().unwrap_or("").to_owned())))
                                .collect()
                        })
                        .unwrap_or_default();
                }
            }
            Pending::StackTrace => {
                let frames: Vec<Frame> = body["stackFrames"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|f| {
                                Some(Frame {
                                    id: f["id"].as_i64()?,
                                    name: f["name"].as_str().unwrap_or("").to_owned(),
                                    path: f["source"]["path"].as_str().map(native_path),
                                    line: f["line"].as_u64().unwrap_or(0) as usize,
                                    pointer: f["instructionPointerReference"].as_str().map(str::to_owned),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Start at the first frame with source we can show (skip runtime code).
                let selected = frames.iter().position(|f| f.path.as_ref().is_some_and(|p| p.exists())).unwrap_or(0);
                if let Some(s) = &mut self.debug.session {
                    s.frames = frames;
                    s.selected = selected;
                }
                self.select_frame(ctx, selected);
            }
            Pending::Scopes => {
                let scopes: Vec<(String, i64, bool)> = body["scopes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| {
                                Some((
                                    s["name"].as_str()?.to_owned(),
                                    s["variablesReference"].as_i64()?,
                                    // Registers too (gdb lists them): seldom wanted, and long.
                                    s["expensive"].as_bool().unwrap_or(false)
                                        || s["presentationHint"].as_str() == Some("registers"),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Everything global to Node is hundreds of names: only if asked for.
                let lazy: HashSet<i64> = scopes
                    .iter()
                    .filter(|(name, _, expensive)| *expensive || name == "Global")
                    .map(|(_, reference, _)| *reference)
                    .collect();
                for (_, reference, _) in &scopes {
                    if !lazy.contains(reference) {
                        self.request(
                            "variables",
                            json!({ "variablesReference": reference }),
                            Pending::Variables(*reference),
                        );
                    }
                }
                if let Some(s) = &mut self.debug.session {
                    s.scopes = scopes.into_iter().map(|(n, r, _)| (n, r)).collect();
                    s.lazy_scopes = lazy;
                }
            }
            Pending::Variables(reference) => {
                let vars = body["variables"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|v| Var {
                                name: v["name"].as_str().unwrap_or("").to_owned(),
                                value: v["value"].as_str().unwrap_or("").to_owned(),
                                ty: v["type"].as_str().unwrap_or("").to_owned(),
                                reference: v["variablesReference"].as_i64().unwrap_or(0),
                                memory: v["memoryReference"].as_str().map(str::to_owned),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(s) = &mut self.debug.session {
                    s.vars.insert(reference, vars);
                }
            }
            Pending::Watch(i, generation) => {
                if let Some(s) = &mut self.debug.session
                    && s.stop_gen == generation
                    && let Some(v) = s.watch_values.get_mut(i)
                {
                    *v = Some(body["result"].as_str().unwrap_or("").to_owned());
                }
            }
            Pending::Repl => {
                let result = body["result"].as_str().unwrap_or("").to_owned();
                self.push_console(&result);
            }
            Pending::Launch | Pending::Other => {}
        }
    }

    fn on_dap_notification(&mut self, ctx: &egui::Context, event: &str, body: Value) {
        match event {
            "initialized" => {
                let files: Vec<PathBuf> = self.debug.breakpoints.values().map(|(p, _)| p.clone()).collect();
                for path in files {
                    self.send_breakpoints(&path);
                }
                self.send_function_breakpoints();
                self.send_exception_breakpoints();
                self.request("configurationDone", json!({}), Pending::ConfigurationDone);
            }
            "stopped" => {
                let thread = body["threadId"].as_i64().unwrap_or(0);
                let reason = body["reason"].as_str().unwrap_or("pause").to_owned();
                if let Some(s) = &mut self.debug.session {
                    s.state = RunState::Stopped { thread, reason: reason.clone() };
                    s.stop_gen += 1;
                    s.frames.clear();
                    s.scopes.clear();
                    s.vars.clear();
                }
                self.request("threads", json!({}), Pending::Threads);
                self.request(
                    "stackTrace",
                    json!({ "threadId": thread, "startFrame": 0, "levels": 64 }),
                    Pending::StackTrace,
                );
                self.status = Some(format!("Paused ({reason})"));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            "continued" => {
                if let Some(s) = &mut self.debug.session {
                    s.state = RunState::Running;
                    s.frames.clear();
                    s.scopes.clear();
                    s.vars.clear();
                }
            }
            "output" => {
                let category = body["category"].as_str().unwrap_or("console");
                if category != "telemetry"
                    && let Some(text) = body["output"].as_str()
                {
                    self.push_console(text);
                }
            }
            // js-debug only knows a script's breakpoints once it has loaded.
            "breakpoint" => {
                let found = &body["breakpoint"];
                if let (Some(path), Some(line), Some(verified)) =
                    (found["source"]["path"].as_str(), found["line"].as_u64(), found["verified"].as_bool())
                    && let Some((_, bps)) = self.debug.breakpoints.get_mut(&key(&native_path(path)))
                    && let Some(bp) = bps.iter_mut().find(|b| b.line + 1 == line as usize)
                {
                    bp.verified = Some(verified);
                }
            }
            // CodeLLDB reports the exit code in the console itself.
            "exited" => {}
            "terminated" => {
                let terminate = self.debug.session.as_ref().is_none_or(|s| s.launch.attach.is_none());
                self.request("disconnect", json!({ "terminateDebuggee": terminate }), Pending::Other);
                self.end_session();
            }
            _ => {}
        }
    }

    /// Starts CodeLLDB's launcher in a terminal tab; the program then runs
    /// there, with its output and input in that tab.
    fn run_in_terminal(&mut self, ctx: &egui::Context, arguments: &Value) -> bool {
        let Some(s) = &self.debug.session else { return false };
        let mut args: Vec<String> =
            arguments["args"].as_array().into_iter().flatten().filter_map(|a| a.as_str().map(str::to_owned)).collect();
        if args.is_empty() {
            return false;
        }
        let program = args.remove(0);
        let cwd = match arguments["cwd"].as_str() {
            Some(c) if !c.is_empty() => PathBuf::from(c),
            _ => s.launch.cwd.clone(),
        };
        let env: Vec<(String, String)> = arguments["env"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
            .collect();
        let title = format!("Debug: {}", s.launch.name);
        let scrollback = self.settings.terminal.scrollback;
        let task = Task { title: &title, program: &program, args: &args, env: &env, cwd: &cwd, scan: None };
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                // Keep the keyboard where it was; the Debug view drives the session.
                self.terminal.focus_requested = false;
                self.terminal.detached = false;
                true
            }
            Err(e) => {
                self.push_console(&format!("Could not open a terminal for the program: {e}"));
                false
            }
        }
    }

    fn end_session(&mut self) {
        if let Some(s) = &mut self.debug.session {
            s.state = RunState::Ended;
        }
    }

    /// Shows frame `i`: its source line, scopes, variables, watches.
    fn select_frame(&mut self, ctx: &egui::Context, i: usize) {
        let Some(s) = &mut self.debug.session else { return };
        let Some(frame) = s.frames.get(i) else { return };
        s.selected = i;
        s.scopes.clear();
        s.vars.clear();
        let (id, path, line, generation) = (frame.id, frame.path.clone(), frame.line, s.stop_gen);
        s.watch_values = vec![None; self.debug.watches.len()];
        self.request("scopes", json!({ "frameId": id }), Pending::Scopes);
        for (n, expr) in self.debug.watches.clone().iter().enumerate() {
            self.request(
                "evaluate",
                json!({ "expression": expr, "frameId": id, "context": "watch" }),
                Pending::Watch(n, generation),
            );
        }
        if let Some(path) = path.filter(|p| p.exists()) {
            self.open_file(&path);
            // Show the line without selecting it or taking keyboard focus,
            // so typing in the Debug view never lands in the editor.
            if let Some(doc) = self.active_doc() {
                let l = line.saturating_sub(1).min(doc.buffer.len_lines() - 1);
                let indent = doc.buffer.line_text(l).chars().take_while(|c| c.is_whitespace()).count();
                let pos = doc.buffer.line_start(l) + indent;
                doc.buffer.set_single(crate::buffer::Selection::cursor(pos));
                doc.reveal = true;
                doc.request_focus = false;
            }
        }
        ctx.request_repaint();
    }

    fn send_breakpoints(&mut self, path: &Path) {
        if !self.debugging() {
            return;
        }
        let list: Vec<Value> = self
            .debug
            .breakpoints
            .get(&key(path))
            .map(|(_, bps)| {
                bps.iter()
                    .filter(|b| b.enabled)
                    .map(|b| {
                        let mut v = json!({ "line": b.line + 1 });
                        if let Some(c) = &b.condition {
                            v["condition"] = json!(c);
                        }
                        if let Some(l) = &b.log {
                            v["logMessage"] = json!(l);
                        }
                        v
                    })
                    .collect()
            })
            .unwrap_or_default();
        let args = json!({ "source": { "path": path }, "breakpoints": list, "sourceModified": false });
        self.request("setBreakpoints", args, Pending::SetBreakpoints(path.to_path_buf()));
    }

    fn send_function_breakpoints(&mut self) {
        let supported = self.debug.session.as_ref().is_some_and(|s| s.support.functions);
        if !self.debugging() || !supported {
            return;
        }
        let list: Vec<FunctionBreakpoint> = self.debug.functions.iter().map(|(f, _)| f.clone()).collect();
        self.request("setFunctionBreakpoints", bpx::function_args(&list), Pending::SetFunctionBreakpoints);
    }

    fn send_exception_breakpoints(&mut self) {
        if !self.debugging() {
            return;
        }
        let Some(s) = &self.debug.session else { return };
        if s.support.exceptions.is_empty() {
            return;
        }
        let on: Vec<String> = s
            .support
            .exceptions
            .iter()
            .filter(|f| self.debug.exceptions.get(&f.id).copied().unwrap_or(f.default))
            .map(|f| f.id.clone())
            .collect();
        self.request("setExceptionBreakpoints", bpx::exception_args(&on), Pending::SetExceptionBreakpoints);
    }

    fn send_data_breakpoints(&mut self) {
        let supported = self.debug.session.as_ref().is_some_and(|s| s.support.data);
        if !self.debugging() || !supported {
            return;
        }
        let list: Vec<DataBreakpoint> = self.debug.data.iter().filter(|d| !d.id.starts_with("gdb:")).cloned().collect();
        self.request("setDataBreakpoints", bpx::data_args(&list), Pending::SetDataBreakpoints);
    }

    /// A gdb command, through its adapter's console.
    fn gdb_command(&mut self, command: String, pending: Pending) {
        let frame = self.debug.session.as_ref().and_then(|s| s.frames.get(s.selected).map(|f| f.id));
        self.request("evaluate", json!({ "expression": command, "frameId": frame, "context": "repl" }), pending);
    }

    /// Break when variable `name` (in container `reference`) changes.
    fn break_on_change(&mut self, reference: i64, name: &str) {
        let Some(s) = &self.debug.session else { return };
        if s.support.data {
            let args = bpx::data_info_args(reference, name);
            self.request("dataBreakpointInfo", args, Pending::DataInfo(name.to_owned()));
        } else if s.launch.kind == DebugKind::Gdb {
            // gdb's adapter has no data breakpoints: a watchpoint of gdb's own, on
            // the variable's memory, so it outlives leaving the function.
            self.gdb_command(format!("watch -location {name}"), Pending::GdbWatch(name.to_owned()));
        }
    }

    /// Turns data breakpoint `i` on or off (`Some`), or removes it (`None`).
    fn change_data_breakpoint(&mut self, i: usize, enable: Option<bool>) {
        let Some(d) = self.debug.data.get_mut(i) else { return };
        let gdb = d.id.strip_prefix("gdb:").map(str::to_owned);
        match enable {
            Some(on) => d.enabled = on,
            None => {
                self.debug.data.remove(i);
            }
        }
        match (gdb, enable) {
            (Some(n), Some(true)) => self.gdb_command(format!("enable {n}"), Pending::Other),
            (Some(n), Some(false)) => self.gdb_command(format!("disable {n}"), Pending::Other),
            (Some(n), None) => self.gdb_command(format!("delete {n}"), Pending::Other),
            (None, _) => self.send_data_breakpoints(),
        }
    }

    /// Debug: Attach to Process: lists what's running, to pick from.
    pub(crate) fn attach_to_process(&mut self, ctx: &egui::Context) {
        if self.debugging() {
            self.status = Some("Stop debugging first, then attach".to_owned());
            return;
        }
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Processes(bpx::processes()));
            wake();
        });
        self.status = Some("Listing processes…".to_owned());
    }

    pub(crate) fn picked_attach_process(&mut self, label: &str) {
        let Some(p) = self.debug.processes.iter().find(|p| format!("{} ({})", p.name, p.pid) == label).cloned() else {
            return;
        };
        let names = bpx::suggested_kinds(&p.name).into_iter().map(str::to_owned).collect();
        self.debug.attach_to = Some(p);
        self.palette.show_picker(PickerKind::AttachDebugger, names, "", "Attach with which debugger?");
    }

    pub(crate) fn picked_attach_debugger(&mut self, ctx: &egui::Context, label: &str) {
        let Some(p) = self.debug.attach_to.take() else { return };
        let which = coiman_core::language::lsp::servers::which;
        let kind = match label {
            "gdb" => DebugKind::Gdb,
            "CodeLLDB" => DebugKind::Lldb,
            "Delve" => DebugKind::Go,
            "js-debug" => DebugKind::Js,
            "debugpy" => match which("python").or_else(|| which("py")) {
                Some(python) => DebugKind::Python { python, env: Vec::new() },
                None => {
                    self.refuse_run(ctx, "Attaching debugpy needs Python on PATH".to_owned());
                    return;
                }
            },
            _ => return,
        };
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let program = PathBuf::from(&p.name);
        let launch = Launch { kind, name: p.name, program, args: Vec::new(), cwd, attach: Some(p.pid) };
        // debugpy attaches only from a Python that has it: offer to install it first.
        let DebugKind::Python { python, .. } = &launch.kind else {
            self.start_session(ctx, launch);
            return;
        };
        let python = python.clone();
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let found = dap::python_has_debugpy(&python);
            let _ = tx.send(Msg::PythonChecked(launch, found));
            wake();
        });
    }

    /// Every breakpoint, as an export.
    fn saved_breakpoints(&self) -> bpx::Saved {
        let root = self.explorer.root.as_deref();
        let lines = self
            .debug
            .breakpoints
            .values()
            .flat_map(|(path, bps)| {
                bps.iter().map(move |b| bpx::SavedLine {
                    path: bpx::portable_path(path, root),
                    line: b.line + 1,
                    condition: b.condition.clone(),
                    log: b.log.clone(),
                    enabled: b.enabled,
                })
            })
            .collect();
        let mut exceptions: Vec<(String, bool)> = self.debug.exceptions.iter().map(|(k, v)| (k.clone(), *v)).collect();
        exceptions.sort();
        bpx::Saved { lines, functions: self.debug.functions.iter().map(|(f, _)| f.clone()).collect(), exceptions }
    }

    fn kept_breakpoints(&mut self) -> &mut BTreeMap<PathBuf, bpx::Saved> {
        self.debug
            .kept
            .get_or_insert_with(|| coiman_core::project::session::load().map(|s| s.breakpoints).unwrap_or_default())
    }

    /// Keeps the open project's breakpoints with the others', for the session file.
    fn keep_breakpoints(&mut self) {
        let Some(root) = self.explorer.root.clone() else { return };
        let saved = self.saved_breakpoints();
        let kept = self.kept_breakpoints();
        if saved.is_empty() {
            kept.remove(&root);
        } else {
            kept.insert(root, saved);
        }
    }

    /// Every project's breakpoints, the open one's as they are now.
    pub(crate) fn breakpoints_by_project(&mut self) -> BTreeMap<PathBuf, bpx::Saved> {
        self.keep_breakpoints();
        self.kept_breakpoints().clone()
    }

    /// Opening folder `root`: the last one's breakpoints are kept, and `root`'s come back.
    pub(crate) fn restore_breakpoints(&mut self, root: &Path) {
        if self.explorer.root.as_deref() == Some(root) {
            return;
        }
        self.keep_breakpoints();
        self.debug.breakpoints.clear();
        self.debug.functions.clear();
        self.debug.exceptions.clear();
        let saved = self.kept_breakpoints().get(root).cloned();
        if let Some(saved) = saved {
            self.merge_breakpoints(saved, Some(root));
        }
    }

    /// Where breakpoint files are saved and read: the project's `.coiman` folder at first.
    fn breakpoints_dialog(&self) -> rfd::FileDialog {
        let dialog = self.file_dialog("breakpoints").add_filter("Breakpoints", &["json"]);
        let coiman = self.explorer.root.as_ref().map(|r| r.join(".coiman")).filter(|d| d.is_dir());
        match coiman {
            Some(dir) if self.dialog_dirs.get("breakpoints").is_none() => dialog.set_directory(dir),
            _ => dialog,
        }
    }

    /// Debug: Export Breakpoints.
    pub(crate) fn export_breakpoints(&mut self) {
        let saved = self.saved_breakpoints();
        let Some(path) = self.breakpoints_dialog().set_file_name(bpx::FILE_NAME).save_file() else { return };
        self.dialog_chose("breakpoints", &path);
        let count = saved.lines.len() + saved.functions.len();
        self.status = Some(match std::fs::write(&path, saved.to_json()) {
            Ok(()) => format!("Exported {} to {}", breakpoints_count(count), file_name(&path)),
            Err(e) => format!("Could not export breakpoints: {e} ({})", path.display()),
        });
    }

    /// Debug: Import Breakpoints: adds a file's breakpoints to those already set.
    pub(crate) fn import_breakpoints(&mut self) {
        let Some(path) = self.breakpoints_dialog().pick_file() else { return };
        self.dialog_chose("breakpoints", &path);
        let saved = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| bpx::Saved::from_json(&t));
        match saved {
            Ok(saved) => {
                let root = self.explorer.root.clone();
                let count = self.merge_breakpoints(saved, root.as_deref());
                self.status = Some(format!("Imported {} from {}", breakpoints_count(count), file_name(&path)));
            }
            Err(e) => self.status = Some(format!("Could not import breakpoints: {e} ({})", file_name(&path))),
        }
    }

    /// Adds `saved` to the breakpoints set (one on the same line or function is replaced),
    /// and sends them to a session running. How many there were.
    fn merge_breakpoints(&mut self, saved: bpx::Saved, root: Option<&Path>) -> usize {
        let count = saved.lines.len() + saved.functions.len();
        let mut files = Vec::new();
        for l in saved.lines {
            let path = bpx::local_path(&l.path, root);
            let entry = self.debug.breakpoints.entry(key(&path)).or_insert_with(|| (path.clone(), Vec::new()));
            let line = l.line - 1;
            entry.1.retain(|b| b.line != line);
            let bp = Breakpoint { line, condition: l.condition, log: l.log, enabled: l.enabled, verified: None };
            entry.1.push(bp);
            entry.1.sort_by_key(|b| b.line);
            if !files.contains(&path) {
                files.push(path);
            }
        }
        for f in saved.functions {
            self.debug.functions.retain(|(g, _)| g.name != f.name);
            self.debug.functions.push((f, None));
        }
        self.debug.exceptions.extend(saved.exceptions);
        for path in files {
            self.send_breakpoints(&path);
        }
        self.send_function_breakpoints();
        self.send_exception_breakpoints();
        count
    }

    // ----- starting -------------------------------------------------------------------------

    /// js-debug's way of running a program: another connection, which is then the session.
    fn start_child_session(&mut self, ctx: &egui::Context, seq: i64, arguments: &Value) {
        let Some(s) = &mut self.debug.session else { return };
        let connected = s.client.address().map(|at| DapClient::connect(at, repaint(ctx)));
        let Some(Ok(child)) = connected else {
            s.client.respond(seq, "startDebugging", false, Value::Null);
            self.push_console("Could not open the program's debugging session");
            return;
        };
        s.client.respond(seq, "startDebugging", true, json!({}));
        let request = arguments["request"].as_str().unwrap_or("launch").to_owned();
        s.asked_launch = Some((request, arguments["configuration"].clone()));
        s.parents.push(std::mem::replace(&mut s.client, child));
        s.pending.clear();
        self.request("initialize", dap::initialize_args(), Pending::Initialize);
    }

    fn start_session(&mut self, ctx: &egui::Context, launch: Launch) {
        if !self.allowed_to_run(&launch.cwd) {
            return;
        }
        let adapter = match &launch.kind {
            DebugKind::Lldb => self.adapter(),
            other => other.adapter(),
        };
        let Some(adapter) = adapter else {
            match &launch.kind {
                DebugKind::Lldb => {}
                DebugKind::Go => {
                    let how = "Debugging Go needs Delve: run `go install github.com/go-delve/delve/cmd/dlv@latest`";
                    self.refuse_run(ctx, how.to_owned());
                    return;
                }
                DebugKind::Js if coiman_core::language::lsp::servers::which("node").is_none() => {
                    self.refuse_run(ctx, "Debugging JavaScript needs Node.js, which isn't on PATH".to_owned());
                    return;
                }
                DebugKind::Js => {
                    self.offer_build_tool(coiman_core::build::build_tools::BuildTool::JsDebug);
                    return;
                }
                DebugKind::Gdb => {
                    let how = "Debugging C and C++ needs gdb 14 or later: not found on PATH or in a MinGW folder (C:\\MinGW\\bin, C:\\mingw32\\bin, C:\\msys64\\ucrt64\\bin...). MSYS2: pacman -S mingw-w64-ucrt-x86_64-gdb";
                    self.refuse_run(ctx, how.to_owned());
                    return;
                }
                other => {
                    let name = other.name();
                    self.refuse_run(ctx, format!("{name} isn't on PATH"));
                    return;
                }
            }
            // Ask to install CodeLLDB; start once it is there.
            self.debug.queued = Some(launch);
            self.debug.offer_dismissed = false;
            self.show_debug_view();
            self.status = Some("The debugger (CodeLLDB) isn't installed".to_owned());
            return;
        };
        self.debug.session = None;
        self.debug.console.clear();
        match launch.attach {
            Some(pid) => {
                let how = format!("Attaching {} to {} (process {pid})", launch.kind.name(), launch.name);
                self.push_console(&how);
            }
            None => self.push_console(&format!("Debugging {} {}", launch.program.display(), launch.args.join(" "))),
        }
        if !self.allowed_to_run(&launch.cwd) {
            return;
        }
        let started = if launch.kind.listens() {
            DapClient::start_listening(&adapter.program, &adapter.args, &launch.cwd, repaint(ctx))
        } else {
            DapClient::start(&adapter.program, &adapter.args, &launch.cwd, repaint(ctx))
        };
        match started {
            Ok(client) => {
                self.debug.last_launch = Some(launch.clone());
                self.debug.session = Some(Session {
                    client,
                    parents: Vec::new(),
                    asked_launch: None,
                    launch,
                    state: RunState::Starting,
                    threads: Vec::new(),
                    frames: Vec::new(),
                    selected: 0,
                    scopes: Vec::new(),
                    lazy_scopes: HashSet::new(),
                    vars: HashMap::new(),
                    expanded: HashSet::new(),
                    watch_values: Vec::new(),
                    pending: HashMap::new(),
                    stop_gen: 0,
                    capabilities: Default::default(),
                    support: Support::default(),
                });
                // A data breakpoint's id only means something to the session that gave it.
                self.debug.data.clear();
                self.debug.functions.iter_mut().for_each(|(_, v)| *v = None);
                self.request("initialize", dap::initialize_args(), Pending::Initialize);
                self.show_debug_view();
                self.panel_visible = true;
                self.panel_tab = PanelTab::DebugConsole;
            }
            Err(e) => self.status = Some(format!("Could not start the debugger: {e}")),
        }
    }

    fn show_debug_view(&mut self) {
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Debug;
    }

    /// F5: continue when paused; otherwise pick a binary, build it and debug it.
    pub(crate) fn start_debugging(&mut self, ctx: &egui::Context) {
        if let Some(s) = &self.debug.session {
            if let RunState::Stopped { thread, .. } = s.state {
                self.request("continue", json!({ "threadId": thread }), Pending::Other);
            }
            return;
        }
        // Source in another language can't be debugged, even in a Cargo folder.
        let active = self.docs.get(self.active).and_then(|d| d.path.clone());
        let source_here = active.as_deref().filter(|p| {
            (run::may_run(p) || self.debug_kind_for(p).is_some()) && p.file_name().is_some_and(|n| n != "Cargo.toml")
        });
        if let Some(file) = source_here.filter(|p| p.extension().is_none_or(|e| e != "rs")) {
            let file = file.to_path_buf();
            self.debug_source_file(ctx, &file);
            return;
        }
        // Run's rules decide what the file means; other files stand for the project.
        let file = source_here.map(Path::to_path_buf).or_else(|| self.cargo_root_dir().map(|d| d.join("Cargo.toml")));
        let Some(file) = file else {
            self.refuse_run(ctx, "Debugging works for Cargo projects so far: open one to debug".to_owned());
            return;
        };
        let root = self.explorer.root.clone();
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Targets(run::debug_actions(&file, root.as_deref())));
            wake();
        });
    }

    /// The debugger for a kind of file besides Rust.
    fn debug_kind_for(&self, file: &Path) -> Option<DebugKind> {
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(language) = registry::by_extension(&ext, true)
            && let (Some(command), Some(debug_type)) = (language.debugger, language.debug_type)
        {
            return Some(DebugKind::Data { command: command.to_owned(), debug_type: debug_type.to_owned() });
        }
        match ext.as_str() {
            "go" => return Some(DebugKind::Go),
            "c" | "h" | "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" => return Some(DebugKind::Gdb),
            "js" | "mjs" | "cjs" => return Some(DebugKind::Js),
            _ => {}
        }
        if matches!(ext.as_str(), "py" | "pyw") {
            let env = self.python_env()?;
            return Some(DebugKind::Python { python: env.python.clone(), env: self.environment_variables() });
        }
        let language = registry::by_extension(&ext, false)?;
        Some(DebugKind::Data { command: language.debugger?.to_owned(), debug_type: language.debug_type?.to_owned() })
    }

    /// Whether F5 means anything here: a Cargo project, or a file with a debugger.
    pub(crate) fn can_debug(&self) -> bool {
        self.cargo_project
            || self.debug.session.is_some()
            || self
                .docs
                .get(self.active)
                .and_then(|d| d.path.as_deref())
                .is_some_and(|p| self.debug_kind_for(p).is_some())
    }

    /// F5 on a file that runs as it is.
    fn debug_source_file(&mut self, ctx: &egui::Context, file: &Path) {
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(kind) = self.debug_kind_for(file) else {
            self.refuse_run(
                ctx,
                format!("No debugger for {name}: Shift+F10 runs it. `debugger` in languages.toml adds one"),
            );
            return;
        };
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty() && self.docs[i].path.is_some() && !self.save_doc(i) {
                let why = format!("Nothing was started: {} couldn't be saved", self.docs[i].title());
                self.refuse_run(ctx, why);
                return;
            }
        }
        let cwd = coiman_core::launch::repl::start_folder(file, self.explorer.root.as_deref());
        if !self.allowed_to_run(&cwd) {
            return;
        }
        // C and C++ are built first, with debug information: as .vscode/launch.json says,
        // else the makefile's `debug` target, else gcc -g.
        if kind == DebugKind::Gdb {
            let (file, root) = (file.to_path_buf(), self.explorer.root.clone());
            self.build_then_debug(ctx, format!("Building {name} for debugging..."), move || {
                let built = coiman_core::build::c_build::debug_build(&file, root.as_deref())?;
                let name = built.program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                Ok(Launch {
                    kind: DebugKind::Gdb,
                    name,
                    program: built.program,
                    args: built.args,
                    cwd: built.cwd,
                    attach: None,
                })
            });
            return;
        }
        let launch =
            Launch { kind: kind.clone(), name, program: file.to_path_buf(), args: Vec::new(), cwd, attach: None };
        let DebugKind::Python { python, .. } = kind else {
            self.start_session(ctx, launch);
            return;
        };
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let found = dap::python_has_debugpy(&python);
            let _ = tx.send(Msg::PythonChecked(launch, found));
            wake();
        });
    }

    /// The user agreed: installs debugpy, then starts the session it was for.
    fn install_debugpy(&mut self, ctx: &egui::Context) {
        let Some(launch) = self.debug.debugpy_offer.take() else { return };
        let DebugKind::Python { python, .. } = &launch.kind else { return };
        let python = python.clone();
        self.debug.installing_debugpy = true;
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let _ = tx.send(Msg::DebugpyInstalled(launch, dap::install_debugpy(&python)));
            wake();
        });
    }

    pub(crate) fn picked_debug_target(&mut self, ctx: &egui::Context, name: &str) {
        if let Some(target) = self.debug.targets.iter().find(|t| t.title == name).cloned() {
            self.debug_target(ctx, target);
        }
    }

    /// Builds what Run would start for a file and debugs it.
    fn debug_target(&mut self, ctx: &egui::Context, target: run::CargoAction) {
        let name = target.name.clone();
        self.build_then_debug(ctx, format!("Building {name} for debugging…"), move || {
            let (flag, package) = (format!("--{}", target.flag), target.package.clone());
            let command = if target.tool == "run" { vec!["build"] } else { vec!["test", "--no-run"] };
            let mut args: Vec<String> = command.into_iter().map(str::to_owned).collect();
            args.extend(["-p".to_owned(), package, flag, name.clone()]);
            let artifacts = cargo::build_executables(&target.dir, &args)?;
            let same = |a: &str| a.replace('-', "_") == name.replace('-', "_");
            let exe = artifacts
                .into_iter()
                .find(|a| a.kind == target.flag && same(&a.name) && a.test == (target.tool != "run"))
                .ok_or_else(|| format!("cargo built no executable for {name}"))?;
            Ok(Launch {
                kind: DebugKind::Lldb,
                name,
                program: exe.executable,
                args: Vec::new(),
                cwd: target.dir,
                attach: None,
            })
        });
    }

    /// Debugs one test (or a test module) from the Testing view or the cursor.
    pub(crate) fn debug_test(&mut self, ctx: &egui::Context, target: FileTarget, item: TestItem) {
        let label = format!("Building tests of {} for debugging…", target.package);
        self.build_then_debug(ctx, label, move || {
            let (args, name, kind) = cargo::test_build_args(&target.package, &target.target);
            let artifacts = cargo::build_executables(&target.dir, &args)?;
            let exe = artifacts
                .into_iter()
                .find(|a| a.test && a.kind == kind && a.name.replace('-', "_") == name.replace('-', "_"))
                .ok_or("cargo built no test executable")?;
            let mut run_args = vec![item.path.clone()];
            if !item.is_module {
                run_args.push("--exact".to_owned());
            }
            run_args.push("--nocapture".to_owned());
            Ok(Launch {
                kind: DebugKind::Lldb,
                name: item.name.clone(),
                program: exe.executable,
                args: run_args,
                cwd: target.dir.clone(),
                attach: None,
            })
        });
    }

    fn build_then_debug(
        &mut self,
        ctx: &egui::Context,
        label: String,
        build: impl FnOnce() -> Result<Launch, String> + Send + 'static,
    ) {
        if self.debug.building.is_some() {
            return;
        }
        self.status = Some(label.clone());
        self.debug.building = Some(label);
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Built(build()));
            wake();
        });
    }

    pub(crate) fn cargo_root_dir(&self) -> Option<PathBuf> {
        if let Some(root) = self.explorer.root.as_ref().filter(|r| r.join("Cargo.toml").exists()) {
            return Some(root.clone());
        }
        let path = self.docs.get(self.active)?.path.as_ref()?;
        path.ancestors().skip(1).find(|d| d.join("Cargo.toml").exists()).map(cargo::workspace_dir)
    }

    // ----- controls -------------------------------------------------------------------------

    fn stopped_thread(&self) -> Option<i64> {
        match &self.debug.session.as_ref()?.state {
            RunState::Stopped { thread, .. } => Some(*thread),
            _ => None,
        }
    }

    /// "next", "stepIn" or "stepOut" on the paused thread.
    pub(crate) fn debug_is_paused(&self) -> bool {
        self.stopped_thread().is_some()
    }

    pub(crate) fn debug_capabilities(&self) -> hexdata::Capabilities {
        self.debug.session.as_ref().map(|s| s.capabilities).unwrap_or_default()
    }

    /// Changes when what a paused program shows does: a new stop, or another frame.
    pub(crate) fn debug_stop_key(&self) -> (u64, usize) {
        self.debug.session.as_ref().map_or((0, 0), |s| (s.stop_gen, s.selected))
    }

    /// Whether this stop's call stack has arrived.
    pub(crate) fn debug_frames_ready(&self) -> bool {
        self.debug.session.as_ref().is_some_and(|s| !s.frames.is_empty())
    }

    pub(crate) fn debug_pointer(&self) -> Option<u64> {
        let s = self.debug.session.as_ref()?;
        s.frames.get(s.selected)?.pointer.as_deref().and_then(hexdata::parse_address)
    }

    pub(crate) fn debug_request_memory(&mut self, start: u64) {
        if self.debug_is_paused() {
            let args = json!({ "memoryReference": format!("0x{start:x}"), "offset": 0, "count": hexdata::PAGE });
            self.request("readMemory", args, Pending::Memory(start));
        }
    }

    pub(crate) fn debug_evaluate_address(&mut self, expression: &str) {
        let frame = self.debug.session.as_ref().and_then(|s| s.frames.get(s.selected)).map(|f| f.id);
        let args = json!({ "expression": expression, "frameId": frame, "context": "watch" });
        self.request("evaluate", args, Pending::MemoryAddress);
    }

    pub(crate) fn debug_request_disassembly(&mut self) {
        let pointer = self.debug.session.as_ref().and_then(|s| s.frames.get(s.selected)?.pointer.clone());
        let Some(pointer) = pointer else {
            self.status = Some("This frame doesn't say where it's executing".to_owned());
            return;
        };
        let args = json!({
            "memoryReference": pointer,
            "offset": 0,
            "instructionOffset": -24,
            "instructionCount": 64,
            "resolveSymbols": true,
        });
        self.request("disassemble", args, Pending::Disassembly);
    }

    pub(crate) fn debug_step_instruction(&mut self, command: &str) {
        if let Some(thread) = self.stopped_thread() {
            self.request(command, json!({ "threadId": thread, "granularity": "instruction" }), Pending::Other);
        }
    }

    pub(crate) fn debug_step(&mut self, command: &str) {
        if let Some(thread) = self.stopped_thread() {
            self.request(command, json!({ "threadId": thread }), Pending::Other);
        }
    }

    pub(crate) fn debug_pause(&mut self) {
        let thread = self.debug.session.as_ref().and_then(|s| s.threads.first().map(|t| t.0)).unwrap_or(0);
        if self.debug.session.as_ref().is_some_and(|s| s.state == RunState::Running) {
            self.request("pause", json!({ "threadId": thread }), Pending::Other);
        }
    }

    /// Stops debugging: ends a program that was started, leaves one attached to running.
    pub(crate) fn debug_stop(&mut self) {
        if let Some(s) = &self.debug.session {
            let terminate = s.launch.attach.is_none();
            self.request("disconnect", json!({ "terminateDebuggee": terminate }), Pending::Other);
            self.end_session();
        }
    }

    pub(crate) fn debug_restart(&mut self, ctx: &egui::Context) {
        let launch = self.debug.last_launch.clone();
        self.debug_stop();
        self.debug.session = None;
        if let Some(l) = launch {
            self.start_session(ctx, l);
        }
    }

    // ----- breakpoints ----------------------------------------------------------------------

    /// F9 / gutter click: add or remove a breakpoint on `line` of the active file.
    pub(crate) fn toggle_breakpoint(&mut self, line: usize) {
        let Some(path) = self.docs.get(self.active).and_then(|d| d.path.clone()) else { return };
        let entry = self.debug.breakpoints.entry(key(&path)).or_insert_with(|| (path.clone(), Vec::new()));
        if let Some(i) = entry.1.iter().position(|b| b.line == line) {
            entry.1.remove(i);
        } else {
            entry.1.push(Breakpoint { line, condition: None, log: None, enabled: true, verified: None });
            entry.1.sort_by_key(|b| b.line);
        }
        if entry.1.is_empty() {
            self.debug.breakpoints.remove(&key(&path));
        }
        self.send_breakpoints(&path);
    }

    pub(crate) fn toggle_breakpoint_at_cursor(&mut self) {
        if let Some(doc) = self.docs.get(self.active) {
            let line = doc.buffer.char_to_line(doc.buffer.primary().head);
            self.toggle_breakpoint(line);
        }
    }

    /// Asks for a condition (`log` false) or log message for the cursor line's breakpoint.
    pub(crate) fn edit_breakpoint(&mut self, log: bool) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.clone() else { return };
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let current = self
            .debug
            .breakpoints
            .get(&key(&path))
            .and_then(|(_, b)| b.iter().find(|b| b.line == line))
            .and_then(|b| if log { b.log.clone() } else { b.condition.clone() })
            .unwrap_or_default();
        self.debug.editing = Some((path, line));
        let (kind, hint) = if log {
            (InputKind::Logpoint, "Log message, e.g. x is {x} (Enter; empty removes it)")
        } else {
            (InputKind::BreakpointCondition, "Break when this expression is true, e.g. i == 3 (Enter)")
        };
        self.palette.show_input(kind, &current, hint);
    }

    pub(crate) fn finish_breakpoint_edit(&mut self, log: bool, text: &str) {
        let Some((path, line)) = self.debug.editing.take() else { return };
        let entry = self.debug.breakpoints.entry(key(&path)).or_insert_with(|| (path.clone(), Vec::new()));
        let bp = match entry.1.iter_mut().position(|b| b.line == line) {
            Some(i) => &mut entry.1[i],
            None => {
                entry.1.push(Breakpoint { line, condition: None, log: None, enabled: true, verified: None });
                entry.1.sort_by_key(|b| b.line);
                let i = entry.1.iter().position(|b| b.line == line).expect("just added");
                &mut entry.1[i]
            }
        };
        let value = Some(text.trim().to_owned()).filter(|t| !t.is_empty());
        if log {
            bp.log = value;
        } else {
            bp.condition = value;
        }
        self.send_breakpoints(&path);
    }

    /// Breakpoint markers and the paused line for document `i`.
    pub(crate) fn debug_decorate(&self, i: usize, deco: &mut Decorations) {
        let Some(path) = self.docs.get(i).and_then(|d| d.path.as_ref()) else { return };
        if let Some((_, bps)) = self.debug.breakpoints.get(&key(path)) {
            deco.breakpoints = bps
                .iter()
                .map(|b| {
                    let color = match (b.enabled, b.verified) {
                        (false, _) | (_, Some(false)) => Color32::from_gray(0x80),
                        _ if b.log.is_some() => Color32::from_rgb(0xE5, 0xC0, 0x7B),
                        _ => Color32::from_rgb(0xE5, 0x14, 0x00),
                    };
                    (b.line, color, b.condition.is_some() || b.log.is_some())
                })
                .collect();
        }
        if let Some(s) = &self.debug.session
            && let Some(f) = s.frames.get(s.selected)
            && f.path.as_deref().is_some_and(|p| same_path(p, path))
        {
            deco.debug_line = f.line.checked_sub(1);
        }
    }

    // ----- installing CodeLLDB --------------------------------------------------------------

    fn install_adapter(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(dap::install_codelldb());
            wake();
        });
        self.debug.adapter = AdapterState::Installing(rx);
        self.status = Some("Downloading CodeLLDB…".to_owned());
    }

    // ----- views ----------------------------------------------------------------------------

    /// The Debug side bar view.
    pub(crate) fn debug_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        ui.horizontal(|ui| {
            ui.label(RichText::new("RUN AND DEBUG").small().strong());
        });

        if self.debug.installing_debugpy {
            ui.label(RichText::new("Installing debugpy…").color(pal.text_dim));
        }
        let offer = self.debug.debugpy_offer.as_ref().map(|launch| match &launch.kind {
            DebugKind::Python { python, .. } => python.display().to_string(),
            _ => String::new(),
        });
        if let Some(python) = offer {
            egui::Frame::new().fill(pal.hover_bg).inner_margin(6.0).show(ui, |ui| {
                ui.label(format!(
                    "Debugging Python needs debugpy, which this Python doesn't have. Install it from PyPI with pip, \
                     into {python}?"
                ));
                ui.horizontal(|ui| {
                    if ui.button("Install").clicked() {
                        self.install_debugpy(&ctx);
                    }
                    if ui.button("Not Now").clicked() {
                        self.debug.debugpy_offer = None;
                    }
                });
            });
        }
        // Offer to install the debugger (nothing is downloaded without a yes).
        let rust = self.docs.get(self.active).is_none_or(|d| d.extension() == "rs") && self.cargo_project;
        let missing = rust
            && (matches!(self.debug.adapter, AdapterState::Missing)
                || (matches!(self.debug.adapter, AdapterState::Unknown) && self.adapter().is_none()));
        if matches!(self.debug.adapter, AdapterState::Installing(_)) {
            ui.label(RichText::new("Downloading and unpacking CodeLLDB…").color(pal.text_dim));
        } else if missing && !self.debug.offer_dismissed {
            egui::Frame::new().fill(pal.hover_bg).inner_margin(6.0).show(ui, |ui| {
                ui.label(
                    "Debugging needs CodeLLDB, which isn't installed. Download it from \
                     github.com/vadimcn/codelldb (about 135 MB once unpacked into Coiman IDE's \
                     config folder)? It uses the system's curl and tar.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Download").clicked() {
                        self.install_adapter(&ctx);
                    }
                    if ui.button("Not Now").clicked() {
                        self.debug.offer_dismissed = true;
                    }
                });
            });
        }

        // Toolbar.
        let state = self.debug.session.as_ref().map(|s| s.state.clone());
        ui.horizontal(|ui| match &state {
            None | Some(RunState::Ended) => {
                if icons::button(ui, Icon::Play, "Start Debugging (F5)").clicked() {
                    self.start_debugging(&ctx);
                }
                if let Some(b) = &self.debug.building {
                    ui.label(RichText::new(b).small().color(pal.text_dim));
                }
            }
            Some(st) => {
                let paused = matches!(st, RunState::Stopped { .. });
                if paused {
                    if icons::button(ui, Icon::Play, "Continue (F5)").clicked() {
                        self.start_debugging(&ctx);
                    }
                } else if icons::button(ui, Icon::Pause, "Pause (F6)").clicked() {
                    self.debug_pause();
                }
                if icons::button(ui, Icon::StepOver, "Step Over (F10)").clicked() {
                    self.debug_step("next");
                }
                if icons::button(ui, Icon::StepInto, "Step Into (F11)").clicked() {
                    self.debug_step("stepIn");
                }
                if icons::button(ui, Icon::StepOut, "Step Out (Shift+F11)").clicked() {
                    self.debug_step("stepOut");
                }
                if icons::button(ui, Icon::Refresh, "Restart (Ctrl+Shift+F5)").clicked() {
                    self.debug_restart(&ctx);
                }
                if icons::button(ui, Icon::Stop, "Stop (Shift+F5)").clicked() {
                    self.debug_stop();
                }
                let text = match st {
                    RunState::Starting => "starting…".to_owned(),
                    RunState::Running => "running".to_owned(),
                    RunState::Stopped { reason, .. } => format!("paused ({reason})"),
                    RunState::Ended => String::new(),
                };
                ui.label(RichText::new(text).small().color(pal.text_dim));
            }
        });
        ui.separator();

        let mut select = None;
        let mut toggle = None;
        let mut expand = Vec::new();
        let mut remove_watch = None;
        let mut goto_bp: Option<(PathBuf, usize)> = None;
        let mut remove_bp: Option<(PathBuf, usize)> = None;
        let mut resend = false;
        let mut memory_at = None;
        let mut data_at: Option<(i64, String)> = None;
        let mut functions_changed = false;
        let mut remove_function = None;
        let mut change_data: Option<(usize, Option<bool>)> = None;
        let mut exceptions_changed = false;
        let mut export = false;
        let mut import = false;
        let mut low_level = None;
        if let Some(s) = self.debug.session.as_ref().filter(|s| matches!(s.state, RunState::Stopped { .. })) {
            ui.horizontal(|ui| {
                if s.capabilities.read_memory && ui.button("Memory").clicked() {
                    low_level = Some(true);
                }
                if s.capabilities.disassemble && ui.button("Disassembly").clicked() {
                    low_level = Some(false);
                }
            });
        }
        // A scroll bar of its own width: a floating one covers the rows' remove buttons.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            let mono = egui::TextStyle::Monospace.resolve(ui.style());

            egui::CollapsingHeader::new(RichText::new("VARIABLES").small().strong()).default_open(true).show(
                ui,
                |ui| match &self.debug.session {
                    Some(s) if !s.scopes.is_empty() => {
                        for (name, reference) in &s.scopes {
                            if s.lazy_scopes.contains(reference) && !s.vars.contains_key(reference) {
                                if ui.link(RichText::new(format!("{name}: show")).small()).clicked() {
                                    expand.push(*reference);
                                }
                                continue;
                            }
                            ui.label(RichText::new(name).color(pal.text_dim));
                            let out = VarOut {
                                toggle: &mut toggle,
                                expand: &mut expand,
                                memory_at: &mut memory_at,
                                data_at: &mut data_at,
                            };
                            var_rows(ui, s, *reference, name, 1, &mono, out);
                        }
                    }
                    _ => {
                        ui.label(RichText::new("Not paused.").color(pal.text_dim));
                    }
                },
            );

            egui::CollapsingHeader::new(RichText::new("WATCH").small().strong()).default_open(true).show(ui, |ui| {
                for (i, expr) in self.debug.watches.iter().enumerate() {
                    let value = self.debug.session.as_ref().and_then(|s| s.watch_values.get(i).cloned().flatten());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(expr).font(mono.clone()));
                        ui.label(
                            RichText::new(format!("= {}", value.unwrap_or_else(|| "…".into())))
                                .font(mono.clone())
                                .color(pal.accent),
                        );
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if icons::button(ui, Icon::Close, "Remove").clicked() {
                                remove_watch = Some(i);
                            }
                        });
                    });
                }
                let edit =
                    ui.add(egui::TextEdit::singleline(&mut self.debug.watch_input).hint_text("Add expression (Enter)"));
                if edit.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.debug.watch_input.trim().is_empty()
                {
                    self.debug.watches.push(std::mem::take(&mut self.debug.watch_input).trim().to_owned());
                    select = self.debug.session.as_ref().map(|s| s.selected);
                }
            });

            egui::CollapsingHeader::new(RichText::new("CALL STACK").small().strong()).default_open(true).show(
                ui,
                |ui| match &self.debug.session {
                    Some(s) if !s.frames.is_empty() => {
                        for (i, f) in s.frames.iter().enumerate() {
                            let file =
                                f.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
                            let place = file.map(|n| format!("{n}:{}", f.line)).unwrap_or_else(|| "(no source)".into());
                            let text = format!("{}   {place}", f.name);
                            let color =
                                if f.path.as_ref().is_some_and(|p| p.exists()) { pal.text } else { pal.text_dim };
                            // Long names of generic code give way, or they'd widen the whole side bar.
                            let row = egui::Button::selectable(i == s.selected, RichText::new(&text).color(color));
                            if ui.add(row.truncate()).on_hover_text(&text).clicked() {
                                select = Some(i);
                            }
                        }
                    }
                    Some(s) if s.state == RunState::Running => {
                        ui.label(RichText::new("Running.").color(pal.text_dim));
                    }
                    _ => {
                        ui.label(RichText::new("Not paused.").color(pal.text_dim));
                    }
                },
            );

            egui::CollapsingHeader::new(RichText::new("BREAKPOINTS").small().strong()).default_open(true).show(
                ui,
                |ui| {
                    if self.debug.breakpoints.is_empty() && self.debug.functions.is_empty() {
                        ui.label(RichText::new("F9 or click left of a line number to add one.").color(pal.text_dim));
                    }
                    for (path, bps) in self.debug.breakpoints.values_mut() {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        for b in bps.iter_mut() {
                            ui.horizontal(|ui| {
                                let changed = ui.checkbox(&mut b.enabled, "").changed();
                                let mut text = format!("{name}:{}", b.line + 1);
                                if let Some(c) = &b.condition {
                                    text.push_str(&format!("  if {c}"));
                                }
                                if let Some(l) = &b.log {
                                    text.push_str(&format!("  log \"{l}\""));
                                }
                                if ui.add(egui::Label::new(text).sense(Sense::click())).clicked() {
                                    goto_bp = Some((path.clone(), b.line));
                                }
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if icons::button(ui, Icon::Close, "Remove").clicked() {
                                        remove_bp = Some((path.clone(), b.line));
                                    }
                                });
                                resend |= changed;
                            });
                        }
                    }
                    for (i, (f, verified)) in self.debug.functions.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            functions_changed |= ui.checkbox(&mut f.enabled, "").changed();
                            let mut text = format!("function {}", f.name);
                            if let Some(c) = &f.condition {
                                text.push_str(&format!("  if {c}"));
                            }
                            let color = if *verified == Some(false) { pal.text_dim } else { pal.text };
                            let label = ui.label(RichText::new(text).color(color));
                            if *verified == Some(false) {
                                label.on_hover_text("The debugger found no function by this name");
                            }
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icons::button(ui, Icon::Close, "Remove").clicked() {
                                    remove_function = Some(i);
                                }
                            });
                        });
                    }
                    for (i, d) in self.debug.data.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let mut on = d.enabled;
                            if ui.checkbox(&mut on, "").changed() {
                                change_data = Some((i, Some(on)));
                            }
                            let color = if d.verified == Some(false) { pal.text_dim } else { pal.text };
                            ui.label(RichText::new(format!("when {} changes", d.description)).color(color));
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icons::button(ui, Icon::Close, "Remove").clicked() {
                                    change_data = Some((i, None));
                                }
                            });
                        });
                    }
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.debug.function_input)
                            .hint_text("Break on a function, e.g. main (Enter)"),
                    );
                    if edit.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && !self.debug.function_input.trim().is_empty()
                    {
                        let name = std::mem::take(&mut self.debug.function_input).trim().to_owned();
                        if !self.debug.functions.iter().any(|(f, _)| f.name == name) {
                            let f = FunctionBreakpoint { name, condition: None, enabled: true };
                            self.debug.functions.push((f, None));
                        }
                        functions_changed = true;
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Export…").on_hover_text("Save every breakpoint to a file").clicked() {
                            export = true;
                        }
                        if ui.small_button("Import…").on_hover_text("Add the breakpoints in a file").clicked() {
                            import = true;
                        }
                    });
                },
            );

            if !self.debug.filters.is_empty() {
                egui::CollapsingHeader::new(RichText::new("EXCEPTIONS").small().strong()).default_open(true).show(
                    ui,
                    |ui| {
                        for f in &self.debug.filters {
                            let mut on = self.debug.exceptions.get(&f.id).copied().unwrap_or(f.default);
                            let check = ui.checkbox(&mut on, &f.label);
                            let check = match &f.description {
                                Some(d) => check.on_hover_text(d),
                                None => check,
                            };
                            if check.changed() {
                                self.debug.exceptions.insert(f.id.clone(), on);
                                exceptions_changed = true;
                            }
                        }
                    },
                );
            }
        });

        match low_level {
            Some(true) => self.show_memory(None),
            Some(false) => self.show_disassembly(),
            None => {}
        }
        if memory_at.is_some() {
            self.show_memory(memory_at);
        }
        if let Some((reference, name)) = data_at {
            self.break_on_change(reference, &name);
        }
        if let Some(i) = remove_function {
            self.debug.functions.remove(i);
            functions_changed = true;
        }
        if functions_changed {
            self.send_function_breakpoints();
        }
        if let Some((i, enable)) = change_data {
            self.change_data_breakpoint(i, enable);
        }
        if exceptions_changed {
            self.send_exception_breakpoints();
        }
        if export {
            self.export_breakpoints();
        }
        if import {
            self.import_breakpoints();
        }
        if let Some(i) = remove_watch {
            self.debug.watches.remove(i);
            select = self.debug.session.as_ref().map(|s| s.selected);
        }
        if let Some(path) = toggle
            && let Some(s) = &mut self.debug.session
            && !s.expanded.remove(&path)
        {
            s.expanded.insert(path);
        }
        if resend {
            // A breakpoint was enabled/disabled: resend every file.
            let files: Vec<PathBuf> = self.debug.breakpoints.values().map(|(p, _)| p.clone()).collect();
            for p in files {
                self.send_breakpoints(&p);
            }
        }
        for reference in expand {
            self.request("variables", json!({ "variablesReference": reference }), Pending::Variables(reference));
        }
        if let Some(i) = select {
            self.select_frame(&ctx, i);
        }
        if let Some((path, line)) = goto_bp {
            self.open_file(&path);
            if let Some(doc) = self.active_doc() {
                doc.goto_line(line + 1);
            }
        }
        if let Some((path, line)) = remove_bp {
            if let Some((_, bps)) = self.debug.breakpoints.get_mut(&key(&path)) {
                bps.retain(|b| b.line != line);
                if bps.is_empty() {
                    self.debug.breakpoints.remove(&key(&path));
                }
            }
            self.send_breakpoints(&path);
        }
    }

    /// The Debug Console panel: program output and expression evaluation.
    pub(crate) fn debug_console(&mut self, ui: &mut egui::Ui) {
        let font = egui::FontId::monospace(self.font_size * 0.9);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let input_h = 28.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(ui.available_height() - input_h)
            .show_rows(ui, row_h, self.debug.console.len(), |ui, range| {
                for line in &self.debug.console[range] {
                    ui.label(RichText::new(line).font(font.clone()));
                }
            });
        let paused = self.stopped_thread().is_some();
        let hint = if paused { "Evaluate an expression (Enter)" } else { "Pause the program to evaluate expressions" };
        let edit = ui.add_enabled(
            paused,
            egui::TextEdit::singleline(&mut self.debug.console_input)
                .hint_text(hint)
                .desired_width(f32::INFINITY)
                .font(font),
        );
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let expr = std::mem::take(&mut self.debug.console_input);
            if !expr.trim().is_empty() {
                self.push_console(&format!("> {expr}"));
                let frame = self.debug.session.as_ref().and_then(|s| s.frames.get(s.selected).map(|f| f.id));
                self.request(
                    "evaluate",
                    json!({ "expression": expr, "frameId": frame, "context": "repl" }),
                    Pending::Repl,
                );
            }
            edit.request_focus();
        }
    }

    /// Status bar text while debugging.
    pub(crate) fn debug_status(&self) -> Option<String> {
        let s = self.debug.session.as_ref()?;
        Some(match &s.state {
            RunState::Starting => format!("Debugging {} (starting)", s.launch.name),
            RunState::Running => format!("Debugging {}", s.launch.name),
            RunState::Stopped { .. } => format!("Debugging {} (paused)", s.launch.name),
            RunState::Ended => return None,
        })
    }
}

/// "1 breakpoint", "3 breakpoints".
fn breakpoints_count(n: usize) -> String {
    if n == 1 { "1 breakpoint".to_owned() } else { format!("{n} breakpoints") }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

/// What clicks on variable rows ask for.
struct VarOut<'a> {
    /// A row opened or closed, by path.
    toggle: &'a mut Option<String>,
    /// Containers whose children should be read.
    expand: &'a mut Vec<i64>,
    /// Memory to show.
    memory_at: &'a mut Option<String>,
    /// A variable to break on when it changes: (container, name).
    data_at: &'a mut Option<(i64, String)>,
}

/// Variable rows under `reference` (at `path`), recursively for expanded
/// ones. Expanded rows whose children aren't loaded yet go into `expand`.
fn var_rows(
    ui: &mut egui::Ui,
    s: &Session,
    reference: i64,
    path: &str,
    depth: usize,
    mono: &egui::FontId,
    out: VarOut,
) {
    let VarOut { toggle, expand, memory_at, data_at } = out;
    // gdb's watchpoints take an expression: the names at the top are enough.
    let watchable = s.support.data || (s.launch.kind == DebugKind::Gdb && depth == 1);
    let pal = theme::colors();
    let Some(vars) = s.vars.get(&reference) else {
        let requested = s.pending.values().any(|p| matches!(p, Pending::Variables(r) if *r == reference));
        if !requested && !expand.contains(&reference) {
            expand.push(reference);
        }
        ui.label(RichText::new("…").color(pal.text_dim));
        return;
    };
    for v in vars {
        let child = format!("{path}/{}", v.name);
        let open = v.reference > 0 && s.expanded.contains(&child);
        let row = crate::ui::inspector::Row {
            name: &v.name,
            value: &v.value,
            type_name: &v.ty,
            depth,
            expandable: v.reference > 0,
            open,
        };
        let resp = crate::ui::inspector::row(ui, &row, mono, |ui| {
            if let Some(address) = v.memory.as_ref().filter(|_| s.capabilities.read_memory)
                && ui.small_button("mem").on_hover_text(format!("Memory at {address}")).clicked()
            {
                *memory_at = Some(address.clone());
            }
            if watchable
                && ui.small_button("on change").on_hover_text(format!("Break when {} changes", v.name)).clicked()
            {
                *data_at = Some((reference, v.name.clone()));
            }
        });
        if resp.clicked() && v.reference > 0 {
            *toggle = Some(child.clone());
        }
        if open {
            let out = VarOut {
                toggle: &mut *toggle,
                expand: &mut *expand,
                memory_at: &mut *memory_at,
                data_at: &mut *data_at,
            };
            var_rows(ui, s, v.reference, &child, depth + 1, mono, out);
        }
    }
}
