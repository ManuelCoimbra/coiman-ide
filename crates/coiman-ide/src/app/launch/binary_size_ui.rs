//! Build: Analyze Binary Size: what takes room in a program, by group (crate or namespace),
//! symbol and section, and what changed since it was last looked at this session.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use coiman_core::build::binary_size::{self, Report, human, human_change};
use eframe::egui::{self, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Groups,
    Symbols,
    Sections,
}

#[derive(Default)]
pub(crate) struct BinarySizeView {
    open: bool,
    tab: Tab,
    candidates: Vec<PathBuf>,
    chosen: Option<PathBuf>,
    report: Option<Result<Report, String>>,
    /// The report before the latest one, for each file looked at this session.
    earlier: HashMap<PathBuf, Report>,
    filter: String,
    running: Option<Receiver<Result<Report, String>>>,
}

/// How many symbols are drawn; the filter finds the rest.
const SHOWN: usize = 400;

/// Smaller than before (the color the rest of Coiman uses for good news).
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x57, 0xC0, 0x6A);

enum Action {
    Analyze(PathBuf),
    Choose,
    Tab(Tab),
    Group(String),
}

impl CodeApp {
    pub(crate) fn show_binary_size(&mut self, ctx: &egui::Context) {
        // The file open in the editor, if it's one; else the last one; else the newest build.
        let active = self.active_doc().and_then(|d| d.path.clone()).filter(|p| binary_size::can_open(p));
        let v = &mut self.binary_size;
        v.open = true;
        v.candidates = self.explorer.root.as_deref().map(binary_size::candidates).unwrap_or_default();
        let pick = active.or_else(|| v.chosen.clone()).or_else(|| v.candidates.first().cloned());
        if let Some(path) = pick {
            self.binary_size_analyze(ctx, path);
        }
    }

    fn binary_size_analyze(&mut self, ctx: &egui::Context, path: PathBuf) {
        let v = &mut self.binary_size;
        if !v.candidates.contains(&path) {
            v.candidates.insert(0, path.clone());
        }
        // The report being replaced becomes the one to compare with.
        if let Some(Ok(old)) = v.report.take()
            && old.path == path
        {
            v.earlier.insert(path.clone(), old);
        }
        v.chosen = Some(path.clone());
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(binary_size::analyze(&path));
            wake();
        });
        v.running = Some(rx);
    }

    pub(crate) fn binary_size_window(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.binary_size.running
            && let Ok(result) = rx.try_recv()
        {
            self.binary_size.running = None;
            self.binary_size.report = Some(result);
        }
        if !self.binary_size.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut action = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.75).min(980.0), (room.y * 0.7).max(360.0));
        egui::Window::new(RichText::new("Binary Size").size(16.0).strong())
            .id(egui::Id::unique("binary_size_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| action = self.binary_size_ui(ui));
        self.binary_size.open = open;
        match action {
            Some(Action::Analyze(path)) => self.binary_size_analyze(ctx, path),
            Some(Action::Choose) => {
                if let Some(path) =
                    self.file_dialog("binary_size").set_title("A program, library or object file").pick_file()
                {
                    self.dialog_chose("binary_size", &path);
                    self.binary_size_analyze(ctx, path);
                }
            }
            Some(Action::Tab(tab)) => self.binary_size.tab = tab,
            Some(Action::Group(group)) => {
                self.binary_size.filter = group;
                self.binary_size.tab = Tab::Symbols;
            }
            None => {}
        }
    }

    fn binary_size_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
        let mut action = None;
        let root = self.explorer.root.clone();
        let v = &mut self.binary_size;
        let shown = |p: &PathBuf| match root.as_deref().and_then(|r| p.strip_prefix(r).ok()) {
            Some(rel) => rel.display().to_string(),
            None => p.display().to_string(),
        };
        ui.horizontal(|ui| {
            let current = v.chosen.as_ref().map_or_else(|| "Choose a file…".to_owned(), shown);
            egui::ComboBox::from_id_salt("binary_size_file")
                .width(ui.available_width() - 250.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for c in &v.candidates {
                        if ui.selectable_label(v.chosen.as_ref() == Some(c), shown(c)).clicked() {
                            action = Some(Action::Analyze(c.clone()));
                        }
                    }
                    if v.candidates.is_empty() {
                        ui.label(dim("Nothing built in this folder's target or build folders."));
                    }
                });
            if ui.button("Choose…").clicked() {
                action = Some(Action::Choose);
            }
            if let Some(path) = &v.chosen
                && ui.button("Analyze again").on_hover_text("After a build: shows what changed").clicked()
            {
                action = Some(Action::Analyze(path.clone()));
            }
        });
        let report = match &v.report {
            _ if v.running.is_some() => {
                ui.label(dim("Reading…"));
                return action;
            }
            None => {
                ui.label(dim("Build the project, or choose a program, a library or an object file."));
                return action;
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e.as_str()).color(pal.bracket_unexpected));
                return action;
            }
            Some(Ok(r)) => r,
        };
        let earlier = v.earlier.get(&report.path);

        // The summary: what it is, and its size.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&report.format).strong());
            ui.label(dim(&format!("· file {}", human(report.file_size))));
            ui.label(dim(&format!("· loaded {}", human(report.loaded()))));
            if let Some(m) = report.memory {
                ui.label(RichText::new(format!("· flash {} · RAM {}", human(m.flash), human(m.ram))).strong())
                    .on_hover_text(
                        "Flash: code, read-only data and data's start values. RAM: data and zero-filled data.",
                    );
            }
            if let Some(old) = earlier {
                let (a, b) = (old.loaded(), report.loaded());
                let color = if b > a { pal.bracket_unexpected } else { GREEN };
                ui.label(RichText::new(format!("· {} since last time", human_change(a, b))).color(color));
            }
        });
        let parts = report.parts();
        let total: u64 = parts.iter().filter(|(p, _)| p.loaded()).map(|(_, n)| *n).sum::<u64>().max(1);
        ui.horizontal_wrapped(|ui| {
            for (part, n) in &parts {
                ui.label(dim(&format!("{} {}", part.label(), human(*n))));
            }
        });
        if let Some(why) = report.no_symbols {
            ui.label(RichText::new(why).color(pal.modified));
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (Tab::Groups, format!("Groups  {}", report.groups().len())),
                (Tab::Symbols, format!("Symbols  {}", report.symbols.len())),
                (Tab::Sections, format!("Sections  {}", report.sections.len())),
            ] {
                if ui.selectable_label(v.tab == tab, label).clicked() {
                    action = Some(Action::Tab(tab));
                }
            }
            if v.tab == Tab::Symbols {
                ui.add(egui::TextEdit::singleline(&mut v.filter).hint_text("Filter").desired_width(220.0));
            }
        });
        ui.separator();
        let bar = |ui: &mut egui::Ui, part: u64, whole: u64| {
            let width = 120.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 3.0, pal.border);
            let fill = (part as f32 / whole.max(1) as f32).clamp(0.0, 1.0);
            let mut done = rect;
            done.set_width((width * fill).max(1.0));
            ui.painter().rect_filled(done, 3.0, pal.accent);
        };
        let percent = |n: u64, whole: u64| format!("{:.1}%", n as f64 * 100.0 / whole.max(1) as f64);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match v.tab {
            Tab::Groups => {
                let groups = report.groups();
                let symbols: u64 = groups.iter().map(|g| g.1).sum();
                let changes: HashMap<String, (u64, u64)> = earlier
                    .map(|old| {
                        binary_size::group_changes(old, report).into_iter().map(|(g, a, b)| (g, (a, b))).collect()
                    })
                    .unwrap_or_default();
                egui::Grid::new("binary_size_groups").striped(true).num_columns(5).show(ui, |ui| {
                    for (group, n, count) in &groups {
                        if ui.link(RichText::new(group).monospace()).on_hover_text("Its symbols").clicked() {
                            action = Some(Action::Group(group.clone()));
                        }
                        ui.label(human(*n));
                        bar(ui, *n, symbols);
                        let what = if *count == 1 { "symbol" } else { "symbols" };
                        ui.label(dim(&format!("{}  · {count} {what}", percent(*n, symbols))));
                        match changes.get(group) {
                            Some((a, b)) => {
                                let color = if b > a { pal.bracket_unexpected } else { GREEN };
                                ui.label(RichText::new(human_change(*a, *b)).color(color));
                            }
                            None => {
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
                // Groups that went away entirely.
                for (group, (a, b)) in &changes {
                    if *b == 0 {
                        ui.label(RichText::new(format!("{group}: gone ({})", human_change(*a, *b))).color(GREEN));
                    }
                }
            }
            Tab::Symbols => {
                let needle = v.filter.to_lowercase();
                let matching: Vec<_> = report
                    .symbols
                    .iter()
                    .filter(|s| {
                        needle.is_empty() || s.name.to_lowercase().contains(&needle) || s.group.to_lowercase() == needle
                    })
                    .collect();
                let biggest = matching.first().map_or(1, |s| s.size);
                egui::Grid::new("binary_size_symbols").striped(true).num_columns(4).show(ui, |ui| {
                    for s in matching.iter().take(SHOWN) {
                        ui.label(human(s.size));
                        bar(ui, s.size, biggest);
                        let mut name = s.name.clone();
                        if name.chars().count() > 110 {
                            name = name.chars().take(109).collect::<String>() + "…";
                        }
                        ui.label(RichText::new(name).monospace().small()).on_hover_text(format!(
                            "{}\n{} · {}",
                            s.name,
                            s.section,
                            if s.code { "code" } else { "data" }
                        ));
                        ui.label(dim(&s.section));
                        ui.end_row();
                    }
                });
                if matching.len() > SHOWN {
                    ui.label(dim(&format!("…and {} smaller ones; the filter finds them.", matching.len() - SHOWN)));
                }
            }
            Tab::Sections => {
                egui::Grid::new("binary_size_sections").striped(true).num_columns(5).show(ui, |ui| {
                    for s in &report.sections {
                        ui.label(RichText::new(&s.name).monospace());
                        ui.label(human(s.size));
                        bar(ui, s.size, if s.part.loaded() { total } else { report.file_size });
                        ui.label(dim(s.part.label()));
                        ui.label(dim(&if s.part.loaded() { percent(s.size, total) } else { "not loaded".to_owned() }));
                        ui.end_row();
                    }
                });
            }
        });
        action
    }
}
