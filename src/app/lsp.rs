//! Language servers: finding and starting them (asking before any download),
//! keeping them in sync with open documents, and the features built on them:
//! diagnostics, completion, hover, go to definition/implementation, find
//! references, rename, code actions and formatting.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use egui::{Align, Key, Modifiers, Pos2, Rect, RichText, Sense};
use rust_code_core::lsp::servers::{self, Found, Installer, Launch, ServerKind};
use rust_code_core::lsp::types as lt;
use rust_code_core::lsp::{Client, Event, convert, language_id};
use rust_code_core::trust;
use serde_json::{Value, json};

use crate::document::Document;
use crate::editor_view::{Decorations, Mark, Output, Severity, severity_color};
use crate::lsp_ui;
use crate::palette::{InputKind, PickerKind};
use crate::theme;
use crate::watcher::same_path;

use super::{CodeApp, PanelTab, repaint};

/// Seconds the pointer rests on a word before hover info is requested.
const HOVER_DELAY: f64 = 0.35;
/// Seconds to wait for format-on-save before saving unformatted.
const FORMAT_ON_SAVE_TIMEOUT: f64 = 2.0;
/// Completion rows visible without scrolling.
const COMPLETION_ROWS: usize = 12;

/// Where we are in finding one kind of server.
pub(super) enum Setup {
    /// Not looked for yet (no file that needs one).
    Idle,
    Probing(Receiver<Found>),
    Ready(Launch),
    Missing {
        installer: Option<Installer>,
        note: String,
    },
    Installing(Receiver<Result<(), String>>),
    /// Stopped (crashed, failed to start or install) until restarted.
    Off,
}

/// What a request in flight was for.
pub(super) enum Pending {
    SemanticTokens {
        uid: u64,
        ci: usize,
        lines: usize,
    },
    InlayHints {
        uid: u64,
        lines: usize,
    },
    SymbolHighlights {
        uid: u64,
        version: u64,
    },
    Signature {
        uid: u64,
    },
    WorkspaceSymbols,
    Completion {
        uid: u64,
    },
    /// `anchor` is the pointer position, or `None` when asked from the keyboard.
    Hover {
        uid: u64,
        pos: usize,
        anchor: Option<Pos2>,
    },
    Locations {
        title: &'static str,
        word: String,
    },
    Rename {
        name: String,
    },
    CodeActions,
    Format {
        uid: u64,
        version: u64,
        save: bool,
    },
    Execute,
}

pub(super) struct CompletionEntry {
    label: String,
    detail: String,
    item: lt::CompletionItem,
}

pub(super) struct Completion {
    uid: u64,
    /// Where the word being completed starts.
    start: usize,
    entries: Vec<CompletionEntry>,
    /// Indices into `entries` matching what has been typed, best first.
    shown: Vec<usize>,
    selected: usize,
    incomplete: bool,
    waiting: bool,
    /// (buffer version, cursor) the list was filtered for.
    filtered_for: (u64, usize),
    scroll_to_selected: bool,
}

pub(super) struct Hover {
    uid: u64,
    /// Buffer version it was made for: editing closes it.
    version: u64,
    pos: usize,
    anchor: Pos2,
    markdown: String,
    problems: Vec<(Severity, String)>,
    /// Opened from the keyboard: stays until the cursor moves or Escape.
    sticky: Option<usize>,
    rect: Option<Rect>,
}

pub(super) struct LocationItem {
    path: PathBuf,
    range: lt::Range,
    preview: String,
}

#[derive(Default)]
pub(super) struct LspState {
    /// Each kind of server's state; absent means not looked for yet.
    setups: HashMap<ServerKind, Setup>,
    /// Servers whose install offer was dismissed for this session.
    offer_dismissed: HashSet<ServerKind>,
    pub(super) clients: Vec<Client>,
    /// Requests in flight, by (server, server root, request id).
    pub(super) pending: HashMap<(ServerKind, PathBuf, i64), Pending>,
    pub(super) extra: super::lsp_extra::LspExtra,
    /// Diagnostics by file (key: [`path_key`]), with the path as reported.
    diagnostics: HashMap<PathBuf, (PathBuf, Vec<lt::Diagnostic>)>,
    /// Buffer version last sent to the server, per document uid.
    synced: HashMap<u64, u64>,
    /// Work-done progress by token: (title, latest message).
    progress: HashMap<String, (String, String)>,
    completion: Option<Completion>,
    hover: Option<Hover>,
    /// The word the pointer rests on: (doc uid, char, since, request sent).
    hover_rest: Option<(u64, usize, f64, bool)>,
    code_actions: Vec<lt::CodeActionOrCommand>,
    locations: Vec<LocationItem>,
    locations_title: String,
    /// Waiting for formatting before saving: (doc uid, started at).
    format_save: Option<(u64, f64)>,
    rename_at: Option<(u64, usize)>,
    /// Last screen rectangle of the primary cursor.
    pub(super) cursor_rect: Option<Rect>,
    /// Show the problems under the cursor after the next editor frame (F8).
    show_problem_at_cursor: bool,
}

/// Path, LSP language id and server of a document a server should handle.
pub(super) fn lsp_target(doc: &Document) -> Option<(PathBuf, &'static str, ServerKind)> {
    let lang = language_id(&doc.extension())?;
    let kind = ServerKind::for_language(lang)?;
    Some((doc.path.clone().filter(|_| !doc.large)?, lang, kind))
}

/// Whether `client` is the server for a file of `kind` at `path`.
fn serves(client: &Client, kind: ServerKind, path: &Path) -> bool {
    client.kind == kind && path.starts_with(&client.root)
}

/// Key for comparing paths (case-insensitive on Windows).
pub(super) fn path_key(path: &Path) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.to_string_lossy().to_lowercase()) } else { path.to_path_buf() }
}

/// The folder a server should be started in for `path`: the open folder if
/// it is a Cargo project containing the file, else the outermost Cargo
/// workspace above the file, else the nearest Cargo package, else its folder.
fn workspace_root(path: &Path, open_folder: Option<&Path>) -> PathBuf {
    if let Some(root) = open_folder.filter(|r| path.starts_with(r) && r.join("Cargo.toml").exists()) {
        return root.to_path_buf();
    }
    let mut nearest = None;
    let mut workspace = None;
    for dir in path.ancestors().skip(1) {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            nearest.get_or_insert_with(|| dir.to_path_buf());
            if std::fs::read_to_string(&manifest).is_ok_and(|t| t.lines().any(|l| l.trim() == "[workspace]")) {
                workspace = Some(dir.to_path_buf());
            }
        }
        if open_folder.is_some_and(|r| dir == r) {
            break;
        }
    }
    workspace.or(nearest).unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf())
}

fn severity_of(d: &lt::Diagnostic) -> Severity {
    match d.severity {
        Some(s) if s == lt::DiagnosticSeverity::WARNING => 2,
        Some(s) if s == lt::DiagnosticSeverity::INFORMATION => 3,
        Some(s) if s == lt::DiagnosticSeverity::HINT => 4,
        _ => 1,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Start of the identifier ending at `pos`.
fn word_start(doc: &Document, pos: usize) -> usize {
    let rope = doc.buffer.rope();
    let mut start = pos;
    while start > 0 && is_word_char(rope.char(start - 1)) {
        start -= 1;
    }
    start
}

/// Case-insensitive subsequence match.
fn fuzzy(text: &str, query: &str) -> bool {
    let mut chars = text.chars().flat_map(char::to_lowercase);
    query.chars().flat_map(char::to_lowercase).all(|q| chars.any(|c| c == q))
}

fn hover_markdown(value: Value) -> String {
    let Ok(Some(hover)) = serde_json::from_value::<Option<lt::Hover>>(value) else { return String::new() };
    let marked = |m: lt::MarkedString| match m {
        lt::MarkedString::String(s) => s,
        lt::MarkedString::LanguageString(l) => format!("```{}\n{}\n```", l.language, l.value),
    };
    match hover.contents {
        lt::HoverContents::Markup(m) => m.value,
        lt::HoverContents::Scalar(m) => marked(m),
        lt::HoverContents::Array(v) => v.into_iter().map(marked).collect::<Vec<_>>().join("\n\n---\n\n"),
    }
}

/// Text edits of a document edit, without annotations.
fn plain_edits(edits: &[lt::OneOf<lt::TextEdit, lt::AnnotatedTextEdit>]) -> Vec<lt::TextEdit> {
    edits
        .iter()
        .map(|e| match e {
            lt::OneOf::Left(t) => t.clone(),
            lt::OneOf::Right(a) => a.text_edit.clone(),
        })
        .collect()
}

/// One step of a workspace edit, in order.
enum Step {
    Edit(lt::Url, Vec<lt::TextEdit>),
    Create(lt::Url),
    Rename(lt::Url, lt::Url),
    Delete(lt::Url),
}

impl CodeApp {
    // ----- servers and document sync -----------------------------------------------

    /// Runs every frame: advances setup, syncs documents, handles messages.
    pub(super) fn pump_lsp(&mut self, ctx: &egui::Context) {
        if !self.settings.lsp.enabled {
            if !self.lsp.clients.is_empty() {
                self.restart_lsp();
            }
            return;
        }
        self.advance_setup(ctx);
        self.sync_documents(ctx);
        let mut batch = Vec::new();
        for client in &mut self.lsp.clients {
            for ev in client.poll() {
                batch.push((client.kind, client.root.clone(), ev));
            }
        }
        for (kind, root, ev) in batch {
            self.handle_event(kind, &root, ev);
        }
        self.lsp.clients.retain(|c| !c.has_exited());
        self.refresh_lsp_views(ctx);

        let now = ctx.input(|i| i.time);
        if let Some((uid, started)) = self.lsp.format_save {
            if now - started > FORMAT_ON_SAVE_TIMEOUT {
                self.lsp.format_save = None;
                if let Some(i) = self.doc_index(uid) {
                    self.save_doc(i);
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
    }

    /// The command configured for a server (`[lsp.paths]`, or
    /// `lsp.rust_analyzer_path` for Rust).
    fn configured_server(&self, kind: ServerKind) -> String {
        let path = self.settings.lsp.paths.get(kind.id()).cloned().unwrap_or_default();
        if path.trim().is_empty() && kind == ServerKind::RustAnalyzer {
            self.settings.lsp.rust_analyzer_path.clone()
        } else {
            path
        }
    }

    fn advance_setup(&mut self, ctx: &egui::Context) {
        // Look for the servers open documents need.
        let needed: HashSet<ServerKind> = self.docs.iter().filter_map(lsp_target).map(|(_, _, k)| k).collect();
        for kind in needed {
            if matches!(self.lsp.setups.get(&kind), None | Some(Setup::Idle)) {
                let configured = self.configured_server(kind);
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                std::thread::spawn(move || {
                    let _ = tx.send(servers::find(kind, &configured));
                    wake();
                });
                self.lsp.setups.insert(kind, Setup::Probing(rx));
            }
        }
        let kinds: Vec<ServerKind> = self.lsp.setups.keys().copied().collect();
        for kind in kinds {
            let next = match &self.lsp.setups[&kind] {
                Setup::Probing(rx) => rx.try_recv().ok().map(|found| match found {
                    Found::Server(launch) => Setup::Ready(launch),
                    Found::Missing { installer, note } => Setup::Missing { installer, note },
                }),
                Setup::Installing(rx) => match rx.try_recv() {
                    Ok(Ok(())) => {
                        self.status = Some(format!("{} installed", kind.name()));
                        Some(Setup::Idle)
                    }
                    Ok(Err(e)) => {
                        self.status = Some(format!("Could not install {}: {e}", kind.name()));
                        Some(Setup::Off)
                    }
                    Err(_) => None,
                },
                _ => None,
            };
            if let Some(next) = next {
                self.lsp.setups.insert(kind, next);
            }
        }
    }

    /// Starts servers for documents that need one and sends new text.
    fn sync_documents(&mut self, ctx: &egui::Context) {
        for i in 0..self.docs.len() {
            let Some((path, _, kind)) = lsp_target(&self.docs[i]) else { continue };
            let Some(Setup::Ready(launch)) = self.lsp.setups.get(&kind) else { continue };
            if !self.lsp.clients.iter().any(|c| serves(c, kind, &path)) {
                let launch = launch.clone();
                let (root, options) = if kind == ServerKind::RustAnalyzer {
                    let root = workspace_root(&path, self.explorer.root.as_deref());
                    (root, json!({ "check": { "command": self.settings.cargo.check_command } }))
                } else {
                    (
                        servers::root_for(kind, &path, self.explorer.root.as_deref()),
                        servers::init_options(kind, &launch),
                    )
                };
                // Servers run a project's code (build scripts, its own TypeScript).
                if !trust::is_trusted(&root) {
                    continue;
                }
                match Client::start(kind, &launch, &root, options, repaint(ctx)) {
                    Ok(client) => self.lsp.clients.push(client),
                    Err(e) => {
                        self.status = Some(format!("Could not start {}: {e}", kind.name()));
                        self.lsp.setups.insert(kind, Setup::Off);
                        continue;
                    }
                }
            }
            self.sync_doc(i);
        }
    }

    /// Sends document `i`'s latest text to its server (opening it there first).
    /// Returns the server index and the document's URI.
    fn sync_doc(&mut self, i: usize) -> Option<(usize, lt::Url)> {
        let doc = self.docs.get(i)?;
        let (path, lang, kind) = lsp_target(doc)?;
        let ci = self.lsp.clients.iter().position(|c| serves(c, kind, &path))?;
        let uri = convert::uri_from_path(&path)?;
        let version = doc.buffer.version();
        let client = &mut self.lsp.clients[ci];
        if !client.is_open(&uri) {
            client.did_open(uri.clone(), lang, doc.text());
            self.lsp.synced.insert(doc.uid, version);
        } else if self.lsp.synced.get(&doc.uid) != Some(&version) {
            client.did_change(&uri, doc.text());
            self.lsp.synced.insert(doc.uid, version);
        }
        Some((ci, uri))
    }

    pub(super) fn lsp_did_save(&mut self, i: usize) {
        if let Some((ci, uri)) = self.sync_doc(i) {
            self.lsp.clients[ci].did_save(&uri);
        }
    }

    pub(super) fn lsp_did_close(&mut self, doc: &Document) {
        self.lsp.synced.remove(&doc.uid);
        let Some((path, _, kind)) = lsp_target(doc) else { return };
        let Some(uri) = convert::uri_from_path(&path) else { return };
        if let Some(client) = self.lsp.clients.iter_mut().find(|c| serves(c, kind, &path)) {
            client.did_close(&uri);
        }
    }

    /// Stops all servers; they start again (after a new search) on the next frame.
    pub(super) fn restart_lsp(&mut self) {
        self.lsp.clients.clear();
        self.lsp.pending.clear();
        self.lsp.synced.clear();
        self.lsp.progress.clear();
        self.lsp.diagnostics.clear();
        self.lsp.completion = None;
        self.lsp.hover = None;
        self.lsp.setups.clear();
    }

    pub(super) fn doc_index(&self, uid: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.uid == uid)
    }

    fn doc_index_by_path(&self, path: &Path) -> Option<usize> {
        self.docs.iter().position(|d| d.path.as_deref().is_some_and(|p| same_path(p, path)))
    }

    fn handle_event(&mut self, kind: ServerKind, root: &Path, ev: Event) {
        match ev {
            Event::Notification { method, params } => match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params) else { return };
                    let Some(path) = convert::path_from_uri(&p.uri) else { return };
                    if p.diagnostics.is_empty() {
                        self.lsp.diagnostics.remove(&path_key(&path));
                    } else {
                        self.lsp.diagnostics.insert(path_key(&path), (path, p.diagnostics));
                    }
                }
                "$/progress" => self.on_progress(&params),
                // Errors and warnings only.
                "window/showMessage" if params["type"].as_i64().is_some_and(|t| t <= 2) => {
                    self.status = Some(format!("{}: {}", kind.name(), params["message"].as_str().unwrap_or("")));
                }
                _ => {}
            },
            Event::Request { id, method, params } => {
                let result = match method.as_str() {
                    // Defaults for every setting asked for.
                    // Defaults, except the check rust-analyzer runs on save (check or clippy).
                    "workspace/configuration" => {
                        let check = json!({ "check": { "command": self.settings.cargo.check_command } });
                        let items = params["items"].as_array().cloned().unwrap_or_default();
                        let answers = items
                            .iter()
                            .map(|i| if i["section"] == "rust-analyzer" { check.clone() } else { Value::Null })
                            .collect();
                        Ok(Value::Array(answers))
                    }
                    "workspace/applyEdit" => {
                        let edit = serde_json::from_value::<lt::WorkspaceEdit>(params["edit"].clone());
                        let applied = edit.is_ok_and(|e| self.apply_workspace_edit(&e).is_ok());
                        Ok(json!({ "applied": applied }))
                    }
                    "window/workDoneProgress/create"
                    | "client/registerCapability"
                    | "client/unregisterCapability"
                    | "window/showMessageRequest" => Ok(Value::Null),
                    m if m.ends_with("/refresh") => {
                        self.lsp_views_stale();
                        Ok(Value::Null)
                    }
                    _ => Err((-32601, format!("{method} is not supported"))),
                };
                if let Some(client) = self.lsp.clients.iter_mut().find(|c| c.kind == kind && c.root == root) {
                    client.respond(id, result);
                }
            }
            Event::Response { id, result } => {
                if let Some(pending) = self.lsp.pending.remove(&(kind, root.to_path_buf(), id)) {
                    self.on_response(kind, pending, result);
                }
            }
            Event::Exited(why) => {
                let why = if why.is_empty() { String::new() } else { format!(": {why}") };
                self.status =
                    Some(format!("{} stopped{why}. Run \"Language Servers: Restart\" to start it again.", kind.name()));
                self.lsp.pending.retain(|(k, r, _), _| !(*k == kind && r == root));
                self.lsp.progress.clear();
                self.lsp.setups.insert(kind, Setup::Off);
            }
        }
    }

    fn on_progress(&mut self, params: &Value) {
        let token = match &params["token"] {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let value = &params["value"];
        let message = || {
            let msg = value["message"].as_str().unwrap_or("");
            match value["percentage"].as_u64() {
                Some(p) if msg.is_empty() => format!("{p}%"),
                Some(p) => format!("{msg} ({p}%)"),
                None => msg.to_owned(),
            }
        };
        match value["kind"].as_str() {
            Some("begin") => {
                let title = value["title"].as_str().unwrap_or("").to_owned();
                self.lsp.progress.insert(token, (title, message()));
            }
            Some("report") => {
                if let Some(p) = self.lsp.progress.get_mut(&token) {
                    p.1 = message();
                }
            }
            Some("end") => {
                self.lsp.progress.remove(&token);
            }
            _ => {}
        }
    }

    fn on_response(&mut self, kind: ServerKind, pending: Pending, result: Result<Value, String>) {
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                match pending {
                    // Routine: the text changed before the server answered.
                    Pending::Completion { .. } => {
                        if let Some(c) = &mut self.lsp.completion {
                            c.waiting = false;
                        }
                    }
                    // Routine too, and the user didn't ask for these.
                    Pending::Hover { .. }
                    | Pending::SemanticTokens { .. }
                    | Pending::InlayHints { .. }
                    | Pending::SymbolHighlights { .. }
                    | Pending::Signature { .. }
                    | Pending::WorkspaceSymbols => {}
                    Pending::Format { uid, save: true, .. } => self.finish_format_save(uid, None),
                    _ => self.status = Some(format!("{}: {e}", kind.name())),
                }
                return;
            }
        };
        match pending {
            Pending::Completion { uid } => self.on_completion(uid, value),
            Pending::Hover { uid, pos, anchor } => self.on_hover(uid, pos, anchor, value),
            Pending::Locations { title, word } => self.on_locations(title, &word, value),
            Pending::Rename { name } => match serde_json::from_value::<Option<lt::WorkspaceEdit>>(value) {
                Ok(Some(edit)) => match self.apply_workspace_edit(&edit) {
                    Ok(files) => self.status = Some(format!("Renamed to '{name}' in {files} file(s)")),
                    Err(e) => self.status = Some(format!("Rename failed: {e}")),
                },
                _ => self.status = Some("Nothing to rename here".to_owned()),
            },
            Pending::CodeActions => self.on_code_actions(value),
            Pending::Format { uid, version, save } => {
                let edits = serde_json::from_value::<Option<Vec<lt::TextEdit>>>(value).ok().flatten();
                if save {
                    self.finish_format_save(uid, edits);
                } else if let Some(i) = self.doc_index(uid)
                    && self.docs[i].buffer.version() == version
                {
                    self.apply_text_edits(i, edits.unwrap_or_default());
                }
            }
            Pending::Execute => {}
            Pending::SemanticTokens { uid, ci, lines } => self.on_semantic_tokens(uid, ci, lines, value),
            Pending::InlayHints { uid, lines } => self.on_inlay_hints(uid, lines, value),
            Pending::SymbolHighlights { uid, version } => self.on_symbol_highlights(uid, version, value),
            Pending::Signature { uid } => self.on_signature(uid, value),
            Pending::WorkspaceSymbols => self.on_workspace_symbols(value),
        }
    }

    /// Sends a request about document `i`. `params` gets the document and its
    /// `textDocument` identifier. Returns false when no server handles it.
    pub(super) fn request_for(
        &mut self,
        i: usize,
        method: &str,
        pending: Pending,
        params: impl FnOnce(&Document, Value) -> Value,
    ) -> bool {
        let Some((ci, uri)) = self.sync_doc(i) else {
            self.status = Some(self.unavailable_reason());
            return false;
        };
        let p = params(&self.docs[i], json!({ "uri": uri }));
        let client = &mut self.lsp.clients[ci];
        let id = client.request(method, p);
        self.lsp.pending.insert((client.kind, client.root.clone(), id), pending);
        true
    }

    fn unavailable_reason(&self) -> String {
        let Some((_, _, kind)) = self.docs.get(self.active).and_then(lsp_target) else {
            return "No language server for this file".to_owned();
        };
        if self.restricted_folder().is_some() {
            return "Language servers are off in Restricted Mode".to_owned();
        }
        let name = kind.name();
        match self.lsp.setups.get(&kind) {
            _ if !self.settings.lsp.enabled => "Language servers are turned off (lsp.enabled)".to_owned(),
            None | Some(Setup::Probing(_) | Setup::Idle | Setup::Ready(_)) => format!("{name} is starting…"),
            Some(Setup::Missing { .. }) => format!("{name} is not installed"),
            Some(Setup::Installing(_)) => format!("{name} is being installed…"),
            Some(Setup::Off) => format!("{name} is stopped (run \"Language Servers: Restart\")"),
        }
    }

    pub(super) fn position_params(doc: &Document, td: Value, pos: usize) -> Value {
        json!({ "textDocument": td, "position": convert::position_of(doc.buffer.rope(), pos) })
    }

    // ----- diagnostics -------------------------------------------------------------

    /// A file's diagnostics from the language server and the last cargo check.
    fn diagnostics_for(&self, path: &Path) -> Vec<lt::Diagnostic> {
        let k = path_key(path);
        let mut all = self.lsp.diagnostics.get(&k).map(|(_, d)| d.clone()).unwrap_or_default();
        all.extend(self.plugin_problems(&k));
        for d in self.cargo.diagnostics.get(&k).into_iter().flat_map(|(_, d)| d) {
            // The same compiler message may come from rust-analyzer's own check.
            let first = |m: &str| m.lines().next().unwrap_or("").to_owned();
            if !all.iter().any(|a| a.range.start == d.range.start && first(&a.message) == first(&d.message)) {
                all.push(d.clone());
            }
        }
        all
    }

    /// Every file with diagnostics, from both sources.
    fn all_diagnostics(&self) -> Vec<(PathBuf, Vec<lt::Diagnostic>)> {
        let plugin_paths = self.plugin_problem_paths();
        let mut keys: Vec<&PathBuf> = self
            .lsp
            .diagnostics
            .keys()
            .chain(self.cargo.diagnostics.keys())
            .chain(plugin_paths.iter().map(|(k, _)| k))
            .collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter_map(|k| {
                let path = self
                    .lsp
                    .diagnostics
                    .get(k)
                    .or_else(|| self.cargo.diagnostics.get(k))
                    .map(|(p, _)| p)
                    .or_else(|| plugin_paths.iter().find(|(key, _)| key == k).map(|(_, p)| p))?
                    .clone();
                let diags = self.diagnostics_for(&path);
                Some((path, diags))
            })
            .collect()
    }

    /// Squiggles for document `i`.
    pub(super) fn lsp_decorations(&self, i: usize) -> Decorations {
        let Some(doc) = self.docs.get(i) else { return Decorations::default() };
        let Some(path) = &doc.path else { return Decorations::default() };
        let rope = doc.buffer.rope();
        let mut marks: Vec<Mark> = self
            .diagnostics_for(path)
            .iter()
            .map(|d| {
                let (start, end) = convert::chars_of(rope, d.range);
                Mark {
                    start,
                    end,
                    line: rope.char_to_line(start),
                    severity: severity_of(d),
                    message: d.message.clone(),
                }
            })
            .collect();
        marks.sort_by_key(|m| (m.start, m.severity));
        Decorations { marks, ..Decorations::default() }
    }

    /// (errors, warnings) across all files.
    pub(super) fn problem_counts(&self) -> (usize, usize) {
        let all = self.all_diagnostics();
        let severities = all.iter().flat_map(|(_, d)| d.iter().map(severity_of));
        severities.fold((0, 0), |(e, w), s| (e + usize::from(s == 1), w + usize::from(s == 2)))
    }

    /// Problems at char `pos` of document `uid`.
    fn problems_at(&self, uid: u64, pos: usize) -> Vec<(Severity, String)> {
        let Some(doc) = self.doc_index(uid).map(|i| &self.docs[i]) else { return Vec::new() };
        let Some(path) = &doc.path else { return Vec::new() };
        let rope = doc.buffer.rope();
        let mut found: Vec<(Severity, String)> = self
            .diagnostics_for(path)
            .iter()
            .filter(|d| {
                let (a, b) = convert::chars_of(rope, d.range);
                a <= pos && pos <= b.max(a + 1) && severity_of(d) <= 3
            })
            .map(|d| (severity_of(d), d.message.clone()))
            .collect();
        found.sort_by_key(|p| p.0);
        found
    }

    /// All problems (no hints), sorted by file then position.
    fn sorted_problems(&self) -> Vec<(PathBuf, lt::Diagnostic)> {
        let all = self.all_diagnostics();
        let mut files: Vec<&(PathBuf, Vec<lt::Diagnostic>)> = all.iter().collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut out = Vec::new();
        for (path, diags) in files {
            let mut diags: Vec<&lt::Diagnostic> = diags.iter().filter(|d| severity_of(d) <= 3).collect();
            diags.sort_by_key(|d| (d.range.start.line, d.range.start.character));
            out.extend(diags.into_iter().map(|d| (path.clone(), d.clone())));
        }
        out
    }

    /// F8 / Shift+F8: moves to the next/previous problem, across files.
    pub(super) fn next_problem(&mut self, forward: bool) {
        let problems = self.sorted_problems();
        if problems.is_empty() {
            self.status = Some("No problems have been detected".to_owned());
            return;
        }
        let here = self.docs.get(self.active).and_then(|d| {
            let path = d.path.clone()?;
            Some((path, convert::position_of(d.buffer.rope(), d.buffer.primary().head)))
        });
        let key = |path: &Path, p: lt::Position| (path_key(path), p.line, p.character);
        let target = match &here {
            Some((path, pos)) => {
                let cur = key(path, *pos);
                if forward {
                    problems.iter().find(|(p, d)| key(p, d.range.start) > cur).or(problems.first())
                } else {
                    problems.iter().rev().find(|(p, d)| key(p, d.range.start) < cur).or(problems.last())
                }
            }
            None => problems.first(),
        };
        if let Some((path, d)) = target.cloned() {
            self.jump_to(&path, d.range, false);
            self.lsp.show_problem_at_cursor = true;
        }
    }

    /// Opens `path` and moves the cursor to `range` (selecting it if `select`).
    pub(super) fn jump_to(&mut self, path: &Path, range: lt::Range, select: bool) {
        match self.doc_index_by_path(path) {
            Some(i) => self.active = i,
            None => self.open_file(path),
        }
        if let Some(doc) = self.docs.get_mut(self.active)
            && doc.path.as_deref().is_some_and(|p| same_path(p, path))
        {
            let (a, b) = convert::chars_of(doc.buffer.rope(), range);
            doc.select(a, if select { b } else { a });
        }
    }

    // ----- editing from the server -------------------------------------------------

    /// Applies LSP text edits to document `i` as one undo step.
    fn apply_text_edits(&mut self, i: usize, edits: Vec<lt::TextEdit>) {
        let Some(doc) = self.docs.get_mut(i) else { return };
        let rope = doc.buffer.rope();
        let edits = edits
            .into_iter()
            .map(|e| {
                let (a, b) = convert::chars_of(rope, e.range);
                (a, b, e.new_text.replace("\r\n", "\n").replace('\n', doc.buffer.line_ending))
            })
            .collect();
        doc.buffer.apply_edits(edits);
    }

    /// Applies a workspace edit (rename, code action). Files that aren't open
    /// are opened in tabs and left unsaved, so every change can be reviewed
    /// and undone. Returns how many files changed.
    pub(super) fn apply_workspace_edit(&mut self, edit: &lt::WorkspaceEdit) -> Result<usize, String> {
        let mut steps = Vec::new();
        if let Some(changes) = &edit.document_changes {
            match changes {
                lt::DocumentChanges::Edits(edits) => {
                    for e in edits {
                        steps.push(Step::Edit(e.text_document.uri.clone(), plain_edits(&e.edits)));
                    }
                }
                lt::DocumentChanges::Operations(ops) => {
                    for op in ops {
                        steps.push(match op {
                            lt::DocumentChangeOperation::Edit(e) => {
                                Step::Edit(e.text_document.uri.clone(), plain_edits(&e.edits))
                            }
                            lt::DocumentChangeOperation::Op(lt::ResourceOp::Create(c)) => Step::Create(c.uri.clone()),
                            lt::DocumentChangeOperation::Op(lt::ResourceOp::Rename(r)) => {
                                Step::Rename(r.old_uri.clone(), r.new_uri.clone())
                            }
                            lt::DocumentChangeOperation::Op(lt::ResourceOp::Delete(d)) => Step::Delete(d.uri.clone()),
                        });
                    }
                }
            }
        } else if let Some(changes) = &edit.changes {
            for (uri, edits) in changes {
                steps.push(Step::Edit(uri.clone(), edits.clone()));
            }
        }

        let active_uid = self.docs.get(self.active).map(|d| d.uid);
        let mut touched: Vec<PathBuf> = Vec::new();
        let to_path = |uri: &lt::Url| convert::path_from_uri(uri).ok_or_else(|| format!("unsupported URI {uri}"));
        for step in steps {
            match step {
                Step::Edit(uri, edits) => {
                    let path = to_path(&uri)?;
                    let i = match self.doc_index_by_path(&path) {
                        Some(i) => i,
                        None => {
                            let doc = Document::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                            self.watcher.watch_file(&path);
                            self.add_doc(doc);
                            let last = self.docs.len() - 1;
                            self.docs[last].request_focus = false;
                            last
                        }
                    };
                    self.apply_text_edits(i, edits);
                    touched.push(path);
                }
                Step::Create(uri) => {
                    let path = to_path(&uri)?;
                    if !path.exists() {
                        if let Some(dir) = path.parent() {
                            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                        }
                        std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    }
                    touched.push(path);
                }
                Step::Rename(old, new) => {
                    let (old, new) = (to_path(&old)?, to_path(&new)?);
                    std::fs::rename(&old, &new).map_err(|e| format!("{}: {e}", old.display()))?;
                    if let Some(i) = self.doc_index_by_path(&old) {
                        self.docs[i].path = Some(new.clone());
                    }
                    self.explorer.refresh();
                    touched.push(new);
                }
                Step::Delete(uri) => {
                    let path = to_path(&uri)?;
                    if self.doc_index_by_path(&path).is_some_and(|i| self.docs[i].is_dirty()) {
                        return Err(format!("{} has unsaved changes", path.display()));
                    }
                    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    self.explorer.refresh();
                    touched.push(path);
                }
            }
        }
        if let Some(i) = active_uid.and_then(|uid| self.doc_index(uid)) {
            self.active = i;
        }
        touched.sort();
        touched.dedup();
        Ok(touched.len())
    }

    // ----- completion --------------------------------------------------------------

    /// Asks for completions at the cursor (`trigger`: the character typed).
    pub(super) fn request_completion(&mut self, trigger: Option<char>) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let (uid, head) = (doc.uid, doc.buffer.primary().head);
        let start = if trigger.is_some() { head } else { word_start(doc, head) };
        let context = match trigger {
            Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
            None => json!({ "triggerKind": 1 }),
        };
        let sent = self.request_for(i, "textDocument/completion", Pending::Completion { uid }, |doc, td| {
            let mut p = Self::position_params(doc, td, head);
            p["context"] = context;
            p
        });
        if !sent {
            return;
        }
        match &mut self.lsp.completion {
            Some(c) if c.uid == uid && c.start == start => c.waiting = true,
            _ => {
                self.lsp.completion = Some(Completion {
                    uid,
                    start,
                    entries: Vec::new(),
                    shown: Vec::new(),
                    selected: 0,
                    incomplete: false,
                    waiting: true,
                    filtered_for: (u64::MAX, 0),
                    scroll_to_selected: true,
                });
            }
        }
    }

    fn on_completion(&mut self, uid: u64, value: Value) {
        let Some(c) = self.lsp.completion.as_mut().filter(|c| c.uid == uid) else { return };
        let (incomplete, items) = match serde_json::from_value::<Option<lt::CompletionResponse>>(value) {
            Ok(Some(lt::CompletionResponse::Array(items))) => (false, items),
            Ok(Some(lt::CompletionResponse::List(list))) => (list.is_incomplete, list.items),
            _ => (false, Vec::new()),
        };
        c.entries = items
            .into_iter()
            .map(|item| {
                let detail = item
                    .label_details
                    .as_ref()
                    .and_then(|d| d.description.clone().or_else(|| d.detail.clone()))
                    .or_else(|| item.detail.clone())
                    .unwrap_or_default();
                CompletionEntry { label: item.label.clone(), detail, item }
            })
            .collect();
        c.incomplete = incomplete;
        c.waiting = false;
        c.filtered_for = (u64::MAX, 0);
        self.refilter_completion();
    }

    /// Re-filters the list for what has been typed since it opened; closes it
    /// when the cursor leaves the word or nothing matches.
    fn refilter_completion(&mut self) {
        let Some(c) = &mut self.lsp.completion else { return };
        let Some(doc) = self.docs.iter().find(|d| d.uid == c.uid) else {
            self.lsp.completion = None;
            return;
        };
        let head = doc.buffer.primary().head;
        let key = (doc.buffer.version(), head);
        if c.filtered_for == key {
            return;
        }
        c.filtered_for = key;
        if head < c.start || doc.buffer.char_to_line(head) != doc.buffer.char_to_line(c.start) {
            self.lsp.completion = None;
            return;
        }
        let typed = doc.buffer.slice_string(c.start, head).to_lowercase();
        let mut shown: Vec<(bool, &str, usize)> = c
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let filter = e.item.filter_text.as_deref().unwrap_or(&e.label);
                fuzzy(filter, &typed).then(|| {
                    let sort = e.item.sort_text.as_deref().unwrap_or(&e.label);
                    (!filter.to_lowercase().starts_with(&typed), sort, i)
                })
            })
            .collect();
        shown.sort();
        c.shown = shown.into_iter().map(|s| s.2).collect();
        c.selected = 0;
        c.scroll_to_selected = true;
        if c.shown.is_empty() && !c.waiting {
            self.lsp.completion = None;
        }
    }

    fn accept_completion(&mut self) {
        let Some(c) = self.lsp.completion.take() else { return };
        let Some(&idx) = c.shown.get(c.selected) else { return };
        let Some(i) = self.doc_index(c.uid) else { return };
        let item = &c.entries[idx].item;
        let doc = &mut self.docs[i];
        let rope = doc.buffer.rope();
        let head = doc.buffer.primary().head;
        let (text, start, end) = match &item.text_edit {
            Some(lt::CompletionTextEdit::Edit(e)) => {
                let (a, b) = convert::chars_of(rope, e.range);
                (e.new_text.clone(), a, b)
            }
            Some(lt::CompletionTextEdit::InsertAndReplace(e)) => {
                let (a, b) = convert::chars_of(rope, e.replace);
                (e.new_text.clone(), a, b)
            }
            None => (item.insert_text.clone().unwrap_or_else(|| item.label.clone()), c.start, head),
        };
        // The user may have typed more since the request: replace up to the cursor.
        let (start, end) = (start.min(head), end.max(head));
        let snippet = item.insert_text_format == Some(lt::InsertTextFormat::SNIPPET);
        let (text, sel) = if snippet {
            convert::expand_snippet(&text)
        } else {
            let n = text.chars().count();
            (text, (n, n))
        };
        let mut edits: Vec<(usize, usize, String)> = item
            .additional_text_edits
            .iter()
            .flatten()
            .map(|e| {
                let (a, b) = convert::chars_of(rope, e.range);
                (a, b, e.new_text.clone())
            })
            .filter(|(a, b, _)| *b <= start || *a >= end)
            .collect();
        // Edits before the completion shift where it lands.
        let shift: isize = edits
            .iter()
            .filter(|(_, b, _)| *b <= start)
            .map(|(a, b, t)| t.chars().count() as isize - (b - a) as isize)
            .sum();
        edits.push((start, end, text));
        doc.buffer.apply_edits(edits);
        let base = (start as isize + shift) as usize;
        doc.buffer.set_single(crate::buffer::Selection::new(base + sel.0, base + sel.1));
        doc.reveal = true;
    }

    pub(super) fn close_completion(&mut self) {
        self.lsp.completion = None;
    }

    /// Keys for open popups, taken before the editor sees them.
    pub(super) fn lsp_keys(&mut self, ctx: &egui::Context) {
        if self.lsp.hover.as_ref().is_some_and(|h| h.sticky.is_some())
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.lsp.hover = None;
        }
        let Some(c) = &mut self.lsp.completion else { return };
        if c.shown.is_empty() {
            return;
        }
        let n = c.shown.len();
        let (up, down, page_up, page_down, accept, escape) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
                i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });
        if up {
            c.selected = c.selected.checked_sub(1).unwrap_or(n - 1);
        }
        if down {
            c.selected = (c.selected + 1) % n;
        }
        if page_up {
            c.selected = c.selected.saturating_sub(COMPLETION_ROWS);
        }
        if page_down {
            c.selected = (c.selected + COMPLETION_ROWS).min(n - 1);
        }
        c.scroll_to_selected |= up || down || page_up || page_down;
        if escape {
            self.lsp.completion = None;
        } else if accept {
            self.accept_completion();
        }
    }

    /// Opens, updates or closes completion after the user typed `typed`.
    fn completion_after_typing(&mut self, typed: &str, focused: bool) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let uid = doc.uid;
        if !focused || self.lsp.completion.as_ref().is_some_and(|c| c.uid != uid) {
            self.lsp.completion = None;
        }
        let Some((ci, _)) = lsp_target(doc).and_then(|(p, _, k)| {
            self.lsp.clients.iter().position(|c| serves(c, k, &p) && c.is_ready()).map(|ci| (ci, p))
        }) else {
            return;
        };
        let open = self.lsp.completion.is_some();
        if let Some(ch) = typed.chars().last() {
            if self.lsp.clients[ci].completion_triggers().contains(&ch) {
                self.request_completion(Some(ch));
                return;
            }
            if is_word_char(ch) {
                if !open || self.lsp.completion.as_ref().is_some_and(|c| c.incomplete) {
                    self.request_completion(None);
                    return;
                }
            } else if open {
                self.lsp.completion = None;
                return;
            }
        }
        self.refilter_completion();
    }

    // ----- hover -------------------------------------------------------------------

    fn same_word(&self, uid: u64, a: usize, b: usize) -> bool {
        a == b
            || self.doc_index(uid).is_some_and(|i| {
                let buf = &self.docs[i].buffer;
                buf.word_at(a).is_some_and(|w| Some(w) == buf.word_at(b))
            })
    }

    /// Ctrl+K Ctrl+I: hover info at the cursor.
    pub(super) fn show_hover_at_cursor(&mut self) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let (uid, pos) = (doc.uid, doc.buffer.primary().head);
        if lsp_target(doc).is_none() {
            self.lsp.show_problem_at_cursor = true;
            return;
        }
        self.request_for(i, "textDocument/hover", Pending::Hover { uid, pos, anchor: None }, |doc, td| {
            Self::position_params(doc, td, pos)
        });
    }

    fn on_hover(&mut self, uid: u64, pos: usize, anchor: Option<Pos2>, value: Value) {
        let markdown = hover_markdown(value);
        let problems = self.problems_at(uid, pos);
        if markdown.trim().is_empty() && problems.is_empty() {
            if anchor.is_none() {
                self.status = Some("No hover information".to_owned());
            }
            return;
        }
        // A pointer hover only shows if the pointer is still on that word.
        if anchor.is_some() && !self.lsp.hover_rest.is_some_and(|(u, p, ..)| u == uid && self.same_word(uid, p, pos)) {
            return;
        }
        let sticky = anchor.is_none().then_some(pos);
        let Some(anchor) = anchor.or_else(|| self.lsp.cursor_rect.map(|r| r.left_bottom())) else { return };
        let version = self.doc_index(uid).map_or(0, |i| self.docs[i].buffer.version());
        self.lsp.hover = Some(Hover { uid, version, pos, anchor, markdown, problems, sticky, rect: None });
    }

    /// Hover bookkeeping after the editor ran: rest timer, closing, F8 popup.
    fn update_hover(&mut self, ctx: &egui::Context, uid: u64, hover: Option<(usize, Pos2)>, head: usize) {
        let now = ctx.input(|i| i.time);
        let version = self.doc_index(uid).map(|i| self.docs[i].buffer.version());
        if let Some(h) = &self.lsp.hover {
            let keep = Some(h.version) == version
                && match h.sticky {
                    Some(at) => h.uid == uid && at == head,
                    None => {
                        let pointer = ctx.pointer_hover_pos();
                        let over_popup = h.rect.zip(pointer).is_some_and(|(r, p)| r.expand(10.0).contains(p));
                        let same_word = hover.is_some_and(|(pos, _)| h.uid == uid && self.same_word(uid, h.pos, pos));
                        over_popup || same_word
                    }
                };
            if !keep {
                self.lsp.hover = None;
            }
        }
        if std::mem::take(&mut self.lsp.show_problem_at_cursor) {
            let problems = self.problems_at(uid, head);
            if let (false, Some(rect)) = (problems.is_empty(), self.lsp.cursor_rect) {
                self.lsp.hover = Some(Hover {
                    uid,
                    version: version.unwrap_or(0),
                    pos: head,
                    anchor: rect.left_bottom(),
                    markdown: String::new(),
                    problems,
                    sticky: Some(head),
                    rect: None,
                });
            }
        }
        let Some((pos, pointer)) = hover else {
            self.lsp.hover_rest = None;
            return;
        };
        let resting = self.lsp.hover_rest.filter(|&(u, p, ..)| u == uid && self.same_word(uid, p, pos));
        match resting {
            None => {
                self.lsp.hover_rest = Some((uid, pos, now, false));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY));
            }
            Some((_, _, since, false)) if self.lsp.hover.is_none() => {
                if now - since < HOVER_DELAY {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOVER_DELAY - (now - since)));
                    return;
                }
                self.lsp.hover_rest = Some((uid, pos, since, true));
                let i = self.active;
                let has_server = self
                    .docs
                    .get(i)
                    .and_then(lsp_target)
                    .is_some_and(|(p, _, k)| self.lsp.clients.iter().any(|c| serves(c, k, &p) && c.is_ready()));
                if has_server {
                    let anchor = Some(pointer);
                    self.request_for(i, "textDocument/hover", Pending::Hover { uid, pos, anchor }, |doc, td| {
                        Self::position_params(doc, td, pos)
                    });
                } else {
                    let problems = self.problems_at(uid, pos);
                    if !problems.is_empty() {
                        let markdown = String::new();
                        self.lsp.hover = Some(Hover {
                            uid,
                            version: version.unwrap_or(0),
                            pos,
                            anchor: pointer,
                            markdown,
                            problems,
                            sticky: None,
                            rect: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    /// Everything to do after the editor ran this frame.
    pub(super) fn after_editor(&mut self, ctx: &egui::Context, out: Output, typed: &str) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let (uid, head) = (doc.uid, doc.buffer.primary().head);
        self.lsp.cursor_rect = Some(out.cursor_rect);
        let focused = self.editor_focused(ctx);
        self.completion_after_typing(typed, focused);
        self.signature_after_typing(typed);
        self.update_hover(ctx, uid, out.hover, head);
        if let Some(pos) = out.ctrl_click {
            self.request_locations("textDocument/definition", "Definitions", Some(pos));
        }
    }

    // ----- navigation --------------------------------------------------------------

    /// Go to Definition / Implementations / References at `pos` (default: cursor).
    pub(super) fn request_locations(&mut self, method: &str, title: &'static str, pos: Option<usize>) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let pos = pos.unwrap_or(doc.buffer.primary().head);
        let word = doc.buffer.word_at(pos).map(|(a, b)| doc.buffer.slice_string(a, b)).unwrap_or_default();
        let references = title == "References";
        self.request_for(i, method, Pending::Locations { title, word }, |doc, td| {
            let mut p = Self::position_params(doc, td, pos);
            if references {
                p["context"] = json!({ "includeDeclaration": true });
            }
            p
        });
    }

    fn on_locations(&mut self, title: &'static str, word: &str, value: Value) {
        let found: Vec<(PathBuf, lt::Range)> =
            match serde_json::from_value::<Option<lt::GotoDefinitionResponse>>(value) {
                Ok(Some(lt::GotoDefinitionResponse::Scalar(l))) => vec![l],
                Ok(Some(lt::GotoDefinitionResponse::Array(v))) => v,
                Ok(Some(lt::GotoDefinitionResponse::Link(links))) => links
                    .into_iter()
                    .map(|l| lt::Location { uri: l.target_uri, range: l.target_selection_range })
                    .collect(),
                _ => Vec::new(),
            }
            .into_iter()
            .filter_map(|l| Some((convert::path_from_uri(&l.uri)?, l.range)))
            .collect();
        if found.is_empty() {
            self.status = Some(format!("No {} found for '{word}'", title.to_lowercase()));
            return;
        }
        let mut found = found;
        found.sort_by_key(|(path, r)| (path_key(path), r.start.line, r.start.character));
        if title != "References" && found.len() == 1 {
            let (path, range) = &found[0];
            self.jump_to(path, *range, false);
            return;
        }
        let mut files: HashMap<PathBuf, Vec<String>> = HashMap::new();
        self.lsp.locations = found
            .into_iter()
            .map(|(path, range)| {
                let line = range.start.line as usize;
                let preview = match self.doc_index_by_path(&path) {
                    Some(i) if line < self.docs[i].buffer.len_lines() => self.docs[i].buffer.line_text(line),
                    _ => files
                        .entry(path.clone())
                        .or_insert_with(|| {
                            std::fs::read_to_string(&path)
                                .map(|t| t.lines().map(str::to_owned).collect())
                                .unwrap_or_default()
                        })
                        .get(line)
                        .cloned()
                        .unwrap_or_default(),
                };
                LocationItem { path, range, preview: preview.trim().to_owned() }
            })
            .collect();
        self.lsp.locations_title = format!("{title} of '{word}': {}", self.lsp.locations.len());
        self.panel_tab = PanelTab::References;
        self.panel_visible = true;
    }

    // ----- rename, code actions, formatting -----------------------------------------

    pub(super) fn start_rename(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let head = doc.buffer.primary().head;
        let Some((a, b)) = doc.buffer.word_at(head).or_else(|| doc.buffer.word_at(head.saturating_sub(1))) else {
            self.status = Some("Nothing to rename here".to_owned());
            return;
        };
        if lsp_target(doc).is_none() {
            self.status = Some(self.unavailable_reason());
            return;
        }
        let word = doc.buffer.slice_string(a, b);
        self.lsp.rename_at = Some((doc.uid, a));
        self.palette.show_input(InputKind::Rename, &word, "Rename symbol: type the new name and press Enter");
    }

    pub(super) fn finish_rename(&mut self, name: &str) {
        let Some((uid, pos)) = self.lsp.rename_at.take() else { return };
        let Some(i) = self.doc_index(uid) else { return };
        let name = name.to_owned();
        self.request_for(i, "textDocument/rename", Pending::Rename { name: name.clone() }, |doc, td| {
            let mut p = Self::position_params(doc, td, pos);
            p["newName"] = json!(name);
            p
        });
    }

    pub(super) fn request_code_actions(&mut self) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let sel = doc.buffer.primary();
        let rope = doc.buffer.rope();
        let range = convert::range_of(rope, sel.start(), sel.end());
        let diagnostics: Vec<lt::Diagnostic> = doc
            .path
            .as_deref()
            .map(|p| self.diagnostics_for(p))
            .unwrap_or_default()
            .iter()
            .filter(|d| d.range.start.line <= range.end.line && d.range.end.line >= range.start.line)
            .cloned()
            .collect();
        self.request_for(i, "textDocument/codeAction", Pending::CodeActions, |_, td| {
            json!({ "textDocument": td, "range": range, "context": { "diagnostics": diagnostics, "triggerKind": 1 } })
        });
    }

    fn on_code_actions(&mut self, value: Value) {
        let actions: Vec<lt::CodeActionOrCommand> = serde_json::from_value::<Option<Vec<_>>>(value)
            .ok()
            .flatten()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| !matches!(a, lt::CodeActionOrCommand::CodeAction(c) if c.disabled.is_some()))
            .collect();
        if actions.is_empty() {
            self.status = Some("No code actions available".to_owned());
            return;
        }
        let titles = actions
            .iter()
            .map(|a| match a {
                lt::CodeActionOrCommand::Command(c) => c.title.clone(),
                lt::CodeActionOrCommand::CodeAction(c) => c.title.clone(),
            })
            .collect();
        self.lsp.code_actions = actions;
        self.palette.show_picker(PickerKind::CodeAction, titles, "", "Select a code action");
    }

    pub(super) fn pick_code_action(&mut self, title: &str) {
        let actions = std::mem::take(&mut self.lsp.code_actions);
        let Some(action) = actions.into_iter().find(|a| match a {
            lt::CodeActionOrCommand::Command(c) => c.title == title,
            lt::CodeActionOrCommand::CodeAction(c) => c.title == title,
        }) else {
            return;
        };
        let command = match action {
            lt::CodeActionOrCommand::Command(c) => Some(c),
            lt::CodeActionOrCommand::CodeAction(a) => {
                if let Some(edit) = &a.edit
                    && let Err(e) = self.apply_workspace_edit(edit)
                {
                    self.status = Some(format!("Code action failed: {e}"));
                    return;
                }
                a.command
            }
        };
        if let Some(cmd) = command {
            let args = cmd.arguments.unwrap_or_default();
            self.request_for(
                self.active,
                "workspace/executeCommand",
                Pending::Execute,
                |_, _| json!({ "command": cmd.command, "arguments": args }),
            );
        }
    }

    /// Asks the server to format document `i` (then save it, if `save`).
    /// Returns false if no server can format it.
    pub(super) fn request_format(&mut self, ctx: &egui::Context, i: usize, save: bool) -> bool {
        let Some(doc) = self.docs.get(i) else { return false };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let can_format = lsp_target(doc).is_some_and(|(p, _, k)| {
            self.lsp.clients.iter().any(|c| {
                serves(c, k, &p)
                    && c.capabilities()
                        .and_then(|caps| caps.document_formatting_provider.as_ref())
                        .is_some_and(|f| !matches!(f, lt::OneOf::Left(false)))
            })
        });
        if !can_format {
            return false;
        }
        let sent = self.request_for(
            i,
            "textDocument/formatting",
            Pending::Format { uid, version, save },
            |_, td| json!({ "textDocument": td, "options": { "tabSize": 4, "insertSpaces": true } }),
        );
        if sent && save {
            self.lsp.format_save = Some((uid, ctx.input(|inp| inp.time)));
        }
        sent
    }

    /// Format-on-save finished (or failed): apply the edits, then save.
    fn finish_format_save(&mut self, uid: u64, edits: Option<Vec<lt::TextEdit>>) {
        if self.lsp.format_save.map(|f| f.0) != Some(uid) {
            return; // Timed out and already saved.
        }
        self.lsp.format_save = None;
        let Some(i) = self.doc_index(uid) else { return };
        if let Some(edits) = edits {
            self.apply_text_edits(i, edits);
        }
        self.save_doc(i);
    }

    // ----- UI ------------------------------------------------------------------------

    /// Offer to install the active file's missing server (nothing is
    /// downloaded without a yes).
    pub(super) fn lsp_offer_bar(&mut self, ui: &mut egui::Ui) {
        let Some((_, _, kind)) = self.docs.get(self.active).and_then(lsp_target) else { return };
        let Some(Setup::Missing { installer, note }) = self.lsp.setups.get(&kind) else { return };
        if self.restricted_folder().is_some() {
            return;
        }
        if self.lsp.offer_dismissed.contains(&kind) || !self.settings.lsp.ask_to_install {
            return;
        }
        let (installer, note) = (installer.clone(), note.clone());
        let (name, langs) = (kind.name(), kind.languages());
        let mut choice = None;
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| match &installer {
                Some(installer) => {
                    ui.label(format!(
                        "{name} is not installed. It provides errors, completion, hover and go to definition \
                         for {langs}. Install it? This will {}.",
                        installer.describe()
                    ));
                    if ui.button("Install").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Not Now").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Don't Ask Again").clicked() {
                        choice = Some(2);
                    }
                }
                None => {
                    ui.label(format!(
                        "No language server for {langs} was found, so its language features are off. {note} \
                         Or set lsp.paths.{} in settings.",
                        kind.id()
                    ));
                    if ui.button("OK").clicked() {
                        choice = Some(1);
                    }
                }
            });
        });
        match (choice, installer) {
            (Some(0), Some(installer)) => {
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ui.ctx());
                std::thread::spawn(move || {
                    let _ = tx.send(servers::install(kind, &installer));
                    wake();
                });
                self.lsp.setups.insert(kind, Setup::Installing(rx));
            }
            (Some(2), _) => {
                self.lsp.offer_dismissed.insert(kind);
                let ctx = ui.ctx().clone();
                if let Err(e) = self.write_user_setting(&ctx, "lsp", "ask_to_install", "false") {
                    self.status = Some(e);
                }
            }
            (Some(_), _) => {
                self.lsp.offer_dismissed.insert(kind);
            }
            _ => {}
        }
    }

    /// The active file's server state for the status bar.
    pub(super) fn lsp_status_text(&self) -> Option<String> {
        let (_, _, kind) = self.docs.get(self.active).and_then(lsp_target)?;
        if !self.settings.lsp.enabled {
            return None;
        }
        if self.restricted_folder().is_some() {
            return None;
        }
        let name = kind.name();
        let starting = || format!("{name}: starting");
        Some(match self.lsp.setups.get(&kind) {
            Some(Setup::Installing(_)) => format!("Installing {name}…"),
            Some(Setup::Missing { .. }) => format!("{name}: not installed"),
            Some(Setup::Off) => format!("{name}: stopped"),
            None | Some(Setup::Idle | Setup::Probing(_)) => starting(),
            Some(Setup::Ready(_)) => match self.lsp.progress.values().next() {
                Some((title, msg)) => format!("{name}: {title} {msg}").trim_end().to_owned(),
                None if self.lsp.clients.iter().any(|c| c.kind == kind && !c.is_ready()) => starting(),
                None => name.to_owned(),
            },
        })
    }

    /// Completion list and hover popups, drawn over everything.
    pub(super) fn lsp_popups(&mut self, ctx: &egui::Context) {
        self.signature_popup(ctx);
        let active_uid = self.docs.get(self.active).map(|d| d.uid);
        let font = egui::FontId::monospace(self.font_size);
        let mut accept = None;
        if let (Some(c), Some(rect)) = (&mut self.lsp.completion, self.lsp.cursor_rect)
            && Some(c.uid) == active_uid
            && !c.shown.is_empty()
        {
            egui::Area::new(egui::Id::unique("lsp_completion"))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.left_bottom() + egui::vec2(0.0, 2.0))
                .constrain(true)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).fill(theme::colors().sidebar_bg).inner_margin(2.0).show(ui, |ui| {
                        ui.set_width(440.0);
                        let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 4.0;
                        egui::ScrollArea::vertical().max_height(row_h * COMPLETION_ROWS as f32).show(ui, |ui| {
                            for (row, &idx) in c.shown.iter().enumerate() {
                                let e = &c.entries[idx];
                                let selected = row == c.selected;
                                let (rect, resp) =
                                    ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
                                if selected {
                                    ui.painter().rect_filled(rect, 2.0, theme::colors().selection);
                                    if c.scroll_to_selected {
                                        resp.scroll_to_me(Some(Align::Center));
                                    }
                                }
                                let (glyph, color) = lsp_ui::kind_glyph(e.item.kind);
                                let y = rect.center().y;
                                let p = ui.painter();
                                p.text(
                                    rect.left_center() + egui::vec2(6.0, 0.0),
                                    egui::Align2::LEFT_CENTER,
                                    glyph,
                                    font.clone(),
                                    color,
                                );
                                p.text(
                                    egui::pos2(rect.left() + 24.0, y),
                                    egui::Align2::LEFT_CENTER,
                                    &e.label,
                                    font.clone(),
                                    theme::colors().text,
                                );
                                if !e.detail.is_empty() {
                                    let small = egui::FontId::proportional(self.font_size * 0.85);
                                    p.text(
                                        egui::pos2(rect.right() - 6.0, y),
                                        egui::Align2::RIGHT_CENTER,
                                        &e.detail,
                                        small,
                                        theme::colors().text_dim,
                                    );
                                }
                                if resp.clicked() {
                                    accept = Some(row);
                                }
                            }
                        });
                        c.scroll_to_selected = false;
                    });
                });
        }
        if let Some(row) = accept {
            if let Some(c) = &mut self.lsp.completion {
                c.selected = row;
            }
            self.accept_completion();
        }

        let code_size = self.font_size * 0.95;
        if let Some(h) = &mut self.lsp.hover
            && Some(h.uid) == active_uid
        {
            let resp = egui::Area::new(egui::Id::unique("lsp_hover"))
                .order(egui::Order::Foreground)
                .fixed_pos(h.anchor + egui::vec2(0.0, 14.0))
                .constrain(true)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).fill(theme::colors().sidebar_bg).inner_margin(8.0).show(ui, |ui| {
                        ui.set_max_width(560.0);
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for (sev, msg) in &h.problems {
                                ui.horizontal_wrapped(|ui| {
                                    lsp_ui::severity_icon(ui, *sev, severity_color(*sev));
                                    ui.label(msg);
                                });
                            }
                            if !h.problems.is_empty() && !h.markdown.trim().is_empty() {
                                ui.separator();
                            }
                            lsp_ui::markdown(ui, &h.markdown, code_size);
                        });
                    });
                });
            // A new or resized popup needs another frame to lay out fully.
            if h.rect != Some(resp.response.rect) {
                ctx.request_repaint();
            }
            h.rect = Some(resp.response.rect);
        }
    }

    /// The Problems panel: every diagnostic, grouped by file.
    pub(super) fn problems_panel(&mut self, ui: &mut egui::Ui) {
        let problems = self.sorted_problems();
        let mut jump = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if problems.is_empty() {
                ui.label(
                    RichText::new("No problems have been detected in the workspace.").color(theme::colors().text_dim),
                );
            }
            let mut last_file: Option<&Path> = None;
            for (path, d) in &problems {
                if last_file != Some(path.as_path()) {
                    last_file = Some(path);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let dir = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(dir).small().color(theme::colors().text_dim));
                    });
                }
                let source = match (&d.source, &d.code) {
                    (Some(s), Some(lt::NumberOrString::String(c))) => format!("{s}({c})"),
                    (Some(s), Some(lt::NumberOrString::Number(c))) => format!("{s}({c})"),
                    (Some(s), None) => s.clone(),
                    _ => String::new(),
                };
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(14.0);
                        let sev = severity_of(d);
                        lsp_ui::severity_icon(ui, sev, severity_color(sev));
                        let msg = d.message.lines().next().unwrap_or_default();
                        let r = ui.add(egui::Label::new(msg).sense(Sense::click()).truncate());
                        ui.label(
                            RichText::new(format!(
                                "{source}  [Ln {}, Col {}]",
                                d.range.start.line + 1,
                                d.range.start.character + 1
                            ))
                            .small()
                            .color(theme::colors().text_dim),
                        );
                        r
                    })
                    .inner;
                if resp.clicked() {
                    jump = Some((path.clone(), d.range));
                }
            }
        });
        if let Some((path, range)) = jump {
            self.jump_to(&path, range, true);
        }
    }

    /// The References panel: results of Find References / multiple definitions.
    pub(super) fn references_panel(&mut self, ui: &mut egui::Ui) {
        let mut jump = None;
        ui.label(RichText::new(&self.lsp.locations_title).strong());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.lsp.locations.is_empty() {
                ui.label(
                    RichText::new("Use Go to References (Shift+F12) to list where a symbol is used.")
                        .color(theme::colors().text_dim),
                );
            }
            let mut last_file: Option<&Path> = None;
            for item in &self.lsp.locations {
                if last_file != Some(item.path.as_path()) {
                    last_file = Some(&item.path);
                    let name = item.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let dir = item.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(name).strong());
                        ui.label(RichText::new(dir).small().color(theme::colors().text_dim));
                    });
                }
                let resp = ui
                    .horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(
                            RichText::new(format!("{}", item.range.start.line + 1))
                                .small()
                                .color(theme::colors().text_dim),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(&item.preview).monospace()).sense(Sense::click()).truncate(),
                        )
                    })
                    .inner;
                if resp.clicked() {
                    jump = Some((item.path.clone(), item.range));
                }
            }
        });
        if let Some((path, range)) = jump {
            self.jump_to(&path, range, true);
        }
    }

    pub(super) fn problems_tab_label(&self) -> String {
        let (e, w) = self.problem_counts();
        if e + w == 0 { "PROBLEMS".to_owned() } else { format!("PROBLEMS {}", e + w) }
    }
}
