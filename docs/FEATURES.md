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
- The view glides when the wheel or keys move it (`editor.smooth_scrolling`, Settings > Smooth
  scrolling); off, the wheel and keys move it at once, as VS Code does. Long jumps cut straight there
- **Minimap** (`editor.minimap`, off by default, or **View: Toggle Minimap**): the whole file
  drawn small, in its syntax colors, in place of the scrollbar, the part in view under a
  lighter band. Click it to
  bring that part to the middle; drag to scroll. A long file's minimap moves with the view
- **Right-click menu** in the text: Go to Definition, Type Definition, Implementations and
  References; Rename Symbol, Quick Fix, Format Document; Undo, Redo; Cut, Copy, Paste; Toggle Line
  Comment, Select All Occurrences; the Command Palette. A right click outside the selection
  moves the cursor there first, as in VS Code; inside it, the menu acts on the selection
- Bookmarks: `Ctrl+Alt+K` toggles, `Ctrl+Alt+L` / `Ctrl+Alt+J` go next and previous, **Bookmarks: List
  All** covers every file. They follow their lines as you edit and survive a restart
- Your own snippets (**Snippets: Manage**): type a prefix and press Tab, or **Snippets: Insert
  Snippet**. Bodies use `$1`, `${1:text}` and `$0`, kept in `<config>/snippets.toml`. Only the first
  stop is selected; there is no tabbing between stops yet
- Terminal search (`Ctrl+F` in a terminal): scrollback included, across wrapped lines, with match
  case; `Enter` and `Shift+Enter` move through matches
- Markdown preview (`Ctrl+K V`, **View > Markdown Preview**) beside the editor, updating as you type. It reads CommonMark with GitHub's tables, task lists, strikethrough and footnotes ([pulldown-cmark](https://crates.io/crates/pulldown-cmark)). Code blocks are highlighted, and the common HTML tags (`<b>`, `<i>`, `<code>`, `<kbd>`, `<a>`, `<img>`, `<details>`) are shown. Pictures beside the file (PNG, JPEG, GIF, WebP, BMP, SVG) are drawn. Pictures on the web aren't downloaded: they show as a link. Links open in the browser, open the file they name here, or scroll to a `#heading`. ```mermaid blocks are drawn as diagrams (flowchart, class, sequence, state, ER, and the other kinds Mermaid knows) by [merman](https://crates.io/crates/merman), without a browser, in Mermaid's light or dark theme to match the editor; a diagram that can't be drawn shows its text and why. `.mmd` and `.mermaid` files preview as their diagram. ```plantuml (or ```puml) blocks and `.puml`, `.plantuml`, `.pu`, `.iuml` and `.wsd` files are drawn by PlantUML itself, in its dark mode with a dark theme: a `plantuml*.jar` in the config folder's `tools/plantuml` (run with Java), or a `plantuml` program on PATH. Coiman downloads nothing; without PlantUML the block shows its text and where to put it. `@startuml`/`@enduml` are added when missing.
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
  **Doc** (or **Search: Find in Documentation**) keeps to documentation: Markdown, reST,
  AsciiDoc and text files whole, and in code only doc comments (`///`, `//!`, `/** */`,
  Python docstrings, Go's comments). Replace All is off while it's on.
- External changes: files edited by other programs reload automatically when the tab has no
  unsaved changes; otherwise a bar offers Reload or Keep mine. Deleted files are flagged
  (saving recreates them). The explorer and Quick Open index follow created/deleted files
- Integrated terminal (`` Ctrl+` ``): real shells in a pseudo-terminal (ConPTY on Windows) with
  alacritty's emulator, so PowerShell/bash line editing, colors and full-screen programs (vim,
  less, htop) work. Several terminals as tabs (`` Ctrl+Shift+` `` for a new one), 10,000 lines
  of scrollback (wheel, `Shift+PageUp/Down`), drag to select, `Ctrl+C` copies a selection or
  interrupts, right-click copies, `Ctrl+V` pastes (bracketed paste). **Pop Out** moves the
  terminal into its own window; closing that window puts it back in the panel. Ctrl+click
  on `file:line[:col]` in the output (`src/main.rs:12:5`, `File "a.py", line 3`,
  `Program.cs(10,7)`, `at x.js:4:2`) opens the file there; with Ctrl held, a file that
  exists is underlined. **Profiles**: the ▾ beside + (or **Terminal: New Terminal With
  Profile...**) starts any shell found (Windows: PowerShell, PowerShell 7, Command Prompt,
  Git Bash, MSYS2, WSL; elsewhere the shells in `/etc/shells`) or one of your own,
  `[[terminal.profiles]]` with `name` and `command`. **Terminal: New SSH Terminal...** (the
  Remote over SSH extension) lists the hosts named in `~/.ssh/config` and opens a terminal running `ssh` to the one picked (ssh asks
  for a password or passphrase there); the tab stays open after ssh ends, with its last words. **Split** (the split button or
  `Ctrl+Shift+5`) shows a second terminal beside the active one; a click gives one the
  keyboard. The shells open when Coiman closes start again next time, in their folders,
  with the split (`terminal.restore`); programs run in a tab don't. **Maximize
  Panel** (the corners button, or **View: Toggle Maximized Panel**) gives the panel all the
  room the editor had, and restores it. The shell is
  `terminal.shell` (default: PowerShell on Windows, `$SHELL` elsewhere)
- Terminal command history, suggestions and project commands come with the **Terminal
  Commands** extension (see [Extensions that come with Coiman](#extensions-that-come-with-coiman)).
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
- **Git Graph**, an extension of the Git pack (`workspace-read`; program `git`), in a panel
  (**Git: Show Graph**): every branch's commits with lines for branches and merges,
  branch and tag names, author and age; a click lists a commit's files (**Show change** shows
  one). Right click a commit to **cherry-pick** it onto the current branch, **revert**
  it, **create a branch** there, or **rebase interactively from here**: the commits after it,
  oldest first, to move up and down (↑ ↓) and pick, reword, squash, fix up or drop (changing
  a message rewords it), then **Start rebase**. On a branch (or commit) also: **Compare with the Current Branch**
  (the commits only on each, and the files it changed since they parted, each opening its
  change), **Merge into the Current Branch** and **Rebase the Current Branch onto** it.
- **Worktrees** (the button at the top of the Git Graph, or **New Worktree for…** on a branch):
  the repository's working folders, each on its own branch, with **Open** (as the project)
  and **Remove…** (asks first; **Even with changes** forces it). A new one takes a
  branch (an existing one, or **New branch** from the current commit) and a folder, named
  `<repository>-<branch>` beside it unless typed. Ones whose folder is gone can be forgotten.
  While a merge, cherry-pick, revert or rebase waits on conflicts, the graph says so, with
  **Resolve conflicts…** (the Merge Conflicts window), **Continue** and **Abort**. **Refresh**
  reads the graph again after work done elsewhere.
- **GitHub Actions**, an extension of the Containers pack (`workspace-read`; program `gh`)
  (**GitHub: Show Actions**): its panel lists the repository's
  latest workflow runs (or **Only failed**), each with its workflow, branch, state and age. A
  run shows its jobs and steps; a failed one has **Log of what failed** (only the errors and
  the lines before each, or the whole log, 3,000 lines at most), **Re-run failed jobs…**
  (asks first: it uses GitHub's minutes), **Open on GitHub**, and
  **Open in workflow** on a failed step, at its line in the workflow file. It all goes
  through the `gh` program and your GitHub login.
- **Merge Conflicts** (**Git: Resolve Merge Conflicts**, or **Resolve Merge Conflicts…** in
  Source Control): while a merge, cherry-pick, revert or rebase waits on conflicts, each
  conflicted file shows its conflicts side by side, current and incoming (and what both
  started from, when git wrote it), with **Accept Current**, **Accept Incoming**, **Accept
  Both** and **Both, Incoming First**, or a **Result** to write by hand and **Use This Result**;
  **Open in Editor** for anything else. **Mark as
  Resolved** stages the file; **Continue** goes on once every file is, **Abort** gives the
  operation up.

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
- **Git Tools**, an extension of the Git pack (`workspace-read`, `workspace-write`; program
  `git`), in a panel with a tab for each:
  - **Search History** (**Git: Search History...**): commits by words in the message, by
    author, by file or folder, or by a change: **Added or removed text** finds the commits
    where a name or text came in or went out (`git log -S`), **Changed lines matching** takes
    a regular expression (`-G`). It can be kept to a file or folder, and cover all branches.
    A commit lists its files (a click shows what it changed in one) and, for a change search,
    the lines that match.
  - **Submodules** (**Git: Submodules**): each one's folder, state (up to date, not fetched,
    at another commit, in conflict), URL and branch, with **Fetch** (at the recorded commit),
    **Newest** (its branch's newest commit; commit to record it) and **Remove…** (asks
    first). **Fetch all** fetches every one; a URL and a folder add a new one.
  - **Hooks** (**Git: Hooks**): the scripts Git runs at points of its work, each on, off,
    an example only, or none. **Create** writes a shell script and opens it, **Off** renames
    it so Git skips it (**On** brings it back), **Run** runs it now as Git would and shows
    what it printed. A `core.hooksPath` set by a tool such as Husky is shown.
  - **.gitignore** (**Git: Explain .gitignore**): every rule, from which file and line, and
    what it hides now (a click lists it); a rule that hides nothing can go. **Why is … ignored?**
    names the rule that hides a path; **New rule** shows what a rule would hide (and the tracked
    files it would match, which stay tracked) before **Add to .gitignore**.

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
- **Breakpoint groups**: right-click a line or function breakpoint in BREAKPOINTS and type a
  name to start a group, or move it to a group there is, or out of its group. Each group shows
  under its name with how many it holds; its box turns them all on or off at once (half-filled
  when some are on), and its cross undoes the group, leaving the breakpoints. Groups are kept
  between runs and go into exports.
- **Export and import**: **Debug: Export Breakpoints...** saves line, function and exception
  breakpoints to a JSON file (paths inside the project are relative, so the file travels with
  it); **Import Breakpoints...** adds a file's breakpoints to those set. Both are under
  BREAKPOINTS too. Without exporting, each project's line, function and exception breakpoints
  are kept between runs of the app.
- **Remote debugging**, the Remote Debugging extension of the Containers pack (`workspace-read`,
  `workspace-write`; programs `ssh`, `scp`), in its panel: **Debug: Remote (gdbserver)...**
  debugs a program on another machine
  with gdb, using a copy of the same build here for its symbols. **Start gdbserver over ssh**
  runs `ssh host gdbserver :PORT program args` with key login only (no password prompt), and
  `-L` carries the port here (to another local port if the same number is taken);
  **Copy the program there first** sends it with scp. **ssh options** go to both ssh and scp
  (`-i key`, `-o Port=2222`, `-F file`). **Connect to a gdbserver running** attaches to
  `host:port`. gdbserver stops the program as gdb connects; Continue runs it to the
  breakpoints. If gdb hears nothing for 15 seconds the debug console says so (wrong host or
  port, gdbserver not running), and Stop gives up. Stop ends the program, gdbserver and ssh;
  Restart starts it all again. The form is kept in the project's `.coiman/remote_debug.toml`.
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

## Agents (opencode)

The **Agents** panel chats with the `opencode` program installed on this computer
(`npm install -g opencode-ai`); Coiman itself opens no connection. It keeps opencode's sessions,
picks the model, adds the open file or the selection as context, and shows the files the agent
changed. By default the agent asks before it edits a file or runs a command (**Manual**);
where opencode has no one to ask, it is turned down and the panel says so. **Auto** lets it
act. Newer opencode releases and older ones (1.19) both work. `workbench.agents_enabled = false`
hides the panel.

**Agents: MCP Servers...** (or the panel's MCP button) lists the MCP servers opencode gives the
agent as tools, for **This project** (`opencode.json`, or `opencode.jsonc`, in its folder) or **Every project**
(`~/.config/opencode/opencode.json`). A server is a program opencode starts (a command, and
environment variables) or an address (with headers, such as `Authorization`). Servers can be
added, edited, turned on and off and removed; **Save** writes only the file's `mcp` section,
keeping the rest (its comments are dropped, and the window says so first). **Check with
opencode** runs `opencode mcp list` to show which servers it reaches.

## Packs

Coiman itself is the editor: files, languages and their servers, search, the terminal, Run,
Debug and testing, Git's status, commits, branches and diffs, settings and looks. What goes
beyond that comes in **packs**, one per kind of work, installed from the **PACKS** section of
the Extensions view. A pack that isn't installed adds nothing: its commands aren't in the
palette (a shortcut to one says which pack has it), its windows, panels and code lenses don't
appear, and its background work doesn't run.

| Pack | What it brings |
| --- | --- |
| Web and APIs | HTTP client, OpenAPI |
| Databases | SQLite, PostgreSQL, MySQL, SQL Server, Redis, MongoDB; Jupyter notebooks |
| Containers, remote and CI | Docker, GitHub Actions, remote folders and SSH terminals, remote debugging |
| Embedded and systems | Serial monitor, binary size, memory and disassembly views |
| Mobile | Android |
| Graphics | Shader problems while typing, shader preview |
| Code analysis | Class diagrams, architecture, flow, code health, overview, session statistics, profiler, coverage, run history, stack traces, rules, search by syntax tree |
| Git, more of it | Git Graph, history search, submodules, hooks, `.gitignore` rules |
| Agents | Agents (opencode), MCP servers |
| Terminal extras | Command history and Ctrl+Space suggestions |

Installing a pack also offers its extensions, one after the other (Class Diagram,
Architecture and Code Health come with Code analysis, Git Graph and Git Tools with Git, HTTP Client
and OpenAPI with Web and APIs, Docker, GitHub Actions, Remote over SSH and Remote Debugging with Containers,
SQL Databases, Redis and MongoDB with Databases, Terminal Commands
with Terminal extras); removing it
uninstalls them. The choice is `[packs] installed`
in the settings (`"all"` for every pack); a project's `.coiman/settings.toml` can set its own,
so a project gets the environment it needs. A new install starts with none; settings from
before packs have every pack, so an update takes nothing away.

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
  requests to the active file's server), `terminal` (the command lines of the terminals,
  typing into them, the shells' history files, and folders outside the open folder). You
  approve them before it is
  installed and can uncheck any of them, then or later. A call without its permission fails
  with `permission denied`.
- **Contribution points** are declared in `plugin.toml` and drawn by the editor:
  - status bar items;
  - bottom panel tabs (headings, text, code, lists, buttons, colored text, tables, diffs);
  - tree views in the Explorer;
  - Command Palette commands;
  - pick lists (`show-pick`): a box to type in and rows to choose from, below the terminal's
    cursor or at the top of the window, narrowed by the editor or by the plugin as typing
    goes on, with Enter, Shift+Enter and a ✕ per row.

  Plugins fill them with data and receive the events they subscribe to: `opened`, `saved`,
  `active-changed`, `edited`, and with the `terminal` permission `terminal-command` (a
  command sent with Enter in a shell) and `terminal-suggest` (Ctrl+Space in a shell; the key
  goes to the shell when no plugin listens).
- **Graphs** (`show-graph`): nodes with a kind, a group, member lines and a place in a file,
  and edges that inherit, implement, hold or use, drawn by the editor as a UML diagram in a
  window of its own, with zoom, filtering, opening and export to SVG, Mermaid and PlantUML.
- **Long work** goes in steps: `continue-later` has the plugin called again (`resumed`) after
  what is waiting, so no call runs into the time limit and the editor stays responsive.
- **More for panels**: `links` blocks (places in files, a click opens one) and `rows` blocks
  (coloured cells with buttons). **`run-in-terminal`** runs an approved program in a terminal
  tab; **`show-markdown`** opens a page; **`syntax-functions`** gives a file's functions with
  their length and complexity from the editor's parser. An approved `python` is the project's
  interpreter.
- **Forms** (`form` blocks): text fields, check boxes and choices on one line with buttons;
  a press, or Enter in a text field, sends `form-submitted` with what each field holds. What
  was typed stays until the plugin sends a field a new value. **`open-commit-diff`** shows
  what a commit changed in a file; **`rename-file`** and **`set-executable`**
  (`workspace-write`) rename a file and make a script runnable. Git never waits on a password
  prompt or opens an editor in a program a plugin runs. A form's `button` field sends the
  form with its id, and `line-break` starts a new line.
- **Code lenses** (`set-code-lenses`, `editor-read`): lines above lines of a file that run a
  plugin command. **`now`** (the time, and how far local time is from UTC), **`random`**,
  **`environment-variable`** (the `environment` permission: they may hold secrets) and
  **`open-text`** (text in a new tab). **`open-url`** opens a web page in the browser.
- **Its own folder** (`data-folder`, `data-write`, `data-remove`, `data-rename`), in the
  config folder, never the project; `start-job-into` puts what a program prints there as it is
  (an archive), and `start-data-job` runs a program in it. With no folder open, programs run
  there. **`ssh-config`** (`terminal`) reads `~/.ssh/config`. Events **`deleted`** and
  **`renamed`** follow the Explorer. What a program run in a terminal prints is read for
  problems. **`start-job-lines`** sends each line a program prints as it's printed
  (`job-line`); **`debug-attach`** starts gdb attached to a gdbserver (a restart runs the
  plugin's `restart` command, the end `debug-ended`); **`debug-console`** writes in the Debug
  Console. `write-file` makes the folders a file goes in.
- **More for forms and jobs**: `password` fields (dots, never kept after the editor closes)
  and `multiline` ones (Ctrl+Enter sends); **`start-job-with`** gives a program environment
  variables for one run (a password, never in its arguments; `PATH` and the like can't be
  set) and can run it in the plugin's own folder. A program named by a plugin is also looked
  for where database clients' installers put it. **Openers** in `plugin.toml` (extensions,
  and how the file starts) hand a file to the plugin instead of a tab. The `document` has
  its `selection`, current when a command runs. A `table` block draws only the rows in view.
- **Commit graphs** (`commit-graph` blocks): commits with the lanes their lines run in, drawn
  by the editor with branch and tag chips, a click and a right-click menu that run plugin
  commands. **`open-compare-diff`** shows how a file differs between two revisions;
  **`copy-text`**, **`show-merge-conflicts`** and **`open-folder`** (a folder as the project).
- **Storage**: each plugin keeps its own text (`storage-read`, `storage-write`, 4 MB) in
  `<config>\plugins\data`, without a permission.
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
- **Plugins: New Plugin...** asks for a name, a folder and a template: **Command and status
  bar item**, **Panel**, or **Problems on save**. It writes a folder that builds as it is
  (`Cargo.toml`, `plugin.toml`, `src/lib.rs`, a copy of `wit/plugin.wit`, `.gitignore`, a
  README) and opens it. The id is `dev.` and the name in lowercase with dashes.
- **Plugins: Start Dev Mode** works on the open plugin folder: it runs `cargo build --release
  --target wasm32-unknown-unknown` (Rust's wasm32 target is needed), turns the module into
  the component Coiman loads, and loads it, asking first for its permissions as an install
  does. After that, each save of a `.rs`, `.toml` or `.wit` file in the folder builds and
  loads it again, quietly while it asks for no more than was granted; a new permission brings
  the prompt back. Cargo's output goes to the Output panel; a build that fails leaves the last
  one running. The Extensions view shows **Dev mode** with **Stop**
  (**Plugins: Stop Dev Mode**); the plugin stays installed as last built.

### Extensions that come with Coiman

Features that not everyone needs come as extensions: they ship with Coiman (in `extensions`
beside the program, or `share/coiman-ide/extensions` on Linux), but nothing of them runs until
you install one under **COMES WITH COIMAN** in the Extensions view. Coiman stays as light as
you keep it; uninstalling one takes it out again. When the copy beside Coiman changes (a new
version, or rebuilt for a newer plugin API), the installed one shows **Update…**.

- **Terminal Commands** (`terminal`, `workspace-read`): keeps each command sent with Enter in
  a shell (the line after the prompt, 1,000 at most, in its storage) and reads the shells' own
  history files (PowerShell, bash, zsh, fish).
  - **Ctrl+Space** in a shell (**Terminal: Suggest Commands**) lists, for the line being
    typed: paths (once the command has a word after it, or the word has a `/` or `\`), the
    project's commands, commands run before (those that start with the line first) and the
    shells' history. Typing narrows the list; arrows move; **Enter** puts the choice on the
    command line in place of what was typed, **Shift+Enter** runs it; **✕** forgets one.
  - **Terminal: Search Command History...** opens the same list with only commands run
    before, this project's first.
  - **Terminal: Run Project Command...** lists what the project offers, and **Enter** runs it:
    `package.json` scripts (with npm, pnpm, yarn or bun, by the lock file), Makefile targets
    (with their `##` comment or first line), `justfile` recipes, Taskfile and Deno tasks, and
    Cargo's build, run (each program and example), test, check, clippy and fmt.
- **Architecture** (`workspace-read`, `editor-read`): the dependency graph, its cycles and
  what a change reaches; see [Architecture](#architecture).
- **Code Health** (`workspace-read`; programs `cargo`, `npm`, `python`, `poetry`, `uv`, `git`):
  secrets, complexity, duplicates, docs, dependencies, licences, audits and the project
  overview; see [Code health](#code-health).
- **SQL Databases** (`workspace-read`, `editor-read`; programs `sqlite3`, `psql`, `mariadb`,
  `mysql`, `sqlcmd`, `SqlLocalDB`): SQLite files and SQL servers; see Databases.
- **Redis** (programs `redis-cli`, `valkey-cli`, `memurai-cli`): a Redis server's keys,
  values and a console; see Databases.
- **MongoDB** (program `mongosh`): a MongoDB server's collections, documents and scripts;
  see Databases.
- **Remote Debugging** (`workspace-read`, `workspace-write`; programs `ssh`, `scp`): gdb on a
  program under gdbserver on another machine; see Remote debugging under Debugging.
- **Remote over SSH** (`workspace-read`, `terminal`; programs `ssh`, `tar`): a folder on
  another machine as a mirror here, terminals and commands there, SSH terminals.
- **GitHub Actions** (`workspace-read`; program `gh`): workflow runs, jobs, failed logs,
  re-runs; see **GitHub Actions** under Git.
- **Docker** (`workspace-read`; program `docker`): containers, images, Compose; see **Docker** further down,
  beside the remote folders.
- **HTTP Client** (`workspace-read`, `editor-read`, `environment`; program `curl`): `.http`
  requests, environments, history; see [HTTP client](#http-client).
- **OpenAPI** (`workspace-read`, `workspace-write`, `editor-read`): a spec's endpoints and
  models; see [OpenAPI](#openapi).
- **Git Graph** (`workspace-read`; program `git`): the commit graph and what's done from a
  commit; see Git Graph under Git.
- **Git Tools** (`workspace-read`, `workspace-write`; program `git`): history search,
  submodules, hooks and `.gitignore` rules; see Git Tools under Git.
- **Class Diagram** (`workspace-read`, `editor-read`): the project's classes, interfaces,
  enums, structs, records and traits as a UML class diagram, read from the code without a
  compiler: Java, Kotlin, C#, TypeScript and JavaScript, C and C++, Swift, Dart, Scala, PHP,
  Python, Rust and Go. Each box has the type's fields and methods with their visibility
  (`+` public, `-` private, `#` protected, `~` package); lines show what it extends (solid,
  hollow triangle) and implements (dashed, hollow triangle), and a diamond the project's types
  it holds in a field. A parent from outside the project (a library's) gets a grey box.
  - **Class Diagram: Whole Project**: every type, the 400 most related when there are more.
  - **Class Diagram: Current File**: the types of the file in front, their parents up to the
    top, their children, and what they hold or are held by.

  The window lays parents above children. Drag to move, scroll to zoom, **Fit**; **Find a
  class** dims the others (Enter picks the first and centres it); a click picks a box and
  lights what it is related to; a double click opens the file at the type. **Members** shows
  or hides the fields and methods. **Export SVG...**, **Copy Mermaid**, **Copy PlantUML** and
  **Preview Mermaid**.

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
| Tests in any language | The Testing view finds a project's runner and reads its results, not its printed text: `go test -json`, pytest, unittest, `node --test`, Vitest, Jest, `dotnet test`, Maven, Gradle and RSpec. Tests are listed before they run where their text shows them, with a run button and a pass or fail mark there and in the gutter. A failure shows its message, with links to the frames in this project. A project adds its own runner in `.coiman/tests.toml`, naming a command and its `format` (`gojson`, `junit`, `trx`, `tap`, `unittest`, `rspec`, `jest`). **Timeline** shows each test as a bar as long as it took, end to end in the order they ran, colored by how it went, with the total and the slowest; a click on a bar selects the test. **At once** (also in Cargo's view, and `[testing] parallel` in settings) sets how many tests run at the same time: the runner's own choice, one at a time, or a number. It's passed on as Go's `-p` and `-parallel`, pytest-xdist's `-n` (when installed), `node --test`'s `--test-concurrency`, Jest's and Vitest's `--maxWorkers`, dotnet's `RunConfiguration.MaxCpuCount`, Maven's `-DforkCount`, Gradle's `--max-workers`, and Cargo's `RUST_TEST_THREADS` (`NEXTEST_TEST_THREADS` for nextest); unittest and RSpec can't be told, and the view says so. A runner in `tests.toml` takes it as `workers = "-j {n}"` (at `{workers}` in its command, else at the end), and `{jobs}` in its command is the number. While runners run in turn, the view names the one running and how many are left |
| Snapshot tests | `Testing: Review Snapshots` lists the snapshots insta left to review (`.snap.new` beside the `.snap`), a new one marked. Each shows what changed against the accepted one, its header left out, removed lines red and added green. **Accept** makes it the snapshot, **Reject** deletes it, and **Accept all** or **Reject all** settle every one; **Open** shows the file. In a project whose `package.json` uses Jest or Vitest, **Update Snapshots** runs `npx jest -u` or `npx vitest run -u` in a terminal |
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
| Theme schedule | `workbench.theme_schedule = "sunrise Coiman Light; 19:00 Coiman Dark"` changes theme by this computer's clock. `sunrise` and `sunset` are worked out here from `workbench.theme_location`, which you type in; nothing looks up where you are. A theme chosen by hand stays until the next time on the list |
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

The **Architecture** extension (in the Code analysis pack) reads which file imports which, for
Rust, Python, JavaScript and TypeScript, Java, C#, Go, and C and C++. It reads the import lines
and finds them among the project's files, without a build or a language server, in steps, so a
project of a few hundred files takes about a second and the editor stays responsive. Libraries
from outside the project are left out.

- **Architecture: Show Dependency Graph**: the files in layers, what a file uses below it,
  imports in a cycle red, test files green, in the graph window (drag, zoom, find, a click
  lights what a file is linked to, a double click opens it). **Architecture: Show Folder
  Graph** shows folders instead, each line labelled with how many imports it stands for.
- The **Architecture** panel lists the **cycles** (files that import each other round in a
  circle, largest first), the **most used** files (with how many use them and how many they
  use), and the **unused** ones (nothing imports them and they import nothing; entry points
  and scripts show here too).
- **Architecture: Impact of Current File**: the files a change to the file in front reaches,
  each with how many steps away, and the tests among them, in the panel; the graph shows just
  them.
- **Export SVG...**, **Copy Mermaid** (a flowchart, one subgraph per folder, colours kept),
  **Copy PlantUML** and **Preview Mermaid**.

Rust: `mod x;` means a file contains another, not that it uses it, so it makes no edge.
Paths through `crate::`, `super::`, `self::`, other crates of the workspace and child modules
count. Python: absolute, relative and `from x import y` of a module. JavaScript and TypeScript:
relative paths, `@/` and `~/` for `src`, `index` files, `require` and dynamic `import()`.
Java: imports, wildcards, static imports and classes of the same package. C#: types through
`using` and the namespace. Go: packages of the module in `go.mod`. C and C++: `#include` of the
project's own headers.

## Shaders

GLSL (`.glsl`, `.vert`, `.frag`, `.geom`, `.comp`, `.tesc`, `.tese`), HLSL (`.hlsl`, `.fx`)
and WGSL (`.wgsl`) files are colored. Nothing needs installing.

- **Problems while typing.** WGSL is read and checked by naga (the checker wgpu uses), as
  you type. GLSL is compiled by your graphics driver itself after each pause in typing,
  through the OpenGL that draws Coiman, and its messages land on the file's own lines
  (NVIDIA, AMD, Intel and Mesa write them differently; all are read). HLSL is colored only:
  checking it needs Microsoft's `dxc`.
- **Preview: Shader** draws the active fragment shader live in a window beside the code. It
  follows the file as you type; when the file doesn't compile, the last picture that did
  stays and the errors show under it. **Pause**, **Play** and **Restart** control the time.
  - Shadertoy style: a `mainImage(out vec4, in vec2)` with `iTime`, `iTimeDelta`, `iFrame`,
    `iResolution` and `iMouse` (where the mouse is while pressed, and where it was pressed).
  - The Book of Shaders style: `void main()` writing `gl_FragColor`, with `u_time`,
    `u_resolution` and `u_mouse` (where the pointer is over the picture).
  - Files without `#version` get 1.20 if they write `gl_FragColor`, else 3.30 core. Uniforms
    the file doesn't declare are added for it.
  - WGSL: a `@fragment` function, turned into GLSL by naga, with y growing downward as in
    wgpu. Time, size and mouse come in a
    uniform block at group 0, binding 0:
    `struct Coiman { time: f32, resolution: vec2f, mouse: vec4f }` and
    `@group(0) @binding(0) var<uniform> coiman: Coiman;`.

## HTTP client

The **HTTP Client** extension of the Web and APIs pack (`workspace-read`, `editor-read`,
`environment` for `{{$processEnv}}`; program `curl`). Requests written in `.http` (or `.rest`)
files, the format JetBrains' HTTP Client and VS Code's REST Client use, are sent through
`curl`: Coiman opens no connection itself. The files are colored without it.

- **Writing them:** `###` between requests (with a title after it), `METHOD URL`, headers,
  a blank line and the body; `< ./file.json` sends a file. `@name = value` lines define
  variables; `{{name}}` uses them. `# @name login` names a request, so later ones can use
  its response: `{{login.response.body.$.token}}` (a path into its JSON) or
  `{{login.response.headers.Location}}`. Also `{{$uuid}}`, `{{$timestamp}}`,
  `{{$randomInt 1 10}}`, `{{$datetime iso8601}}`, `{{$processEnv NAME}}` and
  `{{$dotenv NAME}}` (from `.env`). `# @no-redirect`, `# @timeout 10` and `# @insecure`
  change how it's sent.
- **Sending:** **▶ Send** above each request, or **HTTP: Send Request at Cursor**.
  **Copy as curl** beside it copies the request, values filled in, as a command.
- **Environments:** `http-client.env.json` beside the file (or in a folder above it), with
  `http-client.private.env.json` over it for secrets (keep that one out of Git), or VS
  Code's `rest-client.environmentVariables`. The **HTTP Client** panel chooses one per
  folder; an undefined name says which and where to define it.
- **The response** (in the HTTP Client panel, **HTTP: Show Client**): status, time, size,
  where redirects ended; the body (JSON indented), to copy or open in a tab; the headers.
- **History:** each file keeps its last 30 responses, bodies up to 64 KB, 3 MB for all files
  (in the extension's storage in the config folder, not the project). A body is kept as
  text: a binary download or `< file` isn't kept byte for byte. Choose **A** and **B** to **Compare** them: the status, headers that differ
  (dates and request ids left out), and the body line by line.

## OpenAPI

The **OpenAPI** extension of the Web and APIs pack (`workspace-read`, `workspace-write`,
`editor-read`). An OpenAPI (3.x) or Swagger (2.0) file, in JSON or YAML, gets a line at its
top saying how many endpoints and models it has; a click (or **OpenAPI: Show Endpoints**)
opens the **OpenAPI** panel.

- **Endpoints** by tag, each with its method in color (deprecated ones struck through), and a
  filter (Enter applies it, to models too). **Details** shows its parameters (where each goes, its type, whether it's required), its
  body with an example made from the schema, its responses, and the security it needs.
- **Open as request** adds it to a `.http` file beside the spec (`shop.yaml` → `shop.http`,
  made with `@baseUrl` from the spec's server if new), ready for ▶ Send: path and required
  query parameters as `{{variables}}`, optional ones in a comment, the auth header the spec
  asks for (`Authorization: Bearer {{token}}`, an API key's header), and the example body
  (read-only fields left out). One already there is shown, not added twice. **All as a
  .http file** writes every endpoint at once.
- **Models** lists the schemas with their fields, types and which are required.
- `$ref`s, `allOf`/`oneOf`, server variables and Swagger 2's `host`, `basePath` and body
  parameters are followed. **Go to it** jumps to where the spec declares an endpoint.

## Serial monitor

**Serial: Open Monitor...** opens a board's serial port (an Arduino, ESP32 or Pico on USB),
like the Arduino IDE's monitor. Nothing needs installing: Coiman talks to the port itself.

- **The port** comes from a list of the ports there are (on Windows, what the drivers
  registered, with the kind of chip; on Linux `ttyUSB`/`ttyACM` and built-in ports; on macOS
  `cu.*`), or is typed (`COM7`, `/dev/ttyUSB0`). **Refresh** after plugging a board in.
- **Speed** (300 to 921600 baud, or any typed) and **format** (`8N1`, `8E1`, `8O1`, `7E1`,
  `8N2`). **DTR** and **RTS** can be switched while connected: many boards restart when DTR
  goes on.
- **Text** shows each line as it comes, with the time it came when **Times** is on;
  **Follow** keeps the newest in view. **Hex** shows the last 64 KiB as bytes. **Plot** draws
  the numbers on each line over time, one colored line per value, as Arduino's plotter reads
  them (`21.5,40`, or named: `temp:21.5 hum:40`).
- **Send** sends a line with the chosen ending (none, LF, CR, or CR LF); Up and Down bring
  back the lines sent before. **Save log…** writes what came to a file; **Clear** empties
  the view.
- A Windows named pipe (`\\.\pipe\name`) or a Unix FIFO opens too, with no settings to
  apply, so a program can stand in for a board. Closing the window closes the port.

## Binary size

**Build: Analyze Binary Size...** shows what takes room in a program, a shared library or an
object file (ELF, PE, Mach-O and COFF, read by Coiman itself: no tool needed). It picks the file
open in the editor if it is one, else the newest one in the project's build folders (`target`,
`build`, `out`, `cmake-build-*`); **Choose…** opens another.

- The summary gives the format and processor, the file's size and what is loaded with the
  program, split into code, read-only data, data and zero-filled data. For an ELF program
  (embedded firmware) it adds **flash** (code, read-only data and data's start values) and
  **RAM** (data and zero-filled data), as `size` counts them.
- **Groups** adds up the symbols by crate (Rust) or namespace (C++), names demangled: what
  `core`, `std`, `serde_json` or the program's own code cost. Click one for its symbols.
- **Symbols** lists them biggest first, with a filter. Where the file gives no sizes (PE,
  COFF, Mach-O), a symbol runs to the next one.
- **Sections** lists `.text`, `.rodata`, `.data`, `.bss`, debug info and the rest.
- **Analyze again** after a build shows the change: in total, and for each group (bigger in
  red, smaller in green).
- A program built with Visual C++ (or Rust's MSVC toolchain) keeps its symbols in a `.pdb`
  beside it; the window says so and shows the sections only.

## Profiling

- **Run: Profile Current File** runs the file to its end under a sampling profiler, then shows
  the **PROFILE** panel. Python files use **py-spy** (`pip install py-spy`; Coiman finds it on
  `PATH` or where pip put it for the project's Python). On Linux, anything Run can run uses
  **perf**. The profiler is a program of its own; Coiman doesn't install it.
- The panel has a **flame graph**: the outermost calls on top, what they called below, each as
  wide as its share of the time. Hover for the name, file and line, and its share. A run
  here also gives the time (samples × 5 ms at 200 a second) beside each share; fewer than 10
  samples are flagged, as the run was too short or the profiler started late. Click a
  function to zoom into it, click the top row to zoom back out, and double-click to open its
  code. **Hottest** lists functions by time spent in themselves (then with what they call),
  each a link to its code.
- **Compare**: once there's a profile before this one, the panel's Compare box shows each
  function's share of the time before and now, and the change, biggest first, slower in red
  and faster in green. Shares, not times, so runs of different lengths compare.
- **Profile: Open Profile File...** reads a profile made elsewhere: collapsed stacks
  (`main;work;fib 12`, what py-spy's `raw` format and flame graph tools write) or `perf script`
  output.

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

The **Code Health** extension (Code analysis pack). **Code Health: Check Project** reads the
project's files in steps, Git-ignored ones and folders such as `node_modules`, `target` and
`dist` left out, and shows what it found in the **Code Health** panel, one tab at a time (the
buttons at its top, each with how many it found; **Scan again**). A click on a place opens the
file there. The package managers (`cargo`, `npm`, `python`, `poetry`, `uv`) and `git` run only
as far as they were approved at install, in trusted folders.

- **Secrets:** keys and tokens by their shape: private keys, AWS, GitHub, GitLab, Slack,
  Stripe, Google, npm, OpenAI-style keys, JSON Web Tokens and passwords in URLs. It also
  finds quoted passwords and keys assigned in code that look random enough to be real, and
  unquoted ones in settings files (`.env`, `.ini`, `.properties`, YAML).
  - Values from the environment and placeholders (`${KEY}`, `changeme`, `<token>`) aren't
    reported.
  - A line marked `gitleaks:allow`, `pragma: allowlist secret` or `not-a-secret` is skipped.
  - What's found shows masked: only its first four characters.
- **Complexity:** each function's cyclomatic complexity and length, through the editor's own
  parser (tree-sitter, asked with `syntax-functions`), for
  Rust, Python, JavaScript, TypeScript, Java, C, C++, C# and Go. It shows the 200 most
  complex. Up to 10 is plain, to 20 complex, above that very complex.
- **Undocumented:** public items with no doc comment, each a link to its line: Rust's `pub`
  items (no `///` above), Go's exported names (no `//` above), Python's public top-level `def`
  and `class` (no docstring), JavaScript's and TypeScript's `export`s (no `/** */` above).
  Tests are left out.
- **Project: Generate Overview** (Code Health) writes a Markdown page about the open folder
  and opens it: the README's title and first paragraph, files, lines of code and test
  files, the build files found, languages with files and lines, the largest top-level folders
  and files, and from `git` the branch, commits, first and last dates, authors and last tag.
  Tests inside code files (Rust's `#[cfg(test)]`) aren't counted as test files.
- **Project: Show Session Statistics**: active time per project and day, kept on this
  computer only (`activity.json` in the config folder). Time counts while the window is in
  front and something was typed or clicked in the last two minutes. The page shows today,
  the last 7 and 30 days, the days with use, today's projects, the last 14 days and every
  project's total.
- **Architecture: Show Flow of Function at Cursor** draws the function around the cursor as a
  Mermaid flowchart, opened with its preview: ifs with yes and no, loops with each and done,
  returns to the end and breaks out of their loop; plain statements in a row share one box.
  Rust, Python, JavaScript, TypeScript, Java, C, C++, C# and Go. A `let … else` or a
  `?` stays inside its box.
- **Remote over SSH**, an extension of the Containers pack (`workspace-read`, `terminal` for
  the ssh config; programs `ssh` and `tar`), in the **Remote** panel. **Remote: Open SSH
  Folder...** works on a folder on another machine: it asks for the ssh host (the hosts of
  `~/.ssh/config` are offered), the folder there and any ssh options, brings the folder here
  with `ssh … tar` (build folders such as `target` and `node_modules` left out) into a mirror
  in the extension's own folder (`<config>\plugins\data\coiman.remote\mirrors`), and opens
  the mirror as the project; a failed download leaves the last mirror as it was. Each file saved there goes back over ssh, its folders
  made first; deleting (or moving to the Recycle Bin) and renaming in the Explorer happen there
  too. **Remote: Download Folder Again** brings the remote folder's current state (save
  changed files first: what comes down replaces them). **Remote: Open Terminal There** opens a shell in the remote
  folder, and **Remote: Run Command There...** runs a command in it (`cargo build`, `make`),
  the shell staying for more; errors and warnings it prints go to Problems, paths in the
  remote folder (absolute or relative) pointing at the files here, and Ctrl+click opens them
  in the terminal too. Language servers run here, on the copy, with the tools installed on
  this computer. ssh logs in with a key (no password prompt). The mirror's `.coiman-remote.toml` says where it comes from.
  A file goes back as text: a binary file saved in the mirror isn't sent byte for byte.
- **Docker** (an extension of the Containers pack; `workspace-read`, program `docker`), in a
  panel: **Docker: Show Containers, Images and Compose** asks the `docker` program (Docker
  Desktop, Docker Engine or anything that answers like it). **Containers**: name, image,
  status and ports, running ones in green; **Stop**, **Restart** and **Shell** (bash, else sh,
  in a terminal) for running ones, **Start** and **Remove…** (asks first) for stopped ones,
  **Logs** (followed, in a terminal) for all. **Images**: size and age, **Remove…** (Docker
  refuses one a container uses, and says so). **Compose**, when the folder has `compose.yaml` or
  `docker-compose.yml`: its services with status and ports, **up** (with `--build`) and
  **build** and **logs** in terminals, **down**. Without Docker, or with its engine stopped,
  the panel says which.
- **Android: Devices and Logcat** asks `adb` (on PATH, in the Android SDK, or unpacked into
  the config folder's `tools/platform-tools`). It lists the devices, saying what to do with one
  that is unauthorized or offline. **Logcat** follows a device's log live, colored by level,
  filtered by level, by text in the tag or message, and by app (`com.example.app`: its running
  processes); **Pause** and **Clear**. A Java or Kotlin stack frame naming a file of the open
  project is a link to its line. **Shell** opens `adb shell` in a terminal, and **Install APK…**
  runs `adb install -r` there. Closing the window or Coiman stops logcat.
- **Jupyter notebooks**: an `.ipynb` file opens as its cells. Markdown cells are drawn as
  the Markdown preview draws them (double-click to edit); code cells are edited in place and
  run in a Python kernel (`jupyter_client` and `ipykernel` in that Python): ▶ or Shift+Enter, or
  **Run All**, **Interrupt**, **Restart**. Below each cell: what it printed (stderr in red), its
  value, its error, and pictures (plots). Cells can be added, moved, deleted and switched
  between code and Markdown. **Save** (Ctrl+S) writes nbformat 4, keeping every field Coiman
  doesn't edit and the outputs of cells not run again; **Open as JSON** shows the file's text.
- **Duplicates:** code written twice: runs of 6 lines or more that appear again, in another
  file or the same one, whitespace aside. Blank lines, comments, imports and lines of only
  brackets don't count.
- **Dependencies:** each lockfile in the folder (`Cargo.lock`, `package-lock.json`,
  `poetry.lock`, `uv.lock`) as a tree under **Dependencies** in the Explorer, from the
  project's own packages down, a level opened per click; and the packages pinned at more than
  one version, with what wants each version.
  - **Check for updates** asks `cargo update --dry-run`, `npm outdated` and, for a Python
    project, the project's `pip list --outdated` (they ask their registries themselves). Each
    package shows its version now, what an update gets within the project's requirement,
    and the newest, marked a major, minor or patch step.
  - **Update** and **Remove** run the package manager's own command in a terminal
    (`cargo update -p`, `cargo remove`, `npm update`, `npm uninstall`, `pip install --upgrade`,
    `pip uninstall`, and Poetry's and uv's). Remove is offered for packages the project names
    itself. When the newest is beyond the requirement, there is no Update: raise the version
    in the manifest.
- **Licences:** each dependency's licence, from `cargo metadata --offline` and npm's
  `node_modules`. Each is rated permissive, weak copyleft (MPL, LGPL, EPL), copyleft (GPL,
  AGPL) or unknown, reading SPDX expressions with `OR` and `AND`.
- **Audits:** **Run the audits** runs `cargo audit` beside a `Cargo.lock` and `npm audit`
  beside a `package-lock.json`, most severe first, with the fix and each advisory's address.
  These tools fetch the lists of known vulnerabilities themselves. When one isn't installed,
  the tab says how to get it.

In Restricted Mode, Cargo and npm don't run (they read the project's settings), so licences
come from `node_modules` alone, and the audits, update checks and updates wait for the folder
to be trusted. Lockfiles are read either way.

## Databases (SQLite, PostgreSQL, MySQL, SQL Server, Redis, MongoDB)

SQL databases are the **SQL Databases** extension of the Databases pack (`workspace-read`,
`editor-read`; programs `sqlite3`, `psql`, `mariadb`, `mysql`, `sqlcmd`, `SqlLocalDB`), in its
panel. A SQLite file opens there from the Explorer or Quick Open (it is recognised by its
contents, whatever its name), or with **Database: Open SQLite File...** (a list of the open
folder's `.db`, `.sqlite` and `.sqlite3` files).

- **Tables and views**, each with **Rows** (the first 200) and **Columns**: the columns, with
  the primary key marked, types, `not null`, where a foreign key points (`→ customer.id`),
  and the table's indexes (unique, partial, or made by a constraint).
- **SQL** in the box, one statement or several; **Run** or Ctrl+Enter. Each statement that
  returns rows gets its own result, in a table that only draws the rows in view, with NULL
  said. Statements that change rows say how many, and the tables are read again after a
  change. The last runs are kept (**Again** runs one more time).
- **Explain** shows how the database would carry out each statement (`EXPLAIN QUERY PLAN`),
  as an indented tree. Steps that read a whole table or sort in a temporary table are marked,
  since an index could spare them. Nothing runs: SQLite opens the database read-only for it.
- **Diagram**: the tables as boxes with their columns (`#` for the primary key) and a line
  from each foreign key to the table it points to, in the graph window (zoom, filter, export
  to SVG, Mermaid or PlantUML).
- **Database: Run SQL (Selection or File)** runs the selection, or the whole `.sql` file in
  front, against the open database. **Database: Explain Query Plan (Selection or File)**
  explains it instead, and **Database: Show Tables Diagram (ER)** opens the diagram.

- **PostgreSQL, MySQL (or MariaDB) and SQL Server**: **Connect…** in the panel, or
  **Database: Connect to PostgreSQL, MySQL or SQL Server...**, asks for the host, port, user,
  database and password (shown as dots).
  The same tables, columns, keys, indexes, results, diagram and query plans follow (the plan
  is the server's `EXPLAIN`; nothing runs). Errors say which statement they're in. **Used
  before** keeps the connections, without passwords: the password is held only while the
  extension runs, and given to the client for each run in its environment (`PGPASSWORD`,
  `MYSQL_PWD`, `SQLCMDPASSWORD`), never in its arguments. A connection can have a **Name**
  of its own ("Shop, staging"). The clients are found on `PATH` or where their installers put
  them under Program Files.
- **SQL Server** goes through `sqlcmd` (Microsoft's Go version, or the one with SQL Server's
  tools). An empty user signs in with Windows. The host can be `(localdb)\MSSQLLocalDB`:
  the extension starts that LocalDB instance and finds its pipe. Each statement runs in a
  batch of its own, so `CREATE VIEW` and `CREATE PROCEDURE` work beside others; the plan is
  `SHOWPLAN_TEXT`. Line breaks and tabs inside values show as spaces.
- **Database: Redis...** (the **Redis** extension, in its panel; Redis, Valkey, Memurai,
  through `redis-cli`): the keys, found with
  `SCAN` and a pattern (`user:*`), each with its type, size and time to live; **Open** shows
  what it holds (a string to edit and save, a hash's fields, a list's items, a set's members,
  a sorted set's scores, a stream's entries). **Expires in** sets or removes a key's time to
  live, and **Delete…** asks first. The **console** runs any command and shows the reply as
  `redis-cli` does. The database number is chosen at the top, with how many keys each holds.
  The password goes to `redis-cli` in its environment (`REDISCLI_AUTH`) for each run and is
  never saved.
- **Database: MongoDB...** (the **MongoDB** extension, in its panel, through `mongosh`):
  databases and their collections (with about
  how many documents), and a collection's documents as a table, found with a **Filter**,
  **Sort** and **Fields** in the shell's own syntax (`{ price: { $gt: 2 } }`,
  `ObjectId("…")`), 200 at a time. **Open** on a document shows it as Extended JSON to edit
  and **Save**, or **Delete…**; **Add a document** adds one. **Script** runs any shell script
  in the chosen database and shows its result.

It runs through the `sqlite3` program (on `PATH`, or MSYS2's), in its safe mode, so a
`.sql` file can't reach the shell with dot-commands; servers through `psql`, the `mariadb`
or `mysql` client, `sqlcmd`, `redis-cli` and `mongosh` (on `PATH`, where their installers put
them, or unpacked into the config folder's `tools` folder). No database code is built
into the app, and it opens no connections of its own: the clients do. Results keep 10,000
rows each.

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

## Workspaces with more than one folder

- **Add Folder to Workspace…** (right click the project's row in the Explorer) puts another
  folder in the window, beside the project: its own tree in the Explorer, its files in Quick
  Open (under the folder's name, `shared/x.rs`) and in Find in Files. **Remove Folder from
  Workspace** on its row takes it out; the folder itself stays.
- They're kept in the project's `.coiman/settings.toml` as `[workspace] folders` (relative
  to the project when beside it, `../shared`), so the project opens with them next time.
- A VS Code **`.code-workspace`** file opens as one: its first folder as the project, the
  others added to it.
- **Source Control** has a **Repository** picker then: the project or one of the other
  folders, and Git (status, branch, commits, the Graph) follows the one chosen. A folder not
  trusted says so, with a button to trust it. Run and the language servers stay with the
  project's own folder.

## Settings, keybindings and themes

- **Settings** (`Ctrl+,`): TOML, layered defaults → user
  (`<config>\settings.toml`) → workspace (`<folder>/.coiman/settings.toml`).
  Files can `extends = "other.toml"` and have `[windows]` / `[macos]` / `[linux]` tables.
  Unknown keys and errors show in the status bar; saving a settings file applies it.
  Separate `editor.font_size` and `workbench.ui_font_size`, on top of `workbench.zoom`.
- **Keybindings** (`Ctrl+K Ctrl+S`): `<config>\keybindings.toml` with VS Code
  command IDs, two-key chords, `when = "editorTextFocus"`, and `-command` to remove a
  default. Menus and the palette show the active shortcuts. A project's own
  `<folder>/.coiman/keybindings.toml`, in the same form, goes over yours: its bindings win,
  and its `-command` entries take yours or the defaults away. Saving it applies it.
- **Environment variables** (`[env]` in settings): `RUST_LOG = "debug"` and the like, for
  terminals, Run and tasks. A project's `.coiman/settings.toml` adds to and overrides yours;
  `${NAME}` is a variable as Coiman was started with it (`PATH = "${PATH};C:/tools"`). In
  Restricted Mode only yours apply, so a folder not trusted can't set what a shell runs on
  start. Settings > Terminal > **Environment variables** edits them: each variable with its
  value, × to take one away, and a row to add one; with **This folder** chosen, the project's
  own.
- **Light or dark as the computer is set**: with no theme chosen, Coiman starts in Coiman
  Light, or in Coiman Dark when the computer is set to dark mode (Windows' app mode, macOS,
  and on Linux the window system, else GNOME's style, the GTK theme or KDE's color scheme;
  `GTK_THEME` too), and follows a change while it runs. Coiman Light is the icon's blue on
  white with Light Modern's code colors. **Follow system (light or dark)**, first in the theme
  picker, goes back to it after choosing a theme; a theme chosen by hand or by the schedule
  always wins. Settings from before this kept the Coiman Dark they had.
  `COIMAN_SYSTEM_THEME=light` or `dark` stands in for the computer's mode, to try it.
- **Color themes** (`Ctrl+K Ctrl+T`): any VS Code theme. In the picker, a theme the pointer
  rests on for a second is shown without being chosen; leaving brings the chosen one back. The picker lists themes from the
  installed VS Code (bundled and `~/.vscode/extensions`) and `<config>\themes`;
  `workbench.color_theme` also accepts a path to a theme `.json`. Theme files may use
  comments and `include`; token colors apply to tree-sitter and syntect highlighting.
- **The Welcome page greets you by name** ("Good afternoon, Manuel."): from
  `workbench.greeting_name`, else the first word of Git's `user.name`, else the account's
  name; `-` for no name.
- **The logo follows the theme**: Coiman's icon on the Welcome page, in the dock and as the
  window's icon (taskbar, Alt+Tab) takes the theme's accent color, its hexagon a deep shade
  of it. The accent is the first of the theme's focus border, active border, button and
  link colors that stands out from its background. Wayland keeps the launcher's icon: a
  window can't change it there.
- **Font ligatures** (`editor.font_ligatures`, on): with a code font that has them (Fira Code,
  JetBrains Mono, Cascadia Code, set in `editor.font_family`), `->`, `=>`, `!=`, `>=`, `===`,
  `<!--` and the rest are drawn as the font joins them. Each character keeps its cell, so the
  cursor and selections move through them one character at a time. Turned off, the font's
  characters are drawn each on its own. `editor.font_ligatures_off = ["==", "!="]` leaves
  those out (a longer one holding them, `===` or `!==`, goes too, so no half of a ligature is
  drawn).
- **Coiman High Contrast** and **Coiman High Contrast Light**: black on white or white on
  black, bright borders, and syntax colors chosen to stay readable.
- **Bundled themes**: Catppuccin (Mocha, Macchiato, Frappé, Latte), Dracula (and Soft),
  GitHub (nine), Gruvbox (six), Nord, One Dark Pro (five) and Tokyo Night (Night, Storm,
  Light), taken unchanged from their MIT-licensed extensions (`data/themes/LICENSES.md`).
- **File icon packs** (**Preferences: File Icon Theme**, `workbench.icon_theme`): Material
  Icon Theme (the default), Catppuccin (Latte's icons in light themes) and vscode-icons, all
  MIT. A VS Code icon theme whose icons are SVG files can be added from Open VSX.
- **Themes from Open VSX** (**Preferences: Get Themes from Open VSX**, or Settings > More
  themes): search open-vsx.org for color and icon themes and install them; **Use** applies
  one at once. `curl` does the fetching, so the app opens no connections; only the
  extension's themes are kept, never its code, snippets or languages (see PRIVACY.md).

## Installing and updating

Coiman never updates itself and opens no connection to look for updates. Each way of
installing it has its own updater outside the program, used when you ask for it
(`docs/PUBLISHING.md` for how a release reaches each one):

- **Windows setup program**: installs for one user or for everyone, with a Start menu entry
  and an Apps list entry. Running a newer version's setup replaces the older one in place and
  removes extensions it no longer ships; a running Coiman is asked to close first.
- **Scoop**: installed from the release's `coiman-ide.json` address, which always gives the
  newest version, so `scoop update coiman-ide` finds it.
- **AppImage**: AppImageUpdate downloads only the changed parts, using the `.zsync` file beside
  each release's AppImage.
- **Later**: winget, a signed APT and RPM repository on GitHub Pages, Flathub, the Snap Store
  and the AUR. The release workflow has them; each starts once its account and secret are set
  up (`docs/PUBLISHING.md`).
- **Zip, tar.gz and the package files**: install the newer file the same way.

Settings stay in `%APPDATA%\coiman-ide` or `~/.config/coiman-ide` through every update and
uninstall.
