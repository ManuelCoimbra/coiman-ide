# Roadmap

What's planned for Coiman IDE, in order. `docs/SPEC-COVERAGE.md` has each item of the
feature specification and where it stands; `docs/AUDIT.md` §11 has the phases.

Coiman opens no network connections of its own: where something needs the network, a program
of its own does it (`git`, `gh`, `cargo`, `npm`, `psql`…). AI works the same way: the Agents
panel drives the `opencode` program, if it is installed, and asks before the agent acts.

## Now

- Picking up from "Next" below.

## Done lately

- Git Tools moved out of the program into an extension (Git pack); plugins got forms.
- Code Health and the project overview moved out of the program into an extension.
- Architecture moved out of the program into an extension (Code analysis pack).
- Packs: every extra in a pack by kind of work, installed only when wanted.
- Class diagrams from the code, as an extension that comes with Coiman (UML, 13 languages, SVG/Mermaid/PlantUML).
- Extensions that come with Coiman, installed only when wanted; the first, Terminal Commands: history, Ctrl+Space suggestions, project commands.
- Git Tools: history search (by text and by change), submodules, hooks, `.gitignore` rules.
- OpenAPI and Swagger specs: endpoints, models, each endpoint as a `.http` request.
- An HTTP client: `.http` files, environments, named responses, history and comparison, through curl.
- A serial monitor: text, hex and plot views, sending, DTR/RTS (tested with a pipe standing in for a board).
- SQL Server (with LocalDB), Redis and MongoDB, each through its own command-line client.
- Shaders: GLSL, HLSL and WGSL colored; GLSL and WGSL problems while typing; a live preview.
- Binary size: groups by crate or namespace, symbols, sections, flash and RAM.
- Agents: MCP Servers: the tools opencode gives the agent, for the project or every project.
- The Agents panel (a contributor's): chats with the `opencode` program.
- Jupyter notebooks: cells, a Python kernel, outputs and plots.
- Android: devices, logcat (filters, links to the project's frames), shell and APK installs through adb.
- Docker and Compose window (tested with a stand-in docker: no engine here).
- PlantUML preview (the user's PlantUML draws it).
- Remote folders: a command's errors there link to the files here.
- SSH remote folders: deletes and renames go there; a terminal and commands in the folder.
- SSH remote folders: mirrored here, saves sent back over ssh.
- A function's flowchart (Architecture: Show Flow of Function at Cursor).
- Session statistics: active time per project, kept locally.
- Project: Generate Overview.
- Search the project's documentation (Find in Files' Doc switch).
- Code Health lists public items without a doc comment.
- Plugin templates and dev mode.
- Remote debugging: gdbserver over ssh, or one already running.
- Breakpoint groups.
- Tests at once: one setting for every test runner.
- Snapshot testing: Review Snapshots (insta), Update Snapshots (Jest, Vitest).
- The Testing view's timeline.
- Multi-root: Git follows the folder chosen in Source Control.
- An editor for `[env]` in Settings; named database connections; profiler times.
- Workspaces with more than one folder (Explorer, Quick Open, Find in Files,
  `.code-workspace` files).
- Git worktrees, and a GitHub Actions view (through `gh`).
- The Welcome page greets by name.
- The Dock layout's island gaps, even on both sides.

## Extensions, not built in

Everything beyond editing is in packs (see `docs/FEATURES.md`, Packs), installed only when
wanted. New features come as extensions in a pack. The features still built into the program
move into their pack's extensions one at a time, as the plugin API grows to carry them, so the
program itself gets smaller too:

1. Code analysis: the dependency graph (Architecture) and Code Health with the overview are
   done; left: session statistics, the profiler, coverage, run history, stack traces, rules
   and search by syntax tree, and the function flow (it needs the editor's parser).
2. Git, more of it: Git Tools is done; left: Git Graph (it needs a commit-lane graph).
3. Web and APIs: the HTTP client and OpenAPI (they run curl and read files: panels and jobs).
4. Databases, Containers, Embedded, Mobile: once the API has forms and tables to edit.
5. Graphics and Agents last: shader previews and the agents panel need the most from the host.

## Next, nothing to download

From the items `docs/SPEC-COVERAGE.md` still has as missing, in order: each one covers
several of them, and none needs a download (the network goes through `curl` and `git`).

1. Embedded: a memory map from the ELF and the linker map (what sits where in flash and RAM,
   by section and symbol), and firmware section analysis beside Binary Size.
2. Databases: compare two schemas (tables, columns, keys, indexes) and write the SQL that
   turns one into the other; migrations (sqlx, Diesel, Alembic, Flyway folders) listed with
   which have run.
3. Security checks in Code Health: SQL built from strings (injection), HTML from user input
   (XSS), unsafe memory calls in C and C++ (`strcpy`, `gets`, `sprintf`), and `unsafe`
   blocks in Rust, each with where and why.
4. Technical debt: `TODO`, `FIXME` and `HACK` over time (from history), by area and age.

## Later, each needing a download first

Downloads are asked for first, with the file, where it comes from and its size.

- Nothing left here for now.

## Releases

Releases gather several of these at a time; what's done since the last one is under
"Unreleased" in `CHANGELOG.md`.
