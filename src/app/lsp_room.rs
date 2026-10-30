//! The Language Servers window: what each server is doing, how long it has
//! been at it, what it costs, and the controls to get it unstuck.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui::{self, RichText};
use rust_code_core::lsp::Client;
use rust_code_core::lsp::status::{Health, ServerStatus, format_bytes, format_duration, process_memory};
use serde_json::{Value, json};

use super::CodeApp;
use crate::theme;

/// Seconds between memory readings while the window is open.
const MEMORY_EVERY: f64 = 3.0;

#[derive(Default)]
pub(super) struct Room {
    pub(super) open: bool,
    /// Why a server last stopped, until it starts again.
    pub(super) last_stop: Option<String>,
    /// Latest reading by process id.
    memory: HashMap<u32, u64>,
    reading: Option<Receiver<HashMap<u32, u64>>>,
    next_reading: f64,
}

enum Action {
    Restart(usize),
    RestartAll,
    Request(usize, &'static str),
    Notify(usize, &'static str, Value),
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
        let pids: Vec<u32> = self.lsp.clients.iter().map(Client::pid).collect();
        let (tx, rx) = mpsc::channel();
        room.reading = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(process_memory(&pids));
        });
    }

    pub(super) fn lsp_room(&mut self, ctx: &egui::Context) {
        if !self.lsp.room.open {
            return;
        }
        self.pump_room_memory(ctx.input(|i| i.time));
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let mut open = true;
        let mut action = None;
        egui::Window::new("Language Servers").open(&mut open).collapsible(false).default_width(480.0).show(ctx, |ui| {
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
                    ui.label(self.lsp.room.memory.get(&client.pid()).map_or("…".to_owned(), |b| format_bytes(*b)));
                    ui.end_row();
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
                        action = Some(Action::Notify(i, "rust-analyzer/cancelFlycheck", json!({})));
                    }
                });
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
