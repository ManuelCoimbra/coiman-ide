//! Language: Doctor. Says, for the active file, what works and what doesn't and
//! why, because a missing piece usually just looks like the editor being unhelpful.

use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};
use rust_code_core::lsp::servers::{self, Found, ServerKind};
use rust_code_core::lsp::{language_id, registry};
use rust_code_core::{run, syntax, trust};

use super::lsp::Setup;
use super::{CodeApp, repaint};

#[derive(Clone, Copy, PartialEq)]
enum Health {
    Good,
    Note,
    Bad,
}

struct Row {
    label: &'static str,
    text: String,
    health: Health,
}

#[derive(Default)]
pub(super) struct Doctor {
    open: bool,
    /// Looking for the server can run it, so it happens away from the UI.
    search: Option<Receiver<Found>>,
    found: Option<Found>,
}

impl CodeApp {
    pub(super) fn show_doctor(&mut self, ctx: &egui::Context) {
        self.doctor.open = true;
        self.look_for_server(ctx);
    }

    fn look_for_server(&mut self, ctx: &egui::Context) {
        self.doctor.found = None;
        self.doctor.search = None;
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(kind) = language_id(&doc.extension()).and_then(ServerKind::for_language) else { return };
        let configured = self.configured_server(kind);
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(servers::find(kind, &configured));
            wake();
        });
        self.doctor.search = Some(rx);
    }

    fn doctor_rows(&self) -> (String, Vec<Row>) {
        let row = |label, text: String, health| Row { label, text, health };
        let Some(doc) = self.docs.get(self.active) else {
            return ("No file".to_owned(), vec![row("File", "Open a file to check it".to_owned(), Health::Note)]);
        };
        let ext = doc.extension();
        let mut rows = Vec::new();
        let entry = registry::by_extension(&ext, false);
        let user = registry::by_extension(&ext, true);
        let name = user.map_or_else(|| doc.language_name(), |l| l.name).to_owned();
        let defined = match (user, entry, language_id(&ext)) {
            (Some(_), ..) => "Your languages.toml".to_owned(),
            (None, Some(l), Some(id)) if id == l.language_id => "Shipped data entry".to_owned(),
            (None, _, Some(_)) => "Built into the editor".to_owned(),
            (None, _, None) => "Not known: add it to languages.toml".to_owned(),
        };
        let known = language_id(&ext).is_some();
        rows.push(row("Defined by", defined, if known { Health::Good } else { Health::Note }));
        let colors = syntax::Lang::from_extension(&ext).is_some();
        rows.push(row(
            "Colors",
            if colors { "Syntax highlighting".to_owned() } else { "No grammar for this type".to_owned() },
            if colors { Health::Good } else { Health::Note },
        ));

        match language_id(&ext).and_then(ServerKind::for_language) {
            None => rows.push(row("Server", "None configured for this type".to_owned(), Health::Note)),
            Some(kind) => {
                rows.push(row("Server", kind.name().to_owned(), Health::Good));
                let hint = if let ServerKind::Custom(i) = kind { registry::get(i).and_then(|l| l.hint) } else { None };
                match (&self.doctor.search, &self.doctor.found) {
                    (Some(_), _) => rows.push(row("Found", "Looking…".to_owned(), Health::Note)),
                    (None, Some(Found::Server(launch))) => {
                        let text = match &launch.tcp {
                            Some(address) => format!("Connects to {address}"),
                            None => format!("{} {}", launch.program.display(), launch.args.join(" ")),
                        };
                        rows.push(row("Found", text.trim_end().to_owned(), Health::Good));
                    }
                    (None, Some(Found::Missing { note, .. })) => {
                        let text = hint.map_or_else(|| note.clone(), |h| format!("Not found. {h}"));
                        rows.push(row("Found", text, Health::Bad));
                    }
                    (None, None) => {}
                }
                let path = doc.path.as_deref();
                let client =
                    path.and_then(|p| self.lsp.clients.iter().find(|c| c.kind == kind && p.starts_with(&c.root)));
                let (state, health) = match (client, self.lsp.setups.get(&kind)) {
                    (Some(c), _) if c.is_ready() => ("Running".to_owned(), Health::Good),
                    (Some(_), _) => ("Starting…".to_owned(), Health::Note),
                    (None, _) if doc.buffer.read_only => {
                        ("Off: read-only documents aren't sent to servers".to_owned(), Health::Note)
                    }
                    (None, _) if !self.settings.lsp.enabled => ("Off: lsp.enabled is false".to_owned(), Health::Bad),
                    (None, _) if path.is_none() => ("Off: save the file first".to_owned(), Health::Note),
                    (None, Some(Setup::Off)) => {
                        let why = self.lsp.room.last_stop.clone().unwrap_or_else(|| "It stopped".to_owned());
                        (format!("Stopped. {why}"), Health::Bad)
                    }
                    (None, Some(Setup::Missing { .. })) => ("Not installed".to_owned(), Health::Bad),
                    (None, Some(Setup::Installing(_))) => ("Installing…".to_owned(), Health::Note),
                    (None, _) => ("Not started".to_owned(), Health::Note),
                };
                rows.push(row("State", state, health));
                if let Some(c) = client {
                    rows.push(row("Project root", c.root.display().to_string(), Health::Good));
                    rows.push(row("Offers", offered(c), Health::Good));
                } else if let Some(p) = path {
                    let root = servers::root_for(kind, p, self.explorer.root.as_deref());
                    let trusted = trust::is_trusted(&root);
                    rows.push(row("Project root", root.display().to_string(), Health::Good));
                    if !trusted {
                        let text = "Not trusted: servers stay off until you trust this folder".to_owned();
                        rows.push(row("Trust", text, Health::Bad));
                    }
                }
                if let ServerKind::Custom(i) = kind
                    && registry::get(i).is_some_and(|l| !l.runs_project_code)
                {
                    rows.push(row("Project code", "Doesn't run it, per its entry".to_owned(), Health::Good));
                } else {
                    let text = "This server may run the project's own code, such as build scripts".to_owned();
                    rows.push(row("Project code", text, Health::Note));
                }
            }
        }

        if matches!(ext.as_str(), "py" | "pyw" | "pyi") {
            let text = self.python_env().map_or_else(
                || "No Python on PATH or in this project".to_owned(),
                |env| format!("{}: {}. {}", env.label(), env.python.display(), env.reason),
            );
            let health = if self.python_env().is_some() { Health::Good } else { Health::Bad };
            rows.push(row("Python", text, health));
        }
        if let Some((text, fine)) = self.cpp_doctor_row() {
            rows.push(row("Compile flags", text, if fine { Health::Good } else { Health::Bad }));
        }
        if let Some((text, fine)) = self.formatter_report() {
            rows.push(row("Formatting", text, if fine { Health::Good } else { Health::Bad }));
        }
        let tip = self.run_button_tip();
        let runs = doc.path.as_deref().is_some_and(run::may_run);
        let shown = if runs { tip.replace('\n', ": ") } else { "Nothing to run for this file".to_owned() };
        rows.push(row("Run", shown, if runs { Health::Good } else { Health::Note }));
        let can = self.can_debug() && (ext == "rs" || !self.cargo_project || ext == "py");
        let text = match ext.as_str() {
            "rs" => "CodeLLDB, in Cargo projects".to_owned(),
            "py" | "pyw" => "debugpy, from this Python, offered on F5 if missing".to_owned(),
            _ if can => "The adapter in its languages.toml entry".to_owned(),
            _ => "None: `debugger` in languages.toml adds one".to_owned(),
        };
        rows.push(row("Debug", text, if can { Health::Good } else { Health::Note }));
        (name, rows)
    }

    /// The doctor's window.
    pub(super) fn doctor_window(&mut self, ctx: &egui::Context) {
        if !self.doctor.open {
            return;
        }
        if let Some(search) = &self.doctor.search
            && let Ok(found) = search.try_recv()
        {
            self.doctor.found = Some(found);
            self.doctor.search = None;
        }
        let (name, rows) = self.doctor_rows();
        let mut open = true;
        let (mut again, mut edit) = (false, false);
        egui::Window::new(format!("Language Doctor: {name}"))
            .open(&mut open)
            .resizable(true)
            .default_width(560.0)
            .show(ctx, |ui| {
                egui::Grid::new("doctor").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                    for row in &rows {
                        let (mark, color) = match row.health {
                            Health::Good => ("OK", egui::Color32::from_rgb(0x89, 0xD1, 0x85)),
                            Health::Note => ("--", egui::Color32::from_rgb(0xCC, 0xA7, 0x00)),
                            Health::Bad => ("NO", egui::Color32::from_rgb(0xF1, 0x4C, 0x4C)),
                        };
                        ui.label(RichText::new(mark).strong().color(color));
                        ui.label(RichText::new(row.label).strong());
                        ui.add(egui::Label::new(&row.text).wrap());
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    again = ui.button("Retry").on_hover_text("Restart language servers and look again").clicked();
                    edit = ui.button("Edit languages.toml").clicked();
                });
            });
        self.doctor.open = open;
        if again {
            self.restart_lsp();
            self.look_for_server(ctx);
        }
        if edit {
            self.open_languages_file();
        }
    }

    /// Opens the user's languages file, creating it from the template if needed.
    pub(super) fn open_languages_file(&mut self) {
        let Some(path) = registry::user_file() else {
            self.status = Some("No config directory available".to_owned());
            return;
        };
        if !path.exists() {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, registry::user_file_template()));
            if let Err(e) = written {
                self.status = Some(format!("Could not create {}: {e}", path.display()));
                return;
            }
        }
        self.open_file(&path);
    }
}

/// What the server said it can do.
fn offered(client: &rust_code_core::lsp::Client) -> String {
    let Some(caps) = client.capabilities() else { return "Not known yet".to_owned() };
    let features = [
        ("completion", caps.completion_provider.is_some()),
        ("hover", caps.hover_provider.is_some()),
        ("definition", caps.definition_provider.is_some()),
        ("references", caps.references_provider.is_some()),
        ("rename", caps.rename_provider.is_some()),
        ("formatting", caps.document_formatting_provider.is_some()),
        ("code actions", caps.code_action_provider.is_some()),
        ("code lens", caps.code_lens_provider.is_some()),
        ("inlay hints", caps.inlay_hint_provider.is_some()),
        ("colors", caps.semantic_tokens_provider.is_some()),
    ];
    let has: Vec<&str> = features.iter().filter(|(_, on)| *on).map(|(name, _)| *name).collect();
    if has.is_empty() { "Only problems".to_owned() } else { has.join(", ") }
}
