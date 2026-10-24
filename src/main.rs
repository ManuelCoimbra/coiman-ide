//! Rust Code — a lightweight VS Code–style editor built with egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor_view;
mod explorer;
mod keymap;
mod palette;
mod search_panel;
mod terminal;
mod theme;

use std::path::PathBuf;

// Core modules, re-exported so `crate::buffer::...` paths keep working.
pub(crate) use rust_code_core::{buffer, commands, document, edit_ops, finder, session, settings, syntax, watcher};

fn main() -> eframe::Result {
    // Optional first argument: a folder (opened in the explorer) or a file.
    let start_path = std::env::args().nth(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Rust Code")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Rust Code",
        options,
        Box::new(move |cc| Ok(Box::new(app::CodeApp::new(start_path, &cc.egui_ctx)))),
    )
}
