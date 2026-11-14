# Changelog

All notable changes. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Run History**: test runs with what broke, was fixed or got slower since the run before,
  flaky tests (passing and failing by turns), and builds, checks and runs with their first
  error. Failed Cargo tests can be run again on their own.
- `docs/SPEC-COVERAGE.md`: each item of the feature specification, done, partial or missing.

## [0.4.1] - 2026-11-14

### Added

- **Code Health** window: secrets left in files (shown masked), each function's complexity,
  duplicate code, the dependencies' licences, and `cargo audit` / `npm audit` results.

### Changed

- An Android build of Coiman (an APK that runs its tools through Termux) keeps Android's own
  picker for Open File and Open Folder, since only it grants access to what's picked.

### Removed

- The on-screen keyboard and the Termux:X11 guide added in 0.4.0. Coiman on Android is an APK
  with Android's keyboard and its own keyboard button, so a second keyboard only got in the
  way. Touch sizes are now off unless asked for.

### Fixed

- The dock sat off the middle of the bottom strip when no panel was open (noticeable at 125%):
  the editor and side bar no longer leave a gap of their own above it.

## [0.4.0] - 2026-11-14

### Added

- The Markdown preview draws ```mermaid blocks (flowchart, class, sequence, state, ER and more) with
  merman, matching Mermaid's look without a browser; `.mmd` files preview too.
- The Architecture window's Mermaid export groups files by folder, escapes names Mermaid would
  read as syntax, marks tests green and cycles red, and falls back to the folder graph past
  Mermaid's 500-arrow limit. **Preview Mermaid** shows the diagram in a Markdown draft.
- Files and folders can be dragged onto a folder in the Explorer (or the project row) to move
  them there, with open tabs following and name conflicts reported.
- Dragging a tab works like a browser: the tab follows the pointer and the others slide aside
  to show where it will land. Dragged off the bar, a card follows the pointer, and over the
  right half of the editor a band says it will open beside the current file.
- **Replace in files** (`Ctrl+Shift+H`) opens the search sheet with its replace row, and
  Search Everywhere has a **Replace…** button that takes what was typed along.
- Movable panels: the panel's ≡ menu puts it at the bottom, at the right or in a window of
  its own (dragged to the editor's bottom or right edge, it docks there again). The side bar
  can go right of the editor. Positions, sizes and a pinned sheet are kept with the session;
  **View: Reset Layout** puts everything back.
- Linux packages for x86_64 and ARM64: deb, rpm, AppImage, Flatpak, Snap, Arch PKGBUILDs
  (from source and prebuilt) and a tar.gz with install and uninstall scripts. Each installs a
  launcher with the icon and the file types it opens, and CI installs and starts each one on
  Debian, Ubuntu, Fedora, openSUSE and Arch before a release. See `docs/INSTALL-LINUX.md`.
- `coiman-ide --version` prints the version.
- A File Manager in place of the editor, with one or two panes, tabs, bookmarks, a preview,
  sorting and filtering. It copies, moves, renames and deletes with open files following,
  and drags and the clipboard work with the Explorer. Open File and Open Folder now choose
  with it instead of the system's dialog.
- On Android in Termux (with Termux:X11): an on-screen keyboard made for code, whose button
  cycles OFF, ON and AUTO (`workbench.soft_keyboard`, kept between runs). AUTO shows it while
  typing and hides it while a hardware keyboard is used. It comes with larger touch sizes
  (`workbench.touch`) and the phone's storage and Termux's home in the File Manager. See
  `docs/ANDROID-TERMUX.md`.

### Changed

- One search button: the dock's magnifier is gone, as it did what the top bar's search box
  does. `Ctrl+Shift+F` still opens the search sheet.

### Fixed

- The buttons at the right of the tab bar (Hex, Preview…) sat higher than the tab names in the
  Dock layout; they're centred on the bar as drawn now, at any interface scale. Settings with a
  choice (Font, Keyboard mode and others) show their value at the right edge with ▾, like
  Color theme, instead of left-aligned in a wide drop-down.
- The dock's icons line up and its open-tool dot no longer sits squeezed on the tile's edge.
- The Explorer's type badges follow each language: C, Rust and Go structs show S, not C;
  headers with only functions, typedefs and type aliases show no type.
- Classic layout: clicking the open tool's icon hides the side bar, and that is remembered.
- Up and Down move by rows on screen when lines wrap or folds hide some, keeping the column,
  instead of skipping a wrapped line's rows or landing in a fold. Typing on an empty line
  inside a block starts at the block's indentation.
- The bottom panel's tabs no longer run under its buttons: they scroll sideways in the room
  the buttons leave. The status bar's right side keeps what fits, by priority (line and
  column first), instead of running over the left side.
- The error count drops as errors are fixed: what a cargo check, a run or a test run said
  about a file stops counting once the file is edited, and counts again at the next run.
- Git: branches pushed by others appear in Checkout after Fetch or Pull (as `origin/name`),
  and picking one makes a local branch that tracks it. Fetch and Pull drop branches deleted
  on the remote. The branch label shows only what there is to pull (↓) and push (↑).
- Renaming or moving an open file with unsaved edits no longer says it changed on disk, and
  saving writes to the new name. A change on disk now counts only when the file no longer
  holds what it held when last read or saved.
- Mermaid diagrams too big for merman's default layout budget (a project's folder graph)
  draw instead of showing their text.
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
