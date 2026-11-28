//! API Docs: the project's API documentation, made on this computer. With the project's own
//! tool (`cargo doc`, pydoc, javadoc) through approved programs; as one Markdown page from the
//! doc comments in its files, with links back to the code; and a page of its dependencies, with
//! their versions and licences, from the files already on disk. Nothing is fetched.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod comments;
pub mod deps;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, JobResult, LogLine, LogView, Row, Span,
};
use comments::FileDocs;
use deps::Dep;

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x3FB950;
const DIM: u32 = 0x8B949E;
/// Files read for doc comments, at most.
const MAX_FILES: u32 = 3000;
/// Files read in one call; the rest wait for the next.
const STEP: usize = 40;
/// Python modules given to pydoc, at most.
const MAX_MODULES: usize = 300;

#[derive(Default, Clone, Copy, PartialEq)]
enum Tool {
    #[default]
    Rust,
    Python,
    Java,
}

const TOOLS: [(Tool, &str, &str); 3] = [
    (Tool::Rust, "rust", "Rust: cargo doc"),
    (Tool::Python, "python", "Python: pydoc"),
    (Tool::Java, "java", "Java: javadoc"),
];

fn tool_label(t: Tool) -> &'static str {
    TOOLS.iter().find(|(x, ..)| *x == t).map_or("", |(_, _, l)| l)
}

#[derive(Default, PartialEq, Clone, Copy)]
enum Stage {
    #[default]
    Idle,
    Doc(Tool),
    Metadata,
}

/// Reading doc comments, a step at a time.
#[derive(Default)]
struct Scan {
    files: Vec<String>,
    next: usize,
    done: Vec<FileDocs>,
}

/// Dependencies read so far, waiting for `cargo metadata`.
#[derive(Default)]
struct DepsWork {
    groups: BTreeMap<String, Vec<Dep>>,
    notes: Vec<String>,
}

#[derive(Default)]
struct State {
    tool: Option<Tool>,
    private: bool,
    job: Option<u64>,
    stage: Stage,
    /// Where the generated documentation's front page is (absolute).
    out: Option<String>,
    note: Option<(String, u32)>,
    output: Vec<String>,
    /// The Markdown page made last: the file it would be saved as, its title, its text.
    page: Option<(String, String, String)>,
    confirm_replace: bool,
    scan: Option<Scan>,
    deps: Option<DepsWork>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: String::new() }
}

fn note(text: impl Into<String>, color: u32) {
    with(|s| s.note = Some((text.into(), color)));
}

fn exists(path: &str) -> bool {
    host::file_size(path).is_ok()
}

fn project_name() -> String {
    host::workspace_path()
        .ok()
        .and_then(|p| p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().map(str::to_owned))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "the project".into())
}

/// A `file://` address for a path on this computer.
pub fn file_url(path: &str) -> String {
    let p = path.replace('\\', "/");
    let mut out = String::from("file://");
    if !p.starts_with('/') {
        out.push('/');
    }
    for c in p.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            c => out.push(c),
        }
    }
    out
}

/// The tool the open folder's files suggest.
fn detect() -> Tool {
    if exists("Cargo.toml") {
        Tool::Rust
    } else if ["pom.xml", "build.gradle", "build.gradle.kts"].iter().any(|f| exists(f))
        || host::list_files(".java", 1).is_ok_and(|f| !f.is_empty())
    {
        Tool::Java
    } else if ["pyproject.toml", "setup.py", "requirements.txt"].iter().any(|f| exists(f))
        || host::list_files(".py", 1).is_ok_and(|f| !f.is_empty())
    {
        Tool::Python
    } else {
        Tool::Rust
    }
}

fn tool() -> Tool {
    if let Some(t) = with(|s| s.tool) {
        return t;
    }
    let t = detect();
    with(|s| s.tool = Some(t));
    t
}

// ----- generating with the project's tool -----------------------------------------------------

/// Python's modules in the project, by import name.
pub fn python_modules(files: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for f in files {
        if comments::skipped(f) || f.contains("site-packages") || f.starts_with('.') || f.contains("/.") {
            continue;
        }
        let Some(stem) = f.strip_suffix(".py") else { continue };
        let stem = stem.strip_prefix("src/").unwrap_or(stem);
        let mut parts: Vec<&str> = stem.split('/').collect();
        if parts.last() == Some(&"__init__") {
            parts.pop();
        }
        let identifier = |p: &&str| {
            !p.is_empty()
                && !p.starts_with(|c: char| c.is_ascii_digit())
                && p.chars().all(|c| c.is_alphanumeric() || c == '_')
        };
        if parts.is_empty() || !parts.iter().all(identifier) || parts.last() == Some(&"__main__") {
            continue;
        }
        out.insert(parts.join("."));
    }
    out.into_iter().take(MAX_MODULES).collect()
}

/// Writes each module's page with pydoc, into the folder named first; run in the project.
const PYDOC: &str = "import sys,os,pydoc;out=sys.argv[1];sys.path[:0]=[os.path.abspath('src'),os.path.abspath('.')];\
                     os.makedirs(out,exist_ok=True);os.chdir(out);[pydoc.writedoc(m) for m in sys.argv[2:]]";

fn generate(choice: Option<Tool>) -> Result<(), String> {
    if with(|s| s.job.is_some()) {
        return Err("Something is already running: wait for it, or stop it".into());
    }
    if let Some(t) = choice {
        with(|s| s.tool = Some(t));
    }
    let t = tool();
    let private = with(|s| s.private);
    let (program, args, shown): (&str, Vec<String>, String) = match t {
        Tool::Rust => {
            if !exists("Cargo.toml") {
                return Err("No Cargo.toml at the top of the open folder: nothing for cargo doc".into());
            }
            let mut a = vec!["doc".to_owned(), "--no-deps".to_owned()];
            if private {
                a.push("--document-private-items".into());
            }
            let shown = format!("cargo {}", a.join(" "));
            ("cargo", a, shown)
        }
        Tool::Python => {
            let modules = python_modules(&host::list_files(".py", MAX_FILES)?);
            if modules.is_empty() {
                return Err("No Python modules in the open folder".into());
            }
            let _ = host::data_remove("pydoc");
            let out = host::data_folder("pydoc")?;
            let shown = format!("python -m pydoc -w {} (into {out})", modules.join(" "));
            let mut a = vec!["-c".to_owned(), PYDOC.to_owned(), out];
            a.extend(modules);
            ("python", a, shown)
        }
        Tool::Java => {
            let all = host::list_files(".java", MAX_FILES)?;
            let main: Vec<String> = all.iter().filter(|f| f.starts_with("src/main/java/")).cloned().collect();
            let files =
                if main.is_empty() { all.into_iter().filter(|f| !comments::skipped(f)).collect() } else { main };
            if files.is_empty() {
                return Err("No Java files in the open folder".into());
            }
            let _ = host::data_remove("javadoc");
            let out = host::data_folder("javadoc")?;
            // The files go in a list file: a long command line is cut short on Windows.
            let list: String = files.iter().map(|f| format!("\"{f}\"\n")).collect();
            host::data_write("javadoc-files.txt", &list)?;
            let list_path = format!("{}/javadoc-files.txt", host::data_folder("")?.trim_end_matches(['/', '\\']));
            let a = vec![
                "-d".to_owned(),
                out.clone(),
                "-quiet".into(),
                "-Xdoclint:none".into(),
                format!("@{}", list_path.replace('\\', "/")),
            ];
            let shown =
                format!("javadoc -d {out} over {} file{}", files.len(), if files.len() == 1 { "" } else { "s" });
            ("javadoc", a, shown)
        }
    };
    let id = host::start_job(program, &args, "").map_err(|e| {
        if e.contains("isn't on PATH") { format!("{program} isn't installed (or isn't on PATH)") } else { e }
    })?;
    with(|s| {
        s.job = Some(id);
        s.stage = Stage::Doc(t);
        s.out = None;
        s.output = vec![format!("$ {shown}")];
        s.note = Some((format!("Running {shown}…"), DIM));
    });
    Ok(())
}

/// cargo doc's "Generated …/index.html" line: where the front page went.
pub fn generated_path(output: &str) -> Option<String> {
    output.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("Generated ")?;
        let path = rest.split(" and ").next().unwrap_or(rest).trim();
        path.ends_with(".html").then(|| path.to_owned())
    })
}

/// pydoc's "wrote name.html" lines: the modules written.
pub fn pydoc_written(output: &str) -> Vec<String> {
    output.lines().filter_map(|l| l.trim().strip_prefix("wrote ")?.strip_suffix(".html").map(str::to_owned)).collect()
}

fn pydoc_index(project: &str, modules: &[String]) -> String {
    let items: String = modules.iter().map(|m| format!("<li><a href=\"{m}.html\">{m}</a></li>\n")).collect();
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>{project}: Python modules</title></head>\n\
         <body><h1>{project}: Python modules</h1>\n<ul>\n{items}</ul>\n</body></html>\n"
    )
}

fn doc_done(t: Tool, job: JobResult) {
    let all = format!("{}\n{}", job.stdout, job.stderr);
    with(|s| s.output.extend(all.lines().filter(|l| !l.trim().is_empty()).map(str::to_owned)));
    let failed = |why: String| note(why, RED);
    match job.exit_code {
        None => return failed("Stopped".into()),
        Some(c) if c != 0 => {
            return failed(format!("{} failed (exit code {c}): see its output below", tool_label(t)));
        }
        _ => {}
    }
    let found = match t {
        Tool::Rust => generated_path(&all).or_else(|| {
            let name = host::read_file("Cargo.toml").ok().and_then(|toml| {
                toml.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "name").then(|| v.trim().trim_matches('"').replace('-', "_"))
                })
            })?;
            let root = host::workspace_path().ok()?;
            Some(format!("{root}/target/doc/{name}/index.html"))
        }),
        Tool::Python => {
            let written = pydoc_written(&job.stdout);
            if written.is_empty() {
                None
            } else {
                let index = pydoc_index(&project_name(), &written);
                host::data_write("pydoc/index.html", &index)
                    .ok()
                    .and_then(|_| host::data_folder("pydoc").ok())
                    .map(|d| format!("{}/index.html", d.trim_end_matches(['/', '\\'])))
            }
        }
        Tool::Java => {
            host::data_folder("javadoc").ok().map(|d| format!("{}/index.html", d.trim_end_matches(['/', '\\'])))
        }
    };
    let Some(path) = found else {
        return failed(format!("{} made no pages: see its output below", tool_label(t)));
    };
    with(|s| s.out = Some(path.clone()));
    match host::open_url(&file_url(&path)) {
        Ok(()) => note(format!("Done: opened {path} in the browser"), GREEN),
        Err(_) => note("Done. The editor opens only web addresses in the browser: copy the path below into it.", GREEN),
    }
}

// ----- the page from doc comments ------------------------------------------------------------

fn start_scan() -> Result<(), String> {
    let files: Vec<String> = host::list_files("", MAX_FILES)?
        .into_iter()
        .filter(|f| comments::lang_of(f).is_some() && !comments::skipped(f))
        .collect();
    if files.is_empty() {
        return Err("No Rust, Python, JavaScript, TypeScript, Java, C or C++ files in the open folder".into());
    }
    let n = files.len();
    with(|s| {
        s.scan = Some(Scan { files, ..Scan::default() });
        s.note = Some((format!("Reading the doc comments of {n} files…"), DIM));
    });
    host::continue_later("scan");
    Ok(())
}

fn scan_step() {
    let batch: Vec<String> = with(|s| {
        let Some(scan) = s.scan.as_mut() else { return vec![] };
        let end = (scan.next + STEP).min(scan.files.len());
        let b = scan.files[scan.next..end].to_vec();
        scan.next = end;
        b
    });
    let mut found = Vec::new();
    for path in batch {
        let Some(lang) = comments::lang_of(&path) else { continue };
        let Ok(text) = host::read_file(&path) else { continue };
        let items = comments::items(lang, &text);
        if items.is_empty() {
            continue;
        }
        // The editor's parser names functions surely; the code is read for the rest.
        let names: HashMap<u32, String> =
            host::syntax_functions(&path).unwrap_or_default().into_iter().map(|f| (f.line, f.name)).collect();
        let items = items
            .into_iter()
            .map(|i| {
                let name = names.get(&i.line).cloned().unwrap_or_else(|| comments::name_of(&i.sig));
                (i, name)
            })
            .collect();
        found.push(FileDocs { path, lang, items });
    }
    let finished = with(|s| {
        let Some(scan) = s.scan.as_mut() else { return true };
        scan.done.extend(found);
        scan.next >= scan.files.len()
    });
    if !finished {
        host::continue_later("scan");
        return;
    }
    let Some(scan) = with(|s| s.scan.take()) else { return };
    let project = project_name();
    let md = comments::page(&project, &scan.done);
    let title = format!("API of {project}");
    host::show_markdown(&title, &md);
    let n: usize = scan.done.iter().map(|f| f.items.len()).sum();
    note(format!("{n} doc comments from {} of {} files, opened as a page", scan.done.len(), scan.files.len()), GREEN);
    with(|s| {
        s.page = Some(("docs/API.md".into(), title, md));
        s.confirm_replace = false;
    });
}

// ----- dependencies --------------------------------------------------------------------------

fn start_deps() -> Result<(), String> {
    if with(|s| s.job.is_some()) {
        return Err("Something is already running: wait for it, or stop it".into());
    }
    let mut work = DepsWork::default();
    node_deps(&mut work);
    python_deps(&mut work);
    if let Ok(lock) = host::read_file("Cargo.lock") {
        work.groups.insert("Rust (Cargo.lock)".into(), deps::cargo_lock(&lock));
        // Licences come from cargo metadata, offline: only what is already on this computer.
        let args = ["metadata", "--offline", "--format-version", "1"].map(str::to_owned);
        match host::start_job("cargo", &args, "") {
            Ok(id) => {
                with(|s| {
                    s.job = Some(id);
                    s.stage = Stage::Metadata;
                    s.deps = Some(work);
                    s.note = Some(("Reading licences with cargo metadata --offline…".into(), DIM));
                });
                return Ok(());
            }
            Err(e) => {
                work.notes.push(format!("Rust licences aren't shown: cargo metadata couldn't start ({e})."));
                rust_docs(&mut work, "target");
            }
        }
    } else if exists("Cargo.toml") {
        work.notes
            .push("Cargo.toml has no Cargo.lock beside it yet: build the project once to list its crates.".into());
    }
    deps_done(work);
    Ok(())
}

fn node_deps(work: &mut DepsWork) {
    let Ok(text) = host::read_file("package.json") else { return };
    let wanted = match deps::package_json(&text) {
        Ok(w) => w,
        Err(e) => return work.notes.push(e),
    };
    let mut missing = 0;
    let list = wanted
        .into_iter()
        .map(|(name, range, kind)| {
            let dir = format!("node_modules/{name}");
            let (version, licence) = match host::read_file(&format!("{dir}/package.json")) {
                Ok(t) => deps::installed_package(&t),
                Err(_) => {
                    missing += 1;
                    (format!("{range} (wanted)"), String::new())
                }
            };
            let docs = ["README.md", "readme.md", "Readme.md"]
                .iter()
                .map(|r| format!("{dir}/{r}"))
                .find(|p| exists(p))
                .unwrap_or_default();
            Dep { name, version, licence, kind, docs }
        })
        .collect();
    if missing > 0 {
        work.notes.push(format!(
            "{missing} npm package{} not installed in node_modules: the versions wanted are shown, and no licence.",
            if missing == 1 { " is" } else { "s are" }
        ));
    }
    work.groups.insert("JavaScript (package.json)".into(), list);
}

/// What the project's virtual environment has installed: by normalized name, its version and
/// licence.
fn installed_python() -> HashMap<String, (String, String)> {
    let mut sites = Vec::new();
    for venv in [".venv", "venv", "env"] {
        sites.push(format!("{venv}/Lib/site-packages"));
        if let Ok(entries) = host::read_dir(&format!("{venv}/lib")) {
            sites.extend(
                entries
                    .into_iter()
                    .filter(|e| e.is_dir && e.name.starts_with("python"))
                    .map(|e| format!("{venv}/lib/{}/site-packages", e.name)),
            );
        }
    }
    let mut out = HashMap::new();
    for site in sites {
        let Ok(entries) = host::read_dir(&site) else { continue };
        for e in entries.into_iter().filter(|e| e.is_dir) {
            let Some((name, version)) = deps::dist_info(&e.name) else { continue };
            let licence = host::read_file(&format!("{site}/{}/METADATA", e.name))
                .map(|m| deps::wheel_licence(&m))
                .unwrap_or_default();
            out.insert(name, (version, licence));
        }
    }
    out
}

fn python_deps(work: &mut DepsWork) {
    let mut wanted: Vec<(String, String, String, &str)> = Vec::new();
    if let Ok(t) = host::read_file("requirements.txt") {
        wanted.extend(deps::requirements(&t).into_iter().map(|(n, v)| (n, v, "direct".to_owned(), "requirements.txt")));
    }
    if let Ok(t) = host::read_file("pyproject.toml") {
        wanted.extend(deps::pyproject(&t).into_iter().map(|(n, v, k)| (n, v, k, "pyproject.toml")));
    }
    if wanted.is_empty() {
        return;
    }
    let installed = installed_python();
    let mut seen = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut missing = 0;
    let mut list = Vec::new();
    for (name, spec, kind, source) in wanted {
        if !seen.insert(deps::normalize(&name)) {
            continue;
        }
        sources.insert(source);
        let (version, licence) = match installed.get(&deps::normalize(&name)) {
            Some((v, l)) => (v.clone(), l.clone()),
            None => {
                missing += 1;
                (if spec.is_empty() { "any (wanted)".into() } else { format!("{spec} (wanted)") }, String::new())
            }
        };
        list.push(Dep { name, version, licence, kind, docs: String::new() });
    }
    if missing > 0 {
        work.notes.push(format!(
            "{missing} Python package{} not installed in the project's virtual environment (.venv, venv or env): \
             the versions wanted are shown, and no licence.",
            if missing == 1 { " is" } else { "s are" }
        ));
    }
    work.groups.insert(format!("Python ({})", sources.into_iter().collect::<Vec<_>>().join(", ")), list);
}

/// Each crate's documentation in `<doc>/<crate>`, where `cargo doc` has made it.
fn rust_docs(work: &mut DepsWork, target: &str) {
    let doc = format!("{target}/doc");
    let Ok(entries) = host::read_dir(&doc) else { return };
    let made: BTreeSet<String> = entries.into_iter().filter(|e| e.is_dir).map(|e| e.name).collect();
    if let Some(list) = work.groups.get_mut("Rust (Cargo.lock)") {
        for d in list.iter_mut() {
            let dir = deps::crate_doc_dir(&d.name);
            if made.contains(&dir) {
                d.docs = format!("{doc}/{dir}/index.html");
            }
        }
    }
}

fn metadata_done(job: JobResult) {
    let Some(mut work) = with(|s| s.deps.take()) else { return };
    let mut target = "target".to_owned();
    match (job.exit_code, deps::cargo_metadata(&job.stdout)) {
        (Some(0), Ok(m)) => {
            if let Some(list) = work.groups.get_mut("Rust (Cargo.lock)") {
                for d in list.iter_mut() {
                    if let Some(l) = m.licences.get(&(d.name.clone(), d.version.clone())) {
                        d.licence = l.clone();
                    }
                }
            }
            // The build folder, relative to the open folder when it's inside.
            let root = host::workspace_path().unwrap_or_default().replace('\\', "/");
            let t = m.target_directory.replace('\\', "/");
            if let Some(rel) = t.strip_prefix(&format!("{}/", root.trim_end_matches('/'))) {
                target = rel.to_owned();
            }
        }
        _ => {
            let why = job.stderr.lines().rev().find(|l| l.contains("error")).unwrap_or("").trim().to_owned();
            work.notes.push(format!(
                "Rust licences aren't shown: cargo metadata --offline failed{}. It reads only crates already \
                 downloaded; building the project once downloads them.",
                if why.is_empty() { String::new() } else { format!(" ({why})") }
            ));
        }
    }
    rust_docs(&mut work, &target);
    deps_done(work);
}

fn deps_done(work: DepsWork) {
    let project = project_name();
    let md = deps::page(&project, &work.groups, &work.notes);
    let title = format!("Dependencies of {project}");
    host::show_markdown(&title, &md);
    let n: usize = work.groups.values().map(Vec::len).sum();
    note(format!("{n} dependencies listed, opened as a page"), GREEN);
    with(|s| {
        s.page = Some(("docs/DEPENDENCIES.md".into(), title, md));
        s.confirm_replace = false;
    });
}

// ----- the panel -----------------------------------------------------------------------------

fn render() {
    let tool = tool();
    let blocks = with(|s| {
        let busy = s.job.is_some() || s.scan.is_some();
        let mut b = vec![Block::Form(Form {
            id: "generate".into(),
            fields: vec![
                field(
                    "tool",
                    "Generate with",
                    FieldKind::Choice,
                    tool_label(tool),
                    TOOLS.iter().map(|(_, _, l)| (*l).to_owned()).collect(),
                ),
                field(
                    "private",
                    "Rust: private items too (--document-private-items)",
                    FieldKind::Check,
                    if s.private { "true" } else { "false" },
                    vec![],
                ),
            ],
            buttons: vec!["Generate".into()],
        })];
        let mut buttons = vec![button("Page from doc comments", "comments", ""), button("Dependencies", "deps", "")];
        if s.job.is_some() {
            buttons.insert(0, button("Stop", "stop", ""));
        }
        b.push(Block::Rows(vec![Row { cells: vec![], buttons }]));
        if let Some((text, color)) = &s.note {
            b.push(Block::Spans(vec![span(text, Some(*color))]));
        }
        if let Some(out) = &s.out {
            b.push(Block::Spans(vec![Span { text: out.clone(), color: None, bold: false, monospace: true }]));
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![button("Open in the browser", "open", ""), button("Copy path", "copy", "")],
            }]));
        }
        if let Some((file, _, _)) = &s.page {
            b.push(Block::Rows(vec![Row {
                cells: vec![],
                buttons: vec![
                    button("Show the page again", "page", ""),
                    button(&format!("Save as {file}"), "save", ""),
                ],
            }]));
            if s.confirm_replace {
                b.push(Block::Spans(vec![span(format!("{file} is there already. Replace it?"), Some(RED))]));
                b.push(Block::Rows(vec![Row {
                    cells: vec![],
                    buttons: vec![button("Replace it", "replace", ""), button("Cancel", "cancel", "")],
                }]));
            }
        }
        let failed = s.note.as_ref().is_some_and(|(_, c)| *c == RED);
        if (s.job.is_some() && s.stage != Stage::Metadata || failed) && !s.output.is_empty() {
            let from = s.output.len().saturating_sub(400);
            let lines = s.output[from..]
                .iter()
                .map(|t| LogLine { text: t.clone(), color: None, path: String::new(), line: 0 })
                .collect();
            b.push(Block::Log(LogView { lines, follow: true }));
        }
        if !busy && s.note.is_none() {
            b.push(Block::Text(
                "Generate runs the project's own tool: cargo doc, pydoc or javadoc. Page from doc comments \
                 needs none: it reads ///, //!, /** */ and docstrings. Dependencies lists versions and licences \
                 from the files on disk. Nothing is fetched from the network."
                    .into(),
            ));
        }
        b
    });
    host::set_panel("apidocs", &blocks);
}

fn save(replace: bool) -> Result<(), String> {
    let Some((file, _, md)) = with(|s| s.page.clone()) else { return Err("Make a page first".into()) };
    if !replace && exists(&file) {
        with(|s| s.confirm_replace = true);
        return Ok(());
    }
    // From docs/, the page's links (from the open folder, starting "/") still lead to the code.
    host::write_file(&file, &md)?;
    with(|s| s.confirm_replace = false);
    note(format!("Saved as {file}"), GREEN);
    let _ = host::open_location(&file, 1);
    Ok(())
}

fn submitted(f: FormValues) -> Result<(), String> {
    if f.form != "generate" {
        return Ok(());
    }
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    let chosen = TOOLS.iter().find(|(_, _, l)| *l == get("tool")).map(|(t, ..)| *t);
    with(|s| s.private = get("private") == "true");
    generate(chosen)
}

fn command(id: &str, argument: &str) -> Result<(), String> {
    match id {
        "show" => {}
        "generate" => {
            let chosen = TOOLS.iter().find(|(_, key, _)| *key == argument).map(|(t, ..)| *t);
            generate(chosen)?;
        }
        "comments" => start_scan()?,
        "deps" => start_deps()?,
        "stop" => {
            if let Some(job) = with(|s| s.job) {
                host::cancel_job(job);
            }
        }
        "open" => {
            let out = with(|s| s.out.clone()).ok_or("Generate the documentation first")?;
            if host::open_url(&file_url(&out)).is_err() {
                note("The editor opens only web addresses in the browser: copy the path into it.", DIM);
            }
        }
        "copy" => {
            let out = with(|s| s.out.clone()).ok_or("Generate the documentation first")?;
            host::copy_text(&out);
            note("Path copied", DIM);
        }
        "page" => {
            let (_, title, md) = with(|s| s.page.clone()).ok_or("Make a page first")?;
            host::show_markdown(&title, &md);
        }
        "save" => save(false)?,
        "replace" => save(true)?,
        "cancel" => with(|s| s.confirm_replace = false),
        _ => return Err(format!("unknown command {id}")),
    }
    Ok(())
}

fn job_done(job: JobResult) {
    let stage = with(|s| {
        if s.job != Some(job.id) {
            return Stage::Idle;
        }
        s.job = None;
        std::mem::take(&mut s.stage)
    });
    match stage {
        Stage::Idle => return,
        Stage::Doc(t) => doc_done(t, job),
        Stage::Metadata => metadata_done(job),
    }
    render();
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        let r = command(&id, &argument);
        if let Err(e) = &r {
            note(e.clone(), RED);
        }
        render();
        r
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => job_done(job),
            Event::Resumed(token) if token == "scan" => {
                scan_step();
                render();
            }
            Event::FormSubmitted(f) => {
                if let Err(e) = submitted(f) {
                    note(e, RED);
                }
                render();
            }
            _ => {}
        }
    }
}

export!(Plugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url(r"C:\My docs\target\doc\demo\index.html"),
            "file:///C:/My%20docs/target/doc/demo/index.html"
        );
        assert_eq!(file_url("/home/a/#x/index.html"), "file:///home/a/%23x/index.html");
    }

    #[test]
    fn cargo_doc_output() {
        let out = "   Documenting demo v0.1.0 (C:\\p)\n    Finished `dev` profile\n   Generated C:\\p\\target\\doc\\demo\\index.html and 1 other file\n";
        assert_eq!(generated_path(out).as_deref(), Some("C:\\p\\target\\doc\\demo\\index.html"));
        assert_eq!(
            generated_path("Generated /p/target/doc/demo/index.html").as_deref(),
            Some("/p/target/doc/demo/index.html")
        );
        assert_eq!(generated_path("Finished"), None);
    }

    #[test]
    fn python_module_names() {
        let files: Vec<String> = [
            "src/pkg/__init__.py",
            "src/pkg/core.py",
            "tool.py",
            "tests/test_core.py",
            "setup.py",
            "my-scripts/x.py",
            "pkg/__main__.py",
            ".venv/lib/site-packages/x.py",
        ]
        .map(str::to_owned)
        .to_vec();
        assert_eq!(python_modules(&files), ["pkg", "pkg.core", "tool"]);
        assert_eq!(
            pydoc_written("wrote pkg.html\nno Python documentation found for 'x'\nwrote tool.html\n"),
            ["pkg", "tool"]
        );
    }
}
