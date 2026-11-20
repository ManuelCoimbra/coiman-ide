//! Remote: Open SSH Folder...: a folder on another machine, brought here by `ssh … tar` into a
//! local mirror that opens as the project; each file saved in it goes back with ssh.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::language::lsp::discovery::quiet_command;
use coiman_core::project::remote_folder::RemoteFolder;
use eframe::egui::{self, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

pub(crate) enum Msg {
    Downloaded(RemoteFolder, PathBuf, Result<(), String>),
    Uploaded(String, Result<(), String>),
}

pub(crate) struct RemoteFolders {
    form: Option<RemoteFolder>,
    downloading: bool,
    /// To wake the window when an upload ends (saves come without one).
    ctx: Option<egui::Context>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for RemoteFolders {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { form: None, downloading: false, ctx: None, tx, rx }
    }
}

fn program(name: &str) -> String {
    coiman_core::language::lsp::servers::which(name)
        .map_or_else(|| name.to_owned(), |p| p.to_string_lossy().into_owned())
}

/// What a finished program wrote to stderr, for a message.
fn stderr_of(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut text);
    }
    text.trim().to_owned()
}

/// Brings the folder into `mirror`: into a fresh folder beside it first, so a failed download
/// leaves the last mirror as it was.
fn download(remote: &RemoteFolder, mirror: &Path) -> Result<(), String> {
    let fresh = mirror.with_extension("download");
    let _ = std::fs::remove_dir_all(&fresh);
    std::fs::create_dir_all(&fresh).map_err(|e| format!("{}: {e}", fresh.display()))?;
    let mut ssh = quiet_command(Path::new(&program("ssh")))
        .args(remote.download_args())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;
    let out = ssh.stdout.take().ok_or("ssh gave no output")?;
    let mut tar = quiet_command(Path::new(&program("tar")))
        .args(["-xf", "-", "-C"])
        .arg(&fresh)
        .stdin(Stdio::from(out))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("tar: {e}"))?;
    let tar_ok = tar.wait().map_err(|e| e.to_string())?.success();
    let ssh_ok = ssh.wait().map_err(|e| e.to_string())?.success();
    if !ssh_ok {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(stderr_of(&mut ssh));
    }
    if !tar_ok {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(format!("tar: {}", stderr_of(&mut tar)));
    }
    remote.mark(&fresh)?;
    let _ = std::fs::remove_dir_all(mirror);
    std::fs::rename(&fresh, mirror).map_err(|e| format!("{}: {e}", mirror.display()))
}

fn upload(remote: &RemoteFolder, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let mut ssh = quiet_command(Path::new(&program("ssh")))
        .args(remote.upload_args(rel))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh: {e}"))?;
    if let Some(mut input) = ssh.stdin.take() {
        input.write_all(bytes).map_err(|e| e.to_string())?;
    }
    if ssh.wait().map_err(|e| e.to_string())?.success() { Ok(()) } else { Err(stderr_of(&mut ssh)) }
}

impl CodeApp {
    /// Remote: Open SSH Folder...: the form, as last filled in.
    pub(crate) fn show_remote_folder(&mut self) {
        let last = self.explorer.root.as_deref().and_then(RemoteFolder::of_mirror).unwrap_or_default();
        self.remote_folders.form = Some(last);
    }

    /// Remote: Download Folder Again: the open mirror, fresh from the remote folder.
    pub(crate) fn download_remote_again(&mut self, ctx: &egui::Context) {
        match self.explorer.root.as_deref().and_then(RemoteFolder::of_mirror) {
            Some(remote) => self.start_download(ctx, remote),
            None => self.status = Some("The open folder isn't a remote folder".to_owned()),
        }
    }

    fn start_download(&mut self, ctx: &egui::Context, remote: RemoteFolder) {
        let Some(config) = coiman_core::settings::config_dir() else { return };
        if self.remote_folders.downloading {
            return;
        }
        let mirror = remote.mirror_dir(&config);
        if self.docs.iter().any(|d| d.is_dirty() && d.path.as_deref().is_some_and(|p| p.starts_with(&mirror))) {
            self.status = Some("Save or close the changed files of this folder first".to_owned());
            return;
        }
        self.remote_folders.downloading = true;
        self.status = Some(format!("Downloading {}:{}…", remote.host, remote.path));
        let (tx, wake) = (self.remote_folders.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let result = download(&remote, &mirror);
            let _ = tx.send(Msg::Downloaded(remote, mirror, result));
            wake();
        });
    }

    /// A file saved in a mirror goes back to the remote folder.
    pub(crate) fn remote_saved(&mut self, path: &Path) {
        let Some(root) = self.explorer.root.clone() else { return };
        let Some(remote) = RemoteFolder::of_mirror(&root) else { return };
        let Some(rel) = RemoteFolder::relative(&root, path) else { return };
        let Ok(bytes) = std::fs::read(path) else { return };
        let tx = self.remote_folders.tx.clone();
        let wake = self.remote_folders.ctx.clone();
        self.status = Some(format!("Sending {rel}…"));
        std::thread::spawn(move || {
            let result = upload(&remote, &rel, &bytes);
            let _ = tx.send(Msg::Uploaded(rel, result));
            if let Some(ctx) = wake {
                ctx.request_repaint();
            }
        });
    }

    pub(crate) fn pump_remote_folders(&mut self, ctx: &egui::Context) {
        if self.remote_folders.ctx.is_none() {
            self.remote_folders.ctx = Some(ctx.clone());
        }
        while let Ok(msg) = self.remote_folders.rx.try_recv() {
            match msg {
                Msg::Downloaded(remote, mirror, Ok(())) => {
                    self.remote_folders.downloading = false;
                    let same = self.explorer.root.as_deref() == Some(mirror.as_path());
                    if !same {
                        self.open_folder(mirror);
                    } else {
                        // Open files show what came down.
                        for doc in &mut self.docs {
                            if let Some(p) = doc.path.clone()
                                && !doc.is_dirty()
                                && let Ok(fresh) = coiman_core::editor::document::Document::open(&p)
                            {
                                let uid = doc.uid;
                                *doc = fresh;
                                doc.uid = uid;
                            }
                        }
                    }
                    self.status = Some(format!("{}:{} is here; saves go back to it", remote.host, remote.path));
                }
                Msg::Downloaded(remote, _, Err(e)) => {
                    self.remote_folders.downloading = false;
                    self.status = Some(format!("Couldn't download {}:{}: {e}", remote.host, remote.path));
                }
                Msg::Uploaded(rel, Ok(())) => self.status = Some(format!("Sent {rel}")),
                Msg::Uploaded(rel, Err(e)) => self.status = Some(format!("Couldn't send {rel}: {e}")),
            }
        }
    }

    pub(crate) fn remote_folder_window(&mut self, ctx: &egui::Context) {
        let Some(mut f) = self.remote_folders.form.take() else { return };
        let pal = theme::colors();
        let (mut open, mut start) = (true, false);
        egui::Window::new(RichText::new("Open SSH Folder").size(16.0).strong())
            .id(egui::Id::unique("remote_folder_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(520.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(260.0, 150.0))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ctx, |ui| {
                let dim = |s: &str| RichText::new(s.to_owned()).small().color(pal.text_dim);
                egui::Grid::new("remote_folder_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    ui.label("Host");
                    ui.add(
                        egui::TextEdit::singleline(&mut f.host)
                            .hint_text("user@host, or a name in ~/.ssh/config")
                            .desired_width(340.0),
                    );
                    ui.end_row();
                    ui.label("Folder there");
                    ui.add(egui::TextEdit::singleline(&mut f.path).hint_text("/home/me/project").desired_width(340.0));
                    ui.end_row();
                    ui.label("ssh options");
                    ui.add(
                        egui::TextEdit::singleline(&mut f.ssh_options)
                            .hint_text("-i ~/.ssh/key -o Port=2222")
                            .desired_width(340.0),
                    );
                    ui.end_row();
                });
                ui.label(dim(
                    "The folder is copied here with ssh and tar (build folders like target and node_modules left out) and \
                     opens as the project. Each file you save goes back with ssh. Deleting or renaming isn't sent. \
                     ssh logs in with a key: no password prompt.",
                ));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let problem = f.problem();
                    let busy = self.remote_folders.downloading;
                    let b = ui.add_enabled(problem.is_none() && !busy, egui::Button::new("Download and open"));
                    start = b.clicked();
                    if let Some(p) = problem {
                        ui.label(dim(p));
                    } else if busy {
                        ui.label(dim("Downloading…"));
                    }
                });
            });
        if start {
            self.start_download(ctx, f);
        } else if open {
            self.remote_folders.form = Some(f);
        }
    }
}
