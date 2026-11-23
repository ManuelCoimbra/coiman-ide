//! Agents: MCP Servers: the MCP servers opencode gives the agent, for this project
//! (`opencode.json` in its folder) or for every project (`~/.config/opencode/opencode.json`):
//! listed, added, edited, turned on and off, removed. Saving writes only their section.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver};

use coiman_core::extensions::opencode_mcp::{self, Config, Kind, Server};
use coiman_core::language::lsp::discovery::quiet_command;
use eframe::egui::{self, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Scope {
    #[default]
    Project,
    User,
}

/// A server being added or edited, as typed.
#[derive(Default)]
struct Draft {
    /// The server's index when editing one.
    editing: Option<usize>,
    name: String,
    remote: bool,
    command: String,
    url: String,
    /// `KEY=value`, one a line: environment (local) or headers (remote).
    pairs: String,
}

#[derive(Default)]
pub(crate) struct McpView {
    open: bool,
    scope: Scope,
    config: Option<Result<Config, String>>,
    dirty: bool,
    draft: Option<Draft>,
    /// `opencode mcp list`, when asked: what opencode makes of them.
    status: Option<Receiver<String>>,
    status_text: Option<String>,
}

fn pairs_text(pairs: &[(String, String)]) -> String {
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("\n")
}

fn parse_pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

/// A line of `opencode mcp list` without its frame: the box line or bullet in front (`│`, `●`,
/// or `|`, `•`, `T` where it can't draw them), which is one character then two spaces or nothing.
/// `None` for a line that was only frame.
fn drop_frame(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    let mut chars = rest.chars();
    if chars.next().is_some() {
        let after = chars.as_str();
        if after.is_empty() || after.starts_with("  ") {
            rest = after.trim_start();
        }
    }
    (!rest.is_empty()).then_some(rest)
}

impl Draft {
    fn of(i: usize, s: &Server) -> Self {
        let mut d = Draft { editing: Some(i), name: s.name.clone(), ..Draft::default() };
        match &s.kind {
            Kind::Local { command, env } => {
                d.command = coiman_core::build::compile_db::join_command(command);
                d.pairs = pairs_text(env);
            }
            Kind::Remote { url, headers } => {
                d.remote = true;
                d.url = url.clone();
                d.pairs = pairs_text(headers);
            }
        }
        d
    }

    fn server(&self, enabled: bool) -> Server {
        let kind = if self.remote {
            Kind::Remote { url: self.url.trim().to_owned(), headers: parse_pairs(&self.pairs) }
        } else {
            Kind::Local {
                command: coiman_core::build::compile_db::split_command(self.command.trim()),
                env: parse_pairs(&self.pairs),
            }
        };
        Server { name: self.name.trim().to_owned(), enabled, kind }
    }
}

impl CodeApp {
    fn mcp_file(&self) -> Option<PathBuf> {
        match self.mcp.scope {
            Scope::Project => self.explorer.root.as_deref().map(opencode_mcp::project_file),
            Scope::User => opencode_mcp::user_file(),
        }
    }

    fn mcp_load(&mut self) {
        self.mcp.config = self.mcp_file().map(|f| Config::load(&f));
        self.mcp.dirty = false;
        self.mcp.draft = None;
        self.mcp.status_text = None;
    }

    pub(crate) fn show_mcp(&mut self) {
        self.mcp.open = true;
        if self.explorer.root.is_none() {
            self.mcp.scope = Scope::User;
        }
        self.mcp_load();
    }

    /// `opencode mcp list` in the project's folder: which servers it connects to.
    fn mcp_check(&mut self, ctx: &egui::Context) {
        let Some(program) = coiman_core::language::lsp::servers::which("opencode") else {
            self.mcp.status_text = Some("OpenCode isn't installed: npm install -g opencode-ai".to_owned());
            return;
        };
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let out = quiet_command(&program).args(["mcp", "list"]).current_dir(cwd).stdin(Stdio::null()).output();
            let text = match out {
                Ok(o) => {
                    let all = String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr);
                    // Its colors and box lines aren't wanted here.
                    let plain: String = coiman_core::terminal::strip_colors(&all);
                    plain.lines().filter_map(drop_frame).collect::<Vec<_>>().join("\n")
                }
                Err(e) => format!("opencode: {e}"),
            };
            let _ = tx.send(text);
            wake();
        });
        self.mcp.status = Some(rx);
        self.mcp.status_text = Some("Asking opencode…".to_owned());
    }

    pub(crate) fn mcp_window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.mcp.status
            && let Ok(text) = rx.try_recv()
        {
            self.mcp.status = None;
            self.mcp.status_text = Some(text);
        }
        if !self.mcp.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut save = false;
        let mut reload = None;
        let mut check = false;
        let file = self.mcp_file();
        egui::Window::new(RichText::new("MCP Servers").size(16.0).strong())
            .id(egui::Id::unique("mcp_window"))
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(320.0, 260.0))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ctx, |ui| {
                let dim = |s: &str| RichText::new(s.to_owned()).small().color(pal.text_dim);
                ui.label(dim(
                    "The tools the Agents panel's opencode gets: each a program it starts, or a server at an address.",
                ));
                ui.horizontal(|ui| {
                    let has_root = self.explorer.root.is_some();
                    if ui
                        .add_enabled(
                            has_root,
                            egui::Button::selectable(self.mcp.scope == Scope::Project, "This project"),
                        )
                        .clicked()
                    {
                        reload = Some(Scope::Project);
                    }
                    if ui.selectable_label(self.mcp.scope == Scope::User, "Every project").clicked() {
                        reload = Some(Scope::User);
                    }
                    if let Some(f) = &file {
                        ui.label(dim(&f.display().to_string()));
                    }
                });
                ui.separator();
                let config = match &mut self.mcp.config {
                    None => {
                        ui.label("Open a folder, or choose Every project.");
                        return;
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e.as_str()).color(pal.bracket_unexpected));
                        return;
                    }
                    Some(Ok(c)) => c,
                };
                let mut remove = None;
                let mut edit = None;
                if config.servers.is_empty() && self.mcp.draft.is_none() {
                    ui.label(dim("No MCP servers yet."));
                }
                egui::Grid::new("mcp_grid").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (i, s) in config.servers.iter_mut().enumerate() {
                        if ui.checkbox(&mut s.enabled, "").on_hover_text("On for the agent").changed() {
                            self.mcp.dirty = true;
                        }
                        ui.label(RichText::new(&s.name).strong());
                        let what = match &s.kind {
                            Kind::Local { command, .. } => coiman_core::build::compile_db::join_command(command),
                            Kind::Remote { url, .. } => url.clone(),
                        };
                        // A long command is cut, whole in its tooltip.
                        let short = if what.chars().count() > 64 {
                            what.chars().take(63).collect::<String>() + "…"
                        } else {
                            what.clone()
                        };
                        ui.label(RichText::new(short).monospace().small()).on_hover_text(what);
                        ui.horizontal(|ui| {
                            if ui.small_button("Edit").clicked() {
                                edit = Some(i);
                            }
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
                if let Some(i) = remove {
                    config.servers.remove(i);
                    self.mcp.dirty = true;
                }
                if let Some(i) = edit {
                    self.mcp.draft = Some(Draft::of(i, &config.servers[i]));
                }
                ui.add_space(6.0);
                match &mut self.mcp.draft {
                    None => {
                        if ui.button("+ Add a server").clicked() {
                            self.mcp.draft = Some(Draft::default());
                        }
                    }
                    Some(d) => {
                        let mut done = None;
                        egui::Frame::new()
                            .stroke(Stroke::new(1.0, pal.border))
                            .corner_radius(6.0)
                            .inner_margin(8.0)
                            .show(ui, |ui| {
                                egui::Grid::new("mcp_draft").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                                    ui.label("Name");
                                    ui.add(
                                        egui::TextEdit::singleline(&mut d.name).hint_text("files").desired_width(380.0),
                                    );
                                    ui.end_row();
                                    ui.label("Kind");
                                    ui.horizontal(|ui| {
                                        ui.radio_value(&mut d.remote, false, "A program (local)");
                                        ui.radio_value(&mut d.remote, true, "An address (remote)");
                                    });
                                    ui.end_row();
                                    if d.remote {
                                        ui.label("Address");
                                        ui.add(
                                            egui::TextEdit::singleline(&mut d.url)
                                                .hint_text("https://mcp.example.com/mcp")
                                                .desired_width(380.0),
                                        );
                                        ui.end_row();
                                        ui.label("Headers");
                                        ui.add(
                                            egui::TextEdit::multiline(&mut d.pairs)
                                                .hint_text("Authorization=Bearer …")
                                                .desired_rows(2)
                                                .desired_width(380.0),
                                        );
                                    } else {
                                        ui.label("Command");
                                        ui.add(
                                            egui::TextEdit::singleline(&mut d.command)
                                                .hint_text("npx -y @modelcontextprotocol/server-filesystem .")
                                                .desired_width(380.0),
                                        );
                                        ui.end_row();
                                        ui.label("Environment");
                                        ui.add(
                                            egui::TextEdit::multiline(&mut d.pairs)
                                                .hint_text("KEY=value, one a line")
                                                .desired_rows(2)
                                                .desired_width(380.0),
                                        );
                                    }
                                    ui.end_row();
                                });
                                let others: Vec<Server> = config
                                    .servers
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| Some(*i) != d.editing)
                                    .map(|(_, s)| s.clone())
                                    .collect();
                                let enabled = d.editing.and_then(|i| config.servers.get(i)).is_none_or(|s| s.enabled);
                                let server = d.server(enabled);
                                let problem = opencode_mcp::problem(&server, &others);
                                ui.horizontal(|ui| {
                                    let label = if d.editing.is_some() { "Update" } else { "Add" };
                                    if ui.add_enabled(problem.is_none(), egui::Button::new(label)).clicked() {
                                        done = Some(Some(server));
                                    }
                                    if ui.button("Cancel").clicked() {
                                        done = Some(None);
                                    }
                                    if let Some(p) = problem {
                                        ui.label(dim(p));
                                    }
                                });
                            });
                        if let Some(result) = done {
                            if let Some(server) = result {
                                match self.mcp.draft.as_ref().and_then(|d| d.editing) {
                                    Some(i) => config.servers[i] = server,
                                    None => config.servers.push(server),
                                }
                                self.mcp.dirty = true;
                            }
                            self.mcp.draft = None;
                        }
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.add_enabled(self.mcp.dirty, egui::Button::new("Save")).clicked() {
                        save = true;
                    }
                    if ui
                        .button("Check with opencode")
                        .on_hover_text("opencode mcp list: which servers it reaches")
                        .clicked()
                    {
                        check = true;
                    }
                    if config.had_comments && self.mcp.dirty {
                        ui.label(RichText::new("Saving drops the file's comments").small().color(pal.modified));
                    }
                });
                if let Some(text) = &self.mcp.status_text {
                    ui.add(egui::Label::new(RichText::new(text).monospace().small()).wrap());
                }
            });
        self.mcp.open = open;
        if save && let (Some(Ok(c)), Some(f)) = (self.mcp.config.as_ref(), file) {
            match c.save(&f) {
                Ok(()) => {
                    self.mcp.dirty = false;
                    self.status = Some(format!("Saved the MCP servers in {}", f.display()));
                }
                Err(e) => self.status = Some(format!("Couldn't save: {e}")),
            }
        }
        if let Some(scope) = reload {
            self.mcp.scope = scope;
            self.mcp_load();
        }
        if check {
            self.mcp_check(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_without_its_frame() {
        let drawn = "┌  MCP Servers
│
●  ✓ tiny connected
│      python tiny.py
└  2 server(s)";
        let plain = "T  MCP Servers
|
•  ○ broken disabled
|      no-such-program";
        fn tidy(t: &str) -> Vec<&str> {
            t.lines().filter_map(drop_frame).collect()
        }
        assert_eq!(tidy(drawn), ["MCP Servers", "✓ tiny connected", "python tiny.py", "2 server(s)"]);
        assert_eq!(tidy(plain), ["MCP Servers", "○ broken disabled", "no-such-program"]);
        assert_eq!(drop_frame("x"), None);
        assert_eq!(drop_frame("tiny connected"), Some("tiny connected"));
    }
}
