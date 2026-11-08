# Architecture

Coiman IDE is a Cargo workspace of three crates. The rule that shapes it: **everything that can
work without a window lives in `coiman-core`**, so it is tested and benchmarked headlessly, and
the app only draws and wires.

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
| `extensions` | the WebAssembly plugin host (`plugin`), VS Code import and compatibility |
| `data` | large logs, CSV and TSV tables, JSON, YAML and TOML trees, bytes |

`settings` and `applog` sit at the root: every area reads settings, and anything may log.

Data files the core compiles in are in `crates/coiman-core/data/`. The plugin API is
`crates/coiman-core/wit/plugin.wit`.

## `crates/coiman-ide`

The egui app. `main.rs` opens the window; `app/mod.rs` holds `CodeApp`, the whole state, and the
frame loop. Each file under `app/` adds methods to `CodeApp` for one feature, grouped by area:

| Area | What is in it |
|---|---|
| `shell` | the workbench (menus, side bar, tabs, status bar), command dispatch, configuration, the Settings and Keyboard Shortcuts windows, looks, tool rails, the tour, the Welcome page, the window and frame pacing |
| `editor` | files and saving, split panes, auto save, bookmarks, history, outline, snippets, structural tools, checks shown while typing, web tools |
| `views` | Markdown and SVG previews, live and constant previews, tables, logs, bytes and images |
| `language` | language servers and their features, C and C++, Python environments, the Doctor |
| `launch` | Cargo, Maven, the debugger, the Testing view, coverage, live sessions, REPLs, prints, stack traces |
| `project` | Git, and getting it set up |
| `extensions` | plugins, VS Code import |

Each file is also reachable as `crate::app::<file>`, so moving one between areas changes no
callers. `ui/` holds the widgets the workbench is built from: the editor view, the Explorer, the
palette, the terminal, search, the keymap, icons and themes.

## `crates/rcplugin`

The plugin publisher's tool: key generation, turning a built module into a component, signing and
publishing to a registry folder.

## Plugins

`plugins/built-in/` are compiled into the app; `plugins/examples/` show how to write one. Each
builds on its own for `wasm32-unknown-unknown`:

```
powershell -File scripts/build-plugins.ps1
```

## Privacy

No networking crates may enter the dependency tree; `scripts/check.ps1` fails if one does. What
the app reads and writes is in [PRIVACY.md](../PRIVACY.md).
