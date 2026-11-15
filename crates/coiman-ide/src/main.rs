//! Coiman IDE — a lightweight VS Code–style editor built with egui.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    coiman_ide::run_desktop()
}
