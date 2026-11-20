//! Jupyter notebooks: an `.ipynb` file opens as its cells. Markdown cells are drawn as the
//! Markdown preview draws them (double-click to edit); code cells run in a Python kernel
//! (through `jupyter_client`), their output, values, errors and plots shown below them.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::data::notebook::{Cell, CellKind, Notebook, Output};
use coiman_core::launch::live::{Eval, LiveEvent, Session};
use coiman_core::launch::live_adapters::{self, ConnectOptions, LiveKind};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

type Connecting = Receiver<Result<Box<dyn Session>, String>>;

#[derive(Default)]
pub(crate) struct NotebookView {
    path: Option<PathBuf>,
    nb: Option<Notebook>,
    dirty: bool,
    /// The markdown cell being edited, if any.
    editing: Option<usize>,
    session: Option<Box<dyn Session>>,
    connecting: Option<Connecting>,
    kernel: String,
    /// Cells waiting to run, in order, and the one running (request id, cell).
    queue: VecDeque<usize>,
    running: Option<(u64, usize)>,
    counter: u64,
    textures: HashMap<u64, egui::TextureHandle>,
    /// A path opened as JSON on purpose (not as a notebook).
    pub(crate) as_text: Option<PathBuf>,
    /// Closing with unsaved changes: asking whether to save.
    asking_close: bool,
}

enum Action {
    Run(usize),
    RunAll,
    Interrupt,
    Restart,
    Add(usize, CellKind),
    Delete(usize),
    Move(usize, bool),
    Kind(usize, CellKind),
    Edit(Option<usize>),
    Save,
    AsJson,
    Link(String),
}

fn red() -> Color32 {
    Color32::from_rgb(0xE5, 0x53, 0x4B)
}

fn key(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// A run's output, appended to a cell's: printed text joins the text before it.
fn push_output(outputs: &mut Vec<Output>, out: Output) {
    if let (Some(Output::Stream { text, error }), Output::Stream { text: more, error: e2 }) = (outputs.last_mut(), &out)
        && error == e2
    {
        text.push_str(more);
        return;
    }
    outputs.push(out);
}

impl CodeApp {
    /// Opens an `.ipynb` as a notebook. False when it isn't one Coiman can read.
    pub(crate) fn open_notebook(&mut self, path: &Path) -> bool {
        if self.notebook.as_text.as_deref() == Some(path) {
            self.notebook.as_text = None;
            return false;
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let nb = if text.trim().is_empty() { Ok(Notebook::empty()) } else { Notebook::parse(&text) };
        match nb {
            Ok(nb) => {
                if self.notebook.path.as_deref() != Some(path) {
                    self.close_notebook_kernel();
                    self.notebook.counter = 0;
                }
                self.notebook.path = Some(path.to_path_buf());
                self.notebook.nb = Some(nb);
                self.notebook.dirty = false;
                self.notebook.editing = None;
                true
            }
            Err(e) => {
                self.status = Some(format!("{}: {e}; opened as text", path.display()));
                false
            }
        }
    }

    fn close_notebook_kernel(&mut self) {
        self.notebook.session = None;
        self.notebook.connecting = None;
        self.notebook.queue.clear();
        self.notebook.running = None;
        self.notebook.kernel.clear();
    }

    fn notebook_connect(&mut self, ctx: &egui::Context) {
        if self.notebook.session.is_some() || self.notebook.connecting.is_some() {
            return;
        }
        let cwd = self.notebook.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        if let Some(dir) = &cwd
            && !self.allowed_to_run(dir)
        {
            self.notebook.queue.clear();
            return;
        }
        let options = ConnectOptions {
            python: self.python_env().map(|e| e.python.clone()),
            cwd,
            env: self.environment_variables(),
        };
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let wake2 = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(live_adapters::connect(LiveKind::PythonKernel, "", &options, wake));
            wake2();
        });
        self.notebook.connecting = Some(rx);
        self.notebook.kernel = "Starting the kernel…".to_owned();
    }

    /// Takes in the kernel's news, and starts the next cell when it's free.
    fn notebook_pump(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.notebook.connecting
            && let Ok(result) = rx.try_recv()
        {
            self.notebook.connecting = None;
            match result {
                Ok(s) => self.notebook.session = Some(s),
                Err(e) => {
                    self.notebook.kernel = format!("No kernel: {e}");
                    self.notebook.queue.clear();
                }
            }
        }
        let events = self.notebook.session.as_mut().map(|s| s.poll()).unwrap_or_default();
        for ev in events {
            let cell = self.notebook.running.map(|r| r.1);
            let nb = self.notebook.nb.as_mut();
            let out = |nb: Option<&mut Notebook>, o: Output| {
                if let (Some(nb), Some(i)) = (nb, cell)
                    && let Some(c) = nb.cells.get_mut(i)
                {
                    push_output(&mut c.outputs, o);
                }
            };
            match ev {
                LiveEvent::Connected(id) => self.notebook.kernel = id.system,
                LiveEvent::Output { text, error, .. } => out(nb, Output::Stream { text, error }),
                LiveEvent::Value { text, .. } => out(nb, Output::Value(text)),
                LiveEvent::Image { png, .. } => out(nb, Output::Png(png)),
                LiveEvent::Failed { text, .. } => out(nb, Output::Error(text)),
                LiveEvent::NeedInput { .. } => {
                    out(nb, Output::Error("input() isn't supported in notebooks here".to_owned()));
                    if let Some(s) = &mut self.notebook.session {
                        s.stdin("");
                    }
                }
                LiveEvent::Done { id, .. } => {
                    if self.notebook.running.is_some_and(|r| r.0 == id) {
                        self.notebook.counter += 1;
                        let n = self.notebook.counter;
                        if let (Some(nb), Some(i)) = (self.notebook.nb.as_mut(), cell)
                            && let Some(c) = nb.cells.get_mut(i)
                        {
                            c.count = Some(n);
                        }
                        self.notebook.running = None;
                        self.notebook.dirty = true;
                    }
                }
                LiveEvent::Closed(why) => {
                    self.notebook.kernel = format!("The kernel stopped: {why}");
                    self.notebook.session = None;
                    self.notebook.running = None;
                    self.notebook.queue.clear();
                }
                LiveEvent::Rows { .. } => {}
            }
        }
        if self.notebook.running.is_none()
            && let Some(i) = self.notebook.queue.pop_front()
        {
            if self.notebook.session.is_none() {
                self.notebook.queue.push_front(i);
                self.notebook_connect(ctx);
                return;
            }
            let Some(c) = self.notebook.nb.as_mut().and_then(|nb| nb.cells.get_mut(i)) else { return };
            c.outputs.clear();
            c.count = None;
            let code = c.source.clone();
            let file = self.notebook.path.clone();
            if let Some(s) = &mut self.notebook.session {
                let id = s.eval(Eval { code, file, line: None, namespace: None });
                self.notebook.running = Some((id, i));
            }
        }
    }

    fn save_notebook(&mut self) {
        let (Some(path), Some(nb)) = (&self.notebook.path, &self.notebook.nb) else { return };
        match std::fs::write(path, nb.to_json()) {
            Ok(()) => {
                self.notebook.dirty = false;
                self.status = Some(format!("Saved {}", path.display()));
            }
            Err(e) => self.status = Some(format!("Couldn't save: {e}")),
        }
    }

    /// The notebook's window.
    pub(crate) fn notebook_window(&mut self, ctx: &egui::Context) {
        if self.notebook.nb.is_none() {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.85).min(1300.0), (room.y * 0.85).max(400.0));
        let name = self
            .notebook
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        egui::Window::new(RichText::new(format!("Notebook: {name}")).size(16.0).strong())
            .id(egui::Id::unique("notebook_window"))
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
            .show(ctx, |ui| self.notebook_ui(ui));
        if !open {
            if self.notebook.dirty {
                self.notebook.asking_close = true;
            } else {
                self.close_notebook();
            }
        }
    }

    fn notebook_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.notebook_pump(&ctx);
        if self.notebook.running.is_some() || self.notebook.connecting.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let pal = theme::colors();
        let mut action = None;
        let name = self
            .notebook
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let busy = self.notebook.running.is_some() || !self.notebook.queue.is_empty();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{name}{}", if self.notebook.dirty { " •" } else { "" })).strong());
            if ui.button("Run All").clicked() {
                action = Some(Action::RunAll);
            }
            if busy && ui.button("Interrupt").clicked() {
                action = Some(Action::Interrupt);
            }
            if ui.button("Restart").on_hover_text("A fresh kernel: every variable forgotten").clicked() {
                action = Some(Action::Restart);
            }
            if ui.button("Save").on_hover_text("Ctrl+S").clicked()
                || ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S))
            {
                action = Some(Action::Save);
            }
            if ui.button("Open as JSON").clicked() {
                action = Some(Action::AsJson);
            }
            let kernel = if self.notebook.kernel.is_empty() {
                "No kernel yet: it starts on the first run"
            } else {
                &self.notebook.kernel
            };
            ui.label(RichText::new(kernel).small().color(pal.text_dim));
        });
        if self.notebook.asking_close {
            let mut answer = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Close with unsaved changes?").color(pal.modified));
                if ui.button("Save and close").clicked() {
                    answer = Some(true);
                }
                if ui.button("Close without saving").clicked() {
                    answer = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    self.notebook.asking_close = false;
                }
            });
            if let Some(save) = answer {
                if save {
                    self.save_notebook();
                }
                self.close_notebook();
                return;
            }
        }
        ui.separator();
        let dir = self.notebook.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let root = self.explorer.root.clone();
        let size = self.settings.workbench.ui_font_size + 1.0;
        let code_font = egui::FontId::monospace(self.font_size);
        let running = self.notebook.running.map(|r| r.1);
        let queued: Vec<usize> = self.notebook.queue.iter().copied().collect();
        let editing = self.notebook.editing;
        let Some(nb) = self.notebook.nb.as_mut() else { return };
        let textures = &mut self.notebook.textures;
        let preview = &mut self.md_preview;
        let mut changed = false;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let count = nb.cells.len();
            for (i, cell) in nb.cells.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0, if running == Some(i) { pal.accent } else { pal.border }))
                        .corner_radius(6.0)
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let mark = match (running == Some(i), queued.contains(&i), cell.count) {
                                    (true, ..) => "[*]".to_owned(),
                                    (_, true, _) => "[ ]".to_owned(),
                                    (_, _, Some(n)) => format!("[{n}]"),
                                    _ => String::new(),
                                };
                                if cell.kind == CellKind::Code {
                                    if ui.button("▶").on_hover_text("Run (Shift+Enter in the cell)").clicked() {
                                        action = Some(Action::Run(i));
                                    }
                                    ui.label(RichText::new(mark).monospace().color(pal.text_dim));
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("✕").on_hover_text("Delete the cell").clicked() {
                                        action = Some(Action::Delete(i));
                                    }
                                    if i + 1 < count && ui.small_button("↓").clicked() {
                                        action = Some(Action::Move(i, false));
                                    }
                                    if i > 0 && ui.small_button("↑").clicked() {
                                        action = Some(Action::Move(i, true));
                                    }
                                    let other =
                                        if cell.kind == CellKind::Code { CellKind::Markdown } else { CellKind::Code };
                                    let label = if other == CellKind::Code { "To code" } else { "To Markdown" };
                                    if ui.small_button(label).clicked() {
                                        action = Some(Action::Kind(i, other));
                                    }
                                });
                            });
                            match cell.kind {
                                CellKind::Markdown if editing != Some(i) => {
                                    let r = ui.scope(|ui| {
                                        if cell.source.trim().is_empty() {
                                            ui.label(
                                                RichText::new("Empty Markdown cell: double-click to write")
                                                    .color(pal.text_dim),
                                            );
                                        } else if let Some(link) = crate::app::views::md_preview::draw_markdown(
                                            ui,
                                            preview,
                                            &cell.source,
                                            dir.as_deref(),
                                            root.as_deref(),
                                            size,
                                            i,
                                        ) {
                                            action = Some(Action::Link(link));
                                        }
                                    });
                                    if r.response.interact(egui::Sense::click()).double_clicked() {
                                        action = Some(Action::Edit(Some(i)));
                                    }
                                }
                                _ => {
                                    let edit = egui::TextEdit::multiline(&mut cell.source)
                                        .font(code_font.clone())
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(1)
                                        .lock_focus(true);
                                    let r = ui.add(edit);
                                    changed |= r.changed();
                                    if r.has_focus()
                                        && ui.input(|i| i.modifiers.shift && i.key_pressed(egui::Key::Enter))
                                    {
                                        // Shift+Enter typed a line break: take it back and run.
                                        if cell.source.ends_with('\n') {
                                            cell.source.pop();
                                        }
                                        action = Some(if cell.kind == CellKind::Code {
                                            Action::Run(i)
                                        } else {
                                            Action::Edit(None)
                                        });
                                    }
                                    if cell.kind == CellKind::Markdown && r.lost_focus() {
                                        action = Some(Action::Edit(None));
                                    }
                                }
                            }
                            for o in &cell.outputs {
                                match o {
                                    Output::Stream { text, error } => {
                                        let t = RichText::new(text.trim_end())
                                            .font(code_font.clone())
                                            .color(if *error { red() } else { pal.text });
                                        ui.add(egui::Label::new(t).wrap());
                                    }
                                    Output::Value(text) => {
                                        ui.add(egui::Label::new(RichText::new(text).font(code_font.clone())).wrap());
                                    }
                                    Output::Error(text) => {
                                        ui.add(
                                            egui::Label::new(RichText::new(text).font(code_font.clone()).color(red()))
                                                .wrap(),
                                        );
                                    }
                                    Output::Png(bytes) => {
                                        let k = key(bytes);
                                        if !textures.contains_key(&k)
                                            && let Ok(img) =
                                                image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                                        {
                                            let rgba = img.to_rgba8();
                                            let size = [rgba.width() as usize, rgba.height() as usize];
                                            let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                                            textures.insert(
                                                k,
                                                ui.ctx().load_texture(
                                                    format!("nb{k}"),
                                                    image,
                                                    egui::TextureOptions::LINEAR,
                                                ),
                                            );
                                        }
                                        if let Some(t) = textures.get(&k) {
                                            let s = t.size_vec2() / ui.ctx().pixels_per_point();
                                            let w = s.x.min(ui.available_width());
                                            ui.add(egui::Image::new((t.id(), egui::vec2(w, s.y * w / s.x.max(1.0)))));
                                        }
                                    }
                                }
                            }
                        });
                    ui.horizontal(|ui| {
                        if ui.small_button("+ Code").clicked() {
                            action = Some(Action::Add(i + 1, CellKind::Code));
                        }
                        if ui.small_button("+ Markdown").clicked() {
                            action = Some(Action::Add(i + 1, CellKind::Markdown));
                        }
                    });
                });
            }
            if nb.cells.is_empty() && ui.button("+ Code").clicked() {
                action = Some(Action::Add(0, CellKind::Code));
            }
        });
        if changed {
            self.notebook.dirty = true;
        }
        let Some(action) = action else { return };
        let nb = self.notebook.nb.as_mut().expect("open");
        match action {
            Action::Run(i) => {
                if !self.notebook.queue.contains(&i) && self.notebook.running.map(|r| r.1) != Some(i) {
                    self.notebook.queue.push_back(i);
                }
            }
            Action::RunAll => {
                self.notebook.queue =
                    nb.cells.iter().enumerate().filter(|(_, c)| c.kind == CellKind::Code).map(|(i, _)| i).collect();
            }
            Action::Interrupt => {
                self.notebook.queue.clear();
                if let Some(s) = &mut self.notebook.session {
                    s.interrupt();
                }
            }
            Action::Restart => {
                self.close_notebook_kernel();
                self.notebook.counter = 0;
                self.status = Some("The kernel is restarted on the next run".to_owned());
            }
            Action::Add(at, kind) => {
                nb.cells.insert(at.min(nb.cells.len()), Cell::new(kind));
                self.notebook.dirty = true;
                if kind == CellKind::Markdown {
                    self.notebook.editing = Some(at);
                }
            }
            Action::Delete(i) => {
                if !busy {
                    nb.cells.remove(i);
                    self.notebook.dirty = true;
                } else {
                    self.status = Some("Wait for the cells running, or interrupt them".to_owned());
                }
            }
            Action::Move(i, up) => {
                if !busy {
                    let j = if up { i - 1 } else { i + 1 };
                    nb.cells.swap(i, j);
                    self.notebook.dirty = true;
                }
            }
            Action::Kind(i, kind) => {
                nb.cells[i].kind = kind;
                nb.cells[i].outputs.clear();
                nb.cells[i].count = None;
                self.notebook.dirty = true;
            }
            Action::Edit(e) => self.notebook.editing = e,
            Action::Save => self.save_notebook(),
            Action::AsJson => {
                if let Some(path) = self.notebook.path.clone() {
                    if self.notebook.dirty {
                        self.save_notebook();
                    }
                    self.close_notebook();
                    self.notebook.as_text = Some(path.clone());
                    self.open_file(&path);
                }
            }
            Action::Link(link) => {
                let root = self.explorer.root.clone();
                self.follow_markdown_link(&ctx, &link, dir.as_deref(), root.as_deref());
            }
        }
    }

    /// Leaves the notebook (unsaved changes are asked about first by the caller).
    pub(crate) fn close_notebook(&mut self) {
        self.close_notebook_kernel();
        self.notebook.nb = None;
        self.notebook.path = None;
        self.notebook.asking_close = false;
        self.notebook.textures.clear();
    }
}
