# Privacy

Rust Code is built to be trusted with your code. These are commitments, and
`scripts/check.ps1` enforces the network one on every run.

## What Rust Code never does

- **No telemetry.** No usage statistics, crash reports or analytics are
  collected or sent, and there is no setting to turn them on.
- **No network access.** The editor makes no network connections: no update
  checks, no downloads, no "what's new" feeds. The build fails if a networking
  crate (HTTP clients, TLS, async network runtimes) enters the dependency tree.
- **No account.** Nothing requires signing in.
- **No AI by default.** There are no AI features today. If optional AI is added
  later, it will be off until you turn it on, use the provider/key you choose,
  and never run on its own.

Anything that needs the network asks first and says what will be downloaded
from where:

- **Installing rust-analyzer.** If it is missing, Rust Code offers to run
  `rustup component add rust-analyzer`, which downloads it from
  static.rust-lang.org. Nothing runs until you click **Install**.
- **rust-analyzer itself** is a separate program that runs `cargo metadata` and
  `cargo check` in your project, exactly as it does in other editors. Cargo
  downloads the project's dependencies from crates.io if they aren't already
  on your machine. Set `lsp.enabled = false` to never start it.
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
- **Plugins themselves** have no network access. They run sandboxed with only
  the editor's plugin API, and they reach your files and editor only through
  the permissions you granted.

## What Rust Code reads and writes on your machine

| Location | Read | Written | Why |
| --- | --- | --- | --- |
| Files and folders you open | yes | when you save | editing |
| `%APPDATA%\rust-code\settings.toml`, `keybindings.toml` | yes | when you edit them, pick a theme, or choose "Don't Ask Again" | your settings |
| `%APPDATA%\rust-code\session.json` | yes | every second while editing | hot exit: restores tabs and unsaved text |
| `%APPDATA%\rust-code\themes\*.json` | yes | never | your extra color themes |
| `%APPDATA%\rust-code\history\` | yes | on every save (and outside change) of a file | local history: earlier versions of your files (`history.enabled = false` turns it off) |
| `%APPDATA%\rust-code\plugins\` (plugin files, `plugins.lock`) | yes | when you install, update, uninstall, enable or change a plugin's permissions | installed plugins, their hashes, trusted publisher keys and what you granted |
| `%APPDATA%\rust-code\codelldb\` | yes | when you agree to download the debugger | CodeLLDB, the debug adapter |
| `<folder>\.rust-code\settings.toml` | yes | when you edit it | workspace settings |
| Installed VS Code themes (`%LOCALAPPDATA%\Programs\Microsoft VS Code`, `~\.vscode\extensions`) | theme files only | never | the Color Theme picker |
| An installed VS Code CodeLLDB extension (`~\.vscode\extensions\vadimcn.vscode-lldb-*`) | its adapter only | never | debugging without a second download |

Turning off `files.hot_exit` stops unsaved text from being stored in
`session.json` (open tabs are still remembered).

## Builds

Release builds are not yet code-signed. Until they are, build from source with
`cargo build --release` if you need to verify what you run.
