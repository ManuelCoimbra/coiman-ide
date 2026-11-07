//! Keeps one idea of the project's Python, shows it, and hands it to everything
//! that starts a program.

use std::path::PathBuf;

use coiman_core::environment::{self, PythonEnv};
use coiman_core::lsp::servers::ServerKind;
use eframe::egui;

use super::CodeApp;
use crate::palette::PickerKind;

/// Seconds between looks for an environment that appeared or went away.
const CHECK_EVERY: f64 = 3.0;
/// The picker's choice that goes back to working it out.
const AUTOMATIC: &str = "Automatic";

#[derive(Default)]
pub(super) struct Environment {
    /// What was found, for which folder.
    current: Option<(PathBuf, Option<PythonEnv>)>,
    next_check: f64,
    /// What the picker offered, by label.
    offered: Vec<(String, String)>,
}

impl CodeApp {
    /// The folder the project's tools work from.
    fn environment_start(&self) -> Option<PathBuf> {
        let active = self.docs.get(self.active).and_then(|d| d.path.as_deref());
        active.and_then(|p| p.parent()).map(PathBuf::from).or_else(|| self.explorer.root.clone())
    }

    /// The environment everything should use right now.
    pub(super) fn python_env(&self) -> Option<&PythonEnv> {
        self.environment.current.as_ref()?.1.as_ref()
    }

    /// Variables for a program started from here.
    pub(super) fn environment_variables(&self) -> Vec<(String, String)> {
        self.python_env().map(PythonEnv::variables).unwrap_or_default()
    }

    /// Looks again now and then; a server using an old answer starts over.
    pub(super) fn pump_environment(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let start = self.environment_start();
        let moved = self.environment.current.as_ref().map(|(dir, _)| dir) != start.as_ref();
        // An idle window draws nothing, and an environment can appear at any time: while
        // Python is in play (its file in front, or its server running), look again in a
        // while even with nothing else happening. Otherwise the next frame will.
        let python = self.docs.get(self.active).is_some_and(|d| matches!(d.extension().as_str(), "py" | "pyw" | "pyi"))
            || self.lsp.clients.iter().any(|c| c.kind == ServerKind::Python);
        if start.is_some() && python {
            let wait = (self.environment.next_check - now).max(0.1);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
        }
        if !moved && now < self.environment.next_check {
            return;
        }
        self.environment.next_check = now + CHECK_EVERY;
        let Some(start) = start else {
            self.environment.current = None;
            return;
        };
        let found = environment::resolve(&start, self.explorer.root.as_deref());
        let before = self.environment.current.as_ref().and_then(|(_, env)| env.as_ref()).map(|e| e.python.clone());
        let changed = found.as_ref().map(|e| e.python.clone()) != before;
        self.environment.current = Some((start, found));
        if changed {
            self.restart_python_server();
        }
    }

    /// Stops Python's server, which starts again with the current environment.
    fn restart_python_server(&mut self) {
        let was_running = self.lsp.clients.iter().any(|c| c.kind == ServerKind::Python);
        self.lsp.clients.retain(|c| c.kind != ServerKind::Python);
        if was_running {
            self.lsp.setups.remove(&ServerKind::Python);
            self.lsp.synced.clear();
        }
    }

    /// The status bar's label, for a Python file.
    pub(super) fn python_label(&self) -> Option<(String, String)> {
        let doc = self.docs.get(self.active)?;
        if !matches!(doc.extension().as_str(), "py" | "pyw" | "pyi") {
            return None;
        }
        let Some(env) = self.python_env() else {
            return Some(("Python: none found".to_owned(), "No Python on PATH or in this project".to_owned()));
        };
        Some((format!("Python: {}", env.label()), format!("{}\n{}", env.reason, env.python.display())))
    }

    /// Offers the Pythons this project could use.
    pub(super) fn choose_python(&mut self) {
        let Some(start) = self.environment_start() else {
            self.status = Some("Open a file or folder first".to_owned());
            return;
        };
        let found = environment::candidates(&start, self.explorer.root.as_deref());
        let mut offered = vec![(AUTOMATIC.to_owned(), String::new())];
        offered.extend(
            found
                .iter()
                .map(|e| (format!("{}  {}", e.label(), e.python.display()), e.python.to_string_lossy().into_owned())),
        );
        let labels = offered.iter().map(|(label, _)| label.clone()).collect();
        let current = self
            .python_env()
            .and_then(|env| {
                let chosen = !self.settings.python.interpreter.trim().is_empty();
                chosen.then(|| format!("{}  {}", env.label(), env.python.display()))
            })
            .unwrap_or_else(|| AUTOMATIC.to_owned());
        self.environment.offered = offered;
        self.palette.show_picker(PickerKind::Python, labels, &current, "Select a Python for this project");
    }

    /// Saves the choice with the folder's settings when one is open.
    pub(super) fn picked_python(&mut self, ctx: &egui::Context, label: &str) {
        let Some((_, path)) = self.environment.offered.iter().find(|(l, _)| l == label).cloned() else { return };
        let workspace = self.explorer.root.is_some();
        let value = (!path.is_empty()).then(|| toml_string(&path));
        if let Err(e) = self.change_setting(ctx, workspace, "python", "interpreter", value.as_deref()) {
            self.status = Some(e);
        }
        self.environment.next_check = 0.0;
    }
}

fn toml_string(text: &str) -> String {
    toml::Value::String(text.to_owned()).to_string()
}
