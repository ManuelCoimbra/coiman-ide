//! Debug Adapter Protocol client for CodeLLDB: the adapter process spoken to
//! over stdio (same framing as LSP), plus finding CodeLLDB and installing it.
//! Nothing here downloads unless [`install_codelldb`] is called, which the
//! editor only does after the user agreed.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use serde_json::{Value, json};

use crate::lsp::discovery::quiet_command;
use crate::lsp::transport::{encode, read_message};

/// Something the adapter sent.
#[derive(Debug)]
pub enum DapEvent {
    Response {
        request_seq: i64,
        success: bool,
        message: Option<String>,
        body: Value,
    },
    Event {
        event: String,
        body: Value,
    },
    /// A reverse request (e.g. `runInTerminal`); answer with [`DapClient::respond`].
    Request {
        seq: i64,
        command: String,
        arguments: Value,
    },
    Exited,
}

pub struct DapClient {
    child: Option<Child>,
    out: Sender<Vec<u8>>,
    events: Receiver<DapEvent>,
    next_seq: i64,
}

impl DapClient {
    /// Starts the adapter; `wake` is called from the reader thread on each message.
    pub fn start(
        program: &Path,
        args: &[String],
        cwd: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let mut child = quiet_command(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (out, out_rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in out_rx {
                if stdin.write_all(&bytes).and_then(|()| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok(Some(msg)) = read_message(&mut reader) {
                if let Some(ev) = classify(msg)
                    && tx.send(ev).is_err()
                {
                    return;
                }
                wake();
            }
            let _ = tx.send(DapEvent::Exited);
            wake();
        });
        Ok(Self { child: Some(child), out, events, next_seq: 1 })
    }

    /// Sends a request; returns its sequence number (matched by responses).
    pub fn request(&mut self, command: &str, arguments: Value) -> i64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let msg = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        let _ = self.out.send(encode(&msg));
        seq
    }

    /// Answers a reverse request.
    pub fn respond(&mut self, request_seq: i64, command: &str, success: bool, body: Value) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let msg = json!({
            "seq": seq, "type": "response", "request_seq": request_seq,
            "command": command, "success": success, "body": body,
        });
        let _ = self.out.send(encode(&msg));
    }

    pub fn poll(&mut self) -> Vec<DapEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for DapClient {
    /// Lets the adapter finish a `disconnect` sent just before (closing its
    /// stdin ends it), and kills it if it hasn't exited after two seconds.
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }
}

fn classify(msg: Value) -> Option<DapEvent> {
    match msg["type"].as_str()? {
        "response" => Some(DapEvent::Response {
            request_seq: msg["request_seq"].as_i64()?,
            success: msg["success"].as_bool().unwrap_or(false),
            message: msg["message"].as_str().map(str::to_owned),
            body: msg["body"].clone(),
        }),
        "event" => Some(DapEvent::Event { event: msg["event"].as_str()?.to_owned(), body: msg["body"].clone() }),
        "request" => Some(DapEvent::Request {
            seq: msg["seq"].as_i64()?,
            command: msg["command"].as_str()?.to_owned(),
            arguments: msg["arguments"].clone(),
        }),
        _ => None,
    }
}

/// `initialize` arguments for this client.
pub fn initialize_args() -> Value {
    json!({
        "clientID": "rust-code",
        "clientName": "Rust Code",
        "adapterID": "lldb",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        "supportsMemoryReferences": true,
        // The program runs in one of the editor's terminals: CodeLLDB asks for
        // its launcher there, so the program's output and input stay off the
        // adapter's stdio (on Windows the program would otherwise inherit it).
        "supportsRunInTerminalRequest": true,
    })
}

/// `launch` arguments for CodeLLDB debugging a Rust program. Unless
/// `step_into_std`, stepping skips the standard library (`std`, `core`, `alloc`).
/// The program runs in a terminal tab (see [`initialize_args`]).
pub fn launch_args(program: &Path, args: &[String], cwd: &Path, step_into_std: bool) -> Value {
    let mut init = Vec::new();
    if !step_into_std {
        init.push(r"settings set target.process.thread.step-avoid-regexp ^(std|core|alloc)::".to_owned());
    }
    json!({
        "type": "lldb",
        "request": "launch",
        "name": program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        "program": program,
        "args": args,
        "cwd": cwd,
        "stopOnEntry": false,
        // Loads CodeLLDB's Rust formatters: Vec, String, Option, HashMap… shown readably.
        "sourceLanguages": ["rust"],
        // The console evaluates expressions (LLDB commands take a `` ` `` prefix).
        "_adapterSettings": { "consoleMode": "evaluate" },
        "terminal": "integrated",
        "initCommands": init,
    })
}

// ----- adapters besides CodeLLDB ----------------------------------------------------

/// Which debugger a session uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugKind {
    /// CodeLLDB, for what cargo built.
    Lldb,
    /// debugpy, from the project's Python, with what activates its environment.
    Python { python: PathBuf, env: Vec<(String, String)> },
    /// An adapter named by a language's entry in `languages.toml`.
    Data { command: String, debug_type: String },
}

impl DebugKind {
    pub fn name(&self) -> String {
        match self {
            Self::Lldb => "CodeLLDB".to_owned(),
            Self::Python { .. } => "debugpy".to_owned(),
            Self::Data { command, .. } => command.split_whitespace().next().unwrap_or(command).to_owned(),
        }
    }

    /// The program to start, for anything but CodeLLDB, which has its own search.
    pub fn adapter(&self) -> Option<Adapter> {
        match self {
            Self::Lldb => None,
            Self::Python { python, .. } => {
                Some(Adapter { program: python.clone(), args: vec!["-m".to_owned(), "debugpy.adapter".to_owned()] })
            }
            Self::Data { command, .. } => {
                let mut words = command.split_whitespace();
                let program = crate::lsp::servers::which(words.next()?)?;
                Some(Adapter { program, args: words.map(str::to_owned).collect() })
            }
        }
    }

    /// The `launch` request for this debugger.
    pub fn launch_args(&self, program: &Path, args: &[String], cwd: &Path, step_into_std: bool) -> Value {
        let name = program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        match self {
            Self::Lldb => launch_args(program, args, cwd, step_into_std),
            Self::Python { python, env } => json!({
                "type": "python",
                "request": "launch",
                "name": name,
                "program": program,
                "args": args,
                "cwd": cwd,
                "python": python,
                "console": "integratedTerminal",
                "justMyCode": !step_into_std,
                "env": env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect::<serde_json::Map<_, _>>(),
            }),
            Self::Data { debug_type, .. } => json!({
                "type": debug_type,
                "request": "launch",
                "name": name,
                "program": program,
                "args": args,
                "cwd": cwd,
                "stopOnEntry": false,
            }),
        }
    }
}

/// Whether debugpy is installed in this Python.
pub fn python_has_debugpy(python: &Path) -> bool {
    quiet_command(python)
        .args(["-c", "import debugpy"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Installs debugpy into this Python from PyPI. Only after the user agreed.
pub fn install_debugpy(python: &Path) -> Result<(), String> {
    let out = quiet_command(python)
        .args(["-m", "pip", "install", "--disable-pip-version-check", "debugpy"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run pip: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stderr);
    Err(text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("pip failed").to_owned())
}

// ----- finding and installing CodeLLDB ---------------------------------------------

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// Where Rust Code installs CodeLLDB: `<config>/rust-code/codelldb`.
pub fn install_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("codelldb"))
}

/// The adapter executable and the arguments it needs (`--liblldb`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub program: PathBuf,
    pub args: Vec<String>,
}

fn adapter_in(extension: &Path) -> Option<Adapter> {
    let program = extension.join("adapter").join(exe("codelldb"));
    if !program.exists() {
        return None;
    }
    let lib = if cfg!(windows) {
        "liblldb.dll"
    } else if cfg!(target_os = "macos") {
        "liblldb.dylib"
    } else {
        "liblldb.so"
    };
    let liblldb = [extension.join("lldb").join("bin").join(lib), extension.join("lldb").join("lib").join(lib)]
        .into_iter()
        .find(|p| p.exists());
    let args = liblldb.map(|l| vec!["--liblldb".to_owned(), l.to_string_lossy().into_owned()]).unwrap_or_default();
    Some(Adapter { program, args })
}

/// CodeLLDB from the setting, Rust Code's own install, or VS Code's extensions.
pub fn find_codelldb(configured: &str) -> Option<Adapter> {
    if !configured.trim().is_empty() {
        let p = PathBuf::from(configured.trim());
        // Either the adapter itself or the extension folder.
        if p.is_file() {
            return Some(Adapter { program: p, args: Vec::new() });
        }
        if let Some(a) = adapter_in(&p) {
            return Some(a);
        }
    }
    if let Some(a) = install_dir().and_then(|d| adapter_in(&d.join("extension"))) {
        return Some(a);
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    let extensions = Path::new(&home).join(".vscode").join("extensions");
    let mut found: Vec<PathBuf> = std::fs::read_dir(extensions)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("vadimcn.vscode-lldb-")))
        .collect();
    found.sort();
    found.iter().rev().find_map(|p| adapter_in(p))
}

/// The CodeLLDB release package for this platform.
pub fn package_url() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-win32-x64.vsix")
        }
        ("windows", "aarch64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-win32-arm64.vsix")
        }
        ("linux", "x86_64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-linux-x64.vsix")
        }
        ("linux", "aarch64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-linux-arm64.vsix")
        }
        ("macos", "x86_64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-darwin-x64.vsix")
        }
        ("macos", "aarch64") => {
            Some("https://github.com/vadimcn/codelldb/releases/latest/download/codelldb-darwin-arm64.vsix")
        }
        _ => None,
    }
}

/// Downloads CodeLLDB with the system's `curl` and unpacks it with `tar`
/// (Rust Code itself makes no network connections). Only call this after
/// the user agreed.
pub fn install_codelldb() -> Result<Adapter, String> {
    let url = package_url().ok_or("CodeLLDB has no package for this platform")?;
    let dir = install_dir().ok_or("no config folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let package = dir.join("codelldb.vsix");
    let system = |name: &str| {
        let sys = std::env::var_os("SystemRoot").map(|r| Path::new(&r).join("System32").join(exe(name)));
        sys.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
    };
    let run = |program: PathBuf, args: Vec<String>| -> Result<(), String> {
        let out = quiet_command(&program)
            .args(&args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("{}: {e}", program.display()))?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim().to_owned())
        }
    };
    let pkg = package.to_string_lossy().into_owned();
    run(
        system("curl"),
        vec![
            "-L".into(),
            "--fail".into(),
            "--silent".into(),
            "--show-error".into(),
            "-o".into(),
            pkg.clone(),
            url.into(),
        ],
    )?;
    let _ = std::fs::remove_dir_all(dir.join("extension"));
    run(system("tar"), vec!["-xf".into(), pkg, "-C".into(), dir.to_string_lossy().into_owned(), "extension".into()])?;
    let _ = std::fs::remove_file(&package);
    adapter_in(&dir.join("extension")).ok_or_else(|| "the package didn't contain the CodeLLDB adapter".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_messages_and_builds_launch_args() {
        let r = classify(json!({"type": "response", "request_seq": 2, "success": true, "body": {"x": 1}})).unwrap();
        assert!(matches!(r, DapEvent::Response { request_seq: 2, success: true, .. }));
        let e = classify(json!({"type": "event", "event": "stopped", "body": {"threadId": 1}})).unwrap();
        assert!(matches!(e, DapEvent::Event { event, .. } if event == "stopped"));
        let args = launch_args(Path::new("/p/app"), &["--x".into()], Path::new("/p"), false);
        assert_eq!(args["sourceLanguages"][0], "rust");
        assert!(args["initCommands"][0].as_str().unwrap().contains("step-avoid-regexp"));
        assert_eq!(args["terminal"], "integrated");
        assert_eq!(args["_adapterSettings"]["consoleMode"], "evaluate");
        let std_ok = launch_args(Path::new("/p/app"), &[], Path::new("/p"), true);
        assert!(std_ok["initCommands"].as_array().unwrap().is_empty());
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;

    #[test]
    fn each_debugger_gets_its_own_launch() {
        let python = DebugKind::Python {
            python: PathBuf::from("/v/bin/python"),
            env: vec![("VIRTUAL_ENV".to_owned(), "/v".to_owned())],
        };
        let args = python.launch_args(Path::new("/p/a.py"), &[], Path::new("/p"), false);
        assert_eq!((args["type"].as_str(), args["python"].as_str()), (Some("python"), Some("/v/bin/python")));
        assert_eq!(args["env"]["VIRTUAL_ENV"], "/v");
        assert_eq!(args["justMyCode"], true);
        let adapter = python.adapter().unwrap();
        assert_eq!(adapter.args, ["-m", "debugpy.adapter"]);
        let data = DebugKind::Data { command: "no-such-debugger --dap".to_owned(), debug_type: "x".to_owned() };
        assert_eq!(data.launch_args(Path::new("a"), &[], Path::new("."), false)["type"], "x");
        assert_eq!(data.adapter(), None, "not on PATH");
        assert_eq!(data.name(), "no-such-debugger");
    }
}
