//! The Architecture window: the project's dependency graph, by file or by folder, with its
//! import cycles, the files most used and those used by nothing, and what a change to a file
//! reaches, tests included. The graph is read in the background and drawn here; a click picks
//! a file and lights what it uses and what uses it, a double click opens it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use coiman_core::analysis::arch::{self, Coupling, Impact, Layout};
use coiman_core::analysis::deps::{self, Graph};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::CodeApp;
use crate::ui::theme;

const NODE: Vec2 = egui::vec2(170.0, 30.0);
const GAP: Vec2 = egui::vec2(28.0, 64.0);
const LIST_WIDTH: f32 = 280.0;
const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const GREEN: Color32 = Color32::from_rgb(0x57, 0xC0, 0x6A);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Cycles,
    MostUsed,
    Isolated,
    Impact,
}

/// One view of the graph (files or folders) with everything worked out from it.
struct View {
    graph: Graph,
    layout: Layout,
    cycles: Vec<Vec<usize>>,
    cycle_edges: HashSet<(usize, usize)>,
    coupling: Vec<Coupling>,
}

impl View {
    fn of(graph: Graph) -> Self {
        let layout = arch::layout(&graph);
        let cycles = arch::cycles(&graph);
        let cycle_edges = arch::cycle_edges(&graph);
        let coupling = arch::coupling(&graph);
        Self { graph, layout, cycles, cycle_edges, coupling }
    }

    fn in_cycle(&self, node: usize) -> bool {
        self.cycles.iter().any(|c| c.contains(&node))
    }
}

struct Analysis {
    files: View,
    folders: View,
    seconds: f32,
}

#[derive(Default)]
pub(crate) struct Architecture {
    pub(crate) open: bool,
    by_folder: bool,
    tab: Tab,
    analysis: Option<Analysis>,
    loading: Option<Receiver<Analysis>>,
    /// The picked node in the current view, and the file whose impact is shown.
    selected: Option<usize>,
    impact_of: Option<usize>,
    pan: Vec2,
    zoom: f32,
    /// Center the canvas on the picked node at the next frame.
    center: bool,
}

impl CodeApp {
    /// Opens the window, reading the project if it hasn't been read yet.
    pub(crate) fn show_architecture(&mut self, ctx: &egui::Context) {
        self.architecture.open = true;
        if self.architecture.analysis.is_none() && self.architecture.loading.is_none() {
            self.read_architecture(ctx);
        }
    }

    /// Opens the window on what a change to the file in front reaches.
    pub(crate) fn show_impact_of_current_file(&mut self, ctx: &egui::Context) {
        self.show_architecture(ctx);
        self.architecture.tab = Tab::Impact;
        self.architecture.by_folder = false;
        self.pick_current_file();
    }

    fn pick_current_file(&mut self) {
        let (Some(root), Some(path)) =
            (self.explorer.root.clone(), self.docs.get(self.active).and_then(|d| d.path.clone()))
        else {
            return;
        };
        let Some(analysis) = &self.architecture.analysis else { return };
        let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        if let Some(i) = analysis.files.graph.index_of(&rel) {
            self.architecture.selected = Some(i);
            self.architecture.impact_of = Some(i);
            self.architecture.center = true;
        }
    }

    fn read_architecture(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else {
            self.status = Some("Open a folder to see its architecture".to_owned());
            return;
        };
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let graph = deps::scan(&root);
            let folders = View::of(arch::by_folder(&graph));
            let files = View::of(graph);
            let _ = tx.send(Analysis { files, folders, seconds: started.elapsed().as_secs_f32() });
            wake.request_repaint();
        });
        self.architecture.loading = Some(rx);
    }

    pub(crate) fn architecture_window(&mut self, ctx: &egui::Context) {
        if !self.architecture.open {
            return;
        }
        let impact_wanted = self.architecture.tab == Tab::Impact && self.architecture.impact_of.is_none();
        if let Some(done) = self.architecture.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.architecture.loading = None;
            self.architecture.analysis = Some(done);
            self.architecture.selected = None;
            self.architecture.zoom = 0.0;
            self.architecture.pan = Vec2::ZERO;
            if impact_wanted {
                self.pick_current_file();
            }
        }
        let mut open = true;
        let mut action = None;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.86).min(1280.0), (room.y * 0.82).max(420.0));
        egui::Window::new(RichText::new("Architecture").size(16.0).strong())
            .id(egui::Id::unique("architecture_window"))
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
            .show(ctx, |ui| action = self.architecture_ui(ui));
        self.architecture.open = open;
        match action {
            Some(Action::Refresh) => self.read_architecture(ctx),
            Some(Action::Open(path, line)) => {
                self.open_file(&path);
                self.jump_to_symbol_line(line.saturating_sub(1));
            }
            Some(Action::ExportSvg) => self.export_architecture_svg(),
            Some(Action::CopyMermaid) => {
                if let Some(view) = self.architecture_view() {
                    ctx.copy_text(arch::to_mermaid(&view.graph));
                    self.status = Some("Copied the graph as Mermaid".to_owned());
                }
            }
            None => {}
        }
    }

    fn architecture_view(&self) -> Option<&View> {
        let a = self.architecture.analysis.as_ref()?;
        Some(if self.architecture.by_folder { &a.folders } else { &a.files })
    }

    fn export_architecture_svg(&mut self) {
        let Some(view) = self.architecture_view() else { return };
        let svg = arch::to_svg(&view.graph, &view.layout);
        let name = if self.architecture.by_folder { "folders.svg" } else { "dependencies.svg" };
        let Some(path) =
            self.file_dialog("architecture_svg").set_file_name(name).add_filter("SVG", &["svg"]).save_file()
        else {
            return;
        };
        self.dialog_chose("architecture_svg", &path);
        self.status = Some(match std::fs::write(&path, svg) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Couldn't save {}: {e}", path.display()),
        });
    }

    fn architecture_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let pal = theme::colors();
        // The bar: files or folders, what was found, and what can be done with it.
        ui.horizontal(|ui| {
            for (folders, label) in [(false, "Files"), (true, "Folders")] {
                if pill(ui, label, self.architecture.by_folder == folders).clicked()
                    && self.architecture.by_folder != folders
                {
                    self.architecture.by_folder = folders;
                    self.architecture.selected = None;
                    self.architecture.pan = Vec2::ZERO;
                    self.architecture.zoom = 0.0;
                }
            }
            ui.add_space(10.0);
            if let Some(a) = &self.architecture.analysis {
                let view = if self.architecture.by_folder { &a.folders } else { &a.files };
                let mut text = format!(
                    "{} {} · {} imports · {} cycles · {:.1} s",
                    view.graph.nodes.len(),
                    if self.architecture.by_folder { "folders" } else { "files" },
                    view.graph.edges.len(),
                    view.cycles.len(),
                    a.seconds
                );
                if view.graph.truncated {
                    text.push_str(&format!(" · first {} files only", deps::MAX_FILES));
                }
                ui.label(RichText::new(text).color(pal.text_dim));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ready = self.architecture.analysis.is_some();
                if ui.add_enabled(ready, egui::Button::new("Copy Mermaid")).clicked() {
                    action = Some(Action::CopyMermaid);
                }
                if ui.add_enabled(ready, egui::Button::new("Export SVG...")).clicked() {
                    action = Some(Action::ExportSvg);
                }
                if ui.add_enabled(ready, egui::Button::new("Fit")).clicked() {
                    self.architecture.zoom = 0.0;
                }
                if ui.add_enabled(self.architecture.loading.is_none(), egui::Button::new("Refresh")).clicked() {
                    action = Some(Action::Refresh);
                }
            });
        });
        ui.add_space(6.0);
        if self.architecture.loading.is_some() && self.architecture.analysis.is_none() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the project's imports...");
            });
            return action;
        }
        // Out for the frame, so the lists and the canvas can change the rest of the state.
        let Some(analysis) = self.architecture.analysis.take() else {
            ui.label(RichText::new("Open a folder to see its architecture.").color(pal.text_dim));
            return action;
        };
        let by_folder = self.architecture.by_folder;
        let view = if by_folder { &analysis.folders } else { &analysis.files };
        let area = ui.available_rect_before_wrap();
        let list_rect = Rect::from_min_size(area.min, egui::vec2(LIST_WIDTH.min(area.width() * 0.4), area.height()));
        let canvas = Rect::from_min_max(egui::pos2(list_rect.right() + 12.0, area.top()), area.max);
        ui.allocate_rect(area, Sense::hover());
        let mut list_ui = ui.new_child(egui::UiBuilder::new().max_rect(list_rect));
        let picked = list(&mut list_ui, &mut self.architecture, view, &analysis.files, by_folder);
        let canvas_action = graph_canvas(ui, canvas, &mut self.architecture, view, by_folder);
        self.architecture.analysis = Some(analysis);
        if let Some(i) = picked {
            self.architecture.selected = Some(i);
            self.architecture.center = true;
        }
        action.or(canvas_action)
    }
}

enum Action {
    Refresh,
    Open(PathBuf, usize),
    ExportSvg,
    CopyMermaid,
}

fn pill(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let pal = theme::colors();
    let fill = if on { pal.accent.gamma_multiply(0.18) } else { Color32::TRANSPARENT };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, if on { pal.accent.gamma_multiply(0.6) } else { pal.border }))
        .corner_radius(11.0)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| ui.add(egui::Label::new(label).selectable(false).extend()))
        .response
        .interact(Sense::click())
}

/// The lists beside the graph. Returns a node picked in them.
fn list(ui: &mut egui::Ui, state: &mut Architecture, view: &View, files: &View, by_folder: bool) -> Option<usize> {
    let pal = theme::colors();
    let mut picked = None;
    let isolated = view.coupling.iter().filter(|c| c.fan_in == 0 && c.fan_out == 0).count();
    let tabs = [
        (Tab::Cycles, format!("Cycles {}", view.cycles.len())),
        (Tab::MostUsed, "Most used".to_owned()),
        (Tab::Isolated, format!("Unused {isolated}")),
        (Tab::Impact, "Impact".to_owned()),
    ];
    // Two rows of two: the column is narrow.
    for pair in tabs.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (tab, label) in pair {
                if pill(ui, label, state.tab == *tab).clicked() {
                    state.tab = *tab;
                    if *tab == Tab::Impact && !by_folder {
                        state.impact_of = state.selected.or(state.impact_of);
                    }
                }
            }
        });
        ui.add_space(4.0);
    }
    ui.add_space(6.0);
    let row = |ui: &mut egui::Ui, view: &View, i: usize, note: String, picked: &mut Option<usize>| {
        let node = &view.graph.nodes[i];
        // Folders by their whole path: two can share a name (`src`).
        let name = if by_folder { node.path.clone() } else { arch::label(&node.path) };
        let resp = ui
            .horizontal(|ui| {
                let color = if view.in_cycle(i) {
                    RED
                } else if node.test {
                    GREEN
                } else {
                    pal.text
                };
                ui.add(egui::Label::new(RichText::new(name).color(color)).truncate().selectable(false));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(note).small().color(pal.text_dim));
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_text(&node.path);
        if resp.hovered() {
            ui.painter().rect_filled(resp.rect, 4.0, pal.hover_bg.gamma_multiply(0.6));
        }
        if resp.clicked() {
            *picked = Some(i);
        }
    };
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_width(ui.available_width());
        match state.tab {
            Tab::Cycles => {
                if view.cycles.is_empty() {
                    ui.label(RichText::new("No import cycles.").color(pal.text_dim));
                }
                for (n, cycle) in view.cycles.iter().enumerate() {
                    ui.label(RichText::new(format!("Cycle {} · {} files", n + 1, cycle.len())).strong().color(RED));
                    for &i in cycle.iter().take(40) {
                        row(ui, view, i, String::new(), &mut picked);
                    }
                    if cycle.len() > 40 {
                        ui.label(RichText::new(format!("and {} more", cycle.len() - 40)).small().color(pal.text_dim));
                    }
                    ui.add_space(8.0);
                }
            }
            Tab::MostUsed => {
                let mut order: Vec<usize> = (0..view.graph.nodes.len()).collect();
                order.sort_by_key(|&i| std::cmp::Reverse(view.coupling[i].fan_in));
                ui.label(RichText::new("Used by · uses").small().color(pal.text_dim));
                for &i in order.iter().take(200).filter(|&&i| view.coupling[i].fan_in > 0) {
                    let c = view.coupling[i];
                    row(ui, view, i, format!("{} · {}", c.fan_in, c.fan_out), &mut picked);
                }
            }
            Tab::Isolated => {
                ui.label(
                    RichText::new(
                        "Nothing imports these and they import nothing. Entry points and scripts show here too.",
                    )
                    .small()
                    .color(pal.text_dim),
                );
                for i in (0..view.graph.nodes.len())
                    .filter(|&i| view.coupling[i].fan_in == 0 && view.coupling[i].fan_out == 0)
                {
                    row(ui, view, i, String::new(), &mut picked);
                }
            }
            Tab::Impact => {
                let Some(target) = state.impact_of.filter(|_| !by_folder) else {
                    ui.label(
                        RichText::new(
                            "Pick a file in the graph (in Files), or run Architecture: Impact of Current File.",
                        )
                        .color(pal.text_dim),
                    );
                    return;
                };
                let Impact { direct, indirect, tests } = arch::impact(&files.graph, target);
                ui.label(RichText::new(arch::label(&files.graph.nodes[target].path)).strong());
                ui.label(
                    RichText::new(format!(
                        "A change here reaches {} files: {} import it, {} through others. {} tests cover it.",
                        direct.len() + indirect.len(),
                        direct.len(),
                        indirect.len(),
                        tests.len()
                    ))
                    .small()
                    .color(pal.text_dim),
                );
                for (title, items) in [("Tests", &tests), ("Imported by", &direct), ("Through others", &indirect)] {
                    if items.is_empty() {
                        continue;
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{title} · {}", items.len())).strong().color(pal.accent));
                    for &i in items.iter().take(300) {
                        row(ui, files, i, String::new(), &mut picked);
                    }
                }
            }
        }
    });
    picked
}

/// The graph itself: drag to move, scroll to zoom, click to pick, double click to open.
fn graph_canvas(
    ui: &mut egui::Ui,
    rect: Rect,
    state: &mut Architecture,
    view: &View,
    by_folder: bool,
) -> Option<Action> {
    let pal = theme::colors();
    let mut action = None;
    let resp = ui.interact(rect, egui::Id::unique("architecture_canvas"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 12.0, pal.sidebar_bg);
    painter.rect_stroke(rect, 12.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    // No zoom yet: fit the whole graph, centred.
    if state.zoom <= 0.0 {
        let step = NODE + GAP;
        let content = egui::vec2(view.layout.width.max(1) as f32 * step.x, view.layout.layers.max(1) as f32 * step.y)
            + egui::vec2(32.0, 32.0);
        state.zoom = (rect.width() / content.x).min(rect.height() / content.y).clamp(0.08, 1.0);
        state.pan = (rect.size() - content * state.zoom) / 2.0;
    }
    if resp.dragged() {
        state.pan += resp.drag_delta();
    }
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let factor = zoom * (scroll.y / 400.0).exp();
        if (factor - 1.0).abs() > f32::EPSILON
            && let Some(at) = resp.hover_pos()
        {
            // Zoom about the pointer.
            let new = (state.zoom * factor).clamp(0.08, 3.0);
            let k = new / state.zoom;
            let origin = rect.min + state.pan;
            state.pan += (at - origin) * (1.0 - k);
            state.zoom = new;
        }
    }
    let z = state.zoom;
    let step = (NODE + GAP) * z;
    let place = |i: usize, pan: Vec2| -> Rect {
        let (x, y) = view.layout.pos[i];
        Rect::from_min_size(rect.min + pan + egui::vec2(16.0, 16.0) * z + egui::vec2(x * step.x, y * step.y), NODE * z)
    };
    if state.center
        && let Some(i) = state.selected.filter(|&i| i < view.graph.nodes.len())
    {
        let at = place(i, state.pan).center();
        state.pan += rect.center() - at;
        state.center = false;
    }
    let pan = state.pan;
    // What the picked node uses and what uses it.
    let (uses, used_by): (HashSet<usize>, HashSet<usize>) = match state.selected.filter(|&i| i < view.graph.nodes.len())
    {
        Some(i) => (
            arch::reach(&view.graph, i, true).into_iter().map(|(w, _)| w).collect(),
            arch::reach(&view.graph, i, false).into_iter().map(|(w, _)| w).collect(),
        ),
        None => (HashSet::new(), HashSet::new()),
    };
    let lit =
        |i: usize| state.selected.is_none() || state.selected == Some(i) || uses.contains(&i) || used_by.contains(&i);
    let amber = Color32::from_rgb(0xF2, 0xB1, 0x3C);
    let view_rect = rect.expand(40.0);
    // Edges under the nodes.
    for e in &view.graph.edges {
        let (a, b) = (place(e.from, pan), place(e.to, pan));
        if !view_rect.intersects(a.union(b)) {
            continue;
        }
        let (from, to) = if b.top() > a.bottom() {
            (a.center_bottom(), b.center_top())
        } else if b.bottom() < a.top() {
            (a.center_top(), b.center_bottom())
        } else if b.left() > a.right() {
            (a.right_center(), b.left_center())
        } else {
            (a.left_center(), b.right_center())
        };
        let cyclic = view.cycle_edges.contains(&(e.from, e.to));
        let on_path = state.selected.is_some()
            && ((state.selected == Some(e.from) || uses.contains(&e.from)) && uses.contains(&e.to)
                || (state.selected == Some(e.to) || used_by.contains(&e.to)) && used_by.contains(&e.from));
        let mut color = if cyclic { RED } else { pal.text_dim.gamma_multiply(0.55) };
        if state.selected.is_some() {
            color = if on_path {
                if used_by.contains(&e.from) { amber } else { pal.accent }
            } else {
                color.gamma_multiply(0.18)
            };
        }
        let width = if by_folder { (1.0 + (e.line as f32).ln()).min(4.0) } else { 1.2 };
        arrow(&painter, from, to, Stroke::new(width * z.sqrt(), color));
    }
    let mut hovered = None;
    let pointer = resp.hover_pos();
    let font = egui::FontId::proportional((12.5 * z).clamp(6.0, 20.0));
    for (i, node) in view.graph.nodes.iter().enumerate() {
        let r = place(i, pan);
        if !view_rect.intersects(r) {
            continue;
        }
        let selected = state.selected == Some(i);
        let dim: f32 = if lit(i) { 1.0 } else { 0.25 };
        let edge = if selected {
            pal.accent
        } else if uses.contains(&i) {
            pal.accent.gamma_multiply(0.8)
        } else if used_by.contains(&i) {
            amber
        } else if view.in_cycle(i) {
            RED
        } else if node.test {
            GREEN
        } else {
            pal.border
        };
        let fill = if selected { pal.accent.gamma_multiply(0.25) } else { pal.editor_bg };
        painter.rect_filled(r, 8.0 * z, fill.gamma_multiply(dim.max(0.6)));
        painter.rect_stroke(r, 8.0 * z, Stroke::new(1.2, edge.gamma_multiply(dim)), egui::StrokeKind::Inside);
        if z > 0.35 {
            let name = arch::label(&node.path);
            let clip = r.shrink(6.0 * z);
            painter.with_clip_rect(clip.intersect(rect)).text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                if by_folder { node.path.as_str() } else { name.as_str() },
                font.clone(),
                pal.text.gamma_multiply(dim),
            );
        }
        if pointer.is_some_and(|p| r.contains(p)) {
            hovered = Some(i);
        }
    }
    if let Some(i) = hovered {
        let c = view.coupling[i];
        let tip = format!("{}\nUsed by {} · uses {}", view.graph.nodes[i].path, c.fan_in, c.fan_out);
        resp.clone().on_hover_text_at_pointer(tip);
    }
    if resp.clicked() {
        state.selected = hovered;
        if !by_folder && hovered.is_some() {
            state.impact_of = hovered;
        }
    }
    if resp.double_clicked()
        && !by_folder
        && let Some(i) = hovered
    {
        action = Some(Action::Open(view.graph.full_path(i), 1));
    }
    // A legend in the corner.
    let legend = [(pal.accent, "uses"), (amber, "used by"), (RED, "cycle"), (GREEN, "test")];
    let mut at = egui::pos2(rect.left() + 12.0, rect.bottom() - 14.0);
    for (color, label) in legend {
        painter.circle_filled(at, 4.0, color);
        let g = painter.text(
            at + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(11.0),
            pal.text_dim,
        );
        at.x = g.right() + 14.0;
    }
    painter.text(
        rect.right_bottom() - egui::vec2(12.0, 14.0),
        egui::Align2::RIGHT_CENTER,
        "Drag to move · scroll to zoom · double click to open",
        egui::FontId::proportional(11.0),
        pal.text_dim,
    );
    action
}

fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke) {
    painter.line_segment([from, to], stroke);
    let dir = (to - from).normalized();
    if !dir.x.is_finite() {
        return;
    }
    let size = 4.0 + stroke.width * 2.0;
    let side = egui::vec2(-dir.y, dir.x) * size * 0.5;
    let back = to - dir * size;
    painter.add(egui::Shape::convex_polygon(vec![to, back + side, back - side], stroke.color, Stroke::NONE));
}
