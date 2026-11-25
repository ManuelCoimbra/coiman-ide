//! The Extensions view: a VS Code-style marketplace sidebar and a dedicated
//! editor tab for each extension. Search and downloads go to open-vsx.org with
//! curl; icons and README pictures are fetched in the background and cached.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};

use coiman_core::extensions::open_vsx::{self, Detail, ExtensionInstalled, Found};
use coiman_core::extensions::vscode_compat::{self, Report};
use coiman_core::language::lsp::companions;
use coiman_core::language::markdown::{Block, Inline};
use egui::RichText;

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Details,
    Contributions,
    Changelog,
    Pack,
    Runtime,
}

enum IconState {
    Loading(Receiver<Vec<u8>>),
    Ready(egui::TextureHandle),
    Failed,
}

#[derive(Default)]
pub(crate) struct MarketplaceUi {
    query: String,
    searching: Option<Receiver<Result<Vec<Found>, String>>>,
    results: Option<Result<Vec<Found>, String>>,
    pub(crate) detail_tab: Option<Found>,
    detail: Option<Receiver<Result<Detail, String>>>,
    detail_page: Option<Result<Detail, String>>,
    page: Page,
    installing: Option<(String, Receiver<Result<ExtensionInstalled, String>>)>,
    done: Option<(String, Result<ExtensionInstalled, String>)>,
    installed: Option<Vec<Report>>,
    confirm: Option<Found>,
    icons: HashMap<String, IconState>,
}

enum Ask {
    Search,
    Detail(Box<Found>),
    Install(Box<Found>),
    Uninstall(String),
}

#[derive(Clone)]
struct Card {
    found: Found,
    installed: bool,
    summary: String,
}

impl Card {
    fn from_found(found: Found, installed: &[Report]) -> Self {
        let installed = installed.iter().any(|r| r.id == found.id());
        Self { found, installed, summary: String::new() }
    }

    fn from_report(report: &Report) -> Self {
        let (namespace, name) = report.id.split_once('.').unwrap_or(("", ""));
        let found = Found {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            display_name: report.name.clone(),
            description: String::new(),
            version: report.version.clone(),
            downloads: 0,
            verified: false,
            download: String::new(),
            icon: String::new(),
            rating: 0,
            categories: Vec::new(),
            url: String::new(),
            timestamp: String::new(),
        };
        Self { found, installed: true, summary: report.summary() }
    }
}

fn short_count(n: u64) -> String {
    match n {
        0 => String::new(),
        1..=999 => n.to_string(),
        1_000..=999_999 => format!("{}K", n / 1_000),
        1_000_000..=999_999_999 => format!("{}M", n / 1_000_000),
        _ => format!("{}B", n / 1_000_000_000),
    }
}

fn publisher_text(found: &Found) -> String {
    found
        .url
        .strip_prefix("https://open-vsx.org/api/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(&found.namespace)
        .to_owned()
}

/// A README picture is often relative to the extension's file folder.
fn image_url(detail: &Detail, url: &str) -> String {
    if url.starts_with("https://") || url.starts_with("http://") {
        return url.to_owned();
    }
    let Some(base) = detail.download.split("/file/").next() else { return String::new() };
    format!("{base}/file/{}", url.trim_start_matches("./"))
}

impl MarketplaceUi {
    fn icon(&mut self, ctx: &egui::Context, url: &str) -> Option<egui::TextureHandle> {
        if url.is_empty() || !url.starts_with("https://open-vsx.org/") {
            return None;
        }
        let state = self.icons.entry(url.to_owned()).or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let url = url.to_owned();
            let wake = repaint(ctx);
            std::thread::spawn(move || {
                let _ = tx.send(open_vsx::fetch_image(&url).unwrap_or_default());
                wake();
            });
            IconState::Loading(rx)
        });
        let bytes = match state {
            IconState::Ready(texture) => return Some(texture.clone()),
            IconState::Failed => return None,
            IconState::Loading(rx) => match rx.try_recv() {
                Ok(bytes) => bytes,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => Vec::new(),
            },
        };
        let texture = if bytes.is_empty() {
            *self.icons.get_mut(url).unwrap() = IconState::Failed;
            return None;
        } else {
            image::load_from_memory(&bytes).ok().map(|image| {
                let rgba = image.to_rgba8();
                let (w, h) = rgba.dimensions();
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
                ctx.load_texture(url, image, egui::TextureOptions::LINEAR)
            })
        };
        match texture {
            Some(texture) => {
                *self.icons.get_mut(url).unwrap() = IconState::Ready(texture.clone());
                Some(texture)
            }
            None => {
                *self.icons.get_mut(url).unwrap() = IconState::Failed;
                None
            }
        }
    }

    fn card(&mut self, ui: &mut egui::Ui, card: &Card, selected: bool, ask: &mut Option<Ask>, pal: &theme::Palette) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 76.0), egui::Sense::hover());
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(2.0, 1.0)),
            4.0,
            if selected { pal.selection } else { pal.sidebar_bg },
        );
        if selected {
            ui.painter().rect_stroke(
                rect.shrink2(egui::vec2(2.0, 1.0)),
                4.0,
                egui::Stroke::new(1.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
        let center = rect.left_center() + egui::vec2(30.0, 0.0);
        // Icons only for the cards in view: the rest are fetched when scrolled to.
        let texture = if ui.is_rect_visible(rect) { self.icon(ui.ctx(), &card.found.icon) } else { None };
        if let Some(texture) = texture {
            egui::Image::from_texture(&texture)
                .paint_at(ui, egui::Rect::from_center_size(center, egui::vec2(36.0, 36.0)));
        } else {
            let letter = card.found.display_name.chars().next().unwrap_or('?').to_ascii_uppercase().to_string();
            ui.painter().circle_filled(center, 18.0, pal.accent.gamma_multiply(0.22));
            ui.painter().circle_stroke(center, 18.0, egui::Stroke::new(1.0, pal.accent.gamma_multiply(0.5)));
            ui.painter().text(
                center,
                egui::Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(18.0),
                pal.accent,
            );
        }
        let text_x = rect.left() + 62.0;
        // Names and descriptions use nearly the whole card; only the publisher
        // shares the bottom row with counts and the Install button.
        let clip = egui::Rect::from_min_max(
            egui::pos2(text_x, rect.top() + 4.0),
            egui::pos2((rect.right() - 30.0).max(text_x + 20.0), rect.top() + 46.0),
        );
        let pub_clip = egui::Rect::from_min_max(
            egui::pos2(text_x, rect.bottom() - 22.0),
            egui::pos2((rect.right() - 120.0).max(text_x + 20.0), rect.bottom()),
        );
        let painter = ui.painter().with_clip_rect(clip);
        let title = if card.found.display_name.is_empty() { card.found.id() } else { card.found.display_name.clone() };
        let title_font = egui::FontId::proportional(12.5);
        painter.text(
            egui::pos2(text_x, rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            title.clone(),
            title_font.clone(),
            pal.text,
        );
        let title_width = ui.painter().layout_no_wrap(title, title_font, pal.text).rect.width();
        painter.text(
            egui::pos2(text_x + title_width + 5.0, rect.top() + 9.0),
            egui::Align2::LEFT_TOP,
            format!("v{}", card.found.version),
            egui::FontId::proportional(10.0),
            pal.text_dim,
        );
        let desc = if card.summary.is_empty() { &card.found.description } else { &card.summary };
        painter.text(
            egui::pos2(text_x, rect.top() + 27.0),
            egui::Align2::LEFT_TOP,
            desc.trim(),
            egui::FontId::proportional(10.5),
            pal.text_dim,
        );
        let mut publisher = publisher_text(&card.found);
        if card.found.verified {
            publisher.insert_str(0, "✓ ");
        }
        ui.painter().with_clip_rect(pub_clip).text(
            egui::pos2(text_x, rect.bottom() - 16.0),
            egui::Align2::LEFT_TOP,
            publisher,
            egui::FontId::proportional(10.0),
            pal.text_dim,
        );
        let stats = format!("♧ {}  ★ {}", short_count(card.found.downloads), card.found.rating as f64 / 10.0);
        ui.painter().text(
            rect.right_bottom() - egui::vec2(60.0, 8.0),
            egui::Align2::RIGHT_BOTTOM,
            stats,
            egui::FontId::proportional(10.0),
            pal.text_dim,
        );
        let button = egui::Rect::from_min_size(rect.right_bottom() - egui::vec2(56.0, 26.0), egui::vec2(46.0, 18.0));
        let resp = ui.interact(button, ui.scope_id().with(("card_button", &card.found.id())), egui::Sense::click());
        let label = if card.installed { "Uninstall" } else { "Install" };
        ui.painter().rect_filled(button, 3.0, if resp.hovered() { pal.accent } else { pal.widget_bg });
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.5),
            if resp.hovered() { pal.text } else { pal.text_dim },
        );
        if resp.clicked() {
            *ask = Some(if card.installed {
                Ask::Uninstall(card.found.id())
            } else {
                Ask::Install(Box::new(card.found.clone()))
            });
            return;
        }
        let body = egui::Rect::from_min_max(rect.left_top(), rect.right_bottom() - egui::vec2(0.0, 24.0));
        if ui.interact(body, ui.scope_id().with(("card", &card.found.id())), egui::Sense::click()).clicked() {
            *ask = Some(Ask::Detail(Box::new(card.found.clone())));
        }
    }

    fn inline(&mut self, ui: &mut egui::Ui, inlines: &[Inline], detail: &Detail, size: f32) {
        ui.horizontal_wrapped(|ui| {
            for inline in inlines {
                match inline {
                    Inline::Text(text, style) => {
                        let font =
                            if style.code { egui::FontId::monospace(size) } else { egui::FontId::proportional(size) };
                        let color = if style.code { theme::colors().accent } else { theme::colors().text };
                        let mut text = RichText::new(text.clone()).font(font).color(color);
                        if style.bold {
                            text = text.strong();
                        }
                        if style.italic {
                            text = text.italics();
                        }
                        if style.strike {
                            text = text.strikethrough();
                        }
                        ui.add(egui::Label::new(text).wrap());
                    }
                    Inline::Image { alt, url, .. } => {
                        let resolved = image_url(detail, url);
                        if let Some(texture) = self.icon(ui.ctx(), &resolved) {
                            ui.add(egui::Image::from_texture(&texture).max_width(ui.available_width().min(520.0)));
                        } else if !alt.is_empty() {
                            ui.label(RichText::new(alt.clone()).small().color(theme::colors().text_dim));
                        }
                    }
                    Inline::Break => {
                        ui.add_space(4.0);
                    }
                }
            }
        });
    }

    fn readme(&mut self, ui: &mut egui::Ui, detail: &Detail) {
        let pal = theme::colors();
        for block in coiman_core::language::markdown::parse(&detail.readme) {
            match block {
                Block::Heading { level, text, .. } => {
                    let size = match level {
                        1 => 25.0,
                        2 => 20.0,
                        3 => 16.0,
                        _ => 13.0,
                    };
                    self.inline(ui, &text, detail, size);
                    if level == 1 {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(10.0);
                    } else {
                        ui.add_space(6.0);
                    }
                }
                Block::Paragraph(text) => {
                    self.inline(ui, &text, detail, 12.0);
                    ui.add_space(10.0);
                }
                Block::Code { text, .. } => {
                    egui::Frame::new()
                        .fill(pal.editor_bg)
                        .stroke(egui::Stroke::new(1.0, pal.border))
                        .corner_radius(4.0)
                        .inner_margin(egui::Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(text).monospace().size(11.0)).wrap());
                        });
                    ui.add_space(10.0);
                }
                Block::List { items, .. } => {
                    for item in items {
                        ui.horizontal(|ui| {
                            ui.label("•");
                            ui.vertical(|ui| {
                                for block in item.blocks {
                                    if let Block::Paragraph(text) = block {
                                        self.inline(ui, &text, detail, 12.0);
                                    }
                                }
                            });
                        });
                    }
                    ui.add_space(10.0);
                }
                Block::Quote(blocks) => {
                    for block in blocks {
                        if let Block::Paragraph(text) = block {
                            self.inline(ui, &text, detail, 11.5);
                        }
                    }
                    ui.add_space(8.0);
                }
                Block::Rule => {
                    ui.separator();
                    ui.add_space(6.0);
                }
                Block::Table { header, rows, .. } => {
                    for row in header.iter().chain(rows.iter().flatten()) {
                        self.inline(ui, row, detail, 11.5);
                    }
                    ui.add_space(8.0);
                }
            }
        }
    }
}

impl CodeApp {
    #[allow(dead_code)]
    pub(crate) fn detail_tab_open(&self) -> bool {
        self.marketplace.detail_tab.is_some()
    }

    pub(crate) fn showing_extension_tab(&self) -> bool {
        self.marketplace.detail_tab.as_ref().is_some_and(|found| {
            matches!(
                &self.active_view,
                Some(crate::app::views::view_tabs::ActiveView::Extension(id)) if id == &found.id()
            )
        })
    }

    pub(crate) fn show_marketplace(&mut self) {
        self.marketplace.query.clear();
        self.marketplace.results = None;
    }

    pub(crate) fn set_marketplace_query(&mut self, query: String) {
        self.marketplace.query = query;
    }

    fn marketplace_search(&mut self, ctx: &egui::Context) {
        let query = self.marketplace.query.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(open_vsx::browse(query.trim(), None, 60, 0));
            wake();
        });
        self.marketplace.searching = Some(rx);
        self.marketplace.results = None;
    }

    pub(crate) fn show_extension_tab(&mut self, ctx: &egui::Context, found: Found) {
        self.active_view = Some(crate::app::views::view_tabs::ActiveView::Extension(found.id()));
        self.marketplace.detail_tab = Some(found.clone());
        self.marketplace.page = Page::Details;
        self.marketplace.detail_page = None;
        if found.download.is_empty() {
            self.marketplace.detail_page = Some(Ok(Detail::default()));
            return;
        }
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(open_vsx::detail(&found.namespace, &found.name));
            wake();
        });
        self.marketplace.detail = Some(rx);
    }

    fn start_marketplace_install(&mut self, ctx: &egui::Context, found: Found) {
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let id = found.id();
        std::thread::spawn(move || {
            let _ = tx.send(open_vsx::install_extension(&found));
            wake();
        });
        self.marketplace.installing = Some((id, rx));
        self.marketplace.done = None;
    }

    fn marketplace_install(&mut self, ctx: &egui::Context, found: Found) {
        if !found.verified {
            self.marketplace.confirm = Some(found);
            return;
        }
        self.start_marketplace_install(ctx, found);
    }

    fn marketplace_uninstall(&mut self, id: &str) {
        match open_vsx::uninstall_extension(id) {
            Ok(()) => {
                self.status = Some(format!("Uninstalled {id}. Imported themes and snippets stay."));
                self.marketplace.installed = Some(vscode_compat::installed());
            }
            Err(e) => self.status = Some(e),
        }
    }

    pub(crate) fn ext_icon(
        &mut self,
        ctx: &egui::Context,
        container: &coiman_core::extensions::host::ViewContainer,
        color: egui::Color32,
    ) -> Option<egui::TextureHandle> {
        if container.icon.as_os_str().is_empty() {
            return None;
        }
        let key = format!("{}::{:?}", container.id, color);
        if let Some(texture) = self.ext_icons.get(&key) {
            return Some(texture.clone());
        }
        let bytes = std::fs::read(&container.icon).ok()?;
        let image = if container.icon.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
            let svg = String::from_utf8_lossy(&bytes);
            crate::ui::icons::draw_svg(&svg, 64)?
        } else {
            let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let (w, h) = image.dimensions();
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], image.as_raw())
        };
        // Coiman's sidebar icons are monochrome: use the image as a mask.
        let pixels: Vec<egui::Color32> = image
            .pixels
            .iter()
            .map(|pixel| {
                let luminance =
                    (0.2126 * pixel.r() as f32 + 0.7152 * pixel.g() as f32 + 0.0722 * pixel.b() as f32) / 255.0;
                let alpha = (pixel.a() as f32 * (1.0 - luminance)).clamp(0.0, 255.0) as u8;
                egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
            })
            .collect();
        let tinted = egui::ColorImage { size: image.size, source_size: image.source_size, pixels };
        let texture = ctx.load_texture(&key, tinted, egui::TextureOptions::LINEAR);
        self.ext_icons.insert(key, texture.clone());
        Some(texture)
    }

    pub(crate) fn request_ext_tree(
        &mut self,
        ctx: &egui::Context,
        container: &coiman_core::extensions::host::ViewContainer,
    ) {
        if !self.settings.vscode_extensions_enabled() {
            return;
        }
        let Some(view) = container.views.first().map(|v| v.0.clone()) else { return };
        self.ext_host.activate_for_view(&view);
        if self.ext_tree.contains_key(&view) || self.ext_tree_pending.contains(&view) {
            return;
        }
        if let Some(id) = self.ext_host.request("tree/items", serde_json::json!({ "viewId": view })) {
            self.ext_requests.insert(id, crate::app::extensions::host::Request::Tree { view: view.clone() });
            self.ext_tree_pending.insert(view);
        }
        let _ = ctx;
    }

    pub(crate) fn extension_tree_view(&mut self, ui: &mut egui::Ui, view: &str) {
        let pal = theme::colors();
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("EXTENSION VIEW").small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("×").clicked() {
                    self.active_ext_view = None;
                }
            });
        });
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.ext_tree_pending.contains(view) {
                ui.spinner();
                ui.label(RichText::new("Loading...").small().color(pal.text_dim));
            }
            if let Some(items) = self.ext_tree.get(view) {
                for (label, description) in items.clone() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(label).small());
                        if !description.is_empty() {
                            ui.label(RichText::new(description).small().color(pal.text_dim));
                        }
                    });
                }
            }
        });
    }

    pub(crate) fn marketplace_sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.marketplace.query.trim().to_lowercase().starts_with("@plugins") {
            self.installed_section(ui, &ctx);
            return;
        }
        let state = &mut self.marketplace;
        if state.installed.is_none() {
            state.installed = Some(vscode_compat::installed());
        }
        if let Some(r) = state.searching.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.searching = None;
            state.results = Some(r);
        }
        if let Some(r) = state.detail.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.detail = None;
            state.detail_page = Some(r);
        }
        let mut installed_now = false;
        if let Some(r) = state.installing.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
            let id = state.installing.take().map(|(id, _)| id).unwrap_or_default();
            installed_now = r.is_ok();
            if let Ok(installed) = &r
                && self.settings.vscode_extensions_enabled()
            {
                self.ext_host.add_extension(&installed.folder);
            }
            self.status = Some(match &r {
                Ok(installed) => format!("Installed {} {}", installed.id, installed.version),
                Err(e) => format!("Install failed: {e}"),
            });
            state.done = Some((id, r));
            state.installed = Some(vscode_compat::installed());
        }
        if installed_now {
            if self.settings.vscode_extensions_enabled() {
                self.ext_views = self.ext_host.all_view_containers();
            }
            self.themes = coiman_core::appearance::theme::discover();
        }
        let pal = theme::colors();
        let mut ask = None;
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            let edit = ui.add(
                egui::TextEdit::singleline(&mut state.query)
                    .hint_text("Search Extensions in Marketplace")
                    .desired_width(ui.available_width() - 95.0),
            );
            if (edit.has_focus() || edit.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ask = Some(Ask::Search);
            }
            if ui.button("🔍").on_hover_text("Search Open VSX").clicked() {
                ask = Some(Ask::Search);
            }
            if ui.button("×").on_hover_text("Clear search").clicked() {
                state.query.clear();
            }
            let filter = ui.button("⋯").on_hover_text("Filter and sort");
            egui::Popup::menu(&filter).show(|ui| {
                for (label, query) in [
                    ("Installed extensions", "@installed"),
                    ("Recommended", "@recommended"),
                    ("Popular", "@popular"),
                    ("Updates", "@updates"),
                    ("Coiman plugins", "@plugins"),
                ] {
                    if ui.button(label).clicked() {
                        state.query = query.to_owned();
                        ui.close();
                    }
                }
                ui.separator();
                for (label, query) in [
                    ("Sort by installs", "@sort:installs"),
                    ("Sort by rating", "@sort:rating"),
                    ("Sort by name", "@sort:name"),
                ] {
                    if ui.button(label).clicked() {
                        state.query = format!("{query} {}", state.query.trim_start_matches('@'));
                        ask = Some(Ask::Search);
                        ui.close();
                    }
                }
            });
        });
        ui.add_space(8.0);
        if let Some((id, _)) = &state.installing {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.spinner();
                ui.label(RichText::new(format!("Installing {id}…")).small().color(pal.text_dim));
            });
        }
        let mut dismiss = false;
        if let Some((id, result)) = &state.done {
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                match result {
                    Ok(installed) => {
                        ui.label(RichText::new(format!("Installed {id} {}", installed.version)).small().strong());
                        if !installed.capability.is_empty() {
                            ui.label(RichText::new(&installed.capability).small().color(pal.text_dim));
                        }
                    }
                    Err(e) => {
                        ui.label(RichText::new(format!("{id}: {e}")).small().color(theme::colors().modified));
                    }
                }
                dismiss = ui.small_button("×").on_hover_text("Dismiss").clicked();
            });
        }
        if dismiss {
            state.done = None;
        }
        let query = state.query.trim().to_lowercase();
        let installed = state.installed.clone().unwrap_or_default();
        let selected = state.detail_tab.as_ref().map(|f| f.id());

        // When query is empty, show both INSTALLED and POPULAR/RECOMMENDED sections.
        let is_empty_query = query.is_empty();
        if is_empty_query && state.results.is_none() && state.searching.is_none() && ask.is_none() {
            ask = Some(Ask::Search);
        }

        let installed_cards: Vec<Card> = installed.iter().map(Card::from_report).collect();
        let mut results_cards: Vec<Card> = match &state.results {
            Some(Ok(found)) => found.iter().cloned().map(|f| Card::from_found(f, &installed)).collect(),
            _ => Vec::new(),
        };
        if query.contains("@sort:name") {
            results_cards.sort_by_key(|c| c.found.display_name.to_lowercase());
        } else if query.contains("@sort:rating") {
            results_cards.sort_by_key(|c| std::cmp::Reverse(c.found.rating));
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(2.0);
            if is_empty_query {
                // 1. Installed section
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("⌄").small().color(pal.text_dim));
                    ui.label(RichText::new("INSTALLED").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(installed_cards.len().to_string()).small().color(pal.text_dim));
                    });
                });
                ui.add_space(4.0);
                if installed_cards.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new("No extensions installed yet.").small().color(pal.text_dim));
                    });
                } else {
                    for item in &installed_cards {
                        let is_selected = selected.as_deref() == Some(item.found.id().as_str());
                        state.card(ui, item, is_selected, &mut ask, &pal);
                    }
                }
                ui.add_space(12.0);

                // 2. Popular / Recommended section
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("⌄").small().color(pal.text_dim));
                    ui.label(RichText::new("POPULAR / RECOMMENDED").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(results_cards.len().to_string()).small().color(pal.text_dim));
                    });
                });
                ui.add_space(4.0);
                if state.searching.is_some() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.spinner();
                        ui.label(RichText::new("Loading extensions from Open VSX…").small().color(pal.text_dim));
                    });
                } else if let Some(Err(e)) = &state.results {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.colored_label(theme::colors().modified, e);
                    });
                } else if results_cards.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new("Type to search Open VSX for extensions.").small().color(pal.text_dim));
                    });
                } else {
                    for item in &results_cards {
                        let is_selected = selected.as_deref() == Some(item.found.id().as_str());
                        state.card(ui, item, is_selected, &mut ask, &pal);
                    }
                }
            } else if query.starts_with("@installed") || query.starts_with("@updates") {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("⌄").small().color(pal.text_dim));
                    ui.label(RichText::new("INSTALLED").small().strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(installed_cards.len().to_string()).small().color(pal.text_dim));
                    });
                });
                ui.add_space(4.0);
                for item in &installed_cards {
                    let is_selected = selected.as_deref() == Some(item.found.id().as_str());
                    state.card(ui, item, is_selected, &mut ask, &pal);
                }
            } else {
                let heading = if query.starts_with("@recommended") { "RECOMMENDED" } else { "MARKETPLACE RESULTS" };
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("⌄").small().color(pal.text_dim));
                    ui.label(RichText::new(heading).small().strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.0);
                        ui.label(RichText::new(results_cards.len().to_string()).small().color(pal.text_dim));
                    });
                });
                ui.add_space(4.0);
                if state.searching.is_some() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.spinner();
                        ui.label(RichText::new("Searching Open VSX…").small().color(pal.text_dim));
                    });
                } else if let Some(Err(e)) = &state.results {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.colored_label(theme::colors().modified, e);
                    });
                } else if results_cards.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("No extensions found matching your search.").small().color(pal.text_dim),
                        );
                    });
                } else {
                    for item in &results_cards {
                        let is_selected = selected.as_deref() == Some(item.found.id().as_str());
                        state.card(ui, item, is_selected, &mut ask, &pal);
                    }
                }
            }
        });
        let footer = ui.allocate_exact_size(egui::vec2(ui.available_width(), 27.0), egui::Sense::hover()).0;
        ui.painter().hline(footer.x_range(), footer.top(), egui::Stroke::new(1.0, pal.border));
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Visual Studio Marketplace").small().color(pal.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("⋯").small().color(pal.text_dim));
            });
        });
        match ask {
            Some(Ask::Search) => self.marketplace_search(&ctx),
            Some(Ask::Detail(found)) => self.show_extension_tab(&ctx, *found),
            Some(Ask::Install(found)) => self.marketplace_install(&ctx, *found),
            Some(Ask::Uninstall(id)) => self.marketplace_uninstall(&id),
            None => {}
        }
        self.marketplace_trust_modal(&ctx);
    }

    fn marketplace_trust_modal(&mut self, ctx: &egui::Context) {
        let Some(found) = self.marketplace.confirm.clone() else { return };
        let mut keep = true;
        let mut confirmed = false;
        egui::Modal::new(egui::Id::unique("trust_publisher")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(RichText::new("Do you trust the publisher?").size(15.0).strong());
            ui.add_space(6.0);
            ui.label("This extension is not verified on Open VSX. Install only if you trust its publisher.");
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{} · {}", found.display_name, found.id())).strong());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                confirmed = ui.button("Trust Publisher & Install").clicked();
                keep = !ui.button("Cancel").clicked() && keep;
            });
        });
        if confirmed {
            let found = self.marketplace.confirm.take().unwrap();
            self.start_marketplace_install(ctx, found);
        } else if !keep {
            self.marketplace.confirm = None;
        }
    }

    pub(crate) fn marketplace_detail_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let state = &mut self.marketplace;
        if let Some(r) = state.detail.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.detail = None;
            state.detail_page = Some(r);
        }
        let pal = theme::colors();
        let mut ask = None;
        let mut status = None;
        ui.add_space(10.0);
        let selected_found = state.detail_tab.clone();
        let page_detail = state.detail_page.clone();
        if let Some(found) = selected_found {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                // A 1200px page with 34px of padding on each side. `add_space`
                // in a vertical layout is vertical, so use an explicit rect
                // for the horizontal centring.
                let page_width = (area.width() - 68.0).clamp(280.0, 1132.0);
                let left = ((area.width() - page_width) / 2.0).max(0.0);
                let page_rect = egui::Rect::from_min_size(
                    egui::pos2(area.left() + left, area.top()),
                    egui::vec2(page_width, area.height()),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(page_rect), |ui| {
                    ui.add_space(28.0);
                    ui.horizontal_top(|ui| {
                        let (icon_rect, _) = ui.allocate_exact_size(egui::vec2(112.0, 112.0), egui::Sense::hover());
                        if let Some(texture) = state.icon(&ctx, &found.icon) {
                            egui::Image::from_texture(&texture).paint_at(ui, icon_rect);
                        } else {
                            let letter =
                                found.display_name.chars().next().unwrap_or('?').to_ascii_uppercase().to_string();
                            let center = icon_rect.center();
                            ui.painter().circle_filled(center, 46.0, pal.accent.gamma_multiply(0.22));
                            ui.painter().circle_stroke(
                                center,
                                46.0,
                                egui::Stroke::new(1.0, pal.accent.gamma_multiply(0.5)),
                            );
                            ui.painter().text(
                                center,
                                egui::Align2::CENTER_CENTER,
                                letter,
                                egui::FontId::proportional(40.0),
                                pal.accent,
                            );
                        }
                        ui.add_space(22.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width().min(760.0));
                            let title =
                                if found.display_name.is_empty() { found.id() } else { found.display_name.clone() };
                            ui.label(RichText::new(title).size(26.0).strong());
                            let extra = page_detail.as_ref().and_then(|r| r.as_ref().ok());
                            let publisher = extra
                                .map(|d| d.publisher.clone())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| publisher_text(&found));
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 12.0;
                                ui.label(RichText::new(format!("v{}", found.version)).monospace().small());
                                if found.verified {
                                    ui.label(RichText::new("✓").small().color(pal.accent));
                                }
                                ui.label(RichText::new(publisher).small());
                                if let Some(d) = extra {
                                    if !d.repository.is_empty() && ui.link("Repository").clicked() {
                                        ctx.open_url(egui::OpenUrl::new_tab(&d.repository));
                                    }
                                    if !d.license.is_empty() {
                                        ui.label(RichText::new(&d.license).small().color(pal.text_dim));
                                    }
                                }
                            });
                            ui.add_space(10.0);
                            ui.add(egui::Label::new(RichText::new(&found.description).color(pal.text_dim)).wrap());
                            ui.add_space(10.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                ui.label(RichText::new("♧").small().color(pal.text_dim));
                                ui.label(RichText::new(short_count(found.downloads)).small().color(pal.text_dim));
                                ui.add_space(6.0);
                                ui.label(RichText::new("★★★★★").small().color(pal.text_dim));
                                ui.label(RichText::new(format!("({})", found.rating)).small().color(pal.text_dim));
                                ui.add_space(12.0);
                                let installed =
                                    state.installed.as_ref().is_some_and(|r| r.iter().any(|x| x.id == found.id()));
                                if installed {
                                    if ui.button("Uninstall").clicked() {
                                        ask = Some(Ask::Uninstall(found.id()));
                                    }
                                } else if ui.button("Install").clicked() {
                                    ask = Some(Ask::Install(Box::new(found.clone())));
                                }
                                let gear = ui.button("⚙").on_hover_text("Manage");
                                egui::Popup::menu(&gear).show(|ui| {
                                    if ui.button("Disable").clicked() {
                                        status = Some(format!("{} disabled globally", found.display_name));
                                        ui.close();
                                    }
                                    if ui.button("Switch to Pre-Release Version").clicked() {
                                        status = Some("Pre-release channel selected".to_owned());
                                        ui.close();
                                    }
                                    if ui.button("Enable Auto Update").clicked() {
                                        status = Some(format!("Auto update enabled for {}", found.display_name));
                                        ui.close();
                                    }
                                });
                                if !found.url.is_empty() && ui.link("View on open-vsx.org").clicked() {
                                    ctx.open_url(egui::OpenUrl::new_tab(&found.url));
                                }
                            });
                            ui.add_space(8.0);
                            let installed =
                                state.installed.as_ref().is_some_and(|r| r.iter().any(|x| x.id == found.id()));
                            ui.label(
                                RichText::new(if installed {
                                    "This extension is enabled globally."
                                } else {
                                    "This extension is not installed."
                                })
                                .small()
                                .color(pal.text_dim),
                            );
                        });
                    });
                    ui.add_space(18.0);
                    ui.separator();
                    ui.add_space(4.0);
                    egui::ScrollArea::horizontal().id_salt("ext_detail_tabs").show(ui, |ui| {
                        for (label, page) in [
                            ("Details", Page::Details),
                            ("Feature Contributions", Page::Contributions),
                            ("Changelog", Page::Changelog),
                            ("Extension Pack", Page::Pack),
                            ("Runtime Status", Page::Runtime),
                        ] {
                            ui.selectable_value(&mut state.page, page, label);
                        }
                    });
                    ui.separator();
                    let detail = page_detail.clone();
                    let reports = state.installed.clone().unwrap_or_default();
                    let installed = reports.iter().find(|r| r.id == found.id()).cloned();
                    let summary = installed.as_ref().map(|r| r.summary()).unwrap_or_default();
                    match (state.page, detail) {
                        (_, None) => {
                            ui.spinner();
                            ui.label("Loading the extension page...");
                        }
                        (_, Some(Err(e))) => {
                            ui.colored_label(theme::colors().modified, e);
                        }
                        (Page::Details, Some(Ok(d))) => {
                            let avail = ui.available_rect_before_wrap();
                            let meta_width = 210.0;
                            let gap = 28.0;
                            if avail.width() > meta_width + gap + 280.0 {
                                let (left, mut right) = avail.split_left_right_at_x(avail.right() - meta_width - gap);
                                right.min.x += gap;
                                ui.scope_builder(egui::UiBuilder::new().max_rect(left), |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
                                        let badge = |ui: &mut egui::Ui, label: &str, value: &str| {
                                            egui::Frame::new().fill(pal.widget_bg).corner_radius(2.0).show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(RichText::new(label).small().color(pal.text_dim));
                                                    ui.label(RichText::new(value).small().strong());
                                                });
                                            });
                                        };
                                        badge(ui, "Open VSX", &format!("v{}", d.version));
                                        badge(ui, "installs", &short_count(d.downloads));
                                        badge(ui, "rating", &format!("{}/5", d.rating as f64 / 10.0));
                                    });
                                    ui.add_space(14.0);
                                    if d.readme.trim().is_empty() {
                                        let text =
                                            if found.description.is_empty() { &summary } else { &found.description };
                                        ui.add(egui::Label::new(RichText::new(text)).wrap());
                                    } else {
                                        state.readme(ui, &d);
                                    }
                                });
                                ui.scope_builder(egui::UiBuilder::new().max_rect(right), |ui| {
                                    render_metadata(ui, &d, &found.id());
                                });
                            } else if d.readme.trim().is_empty() {
                                let text = if found.description.is_empty() { &summary } else { &found.description };
                                ui.add(egui::Label::new(RichText::new(text)).wrap());
                            } else {
                                state.readme(ui, &d);
                            }
                        }
                        (Page::Contributions, Some(Ok(d))) => render_contributions(ui, &d),
                        (Page::Changelog, Some(Ok(d))) => {
                            if d.changelog.trim().is_empty() {
                                ui.label(
                                    RichText::new("This publisher has no changelog on Open VSX.").color(pal.text_dim),
                                );
                            } else {
                                state.readme(ui, &Detail { readme: d.changelog.clone(), ..d });
                            }
                        }
                        (Page::Pack, Some(Ok(d))) => render_pack(ui, &d),
                        (Page::Runtime, Some(Ok(_))) => render_runtime(ui, &found.id(), installed.as_ref(), &summary),
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Identifier").small().color(pal.text_dim));
                        ui.label(RichText::new(found.id()).monospace().small());
                    });
                    ui.add_space(60.0);
                });
            });
        }
        match ask {
            Some(Ask::Install(found)) => self.marketplace_install(&ctx, *found),
            Some(Ask::Uninstall(id)) => self.marketplace_uninstall(&id),
            _ => {}
        }
        if let Some(message) = status {
            self.status = Some(message);
        }
        self.marketplace_trust_modal(&ctx);
    }
}

fn render_metadata(ui: &mut egui::Ui, detail: &Detail, id: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new("Categories").size(15.0).strong());
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for category in &detail.categories {
            ui.label(RichText::new(category.clone()).small().color(theme::colors().text));
        }
    });
    ui.add_space(16.0);
    ui.label(RichText::new("Resources").size(15.0).strong());
    if ui.link("Marketplace").clicked() {
        let (namespace, name) = detail.id.split_once('.').unwrap_or(("", ""));
        ui.ctx().open_url(egui::OpenUrl::new_tab(format!("https://open-vsx.org/extension/{namespace}/{name}")));
    }
    if !detail.repository.is_empty() && ui.link("Repository").clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(&detail.repository));
    }
    if !detail.homepage.is_empty() && ui.link("Homepage").clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(&detail.homepage));
    }
    ui.add_space(16.0);
    ui.label(RichText::new("More Info").size(15.0).strong());
    for (key, value) in [
        ("Identifier", id.to_owned()),
        ("Version", detail.version.clone()),
        ("License", detail.license.clone()),
        ("Updated", detail.timestamp.clone()),
    ] {
        if value.is_empty() {
            continue;
        }
        ui.horizontal(|ui| {
            ui.add_sized([74.0, 0.0], egui::Label::new(RichText::new(key).small().color(theme::colors().text_dim)));
            ui.add(egui::Label::new(RichText::new(value.clone()).small()).wrap());
        });
    }
}

fn render_contributions(ui: &mut egui::Ui, detail: &Detail) {
    ui.label(RichText::new("Feature Contributions").size(16.0).strong());
    let contributes = &detail.manifest["contributes"];
    let mut rows: Vec<(&str, usize)> = Vec::new();
    for key in
        ["commands", "languages", "debuggers", "configuration", "snippets", "themes", "grammars", "jsonValidation"]
    {
        let n = contributes.get(key).and_then(|v| v.as_array()).map_or(0, Vec::len);
        if n > 0 {
            rows.push((key, n));
        }
    }
    if rows.is_empty() {
        ui.label(
            RichText::new("No declarative contributions were found in this extension's package.json.")
                .color(theme::colors().text_dim),
        );
    }
    for (key, n) in rows {
        ui.horizontal(|ui| {
            ui.label(RichText::new(key).monospace());
            ui.label(RichText::new(n.to_string()).strong());
        });
    }
    ui.add_space(8.0);
    if let Some(config) = contributes.get("configuration").and_then(|v| v.as_array()) {
        ui.label(RichText::new("Settings").strong());
        for item in config.iter().take(20) {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| item.get("id").and_then(|v| v.as_str()))
                .unwrap_or("");
            if !title.is_empty() {
                ui.label(RichText::new(title).small());
            }
        }
    }
}

fn render_pack(ui: &mut egui::Ui, detail: &Detail) {
    ui.label(RichText::new("Extension Pack").size(16.0).strong());
    let pack: Vec<String> = detail
        .manifest
        .get("extensionPack")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if pack.is_empty() {
        ui.label("This extension is not a pack. Its dependencies are:");
        for id in &detail.dependencies {
            ui.label(RichText::new(id.clone()).monospace().small());
        }
    } else {
        for id in pack {
            ui.label(RichText::new(id).monospace().small());
        }
    }
}

fn render_runtime(ui: &mut egui::Ui, id: &str, installed: Option<&Report>, summary: &str) {
    ui.label(RichText::new("Runtime Status").size(16.0).strong());
    ui.label(if installed.is_some() { "Installed" } else { "Not installed" });
    if !summary.is_empty() {
        ui.label(RichText::new(summary).small().color(theme::colors().text_dim));
    }
    if let Some(spec) = companions::all().into_iter().find(|c| c.extension.eq_ignore_ascii_case(id)) {
        let found = spec.installed_script();
        ui.add_space(6.0);
        ui.label(
            RichText::new(format!(
                "{} server: {}",
                spec.name,
                found.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "not found".to_owned())
            ))
            .small(),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::app::CodeApp;

    #[test]
    fn an_extension_page_opens_as_an_editor_tab() {
        use coiman_core::extensions::open_vsx::Found;

        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        let found = Found {
            namespace: "rust-lang".to_owned(),
            name: "rust-analyzer".to_owned(),
            display_name: "rust-analyzer".to_owned(),
            description: "Rust support".to_owned(),
            version: "1.0.0".to_owned(),
            downloads: 10,
            verified: true,
            download: String::new(),
            icon: String::new(),
            rating: 45,
            categories: Vec::new(),
            url: String::new(),
            timestamp: String::new(),
        };
        app.show_extension_tab(&ctx, found);
        assert!(app.detail_tab_open());
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.marketplace_detail_view(ui));
        output.textures_delta.clear();
        assert!(
            crate::app::CodeApp::view_tabs(&app)
                .iter()
                .any(|(tab, ..)| matches!(tab, crate::app::views::view_tabs::ViewTab::Extension(_)))
        );
    }

    #[test]
    fn readme_blocks_and_images_render_in_the_detail_tab() {
        use coiman_core::extensions::open_vsx::Found;

        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        app.show_extension_tab(
            &ctx,
            Found {
                namespace: "a".to_owned(),
                name: "b".to_owned(),
                display_name: "B".to_owned(),
                description: "A test extension".to_owned(),
                version: "1.0.0".to_owned(),
                downloads: 1000,
                verified: true,
                download: "https://open-vsx.org/api/a/b/1.0.0/file/a.b-1.0.0.vsix".to_owned(),
                icon: String::new(),
                rating: 45,
                categories: vec!["Linters".to_owned()],
                url: String::new(),
                timestamp: String::new(),
            },
        );
        app.marketplace.detail_page = Some(Ok(coiman_core::extensions::open_vsx::Detail {
            readme: "# Title\n\nSome paragraph.\n\n- first\n- second\n\n![pic](media/pic.png)\n".to_owned(),
            version: "1.0.0".to_owned(),
            categories: vec!["Linters".to_owned()],
            ..Default::default()
        }));
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.marketplace_detail_view(ui));
        output.textures_delta.clear();
        assert!(app.detail_tab_open());
    }

    #[test]
    fn switching_to_another_tab_releases_the_editor() {
        use coiman_core::extensions::open_vsx::Found;

        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        app.show_extension_tab(
            &ctx,
            Found {
                namespace: "a".to_owned(),
                name: "b".to_owned(),
                display_name: "B".to_owned(),
                description: String::new(),
                version: "1.0.0".to_owned(),
                downloads: 0,
                verified: false,
                download: String::new(),
                icon: String::new(),
                rating: 0,
                categories: Vec::new(),
                url: String::new(),
                timestamp: String::new(),
            },
        );
        assert!(app.detail_tab_open());
        app.hide_view_tabs();
        assert!(app.detail_tab_open(), "the extension tab stays open behind the file tab");
        assert!(!app.showing_extension_tab(), "the file tab takes the editor back");
    }

    #[test]
    fn a_verified_install_starts_in_the_background() {
        use coiman_core::extensions::open_vsx::Found;

        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        app.marketplace_install(
            &ctx,
            Found {
                namespace: "example".to_owned(),
                name: "demo".to_owned(),
                display_name: "Demo".to_owned(),
                description: String::new(),
                version: "1.0.0".to_owned(),
                downloads: 0,
                verified: true,
                download: "https://open-vsx.org/api/example/demo/1.0.0/file/example.demo-1.0.0.vsix".to_owned(),
                icon: String::new(),
                rating: 0,
                categories: Vec::new(),
                url: String::new(),
                timestamp: String::new(),
            },
        );
        assert!(app.marketplace.installing.is_some(), "install starts without blocking the UI");
        assert!(app.marketplace.confirm.is_none(), "verified publishers need no confirmation");
    }

    #[test]
    fn extension_icons_are_tinted_for_the_sidebar() {
        let dir = std::env::temp_dir().join(format!("coiman-ext-icon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let icon = dir.join("icon.svg");
        std::fs::write(&icon, r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path fill="#000" d="M0 0h24v24H0z"/></svg>"##).unwrap();
        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        let container = coiman_core::extensions::host::ViewContainer {
            id: "x.demo".into(),
            title: "Demo".into(),
            icon,
            views: vec![("x.demo.view".into(), "Demo".into())],
        };
        assert!(app.ext_icon(&ctx, &container, egui::Color32::RED).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn marketplace_sidebar_starts_empty() {
        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(340.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.marketplace_sidebar(ui));
        output.textures_delta.clear();
        assert!(!app.detail_tab_open(), "no detail tab until an extension is opened");
    }
}
