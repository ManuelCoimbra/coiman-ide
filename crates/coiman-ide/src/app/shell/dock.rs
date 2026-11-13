//! The Dock layout: no rails and no docked side bar. A dock at the bottom of the window holds
//! the tools; a tool opens as a sheet floating over the code, which closes when the code is
//! clicked, or can be pinned to stay beside it. The status sits in the strip, right of the dock.

use egui::{Color32, Sense};

use crate::commands::Command;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

use crate::app::{CodeApp, PanelTab, SidebarView};

/// Height of the strip the dock sits in.
pub(crate) const STRIP_HEIGHT: f32 = 46.0;
/// One dock button.
const BUTTON: egui::Vec2 = egui::vec2(38.0, 34.0);
/// How far the icon sits above its tile's middle, and the open-tool dot below it.
const ICON_ABOVE: f32 = 2.0;
const DOT_BELOW: f32 = 10.0;
/// Room between the sheet and the edges of the code.
const SHEET_GAP: f32 = 8.0;

const SIDE: [(SidebarView, Icon, &str, Command); 6] = [
    (SidebarView::Explorer, Icon::Files, "Project (Ctrl+Shift+E)", Command::ShowExplorer),
    (SidebarView::Search, Icon::Search, "Search (Ctrl+Shift+F)", Command::ShowSearch),
    (SidebarView::Git, Icon::SourceControl, "Source Control (Ctrl+Shift+G)", Command::ShowSourceControl),
    (SidebarView::Testing, Icon::Beaker, "Testing", Command::ShowTesting),
    (SidebarView::Debug, Icon::Bug, "Run and Debug (Ctrl+Shift+D)", Command::ShowDebug),
    (SidebarView::Extensions, Icon::Extensions, "Extensions (Ctrl+Shift+X)", Command::ShowExtensions),
];

const PANELS: [(PanelTab, Icon, &str, Command); 3] = [
    (PanelTab::Terminal, Icon::Terminal, "Terminal (Ctrl+`)", Command::ToggleTerminal),
    (PanelTab::Problems, Icon::Alert, "Problems", Command::ShowProblems),
    (PanelTab::Output, Icon::Lines, "Output", Command::ShowOutput),
];

/// "DEBUG CONSOLE" as "Debug console", and "PROBLEMS 3" as "Problems 3".
pub(crate) fn sentence_case(label: &str) -> String {
    let lower = label.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// The bottom panel's card: it floats above the dock, rounded, with room around it.
pub(crate) fn panel_frame() -> egui::Frame {
    let pal = theme::colors();
    egui::Frame::new()
        .fill(pal.sidebar_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(14.0)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .outer_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 0 })
}

/// The main menu's button: the Coiman icon.
pub(crate) fn logo_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 30.0), Sense::click());
    let hovered = theme::eased(ui.ctx(), response.id.with("hover"), response.hovered());
    if hovered > 0.0 {
        ui.painter().rect_filled(rect, 8.0, theme::colors().hover_bg.gamma_multiply(hovered));
    }
    let logo = crate::app::workbench::logo_texture(ui.ctx());
    let at = egui::Rect::from_center_size(rect.center(), egui::vec2(22.0, 22.0));
    egui::Image::new(&logo).paint_at(ui, at);
    response.on_hover_text("Main Menu")
}

impl CodeApp {
    /// Run, what it starts, and Debug, as one green pill in the top bar.
    pub(crate) fn run_pill(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        let pal = theme::colors();
        let green = Color32::from_rgb(0x57, 0xC0, 0x6A);
        let target = self.run_target_label().unwrap_or_else(|| "Current File".to_owned());
        let font = egui::TextStyle::Body.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(target, font, pal.text);
        let text_w = galley.size().x.min(200.0);
        let can_run = self.run_button_visible();
        let can_debug = self.can_debug();
        let seg = 30.0;
        let width = seg + 10.0 + text_w + 22.0 + if can_debug { seg } else { 0.0 };
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 28.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 14.0, green.gamma_multiply(0.12));
        p.rect_stroke(rect, 14.0, egui::Stroke::new(1.0, green.gamma_multiply(0.45)), egui::StrokeKind::Inside);
        // Run.
        let run = egui::Rect::from_min_size(rect.min, egui::vec2(seg, rect.height()));
        let resp = ui.interact(run, ui.scope_id().with("pill_run"), Sense::click());
        if resp.hovered() && can_run {
            ui.painter().rect_filled(run.shrink(2.0), 12.0, green.gamma_multiply(0.22));
        }
        let color = if can_run { green } else { pal.text_dim };
        icons::paint(
            ui.painter(),
            egui::Rect::from_center_size(run.center(), egui::vec2(14.0, 14.0)),
            Icon::Play,
            color,
        );
        if can_run && resp.on_hover_text(self.run_button_tip()).clicked() {
            cmd = Some(Command::RunProgram);
        }
        // What it starts.
        let pick = egui::Rect::from_min_max(
            egui::pos2(run.right(), rect.top()),
            egui::pos2(run.right() + 10.0 + text_w + 22.0, rect.bottom()),
        );
        let resp = ui.interact(pick, ui.scope_id().with("pill_target"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(pick.shrink2(egui::vec2(0.0, 2.0)), 6.0, pal.hover_bg);
        }
        let text_at = egui::pos2(pick.left() + 4.0, rect.center().y - galley.size().y / 2.0);
        let clip = pick.with_max_x(pick.left() + 4.0 + text_w);
        ui.painter().with_clip_rect(clip).galley(text_at, galley, pal.text);
        let (x, y) = (pick.right() - 12.0, rect.center().y);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![egui::pos2(x - 3.5, y - 2.0), egui::pos2(x + 3.5, y - 2.0), egui::pos2(x, y + 2.5)],
            pal.text_dim,
            egui::Stroke::NONE,
        ));
        if resp.on_hover_text("Choose what Run starts").clicked() {
            cmd = Some(Command::ChooseRunTarget);
        }
        // Debug.
        if can_debug {
            let debug = egui::Rect::from_min_max(egui::pos2(pick.right(), rect.top()), rect.max);
            ui.painter().line_segment(
                [egui::pos2(debug.left(), rect.top() + 6.0), egui::pos2(debug.left(), rect.bottom() - 6.0)],
                egui::Stroke::new(1.0, green.gamma_multiply(0.35)),
            );
            let resp = ui.interact(debug, ui.scope_id().with("pill_debug"), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(debug.shrink(2.0), 12.0, green.gamma_multiply(0.22));
            }
            let glyph = egui::Rect::from_center_size(debug.center(), egui::vec2(16.0, 16.0));
            icons::paint(ui.painter(), glyph, Icon::Bug, green);
            if resp.on_hover_text("Debug (F5)").clicked() {
                cmd = Some(Command::StartDebugging);
            }
        }
        self.tour_spot("run", rect);
        cmd
    }

    /// The bottom strip: the dock in the middle, the status on its right.
    pub(crate) fn dock_strip(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        let pal = theme::colors();
        let strip = ui.max_rect();
        let side: Vec<_> = SIDE
            .into_iter()
            .filter(|(view, ..)| match view {
                SidebarView::Testing => self.testing_available(),
                SidebarView::Debug => self.can_debug(),
                _ => true,
            })
            .collect();
        let count = side.len() + PANELS.len();
        let width = count as f32 * BUTTON.x + 12.0 + 16.0;
        let pill = egui::Rect::from_center_size(
            egui::pos2(strip.center().x.max(strip.left() + width / 2.0 + 8.0), strip.center().y),
            egui::vec2(width, BUTTON.y + 6.0),
        );
        let p = ui.painter();
        p.rect_filled(pill, pill.height() / 2.0, pal.sidebar_bg);
        p.rect_stroke(pill, pill.height() / 2.0, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
        let mut x = pill.left() + 8.0;
        let mut open_at = None;
        for (view, icon, tip, command) in side {
            let open = self.sidebar_shown() && self.sidebar_view == view;
            if open {
                open_at = Some(x + BUTTON.x / 2.0);
            }
            let rect = egui::Rect::from_min_size(egui::pos2(x, pill.top() + 3.0), BUTTON);
            if self.dock_button(ui, rect, icon, tip, open) {
                if open {
                    self.sidebar_visible = false;
                    self.welcome_sidebar = false;
                } else {
                    cmd = Some(command);
                }
            }
            let spot = match view {
                SidebarView::Explorer => Some("explorer"),
                SidebarView::Git => Some("git"),
                SidebarView::Extensions => Some("extensions"),
                _ => None,
            };
            if let Some(spot) = spot {
                self.tour_spot(spot, rect);
            }
            x += BUTTON.x;
        }
        // Between the sheets and the panels.
        let sep = egui::pos2(x + 6.0, pill.center().y);
        ui.painter()
            .line_segment([sep - egui::vec2(0.0, 9.0), sep + egui::vec2(0.0, 9.0)], egui::Stroke::new(1.0, pal.border));
        x += 12.0;
        let mut panel_at = None;
        for (tab, icon, tip, command) in PANELS {
            let open = self.panel_visible && self.panel_tab == tab;
            if open {
                panel_at = Some(x + BUTTON.x / 2.0);
            }
            let rect = egui::Rect::from_min_size(egui::pos2(x, pill.top() + 3.0), BUTTON);
            if self.dock_button(ui, rect, icon, tip, open) {
                cmd = Some(command);
            }
            if tab == PanelTab::Terminal {
                self.tour_spot("terminal", rect);
            }
            x += BUTTON.x;
        }
        // The dots under what's open glide from one tool to the next.
        let ctx = ui.ctx().clone();
        // Inside the tile, under its icon (see `dock_button`).
        let dot_y = pill.top() + 3.0 + BUTTON.y / 2.0 + DOT_BELOW;
        for (at, name) in [(open_at, "dock_dot_side"), (panel_at, "dock_dot_panel")] {
            let id = egui::Id::unique(name);
            let shown = theme::eased(&ctx, id.with("shown"), at.is_some());
            let x = match at {
                Some(at) => theme::glide(&ctx, id, at),
                None => ctx.data(|d| d.get_temp::<f32>(id.with("last"))).unwrap_or(0.0),
            };
            if at.is_some() {
                ctx.data_mut(|d| d.insert_temp(id.with("last"), x));
            }
            if shown > 0.0 {
                ui.painter().circle_filled(egui::pos2(x, dot_y), 2.0 * shown, pal.accent.gamma_multiply(shown));
            }
        }
        // What's going on (servers starting, messages), left of the dock.
        let news = egui::Rect::from_min_max(strip.left_top(), egui::pos2(pill.left() - 16.0, strip.bottom()))
            .shrink2(egui::vec2(12.0, 0.0));
        if news.width() > 60.0 {
            let mut child = ui.new_child(
                egui::UiBuilder::new().max_rect(news).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            child.spacing_mut().item_spacing.x = 14.0;
            let small = egui::TextStyle::Small.resolve(child.style()).size + 1.0;
            if let Some(text) = self.lsp_status_text() {
                let label = egui::RichText::new(text).size(small).color(pal.text_dim);
                let resp = child
                    .add(egui::Label::new(label).truncate().sense(Sense::click()).selectable(false))
                    .on_hover_text("Language Servers: state, memory and controls");
                if resp.clicked() {
                    cmd = Some(Command::ShowLanguageServers);
                }
            }
            if let Some(msg) = &self.status {
                let label = egui::RichText::new(msg.clone()).size(small).color(pal.text);
                child.add(egui::Label::new(label).truncate()).on_hover_text(msg);
            }
        }
        // The status, in what's left right of the dock.
        let status = egui::Rect::from_min_max(egui::pos2(pill.right() + 16.0, strip.top()), strip.right_bottom())
            .shrink2(egui::vec2(8.0, 0.0));
        if status.width() > 60.0 {
            let mut child = ui.new_child(
                egui::UiBuilder::new().max_rect(status).layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            cmd = cmd.or(self.status_bar(&mut child));
            self.tour_spot("status", status);
        }
        cmd
    }

    /// A dock button: the icon, a lit tile when hovered, and a dot under it when its tool is open.
    fn dock_button(&mut self, ui: &mut egui::Ui, rect: egui::Rect, icon: Icon, tip: &str, open: bool) -> bool {
        let pal = theme::colors();
        let response = ui.interact(rect, ui.scope_id().with(("dock", tip)), Sense::click()).on_hover_text(tip);
        let hovered = theme::eased(ui.ctx(), response.id.with("hover"), response.hovered());
        let opened = theme::eased(ui.ctx(), response.id.with("open"), open);
        let tile = rect.shrink(3.0);
        if hovered > 0.0 {
            ui.painter().rect_filled(tile, 9.0, pal.hover_bg.gamma_multiply(hovered));
        }
        if opened > 0.0 {
            ui.painter().rect_filled(tile, 9.0, pal.accent.gamma_multiply(0.16 * opened));
        }
        let color = if open {
            pal.accent
        } else if response.hovered() {
            pal.text
        } else {
            pal.text_dim
        };
        // The icon sits a little above the tile's middle in every state, leaving the dot room
        // below it, so icons line up across the dock whether their tool is open or not.
        let glyph = egui::Rect::from_center_size(tile.center() - egui::vec2(0.0, ICON_ABOVE), egui::vec2(17.0, 17.0));
        icons::paint(ui.painter(), glyph, icon, color);
        response.clicked()
    }

    /// The open tool as a sheet over the code (`code` is where the code is), unless pinned.
    /// A click in the code closes it.
    pub(crate) fn side_sheet(&mut self, ctx: &egui::Context, code: egui::Rect, shown: bool) {
        let started = egui::Id::unique("side_sheet_started");
        if !ctx.data(|d| d.get_temp::<bool>(started).unwrap_or(false)) {
            ctx.data_mut(|d| d.insert_temp(started, true));
            self.sidebar_visible = false;
            return;
        }
        let time = ctx.global_style().animation_time * 1.4;
        let t = ctx.animate_bool_with_time(egui::Id::unique("side_sheet_open"), shown, time);
        // Another tool in the same sheet: its content slides in, rather than swapping at once.
        let view_id = egui::Id::unique("side_sheet_view");
        let view = egui::Id::unique(format!("{:?}", self.sidebar_view));
        let now = ctx.input(|i| i.time);
        let since = ctx.data_mut(|d| {
            let (last, at) = d.get_temp::<(egui::Id, f64)>(view_id).unwrap_or((view, 0.0));
            if last != view {
                d.insert_temp(view_id, (view, now));
                0.0
            } else {
                d.insert_temp(view_id, (view, at));
                now - at
            }
        });
        let swap = if time > 0.0 { (since as f32 / time).clamp(0.0, 1.0) } else { 1.0 };
        if swap < 1.0 {
            ctx.request_repaint();
        }
        let swap = 1.0 - (1.0 - swap).powi(3);
        if t <= 0.0 {
            return;
        }
        let pal = theme::colors();
        let width = 320.0_f32.min(code.width() * 0.6);
        // Slides in from the left edge, slowing as it arrives.
        let eased = 1.0 - (1.0 - t).powi(3);
        let rect = egui::Rect::from_min_size(
            code.left_top() + egui::vec2(SHEET_GAP - (1.0 - eased) * (width + SHEET_GAP + 24.0), SHEET_GAP),
            egui::vec2(width, code.height() - 2.0 * SHEET_GAP),
        );
        let frame = egui::Frame::new()
            .fill(pal.sidebar_bg)
            .corner_radius(14.0)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(110) })
            .inner_margin(egui::Margin::symmetric(10, 8));
        let area = egui::Area::new(egui::Id::unique("side_sheet"))
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .interactable(shown)
            .show(ctx, |ui| {
                ui.set_opacity(eased.max(0.35));
                frame.show(ui, |ui| {
                    ui.set_width(rect.width() - 20.0);
                    ui.set_height(rect.height() - 16.0);
                    let inner = ui.max_rect().translate(egui::vec2((1.0 - swap) * 16.0, 0.0));
                    let mut content = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                    content.set_opacity(swap);
                    self.sidebar(&mut content);
                });
            });
        let sheet = area.response.rect;
        self.tour_spot("sidebar", sheet);
        if !shown {
            return;
        }
        if self.pin_chip(ctx, sheet, false) {
            self.sheet_pinned = true;
        }
        // A click in the code, outside the sheet and anything floating, puts it away.
        let away = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.interact_pos()).flatten());
        if let Some(at) = away
            && code.contains(at)
            && !sheet.expand(4.0).contains(at)
            && ctx.layer_id_at(at).is_none_or(|layer| layer.order == egui::Order::Background)
        {
            self.sidebar_visible = false;
            self.welcome_sidebar = false;
        }
    }

    /// The round button on a sheet's right edge that pins it beside the code, or lets it float
    /// again. Returns whether it was clicked.
    pub(crate) fn pin_chip(&mut self, ctx: &egui::Context, sheet: egui::Rect, pinned: bool) -> bool {
        let pal = theme::colors();
        let center = egui::pos2(sheet.right() + 16.0, sheet.top() + 22.0);
        let mut clicked = false;
        egui::Area::new(egui::Id::unique("sheet_pin"))
            .order(egui::Order::Foreground)
            .fixed_pos(center - egui::vec2(12.0, 12.0))
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
                let resp =
                    resp.on_hover_text(if pinned { "Let it float over the code" } else { "Pin beside the code" });
                let fill = if resp.hovered() { pal.hover_bg } else { pal.widget_bg };
                ui.painter().circle_filled(rect.center(), 12.0, fill);
                ui.painter().circle_stroke(rect.center(), 12.0, egui::Stroke::new(1.0, pal.border));
                let color = if pinned { pal.accent } else { pal.text_dim };
                icons::paint(ui.painter(), rect.shrink(6.0), Icon::Pin, color);
                clicked = resp.clicked();
            });
        clicked
    }
}
