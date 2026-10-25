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
  import), `settings`, `session`, `commands`, `search`, `finder`, `watcher`, `lsp` (language
  server client: JSON-RPC over stdio, position conversion, server discovery), `pty` (shell
  sessions: portable-pty + alacritty_terminal), `git` (git CLI: status, hunks, blame, branches), `cargo` (run
  configurations, test discovery, test/check output parsing), `structure` (folds, outline,
  smart selection, text objects), `history` (local history store), `vim` (Vim/Helix modal editing engine),
  `dap` (debug adapter client for CodeLLDB: DAP over stdio, finding and installing CodeLLDB).
- **`src/` (`rust-code`)** — the egui app: `app/` (state and frame loop, split into
  `dispatch`, `files`, `config`, `workbench`, `lsp`, `git`, `cargo`, `outline`, `history`, `debug`), `editor_view` (the editor widget), `icons` (vector
  icons drawn in code, codicon-inspired), `lsp_ui`, `palette`,
  `search_panel`, `explorer`, `terminal` (terminal tabs and grid widget), `keymap`, `theme` (egui colors and visuals).

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
- Integrated terminal (`` Ctrl+` ``): real shells in a pseudo-terminal (ConPTY on Windows) with
  alacritty's emulator, so PowerShell/bash line editing, colors and full-screen programs (vim,
  less, htop) work. Several terminals as tabs (`` Ctrl+Shift+` `` for a new one), 10,000 lines
  of scrollback (wheel, `Shift+PageUp/Down`), drag to select, `Ctrl+C` copies a selection or
  interrupts, right-click copies, `Ctrl+V` pastes (bracketed paste). **Pop Out** moves the
  terminal into its own window; closing that window puts it back in the panel. The shell is
  `terminal.shell` (default: PowerShell on Windows, `$SHELL` elsewhere)
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

### Source control (Git)

Uses the `git` command line, like VS Code, and only local commands (no fetch, pull or
push). All git work runs in the background.

- **Gutter markers** beside the line numbers: green added, blue modified, red wedge for
  deleted lines, compared with the index. Click one to see the old lines and **Stage** or
  **Revert** just that change; `Alt+F5` / `Shift+Alt+F5` jump between changes;
  `Ctrl+K Ctrl+Alt+S` stages and `Ctrl+K Ctrl+R` reverts the change at the cursor.
  Reverting edits the buffer, so it can be undone.
- **Inline blame** after the cursor line ("You, 3 days ago • Fix parser"), including
  unsaved edits; `git.blame_inline` or **Git: Toggle Inline Blame** turns it off.
- **Source Control view** (`Ctrl+Shift+G`): commit message (`Ctrl+Enter` commits), staged,
  unstaged and conflicted files with stage / unstage / discard (discard asks first), and
  **Stage All and Commit** when nothing is staged.
- **Side-by-side diff**: click a file for index vs working tree (or HEAD vs index for
  staged files), with per-change Stage / Revert / Unstage, previous/next change and
  **Ignore whitespace**.
- **Branches**: the branch in the status bar (with `*` for uncommitted changes and
  ahead/behind counts) opens **Git: Checkout to...**, which can also create a branch.
  Switching runs in the background and open files reload when they change.

### Run, test and check (Cargo)

- **Run configurations** live in `.rust-code/launch.toml` (commit it to share them):
  `name`, `command` (run through the shell, so pipes and `&&` work), optional `cwd` and
  `env`. Without the file, Rust Code offers what Cargo implies: run each binary, test,
  check and clippy the workspace. `Ctrl+F5` runs the selected configuration (asks the first
  time), **Run: Select and Start Configuration...** picks another, **Run: Open
  Configurations** creates the file. Each run gets a terminal tab that stays open with its
  exit code.
- **Testing view** (flask icon): every `#[test]` (also `#[tokio::test]`, `#[rstest]`,
  `#[test_case]`) found in the source, grouped by file and module, with its last result.
  Run all, one file, one module or one test. Run buttons also sit in the editor gutter
  next to each test and test module, showing pass/fail after a run.
  `Ctrl+; C` runs the test at the cursor, `Ctrl+; F` the current file, `Ctrl+; A` all.
- **Runner**: `cargo test`, or `cargo nextest` when it is installed
  (`cargo.test_runner = "auto" | "cargo" | "nextest"`). Output streams into the **Output**
  panel (`Ctrl+Shift+U`); runs can be cancelled.
- **Check and clippy**: **Cargo: Check** / **Cargo: Clippy** put compiler errors and
  warnings in Problems and the editor. `cargo.check_command = "clippy"` makes rust-analyzer
  run clippy instead of check on every save.

### Debugging (CodeLLDB)

Rust Code debugs through [CodeLLDB](https://github.com/vadimcn/codelldb) over the Debug
Adapter Protocol. It uses `debug.codelldb_path`, then its own copy in
`%APPDATA%\rust-code\codelldb`, then an installed VS Code CodeLLDB extension. If none is
found, the Debug view offers to download it (about 135 MB once unpacked). Nothing is
downloaded unless you click **Download**.

- **Start**: `F5` builds with cargo and debugs a binary (asks which one when there are
  several). The bug button next to Run in the Testing view, or `Ctrl+; Ctrl+C`, debugs
  the test at the cursor. The program runs in a **Debug:** terminal tab, so it can read
  input too.
- **Breakpoints**: `F9` or a click left of a line number. **Debug: Add Conditional
  Breakpoint...** pauses only when an expression is true. **Debug: Add Logpoint...**
  prints a message to the Debug Console without pausing. The BREAKPOINTS list enables,
  disables and removes them.
- **Controls**: `F5` continue, `F10` step over, `F11` step into, `Shift+F11` step out,
  `F6` pause, `Shift+F5` stop, `Ctrl+Shift+F5` restart.
- **Debug view** (`Ctrl+Shift+D`): variables, watch expressions, call stack and
  breakpoints. Rust types show readably through CodeLLDB's Rust formatters: `Vec` and
  `HashMap` with their sizes and items, `String`, `Option`. Expanded variables stay
  expanded while stepping.
- **Debug Console** (`Ctrl+Shift+Y`): log point messages and the exit code. While paused,
  it evaluates expressions in the selected frame. Prefix LLDB commands with `` ` ``.
- **Not into std**: stepping skips `std`, `core` and `alloc`. Set
  `debug.step_into_std = true` to step into them.

### Rust language features (rust-analyzer)

Opening a `.rs` file starts [rust-analyzer](https://rust-analyzer.github.io/) for its Cargo
project (found on `PATH`, in `~/.cargo/bin`, or at `lsp.rust_analyzer_path`). If it isn't
installed, a bar offers to install it with `rustup component add rust-analyzer`; nothing is
downloaded unless you click **Install** (**Don't Ask Again** sets `lsp.ask_to_install = false`).

| Feature | How |
| --- | --- |
| Errors and warnings | Squiggles, the message after the line (Error Lens style), hover for details; Problems panel (`Ctrl+Shift+M`), counts in the status bar; `F8` / `Shift+F8` next/previous problem |
| Completion | As you type, after `.`/`:`, or `Ctrl+Space`; `Enter`/`Tab` accepts, snippets place the cursor, auto-imports are added |
| Hover | Rest the pointer on a symbol, or `Ctrl+K Ctrl+I` at the cursor |
| Go to Definition / Implementations | `F12` or `Ctrl+Click` / `Ctrl+F12` |
| Find References | `Shift+F12`, results in the References panel |
| Rename Symbol | `F2`; files that aren't open are opened with the changes unsaved, so you can review and undo |
| Code actions | `Ctrl+.` (quick fixes, refactors) |
| Formatting | `Shift+Alt+F`; `editor.format_on_save = true` formats before saving |

Edits are sent to the server as small incremental changes. `rust-analyzer: Restart Server`
restarts it (e.g. after changing `lsp.rust_analyzer_path`); `lsp.enabled = false` turns
language servers off.

### Vim and Helix modes

`editor.keyboard_mode = "vim"` (or **Toggle Vim Mode**) turns on modal editing;
`"helix"` selects first, then acts. The status bar shows the mode and the command being typed.

- **Vim**: normal, insert, visual (`v`, `V`) and command-line modes; counts; motions
  (`h j k l w b e W B E 0 ^ $ gg G f F t T ; , % { } n N * #`); operators `d c y > < g~ gu gU`
  with motions, doubled (`dd`, `cc`, `>>`…) and text objects (`iw aw i( a" i{`…); `x X s S r ~
  J p P o O i a I A D C Y`; registers (`"a`–`"z`, `"0`, `"+` to the clipboard); `.` repeat;
  `u` / `Ctrl+R` undo whole changes; `/ ?` search; `:w`, `:q`, `:q!`, `:wq`, `:x`, `:<line>`,
  `:s/a/b/g`, `:%s/a/b/g`.
- **Helix**: `w e b W E B` select words, `x` lines, `%` everything, `v` extends, `f t` select
  to a char, then `d c y p P r ~ > < J` act on the selection; `i a I A o O` insert; `u U`
  undo/redo; `gg ge gh gl gs` go to; `;` collapses.
- Insert mode is the normal editor (completion, auto-indent, IME); `Esc` returns to normal.

The engine (`core/src/vim.rs`) is checked by golden files (`core/tests/golden/`): every Vim
case is also run in real Vim (`vim -u NONE`, found on `PATH`, in Git for Windows, or at
`RUST_CODE_VIM`) and the test fails if Vim disagrees.

### Local history

A copy of each file is kept every time it is saved and when another program changes it,
independently of Git, in `%APPDATA%\rust-code\history` (50 versions or 30 days per file by
default; `history.enabled`, `history.max_entries`, `history.max_days`, `history.max_file_kb`).

- The **Timeline** section of the Explorer lists the active file's versions ("Saved · 3
  minutes ago"). Click one to compare it side by side with the current text and **Revert**
  single changes, or **Restore File** to bring the whole version back.
- **Local History: Find Entry to Restore...** picks a version from the palette.
- Restoring is an unsaved, undoable edit, and the text it replaces is kept as a "Before
  restore" version.

### Code structure

From the tree-sitter syntax tree (Rust, JSON, TOML, YAML, Markdown); other files fold by
indentation.

- **Folding**: chevrons beside the line numbers (shown on hover; folded regions keep a `›`
  and a `…` marker after the line). `Ctrl+Shift+[` / `Ctrl+Shift+]` fold/unfold at the
  cursor, `Ctrl+K Ctrl+L` toggles, `Ctrl+K Ctrl+0` / `Ctrl+K Ctrl+J` fold/unfold everything.
  Arrow keys step over folded regions; jumping into one (search, go to line) opens it.
- **Outline**: an Outline section at the bottom of the Explorer (functions, types, impls,
  fields, variants, modules; headings in Markdown; tables and keys in TOML/JSON/YAML), with
  the symbol at the cursor highlighted. The breadcrumbs show the symbol path at the cursor
  (`src › main.rs › mod tests › fn adds`); click a symbol to jump, or the file name for
  **Go to Symbol in Editor** (`Ctrl+Shift+O`).
- **Expand / Shrink Selection** (`Shift+Alt+Right` / `Shift+Alt+Left`): grow the selection
  to the enclosing syntax node, and back.
- **Text objects** (Selection menu and palette): Select Function (with its attributes and doc
  comments), Function Body, Type, Argument, Comment; Go to Next/Previous Function.

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
