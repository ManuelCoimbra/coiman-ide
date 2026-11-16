//! Writing plugins in Coiman: Plugins: New Plugin... makes a plugin folder from a template,
//! and dev mode builds the open plugin folder, loads it, and builds and loads it again on
//! every save of its sources.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use coiman_core::extensions::plugin::dev::{self, Template};
use coiman_core::extensions::plugin::{registry, store};
use coiman_core::language::lsp::discovery::quiet_command;
use egui::{RichText, Stroke};

use super::{Consent, Job};
use crate::app::{CodeApp, repaint};
use crate::ui::theme;

/// Dev mode for one plugin folder.
pub(super) struct DevMode {
    folder: PathBuf,
    ctx: egui::Context,
    building: bool,
    /// Saved again while building: build once more after.
    again: bool,
}

/// A dev build's end: Cargo's lines, and the component's size or why it failed.
pub(super) struct Built {
    folder: PathBuf,
    lines: Vec<String>,
    result: Result<usize, String>,
}

/// The New Plugin form.
pub(super) struct NewPlugin {
    name: String,
    template: Template,
    parent: String,
}

fn source_of(folder: &Path) -> String {
    format!("dev:{}", folder.display())
}

impl CodeApp {
    /// Plugins: New Plugin...: the form, its folder under the open folder's parent at first.
    pub(crate) fn show_new_plugin(&mut self) {
        let parent = self
            .explorer
            .root
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(dirs_home)
            .unwrap_or_default();
        self.plugins.new_plugin = Some(NewPlugin {
            name: String::new(),
            template: Template::Command,
            parent: parent.to_string_lossy().into_owned(),
        });
    }

    pub(crate) fn new_plugin_window(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.plugins.new_plugin.take() else { return };
        let pal = theme::colors();
        let (mut open, mut create, mut browse) = (true, false, false);
        let slug = dev::slug(&form.name);
        let folder = Path::new(form.parent.trim()).join(&slug);
        egui::Window::new(RichText::new("New Plugin").size(16.0).strong())
            .id(egui::Id::unique("new_plugin_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(260.0, 200.0))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ctx, |ui| {
                let dim = |s: String| RichText::new(s).small().color(pal.text_dim);
                egui::Grid::new("new_plugin_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("Line Counter").desired_width(340.0));
                    ui.end_row();
                    ui.label("Folder in");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut form.parent).desired_width(270.0));
                        browse = ui.button("Choose…").clicked();
                    });
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.label("Start from");
                for t in Template::ALL {
                    ui.radio_value(&mut form.template, t, t.title());
                    ui.label(dim(format!("      {}", t.describe())));
                }
                ui.add_space(6.0);
                if !slug.is_empty() {
                    ui.label(dim(format!("Creates {} (id dev.{slug})", folder.display())));
                }
                ui.label(dim(
                    "Building needs Rust's wasm32 target: rustup target add wasm32-unknown-unknown".to_owned()
                ));
                ui.add_space(4.0);
                let ready = !slug.is_empty() && !form.parent.trim().is_empty();
                create = ui.add_enabled(ready, egui::Button::new("Create and open")).clicked();
            });
        if browse && let Some(dir) = self.file_dialog("new_plugin").pick_folder() {
            self.dialog_chose("new_plugin", &dir);
            form.parent = dir.to_string_lossy().into_owned();
        }
        if create {
            match dev::create(&folder, &form.name, form.template) {
                Ok(first) => {
                    self.open_folder(folder);
                    self.open_file(&first);
                    self.status = Some("Plugin created: Plugins: Start Dev Mode builds and loads it".to_owned());
                }
                Err(e) => {
                    self.status = Some(format!("Couldn't create the plugin: {e}"));
                    self.plugins.new_plugin = Some(form);
                }
            }
        } else if open {
            self.plugins.new_plugin = Some(form);
        }
    }

    /// Plugins: Start Dev Mode: builds and loads the open plugin folder, again on each save.
    pub(crate) fn start_plugin_dev(&mut self, ctx: &egui::Context) {
        if !self.plugins.started {
            self.plugins_init(ctx);
        }
        if !self.settings.plugins.enabled {
            self.status = Some("Plugins are turned off in Settings".to_owned());
            return;
        }
        let Some(folder) = self.explorer.root.clone().filter(|r| dev::is_plugin_folder(r)) else {
            self.status = Some("Open a plugin's folder first (plugin.toml and Cargo.toml)".to_owned());
            return;
        };
        if !self.allowed_to_run(&folder) {
            return;
        }
        self.plugins.dev = Some(DevMode { folder, ctx: ctx.clone(), building: false, again: false });
        self.dev_build();
    }

    pub(crate) fn stop_plugin_dev(&mut self) {
        if self.plugins.dev.take().is_some() {
            self.status = Some("Dev mode off: the plugin stays installed as last built".to_owned());
        }
    }

    /// Dev mode's line at the top of the Extensions view, with Stop.
    pub(super) fn plugin_dev_ui(&mut self, ui: &mut egui::Ui) {
        let Some(d) = &self.plugins.dev else { return };
        let pal = theme::colors();
        let name = d.folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let state = if d.building { "building…" } else { "reloads on save" };
        let mut stop = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Dev mode: {name}")).strong());
            ui.label(RichText::new(state).small().color(pal.text_dim));
            stop = ui.small_button("Stop").clicked();
        });
        if stop {
            self.stop_plugin_dev();
        }
    }

    /// A save in the plugin's folder: build and load it again.
    pub(super) fn plugin_dev_saved(&mut self, path: &Path) {
        if self.plugins.dev.as_ref().is_some_and(|d| dev::needs_build(&d.folder, path)) {
            self.dev_build();
        }
    }

    fn dev_build(&mut self) {
        let Some(d) = &mut self.plugins.dev else { return };
        if d.building {
            d.again = true;
            return;
        }
        d.building = true;
        let (folder, wake) = (d.folder.clone(), repaint(&d.ctx));
        let tx = self.jobs();
        self.begin_output(format!("cargo {}", dev::build_args().join(" ")));
        self.status = Some("Building the plugin…".to_owned());
        std::thread::spawn(move || {
            let out = quiet_command(Path::new("cargo"))
                .args(dev::build_args())
                .current_dir(&folder)
                .env("CARGO_TERM_COLOR", "never")
                .stdin(Stdio::null())
                .output();
            let (lines, result) = match out {
                Ok(o) => {
                    let text = String::from_utf8_lossy(&o.stderr).into_owned() + &String::from_utf8_lossy(&o.stdout);
                    let lines = text.lines().map(str::to_owned).collect();
                    let result = if o.status.success() {
                        dev::write_component(&folder)
                    } else {
                        Err("cargo build failed (see above)".to_owned())
                    };
                    (lines, result)
                }
                Err(e) => (Vec::new(), Err(format!("cargo: {e}"))),
            };
            let _ = tx.send(Job::DevBuilt(Box::new(Built { folder, lines, result })));
            wake();
        });
    }

    pub(super) fn on_dev_built(&mut self, ctx: &egui::Context, built: Built) {
        for line in built.lines {
            self.push_output(line);
        }
        match built.result {
            Ok(size) => {
                self.push_output(format!("plugin.wasm: {} KB", size / 1024));
                self.dev_load(ctx, &built.folder, size);
            }
            Err(e) => {
                self.push_output(e);
                self.status = Some("The plugin didn't build: see Output".to_owned());
            }
        }
        let again = match &mut self.plugins.dev {
            Some(d) if d.folder == built.folder => {
                d.building = false;
                std::mem::take(&mut d.again)
            }
            _ => false,
        };
        if again {
            self.dev_build();
        }
    }

    /// Installs and loads the freshly built plugin: quietly when it asks for nothing more
    /// than was granted before, else through the usual consent prompt.
    fn dev_load(&mut self, ctx: &egui::Context, folder: &Path, size: usize) {
        let pkg = match registry::read_folder(folder) {
            Ok(p) => p,
            Err(e) => {
                self.status = Some(format!("The plugin can't be loaded: {e}"));
                return;
            }
        };
        let (id, name, source) = (pkg.manifest.id.clone(), pkg.manifest.name.clone(), source_of(folder));
        let granted_before = self
            .plugins
            .lock
            .get(&id)
            .filter(|l| l.source == source)
            .map(|l| (l.granted.clone(), l.granted_programs.clone()));
        let quiet = granted_before.as_ref().is_some_and(|(perms, programs)| {
            pkg.manifest.permissions.is_subset(perms) && pkg.manifest.programs.is_subset(programs)
        });
        if let (true, Some((granted, programs))) = (quiet, granted_before) {
            let Some(dir) = self.plugins.dir.clone().or_else(store::plugins_dir) else { return };
            self.plugins.dir = Some(dir.clone());
            self.unload_plugin(&id);
            if let Err(e) = store::install(&dir, &mut self.plugins.lock, &pkg, &source, "", granted, programs) {
                self.status = Some(format!("Couldn't install {name}: {e}"));
                return;
            }
            self.save_lock();
            self.refresh_infos();
            self.load_plugin(ctx, &id);
            self.status = Some(format!("Reloaded {name} ({} KB)", size / 1024));
            return;
        }
        self.plugins.consent = Some(Consent {
            programs: Default::default(),
            granted: pkg.manifest.permissions.clone(),
            pkg,
            source,
            key: String::new(),
            trusted: false,
            diff: None,
        });
        self.show_extensions();
        self.status = Some(format!("{name} is built: approve what it asks for to load it"));
    }
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from)
}
