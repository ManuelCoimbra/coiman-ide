//! The Code Health window: secrets left in files, the most complex functions, code written
//! twice, the dependencies (their tree, versions pinned twice, what's out of date), their
//! licences, and known vulnerabilities from `cargo audit` and `npm audit`. Files are read on a thread when the window opens; the audits run only when
//! asked, since their tools fetch the advisory lists.

use std::path::PathBuf;
use std::sync::mpsc;

use coiman_core::analysis::lockfiles::{self, Ecosystem, Lock, Outdated};
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
    Dependencies,
    Licences,
    Audits,
}

const TABS: [(Tab, &str); 6] = [
    (Tab::Secrets, "Secrets"),
    (Tab::Complexity, "Complexity"),
    (Tab::Duplicates, "Duplicates"),
    (Tab::Dependencies, "Dependencies"),
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
    /// The lockfiles, read, and what couldn't be.
    locks: Vec<Lock>,
    lock_notes: Vec<String>,
    seconds: f32,
}

/// What the package managers said is out of date, and what they couldn't say.
type Updates = (Vec<Outdated>, Vec<String>);

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
    updates: Option<Updates>,
    checking: Option<mpsc::Receiver<Updates>>,
}

enum Action {
    Rescan,
    Audit,
    Open(PathBuf, usize),
    CheckUpdates,
    /// A package manager's command, run in a terminal: its title, program and arguments.
    Package(String, String, Vec<String>),
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
            let (locks, lock_notes) = lockfiles::read_locks(&root);
            let seconds = started.elapsed().as_secs_f32();
            let _ = tx.send(Scan {
                root,
                secrets,
                functions,
                duplicates,
                licences,
                licence_note,
                locks,
                lock_notes,
                seconds,
            });
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

    /// Asks the package managers what's out of date; they ask their registries themselves.
    fn check_updates(&mut self, ctx: &egui::Context) {
        let Some(root) = self.explorer.root.clone() else { return };
        if self.restricted_folder().is_some() {
            let note = "Restricted Mode: trust the folder to ask its package managers (they read its settings)";
            self.health.updates = Some((Vec::new(), vec![note.to_owned()]));
            return;
        }
        let python = self.python_env().map(|e| e.python.clone());
        let (tx, rx) = mpsc::channel();
        let wake = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(lockfiles::outdated(&root, python.as_deref()));
            wake.request_repaint();
        });
        self.health.checking = Some(rx);
    }

    /// Runs a package manager's command in a terminal, where its output can be read.
    fn run_package_command(&mut self, ctx: &egui::Context, title: &str, program: &str, args: &[String]) {
        let Some(root) = self.explorer.root.clone() else { return };
        if !self.allowed_to_run(&root) {
            return;
        }
        // `npm` is `npm.cmd` on Windows: the terminal needs the file itself.
        let program = coiman_core::language::lsp::servers::which(program)
            .map_or_else(|| program.to_owned(), |p| p.to_string_lossy().into_owned());
        let env = self.environment_variables();
        let task = crate::ui::terminal::Task { title, program: &program, args, env: &env, cwd: &root, scan: None };
        let scrollback = self.settings.terminal.scrollback;
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = crate::app::PanelTab::Terminal;
                self.status = Some(format!("{title}: when it's done, Scan again shows the new versions"));
            }
            Err(e) => self.status = Some(format!("Couldn't start {program}: {e}")),
        }
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
        if let Some(done) = self.health.checking.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.health.checking = None;
            self.health.updates = Some(done);
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
            Some(Action::CheckUpdates) => self.check_updates(ctx),
            Some(Action::Package(title, program, args)) => self.run_package_command(ctx, &title, &program, &args),
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
        let python = self.python_env().map(|e| e.python.clone());
        let h = &mut self.health;
        ui.horizontal(|ui| {
            for (tab, name) in TABS {
                let count = h.scan.as_ref().map(|s| match tab {
                    Tab::Secrets => s.secrets.len(),
                    Tab::Complexity => s.functions.iter().filter(|f| f.complexity > 10).count(),
                    Tab::Duplicates => s.duplicates.len(),
                    Tab::Dependencies => {
                        s.locks.iter().map(|l| l.conflicts().len()).sum::<usize>()
                            + h.updates.as_ref().map_or(0, |u| u.0.len())
                    }
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
            Tab::Dependencies => {
                let python = python.clone();
                dependencies_ui(ui, scan, &h.updates, h.checking.is_some(), python.as_deref(), &mut action);
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

/// The Dependencies tab: what's out of date (once asked), versions pinned twice, and each
/// lockfile's tree.
fn dependencies_ui(
    ui: &mut egui::Ui,
    scan: &Scan,
    updates: &Option<Updates>,
    checking: bool,
    python: Option<&std::path::Path>,
    action: &mut Option<Action>,
) {
    let pal = theme::colors();
    let dim = |text: String| RichText::new(text).color(pal.text_dim);
    let red = egui::Color32::from_rgb(0xE5, 0x53, 0x4B);
    for note in &scan.lock_notes {
        ui.label(RichText::new(note).color(pal.modified));
    }
    // Packages each lockfile's project names itself, which it can remove.
    let direct = |eco: Ecosystem, name: &str| {
        eco == Ecosystem::Pip
            || scan
                .locks
                .iter()
                .filter(|l| l.ecosystem == eco)
                .any(|l| l.direct().iter().any(|&d| l.packages[d].name == name))
    };

    ui.horizontal(|ui| {
        if ui.add_enabled(!checking, egui::Button::new("Check for updates")).clicked() {
            *action = Some(Action::CheckUpdates);
        }
        if checking {
            ui.spinner();
        }
        ui.label(dim(
            "cargo update --dry-run, npm outdated and pip list --outdated: they ask their registries themselves."
                .to_owned(),
        ));
    });
    if let Some((list, notes)) = updates {
        for note in notes {
            ui.label(RichText::new(note).color(pal.modified));
        }
        if list.is_empty() && notes.is_empty() {
            ui.label("Everything is up to date.");
        }
        if !list.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(format!("Out of date: {}", list.len())).strong());
            egui::Grid::new("health_outdated").striped(true).num_columns(7).show(ui, |ui| {
                ui.label(dim("".to_owned()));
                ui.label(dim("Package".to_owned()));
                ui.label(dim("Now".to_owned()));
                ui.label(dim("Update gets".to_owned()));
                ui.label(dim("Newest".to_owned()));
                ui.label(dim("".to_owned()));
                ui.label(dim("".to_owned()));
                ui.end_row();
                for o in list {
                    ui.label(dim(o.ecosystem.label().to_owned()));
                    ui.label(&o.name);
                    ui.label(&o.current);
                    ui.label(o.wanted.clone().unwrap_or_else(|| "—".to_owned()));
                    match &o.latest {
                        Some(latest) => {
                            let size = lockfiles::bump(&o.current, latest);
                            let color = match size {
                                "major" => red,
                                "minor" => pal.modified,
                                _ => pal.text_dim,
                            };
                            ui.label(RichText::new(format!("{latest}  {size}")).color(color)).on_hover_text(
                                "A major step may break the code that uses it; a minor or patch one shouldn't.",
                            );
                        }
                        None => {
                            ui.label("—");
                        }
                    }
                    // Update: as far as the project's requirement allows (pip: to the newest).
                    let can_update =
                        o.wanted.as_ref().is_some_and(|w| w != &o.current) || o.ecosystem == Ecosystem::Pip;
                    match lockfiles::update_command(o.ecosystem, &o.name, python).filter(|_| can_update) {
                        Some((program, args)) => {
                            let tip = format!("Runs `{program} {}`", args.join(" "));
                            if ui.small_button("Update").on_hover_text(tip).clicked() {
                                *action = Some(Action::Package(format!("Update {}", o.name), program, args));
                            }
                        }
                        None => {
                            ui.label(dim("change the requirement".to_owned())).on_hover_text(
                                "The newest is outside what the project asks for: raise the version in its manifest.",
                            );
                        }
                    }
                    match lockfiles::remove_command(o.ecosystem, &o.name, python)
                        .filter(|_| direct(o.ecosystem, &o.name))
                    {
                        Some((program, args)) => {
                            let tip = format!("Runs `{program} {}`", args.join(" "));
                            if ui.small_button("Remove").on_hover_text(tip).clicked() {
                                *action = Some(Action::Package(format!("Remove {}", o.name), program, args));
                            }
                        }
                        None => {
                            ui.label("");
                        }
                    }
                    ui.end_row();
                }
            });
        }
    }
    ui.add_space(8.0);
    if scan.locks.is_empty() {
        ui.label("No lockfile here (Cargo.lock, package-lock.json, poetry.lock or uv.lock).");
        return;
    }
    for lock in &scan.locks {
        let file = lock.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let conflicts = lock.conflicts();
        ui.label(
            RichText::new(format!(
                "{file}: {} packages, {} pinned at more than one version",
                lock.packages.len(),
                conflicts.len()
            ))
            .strong(),
        );
        if !conflicts.is_empty() {
            egui::CollapsingHeader::new(format!("More than one version ({})", conflicts.len()))
                .id_salt(("dep_conflicts", &file))
                .show(ui, |ui| {
                    for c in &conflicts {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&c.name).strong());
                            for (version, by) in &c.versions {
                                // A few names; the rest counted, all of them on hover.
                                const SHOWN: usize = 6;
                                let who = match by.len() {
                                    0 => "the project".to_owned(),
                                    n if n <= SHOWN => by.join(", "),
                                    n => format!("{} and {} more", by[..SHOWN].join(", "), n - SHOWN),
                                };
                                ui.label(RichText::new(version).color(pal.modified));
                                ui.label(dim(format!("wanted by {who}"))).on_hover_text(by.join(
                                    "
",
                                ));
                            }
                        });
                    }
                });
        }
        egui::CollapsingHeader::new("Tree").id_salt(("dep_tree", &file)).show(ui, |ui| {
            let mut path = Vec::new();
            for &r in &lock.roots {
                tree_node(ui, lock, r, &mut path, &file);
            }
        });
        ui.add_space(6.0);
    }
}

/// A package and, opened, what it needs. A package already above it on the way is only named.
fn tree_node(ui: &mut egui::Ui, lock: &Lock, at: usize, path: &mut Vec<usize>, file: &str) {
    let p = &lock.packages[at];
    let label = format!("{} {}", p.name, p.version);
    if p.deps.is_empty() || path.contains(&at) {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.label(label);
        });
        return;
    }
    path.push(at);
    let id = (file, path.clone());
    egui::CollapsingHeader::new(format!("{label}  ({})", p.deps.len())).id_salt(id).show(ui, |ui| {
        for &d in &p.deps {
            tree_node(ui, lock, d, path, file);
        }
    });
    path.pop();
}
