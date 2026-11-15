# Roadmap

What's planned for Coiman IDE, in order. `docs/SPEC-COVERAGE.md` has each item of the
feature specification and where it stands; `docs/AUDIT.md` §11 has the phases.

Coiman has no AI features, and opens no network connections of its own: where something needs
the network, a program of its own does it (`git`, `gh`, `cargo`, `npm`, `psql`…).

## Now

- Picking up from "Next" below.

## Done lately

- Multi-root: Git follows the folder chosen in Source Control.
- An editor for `[env]` in Settings; named database connections; profiler times.
- Workspaces with more than one folder (Explorer, Quick Open, Find in Files,
  `.code-workspace` files).
- Git worktrees, and a GitHub Actions view (through `gh`).
- The Welcome page greets by name.
- The Dock layout's island gaps, even on both sides.

## Next, nothing to download

- **Testing**
  - A test timeline.
  - Snapshot testing (insta, Jest).
  - Managing parallel test runs.
- **Debugging**
  - Breakpoint groups.
  - Remote debugging through `gdbserver` over `ssh`.
- **Plugins:** templates for a new plugin, and a dev mode that reloads it as it's edited.
- **SSH remote projects**, through `ssh`.
- **Experimental:** a project overview, flow views, session statistics.
- **Documentation**
  - Search across the project's documentation.
  - Public functions without a doc comment.
  - PlantUML preview.

## Later, each needing a download first

Downloads are asked for first, with the file, where it comes from and its size.

- Docker and Compose.
- ADB and logcat, for Android devices.
- Notebooks (Jupyter).
- A serial monitor, and binary size, for embedded work.
- Shaders.
- SQL Server, MongoDB and Redis.

## Releases

Releases gather several of these at a time; what's done since the last one is under
"Unreleased" in `CHANGELOG.md`.
