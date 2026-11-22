//! Plugins in the editor: the Extensions view (installed plugins, updates,
//! the registry), the consent prompt before anything is installed, and the
//! contributions plugins fill with data: status bar items, panel tabs, tree
//! views in the Explorer and Command Palette commands. Plugins run in
//! `coiman_core::extensions::plugin::PluginHost`; nothing is fetched from a registry
//! unless the user asks.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::extensions::plugin::host::ServerResponse;
use coiman_core::extensions::plugin::manifest::{Alignment, Manifest, Permission};
use coiman_core::extensions::plugin::registry::{self, Entry, Package};
use coiman_core::extensions::plugin::store::{self, Lockfile, UpdateDiff};
use coiman_core::extensions::plugin::{
    Block, Decoration, Document, Event, EventKind, PluginHost, Problem, Severity, TerminalLine, TreeItem, Update,
};
use coiman_core::language::lsp::types as lt;
use egui::{Align, RichText, Sense};

use crate::app::lsp::path_key;
use crate::app::{CodeApp, PanelTab, SidebarView, repaint};
use crate::ui::editor_view::{Decorations, Highlight};

const WARNING: egui::Color32 = egui::Color32::from_rgb(0xCC, 0xA7, 0x00);
const ERROR: egui::Color32 = egui::Color32::from_rgb(0xF1, 0x4C, 0x4C);
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

/// Extensions that ship with Coiman IDE: sandboxed plugins like any other, with
/// no download. They can be turned off in the Extensions view.
const BUILT_IN: [(&str, &[u8]); 2] = [
    (
        include_str!("../../../../../plugins/built-in/comment-highlights/plugin.toml"),
        include_bytes!("../../../../../plugins/built-in/comment-highlights/plugin.wasm"),
    ),
    (
        include_str!("../../../../../plugins/built-in/trailing-spaces/plugin.toml"),
        include_bytes!("../../../../../plugins/built-in/trailing-spaces/plugin.wasm"),
    ),
];

/// Ids of those. An extension that used to ship with the app can be uninstalled.
const BUILT_IN_IDS: [&str; 2] = ["coiman.comment-highlights", "coiman.trailing-spaces"];

/// Extensions that used to ship with the app and are now part of it: Color Preview is
/// `editor.color_decorators`. A copy that came with the app goes; one the user
/// installed stays.
const RETIRED: [&str; 1] = ["rust-code.color-preview"];

/// Built-in extensions' ids before the rename to Coiman IDE, and now. The old copy goes,
/// and the new one keeps its on/off choice and permissions.
const RENAMED: [(&str, &str); 2] = [
    ("rust-code.comment-highlights", "coiman.comment-highlights"),
    ("rust-code.trailing-spaces", "coiman.trailing-spaces"),
];

/// `Locked::source` of a built-in extension.
const BUILT_IN_SOURCE: &str = "built-in";

/// Seconds after the last edit before plugins are told.
const EDIT_DELAY: f64 = 0.15;

pub(crate) struct StatusItem {
    plugin: String,
    id: String,
    alignment: Alignment,
    command: Option<String>,
    text: String,
    tooltip: String,
}

pub(crate) struct PluginPanel {
    plugin: String,
    id: String,
    pub(crate) title: String,
    blocks: Vec<Block>,
}

pub(crate) struct PluginTree {
    plugin: String,
    id: String,
    title: String,
    items: Vec<TreeItem>,
    open: bool,
    expanded: HashSet<String>,
}

/// An installed plugin as the Extensions view shows it.
struct Info {
    id: String,
    manifest: Option<Manifest>,
    /// Why it isn't running (load error, crash, files changed).
    problem: Option<String>,
    running: bool,
    log: Vec<String>,
    show_log: bool,
}

/// What a registry check found.
struct Checked {
    available: Vec<Entry>,
    updates: Vec<UpdateDiff>,
    /// Updates that couldn't be previewed.
    problems: Vec<String>,
}

enum Job {
    /// The registry's latest versions, and updates for installed plugins.
    Index(Result<Box<Checked>, String>),
    Downloaded {
        entry: Box<Entry>,
        result: Result<Box<Package>, String>,
        diff: Option<Box<UpdateDiff>>,
    },
    /// A dev mode build has ended.
    DevBuilt(Box<dev::Built>),
}

#[path = "plugin_dev.rs"]
mod dev;

#[path = "plugin_pick.rs"]
mod pick;

#[path = "plugin_graph.rs"]
mod graph;

#[path = "plugin_packs.rs"]
pub(crate) mod packs;

/// Waiting for the user to approve an install or update.
struct Consent {
    pkg: Package,
    source: String,
    /// Publisher key (empty: unsigned folder install).
    key: String,
    trusted: bool,
    diff: Option<UpdateDiff>,
    granted: BTreeSet<Permission>,
    programs: BTreeSet<String>,
}

#[derive(Default)]
pub(crate) struct PluginState {
    started: bool,
    host: Option<PluginHost>,
    dir: Option<PathBuf>,
    lock: Lockfile,
    infos: Vec<Info>,
    pub(crate) status_items: Vec<StatusItem>,
    pub(crate) panels: Vec<PluginPanel>,
    pub(crate) trees: Vec<PluginTree>,
    available: Vec<Entry>,
    updates: Vec<UpdateDiff>,
    registry_checked: bool,
    registry_error: Option<String>,
    /// Background work in progress ("Checking the registry…").
    busy: Option<String>,
    jobs: Option<(Sender<Job>, Receiver<Job>)>,
    consent: Option<Consent>,
    workspace: Option<PathBuf>,
    last_active: Option<Option<PathBuf>>,
    last_doc: Option<(u64, u64)>,
    /// A change plugins haven't heard about: (doc uid, version, when seen).
    edit_pending: Option<(u64, u64, f64)>,
    /// Coloring and problems by (plugin, file key).
    decorations: HashMap<(String, PathBuf), Vec<Decoration>>,
    /// All plugins' coloring per file key, by line.
    highlights: HashMap<PathBuf, Arc<HashMap<usize, Vec<Highlight>>>>,
    problems: HashMap<(String, PathBuf), (PathBuf, Vec<lt::Diagnostic>)>,
    /// Dev mode, while a plugin folder is being worked on.
    dev: Option<dev::DevMode>,
    /// Plugins: New Plugin...'s form while it's open.
    new_plugin: Option<dev::NewPlugin>,
    /// A plugin's pick list on screen.
    pick: Option<pick::Shown>,
    /// A plugin's graph in its window.
    graph: Option<graph::Shown>,
    /// Below the terminal's cursor when Ctrl+Space was last pressed there.
    terminal_at: Option<egui::Pos2>,
    /// Extensions that come with Coiman, not installed until asked: (folder, manifest).
    shipped: Option<Vec<(PathBuf, Manifest)>>,
    /// The shipped files' hashes by folder: (plugin.wasm, plugin.toml).
    shipped_hashes: HashMap<PathBuf, (String, String)>,
    /// The project's Python interpreter, as the plugins were last told.
    python: Option<PathBuf>,
    /// Plugin forms' fields: (plugin, form, field) to (the value the plugin last sent, what
    /// the field holds now).
    forms: HashMap<(String, String, String), (String, String)>,
}

/// `Locked::source` prefix of an extension that came with Coiman and was installed on request.
const SHIPPED_SOURCE: &str = "shipped:";

/// What a terminal's command line looks like to plugins.
fn terminal_line(l: &crate::ui::terminal::LineAtCursor) -> TerminalLine {
    TerminalLine { terminal: l.tab, typed: l.typed.clone(), dir: l.dir.to_string_lossy().into_owned() }
}

/// Folders holding the extensions that come with Coiman: `extensions` beside the program
/// (or one level up, in a package), and `plugins/extensions` in a source checkout.
fn shipped_dirs() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let mut dirs = Vec::new();
    for dir in exe.ancestors().skip(1).take(5) {
        for candidate in
            [dir.join("extensions"), dir.join("share/coiman-ide/extensions"), dir.join("plugins/extensions")]
        {
            if candidate.is_dir() && !dirs.contains(&candidate) {
                dirs.push(candidate);
            }
        }
    }
    dirs
}

/// The extensions found there, each once (the first found wins).
fn shipped_extensions() -> Vec<(PathBuf, Manifest)> {
    let mut found: Vec<(PathBuf, Manifest)> = Vec::new();
    for dir in shipped_dirs() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut folders: Vec<PathBuf> =
            read.flatten().map(|e| e.path()).filter(|p| p.join("plugin.wasm").is_file()).collect();
        folders.sort();
        for folder in folders {
            let Ok(text) = std::fs::read_to_string(folder.join("plugin.toml")) else { continue };
            let Ok(m) = coiman_core::extensions::plugin::manifest::parse(&text) else { continue };
            if !found.iter().any(|(_, f)| f.id == m.id) {
                found.push((folder, m));
            }
        }
    }
    found
}

/// Command Palette key for a plugin command.
fn command_key(plugin: &str, command: &str) -> String {
    format!("{plugin}\n{command}")
}

impl CodeApp {
    fn relative(&self, path: &Path) -> String {
        self.explorer
            .root
            .as_ref()
            .and_then(|r| path.strip_prefix(r).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default()
    }

    fn plugin_name(&self, id: &str) -> String {
        self.plugins
            .infos
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.manifest.as_ref())
            .map_or_else(|| id.to_owned(), |m| m.name.clone())
    }

    fn jobs(&mut self) -> Sender<Job> {
        self.plugins.jobs.get_or_insert_with(mpsc::channel).0.clone()
    }

    /// Loads the lockfile and starts every enabled plugin (once).
    fn plugins_init(&mut self, ctx: &egui::Context) {
        self.plugins.started = true;
        if !self.settings.plugins.enabled {
            return;
        }
        let Some(dir) = store::plugins_dir() else { return };
        match Lockfile::load(&dir) {
            Ok(lock) => self.plugins.lock = lock,
            Err(e) => self.status = Some(format!("Plugins: {e}")),
        }
        self.install_built_ins(&dir);
        self.plugins.dir = Some(dir);
        self.refresh_infos();
        let ids: Vec<String> = self.plugins.lock.plugins.iter().filter(|p| p.enabled).map(|p| p.id.clone()).collect();
        for id in ids {
            self.load_plugin(ctx, &id);
        }
    }

    /// Puts the built-in extensions in the store, new or updated. Each one's
    /// on/off choice and permissions are kept.
    fn install_built_ins(&mut self, dir: &Path) {
        let mut changed = false;
        for id in RETIRED {
            let bundled = self.plugins.lock.get(id).is_some_and(|l| l.source == BUILT_IN_SOURCE);
            if bundled && store::uninstall(dir, &mut self.plugins.lock, id).is_ok() {
                coiman_core::applog::info(&format!("retired the built-in extension {id}: now part of the editor"));
                changed = true;
            }
        }
        // What the user chose for each extension under its old name.
        let mut before_rename = std::collections::HashMap::new();
        for (old, new) in RENAMED {
            let Some(was) = self.plugins.lock.get(old).filter(|l| l.source == BUILT_IN_SOURCE).cloned() else {
                continue;
            };
            if store::uninstall(dir, &mut self.plugins.lock, old).is_ok() {
                coiman_core::applog::info(&format!("the built-in extension {old} is now {new}"));
                before_rename.insert(new, was);
                changed = true;
            }
        }
        for (toml, wasm) in BUILT_IN {
            let Ok(manifest) = coiman_core::extensions::plugin::manifest::parse(toml) else { continue };
            let wasm_hash = registry::sha256_hex(wasm);
            let locked = self.plugins.lock.get(&manifest.id);
            if locked.is_some_and(|l| {
                l.wasm_sha256 == wasm_hash && l.manifest_sha256 == registry::sha256_hex(toml.as_bytes())
            }) {
                continue;
            }
            let locked = locked.or_else(|| before_rename.get(manifest.id.as_str()));
            let granted = match locked {
                Some(l) => l.granted.clone(),
                None => manifest.permissions.clone(),
            };
            let enabled = locked.is_none_or(|l| l.enabled);
            let id = manifest.id.clone();
            let pkg = Package { manifest, manifest_bytes: toml.as_bytes().to_vec(), wasm: wasm.to_vec() };
            let programs = locked.map(|l| l.granted_programs.clone()).unwrap_or_default();
            if store::install(dir, &mut self.plugins.lock, &pkg, BUILT_IN_SOURCE, "", granted, programs).is_ok() {
                if let Some(l) = self.plugins.lock.get_mut(&id) {
                    l.enabled = enabled;
                }
                changed = true;
            }
        }
        if changed && let Err(e) = self.plugins.lock.save(dir) {
            self.status = Some(format!("Plugins: {e}"));
        }
    }

    /// Plugin coloring for document `i`.
    /// Tells the editor which languages the enabled plugins add.
    fn sync_plugin_languages(&self) {
        let enabled = self.plugins.infos.iter().filter(|i| self.plugins.lock.get(&i.id).is_some_and(|l| l.enabled));
        let decls: Vec<_> = enabled.filter_map(|i| i.manifest.as_ref()).flat_map(|m| m.languages.clone()).collect();
        coiman_core::language::languages::set(&decls);
    }

    pub(crate) fn plugin_decorate(&self, i: usize, deco: &mut Decorations) {
        self.sync_plugin_languages();
        if let Some(path) = self.docs.get(i).and_then(|d| d.path.as_deref()) {
            deco.highlights.extend(self.plugins.highlights.get(&path_key(path)).cloned());
        }
    }

    /// Plugins' problems for a file key.
    pub(crate) fn plugin_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        let matching = self.plugins.problems.iter().filter(|((_, k), _)| k == key);
        matching.flat_map(|(_, (_, items))| items.iter().cloned()).collect()
    }

    /// Files with plugin problems: (key, path).
    pub(crate) fn plugin_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.plugins.problems.iter().map(|((_, k), (p, _))| (k.clone(), p.clone())).collect()
    }

    /// The file a plugin named: relative to the open folder, "" for the active editor.
    fn plugin_path(&self, path: &str) -> Option<PathBuf> {
        if path.is_empty() {
            self.docs.get(self.active)?.path.clone()
        } else {
            self.explorer.root.as_ref().map(|r| r.join(path))
        }
    }

    /// Rebuilds the merged coloring of a file from every plugin's.
    fn rebuild_highlights(&mut self, key: &Path) {
        let mut lines: HashMap<usize, Vec<Highlight>> = HashMap::new();
        for ((_, k), items) in &self.plugins.decorations {
            if k != key {
                continue;
            }
            for d in items {
                let color =
                    |c: Option<u32>| c.map(|v| egui::Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
                lines.entry(d.line.saturating_sub(1) as usize).or_default().push(Highlight {
                    start: d.start as usize,
                    end: d.end as usize,
                    foreground: color(d.foreground),
                    background: color(d.background),
                    italic: d.italic,
                    strike: false,
                    frame: None,
                });
            }
        }
        if lines.is_empty() {
            self.plugins.highlights.remove(key);
        } else {
            self.plugins.highlights.insert(key.to_path_buf(), Arc::new(lines));
        }
    }

    fn refresh_infos(&mut self) {
        let Some(dir) = self.plugins.dir.clone() else { return };
        let old = std::mem::take(&mut self.plugins.infos);
        for locked in &self.plugins.lock.plugins {
            let prev = old.iter().find(|i| i.id == locked.id);
            let (manifest, problem) = match store::load(&dir, locked) {
                Ok(pkg) => (Some(pkg.manifest), prev.and_then(|p| p.problem.clone())),
                Err(e) => (None, Some(e)),
            };
            self.plugins.infos.push(Info {
                id: locked.id.clone(),
                manifest,
                problem,
                running: prev.is_some_and(|p| p.running),
                log: prev.map(|p| p.log.clone()).unwrap_or_default(),
                show_log: prev.is_some_and(|p| p.show_log),
            });
        }
    }

    fn load_plugin(&mut self, ctx: &egui::Context, id: &str) {
        let (Some(dir), Some(locked)) = (self.plugins.dir.clone(), self.plugins.lock.get(id).cloned()) else { return };
        let pkg = match store::load(&dir, &locked) {
            Ok(pkg) => pkg,
            Err(e) => {
                if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == id) {
                    info.problem = Some(e);
                }
                return;
            }
        };
        if self.plugins.host.is_none() {
            match PluginHost::start(repaint(ctx)) {
                Ok(host) => {
                    host.set_workspace(self.explorer.root.clone());
                    self.plugins.workspace = self.explorer.root.clone();
                    self.plugins.host = Some(host);
                }
                Err(e) => {
                    self.status = Some(format!("Plugins could not start: {e}"));
                    return;
                }
            }
        }
        self.remove_contributions(id);
        let m = &pkg.manifest;
        for s in &m.status_items {
            self.plugins.status_items.push(StatusItem {
                plugin: m.id.clone(),
                id: s.id.clone(),
                alignment: s.alignment,
                command: s.command.clone(),
                text: String::new(),
                tooltip: String::new(),
            });
        }
        for p in &m.panels {
            self.plugins.panels.push(PluginPanel {
                plugin: m.id.clone(),
                id: p.id.clone(),
                title: p.title.clone(),
                blocks: Vec::new(),
            });
        }
        for t in &m.tree_views {
            self.plugins.trees.push(PluginTree {
                plugin: m.id.clone(),
                id: t.id.clone(),
                title: t.title.clone(),
                items: Vec::new(),
                open: true,
                expanded: HashSet::new(),
            });
        }
        if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == id) {
            info.problem = None;
        }
        // The document snapshot is sent again for the new plugin.
        self.plugins.last_doc = None;
        if let Some(host) = &self.plugins.host {
            host.load_with_programs(pkg.manifest, pkg.wasm, locked.granted, locked.granted_programs);
        }
    }

    fn unload_plugin(&mut self, id: &str) {
        if let Some(host) = &self.plugins.host {
            host.unload(id);
        }
        self.remove_contributions(id);
        if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == id) {
            info.running = false;
        }
    }

    fn remove_contributions(&mut self, id: &str) {
        let keys: Vec<PathBuf> =
            self.plugins.decorations.keys().filter(|(p, _)| p == id).map(|(_, k)| k.clone()).collect();
        self.plugins.decorations.retain(|(p, _), _| p != id);
        for key in keys {
            self.rebuild_highlights(&key);
        }
        self.plugins.problems.retain(|(p, _), _| p != id);
        self.plugins.status_items.retain(|s| s.plugin != id);
        self.plugins.trees.retain(|t| t.plugin != id);
        let before = self.plugins.panels.len();
        self.plugins.panels.retain(|p| p.plugin != id);
        if before != self.plugins.panels.len() && matches!(self.panel_tab, PanelTab::Plugin(_)) {
            self.panel_tab = PanelTab::Output;
        }
    }

    fn save_lock(&mut self) {
        if let Some(dir) = &self.plugins.dir
            && let Err(e) = self.plugins.lock.save(dir)
        {
            self.status = Some(format!("Plugins: {e}"));
        }
    }

    /// Each frame: plugin updates, background jobs, editor events.
    pub(crate) fn pump_plugins(&mut self, ctx: &egui::Context) {
        if !self.plugins.started {
            self.plugins_init(ctx);
        }
        self.pump_jobs(ctx);
        self.pump_terminal_events();
        let Some(host) = &self.plugins.host else { return };
        let python = self.python_env().map(|e| e.python.clone());
        if self.plugins.python != python {
            host.set_python(python.clone());
            self.plugins.python = python;
        }
        if self.plugins.workspace != self.explorer.root {
            self.plugins.workspace = self.explorer.root.clone();
            host.set_workspace(self.explorer.root.clone());
        }
        let updates = host.poll();
        for u in updates {
            self.on_plugin_update(ctx, u);
        }
        // The document snapshot for `editor-read`, once typing pauses, then
        // the events: plugins read the snapshot when they hear about a change.
        let now = ctx.input(|i| i.time);
        let key = self.docs.get(self.active).map(|d| (d.uid, d.buffer.version()));
        let switched = self.plugins.last_doc.map(|k| k.0) != key.map(|k| k.0);
        let mut edited = false;
        if key != self.plugins.last_doc {
            let settled = match (self.plugins.edit_pending, key) {
                (Some((uid, version, since)), Some((u, v))) if uid == u && version == v => now - since >= EDIT_DELAY,
                (_, Some((u, v))) => {
                    self.plugins.edit_pending = Some((u, v, now));
                    false
                }
                _ => true,
            };
            if switched || settled {
                edited = !switched;
                self.plugins.edit_pending = None;
                self.plugins.last_doc = key;
                self.sync_plugin_languages();
                let doc = self.docs.get(self.active).map(|d| {
                    let (line, column) = d.cursor();
                    // Very large files stay out of plugins.
                    let text = if d.large { String::new() } else { d.text() };
                    Document {
                        path: d.path.as_deref().map(|p| self.relative(p)).unwrap_or_default(),
                        language: d.language_name().to_owned(),
                        text,
                        line: line as u32,
                        column: column as u32,
                        dirty: d.is_dirty(),
                    }
                });
                if let Some(host) = &self.plugins.host {
                    host.set_document(doc);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(EDIT_DELAY));
            }
        }
        let active = self.docs.get(self.active).and_then(|d| d.path.clone());
        let rel = active.as_deref().map(|p| self.relative(p)).unwrap_or_default();
        if self.plugins.last_active.as_ref() != Some(&active) {
            self.plugins.last_active = Some(active);
            if let Some(host) = &self.plugins.host {
                host.event(EventKind::ActiveChanged, Event::ActiveChanged(rel.clone()));
            }
        }
        if edited && let Some(host) = &self.plugins.host {
            host.event(EventKind::Edited, Event::Edited(rel));
        }
    }

    fn on_plugin_update(&mut self, ctx: &egui::Context, update: Update) {
        match update {
            Update::Activated { plugin } => {
                if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == plugin) {
                    info.running = true;
                    info.problem = None;
                }
                // It may have missed what's on screen while it was loading.
                self.plugins.last_doc = None;
                self.plugins.last_active = None;
            }
            Update::Decorations { plugin, path, items } => {
                if let Some(file) = self.plugin_path(&path) {
                    let key = path_key(&file);
                    if items.is_empty() {
                        self.plugins.decorations.remove(&(plugin, key.clone()));
                    } else {
                        self.plugins.decorations.insert((plugin, key.clone()), items);
                    }
                    self.rebuild_highlights(&key);
                }
            }
            Update::Problems { plugin, path, items } => {
                if let Some(file) = self.plugin_path(&path) {
                    let key = path_key(&file);
                    if items.is_empty() {
                        self.plugins.problems.remove(&(plugin, key));
                    } else {
                        let source = self.plugin_name(&plugin);
                        let diagnostics = items.iter().map(|p| diagnostic(p, &source)).collect();
                        self.plugins.problems.insert((plugin, key), (file, diagnostics));
                    }
                }
            }
            Update::Coverage { plugin, path, lines } => {
                if let Some(file) = self.plugin_path(&path) {
                    self.set_plugin_coverage(&plugin, &file, lines);
                }
            }
            Update::Stopped { plugin, error } => {
                let name = self.plugin_name(&plugin);
                self.status = Some(format!("{name} stopped: {error}"));
                self.remove_contributions(&plugin);
                if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == plugin) {
                    info.running = false;
                    info.log.push(format!("stopped: {error}"));
                    info.problem = Some(error);
                }
            }
            Update::Status { plugin, id, text, tooltip } => {
                if let Some(s) = self.plugins.status_items.iter_mut().find(|s| s.plugin == plugin && s.id == id) {
                    (s.text, s.tooltip) = (text, tooltip);
                }
            }
            Update::Tree { plugin, view, items } => {
                if let Some(t) = self.plugins.trees.iter_mut().find(|t| t.plugin == plugin && t.id == view) {
                    t.items = items;
                }
            }
            Update::Panel { plugin, panel, blocks } => {
                if let Some(i) = self.plugins.panels.iter().position(|p| p.plugin == plugin && p.id == panel) {
                    self.plugins.panels[i].blocks = blocks;
                    self.panel_visible = true;
                    self.panel_tab = PanelTab::Plugin(i);
                }
            }
            Update::Log { plugin, line } => {
                if let Some(info) = self.plugins.infos.iter_mut().find(|i| i.id == plugin) {
                    info.log.push(line);
                    let excess = info.log.len().saturating_sub(200);
                    info.log.drain(..excess);
                }
            }
            Update::Message { plugin, text } => {
                self.status = Some(format!("{}: {text}", self.plugin_name(&plugin)));
            }
            Update::Open { path, line } => {
                self.open_file(&path);
                if let Some(doc) = self.active_doc() {
                    doc.goto_line(line.max(1) as usize);
                }
            }
            Update::ServerRequest { plugin, id, method, params } => {
                self.plugin_server_request(plugin, id, &method, &params)
            }
            Update::Insert { text, .. } => {
                if let Some(doc) = self.active_doc() {
                    doc.buffer.insert(&text);
                }
            }
            Update::Pick { plugin, pick } => self.show_plugin_pick(plugin, pick),
            Update::Graph { plugin, graph } => self.show_plugin_graph(plugin, graph),
            Update::Markdown { text, .. } => self.open_markdown_page(&text),
            Update::CommitDiff { commit, path, .. } => {
                let short: String = commit.chars().take(7).collect();
                self.open_commit_file(ctx, commit, short, path);
            }
            Update::CompareDiff { base, other, path, .. } => self.open_compared_file(ctx, base, other, path),
            Update::CopyText(text) => ctx.copy_text(text),
            Update::MergeConflicts => self.show_merge_conflicts(),
            Update::OpenFolder(path) => self.open_folder(path),
            Update::RunInTerminal { title, program, args, folder, .. } => {
                let program = program.to_string_lossy().into_owned();
                let env = self.environment_variables();
                let task = crate::ui::terminal::Task {
                    title: &title,
                    program: &program,
                    args: &args,
                    env: &env,
                    cwd: &folder,
                    scan: None,
                };
                let scrollback = self.settings.terminal.scrollback;

                match self.terminal.run_task(ctx, task, scrollback) {
                    Ok(()) => {
                        self.panel_visible = true;
                        self.panel_tab = PanelTab::Terminal;
                    }
                    Err(e) => self.status = Some(format!("Couldn't start {program}: {e}")),
                }
            }
            Update::TerminalType { terminal, text, .. } => {
                self.terminal.type_into(terminal, &text);
                self.terminal.show_tab(terminal);
                self.terminal.focus_requested = true;
            }
        }
    }

    /// Whether a running plugin listens for `kind` and may see the terminals.
    fn plugin_listens(&self, kind: EventKind) -> bool {
        self.plugins.infos.iter().filter(|i| i.running).any(|i| {
            i.manifest.as_ref().is_some_and(|m| m.events.contains(&kind))
                && self.plugins.lock.get(&i.id).is_some_and(|l| l.granted.contains(&Permission::Terminal))
        })
    }

    /// The terminal's side of plugins: what was run, Ctrl+Space, and the line for commands.
    fn pump_terminal_events(&mut self) {
        self.terminal.catch_commands = self.plugin_listens(EventKind::TerminalCommand);
        self.terminal.suggest_key = self.plugin_listens(EventKind::TerminalSuggest);
        let entered = std::mem::take(&mut self.terminal.entered);
        let suggest = self.terminal.suggest.take();
        let Some(host) = &self.plugins.host else { return };
        for line in &entered {
            host.event(EventKind::TerminalCommand, Event::TerminalCommand(terminal_line(line)));
        }
        if let Some(line) = suggest {
            self.plugins.terminal_at = Some(line.at);
            host.set_terminal_line(Some(terminal_line(&line)));
            host.event(EventKind::TerminalSuggest, Event::TerminalSuggest(terminal_line(&line)));
        }
    }

    pub(crate) fn plugins_saved(&mut self, index: usize) {
        let Some(path) = self.docs.get(index).and_then(|d| d.path.clone()) else { return };
        let rel = self.relative(&path);
        if let Some(host) = &self.plugins.host {
            host.event(EventKind::Saved, Event::Saved(rel));
        }
        self.plugin_dev_saved(&path);
        self.remote_saved(&path);
    }

    pub(crate) fn plugins_opened(&mut self, path: &Path) {
        let rel = self.relative(path);
        if let Some(host) = &self.plugins.host {
            host.event(EventKind::Opened, Event::Opened(rel));
        }
    }

    /// Runs a palette entry from [`plugin_commands`](Self::plugin_commands).
    pub(crate) fn run_plugin_command(&mut self, key: &str) {
        if let Some((plugin, command)) = key.split_once('\n') {
            self.run_plugin(plugin, command, "");
        }
    }

    fn run_plugin(&mut self, plugin: &str, command: &str, argument: &str) {
        match &self.plugins.host {
            Some(host) if self.plugins.infos.iter().any(|i| i.id == plugin && i.running) => {
                // The command line as it is now, for `active-terminal-line`.
                host.set_terminal_line(self.terminal.line_at_cursor().as_ref().map(terminal_line));
                host.run_command(plugin, command, argument);
            }
            _ => self.status = Some(format!("{} isn't running", self.plugin_name(plugin))),
        }
    }

    /// Palette entries for running plugins' commands: (title, key).
    pub(crate) fn plugin_commands(&self) -> Vec<(String, String)> {
        self.plugins
            .infos
            .iter()
            .filter(|i| i.running)
            .filter_map(|i| i.manifest.as_ref())
            .flat_map(|m| m.commands.iter().map(|c| (c.title.clone(), command_key(&m.id, &c.id))))
            .collect()
    }

    // ----- registry and installs ----------------------------------------------------

    fn pump_jobs(&mut self, ctx: &egui::Context) {
        let Some((_, rx)) = &self.plugins.jobs else { return };
        let jobs: Vec<Job> = rx.try_iter().collect();
        for job in jobs {
            self.plugins.busy = None;
            match job {
                Job::Index(Ok(checked)) => {
                    let Checked { available, updates, problems } = *checked;
                    self.plugins.available = available;
                    self.plugins.updates = updates;
                    self.plugins.registry_checked = true;
                    self.plugins.registry_error = (!problems.is_empty()).then(|| problems.join("\n"));
                }
                Job::Index(Err(e)) => {
                    self.plugins.registry_checked = true;
                    self.plugins.registry_error = Some(e);
                }
                Job::Downloaded { entry, result: Ok(pkg), diff } => {
                    let granted = match self.plugins.lock.get(&entry.id) {
                        // Keep what was granted; new permissions are offered checked.
                        Some(l) => {
                            let added = diff.as_ref().map(|d| d.manifest.added_permissions.clone()).unwrap_or_default();
                            l.granted.iter().copied().chain(added).collect()
                        }
                        None => pkg.manifest.permissions.clone(),
                    };
                    // New programs always start unapproved.
                    let programs =
                        self.plugins.lock.get(&entry.id).map(|l| l.granted_programs.clone()).unwrap_or_default();
                    self.plugins.consent = Some(Consent {
                        programs,
                        trusted: self.plugins.lock.trusts(&entry.publisher_key),
                        key: entry.publisher_key.clone(),
                        source: self.settings.plugins.registry.trim().to_owned(),
                        pkg: *pkg,
                        diff: diff.map(|d| *d),
                        granted,
                    });
                    self.show_extensions();
                }
                Job::Downloaded { entry, result: Err(e), .. } => {
                    self.status = Some(format!("Could not download {}: {e}", entry.name));
                    self.plugins.registry_error = Some(e);
                }
                Job::DevBuilt(built) => self.on_dev_built(ctx, *built),
            }
            ctx.request_repaint();
        }
    }

    pub(crate) fn show_extensions(&mut self) {
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Extensions;
    }

    /// Plugins: Check for Updates — reads the registry (only when asked).
    pub(crate) fn check_registry(&mut self, ctx: &egui::Context) {
        if !self.plugins.started {
            self.plugins_init(ctx);
        }
        let base = self.settings.plugins.registry.trim().to_owned();
        if base.is_empty() {
            self.plugins.registry_error = Some("Set plugins.registry to a registry folder or URL first.".to_owned());
            self.show_extensions();
            return;
        }
        let Some(dir) = self.plugins.dir.clone().or_else(store::plugins_dir) else { return };
        let lock = self.plugins.lock.clone();
        let tx = self.jobs();
        let wake = repaint(ctx);
        self.plugins.busy = Some("Checking the registry…".to_owned());
        self.show_extensions();
        std::thread::spawn(move || {
            let result = registry::load_index(&base).map(|index| {
                let latest: Vec<Entry> = registry::latest(&index).into_iter().cloned().collect();
                let mut updates = Vec::new();
                let mut problems = Vec::new();
                for locked in lock.plugins.iter().filter(|l| l.signed() && l.source == base) {
                    let Some(entry) = latest.iter().find(|e| e.id == locked.id) else { continue };
                    if !coiman_core::extensions::plugin::manifest::is_newer(&entry.version, &locked.version) {
                        continue;
                    }
                    let diff = store::load(&dir, locked).and_then(|installed| {
                        let (next, _) = registry::fetch_manifest(&base, entry)?;
                        Ok(store::update_diff(locked, &installed, entry, &next))
                    });
                    match diff {
                        Ok(d) => updates.push(d),
                        Err(e) => problems.push(format!("{} {}: {e}", entry.id, entry.version)),
                    }
                }
                Box::new(Checked { available: latest, updates, problems })
            });
            let _ = tx.send(Job::Index(result));
            wake();
        });
    }

    fn download(&mut self, ctx: &egui::Context, entry: Entry, diff: Option<UpdateDiff>) {
        let base = self.settings.plugins.registry.trim().to_owned();
        let tx = self.jobs();
        let wake = repaint(ctx);
        self.plugins.busy = Some(format!("Downloading {} {}…", entry.name, entry.version));
        std::thread::spawn(move || {
            let result = registry::download(&base, &entry).map(Box::new);
            let _ = tx.send(Job::Downloaded { entry: Box::new(entry), result, diff: diff.map(Box::new) });
            wake();
        });
    }

    /// Plugins: Install from Folder… — an unsigned plugin under development.
    pub(crate) fn install_plugin_from_folder(&mut self, ctx: &egui::Context) {
        if !self.plugins.started {
            self.plugins_init(ctx);
        }
        let Some(folder) =
            self.file_dialog("plugin_folder").set_title("Plugin folder (plugin.toml and plugin.wasm)").pick_folder()
        else {
            return;
        };
        self.dialog_chose("plugin_folder", &folder);
        match registry::read_folder(&folder) {
            Ok(pkg) => {
                self.plugins.consent = Some(Consent {
                    programs: BTreeSet::new(),
                    granted: pkg.manifest.permissions.clone(),
                    pkg,
                    source: format!("folder:{}", folder.display()),
                    key: String::new(),
                    trusted: false,
                    diff: None,
                });
                self.show_extensions();
            }
            Err(e) => self.status = Some(format!("Not a plugin folder: {e}")),
        }
    }

    /// Passes a plugin's request to the active file's language server.
    fn plugin_server_request(&mut self, plugin: String, id: u64, method: &str, params: &str) {
        let mut params = serde_json::from_str::<serde_json::Value>(params).unwrap_or_default();
        let i = self.active;
        let sent =
            self.request_for(i, method, crate::app::lsp::Pending::Plugin { plugin: plugin.clone(), id }, |_, td| {
                if params.is_null() {
                    params = serde_json::json!({});
                }
                if params.is_object() && params.get("textDocument").is_none() {
                    params["textDocument"] = td;
                }
                params
            });
        if !sent && let Some(host) = &self.plugins.host {
            let error = Some("The active file has no language server".to_owned());
            host.server_answered(&plugin, ServerResponse { id, answer: String::new(), error });
        }
    }

    /// A language server answered a plugin.
    pub(crate) fn plugin_server_answer(&self, plugin: &str, id: u64, answer: Result<serde_json::Value, String>) {
        let Some(host) = &self.plugins.host else { return };
        let response = match answer {
            Ok(value) => ServerResponse { id, answer: value.to_string(), error: None },
            Err(e) => ServerResponse { id, answer: String::new(), error: Some(e) },
        };
        host.server_answered(plugin, response);
    }

    fn accept_consent(&mut self, ctx: &egui::Context) {
        let Some(c) = self.plugins.consent.take() else { return };
        let Some(dir) = self.plugins.dir.clone().or_else(store::plugins_dir) else { return };
        self.plugins.dir = Some(dir.clone());
        let id = c.pkg.manifest.id.clone();
        self.unload_plugin(&id);
        if let Err(e) = store::install(&dir, &mut self.plugins.lock, &c.pkg, &c.source, &c.key, c.granted, c.programs) {
            self.status = Some(format!("Could not install {}: {e}", c.pkg.manifest.name));
            return;
        }
        if !c.key.is_empty() && !c.trusted {
            self.plugins.lock.trusted_keys.push(c.key.clone());
        }
        self.save_lock();
        self.plugins.updates.retain(|u| u.id != id);
        self.refresh_infos();
        self.load_plugin(ctx, &id);
        self.status = Some(format!("Installed {} {}", c.pkg.manifest.name, c.pkg.manifest.version));
        // The pack's next extension, if it has more to install.
        if let Some(pack) = coiman_core::packs::pack_of_extension(&id)
            && self.settings.pack_on(pack.id)
            && let Some(next) = pack.extensions.iter().find(|e| self.plugins.lock.get(e).is_none())
        {
            self.offer_shipped(next);
        }
    }

    fn uninstall_plugin(&mut self, id: &str) {
        self.unload_plugin(id);
        if let Some(dir) = self.plugins.dir.clone()
            && let Err(e) = store::uninstall(&dir, &mut self.plugins.lock, id)
        {
            self.status = Some(format!("Could not uninstall: {e}"));
        }
        self.save_lock();
        self.refresh_infos();
    }

    // ----- views ------------------------------------------------------------------------

    /// The Extensions view: consent prompt, installed plugins, updates, registry.
    pub(crate) fn extensions_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        ui.horizontal(|ui| {
            ui.label(RichText::new("EXTENSIONS").small().strong());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, Icon::Refresh, "Check the registry for plugins and updates").clicked() {
                    self.check_registry(&ctx);
                }
                if icons::button(ui, Icon::FolderOpen, "Install from Folder…").clicked() {
                    self.install_plugin_from_folder(&ctx);
                }
                if icons::button(ui, Icon::ArrowDown, "Import from VS Code: themes, snippets, languages").clicked() {
                    self.import_from_vscode();
                }
                if icons::button(ui, Icon::Check, "VS Code Compatibility Report: which extensions work here, and why")
                    .clicked()
                {
                    self.vscode_compatibility(&ctx);
                }
            });
        });
        if !self.settings.plugins.enabled {
            ui.label(RichText::new("Plugins are turned off (plugins.enabled = false).").color(pal.text_dim));
            return;
        }
        if let Some(busy) = &self.plugins.busy {
            ui.label(RichText::new(busy).color(pal.text_dim));
        }
        self.plugin_dev_ui(ui);

        let mut accept = false;
        let mut cancel = false;
        if let Some(c) = &mut self.plugins.consent {
            egui::Frame::group(ui.style()).fill(pal.editor_bg).show(ui, |ui| {
                let m = &c.pkg.manifest;
                let verb = if c.diff.is_some() { "Update" } else { "Install" };
                ui.label(RichText::new(format!("{verb} {} {}?", m.name, m.version)).strong());
                if !m.description.is_empty() {
                    ui.label(RichText::new(&m.description).color(pal.text_dim));
                }
                if c.source.starts_with(SHIPPED_SOURCE) {
                    ui.label(RichText::new("Comes with Coiman IDE.").color(pal.text_dim));
                } else if c.key.is_empty() {
                    ui.label(
                        RichText::new("Unsigned: installed from a folder, its origin isn't verified.").color(WARNING),
                    );
                } else if c.trusted {
                    ui.label(format!("Signed by a trusted publisher ({})", registry::fingerprint(&c.key)));
                } else {
                    ui.label(
                        RichText::new(format!(
                            "Signed by a publisher you haven't trusted yet. Key fingerprint: {}. Installing trusts it.",
                            registry::fingerprint(&c.key)
                        ))
                        .color(WARNING),
                    );
                }
                if let Some(d) = &c.diff {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("Changes from {} to {}", d.from, d.to)).strong());
                    if d.key_changed {
                        ui.label(
                            RichText::new("Warning: signed by a different key than the installed version.")
                                .color(ERROR),
                        );
                    }
                    for p in &d.manifest.removed_permissions {
                        ui.label(format!("- no longer asks: {}", p.describe()));
                    }
                    for a in &d.manifest.added {
                        ui.label(format!("+ {a}"));
                    }
                    for r in &d.manifest.removed {
                        ui.label(format!("- {r}"));
                    }
                    let kb = |b: u64| b.div_ceil(1024);
                    ui.label(
                        RichText::new(format!("plugin.wasm: {} KB, was {} KB", kb(d.size.1), kb(d.size.0)))
                            .color(pal.text_dim),
                    );
                    if !d.changelog.is_empty() {
                        ui.label(RichText::new(&d.changelog).italics());
                    }
                }
                ui.add_space(4.0);
                if m.permissions.is_empty() {
                    ui.label("It asks for no permissions.");
                } else {
                    ui.label("It asks to:");
                    let new = c.diff.as_ref().map(|d| d.manifest.added_permissions.clone()).unwrap_or_default();
                    for p in &m.permissions {
                        let mut on = c.granted.contains(p);
                        let mark = if new.contains(p) { " (new)" } else { "" };
                        if ui.checkbox(&mut on, format!("{}{mark}", p.describe())).changed() {
                            if on {
                                c.granted.insert(*p);
                            } else {
                                c.granted.remove(p);
                            }
                        }
                    }
                    ui.label(
                        RichText::new("Unchecked permissions are refused when the plugin asks.")
                            .small()
                            .color(pal.text_dim),
                    );
                }
                if !m.programs.is_empty() {
                    ui.label(RichText::new("It also wants to start programs.").strong().color(WARNING));
                    ui.label(
                        RichText::new(
                            "A program runs outside the sandbox with all your rights: it can read and change your \
                             files and use the network. Approve only what you trust this plugin to run, in folders \
                             you've trusted. Each start is written to its log.",
                        )
                        .small()
                        .color(WARNING),
                    );
                    for program in &m.programs {
                        let mut on = c.programs.contains(program);
                        if ui.checkbox(&mut on, RichText::new(program).monospace()).changed() {
                            if on {
                                c.programs.insert(program.clone());
                            } else {
                                c.programs.remove(program);
                            }
                        }
                    }
                }
                ui.horizontal(|ui| {
                    accept = ui.button(verb).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            ui.add_space(6.0);
        }
        if accept {
            self.accept_consent(&ctx);
        }
        if cancel {
            self.plugins.consent = None;
        }

        // A floating bar would cover each plugin's uninstall button when it widens.
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.installed_section(ui, &ctx);
            self.packs_section(ui);
            self.shipped_section(ui);
            if !self.plugins.updates.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("UPDATES").small().strong());
                let mut update = None;
                for d in &self.plugins.updates {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} {} to {}", d.name, d.from, d.to));
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Update…").clicked() {
                                update = Some(d.clone());
                            }
                        });
                    });
                    if !d.manifest.added_permissions.is_empty() || !d.manifest.added_programs.is_empty() || d.key_changed {
                        ui.label(RichText::new("Asks for more, review before updating").small().color(WARNING));
                    }
                }
                if let Some(d) = update {
                    self.download(&ctx, d.entry.clone(), Some(d));
                }
            }
            ui.add_space(8.0);
            ui.label(RichText::new("REGISTRY").small().strong());
            let registry = self.settings.plugins.registry.trim();
            if registry.is_empty() {
                ui.label(
                    RichText::new("No registry set. Set plugins.registry to a folder or URL, then check it with the refresh button.")
                        .color(pal.text_dim),
                );
            } else {
                ui.label(RichText::new(registry).small().color(pal.text_dim));
                if !self.plugins.registry_checked && self.plugins.busy.is_none() {
                    ui.label(RichText::new("Not checked yet (nothing is fetched until you click refresh).").color(pal.text_dim));
                }
            }
            if let Some(e) = &self.plugins.registry_error {
                ui.label(RichText::new(e).color(ERROR));
            }
            let mut install = None;
            for e in &self.plugins.available {
                let installed = self.plugins.lock.get(&e.id).map(|l| l.version.clone());
                let broken = self.plugins.infos.iter().any(|i| i.id == e.id && i.manifest.is_none());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&e.name).strong());
                    ui.label(RichText::new(&e.version).small().color(pal.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| match &installed {
                        Some(v) if *v == e.version && broken => {
                            if ui.small_button("Reinstall…").clicked() {
                                install = Some(e.clone());
                            }
                        }
                        Some(v) if *v == e.version => {
                            ui.label(RichText::new("installed").small().color(pal.text_dim));
                        }
                        Some(_) => {}
                        None => {
                            if ui.small_button("Install…").clicked() {
                                install = Some(e.clone());
                            }
                        }
                    });
                });
                if !e.description.is_empty() {
                    ui.label(RichText::new(&e.description).small().color(pal.text_dim));
                }
            }
            if let Some(e) = install {
                self.download(&ctx, e, None);
            }
        });
    }

    /// Extensions that come with Coiman, to install when wanted: Coiman stays as light as
    /// the user keeps it.
    fn shipped_section(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let shipped = self.plugins.shipped.get_or_insert_with(shipped_extensions).clone();
        let offered: Vec<_> = shipped
            .iter()
            // Those of a pack are installed with it.
            .filter(|(_, m)| coiman_core::packs::pack_of_extension(&m.id).is_none())
            .filter(|(_, m)| {
                self.plugins.lock.get(&m.id).is_none_or(|l| {
                    l.source.starts_with(SHIPPED_SOURCE)
                        && coiman_core::extensions::plugin::manifest::is_newer(&m.version, &l.version)
                })
            })
            .collect();
        if offered.is_empty() {
            return;
        }
        ui.add_space(8.0);
        ui.label(RichText::new("COMES WITH COIMAN").small().strong());
        ui.label(RichText::new("Not installed until you want them.").small().color(pal.text_dim));
        let mut install = None;
        for (folder, m) in offered {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new(&m.name).strong());
                ui.label(RichText::new(&m.version).small().color(pal.text_dim));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let verb = if self.plugins.lock.get(&m.id).is_some() { "Update…" } else { "Install…" };
                    if ui.small_button(verb).clicked() {
                        install = Some(folder.clone());
                    }
                });
            });
            if !m.description.is_empty() {
                ui.label(RichText::new(&m.description).small().color(pal.text_dim));
            }
        }
        if let Some(folder) = install {
            match registry::read_folder(&folder) {
                Ok(pkg) => {
                    let locked = self.plugins.lock.get(&pkg.manifest.id);
                    let granted = locked.map_or_else(|| pkg.manifest.permissions.clone(), |l| l.granted.clone());
                    let programs = locked.map(|l| l.granted_programs.clone()).unwrap_or_default();
                    self.plugins.consent = Some(Consent {
                        programs,
                        granted,
                        source: format!("{SHIPPED_SOURCE}{}", folder.display()),
                        pkg,
                        key: String::new(),
                        trusted: false,
                        diff: None,
                    });
                }
                Err(e) => self.status = Some(format!("Could not read {}: {e}", folder.display())),
            }
        }
    }

    fn installed_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = theme::colors();
        ui.label(RichText::new("INSTALLED").small().strong());
        if self.plugins.infos.is_empty() {
            ui.label(RichText::new("No plugins installed.").color(pal.text_dim));
        }
        enum Action {
            Enable(String, bool),
            Grant(String, Permission, bool),
            GrantProgram(String, String, bool),
            Uninstall(String),
            Update(String),
        }
        let mut actions = Vec::new();
        let ids: Vec<String> = self.plugins.infos.iter().map(|i| i.id.clone()).collect();
        let changed: Vec<String> = ids.into_iter().filter(|id| self.shipped_differs(id)).collect();
        for info in &mut self.plugins.infos {
            let Some(locked) = self.plugins.lock.get(&info.id) else { continue };
            ui.separator();
            ui.horizontal(|ui| {
                let name = info.manifest.as_ref().map_or(info.id.as_str(), |m| m.name.as_str());
                ui.label(RichText::new(name).strong());
                ui.label(RichText::new(&locked.version).small().color(pal.text_dim));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let removable = locked.source != BUILT_IN_SOURCE || !BUILT_IN_IDS.contains(&info.id.as_str());
                    if removable && icons::button(ui, Icon::Trash, "Uninstall").clicked() {
                        actions.push(Action::Uninstall(info.id.clone()));
                    }
                    if changed.contains(&info.id)
                        && ui
                            .small_button("Update…")
                            .on_hover_text("The copy that comes with Coiman has changed since it was installed")
                            .clicked()
                    {
                        actions.push(Action::Update(info.id.clone()));
                    }
                    let mut enabled = locked.enabled;
                    if ui.checkbox(&mut enabled, "").on_hover_text("Enabled").changed() {
                        actions.push(Action::Enable(info.id.clone(), enabled));
                    }
                });
            });
            let state = match (&info.problem, info.running, locked.enabled) {
                (Some(p), _, _) => RichText::new(p).color(ERROR),
                (None, true, _) => RichText::new("running").color(pal.text_dim),
                (None, false, false) => RichText::new("disabled").color(pal.text_dim),
                (None, false, true) => RichText::new("starting…").color(pal.text_dim),
            };
            ui.label(state.small());
            let origin = if locked.source == BUILT_IN_SOURCE {
                "built in".to_owned()
            } else if locked.source.starts_with(SHIPPED_SOURCE) {
                "came with Coiman IDE".to_owned()
            } else if locked.signed() {
                format!("signed, key {}", registry::fingerprint(&locked.publisher_key))
            } else {
                format!("unsigned, {}", locked.source)
            };
            ui.label(RichText::new(origin).small().color(pal.text_dim));
            if let Some(m) = &info.manifest {
                for p in &m.permissions {
                    let mut on = locked.granted.contains(p);
                    if ui.checkbox(&mut on, RichText::new(p.describe()).small()).changed() {
                        actions.push(Action::Grant(info.id.clone(), *p, on));
                    }
                }
                for program in &m.programs {
                    let mut on = locked.granted_programs.contains(program);
                    let label = RichText::new(format!("May start {program} (outside the sandbox)")).small();
                    if ui.checkbox(&mut on, label).changed() {
                        actions.push(Action::GrantProgram(info.id.clone(), program.clone(), on));
                    }
                }
            }
            if !info.log.is_empty() {
                let label = if info.show_log { "Hide log" } else { "Show log" };
                if ui.small_button(label).clicked() {
                    info.show_log = !info.show_log;
                }
                if info.show_log {
                    for line in info.log.iter().rev().take(50).rev() {
                        ui.label(RichText::new(line).small().monospace());
                    }
                }
            }
        }
        for action in actions {
            match action {
                Action::Enable(id, on) => {
                    if let Some(l) = self.plugins.lock.get_mut(&id) {
                        l.enabled = on;
                    }
                    self.save_lock();
                    if on {
                        self.load_plugin(ctx, &id);
                    } else {
                        self.unload_plugin(&id);
                    }
                }
                Action::Grant(id, p, on) => {
                    let granted = self.plugins.lock.get_mut(&id).map(|l| {
                        if on {
                            l.granted.insert(p);
                        } else {
                            l.granted.remove(&p);
                        }
                        l.granted.clone()
                    });
                    self.save_lock();
                    if let (Some(host), Some(g)) = (&self.plugins.host, granted) {
                        host.grant(&id, g);
                    }
                    self.plugins.last_doc = None;
                }
                Action::GrantProgram(id, program, on) => {
                    let programs = self.plugins.lock.get_mut(&id).map(|l| {
                        if on {
                            l.granted_programs.insert(program);
                        } else {
                            l.granted_programs.remove(&program);
                        }
                        l.granted_programs.clone()
                    });
                    self.save_lock();
                    if let (Some(host), Some(programs)) = (&self.plugins.host, programs) {
                        host.grant_programs(&id, programs);
                    }
                }
                Action::Uninstall(id) => self.uninstall_plugin(&id),
                Action::Update(id) => self.offer_shipped(&id),
            }
        }
    }

    /// Status bar items on one side; clicking one runs its command.
    pub(crate) fn plugin_status_items(&mut self, ui: &mut egui::Ui, side: Alignment, size: f32) {
        let mut run = None;
        for s in self.plugins.status_items.iter().filter(|s| s.alignment == side && !s.text.is_empty()) {
            let text = RichText::new(&s.text).size(size).color(theme::colors().status_fg);
            let mut resp = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
            if !s.tooltip.is_empty() {
                resp = resp.on_hover_text(&s.tooltip);
            }
            if resp.clicked()
                && let Some(c) = &s.command
            {
                run = Some((s.plugin.clone(), c.clone()));
            }
        }
        if let Some((plugin, command)) = run {
            self.run_plugin(&plugin, &command, "");
        }
    }

    /// A plugin panel tab: its blocks drawn with the editor's widgets.
    pub(crate) fn plugin_panel(&mut self, ui: &mut egui::Ui, index: usize) {
        let pal = theme::colors();
        let Some(panel) = self.plugins.panels.get(index) else { return };
        let mut run = None;
        let mut open_at: Option<(String, u32)> = None;
        let mut forms = std::mem::take(&mut self.plugins.forms);
        let mut submitted: Option<coiman_core::extensions::plugin::FormValues> = None;
        let rgb = |c: u32| egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if panel.blocks.is_empty() {
                ui.label(RichText::new("Nothing to show yet.").color(pal.text_dim));
            }
            for (index, block) in panel.blocks.iter().enumerate() {
                match block {
                    Block::Heading(t) => {
                        ui.label(RichText::new(t).strong().size(self.font_size * 1.1));
                    }
                    Block::Text(t) => {
                        ui.label(t);
                    }
                    Block::Code(t) => {
                        egui::Frame::new().fill(pal.editor_bg).inner_margin(4.0).show(ui, |ui| {
                            ui.label(RichText::new(t).monospace());
                        });
                    }
                    Block::Bullets(items) => {
                        for item in items {
                            ui.label(format!("•  {item}"));
                        }
                    }
                    Block::Separator => {
                        ui.separator();
                    }
                    Block::Spans(spans) => {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for span in spans {
                                let mut text = RichText::new(&span.text);
                                if let Some(rgb) = span.color {
                                    text = text.color(egui::Color32::from_rgb(
                                        (rgb >> 16) as u8,
                                        (rgb >> 8) as u8,
                                        rgb as u8,
                                    ));
                                }
                                if span.bold {
                                    text = text.strong();
                                }
                                if span.monospace {
                                    text = text.monospace();
                                }
                                ui.label(text);
                            }
                        });
                    }
                    Block::Table(table) => {
                        egui::Grid::new(("plugin_table", index)).striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
                            for header in &table.headers {
                                ui.label(RichText::new(header).strong());
                            }
                            ui.end_row();
                            for row in &table.rows {
                                for cell in row {
                                    ui.label(cell);
                                }
                                ui.end_row();
                            }
                        });
                    }
                    Block::Diff(text) => {
                        egui::Frame::new().fill(pal.editor_bg).inner_margin(4.0).show(ui, |ui| {
                            for line in text.lines() {
                                let color = if line.starts_with('+') && !line.starts_with("+++") {
                                    egui::Color32::from_rgb(0x89, 0xD1, 0x85)
                                } else if line.starts_with('-') && !line.starts_with("---") {
                                    egui::Color32::from_rgb(0xF1, 0x4C, 0x4C)
                                } else {
                                    pal.text
                                };
                                ui.label(RichText::new(line).monospace().color(color));
                            }
                        });
                    }
                    Block::Button(b) => {
                        if ui.button(&b.label).clicked() {
                            run = Some((b.command.clone(), b.argument.clone()));
                        }
                    }
                    Block::CommitGraph(graph) => {
                        // The graph shares the panel with what follows it.
                        let last = index + 1 == panel.blocks.len();
                        let room = ui.clip_rect().height().max(200.0);
                        let height = if last { room - 8.0 } else { (room * 0.6).max(160.0) };
                        if let Some(r) = commit_graph_ui(ui, index, graph, height) {
                            run = Some(r);
                        }
                    }
                    Block::Form(form) => {
                        if let Some(values) = form_ui(ui, &panel.plugin, form, &mut forms) {
                            submitted = Some(values);
                        }
                    }
                    Block::Links(links) => {
                        for l in links {
                            ui.horizontal(|ui| {
                                if ui.link(RichText::new(&l.text).monospace()).clicked() {
                                    open_at = Some((l.path.clone(), l.line));
                                }
                                if !l.detail.is_empty() {
                                    ui.label(RichText::new(&l.detail).color(pal.text_dim));
                                }
                            });
                        }
                    }
                    Block::Rows(rows) => {
                        // Rows without cells are a bar of buttons.
                        if rows.iter().all(|r| r.cells.is_empty()) {
                            ui.horizontal_wrapped(|ui| {
                                for b in rows.iter().flat_map(|r| &r.buttons) {
                                    if ui.button(&b.label).clicked() {
                                        run = Some((b.command.clone(), b.argument.clone()));
                                    }
                                }
                            });
                            continue;
                        }
                        let columns = rows.iter().map(|r| r.cells.len() + usize::from(!r.buttons.is_empty())).max();
                        egui::Grid::new(("plugin_rows", index))
                            .striped(true)
                            .spacing([12.0, 3.0])
                            .num_columns(columns.unwrap_or(1))
                            .show(ui, |ui| {
                                for row in rows {
                                    for span in &row.cells {
                                        let mut text = RichText::new(&span.text);
                                        if let Some(c) = span.color {
                                            text = text.color(rgb(c));
                                        }
                                        if span.bold {
                                            text = text.strong();
                                        }
                                        if span.monospace {
                                            text = text.monospace();
                                        }
                                        ui.label(text);
                                    }
                                    if !row.buttons.is_empty() {
                                        ui.horizontal(|ui| {
                                            for b in &row.buttons {
                                                if ui.small_button(&b.label).clicked() {
                                                    run = Some((b.command.clone(), b.argument.clone()));
                                                }
                                            }
                                        });
                                    }
                                    ui.end_row();
                                }
                            });
                    }
                }
            }
        });
        let plugin = panel.plugin.clone();
        self.plugins.forms = forms;
        if let Some(values) = submitted
            && let Some(host) = &self.plugins.host
        {
            host.deliver(&plugin, Event::FormSubmitted(values));
        }
        if let Some((command, argument)) = run {
            self.run_plugin(&plugin, &command, &argument);
        }
        if let Some((path, line)) = open_at
            && let Some(root) = self.explorer.root.clone()
        {
            self.open_file(&root.join(path));
            self.jump_to_symbol_line(line.saturating_sub(1) as usize);
        }
    }

    /// Plugin tree views, as sections at the bottom of the Explorer.
    pub(crate) fn plugin_tree_sections(&mut self, ui: &mut egui::Ui) {
        let mut run = None;
        // A plugin with nothing to show takes no room.
        for tree in self.plugins.trees.iter_mut().filter(|t| !t.items.is_empty()) {
            let open = tree.open;
            let panel = egui::Panel::bottom(egui::Id::unique(("plugin_tree", &tree.plugin, &tree.id)))
                .resizable(open)
                .frame(egui::Frame::NONE)
                .show_separator_line(true);
            let panel = if open { panel.default_size(160.0).size_range(60.0..=500.0) } else { panel.exact_size(26.0) };
            panel.show(ui, |ui| {
                let header = ui
                    .horizontal(|ui| {
                        let size = ui.text_style_height(&egui::TextStyle::Body);
                        let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
                        let a =
                            icons::show(ui, chevron, size, theme::colors().text_dim).interact(Sense::click()).clicked();
                        let title = RichText::new(tree.title.to_uppercase()).small().strong();
                        let b = ui.add(egui::Label::new(title).sense(Sense::click())).clicked();
                        a || b
                    })
                    .inner;
                if header {
                    tree.open = !tree.open;
                }
                if open {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        tree_rows(ui, &tree.items, &mut tree.expanded, "", 0, &mut |item: &TreeItem| {
                            if let Some(c) = &item.command {
                                run = Some((tree.plugin.clone(), c.clone(), item.id.clone()));
                            }
                        });
                    });
                }
            });
        }
        if let Some((plugin, command, argument)) = run {
            self.run_plugin(&plugin, &command, &argument);
        }
    }
}

/// A plugin's form on one line: its fields (keeping what was typed) and buttons. What it holds
/// when a button is pressed, or Enter in a text field (the first button).
fn form_ui(
    ui: &mut egui::Ui,
    plugin: &str,
    form: &coiman_core::extensions::plugin::Form,
    state: &mut HashMap<(String, String, String), (String, String)>,
) -> Option<coiman_core::extensions::plugin::FormValues> {
    use coiman_core::extensions::plugin::FieldKind;
    let mut pressed: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        for field in &form.fields {
            let key = (plugin.to_owned(), form.id.clone(), field.id.clone());
            let entry = state.entry(key).or_insert_with(|| (field.value.clone(), field.value.clone()));
            // The plugin sent another value: it wants the field to hold it.
            if entry.0 != field.value {
                *entry = (field.value.clone(), field.value.clone());
            }
            let labelled = !matches!(field.kind, FieldKind::Check | FieldKind::Button | FieldKind::LineBreak);
            if !field.label.is_empty() && labelled {
                ui.label(&field.label);
            }
            match field.kind {
                FieldKind::LineBreak => {
                    ui.end_row();
                    continue;
                }
                FieldKind::Button => {
                    if ui.small_button(&field.label).clicked() {
                        pressed = Some(field.id.clone());
                    }
                    continue;
                }
                FieldKind::Text => {
                    let width = if field.hint.len() > 24 { 300.0 } else { 200.0 };
                    let r =
                        ui.add(egui::TextEdit::singleline(&mut entry.1).hint_text(&field.hint).desired_width(width));
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        pressed = form.buttons.first().cloned();
                    }
                }
                FieldKind::Check => {
                    let mut on = entry.1 == "true";
                    if ui.checkbox(&mut on, &field.label).changed() {
                        entry.1 = on.to_string();
                    }
                }
                FieldKind::Choice => {
                    egui::ComboBox::from_id_salt((plugin, &form.id, &field.id)).selected_text(entry.1.clone()).show_ui(
                        ui,
                        |ui| {
                            for o in &field.options {
                                ui.selectable_value(&mut entry.1, o.clone(), o);
                            }
                        },
                    );
                }
            }
        }
        for b in &form.buttons {
            if ui.button(b).clicked() {
                pressed = Some(b.clone());
            }
        }
    });
    let button = pressed?;
    let values = form
        .fields
        .iter()
        .map(|f| {
            let key = (plugin.to_owned(), form.id.clone(), f.id.clone());
            (f.id.clone(), state.get(&key).map(|v| v.1.clone()).unwrap_or_default())
        })
        .collect();
    Some(coiman_core::extensions::plugin::FormValues { form: form.id.clone(), button, values })
}

/// A color for a lane of a commit graph: the accent, a few more, then the bracket colors.
fn lane_color(lane: usize) -> egui::Color32 {
    let pal = theme::colors();
    let more = [
        pal.accent,
        egui::Color32::from_rgb(0x57, 0xC0, 0x6A),
        egui::Color32::from_rgb(0xE0, 0x9A, 0x3E),
        egui::Color32::from_rgb(0xC7, 0x7D, 0xBB),
    ];
    let all: Vec<egui::Color32> = more.into_iter().chain(pal.rainbow).collect();
    all[lane % all.len()]
}

/// A plugin's commit graph: each commit's lines and node, its branches and tags, subject and
/// detail; a click or a menu item gives (command, the row's id).
fn commit_graph_ui(
    ui: &mut egui::Ui,
    index: usize,
    graph: &coiman_core::extensions::plugin::CommitGraph,
    height: f32,
) -> Option<(String, String)> {
    use egui::{Color32, Pos2, Stroke, Vec2};
    const ROW_H: f32 = 24.0;
    const LANE_W: f32 = 14.0;
    const LANES: u32 = 24;
    let pal = theme::colors();
    let mut run = None;
    let width = ui.available_width();
    let widest = graph
        .rows
        .iter()
        .flat_map(|r| r.top.iter().chain(&r.bottom).flat_map(|&(a, b)| [a, b]).chain([r.lane]))
        .max()
        .map_or(1, |m| m + 1)
        .min(LANES);
    let graph_w = widest as f32 * LANE_W + 8.0;
    egui::ScrollArea::vertical()
        .id_salt(("plugin_commit_graph", index))
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_H, graph.rows.len(), |ui, range| {
            for row in &graph.rows[range] {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                if !graph.selected.is_empty() && graph.selected == row.id {
                    ui.painter().rect_filled(rect, 4.0, pal.accent.gamma_multiply(0.18));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, pal.hover_bg);
                }
                let x = |lane: u32| rect.left() + 6.0 + lane as f32 * LANE_W;
                let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
                let painter = ui.painter();
                let fits = |&&(a, b): &&(u32, u32)| a < LANES && b < LANES;
                for &(a, b) in row.top.iter().filter(fits) {
                    let stroke = Stroke::new(1.6, lane_color(a as usize));
                    painter.line_segment([Pos2::new(x(a), top), Pos2::new(x(b), mid)], stroke);
                }
                for &(a, b) in row.bottom.iter().filter(fits) {
                    let stroke = Stroke::new(1.6, lane_color(b as usize));
                    painter.line_segment([Pos2::new(x(a), mid), Pos2::new(x(b), bottom)], stroke);
                }
                let color = lane_color(row.lane as usize);
                let node = Pos2::new(x(row.lane.min(LANES - 1)), mid);
                painter.circle(node, if row.head { 5.0 } else { 4.0 }, pal.editor_bg, Stroke::new(2.0, color));
                if row.merge {
                    painter.circle_filled(node, 2.0, color);
                }
                // Branches and tags, then the subject, then the detail at the right.
                let mut text_x = rect.left() + graph_w;
                for r in &row.refs {
                    let galley = painter.layout_no_wrap(r.name.clone(), egui::FontId::proportional(11.5), pal.text);
                    let chip = egui::Rect::from_min_size(
                        Pos2::new(text_x, mid - galley.size().y / 2.0 - 2.0),
                        galley.size() + Vec2::new(10.0, 4.0),
                    );
                    let c = if r.tag { Color32::from_rgb(0xE0, 0x9A, 0x3E) } else { color };
                    painter.rect(
                        chip,
                        8.0,
                        c.gamma_multiply(if r.current { 0.45 } else { 0.18 }),
                        Stroke::new(1.0, c.gamma_multiply(0.8)),
                        egui::StrokeKind::Inside,
                    );
                    painter.galley(chip.min + Vec2::new(5.0, 2.0), galley, pal.text);
                    text_x = chip.right() + 6.0;
                }
                let detail = painter.layout_no_wrap(row.detail.clone(), egui::FontId::proportional(11.5), pal.text_dim);
                let right_x = rect.right() - detail.size().x - 6.0;
                let subject = painter.layout(
                    row.subject.clone(),
                    egui::FontId::proportional(13.0),
                    if row.head { pal.text } else { pal.text.gamma_multiply(0.92) },
                    (right_x - text_x - 12.0).max(40.0),
                );
                let subject_y = mid - subject.size().y.min(16.0) / 2.0;
                painter
                    .with_clip_rect(egui::Rect::from_min_max(
                        Pos2::new(text_x, rect.top()),
                        Pos2::new(right_x - 8.0, rect.bottom()),
                    ))
                    .galley(Pos2::new(text_x, subject_y), subject, pal.text);
                painter.galley(Pos2::new(right_x, mid - detail.size().y / 2.0), detail, pal.text_dim);
                if resp.clicked() && !graph.click.is_empty() {
                    run = Some((graph.click.clone(), row.id.clone()));
                }
                if graph.menu.is_empty() {
                    continue;
                }
                resp.context_menu(|ui| {
                    for item in &graph.menu {
                        if item.label == "-" {
                            ui.separator();
                            continue;
                        }
                        if item.label.contains("{branch}") && row.branch.is_empty() {
                            continue;
                        }
                        let label = item.label.replace("{target}", &row.target).replace("{branch}", &row.branch);
                        if ui.button(label).clicked() {
                            run = Some((item.command.clone(), row.id.clone()));
                            ui.close();
                        }
                    }
                });
            }
        });
    run
}

fn diagnostic(p: &Problem, source: &str) -> lt::Diagnostic {
    let line = p.line.saturating_sub(1);
    lt::Diagnostic {
        range: lt::Range::new(lt::Position::new(line, p.start), lt::Position::new(line, p.end)),
        severity: Some(match p.severity {
            Severity::Error => lt::DiagnosticSeverity::ERROR,
            Severity::Warning => lt::DiagnosticSeverity::WARNING,
            Severity::Information => lt::DiagnosticSeverity::INFORMATION,
            Severity::Hint => lt::DiagnosticSeverity::HINT,
        }),
        source: Some(source.to_owned()),
        message: p.message.clone(),
        ..Default::default()
    }
}

/// Rows whose parent is `parent`, children under expanded rows.
fn tree_rows(
    ui: &mut egui::Ui,
    items: &[TreeItem],
    expanded: &mut HashSet<String>,
    parent: &str,
    depth: usize,
    clicked: &mut dyn FnMut(&TreeItem),
) {
    let pal = theme::colors();
    for item in items.iter().filter(|i| i.parent == parent) {
        let has_children = items.iter().any(|c| c.parent == item.id);
        let open =
            expanded.contains(&item.id) || (depth == 0 && has_children && !expanded.contains(&format!("-{}", item.id)));
        let resp = ui
            .horizontal(|ui| {
                ui.add_space(depth as f32 * 12.0);
                let size = ui.text_style_height(&egui::TextStyle::Body);
                if has_children {
                    icons::show(ui, if open { Icon::ChevronDown } else { Icon::ChevronRight }, size, pal.text_dim);
                } else {
                    ui.add_space(size);
                }
                // Not selectable, so a click reaches the row.
                ui.add(egui::Label::new(&item.label).selectable(false));
                if !item.description.is_empty() {
                    let description = RichText::new(&item.description).small().color(pal.text_dim);
                    ui.add(egui::Label::new(description).selectable(false));
                }
            })
            .response
            .interact(Sense::click());
        let resp = if item.tooltip.is_empty() { resp } else { resp.on_hover_text(&item.tooltip) };
        if resp.clicked() {
            if has_children {
                // Top-level rows start open; "-id" remembers one closed by the user.
                if open {
                    expanded.remove(&item.id);
                    expanded.insert(format!("-{}", item.id));
                } else {
                    expanded.remove(&format!("-{}", item.id));
                    expanded.insert(item.id.clone());
                }
            }
            clicked(item);
        }
        if open && has_children {
            tree_rows(ui, items, expanded, &item.id, depth + 1, clicked);
        }
    }
}
