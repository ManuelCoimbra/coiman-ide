# Rust Code

A lightweight VS Code–style editor written in Rust with [egui](https://github.com/emilk/egui).

The code font is `editor.font_family`: any fixed-width font installed on the computer, picked in
Settings, or a path to a font file.

Settings (`Ctrl+,`) and keyboard shortcuts (`Ctrl+K Ctrl+S`) each have a window, so no file editing
is needed; **Edit as File** still opens the TOML behind them.

`<config>` below is your config folder: `%APPDATA%\rust-code` on Windows,
`$XDG_CONFIG_HOME/rust-code` or `~/.config/rust-code` elsewhere.

## Run

```
cargo run --release -- [folder-or-file]
```

## Project structure

A Cargo workspace with three crates:

- **`core/` (`rust-code-core`)** — everything that doesn't need a UI, so it can be tested
  and benchmarked headlessly: `buffer` (rope, selections, undo), `edit_ops` (line commands),
  `document`, `syntax` (tree-sitter worker) and `highlight` (syntect), `theme` (data, VS Code
  import), `settings`, `session`, `commands`, `search`, `finder`, `watcher`, `lsp` (language
  server client: JSON-RPC over stdio, position conversion, server discovery), `pty` (shell
  sessions: portable-pty + alacritty_terminal), `git` (git CLI: status, hunks, blame, branches), `cargo` (run
  configurations, test discovery, test/check output parsing), `structure` (folds, outline,
  smart selection, text objects), `history` (local history store), `vim` (Vim/Helix modal editing engine),
  `dap` (debug adapter client for CodeLLDB: DAP over stdio, finding and installing CodeLLDB),
  `plugin` (WebAssembly plugin runtime on Wasmtime, manifests and permissions, signed
  registries, lockfile). The plugin API is `core/wit/plugin.wit`.
- **`src/` (`rust-code`)** — the egui app: `app/` (state and frame loop, split into
  `dispatch`, `files`, `config`, `workbench`, `lsp`, `git`, `cargo`, `outline`, `history`, `debug`, `plugins`), `editor_view` (the editor widget), `icons` (vector
  icons drawn in code, codicon-inspired), `lsp_ui`, `palette`,
  `search_panel`, `explorer`, `terminal` (terminal tabs and grid widget), `keymap`, `theme` (egui colors and visuals).
- **`tools/rcplugin`** — the plugin publisher's tool: key generation, turning a built
  module into a component, signing and publishing to a registry folder.

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

- Integrated title bar, as in VS Code: menus, the window title and the window buttons share
  one bar. Drag it to move the window, double-click it to maximize, resize from any edge.
- File explorer (lazy folder tree), tabs with unsaved markers, breadcrumbs, file-type icons, and create, rename and delete in the Explorer (right-click; delete goes to the Recycle Bin)
- Split the editor with `Ctrl+\\` or by dragging a tab onto the right half of the editor; each pane
  scrolls on its own. `Ctrl+1` / `Ctrl+2` move between them, `Shift+Alt+0` flips the layout, the
  divider drags, and **View > Single Editor** goes back to one. Drag tabs to reorder them
- Bidi controls, invisible characters and Private Use Area characters show as a red `?`, with a
  warning above the file and **Next** to step through them, naming each one in the status bar. They
  stay in the file until you delete them (`editor.unicode_warnings`)
- `Ctrl+R` reopens a recent folder (also **File > Open Recent** and the welcome page)
- The status bar's indentation opens a picker: spaces, tabs, or detect from the content
- The view glides when keys move it (`editor.smooth_scrolling`); long jumps cut straight there
- Bookmarks: `Ctrl+Alt+K` toggles, `Ctrl+Alt+L` / `Ctrl+Alt+J` go next and previous, **Bookmarks: List
  All** covers every file. They follow their lines as you edit and survive a restart
- Your own snippets (**Snippets: Manage**): type a prefix and press Tab, or **Snippets: Insert
  Snippet**. Bodies use `$1`, `${1:text}` and `$0`, kept in `<config>/snippets.toml`. Only the first
  stop is selected; there is no tabbing between stops yet
- Terminal search (`Ctrl+F` in a terminal): scrollback included, across wrapped lines, with match
  case; `Enter` and `Shift+Enter` move through matches
- Markdown preview (`Ctrl+K V`, **View > Markdown Preview**) beside the editor, updating as you type.
  Links open only when clicked, and images show as their alt text: nothing is fetched
- Auto save (`files.auto_save`): after a delay, or on leaving the file or window. Saves after a
  delay don't each add a local history entry, and files changed on disk wait for you
- Find and replace (`Ctrl+F`, `Ctrl+H`): match case, whole word, regular expressions with `$1`
  groups. The Search view replaces across the project after showing the count; open files change
  with undo and stay unsaved, others change on disk (UTF-8 only)
- Multiple cursors: `Ctrl+D` adds the next occurrence, `Ctrl+Shift+L` selects all of them,
  `Alt+click` and `Ctrl+Alt+Up/Down` add cursors. Column selection with `Shift+Alt`+drag (from the
  cursor) or a middle-button drag
- Brackets and quotes type in pairs, step over their closer, surround a selection, and Backspace
  removes an empty pair (`editor.auto_close_brackets`)
- Indentation follows `.editorconfig`, then how the file is already indented, then
  `editor.tab_size` and `editor.insert_spaces`. `.editorconfig` also sets
  `trim_trailing_whitespace` and `insert_final_newline` on save. Tabs display `editor.tab_size` columns wide
- Custom editor widget on a rope (`ropey`): only visible rows are laid out and drawn, so
  million-line files stay responsive; files over 16 MB / 100k lines open in large-file mode
  (no highlighting or wrap)
- Tree-sitter highlighting for Rust, Python, JavaScript/JSX, TypeScript/TSX, Java, C, C++,
  C#, Go, HTML, CSS, Bash, JSON, TOML, YAML and Markdown: incremental parsing on a
  background thread, queries only over the visible lines, VS Code Dark+ colors. Code
  folding, rainbow brackets, expand/shrink selection and the Outline work in all of them
  (for Python, JS/TS, Java, C/C++, C# and Go the Outline lists classes, interfaces,
  functions and methods).
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
  (backed up every second to `<config>\session.json`; no "save changes?" prompt on exit)
- **Run** (`Shift+F10`): the title bar button shows only for files that can run, and its tooltip names the
  program and command before you click (`(project)` when the file stands for its package). In a Cargo
  project, what runs comes from `cargo metadata`: a file that starts a target runs that target
  (tests and benches through `cargo test` and `cargo bench`), and modules, the library and
  `Cargo.toml` run the package's program. C# reads `OutputType` and solutions, C and C++ use
  `add_executable` and `make run`, and `package.json`, `Makefile`, `Dockerfile` and Compose files
  offer what they define. `F5` follows the same rules for Rust, and says so for other languages.
  The gutter also runs single Go, Python and JavaScript or TypeScript tests. Anything else says why it can't, beside the button
- Zoom the whole window, text, icons and panels (`Ctrl+=`, `Ctrl+-`, `Ctrl+0`, `Ctrl`+wheel,
  pinch), saved as `workbench.zoom`, 50% to 300%, default 125%. `Ctrl`+wheel or pinch over the
  code or terminal zooms only that part (`editor.font_size`, `terminal.font_size`), and
  `Ctrl+Shift+=`, `Ctrl+Shift+-`, `Ctrl+Shift+0` zoom the code. The magnifying glass in the status bar has
  buttons for each; toggle side bar (`Ctrl+B`)

### Source control (Git)

Uses the `git` command line, like VS Code, and only local commands (no fetch, pull or
push). All git work runs in the background.

- **Gutter markers** beside the line numbers: green added, blue modified, red wedge for
  deleted lines, compared with the index. Click one to see the old lines and **Stage** or
  **Revert** just that change; `Alt+F5` / `Shift+Alt+F5` jump between changes;
  `Ctrl+K Ctrl+Alt+S` stages and `Ctrl+K Ctrl+R` reverts the change at the cursor, or only the
  selected lines when there is a selection.
  Reverting edits the buffer, so it can be undone.
- **Inline blame** after the cursor line ("You, 3 days ago • Fix parser"), including
  unsaved edits; `git.blame_inline` or **Git: Toggle Inline Blame** turns it off.
- **Source Control view** (`Ctrl+Shift+G`): commit message (`Ctrl+Enter` commits), staged,
  unstaged and conflicted files with stage / unstage / discard (discard asks first), and
  **Stage All and Commit** when nothing is staged. **Pull**, **Push** (publishing a new branch) and
  **Fetch** show how many commits each way; they run only when you ask, using your Git credentials.
  **Amend last commit** replaces the last commit. **Stash** puts all changes away, with **Apply**,
  **Pop** and **Drop** on each stash. **History** lists the latest 200 commits; click one for its
  files, and a file for what the commit changed (**Git: Show History**).
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
  check and clippy the workspace. The title bar's **Current File** selector sets what Run starts: the active file, or a
  configuration, remembered per folder. `Ctrl+F5` runs the selected configuration (asks the first
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

### Run Program

The green button in the title bar (or `Shift+F10`, or the Run menu) saves your files and runs
the active file the way its language runs, in a terminal tab:

| Language | What runs |
| --- | --- |
| Python | The project's `.venv`, `venv` or `env` if there is one, else `python` |
| JavaScript, TypeScript | `node` (a project's `tsx` for TypeScript if it has one) |
| Java | Finds the class with `main` and its package, even from another file (asks if several fit). `mvn exec:java` or `gradle run` in those projects, otherwise `javac` then `java`. Java 25 classless files run directly |
| C, C++ | Checks for `main`, compiles with `gcc`/`g++` or `clang`/`clang++`, runs. CMake, Make and Meson projects need a run configuration |
| Rust | `cargo run`, `--bin` or `--example` from the file's place (asks if several binaries fit). `rustc` outside Cargo |
| Go | `go run .` in a module, else the file. Only `package main` |
| C# | `dotnet run --project`, or the file alone without a project (.NET 10) |
| Others | Shell, PowerShell, batch, Ruby, PHP, Lua, Perl, R, Dart, Swift, Kotlin; HTML opens in the browser |

Build output goes to the system temp folder, never your project. For anything else, use
**Run: Open Configurations**.

### Workspace trust

Opening a folder runs nothing from it. Until you click **Trust Folder**, it stays in Restricted
Mode: you can read and edit files, but its `.rust-code/settings.toml` is ignored, and language
servers, Git, Cargo, run configurations, Run Program and debugging are off. Trust covers
subfolders and is remembered outside the project. **Workspaces: Trust or Restrict This Folder**
changes your mind later. The built-in terminal is your own shell and stays available.

### Debugging (CodeLLDB)

Rust Code debugs through [CodeLLDB](https://github.com/vadimcn/codelldb) over the Debug
Adapter Protocol. It uses `debug.codelldb_path`, then its own copy in
`<config>\codelldb`, then an installed VS Code CodeLLDB extension. If none is
found, the Debug view offers to download it (about 135 MB once unpacked). Nothing is
downloaded unless you click **Download**.

- **Start**: `F5` builds with cargo and debugs a binary (asks which one when there are
  several). The bug button next to Run in the Testing view, or `Ctrl+; Ctrl+C`, debugs
  the test at the cursor. The program runs in a **Debug:** terminal tab, so it can read
  input too.
- **Print debugging**: values printed by `dbg!` in any terminal or in test output collect in
  the **DBG** panel, where a filter narrows them and clicking a location goes to the call. The
  latest value, and how many were printed, show beside each call while the line still holds
  it. **Rust: Remove dbg! from This File** and **from Workspace** take the calls out and keep
  what they evaluate: `dbg!(f(x))` becomes `f(x)`, and `dbg!(x);` goes entirely. Workspace
  removal asks first and leaves files open and unsaved to review or undo. Calls holding comments
  stay. Not done yet: comparing with logpoints, and reading `dbg!` in the Debug Console
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

### Plugins (WebAssembly)

Plugins are WebAssembly components written against `core/wit/plugin.wit` and run in
[Wasmtime](https://wasmtime.dev/), on their own thread.

- **Sandboxed**: a plugin gets only the editor's plugin API. It has no WASI, so it has no
  file system, network, clock or environment of its own. File access stays inside the open
  folder. Each call has a 2-second time limit and each plugin a 64 MB memory limit. A plugin
  that crashes or runs too long is stopped, and the Extensions view shows why.
- **Programs**: `programs = ["git"]` in `plugin.toml` asks to start programs by name. They
  run outside the sandbox with your rights, so each starts unapproved until you tick it, only
  in trusted folders, and each start is logged. `start-job` has no time limit and reports back
  with a `job-finished` event, with stdin, stdout, stderr and the exit code.
- **Permissions**: `plugin.toml` lists what a plugin needs: `workspace-read`,
  `workspace-write`, `editor-read`, `editor-write`, `language-server` (standard or custom
  requests to the active file's server). You approve them before it is
  installed and can uncheck any of them, then or later. A call without its permission fails
  with `permission denied`.
- **Contribution points** are declared in `plugin.toml` and drawn by the editor:
  - status bar items;
  - bottom panel tabs (headings, text, code, lists, buttons, colored text, tables, diffs);
  - tree views in the Explorer;
  - Command Palette commands.

  Plugins fill them with data and receive the events they subscribe to: `opened`, `saved`
  and `active-changed`.
- **Registry**: `plugins.registry` is a folder or URL with a `registry.json`. Nothing is
  fetched until you click refresh in the Extensions view (`Ctrl+Shift+X`). Each version is
  signed with the publisher's Ed25519 key. A plugin installs only if its files match their
  hashes and the signature verifies. The first install from a publisher shows the key's
  fingerprint and asks you to trust it.
- **Lockfile**: `<config>\plugins\plugins.lock` records each plugin's version,
  source, publisher key, file hashes and granted permissions. A plugin whose files changed
  since install isn't loaded; **Reinstall…** fetches it again.
- **Updates** are only looked for when you check the registry. Before updating you see:
  - the version change and changelog;
  - new and dropped permissions (new ones are marked);
  - added and removed contributions;
  - the size change;
  - a warning if the signing key changed.
- **Install from Folder…** installs an unsigned plugin you are developing (marked
  unsigned).

**Built in**, as plugins you can turn off in the Extensions view:

- **Comment Highlights**: TODO, FIXME, BUG, HACK, XXX and NOTE tags, and comments starting with
  `!`, `?` or `*`.
- **Trailing Spaces**: marks whitespace at line ends.
- **Color Preview**: `#rgb`, `#rrggbb` and `rgb()` shown in their own color.
- **SCS Units** (optional, not installed by default; **Plugins: Install from Folder...** on
  `extensions/scs-sii`): Euro Truck Simulator 2 and American Truck Simulator `.sii` files get colors, a
  **SII Units** list in the Explorer, and checks for unbalanced braces, unclosed strings, duplicate
  unit names, malformed lines and a missing `SiiNunit`.

Plugins can color text (`set-decorations`) and report in the Problems panel (`set-problems`), and
hear about edits (`edited` event). VS Code extensions themselves can't load here: they need VS
Code's JavaScript host and full system access, which is what this sandbox exists to avoid.

Writing one: see `examples/plugins/word-count`. It has a status item, a panel, a TODO tree
view and commands. Build it with `scripts/build-plugins.ps1`, which needs
`rustup target add wasm32-unknown-unknown`. Publish it with:

```
rcplugin keygen my.key
rcplugin publish examples/plugins/word-count <registry-folder> --key my.key --changelog "..."
```

### Language servers

Opening a file starts its language server for the file's project. Each server is found on
`PATH`, in the usual tool folders, or at `lsp.paths.<language>` (a program, optionally with
arguments):

| Language | Server | If it is missing, the bar offers to |
| --- | --- | --- |
| Rust | [rust-analyzer](https://rust-analyzer.github.io/) (`lsp.rust_analyzer_path` also works) | `rustup component add rust-analyzer` |
| Python | pyright (or basedpyright, pylsp) | `npm install -g pyright`, or `pip install --user python-lsp-server` without Node.js |
| JavaScript, TypeScript | typescript-language-server | `npm install -g typescript-language-server typescript@6` |
| C, C++ | clangd (also found in `C:\Program Files\LLVM`) | download the latest clangd release |
| Java | Eclipse JDT Language Server | download it; plus a Java 25 runtime (Eclipse Temurin) if Java 25+ isn't installed |
| C# | csharp-ls (or a downloaded OmniSharp) | `dotnet tool install --global csharp-ls` |
| Go | gopls | `go install golang.org/x/tools/gopls@latest` |

The bar says exactly what it will run or download, and nothing happens unless you click
**Install** (**Don't Ask Again** sets `lsp.ask_to_install = false`). Without the tools to
install a server (Node.js, the .NET SDK, Go), it says what to install. Downloaded servers go to
`<config>\lsp`. The status bar shows the active file's server.

| Feature | How |
| --- | --- |
| Errors and warnings | Squiggles, the message after the line (Error Lens style), hover for details; Problems panel (`Ctrl+Shift+M`), counts in the status bar; `F8` / `Shift+F8` next/previous problem |
| Completion | As you type, after `.`/`:`, or `Ctrl+Space`; `Enter`/`Tab` accepts, snippets place the cursor, auto-imports are added |
| Hover | Rest the pointer on a symbol, or `Ctrl+K Ctrl+I` at the cursor |
| Go to Definition / Implementations | `F12` or `Ctrl+Click` / `Ctrl+F12` |
| Go to Type Definition | **Go > Go to Type Definition**, or the command palette |
| Workspace symbols | `Ctrl+T`, or type `#` in the palette |
| Signature help | Shows after `(` and `,`; `Ctrl+Shift+Space` at the cursor |
| Inlay hints | Inferred types and parameter names inside the line (`editor.inlay_hints`) |
| Code lens | Reference counts and the like at the end of the line (`editor.code_lens`); **Show Code Lens on This Line** runs commands the server itself handles |
| Call and type hierarchy | **Show Incoming Calls** (`Shift+Alt+H`), **Outgoing Calls**, **Supertypes** and **Subtypes** list one level in the locations panel |
| Pull diagnostics | Servers that report problems on request, like kotlin-lsp, show them with the rest |
| Virtual documents | Go to definition reaches what only the server can read, like classes in a jar (`jdt://`, `workspace/textDocumentContent`), in a read-only tab kept under `<config>\virtual`. Hover, go to definition and references work inside it during the session, several results list together with real files, and going there again refreshes the text |
| Debugging | `F5` debugs Rust with CodeLLDB and Python with debugpy, from the project's environment, offering to install it first. `debugger` and `debug_type` in `languages.toml` add any adapter speaking DAP over stdio |
| REPL | `Ctrl+Enter` sends the selection, the `# %%` cell or the line to an interactive session in a terminal tab, then moves on, and shows the answer beside the code. Cells get a run button. Python uses the project's environment. Node, R and Julia are built in, and `repl` in `languages.toml` adds others |
| C and C++ flags | `C/C++: Show Compile Flags`, or the status bar, says what clangd reads for this file: the database, its compiler, standard, defines and includes, whether the file is listed or its flags are borrowed or guessed, and whether the database is older than `CMakeLists.txt`. Choose among databases clangd wouldn't find (`out/build/*`, `cmake-build-*`) and, in a Ninja Multi-Config database, a configuration (`cpp.compile_commands_dir`, `cpp.config`). With no database it gives the command that fits, knowing CMake's Visual Studio generators don't write one. `Alt+O` switches between header and source |
| Stack traces | `Stack Trace: Analyze` takes a trace pasted from a log, ticket or chat, or the selected text. It rejoins wrapped lines, opens up traces flattened into one line or escaped in JSON, drops timestamps and quoting, and links frames from Java, Kotlin, C#, Python, JavaScript, TypeScript, Rust, Go, C and C++, PHP, Ruby, Dart, Elixir and more to files in this project, even when the paths come from another machine. A frame matching only by file name is marked as a guess |
| Language profiles | A `languages.toml` entry can ask for rulers, dim columns, backslash symbols (`\\to` becomes `→` as you type, or on Tab), a bar of glyphs to click, and an include statement with search paths for `Go to Include at Cursor`. COBOL, fixed-form Fortran, Lean, Agda and APL ship with theirs. `Insert Symbol` finds any symbol by name in any file. `[symbols]` in settings adds your own names, `editor.rulers` draws rulers elsewhere, and `editor.include_paths` adds folders |
| CSV and TSV | Columns are colored in the text. `CSV: Toggle Table View`, or the status bar, shows a table that stays quick at a million rows: click a header to sort (numbers by value, gaps last), filter, drag column edges, move with the arrow keys, copy a cell, and double-click a row for its line in the text. It follows edits, finds the delimiter and says when rows are uneven or a quote never closes. The table only views: edit in the text |
| Tests in any language | The Testing view finds a project's runner and reads its results, not its printed text: `go test -json`, pytest, unittest, `node --test`, Vitest, Jest, `dotnet test`, Maven, Gradle and RSpec. Tests are listed before they run where their text shows them, with a run button and a pass or fail mark there and in the gutter. A failure shows its message, with links to the frames in this project. A project adds its own runner in `.rust-code/tests.toml`, naming a command and its `format` (`gojson`, `junit`, `trx`, `tap`, `unittest`, `rspec`, `jest`) |
| Large logs | Logs from 16 MB, and any file from 128 MB, open in a view that reads only the lines on screen, so size doesn't matter. `Log: Open in Log View` opens any file there. Filter by regex as you type (case, invert), follow as lines arrive, and double-click a filtered row for the lines around it. It starts over if the file is truncated or rotated, colors lines by level, and hands the selected line onward to `Stack Trace: Analyze`. It keeps reading behind other tabs; the status bar brings it back |
| Bytes | A binary file opens as hex instead of garbled text, and `Hex: Open File` shows any file that way, at any size, with a jump to an offset. While debugging, if the adapter offers them, the Debug view adds Memory and Disassembly: read memory at an address, an expression or a variable (`mem`), with bytes that changed since the last stop in red, and step by instruction with the current one highlighted |
| Coverage | Beside the code, bars show which lines ran, which only partly, and which never did. Reports are read from where tools leave them (LCOV, Cobertura, JaCoCo, coverage.py JSON, Go profiles), or with `Coverage: Load Report`, including ones written on another machine. The Testing view's Coverage button runs tests with it for `node --test`, pytest with pytest-cov, `dotnet test`, Go, Jest and Vitest. The COVERAGE tab lists files, least covered first. Marks hide once a file changes, as its line numbers no longer match. Plugins add other formats through `set-coverage` |
| At the cursor | `View: Show Cursor Info` opens a panel that follows the cursor with what the language server says about that spot and any problems there. In Lean and Coq it shows the goals too, with lines new since the last answer in green. Pause freezes it, and Pin keeps watching a place while you work elsewhere, asking again as its file changes |
| Structure and rules | In Lisp files, `Ctrl+Alt+Right` slurps and `Ctrl+Alt+Left` barfs, with `Shift` for the backward moves. `Shift+Alt+R` raises, `Shift+Alt+S` splices and `Shift+Alt+W` wraps, and the `Structure:` commands wrap in other brackets. Other languages raise and wrap through their syntax tree. `Search: Find by Syntax Tree` runs a tree-sitter query over the project. A project's own lint rules go in `.rust-code/rules.toml` (`id`, `language`, `query`, `message`, optional `severity` and `fix` using `$capture`), and reach Problems for the open file or with `Rules: Check Project`. `Rules: Apply Fixes in File` applies fixes. Rules see syntax only, not types |
| Lenses and hierarchies | Code lens titles beside the code can be clicked: rust-analyzer's Run, Run Test and implementation counts work here. Call and type hierarchies open as a tree in the References panel, a level at a time |
| Live sessions | `Preview: Constant Preview` keeps showing what the active file does: a page or stylesheet follows edits in your browser, Markdown shows its preview, and a program saves and runs again after each pause in typing, marking the lines of output that changed, or runs its tests if it's a library. Only in trusted folders, so anything the program does happens again on each run. `Live: Show Session` attaches to a running system on this computer: nREPL (reading `.nrepl-port` if there is one), a Python kernel (a new one, or an existing `kernel-*.json`; needs `jupyter_client` and `ipykernel`), Swank for Common Lisp, a BEAM node for Elixir and Erlang (needs Erlang on `PATH`), or RIDE for Dyalog APL. It says what it's attached to and what the server offers, and Interrupt only appears if the server can. In Clojure files, `Ctrl+Enter` sends the selection or the top-level form and shows the answer beside it, gone once the line changes, and slow code never holds up the editor. The inspector shows the namespace's values, in the same rows as the debugger's variables, opening maps and lists. New systems add a `Session` adapter |
| Web authoring | Tab grows Emmet abbreviations in HTML and CSS (`ul>li.item$*3`, `m10-20`); `Emmet: Expand Abbreviation` also works in JSX and PHP. `Color: Pick Color at Cursor` keeps a color's format and shows WCAG contrast and when it leaves sRGB or Display P3. Hover shows Baseline availability and selector specificity. `web.baseline` (`widely`, `newly` or a year, in `.rust-code/settings.toml`) flags what falls below a project's target. Image files open as images. HTML, CSS and JSON use `vscode-langservers-extracted` if it's on PATH. Format on save leaves a file alone if the project has Prettier, Biome or dprint, and the doctor names formatters that conflict. Baseline data comes from [web-features](https://github.com/web-platform-dx/web-features) (Apache-2.0), regenerated by `scripts/gen-baseline.py` |
| One Python | The terminal, Run and Python's language server all use the same Python: the project's `.venv`, `venv` or `env`, then `.python-version` with pyenv, `environment.yml` with conda, what was active at launch, then PATH. The status bar names it and why; click it to choose (`python.interpreter`) |
| Languages as data | Add a language with a few lines in `languages.toml` (**Language: Open languages.toml**): extensions, comment, server, root markers, run command. Thirteen ship ready, finding their servers on PATH. **Language: Doctor** shows for the active file what works, what doesn't and why |
| Live preview | Run on an HTML file (or **Preview: Open Live Preview**) shows it in your browser from a server on this computer. Edits appear as you type: stylesheets swap in place, anything else reloads. TypeScript compiles with no setup, `console` output lands in the **PAGE** panel, script errors in Problems, and `Alt`+click in the page jumps to the element's line |
| Problems from running | Errors printed by what Run and run configurations start land in Problems: gcc, clang, javac, Go, MSBuild, tsc and rustc, and crashes in Python, JavaScript, Java, .NET and Rust. `matcher` in `launch.toml` sets a pattern of your own. `Ctrl`+click opens a web address in a terminal |
| Servers over TCP | `[lsp.paths] go = "tcp://127.0.0.1:7000"` connects to a server already running on this computer. `.gd` files use Godot's editor on port 6005; start it first, or run **Language Servers: Restart** after. Other addresses are refused |
| Show document | A server can open a file or a page on this computer; other web addresses are not opened |
| Semantic highlighting | Colors from the server over the syntax colors (`editor.semantic_highlighting`) |
| Symbol highlight | Other uses of the symbol under the cursor are marked |
| Find References | `Shift+F12`, results in the References panel |
| Rename Symbol | `F2`; files that aren't open are opened with the changes unsaved, so you can review and undo |
| Code actions | `Ctrl+.` (quick fixes, refactors) |
| File icons | Every file type gets its own icon, from [Material Icon Theme](https://github.com/material-extensions/vscode-material-icon-theme) (MIT, license in `core/data/file_icons_LICENSE.txt`; `scripts/build_file_icons.py` builds the pack). Language logos remain their owners' trademarks. Test files get a flask on their icon |
| Islands | The window's panels are rounded islands on a darker canvas, in any color theme. `workbench.ui = "classic"` brings back square panels edge to edge |
| Top bar | In the island layout: every menu behind one button, the project with its own initials and color, the branch, what Run starts, Run, Debug, Go to File and Settings |
| Tool rails | In the island layout, icons on both edges of the window open tools on that side; click again to close. Drag an icon to the other rail to move its tool there: the panel's tools take the whole panel to the right. It's remembered with the session |
| Search Everywhere | Shift twice, or the search button, opens one box over files, symbols, commands, settings and text. Tab goes through them; Ctrl+P, Ctrl+Shift+P and the rest keep working. `workbench.double_shift` turns Shift twice off |
| Logs | Two files in `<config>/logs`. `rust-code.log` says what the app did: commands, files opened, clicks, status messages and each change to what's on screen, with times. `crash.log` holds each panic's place, message, backtrace and the last lines of the log, and notes a session that ended without closing. **Developer: Open Log** and **Developer: Open Crash Log** open them. They stay on this computer and never hold text you type |
| Motion | Panels slide, a line slides between tabs, and rail icons and popups ease in, in under a sixth of a second. `workbench.reduced_motion` makes it all immediate |
| File buttons | Beside the tabs, what the active file can do: Preview for Markdown and SVG, Table for CSV, Live Preview for HTML, Tree and Format for JSON, YAML and TOML, Scripts for `package.json`, Header/Source and Flags for C and C++, Log View, Run Tests and Coverage, REPL, and Hex for anything |
| Import from VS Code | **Extensions: Import from VS Code**, or the arrow in Extensions, takes themes, snippets and language names and comments from an installed extension, a `.vsix` or a folder. Extensions' code can't run here and their TextMate colors aren't read: the status bar says what stayed behind. What you already have stays as it is |
| Project tree | The Explorer reads like a JetBrains project tree, going by each language's conventions: a project root row, blue source roots, green test roots, resources with bars, packages with a dot and chains as one `org.example` row (`workbench.compact_folders`), orange build output, a blue square on modules. In every language a file named after the type it declares shows that type in a ring (C, I, E, R, A, @, T, O); Java, Kotlin, Scala, Groovy and C# also drop the extension. A green triangle marks where a program starts and a flask where tests are, in the tree and on tabs. The gutter gets a play button on `main` in some 25 languages. Completion rows use the same rings, from any language server. New (the + button or right click) offers 29 kinds of file, those already in the project first, filling in `package` where it is known. Web projects get the same: `app`, `pages`, `components` as source roots and `public` as resources beside a `package.json`, framework output in orange, rings for components, hooks, pages, routes and CSS Modules, and a file's tests, styles and built files nested under it (`workbench.nest_files`). The gutter gets a play button on each `package.json` script, on `<html>` and `<script>`, on `describe`, and on a page, which opens its address on the dev server, starting it if needed; stylesheets show their colors there. A running dev server's address appears in the status bar. New offers React, Vue, Svelte, Astro, CSS Module and test files according to the project's dependencies. Collapse All and Select Opened File |
| Other tabs | Log views (several at once, kept across restarts), the byte view and images each get a tab beside the files, and go on existing behind other tabs |
| Word colors | COBOL, Lean, Agda, Coq and APL have no grammar here, so they're colored by reading their comments, strings, numbers and keywords |
| SVG | `Preview: Constant Preview` on an `.svg` draws it beside its text, following edits. It reads only the text: no files or addresses the picture names |
| Debugging Go and JavaScript | `F5` on a `.go` file uses Delve (`dlv`, on `PATH` or in `go install`'s folder). On a `.js`, `.mjs` or `.cjs` file it uses js-debug on Node, offered as a small pinned download the first time. TypeScript isn't covered yet |
| Quick fixes | A light bulb in the margin of the cursor's line when the language server offers actions there: yellow for fixes, grey for refactorings. Click it or press `Ctrl+.` |
| Likely problems | Warns while you type about loops that look endless (Python, JS, TS, Java, C, C++, C#, Go, Rust, Zig) and memory that looks leaked (C, C++, Zig; `mem::forget` in Rust). C and C++ also use `clang-tidy`'s analyzer on save if it's on `PATH`, in trusted folders. These are likely findings, not proof: some are wrong and some leaks are missed. `editor.likely_problems = false` turns them off |
| Formatting | `Shift+Alt+F`; `editor.format_on_save = true` formats before saving |

Click the server's name in the status bar, or run **Language Servers: Show Status**, for its state,
how long it has been running, memory use, and each task with its elapsed time. **Restart** restarts
that server; rust-analyzer also gets **Reload Workspace**, **Rebuild Proc Macros**, **Run Check** and
**Cancel Check**, and the window shows the size of the build folder.

**Memory use** picks a preset (`lsp.rust_analyzer_memory`): `default`, `low` (fewer cached syntax
trees, no cache warm-up on load) or `minimal` (also no build scripts or proc macros, so code that
depends on them shows false errors). **Own build folder** (`lsp.rust_analyzer_separate_target`)
stops cargo waiting on rust-analyzer's checks, and costs disk because build output is duplicated
under `target/rust-analyzer`; it is off by default. Either change restarts rust-analyzer. When a cargo
job started here waits for the build folder, a bar says what holds it and offers both ways out.
Waits in the terminal aren't detected.

Edits are sent to the server as small incremental changes. **Language Servers: Restart**
restarts them (e.g. after changing `lsp.paths`); `lsp.enabled = false` turns language
servers off.

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
independently of Git, in `<config>\history` (50 versions or 30 days per file by
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
  (`<config>\settings.toml`) → workspace (`<folder>/.rust-code/settings.toml`).
  Files can `extends = "other.toml"` and have `[windows]` / `[macos]` / `[linux]` tables.
  Unknown keys and errors show in the status bar; saving a settings file applies it.
  Separate `editor.font_size` and `workbench.ui_font_size`, on top of `workbench.zoom`.
- **Keybindings** (`Ctrl+K Ctrl+S`): `<config>\keybindings.toml` with VS Code
  command IDs, two-key chords, `when = "editorTextFocus"`, and `-command` to remove a
  default. Menus and the palette show the active shortcuts.
- **Color themes** (`Ctrl+K Ctrl+T`): any VS Code theme. The picker lists themes from the
  installed VS Code (bundled and `~/.vscode/extensions`) and `<config>\themes`;
  `workbench.color_theme` also accepts a path to a theme `.json`. Theme files may use
  comments and `include`; token colors apply to tree-sitter and syntect highlighting.
