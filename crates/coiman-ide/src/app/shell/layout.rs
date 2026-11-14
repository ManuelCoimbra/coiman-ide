//! Where the side bar and the panel sit, and how big they are: moved from the panel's
//! layout menu or by command, kept with the session, put back by View: Reset Layout.

use coiman_core::editor::commands::Command;
use coiman_core::project::session::SavedLayout;
use eframe::egui;

use crate::app::CodeApp;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

/// Sizes the side bar and panel start at.
pub(crate) const SIDE_WIDTH: f32 = 260.0;
pub(crate) const PANEL_HEIGHT: f32 = 240.0;
pub(crate) const PANEL_WIDTH: f32 = 420.0;
/// The panel's window, when it first floats.
const WINDOW_SIZE: egui::Vec2 = egui::vec2(720.0, 320.0);
/// How near an edge of the window a floating panel is let go to dock there.
const SNAP: f32 = 28.0;

/// The layout, and what's needed to tell a resize from a slide.
#[derive(Default)]
pub(crate) struct Layout {
    pub(crate) saved: SavedLayout,
    side: Settle,
    height: Settle,
    width: Settle,
    /// Goes up on Reset Layout: the panels get new ids, so egui forgets the sizes it kept.
    generation: u32,
}

/// A size is kept only as a drag of the mouse ends with it holding still, which is how an edge
/// is dragged: a side bar or panel sliding open or shut, or sized by what it holds, doesn't count.
#[derive(Default)]
struct Settle {
    last: f32,
}

impl Settle {
    fn see(&mut self, now: f32, released: bool, keep: &mut Option<f32>, first: f32) {
        let moved = (now - keep.unwrap_or(first)).abs() >= 0.5;
        if released && (now - self.last).abs() < 0.5 && moved {
            *keep = Some(now);
        }
        self.last = now;
    }
}

impl Layout {
    /// The side bar is on the right. The Islands layout places each tool on a rail instead.
    pub(crate) fn side_right(&self) -> bool {
        self.saved.side_right && !rails_layout()
    }

    pub(crate) fn side_width(&self) -> f32 {
        self.saved.side_size.unwrap_or(SIDE_WIDTH)
    }

    pub(crate) fn panel_height(&self) -> f32 {
        self.saved.panel_height.unwrap_or(PANEL_HEIGHT)
    }

    pub(crate) fn panel_width(&self) -> f32 {
        self.saved.panel_width.unwrap_or(PANEL_WIDTH)
    }

    /// A panel's id, new after each Reset Layout.
    pub(crate) fn id(&self, name: &str) -> String {
        format!("{name}_{}", self.generation)
    }

    /// The side bar's egui panel, on the left or the right; each side has its own id so
    /// neither takes the other's size.
    pub(crate) fn side_panel(&self, name: &str) -> egui::Panel {
        if self.side_right() {
            egui::Panel::right(self.id(&format!("{name}_right")))
        } else {
            egui::Panel::left(self.id(name))
        }
    }

    /// The side bar's width as drawn this frame (its outer rect, with any margin).
    pub(crate) fn saw_side(&mut self, ctx: &egui::Context, width: f32) {
        let released = ctx.input(|i| i.pointer.primary_released());
        self.side.see(width, released, &mut self.saved.side_size, SIDE_WIDTH);
    }

    /// The docked panel's size as drawn this frame.
    pub(crate) fn saw_panel(&mut self, ctx: &egui::Context, size: egui::Vec2, right: bool) {
        let released = ctx.input(|i| i.pointer.primary_released());
        if right {
            self.width.see(size.x, released, &mut self.saved.panel_width, PANEL_WIDTH);
        } else {
            self.height.see(size.y, released, &mut self.saved.panel_height, PANEL_HEIGHT);
        }
    }
}

impl CodeApp {
    /// Runs the layout commands.
    pub(crate) fn layout_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleSidebarPosition => {
                if rails_layout() {
                    self.status = Some("In the Islands layout, drag a tool's icon to the other rail.".to_owned());
                    return;
                }
                self.layout.saved.side_right = !self.layout.saved.side_right;
                self.sidebar_visible = true;
            }
            Command::PanelBottom => self.place_panel(false, false),
            Command::PanelRight => self.place_panel(true, false),
            Command::PanelFloat => self.place_panel(self.rails.panel_right, true),
            Command::ResetLayout => {
                self.layout.saved = SavedLayout::default();
                self.layout.generation += 1;
                self.sheet_pinned = false;
                self.rails.panel_right = false;
                self.status = Some("Layout reset".to_owned());
            }
            _ => {}
        }
    }

    fn place_panel(&mut self, right: bool, floating: bool) {
        self.rails.panel_right = right;
        self.layout.saved.panel_floating = floating;
        self.panel_visible = true;
    }

    /// The button on the panel's header that moves it.
    pub(crate) fn panel_layout_button(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let button = icons::button(ui, Icon::Menu, "Move panel");
        let mut cmd = None;
        egui::Popup::menu(&button).show(|ui| {
            let floating = self.layout.saved.panel_floating;
            let right = self.rails.panel_right && !floating;
            let bottom = !self.rails.panel_right && !floating;
            let mut item = |ui: &mut egui::Ui, on: bool, text: &str, command: Command| {
                if ui.selectable_label(on, text).clicked() {
                    cmd = Some(command);
                }
            };
            item(ui, bottom, "Bottom", Command::PanelBottom);
            item(ui, right, "Right", Command::PanelRight);
            item(ui, floating, "In a window", Command::PanelFloat);
            ui.separator();
            if !rails_layout() {
                let side = if self.layout.saved.side_right { "Side bar to the left" } else { "Side bar to the right" };
                item(ui, false, side, Command::ToggleSidebarPosition);
            }
            item(ui, false, "Reset layout", Command::ResetLayout);
        });
        cmd
    }

    /// The panel in a window over the editor, when floating. `code` is where the editor is.
    /// Let go near the bottom or right edge of the editor, it docks there.
    pub(crate) fn panel_window(&mut self, ctx: &egui::Context, code: egui::Rect) {
        let saved = self.layout.saved.panel_window;
        let start = saved.map_or_else(
            || egui::Rect::from_center_size(code.center(), WINDOW_SIZE.min(code.size() * 0.9)),
            |[x, y, w, h]| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
        );
        let pal = theme::colors();
        let frame = egui::Frame::new()
            .fill(pal.editor_bg)
            .corner_radius(12.0)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: egui::Color32::from_black_alpha(110) })
            .inner_margin(egui::Margin::symmetric(12, 6));
        let shown = egui::Window::new("Panel")
            .id(egui::Id::unique("panel_window"))
            .title_bar(false)
            .frame(frame)
            .default_rect(start)
            .min_size(egui::vec2(260.0, 120.0))
            .resizable(true)
            .constrain_to(ctx.content_rect())
            .show(ctx, |ui| {
                ui.set_min_size(ui.available_size());
                self.panel(ui);
            });
        let Some(shown) = shown else { return };
        let rect = shown.response.rect;
        self.layout.saved.panel_window = Some([rect.left(), rect.top(), rect.width(), rect.height()]);
        // Dragged by its header to an edge: docks there, showing where while it's held.
        let held = ctx.input(|i| i.pointer.primary_down()) && shown.response.dragged();
        let pointer = ctx.input(|i| i.pointer.latest_pos());
        let edge = pointer.and_then(|p| {
            if (code.right() - p.x).abs() < SNAP && code.y_range().contains(p.y) {
                Some(true)
            } else if (code.bottom() - p.y).abs() < SNAP && code.x_range().contains(p.x) {
                Some(false)
            } else {
                None
            }
        });
        if let Some(right) = edge {
            if held {
                let zone = if right {
                    egui::Rect::from_min_max(
                        egui::pos2(code.right() - PANEL_WIDTH.min(code.width() / 2.0), code.top()),
                        code.max,
                    )
                } else {
                    egui::Rect::from_min_max(
                        egui::pos2(code.left(), code.bottom() - PANEL_HEIGHT.min(code.height() / 2.0)),
                        code.max,
                    )
                }
                .shrink(6.0);
                let painter =
                    ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("dock_zone")));
                painter.rect_filled(zone, 12.0, pal.accent.gamma_multiply(0.12));
                painter.rect_stroke(zone, 12.0, egui::Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
                let text = if right { "Dock at the right" } else { "Dock at the bottom" };
                painter.text(
                    zone.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(15.0),
                    pal.text,
                );
            } else if shown.response.drag_stopped() {
                self.place_panel(right, false);
            }
        }
    }
}

/// The Islands layout, where each tool has a place on a rail instead of in one side bar.
/// (The Dock layout draws islands too, but has a side bar of its own.)
fn rails_layout() -> bool {
    theme::islands() && !theme::dock()
}
