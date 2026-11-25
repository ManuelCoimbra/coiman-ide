//! Launches the Node extension host and speaks newline-delimited JSON to it.
//! The host loads extension `main` scripts with a `vscode` API shim; Coiman
//! only starts it after the first frame so startup stays fast.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use serde_json::{Value, json};

const HOST_JS: &str = include_str!("host.js");

/// A message from the extension host.
#[derive(Debug, Clone)]
pub struct Message {
    pub method: String,
    pub params: Value,
    pub id: Option<Value>,
}

/// One extension the host can activate.
#[derive(Clone, Debug)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub dir: PathBuf,
    pub main: String,
    pub version: String,
    pub package: Value,
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&crate::appearance::theme::strip_jsonc(&text)).ok()
}

fn extension_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        out.push(home.join(".vscode").join("extensions"));
        out.push(home.join(".vscode-insiders").join("extensions"));
        out.push(home.join(".vscode-server").join("extensions"));
    }
    if let Some(dir) = crate::extensions::vscode_import::coiman_extensions_dir() {
        out.push(dir);
    }
    for sys in [
        "/usr/share/code/resources/app/extensions",
        "/usr/lib/code/extensions",
        "/opt/visual-studio-code/resources/app/extensions",
        "/var/lib/flatpak/app/com.visualstudio.code/current/active/files/extra/vscode/resources/app/extensions",
    ] {
        let p = PathBuf::from(sys);
        if p.is_dir() {
            out.push(p);
        }
    }
    out
}

/// Installed extensions with a `main` entrypoint or `contributes`, newest first per id.
pub fn extensions() -> Vec<Extension> {
    let mut found: Vec<Extension> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in extension_dirs() {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let Some(package) = read_json(&folder.join("package.json")) else { continue };
            let publisher = package.get("publisher").and_then(Value::as_str).unwrap_or("");
            let name = package.get("name").and_then(Value::as_str).unwrap_or("");
            if publisher.is_empty() || name.is_empty() {
                continue;
            }
            let id = format!("{publisher}.{name}").to_lowercase();
            let main = package.get("main").and_then(Value::as_str).unwrap_or("").trim_start_matches("./");
            if !main.is_empty() {
                let script = folder.join(main);
                if !script.is_file() && !script.with_extension("js").is_file() {
                    continue;
                }
            } else if package.get("contributes").is_none() {
                continue;
            }
            if !seen.insert(id.clone()) {
                continue;
            }
            found.push(Extension {
                id,
                name: package.get("displayName").and_then(Value::as_str).unwrap_or(name).to_owned(),
                dir: folder,
                main: main.to_owned(),
                version: package.get("version").and_then(Value::as_str).unwrap_or("").to_owned(),
                package,
            });
        }
    }
    found
}

fn script_path() -> Option<PathBuf> {
    let dir = crate::settings::config_dir()?;
    let file = dir.join("coiman-ext-host.js");
    if std::fs::read_to_string(&file).ok().as_deref() != Some(HOST_JS) {
        std::fs::write(&file, HOST_JS).ok()?;
    }
    let vscode_pkg_dir = dir.join("node_modules").join("vscode");
    let _ = std::fs::create_dir_all(&vscode_pkg_dir);
    let pkg_json = vscode_pkg_dir.join("package.json");
    let _ = std::fs::write(
        &pkg_json,
        r#"{"name":"vscode","version":"1.99.0","main":"index.js","module":"index.mjs","exports":{".":{"import":"./index.mjs","require":"./index.js"},"./package.json":"./package.json"}}"#,
    );
    let index_js = vscode_pkg_dir.join("index.js");
    let _ = std::fs::write(&index_js, "module.exports = globalThis.vscode || {};\n");
    let index_mjs = vscode_pkg_dir.join("index.mjs");
    let _ = std::fs::write(&index_mjs, "const _v = globalThis.vscode || {}; export default _v;\n");
    Some(file)
}

/// A view container an extension adds to the activity bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewContainer {
    pub id: String,
    pub title: String,
    pub icon: PathBuf,
    pub views: Vec<(String, String)>,
}

fn resolve_nls_title(dir: &Path, raw_title: &str) -> String {
    if raw_title.starts_with('%') && raw_title.ends_with('%') && raw_title.len() > 2 {
        let key = &raw_title[1..raw_title.len() - 1];
        let nls_path = dir.join("package.nls.json");
        if let Some(nls) = read_json(&nls_path)
            && let Some(val) = nls.get(key).and_then(Value::as_str)
        {
            return val.to_owned();
        }
        let last = key.split('.').next_back().unwrap_or(key);
        let mut s = String::new();
        for (i, ch) in last.chars().enumerate() {
            if ch == '_' || ch == '-' {
                s.push(' ');
            } else if i == 0 {
                s.extend(ch.to_uppercase());
            } else {
                s.push(ch);
            }
        }
        return s;
    }
    raw_title.to_owned()
}

/// Activity-bar containers and their views from installed extensions.
pub fn view_containers() -> Vec<ViewContainer> {
    Host::default().all_view_containers()
}

/// A running extension host.
pub struct Host {
    child: Option<Child>,
    stdin: Option<std::process::ChildStdin>,
    rx: Receiver<Message>,
    pub extensions: Vec<Extension>,
    next_id: i64,
    documents: std::collections::HashMap<String, u64>,
    activated: std::collections::HashSet<String>,
    pub webview_server: Option<Arc<super::webview::WebviewServer>>,
}

impl Default for Host {
    fn default() -> Self {
        let (_, rx) = mpsc::channel();
        Self {
            child: None,
            stdin: None,
            rx,
            extensions: Vec::new(),
            next_id: 1,
            documents: std::collections::HashMap::new(),
            activated: std::collections::HashSet::new(),
            webview_server: None,
        }
    }
}

impl Host {
    pub fn running(&self) -> bool {
        self.child.is_some()
    }

    /// Starts the Node host. Called after the first frame, never at startup.
    pub fn start(&mut self) -> Result<(), String> {
        self.start_with(Vec::new(), json!({}))
    }

    /// Starts the Node host with workspace folders and initial configuration.
    pub fn start_with(&mut self, workspace_folders: Vec<Value>, config: Value) -> Result<(), String> {
        if self.child.is_some() {
            return Ok(());
        }
        let node = crate::language::lsp::servers::which("node").ok_or("Node.js is needed for VS Code extensions")?;
        let script = script_path().ok_or("Couldn't write the extension host script")?;
        let node_path = script.parent().map(|p| p.join("node_modules")).unwrap_or_default();
        let mut cmd = Command::new(node);
        cmd.arg(&script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        if !node_path.as_os_str().is_empty() {
            let env_node_path = match std::env::var_os("NODE_PATH") {
                Some(existing) => {
                    let mut p = std::ffi::OsString::from(&node_path);
                    p.push(":");
                    p.push(existing);
                    p
                }
                None => node_path.into_os_string(),
            };
            cmd.env("NODE_PATH", env_node_path);
        }
        let webview_server = super::webview::WebviewServer::start(0, || {}).ok().map(Arc::new);
        let webview_port = webview_server.as_ref().map(|s| s.port()).unwrap_or(0);
        if webview_port > 0 {
            cmd.env("WEBVIEW_PORT", webview_port.to_string());
        }
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the extension host: {e}"))?;
        let stdin = child.stdin.take().ok_or("No extension host stdin")?;
        self.stdin = Some(stdin);
        let stdout = child.stdout.take().ok_or("No extension host stdout")?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("[coiman-ext-host] {line}");
                }
            });
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let message = Message {
                    method: value.get("method").and_then(Value::as_str).unwrap_or("").to_owned(),
                    params: value.get("params").cloned().unwrap_or(Value::Null),
                    id: value.get("id").cloned(),
                };
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        self.child = Some(child);
        self.rx = rx;
        self.webview_server = webview_server;
        self.extensions = extensions();
        let list: Vec<Value> = self
            .extensions
            .iter()
            .map(|e| {
                json!({
                    "id": e.id, "name": e.name, "dir": e.dir, "main": e.main,
                    "version": e.version, "package": e.package,
                })
            })
            .collect();
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = serde_json::to_writer(
                &mut *stdin,
                &json!({
                    "jsonrpc": "2.0", "method": "initialize",
                    "params": {
                        "workspaceFolders": workspace_folders,
                        "config": config,
                        "extensions": list,
                        "webviewPort": webview_port,
                    },
                }),
            );
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
        Ok(())
    }

    pub fn webview_server(&self) -> Option<&Arc<super::webview::WebviewServer>> {
        self.webview_server.as_ref()
    }

    pub fn webview_port(&self) -> Option<u16> {
        self.webview_server.as_ref().map(|s| s.port())
    }

    pub fn set_selection(&mut self, uri: &str, start_line: u32, start_col: u32, end_line: u32, end_col: u32) {
        if self.running() {
            self.notify(
                "editor/selection",
                json!({
                    "uri": uri,
                    "start": { "line": start_line, "character": start_col },
                    "end": { "line": end_line, "character": end_col },
                }),
            );
        }
    }

    pub fn discover(&mut self) {
        self.extensions = extensions();
    }

    /// Shuts down the Node extension host process and cleans up any servers.
    pub fn shutdown(&mut self) -> Result<(), String> {
        if self.child.is_none() {
            return Ok(());
        }
        if let Some(mut stdin) = self.stdin.take() {
            let _ = serde_json::to_writer(
                &mut stdin,
                &json!({
                    "jsonrpc": "2.0",
                    "method": "shutdown",
                    "params": {},
                }),
            );
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.webview_server = None;
        self.activated.clear();
        self.documents.clear();
        let (_, rx) = mpsc::channel();
        self.rx = rx;
        Ok(())
    }

    /// Adds and registers an extension from a folder while the host is running.
    pub fn add_extension(&mut self, folder: &Path) {
        let Some(package) = read_json(&folder.join("package.json")) else { return };
        let publisher = package.get("publisher").and_then(Value::as_str).unwrap_or("");
        let name = package.get("name").and_then(Value::as_str).unwrap_or("");
        if publisher.is_empty() || name.is_empty() {
            return;
        }
        let id = format!("{publisher}.{name}").to_lowercase();
        let main = package.get("main").and_then(Value::as_str).unwrap_or("").trim_start_matches("./");
        if main.is_empty() && package.get("contributes").is_none() {
            return;
        }
        let ext = Extension {
            id: id.clone(),
            name: package.get("displayName").and_then(Value::as_str).unwrap_or(name).to_owned(),
            dir: folder.to_path_buf(),
            main: main.to_owned(),
            version: package.get("version").and_then(Value::as_str).unwrap_or("").to_owned(),
            package: package.clone(),
        };
        if !self.extensions.iter().any(|e| e.id == id) {
            self.extensions.push(ext.clone());
        }
        if self.running() {
            self.notify(
                "extension/register",
                json!({
                    "extension": {
                        "id": ext.id, "name": ext.name, "dir": ext.dir, "main": ext.main,
                        "version": ext.version, "package": ext.package,
                    }
                }),
            );
            if Self::has_startup_event(&ext) {
                self.activate(&ext.id);
            }
        }
    }

    /// Updates workspace folders in the running extension host.
    pub fn set_workspace_folders(&mut self, folders: Vec<Value>) {
        if self.running() {
            self.notify("workspace/setFolders", json!({ "workspaceFolders": folders }));
        }
    }

    pub fn poll(&mut self) -> Vec<Message> {
        let mut out = Vec::new();
        while let Ok(message) = self.rx.try_recv() {
            out.push(message);
        }
        out
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Option<&'static str> {
        self.child.as_ref()?;
        let stdin = self.stdin.as_mut()?;
        let _ = serde_json::to_writer(&mut *stdin, &json!({ "jsonrpc": "2.0", "method": method, "params": params }));
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
        None
    }

    /// Sends a document's text when it first opens or changes.
    pub fn document(&mut self, uri: &str, language: &str, text: &str, version: u64) {
        self.activate_for_language(language);
        let changed = self.documents.get(uri).is_none_or(|sent| *sent != version);
        if self.child.is_none() || !changed {
            return;
        }
        self.documents.insert(uri.to_owned(), version);
        self.notify("document/change", json!({ "uri": uri, "languageId": language, "text": text, "version": version }));
    }

    fn event_matches(extension: &Extension, needle: &str) -> bool {
        extension
            .package
            .get("activationEvents")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|event| event == needle)
    }

    fn has_startup_event(extension: &Extension) -> bool {
        Self::event_matches(extension, "onStartupFinished")
    }

    fn has_view_event(extension: &Extension, view: &str) -> bool {
        Self::event_matches(extension, &format!("onView:{view}"))
    }

    /// Activates one extension by id.
    pub fn activate(&mut self, id: &str) {
        let id = id.to_lowercase();
        if !self.activated.insert(id.clone()) || self.child.is_none() {
            return;
        }
        self.notify("activate/extension", serde_json::json!({ "id": id }));
        self.notify("extension/activate", serde_json::json!({ "id": id }));
    }

    /// Activates extensions that asked for a language.
    pub fn activate_for_language(&mut self, language: &str) {
        let ids: Vec<String> = self
            .extensions
            .iter()
            .filter(|e| {
                Self::event_matches(e, &format!("onLanguage:{language}")) || Self::event_matches(e, "onLanguage")
            })
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            self.activate(&id);
        }
    }

    /// Activates extensions that asked for a command.
    pub fn activate_for_command(&mut self, command: &str) {
        let ids: Vec<String> = self
            .extensions
            .iter()
            .filter(|e| Self::event_matches(e, &format!("onCommand:{command}")))
            .map(|e| e.id.clone())
            .collect();
        for id in ids {
            self.activate(&id);
        }
    }

    /// Activates extensions that own a view.
    pub fn activate_for_view(&mut self, view: &str) {
        let ids: Vec<String> =
            self.extensions.iter().filter(|e| Self::has_view_event(e, view)).map(|e| e.id.clone()).collect();
        for id in ids {
            self.activate(&id);
        }
    }

    /// All view containers contributed by installed extensions, with localized titles.
    pub fn all_view_containers(&self) -> Vec<ViewContainer> {
        let mut out = Vec::new();
        let exts = if self.extensions.is_empty() { extensions() } else { self.extensions.clone() };
        for extension in &exts {
            let contributes = &extension.package["contributes"];
            let containers =
                contributes.get("viewsContainers").and_then(|v| v.get("activitybar")).and_then(Value::as_array);
            let Some(containers) = containers else { continue };
            for container in containers {
                let Some(id) = container.get("id").and_then(Value::as_str) else { continue };
                let raw_title = container.get("title").and_then(Value::as_str).unwrap_or(id);
                let title = resolve_nls_title(&extension.dir, raw_title);
                let icon = container
                    .get("icon")
                    .and_then(Value::as_str)
                    .map(|icon| extension.dir.join(icon))
                    .filter(|icon| icon.is_file())
                    .unwrap_or_default();
                let mut views: Vec<(String, String)> = contributes
                    .get("views")
                    .and_then(|v| v.get(id))
                    .and_then(Value::as_array)
                    .map(|views| {
                        views
                            .iter()
                            .filter_map(|view| {
                                Some((
                                    view.get("id").and_then(Value::as_str)?.to_owned(),
                                    view.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if views.is_empty() {
                    views.push((id.to_owned(), title.clone()));
                }
                out.push(ViewContainer { id: id.to_owned(), title, icon, views });
            }
        }
        out
    }

    /// Containers whose extension is active or asked to appear at startup.
    pub fn view_containers(&self) -> Vec<ViewContainer> {
        let mut out = Vec::new();
        for extension in &self.extensions {
            if !self.activated.contains(&extension.id) && !Self::has_startup_event(extension) {
                continue;
            }
            let contributes = &extension.package["contributes"];
            let containers =
                contributes.get("viewsContainers").and_then(|v| v.get("activitybar")).and_then(Value::as_array);
            let Some(containers) = containers else { continue };
            for container in containers {
                let Some(id) = container.get("id").and_then(Value::as_str) else { continue };
                let title = container.get("title").and_then(Value::as_str).unwrap_or(id).to_owned();
                let icon = container
                    .get("icon")
                    .and_then(Value::as_str)
                    .map(|icon| extension.dir.join(icon))
                    .filter(|icon| icon.is_file())
                    .unwrap_or_default();
                let views: Vec<(String, String)> = contributes
                    .get("views")
                    .and_then(|v| v.get(id))
                    .and_then(Value::as_array)
                    .map(|views| {
                        views
                            .iter()
                            .filter_map(|view| {
                                Some((
                                    view.get("id").and_then(Value::as_str)?.to_owned(),
                                    view.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !views.is_empty() {
                    out.push(ViewContainer { id: id.to_owned(), title, icon, views });
                }
            }
        }
        out
    }

    /// Commands contributed by installed extensions, as (title, id).
    pub fn commands(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for extension in &self.extensions {
            let Some(commands) =
                extension.package.get("contributes").and_then(|c| c.get("commands")).and_then(Value::as_array)
            else {
                continue;
            };
            for command in commands {
                let Some(id) = command.get("command").and_then(Value::as_str) else { continue };
                let title = command.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or(id);
                out.push((title.to_owned(), id.to_owned()));
            }
        }
        out
    }

    pub fn request(&mut self, method: &str, params: Value) -> Option<i64> {
        let id = self.next_id;
        self.next_id += 1;
        self.child.as_ref()?;
        let stdin = self.stdin.as_mut()?;
        let _ = serde_json::to_writer(
            &mut *stdin,
            &json!({
                "jsonrpc": "2.0", "id": id, "method": method, "params": params,
            }),
        );
        let _ = stdin.write_all(b"\n");
        let _ = stdin.flush();
        Some(id)
    }

    pub fn stop(&mut self) {
        let _ = self.shutdown();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_script_is_embedded() {
        assert!(HOST_JS.contains("registerCompletionItemProvider"));
        assert!(HOST_JS.contains("getConfiguration"));
        assert!(HOST_JS.contains("createDiagnosticCollection"));
        assert!(HOST_JS.contains("global.vscode = vscode"));
        for api in [
            "registerInlineCompletionItemProvider",
            "registerDocumentSemanticTokensProvider",
            "createLanguageStatusItem",
            "registerTool",
            "createChatParticipant",
            "registerNotebookController",
            "registerUriHandler",
            "registerTextDocumentContentProvider",
            "globalStorageUri",
            "getConfiguration",
        ] {
            assert!(HOST_JS.contains(api), "{api} is in the shim");
        }
    }

    #[test]
    fn a_default_host_is_not_running() {
        let host = Host::default();
        assert!(!host.running());
        assert!(host.extensions.is_empty());
    }

    #[test]
    fn view_containers_wait_for_activation() {
        let package = |events: &[&str]| {
            serde_json::json!({
                "activationEvents": events,
                "contributes": {
                    "viewsContainers": { "activitybar": [{ "id": "view", "title": "View" }] },
                    "views": { "view": [{ "id": "view.tree", "name": "Tree" }] }
                }
            })
        };
        let extension = |id: &str, events: &[&str]| Extension {
            id: id.into(),
            name: id.into(),
            dir: std::env::temp_dir(),
            main: "main.js".into(),
            version: "1.0.0".into(),
            package: package(events),
        };
        let mut host = Host::default();
        host.extensions = vec![
            extension("x.language", &["onLanguage:python"]),
            extension("x.startup", &["onStartupFinished"]),
            extension("x.view", &["onView:view.tree"]),
        ];
        assert_eq!(host.view_containers().len(), 1, "only startup extensions show first");
        host.activate("x.language");
        assert_eq!(host.view_containers().len(), 2, "activated extensions show their view");
    }

    #[test]
    fn extension_commands_are_listed_for_the_palette() {
        let mut host = Host::default();
        host.extensions.push(Extension {
            id: "example.demo".into(),
            name: "Demo".into(),
            dir: std::env::temp_dir(),
            main: "main.js".into(),
            version: "1.0.0".into(),
            package: serde_json::json!({
                "contributes": { "commands": [{ "command": "demo.hello", "title": "Say Hello" }] }
            }),
        });
        assert_eq!(host.commands(), vec![("Say Hello".to_owned(), "demo.hello".to_owned())]);
    }
}
