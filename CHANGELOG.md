# Changelog

All notable changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- The Markdown preview draws ```mermaid blocks (flowchart, class, sequence, state, ER and more) with
  merman, matching Mermaid's look without a browser; `.mmd` files preview too.
- The Architecture window's Mermaid export groups files by folder, escapes names Mermaid would
  read as syntax, marks tests green and cycles red, and falls back to the folder graph past
  Mermaid's 500-arrow limit. **Preview Mermaid** shows the diagram in a Markdown draft.

### Fixed

- Deleting many files or folders no longer freezes the window: the work runs in the
  background, unsaved tabs under the deleted paths are saved first, and the status bar
  reports how many items went to the Recycle Bin.
- New File and New Folder make the item in the selected folder (or the selected file's
  folder) from the Explorer's header and a folder's own New menu, not always in the project
  root.
- Shift+Tab outdents the caret's line, or every selected line, as Tab indents them.

## [0.3.0] - 2026-11-08

### Changed

- The last traces of the old name, Rust Code, are gone:
  - `PRIVACY.md` names Coiman IDE and its folders.
  - The publisher's tool is `coiman-plugin` (was `rcplugin`).
  - The plugin API is `coiman:plugin@0.5.0`. Plugins built for the old package are told to
    rebuild; registry signatures made before the rename are still accepted.
  - Built-in extensions are `coiman.*`, keeping their on/off choice and permissions.
  - Command ids are `coiman.*`. Keybindings saved with `rustCode.*`, `rustcode.*` or
    `rust-code.*` still work.
  - Development variables are `COIMAN_TEST_PANIC`, `COIMAN_REPAINT_LOG` and `COIMAN_VIM`.

### Fixed

- The Debug view's floating scroll bar covered the remove buttons of breakpoints.
- A debugger that quit before starting ended the session without saying why.

### Added

- Debugging: function breakpoints, exception breakpoints (the debugger's own filters, such as
  C++ throw and catch in gdb), data breakpoints ("on change" beside a variable; gdb through its
  watchpoints), export and import of breakpoints as JSON, and Debug: Attach to Process with
  gdb, CodeLLDB, debugpy, js-debug or Delve. Breakpoints are kept per project between runs.
- `scripts/tidy.ps1`: frees the space old builds leave in `target/` (it reached 38 GB here).

## [0.2.0] - 2026-11-08

### Changed

- The project is a workspace of `crates/coiman-core`, `crates/coiman-ide` and `crates/rcplugin`,
  with plugins in `plugins/`. Core modules and app files are grouped in folders by area.
- Docs split: features in `docs/FEATURES.md`, the layout in `docs/ARCHITECTURE.md`.

### Fixed

- Workspace trust: a file not created yet in a trusted folder counted as untrusted when the
  folder's path was written another way (a junction, `..`, or a Windows short name such as
  `RUNNER~1`).

### Added

- The Database window for SQLite: tables, views, columns and foreign keys, SQL with a result
  grid per statement, history, and running a `.sql` file or selection against it.
- The Architecture window: the project's dependency graph by file or folder for eight languages,
  import cycles, the most used and unused files, the impact of a change (files and tests),
  SVG export and Mermaid copy. Commands: Architecture: Show Dependency Graph, Architecture:
  Impact of Current File.
- MIT license, contributing guide, this changelog, and CI on Windows (format, clippy, tests).
- The Dock layout, now the default (`workbench.ui = "dock"`): tools in a dock at the bottom,
  opening as sheets over the code that can be pinned beside it; open files as cards; the
  bottom panel as a floating card; a new top bar with the icon as the menu and Run, its target
  and Debug in one pill. Islands and Classic remain as settings.
- Settings with section pills, rows on cards and switches; a restyled command palette.
- Transitions: the lit file card and the dock's dot glide, sheets slide in, a tool's content
  slides in when it changes. They follow the reduced motion setting.

## [0.1.0] - 2026-11-07

The first version, as Coiman IDE (it began as Rust Code on 2026-10-24).

### Added

- **Editor**: rope buffer, multiple cursors and column selection, find and replace, folding,
  outline, smart selection, Vim and Helix modes, indentation by each language's blocks, tags and
  quotes closed as you type, Emmet, snippets, bookmarks, split panes, local history, auto save.
- **Languages**: tree-sitter and syntect highlighting, rainbow brackets, language servers for
  Rust, Python, JavaScript and TypeScript, C and C++, Java, C# and Go and more, with completion,
  navigation, inlay hints, semantic tokens, code lens and hierarchies; ESLint and Tailwind from
  VS Code extensions; HTML, XML, Vue and Svelte checked while typing; a language Doctor.
- **Run and debug**: Run for every language with project rules, run configurations, Cargo and
  Maven, a Testing view, coverage, debugging with CodeLLDB, debugpy, Delve, js-debug and gdb,
  REPLs and live sessions, print debugging, stack trace analysis, problem matchers.
- **Project**: Explorer with a JetBrains-style tree, keyboard and file operations, Quick Open,
  Search Everywhere, find in files, Git (status, staging lines, history, stash, push and pull,
  setup), workspace trust, recent projects and files.
- **Views**: Markdown and SVG previews, live preview in the browser, CSV tables, logs of any size,
  bytes, memory and disassembly, images.
- **Look**: Coiman Dark theme, VS Code themes, island layout, background pictures, themes by time
  of day, file icons, a Welcome page and a tour.
- **Extensions**: WebAssembly plugins with signed registries; import of VS Code themes, snippets
  and languages; a VS Code compatibility report.
- **Privacy**: no telemetry, no network access from the app, no account.
