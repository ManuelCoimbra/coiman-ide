//! The buttons beside the tabs for what the active file can do, and the tree they can open.

use coiman_core::buffer::minimal_edits;
use coiman_core::commands::Command;
use coiman_core::data_tree::{self, DataNode};
use coiman_core::file_actions::{self, FileAction};
use egui::{RichText, Sense};

use super::CodeApp;
use crate::theme;

/// Room a button takes around its label.
const PADDING: f32 = 16.0;
const GAP: f32 = 2.0;

#[derive(Default)]
pub(super) struct DataTree {
    pub(super) open: bool,
    /// (doc uid, version, what's in it).
    parsed: Option<(u64, u64, Vec<DataNode>)>,
}

impl CodeApp {
    /// What the active file can do here.
    fn file_buttons(&self) -> Vec<FileAction> {
        let Some(doc) = self.docs.get(self.active) else { return Vec::new() };
        let Some(path) = &doc.path else { return Vec::new() };
        let root = self.explorer.root.as_deref();
        let relative = root.and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
        let mut actions = file_actions::for_file(&relative.to_string_lossy());
        actions.retain(|a| match a.command {
            Command::ToggleCoverage => self.has_coverage(),
            _ => true,
        });
        actions
    }

    fn file_button_on(&self, command: Command) -> bool {
        match command {
            Command::MarkdownPreview => self.md_preview.open,
            Command::ToggleTableView => self.docs.get(self.active).is_some_and(|d| self.table_showing(d.uid)),
            Command::ToggleSvgPreview => self.svg_preview_open(),
            Command::ToggleDataTree => self.data_tree.open,
            _ => false,
        }
    }

    /// Draws the buttons at the right of a tab bar `height` tall, and returns the width they took.
    pub(super) fn file_buttons_ui(&mut self, ui: &mut egui::Ui, height: f32) -> f32 {
        let actions = self.file_buttons();
        if actions.is_empty() {
            return 0.0;
        }
        let pal = theme::colors();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let width: f32 = actions
            .iter()
            .map(|a| ui.painter().layout_no_wrap(a.label.to_owned(), font.clone(), pal.text).size().x + PADDING + GAP)
            .sum();
        let top = ui.cursor().top();
        let right = ui.max_rect().right();
        let mut x = right - width - 6.0;
        let mut chosen = None;
        for button in &actions {
            let on = self.file_button_on(button.command);
            let color = if on { pal.tab_active_fg } else { pal.text_dim };
            let galley = ui.painter().layout_no_wrap(button.label.to_owned(), font.clone(), color);
            let size = egui::vec2(galley.size().x + PADDING, (height - 10.0).max(18.0));
            let rect = egui::Rect::from_min_size(egui::pos2(x, top + (height - size.y) / 2.0), size);
            let response = ui.interact(rect, ui.scope_id().with(button.label), Sense::click());
            let fill = if on { pal.selection } else { pal.hover_bg };
            let shown = theme::eased(ui.ctx(), response.id.with("shown"), on || response.hovered());
            if shown > 0.0 {
                ui.painter().rect_filled(rect, 5.0, fill.gamma_multiply(shown));
            }
            let at = rect.center() - galley.size() / 2.0;
            ui.painter().galley(at, galley, color);
            if response.on_hover_text(button.tip).clicked() {
                chosen = Some(button.command);
            }
            x += size.x + GAP;
        }
        if let Some(command) = chosen {
            let ctx = ui.ctx().clone();
            self.run_command(&ctx, command);
        }
        width + 10.0
    }

    /// Tidies a JSON file here; other files need their language server.
    pub(super) fn format_json_active(&mut self) -> bool {
        let Some(doc) = self.docs.get_mut(self.active) else { return false };
        if !matches!(doc.extension().as_str(), "json") {
            return false;
        }
        let text = doc.text();
        match data_tree::format_json(&text) {
            Ok(tidy) => {
                let tidy = if text.contains("\r\n") { tidy.replace('\n', "\r\n") } else { tidy };
                doc.buffer.apply_edits(minimal_edits(&text, &tidy));
            }
            Err(why) => self.status = Some(why),
        }
        true
    }

    /// The tree beside the text, for JSON, YAML and TOML.
    pub(super) fn data_tree_ui(&mut self, ui: &mut egui::Ui) {
        if !self.data_tree.open {
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let (uid, version, extension) = (doc.uid, doc.buffer.version(), doc.extension());
        if !matches!(extension.as_str(), "json" | "jsonc" | "yaml" | "yml" | "toml") {
            return;
        }
        if self.data_tree.parsed.as_ref().is_none_or(|p| (p.0, p.1) != (uid, version)) {
            let nodes =
                if doc.large { Vec::new() } else { data_tree::parse(&extension, &doc.text()).unwrap_or_default() };
            self.data_tree.parsed = Some((uid, version, nodes));
        }
        let large = doc.large;
        let half = ui.available_width() / 2.0;
        let mut jump = None;
        let nodes = self.data_tree.parsed.as_ref().map_or(&[][..], |p| p.2.as_slice());
        egui::Panel::right("data_tree")
            .resizable(true)
            .default_size(half.min(420.0))
            .size_range(160.0..=(half * 2.0 - 200.0).max(200.0))
            .frame(egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(10.0))
            .show(ui, |ui| {
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    if large {
                        ui.label("This file is too large to show as a tree.");
                    } else if nodes.is_empty() {
                        ui.label(RichText::new("Nothing to show.").color(theme::colors().text_dim));
                    }
                    node_rows(ui, nodes, 0, &mut jump);
                });
            });
        if let (Some(line), Some(doc)) = (jump, self.docs.get_mut(self.active)) {
            doc.goto_line(line + 1);
        }
    }
}

fn node_rows(ui: &mut egui::Ui, nodes: &[DataNode], depth: usize, jump: &mut Option<usize>) {
    let pal = theme::colors();
    for (i, node) in nodes.iter().enumerate() {
        if node.children.is_empty() {
            let mut job = egui::text::LayoutJob::default();
            let font = egui::TextStyle::Body.resolve(ui.style());
            let format = |color| egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
            job.append(&node.key, 0.0, format(pal.text));
            job.append(&node.value, 12.0, format(pal.text_dim));
            let row = ui.add(egui::Label::new(job).sense(Sense::click()).selectable(false));
            if row.on_hover_text(format!("Line {}", node.line + 1)).clicked() {
                *jump = Some(node.line);
            }
        } else {
            let title = format!("{}  ({})", node.key, node.children.len());
            let id = ui.scope_id().with((depth, i, &node.key));
            let header = egui::CollapsingHeader::new(title).id_salt(id).default_open(depth < 2).show(ui, |ui| {
                node_rows(ui, &node.children, depth + 1, jump);
            });
            if header.header_response.secondary_clicked() {
                *jump = Some(node.line);
            }
        }
    }
}
