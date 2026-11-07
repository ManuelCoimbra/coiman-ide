//! Runs plugins: WebAssembly components (the `plugin` world in
//! `wit/plugin.wit`) in Wasmtime, on a worker thread so a slow plugin never
//! blocks the editor.
//!
//! The sandbox: a plugin can only call the functions of the `host`
//! interface (no WASI, so no files, network, clock or environment of its
//! own); file and editor access go through permission checks and stay
//! inside the open folder; each call has a time limit and each plugin a
//! memory limit. A plugin that traps or runs out of time is stopped.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::path::{Component as PathPart, Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use super::manifest::{EventKind, Manifest, Permission};
use crate::lsp::discovery::quiet_command;
use crate::lsp::servers::which;

wasmtime::component::bindgen!({
    path: "wit/plugin.wit",
    world: "plugin",
});

pub use rust_code::plugin::types::{
    Block, Button, Decoration, Document, JobResult, Problem, ServerResponse, Severity, Span, Table, TreeItem,
};

/// How often the time-limit clock ticks, and how many ticks a call may take.
const TICK: Duration = Duration::from_millis(10);
const CALL_TICKS: u64 = 200;
/// Memory a plugin may use.
const MEMORY_LIMIT: usize = 64 << 20;
const MAX_READ_BYTES: u64 = 8 << 20;
const MAX_LIST: u32 = 10_000;
/// Jobs one plugin may have running, and output kept from each stream.
const MAX_JOBS: usize = 8;
const MAX_JOB_OUTPUT: u64 = 16 << 20;
const JOB_POLL: Duration = Duration::from_millis(20);

/// What plugins ask the editor to show or do.
#[derive(Clone, Debug)]
pub enum Update {
    Activated {
        plugin: String,
    },
    /// Loading, activating or a call failed; the plugin was stopped.
    Stopped {
        plugin: String,
        error: String,
    },
    Status {
        plugin: String,
        id: String,
        text: String,
        tooltip: String,
    },
    Tree {
        plugin: String,
        view: String,
        items: Vec<TreeItem>,
    },
    Panel {
        plugin: String,
        panel: String,
        blocks: Vec<Block>,
    },
    Log {
        plugin: String,
        line: String,
    },
    Message {
        plugin: String,
        text: String,
    },
    Open {
        path: PathBuf,
        line: u32,
    },
    Insert {
        plugin: String,
        text: String,
    },
    /// `path` is relative to the open folder; "" means the active editor.
    Decorations {
        plugin: String,
        path: String,
        items: Vec<Decoration>,
    },
    Problems {
        plugin: String,
        path: String,
        items: Vec<Problem>,
    },
    Coverage {
        plugin: String,
        path: String,
        lines: Vec<crate::coverage::LineCoverage>,
    },
    /// A request for the active file's language server.
    ServerRequest {
        plugin: String,
        id: u64,
        method: String,
        params: String,
    },
}

/// Editor state plugins may read, kept current by the editor.
#[derive(Default)]
pub struct Context {
    pub workspace: Option<PathBuf>,
    pub document: Option<Document>,
}

enum Msg {
    Load {
        manifest: Box<Manifest>,
        wasm: Vec<u8>,
        granted: BTreeSet<Permission>,
        programs: BTreeSet<String>,
    },
    Grant {
        plugin: String,
        granted: BTreeSet<Permission>,
    },
    GrantPrograms {
        plugin: String,
        programs: BTreeSet<String>,
    },
    /// An event for one plugin: a job it started, an answer it asked for.
    Deliver {
        plugin: String,
        event: Event,
    },
    Unload(String),
    Command {
        plugin: String,
        id: String,
        argument: String,
    },
    Event(EventKind, Event),
}

/// The editor's handle to the plugin worker.
pub struct PluginHost {
    tx: Sender<Msg>,
    updates: Receiver<Update>,
    context: Arc<Mutex<Context>>,
}

impl PluginHost {
    /// Starts the worker; `wake` is called whenever an update is waiting.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let mut config = Config::new();
        config.epoch_interruption(true).wasm_component_model(true).max_wasm_stack(1 << 20);
        let engine = Engine::new(&config).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let context = Arc::new(Mutex::new(Context::default()));
        let worker = Worker {
            engine: engine.clone(),
            plugins: HashMap::new(),
            updates: update_tx,
            wake: Arc::new(wake),
            context: context.clone(),
            messages: tx.clone(),
        };
        let running = Arc::new(AtomicBool::new(true));
        let ticking = running.clone();
        std::thread::spawn(move || {
            while ticking.load(Ordering::Relaxed) {
                std::thread::sleep(TICK);
                engine.increment_epoch();
            }
        });
        std::thread::spawn(move || {
            worker.run(rx);
            running.store(false, Ordering::Relaxed);
        });
        Ok(Self { tx, updates, context })
    }

    pub fn load(&self, manifest: Manifest, wasm: Vec<u8>, granted: BTreeSet<Permission>) {
        self.load_with_programs(manifest, wasm, granted, BTreeSet::new());
    }

    pub fn load_with_programs(
        &self,
        manifest: Manifest,
        wasm: Vec<u8>,
        granted: BTreeSet<Permission>,
        programs: BTreeSet<String>,
    ) {
        let _ = self.tx.send(Msg::Load { manifest: Box::new(manifest), wasm, granted, programs });
    }

    /// Changes which programs a running plugin may start.
    pub fn grant_programs(&self, plugin: &str, programs: BTreeSet<String>) {
        let _ = self.tx.send(Msg::GrantPrograms { plugin: plugin.to_owned(), programs });
    }

    /// Gives a plugin the language server's answer to its request.
    pub fn server_answered(&self, plugin: &str, response: ServerResponse) {
        let event = Event::ServerAnswered(response);
        let _ = self.tx.send(Msg::Deliver { plugin: plugin.to_owned(), event });
    }

    /// Changes what a running plugin may do.
    pub fn grant(&self, plugin: &str, granted: BTreeSet<Permission>) {
        let _ = self.tx.send(Msg::Grant { plugin: plugin.to_owned(), granted });
    }

    pub fn unload(&self, plugin: &str) {
        let _ = self.tx.send(Msg::Unload(plugin.to_owned()));
    }

    pub fn run_command(&self, plugin: &str, id: &str, argument: &str) {
        let _ =
            self.tx.send(Msg::Command { plugin: plugin.to_owned(), id: id.to_owned(), argument: argument.to_owned() });
    }

    pub fn event(&self, kind: EventKind, event: Event) {
        let _ = self.tx.send(Msg::Event(kind, event));
    }

    pub fn poll(&self) -> Vec<Update> {
        self.updates.try_iter().collect()
    }

    pub fn set_workspace(&self, root: Option<PathBuf>) {
        if let Ok(mut c) = self.context.lock() {
            c.workspace = root;
        }
    }

    pub fn set_document(&self, document: Option<Document>) {
        if let Ok(mut c) = self.context.lock() {
            c.document = document;
        }
    }
}

/// The store data of one plugin: what host calls see.
struct State {
    plugin: String,
    manifest: Manifest,
    granted: BTreeSet<Permission>,
    programs: BTreeSet<String>,
    context: Arc<Mutex<Context>>,
    updates: Sender<Update>,
    wake: Arc<dyn Fn() + Send + Sync>,
    limits: StoreLimits,
    messages: Sender<Msg>,
    /// Programs running for this plugin; stopped when it is.
    jobs: Arc<Mutex<HashMap<u64, Arc<Mutex<Child>>>>>,
    next_id: u64,
}

impl Drop for State {
    fn drop(&mut self) {
        for child in self.jobs.lock().map(|jobs| jobs.values().cloned().collect::<Vec<_>>()).unwrap_or_default() {
            if let Ok(mut child) = child.lock() {
                crate::lsp::discovery::kill_tree(&mut child);
            }
        }
    }
}

/// Keeps what a program prints, up to the limit, and goes on reading the rest so
/// it never blocks on a full pipe.
fn read_output(mut from: impl Read + Send + 'static) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let _ = (&mut from).take(MAX_JOB_OUTPUT).read_to_end(&mut kept);
        let _ = std::io::copy(&mut from, &mut std::io::sink());
        String::from_utf8_lossy(&kept).into_owned()
    })
}

struct Loaded {
    store: Store<State>,
    bindings: Plugin,
}

struct Worker {
    engine: Engine,
    plugins: HashMap<String, Loaded>,
    updates: Sender<Update>,
    wake: Arc<dyn Fn() + Send + Sync>,
    context: Arc<Mutex<Context>>,
    messages: Sender<Msg>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        for msg in rx {
            match msg {
                Msg::Load { manifest, wasm, granted, programs } => self.load(*manifest, &wasm, granted, programs),
                Msg::GrantPrograms { plugin, programs } => {
                    if let Some(p) = self.plugins.get_mut(&plugin) {
                        p.store.data_mut().programs = programs;
                    }
                }
                Msg::Deliver { plugin, event } => {
                    self.call(&plugin, |b, s| b.call_on_event(s, &event).map(|()| None));
                }
                Msg::Grant { plugin, granted } => {
                    if let Some(p) = self.plugins.get_mut(&plugin) {
                        p.store.data_mut().granted = granted;
                    }
                }
                Msg::Unload(id) => {
                    self.plugins.remove(&id);
                }
                Msg::Command { plugin, id, argument } => {
                    self.call(&plugin, |b, s| b.call_run_command(s, &id, &argument).map(|r| r.err()));
                }
                Msg::Event(kind, event) => {
                    let ids: Vec<String> = self
                        .plugins
                        .iter()
                        .filter(|(_, p)| p.store.data().manifest.events.contains(&kind))
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in ids {
                        self.call(&id, |b, s| b.call_on_event(s, &event).map(|()| None));
                    }
                }
            }
        }
    }

    fn send(&self, update: Update) {
        let _ = self.updates.send(update);
        (self.wake)();
    }

    fn load(&mut self, manifest: Manifest, wasm: &[u8], granted: BTreeSet<Permission>, programs: BTreeSet<String>) {
        let id = manifest.id.clone();
        self.plugins.remove(&id);
        let loaded = (|| -> wasmtime::Result<Loaded> {
            let component = Component::new(&self.engine, wasm)?;
            let mut linker = Linker::<State>::new(&self.engine);
            Plugin::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s)?;
            let state = State {
                plugin: id.clone(),
                manifest,
                granted,
                programs,
                messages: self.messages.clone(),
                jobs: Arc::default(),
                next_id: 1,
                context: self.context.clone(),
                updates: self.updates.clone(),
                wake: self.wake.clone(),
                limits: StoreLimitsBuilder::new()
                    .memory_size(MEMORY_LIMIT)
                    .instances(16)
                    .tables(16)
                    .memories(4)
                    .build(),
            };
            let mut store = Store::new(&self.engine, state);
            store.limiter(|s| &mut s.limits);
            store.set_epoch_deadline(CALL_TICKS);
            store.epoch_deadline_trap();
            let bindings = Plugin::instantiate(&mut store, &component, &linker)?;
            Ok(Loaded { store, bindings })
        })();
        match loaded {
            Ok(p) => {
                self.plugins.insert(id.clone(), p);
                if self.call(&id, |b, s| b.call_activate(s).map(|r| r.err())) {
                    self.send(Update::Activated { plugin: id });
                }
            }
            Err(e) => self.send(Update::Stopped { plugin: id, error: describe(&e) }),
        }
    }

    /// Calls into a plugin with a fresh time limit. An `Err` the plugin
    /// returns is shown; a trap or timeout stops the plugin. Returns whether
    /// the plugin is still running.
    fn call(
        &mut self,
        id: &str,
        f: impl FnOnce(&Plugin, &mut Store<State>) -> wasmtime::Result<Option<String>>,
    ) -> bool {
        let Some(p) = self.plugins.get_mut(id) else { return false };
        p.store.set_epoch_deadline(CALL_TICKS);
        match f(&p.bindings, &mut p.store) {
            Ok(None) => true,
            Ok(Some(err)) => {
                self.send(Update::Message { plugin: id.to_owned(), text: err });
                true
            }
            Err(e) => {
                self.plugins.remove(id);
                self.send(Update::Stopped { plugin: id.to_owned(), error: describe(&e) });
                false
            }
        }
    }
}

fn describe(e: &wasmtime::Error) -> String {
    if let Some(trap) = e.downcast_ref::<wasmtime::Trap>() {
        return match trap {
            wasmtime::Trap::Interrupt => {
                format!("took longer than {} s and was stopped", CALL_TICKS * TICK.as_millis() as u64 / 1000)
            }
            other => format!("crashed: {other}"),
        };
    }
    let text = format!("{e:#}");
    if text.contains("rust-code:plugin/") {
        return format!("was built for an older plugin API and needs rebuilding: {text}");
    }
    if text.contains("import") && text.contains("wasi") {
        return format!("asks for system access (WASI) that plugins don't get: {text}");
    }
    text
}

impl State {
    fn send(&self, update: Update) {
        let _ = self.updates.send(update);
        (self.wake)();
    }

    fn need(&self, p: Permission) -> Result<(), String> {
        if self.granted.contains(&p) { Ok(()) } else { Err(format!("permission denied: {}", p.name())) }
    }

    fn workspace(&self) -> Result<PathBuf, String> {
        self.context.lock().ok().and_then(|c| c.workspace.clone()).ok_or_else(|| "no folder is open".to_owned())
    }

    /// `rel` inside the open folder, never outside it (no `..`, no absolute
    /// paths, no symlinks pointing out).
    fn resolve(&self, rel: &str, must_exist: bool) -> Result<PathBuf, String> {
        let root = self.workspace()?;
        let rel_path = Path::new(rel);
        if rel.is_empty() || !rel_path.components().all(|c| matches!(c, PathPart::Normal(_) | PathPart::CurDir)) {
            return Err(format!("\"{rel}\" isn't a path inside the open folder"));
        }
        let path = root.join(rel_path);
        let canon_root = root.canonicalize().map_err(|e| e.to_string())?;
        let check = if must_exist { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_default() };
        let canon = check.canonicalize().map_err(|_| format!("{rel}: not found"))?;
        if !canon.starts_with(&canon_root) {
            return Err(format!("\"{rel}\" isn't a path inside the open folder"));
        }
        Ok(path)
    }

    fn declared(&self, kind: &str, id: &str, ids: impl IntoIterator<Item = String>) -> bool {
        if ids.into_iter().any(|d| d == id) {
            return true;
        }
        self.send(Update::Log {
            plugin: self.plugin.clone(),
            line: format!("{kind} \"{id}\" isn't declared in plugin.toml"),
        });
        false
    }
}

impl rust_code::plugin::types::Host for State {}

impl rust_code::plugin::host::Host for State {
    fn log(&mut self, message: String) {
        self.send(Update::Log { plugin: self.plugin.clone(), line: message });
    }

    fn show_message(&mut self, message: String) {
        self.send(Update::Message { plugin: self.plugin.clone(), text: message });
    }

    fn set_status_item(&mut self, id: String, text: String, tooltip: String) {
        let ids = self.manifest.status_items.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        if self.declared("status item", &id, ids) {
            self.send(Update::Status { plugin: self.plugin.clone(), id, text, tooltip });
        }
    }

    fn set_tree_items(&mut self, view: String, items: Vec<TreeItem>) {
        let ids = self.manifest.tree_views.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        if self.declared("tree view", &view, ids) {
            self.send(Update::Tree { plugin: self.plugin.clone(), view, items });
        }
    }

    fn set_panel(&mut self, panel: String, blocks: Vec<Block>) {
        let ids = self.manifest.panels.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        if self.declared("panel", &panel, ids) {
            self.send(Update::Panel { plugin: self.plugin.clone(), panel, blocks });
        }
    }

    fn set_decorations(&mut self, path: String, items: Vec<Decoration>) {
        self.send(Update::Decorations { plugin: self.plugin.clone(), path, items });
    }

    fn set_problems(&mut self, path: String, items: Vec<Problem>) {
        self.send(Update::Problems { plugin: self.plugin.clone(), path, items });
    }

    fn set_coverage(&mut self, path: String, lines: Vec<rust_code::plugin::types::LineCoverage>) {
        let lines = lines
            .into_iter()
            .map(|l| crate::coverage::LineCoverage { line: l.line, hits: l.hits, partial: l.partial })
            .collect();
        self.send(Update::Coverage { plugin: self.plugin.clone(), path, lines });
    }

    fn open_location(&mut self, path: String, line: u32) -> Result<(), String> {
        let path = self.resolve(&path, true)?;
        self.send(Update::Open { path, line });
        Ok(())
    }

    fn read_file(&mut self, path: String) -> Result<String, String> {
        self.need(Permission::WorkspaceRead)?;
        let full = self.resolve(&path, true)?;
        let len = std::fs::metadata(&full).map_err(|e| format!("{path}: {e}"))?.len();
        if len > MAX_READ_BYTES {
            return Err(format!("{path}: larger than {} MB", MAX_READ_BYTES >> 20));
        }
        let bytes = std::fs::read(&full).map_err(|e| format!("{path}: {e}"))?;
        String::from_utf8(bytes).map_err(|_| format!("{path}: not UTF-8 text"))
    }

    fn list_files(&mut self, suffix: String, limit: u32) -> Result<Vec<String>, String> {
        self.need(Permission::WorkspaceRead)?;
        let root = self.workspace()?;
        let limit = limit.min(MAX_LIST) as usize;
        let mut out = Vec::new();
        for entry in ignore::WalkBuilder::new(&root).build().filter_map(Result::ok) {
            if out.len() >= limit {
                break;
            }
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if rel.ends_with(&suffix) {
                out.push(rel);
            }
        }
        out.sort();
        Ok(out)
    }

    fn write_file(&mut self, path: String, contents: String) -> Result<(), String> {
        self.need(Permission::WorkspaceWrite)?;
        let full = self.resolve(&path, false)?;
        std::fs::write(&full, contents).map_err(|e| format!("{path}: {e}"))
    }

    fn active_document(&mut self) -> Result<Option<Document>, String> {
        self.need(Permission::EditorRead)?;
        Ok(self.context.lock().ok().and_then(|c| c.document.clone()))
    }

    fn insert_text(&mut self, text: String) -> Result<(), String> {
        self.need(Permission::EditorWrite)?;
        self.send(Update::Insert { plugin: self.plugin.clone(), text });
        Ok(())
    }

    fn start_job(&mut self, program: String, args: Vec<String>, stdin: String) -> Result<u64, String> {
        if !self.programs.contains(&program) {
            let line = format!("refused: {program} isn't approved");
            self.send(Update::Log { plugin: self.plugin.clone(), line });
            return Err(format!("program not approved: {program}"));
        }
        let root = self.workspace()?;
        if !crate::trust::is_trusted(&root) {
            return Err("this folder isn't trusted, so plugins can't start programs in it".to_owned());
        }
        let jobs = self.jobs.clone();
        if jobs.lock().map_err(|e| e.to_string())?.len() >= MAX_JOBS {
            return Err(format!("this plugin already has {MAX_JOBS} programs running"));
        }
        let exe = which(&program).ok_or_else(|| format!("{program} isn't on PATH"))?;
        let mut child = quiet_command(&exe)
            .args(&args)
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start {program}: {e}"))?;
        // In the log, so what a plugin ran can always be checked.
        let line = format!("started: {} {}", exe.display(), args.join(" "));
        self.send(Update::Log { plugin: self.plugin.clone(), line });
        let id = self.next_id;
        self.next_id += 1;
        let (out, err) = (child.stdout.take(), child.stderr.take());
        if let Some(mut input) = child.stdin.take() {
            std::thread::spawn(move || {
                let _ = input.write_all(stdin.as_bytes());
            });
        }
        let out = out.map(read_output);
        let err = err.map(read_output);
        let child = Arc::new(Mutex::new(child));
        jobs.lock().map_err(|e| e.to_string())?.insert(id, child.clone());
        let (messages, plugin) = (self.messages.clone(), self.plugin.clone());
        std::thread::spawn(move || {
            let status = loop {
                let polled = child.lock().map(|mut c| c.try_wait());
                match polled {
                    Ok(Ok(Some(status))) => break Some(status),
                    Ok(Ok(None)) => std::thread::sleep(JOB_POLL),
                    _ => break None,
                }
            };
            if let Ok(mut jobs) = jobs.lock() {
                jobs.remove(&id);
            }
            let text = |handle: Option<JoinHandle<String>>| handle.and_then(|h| h.join().ok()).unwrap_or_default();
            let result =
                JobResult { id, exit_code: status.and_then(|s| s.code()), stdout: text(out), stderr: text(err) };
            let _ = messages.send(Msg::Deliver { plugin, event: Event::JobFinished(result) });
        });
        Ok(id)
    }

    fn cancel_job(&mut self, id: u64) {
        let child = self.jobs.lock().ok().and_then(|jobs| jobs.get(&id).cloned());
        if let Some(child) = child
            && let Ok(mut child) = child.lock()
        {
            crate::lsp::discovery::kill_tree(&mut child);
        }
    }

    fn language_server_request(&mut self, method: String, params: String) -> Result<u64, String> {
        self.need(Permission::LanguageServer)?;
        serde_json::from_str::<serde_json::Value>(&params).map_err(|e| format!("params aren't JSON: {e}"))?;
        let id = self.next_id;
        self.next_id += 1;
        self.send(Update::ServerRequest { plugin: self.plugin.clone(), id, method, params });
        Ok(id)
    }
}
