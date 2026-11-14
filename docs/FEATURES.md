# Features

Everything Coiman IDE does, area by area. The [README](../README.md) says how to build it.

- Integrated title bar, as in VS Code: menus, the window title and the window buttons share
  one bar. Drag it to move the window, double-click it to maximize, resize from any edge.
- File explorer (lazy folder tree), tabs with unsaved markers, breadcrumbs, file-type icons, and create, rename and delete in the Explorer (right-click; delete goes to the Recycle Bin). **New** makes files and folders in the selected folder, or next to the selected file.
- The Explorer works from the keyboard, with VS Code's keys. A click selects a row and opens a file
  while the keyboard stays in the Explorer; Ctrl+click and Shift+click pick more rows; a double
  click or **Enter** takes the file to the editor, and **Space** opens it and stays. Arrows move,
  Left and Right close and open folders or go up and in, Home, End and Page keys jump, and typing a
  name jumps to it. **F2** renames, **Delete** moves the selected rows to the Recycle Bin and
  **Shift+Delete** deletes them for good, each after asking (Enter confirms, Escape cancels).
  **Ctrl+C**, **Ctrl+X** and **Ctrl+V** copy, cut and paste files and folders ("a copy.txt" when the
  name is taken; open files follow a move), **Shift+Alt+C** copies paths, **Ctrl+A** selects all
  and Escape drops the selection and a pending cut. Drag a file or folder onto a folder to move
  it there, or onto the project row to bring it back to the root; open files follow. Conflicting
  names are left where they are, with the reason in the status bar. The open folder itself is
  never deleted, cut or
  renamed. These keys are commands in **Keyboard Shortcuts**, marked Explorer, and can be changed
  there or in the keybindings file with `when = "filesExplorerFocus"`.
- **Filter Files** (`Ctrl+Alt+F`, or the magnifier in the Explorer's header) filters the whole
  project, not only open folders: matches show as a tree, Enter opens the first, Down goes into
  them, Escape closes the filter. It searches an index built off the UI thread and rebuilt on
  Refresh.
- **Sort order** (Settings > Workbench) orders folders like VS Code's `explorer.sortOrder`: folders
  first, mixed, files first, by type or by date modified.
- Folders are read on their own thread, so a slow or huge folder never freezes the window; a
  refresh keeps what's shown until the new listing arrives. Rows out of sight are skipped, so a
  folder of 20,000 files scrolls like a small one.
- Each file dialog opens where it was last left (Open File, Open Folder, Save As for new files,
  background pictures, plugin folders and the rest each remember their own), kept in
  `dialogs.json` in the config folder.
- Split the editor with `Ctrl+\\` or by dragging a tab onto the right half of the editor; each pane
  scrolls on its own. `Ctrl+1` / `Ctrl+2` move between them, `Shift+Alt+0` flips the layout, the
  divider drags, and **View > Single Editor** goes back to one. Drag tabs to reorder them
- Bidi controls, invisible characters and Private Use Area characters show as a red `?`, with a
  warning above the file and **Next** to step through them, naming each one in the status bar. They
  stay in the file until you delete them (`editor.unicode_warnings`)
- Recent projects (folders) and recent files are kept apart, newest first, with when each was last
  opened: 20 projects and 30 files. `Ctrl+R` (**Open Recent**) lists both, then **Clear Recently
  Opened**; the × on a row, or Shift+Delete, removes it and the list stays open. The Welcome page
  shows a few of each with an × each, More and Clear; **File > Open Recent** has the same with
  their places, and the project menu (the folder name at the top) lists recent projects as the bar
  shows the open one: badge, name, a Git mark for a repository and an × that removes it while the
  menu stays open. The menu is as wide as the longest name. Then **Manage Recent**. A folder or file that's gone stays
  listed, struck through and marked "not found", since its drive may only be disconnected: opening
  it says so, and **File: Remove Missing from Recently Opened** drops all such entries. Clearing
  asks first. The list lives in `recent.json` in the config folder; the old folders-only list
  still reads.
- The status bar's indentation opens a picker: spaces, tabs, or detect from the content
- The view glides when keys move it (`editor.smooth_scrolling`); long jumps cut straight there
- Bookmarks: `Ctrl+Alt+K` toggles, `Ctrl+Alt+L` / `Ctrl+Alt+J` go next and previous, **Bookmarks: List
  All** covers every file. They follow their lines as you edit and survive a restart
- Your own snippets (**Snippets: Manage**): type a prefix and press Tab, or **Snippets: Insert
  Snippet**. Bodies use `$1`, `${1:text}` and `$0`, kept in `<config>/snippets.toml`. Only the first
  stop is selected; there is no tabbing between stops yet
- Terminal search (`Ctrl+F` in a terminal): scrollback included, across wrapped lines, with match
  case; `Enter` and `Shift+Enter` move through matches
- Markdown preview (`Ctrl+K V`, **View > Markdown Preview**) beside the editor, updating as you type. It reads CommonMark with GitHub's tables, task lists, strikethrough and footnotes ([pulldown-cmark](https://crates.io/crates/pulldown-cmark)). Code blocks are highlighted, and the common HTML tags (`<b>`, `<i>`, `<code>`, `<kbd>`, `<a>`, `<img>`, `<details>`) are shown. Pictures beside the file (PNG, JPEG, GIF, WebP, BMP, SVG) are drawn. Pictures on the web aren't downloaded: they show as a link. Links open in the browser, open the file they name here, or scroll to a `#heading`. ```mermaid blocks are drawn as diagrams (flowchart, class, sequence, state, ER, and the other kinds Mermaid knows) by [merman](https://crates.io/crates/merman), without a browser, in Mermaid's light or dark theme to match the editor; a diagram that can't be drawn shows its text and why. `.mmd` and `.mermaid` files preview as their diagram.
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
- Rainbow brackets by nesting depth, in every highlighted language (by syntax tree where there
  is one, else by the grammar's scopes, so brackets in strings and comments are left alone). A
  closing bracket that closes nothing, or the wrong kind, is red. Themes' `editorBracketHighlight`
  colors are used.
- Indent guides, with the guide of the block holding the cursor brightened (the block a line
  opens, when the cursor is on its first line), and a line in the bracket's own color down the
  bracket pair the cursor is in, from opener to closer. Line numbers, current-line and
  matching-bracket highlight. Matching, the pair guide, **Go to Bracket** and Vim's `%` pass over
  brackets in strings and comments, as the highlighting knows them.
- In HTML, XML, Vue, Svelte and JSX, the cursor in a tag's name frames that tag and its partner.
  Elements nest by name; comments, `<script>` and `<style>` text and attribute values are skipped;
  void and self-closing tags have none. Settings: **Rainbow brackets**, **Active indent guide**,
  **Bracket pair guide**, **Matching tags**.
- Multi-cursor: `Ctrl+D` adds the next match, `Ctrl+Alt+Up/Down` adds cursors, `Alt+Click`,
  `Esc` returns to one cursor; per-document undo/redo (`Ctrl+Z`, `Ctrl+Y`) grouped by word
- VS Code editing behaviour: auto-indent on Enter (splits `{|}`), smart Home, `Ctrl+L` line
  select, copy/cut whole line with no selection, `Ctrl+Enter` line below
- Quick Open (`Ctrl+P`): background index of the folder (respects `.gitignore`, skips
  `target`/`node_modules`/`.git`) ranked by nucleo, with matched letters highlighted;
  Command Palette (`Ctrl+Shift+P`), Go to Line (`Ctrl+G`)
- Find in file (`Ctrl+F`); Find in files (`Ctrl+Shift+F`) with the ripgrep crates: search as
  you type, results stream in grouped by file, match case / whole word / regex, files to
  include/exclude globs, binary files skipped; clicking a result selects the match.
  Replace in files (`Ctrl+Shift+H`, or **Replace…** in Search Everywhere, which brings what
  was typed) opens the same sheet with its replace row
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

## Source control (Git)

Uses the `git` command line, like VS Code. Fetch, pull, push, clone and publishing reach the
network, and only when you ask. All git work runs in the background.

- **Setting up**: Source Control says when Git isn't installed (with a link to git-scm.com). When
  Git doesn't know your name and email, a card says so and opens a form: they're what Git writes
  into each commit, set for all repositories (`git config --global`) or only this one, and the
  form explains that signing in to GitHub doesn't set them (GitHub links commits to your account
  by that email). Otherwise one line says who you commit as, with **Change**
  (**Git: Set Commit Author**).
- **Starting a repository**: a folder that isn't one offers **Initialize Repository** (`git init`,
  first branch `main`) and **Clone Repository...** (**Git: Clone**): a URL, then the folder it goes
  into; it opens when done.
- **Remotes**: listed under Remotes, each with an × to remove it; a form adds one (the first is
  `origin`). **Publish to GitHub** creates the repository on GitHub, adds it as `origin` and pushes,
  through the GitHub CLI (`gh`, signed in with `gh auth login`); without it, it says how else.
- Under the branch, one line says where it stands: up to date with `origin/main`, commits to push
  or pull, not on the remote yet (Push publishes it), or no remote yet.
- Git's errors come with what they mean and what to do: no author set, signing in, an SSH key not
  accepted, a branch with no upstream, a rejected push, a remote not found, no connection.

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

## Run, test and check (Cargo)

- **Run configurations** live in `.coiman/launch.toml` (commit it to share them):
  `name`, `command` (run through the shell, so pipes and `&&` work), optional `cwd` and
  `env`. Without the file, Coiman IDE offers what Cargo implies: run each binary, test,
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

## Indentation

Enter keeps the line's indentation and goes a level in after what opens a block in the file's
language:

- **Brackets:** `{`, `(` and `[` in C-like languages, CSS and JSON. `{|}` splits onto three lines.
- **Python:** `:`.
- **HTML and XML:** a start tag. `<p>|</p>` splits onto three lines.
- **Ruby:** `do`, `def`, `if` and the like.
- **Lua and shell:** `then` and `do`.
- **Elixir:** `do`.
- **YAML:** a key ending in `:`.

In Python, the line after `return`, `pass`, `break`, `continue` or `raise` comes out a level.

Typing these moves the line back to where it belongs:

- a closing bracket alone on its line, back to its opener's level
- `</` in markup
- Python's `else:`, `elif …:`, `except …:` and `finally:`
- Ruby's, Lua's, shell's and Elixir's `end`, `else`, `fi`, `done` and the like, once the word is finished or on Enter, so `ending` stays

Other behaviour:

- `/**` comments go on with ` * `.
- A line left blank keeps no spaces.
- Backspace in indentation made of spaces goes back to the tab stop before.
- Tabs or spaces, and their width, follow `.editorconfig`, then the file itself, then the settings.

## Tags and quotes

In HTML, XML and JSX, typing does part of the tag for you, as VS Code's `html.autoClosingTags`
and `html.autoCreateQuotes` do:

- `>` at the end of a start tag adds its end tag, with the cursor between them, and Enter there
  splits them onto three lines. Not after a void element such as `<br>`, a self-closing tag, a
  comment or `<!doctype>`.
- `</` finishes the end tag of the innermost element still open, and moves the line back if
  that's all it holds.
- `=` after an attribute's name adds `""` with the cursor inside, in HTML and XML. The closing
  quote is stepped over when you type it.

In JSX and TSX a tag only starts where an expression can, so `Array<string>` and
`useState<number>` are left alone. Nothing is completed inside `<script>` or `<style>`.
`editor.auto_close_tags` and `editor.auto_create_quotes` turn these off.

With no HTML language server, HTML, its templates and Vue still get suggestions as you type,
or with `Ctrl+Space`:

- element names after `<`
- the element's own attributes first, then global ones and events; an attribute that takes a
  value goes in as `name="|"`, and those already on the tag aren't offered
- values for attributes with a fixed set: `type`, `target`, `rel` (one word at a time),
  `method`, `loading`, `autocomplete`, `<meta name>` and others

In Vue files, the suggestions also include:

- Vue's directives (`v-if`, `v-for`, `v-model`…)
- `:` for bound attributes and `@` for events
- Vue's own components (`transition`, `keep-alive`…)
- `<script setup>`, `<style scoped>` and the other block attributes

Vue templates are checked like HTML with Vue's rules: any tag may close itself, `{{ }}` and
`<script setup>` are fine, and a `lang="pug"` template is left alone. Svelte components are
checked too, reading around `{#if}`, `{:else}`, `{/if}`, `{@html}` and `{expressions}`, with
`on:`, `bind:` and `class:` attributes and `<script context>` allowed.

## Colors

Colors written in a file are shown on themselves: the text of `#e53935`, `rgb(...)`, `hsl(...)`,
`oklch(...)` or `tomato` sits on that color, in black or white, whichever reads. That's white on
red, blue or black, and black on yellow, lime or white. A color too close to the editor's
background, such as black on a dark theme, gets a thin frame. A see-through color is shown as it
would look over the editor.

Where colors are found:

- **Style sheets** (CSS, SCSS, Sass, Less, Stylus): everywhere, names included.
- **Pages, Vue, Svelte and SVG:** in `<style>`, `style="..."`, color attributes such as `fill` and
  `stroke`, and anything written as a color.
- **Code** (JS, TS, JSON, YAML, TOML, Python, Go, Rust, C#, Java, Kotlin, Swift, Dart, Lua, Ruby):
  inside strings only, so `#cafe` in a comment or `#[derive]` stays plain. Flutter's
  `Color(0xFF42A5F5)` counts too.

`editor.color_decorators = false` turns this off. It replaces the Color Preview extension that
used to come with the app, which is removed if it was the bundled copy.

With a language server's semantic colors, kinds of type each have their own:

| Dark Modern | Kinds |
|---|---|
| Teal | Classes |
| Green | Structs |
| Pale green | Interfaces and traits, enums, type parameters |
| Light blue | Constants, read-only variables, enum members |

Deprecated code is struck through. Themes can set these themselves: a VS Code theme's
`semanticTokenColors` (`class`, `interface`, `*.deprecated`, `variable.readonly`…) are read. A
theme without them uses its TextMate colors for `entity.name.type.class`,
`entity.name.type.interface` and the like, falling back to types in general.

## Fonts

Font names in a font list are written in their own font: in `font-family: "Segoe UI", Georgia,
serif`, "Segoe UI" shows in Segoe UI and Georgia in Georgia, quotes included, in the code's
syntax color and size and on its baseline. Each name takes its own font's width and the rest of
the line closes up behind it, as text set in that font would; clicks, selections and marks follow.
A generic family shows in the font it
stands for here, such as `serif` in Times New Roman. A font that isn't installed stays as written,
so a name in plain code is one the page can't use on this machine. The line with the cursor shows
plain text, for editing.

Lists are found in style sheets, in `<style>` and `style="..."` in pages, Vue and Svelte, and in
`fontFamily` strings in JS and TS. Resting the pointer on a list opens a card with a sample line in
each font, marking the one a browser here would use and showing a missing one in its fallback.
**CSS: Preview Font Family** opens the card for the list under the cursor. Both are under
**Settings > Editor > Fonts in themselves**.

## Run Program

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

What Run starts is chosen in the title bar: **Current File**, or a configuration. Picking one with
`Ctrl+F5` chooses it too, and the status bar says so. **Run: Run Current File** (`Ctrl+Shift+F10`)
and the ▶ beside `main` always run the file in front, whatever is chosen. Every way of running
saves changed files first. Programs start in the folder the project is opened at.

## Workspace trust

Opening a folder runs nothing from it. Until you click **Trust Folder**, it stays in Restricted
Mode: you can read and edit files, but its `.coiman/settings.toml` is ignored, and language
servers, Git, Cargo, run configurations, Run Program and debugging are off. Trust covers
subfolders and is remembered outside the project. **Workspaces: Trust or Restrict This Folder**
changes your mind later. The built-in terminal is your own shell and stays available.

## C and C++ with MinGW (course projects)

Projects laid out as C courses give them (a `makefile`, MinGW on Windows, and VS Code's
`.vscode/launch.json` and `tasks.json`) run and debug as they do in VS Code:

- **Run** (`Ctrl+Shift+F10` or ▶) builds with the makefile, then runs the program it names
  (`-o prog`, with variables such as `$(EXECUTABLE)` read), in a terminal tab that takes
  input: `make; .\prog.exe`. `make` is used when there is one, else MinGW's `mingw32-make`,
  found on PATH or in `C:\MinGW\bin`, `C:\mingw32\bin`, `D:\mingw32\bin`,
  `C:\msys64\ucrt64\bin` and the like. A MinGW folder that isn't on PATH is added to the
  app's PATH when it starts, so make finds gcc and programs find MinGW's DLLs.
- **Debug** (`F5`) uses gdb's own debug adapter (gdb 14 or later). It builds first: as
  `.vscode/launch.json` says when it has a `cppdbg` launch (running its `preLaunchTask` from
  `tasks.json`, with Linux paths such as `/usr/bin/make` and `/usr/bin/gdb` replaced by the
  tools found here), else with the makefile's `debug` target, else with `gcc -g`. The
  program gets its own console window, for its input and output. Breakpoints, stepping,
  Locals, Watch (`str[i]`) and the call stack work as in VS Code; registers wait behind
  "show".
- **Doxygen**: `/**` and Enter above a function writes its comment, as VS Code's Doxygen
  Documentation Generator does: `@brief`, one `@param` per parameter, and `@return` unless it
  returns `void`. Above anything else, `@brief`; elsewhere `/**` is closed with ` */`. For C,
  C++, Java, JavaScript, TypeScript, C#, PHP, Kotlin, Swift, Scala, Groovy and Dart
  (**Settings > Editor > Doc comment templates**).

## First steps, search and frame rate

- **Help: Take the Tour** shows where things are, one part of the window at a time, with a
  button to try each: projects, the editor, search and commands, run and debug, the
  terminal, language tooling, Git, settings and extensions. The Welcome page offers it
  until it's been taken. **Help: New Sample Project...** makes a small Python, C, Java,
  JavaScript or Rust project and opens it, ready to run with Ctrl+Shift+F10.
- **Search in the top bar**: the box in the middle opens Search Everywhere (or press Shift
  twice). `workbench.search_in_top_bar = false` puts a magnifier on the right instead. It is
  the one search button: the dock has none.
- **File Manager** (**View: Open File Manager**, or **Open in File Manager** on a folder in
  the Explorer): folders in place of the editor, in one pane or two, each pane with tabs.
  Beside them are the project, the home folder and its usual folders, the drives, and
  bookmarks. A preview shows the picked file's details and first lines.
  - Columns sort by name (file2 before file10), size, date or type, folders first. A filter
    takes words or patterns such as `*.rs`, and hidden files show on request.
  - Click, Ctrl+click and Shift+click (or Shift+arrows) pick several. Drag them onto a
    folder to move them, holding Ctrl to copy. Drags work between the panes and the Explorer
    both ways, and the clipboard is the Explorer's too.
  - Keys: Enter opens, Backspace goes up, Alt+Left and Alt+Right go back and forward, F2
    renames, F5 and F6 copy and move to the other pane, F7 makes a folder, Delete goes to
    the Recycle Bin (Shift+Delete deletes for good, after asking), Tab changes pane, Ctrl+T
    opens a tab.
  - Open files follow what moves or is renamed, and unsaved work is saved before a delete.
  - Bookmarks, the panes' folders and tabs are kept between runs.
  - **Open File** and **Open Folder** choose with the File Manager instead of the system's
    dialog. A bar says what is being chosen, with Open and Cancel; a double-click on a file
    opens it. An Android build keeps Android's picker, which is what grants it access.
  - On Android, the File Manager also lists the phone's storage (once access is granted) and
    Termux's home folder, when they can be read.
- **Touch sizes** (`workbench.touch`): larger buttons, rows and scroll bars for fingers. Off
  unless turned on; Auto turns them on on Android.
- **Moving the panel and side bar**: the ≡ button on the panel's header puts the panel at the
  bottom, at the right or in a window of its own over the editor. Drag that window by its
  header to the editor's bottom or right edge and it docks there again. The same menu moves
  the side bar to the other side of the editor (in the Islands layout, drag a tool's icon to
  the other rail instead). Positions, the sizes edges are dragged to, and a pinned side bar
  are kept with the project. **View: Reset Layout** puts everything back as it started.
- **Frame rate**: nothing is drawn while nothing changes. While something does, frames
  keep to the refresh rate of the display the window is on; `workbench.max_fps` sets a
  lower limit to save power (0 follows the display).

## Servers from VS Code extensions, paths and tag pairs

- **ESLint and Tailwind CSS** run beside a file's own language server, from their installed
  VS Code extensions (`dbaeumer.vscode-eslint`, `bradlc.vscode-tailwindcss`): no copy of
  their own is needed, only Node.js. They start for projects that use the tool (an ESLint
  configuration, `tailwindcss` in `package.json` or a Tailwind configuration), in trusted
  folders, since they run the project's own configuration. ESLint's problems show with the
  rest and its fixes are on `Ctrl+.`; Tailwind completes class names (with their colors),
  and reports its problems. `lsp.extension_servers = false` turns them off. The Language
  Servers window lists them.
- **Paths complete in strings**: in `"./`, `src="`, `href="`, `url(`, imports and `#include`,
  the folder's files and folders are offered, folders first; picking a folder lists
  what's in it. `/` starts at the open folder. `editor.path_completion` turns it off.
- **Tags rename in pairs**: typing in a tag's name changes its partner too, in HTML, XML,
  JSX and Vue, and one undo takes both back. `editor.rename_tags` turns it off.

## Code generation and NULL checks

Ctrl+. (or the light bulb) offers code made here, with no language server needed, after
any the server offers:

- **Java**, in a class: Generate Getters and Setters (or just one kind), a Constructor,
  `toString()`, and `equals()` with `hashCode()`. Only what's missing is offered.
- **C#**, in a class or struct: Generate Properties for fields, a Constructor,
  `ToString()`, and `Equals()` with `GetHashCode()`.
- **C and C++**: a pointer from `malloc`, `fopen`, `strchr` and the like, read before it's
  checked, gets a note on the line, and the yellow bulb offers to add the check after
  the line that set it.

Nothing is changed until an action is picked.

## Maven

A folder with a `pom.xml` gets a MAVEN section in the Explorer, as the VS Code Maven
extension has it:

- **Projects and modules**, each with its **Lifecycle** (click a phase, or its ▶, to run
  `mvn <phase>` in a terminal tab), **Plugins** with their goals (goals Coiman IDE doesn't
  know are read with `mvn help:describe`), and **Dependencies** as a tree.
- The tree is read from the local repository (`~/.m2/repository`, or `<localRepository>`
  in `settings.xml`) the way Maven resolves it: nearest wins, scopes, exclusions, parents,
  imported BOMs, profiles for this OS and JDK, classifiers such as JavaFX's `win` jars.
  Conflicts and what isn't downloaded yet are marked. **Resolve Dependencies** (right
  click) has Maven download what's missing and shows its own `dependency:tree`.
- **Maven: Run Goal...**, **Custom Goal...** (with a history), **Show Effective POM**,
  **Add Dependency...** (searches Maven Central, asks the scope, writes it into the pom as
  one undo step and saves), **Resolve Dependencies**, **New Project from Archetype...**
  (quickstart, webapp or simple; inside a parent with `<packaging>pom</packaging>` it's
  added as a module), **Refresh** and **View: Show Maven**, in the palette.
- poms are read again when they change, and the Java language server is told.
- The project's `mvnw` is used first, then `mvn` from PATH. Without either, Coiman IDE
  offers to download Apache Maven 3.9.16 (9 MB, checked against its SHA-512) into its
  config folder. Maven needs Java.

## Debugging (CodeLLDB)

Coiman IDE debugs through [CodeLLDB](https://github.com/vadimcn/codelldb) over the Debug
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
- **Function breakpoints**: type a function's name under BREAKPOINTS (Enter) to pause when it
  is entered, with debuggers that take them (gdb, CodeLLDB, debugpy, Delve, js-debug).
- **Exception breakpoints**: the EXCEPTIONS list shows what the debugger offers, such as gdb's
  C++ exceptions when thrown, re-thrown or caught, or debugpy's raised and uncaught exceptions.
  Choices are kept for the next session.
- **Data breakpoints**: while paused, **on change** beside a variable pauses when it is written.
  CodeLLDB uses data breakpoints; gdb, whose adapter has none, uses its own watchpoint on the
  variable's memory (`watch -location`), so only the names at the top of a scope offer it there.
  They last for the session.
- **Export and import**: **Debug: Export Breakpoints...** saves line, function and exception
  breakpoints to a JSON file (paths inside the project are relative, so the file travels with
  it); **Import Breakpoints...** adds a file's breakpoints to those set. Both are under
  BREAKPOINTS too. Without exporting, each project's line, function and exception breakpoints
  are kept between runs of the app.
- **Attach to a process**: **Debug: Attach to Process...** (also in the Run menu) lists what's
  running, then asks which debugger: gdb, CodeLLDB, debugpy, js-debug or Delve, the likeliest
  first (debugpy for `python.exe`, js-debug for `node.exe`). Stop detaches and leaves the
  program running. Attaching debugpy to a Python without it offers to install it first.
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

## Plugins (WebAssembly)

Plugins are WebAssembly components written against `crates/coiman-core/wit/plugin.wit` and run in
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
- **SCS Units** (optional, not installed by default; **Plugins: Install from Folder...** on
  `plugins/built-in/scs-sii`): Euro Truck Simulator 2 and American Truck Simulator `.sii` files get colors, a
  **SII Units** list in the Explorer, and checks for unbalanced braces, unclosed strings, duplicate
  unit names, malformed lines and a missing `SiiNunit`.

Plugins can color text (`set-decorations`) and report in the Problems panel (`set-problems`), and
hear about edits (`edited` event). VS Code extensions themselves can't load here: they need VS
Code's JavaScript host and full system access, which is what this sandbox exists to avoid.

Writing one: see `plugins/examples/word-count`. It has a status item, a panel, a TODO tree
view and commands. Build it with `scripts/build-plugins.ps1`, which needs
`rustup target add wasm32-unknown-unknown`. Publish it with:

```
coiman-plugin keygen my.key
coiman-plugin publish plugins/examples/word-count <registry-folder> --key my.key --changelog "..."
```

## Language servers

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
| Completion | As you type, after `.`/`:`, or `Ctrl+Space`; `Enter`/`Tab` accepts, snippets place the cursor. The picked entry's signature and docs show beside the list (to its left when there's no room). Completions of names not yet imported (rust-analyzer's `HashMap` from anywhere) add the `use` line when taken: the server fills an entry in on request (`completionItem/resolve`), even after the list was fetched again as you typed |
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
| Tests in any language | The Testing view finds a project's runner and reads its results, not its printed text: `go test -json`, pytest, unittest, `node --test`, Vitest, Jest, `dotnet test`, Maven, Gradle and RSpec. Tests are listed before they run where their text shows them, with a run button and a pass or fail mark there and in the gutter. A failure shows its message, with links to the frames in this project. A project adds its own runner in `.coiman/tests.toml`, naming a command and its `format` (`gojson`, `junit`, `trx`, `tap`, `unittest`, `rspec`, `jest`) |
| Large logs | Logs from 16 MB, and any file from 128 MB, open in a view that reads only the lines on screen, so size doesn't matter. `Log: Open in Log View` opens any file there. Filter by regex as you type (case, invert), follow as lines arrive, and double-click a filtered row for the lines around it. It starts over if the file is truncated or rotated, colors lines by level, and hands the selected line onward to `Stack Trace: Analyze`. It keeps reading behind other tabs; the status bar brings it back |
| Bytes | A binary file opens as hex instead of garbled text, and `Hex: Open File` shows any file that way, at any size, with a jump to an offset. While debugging, if the adapter offers them, the Debug view adds Memory and Disassembly: read memory at an address, an expression or a variable (`mem`), with bytes that changed since the last stop in red, and step by instruction with the current one highlighted |
| Coverage | Beside the code, bars show which lines ran, which only partly, and which never did. Reports are read from where tools leave them (LCOV, Cobertura, JaCoCo, coverage.py JSON, Go profiles), or with `Coverage: Load Report`, including ones written on another machine. The Testing view's Coverage button runs tests with it for `node --test`, pytest with pytest-cov, `dotnet test`, Go, Jest and Vitest. The COVERAGE tab lists files, least covered first. Marks hide once a file changes, as its line numbers no longer match. Plugins add other formats through `set-coverage` |
| At the cursor | `View: Show Cursor Info` opens a panel that follows the cursor with what the language server says about that spot and any problems there. In Lean and Coq it shows the goals too, with lines new since the last answer in green. Pause freezes it, and Pin keeps watching a place while you work elsewhere, asking again as its file changes |
| Structure and rules | In Lisp files, `Ctrl+Alt+Right` slurps and `Ctrl+Alt+Left` barfs, with `Shift` for the backward moves. `Shift+Alt+R` raises, `Shift+Alt+S` splices and `Shift+Alt+W` wraps, and the `Structure:` commands wrap in other brackets. Other languages raise and wrap through their syntax tree. `Search: Find by Syntax Tree` runs a tree-sitter query over the project. A project's own lint rules go in `.coiman/rules.toml` (`id`, `language`, `query`, `message`, optional `severity` and `fix` using `$capture`), and reach Problems for the open file or with `Rules: Check Project`. `Rules: Apply Fixes in File` applies fixes. Rules see syntax only, not types |
| Lenses and hierarchies | Code lens titles beside the code can be clicked: rust-analyzer's Run, Run Test and implementation counts work here. Call and type hierarchies open as a tree in the References panel, a level at a time |
| Live sessions | `Preview: Constant Preview` keeps showing what the active file does: a page or stylesheet follows edits in your browser, Markdown shows its preview, and a program saves and runs again after each pause in typing, marking the lines of output that changed, or runs its tests if it's a library. Only in trusted folders, so anything the program does happens again on each run. `Live: Show Session` attaches to a running system on this computer: nREPL (reading `.nrepl-port` if there is one), a Python kernel (a new one, or an existing `kernel-*.json`; needs `jupyter_client` and `ipykernel`), Swank for Common Lisp, a BEAM node for Elixir and Erlang (needs Erlang on `PATH`), or RIDE for Dyalog APL. It says what it's attached to and what the server offers, and Interrupt only appears if the server can. In Clojure files, `Ctrl+Enter` sends the selection or the top-level form and shows the answer beside it, gone once the line changes, and slow code never holds up the editor. The inspector shows the namespace's values, in the same rows as the debugger's variables, opening maps and lists. New systems add a `Session` adapter |
| Web authoring | Tab grows Emmet abbreviations in HTML and CSS (`ul>li.item$*3`, `m10-20`); `Emmet: Expand Abbreviation` also works in JSX and PHP. `Color: Pick Color at Cursor` keeps a color's format and shows WCAG contrast and when it leaves sRGB or Display P3. Hover shows Baseline availability and selector specificity. `web.baseline` (`widely`, `newly` or a year, in `.coiman/settings.toml`) flags what falls below a project's target. Image files (PNG, JPEG, GIF, WebP, BMP, ICO, TIFF) open in a viewer. It fits the picture or zooms it: `+`, `-`, `1` for actual size and `0` to fit, Ctrl and the wheel or a pinch about the pointer, a double click to fit. Drag or the wheel moves it. It's smooth when smaller and shows crisp pixels when zoomed in, with a checkerboard behind transparency. Photos are turned the way their EXIF says. GIFs and WebPs animate. A picture wider than the graphics card holds is shown smaller, and says so. A broken file says what is wrong, and AVIF, HEIC, JPEG XL and PSD say they can't be shown. HTML, CSS and JSON use `vscode-langservers-extracted` if it's on PATH. Format on save leaves a file alone if the project has Prettier, Biome or dprint, and the doctor names formatters that conflict. Baseline data comes from [web-features](https://github.com/web-platform-dx/web-features) (Apache-2.0), regenerated by `scripts/gen-baseline.py` |
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
| Logs | Two files in `<config>/logs`. `coiman-ide.log` says what the app did: commands, files opened, clicks, status messages and each change to what's on screen, with times. `crash.log` holds each panic's place, message, backtrace and the last lines of the log, and notes a session that ended without closing. **Developer: Open Log** and **Developer: Open Crash Log** open them. They stay on this computer and never hold text you type |
| Motion | Panels slide, a line slides between tabs, and rail icons and popups ease in, in under a sixth of a second. `workbench.reduced_motion` makes it all immediate |
| File buttons | Beside the tabs, what the active file can do: Preview for Markdown and SVG, Table for CSV, Live Preview for HTML, Tree and Format for JSON, YAML and TOML, Scripts for `package.json`, Header/Source and Flags for C and C++, Log View, Run Tests and Coverage, REPL, and Hex for anything |
| Import from VS Code | **Extensions: Import from VS Code**, or the arrow in Extensions, takes themes, snippets and language names and comments from an installed extension, a `.vsix` or a folder. **Extensions: VS Code Compatibility Report**, or the check mark beside the arrow, lists every installed extension: built in here (and by what), data that imports, a pack, or code that can't run (with the VS Code APIs it calls and what's similar here). Extensions' code can't run here and their TextMate colors aren't read: the status bar says what stayed behind. What you already have stays as it is |
| Background image | **Background: Choose Image** or **Choose Folder of Images** lays a picture faintly over the window or just the editor, without patching anything. The editor, side bars and panel can have their own pictures over it. Set its strength with `workbench.background_opacity`, a folder's rotation with `workbench.background_interval`, and `workbench.background_random` for no order |
| Theme schedule | `workbench.theme_schedule = "sunrise Light Modern; 19:00 Dark Modern"` changes theme by this computer's clock. `sunrise` and `sunset` are worked out here from `workbench.theme_location`, which you type in; nothing looks up where you are. A theme chosen by hand stays until the next time on the list |
| Project tree | The Explorer reads like a JetBrains project tree, going by each language's conventions: a project root row, blue source roots, green test roots, resources with bars, packages with a dot and chains as one `org.example` row (`workbench.compact_folders`), orange build output, a blue square on modules. In every language a file named after the type it declares shows that type in a ring (C, I, E, R, A, @, T, O); Java, Kotlin, Scala, Groovy and C# also drop the extension. A green triangle marks where a program starts and a flask where tests are, in the tree and on tabs. The gutter gets a play button on `main` in some 25 languages. Completion rows use the same rings, from any language server. New (the + button or right click) offers 29 kinds of file, those already in the project first, filling in `package` where it is known. Web projects get the same: `app`, `pages`, `components` as source roots and `public` as resources beside a `package.json`, framework output in orange, rings for components, hooks, pages, routes and CSS Modules, and a file's tests, styles and built files nested under it (`workbench.nest_files`). The gutter gets a play button on each `package.json` script, on `<html>` and `<script>`, on `describe`, and on a page, which opens its address on the dev server, starting it if needed; stylesheets show their colors there. A running dev server's address appears in the status bar. New offers React, Vue, Svelte, Astro, CSS Module and test files according to the project's dependencies. Collapse All and Select Opened File |
| Other tabs | Log views (several at once, kept across restarts), the byte view and images each get a tab beside the files, and go on existing behind other tabs |
| Word colors | COBOL, Lean, Agda, Coq and APL have no grammar here, so they're colored by reading their comments, strings, numbers and keywords |
| SVG | `Preview: Constant Preview` on an `.svg` draws it beside its text, following edits. It reads only the text: no files or addresses the picture names |
| Debugging Go and JavaScript | `F5` on a `.go` file uses Delve (`dlv`, on `PATH` or in `go install`'s folder). On a `.js`, `.mjs` or `.cjs` file it uses js-debug on Node, offered as a small pinned download the first time. TypeScript isn't covered yet |
| Quick fixes | A light bulb in the margin of the cursor's line when the language server offers actions there: yellow for fixes, grey for refactorings. Click it or press `Ctrl+.` |
| Likely problems | Warns while you type about loops that look endless (Python, JS, TS, Java, C, C++, C#, Go, Rust, Zig) and memory that looks leaked (C, C++, Zig; `mem::forget` in Rust). C and C++ also use `clang-tidy`'s analyzer on save if it's on `PATH`, in trusted folders. These are likely findings, not proof: some are wrong and some leaks are missed. `editor.likely_problems = false` turns them off |
| Markup problems | HTML is checked while you type, with no language server. That covers `.html`, `.htm` and `.xhtml`, and Jinja, Nunjucks, Handlebars, Mustache, EJS, Twig and Liquid templates. It finds tags never closed, closed in the wrong order or never opened; broken tags such as an unclosed quote; a repeated attribute; `<div/>`, which HTML doesn't close; and attributes an element doesn't have, from the HTML standard's list. `data-*`, `aria-*`, event handlers and framework attributes (Vue, Angular, Alpine, htmx, Svelte) are left alone, and obsolete ones such as `bgcolor` are noted. Tags HTML closes by itself (`<li>`, `<p>`, `<td>` and others) are noted, not called errors. Template tags (`{% %}`, `{{ }}`, `<% %>`, `<?php ?>`) are read around. Where a template has branches, which can open and close tags, a tag that looks unbalanced is only noted. XML files (`.xml`, `.svg`, `.csproj`, `.xaml` and others) report every mismatched tag and repeated attribute, and read on past a syntax error where they can. They show in the editor and in Problems. `editor.markup_problems = false` turns this off |
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

## Vim and Helix modes

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

The engine (`crates/coiman-core/src/vim.rs`) is checked by golden files (`crates/coiman-core/tests/golden/`): every Vim
case is also run in real Vim (`vim -u NONE`, found on `PATH`, in Git for Windows, or at
`COIMAN_VIM`) and the test fails if Vim disagrees.

## Local history

A copy of each file is kept every time it is saved and when another program changes it,
independently of Git, in `<config>\history` (50 versions or 30 days per file by
default; `history.enabled`, `history.max_entries`, `history.max_days`, `history.max_file_kb`).

- The **Timeline** section of the Explorer lists the active file's versions ("Saved · 3
  minutes ago"). Click one to compare it side by side with the current text and **Revert**
  single changes, or **Restore File** to bring the whole version back.
- **Local History: Find Entry to Restore...** picks a version from the palette.
- Restoring is an unsaved, undoable edit, and the text it replaces is kept as a "Before
  restore" version.

## Architecture

**Architecture: Show Dependency Graph** reads which file imports which, for Rust, Python,
JavaScript and TypeScript, Java, C#, Go, and C and C++. It reads the import lines and finds
them among the project's files, without a build or a language server, so a project of a few
hundred files takes under a second. Libraries from outside the project are left out.

- **Graph:** files, or folders (with **Folders**), in layers: what a file uses sits below it.
  Imports in a cycle are red, test files green. Drag to move, scroll to zoom, **Fit** to see it
  all. A click picks a file: what it uses lights blue and what uses it amber. A double click
  opens the file.
- **Cycles:** groups of files that import each other round in a circle, largest first.
- **Most used:** files by how many others import them, with how many they import.
- **Unused:** files nothing imports and that import nothing. Entry points and scripts show
  here too.
- **Impact:** what a change to the picked file reaches, the files that import it and those that
  reach it through others, and the tests among them. **Architecture: Impact of Current File**
  opens it on the file in front.
- **Export SVG** saves the picture; **Copy Mermaid** copies it as a Mermaid flowchart for
  Markdown and wikis, and **Preview Mermaid** shows that diagram in a Markdown draft. Files of
  one folder share a subgraph, tests are green, imports in a cycle are red, and past Mermaid's
  500-arrow limit the folders are exported instead, which stays readable.

Rust: `mod x;` means a file contains another, not that it uses it, so it makes no edge.
Paths through `crate::`, `super::`, `self::`, other crates of the workspace and child modules
count. Python: absolute, relative and `from x import y` of a module. JavaScript and TypeScript:
relative paths, `@/` and `~/` for `src`, `index` files, `require` and dynamic `import()`.
Java: imports, wildcards, static imports and classes of the same package. C#: types through
`using` and the namespace. Go: packages of the module in `go.mod`. C and C++: `#include` of the
project's own headers.

## Run history

**Testing: Show Run History** keeps every test run, build, check and program started with Run,
per project and between sessions (the last 100 of each).

- **Test runs:** newest first, with what passed, failed and was skipped, and how long the run
  took. A picked run shows what changed since the run before by the same runner: tests that
  broke, were fixed, got slower (half as long again, and 100 ms more), are new or are gone. Its
  failures show with their message, and how many runs in a row each has failed. **Run the
  failed ones again** runs just those Cargo tests (`cargo test -- … --exact`, or nextest's
  filter).
- **Flaky:** tests that both passed and failed in their last 10 runs, those that flip most
  first, each run a dot.
- **Builds:** `cargo check` and `cargo clippy`, and programs started with Run, with how long
  they took, their exit code, the errors and warnings their output reported, and the first
  error, which opens the file there.

Cargo's tests and every Testing runner (pytest, Go, Jest and the others) are recorded.

## Code health

**Code Health: Secrets, Complexity, Duplicates, Licences and Audits** reads the project's files
on its own thread, Git-ignored ones and folders such as `node_modules`, `target` and `dist`
left out. A click on a place opens the file there.

- **Secrets:** keys and tokens by their shape: private keys, AWS, GitHub, GitLab, Slack,
  Stripe, Google, npm, OpenAI-style keys, JSON Web Tokens and passwords in URLs. It also
  finds quoted passwords and keys assigned in code that look random enough to be real, and
  unquoted ones in settings files (`.env`, `.ini`, `.properties`, YAML).
  - Values from the environment and placeholders (`${KEY}`, `changeme`, `<token>`) aren't
    reported.
  - A line marked `gitleaks:allow`, `pragma: allowlist secret` or `not-a-secret` is skipped.
  - What's found shows masked: only its first four characters.
- **Complexity:** each function's cyclomatic complexity and length, through tree-sitter, for
  Rust, Python, JavaScript, TypeScript, Java, C, C++, C# and Go. It shows the 200 most
  complex. Up to 10 is plain, to 20 complex, above that very complex.
- **Duplicates:** code written twice: runs of 6 lines or more that appear again, in another
  file or the same one, whitespace aside. Blank lines, comments, imports and lines of only
  brackets don't count.
- **Licences:** each dependency's licence, from `cargo metadata --offline` and npm's
  `node_modules`. Each is rated permissive, weak copyleft (MPL, LGPL, EPL), copyleft (GPL,
  AGPL) or unknown, reading SPDX expressions with `OR` and `AND`.
- **Audits:** **Run the audits** runs `cargo audit` beside a `Cargo.lock` and `npm audit`
  beside a `package-lock.json`, most severe first, with the fix and a link to each advisory.
  These tools fetch the lists of known vulnerabilities themselves. When one isn't installed,
  the tab says how to get it.

In Restricted Mode, Cargo and npm don't run (they read the project's settings), so licences
come from `node_modules` alone and the audits wait for the folder to be trusted.

## Databases (SQLite)

A SQLite file opens in the **Database** window from the Explorer or Quick Open (it is
recognised by its contents, whatever its name), or with **Database: Open SQLite File...**.

- **Tables and views** on the left. A click shows the columns, with the primary key marked,
  types, `not null`, where a foreign key points (`→ customer.id`), and the table's indexes
  (unique, partial, or made by a constraint). A double click shows the first 500 rows.
- **SQL** in the box, one statement or several; **Run** or Ctrl+Enter. Each statement that
  returns rows gets its own result, in a grid that only draws what's in view, with NULL
  greyed. Statements that change rows say how many. The list of tables refreshes after a
  change, and **History** keeps the last 30 runs.
- **Explain** (Ctrl+Shift+Enter) shows the **Query plan**: how SQLite would carry out each
  statement (`EXPLAIN QUERY PLAN`), as a tree. Steps that read a whole table or sort in a
  temporary table are marked, since an index could spare them. Nothing runs: the database
  is opened read-only for it.
- **Diagram**: the tables as boxes with their columns (primary and foreign keys marked) and
  a line from each foreign key to the table it points to. Tables pointed to sit to the
  left; hovering a table lights its links, and a double click shows its rows. **Copy as
  Mermaid** and **Preview as Mermaid** give the same tables as an `erDiagram`.
- **Database: Run SQL (Selection or File)** runs the selection, or the whole `.sql` file in
  front, against the open database. **Database: Explain Query Plan (Selection or File)**
  explains it instead, and **Database: Show Tables Diagram (ER)** opens the diagram.

It runs through the `sqlite3` program (on `PATH`, or MSYS2's), in its safe mode, so a
`.sql` file can't reach the shell with dot-commands. No database code is built into the app,
and it opens no connections of its own. Results keep 10,000 rows each.

## Code structure

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

## VS Code editor actions

Command IDs, labels and default keybindings were taken from VS Code 1.143.1's
shipped `workbench.desktop.main.js` and `nls.messages.json`.

| Command | Keys |
| --- | --- |
| Toggle Line Comment | `Ctrl+/` |
| Move Line Up / Down | `Alt+Up` / `Alt+Down` |
| Copy Line Up / Down | `Shift+Alt+Up` / `Shift+Alt+Down` |
| Delete Line | `Ctrl+Shift+K` |
| Indent / Outdent Line | `Ctrl+]` / `Ctrl+[` (also `Tab` / `Shift+Tab` on a line or a selection) |
| Add Selection to Next Find Match (single selection) | `Ctrl+D` |
| Go to Bracket | `Ctrl+Shift+\` |
| Toggle Word Wrap | `Alt+Z` |
| Toggle Zen Mode | `Ctrl+K Z` (`Esc` exits) |
| Reopen Closed Editor | `Ctrl+Shift+T` |

Palette only: Join Lines, Duplicate Selection, Sort Lines Ascending/Descending,
Delete Duplicate Lines, Reverse Lines, Transform to Uppercase/Lowercase/Title Case.

## Settings, keybindings and themes

- **Settings** (`Ctrl+,`): TOML, layered defaults → user
  (`<config>\settings.toml`) → workspace (`<folder>/.coiman/settings.toml`).
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
