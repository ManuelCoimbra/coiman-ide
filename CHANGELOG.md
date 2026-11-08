# Changelog

All notable changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

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
