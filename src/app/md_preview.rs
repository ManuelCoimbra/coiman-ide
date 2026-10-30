//! Markdown preview beside the editor, following the text as it is typed.

use eframe::egui::{self, RichText, Stroke};
use rust_code_core::markdown::{self, Block, Inline};

use super::CodeApp;
use crate::theme;

#[derive(Default)]
pub(super) struct MdPreview {
    pub(super) open: bool,
    /// Blocks of this version of this document.
    parsed: Option<(u64, u64, Vec<Block>)>,
}

fn is_markdown(doc: &crate::document::Document) -> bool {
    matches!(doc.extension().as_str(), "md" | "markdown")
}

fn spans(ui: &mut egui::Ui, inlines: &[Inline], size: f32, strong: bool) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for inline in inlines {
            let rich = |s: &str| {
                let t = RichText::new(s).size(size);
                if strong { t.strong() } else { t }
            };
            match inline {
                Inline::Text(t) => {
                    ui.label(rich(t));
                }
                Inline::Bold(t) => {
                    ui.label(rich(t).strong());
                }
                Inline::Italic(t) => {
                    ui.label(rich(t).italics());
                }
                Inline::Strike(t) => {
                    ui.label(rich(t).strikethrough());
                }
                Inline::Code(t) => {
                    ui.label(rich(t).monospace().background_color(theme::colors().hover_bg));
                }
                Inline::Link { text, url } => {
                    // Only addresses a browser should open; a local path is just text.
                    if ["http://", "https://", "mailto:"].iter().any(|p| url.starts_with(p)) {
                        ui.hyperlink_to(rich(text).color(theme::colors().accent).underline(), url).on_hover_text(url);
                    } else {
                        ui.label(rich(text).underline()).on_hover_text(url);
                    }
                }
            }
        }
    });
}

fn show_blocks(ui: &mut egui::Ui, blocks: &[Block], size: f32) {
    let pal = theme::colors();
    for (i, block) in blocks.iter().enumerate() {
        match block {
            Block::Heading(level, text) => {
                ui.add_space(if i == 0 { 0.0 } else { 10.0 });
                let scale = [2.0, 1.6, 1.3, 1.15, 1.05, 1.0][usize::from(*level - 1).min(5)];
                spans(ui, text, size * scale, true);
                if *level <= 2 {
                    ui.separator();
                }
            }
            Block::Paragraph(text) => {
                spans(ui, text, size, false);
                ui.add_space(6.0);
            }
            Block::Item { depth, marker, text } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(*depth as f32 * 18.0 + 4.0);
                    ui.label(RichText::new(marker).size(size).color(pal.text_dim));
                    ui.vertical(|ui| spans(ui, text, size, false));
                });
            }
            Block::Quote(text) => {
                ui.add_space(2.0);
                let response =
                    egui::Frame::new().inner_margin(egui::Margin { left: 12, ..Default::default() }).show(ui, |ui| {
                        spans(ui, text, size, false);
                    });
                let r = response.response.rect;
                ui.painter().vline(r.left() + 3.0, r.y_range(), Stroke::new(3.0, pal.text_dim));
                ui.add_space(6.0);
            }
            Block::Code { text, .. } => {
                egui::Frame::new().fill(pal.hover_bg).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::horizontal().id_salt(("md_code", i)).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(text).monospace().size(size - 1.0)).extend());
                    });
                });
                ui.add_space(6.0);
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Table { header, rows } => {
                egui::Grid::new(("md_table", i)).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                    for cell in header {
                        spans(ui, cell, size, true);
                    }
                    ui.end_row();
                    for row in rows {
                        for cell in row {
                            spans(ui, cell, size, false);
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }
        }
    }
}

impl CodeApp {
    pub(super) fn toggle_markdown_preview(&mut self) {
        if self.docs.get(self.active).is_some_and(is_markdown) {
            self.md_preview.open = !self.md_preview.open;
        } else {
            self.status = Some("Open a Markdown file to preview it".to_owned());
        }
    }

    pub(super) fn active_is_markdown(&self) -> bool {
        self.docs.get(self.active).is_some_and(is_markdown)
    }

    /// The preview panel, to the right of the editor, for a Markdown file.
    pub(super) fn markdown_preview(&mut self, ui: &mut egui::Ui) {
        if !self.md_preview.open || !self.active_is_markdown() {
            return;
        }
        let doc = &self.docs[self.active];
        let (uid, version) = (doc.uid, doc.buffer.version());
        if self.md_preview.parsed.as_ref().is_none_or(|p| (p.0, p.1) != (uid, version)) {
            let blocks = if doc.large { Vec::new() } else { markdown::parse(&doc.text()) };
            self.md_preview.parsed = Some((uid, version, blocks));
        }
        let large = doc.large;
        let size = self.settings.workbench.ui_font_size + 1.0;
        let half = ui.available_width() / 2.0;
        let blocks = self.md_preview.parsed.as_ref().map_or(&[][..], |p| p.2.as_slice());
        egui::Panel::right("markdown_preview")
            .resizable(true)
            .default_size(half)
            .size_range(200.0..=(half * 2.0 - 200.0).max(200.0))
            .frame(egui::Frame::new().fill(theme::colors().editor_bg).inner_margin(14.0))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    if large {
                        ui.label("This file is too large to preview.");
                    }
                    show_blocks(ui, blocks, size);
                });
            });
    }
}
