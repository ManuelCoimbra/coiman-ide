//! OpenAPI: Show Endpoints: a spec's endpoints by tag and its models; an endpoint opens as a
//! request in a `.http` file beside the spec, ready for ▶ Send.

use std::path::PathBuf;

use coiman_core::language::lsp::types as lt;
use coiman_core::project::openapi::{self, Endpoint, Spec};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::theme;

/// The lens command on a spec's first line.
pub(crate) const SHOW: &str = "coiman.openapi.show";

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Endpoints,
    Models,
}

#[derive(Default)]
pub(crate) struct OpenApiView {
    open: bool,
    file: Option<PathBuf>,
    spec: Option<Result<Spec, String>>,
    text: String,
    tab: Tab,
    filter: String,
    selected: Option<usize>,
    /// Lenses made for (doc uid → buffer version).
    lensed: std::collections::HashMap<u64, u64>,
}

fn method_color(m: &str) -> Color32 {
    match m {
        "GET" => Color32::from_rgb(0x57, 0xC0, 0x6A),
        "POST" => Color32::from_rgb(0x4F, 0x9D, 0xFF),
        "PUT" => Color32::from_rgb(0xF2, 0x9E, 0x4C),
        "PATCH" => Color32::from_rgb(0xB0, 0x7C, 0xF7),
        "DELETE" => Color32::from_rgb(0xE5, 0x48, 0x4D),
        _ => Color32::GRAY,
    }
}

enum Ask {
    OpenRequest(usize),
    WholeFile,
    GoTo(usize),
}

impl CodeApp {
    /// A lens on a spec's first line: how many endpoints, and a way to see them.
    pub(crate) fn pump_openapi(&mut self) {
        let mut todo = Vec::new();
        for doc in &self.docs {
            let ext = doc.extension();
            if !matches!(ext.as_str(), "json" | "yaml" | "yml")
                || self.openapi.lensed.get(&doc.uid) == Some(&doc.buffer.version())
            {
                continue;
            }
            let text = doc.text();
            if !openapi::looks_like_spec(&ext, &text) {
                self.openapi.lensed.insert(doc.uid, doc.buffer.version());
                continue;
            }
            let title = match openapi::parse(&text) {
                Ok(s) => format!("▸ {} endpoints, {} models: show them", s.endpoints.len(), s.models.len()),
                Err(_) => "▸ OpenAPI: show what can be read".to_owned(),
            };
            let range = lt::Range::new(lt::Position::new(0, 0), lt::Position::new(0, 0));
            let lens = lt::CodeLens {
                range,
                command: Some(lt::Command { title, command: SHOW.into(), arguments: None }),
                data: None,
            };
            todo.push((doc.uid, doc.buffer.version(), lens));
        }
        for (uid, version, lens) in todo {
            self.openapi.lensed.insert(uid, version);
            self.set_local_lenses(uid, vec![lens]);
        }
    }

    /// OpenAPI: Show Endpoints, for the spec in the editor.
    pub(crate) fn show_openapi(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let text = doc.text();
        if !openapi::looks_like_spec(&doc.extension(), &text) {
            self.status = Some("Open an OpenAPI or Swagger file (JSON or YAML) first".to_owned());
            return;
        }
        let v = &mut self.openapi;
        v.open = true;
        v.file = doc.path.clone();
        v.spec = Some(openapi::parse(&text));
        v.text = text;
        v.selected = None;
    }

    /// The `.http` file beside the spec: `orders.yaml` → `orders.http`.
    fn openapi_http_path(&self) -> Option<PathBuf> {
        let spec = self.openapi.file.as_ref()?;
        Some(spec.with_extension("http"))
    }

    /// Adds the endpoint's request to the `.http` file (made with `@baseUrl` if new), unless it's
    /// there already, and shows it.
    fn openapi_open_request(&mut self, index: usize) {
        let Some(Ok(spec)) = &self.openapi.spec else { return };
        let Some(endpoint) = spec.endpoints.get(index) else { return };
        let request = spec.request(endpoint);
        let what = format!("{} {}", endpoint.method, endpoint.path);
        let head =
            format!("@baseUrl = {}\n", spec.servers.first().cloned().unwrap_or_else(|| "http://localhost:8080".into()));
        let Some(path) = self.openapi_http_path() else {
            self.status = Some("Save the spec first: its requests go in a .http file beside it".to_owned());
            return;
        };
        if !path.exists()
            && let Err(e) = std::fs::write(&path, &head)
        {
            self.status = Some(format!("Couldn't make {}: {e}", path.display()));
            return;
        }
        self.open_file(&path);
        let Some(doc) = self.active_doc() else { return };
        let text = doc.text();
        // Already there (by its name, or its first line): just show it.
        let first = request.lines().find(|l| !l.starts_with('#')).unwrap_or("").to_owned();
        let named = request.lines().find(|l| l.starts_with("# @name ")).map(str::to_owned);
        let found = text.lines().position(|l| named.as_deref() == Some(l) || l == first);
        let line = match found {
            Some(at) => at,
            None => {
                let end = doc.buffer.len_chars();
                doc.select(end, end);
                let sep = if text.ends_with("\n\n") || text.is_empty() {
                    ""
                } else if text.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                };
                doc.buffer.insert(&format!("{sep}{request}"));
                let now = doc.text();
                now.lines().enumerate().filter(|(_, l)| *l == first).last().map_or(0, |(i, _)| i)
            }
        };
        doc.goto_line(line + 1);
        self.status = Some(format!("{what} is in {}: ▶ Send above it", path.display()));
    }

    fn openapi_whole_file(&mut self) {
        let Some(Ok(spec)) = &self.openapi.spec else { return };
        let (file, count) = (spec.http_file(), spec.endpoints.len());
        let Some(path) = self.openapi_http_path() else { return };
        if path.exists() {
            self.open_file(&path);
            self.status =
                Some(format!("{} is there already: Open as request adds endpoints to it one by one", path.display()));
            return;
        }
        match std::fs::write(&path, file) {
            Ok(()) => {
                self.open_file(&path);
                self.status = Some(format!("Wrote {count} endpoints to {}", path.display()));
            }
            Err(e) => self.status = Some(format!("Couldn't write {}: {e}", path.display())),
        }
    }

    pub(crate) fn openapi_window(&mut self, ctx: &egui::Context) {
        if !self.openapi.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut ask = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.7).min(1000.0), (room.y * 0.75).max(380.0));
        egui::Window::new(RichText::new("OpenAPI").size(16.0).strong())
            .id(egui::Id::unique("openapi_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .max_height(room.y - 40.0)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| ask = openapi_ui(ui, &mut self.openapi, room.y * 0.75 - 130.0));
        self.openapi.open = open;
        match ask {
            Some(Ask::OpenRequest(i)) => self.openapi_open_request(i),
            Some(Ask::WholeFile) => self.openapi_whole_file(),
            Some(Ask::GoTo(i)) => {
                let path = self.openapi.file.clone();
                let line = self
                    .openapi
                    .spec
                    .as_ref()
                    .and_then(|s| s.as_ref().ok())
                    .and_then(|s| s.endpoints.get(i))
                    .and_then(|e| openapi::line_of_path(&self.openapi.text, &e.path));
                if let (Some(path), Some(line)) = (path, line) {
                    self.open_file(&path);
                    if let Some(doc) = self.active_doc() {
                        doc.goto_line(line + 1);
                    }
                }
            }
            None => {}
        }
    }
}

fn matches(e: &Endpoint, needle: &str) -> bool {
    needle.is_empty()
        || [&e.path, &e.summary, &e.method, e.operation_id.as_deref().unwrap_or("")]
            .iter()
            .any(|s| s.to_lowercase().contains(needle))
        || e.tags.iter().any(|t| t.to_lowercase().contains(needle))
}

fn openapi_ui(ui: &mut egui::Ui, v: &mut OpenApiView, body: f32) -> Option<Ask> {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    let mut ask = None;
    let spec = match &v.spec {
        None => return None,
        Some(Err(e)) => {
            ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
            return None;
        }
        Some(Ok(s)) => s,
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} {}", spec.title, spec.version)).strong());
        ui.label(dim(&spec.kind));
        for s in &spec.servers {
            ui.label(RichText::new(s).monospace().small());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("All as a .http file")
                .on_hover_text("Every endpoint as a request, in a .http file beside the spec")
                .clicked()
            {
                ask = Some(Ask::WholeFile);
            }
        });
    });
    ui.horizontal(|ui| {
        for (tab, label) in [
            (Tab::Endpoints, format!("Endpoints  {}", spec.endpoints.len())),
            (Tab::Models, format!("Models  {}", spec.models.len())),
        ] {
            if ui.selectable_label(v.tab == tab, label).clicked() {
                v.tab = tab;
            }
        }
        ui.add(egui::TextEdit::singleline(&mut v.filter).hint_text("Filter").desired_width(220.0));
    });
    ui.separator();
    let needle = v.filter.to_lowercase();
    let height = body.max(200.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| match v.tab {
        Tab::Endpoints => {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() * 0.45).max(280.0));
                    egui::ScrollArea::vertical().id_salt("openapi_list").auto_shrink([false, false]).show(ui, |ui| {
                        for (tag, list) in spec.by_tag() {
                            let shown: Vec<usize> =
                                list.into_iter().filter(|&i| matches(&spec.endpoints[i], &needle)).collect();
                            if shown.is_empty() {
                                continue;
                            }
                            ui.label(RichText::new(tag).strong());
                            for i in shown {
                                let e = &spec.endpoints[i];
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("{:<6}", e.method))
                                            .monospace()
                                            .strong()
                                            .color(method_color(&e.method)),
                                    );
                                    let mut path = RichText::new(&e.path).monospace();
                                    if e.deprecated {
                                        path = path.strikethrough().color(pal.text_dim);
                                    }
                                    if ui
                                        .selectable_label(v.selected == Some(i), path)
                                        .on_hover_text(&e.summary)
                                        .clicked()
                                    {
                                        v.selected = Some(i);
                                    }
                                });
                            }
                            ui.add_space(4.0);
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let Some(e) = v.selected.and_then(|i| spec.endpoints.get(i)) else {
                        ui.label(dim("Choose an endpoint."));
                        return;
                    };
                    let i = v.selected.unwrap_or_default();
                    egui::ScrollArea::vertical().id_salt("openapi_detail").auto_shrink([false, false]).show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&e.method).monospace().strong().color(method_color(&e.method)));
                            ui.label(RichText::new(&e.path).monospace().strong());
                            if e.deprecated {
                                ui.label(dim("(deprecated)"));
                            }
                        });
                        if !e.summary.is_empty() {
                            ui.label(&e.summary);
                        }
                        if !e.description.is_empty() {
                            ui.label(dim(&e.description));
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .button("Open as request")
                                .on_hover_text("In the .http file beside the spec, ready to send")
                                .clicked()
                            {
                                ask = Some(Ask::OpenRequest(i));
                            }
                            if ui.button("Copy request").clicked() {
                                ui.ctx().copy_text(spec.request(e));
                            }
                            if ui.button("Go to it").on_hover_text("Where the spec declares it").clicked() {
                                ask = Some(Ask::GoTo(i));
                            }
                        });
                        if !e.security.is_empty() {
                            ui.label(dim(&format!("Needs: {}", e.security.join(", "))));
                        }
                        if !e.params.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Parameters").strong());
                            egui::Grid::new("openapi_params").striped(true).num_columns(4).show(ui, |ui| {
                                for p in &e.params {
                                    ui.label(RichText::new(&p.name).monospace().strong());
                                    ui.label(dim(&p.place));
                                    ui.label(RichText::new(&p.ty).monospace().small());
                                    ui.label(dim(&format!(
                                        "{}{}",
                                        if p.required { "required. " } else { "" },
                                        p.description
                                    )));
                                    ui.end_row();
                                }
                            });
                        }
                        if let (Some((media, schema)), Some(example)) = (&e.body, spec.body_example(e)) {
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "Body: {} ({media}){}",
                                    openapi::type_name(schema),
                                    if e.body_required { ", required" } else { "" }
                                ))
                                .strong(),
                            );
                            ui.label(RichText::new(example).monospace().small());
                        }
                        if !e.responses.is_empty() {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Responses").strong());
                            egui::Grid::new("openapi_responses").striped(true).num_columns(3).show(ui, |ui| {
                                for (code, desc, ty) in &e.responses {
                                    let color = if code.starts_with('2') {
                                        method_color("GET")
                                    } else if code.starts_with('4') || code.starts_with('5') {
                                        method_color("DELETE")
                                    } else {
                                        pal.text
                                    };
                                    ui.label(RichText::new(code).monospace().color(color));
                                    ui.label(RichText::new(ty).monospace().small());
                                    ui.label(dim(desc));
                                    ui.end_row();
                                }
                            });
                        }
                    });
                });
            });
        }
        Tab::Models => {
            egui::ScrollArea::vertical().id_salt("openapi_models").auto_shrink([false, false]).show(ui, |ui| {
                for m in spec.models.iter().filter(|m| needle.is_empty() || m.name.to_lowercase().contains(&needle)) {
                    egui::CollapsingHeader::new(RichText::new(&m.name).strong().monospace())
                        .id_salt(("model", &m.name))
                        .show(ui, |ui| {
                            if !m.description.is_empty() {
                                ui.label(dim(&m.description));
                            }
                            egui::Grid::new(("model_fields", &m.name)).striped(true).num_columns(3).show(ui, |ui| {
                                for (name, ty, required, desc) in &m.fields {
                                    ui.label(
                                        RichText::new(format!("{name}{}", if *required { " *" } else { "" }))
                                            .monospace(),
                                    );
                                    ui.label(RichText::new(ty).monospace().small());
                                    ui.label(dim(desc));
                                    ui.end_row();
                                }
                            });
                        });
                }
            });
        }
    });
    ask
}
