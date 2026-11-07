//! Getting Git set up: whether it's installed, who commits are made by (and how that
//! differs from signing in to GitHub), starting a repository, cloning one, its
//! remotes, and creating it on GitHub. Slow work runs on its own thread.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use coiman_core::project::git::{self, Identity, Repo};
use egui::{RichText, Sense};

use super::{CodeApp, repaint};
use crate::commands::Command;
use crate::palette::{InputKind, PickerKind};
use crate::theme;

/// What was found about Git and the open repository.
#[derive(Clone, Debug, Default)]
struct SetupInfo {
    /// Git's version; `None` when it isn't installed.
    installed: Option<String>,
    global: Identity,
    /// The repository's own (with the global ones under it), when there's one.
    repo: Option<Identity>,
    remotes: Vec<(String, String)>,
    github_cli: bool,
}

/// The author form, while open.
struct AuthorForm {
    name: String,
    email: String,
    /// For every repository, else only this one.
    global: bool,
    error: Option<String>,
}

#[derive(Default)]
pub(super) struct GitSetup {
    /// The repository root the info was read for, and the info.
    read_for: Option<Option<PathBuf>>,
    info: Option<SetupInfo>,
    reading: Option<Receiver<SetupInfo>>,
    form: Option<AuthorForm>,
    remotes_open: bool,
    remote_name: String,
    remote_url: String,
    /// A clone running: its URL, and its result.
    clone_url: Option<String>,
    cloning: Option<Receiver<Result<PathBuf, String>>>,
    /// A GitHub repository being created: its name, then its result.
    publish_name: Option<String>,
    publishing: Option<Receiver<Result<String, String>>>,
}

fn read_info(root: Option<PathBuf>) -> SetupInfo {
    let installed = git::installed();
    if installed.is_none() {
        return SetupInfo::default();
    }
    let repo = root.map(|root| Repo { root });
    SetupInfo {
        installed,
        global: git::global_identity(),
        repo: repo.as_ref().map(Repo::identity),
        remotes: repo.as_ref().and_then(|r| r.remotes().ok()).unwrap_or_default(),
        github_cli: git::github_cli_installed(),
    }
}

impl CodeApp {
    /// Reads Git's setup again soon: after the author or remotes change.
    fn git_setup_stale(&mut self) {
        self.git_setup.read_for = None;
    }

    /// Keeps the setup info current, and takes in clones and GitHub creations that
    /// finished. Runs every frame.
    pub(super) fn pump_git_setup(&mut self, ctx: &egui::Context) {
        let root = self.git_repo().map(|r| r.root.clone());
        if self.git_setup.read_for.as_ref() != Some(&root) && self.git_setup.reading.is_none() {
            self.git_setup.read_for = Some(root.clone());
            let (tx, rx) = mpsc::channel();
            let wake = repaint(ctx);
            std::thread::spawn(move || {
                let _ = tx.send(read_info(root));
                wake();
            });
            self.git_setup.reading = Some(rx);
        }
        if let Some(rx) = &self.git_setup.reading
            && let Ok(info) = rx.try_recv()
        {
            self.git_setup.reading = None;
            self.git_setup.info = Some(info);
        }
        if let Some(rx) = &self.git_setup.cloning
            && let Ok(result) = rx.try_recv()
        {
            self.git_setup.cloning = None;
            let url = self.git_setup.clone_url.take().unwrap_or_default();
            match result {
                Ok(folder) => {
                    self.status = Some(format!("Cloned {url}"));
                    self.open_folder(folder);
                }
                Err(e) => self.status = Some(format!("Clone failed: {}", git::explain(&e))),
            }
        }
        if let Some(rx) = &self.git_setup.publishing
            && let Ok(result) = rx.try_recv()
        {
            self.git_setup.publishing = None;
            self.git_setup.publish_name = None;
            self.status = Some(match result {
                Ok(url) if !url.is_empty() => format!("Published to {url}"),
                Ok(_) => "Published to GitHub".to_owned(),
                Err(e) => format!("Publishing failed: {}", git::explain(&e)),
            });
            self.git_setup_stale();
            self.git_refresh_soon(ctx, 0.0);
        }
    }

    /// `Git: Clone`: asks for the URL, then the folder to clone into.
    pub(super) fn git_clone_start(&mut self) {
        if self.git_setup.cloning.is_some() {
            self.status = Some("A clone is already running".to_owned());
            return;
        }
        self.palette.show_input(InputKind::CloneUrl, "", "Repository URL to clone, then Enter");
    }

    /// The URL was entered: choose where it goes, then clone on its own thread.
    pub(super) fn git_clone_to(&mut self, ctx: &egui::Context, url: &str) {
        let url = url.trim().to_owned();
        if git::repo_name_from_url(&url).is_none() {
            self.status = Some("That doesn't look like a repository URL".to_owned());
            return;
        }
        let Some(parent) = self.file_dialog("clone_parent").set_title("Clone into this folder").pick_folder() else {
            return;
        };
        // Remembered as the folder holding the clone, for the next one.
        self.dialog_chose("clone_parent", &parent.join("x"));
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let job = url.clone();
        std::thread::spawn(move || {
            let _ = tx.send(git::clone(&job, &parent));
            wake();
        });
        self.status = Some(format!("Cloning {url}..."));
        self.git_setup.clone_url = Some(url);
        self.git_setup.cloning = Some(rx);
    }

    /// `Git: Initialize Repository` in the open folder.
    pub(super) fn git_init(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else {
            self.status = Some("Open a folder first: the repository is made in it".to_owned());
            return;
        };
        if self.git_repo().is_some() {
            self.status = Some("This folder is already in a Git repository".to_owned());
            return;
        }
        match git::init(&root, "main") {
            Ok(_) => {
                self.status = Some(format!("Made a Git repository in {}", root.display()));
                // Look for it again.
                self.git.discovered_for = None;
                self.git_setup_stale();
                self.git_refresh_soon(ctx, 0.0);
            }
            Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
        }
    }

    /// `Git: Set Author`: opens the form, filled in with what's set now.
    pub(super) fn git_author_form(&mut self) {
        self.sidebar_visible = true;
        self.sidebar_view = super::SidebarView::Git;
        let info = self.git_setup.info.clone().unwrap_or_default();
        let now = info.repo.clone().unwrap_or_else(|| info.global.clone());
        self.git_setup.form = Some(AuthorForm {
            name: now.name.unwrap_or_default(),
            email: now.email.unwrap_or_default(),
            global: true,
            error: None,
        });
    }

    /// `Git: Publish to GitHub`: asks for the name, then public or private.
    pub(super) fn git_publish_start(&mut self) {
        let Some(info) = &self.git_setup.info else { return };
        if self.git_repo().is_none() {
            self.status = Some("Initialize a repository first".to_owned());
            return;
        }
        if !info.github_cli {
            self.status = Some(
                "Publishing to GitHub needs the GitHub CLI (gh), signed in with `gh auth login`. Or create the repository on github.com and add it under Remotes.".to_owned(),
            );
            return;
        }
        let name = self
            .git_repo()
            .and_then(|r| r.root.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        self.palette.show_input(InputKind::GitHubName, &name, "Name of the new GitHub repository, then Enter");
    }

    pub(super) fn git_publish_named(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name.contains(char::is_whitespace) {
            self.status = Some("A GitHub repository's name has no spaces".to_owned());
            return;
        }
        self.git_setup.publish_name = Some(name.to_owned());
        let items = vec![
            format!("Private: only you, and who you invite, see {name}"),
            format!("Public: anyone can see {name}"),
        ];
        self.palette.show_picker(PickerKind::GitHubVisibility, items, "", "Who can see it on GitHub?");
    }

    pub(super) fn git_publish_visibility(&mut self, ctx: &egui::Context, label: &str) {
        let (Some(repo), Some(name)) = (self.git_repo().cloned(), self.git_setup.publish_name.clone()) else { return };
        let private = label.starts_with("Private");
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(repo.create_on_github(&name, private));
            wake();
        });
        self.status = Some("Creating the repository on GitHub...".to_owned());
        self.git_setup.publishing = Some(rx);
    }

    /// Above the Source Control view: Git missing, the author, and a folder that isn't a
    /// repository yet. True when there's nothing more to show.
    pub(super) fn git_setup_section(&mut self, ui: &mut egui::Ui) -> bool {
        let pal = theme::colors();
        let ctx = ui.ctx().clone();
        let Some(info) = self.git_setup.info.clone() else {
            ui.label(RichText::new("Looking for Git...").color(pal.text_dim));
            return true;
        };
        if info.installed.is_none() {
            ui.label("Git isn't installed, or isn't on the PATH.");
            ui.label(
                RichText::new("Install it from git-scm.com, then restart the editor. Coiman IDE uses the git command for everything here.")
                    .small()
                    .color(pal.text_dim),
            );
            if ui.link("Download Git (git-scm.com)").clicked() {
                ctx.open_url(egui::OpenUrl::new_tab("https://git-scm.com/downloads"));
            }
            return true;
        }
        self.author_section(ui, &info);
        if self.git_repo().is_none() {
            ui.add_space(6.0);
            let open = self.explorer.root.is_some();
            let text = if open { "The open folder isn't in a Git repository." } else { "No folder is open." };
            ui.label(RichText::new(text).color(pal.text_dim));
            ui.add_space(4.0);
            if open
                && ui
                    .button("Initialize Repository")
                    .on_hover_text("Run git init here, with a first branch named main")
                    .clicked()
            {
                self.git_init(&ctx);
            }
            if ui.button("Clone Repository...").on_hover_text("Copy a repository from a URL into a folder").clicked() {
                self.git_clone_start();
            }
            return true;
        }
        false
    }

    /// Who commits are made by: a warning and the form when unset, else one line.
    fn author_section(&mut self, ui: &mut egui::Ui, info: &SetupInfo) {
        let pal = theme::colors();
        let current = info.repo.clone().unwrap_or_else(|| info.global.clone());
        if self.git_setup.form.is_none() {
            if current.is_complete() {
                ui.horizontal_wrapped(|ui| {
                    let who = format!(
                        "Commits as {} <{}>",
                        current.name.as_deref().unwrap_or(""),
                        current.email.as_deref().unwrap_or("")
                    );
                    ui.label(RichText::new(who).small().color(pal.text_dim));
                    if ui.small_button("Change").clicked() {
                        self.git_author_form();
                    }
                });
                return;
            }
            egui::Frame::new().fill(pal.hover_bg).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
                ui.label(RichText::new("Git doesn't know who you are yet").strong());
                ui.label(
                    RichText::new("Each commit records a name and an email, and Git refuses to commit without them.")
                        .small(),
                );
                if ui.button("Set Name and Email...").clicked() {
                    self.git_author_form();
                }
            });
            return;
        }
        let in_repo = self.git_repo().is_some();
        let mut save = false;
        let mut cancel = false;
        egui::Frame::new().fill(pal.hover_bg).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
            let Some(form) = &mut self.git_setup.form else { return };
            ui.label(RichText::new("Commit author").strong());
            ui.label(
                RichText::new(
                    "The name and email Git writes into each commit. Signing in to GitHub doesn't set them: \
                     it lets you push. GitHub links a commit to your account by its email, so use one on your \
                     account (or your GitHub noreply address).",
                )
                .small()
                .color(pal.text_dim),
            );
            ui.add(egui::TextEdit::singleline(&mut form.name).hint_text("Name").desired_width(f32::INFINITY));
            ui.add(egui::TextEdit::singleline(&mut form.email).hint_text("Email").desired_width(f32::INFINITY));
            ui.radio_value(&mut form.global, true, "For all repositories (git config --global)");
            if in_repo {
                ui.radio_value(&mut form.global, false, "For this repository only");
            }
            if let Some(error) = &form.error {
                ui.label(RichText::new(error).small().color(pal.modified));
            }
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel {
            self.git_setup.form = None;
        }
        if save && let Some(form) = &mut self.git_setup.form {
            let result = git::check_identity(&form.name, &form.email).and_then(|()| {
                match (form.global, self.git.repo_for_setup()) {
                    (false, Some(repo)) => repo.set_identity(form.name.trim(), form.email.trim()),
                    _ => git::set_global_identity(form.name.trim(), form.email.trim()),
                }
            });
            match result {
                Ok(()) => {
                    self.status = Some(format!("Commits will be made as {} <{}>", form.name.trim(), form.email.trim()));
                    self.git_setup.form = None;
                    self.git_setup_stale();
                }
                Err(e) => form.error = Some(e),
            }
        }
    }

    /// Where the branch goes: its remote branch, or that it isn't on one yet.
    pub(super) fn git_upstream_line(
        &self,
        ui: &mut egui::Ui,
        branch: &str,
        upstream: Option<&str>,
        ahead: u32,
        behind: u32,
    ) {
        let pal = theme::colors();
        let has_remote = self.git_setup.info.as_ref().is_some_and(|i| !i.remotes.is_empty());
        let text = match upstream {
            Some(up) if ahead == 0 && behind == 0 => format!("{branch} is up to date with {up}"),
            Some(up) => {
                let mut parts = Vec::new();
                if ahead > 0 {
                    parts.push(format!("{ahead} to push"));
                }
                if behind > 0 {
                    parts.push(format!("{behind} to pull"));
                }
                format!("{branch} follows {up}: {}", parts.join(", "))
            }
            None if has_remote => format!("{branch} isn't on the remote yet: Push publishes it"),
            None => "No remote yet: add one under Remotes, or publish to GitHub".to_owned(),
        };
        ui.label(RichText::new(text).small().color(pal.text_dim));
    }

    /// The repository's remotes: each with an ×, a form to add one, and GitHub.
    pub(super) fn git_remotes_section(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let Some(info) = self.git_setup.info.clone() else { return };
        let header = format!("REMOTES  {}", info.remotes.len());
        let open = self.git_setup.remotes_open;
        let chevron = if open { "▾" } else { "▸" };
        let resp = ui
            .add(egui::Label::new(RichText::new(format!("{chevron} {header}")).small().strong()).sense(Sense::click()));
        if resp.clicked() {
            self.git_setup.remotes_open = !open;
        }
        if !open {
            return;
        }
        let mut remove = None;
        // Rows as wide as the side bar: a long URL is cut short, never widens it.
        let width = ui.available_width();
        let row_h = ui.spacing().interact_size.y;
        for (name, url) in &info.remotes {
            let right_to_left = egui::Layout::right_to_left(egui::Align::Center);
            ui.allocate_ui_with_layout(egui::vec2(width, row_h), right_to_left, |ui| {
                if ui.small_button("×").on_hover_text(format!("Remove the remote {name}")).clicked() {
                    remove = Some(name.clone());
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(RichText::new(name).strong());
                    ui.add(egui::Label::new(RichText::new(url).small().color(pal.text_dim)).truncate())
                        .on_hover_text(url);
                });
            });
        }
        if let (Some(name), Some(repo)) = (remove, self.git_repo().cloned()) {
            self.status = Some(match repo.remove_remote(&name) {
                Ok(()) => format!("Removed the remote {name}"),
                Err(e) => format!("Git: {}", git::explain(&e)),
            });
            self.git_setup_stale();
        }
        let left_to_right = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(egui::vec2(width, row_h), left_to_right, |ui| {
            let name_hint = if info.remotes.is_empty() { "origin" } else { "name" };
            ui.add(
                egui::TextEdit::singleline(&mut self.git_setup.remote_name).hint_text(name_hint).desired_width(70.0),
            );
            let rest = ui.available_width();
            ui.add(
                egui::TextEdit::singleline(&mut self.git_setup.remote_url)
                    .hint_text("https://github.com/you/repo.git")
                    .desired_width(rest),
            );
        });
        ui.horizontal(|ui| {
            let ready = !self.git_setup.remote_url.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Add Remote")).clicked()
                && let Some(repo) = self.git_repo().cloned()
            {
                let typed = self.git_setup.remote_name.trim();
                let name = if typed.is_empty() && info.remotes.is_empty() { "origin" } else { typed }.to_owned();
                let url = self.git_setup.remote_url.trim().to_owned();
                match repo.add_remote(&name, &url) {
                    Ok(()) => {
                        self.status = Some(format!("Added the remote {name}"));
                        self.git_setup.remote_name.clear();
                        self.git_setup.remote_url.clear();
                        self.git_setup_stale();
                    }
                    Err(e) => self.status = Some(format!("Git: {}", git::explain(&e))),
                }
            }
            let tip = if info.github_cli {
                "Create this repository on GitHub, add it as origin and push (GitHub CLI)"
            } else {
                "Needs the GitHub CLI (gh), signed in with gh auth login"
            };
            if ui
                .add_enabled(self.git_setup.publishing.is_none(), egui::Button::new("Publish to GitHub..."))
                .on_hover_text(tip)
                .clicked()
            {
                self.git_publish_start();
            }
        });
    }

    /// Runs the setup commands.
    pub(super) fn git_setup_command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::GitClone => self.git_clone_start(),
            Command::GitInit => self.git_init(ctx),
            Command::GitSetAuthor => self.git_author_form(),
            Command::GitPublishGitHub => self.git_publish_start(),
            _ => {}
        }
    }
}
