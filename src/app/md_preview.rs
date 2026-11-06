//! Markdown preview beside the editor, following the text as it is typed.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Align, FontId, Sense, Stroke, text::LayoutJob};
use rust_code_core::markdown::{self, Align as Cell, Block, Inline, Item, Style};

use super::{CodeApp, repaint};
use crate::theme;

/// Pictures larger than this aren't read.
const MAX_PICTURE_BYTES: u64 = 64 * 1024 * 1024;
/// Code blocks remembered with their colors.
const MAX_CODE_CACHE: usize = 400;

enum Picture {
    /// The picture, and the size it's meant to be shown at.
    Loading(Receiver<Result<(egui::ColorImage, egui::Vec2), String>>),
    Ready(egui::TextureHandle, egui::Vec2),
    Failed(String),
}

#[derive(Default)]
pub(super) struct MdPreview {
    pub(super) open: bool,
    /// Blocks of this version of this document.
    parsed: Option<(u64, u64, Vec<Block>)>,
    pictures: HashMap<PathBuf, Picture>,
    /// Colors of code blocks, by a hash of their language and text.
    code: HashMap<u64, Option<Vec<Vec<rust_code_core::highlight::Span>>>>,
    /// A heading to bring into view, by its anchor.
    scroll_to: Option<String>,
}

fn is_markdown(doc: &crate::document::Document) -> bool {
    matches!(doc.extension().as_str(), "md" | "markdown")
}

/// What a frame of the preview needs to know, and what was clicked in it.
struct Draw<'a> {
    size: f32,
    /// The Markdown file's folder, which relative paths start from.
    dir: Option<&'a Path>,
    pictures: &'a mut HashMap<PathBuf, Picture>,
    code: &'a mut HashMap<u64, Option<Vec<Vec<rust_code_core::highlight::Span>>>>,
    scroll_to: &'a mut Option<String>,
    clicked: Option<String>,
}

/// Text with its links, as char ranges and where they lead.
fn layout(ui: &egui::Ui, inlines: &[Inline], size: f32, heading: bool) -> (LayoutJob, Vec<(usize, usize, String)>) {
    let pal = theme::colors();
    let strong = ui.visuals().strong_text_color();
    let mut job = LayoutJob::default();
    let mut links = Vec::new();
    let mut chars = 0;
    for inline in inlines {
        let (text, style) = match inline {
            Inline::Text(text, style) => (text.as_str(), style.clone()),
            Inline::Break => ("\n", Style::default()),
            // Pictures are drawn between runs of text; one inside a table cell shows its alt text.
            Inline::Image { alt, link, .. } => (alt.as_str(), Style { link: link.clone(), ..Style::default() }),
        };
        let mut format = egui::TextFormat {
            font_id: FontId::proportional(size),
            color: if heading || style.bold { strong } else { pal.text },
            italics: style.italic,
            ..Default::default()
        };
        if style.code {
            format.font_id = FontId::monospace(size * 0.9);
            format.background = pal.hover_bg;
        }
        if style.strike {
            format.strikethrough = Stroke::new(1.0, format.color);
        }
        if let Some(url) = &style.link {
            format.color = pal.accent;
            format.underline = Stroke::new(1.0, pal.accent);
            let count = text.chars().count();
            links.push((chars, chars + count, url.clone()));
        }
        chars += text.chars().count();
        job.append(text, 0.0, format);
    }
    (job, links)
}

/// Paints laid-out text at `pos`. Says which link was clicked, and shows where each leads.
fn paint_text(
    ui: &mut egui::Ui,
    pos: egui::Pos2,
    galley: std::sync::Arc<egui::Galley>,
    links: &[(usize, usize, String)],
    id: egui::Id,
) -> Option<String> {
    let rect = egui::Rect::from_min_size(pos, galley.size());
    let response = ui.interact(rect, id, if links.is_empty() { Sense::hover() } else { Sense::click() });
    let mut under = None;
    if !links.is_empty()
        && let Some(pointer) = response.hover_pos()
    {
        let at = galley.cursor_from_pos(pointer - rect.min).index.0;
        under = links.iter().find(|(start, end, _)| (*start..*end).contains(&at)).map(|l| l.2.clone());
    }
    ui.painter().galley(rect.min, galley, theme::colors().text);
    if let Some(url) = &under {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        response.clone().on_hover_text(url.as_str());
        if response.clicked() {
            return under;
        }
    }
    None
}

/// Text that wraps to the width it's given.
fn paragraph(ui: &mut egui::Ui, d: &mut Draw, inlines: &[Inline], size: f32, heading: bool) -> egui::Response {
    let (mut job, links) = layout(ui, inlines, size, heading);
    job.wrap.max_width = ui.available_width();
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    if let Some(url) = paint_text(ui, rect.min, galley, &links, response.id.with("text")) {
        d.clicked = Some(url);
    }
    response
}

/// Runs of text with the pictures between them, each picture on its own line.
fn inlines(ui: &mut egui::Ui, d: &mut Draw, all: &[Inline]) {
    let mut run = Vec::new();
    for inline in all {
        match inline {
            Inline::Image { alt, url, link } => {
                if !run.is_empty() {
                    paragraph(ui, d, &std::mem::take(&mut run), d.size, false);
                }
                picture(ui, d, alt, url, link.as_deref());
            }
            other => run.push(other.clone()),
        }
    }
    if !run.is_empty() {
        paragraph(ui, d, &run, d.size, false);
    }
}

fn is_remote(url: &str) -> bool {
    ["http://", "https://", "mailto:", "ftp://"].iter().any(|p| url.to_ascii_lowercase().starts_with(p))
}

/// A path written in a link, with `%20` and the like decoded and any `#part` dropped.
fn local_path(dir: Option<&Path>, url: &str) -> Option<PathBuf> {
    let url = url.split(['#', '?']).next().unwrap_or("");
    if url.is_empty() || url.starts_with("data:") {
        return None;
    }
    let mut bytes = Vec::new();
    let raw = url.trim_start_matches("file://").as_bytes();
    let mut i = 0;
    while i < raw.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if raw[i] == b'%'
            && let (Some(Some(a)), Some(Some(b))) = (raw.get(i + 1).map(|&b| hex(b)), raw.get(i + 2).map(|&b| hex(b)))
        {
            bytes.push((a * 16 + b) as u8);
            i += 3;
            continue;
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&bytes).into_owned());
    let path = if path.is_absolute() { path } else { dir?.join(path) };
    // One kind of separator, the system's.
    Some(path.components().collect())
}

fn load_picture(ctx: &egui::Context, path: PathBuf) -> Picture {
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_PICTURE_BYTES) {
        return Picture::Failed("too large to show".to_owned());
    }
    let (tx, rx) = mpsc::channel();
    let wake = repaint(ctx);
    std::thread::spawn(move || {
        let svg = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        let image = if svg {
            std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| super::svg_preview::draw(&text))
                // Drawn larger than it's meant to be, for sharpness.
                .map(|(size, pixels, [w, h])| {
                    (egui::ColorImage::from_rgba_premultiplied(size, &pixels), egui::vec2(w, h))
                })
        } else {
            super::image_view::decode(&path).map(|d| {
                let natural = egui::vec2(d.size[0] as f32, d.size[1] as f32);
                (egui::ColorImage::from_rgba_unmultiplied(d.size, &d.rgba), natural)
            })
        };
        let _ = tx.send(image);
        wake();
    });
    Picture::Loading(rx)
}

fn picture(ui: &mut egui::Ui, d: &mut Draw, alt: &str, url: &str, link: Option<&str>) {
    let pal = theme::colors();
    let small = FontId::proportional(d.size * 0.9);
    let note = |ui: &mut egui::Ui, d: &mut Draw, text: String, target: Option<String>| {
        let label = egui::RichText::new(text).font(small.clone()).color(pal.text_dim);
        let response = ui.add(egui::Label::new(label).sense(Sense::click()));
        if let Some(target) = target {
            let response = response.on_hover_text(target.as_str());
            if response.clicked() {
                d.clicked = Some(target);
            }
        }
    };
    let name = if alt.is_empty() { "picture" } else { alt };
    if is_remote(url) {
        // The preview fetches nothing from the web.
        note(ui, d, format!("[{name}: a picture on the web, not downloaded]"), Some(url.to_owned()));
        return;
    }
    let Some(path) = local_path(d.dir, url) else {
        note(ui, d, format!("[{name}]"), None);
        return;
    };
    let ctx = ui.ctx().clone();
    let entry = d.pictures.entry(path.clone()).or_insert_with(|| load_picture(&ctx, path.clone()));
    if let Picture::Loading(rx) = entry
        && let Ok(result) = rx.try_recv()
    {
        *entry = match result {
            Ok((image, size)) => {
                let texture = ctx.load_texture(path.display().to_string(), image, egui::TextureOptions::LINEAR);
                Picture::Ready(texture, size)
            }
            Err(why) => Picture::Failed(why),
        };
    }
    match entry {
        Picture::Loading(_) => note(ui, d, format!("[{name}: loading…]"), None),
        Picture::Failed(why) => {
            let why = if path.exists() { why.clone() } else { "no such file".to_owned() };
            note(ui, d, format!("[{name}: {why}]"), Some(path.display().to_string()));
        }
        Picture::Ready(texture, natural) => {
            let width = natural.x.min(ui.available_width());
            let size = egui::vec2(width, natural.y * width / natural.x.max(1.0));
            let sense = if link.is_some() { Sense::click() } else { Sense::hover() };
            let response = ui.add(egui::Image::new((texture.id(), size)).sense(sense));
            let response = if alt.is_empty() { response } else { response.on_hover_text(alt) };
            if let Some(link) = link
                && response.clicked()
            {
                d.clicked = Some(link.to_owned());
            }
        }
    }
}

fn code_block(ui: &mut egui::Ui, d: &mut Draw, lang: &str, text: &str, salt: usize) {
    let pal = theme::colors();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (lang, text, theme_generation()).hash(&mut hasher);
    let key = hasher.finish();
    if d.code.len() > MAX_CODE_CACHE {
        d.code.clear();
    }
    let spans = d
        .code
        .entry(key)
        .or_insert_with(|| (!lang.is_empty()).then(|| rust_code_core::highlight::snippet(lang, text)).flatten());
    let font = FontId::monospace(d.size * 0.92);
    let mut job = LayoutJob::default();
    for (n, line) in text.split('\n').enumerate() {
        if n > 0 {
            job.append("\n", 0.0, egui::TextFormat::simple(font.clone(), pal.text));
        }
        let colored = spans.as_ref().and_then(|s| s.get(n));
        let mut rest = line.chars();
        for span in colored.into_iter().flatten() {
            let piece: String = rest.by_ref().take(span.len).collect();
            let format = egui::TextFormat {
                italics: span.italic,
                ..egui::TextFormat::simple(font.clone(), theme::c32(span.color))
            };
            job.append(&piece, 0.0, format);
        }
        job.append(&rest.collect::<String>(), 0.0, egui::TextFormat::simple(font.clone(), pal.text));
    }
    egui::Frame::new().fill(pal.hover_bg).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        if !lang.is_empty() {
            ui.label(egui::RichText::new(lang).font(FontId::proportional(d.size * 0.75)).color(pal.text_dim));
        }
        egui::ScrollArea::horizontal().id_salt(("md_code", salt)).show(ui, |ui| {
            ui.add(egui::Label::new(job).extend());
        });
    });
}

fn theme_generation() -> u64 {
    rust_code_core::theme::generation()
}

/// Columns as wide as their text when it fits, else shared out by how much each wants,
/// never narrower than its longest word.
fn column_widths(natural: &[f32], least: &[f32], room: f32) -> Vec<f32> {
    let total: f32 = natural.iter().sum();
    if total <= room {
        return natural.to_vec();
    }
    let floor: f32 = least.iter().sum();
    if floor >= room {
        return least.to_vec();
    }
    let want: f32 = natural.iter().zip(least).map(|(n, l)| n - l).sum::<f32>().max(1.0);
    natural.iter().zip(least).map(|(n, l)| l + (n - l) / want * (room - floor)).collect()
}

fn table(
    ui: &mut egui::Ui,
    d: &mut Draw,
    align: &[Cell],
    header: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
    salt: usize,
) {
    const PAD: f32 = 8.0;
    let pal = theme::colors();
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return;
    }
    let lines: Vec<(bool, &[Vec<Inline>])> =
        std::iter::once((true, header)).chain(rows.iter().map(|r| (false, r.as_slice()))).collect();
    let (mut natural, mut least) = (vec![0.0f32; columns], vec![0.0f32; columns]);
    for (head, cells) in &lines {
        for (c, cell) in cells.iter().enumerate().take(columns) {
            let (job, _) = layout(ui, cell, d.size, *head);
            let wide = ui.fonts_mut(|f| f.layout_job(job)).size().x;
            // Never narrower than its longest word, so words aren't broken.
            let font = FontId::proportional(d.size);
            let word = markdown::plain(cell)
                .split_whitespace()
                .map(|w| ui.fonts_mut(|f| f.layout_no_wrap(w.to_owned(), font.clone(), egui::Color32::WHITE)).size().x)
                .fold(0.0, f32::max);
            natural[c] = natural[c].max(wide);
            least[c] = least[c].max(word.min(wide));
        }
    }
    let room = ui.available_width() - PAD * 2.0 * columns as f32;
    let widths = column_widths(&natural, &least, room.max(0.0));
    let total = widths.iter().sum::<f32>() + PAD * 2.0 * columns as f32;
    egui::ScrollArea::horizontal().id_salt(("md_table", salt)).show(ui, |ui| {
        for (r, (head, cells)) in lines.iter().enumerate() {
            let laid: Vec<_> = (0..columns)
                .map(|c| {
                    let empty = Vec::new();
                    let (mut job, links) = layout(ui, cells.get(c).unwrap_or(&empty), d.size, *head);
                    job.wrap.max_width = widths[c].max(1.0);
                    (ui.fonts_mut(|f| f.layout_job(job)), links)
                })
                .collect();
            let height = laid.iter().map(|(g, _)| g.size().y).fold(0.0, f32::max) + PAD;
            let (rect, response) = ui.allocate_exact_size(egui::vec2(total, height), Sense::hover());
            let fill = if *head { Some(pal.widget_bg) } else { (r % 2 == 0).then(|| pal.hover_bg.gamma_multiply(0.5)) };
            if let Some(fill) = fill {
                ui.painter().rect_filled(rect, 0.0, fill);
            }
            let mut x = rect.left();
            for (c, (galley, links)) in laid.into_iter().enumerate() {
                let cell_width = widths[c] + PAD * 2.0;
                let slack = widths[c] - galley.size().x;
                let shift = match align.get(c) {
                    Some(Cell::Right) => slack,
                    Some(Cell::Center) => slack / 2.0,
                    _ => 0.0,
                };
                let pos = egui::pos2(x + PAD + shift.max(0.0), rect.top() + PAD / 2.0);
                if let Some(url) = paint_text(ui, pos, galley, &links, response.id.with(c)) {
                    d.clicked = Some(url);
                }
                x += cell_width;
                if c + 1 < columns {
                    ui.painter().vline(x, rect.y_range(), Stroke::new(1.0, pal.border));
                }
            }
            ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, pal.border));
        }
    });
}

fn list(ui: &mut egui::Ui, d: &mut Draw, start: Option<u64>, items: &[Item], depth: usize, salt: usize) {
    let pal = theme::colors();
    for (n, item) in items.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let marker_width = d.size * 1.6;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(marker_width, d.size * 1.3), Sense::hover());
            let center = egui::pos2(rect.right() - d.size * 0.6, rect.center().y);
            match (item.task, start) {
                (Some(done), _) => {
                    let square = egui::Rect::from_center_size(center, egui::Vec2::splat(d.size * 0.8));
                    ui.painter().rect_stroke(square, 2.0, Stroke::new(1.2, pal.text_dim), egui::StrokeKind::Inside);
                    if done {
                        let tick = [
                            square.left_center() + egui::vec2(square.width() * 0.2, 0.0),
                            square.center_bottom() - egui::vec2(0.0, square.height() * 0.25),
                            square.right_top() + egui::vec2(-square.width() * 0.2, square.height() * 0.25),
                        ];
                        ui.painter().line(tick.to_vec(), Stroke::new(1.6, pal.accent));
                    }
                }
                (None, Some(first)) => {
                    let number = format!("{}.", first + n as u64);
                    let font = FontId::proportional(d.size);
                    let at = egui::pos2(rect.right() - 4.0, rect.center().y);
                    ui.painter().text(at, egui::Align2::RIGHT_CENTER, number, font, pal.text_dim);
                }
                (None, None) => {
                    let radius = d.size * 0.16;
                    match depth % 3 {
                        0 => {
                            ui.painter().circle_filled(center, radius, pal.text_dim);
                        }
                        1 => {
                            ui.painter().circle_stroke(center, radius, Stroke::new(1.0, pal.text_dim));
                        }
                        _ => {
                            let square = egui::Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0));
                            ui.painter().rect_filled(square, 0.0, pal.text_dim);
                        }
                    }
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                blocks(ui, d, &item.blocks, depth + 1, salt * 31 + n);
            });
        });
    }
}

fn blocks(ui: &mut egui::Ui, d: &mut Draw, all: &[Block], depth: usize, salt: usize) {
    let pal = theme::colors();
    for (i, block) in all.iter().enumerate() {
        let salt = salt * 131 + i;
        match block {
            Block::Heading { level, text, anchor } => {
                ui.add_space(if i == 0 { 0.0 } else { d.size * 0.6 });
                let scale = [2.0, 1.6, 1.3, 1.15, 1.05, 1.0][usize::from(level.saturating_sub(1)).min(5)];
                let response = paragraph(ui, d, text, d.size * scale, true);
                if d.scroll_to.as_deref() == Some(anchor.as_str()) {
                    response.scroll_to_me(Some(Align::TOP));
                    *d.scroll_to = None;
                }
                if *level <= 2 {
                    ui.separator();
                }
            }
            Block::Paragraph(text) => {
                inlines(ui, d, text);
                ui.add_space(4.0);
            }
            Block::List { start, items } => {
                list(ui, d, *start, items, depth, salt);
                ui.add_space(4.0);
            }
            Block::Quote(inner) => {
                let response =
                    egui::Frame::new().inner_margin(egui::Margin { left: 14, ..Default::default() }).show(ui, |ui| {
                        blocks(ui, d, inner, depth, salt);
                    });
                let r = response.response.rect;
                ui.painter().vline(r.left() + 3.0, r.y_range(), Stroke::new(3.0, pal.border));
                ui.add_space(4.0);
            }
            Block::Code { lang, text } => {
                code_block(ui, d, lang, text, salt);
                ui.add_space(6.0);
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Table { align, header, rows } => {
                table(ui, d, align, header, rows, salt);
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
        let dir = doc.path.as_deref().and_then(Path::parent).map(Path::to_path_buf);
        let size = self.settings.workbench.ui_font_size + 1.0;
        let half = ui.available_width() / 2.0;
        let parsed = self.md_preview.parsed.take();
        let preview = &mut self.md_preview;
        let mut d = Draw {
            size,
            dir: dir.as_deref(),
            pictures: &mut preview.pictures,
            code: &mut preview.code,
            scroll_to: &mut preview.scroll_to,
            clicked: None,
        };
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
                    let shown = parsed.as_ref().map_or(&[][..], |p| p.2.as_slice());
                    blocks(ui, &mut d, shown, 0, 0);
                });
            });
        let clicked = d.clicked.take();
        self.md_preview.parsed = parsed;
        if let Some(url) = clicked {
            self.follow_markdown_link(ui.ctx(), &url, dir.as_deref());
        }
    }

    /// A web address opens in the browser, `#part` scrolls the preview, and a file opens here.
    fn follow_markdown_link(&mut self, ctx: &egui::Context, url: &str, dir: Option<&Path>) {
        rust_code_core::applog::info(&format!("markdown link {url}"));
        if is_remote(url) {
            ctx.open_url(egui::OpenUrl::new_tab(url));
        } else if let Some(anchor) = url.strip_prefix('#') {
            self.md_preview.scroll_to = Some(anchor.to_lowercase());
        } else if let Some(path) = local_path(dir, url) {
            if path.is_file() {
                self.open_file(&path);
            } else if path.is_dir() {
                self.status = Some(format!("{} is a folder", path.display()));
            } else {
                self.status = Some(format!("No file at {}", path.display()));
            }
        } else {
            self.status = Some(format!("Can't open {url}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fit_or_share_the_room() {
        assert_eq!(column_widths(&[50.0, 100.0], &[20.0, 30.0], 400.0), [50.0, 100.0]);
        let shared = column_widths(&[100.0, 300.0], &[20.0, 40.0], 200.0);
        assert!((shared.iter().sum::<f32>() - 200.0).abs() < 0.01, "{shared:?}");
        assert!(shared[0] >= 20.0 && shared[1] > shared[0]);
        assert_eq!(column_widths(&[100.0, 300.0], &[80.0, 90.0], 100.0), [80.0, 90.0]);
    }

    #[test]
    fn link_paths_are_decoded_and_relative_to_the_file() {
        let dir = Path::new("/docs");
        assert_eq!(local_path(Some(dir), "a%20b.md#part"), Some(PathBuf::from("/docs/a b.md")));
        assert_eq!(local_path(Some(dir), "#only"), None);
        assert_eq!(local_path(None, "x.md"), None);
        assert_eq!(local_path(Some(dir), "data:image/png;base64,AAA"), None);
        assert!(is_remote("HTTPS://x"));
        assert!(!is_remote("img/a.png"));
    }
}
