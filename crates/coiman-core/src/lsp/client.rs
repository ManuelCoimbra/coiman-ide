//! One running language server: a child process spoken to over stdio, or a
//! server already running on this computer spoken to over TCP, with background
//! threads for reading and writing so the UI never blocks.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lsp_types::{ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url};
use serde_json::{Value, json};

use super::convert::diff_change;
use super::discovery::quiet_command;
use super::servers::{Launch, ServerKind, is_local_address};
use super::status::ServerStatus;
use super::transport::{encode, read_message};

/// How long to wait for a server on this computer to answer the connection.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Lines of the server's stderr kept to explain a crash.
const STDERR_LINES: usize = 40;

/// Something the server sent.
#[derive(Debug)]
pub enum Event {
    /// Reply to one of our requests.
    Response {
        id: i64,
        result: Result<Value, String>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A request from the server; answer it with [`Client::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The server process ended (or failed to initialize); the text says why.
    Exited(String),
}

struct OpenDoc {
    version: i32,
    /// The text the server has, to send changes as small diffs.
    text: String,
}

enum Server {
    Process(Child),
    Socket(TcpStream),
}

/// Connects to a server that's already running. Only ever on this computer.
fn connect(address: &str) -> std::io::Result<TcpStream> {
    use std::io::{Error, ErrorKind};
    if !is_local_address(address) {
        return Err(Error::new(ErrorKind::PermissionDenied, format!("{address} isn't on this computer")));
    }
    let target = address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("{address} doesn't resolve")))?;
    let stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)
        .map_err(|e| Error::new(e.kind(), format!("nothing on {address} ({})", e.kind())))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

pub struct Client {
    pub kind: ServerKind,
    pub root: PathBuf,
    /// What's at the other end: a program we started, or one we only connected to.
    server: Server,
    out: Sender<Vec<u8>>,
    events: Receiver<Event>,
    next_id: i64,
    init_id: i64,
    capabilities: Option<ServerCapabilities>,
    /// Messages held back until the server has answered `initialize`.
    queued: Vec<Value>,
    open: HashMap<Url, OpenDoc>,
    exited: bool,
    pub started: Instant,
    /// The server's own report, for servers that send one.
    pub status: Option<ServerStatus>,
}

impl Client {
    /// Starts the server in `root` and sends `initialize` (with the server's
    /// own `options`). `wake` is called from a background thread whenever a
    /// message arrives.
    pub fn start(
        kind: ServerKind,
        launch: &Launch,
        root: &Path,
        options: Value,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        Self::start_with(kind, launch, root, options, false, wake)
    }

    /// `start`, saying whether the client tells the server about changed files
    /// (`workspace/didChangeWatchedFiles`): a server that's told so doesn't watch them
    /// itself.
    pub fn start_with(
        kind: ServerKind,
        launch: &Launch,
        root: &Path,
        options: Value,
        client_watches: bool,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (server, mut stdin, stdout, stderr): (_, Box<dyn Write + Send>, Box<dyn Read + Send>, _) =
            if let Some(address) = &launch.tcp {
                let stream = connect(address)?;
                (Server::Socket(stream.try_clone()?), Box::new(stream.try_clone()?), Box::new(stream), None)
            } else {
                let mut child = quiet_command(&launch.program)
                    .args(&launch.args)
                    .envs(launch.env.iter().map(|(k, v)| (k, v)))
                    .current_dir(root)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;
                let stdin = child.stdin.take().expect("piped stdin");
                let stdout = child.stdout.take().expect("piped stdout");
                let stderr = child.stderr.take().expect("piped stderr");
                (Server::Process(child), Box::new(stdin), Box::new(stdout), Some(stderr))
            };

        let (out, out_rx) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new().name("lsp-writer".into()).spawn(move || {
            for bytes in out_rx {
                if stdin.write_all(&bytes).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        })?;

        let tail = Arc::new(Mutex::new(VecDeque::new()));
        let tail_w = Arc::clone(&tail);
        if let Some(stderr) = stderr {
            std::thread::Builder::new().name("lsp-stderr".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let mut t = tail_w.lock().unwrap_or_else(|e| e.into_inner());
                    if t.len() == STDERR_LINES {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })?;
        }

        let (tx, events) = mpsc::channel();
        std::thread::Builder::new().name("lsp-reader".into()).spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_message(&mut reader) {
                    Ok(Some(msg)) => {
                        if let Some(ev) = classify(msg)
                            && tx.send(ev).is_err()
                        {
                            return;
                        }
                    }
                    Ok(None) | Err(_) => {
                        // Give the stderr thread a moment to collect the last lines.
                        std::thread::sleep(Duration::from_millis(100));
                        let t = tail.lock().unwrap_or_else(|e| e.into_inner());
                        let why = t.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
                        let _ = tx.send(Event::Exited(why));
                        wake();
                        return;
                    }
                }
                wake();
            }
        })?;

        let mut client = Self {
            kind,
            root: root.to_path_buf(),
            server,
            out,
            events,
            next_id: 1,
            init_id: 0,
            capabilities: None,
            queued: Vec::new(),
            open: HashMap::new(),
            exited: false,
            started: Instant::now(),
            status: None,
        };
        let root_uri = Url::from_directory_path(root).ok();
        let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
        let mut capabilities = client_capabilities();
        if client_watches {
            capabilities["workspace"]["didChangeWatchedFiles"] = json!({ "dynamicRegistration": true });
        }
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "Coiman IDE", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": root_uri,
            "workspaceFolders": [{ "uri": root_uri, "name": name }],
            "capabilities": capabilities,
            "initializationOptions": options,
        });
        client.init_id = client.next_id;
        client.next_id += 1;
        client.write(&json!({ "jsonrpc": "2.0", "id": client.init_id, "method": "initialize", "params": params }));
        Ok(client)
    }

    fn write(&self, msg: &Value) {
        let _ = self.out.send(encode(msg));
    }

    fn send(&mut self, msg: Value) {
        if self.capabilities.is_some() {
            self.write(&msg);
        } else {
            self.queued.push(msg);
        }
    }

    /// The server's process, when we started it.
    pub fn pid(&self) -> Option<u32> {
        match &self.server {
            Server::Process(child) => Some(child.id()),
            Server::Socket(_) => None,
        }
    }

    /// Whether the server has answered `initialize`.
    pub fn is_ready(&self) -> bool {
        self.capabilities.is_some()
    }

    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    /// Sends a request and returns its id (held until the server is ready).
    pub fn request(&mut self, method: &str, params: Value) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Answers a request from the server.
    pub fn respond(&mut self, id: Value, result: Result<Value, (i64, String)>) {
        let msg = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        self.write(&msg);
    }

    /// Everything received since the last call. Finishes the handshake
    /// (`initialized`, queued messages) when `initialize` is answered.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(ev) = self.events.try_recv() {
            match ev {
                Event::Response { id, result } if id == self.init_id => match result {
                    Ok(v) => {
                        let caps = v.get("capabilities").cloned().unwrap_or(Value::Null);
                        self.capabilities = Some(serde_json::from_value(caps).unwrap_or_default());
                        self.write(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
                        for msg in std::mem::take(&mut self.queued) {
                            self.write(&msg);
                        }
                    }
                    Err(e) => {
                        self.exited = true;
                        events.push(Event::Exited(format!("initialize failed: {e}")));
                    }
                },
                Event::Exited(why) => {
                    self.exited = true;
                    events.push(Event::Exited(why));
                }
                ev => events.push(ev),
            }
        }
        events
    }

    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// Characters that should open completion (e.g. `.` and `:`).
    pub fn completion_triggers(&self) -> Vec<char> {
        let chars = self.capabilities.as_ref().and_then(|c| c.completion_provider.as_ref()?.trigger_characters.clone());
        chars.unwrap_or_default().iter().filter_map(|s| s.chars().next()).collect()
    }

    pub fn is_open(&self, uri: &Url) -> bool {
        self.open.contains_key(uri)
    }

    pub fn did_open(&mut self, uri: Url, language_id: &str, text: String) {
        if self.open.contains_key(&uri) {
            return;
        }
        let item = json!({ "uri": uri, "languageId": language_id, "version": 0, "text": text });
        self.notify("textDocument/didOpen", json!({ "textDocument": item }));
        self.open.insert(uri, OpenDoc { version: 0, text });
    }

    /// Sends the new text of an open document (only the changed span, unless
    /// the server asked for full documents).
    pub fn did_change(&mut self, uri: &Url, text: String) {
        let full = self.capabilities.as_ref().and_then(|c| c.text_document_sync.as_ref()).is_some_and(|s| match s {
            TextDocumentSyncCapability::Kind(k) => *k == TextDocumentSyncKind::FULL,
            TextDocumentSyncCapability::Options(o) => o.change == Some(TextDocumentSyncKind::FULL),
        });
        let Some(doc) = self.open.get_mut(uri) else { return };
        let change = if full {
            if doc.text == text {
                return;
            }
            json!({ "text": text })
        } else {
            let Some((range, new)) = diff_change(&doc.text, &text) else { return };
            json!({ "range": range, "text": new })
        };
        doc.version += 1;
        doc.text = text;
        let params = json!({ "textDocument": { "uri": uri, "version": doc.version }, "contentChanges": [change] });
        self.notify("textDocument/didChange", params);
    }

    pub fn did_save(&mut self, uri: &Url) {
        if self.open.contains_key(uri) {
            self.notify("textDocument/didSave", json!({ "textDocument": { "uri": uri } }));
        }
    }

    pub fn did_close(&mut self, uri: &Url) {
        if self.open.remove(uri).is_some() {
            self.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri } }));
        }
    }
}

impl Drop for Client {
    /// Asks the server to exit, then makes sure the process is gone.
    fn drop(&mut self) {
        // It isn't ours to stop: only hang up, which also ends the reader thread.
        if let Server::Socket(stream) = &self.server {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        let Server::Process(child) = &mut self.server else { return };
        if !self.exited {
            let _ = self
                .out
                .send(encode(&json!({ "jsonrpc": "2.0", "id": self.next_id, "method": "shutdown", "params": null })));
            let _ = self.out.send(encode(&json!({ "jsonrpc": "2.0", "method": "exit", "params": null })));
            let deadline = Instant::now() + Duration::from_millis(500);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Sorts an incoming message into a response, notification or request.
fn classify(mut msg: Value) -> Option<Event> {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let id = msg.get_mut("id").map(Value::take);
    let params = msg.get_mut("params").map(Value::take).unwrap_or(Value::Null);
    match (method, id) {
        (Some(method), Some(id)) => Some(Event::Request { id, method, params }),
        (Some(method), None) => Some(Event::Notification { method, params }),
        (None, Some(id)) => {
            let id = id.as_i64()?;
            let result = match msg.get_mut("error") {
                Some(err) => Err(err.get("message").and_then(Value::as_str).unwrap_or("request failed").to_owned()),
                None => Ok(msg.get_mut("result").map(Value::take).unwrap_or(Value::Null)),
            };
            Some(Event::Response { id, result })
        }
        (None, None) => None,
    }
}

/// What this editor supports, so servers only send what it can show.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "textDocument": {
            "synchronization": { "didSave": true },
            "publishDiagnostics": { "versionSupport": true },
            "completion": {
                "completionItem": {
                    "snippetSupport": true,
                    "documentationFormat": ["markdown", "plaintext"],
                    "labelDetailsSupport": true,
                    "insertReplaceSupport": true,
                    "deprecatedSupport": true,
                    "tagSupport": { "valueSet": [1] },
                    // Filled in when picked or shown: rust-analyzer only offers completions
                    // that add an import (`additionalTextEdits`) to a client that asks.
                    "resolveSupport": { "properties": ["documentation", "detail", "additionalTextEdits"] },
                },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": true },
            "implementation": { "linkSupport": true },
            "references": {},
            "rename": {},
            "codeAction": {
                "codeActionLiteralSupport": {
                    "codeActionKind": {
                        "valueSet": ["", "quickfix", "refactor", "refactor.extract", "refactor.inline",
                                     "refactor.rewrite", "source", "source.organizeImports"],
                    },
                },
            },
            "formatting": {},
            "typeDefinition": { "linkSupport": true },
            "documentHighlight": {},
            "signatureHelp": {
                "signatureInformation": {
                    "documentationFormat": ["markdown", "plaintext"],
                    "parameterInformation": { "labelOffsetSupport": true },
                },
            },
            "inlayHint": {},
            "diagnostic": {},
            "codeLens": {},
            "callHierarchy": {},
            "typeHierarchy": {},
            "semanticTokens": {
                "requests": { "full": true },
                "tokenTypes": ["namespace", "type", "class", "enum", "interface", "struct", "typeParameter",
                               "parameter", "variable", "property", "enumMember", "function", "method", "macro",
                               "keyword", "modifier", "comment", "string", "number", "operator"],
                "tokenModifiers": [],
                "formats": ["relative"],
            },
        },
        "workspace": {
            "applyEdit": true,
            "workspaceEdit": { "documentChanges": true, "resourceOperations": ["create", "rename", "delete"] },
            "configuration": true,
            "workspaceFolders": true,
            "symbol": {},
            "semanticTokens": { "refreshSupport": true },
            "inlayHint": { "refreshSupport": true },
            "diagnostics": { "refreshSupport": true },
            "codeLens": { "refreshSupport": true },
            "textDocumentContent": {},
        },
        "window": { "workDoneProgress": true, "showMessage": {}, "showDocument": { "support": true } },
        "experimental": {
            "serverStatusNotification": true,
            // rust-analyzer offers a lens only if the client says it can do what it runs.
            "commands": { "commands": ["rust-analyzer.runSingle", "rust-analyzer.showReferences", "rust-analyzer.gotoLocation"] },
        },
    })
}

/// The LSP language id for a file extension, if a server is known for it.
pub fn language_id(ext: &str) -> Option<&'static str> {
    use super::registry::by_extension;
    if let Some(language) = by_extension(ext, true) {
        return Some(language.language_id);
    }
    built_in_language_id(ext).or_else(|| by_extension(ext, false).map(|l| l.language_id))
}

fn built_in_language_id(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "py" | "pyw" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "c" => "c",
        // Headers go to clangd as C++; it reads compile flags to tell.
        "h" | "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "inl" => "cpp",
        "java" => "java",
        "cs" => "csharp",
        "go" => "go",
        "gd" => "gdscript",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completions_can_be_filled_in_on_request() {
        // Without this, rust-analyzer offers no completion that adds an import.
        let caps = client_capabilities();
        let item = &caps["textDocument"]["completion"]["completionItem"];
        let props: Vec<&str> =
            item["resolveSupport"]["properties"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(props.contains(&"additionalTextEdits"));
        assert!(props.contains(&"documentation"));
        assert_eq!(item["insertReplaceSupport"], true);
    }

    #[test]
    fn classifies_messages() {
        let ev = classify(json!({"jsonrpc": "2.0", "id": 3, "result": {"x": 1}})).unwrap();
        assert!(matches!(ev, Event::Response { id: 3, result: Ok(v) } if v["x"] == 1));
        let ev = classify(json!({"jsonrpc": "2.0", "id": 4, "error": {"code": -1, "message": "nope"}})).unwrap();
        assert!(matches!(ev, Event::Response { id: 4, result: Err(e) } if e == "nope"));
        let ev = classify(json!({"jsonrpc": "2.0", "method": "$/progress", "params": {}})).unwrap();
        assert!(matches!(ev, Event::Notification { method, .. } if method == "$/progress"));
        let ev = classify(json!({"jsonrpc": "2.0", "id": "a", "method": "workspace/configuration"})).unwrap();
        assert!(matches!(ev, Event::Request { method, .. } if method == "workspace/configuration"));
    }
}
