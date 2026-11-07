//! Debug Adapter Protocol client for CodeLLDB: the adapter process spoken to
//! over stdio (same framing as LSP), plus finding CodeLLDB and installing it.
//! Nothing here downloads unless [`install_codelldb`] is called, which the
//! editor only does after the user agreed.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use serde_json::{Value, json};

use crate::lsp::discovery::{kill_tree, quiet_command};
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
    /// For an adapter reached by a socket: where, for its further sessions.
    address: Option<String>,
}

/// How long an adapter gets to say which port it listens on.
const LISTEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The address in an adapter's "listening at 127.0.0.1:1234". Only this computer's counts.
fn listening_address(line: &str) -> Option<String> {
    let at = line.find("127.0.0.1:")?;
    let rest = &line[at + "127.0.0.1:".len()..];
    let port: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!port.is_empty()).then(|| format!("127.0.0.1:{port}"))
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
        Ok(Self { child: Some(child), out, events, next_seq: 1, address: None })
    }

    /// Starts an adapter that listens on a port of this computer, and connects to it.
    pub fn start_listening(
        program: &Path,
        args: &[String],
        cwd: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let mut child = quiet_command(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let (found, address) = mpsc::channel();
        // It goes on reading, so the adapter never blocks on a full pipe.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(at) = listening_address(&line) {
                    let _ = found.send(at);
                }
            }
        });
        let connected = address
            .recv_timeout(LISTEN_TIMEOUT)
            .map_err(|_| std::io::Error::other("It didn't say where it's listening"))
            .and_then(|at| Self::connect(&at, wake));
        match connected {
            Ok(mut client) => {
                client.child = Some(child);
                Ok(client)
            }
            Err(why) => {
                kill_tree(&mut child);
                Err(why)
            }
        }
    }

    /// Opens another session with an adapter already listening.
    pub fn connect(address: &str, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let stream = TcpStream::connect(address)?;
        let reader = stream.try_clone()?;
        let mut writer = stream;
        let (out, out_rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            for bytes in out_rx {
                if writer.write_all(&bytes).and_then(|()| writer.flush()).is_err() {
                    break;
                }
            }
            // What was queued, such as a `disconnect`, has gone: this ends the session.
            let _ = writer.shutdown(std::net::Shutdown::Both);
        });
        let (tx, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
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
        Ok(Self { child: None, out, events, next_seq: 1, address: Some(address.to_owned()) })
    }

    /// Where a further session with this adapter can connect.
    pub fn address(&self) -> Option<&str> {
        self.address.as_deref()
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
        // One that listens doesn't end with its connection, and a thread wouldn't outlive
        // the editor closing. What it's debugging goes with it.
        if self.address.is_some() {
            kill_tree(&mut child);
            return;
        }
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            kill_tree(&mut child);
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
        "clientID": "coiman-ide",
        "clientName": "Coiman IDE",
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
        // js-debug runs each program in a session of its own, on another connection.
        "supportsStartDebuggingRequest": true,
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
    /// Delve, for Go.
    Go,
    /// js-debug, for JavaScript on Node.
    Js,
    /// gdb's own adapter (`gdb -i dap`, gdb 14 and later), for C and C++ built with
    /// MinGW or GCC.
    Gdb,
}

/// Where Delve is: on PATH, or where `go install` puts it.
fn find_dlv() -> Option<PathBuf> {
    crate::lsp::servers::which("dlv").or_else(|| {
        let gopath = std::env::var_os("GOPATH").map(PathBuf::from);
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
        let places = gopath.into_iter().chain(home.map(|h| Path::new(&h).join("go")));
        places.map(|p| p.join("bin").join(exe("dlv"))).find(|p| p.is_file())
    })
}

/// What Delve should build for this file: the package, when there's a module around it.
fn go_target(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(file);
    let in_module = dir.ancestors().any(|d| d.join("go.mod").is_file());
    if in_module || is_go_test(file) { dir.to_path_buf() } else { file.to_path_buf() }
}

/// Where Delve's build goes. One killed along with Delve stays behind, so older ones go
/// here; one still running can't be deleted, so another editor's is safe.
fn go_build_path() -> PathBuf {
    let dir = std::env::temp_dir().join("rust-code-run");
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with("dlv-") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    dir.join(format!("dlv-{}", std::process::id()))
}

fn is_go_test(file: &Path) -> bool {
    file.file_name().is_some_and(|n| n.to_string_lossy().ends_with("_test.go"))
}

impl DebugKind {
    pub fn name(&self) -> String {
        match self {
            Self::Lldb => "CodeLLDB".to_owned(),
            Self::Python { .. } => "debugpy".to_owned(),
            Self::Data { command, .. } => command.split_whitespace().next().unwrap_or(command).to_owned(),
            Self::Go => "Delve".to_owned(),
            Self::Js => "js-debug".to_owned(),
            Self::Gdb => "gdb".to_owned(),
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
            Self::Go => {
                let args = ["dap", "--listen=127.0.0.1:0"].map(str::to_owned).to_vec();
                Some(Adapter { program: find_dlv()?, args })
            }
            Self::Gdb => {
                let mut args = vec!["--interpreter=dap".to_owned()];
                // The program's console is its own window: gdb's streams carry the
                // protocol, and the program can read what's typed.
                if cfg!(windows) {
                    args.extend(["-iex".to_owned(), "set new-console on".to_owned()]);
                }
                Some(Adapter { program: crate::c_build::find_gdb()?, args })
            }
            Self::Js => {
                let server = crate::build_tools::js_debug_server()?.to_string_lossy().into_owned();
                let program = crate::lsp::servers::which("node")?;
                Some(Adapter { program, args: vec![server, "0".to_owned(), "127.0.0.1".to_owned()] })
            }
        }
    }

    /// Whether it's reached by a port on this computer, rather than its standard streams.
    pub fn listens(&self) -> bool {
        matches!(self, Self::Go | Self::Js)
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
            Self::Go => json!({
                "type": "go",
                "request": "launch",
                "name": name,
                "mode": if is_go_test(program) { "test" } else { "debug" },
                "program": go_target(program),
                "args": args,
                "cwd": cwd,
                "stopOnEntry": false,
                // What the program prints comes as output events, not to Delve's own stream.
                "outputMode": "remote",
                // Its build goes outside the project.
                "output": go_build_path(),
            }),
            Self::Gdb => json!({
                "type": "gdb",
                "request": "launch",
                "name": name,
                "program": program,
                "args": args,
                "cwd": cwd,
                "stopAtBeginningOfMainSubprogram": false,
            }),
            Self::Js => json!({
                "type": "pwa-node",
                "request": "launch",
                "name": name,
                "program": program,
                "args": args,
                "cwd": cwd,
                "console": "internalConsole",
                "stopOnEntry": false,
                "skipFiles": if step_into_std { json!([]) } else { json!(["<node_internals>/**"]) },
            }),
        }
    }
}

#[cfg(test)]
mod socket_tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn reads_where_an_adapter_listens() {
        let found = |line| listening_address(line);
        assert_eq!(found("DAP server listening at: 127.0.0.1:53124").as_deref(), Some("127.0.0.1:53124"));
        assert_eq!(found("Debug server listening at 127.0.0.1:9229").as_deref(), Some("127.0.0.1:9229"));
        assert_eq!(found("listening at 192.0.2.1:9229"), None);
        assert_eq!(found("127.0.0.1: nothing"), None);
    }

    #[test]
    fn talks_over_a_socket_and_ends_with_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            let msg = read_message(&mut reader).unwrap().unwrap();
            let answer = json!({ "type": "response", "request_seq": msg["seq"], "success": true, "body": { "ok": 1 } });
            writer.write_all(&encode(&answer)).unwrap();
            // At once, whatever else is running: a process started meanwhile can hold a copy.
            writer.shutdown(std::net::Shutdown::Both).unwrap();
        });
        let mut client = DapClient::connect(&address, || {}).unwrap();
        assert_eq!(client.address(), Some(address.as_str()));
        let seq = client.request("initialize", json!({}));
        let mut seen = Vec::new();
        for _ in 0..1500 {
            seen.extend(client.poll());
            if seen.iter().any(|e| matches!(e, DapEvent::Exited)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            matches!(seen.first(), Some(DapEvent::Response { request_seq, success: true, .. }) if *request_seq == seq),
            "{seen:?}"
        );
        assert!(matches!(seen.last(), Some(DapEvent::Exited)), "{seen:?}");
    }

    #[test]
    fn launches_go_and_js_as_they_expect() {
        let (file, cwd) = (Path::new("C:/none/main.go"), Path::new("C:/none"));
        let go = DebugKind::Go.launch_args(file, &[], cwd, false);
        assert_eq!(go["type"], "go");
        assert_eq!(go["mode"], "debug");
        assert_eq!(go["program"], json!(file));
        let test = DebugKind::Go.launch_args(Path::new("C:/none/a_test.go"), &[], cwd, false);
        assert_eq!(test["mode"], "test");
        assert_eq!(test["program"], json!(cwd));
        let js = DebugKind::Js.launch_args(Path::new("C:/none/a.js"), &[], cwd, false);
        assert_eq!(js["type"], "pwa-node");
        assert_eq!(js["skipFiles"][0], "<node_internals>/**");
        assert!(DebugKind::Go.listens() && DebugKind::Js.listens() && !DebugKind::Lldb.listens());
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

/// Where Coiman IDE installs CodeLLDB: `<config>/rust-code/codelldb`.
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

/// CodeLLDB from the setting, Coiman IDE's own install, or VS Code's extensions.
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
/// (Coiman IDE itself makes no network connections). Only call this after
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
