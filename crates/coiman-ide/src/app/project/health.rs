//! The Code Health window: secrets left in files, the most complex functions, code written
//! twice, the dependencies' licences, and known vulnerabilities from `cargo audit` and
//! `npm audit`. Files are read on a thread when the window opens; the audits run only when
//! asked, since their tools fetch the advisory lists.

use std::path::PathBuf;
use std::sync::mpsc;

use coiman_core::analysis::packages::{self, Advisory, Kind, Licensed};
use coiman_core::analysis::{complexity, duplicates, secrets};
use eframe::egui::{self, RichText, Stroke};

use crate::app::CodeApp;
use crate::ui::theme;

/// Functions shown in the Complexity tab.
const TOP_FUNCTIONS: usize = 200;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Secrets,
    Complexity,
    Duplicates,
    Licences,
    Audits,
}

const TABS: [(Tab, &str); 5] = [
    (Tab::Secrets, "Secrets"),
    (Tab::Complexity, "Complexity"),
    (Tab::Duplicates, "Duplicates"),
    (Tab::Licences, "Licences"),
    (Tab::Audits, "Audits"),
];

/// What reading the files found.
struct Scan {
    root: PathBuf,
    secrets: Vec<secrets::Finding>,
    functions: Vec<complexity::Function>,
    duplicates: Vec<duplicates::Duplicate>,
    licences: Vec<Licensed>,
    licence_note: Option<String>,
    seconds: f32,
}

#[derive(Default)]
pub(crate) struct Health {
    open: bool,
    tab: Tab,
    scan: Option<Scan>,
    scanning: Option<mpsc::Receiver<Scan>>,
    audit: Option<(Vec<Advisory>, Vec<String>)>,
    auditing: Option<mpsc::Receiver<(Vec<Advisory>, Vec<String>)>>,
    /// Licences: only those that need a look (not permissive).
    licences_flagged_only: bool,
}

enum Action {
    Rescan,
    Audit,
    Open(PathBuf, usize),
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

impl CodeApp {
    pub(crate) fn show_code_health(&mut self, ctx: &egui::Context) {
        self.health.open = true;
        if self.health.scan.is_none() && self.health.scanning.is_none() {
            self.scan_code_health(ctx);
        }
    }

    fn scan_code_health(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else {
            self.status = Some("Open a folder to check its code health".to_owned());
            self.health.open = false;
            return;
        };
        // Cargo can run what a project's .cargo/config.toml names: only once it's trusted.
        let trusted = self.restricted_folder().is_none();
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let secrets = secrets::scan(&root);
            let mut functions = complexity::scan(&root);
            functions.truncate(TOP_FUNCTIONS);
            let duplicates = duplicates::scan(&root);
            let (licences, licence_note) = if trusted {
                packages::licences(&root)
            } else {
                let note = "Restricted Mode: trust the folder to list its Rust dependencies (Cargo would read \
                            its settings). npm's are read from node_modules.";
                (packages::npm_licences(&root), Some(note.to_owned()))
            };
            let seconds = started.elapsed().as_secs_f32();
            let _ = tx.send(Scan { root, secrets, functions, duplicates, licences, licence_note, seconds });
            wake.request_repaint();
        });
        self.health.scanning = Some(rx);
    }

    fn run_audits(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else { return };
        if self.restricted_folder().is_some() {
            let note = "Restricted Mode: trust the folder to run its audits (the tools read its settings)";
            self.health.audit = Some((Vec::new(), vec![note.to_owned()]));
            return;
        }
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(packages::audit(&root));
            wake.request_repaint();
        });
        self.health.auditing = Some(rx);
    }

    pub(crate) fn code_health_window(&mut self, ctx: &egui::Context) {
        if !self.health.open {
            return;
        }
        if let Some(done) = self.health.scanning.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.health.scanning = None;
            self.health.scan = Some(done);
        }
        if let Some(done) = self.health.auditing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.health.auditing = None;
            self.health.audit = Some(done);
        }
        let mut open = true;
        let mut action = None;
        let pal = theme::colors();
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.8).min(1100.0), (room.y * 0.78).max(400.0));
        egui::Window::new(RichText::new("Code Health").size(16.0).strong())
            .id(egui::Id::unique("code_health_window"))
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
            .show(ctx, |ui| action = self.health_ui(ui));
        self.health.open = open;
        match action {
            Some(Action::Rescan) => self.scan_code_health(ctx),
            Some(Action::Audit) => self.run_audits(ctx),
            Some(Action::Open(path, line)) => {
                self.open_file(&path);
                self.jump_to_symbol_line(line.saturating_sub(1));
            }
            None => {}
        }
    }

    fn health_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let mut action = None;
        let h = &mut self.health;
        ui.horizontal(|ui| {
            for (tab, name) in TABS {
                let count = h.scan.as_ref().map(|s| match tab {
                    Tab::Secrets => s.secrets.len(),
                    Tab::Complexity => s.functions.iter().filter(|f| f.complexity > 10).count(),
                    Tab::Duplicates => s.duplicates.len(),
                    Tab::Licences => {
                        s.licences.iter().filter(|l| packages::kind(&l.license) != Kind::Permissive).count()
                    }
                    Tab::Audits => h.audit.as_ref().map_or(0, |a| a.0.len()),
                });
                let label = match count {
                    Some(n) if n > 0 => format!("{name}  {n}"),
                    _ => name.to_owned(),
                };
                if ui.selectable_label(h.tab == tab, label).clicked() {
                    h.tab = tab;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = h.scanning.is_some();
                if ui.add_enabled(!busy, egui::Button::new("Scan again")).clicked() {
                    action = Some(Action::Rescan);
                }
                if let Some(s) = &h.scan {
                    ui.label(RichText::new(format!("{:.1} s", s.seconds)).color(pal.text_dim));
                }
            });
        });
        ui.separator();
        let Some(scan) = h.scan.as_ref() else {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading the project's files…");
            });
            return action;
        };
        let root = scan.root.clone();
        let open = |rel: &str, line: usize| Action::Open(root.join(rel), line);
        let dim = |text: String| RichText::new(text).color(pal.text_dim);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match h.tab {
            Tab::Secrets => {
                if scan.secrets.is_empty() {
                    ui.label("No secrets found in the project's files (Git-ignored ones aren't read).");
                    return;
                }
                ui.label(dim(format!(
                    "{} that look real. Remove them from the files, move them to settings or the environment, and \
                     replace any that were committed: they stay in Git's history.",
                    plural(scan.secrets.len(), "secret")
                )));
                ui.add_space(6.0);
                egui::Grid::new("health_secrets").striped(true).num_columns(3).show(ui, |ui| {
                    for f in &scan.secrets {
                        if ui.link(format!("{}:{}", f.path, f.line)).clicked() {
                            action = Some(open(&f.path, f.line));
                        }
                        ui.label(f.kind);
                        ui.label(RichText::new(&f.masked).monospace());
                        ui.end_row();
                    }
                });
            }
            Tab::Complexity => {
                ui.label(dim(
                    "Cyclomatic complexity: 1, plus one for each branch (if, loop, case, catch, &&, ||, ?:). Up to \
                     10 is easy to follow and test; above 20, consider splitting the function."
                        .to_owned(),
                ));
                ui.add_space(6.0);
                if scan.functions.is_empty() {
                    ui.label("No functions found in languages Coiman reads (Rust, Python, JavaScript, TypeScript, Java, C, C++, C#, Go).");
                    return;
                }
                egui::Grid::new("health_complexity").striped(true).num_columns(4).show(ui, |ui| {
                    ui.label(dim("Complexity".to_owned()));
                    ui.label(dim("Lines".to_owned()));
                    ui.label(dim("Function".to_owned()));
                    ui.label(dim("Where".to_owned()));
                    ui.end_row();
                    for f in &scan.functions {
                        let color = match complexity::rating(f.complexity) {
                            "simple" => pal.text,
                            "complex" => pal.modified,
                            _ => egui::Color32::from_rgb(0xE5, 0x53, 0x4B),
                        };
                        ui.label(RichText::new(f.complexity.to_string()).color(color).strong());
                        ui.label(f.lines.to_string());
                        ui.label(RichText::new(&f.name).monospace());
                        if ui.link(format!("{}:{}", f.path, f.line)).clicked() {
                            action = Some(open(&f.path, f.line));
                        }
                        ui.end_row();
                    }
                });
            }
            Tab::Duplicates => {
                if scan.duplicates.is_empty() {
                    ui.label(format!(
                        "No code written twice ({} or more lines alike, blank lines and comments aside).",
                        duplicates::MIN_LINES
                    ));
                    return;
                }
                let lines: usize = scan.duplicates.iter().map(|d| d.lines * (d.places.len() - 1)).sum();
                ui.label(dim(format!(
                    "{} of code written more than once, about {lines} lines that could be shared.",
                    plural(scan.duplicates.len(), "block")
                )));
                ui.add_space(6.0);
                for (i, d) in scan.duplicates.iter().enumerate() {
                    ui.label(RichText::new(format!("{} lines, in {} places", d.lines, d.places.len())).strong());
                    for (path, first, last) in &d.places {
                        if ui.link(format!("    {path}:{first}–{last}")).clicked() {
                            action = Some(open(path, *first));
                        }
                    }
                    if i + 1 < scan.duplicates.len() {
                        ui.add_space(6.0);
                    }
                }
            }
            Tab::Licences => {
                if let Some(note) = &scan.licence_note {
                    ui.label(RichText::new(note).color(pal.modified));
                }
                if scan.licences.is_empty() {
                    ui.label("No dependencies found (Cargo.toml, or npm's node_modules).");
                    return;
                }
                let mut counts = std::collections::BTreeMap::new();
                for l in &scan.licences {
                    *counts.entry(packages::kind(&l.license)).or_insert(0) += 1;
                }
                let summary: Vec<String> =
                    counts.iter().map(|(k, n)| format!("{} {}", n, k.label().to_lowercase())).collect();
                ui.horizontal(|ui| {
                    ui.label(dim(format!("{} dependencies: {}.", scan.licences.len(), summary.join(", "))));
                    ui.checkbox(&mut h.licences_flagged_only, "Only those to look at");
                });
                ui.add_space(6.0);
                egui::Grid::new("health_licences").striped(true).num_columns(4).show(ui, |ui| {
                    for l in &scan.licences {
                        let kind = packages::kind(&l.license);
                        if h.licences_flagged_only && kind == Kind::Permissive {
                            continue;
                        }
                        let color = match kind {
                            Kind::Permissive => pal.text_dim,
                            Kind::WeakCopyleft => pal.modified,
                            Kind::Copyleft | Kind::Unknown => egui::Color32::from_rgb(0xE5, 0x53, 0x4B),
                        };
                        ui.label(format!("{} {}", l.name, l.version));
                        ui.label(if l.license.is_empty() { "(none declared)" } else { &l.license });
                        ui.label(RichText::new(kind.label()).color(color));
                        ui.label(dim(l.ecosystem.to_owned()));
                        ui.end_row();
                    }
                });
            }
            Tab::Audits => {
                ui.horizontal(|ui| {
                    let busy = h.auditing.is_some();
                    let run = ui.add_enabled(!busy, egui::Button::new("Run the audits"));
                    if run.clicked() {
                        action = Some(Action::Audit);
                    }
                    if busy {
                        ui.spinner();
                    }
                    ui.label(dim(
                        "cargo audit beside a Cargo.lock, npm audit beside a package-lock.json. They download \
                         the lists of known vulnerabilities themselves."
                            .to_owned(),
                    ));
                });
                ui.add_space(6.0);
                let Some((advisories, notes)) = &h.audit else { return };
                for note in notes {
                    ui.label(RichText::new(note).color(pal.modified));
                }
                if advisories.is_empty() && notes.is_empty() {
                    ui.label("No known vulnerabilities in the dependencies.");
                }
                egui::Grid::new("health_audits").striped(true).num_columns(5).show(ui, |ui| {
                    for a in advisories {
                        let color = match packages::severity_rank(&a.severity) {
                            0 | 1 => egui::Color32::from_rgb(0xE5, 0x53, 0x4B),
                            2 => pal.modified,
                            _ => pal.text_dim,
                        };
                        let severity = if a.severity.is_empty() { "—" } else { &a.severity };
                        ui.label(RichText::new(severity).color(color).strong());
                        ui.label(format!("{} {}", a.package, a.version));
                        ui.label(&a.title);
                        ui.label(dim(a.fix.clone()));
                        if a.url.is_empty() {
                            ui.label(&a.id);
                        } else {
                            ui.hyperlink_to(&a.id, &a.url);
                        }
                        ui.end_row();
                    }
                });
            }
        });
        action
    }
}
