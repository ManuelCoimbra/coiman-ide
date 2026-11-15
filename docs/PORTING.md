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
