//! Docker: Show Containers and Images: the `docker` program's containers, images and the
//! project's Compose services, with what can be done to them. Quick actions run in the
//! background; logs, shells and Compose runs open in terminals.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::language::lsp::discovery::quiet_command;
use coiman_core::project::docker::{self, Container, ContainerAction, Image, Service};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Containers,
    Images,
    Compose,
}

/// What a refresh found.
struct Found {
    containers: Result<Vec<Container>, String>,
    images: Result<Vec<Image>, String>,
    services: Option<Result<Vec<Service>, String>>,
}

enum Msg {
    Found(Box<Found>),
    /// What was asked ("stop web") and what it says when done ("Stopped web").
    Done(String, String, Result<(), String>),
}

pub(crate) struct DockerView {
    open: bool,
    tab: Tab,
    found: Option<Found>,
    busy: Option<String>,
    compose: Option<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for DockerView {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { open: false, tab: Tab::default(), found: None, busy: None, compose: None, tx, rx }
    }
}

enum Action {
    Refresh,
    Container(ContainerAction, String),
    RemoveImage(String),
    Logs(String),
    Shell(String),
    Compose(&'static str),
    Tab(Tab),
}

fn program() -> String {
    coiman_core::language::lsp::servers::which("docker")
        .map_or_else(|| "docker".to_owned(), |p| p.to_string_lossy().into_owned())
}

/// Runs `docker args`; its output, or what its failure means.
fn run(args: &[String], cwd: &Path) -> Result<String, String> {
    let out = quiet_command(Path::new(&program())).args(args).current_dir(cwd).stdin(Stdio::null()).output().map_err(
        |e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "Docker isn't installed (no docker program on PATH)".to_owned()
            } else {
                format!("docker: {e}")
            }
        },
    )?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(docker::explain(&String::from_utf8_lossy(&out.stderr)))
    }
}

fn state_color(state: &str) -> Color32 {
    match state {
        "running" => Color32::from_rgb(0x57, 0xC0, 0x6A),
        "paused" | "restarting" | "created" => Color32::from_rgb(0xCC, 0xA7, 0x00),
        _ => theme::colors().text_dim,
    }
}

impl CodeApp {
    pub(crate) fn show_docker(&mut self, ctx: &egui::Context) {
        self.docker.open = true;
        self.docker.compose = self.explorer.root.as_deref().and_then(docker::compose_file);
        if self.docker.compose.is_some() && self.docker.found.is_none() {
            self.docker.tab = Tab::Compose;
        }
        self.docker_refresh(ctx);
    }

    fn docker_refresh(&mut self, ctx: &egui::Context) {
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let compose = self.docker.compose.clone();
        let (tx, wake) = (self.docker.tx.clone(), repaint(ctx));
        self.docker.busy = Some("Asking Docker…".to_owned());
        std::thread::spawn(move || {
            let containers = run(&docker::ps_args(), &cwd).map(|o| docker::parse_containers(&o));
            // Without an engine, the rest would only say so again.
            let (images, services) = match &containers {
                Err(e) => (Err(e.clone()), compose.as_ref().map(|_| Err(e.clone()))),
                Ok(_) => (
                    run(&docker::images_args(), &cwd).map(|o| docker::parse_images(&o)),
                    compose.map(|f| run(&docker::compose_ps_args(&f), &cwd).map(|o| docker::parse_services(&o))),
                ),
            };
            let _ = tx.send(Msg::Found(Box::new(Found { containers, images, services })));
            wake();
        });
    }

    /// A quick command in the background, then a refresh.
    fn docker_do(&mut self, ctx: &egui::Context, args: Vec<String>, what: String, done: String) {
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let (tx, wake) = (self.docker.tx.clone(), repaint(ctx));
        self.docker.busy = Some(format!("docker {}…", args.join(" ")));
        std::thread::spawn(move || {
            let result = run(&args, &cwd).map(|_| ());
            let _ = tx.send(Msg::Done(what, done, result));
            wake();
        });
    }

    /// A long-running docker command in a terminal.
    fn docker_terminal(&mut self, ctx: &egui::Context, title: String, args: Vec<String>) {
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        if !self.allowed_to_run(&cwd) {
            return;
        }
        let program = program();
        let env = self.environment_variables();
        let task = crate::ui::terminal::Task {
            title: &title,
            program: &program,
            args: &args,
            env: &env,
            cwd: &cwd,
            scan: None,
        };
        let scrollback = self.settings.terminal.scrollback;
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = crate::app::PanelTab::Terminal;
            }
            Err(e) => self.status = Some(format!("Couldn't start docker: {e}")),
        }
    }

    pub(crate) fn docker_window(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.docker.rx.try_recv() {
            self.docker.busy = None;
            match msg {
                Msg::Found(found) => self.docker.found = Some(*found),
                Msg::Done(_, done, Ok(())) => {
                    self.status = Some(done);
                    self.docker_refresh(ctx);
                }
                Msg::Done(what, _, Err(e)) => self.status = Some(format!("Couldn't {what}: {e}")),
            }
        }
        if !self.docker.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut action = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.75).min(1000.0), (room.y * 0.65).max(340.0));
        egui::Window::new(RichText::new("Docker").size(16.0).strong())
            .id(egui::Id::unique("docker_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| action = self.docker_ui(ui));
        self.docker.open = open;
        let Some(action) = action else { return };
        match action {
            Action::Refresh => self.docker_refresh(ctx),
            Action::Tab(t) => self.docker.tab = t,
            Action::Container(a, id) => {
                let (what, done) =
                    (format!("{} {id}", a.args(&id)[0].replace("rm", "remove")), format!("{} {id}", a.done()));
                self.docker_do(ctx, a.args(&id), what, done);
            }
            Action::RemoveImage(r) => {
                self.docker_do(ctx, vec!["rmi".to_owned(), r.clone()], format!("remove {r}"), format!("Removed {r}"))
            }
            Action::Logs(id) => self.docker_terminal(ctx, format!("logs {id}"), docker::logs_args(&id)),
            Action::Shell(id) => self.docker_terminal(ctx, format!("shell {id}"), docker::shell_args(&id)),
            Action::Compose(what) => {
                let Some(file) = self.docker.compose.clone() else { return };
                let mut args = docker::compose(&file);
                match what {
                    "up" => args.extend(["up".to_owned(), "--build".to_owned()]),
                    "logs" => args.extend(["logs".to_owned(), "-f".to_owned(), "--tail".to_owned(), "200".to_owned()]),
                    other => args.push(other.to_owned()),
                }
                let title = format!("compose {what}");
                if what == "down" {
                    self.docker_do(ctx, args, "run compose down".to_owned(), "Compose: down".to_owned());
                } else {
                    self.docker_terminal(ctx, title, args);
                }
            }
        }
    }

    fn docker_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
        let mut action = None;
        let v = &self.docker;
        ui.horizontal(|ui| {
            let count = |n: Option<usize>, name: &str| match n {
                Some(n) if n > 0 => format!("{name}  {n}"),
                _ => name.to_owned(),
            };
            let f = v.found.as_ref();
            let tabs = [
                (Tab::Containers, count(f.and_then(|f| f.containers.as_ref().ok()).map(Vec::len), "Containers")),
                (Tab::Images, count(f.and_then(|f| f.images.as_ref().ok()).map(Vec::len), "Images")),
                (Tab::Compose, count(f.and_then(|f| f.services.as_ref()?.as_ref().ok()).map(Vec::len), "Compose")),
            ];
            for (tab, label) in tabs {
                if ui.selectable_label(v.tab == tab, label).clicked() {
                    action = Some(Action::Tab(tab));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    action = Some(Action::Refresh);
                }
                if let Some(b) = &v.busy {
                    ui.label(dim(b));
                }
            });
        });
        ui.separator();
        let Some(found) = &v.found else {
            ui.label(dim("Asking Docker…"));
            return action;
        };
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match v.tab {
            Tab::Containers => match &found.containers {
                Err(e) => {
                    ui.label(RichText::new(e).color(pal.bracket_unexpected));
                }
                Ok(list) if list.is_empty() => {
                    ui.label(dim("No containers."));
                }
                Ok(list) => {
                    egui::Grid::new("docker_containers").striped(true).num_columns(6).show(ui, |ui| {
                        for h in ["Name", "Image", "Status", "Ports", ""] {
                            ui.label(dim(h));
                        }
                        ui.end_row();
                        for c in list {
                            ui.label(RichText::new(&c.name).strong().color(state_color(&c.state)));
                            ui.label(RichText::new(&c.image).monospace());
                            ui.label(&c.status);
                            ui.label(dim(&c.ports));
                            ui.horizontal(|ui| {
                                let id = c.name.clone();
                                if c.running() {
                                    if ui.small_button("Stop").clicked() {
                                        action = Some(Action::Container(ContainerAction::Stop, id.clone()));
                                    }
                                    if ui.small_button("Restart").clicked() {
                                        action = Some(Action::Container(ContainerAction::Restart, id.clone()));
                                    }
                                    if ui
                                        .small_button("Shell")
                                        .on_hover_text("A shell inside it, in a terminal")
                                        .clicked()
                                    {
                                        action = Some(Action::Shell(id.clone()));
                                    }
                                } else {
                                    if ui.small_button("Start").clicked() {
                                        action = Some(Action::Container(ContainerAction::Start, id.clone()));
                                    }
                                    if ui.small_button("Remove").clicked() {
                                        action = Some(Action::Container(ContainerAction::Remove, id.clone()));
                                    }
                                }
                                if ui
                                    .small_button("Logs")
                                    .on_hover_text("Its output, followed, in a terminal")
                                    .clicked()
                                {
                                    action = Some(Action::Logs(id));
                                }
                            });
                            ui.end_row();
                        }
                    });
                }
            },
            Tab::Images => match &found.images {
                Err(e) => {
                    ui.label(RichText::new(e).color(pal.bracket_unexpected));
                }
                Ok(list) if list.is_empty() => {
                    ui.label(dim("No images."));
                }
                Ok(list) => {
                    egui::Grid::new("docker_images").striped(true).num_columns(4).show(ui, |ui| {
                        for h in ["Image", "Size", "Created", ""] {
                            ui.label(dim(h));
                        }
                        ui.end_row();
                        for i in list {
                            ui.label(RichText::new(i.reference()).monospace());
                            ui.label(&i.size);
                            ui.label(dim(&i.created));
                            if ui
                                .small_button("Remove")
                                .on_hover_text("docker rmi (refused while a container uses it)")
                                .clicked()
                            {
                                action = Some(Action::RemoveImage(i.reference()));
                            }
                            ui.end_row();
                        }
                    });
                }
            },
            Tab::Compose => {
                let Some(file) = &v.compose else {
                    ui.label(dim("No Compose file in the open folder (compose.yaml, docker-compose.yml)."));
                    return;
                };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()).monospace(),
                    );
                    for (what, tip) in [
                        ("up", "Build and start every service, in a terminal (Ctrl+C stops them)"),
                        ("down", "Stop and remove the services"),
                        ("build", "Build the services' images, in a terminal"),
                        ("logs", "Every service's output, followed, in a terminal"),
                    ] {
                        if ui.button(what).on_hover_text(tip).clicked() {
                            action = Some(Action::Compose(what));
                        }
                    }
                });
                ui.add_space(4.0);
                match &found.services {
                    None => {}
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).color(pal.bracket_unexpected));
                    }
                    Some(Ok(list)) if list.is_empty() => {
                        ui.label(dim("No services running: up starts them."));
                    }
                    Some(Ok(list)) => {
                        egui::Grid::new("docker_services").striped(true).num_columns(4).show(ui, |ui| {
                            for h in ["Service", "Container", "Status", "Ports"] {
                                ui.label(dim(h));
                            }
                            ui.end_row();
                            for s in list {
                                ui.label(RichText::new(&s.service).strong().color(state_color(&s.state)));
                                ui.label(RichText::new(&s.name).monospace());
                                ui.label(&s.status);
                                ui.label(dim(&s.ports));
                                ui.end_row();
                            }
                        });
                    }
                }
            }
        });
        action
    }
}
