//! The language servers Coiman IDE knows: which languages each one serves,
//! how to find it, how to start it, and how it can be installed. Nothing
//! here downloads anything unless [`install`] is called, which the editor
//! only does after the user agreed.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use super::discovery::{self, quiet_command};
use super::registry;

/// A language server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ServerKind {
    RustAnalyzer,
    Python,
    TypeScript,
    Clangd,
    Jdtls,
    CSharp,
    Gopls,
    Godot,
    /// A language described as data ([`registry`]), by its place in the list.
    Custom(u16),
}

impl ServerKind {
    pub const ALL: [ServerKind; 8] = [
        ServerKind::RustAnalyzer,
        ServerKind::Python,
        ServerKind::TypeScript,
        ServerKind::Clangd,
        ServerKind::Jdtls,
        ServerKind::CSharp,
        ServerKind::Gopls,
        ServerKind::Godot,
    ];

    /// The server for an LSP language id.
    pub fn for_language(language_id: &str) -> Option<Self> {
        if let Some(index) = registry::index_of(language_id, true) {
            return Some(Self::Custom(index));
        }
        Some(match language_id {
            "rust" => Self::RustAnalyzer,
            "python" => Self::Python,
            "javascript" | "javascriptreact" | "typescript" | "typescriptreact" => Self::TypeScript,
            "c" | "cpp" => Self::Clangd,
            "java" => Self::Jdtls,
            "csharp" => Self::CSharp,
            "go" => Self::Gopls,
            "gdscript" => Self::Godot,
            // An entry may only be there for its profile, with no server to look for.
            _ => {
                let index = registry::index_of(language_id, false)?;
                return registry::get(index).and_then(|l| l.server).map(|_| Self::Custom(index));
            }
        })
    }

    /// Key in `[lsp.paths]` for a custom server command.
    pub fn id(self) -> &'static str {
        match self {
            Self::RustAnalyzer => "rust",
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::Clangd => "clangd",
            Self::Jdtls => "java",
            Self::CSharp => "csharp",
            Self::Gopls => "go",
            Self::Godot => "godot",
            Self::Custom(i) => registry::get(i).map_or("", |l| l.language_id),
        }
    }

    /// Shown in the status bar and messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::RustAnalyzer => "rust-analyzer",
            Self::Python => "Python language server",
            Self::TypeScript => "typescript-language-server",
            Self::Clangd => "clangd",
            Self::Jdtls => "Eclipse JDT Language Server",
            Self::CSharp => "C# language server",
            Self::Gopls => "gopls",
            Self::Godot => "Godot's language server",
            Self::Custom(i) => registry::get(i).and_then(|l| l.server).map_or("language server", server_name),
        }
    }

    /// Languages it serves, for the install prompt.
    pub fn languages(self) -> &'static str {
        match self {
            Self::RustAnalyzer => "Rust",
            Self::Python => "Python",
            Self::TypeScript => "JavaScript and TypeScript",
            Self::Clangd => "C and C++",
            Self::Jdtls => "Java",
            Self::CSharp => "C#",
            Self::Gopls => "Go",
            Self::Godot => "GDScript",
            Self::Custom(i) => registry::get(i).map_or("", |l| l.name),
        }
    }

    /// Files that mark a project root for this server (besides `.git`).
    fn root_markers(self) -> &'static [&'static str] {
        match self {
            Self::RustAnalyzer => &["Cargo.toml"],
            Self::Python => &["pyproject.toml", "setup.py", "setup.cfg", "requirements.txt", "pyrightconfig.json"],
            Self::TypeScript => &["tsconfig.json", "jsconfig.json", "package.json"],
            Self::Clangd => &["compile_commands.json", "compile_flags.txt", ".clangd", "CMakeLists.txt", "Makefile"],
            Self::Jdtls => &["pom.xml", "build.gradle", "build.gradle.kts", "settings.gradle", ".project"],
            Self::CSharp => &[".sln", ".csproj"],
            Self::Gopls => &["go.work", "go.mod"],
            Self::Godot => &["project.godot"],
            Self::Custom(i) => registry::get(i).map_or(&[], |l| l.root_markers),
        }
    }
}

/// The program a command line starts, for messages.
fn server_name(command: &'static str) -> &'static str {
    command.split_whitespace().next().unwrap_or(command)
}

/// How to start a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Connect to a server already running on this computer, at this
    /// `host:port`, instead of starting a program.
    pub tcp: Option<String>,
    /// Set for the server: the project's environment, say.
    pub env: Vec<(String, String)>,
}

impl Launch {
    fn new(program: impl Into<PathBuf>, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            tcp: None,
            env: Vec::new(),
        }
    }

    fn tcp(address: &str) -> Self {
        Self { program: PathBuf::new(), args: Vec::new(), tcp: Some(address.to_owned()), env: Vec::new() }
    }
}

/// Godot's editor serves GDScript on this port while it runs.
const GODOT_ADDRESS: &str = "127.0.0.1:6005";
/// How `[lsp.paths]` names a server to connect to.
const TCP_SCHEME: &str = "tcp://";

/// Whether `host:port` stays on this computer: the editor reaches nowhere else.
pub fn is_local_address(address: &str) -> bool {
    let Some((host, port)) = address.rsplit_once(':') else { return false };
    matches!(host, "127.0.0.1" | "localhost" | "[::1]") && port.parse::<u16>().is_ok()
}

/// A way to install a missing server, run only after the user agreed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installer {
    /// `rustup component add rust-analyzer`.
    Rustup(PathBuf),
    /// `npm install -g <packages>` (from the npm registry).
    Npm(PathBuf, Vec<&'static str>),
    /// `python -m pip install --user <package>` (from PyPI).
    Pip(PathBuf, &'static str),
    /// `go install <module>` (from the Go module proxy).
    Go(PathBuf, &'static str),
    /// `dotnet tool install --global <tool>` (from NuGet).
    Dotnet(PathBuf, &'static str),
    /// Download an archive with the system `curl` and unpack it with `tar`
    /// into Coiman IDE's `lsp` folder. `api` is a GitHub releases API URL to
    /// read the archive's address from, when it isn't fixed.
    /// `runtime` is a Java runtime archive to unpack into `runtime/` too.
    Download { url: &'static str, api: Option<(&'static str, &'static str)>, runtime: Option<&'static str> },
}

impl Installer {
    /// What it will do, for the consent prompt.
    pub fn describe(&self) -> String {
        match self {
            Installer::Rustup(_) => {
                "run `rustup component add rust-analyzer` (downloads from static.rust-lang.org)".to_owned()
            }
            Installer::Npm(_, p) => format!("run `npm install -g {}` (downloads from registry.npmjs.org)", p.join(" ")),
            Installer::Pip(_, p) => format!("run `python -m pip install --user {p}` (downloads from pypi.org)"),
            Installer::Go(_, m) => format!("run `go install {m}` (downloads from proxy.golang.org)"),
            Installer::Dotnet(_, t) => format!("run `dotnet tool install --global {t}` (downloads from nuget.org)"),
            Installer::Download { url, api, runtime } => {
                let what = match api {
                    None => format!("download {url}"),
                    Some((api, _)) => format!("download the latest release listed at {api}"),
                };
                match runtime {
                    Some(rt) => format!("{what} and a Java {JDTLS_JAVA} runtime (Eclipse Temurin, {rt}) with curl"),
                    None => format!("{what} with curl"),
                }
            }
        }
    }
}

/// What looking for a server found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Server(Launch),
    /// Not installed; `installer` is set when it can be installed from here.
    /// `note` says what is missing otherwise (e.g. a Java runtime).
    Missing {
        installer: Option<Installer>,
        note: String,
    },
}

/// rust-analyzer's settings: its check command, memory preset and build folder.
pub fn rust_analyzer_options(check: &str, memory: &str, separate_target: bool) -> serde_json::Value {
    let mut options = serde_json::json!({ "check": { "command": check } });
    if separate_target {
        options["cargo"]["targetDir"] = true.into();
    }
    if matches!(memory, "low" | "minimal") {
        options["lru"]["capacity"] = 32.into();
        options["cachePriming"]["enable"] = false.into();
    }
    if memory == "minimal" {
        options["procMacro"]["enable"] = false.into();
        options["cargo"]["buildScripts"]["enable"] = false.into();
    }
    options
}

/// `initializationOptions` a server needs (besides rust-analyzer's, which
/// the editor builds from its settings).
pub fn init_options(kind: ServerKind, launch: &Launch) -> serde_json::Value {
    match kind {
        // Lets go to definition reach classes inside jars.
        ServerKind::Jdtls => {
            serde_json::json!({ "extendedClientCapabilities": { "classFileContentsSupport": true } })
        }
        // A global typescript-language-server needs to be told where the
        // global TypeScript is; a project's own TypeScript still wins.
        ServerKind::TypeScript => {
            // Beside a global install's command, or above a project's `.bin`.
            let folder = launch.program.parent();
            let global = folder.map(|d| d.join("node_modules"));
            let local = folder.and_then(Path::parent).map(Path::to_path_buf);
            let found = [global, local].into_iter().flatten().map(|d| d.join("typescript").join("lib"));
            match found.into_iter().find(|l| l.join("tsserver.js").is_file()) {
                Some(lib) => serde_json::json!({ "tsserver": { "fallbackPath": lib } }),
                None => serde_json::Value::Null,
            }
        }
        // Says what it's doing with each file, for the C/C++ window.
        ServerKind::Clangd => serde_json::json!({ "clangdFileStatus": true }),
        // These only offer formatting when asked to.
        ServerKind::Custom(i) if registry::get(i).and_then(|l| l.server).is_some_and(|s| s.starts_with("vscode-")) => {
            serde_json::json!({ "provideFormatter": true, "embeddedLanguages": { "css": true, "javascript": true } })
        }
        _ => serde_json::Value::Null,
    }
}

/// `<config>/rust-code/lsp/<id>`: servers Coiman IDE downloaded itself.
pub fn install_dir(kind: ServerKind) -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("lsp").join(kind.id()))
}

/// An executable on `PATH` (trying Windows' `.exe`, `.cmd`, `.bat`).
pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    let path = std::env::var_os("PATH")?;
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    // Global tool folders that may not be on PATH yet right after an install.
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(Path::new(&appdata).join("npm"));
    }
    dirs.extend(crate::build_tools::dirs());
    if let Some(home) = home() {
        dirs.push(home.join(".dotnet").join("tools"));
        dirs.push(home.join("go").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
    }
    dirs.iter().flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}")))).find(|p| p.is_file())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}

/// Whether `program args…` runs successfully.
fn runs(program: &Path, args: &[&str]) -> bool {
    quiet_command(program)
        .args(args)
        // Not the app's folder: `python -c "import pylsp"` would import from it.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn python() -> Option<PathBuf> {
    // The Microsoft Store alias exists even when Python isn't installed, so run it.
    ["python", "python3", "py"].into_iter().filter_map(which).find(|p| runs(p, &["--version"]))
}

/// Java's major version from `java -version` (printed on stderr).
fn java_major(java: &Path) -> Option<u32> {
    let out = quiet_command(java).arg("-version").stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let quoted = text.split('"').nth(1)?;
    let mut parts = quoted.split('.');
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

/// Java version the Java language server (latest jdtls) needs.
pub const JDTLS_JAVA: u32 = 25;

/// A Java runtime new enough for jdtls: the one Coiman IDE downloaded, then
/// `JAVA_HOME`, then `PATH`.
fn java() -> Option<PathBuf> {
    let own = install_dir(ServerKind::Jdtls).and_then(|d| find_file(&d.join("runtime"), "java"));
    let from_home = std::env::var_os("JAVA_HOME")
        .map(|h| Path::new(&h).join("bin").join(format!("java{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.is_file());
    [own, from_home, which("java")].into_iter().flatten().find(|j| java_major(j).is_some_and(|v| v >= JDTLS_JAVA))
}

/// Eclipse Temurin's latest Java runtime for this platform.
fn temurin_runtime() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => {
            "https://api.adoptium.net/v3/binary/latest/25/ga/windows/x64/jre/hotspot/normal/eclipse"
        }
        ("linux", "x86_64") => "https://api.adoptium.net/v3/binary/latest/25/ga/linux/x64/jre/hotspot/normal/eclipse",
        ("linux", "aarch64") => {
            "https://api.adoptium.net/v3/binary/latest/25/ga/linux/aarch64/jre/hotspot/normal/eclipse"
        }
        ("macos", "aarch64") => {
            "https://api.adoptium.net/v3/binary/latest/25/ga/mac/aarch64/jre/hotspot/normal/eclipse"
        }
        ("macos", "x86_64") => "https://api.adoptium.net/v3/binary/latest/25/ga/mac/x64/jre/hotspot/normal/eclipse",
        _ => return None,
    })
}

/// A server named by a command line, or by an address.
fn find_configured(configured: &str) -> Found {
    if let Some(address) = configured.strip_prefix(TCP_SCHEME) {
        return Found::Server(Launch::tcp(address));
    }
    let mut words = configured.split_whitespace();
    let program = PathBuf::from(words.next().unwrap_or_default());
    let program = if program.is_file() { program } else { which(&program.to_string_lossy()).unwrap_or(program) };
    Found::Server(Launch { program, args: words.map(str::to_owned).collect(), tcp: None, env: Vec::new() })
}

/// Looks for a server: the configured command (`[lsp.paths]`, a program
/// optionally followed by arguments), then the usual places.
pub fn find(kind: ServerKind, configured: &str) -> Found {
    let configured = configured.trim();
    if !configured.is_empty() {
        return find_configured(configured);
    }
    match kind {
        ServerKind::Godot => Found::Server(Launch::tcp(GODOT_ADDRESS)),
        ServerKind::Custom(i) => {
            let language = registry::get(i);
            let note = language.and_then(|l| l.hint).unwrap_or("Its entry names no way to get it.").to_owned();
            match language.and_then(|l| l.server) {
                Some(command) if command.starts_with(TCP_SCHEME) => find_configured(command),
                Some(command) if which(server_name(command)).is_some() || Path::new(server_name(command)).is_file() => {
                    find_configured(command)
                }
                _ => Found::Missing { installer: None, note },
            }
        }
        ServerKind::RustAnalyzer => match discovery::find_rust_analyzer("") {
            discovery::Found::Server(p) => Found::Server(Launch::new(p, &[])),
            discovery::Found::Missing { rustup } => Found::Missing {
                installer: rustup.map(Installer::Rustup),
                note: "Install it with `rustup component add rust-analyzer`.".to_owned(),
            },
        },
        ServerKind::Python => {
            if let Some(p) = which("pyright-langserver") {
                return Found::Server(Launch::new(p, &["--stdio"]));
            }
            if let Some(p) = which("basedpyright-langserver") {
                return Found::Server(Launch::new(p, &["--stdio"]));
            }
            if let Some(p) = which("pylsp") {
                return Found::Server(Launch::new(p, &[]));
            }
            let py = python();
            if let Some(py) = &py
                && runs(py, &["-c", "import pylsp"])
            {
                return Found::Server(Launch::new(py, &["-m", "pylsp"]));
            }
            let installer = match (which("npm"), py) {
                (Some(npm), _) => Some(Installer::Npm(npm, vec!["pyright"])),
                (None, Some(py)) => Some(Installer::Pip(py, "python-lsp-server")),
                _ => None,
            };
            Found::Missing { installer, note: "Install Node.js (for pyright) or Python (for pylsp).".to_owned() }
        }
        ServerKind::TypeScript => match which("typescript-language-server") {
            Some(p) => Found::Server(Launch::new(p, &["--stdio"])),
            None => Found::Missing {
                installer: which("npm")
                    .map(|npm| Installer::Npm(npm, vec!["typescript-language-server", "typescript@6"])),
                note: "Install Node.js, then `npm install -g typescript-language-server typescript@6`.".to_owned(),
            },
        },
        ServerKind::Clangd => {
            let own = install_dir(kind).and_then(|d| find_file(&d, "clangd"));
            let llvm = std::env::var_os("ProgramFiles")
                .map(|p| Path::new(&p).join("LLVM").join("bin").join("clangd.exe"))
                .filter(|p| p.is_file());
            match which("clangd").or(llvm).or(own) {
                Some(p) => Found::Server(Launch::new(p, &["--background-index"])),
                None => Found::Missing {
                    installer: clangd_asset().map(|asset| Installer::Download {
                        url: "https://github.com/clangd/clangd/releases",
                        api: Some(("https://api.github.com/repos/clangd/clangd/releases/latest", asset)),
                        runtime: None,
                    }),
                    note: "Install LLVM (it includes clangd).".to_owned(),
                },
            }
        }
        ServerKind::Jdtls => {
            const URL: &str = "https://download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz";
            let java = java();
            if let Some(launch) = java.as_ref().and_then(|j| install_dir(kind).and_then(|d| jdtls_launch(j, &d))) {
                return Found::Server(launch);
            }
            // Without a new enough Java, the install brings its own runtime.
            let runtime = if java.is_some() { None } else { temurin_runtime() };
            let installer =
                (java.is_some() || runtime.is_some()).then_some(Installer::Download { url: URL, api: None, runtime });
            Found::Missing {
                installer,
                note: format!("The Java language server needs Java {JDTLS_JAVA} or later (set JAVA_HOME)."),
            }
        }
        ServerKind::CSharp => {
            if let Some(p) = which("csharp-ls") {
                return Found::Server(Launch::new(p, &[]));
            }
            if let Some(p) = install_dir(kind).and_then(|d| find_file(&d, "OmniSharp")) {
                return Found::Server(Launch::new(p, &["-lsp"]));
            }
            let dotnet = which("dotnet").filter(|d| {
                quiet_command(d)
                    .arg("--list-sdks")
                    .stdin(Stdio::null())
                    .output()
                    .is_ok_and(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty())
            });
            Found::Missing {
                installer: dotnet.map(|d| Installer::Dotnet(d, "csharp-ls")),
                note: "Install the .NET SDK, then `dotnet tool install --global csharp-ls`.".to_owned(),
            }
        }
        ServerKind::Gopls => match which("gopls") {
            Some(p) => Found::Server(Launch::new(p, &[])),
            None => Found::Missing {
                installer: which("go").map(|go| Installer::Go(go, "golang.org/x/tools/gopls@latest")),
                note: "Install Go, then `go install golang.org/x/tools/gopls@latest`.".to_owned(),
            },
        },
    }
}

fn clangd_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("clangd-windows-"),
        ("linux", "x86_64") => Some("clangd-linux-"),
        ("macos", _) => Some("clangd-mac-"),
        _ => None,
    }
}

/// The first file named `name` (plus the platform's exe suffix) under `dir`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let want = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).ok()?.filter_map(Result::ok) {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(&want)) {
                return Some(p);
            }
        }
    }
    None
}

/// `java … -jar plugins/org.eclipse.equinox.launcher_*.jar -configuration config_<os> -data <dir>`.
fn jdtls_launch(java: &Path, dir: &Path) -> Option<Launch> {
    let launcher =
        std::fs::read_dir(dir.join("plugins")).ok()?.filter_map(Result::ok).map(|e| e.path()).find(|p| {
            p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("org.eclipse.equinox.launcher_"))
        })?;
    let config = match std::env::consts::OS {
        "windows" => "config_win",
        "macos" => "config_mac",
        _ => "config_linux",
    };
    // Its own index and settings, kept with the server.
    let data = dir.join("workspace-data");
    let args = vec![
        "-Declipse.application=org.eclipse.jdt.ls.core.id1".to_owned(),
        "-Dosgi.bundles.defaultStartLevel=4".to_owned(),
        "-Declipse.product=org.eclipse.jdt.ls.core.product".to_owned(),
        "-Xmx1G".to_owned(),
        "--add-modules=ALL-SYSTEM".to_owned(),
        "--add-opens".to_owned(),
        "java.base/java.util=ALL-UNNAMED".to_owned(),
        "--add-opens".to_owned(),
        "java.base/java.lang=ALL-UNNAMED".to_owned(),
        "-jar".to_owned(),
        launcher.to_string_lossy().into_owned(),
        "-configuration".to_owned(),
        dir.join(config).to_string_lossy().into_owned(),
        "-data".to_owned(),
        data.to_string_lossy().into_owned(),
    ];
    Some(Launch { program: java.to_path_buf(), args, tcp: None, env: Vec::new() })
}

fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let out = quiet_command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let out_text = String::from_utf8_lossy(&out.stdout);
        let last = err.lines().chain(out_text.lines()).rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
        Err(last.trim().to_owned())
    }
}

fn system(name: &str) -> PathBuf {
    let sys = std::env::var_os("SystemRoot")
        .map(|r| Path::new(&r).join("System32").join(format!("{name}{}", std::env::consts::EXE_SUFFIX)));
    sys.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from(name))
}

/// A program from the system's own folder, for downloads and unpacking.
pub fn system_tool(name: &str) -> PathBuf {
    system(name)
}

/// Installs a server. Only call this after the user agreed.
pub fn install(kind: ServerKind, installer: &Installer) -> Result<(), String> {
    match installer {
        Installer::Rustup(rustup) => discovery::install_rust_analyzer(rustup),
        Installer::Npm(npm, packages) => {
            let mut args = vec!["install", "-g"];
            args.extend(packages.iter().copied());
            run(npm, &args).map(drop)
        }
        Installer::Pip(py, package) => run(py, &["-m", "pip", "install", "--user", package]).map(drop),
        Installer::Go(go, module) => run(go, &["install", module]).map(drop),
        Installer::Dotnet(dotnet, tool) => run(dotnet, &["tool", "install", "--global", tool]).map(drop),
        Installer::Download { url, api, runtime } => {
            let dir = install_dir(kind).ok_or("no config folder")?;
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let curl = system("curl");
            let url = match api {
                None => (*url).to_owned(),
                Some((api, prefix)) => {
                    let json = run(&curl, &["-L", "--fail", "--silent", "--show-error", api])?;
                    let release: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
                    release["assets"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|a| a["browser_download_url"].as_str())
                        .find(|u| u.rsplit('/').next().is_some_and(|n| n.starts_with(prefix) && n.ends_with(".zip")))
                        .ok_or("no package for this platform in the latest release")?
                        .to_owned()
                }
            };
            let name = if url.ends_with(".zip") { "server.zip" } else { "server.tar.gz" };
            let archive = dir.join(name);
            let archive_s = archive.to_string_lossy().into_owned();
            run(&curl, &["-L", "--fail", "--silent", "--show-error", "-o", &archive_s, &url])?;
            run(&system("tar"), &["-xf", &archive_s, "-C", &dir.to_string_lossy()])?;
            let _ = std::fs::remove_file(&archive);
            if let Some(rt) = runtime {
                let rt_dir = dir.join("runtime");
                std::fs::create_dir_all(&rt_dir).map_err(|e| e.to_string())?;
                let name = if cfg!(windows) { "runtime.zip" } else { "runtime.tar.gz" };
                let rt_archive = dir.join(name).to_string_lossy().into_owned();
                run(&curl, &["-L", "--fail", "--silent", "--show-error", "-o", &rt_archive, rt])?;
                run(&system("tar"), &["-xf", &rt_archive, "-C", &rt_dir.to_string_lossy()])?;
                let _ = std::fs::remove_file(&rt_archive);
            }
            Ok(())
        }
    }
}

/// The folder a server should start in for `path`: the open folder if it
/// contains the file and a root marker, else the outermost (Rust: workspace)
/// or nearest folder with a marker above the file, else the open folder if
/// it contains the file, else the file's folder.
pub fn root_for(kind: ServerKind, path: &Path, open_folder: Option<&Path>) -> PathBuf {
    let markers = kind.root_markers();
    // Markers like ".csproj" match any file with that extension.
    let has_marker = |dir: &Path| {
        markers.iter().any(|m| match m.strip_prefix('.').filter(|ext| matches!(*ext, "sln" | "csproj")) {
            Some(ext) => std::fs::read_dir(dir).is_ok_and(|rd| {
                rd.filter_map(Result::ok).any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
            }),
            None => dir.join(m).exists(),
        })
    };
    let inside = open_folder.filter(|r| path.starts_with(r));
    if let Some(root) = inside.filter(|r| has_marker(r)) {
        return root.to_path_buf();
    }
    let mut nearest = None;
    for dir in path.ancestors().skip(1) {
        if has_marker(dir) {
            nearest.get_or_insert_with(|| dir.to_path_buf());
        }
        if inside.is_some_and(|r| dir == r) {
            break;
        }
    }
    nearest.or_else(|| inside.map(Path::to_path_buf)).unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_analyzer_options_follow_the_settings() {
        use serde_json::json;
        assert_eq!(rust_analyzer_options("clippy", "default", false), json!({ "check": { "command": "clippy" } }));
        assert_eq!(
            rust_analyzer_options("check", "low", true),
            json!({
                "check": { "command": "check" },
                "cargo": { "targetDir": true },
                "lru": { "capacity": 32 },
                "cachePriming": { "enable": false },
            })
        );
        assert_eq!(
            rust_analyzer_options("check", "minimal", true),
            json!({
                "check": { "command": "check" },
                "cargo": { "targetDir": true, "buildScripts": { "enable": false } },
                "lru": { "capacity": 32 },
                "cachePriming": { "enable": false },
                "procMacro": { "enable": false },
            })
        );
    }

    #[test]
    fn languages_map_to_servers_and_roots_follow_markers() {
        assert_eq!(ServerKind::for_language("typescriptreact"), Some(ServerKind::TypeScript));
        assert_eq!(ServerKind::for_language("cpp"), Some(ServerKind::Clangd));
        assert_eq!(ServerKind::for_language("xml"), None);
        assert!(ServerKind::for_language("html").is_some());

        let tmp = std::env::temp_dir().join(format!("rust-code-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("web/src")).unwrap();
        std::fs::create_dir_all(tmp.join("cs/App")).unwrap();
        std::fs::write(tmp.join("web/package.json"), "{}").unwrap();
        std::fs::write(tmp.join("cs/App/App.csproj"), "").unwrap();
        let file = tmp.join("web/src/a.ts");
        assert_eq!(root_for(ServerKind::TypeScript, &file, Some(&tmp)), tmp.join("web"));
        // No marker: the open folder.
        assert_eq!(root_for(ServerKind::Python, &tmp.join("web/src/a.py"), Some(&tmp)), tmp);
        // Marker files found by extension (C# projects).
        assert_eq!(root_for(ServerKind::CSharp, &tmp.join("cs/App/Program.cs"), Some(&tmp)), tmp.join("cs/App"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn data_languages_get_servers() {
        use crate::lsp::language_id;
        assert_eq!(language_id("gleam"), Some("gleam"));
        let kind = ServerKind::for_language("gleam").unwrap();
        assert!(matches!(kind, ServerKind::Custom(_)));
        assert_eq!(kind.id(), "gleam");
        assert_eq!(kind.name(), "gleam");
        assert_eq!(kind.languages(), "Gleam");
        // Built-in languages stay built in when nothing overrides them.
        assert_eq!(ServerKind::for_language("rust"), Some(ServerKind::RustAnalyzer));
        assert_eq!(language_id("kt"), Some("kotlin"));
        // A server that isn't installed says how to get it.
        if which("gleam").is_none() {
            let Found::Missing { note, installer } = find(kind, "") else { panic!() };
            assert!(note.contains("gleam.run") && installer.is_none());
        }
        assert_eq!(find(kind, "tcp://127.0.0.1:9"), Found::Server(Launch::tcp("127.0.0.1:9")), "the user's path wins");
    }

    #[test]
    fn tcp_servers_stay_on_this_computer() {
        let found = find(ServerKind::Gopls, "tcp://127.0.0.1:7000");
        assert_eq!(found, Found::Server(Launch::tcp("127.0.0.1:7000")));
        assert_eq!(find(ServerKind::Godot, ""), Found::Server(Launch::tcp("127.0.0.1:6005")));
        assert_eq!(ServerKind::for_language("gdscript"), Some(ServerKind::Godot));
        assert!(is_local_address("127.0.0.1:6005") && is_local_address("localhost:1") && is_local_address("[::1]:9"));
        assert!(!is_local_address("example.com:6005") && !is_local_address("192.168.1.2:6005"));
        assert!(!is_local_address("127.0.0.1") && !is_local_address("127.0.0.1:port"));
    }

    #[test]
    fn configured_command_wins() {
        let found = find(ServerKind::Gopls, "C:/tools/gopls.exe -remote=auto");
        assert_eq!(found, Found::Server(Launch::new("C:/tools/gopls.exe", &["-remote=auto"])));
    }
}
