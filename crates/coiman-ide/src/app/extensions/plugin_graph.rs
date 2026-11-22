//! A plugin's graph (`show-graph`) in a window: UML boxes laid out with what is inherited
//! above what inherits it. Drag to move, scroll to zoom, a click picks a box and lights what
//! it is related to, a double click opens its file; a filter, members on or off, and export
//! to SVG, Mermaid and PlantUML.

use std::collections::{HashMap, HashSet};

use coiman_core::extensions::plugin::uml::{self, Placed};
use coiman_core::extensions::plugin::{EdgeKind, Graph};
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::CodeApp;
use crate::ui::theme;

/// The graph on screen.
pub(super) struct Shown {
    plugin: String,
    graph: Graph,
    members: bool,
    placed: Placed,
    /// 0 until fitted to the window.
    zoom: f32,
    pan: Vec2,
    selected: Option<usize>,
    center: bool,
    filter: String,
    open: bool,
}

enum Action {
    Open(String, u32),
    Svg,
    Mermaid,
    PlantUml,
    Preview,
}

impl Shown {
    fn new(plugin: String, graph: Graph) -> Self {
        let placed = uml::layout(&graph, true);
        let selected = graph.focus.as_ref().and_then(|f| graph.nodes.iter().position(|n| &n.id == f));
        Self {
            plugin,
            graph,
            members: true,
            placed,
            zoom: 0.0,
            pan: Vec2::ZERO,
            selected,
            center: selected.is_some(),
            filter: String::new(),
            open: true,
        }
    }

    /// Nodes related to `i` by any edge.
    fn related(&self, i: usize, index: &HashMap<&str, usize>) -> HashSet<usize> {
        let id = self.graph.nodes[i].id.as_str();
        let mut out = HashSet::new();
        for e in &self.graph.edges {
            if e.source == id {
                out.extend(index.get(e.target.as_str()));
            } else if e.target == id {
                out.extend(index.get(e.source.as_str()));
            }
        }
        out
    }
}

impl CodeApp {
    /// A plugin shows, replaces or (with an empty title) closes its graph.
    pub(super) fn show_plugin_graph(&mut self, plugin: String, graph: Graph) {
        if graph.title.is_empty() {
            if self.plugins.graph.as_ref().is_some_and(|g| g.plugin == plugin) {
                self.plugins.graph = None;
            }
            return;
        }
        self.plugins.graph = Some(Shown::new(plugin, graph));
    }

    pub(crate) fn plugin_graph_window(&mut self, ctx: &egui::Context) {
        let Some(mut shown) = self.plugins.graph.take() else { return };
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.86).min(1400.0), (room.y * 0.84).max(420.0));
        let mut open = shown.open;
        let mut action = None;
        egui::Window::new(RichText::new(&shown.graph.title).size(16.0).strong())
            .id(egui::Id::unique("plugin_graph_window"))
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
            .show(ctx, |ui| action = graph_ui(ui, &mut shown));
        shown.open = open;
        match action {
            Some(Action::Open(path, line)) => {
                if let Some(root) = self.explorer.root.clone() {
                    self.open_file(&root.join(&path));
                    self.jump_to_symbol_line(line.saturating_sub(1) as usize);
                }
            }
            Some(Action::Svg) => {
                let svg = uml::to_svg(&shown.graph, &shown.placed, shown.members);
                let name = "classes.svg";
                if let Some(path) =
                    self.file_dialog("plugin_graph_svg").set_file_name(name).add_filter("SVG", &["svg"]).save_file()
                {
                    self.dialog_chose("plugin_graph_svg", &path);
                    self.status = Some(match std::fs::write(&path, svg) {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Couldn't save {}: {e}", path.display()),
                    });
                }
            }
            Some(Action::Mermaid) => {
                ctx.copy_text(uml::to_mermaid(&shown.graph, shown.members));
                self.status = Some("Copied the diagram as Mermaid".to_owned());
            }
            Some(Action::PlantUml) => {
                ctx.copy_text(uml::to_plantuml(&shown.graph, shown.members));
                self.status = Some("Copied the diagram as PlantUML".to_owned());
            }
            Some(Action::Preview) => {
                let text = uml::to_mermaid(&shown.graph, shown.members);
                self.preview_mermaid(&text);
            }
            None => {}
        }
        if shown.open {
            self.plugins.graph = Some(shown);
        }
    }
}

fn graph_ui(ui: &mut egui::Ui, shown: &mut Shown) -> Option<Action> {
    let pal = theme::colors();
    let mut action = None;
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut shown.filter).hint_text("Find by name").desired_width(200.0));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let f = shown.filter.to_lowercase();
            if let Some(i) = shown.graph.nodes.iter().position(|n| n.label.to_lowercase().contains(&f)) {
                shown.selected = Some(i);
                shown.center = true;
            }
        }
        // Only boxes with something inside them have members to show or hide.
        if shown.graph.nodes.iter().any(|n| !n.members.is_empty())
            && ui.checkbox(&mut shown.members, "Members").changed()
        {
            shown.placed = uml::layout(&shown.graph, shown.members);
        }
        if ui.button("Fit").clicked() {
            shown.zoom = 0.0;
        }
        ui.separator();
        if ui.button("Export SVG...").clicked() {
            action = Some(Action::Svg);
        }
        if ui.button("Copy Mermaid").clicked() {
            action = Some(Action::Mermaid);
        }
        if ui.button("Copy PlantUML").clicked() {
            action = Some(Action::PlantUml);
        }
        if ui.button("Preview Mermaid").clicked() {
            action = Some(Action::Preview);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (n, e) = (shown.graph.nodes.len(), shown.graph.edges.len());
            ui.label(RichText::new(format!("{n} shown · {e} links")).color(pal.text_dim));
        });
    });
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    if shown.graph.nodes.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Nothing to show",
            egui::FontId::proportional(14.0),
            pal.text_dim,
        );
        return action;
    }
    canvas(ui, rect, shown).or(action)
}

const AMBER: Color32 = Color32::from_rgb(0xF2, 0xB1, 0x3C);

fn canvas(ui: &mut egui::Ui, rect: Rect, shown: &mut Shown) -> Option<Action> {
    let pal = theme::colors();
    let mut action = None;
    let resp = ui.interact(rect, egui::Id::unique("plugin_graph_canvas"), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 12.0, pal.sidebar_bg);
    painter.rect_stroke(rect, 12.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let content = egui::vec2(shown.placed.size.0, shown.placed.size.1) + egui::vec2(48.0, 48.0);
    if shown.zoom <= 0.0 {
        shown.zoom = (rect.width() / content.x).min(rect.height() / content.y).clamp(0.05, 1.0);
        shown.pan = (rect.size() - content * shown.zoom) / 2.0;
        // All of it fits readably: the picked box needn't be brought to the middle.
        if shown.zoom >= 0.6 {
            shown.center = false;
        }
    }
    if resp.dragged() {
        shown.pan += resp.drag_delta();
    }
    if resp.hovered() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
        let factor = zoom * (scroll.y / 400.0).exp();
        if (factor - 1.0).abs() > f32::EPSILON
            && let Some(at) = resp.hover_pos()
        {
            let new = (shown.zoom * factor).clamp(0.05, 3.0);
            let k = new / shown.zoom;
            let origin = rect.min + shown.pan;
            shown.pan += (at - origin) * (1.0 - k);
            shown.zoom = new;
        }
    }
    let z = shown.zoom;
    let place = |i: usize, pan: Vec2| -> Rect {
        let [x, y, w, h] = shown.placed.rects[i];
        Rect::from_min_size(rect.min + pan + egui::vec2(24.0 + x, 24.0 + y) * z, egui::vec2(w, h) * z)
    };
    if shown.center
        && let Some(i) = shown.selected
    {
        if z < 0.6 {
            // Close enough to read.
            let at = place(i, shown.pan).center() - rect.min;
            shown.zoom = 0.8;
            shown.pan = (shown.pan - at) * (0.8 / z) + at;
        }
        let at = place(i, shown.pan).center();
        shown.pan += rect.center() - at;
        shown.center = false;
    }
    let z = shown.zoom;
    let place = |i: usize, pan: Vec2| -> Rect {
        let [x, y, w, h] = shown.placed.rects[i];
        Rect::from_min_size(rect.min + pan + egui::vec2(24.0 + x, 24.0 + y) * z, egui::vec2(w, h) * z)
    };
    let pan = shown.pan;
    let index: HashMap<&str, usize> = shown.graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let related = shown.selected.map(|i| shown.related(i, &index)).unwrap_or_default();
    let filter = shown.filter.to_lowercase();
    let matches = |i: usize| filter.is_empty() || shown.graph.nodes[i].label.to_lowercase().contains(&filter);
    let lit = |i: usize| {
        let picked = shown.selected.is_none_or(|s| s == i || related.contains(&i));
        picked && matches(i)
    };
    let view = rect.expand(60.0);
    let line = pal.text_dim.gamma_multiply(0.8);
    for e in &shown.graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        let (ra, rb) = (place(a, pan), place(b, pan));
        if !view.intersects(ra.union(rb)) {
            continue;
        }
        let to_f = |r: Rect| [r.min.x, r.min.y, r.width(), r.height()];
        let pts: Vec<Pos2> = uml::route(to_f(ra), to_f(rb)).into_iter().map(|(x, y)| Pos2::new(x, y)).collect();
        let last = pts.len() - 1;
        // The ends, and the points next to them, for the arrowheads.
        let (p, p1, q0, q) = (pts[0], pts[1], pts[last - 1], pts[last]);
        let on = shown.selected.is_some_and(|s| s == a || s == b);
        let own = e.accent.map(rgb).unwrap_or(line);
        let color = if on {
            pal.accent
        } else if shown.selected.is_some() || !filter.is_empty() {
            own.gamma_multiply(0.25)
        } else {
            own
        };
        let stroke = Stroke::new((1.2 * z.sqrt()).max(0.8), color);
        let size = (12.0 * z).clamp(5.0, 16.0);
        match e.kind {
            EdgeKind::Inherits => {
                painter.add(egui::Shape::line(pts.clone(), stroke));
                triangle(&painter, q0, q, size, stroke, pal.sidebar_bg);
            }
            EdgeKind::Implements => {
                painter.extend(egui::Shape::dashed_line(&pts, stroke, 6.0 * z.max(0.4), 4.0 * z.max(0.4)));
                triangle(&painter, q0, q, size, stroke, pal.sidebar_bg);
            }
            EdgeKind::Has => {
                painter.add(egui::Shape::line(pts.clone(), stroke));
                diamond(&painter, p1, p, size, stroke.color);
            }
            EdgeKind::Uses => {
                painter.extend(egui::Shape::dashed_line(&pts, stroke, 6.0 * z.max(0.4), 4.0 * z.max(0.4)));
                open_arrow(&painter, q0, q, size, stroke);
            }
        }
        if !e.label.is_empty() && z > 0.5 {
            painter.text(
                pts[last / 2],
                egui::Align2::CENTER_CENTER,
                &e.label,
                egui::FontId::proportional(10.5 * z),
                color,
            );
        }
    }
    let mut hovered = None;
    let pointer = resp.hover_pos();
    let name_font = egui::FontId::proportional((12.5 * z).clamp(5.0, 22.0));
    let small = egui::FontId::proportional((11.0 * z).clamp(4.0, 19.0));
    let mono = egui::FontId::monospace((11.0 * z).clamp(4.0, 19.0));
    for (i, node) in shown.graph.nodes.iter().enumerate() {
        let r = place(i, pan);
        if !view.intersects(r) {
            continue;
        }
        let dim: f32 = if lit(i) { 1.0 } else { 0.3 };
        let selected = shown.selected == Some(i);
        let external = node.kind == "external";
        let fill = if selected {
            pal.accent.gamma_multiply(0.22)
        } else if external {
            pal.sidebar_bg
        } else {
            pal.editor_bg
        };
        let edge = if selected {
            pal.accent
        } else if related.contains(&i) {
            AMBER
        } else {
            node.accent.map(rgb).unwrap_or(pal.border)
        };
        painter.rect_filled(r, 6.0 * z, fill.gamma_multiply(dim.max(0.6)));
        painter.rect_stroke(r, 6.0 * z, Stroke::new(1.2, edge.gamma_multiply(dim)), egui::StrokeKind::Inside);
        if z > 0.28 {
            let clip = painter.with_clip_rect(r.shrink(2.0).intersect(rect));
            let mut y = r.top() + (uml::PAD + uml::ROW_H / 2.0) * z;
            if let Some(s) = uml::stereotype(&node.kind) {
                clip.text(
                    Pos2::new(r.center().x, y),
                    egui::Align2::CENTER_CENTER,
                    s,
                    small.clone(),
                    pal.text_dim.gamma_multiply(dim),
                );
                y += uml::ROW_H * z;
            }
            let name_color = if external { pal.text_dim } else { pal.text };
            clip.text(
                Pos2::new(r.center().x, y),
                egui::Align2::CENTER_CENTER,
                &node.label,
                name_font.clone(),
                name_color.gamma_multiply(dim),
            );
            let lines = if shown.members { uml::shown_members(node) } else { Vec::new() };
            if !lines.is_empty() {
                let sep = y + (uml::ROW_H / 2.0 + uml::PAD * 0.5) * z;
                clip.line_segment(
                    [Pos2::new(r.left(), sep), Pos2::new(r.right(), sep)],
                    Stroke::new(1.0, pal.border.gamma_multiply(dim)),
                );
                let mut my = sep + (uml::PAD * 0.5 + uml::ROW_H / 2.0) * z;
                for m in lines {
                    clip.text(
                        Pos2::new(r.left() + uml::PAD * z, my),
                        egui::Align2::LEFT_CENTER,
                        m,
                        mono.clone(),
                        pal.text.gamma_multiply(0.85 * dim),
                    );
                    my += uml::ROW_H * z;
                }
            }
        }
        if pointer.is_some_and(|p| r.contains(p)) {
            hovered = Some(i);
        }
    }
    if let Some(i) = hovered {
        let n = &shown.graph.nodes[i];
        let mut tip = n.label.clone();
        if !n.kind.is_empty() && n.kind != "class" {
            tip.push_str(&format!(" ({})", n.kind));
        }
        if !n.group.is_empty() {
            tip.push_str(&format!("\n{}", n.group));
        }
        if !n.path.is_empty() {
            tip.push_str(&format!("\n{}:{}", n.path, n.line));
        }
        resp.clone().on_hover_text_at_pointer(tip);
    }
    if resp.clicked() {
        shown.selected = hovered;
    }
    if resp.double_clicked()
        && let Some(i) = hovered
        && !shown.graph.nodes[i].path.is_empty()
    {
        let n = &shown.graph.nodes[i];
        action = Some(Action::Open(n.path.clone(), n.line));
    }
    if !filter.is_empty() && !(0..shown.graph.nodes.len()).any(matches) {
        painter.text(
            egui::pos2(rect.center().x, rect.top() + 18.0),
            egui::Align2::CENTER_CENTER,
            format!("Nothing here is called “{}”", shown.filter.trim()),
            egui::FontId::proportional(13.0),
            AMBER,
        );
    }
    let hint = "Drag to move · scroll to zoom · click to pick · double click to open";
    painter.text(
        egui::pos2(rect.right() - 12.0, rect.bottom() - 14.0),
        egui::Align2::RIGHT_CENTER,
        hint,
        egui::FontId::proportional(11.0),
        pal.text_dim,
    );
    action
}

/// A plugin's 0xRRGGBB.
fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The direction from `from` to `to`, and the side of it, scaled to `size`.
fn frame(from: Pos2, to: Pos2, size: f32) -> Option<(Vec2, Vec2)> {
    let dir = (to - from).normalized();
    dir.x.is_finite().then(|| (dir * size, egui::vec2(-dir.y, dir.x) * size * 0.55))
}

/// A hollow triangle with its tip at `to`: inherits, implements.
fn triangle(painter: &egui::Painter, from: Pos2, to: Pos2, size: f32, stroke: Stroke, fill: Color32) {
    let Some((d, side)) = frame(from, to, size) else { return };
    let back = to - d;
    painter.add(egui::Shape::convex_polygon(vec![to, back + side, back - side], fill, stroke));
}

/// A filled diamond with its tip at `at`, pointing away from `from`: has.
fn diamond(painter: &egui::Painter, from: Pos2, at: Pos2, size: f32, color: Color32) {
    let Some((d, side)) = frame(from, at, size * 0.6) else { return };
    let mid = at - d;
    painter.add(egui::Shape::convex_polygon(
        vec![at, mid + side * 0.8, at - d * 2.0, mid - side * 0.8],
        color,
        Stroke::NONE,
    ));
}

/// An open arrowhead at `to`: uses.
fn open_arrow(painter: &egui::Painter, from: Pos2, to: Pos2, size: f32, stroke: Stroke) {
    let Some((d, side)) = frame(from, to, size) else { return };
    let back = to - d;
    painter.line_segment([to, back + side], stroke);
    painter.line_segment([to, back - side], stroke);
}
