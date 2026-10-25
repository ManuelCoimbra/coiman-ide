//! Rust Code — a lightweight VS Code–style editor built with egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor_view;
mod explorer;
mod icons;
mod keymap;
mod lsp_ui;
mod palette;
mod search_panel;
mod terminal;
mod theme;

use std::path::PathBuf;

// Core modules, re-exported so `crate::buffer::...` paths keep working.
pub(crate) use rust_code_core::{buffer, commands, document, edit_ops, finder, session, settings, syntax, watcher};

fn main() -> eframe::Result {
    let started = std::time::Instant::now();
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rust Code")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0])
            // The app draws its own title bar (menus, title, window buttons).
            .with_decorations(false),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Code",
        options,
        Box::new(move |cc| {
            // Time until here is window + GPU context creation inside eframe.
            let window_ready = started.elapsed();
            let mut app = app::CodeApp::new(start_path, &cc.egui_ctx);
            app.startup_probe = startup_probe.map(|(t, file)| (t, window_ready, file));
            Ok(Box::new(app))
        }),
    )
}
