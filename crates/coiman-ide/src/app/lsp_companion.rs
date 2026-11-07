//! Servers from VS Code extensions (ESLint, Tailwind CSS) running beside a file's own
//! language server: they get the file's text, and their problems, fixes and completions
//! join the main server's. Started only in trusted folders, since they run the project's
//! own configuration (an `eslint.config.js` is code).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use coiman_core::lsp::companions::{self, Companion};
use coiman_core::lsp::convert;
use coiman_core::lsp::servers::{Launch, ServerKind, which};
use coiman_core::lsp::types as lt;
use coiman_core::lsp::{Client, Event};
use coiman_core::trust;
use eframe::egui;
use serde_json::{Value, json};

use super::lsp::path_key;
use super::{CodeApp, repaint};

/// What a request to a companion was for.
enum Ask {
    /// Its problems in a file (`textDocument/diagnostic`).
    Diagnostics {
        path: PathBuf,
    },
    Actions,
    Completion {
        uid: u64,
    },
    Execute,
}

struct Running {
    id: u64,
    spec: &'static Companion,
    root: PathBuf,
    client: Client,
    /// Documents sent to it: uri, and the version sent, by document uid.
    open: HashMap<u64, (lt::Url, u64)>,
}

#[derive(Default)]
pub(super) struct Companions {
    running: Vec<Running>,
    next_id: u64,
    /// (server, project) pairs that stopped or couldn't start: not tried again.
    failed: HashSet<(&'static str, PathBuf)>,
    /// Problems by (server id, file key), with the path as reported.
    diagnostics: HashMap<(u64, PathBuf), (PathBuf, Vec<lt::Diagnostic>)>,
    pending: HashMap<(u64, i64), Ask>,
    /// Code action titles, and which server to ask to carry out each.
    pub(super) owners: HashMap<String, u64>,
}

impl CodeApp {
    /// Starts what the open files need, keeps them in sync, and reads their messages.
    pub(super) fn pump_companions(&mut self, ctx: &egui::Context) {
        if !self.settings.lsp.enabled || !self.settings.lsp.extension_servers {
            return;
        }
        let top = self.explorer.root.clone();
        // Start and sync.
        for i in 0..self.docs.len() {
            let doc = &self.docs[i];
            let Some(path) = doc.path.clone().filter(|_| !doc.large) else { continue };
            let Some(language) = companions::language_of(&doc.extension()) else { continue };
            for spec in companions::ALL {
                if !spec.languages.contains(&language) {
                    continue;
                }
                let Some(root) = spec.root_for(&path, top.as_deref()) else { continue };
                let at = self.companions.running.iter().position(|r| r.spec == spec && r.root == root);
                let at = match at {
                    Some(at) => at,
                    None => {
                        if self.companions.failed.contains(&(spec.name, root.clone())) || !trust::is_trusted(&root) {
                            continue;
                        }
                        match start(spec, &root, ctx) {
                            Ok(client) => {
                                let id = self.companions.next_id;
                                self.companions.next_id += 1;
                                coiman_core::applog::info(&format!(
                                    "companion: started {} in {}",
                                    spec.name,
                                    root.display()
                                ));
                                self.companions.running.push(Running {
                                    id,
                                    spec,
                                    root: root.clone(),
                                    client,
                                    open: HashMap::new(),
                                });
                                self.companions.running.len() - 1
                            }
                            Err(why) => {
                                self.status = Some(format!("{}: {why}", spec.name));
                                self.companions.failed.insert((spec.name, root));
                                continue;
                            }
                        }
                    }
                };
                let running = &mut self.companions.running[at];
                if !running.client.is_ready() {
                    continue;
                }
                if running.open.is_empty() {
                    let pulls = running.client.capabilities().is_some_and(|c| c.diagnostic_provider.is_some());
                    coiman_core::applog::info(&format!("companion: {} ready (asked for problems: {pulls})", spec.name));
                }
                let doc = &self.docs[i];
                let version = doc.buffer.version();
                let sent = match running.open.get(&doc.uid) {
                    None => {
                        let Some(uri) = convert::uri_from_path(&path) else { continue };
                        running.client.did_open(uri.clone(), language, doc.text());
                        running.open.insert(doc.uid, (uri.clone(), version));
                        Some(uri)
                    }
                    Some((uri, sent)) if *sent != version => {
                        let uri = uri.clone();
                        running.client.did_change(&uri, doc.text());
                        running.open.insert(doc.uid, (uri.clone(), version));
                        Some(uri)
                    }
                    _ => None,
                };
                // Servers that report problems when asked (ESLint does) are asked.
                if let Some(uri) = sent {
                    let id = running.id;
                    if let Some(rid) = pull(&mut running.client, &uri) {
                        self.companions.pending.insert((id, rid), Ask::Diagnostics { path: path.clone() });
                    }
                }
            }
        }
        // Closed tabs are closed there too.
        let uids: HashSet<u64> = self.docs.iter().map(|d| d.uid).collect();
        for running in &mut self.companions.running {
            let gone: Vec<u64> = running.open.keys().filter(|u| !uids.contains(u)).copied().collect();
            for uid in gone {
                if let Some((uri, _)) = running.open.remove(&uid) {
                    running.client.did_close(&uri);
                }
            }
        }
        // Messages.
        let mut batch = Vec::new();
        for running in &mut self.companions.running {
            for event in running.client.poll() {
                batch.push((running.id, event));
            }
        }
        for (id, event) in batch {
            self.companion_event(id, event);
        }
    }

    fn companion_event(&mut self, id: u64, event: Event) {
        let Some(at) = self.companions.running.iter().position(|r| r.id == id) else { return };
        let (spec, root) = (self.companions.running[at].spec, self.companions.running[at].root.clone());
        match event {
            Event::Notification { method, params } => {
                if method == "window/logMessage" && params["type"].as_i64().is_some_and(|t| t <= 1) {
                    coiman_core::applog::info(&format!(
                        "companion: {} error: {}",
                        spec.name,
                        params["message"].as_str().unwrap_or("")
                    ));
                }
                if method == "textDocument/publishDiagnostics" {
                    let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params) else { return };
                    let Some(path) = convert::path_from_uri(&p.uri) else { return };
                    let key = (id, path_key(&path));
                    if p.diagnostics.is_empty() {
                        self.companions.diagnostics.remove(&key);
                    } else {
                        // Named, so a problem says which tool found it.
                        let mut found = p.diagnostics;
                        for d in &mut found {
                            d.source.get_or_insert_with(|| spec.name.to_owned());
                        }
                        self.companions.diagnostics.insert(key, (path, found));
                    }
                } else if let Some(notice) = spec.notice(&method, &params) {
                    self.status = Some(notice);
                }
            }
            Event::Request { id: rid, method, params } => {
                let root_uri = convert::uri_from_path(&root).map(|u| u.to_string()).unwrap_or_default();
                let result = match method.as_str() {
                    "workspace/configuration" => {
                        let items = params["items"].as_array().cloned().unwrap_or_default();
                        Ok(Value::Array(
                            items.iter().map(|i| spec.setting(i["section"].as_str(), &root, &root_uri)).collect(),
                        ))
                    }
                    "workspace/applyEdit" => {
                        let edit = serde_json::from_value::<lt::WorkspaceEdit>(params["edit"].clone());
                        let applied = edit.is_ok_and(|e| self.apply_workspace_edit(&e).is_ok());
                        Ok(json!({ "applied": applied }))
                    }
                    "window/workDoneProgress/create"
                    | "client/registerCapability"
                    | "client/unregisterCapability"
                    | "window/showMessageRequest"
                    | "workspace/workspaceFolders" => Ok(Value::Null),
                    "workspace/diagnostic/refresh" => {
                        // Ask again for every file it has open.
                        if let Some(r) = self.companions.running.iter_mut().find(|r| r.id == id) {
                            let open: Vec<lt::Url> = r.open.values().map(|(u, _)| u.clone()).collect();
                            for uri in open {
                                if let (Some(rid), Some(path)) =
                                    (pull(&mut r.client, &uri), convert::path_from_uri(&uri))
                                {
                                    self.companions.pending.insert((id, rid), Ask::Diagnostics { path });
                                }
                            }
                        }
                        Ok(Value::Null)
                    }
                    m if m.ends_with("/refresh") => Ok(Value::Null),
                    m => match spec.own_request(m) {
                        Some(answer) => {
                            if let Some(notice) = spec.notice(m, &params) {
                                self.status = Some(notice);
                            }
                            Ok(answer)
                        }
                        None => Err((-32601, format!("{m} is not supported"))),
                    },
                };
                if let Some(r) = self.companions.running.iter_mut().find(|r| r.id == id) {
                    r.client.respond(rid, result);
                }
            }
            Event::Response { id: rid, result } => {
                let Some(ask) = self.companions.pending.remove(&(id, rid)) else { return };
                if let Err(e) = &result {
                    coiman_core::applog::info(&format!("companion: {} answered with an error: {e}", spec.name));
                }
                let value = result.unwrap_or(Value::Null);
                match ask {
                    Ask::Diagnostics { path } => {
                        // A full report replaces what it said before; "unchanged" keeps it.
                        if value["kind"].as_str() == Some("full") {
                            let mut found: Vec<lt::Diagnostic> =
                                serde_json::from_value(value["items"].clone()).unwrap_or_default();
                            for d in &mut found {
                                d.source.get_or_insert_with(|| spec.name.to_owned());
                            }
                            let key = (id, path_key(&path));
                            if found.is_empty() {
                                self.companions.diagnostics.remove(&key);
                            } else {
                                self.companions.diagnostics.insert(key, (path, found));
                            }
                        }
                    }
                    Ask::Actions => {
                        let actions: Vec<lt::CodeActionOrCommand> =
                            serde_json::from_value::<Option<Vec<_>>>(value).ok().flatten().unwrap_or_default();
                        for a in &actions {
                            let title = match a {
                                lt::CodeActionOrCommand::Command(c) => c.title.clone(),
                                lt::CodeActionOrCommand::CodeAction(c) => c.title.clone(),
                            };
                            self.companions.owners.insert(title, id);
                        }
                        self.add_companion_actions(actions);
                    }
                    Ask::Completion { uid } => {
                        let items = match serde_json::from_value::<Option<lt::CompletionResponse>>(value) {
                            Ok(Some(lt::CompletionResponse::Array(items))) => items,
                            Ok(Some(lt::CompletionResponse::List(list))) => list.items,
                            _ => Vec::new(),
                        };
                        self.add_companion_completions(uid, items);
                    }
                    Ask::Execute => {}
                }
            }
            Event::Exited(why) => {
                coiman_core::applog::info(&format!("companion: {} stopped: {why}", spec.name));
                self.status = Some(format!("{} stopped", spec.name));
                self.companions.failed.insert((spec.name, root));
                self.companions.running.remove(at);
                self.companions.diagnostics.retain(|(sid, _), _| *sid != id);
                let dropped = self.companions.pending.keys().filter(|(sid, _)| *sid == id).count();
                self.companions.pending.retain(|(sid, _), _| *sid != id);
                for _ in 0..dropped {
                    self.add_companion_actions(Vec::new());
                }
            }
        }
    }

    /// Problems the companions found in a file.
    pub(super) fn companion_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        self.companions
            .diagnostics
            .iter()
            .filter(|((_, k), _)| k == key)
            .flat_map(|(_, (_, d))| d.iter().cloned())
            .collect()
    }

    /// The companions that have document `i` open, by index, each sent its latest text
    /// first: a request made as a key is typed comes before the frame's syncing.
    fn serving(&mut self, i: usize) -> Vec<usize> {
        let Some(doc) = self.docs.get(i) else { return Vec::new() };
        let (uid, version) = (doc.uid, doc.buffer.version());
        let mut out = Vec::new();
        for (n, r) in self.companions.running.iter_mut().enumerate() {
            if !r.client.is_ready() {
                continue;
            }
            let Some((uri, sent)) = r.open.get(&uid).cloned() else { continue };
            if sent != version {
                r.client.did_change(&uri, doc.text());
                r.open.insert(uid, (uri, version));
            }
            out.push(n);
        }
        out
    }

    /// Whether a companion has document `i` open.
    fn has_companion(&self, i: usize) -> bool {
        let Some(doc) = self.docs.get(i) else { return false };
        self.companions.running.iter().any(|r| r.client.is_ready() && r.open.contains_key(&doc.uid))
    }

    /// Asks the companions for code actions on `range` of document `i`. Returns how many
    /// were asked.
    pub(super) fn companion_code_actions(&mut self, i: usize, range: lt::Range) -> usize {
        let Some(path) = self.docs.get(i).and_then(|d| d.path.clone()) else { return 0 };
        let mut asked = 0;
        for n in self.serving(i) {
            let id = self.companions.running[n].id;
            let Some((uri, _)) = self.companions.running[n].open.get(&self.docs[i].uid).cloned() else { continue };
            // Its own problems on these lines: fixes come for them.
            let diagnostics: Vec<lt::Diagnostic> = self
                .companions
                .diagnostics
                .get(&(id, path_key(&path)))
                .map(|(_, d)| {
                    d.iter()
                        .filter(|d| d.range.start.line <= range.end.line && d.range.end.line >= range.start.line)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let params = json!({
                "textDocument": { "uri": uri },
                "range": range,
                "context": { "diagnostics": diagnostics, "triggerKind": 1 }
            });
            let rid = self.companions.running[n].client.request("textDocument/codeAction", params);
            self.companions.pending.insert((id, rid), Ask::Actions);
            asked += 1;
        }
        asked
    }

    /// Whether a companion found a problem on `line` of document `i`.
    pub(super) fn companion_problem_on(&self, i: usize, line: u32) -> bool {
        let Some(path) = self.docs.get(i).and_then(|d| d.path.as_deref()) else { return false };
        let key = path_key(path);
        self.companions.diagnostics.iter().any(|((_, k), (_, d))| {
            *k == key && d.iter().any(|d| d.range.start.line <= line && d.range.end.line >= line)
        })
    }

    /// Asks the companions to complete at `head` of document `i`. Returns how many.
    pub(super) fn companion_completion(&mut self, i: usize, head: usize, context: &Value) -> usize {
        let Some(doc) = self.docs.get(i) else { return 0 };
        let (uid, position) = (doc.uid, convert::position_of(doc.buffer.rope(), head));
        let mut asked = 0;
        for n in self.serving(i) {
            // Only those that complete: ESLint doesn't.
            if self.companions.running[n].client.capabilities().is_none_or(|c| c.completion_provider.is_none()) {
                continue;
            }
            let Some((uri, _)) = self.companions.running[n].open.get(&uid).cloned() else { continue };
            let params = json!({ "textDocument": { "uri": uri }, "position": position, "context": context });
            let id = self.companions.running[n].id;
            let rid = self.companions.running[n].client.request("textDocument/completion", params);
            self.companions.pending.insert((id, rid), Ask::Completion { uid });
            asked += 1;
        }
        asked
    }

    /// A file was saved: the companions under its folder hear of the change, as from a
    /// file watcher (Tailwind reads its CSS and configuration again).
    pub(super) fn companion_saved(&mut self, path: &Path) {
        let Some(uri) = convert::uri_from_path(path) else { return };
        for r in &mut self.companions.running {
            if path.starts_with(&r.root) {
                r.client.notify("workspace/didChangeWatchedFiles", json!({ "changes": [{ "uri": uri, "type": 2 }] }));
            }
        }
    }

    /// Whether a companion has document `i` open, and the characters that make it complete.
    pub(super) fn companion_triggers(&self, i: usize) -> Option<Vec<char>> {
        let uid = self.docs.get(i)?.uid;
        self.has_companion(i).then(|| {
            self.companions
                .running
                .iter()
                .filter(|r| r.open.contains_key(&uid))
                .flat_map(|r| r.client.completion_triggers())
                .collect()
        })
    }

    /// Carries out a companion's command (`eslint.applySingleFix` and the like).
    pub(super) fn companion_execute(&mut self, owner: u64, command: lt::Command) {
        let Some(r) = self.companions.running.iter_mut().find(|r| r.id == owner) else { return };
        let params = json!({ "command": command.command, "arguments": command.arguments.unwrap_or_default() });
        let rid = r.client.request("workspace/executeCommand", params);
        self.companions.pending.insert((owner, rid), Ask::Execute);
    }

    /// The companions running, for the language room: name and folder.
    pub(super) fn companion_names(&self) -> Vec<(String, PathBuf)> {
        self.companions.running.iter().map(|r| (r.spec.name.to_owned(), r.root.clone())).collect()
    }
}

/// Asks for a file's problems, if the server answers such requests.
fn pull(client: &mut Client, uri: &lt::Url) -> Option<i64> {
    client.capabilities()?.diagnostic_provider.as_ref()?;
    Some(client.request("textDocument/diagnostic", json!({ "textDocument": { "uri": uri } })))
}

fn start(spec: &'static Companion, root: &Path, ctx: &egui::Context) -> Result<Client, String> {
    let script =
        spec.installed_script().ok_or_else(|| format!("its VS Code extension ({}) isn't installed", spec.extension))?;
    let node = which("node").ok_or("needs Node.js (node on PATH)")?;
    let launch = Launch {
        program: node,
        args: vec![script.to_string_lossy().into_owned(), "--stdio".to_owned()],
        tcp: None,
        env: Vec::new(),
    };
    // A kind of its own: these never mix with the main servers.
    let kind = ServerKind::Custom(u16::MAX - companions::ALL.iter().position(|c| *c == spec).unwrap_or(0) as u16);
    // Files are watched here: Tailwind's own watcher reads its start as changes, and
    // restarts its project over and over.
    Client::start_with(kind, &launch, root, Value::Null, true, repaint(ctx)).map_err(|e| e.to_string())
}
