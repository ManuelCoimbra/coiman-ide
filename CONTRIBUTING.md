# Contributing

Thanks for helping. A few rules keep the editor fast, private and easy to change.

## Setup

Rust stable (edition 2024). On Windows the GNU toolchain (`stable-x86_64-pc-windows-gnu`) is
what the project is built and tested with; MinGW's `windres` puts the icon into the program.

```
cargo run --release -- [folder-or-file]
```

## Before a pull request

```
powershell -File scripts/check.ps1
```

It runs, and CI runs the same:

- no networking crates in the dependency tree (see [PRIVACY.md](PRIVACY.md)),
- `cargo fmt --all -- --check`,
- `cargo lint` (clippy, warnings are errors),
- `cargo test --workspace`.

## Where code goes

- Anything that works without a window goes in `crates/coiman-core`, with unit tests. The app
  crate only draws and wires. [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) lists the areas.
- A new feature in the app is a file under `crates/coiman-ide/src/app/<area>/` adding methods to
  `CodeApp`, declared in that area's `mod.rs` and in the `use` list in `app/mod.rs`.
- Built-in plugins are WebAssembly, in `plugins/built-in/`; rebuild them with
  `scripts/build-plugins.ps1`.

## Style

- `rustfmt.toml` sets the format.
- Comments and docs in plain English: short sentences, saying what and why.
- No telemetry and no network access from the app itself. Downloads (language servers,
  debuggers) only ever happen after asking, through tools the user already has.
- Never commit secrets, personal credentials or local configuration.

## Commits

One change per commit, with a subject saying what it does in the imperative ("Add ...", "Fix ...").
Add what you changed to [CHANGELOG.md](CHANGELOG.md) under Unreleased.
