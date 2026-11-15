# Porting Coiman IDE to another platform

A port (the Android build, say) shouldn't have to edit Coiman's source: an edit that finds its
place by exact text breaks when that text changes. Coiman offers these instead. They are kept
stable: a change to one is noted in the CHANGELOG.

## The library

`coiman-ide` is a library as well as the desktop program. `main.rs` only calls
`coiman_ide::run_desktop()`. A port uses:

| Item | What it does |
|---|---|
| `coiman_ide::CodeApp` | The app. `CodeApp::new(start_path, &ctx)` creates it; it is an `eframe::App`, so a port's own shell can draw it. |
| `coiman_ide::open_file(&mut app, path)` | Opens a file, as File > Open does. |
| `coiman_ide::open_folder(&mut app, path)` | Opens a project folder, as File > Open Folder does. |
| `coiman_ide::theme_colors()` | The current theme's colors, for a port's own widgets. |

## Hooks

Each is set once, before the app is created. Only the first call counts; it returns whether it
did. The desktop sets none.

| Hook | Use |
|---|---|
| `coiman_core::platform::set_command_router(Box<dyn Fn(&Path) -> Option<Command>>)` | Every program Coiman runs (git, cargo, compilers, language servers, formatters) is made by `quiet_command`. The router may return another command for a program: on Android, the shim that runs it inside Termux. `None` runs the program as it is. |
| `coiman_core::platform::set_tool_probe(Box<dyn Fn(&str) -> Option<PathBuf>>)` | `which` asks the probe first whether a program is there, for programs `PATH` can't show (Termux's folder belongs to another app). |
| `coiman_ide::hooks::set_status_bar_item(Box<dyn Fn(&mut egui::Ui)>)` | A widget at the right end of the status bar (Android's soft keyboard button). |
| `coiman_ide::hooks::set_text_tapped(Box<dyn Fn()>)` | Called when the editor or a terminal is clicked or tapped to type in (to bring the soft keyboard back). |
| `coiman_ide::hooks::set_file_picker(Box<dyn Fn(&PickRequest) -> Option<PathBuf>>)` | Every file dialog (open, save, choose a folder) asks the picker: on Android, the system's document picker. A `PickRequest` has the kind (file, folder, save), title, starting folder, file name and filters. On Android, Open File and Open Folder use it too, else Coiman's File Manager. |

## Published state

`coiman_core::selection::get()` says where the selected text is on screen, as
`[min.x, min.y, max.x, max.y]` in egui points, or `None`. The editor or terminal with the
keyboard publishes it each frame, and the frame starts with `None`. On Android the editor and the
terminal also store their selection as a `TextEditState` under their own id, where a platform
text bar looks for it.

## Already handled on Android

- No caption buttons (minimize, maximize, close), and no space kept for them: the system owns the
  window.
- A tap is a press and a release in one frame: the editor places the cursor on it, and the
  command palette (and Explorer's New File, Rename…) doesn't take the tap that opened it as a
  press outside, which would close it.
- No `trash` crate: Move to Trash says there's no Trash, rather than delete for good unasked.
- No `rfd` crate (it has no Android dialogs): file dialogs go to the port's picker, or show
  nothing without one.
- eframe uses the native Activity (`android-native-activity`), without `accesskit` (eframe has it
  on Android only with GameActivity).

CI checks the Android build with clippy on every push (the `android` job in `ci.yml`), so a
change that breaks it is caught here.

## Left to the port

- Its own eframe shell, renderer (wgpu, if it wants it rather than glow) and Activity.
- Its egui fork, if it uses one (fields of `IMEOutput` differ from crates.io egui).
- The terminal's shell: `coiman_core::terminal::pty` builds on Android, but the programs a
  terminal runs there are Termux's, which another app can't start directly.

## Moving the Android adapter over

For the APK builder's adapter (`adapters/coimanide` in artcraft-android), from Coiman d6920ad on.

**Its `prepare.sh` no longer runs as it is**: several of its edits look for text that has
changed, and it would write its own `lib.rs` over Coiman's. The edits below go; most of the
code they carried moves into the shell crate (`apps/coimanide-android`), set up through the hooks.

### What each edit becomes

| `prepare.sh` edit | Now |
|---|---|
| `[lib]` in `coiman-ide/Cargo.toml`, and the written `src/lib.rs` | Drop both. Coiman has its own `lib.rs`: `coiman_ide::CodeApp`, `open_file`, `open_folder` are there. |
| eframe line in `coiman-ide/Cargo.toml` (wgpu, native Activity, no accesskit) | Drop. Coiman turns on `android-native-activity` and leaves `accesskit` off on Android. The shell crate asks for `wgpu` on its own eframe dependency; Cargo joins the features, and `NativeOptions::renderer` picks wgpu. |
| `trash` stub in `[patch.crates-io]` | Drop. Coiman doesn't use `trash` on Android. |
| The `rfd` fork with `artcraft_saf` | Drop. Coiman doesn't use `rfd` on Android; set `hooks::set_file_picker` (below). |
| `selection.rs` copied into core, and `pub mod selection;` | Drop. `coiman_core::selection` is Coiman's, same `set`/`get`. |
| `android_termux.rs` in core | Move it into the shell crate (only the shell and the terminal use it). |
| `quiet_command` → Termux shim | Drop; `coiman_core::platform::set_command_router` (below). |
| `which` → Termux tools | Drop; `coiman_core::platform::set_tool_probe` (below). |
| Caption buttons hidden; their space given back | Drop. Coiman does both on Android. |
| Keyboard button in the status bar | Drop the edit; `hooks::set_status_bar_item`. `android_keyboard.rs` moves into the shell crate; its `theme::colors()` becomes `coiman_ide::theme_colors()`. |
| `text_tapped()` in the editor and the terminal | Drop; `hooks::set_text_tapped`. |
| Palette: ignore the press that opened it (both places) | Drop. Coiman skips the outside-press check once after opening. |
| Editor: a tap counts as a press | Drop. Coiman has it. |
| Editor and terminal: selection to the text bar (`selection::set` and `TextEditState`) | Drop. Coiman publishes both, the `TextEditState` on Android only. |
| `egui::text::cursor::CCursor` path fix | Drop (it fixed the dropped edits). |
| egui fork in `[patch]` | Keep, if the builder wants its fork. |
| Editor `IMEOutput` with the fork's fields | Keep: the fork's struct differs from crates.io egui's. |
| Terminal `IMEOutput` | Keep for now (Coiman's terminal publishes none). |
| `portable-pty` desktop-only, `pty_android.rs` in place of `pty.rs` | Keep for now. Coiman builds with `portable-pty` on Android, but its terminal runs Termux's programs, which need the shim. |
| Workspace member `apps/coimanide-android`, Activity, broker, shim, file associations, text bar | Keep: the adapter's own. |

### In the shell crate, before `CodeApp::new`

```rust
use coiman_core::platform;
use coiman_ide::hooks::{self, PickKind, PickRequest};

// Programs Termux has: run through the shim.
platform::set_command_router(Box::new(|program| {
    let tool = termux::tool_for(program)?;
    let mut cmd = std::process::Command::new(termux::shim()?);
    cmd.env("COIMAN_TERMUX_TOOL", tool);
    Some(cmd)
}));
// Termux's folder belongs to another app: say its tools are there.
platform::set_tool_probe(Box::new(|name| {
    termux::tool_for(std::path::Path::new(name)).map(std::path::PathBuf::from)
}));

hooks::set_status_bar_item(Box::new(android_keyboard::button));
hooks::set_text_tapped(Box::new(android_keyboard::text_tapped));

// The system's document picker, in place of the rfd fork's launcher.
hooks::set_file_picker(Box::new(move |request: &PickRequest| {
    let id = state.next_id();
    let answer = match request.kind {
        PickKind::File => bridge.pick_open_blocking(id, false, saf::PICK_TIMEOUT),
        PickKind::Save => bridge.pick_blocking(id, request.file_name.as_deref(), saf::PICK_TIMEOUT),
        PickKind::Folder => bridge.pick_folder_blocking(id, saf::PICK_TIMEOUT),
    };
    match answer {
        Ok(saf::PickAnswer::Paths(paths)) => {
            let path = std::path::PathBuf::from(paths.into_iter().next()?);
            if request.kind != PickKind::Folder {
                state.mirror.track(id, &path);
            }
            Some(path)
        }
        Ok(saf::PickAnswer::Cancelled) => None,
        Err(error) => {
            log::error!("Android file picker: {error}");
            None
        }
    }
}));
```

`termux`, `android_keyboard`, `bridge`, `state` and `saf` are the adapter's own, as in its
`src/lib.rs` today; the types may need adjusting to how they are declared there. Coiman's dialogs
pick one file at a time, so `multiple` is always false. With the picker set, Open File and Open
Folder use it too (else Coiman's File Manager chooses).
