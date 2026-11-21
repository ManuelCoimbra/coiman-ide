//! HTTP: `.http` files get "▶ Send" above each request; the HTTP Client window shows the
//! response, the environment used, the file's history of responses, and two compared.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::language::lsp::types as lt;
use coiman_core::project::http_client::{self, Entry, Response};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const GREEN: Color32 = Color32::from_rgb(0x57, 0xC0, 0x6A);
const AMBER: Color32 = Color32::from_rgb(0xF2, 0x9E, 0x4C);

/// The lens commands.
pub(crate) const SEND: &str = "coiman.http.send";
pub(crate) const CURL: &str = "coiman.http.curl";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Body,
    Headers,
    History,
    Compare,
}

struct Sent {
    file: PathBuf,
    name: Option<String>,
    label: String,
    method: String,
    url: String,
    result: Result<Response, String>,
}

#[derive(Default)]
pub(crate) struct HttpView {
    open: bool,
    /// The `.http` file the window is about.
    file: Option<PathBuf>,
    /// The environment chosen, by file's folder.
    env: HashMap<PathBuf, String>,
    envs: Vec<(String, HashMap<String, String>)>,
    /// Responses of named requests this session, by file: what `{{name.response…}}` reads.
    named: HashMap<PathBuf, HashMap<String, Response>>,
    running: Option<(String, Receiver<Sent>)>,
    shown: Option<Entry>,
    error: Option<String>,
    history: Vec<Entry>,
    tab: Tab,
    /// Two history entries to compare (indexes).
    pick: (Option<usize>, Option<usize>),
    /// Lenses made for (doc uid → buffer version).
    lensed: HashMap<u64, u64>,
}

fn status_color(status: u16) -> Color32 {
    match status {
        200..=299 => GREEN,
        300..=399 => AMBER,
        _ => RED,
    }
}

fn bytes(n: usize) -> String {
    coiman_core::build::binary_size::human(n as u64)
}

fn when(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

impl CodeApp {
    /// "▶ Send" and "Copy as curl" above each request of the open `.http` files.
    pub(crate) fn pump_http(&mut self) {
        let mut todo = Vec::new();
        for doc in &self.docs {
            if !http_client::is_http_file(&doc.extension()) {
                continue;
            }
            if self.http.lensed.get(&doc.uid) == Some(&doc.buffer.version()) {
                continue;
            }
            let file = http_client::parse(&doc.text());
            let lenses: Vec<lt::CodeLens> = file
                .requests
                .iter()
                .flat_map(|r| {
                    let range =
                        lt::Range::new(lt::Position::new(r.line as u32, 0), lt::Position::new(r.line as u32, 0));
                    let arg = Some(vec![serde_json::json!(r.line)]);
                    [
                        lt::CodeLens {
                            range,
                            command: Some(lt::Command {
                                title: "▶ Send".into(),
                                command: SEND.into(),
                                arguments: arg.clone(),
                            }),
                            data: None,
                        },
                        lt::CodeLens {
                            range,
                            command: Some(lt::Command {
                                title: "Copy as curl".into(),
                                command: CURL.into(),
                                arguments: arg,
                            }),
                            data: None,
                        },
                    ]
                })
                .collect();
            todo.push((doc.uid, doc.buffer.version(), lenses));
        }
        for (uid, version, lenses) in todo {
            self.http.lensed.insert(uid, version);
            self.set_local_lenses(uid, lenses);
        }
    }

    /// The `.http` text, path and request at `line` of the active document.
    fn http_request_at(&self, line: Option<usize>) -> Option<(PathBuf, http_client::File, http_client::Request)> {
        let doc = self.docs.get(self.active)?;
        if !http_client::is_http_file(&doc.extension()) {
            return None;
        }
        let path = doc.path.clone().unwrap_or_else(|| PathBuf::from(format!("untitled-{}.http", doc.uid)));
        let line = line.unwrap_or_else(|| doc.buffer.rope().char_to_line(doc.buffer.primary().head));
        let file = http_client::parse(&doc.text());
        let req = http_client::request_at(&file, line)?.clone();
        Some((path, file, req))
    }

    fn http_ready(
        &mut self,
        path: &Path,
        file: &http_client::File,
        req: &http_client::Request,
    ) -> Result<http_client::Ready, String> {
        let root = self.explorer.root.clone();
        self.http.envs = http_client::environments(path, root.as_deref());
        let key = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let env_name = self.http.env.get(&key).cloned();
        let env = env_name.as_ref().and_then(|n| self.http.envs.iter().find(|(e, _)| e == n)).map(|(_, v)| v);
        let empty = HashMap::new();
        let named = self.http.named.get(path).unwrap_or(&empty);
        let values = http_client::Values::new(file, env, named, path, root.as_deref());
        http_client::prepare(req, &values, path)
    }

    /// HTTP: Send Request at Cursor (or at `line`, from its lens).
    pub(crate) fn http_send(&mut self, ctx: &egui::Context, line: Option<usize>) {
        let Some((path, file, req)) = self.http_request_at(line) else {
            self.status = Some("Put the cursor in a request of a .http file".to_owned());
            return;
        };
        self.http.open = true;
        if self.http.file.as_ref() != Some(&path) {
            self.http.history = http_client::history(&path);
            self.http.pick = (None, None);
        }
        self.http.file = Some(path.clone());
        let ready = match self.http_ready(&path, &file, &req) {
            Ok(r) => r,
            Err(e) => {
                self.http.error = Some(e);
                return;
            }
        };
        self.http.error = None;
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let (name, label) = (req.name.clone(), req.label());
        self.http.running = Some((format!("{} {}", ready.method, ready.url), rx));
        std::thread::spawn(move || {
            let result = http_client::send(&ready);
            let _ = tx.send(Sent { file: path, name, label, method: ready.method, url: ready.url, result });
            wake();
        });
    }

    /// Copies the request at `line` (or the cursor) as a curl command, values filled in.
    pub(crate) fn http_copy_curl(&mut self, ctx: &egui::Context, line: Option<usize>) {
        let Some((path, file, req)) = self.http_request_at(line) else { return };
        match self.http_ready(&path, &file, &req) {
            Ok(ready) => {
                ctx.copy_text(http_client::curl_line(&ready));
                self.status = Some("Copied the request as a curl command".to_owned());
            }
            Err(e) => self.status = Some(e),
        }
    }

    pub(crate) fn show_http(&mut self) {
        self.http.open = true;
        if let Some(doc) = self.docs.get(self.active)
            && http_client::is_http_file(&doc.extension())
            && let Some(path) = doc.path.clone()
        {
            if self.http.file.as_ref() != Some(&path) {
                self.http.history = http_client::history(&path);
                self.http.pick = (None, None);
            }
            self.http.envs = http_client::environments(&path, self.explorer.root.as_deref());
            self.http.file = Some(path);
        }
    }

    pub(crate) fn http_window(&mut self, ctx: &egui::Context) {
        if let Some((_, rx)) = &self.http.running
            && let Ok(sent) = rx.try_recv()
        {
            self.http.running = None;
            match sent.result {
                Ok(resp) => {
                    if let Some(name) = &sent.name {
                        self.http.named.entry(sent.file.clone()).or_default().insert(name.clone(), resp.clone());
                    }
                    let entry = Entry {
                        when: chrono::Utc::now().timestamp(),
                        request: sent.label,
                        method: sent.method,
                        url: sent.url,
                        response: resp,
                    };
                    match http_client::remember(&sent.file, entry.clone()) {
                        Ok(all) => self.http.history = all,
                        Err(e) => self.status = Some(format!("Couldn't keep the response: {e}")),
                    }
                    self.status = Some(format!(
                        "{} {} in {:.0} ms",
                        entry.response.status, entry.response.reason, entry.response.millis
                    ));
                    // Older picks now point one further down.
                    let shift = |p: Option<usize>| p.map(|i| i + 1);
                    self.http.pick = (shift(self.http.pick.0), shift(self.http.pick.1));
                    self.http.shown = Some(entry);
                    if self.http.tab == Tab::Compare || self.http.tab == Tab::History {
                        self.http.tab = Tab::Body;
                    }
                }
                Err(e) => self.http.error = Some(e),
            }
        }
        if !self.http.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut send = false;
        let mut open_body = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.45).clamp(420.0, 760.0), (room.y * 0.75).max(360.0));
        egui::Window::new(RichText::new("HTTP Client").size(16.0).strong())
            .id(egui::Id::unique("http_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .max_height(room.y - 40.0)
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-size.x - 24.0, 56.0))
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ctx, |ui| {
                let v = &mut self.http;
                let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
                ui.horizontal(|ui| {
                    let name = v
                        .file
                        .as_ref()
                        .and_then(|f| f.file_name())
                        .map_or("No .http file yet".into(), |n| n.to_string_lossy().into_owned());
                    ui.label(RichText::new(name).strong());
                    if let Some(file) = v.file.clone() {
                        let key = file.parent().map(Path::to_path_buf).unwrap_or_default();
                        let current = v.env.get(&key).cloned();
                        egui::ComboBox::from_id_salt("http_env")
                            .selected_text(current.clone().unwrap_or_else(|| "No environment".into()))
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(current.is_none(), "No environment").clicked() {
                                    v.env.remove(&key);
                                }
                                for (name, _) in &v.envs {
                                    if ui.selectable_label(current.as_ref() == Some(name), name).clicked() {
                                        v.env.insert(key.clone(), name.clone());
                                    }
                                }
                            })
                            .response
                            .on_hover_text(
                                "From http-client.env.json (and .private), or VS Code's rest-client settings",
                            );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Send").on_hover_text("The request at the cursor").clicked() {
                            send = true;
                        }
                    });
                });
                if let Some((what, _)) = &v.running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(dim(what));
                    });
                }
                if let Some(e) = &v.error {
                    ui.colored_label(RED, e);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (Tab::Body, "Body"),
                        (Tab::Headers, "Headers"),
                        (Tab::History, "History"),
                        (Tab::Compare, "Compare"),
                    ] {
                        let label = if tab == Tab::History {
                            format!("History  {}", v.history.len())
                        } else {
                            label.to_owned()
                        };
                        if ui.selectable_label(v.tab == tab, label).clicked() {
                            v.tab = tab;
                        }
                    }
                });
                if let Some(entry) = &v.shown
                    && matches!(v.tab, Tab::Body | Tab::Headers)
                {
                    let r = &entry.response;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(format!("{} {}", r.status, r.reason)).strong().color(status_color(r.status)),
                        );
                        ui.label(dim(&format!("{:.0} ms · {} · {}", r.millis, bytes(r.body.len()), r.version)));
                        ui.label(dim(&format!("{} {}", entry.method, r.url)));
                    });
                }
                let height = ui.available_height().min(room.y * 0.75 - 140.0).max(160.0);
                ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| match v.tab {
                    Tab::Body => match &v.shown {
                        None => {
                            ui.label(dim(
                                "Send a request: ▶ Send above it in a .http file, or Send here with the cursor in it.",
                            ));
                        }
                        Some(entry) => {
                            ui.horizontal(|ui| {
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(entry.response.pretty());
                                }
                                if ui.small_button("Open in a tab").clicked() {
                                    open_body = Some(entry.response.pretty());
                                }
                            });
                            let text = entry.response.pretty();
                            egui::ScrollArea::both().id_salt("http_body").auto_shrink([false, false]).show(ui, |ui| {
                                ui.label(
                                    RichText::new(if text.is_empty() { "(no body)".to_owned() } else { text })
                                        .monospace(),
                                );
                            });
                        }
                    },
                    Tab::Headers => {
                        if let Some(entry) = &v.shown {
                            egui::ScrollArea::vertical().id_salt("http_headers").auto_shrink([false, false]).show(
                                ui,
                                |ui| {
                                    egui::Grid::new("http_headers_grid").striped(true).num_columns(2).show(ui, |ui| {
                                        for (k, val) in &entry.response.headers {
                                            ui.label(RichText::new(k).monospace().strong());
                                            ui.label(RichText::new(val).monospace());
                                            ui.end_row();
                                        }
                                    });
                                },
                            );
                        }
                    }
                    Tab::History => {
                        if v.history.is_empty() {
                            ui.label(dim("Responses to this file's requests are kept here (the last 50)."));
                        }
                        ui.label(dim("Click one to show it; A and B choose two to compare."));
                        let mut show = None;
                        egui::ScrollArea::vertical().id_salt("http_history").auto_shrink([false, false]).show(
                            ui,
                            |ui| {
                                egui::Grid::new("http_history_grid").striped(true).num_columns(5).show(ui, |ui| {
                                    for (i, e) in v.history.iter().enumerate() {
                                        ui.horizontal(|ui| {
                                            if ui.selectable_label(v.pick.0 == Some(i), "A").clicked() {
                                                v.pick.0 = Some(i);
                                            }
                                            if ui.selectable_label(v.pick.1 == Some(i), "B").clicked() {
                                                v.pick.1 = Some(i);
                                            }
                                        });
                                        ui.label(dim(&when(e.when)));
                                        ui.label(
                                            RichText::new(e.response.status.to_string())
                                                .color(status_color(e.response.status)),
                                        );
                                        if ui
                                            .link(format!("{} {}", e.method, e.url))
                                            .on_hover_text(&e.request)
                                            .clicked()
                                        {
                                            show = Some(i);
                                        }
                                        ui.label(dim(&format!("{:.0} ms", e.response.millis)));
                                        ui.end_row();
                                    }
                                });
                            },
                        );
                        if let Some(i) = show {
                            v.shown = v.history.get(i).cloned();
                            v.tab = Tab::Body;
                        }
                    }
                    Tab::Compare => {
                        let (Some(a), Some(b)) =
                            (v.pick.0.and_then(|i| v.history.get(i)), v.pick.1.and_then(|i| v.history.get(i)))
                        else {
                            ui.label(dim("Choose A and B in History."));
                            return;
                        };
                        ui.label(dim(&format!(
                            "A: {} {} at {}   B: {} {} at {}",
                            a.method,
                            a.url,
                            when(a.when),
                            b.method,
                            b.url,
                            when(b.when)
                        )));
                        let lines = http_client::compare(&a.response, &b.response);
                        if lines.iter().all(|(m, _)| *m == ' ') {
                            ui.label(RichText::new("The same, but for dates and request ids.").color(GREEN));
                        }
                        egui::ScrollArea::both().id_salt("http_compare").auto_shrink([false, false]).show(ui, |ui| {
                            for (mark, line) in &lines {
                                let color = match mark {
                                    '-' => RED,
                                    '+' => GREEN,
                                    _ => pal.text_dim,
                                };
                                ui.label(RichText::new(format!("{mark} {line}")).monospace().color(color));
                            }
                        });
                    }
                });
                if !v.history.is_empty()
                    && v.tab == Tab::History
                    && ui.small_button("Forget this file's history").clicked()
                    && let Some(f) = &v.file
                {
                    http_client::clear_history(f);
                    v.history.clear();
                    v.pick = (None, None);
                }
            });
        self.http.open = open;
        if send {
            self.http_send(ctx, None);
        }
        if let Some(text) = open_body {
            self.untitled_counter += 1;
            let doc = coiman_core::editor::document::Document::untitled_with(self.untitled_counter, &text);
            self.add_doc(doc);
        }
    }
}
