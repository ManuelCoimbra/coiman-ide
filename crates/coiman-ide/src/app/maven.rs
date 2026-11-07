//! Maven, as the VS Code Maven extension has it: the MAVEN section of the Explorer
//! (projects and modules, lifecycle phases, plugins and their goals, the dependency
//! tree), goals run in a terminal tab, custom goals with a history, the effective POM,
//! adding a dependency found on Maven Central, resolving dependencies, and new projects
//! from an archetype. Maven runs in the background where its answer is read back.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::SystemTime;

use coiman_core::build_tools::BuildTool;
use coiman_core::lsp::discovery::quiet_command;
use coiman_core::maven::{self, DepNode, DepState, Found, Mvn, Plugin, Project};
use coiman_core::run::RunPlan;
use egui::{Align, RichText, Sense};

use super::{CodeApp, SidebarView, repaint};
use crate::icons::{self, Icon};
use crate::palette::{InputKind, PickerKind};
use crate::theme;

const CUSTOM: &str = "Custom goal...";
const HISTORY_MAX: usize = 20;
const SCOPES: &[&str] = &["compile", "test", "provided", "runtime"];

/// A dependency tree, and whether Maven made it (else it was read from the local
/// repository here).
struct Tree {
    nodes: Vec<DepNode>,
    from_maven: bool,
}

/// Each project's pom, with its dependency tree.
type Trees = Vec<(PathBuf, Vec<DepNode>)>;

enum JobKind {
    EffectivePom(PathBuf),
    /// The project's pom, and the file Maven writes its tree to.
    Resolve(PathBuf, PathBuf),
    Describe(PathBuf, Plugin),
    /// The folder the project is made in, and its name.
    NewProject(PathBuf, String),
}

struct Job {
    kind: JobKind,
    title: String,
    rx: Receiver<Result<String, String>>,
}

pub(super) struct MavenState {
    projects: Vec<Project>,
    read_for: Option<PathBuf>,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    checked_at: f64,
    trees: HashMap<PathBuf, Tree>,
    resolving: Option<Receiver<Trees>>,
    job: Option<Job>,
    /// Goals Maven gave for plugins not known here: (pom, plugin artifactId).
    described: HashMap<(PathBuf, String), Vec<String>>,
    history: Vec<String>,
    /// The pom the palette is acting on.
    target: Option<PathBuf>,
    searching: Option<Receiver<Result<Vec<Found>, String>>>,
    found: Vec<Found>,
    chosen: Option<Found>,
    archetype: usize,
    group: String,
    expanded: HashSet<String>,
    pub(super) open: bool,
}

impl Default for MavenState {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            read_for: None,
            stamps: Vec::new(),
            checked_at: 0.0,
            trees: HashMap::new(),
            resolving: None,
            job: None,
            described: HashMap::new(),
            history: load_history(),
            target: None,
            searching: None,
            found: Vec::new(),
            chosen: None,
            archetype: 0,
            group: String::new(),
            expanded: HashSet::new(),
            open: true,
        }
    }
}

fn history_file() -> Option<PathBuf> {
    coiman_core::settings::config_dir().map(|d| d.join("maven_history.txt"))
}

fn load_history() -> Vec<String> {
    history_file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect())
        .unwrap_or_default()
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Runs Maven to its end on this thread: its output, or why it failed.
fn run_mvn(mvn: &Mvn, args: &[String], cwd: &Path) -> Result<String, String> {
    let out = quiet_command(&mvn.program)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run {}: {e}", mvn.program.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(text) }
}

fn state_note(node: &DepNode) -> Option<String> {
    match &node.state {
        DepState::Resolved => None,
        DepState::NotDownloaded => Some("not downloaded".to_owned()),
        DepState::Omitted { used } if *used == node.version => Some("omitted: duplicate".to_owned()),
        DepState::Omitted { used } => Some(format!("omitted: conflict with {used}")),
    }
}

impl CodeApp {
    /// Keeps the projects current: read again when the folder changes or a pom does
    /// (checked every 2 seconds), and picks up background work.
    pub(super) fn pump_maven(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let root = self.explorer.root.clone();
        if root != self.maven.read_for {
            self.read_maven(ctx);
        } else if now - self.maven.checked_at > 2.0 {
            self.maven.checked_at = now;
            let changed = self.maven.stamps.iter().any(|(p, t)| stamp(p) != *t)
                || (self.maven.projects.is_empty() && root.as_deref().is_some_and(|r| r.join("pom.xml").is_file()));
            if changed {
                self.read_maven(ctx);
                self.pom_changed();
            }
        }
        if let Some(rx) = &self.maven.resolving
            && let Ok(trees) = rx.try_recv()
        {
            self.maven.resolving = None;
            for (pom, nodes) in trees {
                // Maven's own tree stays until the pom changes.
                if !self.maven.trees.get(&pom).is_some_and(|t| t.from_maven) {
                    self.maven.trees.insert(pom, Tree { nodes, from_maven: false });
                }
            }
        }
        if let Some(rx) = &self.maven.searching
            && let Ok(result) = rx.try_recv()
        {
            self.maven.searching = None;
            match result {
                Ok(found) if found.is_empty() => self.status = Some("Nothing found on Maven Central".to_owned()),
                Ok(found) => {
                    let items = found.iter().map(|f| format!("{}:{}", f.group_id, f.artifact_id)).collect();
                    let details = found.iter().map(|f| f.version.clone()).collect();
                    self.maven.found = found;
                    self.palette.show_detailed_picker(
                        PickerKind::MavenFound,
                        items,
                        details,
                        "Select the dependency to add (latest version shown)",
                    );
                }
                Err(e) => self.status = Some(e),
            }
        }
        if let Some(job) = &self.maven.job
            && let Ok(result) = job.rx.try_recv()
        {
            let job = self.maven.job.take().expect("checked above");
            self.maven_job_done(ctx, job, result);
        }
        if self.maven.job.is_some() || self.maven.searching.is_some() || self.maven.resolving.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn read_maven(&mut self, ctx: &egui::Context) {
        let root = self.explorer.root.clone();
        self.maven.read_for = root.clone();
        self.maven.checked_at = ctx.input(|i| i.time);
        self.maven.projects = root.as_deref().map(maven::find_projects).unwrap_or_default();
        self.maven.stamps = maven::all_projects(&self.maven.projects)
            .iter()
            .map(|p| (p.pom_path.clone(), stamp(&p.pom_path)))
            .collect();
        self.maven.trees.clear();
        self.maven.described.clear();
        if self.maven.projects.is_empty() {
            return;
        }
        coiman_core::applog::info(&format!("maven: {} project(s)", self.maven.stamps.len()));
        let projects: Vec<Project> = maven::all_projects(&self.maven.projects).into_iter().cloned().collect();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let Some(repo) = maven::local_repository() else { return };
            let mut resolver = maven::Resolver::new(repo);
            let trees = projects.iter().map(|p| (p.pom_path.clone(), resolver.resolve(p))).collect();
            let _ = tx.send(trees);
            wake();
        });
        self.maven.resolving = Some(rx);
    }

    /// A pom changed: Java's language server reads the project again, as VS Code's Java
    /// extension asks it to.
    fn pom_changed(&mut self) {
        for (pom, _) in &self.maven.stamps {
            let Some(uri) = coiman_core::lsp::convert::uri_from_path(pom) else { continue };
            for client in &mut self.lsp.clients {
                if client.kind == coiman_core::lsp::servers::ServerKind::Jdtls && pom.starts_with(&client.root) {
                    client.notify("java/projectConfigurationUpdate", serde_json::json!({ "uri": uri.to_string() }));
                    coiman_core::applog::info("maven: pom changed, told jdtls");
                }
            }
        }
    }

    /// The project the palette's Maven commands act on: the one holding the active file
    /// (the deepest), else the first.
    fn maven_project(&self) -> Option<Project> {
        let all = maven::all_projects(&self.maven.projects);
        let active = self.docs.get(self.active).and_then(|d| d.path.clone());
        active
            .and_then(|path| {
                all.iter().filter(|p| path.starts_with(p.dir())).max_by_key(|p| p.dir().components().count()).copied()
            })
            .or(all.first().copied())
            .cloned()
    }

    fn project_by_pom(&self, pom: &Path) -> Option<Project> {
        maven::all_projects(&self.maven.projects).into_iter().find(|p| p.pom_path == pom).cloned()
    }

    /// Maven for a folder, or the offer to fetch it.
    fn mvn_for(&mut self, dir: &Path) -> Option<Mvn> {
        let found = maven::find_mvn(dir, self.explorer.root.as_deref());
        if found.is_none() {
            self.offer_build_tool(BuildTool::Maven);
        }
        found
    }

    /// A goal line (`clean install`) run in a terminal tab, in the project's folder.
    pub(super) fn run_maven_goal(&mut self, ctx: &egui::Context, pom: &Path, goal: &str) {
        let goal = match maven::clean_goal(goal) {
            Ok(g) => g,
            Err(e) => {
                self.status = Some(e);
                return;
            }
        };
        let dir = pom.parent().map(Path::to_path_buf).unwrap_or_default();
        let Some(mvn) = self.mvn_for(&dir) else { return };
        let plan = RunPlan { title: format!("mvn {goal}"), command: mvn.command(&goal), cwd: dir };
        self.start_run(ctx, &plan);
        // Lifecycle phases are always there: only typed goals are worth keeping.
        if !maven::LIFECYCLE.contains(&goal.as_str()) {
            self.maven.history.retain(|h| *h != goal);
            self.maven.history.insert(0, goal);
            self.maven.history.truncate(HISTORY_MAX);
            if let Some(file) = history_file() {
                let _ = std::fs::write(file, self.maven.history.join("\n"));
            }
        }
    }

    fn need_project(&mut self) -> Option<Project> {
        let p = self.maven_project();
        if p.is_none() {
            self.status = Some("No Maven project here: open a folder with a pom.xml".to_owned());
        }
        p
    }

    /// Maven: Run Goal... — phases, the plugins' goals, recent custom goals, or a new one.
    pub(super) fn maven_goal_picker(&mut self, pom: Option<PathBuf>) {
        let Some(project) = pom.and_then(|p| self.project_by_pom(&p)).or_else(|| self.need_project()) else { return };
        let mut items: Vec<String> = self.maven.history.clone();
        items.extend(maven::LIFECYCLE.iter().map(|s| (*s).to_owned()));
        for plugin in project.plugins() {
            for goal in self.goals_of(&project.pom_path, &plugin) {
                items.push(format!("{}:{goal}", plugin.prefix()));
            }
        }
        let mut seen = HashSet::new();
        items.retain(|i| seen.insert(i.clone()));
        items.push(CUSTOM.to_owned());
        self.maven.target = Some(project.pom_path.clone());
        self.palette.show_picker(PickerKind::MavenGoal, items, "", "Select a goal to run");
    }

    fn goals_of(&self, pom: &Path, plugin: &Plugin) -> Vec<String> {
        let known = plugin.known_goals();
        if !known.is_empty() {
            return known.iter().map(|g| (*g).to_owned()).collect();
        }
        self.maven.described.get(&(pom.to_path_buf(), plugin.artifact_id.clone())).cloned().unwrap_or_default()
    }

    pub(super) fn picked_maven_goal(&mut self, ctx: &egui::Context, label: &str) {
        let Some(pom) = self.maven.target.clone() else { return };
        if label == CUSTOM {
            self.palette.show_input(
                InputKind::MavenGoal,
                "",
                "Goals and options for mvn (e.g. clean install -DskipTests), then Enter",
            );
        } else {
            self.run_maven_goal(ctx, &pom, label);
        }
    }

    /// Maven: Custom Goal... — typed, then run.
    pub(super) fn maven_custom_goal(&mut self, pom: Option<PathBuf>) {
        let Some(project) = pom.and_then(|p| self.project_by_pom(&p)).or_else(|| self.need_project()) else { return };
        self.maven.target = Some(project.pom_path);
        let last = self.maven.history.first().cloned().unwrap_or_default();
        self.palette.show_input(
            InputKind::MavenGoal,
            &last,
            "Goals and options for mvn (e.g. clean install -DskipTests), then Enter",
        );
    }

    pub(super) fn maven_goal_typed(&mut self, ctx: &egui::Context, text: &str) {
        if let Some(pom) = self.maven.target.clone() {
            self.run_maven_goal(ctx, &pom, text);
        }
    }

    fn start_maven_job(
        &mut self,
        ctx: &egui::Context,
        kind: JobKind,
        title: String,
        mvn: Mvn,
        args: Vec<String>,
        cwd: PathBuf,
    ) {
        if self.maven.job.is_some() {
            self.status = Some("Maven is busy: wait for it to finish".to_owned());
            return;
        }
        if !self.allowed_to_run(&cwd) {
            return;
        }
        coiman_core::applog::info(&format!("maven job: {title}"));
        self.status = Some(format!("{title}..."));
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        let resolve_first = matches!(kind, JobKind::Resolve(..));
        std::thread::spawn(move || {
            let result = if resolve_first {
                run_mvn(&mvn, &["dependency:resolve".to_owned()], &cwd).and_then(|_| run_mvn(&mvn, &args, &cwd))
            } else {
                run_mvn(&mvn, &args, &cwd)
            };
            let _ = tx.send(result);
            wake();
        });
        self.maven.job = Some(Job { kind, title, rx });
    }

    /// Maven: Show Effective POM.
    pub(super) fn maven_effective_pom(&mut self, ctx: &egui::Context, pom: Option<PathBuf>) {
        let Some(project) = pom.and_then(|p| self.project_by_pom(&p)).or_else(|| self.need_project()) else { return };
        let dir = project.dir();
        let Some(mvn) = self.mvn_for(&dir) else { return };
        let out = std::env::temp_dir().join("rust-code-maven");
        let _ = std::fs::create_dir_all(&out);
        let file = out.join(format!("effective-pom-{}.xml", project.pom.artifact_id));
        let _ = std::fs::remove_file(&file);
        let args = maven::effective_pom_args(&file);
        self.start_maven_job(
            ctx,
            JobKind::EffectivePom(file),
            format!("Effective POM of {}", project.label()),
            mvn,
            args,
            dir,
        );
    }

    /// Maven: Resolve Dependencies — Maven downloads what's missing, then its own tree
    /// replaces the one read here.
    pub(super) fn maven_resolve(&mut self, ctx: &egui::Context, pom: Option<PathBuf>) {
        let Some(project) = pom.and_then(|p| self.project_by_pom(&p)).or_else(|| self.need_project()) else { return };
        let dir = project.dir();
        let Some(mvn) = self.mvn_for(&dir) else { return };
        let out = std::env::temp_dir().join("rust-code-maven");
        let _ = std::fs::create_dir_all(&out);
        let file = out.join(format!("tree-{}.txt", project.pom.artifact_id));
        let _ = std::fs::remove_file(&file);
        let args = maven::tree_args(&file);
        let title = format!("Resolving the dependencies of {}", project.label());
        self.start_maven_job(ctx, JobKind::Resolve(project.pom_path.clone(), file), title, mvn, args, dir);
    }

    fn maven_describe(&mut self, ctx: &egui::Context, pom: &Path, plugin: &Plugin) {
        let dir = pom.parent().map(Path::to_path_buf).unwrap_or_default();
        let Some(mvn) = self.mvn_for(&dir) else { return };
        let title = format!("Reading the goals of {}", plugin.artifact_id);
        let args = maven::describe_args(plugin);
        self.start_maven_job(ctx, JobKind::Describe(pom.to_path_buf(), plugin.clone()), title, mvn, args, dir);
    }

    fn maven_job_done(&mut self, ctx: &egui::Context, job: Job, result: Result<String, String>) {
        let output = match result {
            Ok(output) => output,
            Err(output) => {
                let why = maven::error_summary(&output);
                coiman_core::applog::info(&format!("maven job failed: {}", job.title));
                self.begin_output(job.title.clone());
                for line in output.lines() {
                    self.push_output(line.to_owned());
                }
                self.status = Some(format!("{} failed: {why}", job.title));
                return;
            }
        };
        coiman_core::applog::info(&format!("maven job done: {}", job.title));
        match job.kind {
            JobKind::EffectivePom(file) => {
                if file.is_file() {
                    self.open_file(&file);
                    self.status = Some(format!("{}: Maven's view of the pom, with all it inherits", job.title));
                } else {
                    self.status = Some(format!("{}: Maven wrote nothing", job.title));
                }
            }
            JobKind::Resolve(pom, file) => {
                let nodes = maven::parse_tree(&std::fs::read_to_string(&file).unwrap_or_default());
                let (all, _, _) = maven::tree_counts(&nodes);
                self.maven.trees.insert(pom.clone(), Tree { nodes, from_maven: true });
                self.maven.expanded.insert(format!("{}#deps", pom.display()));
                self.maven.open = true;
                self.status = Some(format!("Dependencies resolved by Maven: {all}"));
            }
            JobKind::Describe(pom, plugin) => {
                let goals = maven::parse_describe(&output);
                self.status = Some(format!("{}: {} goal(s)", plugin.artifact_id, goals.len()));
                self.maven.described.insert((pom, plugin.artifact_id), goals);
            }
            JobKind::NewProject(dir, name) => {
                let folder = dir.join(&name);
                let in_open_project =
                    self.explorer.root.as_deref().is_some_and(|r| dir.starts_with(r)) && dir.join("pom.xml").is_file();
                if folder.join("pom.xml").is_file() && in_open_project {
                    // A module of the open project: it stays open, and the module shows.
                    self.status = Some(format!("Added the module {name}"));
                    self.maven.checked_at = 0.0;
                    self.open_file(&folder.join("pom.xml"));
                } else if folder.join("pom.xml").is_file() {
                    self.status = Some(format!("Made {name}"));
                    self.open_folder(folder);
                } else {
                    self.status = Some(format!("Maven made no {name}/pom.xml"));
                }
            }
        }
        let _ = ctx;
    }

    // ----- adding a dependency --------------------------------------------------------

    /// Maven: Add Dependency... — search Maven Central.
    pub(super) fn maven_add_dependency(&mut self, pom: Option<PathBuf>) {
        let Some(project) = pom.and_then(|p| self.project_by_pom(&p)).or_else(|| self.need_project()) else { return };
        self.maven.target = Some(project.pom_path);
        self.palette.show_input(
            InputKind::MavenSearch,
            "",
            "Search Maven Central (e.g. guava, or org.junit.jupiter:junit-jupiter), then Enter",
        );
    }

    pub(super) fn maven_search(&mut self, ctx: &egui::Context, query: &str) {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.status = Some(format!("Searching Maven Central for {query}..."));
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(maven::search_central(&query));
            wake();
        });
        self.maven.searching = Some(rx);
    }

    pub(super) fn picked_maven_found(&mut self, label: &str) {
        let Some(found) =
            self.maven.found.iter().find(|f| format!("{}:{}", f.group_id, f.artifact_id) == label).cloned()
        else {
            return;
        };
        let likely_test = ["junit", "mockito", "assertj", "hamcrest", "testng"]
            .iter()
            .any(|t| found.group_id.contains(t) || found.artifact_id.contains(t));
        self.maven.chosen = Some(found);
        let items = SCOPES.iter().map(|s| (*s).to_owned()).collect();
        self.palette.show_picker(
            PickerKind::MavenScope,
            items,
            if likely_test { "test" } else { "compile" },
            "Select the scope",
        );
    }

    /// Adds the chosen dependency to the pom, in the editor (one undo step), and saves.
    pub(super) fn picked_maven_scope(&mut self, scope: &str) {
        let (Some(found), Some(pom)) = (self.maven.chosen.take(), self.maven.target.clone()) else { return };
        match self.doc_index_by_path(&pom) {
            Some(i) => self.active = i,
            None => self.open_file(&pom),
        }
        let Some(index) = self.doc_index_by_path(&pom) else { return };
        let text = self.docs[index].text();
        match maven::add_dependency(&text, &found, Some(scope)) {
            Ok((new, line)) => {
                let doc = &mut self.docs[index];
                let at = doc.buffer.line_start(line.min(doc.buffer.len_lines().saturating_sub(1)));
                doc.buffer.replace_all(&new, coiman_core::buffer::Selection { anchor: at, head: at, goal: None });
                // Selecting it scrolls it into view.
                doc.select(at, at);
                self.save_doc(index);
                self.status =
                    Some(format!("Added {}:{}:{} ({scope})", found.group_id, found.artifact_id, found.version));
            }
            Err(e) => self.status = Some(e),
        }
    }

    // ----- new projects ---------------------------------------------------------------

    /// Maven: New Project... — an archetype, a groupId, an artifactId, then a folder.
    pub(super) fn maven_new_project(&mut self) {
        let items = maven::ARCHETYPES.iter().map(|a| a.0.to_owned()).collect();
        self.palette.show_picker(PickerKind::MavenArchetype, items, "", "Select an archetype for the new project");
    }

    pub(super) fn picked_maven_archetype(&mut self, label: &str) {
        self.maven.archetype = maven::ARCHETYPES.iter().position(|a| a.0 == label).unwrap_or(0);
        self.palette.show_input(InputKind::MavenGroupId, "com.example", "groupId of the new project, then Enter");
    }

    pub(super) fn maven_group_typed(&mut self, group: &str) {
        let group = group.trim();
        if !maven::valid_id(group) {
            self.status = Some(format!("\"{group}\" isn't a groupId: letters, digits, . - _"));
            return;
        }
        self.maven.group = group.to_owned();
        self.palette.show_input(InputKind::MavenArtifactId, "demo", "artifactId (the project's folder), then Enter");
    }

    pub(super) fn maven_artifact_typed(&mut self, ctx: &egui::Context, artifact: &str) {
        let artifact = artifact.trim().to_owned();
        if !maven::valid_id(&artifact) {
            self.status = Some(format!("\"{artifact}\" isn't an artifactId: letters, digits, . - _"));
            return;
        }
        let Some(dir) = self.file_dialog("maven_new").set_title("Folder to make the project in").pick_folder() else {
            return;
        };
        self.dialog_chose("maven_new", &dir);
        if dir.join(&artifact).exists() {
            self.status = Some(format!("{} already exists", dir.join(&artifact).display()));
            return;
        }
        if let Some(why) = maven::new_project_blocked(&dir) {
            self.status = Some(why);
            return;
        }
        let Some(mvn) = self.mvn_for(&dir) else { return };
        let (_, g, a, v) = maven::ARCHETYPES[self.maven.archetype];
        let args = maven::archetype_args((g, a, v), &self.maven.group, &artifact);
        let title = format!("Making {artifact} from {a}");
        self.start_maven_job(ctx, JobKind::NewProject(dir.clone(), artifact), title, mvn, args, dir);
    }

    // ----- the Explorer section -------------------------------------------------------

    pub(super) fn has_maven(&self) -> bool {
        !self.maven.projects.is_empty()
    }

    pub(super) fn maven_header_buttons(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.maven.job.is_some() || self.maven.resolving.is_some() {
            ui.spinner();
        }
        if icons::button(ui, Icon::Refresh, "Refresh (reads the poms again)").clicked() {
            self.read_maven(ctx);
            self.pom_changed();
        }
        if icons::button(ui, Icon::Plus, "Add Dependency...").clicked() {
            self.maven_add_dependency(None);
        }
        if icons::button(ui, Icon::Play, "Run Goal...").clicked() {
            self.maven_goal_picker(None);
        }
    }

    pub(super) fn maven_view(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut actions: Vec<Action> = Vec::new();
        let projects = self.maven.projects.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("maven_tree").show(ui, |ui| {
            for project in &projects {
                self.project_rows(ui, project, 0, &mut actions);
            }
        });
        for action in actions {
            match action {
                Action::Goal(pom, goal) => self.run_maven_goal(&ctx, &pom, &goal),
                Action::Open(path) => self.open_file(&path),
                Action::Picker(pom) => self.maven_goal_picker(Some(pom)),
                Action::Custom(pom) => self.maven_custom_goal(Some(pom)),
                Action::Effective(pom) => self.maven_effective_pom(&ctx, Some(pom)),
                Action::Add(pom) => self.maven_add_dependency(Some(pom)),
                Action::Resolve(pom) => self.maven_resolve(&ctx, Some(pom)),
                Action::Describe(pom, plugin) => self.maven_describe(&ctx, &pom, &plugin),
                Action::Toggle(key) => {
                    if !self.maven.expanded.remove(&key) {
                        self.maven.expanded.insert(key);
                    }
                }
            }
        }
    }

    fn project_rows(&self, ui: &mut egui::Ui, project: &Project, depth: usize, actions: &mut Vec<Action>) {
        let pal = theme::colors();
        let pom = project.pom_path.clone();
        let key = pom.display().to_string();
        // Top projects start open; modules start closed.
        let closed = format!("{key}#closed");
        let open = if depth == 0 { !self.maven.expanded.contains(&closed) } else { self.maven.expanded.contains(&key) };
        let (resp, toggle) =
            row(ui, depth, Some(open), RichText::new(project.label()).color(pal.text), Some(Icon::Folder));
        let resp = resp.on_hover_text(format!("{}\n{}", project.pom.coordinates(), project.pom_path.display()));
        // A click on the name opens or closes it too; a double click opens its pom.
        if toggle || resp.clicked() {
            actions.push(Action::Toggle(if depth == 0 { closed } else { key.clone() }));
        }
        if resp.double_clicked() {
            actions.push(Action::Open(pom.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Run Goal...").clicked() {
                actions.push(Action::Picker(pom.clone()));
            }
            if ui.button("Custom Goal...").clicked() {
                actions.push(Action::Custom(pom.clone()));
            }
            ui.separator();
            if ui.button("Open POM").clicked() {
                actions.push(Action::Open(pom.clone()));
            }
            if ui.button("Show Effective POM").clicked() {
                actions.push(Action::Effective(pom.clone()));
            }
            if ui.button("Add Dependency...").clicked() {
                actions.push(Action::Add(pom.clone()));
            }
            if ui.button("Resolve Dependencies").clicked() {
                actions.push(Action::Resolve(pom.clone()));
            }
        });
        if !open {
            return;
        }
        // Lifecycle.
        let lk = format!("{key}#lifecycle");
        let lopen = self.maven.expanded.contains(&lk);
        let (r, t) = row(ui, depth + 1, Some(lopen), RichText::new("Lifecycle").color(pal.text), None);
        if t || r.clicked() {
            actions.push(Action::Toggle(lk.clone()));
        }
        if lopen {
            for phase in maven::LIFECYCLE {
                goal_row(ui, depth + 2, phase, &pom, actions);
            }
        }
        // Plugins.
        let pk = format!("{key}#plugins");
        let popen = self.maven.expanded.contains(&pk);
        let (r, t) = row(ui, depth + 1, Some(popen), RichText::new("Plugins").color(pal.text), None);
        if t || r.clicked() {
            actions.push(Action::Toggle(pk.clone()));
        }
        if popen {
            for plugin in project.plugins() {
                let k = format!("{key}#plugin#{}", plugin.artifact_id);
                let o = self.maven.expanded.contains(&k);
                let version = plugin.version.as_deref().map(|v| format!(" {v}")).unwrap_or_default();
                let text = RichText::new(format!("{}{version}", plugin.prefix())).color(pal.text);
                let (r, t) = row(ui, depth + 2, Some(o), text, None);
                let r = r.on_hover_text(format!("{}:{}", plugin.group_id, plugin.artifact_id));
                if t || r.clicked() {
                    actions.push(Action::Toggle(k.clone()));
                }
                if o {
                    let goals = self.goals_of(&pom, &plugin);
                    if goals.is_empty() {
                        let (r, _) =
                            row(ui, depth + 3, None, RichText::new("Ask Maven for its goals").color(pal.accent), None);
                        if r.on_hover_text("mvn help:describe").clicked() {
                            actions.push(Action::Describe(pom.clone(), plugin.clone()));
                        }
                    }
                    for goal in goals {
                        goal_row(ui, depth + 3, &format!("{}:{goal}", plugin.prefix()), &pom, actions);
                    }
                }
            }
        }
        // Dependencies.
        let dk = format!("{key}#deps");
        let dopen = self.maven.expanded.contains(&dk);
        let tree = self.maven.trees.get(&pom);
        let label = match tree {
            Some(t) => {
                let (all, missing, _) = maven::tree_counts(&t.nodes);
                let mut s = format!("Dependencies ({all})");
                if missing > 0 {
                    s.push_str(&format!(", {missing} not downloaded"));
                }
                s
            }
            None => "Dependencies".to_owned(),
        };
        let (r, t) = row(ui, depth + 1, Some(dopen), RichText::new(label).color(pal.text), None);
        let r = r.on_hover_text(match tree {
            Some(t) if t.from_maven => "As Maven resolved them (dependency:tree)",
            Some(_) => "Read from the poms in the local repository. Right-click: Resolve Dependencies, for Maven's own",
            None => "Reading...",
        });
        if t || r.clicked() {
            actions.push(Action::Toggle(dk.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Resolve Dependencies").clicked() {
                actions.push(Action::Resolve(pom.clone()));
            }
            if ui.button("Add Dependency...").clicked() {
                actions.push(Action::Add(pom.clone()));
            }
        });
        if dopen && let Some(tree) = tree {
            if tree.nodes.is_empty() {
                row(ui, depth + 2, None, RichText::new("None").color(pal.text_dim), None);
            }
            let repo = maven::local_repository();
            for (i, node) in tree.nodes.iter().enumerate() {
                self.dep_rows(ui, node, depth + 2, &format!("{dk}#{i}"), repo.as_deref(), actions);
            }
        }
        // Modules.
        for module in &project.modules {
            self.project_rows(ui, module, depth + 1, actions);
        }
    }

    fn dep_rows(
        &self,
        ui: &mut egui::Ui,
        node: &DepNode,
        depth: usize,
        key: &str,
        repo: Option<&Path>,
        actions: &mut Vec<Action>,
    ) {
        let pal = theme::colors();
        let has_children = !node.children.is_empty();
        let open = self.maven.expanded.contains(key);
        let mut text = match &node.classifier {
            Some(c) => format!("{}:{c}:{}", node.artifact_id, node.version),
            None => format!("{}:{}", node.artifact_id, node.version),
        };
        if !node.scope.is_empty() && node.scope != "compile" {
            text.push_str(&format!(" ({})", node.scope));
        }
        let note = state_note(node);
        if let Some(n) = &note {
            text.push_str(&format!(" [{n}]"));
        }
        let color = match node.state {
            DepState::Resolved => pal.text,
            DepState::NotDownloaded => pal.modified,
            DepState::Omitted { .. } => pal.text_dim,
        };
        let (r, t) = row(ui, depth, has_children.then_some(open), RichText::new(text).color(color), None);
        let r = r.on_hover_text(node.coordinates());
        if t || (has_children && r.clicked()) {
            actions.push(Action::Toggle(key.to_owned()));
        }
        if r.double_clicked()
            && let Some(repo) = repo
        {
            let file = maven::artifact_path(repo, &node.group_id, &node.artifact_id, &node.version, "pom");
            if file.is_file() {
                actions.push(Action::Open(file));
            }
        }
        if open {
            for (i, child) in node.children.iter().enumerate() {
                self.dep_rows(ui, child, depth + 1, &format!("{key}#{i}"), repo, actions);
            }
        }
    }

    pub(super) fn maven_refresh(&mut self, ctx: &egui::Context) {
        self.read_maven(ctx);
        self.pom_changed();
        self.status = Some(format!("Maven: {} project(s)", self.maven.stamps.len()));
    }

    pub(super) fn show_maven(&mut self) {
        self.sidebar_visible = true;
        self.sidebar_view = SidebarView::Explorer;
        self.maven.open = true;
        if !self.has_maven() {
            self.status = Some("No Maven project here: open a folder with a pom.xml".to_owned());
        }
    }

    /// The Maven fetched with the user's yes is ready.
    pub(super) fn maven_installed(&mut self) {
        self.status = Some("Maven is ready: run the goal again".to_owned());
    }
}

enum Action {
    Goal(PathBuf, String),
    Open(PathBuf),
    Picker(PathBuf),
    Custom(PathBuf),
    Effective(PathBuf),
    Add(PathBuf),
    Resolve(PathBuf),
    Describe(PathBuf, Plugin),
    Toggle(String),
}

/// One row of the tree: indent, a chevron when it opens, an icon, its text. Returns the
/// text's response and whether the chevron was clicked.
fn row(
    ui: &mut egui::Ui,
    depth: usize,
    open: Option<bool>,
    text: RichText,
    icon: Option<Icon>,
) -> (egui::Response, bool) {
    let pal = theme::colors();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let size = ui.text_style_height(&egui::TextStyle::Body);
        ui.add_space(depth as f32 * 12.0);
        let toggled = match open {
            Some(o) => icons::show(ui, if o { Icon::ChevronDown } else { Icon::ChevronRight }, size, pal.text_dim)
                .interact(Sense::click())
                .clicked(),
            None => {
                // As wide as a chevron and the gap after it, so rows line up.
                ui.add_space(size + ui.spacing().item_spacing.x);
                false
            }
        };
        if let Some(icon) = icon {
            icons::show(ui, icon, size, pal.accent);
        }
        let resp = ui.add(egui::Label::new(text).sense(Sense::click()).truncate());
        (resp, toggled)
    })
    .inner
}

/// A goal: click runs it.
fn goal_row(ui: &mut egui::Ui, depth: usize, goal: &str, pom: &Path, actions: &mut Vec<Action>) {
    let pal = theme::colors();
    ui.horizontal(|ui| {
        // The button first, from the right: a long goal is cut short before it.
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let run = icons::button(ui, Icon::Play, "Run");
            let r = ui
                .with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    row(ui, depth, None, RichText::new(goal).color(pal.text), None).0
                })
                .inner;
            if r.on_hover_text(format!("Click to run mvn {goal}")).clicked() || run.clicked() {
                actions.push(Action::Goal(pom.to_path_buf(), goal.to_owned()));
            }
        });
    });
}
