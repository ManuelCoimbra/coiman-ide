# Roadmap

What's planned for Coiman IDE, in order. `docs/SPEC-COVERAGE.md` has each item of the
feature specification and where it stands; `docs/AUDIT.md` §11 has the phases.

Coiman has no AI features, and opens no network connections of its own: where something needs
the network, a program of its own does it (`git`, `gh`, `cargo`, `npm`, `psql`…).

## Now

- Picking up from "Next" below.

## Done lately

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

- **SSH remote projects**, through `ssh` (started: an ssh terminal to a host from `~/.ssh/config`;
  next, the remote folder in the Explorer).
- **Experimental:** a project overview, flow views, session statistics.
- **Documentation**
  - Search across the project's documentation.
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
