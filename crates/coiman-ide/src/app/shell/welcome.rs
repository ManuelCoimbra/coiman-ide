//! The Welcome page: Coiman IDE's own, not a list of links. A greeting with the icon, four
//! tiles to start from (a folder, a clone, a sample project, a new file), recent projects
//! as cards with their badges, and the keys worth knowing.

use coiman_core::project::project_tree::project_badge;
use eframe::egui;
use egui::{Align2, Color32, RichText, Sense};

use crate::app::CodeApp;
use crate::app::recent_ui::RecentClick;
use crate::commands::Command;
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

const WIDTH: f32 = 860.0;

/// "Good morning" and the like, by the clock here.
fn greeting() -> &'static str {
    let hour = coiman_core::appearance::theme_schedule::minute_now() / 60;
    match hour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// A tile: an icon, what it does, a line on it, and its keys. Returns whether it was clicked.
fn tile(ui: &mut egui::Ui, width: f32, icon: Icon, title: &str, line: &str, keys: &str) -> bool {
    let pal = theme::colors();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 108.0), Sense::click());
    let hover = theme::eased(ui.ctx(), resp.id.with("hover"), resp.hovered());
    let p = ui.painter();
    p.rect_filled(rect, 12.0, pal.widget_bg.lerp_to_gamma(pal.hover_bg, hover));
    let edge = pal.border.lerp_to_gamma(pal.accent, hover);
    p.rect_stroke(rect, 12.0, egui::Stroke::new(1.0 + hover, edge), egui::StrokeKind::Inside);
    // The icon on a round tile of the accent color.
    let spot = egui::Rect::from_min_size(rect.min + egui::vec2(16.0, 16.0), egui::vec2(34.0, 34.0));
    p.rect_filled(spot, 9.0, pal.accent.gamma_multiply(0.18 + 0.12 * hover));
    icons::paint(p, spot.shrink(7.0), icon, pal.accent);
    let font = egui::TextStyle::Body.resolve(ui.style());
    p.text(
        rect.min + egui::vec2(16.0, 64.0),
        Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(font.size * 1.1),
        pal.text,
    );
    let small = egui::FontId::proportional(font.size * 0.85);
    let clip = rect.shrink2(egui::vec2(16.0, 0.0));
    p.with_clip_rect(clip).text(rect.min + egui::vec2(16.0, 84.0), Align2::LEFT_TOP, line, small.clone(), pal.text_dim);
    if !keys.is_empty() {
        p.text(rect.right_top() + egui::vec2(-14.0, 16.0), Align2::RIGHT_TOP, keys, small, pal.text_dim);
    }
    resp.clicked()
}

impl CodeApp {
    /// Whether the side panel shows: as set, but not on the Welcome page unless asked for
    /// there.
    pub(crate) fn sidebar_shown(&self) -> bool {
        self.sidebar_visible && (!self.docs.is_empty() || self.welcome_sidebar)
    }

    pub(crate) fn welcome(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut cmd = None;
        let mut recent_click = None;
        let pal = theme::colors();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = WIDTH.min(ui.available_width() - 48.0);
            let side = ((ui.available_width() - width) / 2.0).max(24.0);
            ui.add_space((ui.available_height() * 0.12).clamp(40.0, 140.0));
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width);
                    // The greeting, with the icon.
                    ui.horizontal(|ui| {
                        let logo = crate::app::workbench::logo_texture(ui.ctx());
                        ui.add(egui::Image::new(&logo).fit_to_exact_size(egui::vec2(64.0, 64.0)));
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ui.add_space(6.0);
                            ui.label(RichText::new(format!("{}.", greeting())).size(30.0).strong().color(pal.text));
                            ui.label(
                                RichText::new("Coiman IDE. Open a project, or start one.")
                                    .size(15.0)
                                    .color(pal.text_dim),
                            );
                        });
                    });
                    // First time here: the tour, offered.
                    if !crate::app::tour::seen() {
                        ui.add_space(18.0);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 52.0), Sense::click());
                        let hover = theme::eased(ui.ctx(), resp.id.with("hover"), resp.hovered());
                        let p = ui.painter();
                        p.rect_filled(rect, 12.0, pal.accent.gamma_multiply(0.14 + 0.08 * hover));
                        p.rect_stroke(rect, 12.0, egui::Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                        let font = egui::FontId::proportional(15.0);
                        p.text(
                            rect.left_center() + egui::vec2(18.0, 0.0),
                            Align2::LEFT_CENTER,
                            "New here? A two-minute tour shows where everything is.",
                            font.clone(),
                            pal.text,
                        );
                        p.text(
                            rect.right_center() - egui::vec2(18.0, 0.0),
                            Align2::RIGHT_CENTER,
                            "Take the tour  →",
                            font,
                            pal.accent,
                        );
                        if resp.clicked() {
                            cmd = Some(Command::TakeTour);
                        }
                    }
                    // Where to start.
                    ui.add_space(26.0);
                    ui.label(RichText::new("START").small().strong().color(pal.text_dim));
                    ui.add_space(8.0);
                    let gap = 12.0;
                    let tile_w = (width - 3.0 * gap) / 4.0;
                    let keys = |c| self.keymap.shortcut_text(c);
                    let tiles = [
                        (
                            Icon::FolderOpen,
                            "Open Folder",
                            "A project on this computer",
                            keys(Command::OpenFolder),
                            Command::OpenFolder,
                        ),
                        (
                            Icon::SourceControl,
                            "Clone",
                            "A repository, from its URL",
                            keys(Command::GitClone),
                            Command::GitClone,
                        ),
                        (
                            Icon::Play,
                            "Sample Project",
                            "Python, C, Java, JS or Rust",
                            String::new(),
                            Command::NewSampleProject,
                        ),
                        (Icon::Plus, "New File", "Write now, save it later", keys(Command::NewFile), Command::NewFile),
                    ];
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for (icon, title, line, k, c) in tiles {
                            if tile(ui, tile_w, icon, title, line, &k) {
                                cmd = Some(c);
                            }
                        }
                    });
                    // Recent projects, as cards.
                    if !self.recent.projects.is_empty() {
                        ui.add_space(26.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("RECENT PROJECTS").small().strong().color(pal.text_dim));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.link(RichText::new("Clear...").small()).clicked() {
                                    recent_click = Some(RecentClick::Clear);
                                }
                                if ui.link(RichText::new("All...").small()).clicked() {
                                    recent_click = Some(RecentClick::More);
                                }
                            });
                        });
                        ui.add_space(8.0);
                        let card_w = (width - 2.0 * gap) / 3.0;
                        let projects: Vec<_> = self.recent.projects.iter().take(6).cloned().collect();
                        for row in projects.chunks(3) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = gap;
                                for entry in row {
                                    if let Some(click) = project_card(ui, card_w, entry) {
                                        recent_click = Some(click);
                                    }
                                }
                            });
                            ui.add_space(gap);
                        }
                    }
                    // Recent files, in a line each.
                    if !self.recent.files.is_empty() {
                        ui.add_space(14.0);
                        ui.label(RichText::new("RECENT FILES").small().strong().color(pal.text_dim));
                        ui.add_space(4.0);
                        for entry in self.recent.files.iter().take(5) {
                            ui.horizontal(|ui| {
                                icons::show_file(ui, &entry.name(), 15.0);
                                let link = ui.link(entry.name()).on_hover_text(entry.path.display().to_string());
                                if link.clicked() {
                                    recent_click = Some(RecentClick::Open(entry.path.clone()));
                                }
                                let folder = entry.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
                                ui.add(egui::Label::new(RichText::new(folder).small().color(pal.text_dim)).truncate());
                            });
                        }
                    }
                    // The keys worth knowing.
                    ui.add_space(28.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 18.0;
                        for (what, c, fallback) in [
                            ("Search anything", Command::SearchEverywhere, "Double Shift"),
                            ("Every command", Command::CommandPalette, ""),
                            ("Go to file", Command::QuickOpen, ""),
                            ("Terminal", Command::ToggleTerminal, ""),
                            ("Run this file", Command::RunCurrentFile, ""),
                        ] {
                            let k = self.keymap.shortcut_text(c);
                            let k = if k.is_empty() { fallback.to_owned() } else { k };
                            let r = ui.add(
                                egui::Label::new(RichText::new(format!("{what}  {k}")).small().color(pal.text_dim))
                                    .sense(Sense::click()),
                            );
                            if r.on_hover_text("Click to do it").clicked() {
                                cmd = Some(c);
                            }
                        }
                        if ui.link(RichText::new("Take the tour").small()).clicked() {
                            cmd = Some(Command::TakeTour);
                        }
                    });
                    ui.add_space(30.0);
                });
            });
        });
        if let Some(click) = recent_click {
            self.recent_click(click);
        }
        cmd
    }
}

/// A recent project's card: its badge, name, folder and a Git mark; × removes it.
fn project_card(ui: &mut egui::Ui, width: f32, entry: &coiman_core::project::recent::Entry) -> Option<RecentClick> {
    let pal = theme::colors();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 62.0), Sense::click());
    let hover = theme::eased(ui.ctx(), resp.id.with("hover"), resp.hovered());
    let there = entry.exists();
    let p = ui.painter();
    p.rect_filled(rect, 10.0, pal.widget_bg.lerp_to_gamma(pal.hover_bg, hover));
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, pal.border.lerp_to_gamma(pal.accent, hover)),
        egui::StrokeKind::Inside,
    );
    let name = entry.name();
    let (letters, [r, g, b]) = project_badge(&name);
    let badge = egui::Rect::from_min_size(rect.min + egui::vec2(12.0, 13.0), egui::vec2(36.0, 36.0));
    let color = if there { Color32::from_rgb(r, g, b) } else { pal.text_dim };
    p.rect_filled(badge, 8.0, color);
    p.text(badge.center(), Align2::CENTER_CENTER, letters, egui::FontId::proportional(14.0), Color32::WHITE);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text_x = badge.right() + 12.0;
    let clip = rect.with_min_x(text_x).with_max_x(rect.right() - 30.0);
    let name_color = if there { pal.text } else { pal.text_dim };
    p.with_clip_rect(clip).text(
        egui::pos2(text_x, rect.top() + 14.0),
        Align2::LEFT_TOP,
        &name,
        font.clone(),
        name_color,
    );
    let folder = entry.path.parent().map(|f| f.display().to_string()).unwrap_or_default();
    let note = if there { folder } else { format!("not found · {folder}") };
    let small = egui::FontId::proportional(font.size * 0.8);
    p.with_clip_rect(clip).text(egui::pos2(text_x, rect.top() + 35.0), Align2::LEFT_TOP, note, small, pal.text_dim);
    if entry.path.join(".git").exists() {
        let mark =
            egui::Rect::from_min_size(egui::pos2(rect.right() - 26.0, rect.top() + 10.0), egui::vec2(14.0, 14.0));
        icons::paint(p, mark, Icon::SourceControl, pal.text_dim);
    }
    // The × shows on hover, bottom right.
    let cross =
        egui::Rect::from_min_size(egui::pos2(rect.right() - 28.0, rect.bottom() - 28.0), egui::vec2(20.0, 20.0));
    let on_cross = hover > 0.5 && ui.ctx().pointer_hover_pos().is_some_and(|pos| cross.contains(pos));
    if hover > 0.0 {
        let c = if on_cross { pal.text } else { pal.text_dim.gamma_multiply(hover) };
        p.text(cross.center(), Align2::CENTER_CENTER, "×", egui::FontId::proportional(15.0), c);
    }
    let resp = resp.on_hover_text(entry.path.display().to_string());
    if resp.clicked() {
        return Some(if on_cross {
            RecentClick::Remove(entry.path.clone())
        } else {
            RecentClick::Open(entry.path.clone())
        });
    }
    None
}
