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

Future features that need the network (for example downloading a language
server) will ask first, every time, and say exactly what will be downloaded
from where.

## What Rust Code reads and writes on your machine

| Location | Read | Written | Why |
| --- | --- | --- | --- |
| Files and folders you open | yes | when you save | editing |
| `%APPDATA%\rust-code\settings.toml`, `keybindings.toml` | yes | when you edit them or pick a theme | your settings |
| `%APPDATA%\rust-code\session.json` | yes | every second while editing | hot exit: restores tabs and unsaved text |
| `%APPDATA%\rust-code\themes\*.json` | yes | never | your extra color themes |
| `<folder>\.rust-code\settings.toml` | yes | when you edit it | workspace settings |
| Installed VS Code themes (`%LOCALAPPDATA%\Programs\Microsoft VS Code`, `~\.vscode\extensions`) | theme files only | never | the Color Theme picker |

Turning off `files.hot_exit` stops unsaved text from being stored in
`session.json` (open tabs are still remembered).

## Builds

Release builds are not yet code-signed. Until they are, build from source with
`cargo build --release` if you need to verify what you run.
