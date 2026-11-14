//! Coiman IDE — a lightweight VS Code–style editor built with egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod ui;

use std::path::PathBuf;

// Core modules, re-exported so `crate::buffer::...` paths keep working.
pub(crate) use coiman_core::{
    editor::buffer, editor::commands, editor::document, editor::edit_ops, language::syntax, project::finder,
    project::session, project::watcher, settings,
};

/// The window's icon (title bar, taskbar, Alt+Tab), from the icon's 256-pixel picture.
fn app_icon() -> egui::IconData {
    let png = include_bytes!("../../../assets/icon/coiman-256.png");
    match image::load_from_memory(png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (width, height) = rgba.dimensions();
            egui::IconData { rgba: rgba.into_raw(), width, height }
        }
        Err(_) => egui::IconData::default(),
    }
}

/// Under Wayland the taskbar shows the icon of the launcher file named after the app id. Run
/// from the tar.gz, an AppImage or a build there is none, so one is written to
/// `~/.local/share` (see `coiman_core::desktop_entry`). Packages bring their own.
#[cfg(target_os = "linux")]
fn launcher_entry() {
    use coiman_core::desktop_entry::{self, DataDirs, Icons};
    if std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some() {
        return;
    }
    // An AppImage runs from a temporary mount; the launcher has to start the AppImage itself.
    let program = match std::env::var_os("APPIMAGE") {
        Some(image) => PathBuf::from(image),
        None => match std::env::current_exe() {
            Ok(exe) => exe,
            Err(_) => return,
        },
    };
    let Some(dirs) = DataDirs::from_env() else { return };
    let icons = Icons {
        png: &[
            (48, include_bytes!("../../../assets/icon/coiman-48.png")),
            (128, include_bytes!("../../../assets/icon/coiman-128.png")),
            (256, include_bytes!("../../../assets/icon/coiman-256.png")),
        ],
        svg: include_bytes!("../../../assets/icon/coiman.svg"),
    };
    match desktop_entry::ensure(&dirs, &program, &icons) {
        Ok(true) => {
            coiman_core::applog::info(&format!("wrote the launcher entry for {}", program.display()));
            // KDE reads launchers from its cache; have it notice the new one.
            for tool in ["kbuildsycoca6", "kbuildsycoca5"] {
                if std::process::Command::new(tool)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .is_ok()
                {
                    break;
                }
            }
        }
        Ok(false) => {}
        Err(e) => coiman_core::applog::warn(&format!("couldn't write the launcher entry: {e}")),
    }
}

fn main() -> eframe::Result {
    let started = std::time::Instant::now();
    // MinGW installed but not on PATH (the usual for a course's setup): add its folder,
    // so Run's make finds gcc, and gdb and the programs it debugs find their DLLs.
    let mingw = coiman_core::build::c_build::mingw_dir_off_path();
    if let Some(dir) = &mingw
        && let Some(path) = coiman_core::build::c_build::path_with(&std::env::var_os("PATH").unwrap_or_default(), dir)
    {
        // SAFETY: first thing in main, before any thread starts.
        unsafe { std::env::set_var("PATH", path) };
    }
    let adopted = coiman_core::settings::adopt_old_config();
    coiman_core::applog::start();
    if let Some(what) = adopted {
        coiman_core::applog::info(&what);
    }
    if let Some(dir) = mingw {
        coiman_core::applog::info(&format!("MinGW not on PATH: added {}", dir.display()));
    }
    // Proves the crash log in the real app.
    let test_panic = std::env::var("COIMAN_TEST_PANIC").ok();
    if test_panic.as_deref() == Some("main") {
        panic!("COIMAN_TEST_PANIC was set to main");
    }
    if test_panic.is_some() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            panic!("COIMAN_TEST_PANIC was set");
        });
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("--version" | "-V")) {
        println!("coiman-ide {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // `--measure-startup <file>`: write the time to the first frame to <file>
    // and exit (used by scripts/bench.ps1).
    let startup_probe = match args.first().map(String::as_str) {
        Some("--measure-startup") if args.len() >= 2 => {
            let file = PathBuf::from(args.remove(1));
            args.remove(0);
            Some((started, file))
        }
        _ => None,
    };
    // Optional argument: a folder (opened in the explorer) or a file.
    let start_path = args.first().map(PathBuf::from);

    #[cfg(target_os = "linux")]
    launcher_entry();
    let saved_window = session::load().and_then(|s| s.window);
    let size = saved_window.map_or([1280.0, 800.0], |w| [w.width, w.height]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Coiman IDE")
            // What Linux desktops match against the launcher (its StartupWMClass) and the
            // Wayland app id, so the window gets the launcher's icon and grouping.
            .with_app_id(coiman_core::desktop_entry::app_id(std::env::var("SNAP_INSTANCE_NAME").ok().as_deref()))
            .with_icon(std::sync::Arc::new(app_icon()))
            .with_inner_size(size)
            .with_maximized(saved_window.is_some_and(|w| w.maximized))
            .with_min_inner_size([640.0, 400.0])
            // The app draws its own title bar (menus, title, window buttons).
            .with_decorations(false),
        ..Default::default()
    };

    let result = eframe::run_native(
        "Coiman IDE",
        options,
        Box::new(move |cc| {
            // Time until here is window + GPU context creation inside eframe.
            let window_ready = started.elapsed();
            let mut app = app::CodeApp::new(start_path, &cc.egui_ctx);
            // How long starting took, step by step, for a slow start to be told apart.
            coiman_core::applog::info(&format!(
                "startup: window ready after {} ms, app ready after {} ms",
                window_ready.as_millis(),
                started.elapsed().as_millis()
            ));
            app.first_frame_from = Some(started);
            app.startup_probe = startup_probe.map(|(t, file)| (t, window_ready, file));
            Ok(Box::new(app))
        }),
    );
    coiman_core::applog::close();
    result
}
