//! Debug: Remote (gdbserver): a form for where the program runs, then ssh starts gdbserver
//! there (or gdb connects to one running) and gdb attaches with the local copy's symbols.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;

use coiman_core::language::lsp::discovery::{kill_tree, quiet_command};
use coiman_core::launch::dap::DebugKind;
use coiman_core::launch::remote_debug::{self as rd, RemoteTarget};
use eframe::egui::{self, RichText, Stroke};

use super::{Launch, Msg};
use crate::app::{CodeApp, PanelTab, repaint};
use crate::ui::theme;

/// Lines of the ssh run, and how it ended.
pub(super) enum RemoteMsg {
    Copied(RemoteTarget, Result<(), String>),
    Line(String),
    Ended,
}

impl CodeApp {
    /// Debug: Remote (gdbserver)...: the form, filled in as last time for this project.
    pub(crate) fn show_remote_debug(&mut self) {
        if self.debugging() {
            self.status = Some("Stop debugging first".to_owned());
            return;
        }
        let root = self.explorer.root.clone();
        let target = root.as_deref().map(RemoteTarget::load).unwrap_or_default();
        self.debug.remote_form = Some(target);
    }

    pub(crate) fn remote_debug_window(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.debug.remote_form.take() else { return };
        let pal = theme::colors();
        let mut open = true;
        let mut start = false;
        let mut browse = false;
        egui::Window::new(RichText::new("Remote Debugging").size(16.0).strong())
            .id(egui::Id::unique("remote_debug_window"))
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
                ui.horizontal(|ui| {
                    ui.radio_value(&mut t.over_ssh, true, "Start gdbserver over ssh");
                    ui.radio_value(&mut t.over_ssh, false, "Connect to a gdbserver running");
                });
                ui.add_space(6.0);
                let dim = |s: &str| RichText::new(s.to_owned()).small().color(pal.text_dim);
                egui::Grid::new("remote_debug_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    let host_hint = if t.over_ssh { "user@host, or a name in ~/.ssh/config" } else { "host, or host:port" };
                    ui.label("Host");
                    ui.add(egui::TextEdit::singleline(&mut t.host).hint_text(host_hint).desired_width(340.0));
                    ui.end_row();
                    ui.label("Port");
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut t.port).range(1..=65535));
                        if t.over_ssh {
                            ui.label(dim("here"));
                            ui.add(egui::DragValue::new(&mut t.local_port).range(0..=65535))
                                .on_hover_text("The port here that leads to it; 0: the same");
                        }
                    });
                    ui.end_row();
                    if t.over_ssh {
                        ui.label("Program there");
                        ui.add(
                            egui::TextEdit::singleline(&mut t.remote_program)
                                .hint_text("/home/me/app")
                                .desired_width(340.0),
                        );
                        ui.end_row();
                        ui.label("Arguments");
                        ui.add(egui::TextEdit::singleline(&mut t.args).desired_width(340.0));
                        ui.end_row();
                        ui.label("ssh options");
                        ui.add(
                            egui::TextEdit::singleline(&mut t.ssh_options)
                                .hint_text("-i ~/.ssh/board -o Port=2222 (for ssh and scp)")
                                .desired_width(340.0),
                        );
                        ui.end_row();
                        ui.label("gdbserver");
                        ui.add(egui::TextEdit::singleline(&mut t.gdbserver).desired_width(340.0));
                        ui.end_row();
                    }
                    ui.label("Program here");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut t.local_program)
                                .hint_text("the same build, for its symbols")
                                .desired_width(270.0),
                        );
                        browse = ui.button("Choose…").clicked();
                    });
                    ui.end_row();
                });
                if t.over_ssh {
                    ui.checkbox(&mut t.copy_first, "Copy the program there first (scp)");
                    ui.label(dim(
                        "ssh logs in with a key (no password prompt), starts gdbserver there, and carries its port here.",
                    ));
                } else {
                    ui.label(dim("Start it there first, e.g. gdbserver :2345 ./app"));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let problem = t.problem();
                    let button = ui.add_enabled(problem.is_none(), egui::Button::new("Start debugging"));
                    start = button.on_disabled_hover_text(problem.unwrap_or_default()).clicked();
                    if let Some(p) = problem {
                        ui.label(dim(p));
                    }
                });
            });
        if browse && let Some(path) = self.file_dialog("remote_program").pick_file() {
            self.dialog_chose("remote_program", &path);
            t.local_program = path.to_string_lossy().into_owned();
        }
        if start {
            self.start_remote(ctx, t);
        } else if open {
            self.debug.remote_form = Some(t);
        }
    }

    /// Saves the target for the project and starts: copy, ssh and gdbserver, then gdb.
    pub(super) fn start_remote(&mut self, ctx: &egui::Context, t: RemoteTarget) {
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        if !self.allowed_to_run(&cwd) {
            return;
        }
        if let Some(root) = &self.explorer.root
            && let Err(e) = t.save(root)
        {
            self.status = Some(format!("Couldn't keep the remote target: {e}"));
        }
        self.debug.console.clear();
        self.panel_visible = true;
        self.panel_tab = PanelTab::DebugConsole;
        if !t.over_ssh {
            self.attach_remote(ctx, t);
            return;
        }
        if t.copy_first {
            self.push_console(&format!("scp {}", t.scp_args().join(" ")));
            self.status = Some("Copying the program there…".to_owned());
            let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
            std::thread::spawn(move || {
                let out = quiet_command(Path::new("scp")).args(t.scp_args()).stdin(Stdio::null()).output();
                let result = match out {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_owned()),
                    Err(e) => Err(format!("scp: {e}")),
                };
                let _ = tx.send(Msg::Remote(RemoteMsg::Copied(t, result)));
                wake();
            });
            return;
        }
        self.run_gdbserver(ctx, t);
    }

    /// ssh runs gdbserver there; its lines come back as messages.
    fn run_gdbserver(&mut self, ctx: &egui::Context, t: RemoteTarget) {
        let args = t.ssh_args();
        self.push_console(&format!("ssh {}", args.join(" ")));
        let spawned = quiet_command(Path::new("ssh"))
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(format!("Couldn't start ssh: {e}"));
                return;
            }
        };
        let (done_tx, done_rx) = mpsc::channel::<()>();
        for pipe in [
            child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
            child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let (tx, wake, done) = (self.debug.tx.clone(), repaint(ctx), done_tx.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    let _ = tx.send(Msg::Remote(RemoteMsg::Line(line)));
                    wake();
                }
                drop(done);
            });
        }
        drop(done_tx);
        // Both pipes closed: ssh has ended.
        let (tx, wake) = (self.debug.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let _ = done_rx.recv();
            let _ = tx.send(Msg::Remote(RemoteMsg::Ended));
            wake();
        });
        self.debug.remote_child = Some(child);
        self.debug.remote_waiting = Some(t);
        self.status = Some("Starting gdbserver over ssh…".to_owned());
    }

    /// gdb attaches to the target, with the local program's symbols.
    fn attach_remote(&mut self, ctx: &egui::Context, t: RemoteTarget) {
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let program = PathBuf::from(t.local_program.trim());
        let name =
            format!("{} on {}", program.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), t.target());
        let launch =
            Launch { kind: DebugKind::Gdb, name, program, args: Vec::new(), cwd, attach: None, remote: Some(t) };
        self.start_session(ctx, launch);
    }

    pub(super) fn on_remote_msg(&mut self, ctx: &egui::Context, msg: RemoteMsg) {
        match msg {
            RemoteMsg::Copied(t, Ok(())) => self.run_gdbserver(ctx, t),
            RemoteMsg::Copied(_, Err(e)) => {
                self.push_console(&e);
                self.status = Some("Couldn't copy the program there (see the debug console)".to_owned());
            }
            RemoteMsg::Line(line) => {
                self.push_console(&line);
                if rd::listening(&line)
                    && let Some(t) = self.debug.remote_waiting.take()
                {
                    self.attach_remote(ctx, t);
                } else if rd::failed(&line) && self.debug.remote_waiting.is_some() {
                    self.status = Some("Remote debugging couldn't start (see the debug console)".to_owned());
                }
            }
            RemoteMsg::Ended => {
                if let Some(mut child) = self.debug.remote_child.take() {
                    let code = child.wait().ok().and_then(|s| s.code());
                    if self.debug.remote_waiting.take().is_some() {
                        let code = code.map_or(String::new(), |c| format!(" (exit {c})"));
                        self.status =
                            Some(format!("ssh ended before gdbserver was ready{code}: see the debug console"));
                    }
                }
            }
        }
    }

    /// Ends the ssh run, if there is one (after the session, or when stopped early).
    pub(super) fn stop_remote(&mut self) {
        self.debug.remote_waiting = None;
        if let Some(mut child) = self.debug.remote_child.take() {
            kill_tree(&mut child);
        }
    }
}
