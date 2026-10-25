# Rust Code

A lightweight VS Code–style editor written in Rust with [egui](https://github.com/emilk/egui).

## Run

```
cargo run --release -- [folder-or-file]
```

## Project structure

A Cargo workspace with two crates:

- **`core/` (`rust-code-core`)** — everything that doesn't need a UI, so it can be tested
  and benchmarked headlessly: `buffer` (rope, selections, undo), `edit_ops` (line commands),
  `document`, `syntax` (tree-sitter worker) and `highlight` (syntect), `theme` (data, VS Code
  import), `settings`, `session`, `commands`, `search`, `finder`, `watcher`.
- **`src/` (`rust-code`)** — the egui app: `app/` (state and frame loop, split into
  `dispatch`, `files`, `config`, `workbench`), `editor_view` (the editor widget), `palette`,
  `search_panel`, `explorer`, `terminal`, `keymap`, `theme` (egui colors and visuals).

Checks (no networking crates, formatting, clippy with warnings as errors, tests):

```
powershell -File scripts/check.ps1
```

`cargo lint` runs just clippy for the workspace.

Benchmarks against the performance targets (keystroke latency, incremental reparse, line
commands, large-file open, Quick Open over 100k files, startup time, idle memory):

```
powershell -File scripts/bench.ps1 [-Quick]
```

`cargo bench -p rust-code-core` runs only the headless part. Each row prints p50/p99 and
PASS, FAIL or GAP; a GAP is a known shortfall tracked on the roadmap and doesn't fail the run.

## Privacy

No telemetry, no network access, no account, no AI. See [PRIVACY.md](PRIVACY.md) for what
is read and written on your machine.

## Features

- File explorer (lazy folder tree), tabs with unsaved markers, breadcrumbs
- Custom editor widget on a rope (`ropey`): only visible rows are laid out and drawn, so
  million-line files stay responsive; files over 16 MB / 100k lines open in large-file mode
  (no highlighting or wrap)
- Tree-sitter highlighting for Rust, JSON, TOML, YAML and Markdown: incremental parsing on a
  background thread, queries only over the visible lines, VS Code Dark+ colors
- Other languages: syntect highlighting with parser checkpoints every 32 lines
- Rainbow brackets by nesting depth (tree-sitter languages), indent guides, line numbers,
  current-line and matching-bracket highlight
- Multi-cursor: `Ctrl+D` adds the next match, `Ctrl+Alt+Up/Down` adds cursors, `Alt+Click`,
  `Esc` returns to one cursor; per-document undo/redo (`Ctrl+Z`, `Ctrl+Y`) grouped by word
- VS Code editing behaviour: auto-indent on Enter (splits `{|}`), smart Home, `Ctrl+L` line
  select, copy/cut whole line with no selection, `Ctrl+Enter` line below
- Quick Open (`Ctrl+P`): background index of the folder (respects `.gitignore`, skips
  `target`/`node_modules`/`.git`) ranked by nucleo, with matched letters highlighted;
  Command Palette (`Ctrl+Shift+P`), Go to Line (`Ctrl+G`)
- Find in file (`Ctrl+F`); Find in files (`Ctrl+Shift+F`) with the ripgrep crates: search as
  you type, results stream in grouped by file, match case / whole word / regex, files to
  include/exclude globs, binary files skipped; clicking a result selects the match
- External changes: files edited by other programs reload automatically when the tab has no
  unsaved changes; otherwise a bar offers Reload or Keep mine. Deleted files are flagged
  (saving recreates them). The explorer and Quick Open index follow created/deleted files
- Integrated terminal (`` Ctrl+` ``) with history and `cd`
- Save / Save As / Save All (`Ctrl+K S`)
- Encodings: files are detected from their BOM, then UTF-8, falling back to Windows 1252.
  Click the encoding in the status bar to Reopen with Encoding or Save with Encoding
  (UTF-8 with/without BOM, UTF-16 LE/BE, Windows and ISO code pages, Shift JIS, GBK, Big5,
  EUC-KR...). Saving refuses characters the encoding can't represent instead of losing them
- Line endings: click `LF`/`CRLF` in the status bar to convert the file (undoable). New
  files use `files.eol` (`"auto"`, `"lf"`, `"crlf"`) and `files.encoding`
- Hot exit: open folder, tabs and unsaved text are restored on next launch, even after a crash
  (backed up every second to `%APPDATA%\rust-code\session.json`; no "save changes?" prompt on exit)
- Zoom (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`), toggle side bar (`Ctrl+B`)

### VS Code editor actions

Command IDs, labels and default keybindings were taken from VS Code 1.143.1's
shipped `workbench.desktop.main.js` and `nls.messages.json`.

| Command | Keys |
| --- | --- |
| Toggle Line Comment | `Ctrl+/` |
| Move Line Up / Down | `Alt+Up` / `Alt+Down` |
| Copy Line Up / Down | `Shift+Alt+Up` / `Shift+Alt+Down` |
| Delete Line | `Ctrl+Shift+K` |
| Indent / Outdent Line | `Ctrl+]` / `Ctrl+[` (also `Tab` / `Shift+Tab` on a multi-line selection) |
| Add Selection to Next Find Match (single selection) | `Ctrl+D` |
| Go to Bracket | `Ctrl+Shift+\` |
| Toggle Word Wrap | `Alt+Z` |
| Toggle Zen Mode | `Ctrl+K Z` (`Esc` exits) |
| Reopen Closed Editor | `Ctrl+Shift+T` |

Palette only: Join Lines, Duplicate Selection, Sort Lines Ascending/Descending,
Delete Duplicate Lines, Reverse Lines, Transform to Uppercase/Lowercase/Title Case.

### Settings, keybindings and themes

- **Settings** (`Ctrl+,`): TOML, layered defaults → user
  (`%APPDATA%\rust-code\settings.toml`) → workspace (`<folder>/.rust-code/settings.toml`).
  Files can `extends = "other.toml"` and have `[windows]` / `[macos]` / `[linux]` tables.
  Unknown keys and errors show in the status bar; saving a settings file applies it.
  Separate `editor.font_size` and `workbench.ui_font_size`.
- **Keybindings** (`Ctrl+K Ctrl+S`): `%APPDATA%\rust-code\keybindings.toml` with VS Code
  command IDs, two-key chords, `when = "editorTextFocus"`, and `-command` to remove a
  default. Menus and the palette show the active shortcuts.
- **Color themes** (`Ctrl+K Ctrl+T`): any VS Code theme. The picker lists themes from the
  installed VS Code (bundled and `~/.vscode/extensions`) and `%APPDATA%\rust-code\themes`;
  `workbench.color_theme` also accepts a path to a theme `.json`. Theme files may use
  comments and `include`; token colors apply to tree-sitter and syntect highlighting.
