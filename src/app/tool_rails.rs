//! The island layout's tool rails: icons along both edges of the window open tools on that
//! side, and dragging an icon to the other rail moves its tool there.
//!
//! The rest of the app asks for a tool by setting `sidebar_view`. Before each frame's panels,
//! [`CodeApp::place_tools`] sends what belongs on the right there.

use egui::{Color32, Sense};

use crate::commands::Command;
use crate::icons::{self, Icon};
use crate::theme;

use super::{CodeApp, PanelTab, SidebarView};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RailTool {
    Side(SidebarView),
    Panel(PanelTab),
}

const SIDE_TOOLS: [(SidebarView, &str, Icon, &str, Command); 6] = [
    (SidebarView::Explorer, "explorer", Icon::Files, "Project (Ctrl+Shift+E)", Command::ShowExplorer),
    (SidebarView::Search, "search", Icon::Search, "Search (Ctrl+Shift+F)", Command::ShowSearch),
    (SidebarView::Git, "git", Icon::SourceControl, "Source Control (Ctrl+Shift+G)", Command::ShowSourceControl),
    (SidebarView::Testing, "testing", Icon::Beaker, "Testing", Command::ShowTesting),
    (SidebarView::Debug, "debug", Icon::Bug, "Run and Debug (Ctrl+Shift+D)", Command::ShowDebug),
    (SidebarView::Extensions, "extensions", Icon::Extensions, "Extensions (Ctrl+Shift+X)", Command::ShowExtensions),
];

const PANEL_TOOLS: [(PanelTab, Icon, &str, Command); 3] = [
    (PanelTab::Output, Icon::Lines, "Output", Command::ShowOutput),
    (PanelTab::Problems, Icon::Alert, "Problems", Command::ShowProblems),
    (PanelTab::Terminal, Icon::Terminal, "Terminal (Ctrl+`)", Command::ToggleTerminal),
];

const RAIL_WIDTH: f32 = 44.0;

pub(super) struct ToolRails {
    /// The side tools on the right rail; the rest are on the left.
    right_tools: Vec<SidebarView>,
    /// The panel's tools are on the right rail, and it opens at the right, not the bottom.
    pub(super) panel_right: bool,
    /// What's open on the right.
    pub(super) right_view: Option<SidebarView>,
    /// What was last open on the right, to slide away with.
    pub(super) right_last: Option<SidebarView>,
    /// What's open on the left, as a tool opening on the right mustn't change it.
    left_view: SidebarView,
    left_visible: bool,
    dragging: Option<RailTool>,
}

impl ToolRails {
    pub(super) fn new() -> Self {
        Self {
            right_tools: Vec::new(),
            panel_right: false,
            right_view: None,
            right_last: None,
            left_view: SidebarView::Explorer,
            left_visible: true,
            dragging: None,
        }
    }

    /// What the session keeps: the names of the tools on the right.
    pub(super) fn saved(&self) -> Vec<String> {
        let name = |view: &SidebarView| SIDE_TOOLS.iter().find(|t| t.0 == *view).map(|t| t.1.to_owned());
        self.right_tools.iter().filter_map(name).collect()
    }

    pub(super) fn restore(&mut self, names: &[String], panel_right: bool) {
        let known = |name: &String| SIDE_TOOLS.iter().find(|t| t.1 == name).map(|t| t.0);
        self.right_tools = names.iter().filter_map(known).collect();
        self.panel_right = panel_right;
    }
}

impl CodeApp {
    /// Sends a tool asked for to its side, before the panels are drawn.
    pub(super) fn place_tools(&mut self) {
        if !theme::islands() {
            return;
        }
        let gone = |view| match view {
            SidebarView::Testing => !self.testing_available(),
            SidebarView::Debug => !self.can_debug(),
            _ => false,
        };
        if self.rails.right_view.is_some_and(gone) {
            self.rails.right_view = None;
        }
        let rails = &mut self.rails;
        if rails.right_tools.contains(&self.sidebar_view) {
            if self.sidebar_visible {
                rails.right_view = Some(self.sidebar_view);
            }
            self.sidebar_view = rails.left_view;
            self.sidebar_visible = rails.left_visible && !rails.right_tools.contains(&rails.left_view);
        } else {
            rails.left_view = self.sidebar_view;
            rails.left_visible = self.sidebar_visible;
        }
    }

    fn move_tool(&mut self, tool: RailTool, to_right: bool) {
        let side = if to_right { "right" } else { "left" };
        rust_code_core::applog::info(&format!("moved {tool:?} to the {side} rail"));
        match tool {
            RailTool::Panel(_) => self.rails.panel_right = to_right,
            RailTool::Side(view) if to_right => {
                if self.rails.right_tools.contains(&view) {
                    return;
                }
                self.rails.right_tools.push(view);
                let open = self.sidebar_visible && self.sidebar_view == view;
                if open {
                    self.rails.right_view = Some(view);
                }
                // The left falls back on a tool still there, closed if it was showing this one.
                if self.sidebar_view == view {
                    let rest = SIDE_TOOLS.iter().map(|t| t.0).find(|v| !self.rails.right_tools.contains(v));
                    self.sidebar_view = rest.unwrap_or(view);
                    self.sidebar_visible = false;
                }
            }
            RailTool::Side(view) => {
                if !self.rails.right_tools.contains(&view) {
                    return;
                }
                self.rails.right_tools.retain(|v| *v != view);
                if self.rails.right_view == Some(view) {
                    self.rails.right_view = None;
                    self.sidebar_view = view;
                    self.sidebar_visible = true;
                }
            }
        }
    }

    /// One rail: its side tools from the top, the panel's from the bottom.
    pub(super) fn tool_rail(&mut self, ui: &mut egui::Ui, right: bool) -> Option<Command> {
        let mut cmd = None;
        let whole = ui.max_rect();
        // Where a dragged icon would land.
        if self.rails.dragging.is_some()
            && let Some(at) = ui.ctx().pointer_interact_pos()
            && (at.x > ui.ctx().content_rect().center().x) == right
        {
            ui.painter().rect_filled(whole.shrink(2.0), 8.0, theme::colors().selection.gamma_multiply(0.5));
        }
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(4.0);
        for (view, _, icon, tip, command) in SIDE_TOOLS {
            let available = match view {
                SidebarView::Testing => self.testing_available(),
                SidebarView::Debug => self.can_debug(),
                _ => true,
            };
            if !available || self.rails.right_tools.contains(&view) != right {
                continue;
            }
            let open = if right {
                self.rails.right_view == Some(view)
            } else {
                self.sidebar_visible && self.sidebar_view == view
            };
            if self.rail_icon(ui, RailTool::Side(view), icon, tip, open) {
                if !open {
                    cmd = Some(command);
                } else if right {
                    self.rails.right_view = None;
                } else {
                    self.sidebar_visible = false;
                }
            }
        }
        if self.rails.panel_right == right {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                for (tab, icon, tip, command) in PANEL_TOOLS.into_iter().rev() {
                    let open = self.panel_visible && self.panel_tab == tab;
                    if self.rail_icon(ui, RailTool::Panel(tab), icon, tip, open) {
                        cmd = Some(command);
                    }
                }
            });
        }
        cmd
    }

    /// An icon that can be clicked or dragged to the other rail. Returns whether it was clicked.
    fn rail_icon(&mut self, ui: &mut egui::Ui, tool: RailTool, icon: Icon, tip: &str, open: bool) -> bool {
        let pal = theme::colors();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(RAIL_WIDTH, 38.0), Sense::click_and_drag());
        let tile = rect.shrink2(egui::vec2(5.0, 1.0));
        let spot = match tool {
            RailTool::Side(SidebarView::Explorer) => Some("explorer"),
            RailTool::Side(SidebarView::Git) => Some("git"),
            RailTool::Side(SidebarView::Extensions) => Some("extensions"),
            RailTool::Panel(PanelTab::Terminal) => Some("terminal"),
            _ => None,
        };
        if let Some(spot) = spot {
            self.tour_spot(spot, tile);
        }
        let opened = theme::eased(ui.ctx(), response.id.with("open"), open);
        let hovered = theme::eased(ui.ctx(), response.id.with("hover"), response.hovered());
        if hovered > 0.0 {
            ui.painter().rect_filled(tile, 8.0, pal.hover_bg.gamma_multiply(hovered));
        }
        if opened > 0.0 {
            ui.painter().rect_filled(tile, 8.0, pal.selection.gamma_multiply(opened));
        }
        let color = if open || response.hovered() { pal.tab_active_fg } else { pal.text_dim };
        let glyph = egui::Rect::from_center_size(rect.center(), egui::vec2(21.0, 21.0));
        icons::paint(ui.painter(), glyph, icon, color);
        if response.drag_started() {
            self.rails.dragging = Some(tool);
        }
        if self.rails.dragging == Some(tool)
            && let Some(at) = ui.ctx().pointer_interact_pos()
        {
            let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("rail_drag"));
            let painter = ui.ctx().layer_painter(layer);
            let ghost = egui::Rect::from_center_size(at, egui::vec2(34.0, 34.0));
            painter.rect_filled(ghost, 8.0, pal.selection);
            icons::paint(&painter, ghost.shrink(7.0), icon, Color32::WHITE);
        }
        if response.drag_stopped() {
            self.rails.dragging = None;
            if let Some(at) = ui.ctx().pointer_interact_pos().or_else(|| response.interact_pointer_pos()) {
                self.move_tool(tool, at.x > ui.ctx().content_rect().center().x);
            }
        }
        response.on_hover_text(tip).clicked()
    }
}
