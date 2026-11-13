//! Two editor panes. The focused pane always shows `CodeApp::active`, so the rest
//! of the app keeps working with one active document. The other pane remembers
//! its document and its own scroll position; clicking it swaps the two.

use eframe::egui::{self, Rect, Sense, Stroke};

use crate::app::CodeApp;
use crate::ui::editor_view::{self, Decorations, ViewState};
use crate::ui::theme;

const DIVIDER: f32 = 5.0;

pub(crate) struct Split {
    /// Document in the pane without the focus.
    other: u64,
    other_view: ViewState,
    /// Whether the focused pane is the right or lower one.
    second_active: bool,
    /// Side by side, or one above the other.
    side_by_side: bool,
    /// Share of the room the first pane gets.
    ratio: f32,
}

impl CodeApp {
    /// Which pane is focused: 0 is the left or upper one.
    pub(crate) fn active_pane(&self) -> u8 {
        u8::from(self.split.as_ref().is_some_and(|s| s.second_active))
    }

    /// Splits the editor, showing the active document in both panes.
    pub(crate) fn split_editor(&mut self) {
        if self.split.is_some() {
            self.status = Some("The editor is already split".to_owned());
        } else if let Some(uid) = self.docs.get(self.active).map(|d| d.uid) {
            self.open_beside(uid);
        }
    }

    /// Opens a document in the second pane and focuses it, splitting if needed.
    pub(crate) fn open_beside(&mut self, uid: u64) {
        let Some(i) = self.docs.iter().position(|d| d.uid == uid) else { return };
        match &self.split {
            Some(split) => {
                if !split.second_active {
                    self.focus_other_pane();
                }
            }
            None => {
                let Some(other) = self.docs.get(self.active).map(|d| d.uid) else { return };
                let other_view = self.views.remove(&other).unwrap_or_default();
                self.split = Some(Split { other, other_view, second_active: true, side_by_side: true, ratio: 0.5 });
            }
        }
        self.active = i;
        let doc = &mut self.docs[i];
        doc.request_focus = true;
        doc.reveal = true;
    }

    pub(crate) fn close_split(&mut self) {
        self.split = None;
    }

    /// After closing the focused tab: if the next tab is the one the other pane
    /// already shows, one pane is enough.
    pub(crate) fn close_split_if_doubled(&mut self) {
        let active = self.docs.get(self.active).map(|d| d.uid);
        if self.split.as_ref().is_some_and(|s| Some(s.other) == active) {
            self.split = None;
        }
    }

    pub(crate) fn toggle_split_layout(&mut self) {
        if let Some(split) = &mut self.split {
            split.side_by_side = !split.side_by_side;
        }
    }

    pub(crate) fn focus_pane(&mut self, second: bool) {
        if self.split.as_ref().is_some_and(|s| s.second_active != second) {
            self.focus_other_pane();
        } else if let Some(doc) = self.docs.get_mut(self.active) {
            doc.request_focus = true;
        }
    }

    /// Swaps the panes' roles: each view stays with its pane.
    fn focus_other_pane(&mut self) {
        let Some(split) = &mut self.split else { return };
        let Some(i) = self.docs.iter().position(|d| d.uid == split.other) else { return };
        let Some(current) = self.docs.get(self.active).map(|d| d.uid) else { return };
        let view = self.views.remove(&current).unwrap_or_default();
        let view = std::mem::replace(&mut split.other_view, view);
        self.views.insert(split.other, view);
        split.other = current;
        split.second_active = !split.second_active;
        self.active = i;
        self.docs[i].request_focus = true;
    }

    /// Divides the room for the editor, draws the divider, and returns the
    /// focused pane's rectangle and the other pane's if there is one.
    pub(crate) fn split_rects(&mut self, ui: &mut egui::Ui) -> (Rect, Option<Rect>) {
        let area = ui.available_rect_before_wrap();
        // Its document was closed: back to one pane.
        if self.split.as_ref().is_some_and(|s| !self.docs.iter().any(|d| d.uid == s.other)) {
            self.split = None;
        }
        let Some(split) = &mut self.split else { return (area, None) };
        let size = if split.side_by_side { area.width() } else { area.height() };
        let at = (size * split.ratio).round();
        let (first, handle, second) = if split.side_by_side {
            let x = area.left() + at;
            let (first, rest) = area.split_left_right_at_x(x);
            let (handle, second) = rest.split_left_right_at_x(x + DIVIDER);
            (first, handle, second)
        } else {
            let y = area.top() + at;
            let (first, rest) = area.split_top_bottom_at_y(y);
            let (handle, second) = rest.split_top_bottom_at_y(y + DIVIDER);
            (first, handle, second)
        };
        let response = ui.interact(handle, egui::Id::unique("editor_split_divider"), Sense::drag());
        if response.dragged() {
            let moved = if split.side_by_side { response.drag_delta().x } else { response.drag_delta().y };
            split.ratio = (split.ratio + moved / size.max(1.0)).clamp(0.15, 0.85);
        }
        if response.hovered() || response.dragged() {
            let icon =
                if split.side_by_side { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical };
            ui.ctx().set_cursor_icon(icon);
        }
        ui.painter().rect_filled(handle, 0.0, theme::colors().editor_bg);
        let line = Stroke::new(1.0, theme::colors().border);
        if split.side_by_side {
            ui.painter().vline(handle.center().x, handle.y_range(), line);
        } else {
            ui.painter().hline(handle.x_range(), handle.center().y, line);
        }
        if split.second_active { (second, Some(first)) } else { (first, Some(second)) }
    }

    /// Everything drawn over document `i`'s text.
    pub(crate) fn decorations(&mut self, i: usize) -> Decorations {
        let mut deco = self.lsp_decorations(i);
        self.git_decorate(i, &mut deco);
        self.cargo_decorate(i, &mut deco);
        self.debug_decorate(i, &mut deco);
        // Plugins last: their colors go over the server's.
        self.light_colors_decorate(i, &mut deco);
        self.lsp_views_decorate(i, &mut deco);
        self.table_decorate(i, &mut deco);
        self.plugin_decorate(i, &mut deco);
        self.bookmarks_decorate(i, &mut deco);
        self.prints_decorate(i, &mut deco);
        self.repl_decorate(i, &mut deco);
        // Colors on themselves, over the syntax's and the server's.
        self.color_preview_decorate(i, &mut deco);
        // Over everything: these must stay visible.
        self.hidden_decorate(i, &mut deco);
        self.profile_decorate(i, &mut deco);
        self.coverage_decorate(i, &mut deco);
        if let Some(doc) = self.docs.get_mut(i) {
            deco.folds = doc.structure().folds.clone();
        }
        deco
    }

    /// Draws the pane without the focus. Pressing a button in it moves the focus there.
    pub(crate) fn other_pane(&mut self, ui: &mut egui::Ui, rect: Rect, opts: editor_view::Options) {
        let Some(uid) = self.split.as_ref().map(|s| s.other) else { return };
        let Some(i) = self.docs.iter().position(|d| d.uid == uid) else { return };
        let mut deco = self.decorations(i);
        self.font_names_decorate(&ui.ctx().clone(), i, &mut deco);
        let pane = 1 - self.active_pane();
        let Some(split) = &mut self.split else { return };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let doc = &mut self.docs[i];
        let opts = editor_view::Options { pane, active: false, word_wrap: opts.word_wrap && !doc.large, ..opts };
        editor_view::show(&mut child, doc, &mut split.other_view, opts, &deco, None);
        let pressed = ui.input(|i| i.pointer.any_pressed());
        if pressed && ui.rect_contains_pointer(rect) {
            self.focus_other_pane();
        }
    }

    /// Follows a tab being dragged: moves it among the tabs, and on release over
    /// the right half of the editor opens it beside the current one.
    pub(crate) fn drag_tab(&mut self, ui: &egui::Ui, uid: u64, tabs: &[Rect], bar: Rect) {
        let Some(p) = ui.input(|i| i.pointer.latest_pos()) else { return };
        if !ui.input(|i| i.pointer.primary_down()) {
            self.tab_drag = None;
            if p.y > bar.bottom() && p.x > bar.center().x && p.x < bar.right() {
                self.open_beside(uid);
            }
            return;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let Some(from) = self.docs.iter().position(|d| d.uid == uid) else { return };
        self.paint_tab_drag(ui, from, bar, p);
        if !bar.y_range().contains(p.y) {
            return;
        }
        // The tab swaps with a neighbour once its edge passes that neighbour's middle,
        // so the neighbour is pushed aside before the pointer gets there.
        let Some(width) = tabs.get(from).map(|r| r.width()) else { return };
        let left = p.x - self.tab_grab;
        let to = if tabs.get(from + 1).is_some_and(|r| left + width > r.center().x) {
            from + 1
        } else if from > 0 && tabs.get(from - 1).is_some_and(|r| left < r.center().x) {
            from - 1
        } else {
            return;
        };
        {
            let active = self.docs.get(self.active).map(|d| d.uid);
            let doc = self.docs.remove(from);
            self.docs.insert(to, doc);
            self.active = self.docs.iter().position(|d| Some(d.uid) == active).unwrap_or(to);
        }
    }

    /// What a tab drag will do, beyond the tab sliding along the bar: over the right half of
    /// the editor a band says it will open beside the current file, and off the bar a card
    /// follows the pointer.
    fn paint_tab_drag(&self, ui: &egui::Ui, from: usize, bar: Rect, p: egui::Pos2) {
        let pal = crate::ui::theme::colors();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("tab_drag")));
        // Beside the current file, when let go here.
        let content = ui.ctx().content_rect();
        let beside = p.y > bar.bottom() && p.x > bar.center().x && p.x < bar.right();
        if beside {
            let zone = Rect::from_min_max(
                egui::pos2(bar.center().x + 4.0, bar.bottom() + 4.0),
                egui::pos2(bar.right() - 4.0, content.bottom() - 60.0),
            );
            painter.rect_filled(zone, 12.0, pal.accent.gamma_multiply(0.10));
            painter.rect_stroke(zone, 12.0, egui::Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
            painter.text(
                zone.center() - egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_CENTER,
                "Open beside",
                egui::FontId::proportional(15.0),
                pal.text,
            );
        }
        if bar.y_range().contains(p.y) {
            return;
        }
        // Off the bar, the tab as a card under the pointer.
        let title = self.docs.get(from).map(|d| d.title()).unwrap_or_default();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let galley = painter.layout_no_wrap(title, font, pal.text);
        let size = egui::vec2(galley.size().x + 24.0, galley.size().y + 12.0);
        let ghost = Rect::from_min_size(p + egui::vec2(10.0, 10.0), size);
        painter.rect_filled(ghost, 9.0, pal.sidebar_bg.gamma_multiply(0.95));
        painter.rect_stroke(ghost, 9.0, egui::Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
        painter.galley(ghost.center() - galley.size() / 2.0, galley, pal.text);
    }
}
