# Roadmap

What's planned for Coiman IDE, in order. `docs/SPEC-COVERAGE.md` has each item of the
feature specification and where it stands; `docs/AUDIT.md` §11 has the phases.

Coiman opens no network connections of its own: where something needs the network, a program
of its own does it (`git`, `gh`, `cargo`, `npm`, `psql`…). AI works the same way: the Agents
panel drives the `opencode` program, if it is installed, and asks before the agent acts.

## Now

- Picking up from "Next" below.

## Done lately

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

## Next, nothing to download

- Nothing left: what's next is under "Later", each needing a download first.

## Later, each needing a download first

Downloads are asked for first, with the file, where it comes from and its size.

- A serial monitor, and binary size, for embedded work.
- Shaders.
- SQL Server, MongoDB and Redis.

## Releases

Releases gather several of these at a time; what's done since the last one is under
"Unreleased" in `CHANGELOG.md`.
