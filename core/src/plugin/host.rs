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
use std::path::{Component as PathPart, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};

use super::manifest::{EventKind, Manifest, Permission};

wasmtime::component::bindgen!({
    path: "wit/plugin.wit",
    world: "plugin",
});

pub use rust_code::plugin::types::{Block, Button, Document, TreeItem};

/// How often the time-limit clock ticks, and how many ticks a call may take.
const TICK: Duration = Duration::from_millis(10);
const CALL_TICKS: u64 = 200;
/// Memory a plugin may use.
const MEMORY_LIMIT: usize = 64 << 20;
const MAX_READ_BYTES: u64 = 8 << 20;
const MAX_LIST: u32 = 10_000;

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
}

/// Editor state plugins may read, kept current by the editor.
#[derive(Default)]
pub struct Context {
    pub workspace: Option<PathBuf>,
    pub document: Option<Document>,
}

enum Msg {
    Load { manifest: Box<Manifest>, wasm: Vec<u8>, granted: BTreeSet<Permission> },
    Grant { plugin: String, granted: BTreeSet<Permission> },
    Unload(String),
    Command { plugin: String, id: String, argument: String },
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
        let _ = self.tx.send(Msg::Load { manifest: Box::new(manifest), wasm, granted });
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
    context: Arc<Mutex<Context>>,
    updates: Sender<Update>,
    wake: Arc<dyn Fn() + Send + Sync>,
    limits: StoreLimits,
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
}

impl Worker {
    fn run(mut self, rx: Receiver<Msg>) {
        for msg in rx {
            match msg {
                Msg::Load { manifest, wasm, granted } => self.load(*manifest, &wasm, granted),
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

    fn load(&mut self, manifest: Manifest, wasm: &[u8], granted: BTreeSet<Permission>) {
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
}
