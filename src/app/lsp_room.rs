//! The Language Servers window: what each server is doing, how long it has
//! been at it, what it costs, and the controls to get it unstuck.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, RichText};
use rust_code_core::lsp::Client;
use rust_code_core::lsp::servers::{ServerKind, rust_analyzer_options};
use rust_code_core::lsp::status::{
    Health, ServerStatus, build_folder_sizes, format_bytes, format_duration, process_memory,
};
use serde_json::{Value, json};

use super::CodeApp;
use crate::theme;

/// Seconds between memory readings while the window is open.
const MEMORY_EVERY: f64 = 3.0;
/// Seconds between measuring the build folders while the window is open.
const SIZES_EVERY: f64 = 30.0;
/// rust-analyzer's memory presets: setting, name, what it does.
const MEMORY_PRESETS: [(&str, &str, &str); 3] = [
    ("default", "Default", "rust-analyzer's own defaults."),
    ("low", "Low", "Keeps fewer syntax trees and doesn't warm its caches on load, so first answers are slower."),
    (
        "minimal",
        "Minimal",
        "Also skips build scripts and proc macros, so code that depends on them shows false errors.",
    ),
];

#[derive(Default)]
pub(super) struct Room {
    pub(super) open: bool,
    /// Why a server last stopped, until it starts again.
    pub(super) last_stop: Option<String>,
    /// Latest reading by process id.
    memory: HashMap<u32, u64>,
    reading: Option<Receiver<HashMap<u32, u64>>>,
    next_reading: f64,
    /// Target folder bytes and rust-analyzer's share, by workspace.
    sizes: HashMap<PathBuf, (u64, u64)>,
    sizing: Option<Receiver<HashMap<PathBuf, (u64, u64)>>>,
    next_sizing: f64,
}

enum Action {
    Restart(usize),
    RestartAll,
    Request(usize, &'static str),
    Notify(usize, &'static str, Value),
    Memory(&'static str),
    SeparateTarget(bool),
}

/// What holds the build folder, as far as this app can tell.
enum Holder {
    /// rust-analyzer is running a check, which can be cancelled.
    Check,
    /// rust-analyzer is building (build scripts, proc macros).
    Build,
    Unknown,
}

fn state_of(client: &Client, working: bool) -> &'static str {
    match client.status.as_ref() {
        _ if !client.is_ready() => "Starting",
        Some(ServerStatus { health: Health::Error, .. }) => "Error",
        Some(ServerStatus { health: Health::Warning, .. }) => "Warning",
        Some(ServerStatus { quiescent: false, .. }) => "Working",
        _ if working => "Working",
        _ => "Ready",
    }
}

impl CodeApp {
    pub(super) fn show_lsp_room(&mut self) {
        self.lsp.room.open = true;
    }

    /// Reads the servers' memory in the background, every few seconds.
    fn pump_room_memory(&mut self, now: f64) {
        let room = &mut self.lsp.room;
        if let Some(rx) = &room.reading {
            match rx.try_recv() {
                Ok(memory) => {
                    room.memory = memory;
                    room.reading = None;
                }
                Err(TryRecvError::Disconnected) => room.reading = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if room.reading.is_some() || now < room.next_reading {
            return;
        }
        room.next_reading = now + MEMORY_EVERY;
        let pids: Vec<u32> = self.lsp.clients.iter().filter_map(Client::pid).collect();
        let (tx, rx) = mpsc::channel();
        room.reading = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(process_memory(&pids));
        });
    }

    /// Measures the Rust workspaces' build folders in the background.
    fn pump_room_sizes(&mut self, now: f64) {
        let room = &mut self.lsp.room;
        if let Some(rx) = &room.sizing {
            match rx.try_recv() {
                Ok(sizes) => {
                    room.sizes = sizes;
                    room.sizing = None;
                }
                Err(TryRecvError::Disconnected) => room.sizing = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if room.sizing.is_some() || now < room.next_sizing {
            return;
        }
        room.next_sizing = now + SIZES_EVERY;
        let rust = self.lsp.clients.iter().filter(|c| c.kind == ServerKind::RustAnalyzer);
        let roots: Vec<PathBuf> = rust.map(|c| c.root.clone()).collect();
        let (tx, rx) = mpsc::channel();
        room.sizing = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(roots.into_iter().map(|r| (r.clone(), build_folder_sizes(&r))).collect());
        });
    }

    pub(super) fn rust_analyzer_settings(&self) -> Value {
        let (cargo, lsp) = (&self.settings.cargo, &self.settings.lsp);
        rust_analyzer_options(&cargo.check_command, &lsp.rust_analyzer_memory, lsp.rust_analyzer_separate_target)
    }

    pub(super) fn restart_rust_analyzer(&mut self) {
        while let Some(i) = self.lsp.clients.iter().position(|c| c.kind == ServerKind::RustAnalyzer) {
            self.restart_client(i);
        }
    }

    /// Saves a rust-analyzer setting; reloading settings restarts the server.
    fn set_rust_analyzer_setting(&mut self, ctx: &egui::Context, key: &str, value: &str) {
        let before = self.settings.lsp.clone();
        match self.write_user_setting(ctx, "lsp", key, value) {
            Ok(()) if self.settings.lsp == before => {
                self.status = Some("Saved, but workspace settings override it".to_owned());
            }
            Ok(()) => {}
            Err(e) => self.status = Some(e),
        }
    }

    fn set_separate_target(&mut self, ctx: &egui::Context, on: bool) {
        self.set_rust_analyzer_setting(ctx, "rust_analyzer_separate_target", if on { "true" } else { "false" });
    }

    fn lock_holder(&self) -> Holder {
        // With its own folder, rust-analyzer can't be the one in the way.
        if self.settings.lsp.rust_analyzer_separate_target
            || !self.lsp.clients.iter().any(|c| c.kind == ServerKind::RustAnalyzer)
        {
            return Holder::Unknown;
        }
        let titles = || self.lsp.progress.values().map(|t| t.0.to_lowercase());
        if titles().any(|t| t.contains("check") || t.contains("clippy")) {
            Holder::Check
        } else if titles().any(|t| t.contains("build")) {
            Holder::Build
        } else {
            Holder::Unknown
        }
    }

    /// Says what the running cargo job is waiting for, and offers a way out.
    pub(super) fn lock_bar(&mut self, ui: &mut egui::Ui) {
        if !self.cargo.lock_wait {
            return;
        }
        let holder = self.lock_holder();
        let message = match holder {
            Holder::Check => "cargo is waiting for the build folder: rust-analyzer's check is using it.",
            Holder::Build => "cargo is waiting for the build folder: rust-analyzer is building with it.",
            Holder::Unknown => "cargo is waiting for the build folder: another cargo is using it, maybe in a terminal.",
        };
        let (mut cancel, mut separate) = (false, false);
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(message).color(theme::colors().modified));
                cancel = matches!(holder, Holder::Check) && ui.button("Cancel Check").clicked();
                if !matches!(holder, Holder::Unknown) {
                    separate = ui
                        .button("Give rust-analyzer Its Own Build Folder")
                        .on_hover_text("No more waiting, but its build output takes extra disk space.")
                        .clicked();
                }
            });
        });
        if cancel {
            for client in self.lsp.clients.iter_mut().filter(|c| c.kind == ServerKind::RustAnalyzer) {
                client.notify("rust-analyzer/cancelFlycheck", Value::Null);
            }
        }
        if separate {
            self.set_separate_target(&ui.ctx().clone(), true);
        }
    }

    pub(super) fn lsp_room(&mut self, ctx: &egui::Context) {
        if !self.lsp.room.open {
            return;
        }
        self.pump_room_memory(ctx.input(|i| i.time));
        self.pump_room_sizes(ctx.input(|i| i.time));
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let mut open = !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut action = None;
        let window = egui::Window::new("Language Servers").open(&mut open).collapsible(false).default_width(480.0);
        window.default_pos(theme::window_spot(ctx, 480.0)).show(ctx, |ui| {
            let dim = theme::colors().text_dim;
            if self.lsp.clients.is_empty() {
                ui.label(RichText::new("No language server is running.").color(dim));
            }
            if let Some(why) = &self.lsp.room.last_stop {
                ui.label(RichText::new(why).color(theme::colors().modified));
                if ui.button("Start Again").clicked() {
                    action = Some(Action::RestartAll);
                }
            }
            let working = !self.lsp.progress.is_empty();
            for (i, client) in self.lsp.clients.iter().enumerate() {
                let root = client.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.heading(format!("{}: {root}", client.kind.name()));
                let running = format_duration(client.started.elapsed().as_secs());
                egui::Grid::new(("lsp_room", i)).num_columns(2).show(ui, |ui| {
                    ui.label(RichText::new("State").color(dim));
                    ui.label(state_of(client, working));
                    ui.end_row();
                    ui.label(RichText::new("Running for").color(dim));
                    ui.label(running);
                    ui.end_row();
                    ui.label(RichText::new("Memory").color(dim));
                    ui.label(
                        client
                            .pid()
                            .and_then(|pid| self.lsp.room.memory.get(&pid))
                            .map_or("…".to_owned(), |b| format_bytes(*b)),
                    );
                    ui.end_row();
                    if let Some((total, own)) = self.lsp.room.sizes.get(&client.root).filter(|s| s.0 > 0) {
                        ui.label(RichText::new("Build folder").color(dim));
                        ui.label(if *own > 0 {
                            format!("{}, of which rust-analyzer's own {}", format_bytes(*total), format_bytes(*own))
                        } else {
                            format_bytes(*total)
                        });
                        ui.end_row();
                    }
                    if let Some(message) = client.status.as_ref().and_then(|s| s.message.as_ref()) {
                        ui.label(RichText::new("Message").color(dim));
                        ui.add(egui::Label::new(message.as_str()).wrap());
                        ui.end_row();
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Restart").clicked() {
                        action = Some(Action::Restart(i));
                    }
                    if client.kind.name() != "rust-analyzer" {
                        return;
                    }
                    if ui.button("Reload Workspace").clicked() {
                        action = Some(Action::Request(i, "rust-analyzer/reloadWorkspace"));
                    }
                    if ui.button("Rebuild Proc Macros").clicked() {
                        action = Some(Action::Request(i, "rust-analyzer/rebuildProcMacros"));
                    }
                    if ui.button("Run Check").clicked() {
                        let params = json!({ "textDocument": null });
                        action = Some(Action::Notify(i, "rust-analyzer/runFlycheck", params));
                    }
                    if ui.button("Cancel Check").clicked() {
                        action = Some(Action::Notify(i, "rust-analyzer/cancelFlycheck", Value::Null));
                    }
                });
                if client.kind == ServerKind::RustAnalyzer {
                    let memory = self.settings.lsp.rust_analyzer_memory.as_str();
                    let shown = MEMORY_PRESETS.iter().find(|p| p.0 == memory).map_or("Default", |p| p.1);
                    egui::ComboBox::new(("lsp_room_memory", i), "Memory use").selected_text(shown).show_ui(ui, |ui| {
                        for (value, name, what) in MEMORY_PRESETS {
                            if ui.selectable_label(value == memory, name).on_hover_text(what).clicked() {
                                action = Some(Action::Memory(value));
                            }
                        }
                    });
                    let mut separate = self.settings.lsp.rust_analyzer_separate_target;
                    let toggled = ui.checkbox(&mut separate, "Own build folder").on_hover_text(
                        "Cargo no longer waits for rust-analyzer's checks, but build output is duplicated.",
                    );
                    if toggled.changed() {
                        action = Some(Action::SeparateTarget(separate));
                    }
                }
                ui.add_space(8.0);
            }
            if working {
                ui.separator();
                let mut tasks: Vec<_> = self.lsp.progress.values().collect();
                tasks.sort_by_key(|t| t.2);
                for (title, message, started) in tasks {
                    let took = format_duration(started.elapsed().as_secs());
                    ui.label(format!("{took}  {title} {message}"));
                }
            }
        });
        self.lsp.room.open = open;
        match action {
            Some(Action::Restart(i)) => self.restart_client(i),
            Some(Action::RestartAll) => self.restart_lsp(),
            Some(Action::Request(i, method)) => {
                if let Some(client) = self.lsp.clients.get_mut(i) {
                    client.request(method, Value::Null);
                    self.status = Some(format!("Sent {method}"));
                }
            }
            Some(Action::Notify(i, method, params)) => {
                if let Some(client) = self.lsp.clients.get_mut(i) {
                    client.notify(method, params);
                    self.status = Some(format!("Sent {method}"));
                }
            }
            Some(Action::Memory(value)) => {
                let value = toml::Value::String(value.to_owned()).to_string();
                self.set_rust_analyzer_setting(ctx, "rust_analyzer_memory", &value);
            }
            Some(Action::SeparateTarget(on)) => self.set_separate_target(ctx, on),
            None => {}
        }
    }

    /// Stops one server; it starts again on the next frame, after a new search.
    fn restart_client(&mut self, i: usize) {
        if i >= self.lsp.clients.len() {
            return;
        }
        let client = self.lsp.clients.remove(i);
        let (kind, root) = (client.kind, client.root.clone());
        drop(client);
        self.lsp.pending.retain(|(k, r, _), _| !(*k == kind && *r == root));
        self.lsp.synced.clear();
        self.lsp.progress.clear();
        self.lsp.setups.remove(&kind);
        self.status = Some(format!("Restarting {}", kind.name()));
    }
}
