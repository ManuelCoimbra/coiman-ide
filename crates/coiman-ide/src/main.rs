//! Coiman IDE — a lightweight VS Code–style editor built with egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor_view;
mod explorer;
mod grid_view;
mod icons;
mod inspector;
mod keymap;
mod lsp_ui;
mod palette;
mod search_panel;
mod terminal;
mod theme;

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
    let test_panic = std::env::var("RUST_CODE_TEST_PANIC").ok();
    if test_panic.as_deref() == Some("main") {
        panic!("RUST_CODE_TEST_PANIC was set to main");
    }
    if test_panic.is_some() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            panic!("RUST_CODE_TEST_PANIC was set");
        });
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
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

    let saved_window = session::load().and_then(|s| s.window);
    let size = saved_window.map_or([1280.0, 800.0], |w| [w.width, w.height]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Coiman IDE")
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
            app.startup_probe = startup_probe.map(|(t, file)| (t, window_ready, file));
            Ok(Box::new(app))
        }),
    );
    coiman_core::applog::close();
    result
}
