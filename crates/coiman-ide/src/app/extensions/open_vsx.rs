//! The Open VSX window: search open-vsx.org for color and icon themes and install them. Only
//! themes are taken (see `coiman_core::extensions::open_vsx`); `curl` does the fetching.

use std::sync::mpsc::{self, Receiver};

use coiman_core::extensions::open_vsx::{self, Found, Installed};
use egui::{Color32, RichText};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Default)]
pub(crate) struct OpenVsxUi {
    pub(crate) open: bool,
    query: String,
    searching: Option<Receiver<Result<Vec<Found>, String>>>,
    results: Option<Result<Vec<Found>, String>>,
    /// The extension being installed, and its result to come.
    installing: Option<(String, Receiver<Result<Installed, String>>)>,
    /// What the last install added, to use at once: (extension, what).
    done: Option<(String, Result<Installed, String>)>,
}

/// What the window asks the app to do.
enum Ask {
    Search,
    Install(Box<Found>),
    UseTheme(String),
    UseIcons(String),
}

impl CodeApp {
    pub(crate) fn show_open_vsx(&mut self) {
        self.open_vsx.open = true;
    }

    fn open_vsx_search(&mut self, ctx: &egui::Context) {
        let query = self.open_vsx.query.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(open_vsx::search(&query));
            wake();
        });
        self.open_vsx.searching = Some(rx);
    }

    fn open_vsx_install(&mut self, ctx: &egui::Context, found: Found) {
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let id = found.id();
        std::thread::spawn(move || {
            let _ = tx.send(open_vsx::install(&found));
            wake();
        });
        self.open_vsx.installing = Some((id, rx));
        self.open_vsx.done = None;
    }

    pub(crate) fn open_vsx_window(&mut self, ctx: &egui::Context) {
        let state = &mut self.open_vsx;
        if !state.open {
            return;
        }
        let mut ask = None;
        if let Some(r) = state.searching.as_ref().and_then(|rx| rx.try_recv().ok()) {
            state.searching = None;
            state.results = Some(r);
        }
        let mut installed_now = false;
        if let Some(r) = state.installing.as_ref().and_then(|(_, rx)| rx.try_recv().ok()) {
            let id = state.installing.take().map(|(id, _)| id).unwrap_or_default();
            installed_now = r.is_ok();
            state.done = Some((id, r));
        }
        if installed_now {
            self.themes = coiman_core::appearance::theme::discover();
        }
        let state = &mut self.open_vsx;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x - 52.0).clamp(200.0, 760.0), (room.y * 0.8).clamp(200.0, room.y - 80.0));
        let mut open = true;
        egui::Window::new(RichText::new("Themes from Open VSX").size(16.0).strong())
            .id(egui::Id::unique(("open_vsx_window", size.x.round() as i32)))
            .open(&mut open)
            .collapsible(false)
            .fixed_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .constrain(true)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(18, 12)),
            )
            .show(ctx, |ui| {
                ui.set_max_width(size.x - 36.0);
                ui.label(
                    RichText::new(
                        "Color and icon themes from open-vsx.org, fetched with curl. Only an extension's themes \
                         are taken: never its code, snippets or languages.",
                    )
                    .color(pal.text_dim),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .hint_text("Search themes: nord, catppuccin, icons...")
                            .desired_width((ui.available_width() - 90.0).max(100.0)),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.add_enabled(state.searching.is_none(), egui::Button::new("Search")).clicked() || enter {
                        ask = Some(Ask::Search);
                    }
                    if state.searching.is_some() {
                        ui.spinner();
                    }
                });
                if let Some((id, result)) = &state.done {
                    ui.add_space(6.0);
                    match result {
                        Ok(added) => {
                            ui.label(RichText::new(format!("Installed from {id}:")).strong());
                            ui.horizontal_wrapped(|ui| {
                                for label in &added.themes {
                                    if ui.button(format!("Use {label}")).clicked() {
                                        ask = Some(Ask::UseTheme(label.clone()));
                                    }
                                }
                                for (pack, label) in &added.icon_packs {
                                    if ui.button(format!("Use {label} icons")).clicked() {
                                        ask = Some(Ask::UseIcons(pack.clone()));
                                    }
                                }
                            });
                            for why in &added.skipped {
                                ui.label(RichText::new(why).small().color(pal.text_dim));
                            }
                        }
                        Err(e) => {
                            ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), format!("{id}: {e}"));
                        }
                    }
                }
                ui.separator();
                let installing = state.installing.as_ref().map(|(id, _)| id.clone());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match &state.results {
                    None if state.searching.is_some() => {
                        ui.label(RichText::new("Searching...").color(pal.text_dim));
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "Nothing is fetched until you search. An empty search lists the most downloaded.",
                            )
                            .color(pal.text_dim),
                        );
                    }
                    Some(Err(e)) => {
                        ui.colored_label(Color32::from_rgb(0xE5, 0x48, 0x4D), e);
                    }
                    Some(Ok(found)) if found.is_empty() => {
                        ui.label(RichText::new("No themes found.").color(pal.text_dim));
                    }
                    Some(Ok(found)) => {
                        for item in found {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.set_width((ui.available_width() - 110.0).max(120.0));
                                    let mut title = RichText::new(&item.display_name).strong();
                                    if item.display_name.is_empty() {
                                        title = RichText::new(item.id()).strong();
                                    }
                                    ui.label(title);
                                    let mut by =
                                        format!("{} · {} · {} downloads", item.id(), item.version, item.downloads);
                                    if item.verified {
                                        by.push_str(" · verified publisher");
                                    }
                                    ui.label(RichText::new(by).small().color(pal.text_dim));
                                    if !item.description.is_empty() {
                                        ui.add(
                                            egui::Label::new(RichText::new(&item.description).color(pal.text_dim))
                                                .wrap(),
                                        );
                                    }
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if installing.as_deref() == Some(item.id().as_str()) {
                                        ui.spinner();
                                    } else if ui
                                        .add_enabled(installing.is_none(), egui::Button::new("Install"))
                                        .clicked()
                                    {
                                        ask = Some(Ask::Install(Box::new(item.clone())));
                                    }
                                });
                            });
                            ui.add_space(6.0);
                        }
                    }
                });
            });
        self.open_vsx.open = open;
        match ask {
            Some(Ask::Search) => self.open_vsx_search(ctx),
            Some(Ask::Install(found)) => self.open_vsx_install(ctx, *found),
            Some(Ask::UseTheme(label)) => self.set_color_theme(ctx, &label),
            Some(Ask::UseIcons(id)) => {
                let value = toml::Value::String(id).to_string();
                if let Err(e) = self.write_user_setting(ctx, "workbench", "icon_theme", &value) {
                    self.status = Some(e);
                }
            }
            None => {}
        }
    }
}
