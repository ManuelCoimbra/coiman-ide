# Privacy

Rust Code is built to be trusted with your code. These are commitments, and
`scripts/check.ps1` enforces the network one on every run.

## What Rust Code never does

- **No telemetry.** No usage statistics, crash reports or analytics are
  collected or sent, and there is no setting to turn them on.
- **No network access.** The editor makes no network connections: no update
  checks, no downloads, no "what's new" feeds. Debugging Go or JavaScript starts Delve or js-debug listening on a port of this computer only, and connects to it. js-debug 1.140.0 is downloaded from github.com/microsoft only if you agree, and checked against its hash.

The live session panel connects only when you press Connect, to an address on this computer. For a BEAM node, Rust Code's helper joins it as a hidden node on this computer, using Erlang's own distribution. For a Python kernel or a BEAM node, a small helper script is written to the system's temporary folder and run from there. The one other socket it opens is to a
  language server already running on this computer (`tcp://` in `[lsp.paths]`,
  or Godot's editor for `.gd` files), and it refuses any other address. The live
  preview listens on `127.0.0.1` only, while it runs, and answers only requests
  addressed to that name; it serves the open folder but not hidden files. The build fails if a networking
  crate (HTTP clients, TLS, async network runtimes) enters the dependency tree.
- **No account.** Nothing requires signing in.
- **No AI by default.** There are no AI features today. If optional AI is added
  later, it will be off until you turn it on, use the provider/key you choose,
  and never run on its own.

Anything that needs the network asks first and says what will be downloaded
from where:

- **Installing language servers.** If the server for a file is missing, Rust
  Code offers to install it and says exactly how:
  - rust-analyzer: `rustup component add rust-analyzer` (static.rust-lang.org);
  - pyright and typescript-language-server: `npm install -g …` (registry.npmjs.org);
  - pylsp: `python -m pip install --user python-lsp-server` (pypi.org);
  - csharp-ls: `dotnet tool install --global csharp-ls` (nuget.org);
  - gopls: `go install golang.org/x/tools/gopls@latest` (proxy.golang.org);
  - clangd and the Java language server: downloaded with your system's `curl`
    from github.com/clangd and download.eclipse.org. When no Java 25 is
    installed, a Java runtime from Eclipse Temurin (api.adoptium.net) comes too.

  Nothing runs until you click **Install**. npm, pip, dotnet and go tools install
  into those tools' own global folders.
- **Git: Push, Pull and Fetch.** They run `git` against your repository's remote,
  and only when you click them or run the command. Nothing fetches on a timer.
- **Language servers themselves** are separate programs, as in other editors.
  Some fetch what your project needs. rust-analyzer runs `cargo metadata` and
  `cargo check`, so Cargo downloads dependencies from crates.io. jdtls may
  resolve Maven or Gradle dependencies. Set `lsp.enabled = false` to never
  start them.
- **Downloading the debugger.** If CodeLLDB isn't found, the Debug view offers
  to download it from github.com/vadimcn/codelldb (the latest release
  package). The download runs your system's `curl`, and the system `tar`
  unpacks it. Rust Code itself still opens no connection. Nothing is
  downloaded until you click **Download**.
- **Plugin registries.** The Extensions view reads the registry set in
  `plugins.registry` only when you click refresh, **Install…** or **Update…**.
  There are no automatic update checks. A URL is fetched with your system's
  `curl`; a folder is read directly. Nothing is installed until you approve the
  plugin's permissions.
  - Build tools: when Run finds a CMake project on Windows with no build tool or compiler, it
  offers Ninja from github.com/ninja-build, or GCC from github.com/brechtsanders. Fixed versions,
  fetched with your system's `curl` and `tar`, and checked against a known hash. Nothing is
  downloaded until you click **Download**.
- **Plugins themselves** have no network access. They run sandboxed with only
  the editor's plugin API, and they reach your files and editor only through
  the permissions you granted.
- **Programs started by plugins are not sandboxed.** A plugin's `plugin.toml` can
  name programs it wants to start, such as `dotnet`. Each one starts unapproved
  and runs only if you tick it, by that exact name found on PATH, in a folder you
  trust. A program you approve runs with all your rights: it can read and change
  your files and use the network, so this promise stops at that program. Every
  start is written to the plugin's log, and you can withdraw approval at any time.

## What Rust Code reads and writes on your machine

| Location | Read | Written | Why |
| --- | --- | --- | --- |
| Files and folders you open | yes | when you save | editing |
| `<config>\logs\rust-code.log`, `crash.log` | when you open them | as the app runs, and when it crashes; they go nowhere | what the app did and why it stopped: commands, file paths, clicks, what's on screen, run titles. Never text you type, or run command lines |
| `~\.vscode\extensions`, a `.vsix` or folder you choose | when you use **Extensions: Import from VS Code** | copies to `<config>\themes`, `snippets.toml`, `languages.toml` | themes, snippets and language names only. An extension's code is never run, and nothing is downloaded |
| A picture or folder named by `workbench.background_image` | when set | no | a faint backdrop. Local files only, never a web address |
| `<config>\settings.toml`, `keybindings.toml` | yes | when you edit them, pick a theme, or choose "Don't Ask Again" | your settings |
| `<config>\session.json` | yes | every second while editing | hot exit: restores tabs and unsaved text |
| `<config>\themes\*.json` | yes | never | your extra color themes |
| `<config>\trusted_folders.toml` | yes | when you trust or restrict a folder | which folders may run things (workspace trust) |
| `<config>\history\` | yes | on every save (and outside change) of a file | local history: earlier versions of your files (`history.enabled = false` turns it off) |
| `<config>\plugins\` (plugin files, `plugins.lock`) | yes | when you install, update, uninstall, enable or change a plugin's permissions | installed plugins, their hashes, trusted publisher keys and what you granted |
| `<config>\lsp\` | yes | when you agree to download a language server (clangd, jdtls, its Java runtime) | language servers Rust Code installed itself; jdtls keeps its index there too |
| `<config>\codelldb\` | yes | when you agree to download the debugger | CodeLLDB, the debug adapter |
| `<config>\tools\` | yes | when you agree to download a build tool | Ninja, or GCC from winlibs, for CMake projects; only what Rust Code runs uses them |
| `<folder>\.rust-code\settings.toml` | yes | when you edit it | workspace settings |
| Installed VS Code themes (`%LOCALAPPDATA%\Programs\Microsoft VS Code`, `~\.vscode\extensions`) | theme files only | never | the Color Theme picker |
| An installed VS Code CodeLLDB extension (`~\.vscode\extensions\vadimcn.vscode-lldb-*`) | its adapter only | never | debugging without a second download |

Turning off `files.hot_exit` stops unsaved text from being stored in
`session.json` (open tabs are still remembered).

## Builds

Release builds are not yet code-signed. Until they are, build from source with
`cargo build --release` if you need to verify what you run.

`<config>` is `%APPDATA%\rust-code` on Windows, `$XDG_CONFIG_HOME/rust-code` or `~/.config/rust-code` elsewhere.
