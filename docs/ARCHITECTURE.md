# Architecture

Coiman IDE is a Cargo workspace of three crates, and a set of extensions built apart from it.
Two rules shape it: **everything that can work without a window lives in `coiman-core`**, so it
is tested and benchmarked headlessly, and the app only draws and wires; and **everything beyond
editing is an extension** in a pack (see Packs and extensions below), installed only when wanted.

## `crates/coiman-core`

The editor's model and services. No UI dependencies. Modules are grouped by area:

| Area | What is in it |
|---|---|
| `editor` | the rope buffer with selections and undo, documents, line edits, find, Vim and Helix, indent rules, auto tags, Emmet, snippets, structure (folds, outline, smart selection), code generation, commands |
| `language` | tree-sitter and syntect highlighting, language data, the language server client (`lsp`), CSS and HTML knowledge, markup and Markdown, Baseline, checks made without a server |
| `launch` | Run and its rules, the debug adapter client (`dap`), REPLs, live sessions, tests, coverage, problem matchers, stack traces, print debugging, Python environments, live preview |
| `build` | Cargo, Maven, C and C++ builds, compile databases, formatters |
| `project` | Quick Open, search, file watching, file operations, the project tree, recent items, sessions, workspace trust, local history, Git |
| `appearance` | themes and their schedule, fonts, file icons |
| `terminal` | pseudo-terminals and search in their text |
| `extensions` | the WebAssembly plugin host (`plugin`): the sandbox, permissions, approved programs and the services it offers plugins; VS Code import and compatibility; Open VSX (through `curl`); the Node.js host for VS Code extensions (`host`, `host.js`) and the loopback server for their webviews (`webview`) |
| `data` | large logs, CSV and TSV tables, JSON, YAML and TOML trees, bytes, and where client programs are off `PATH` (`clients`: database clients, `adb`) |

`settings`, `packs` (the packs and which commands and extensions each has) and `applog` sit at
the root: every area reads settings, and anything may log.

Data files the core compiles in are in `crates/coiman-core/data/`. The plugin API is
`crates/coiman-core/wit/plugin.wit`.

## `crates/coiman-ide`

The egui app, a library with a small program. `main.rs` calls `lib.rs`'s `run_desktop`, which opens the window; `hooks.rs` lets a port add to the window (see [PORTING.md](PORTING.md)); `app/mod.rs` holds `CodeApp`, the whole state, and the
frame loop. Each file under `app/` adds methods to `CodeApp` for one feature, grouped by area:

| Area | What is in it |
|---|---|
| `shell` | the workbench (menus, side bar, tabs, status bar), command dispatch, configuration, the Settings and Keyboard Shortcuts windows, looks, tool rails, the tour, the Welcome page, the window and frame pacing |
| `editor` | files and saving, split panes, auto save, bookmarks, history, outline, snippets, structural tools, checks shown while typing, web tools |
| `views` | Markdown and SVG previews, live and constant previews, tables, logs, bytes and images, and the OpenGL service extensions draw shaders with (`shader_ui`) |
| `language` | language servers and their features, C and C++, Python environments, the Doctor |
| `launch` | Cargo, Maven, the debugger, the Testing view, coverage, live sessions, REPLs, prints, stack traces |
| `project` | Git, and getting it set up |
| `extensions` | plugins and their panels, packs, the Open VSX marketplace, the VS Code extension host and its webviews, VS Code import, MCP servers for opencode |

Each file is also reachable as `crate::app::<file>`, so moving one between areas changes no
callers. `ui/` holds the widgets the workbench is built from: the editor view, the Explorer, the
palette, the terminal, search, the keymap, icons and themes.

## `crates/coiman-plugin`

The plugin publisher's tool: key generation, turning a built module into a component, signing and
publishing to a registry folder.

## Packs and extensions

A pack is a kind of work (Databases, Embedded and systems, Mobile…), listed in
`crates/coiman-core/src/packs.rs`. A pack that isn't installed adds nothing: no commands,
windows or background work. Each pack's features are extensions: WebAssembly plugins in
`plugins/extensions/<name>/`, written against `crates/coiman-core/wit/plugin.wit` (API 0.7).
They come with Coiman (the release carries their `plugin.toml` and `plugin.wasm`) and are
installed only when asked for, like any plugin: in the sandbox, with the permissions and
programs the user approved.

A plugin draws through blocks the editor renders (forms, rows, tables, logs, plots, graphs,
shaders) or fills the editor's chat view (the Agents panel), and asks the editor for what
the sandbox can't do: start approved programs (jobs),
read files a piece at a time, open serial ports, have the graphics driver compile a shader,
show a pick list or a graph. Long work goes in steps (`continue-later`), since each call has
a time limit.

`plugins/built-in/` are compiled into the app; `plugins/examples/` show how to write one. Each
builds on its own for `wasm32-unknown-unknown`, and the built `plugin.wasm` is committed:

```
powershell -File scripts/build-plugins.ps1
```

A few features stay in the program because they are the editor itself or need it closely:
notebooks, the debugger's memory and disassembly views, some code analysis views, the chat
view the Agents extension fills (`app/agents.rs`), and MCP Servers (it edits opencode's
settings in the home folder).

## Privacy

No networking crates may enter the dependency tree; `scripts/check.ps1` fails if one does. The
network goes through programs on the computer (`git`, `curl`, `ssh`, database clients), only
when the user asks. Two sockets are opened on this computer alone (127.0.0.1): the live preview,
and the webview server for VS Code extensions, which runs only while a webview is open. What the
app reads and writes is in [PRIVACY.md](../PRIVACY.md).
