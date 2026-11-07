//! C and C++: what clangd reads for the active file, choosing a database and a
//! configuration, getting a database when there is none, and header switching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::build::compile_db::{self, Report, Standing};
use coiman_core::language::lsp::convert;
use coiman_core::language::lsp::servers::{self, ServerKind};
use coiman_core::language::lsp::types as lt;
use eframe::egui::{self, RichText};
use serde_json::Value;

use super::{CodeApp, repaint};
use crate::terminal::Task;
use crate::theme;

/// Seconds between looks at the disk, with the window open and closed.
const CHECK_OPEN: f64 = 3.0;
const CHECK_CLOSED: f64 = 30.0;

#[derive(Default)]
pub(super) struct Cpp {
    open: bool,
    report: Option<(PathBuf, Report)>,
    looking: Option<Receiver<(PathBuf, Report)>>,
    /// What the report in hand or on its way was asked about.
    asked: Option<(PathBuf, String, String)>,
    next_check: f64,
    /// What clangd last said it's doing with a file.
    file_status: HashMap<PathBuf, String>,
}

enum Action {
    Database(String),
    Config(String),
    Run(String),
}

impl CodeApp {
    fn cpp_file(&self) -> Option<PathBuf> {
        let doc = self.docs.get(self.active)?;
        if !compile_db::is_c_family(&doc.extension()) {
            return None;
        }
        doc.path.clone()
    }

    fn cpp_root(&self, file: &Path) -> PathBuf {
        servers::root_for(ServerKind::Clangd, file, self.explorer.root.as_deref())
    }

    /// Keeps the report on the active file current.
    pub(super) fn pump_cpp(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.cpp.looking
            && let Ok(done) = rx.try_recv()
        {
            self.cpp.report = Some(done);
            self.cpp.looking = None;
        }
        let Some(file) = self.cpp_file() else { return };
        let (dir, config) = (self.settings.cpp.compile_commands_dir.clone(), self.settings.cpp.config.clone());
        let asked = (file.clone(), dir.clone(), config.clone());
        let now = ctx.input(|i| i.time);
        if self.cpp.looking.is_some() || (self.cpp.asked.as_ref() == Some(&asked) && now < self.cpp.next_check) {
            if self.cpp.open {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(CHECK_OPEN));
            }
            return;
        }
        self.cpp.asked = Some(asked);
        self.cpp.next_check = now + if self.cpp.open { CHECK_OPEN } else { CHECK_CLOSED };
        let root = self.cpp_root(&file);
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let report = compile_db::report(&file, &root, &dir, &config);
            let _ = tx.send((file, report));
            wake();
        });
        self.cpp.looking = Some(rx);
    }

    fn cpp_report(&self) -> Option<&Report> {
        let file = self.cpp_file()?;
        self.cpp.report.as_ref().filter(|(f, _)| *f == file).map(|(_, r)| r)
    }

    /// The status bar's label: (text, more about it).
    pub(super) fn cpp_label(&self) -> Option<(String, String)> {
        let report = self.cpp_report()?;
        Some((format!("C/C++: {}", report.short()), "What clangd reads for this file. Click for details.".to_owned()))
    }

    /// One line for the doctor, with whether all is well.
    pub(super) fn cpp_doctor_row(&self) -> Option<(String, bool)> {
        let report = self.cpp_report()?;
        Some((report.short(), report.standing == Standing::Flags))
    }

    pub(super) fn show_compile_info(&mut self) {
        if self.cpp_file().is_none() {
            self.status = Some("Open a saved C or C++ file to see how it's compiled".to_owned());
            return;
        }
        self.cpp.open = true;
        self.cpp.next_check = 0.0;
    }

    pub(super) fn on_clangd_file_status(&mut self, params: &Value) {
        let uri = params["uri"].as_str().and_then(|u| u.parse::<lt::Url>().ok());
        let (Some(path), Some(state)) = (uri.as_ref().and_then(convert::path_from_uri), params["state"].as_str())
        else {
            return;
        };
        self.cpp.file_status.insert(super::lsp::path_key(&path), state.to_owned());
    }

    /// What clangd is told beyond its usual arguments: the database chosen here.
    pub(super) fn clangd_arguments(&mut self, root: &Path) -> Vec<String> {
        let (dir, config) = (&self.settings.cpp.compile_commands_dir, &self.settings.cpp.config);
        match compile_db::clangd_directory(root, dir, config) {
            Ok(Some(dir)) => vec![format!("--compile-commands-dir={}", dir.display())],
            Ok(None) => Vec::new(),
            Err(why) => {
                self.status = Some(format!("Compile database: {why}"));
                Vec::new()
            }
        }
    }

    /// Stops clangd, for the next document sync to start it anew.
    pub(super) fn restart_clangd(&mut self) {
        let was_running = self.lsp.clients.iter().any(|c| c.kind == ServerKind::Clangd);
        self.lsp.clients.retain(|c| c.kind != ServerKind::Clangd);
        if was_running {
            self.lsp.setups.remove(&ServerKind::Clangd);
            self.lsp.synced.clear();
        }
        self.cpp.file_status.clear();
        self.cpp.next_check = 0.0;
    }

    fn cpp_change(&mut self, ctx: &egui::Context, key: &str, value: &str) {
        let literal = (!value.is_empty()).then(|| toml::Value::String(value.to_owned()).to_string());
        let workspace = self.explorer.root.is_some();
        if let Err(e) = self.change_setting(ctx, workspace, "cpp", key, literal.as_deref()) {
            self.status = Some(e);
        }
    }

    fn run_fix(&mut self, ctx: &egui::Context, command: &str, cwd: &Path) {
        if !self.allowed_to_run(cwd) {
            return;
        }
        const TITLE: &str = "Compile database";
        self.terminal.close_finished(TITLE);
        let (program, args) = super::cargo::shell(command);
        let env = self.environment_variables();
        let scrollback = self.settings.terminal.scrollback;
        let task = Task { title: TITLE, program: &program, args: &args, env: &env, cwd, scan: None };
        if let Err(e) = self.terminal.run_task(ctx, task, scrollback) {
            self.status = Some(format!("Could not start it: {e}"));
        }
        self.panel_tab = super::PanelTab::Terminal;
        self.panel_visible = true;
    }

    pub(super) fn cpp_window(&mut self, ctx: &egui::Context) {
        if !self.cpp.open {
            return;
        }
        let Some(file) = self.cpp_file() else {
            self.cpp.open = false;
            return;
        };
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let key = super::lsp::path_key(&file);
        let state = self.cpp.file_status.get(&key).cloned();
        let running = self.lsp.clients.iter().any(|c| c.kind == ServerKind::Clangd && file.starts_with(&c.root));
        let indexing: Vec<String> = self
            .lsp
            .progress
            .values()
            .filter(|(title, ..)| title.to_lowercase().contains("index"))
            .map(|(title, message, _)| format!("{title} {message}").trim().to_owned())
            .collect();
        let chosen = self.settings.cpp.compile_commands_dir.clone();
        let root = self.cpp_root(&file);
        let report = self.cpp_report().cloned();
        let mut open = true;
        let mut action = None;
        let window = egui::Window::new(format!("C/C++: {name}"))
            .open(&mut open)
            .collapsible(false)
            .default_pos(crate::theme::window_spot(ctx, 520.0));
        // Paths are long: they wrap, rather than the window growing past the screen.
        window.default_width(620.0).max_width(620.0).show(ctx, |ui| {
            let Some(report) = &report else {
                ui.label("Looking…");
                return;
            };
            let pal = theme::colors();
            let warn = egui::Color32::from_rgb(0xCC, 0xA7, 0x00);
            egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                ui.label(RichText::new(report.headline()).strong());
                for note in report.notes() {
                    ui.label(RichText::new(note).color(warn));
                }
                ui.add_space(4.0);
                egui::Grid::new("cpp_state").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("clangd");
                    let doing = match (running, &state) {
                        (false, _) => "Not running".to_owned(),
                        (true, Some(s)) => s.clone(),
                        (true, None) => "Running".to_owned(),
                    };
                    ui.label(doing);
                    ui.end_row();
                    if !indexing.is_empty() {
                        ui.label("Index");
                        ui.label(indexing.join(", "));
                        ui.end_row();
                    }
                    ui.label("Database");
                    let shown = report.database.as_ref().map_or("None".to_owned(), |d| d.display().to_string());
                    ui.add(egui::Label::new(shown).wrap());
                    ui.end_row();
                    if !report.candidates.is_empty() || !chosen.is_empty() {
                        ui.label("Use");
                        ui.horizontal_wrapped(|ui| {
                            if ui.selectable_label(chosen.is_empty(), "Where clangd looks").clicked() {
                                action = Some(Action::Database(String::new()));
                            }
                            for candidate in &report.candidates {
                                if ui.selectable_label(chosen == candidate.label, &candidate.label).clicked() {
                                    action = Some(Action::Database(candidate.label.clone()));
                                }
                            }
                        });
                        ui.end_row();
                    }
                    if !report.configs.is_empty() {
                        ui.label("Configuration");
                        ui.horizontal_wrapped(|ui| {
                            for config in &report.configs {
                                let on = report.config.as_deref() == Some(*config);
                                if ui.selectable_label(on, *config).clicked() {
                                    action = Some(Action::Config((*config).to_owned()));
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
                if let Some(flags) = &report.flags {
                    ui.separator();
                    egui::Grid::new("cpp_flags").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, label: &str, text: String| {
                            if !text.is_empty() {
                                ui.label(label);
                                ui.add(egui::Label::new(RichText::new(text).monospace()).wrap());
                                ui.end_row();
                            }
                        };
                        row(ui, "Compiler", flags.compiler.clone().unwrap_or_default());
                        row(ui, "Standard", flags.standard.clone().unwrap_or_default());
                        row(ui, "Target", flags.target.clone().unwrap_or_default());
                        row(ui, "Defines", flags.defines.join("\n"));
                        row(ui, "Includes", flags.includes.join("\n"));
                        row(ui, "Other", flags.other.join(" "));
                    });
                }
                if let Some(fix) = &report.fix {
                    ui.separator();
                    ui.label(&fix.why);
                    if let Some(command) = &fix.command {
                        ui.label(RichText::new(command).monospace().color(pal.editor_fg));
                        if ui.button("Run in Terminal").clicked() {
                            action = Some(Action::Run(command.clone()));
                        }
                    }
                    if let Some(then) = &fix.then {
                        ui.label(RichText::new(then).color(pal.text_dim));
                    }
                }
            });
        });
        match action {
            Some(Action::Database(dir)) => self.cpp_change(ctx, "compile_commands_dir", &dir),
            Some(Action::Config(config)) => self.cpp_change(ctx, "config", &config),
            Some(Action::Run(command)) => self.run_fix(ctx, &command, &root),
            None => {}
        }
        if !open {
            self.cpp.open = false;
        }
    }

    // ----- switching ---------------------------------------------------------------------

    /// Alt+O: clangd knows best, and names on disk do when it doesn't.
    pub(super) fn switch_header_source(&mut self) {
        let Some(path) = self.cpp_file() else {
            self.status = Some("Open a saved C or C++ file to switch".to_owned());
            return;
        };
        if self.server_ready_for(self.active) {
            let i = self.active;
            self.request_for(
                i,
                "textDocument/switchSourceHeader",
                super::lsp::Pending::SwitchHeader { path: path.clone() },
                |_, td| td,
            );
        } else {
            self.switch_by_name(&path);
        }
    }

    pub(super) fn switch_by_name(&mut self, path: &Path) {
        match compile_db::counterpart(path, self.explorer.root.as_deref()) {
            Some(other) => self.open_file(&other),
            None => self.status = Some("No matching header or source file".to_owned()),
        }
    }

    pub(super) fn on_switch_header(&mut self, path: &Path, value: &Value) {
        let uri = value.as_str().and_then(|u| u.parse::<lt::Url>().ok());
        match uri.as_ref().and_then(convert::path_from_uri) {
            Some(other) => self.open_file(&other),
            None => self.switch_by_name(path),
        }
    }
}
