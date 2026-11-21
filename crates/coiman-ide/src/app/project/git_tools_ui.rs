//! Git Tools: Search History, Submodules, Hooks, and .gitignore (what each rule hides), in one
//! window with a tab for each. Every git call runs on a thread of its own.

use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::project::git::Repo;
use coiman_core::project::git_tools::{self, Found, Hook, HookState, Rule, SearchKind, SubState, Submodule};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
const GREEN: Color32 = Color32::from_rgb(0x57, 0xC0, 0x6A);
const AMBER: Color32 = Color32::from_rgb(0xF2, 0x9E, 0x4C);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Search,
    Submodules,
    Hooks,
    Ignore,
}

/// A matching line: file, + or -, text.
type Line = (String, char, String);
/// The ignore file, line, and rule that hide a path.
type Why = (String, usize, String);
/// What a rule would hide: untracked, then tracked paths.
type Preview = (Vec<String>, Vec<String>);

enum Msg {
    Found(Result<Vec<Found>, String>),
    Lines(String, Result<Vec<Line>, String>),
    Subs(Result<Vec<Submodule>, String>),
    Hooks(Result<(Vec<Hook>, Option<String>), String>),
    Rules(Result<Vec<Rule>, String>),
    Why(String, Result<Option<Why>, String>),
    Preview(String, Result<Preview, String>),
    Done(Result<String, String>, Tab),
}

pub(crate) struct GitToolsView {
    open: bool,
    tab: Tab,
    kind: SearchKind,
    query: String,
    within: String,
    all: bool,
    found: Option<Vec<Found>>,
    selected: Option<usize>,
    lines: Option<(String, Vec<Line>)>,
    subs: Option<Vec<Submodule>>,
    add_url: String,
    add_path: String,
    confirm_remove: Option<String>,
    hooks: Option<(Vec<Hook>, Option<String>)>,
    hook_output: Option<(String, Result<String, String>)>,
    rules: Option<Vec<Rule>>,
    expanded: Option<(String, usize)>,
    why: String,
    why_answer: Option<(String, Result<Option<Why>, String>)>,
    pattern: String,
    preview: Option<(String, Result<Preview, String>)>,
    busy: Option<String>,
    error: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for GitToolsView {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            tab: Tab::default(),
            kind: SearchKind::default(),
            query: String::new(),
            within: String::new(),
            all: false,
            found: None,
            selected: None,
            lines: None,
            subs: None,
            add_url: String::new(),
            add_path: String::new(),
            confirm_remove: None,
            hooks: None,
            hook_output: None,
            rules: None,
            expanded: None,
            why: String::new(),
            why_answer: None,
            pattern: String::new(),
            preview: None,
            busy: None,
            error: None,
            tx,
            rx,
        }
    }
}

enum Ask {
    Search,
    Select(usize),
    OpenFile(usize, String),
    Subs,
    Update(Option<String>),
    Latest(String),
    Add,
    Remove(String),
    Hooks,
    HookCreate(String),
    HookToggle(String),
    HookRun(String),
    HookEdit(std::path::PathBuf),
    Rules,
    Why,
    Preview,
    AddPattern,
    OpenIgnore(String, usize),
}

fn when(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

impl CodeApp {
    pub(crate) fn show_git_tools(&mut self, ctx: &egui::Context, tab: Tab) {
        if self.git.repo.is_none() {
            self.status = Some("Open a folder in a Git repository first".to_owned());
            return;
        }
        let v = &mut self.git_tools;
        v.open = true;
        v.tab = tab;
        v.error = None;
        match tab {
            Tab::Search => {}
            Tab::Submodules => self.git_tools_ask(ctx, Ask::Subs),
            Tab::Hooks => self.git_tools_ask(ctx, Ask::Hooks),
            Tab::Ignore => self.git_tools_ask(ctx, Ask::Rules),
        }
    }

    fn git_tools_spawn(&mut self, ctx: &egui::Context, what: &str, job: impl FnOnce(Repo) -> Msg + Send + 'static) {
        let Some(repo) = self.git.repo.clone() else { return };
        let (tx, wake) = (self.git_tools.tx.clone(), repaint(ctx));
        self.git_tools.busy = Some(what.to_owned());
        std::thread::spawn(move || {
            let _ = tx.send(job(repo));
            wake();
        });
    }

    fn git_tools_ask(&mut self, ctx: &egui::Context, ask: Ask) {
        let v = &mut self.git_tools;
        match ask {
            Ask::Search => {
                let (kind, query, all, within) = (v.kind, v.query.clone(), v.all, v.within.clone());
                v.selected = None;
                v.lines = None;
                self.git_tools_spawn(ctx, "Searching…", move |r| {
                    Msg::Found(git_tools::search(
                        &r,
                        kind,
                        &query,
                        all,
                        Some(within.as_str()).filter(|w| !w.is_empty()),
                    ))
                });
            }
            Ask::Select(i) => {
                v.selected = Some(i);
                v.lines = None;
                let hash = v.found.as_ref().and_then(|f| f.get(i)).map(|f| f.hash.clone());
                if let Some(hash) = hash
                    && matches!(v.kind, SearchKind::Change | SearchKind::Regex)
                {
                    let (query, regex) = (v.query.clone(), v.kind == SearchKind::Regex);
                    self.git_tools_spawn(ctx, "Reading…", move |r| {
                        let lines = git_tools::matching_lines(&r, &hash, &query, regex);
                        Msg::Lines(hash, lines)
                    });
                }
            }
            Ask::OpenFile(i, rel) => {
                if let Some(f) = v.found.as_ref().and_then(|f| f.get(i)) {
                    let (hash, short) = (f.hash.clone(), f.short.clone());
                    self.open_commit_file(ctx, hash, short, rel);
                }
            }
            Ask::Subs => self.git_tools_spawn(ctx, "Reading…", |r| Msg::Subs(git_tools::submodules(&r))),
            Ask::Update(path) => self.git_tools_spawn(ctx, "Fetching submodules…", move |r| {
                Msg::Done(git_tools::submodule_update(&r, path.as_deref()), Tab::Submodules)
            }),
            Ask::Latest(path) => self.git_tools_spawn(ctx, "Updating…", move |r| {
                Msg::Done(git_tools::submodule_to_latest(&r, &path), Tab::Submodules)
            }),
            Ask::Add => {
                let (url, path) = (v.add_url.clone(), v.add_path.clone());
                self.git_tools_spawn(ctx, "Adding…", move |r| {
                    Msg::Done(git_tools::submodule_add(&r, &url, &path), Tab::Submodules)
                });
            }
            Ask::Remove(path) => {
                v.confirm_remove = None;
                self.git_tools_spawn(ctx, "Removing…", move |r| {
                    Msg::Done(git_tools::submodule_remove(&r, &path), Tab::Submodules)
                });
            }
            Ask::Hooks => self.git_tools_spawn(ctx, "Reading…", |r| {
                Msg::Hooks(git_tools::hooks(&r).and_then(|h| Ok((h, git_tools::hooks_dir(&r)?.1))))
            }),
            Ask::HookCreate(name) => match self.git.repo.as_ref().map(|r| git_tools::hook_create(r, &name)) {
                Some(Ok(path)) => {
                    self.open_file(&path);
                    self.git_tools_ask(ctx, Ask::Hooks);
                }
                Some(Err(e)) => self.git_tools.error = Some(e),
                None => {}
            },
            Ask::HookToggle(name) => {
                if let Some(Err(e)) = self.git.repo.as_ref().map(|r| git_tools::hook_toggle(r, &name)) {
                    self.git_tools.error = Some(e);
                }
                self.git_tools_ask(ctx, Ask::Hooks);
            }
            Ask::HookRun(name) => {
                let label = name.clone();
                self.git_tools_spawn(ctx, "Running the hook…", move |r| {
                    let out = git_tools::hook_run(&r, &name);
                    Msg::Done(out.map(|o| format!("{label}\n{o}")), Tab::Hooks)
                });
            }
            Ask::HookEdit(path) => self.open_file(&path),
            Ask::Rules => {
                self.git_tools_spawn(ctx, "Reading the rules…", |r| Msg::Rules(git_tools::ignore_rules(&r)))
            }
            Ask::Why => {
                let path = v.why.trim().to_owned();
                self.git_tools_spawn(ctx, "Checking…", move |r| {
                    let answer = git_tools::why_ignored(&r, &path);
                    Msg::Why(path, answer)
                });
            }
            Ask::Preview => {
                let pattern = v.pattern.trim().to_owned();
                self.git_tools_spawn(ctx, "Looking…", move |r| {
                    let found = git_tools::preview_pattern(&r, &pattern);
                    Msg::Preview(pattern, found)
                });
            }
            Ask::AddPattern => {
                let pattern = v.pattern.trim().to_owned();
                match self.git.repo.as_ref().map(|r| git_tools::add_pattern(r, &pattern)) {
                    Some(Ok(())) => {
                        self.status = Some(format!("Added {pattern} to .gitignore"));
                        self.git_tools.pattern.clear();
                        self.git_tools.preview = None;
                        self.git_tools_ask(ctx, Ask::Rules);
                    }
                    Some(Err(e)) => self.git_tools.error = Some(e),
                    None => {}
                }
            }
            Ask::OpenIgnore(source, line) => {
                let Some(root) = self.git.repo.as_ref().map(|r| r.root.clone()) else { return };
                let path = if std::path::Path::new(&source).is_absolute() { source.into() } else { root.join(source) };
                self.open_file(&path);
                if let Some(doc) = self.active_doc() {
                    doc.goto_line(line);
                }
            }
        }
    }

    pub(crate) fn git_tools_window(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.git_tools.rx.try_recv() {
            let v = &mut self.git_tools;
            v.busy = None;
            match msg {
                Msg::Found(r) => match r {
                    Ok(f) => {
                        v.error = None;
                        v.found = Some(f);
                    }
                    Err(e) => v.error = Some(e),
                },
                Msg::Lines(hash, r) => match r {
                    Ok(l) => v.lines = Some((hash, l)),
                    Err(e) => v.error = Some(e),
                },
                Msg::Subs(r) => match r {
                    Ok(s) => v.subs = Some(s),
                    Err(e) => v.error = Some(e),
                },
                Msg::Hooks(r) => match r {
                    Ok(h) => v.hooks = Some(h),
                    Err(e) => v.error = Some(e),
                },
                Msg::Rules(r) => match r {
                    Ok(rules) => v.rules = Some(rules),
                    Err(e) => v.error = Some(e),
                },
                Msg::Why(path, r) => v.why_answer = Some((path, r)),
                Msg::Preview(p, r) => v.preview = Some((p, r)),
                Msg::Done(result, tab) => {
                    if tab == Tab::Hooks {
                        let (name, rest) = match &result {
                            Ok(t) => t
                                .split_once('\n')
                                .map_or((t.clone(), String::new()), |(n, r)| (n.to_owned(), r.to_owned())),
                            Err(_) => (String::new(), String::new()),
                        };
                        v.hook_output = Some((name, result.map(|_| rest)));
                    } else {
                        match result {
                            Ok(done) => self.status = Some(done),
                            Err(e) => self.git_tools.error = Some(e),
                        }
                        if tab == Tab::Submodules {
                            self.git_tools_ask(ctx, Ask::Subs);
                        }
                    }
                }
            }
        }
        if !self.git_tools.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut ask = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.72).min(1050.0), (room.y * 0.72).max(380.0));
        egui::Window::new(RichText::new("Git Tools").size(16.0).strong())
            .id(egui::Id::unique("git_tools_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .max_height(room.y - 40.0)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| ask = git_tools_ui(ui, &mut self.git_tools, room.y * 0.72 - 120.0));
        self.git_tools.open = open;
        if let Some(a) = ask {
            // A tab chosen: read what it shows.
            self.git_tools_ask(ctx, a);
        }
    }
}

fn git_tools_ui(ui: &mut egui::Ui, v: &mut GitToolsView, body: f32) -> Option<Ask> {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    let mut ask = None;
    ui.horizontal(|ui| {
        for (tab, label, load) in [
            (Tab::Search, "Search History", None),
            (Tab::Submodules, "Submodules", Some(Ask::Subs)),
            (Tab::Hooks, "Hooks", Some(Ask::Hooks)),
            (Tab::Ignore, ".gitignore", Some(Ask::Rules)),
        ] {
            if ui.selectable_label(v.tab == tab, label).clicked() && v.tab != tab {
                v.tab = tab;
                v.error = None;
                ask = load;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(b) = &v.busy {
                ui.spinner();
                ui.label(dim(b));
            }
        });
    });
    if let Some(e) = &v.error {
        ui.colored_label(RED, e);
    }
    ui.separator();
    let height = body.max(220.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| match v.tab {
        Tab::Search => search_ui(ui, v, &mut ask),
        Tab::Submodules => subs_ui(ui, v, &mut ask),
        Tab::Hooks => hooks_ui(ui, v, &mut ask),
        Tab::Ignore => ignore_ui(ui, v, &mut ask),
    });
    ask
}

fn search_ui(ui: &mut egui::Ui, v: &mut GitToolsView, ask: &mut Option<Ask>) {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("git_search_kind").selected_text(v.kind.label()).show_ui(ui, |ui| {
            for k in SearchKind::ALL {
                ui.selectable_value(&mut v.kind, k, k.label());
            }
        });
        let hint = match v.kind {
            SearchKind::Message => "words in the message",
            SearchKind::Change => "a name or text that came or went",
            SearchKind::Regex => r"a regular expression: fn \w+_test",
            SearchKind::Author => "a name or e-mail",
            SearchKind::Path => "src/parser.rs, docs/, *.toml",
        };
        let r = ui.add(egui::TextEdit::singleline(&mut v.query).hint_text(hint).desired_width(300.0));
        if v.kind != SearchKind::Path {
            ui.add(
                egui::TextEdit::singleline(&mut v.within).hint_text("only in: a file or folder").desired_width(160.0),
            );
        }
        ui.checkbox(&mut v.all, "All branches");
        if ui.button("Search").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            *ask = Some(Ask::Search);
        }
    });
    let Some(found) = &v.found else {
        ui.label(dim("Added or removed text finds where something came in or went out (git log -S)."));
        return;
    };
    ui.label(dim(&format!(
        "{} commits{}",
        found.len(),
        if found.len() >= git_tools::FOUND_MAX { " (the newest; narrow the search for older ones)" } else { "" }
    )));
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() * 0.5).max(320.0));
            egui::ScrollArea::vertical().id_salt("git_found").auto_shrink([false, false]).show(ui, |ui| {
                for (i, f) in found.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&f.short).monospace().color(pal.accent));
                        if ui.selectable_label(v.selected == Some(i), &f.subject).clicked() {
                            *ask = Some(Ask::Select(i));
                        }
                    });
                    ui.label(dim(&format!("    {} · {}", f.author, when(f.when))).small());
                }
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            let Some(i) = v.selected else {
                ui.label(dim("Choose a commit to see its files."));
                return;
            };
            let Some(f) = found.get(i) else { return };
            egui::ScrollArea::vertical().id_salt("git_found_detail").auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(&f.subject).strong());
                ui.label(dim(&format!("{} · {} · {}", f.hash, f.author, when(f.when))).small());
                ui.add_space(4.0);
                ui.label(dim("Click a file for what this commit changed in it."));
                for (status, file) in &f.files {
                    let color = match status {
                        'A' => GREEN,
                        'D' => RED,
                        _ => AMBER,
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(status.to_string()).monospace().color(color));
                        if ui.link(RichText::new(file).monospace()).clicked() {
                            *ask = Some(Ask::OpenFile(i, file.clone()));
                        }
                    });
                }
                if let Some((hash, lines)) = &v.lines
                    && *hash == f.hash
                {
                    ui.add_space(6.0);
                    ui.label(RichText::new("The lines that match").strong());
                    for (file, mark, text) in lines {
                        let color = if *mark == '+' { GREEN } else { RED };
                        ui.label(RichText::new(format!("{mark} {text}")).monospace().small().color(color))
                            .on_hover_text(file);
                    }
                }
            });
        });
    });
}

fn subs_ui(ui: &mut egui::Ui, v: &mut GitToolsView, ask: &mut Option<Ask>) {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    ui.horizontal(|ui| {
        if ui.button("Fetch all").on_hover_text("git submodule update --init --recursive").clicked() {
            *ask = Some(Ask::Update(None));
        }
        if ui.button("Refresh").clicked() {
            *ask = Some(Ask::Subs);
        }
    });
    match &v.subs {
        None => {
            ui.label(dim("Reading…"));
        }
        Some(subs) if subs.is_empty() => {
            ui.label(dim("No submodules in this repository."));
        }
        Some(subs) => {
            egui::Grid::new("git_subs").striped(true).num_columns(4).spacing([14.0, 6.0]).show(ui, |ui| {
                for s in subs {
                    let color = match s.state {
                        SubState::Ok => GREEN,
                        SubState::NotInitialized | SubState::Moved => AMBER,
                        SubState::Conflict => RED,
                    };
                    ui.label(RichText::new(&s.path).monospace().strong());
                    ui.label(RichText::new(s.state.label()).color(color))
                        .on_hover_text(format!("{} {}", s.commit, s.describe));
                    ui.label(
                        dim(&format!("{}{}", s.url, s.branch.as_ref().map(|b| format!(" ({b})")).unwrap_or_default()))
                            .small(),
                    );
                    ui.horizontal(|ui| {
                        if ui.small_button("Fetch").on_hover_text("At the recorded commit").clicked() {
                            *ask = Some(Ask::Update(Some(s.path.clone())));
                        }
                        if ui
                            .small_button("Newest")
                            .on_hover_text("At its branch's newest commit (then commit to record it)")
                            .clicked()
                        {
                            *ask = Some(Ask::Latest(s.path.clone()));
                        }
                        if v.confirm_remove.as_deref() == Some(s.path.as_str()) {
                            if ui.small_button(RichText::new("Remove it").color(RED)).clicked() {
                                *ask = Some(Ask::Remove(s.path.clone()));
                            }
                            if ui.small_button("Keep").clicked() {
                                v.confirm_remove = None;
                            }
                        } else if ui.small_button("Remove…").clicked() {
                            v.confirm_remove = Some(s.path.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        }
    }
    ui.separator();
    ui.label(RichText::new("Add a submodule").strong());
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut v.add_url)
                .hint_text("https://github.com/owner/lib.git")
                .desired_width(320.0),
        );
        ui.add(egui::TextEdit::singleline(&mut v.add_path).hint_text("into: vendor/lib").desired_width(160.0));
        if ui.button("Add").clicked() {
            *ask = Some(Ask::Add);
        }
    });
}

fn hooks_ui(ui: &mut egui::Ui, v: &mut GitToolsView, ask: &mut Option<Ask>) {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    let Some((hooks, custom)) = &v.hooks else {
        ui.label(dim("Reading…"));
        return;
    };
    if let Some(c) = custom {
        ui.label(
            RichText::new(format!("core.hooksPath is {c}: a tool such as Husky or Lefthook runs these.")).color(AMBER),
        );
    }
    ui.label(dim(
        "Scripts Git runs at points of its work. Off renames one so Git skips it; Run runs it now, as Git would.",
    ));
    if let Some((name, out)) = &v.hook_output {
        ui.add_space(2.0);
        match out {
            Ok(text) => {
                ui.label(RichText::new(format!("{name} ran:")).strong().color(GREEN));
                ui.label(RichText::new(text).monospace().small());
            }
            Err(e) => {
                ui.label(RichText::new(e).monospace().small().color(RED));
            }
        }
    }
    egui::ScrollArea::vertical().id_salt("git_hooks").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("git_hooks_grid").striped(true).num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
            for h in hooks {
                let (state, color) = match h.state {
                    HookState::On => ("on", GREEN),
                    HookState::Off => ("off", AMBER),
                    HookState::Sample => ("example only", pal.text_dim),
                    HookState::None => ("", pal.text_dim),
                };
                ui.label(RichText::new(&h.name).monospace());
                ui.label(RichText::new(state).color(color));
                ui.horizontal(|ui| match h.state {
                    HookState::On => {
                        if ui.small_button("Edit").clicked() {
                            *ask = Some(Ask::HookEdit(h.path.clone()));
                        }
                        if ui.small_button("Run").clicked() {
                            *ask = Some(Ask::HookRun(h.name.clone()));
                        }
                        if ui.small_button("Off").clicked() {
                            *ask = Some(Ask::HookToggle(h.name.clone()));
                        }
                    }
                    HookState::Off => {
                        if ui.small_button("On").clicked() {
                            *ask = Some(Ask::HookToggle(h.name.clone()));
                        }
                    }
                    _ => {
                        if ui.small_button("Create").on_hover_text("A shell script, opened to edit").clicked() {
                            *ask = Some(Ask::HookCreate(h.name.clone()));
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
}

fn ignore_ui(ui: &mut egui::Ui, v: &mut GitToolsView, ask: &mut Option<Ask>) {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    ui.horizontal(|ui| {
        ui.label("Why is");
        let r =
            ui.add(egui::TextEdit::singleline(&mut v.why).hint_text("a path: target/debug/app").desired_width(260.0));
        ui.label("ignored?");
        if ui.button("Check").clicked() || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
            *ask = Some(Ask::Why);
        }
        if let Some((path, answer)) = &v.why_answer {
            match answer {
                Ok(Some((source, line, pattern))) => {
                    ui.label(RichText::new(format!("{path}: {pattern}")).monospace().color(AMBER));
                    if ui.link(format!("{source}:{line}")).clicked() {
                        *ask = Some(Ask::OpenIgnore(source.clone(), *line));
                    }
                }
                Ok(None) => {
                    ui.label(RichText::new(format!("{path} isn't ignored")).color(GREEN));
                }
                Err(e) => {
                    ui.colored_label(RED, e);
                }
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("New rule");
        let r = ui.add(
            egui::TextEdit::singleline(&mut v.pattern).hint_text("*.log, build/, /secret.env").desired_width(220.0),
        );
        if ui.button("What would it hide?").clicked()
            || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
        {
            *ask = Some(Ask::Preview);
        }
        if !v.pattern.trim().is_empty() && ui.button("Add to .gitignore").clicked() {
            *ask = Some(Ask::AddPattern);
        }
    });
    if let Some((p, preview)) = &v.preview {
        match preview {
            Ok((untracked, tracked)) => {
                ui.label(dim(&format!(
                    "{p} would hide {} untracked {}",
                    untracked.len(),
                    if untracked.len() == 1 { "entry" } else { "entries" }
                )));
                if !untracked.is_empty() {
                    ui.label(
                        RichText::new(untracked.iter().take(12).cloned().collect::<Vec<_>>().join("   "))
                            .monospace()
                            .small(),
                    );
                }
                if !tracked.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "{} tracked {} too, and {} tracked: git rm --cached to stop tracking ({})",
                            tracked.len(),
                            if tracked.len() == 1 { "file matches" } else { "files match" },
                            if tracked.len() == 1 { "it stays" } else { "they stay" },
                            tracked.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                        ))
                        .color(AMBER),
                    );
                }
            }
            Err(e) => {
                ui.colored_label(RED, e);
            }
        }
    }
    ui.separator();
    let Some(rules) = &v.rules else {
        ui.label(dim("Reading…"));
        return;
    };
    ui.label(dim("Each rule and what it hides now (folders count once); a rule that hides nothing can go."));
    egui::ScrollArea::vertical().id_salt("git_rules").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("git_rules_grid").striped(true).num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
            for r in rules {
                if ui.link(dim(&format!("{}:{}", r.source, r.line))).clicked() {
                    *ask = Some(Ask::OpenIgnore(r.source.clone(), r.line));
                }
                ui.label(RichText::new(&r.pattern).monospace().strong());
                let key = (r.source.clone(), r.line);
                if r.count == 0 {
                    ui.label(RichText::new("hides nothing").color(pal.text_dim));
                } else {
                    let open = v.expanded.as_ref() == Some(&key);
                    let label = format!("{} {}", r.count, if r.count == 1 { "entry" } else { "entries" });
                    if ui.selectable_label(open, label).clicked() {
                        v.expanded = if open { None } else { Some(key.clone()) };
                    }
                }
                ui.end_row();
                if v.expanded.as_ref() == Some(&key) {
                    ui.label("");
                    ui.label(RichText::new(r.hidden.join("\n")).monospace().small().color(pal.text_dim));
                    ui.label("");
                    ui.end_row();
                }
            }
        });
    });
}
