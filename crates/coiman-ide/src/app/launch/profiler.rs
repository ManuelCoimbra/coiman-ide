//! Run: Profile Current File, and the PROFILE panel: a flame graph of where the time went, and
//! the hottest functions, each a link to its code.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use coiman_core::language::lsp::types as lt;
use coiman_core::launch::profiler::{self, Frame, Profile};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::app::{CodeApp, PanelTab, repaint};
use crate::ui::theme;

/// A finished run: what it was of, what it printed, and the profile or why there's none.
type Finished = Result<(String, profiler::Ran), String>;

#[derive(Default)]
pub(crate) struct Profiler {
    /// The panel has been asked for: its tab stays until the app closes.
    pub(crate) used: bool,
    profile: Option<Profile>,
    /// What the profile is of.
    label: String,
    /// The profile before this one, to compare with, and what it was of.
    previous: Option<(Profile, String)>,
    /// The hottest list shows what changed since the previous profile.
    comparing: bool,
    /// Where its files are named from.
    base: Option<PathBuf>,
    running: Option<Receiver<Finished>>,
    error: Option<String>,
    /// The frame shown full width: child indices from the root.
    zoom: Vec<usize>,
    /// The header's buttons, acted on once the panel no longer borrows the profile.
    ask_profile: bool,
    ask_open: bool,
}

/// Height of one row of the flame graph.
const ROW: f32 = 20.0;

impl CodeApp {
    fn show_profile_panel(&mut self) {
        self.profiler.used = true;
        self.panel_tab = PanelTab::Profile;
        self.panel_visible = true;
    }

    /// Run: Profile Current File.
    pub(crate) fn profile_current_file(&mut self, ctx: &egui::Context) {
        if !self.save_before_run(ctx) {
            return;
        }
        let Some((file, source)) = self.docs.get(self.active).and_then(|d| d.path.clone().map(|p| (p, d.text())))
        else {
            self.status = Some("Open a saved file to profile".to_owned());
            return;
        };
        let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();
        if !self.allowed_to_run(&dir) {
            return;
        }
        let root = self.explorer.root.clone();
        let out_dir = std::env::temp_dir().join(format!("coiman-profile-{}", std::process::id()));
        let label = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let result = std::fs::create_dir_all(&out_dir)
                .map_err(|e| e.to_string())
                .and_then(|()| profiler::plan(&file, &source, root.as_deref(), &out_dir))
                .and_then(|plan| {
                    let _ = std::fs::remove_file(&plan.output);
                    profiler::run(&plan)
                })
                .map(|ran| (label, ran));
            let _ = tx.send(result);
            wake();
        });
        self.profiler.running = Some(rx);
        self.profiler.error = None;
        self.profiler.base = Some(dir);
        self.status = Some("Profiling… the program runs to its end".to_owned());
        self.show_profile_panel();
    }

    /// Profile: Open Profile File — collapsed stacks or `perf script` output.
    pub(crate) fn open_profile_file(&mut self) {
        let Some(path) = self
            .file_dialog("profile")
            .set_title("Open a profile (collapsed stacks or perf script output)")
            .pick_file()
        else {
            return;
        };
        self.dialog_chose("profile", &path);
        let read = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| profiler::parse(&t));
        match read {
            Ok(profile) => {
                self.keep_previous_profile();
                self.profiler.profile = Some(profile);
                self.profiler.label = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.profiler.base = path.parent().map(Path::to_path_buf);
                self.profiler.zoom.clear();
                self.profiler.error = None;
            }
            Err(e) => self.profiler.error = Some(format!("Couldn't read {}: {e}", path.display())),
        }
        self.show_profile_panel();
    }

    fn pump_profiler(&mut self) {
        let Some(rx) = &self.profiler.running else { return };
        let Ok(done) = rx.try_recv() else { return };
        self.profiler.running = None;
        let done = done.and_then(|(label, ran)| {
            // What the program printed goes to the Output panel; the Profile panel stays in front.
            self.begin_output(format!("Profile {label}"));
            for line in ran.printed.lines() {
                self.push_output(line.to_owned());
            }
            self.panel_tab = PanelTab::Profile;
            ran.profile.map(|p| (p, label))
        });
        match done {
            Ok((profile, label)) => {
                // A handful of samples says more about the run than the program: too short, or the
                // profiler started late (a first run, scanned by the antivirus).
                self.profiler.error = (profile.samples() < 10).then(|| {
                    format!(
                        "Only {} samples: the program ran too briefly, or the profiler couldn't follow it. \
                         Profile again, or give it more to do.",
                        profile.samples()
                    )
                });
                self.status =
                    Some(format!("Profiled {label}: {} samples (its output is in Output)", profile.samples()));
                self.keep_previous_profile();
                self.profiler.profile = Some(profile);
                self.profiler.label = label;
                self.profiler.zoom.clear();
            }
            Err(e) => {
                self.status = Some(format!("Profiling failed: {e}"));
                self.profiler.error = Some(e);
            }
        }
    }

    /// The profile in hand becomes the one to compare the next with.
    fn keep_previous_profile(&mut self) {
        if let Some(p) = self.profiler.profile.take() {
            self.profiler.previous = Some((p, std::mem::take(&mut self.profiler.label)));
        }
    }

    /// Where a profile's `file:line` is, if it's a file here.
    fn profile_place(&self, file: &str) -> Option<PathBuf> {
        let path = Path::new(file);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_path_buf());
        }
        [self.profiler.base.as_deref(), self.explorer.root.as_deref()]
            .into_iter()
            .flatten()
            .map(|dir| dir.join(path))
            .find(|p| p.is_file())
    }

    pub(crate) fn profile_panel(&mut self, ui: &mut egui::Ui) {
        self.pump_profiler();
        let pal = theme::colors();
        let mut open: Option<(String, u32)> = None;
        ui.horizontal(|ui| {
            if self.profiler.running.is_some() {
                ui.spinner();
                ui.label("Profiling: the program runs to its end");
            } else if let Some(p) = &self.profiler.profile {
                ui.label(RichText::new(format!("{} samples", p.samples())).strong());
                ui.label(RichText::new(format!("from {}", self.profiler.label)).color(pal.text_dim));
            } else {
                ui.label(RichText::new("Profile a file to see where its time goes").color(pal.text_dim));
            }
            if ui
                .button("Profile Current File")
                .on_hover_text("Python with py-spy; on Linux anything, with perf")
                .clicked()
            {
                self.profiler.ask_profile = true;
            }
            if ui.button("Open Profile…").on_hover_text("Collapsed stacks (a;b;c 12) or perf script output").clicked()
            {
                self.profiler.ask_open = true;
            }
            if let Some((_, before)) = &self.profiler.previous {
                ui.checkbox(&mut self.profiler.comparing, "Compare")
                    .on_hover_text(format!("Each function's share of the time before ({before}) and now"));
            }
            if !self.profiler.zoom.is_empty() && ui.button("Reset Zoom").clicked() {
                self.profiler.zoom.clear();
            }
        });
        if let Some(e) = &self.profiler.error {
            ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
        }
        let Some(profile) = &self.profiler.profile else {
            self.profile_buttons(ui.ctx());
            return;
        };
        let samples = profile.samples().max(1) as f32;
        let percent = |n: u64| format!("{:.1}%", n as f32 * 100.0 / samples);
        let mut zoom_to: Option<Vec<usize>> = None;
        ui.horizontal_top(|ui| {
            // The hottest functions, by time in themselves; the flame graph gets the rest.
            let list_w = (ui.available_width() * 0.3).clamp(220.0, 360.0);
            ui.vertical(|ui| {
                ui.set_width(list_w);
                let previous = self.profiler.previous.as_ref().filter(|_| self.profiler.comparing);
                if let Some((before, label)) = previous {
                    ui.label(RichText::new(format!("Before ({label}), now, change")).small().strong());
                    egui::ScrollArea::vertical().id_salt("profile_cmp").show(ui, |ui| {
                        for c in profiler::compare(profile, before).into_iter().take(60) {
                            let delta = (c.now - c.before) * 100.0;
                            let text = format!(
                                "{:>5.1}% {:>5.1}% {:>+6.1}  {}",
                                c.before * 100.0,
                                c.now * 100.0,
                                delta,
                                c.name
                            );
                            // Slower in red, faster in green.
                            let color = if delta > 0.5 {
                                Color32::from_rgb(0xE5, 0x6B, 0x5D)
                            } else if delta < -0.5 {
                                Color32::from_rgb(0x6B, 0xC2, 0x7A)
                            } else {
                                pal.text_dim
                            };
                            let mut resp = ui.add(
                                egui::Label::new(RichText::new(text).monospace().color(color))
                                    .sense(Sense::click())
                                    .truncate(),
                            );
                            if let Some((file, line)) = &c.place {
                                resp = resp.on_hover_text(format!("{file}:{line}"));
                                if resp.clicked() {
                                    open = Some((file.clone(), *line));
                                }
                            }
                        }
                    });
                    return;
                }
                ui.label(RichText::new("Hottest (self, with calls)").small().strong());
                egui::ScrollArea::vertical().id_salt("profile_hot").show(ui, |ui| {
                    for hot in profile.hottest().into_iter().take(60) {
                        // With the rate known, the time too: samples × the time each stands for.
                        let time = |n: u64| {
                            profile.rate.filter(|r| *r > 0).map_or_else(String::new, |rate| {
                                let t = n as f64 * 1000.0 / f64::from(rate);
                                if t >= 1000.0 { format!(" {:>5.1}s", t / 1000.0) } else { format!(" {:>4.0}ms", t) }
                            })
                        };
                        let text = format!(
                            "{:>6}{} {:>6}{}  {}",
                            percent(hot.own),
                            time(hot.own),
                            percent(hot.total),
                            time(hot.total),
                            hot.name
                        );
                        let mut resp =
                            ui.add(egui::Label::new(RichText::new(text).monospace()).sense(Sense::click()).truncate());
                        if let Some((file, line)) = &hot.place {
                            resp = resp.on_hover_text(format!("{file}:{line}"));
                            if resp.clicked() {
                                open = Some((file.clone(), *line));
                            }
                        }
                    }
                });
            });
            ui.separator();
            // The flame graph: callers above, what they called below, widths by time.
            let (shown, path) = zoomed(&profile.root, &self.profiler.zoom);
            let depth = deepest(shown);
            egui::ScrollArea::vertical().id_salt("profile_flame").show(ui, |ui| {
                let width = ui.available_width().max(100.0);
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(width, (depth + 1) as f32 * ROW), Sense::click());
                let mut boxes = Vec::new();
                place(shown, rect.left(), width, rect.top(), Vec::new(), &mut boxes);
                let painter = ui.painter_at(rect);
                let hover = response.hover_pos();
                let mut hovered: Option<&(Rect, Vec<usize>, &Frame)> = None;
                for b in &boxes {
                    let (r, _, frame) = b;
                    let lit = hover.is_some_and(|p| r.contains(p));
                    if lit {
                        hovered = Some(b);
                    }
                    painter.rect_filled(r.shrink(0.5), 2.0, flame_color(&frame.name, lit));
                    if r.width() > 28.0 {
                        let font = egui::FontId::proportional(12.0);
                        let galley = painter.layout(
                            frame.name.clone(),
                            font,
                            Color32::from_rgb(0x20, 0x16, 0x10),
                            r.width() - 6.0,
                        );
                        painter.galley(
                            Pos2::new(r.left() + 3.0, r.center().y - galley.size().y / 2.0),
                            galley,
                            Color32::BLACK,
                        );
                    }
                }
                if let Some((r, at, frame)) = hovered {
                    painter.rect_stroke(*r, 2.0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                    let place = frame.place.as_ref().map_or(String::new(), |(f, l)| format!("\n{f}:{l}"));
                    let tip = format!(
                        "{}{place}\n{} of samples ({} in itself)\nClick to zoom in{}",
                        frame.name,
                        percent(frame.total),
                        percent(frame.own),
                        if frame.place.is_some() { "; double-click for the code" } else { "" }
                    );
                    let resp = response.clone().on_hover_text(tip);
                    if resp.double_clicked() {
                        open = frame.place.clone();
                    } else if resp.clicked() {
                        // The top row (the frame zoomed into) goes back up one.
                        zoom_to = Some(if at.is_empty() {
                            let mut up = path.clone();
                            up.pop();
                            up
                        } else {
                            path.iter().chain(at).copied().collect()
                        });
                    }
                }
            });
        });
        if let Some(z) = zoom_to {
            self.profiler.zoom = z;
        }
        if let Some((file, line)) = open {
            match self.profile_place(&file) {
                Some(path) => {
                    let at = lt::Position::new(line.saturating_sub(1), 0);
                    self.jump_to(&path, lt::Range::new(at, at), false);
                }
                None => self.status = Some(format!("{file} isn't a file here")),
            }
        }
        self.profile_buttons(ui.ctx());
    }

    /// The header's buttons, acted on once the panel no longer borrows the profile.
    fn profile_buttons(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.profiler.ask_profile) {
            self.profile_current_file(ctx);
        }
        if std::mem::take(&mut self.profiler.ask_open) {
            self.open_profile_file();
        }
    }
}

/// The frame at `zoom` (as far as it still leads), and the path that reached it.
fn zoomed<'a>(root: &'a Frame, zoom: &[usize]) -> (&'a Frame, Vec<usize>) {
    let mut at = root;
    let mut path = Vec::new();
    for &i in zoom {
        match at.children.get(i) {
            Some(c) => {
                at = c;
                path.push(i);
            }
            None => break,
        }
    }
    (at, path)
}

fn deepest(f: &Frame) -> usize {
    f.children.iter().map(|c| 1 + deepest(c)).max().unwrap_or(0)
}

/// Lays `f` out at `x` with width `w`, and its children under it by their share.
fn place<'a>(f: &'a Frame, x: f32, w: f32, y: f32, at: Vec<usize>, out: &mut Vec<(Rect, Vec<usize>, &'a Frame)>) {
    out.push((Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, ROW)), at.clone(), f));
    let mut cx = x;
    for (i, c) in f.children.iter().enumerate() {
        let cw = w * c.total as f32 / f.total.max(1) as f32;
        if cw >= 1.0 {
            let mut path = at.clone();
            path.push(i);
            place(c, cx, cw, y + ROW, path, out);
        }
        cx += cw;
    }
}

/// Warm colors, the same for the same name, as flame graphs draw them.
fn flame_color(name: &str, lit: bool) -> Color32 {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
    let r = 205 + (h % 50) as u8;
    let g = 90 + ((h >> 8) % 120) as u8;
    let b = 40 + ((h >> 16) % 50) as u8;
    if lit {
        Color32::from_rgb(r.saturating_add(20), g.saturating_add(30), b.saturating_add(30))
    } else {
        Color32::from_rgb(r, g, b)
    }
}
