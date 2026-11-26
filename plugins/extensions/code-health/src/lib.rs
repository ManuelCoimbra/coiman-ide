//! Code Health: secrets left in files, code an attacker could use (SQL and HTML made from
//! strings, unsafe C calls, Rust unsafe), the most complex functions, code written twice, public
//! items with no docs, the dependencies (versions pinned twice, what's out of date, a tree),
//! their licences, and known vulnerabilities from `cargo audit` and `npm audit`, in a panel.
//! And Project: Generate Overview, a page about the project.
//!
//! Files are read in steps (`continue-later`). Complexity comes from the editor's own parser
//! (`syntax-functions`). The package managers run as approved programs, only when asked, since
//! they fetch from their registries; an update or removal runs in a terminal.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod debt;
mod duplicates;
mod lockfiles;
mod overview;
mod packages;
mod secrets;
mod security;
mod undocumented;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FileLink, JobResult, Row, Span, TreeItem};
use lockfiles::{Ecosystem, Lock, Outdated};
use packages::{Advisory, Kind, Licensed};

/// Files looked at, at most; text read per step; functions listed.
const MAX_FILES: u32 = 20_000;
const STEP_BYTES: usize = 1 << 20;
const STEP: usize = 150;
const TOP_FUNCTIONS: usize = 200;

const RED: u32 = 0xE5534B;
const AMBER: u32 = 0xE2A33D;
const DIM: u32 = 0x8B949E;

/// Languages the editor's parser measures.
const PARSED: [&str; 13] = ["rs", "py", "js", "jsx", "mjs", "ts", "tsx", "java", "c", "h", "cpp", "cs", "go"];

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Secrets,
    Security,
    Complexity,
    Debt,
    Duplicates,
    Undocumented,
    Dependencies,
    Licences,
    Audits,
}

const TABS: [(Tab, &str, &str); 9] = [
    (Tab::Secrets, "secrets", "Secrets"),
    (Tab::Security, "security", "Security"),
    (Tab::Complexity, "complexity", "Complexity"),
    (Tab::Debt, "debt", "Debt"),
    (Tab::Duplicates, "duplicates", "Duplicates"),
    (Tab::Undocumented, "undocumented", "Undocumented"),
    (Tab::Dependencies, "dependencies", "Dependencies"),
    (Tab::Licences, "licences", "Licences"),
    (Tab::Audits, "audits", "Audits"),
];

struct Function {
    path: String,
    name: String,
    line: usize,
    lines: usize,
    complexity: usize,
}

/// What a job that is running is for.
#[derive(Clone, Copy)]
enum Job {
    Metadata,
    Outdated(Ecosystem),
    Audit(&'static str),
    Git(usize),
    /// `git blame` of `debt_files[i]`.
    Blame(usize),
}

#[derive(Default)]
struct Health {
    // What is being read.
    files: Vec<String>,
    next: usize,
    reading: bool,
    sources: Vec<(String, String)>,
    // What was found.
    secrets: Vec<secrets::Finding>,
    security: Vec<security::Finding>,
    debt: Vec<debt::Note>,
    /// Files with notes, blamed one at a time for their ages.
    debt_files: Vec<String>,
    /// Why the notes have no ages, when they don't.
    debt_note: Option<String>,
    /// Blamed so far.
    blamed: usize,
    functions: Vec<Function>,
    duplicates: Vec<duplicates::Duplicate>,
    undocumented: Vec<undocumented::Item>,
    locks: Vec<Lock>,
    lock_notes: Vec<String>,
    licences: Vec<Licensed>,
    licence_note: Option<String>,
    updates: Option<(Vec<Outdated>, Vec<String>)>,
    audit: Option<(Vec<Advisory>, Vec<String>)>,
    // Asked of programs, by job id.
    jobs: HashMap<u64, Job>,
    tab: Tab,
    flagged_only: bool,
    /// Dependency tree rows opened.
    open_rows: HashSet<String>,
    // The overview, while it is made.
    overview: Option<Overview>,
}

struct Overview {
    files: Vec<String>,
    next: usize,
    counts: overview::Counts,
    git: overview::GitFacts,
    git_jobs: usize,
}

static STATE: Mutex<Option<Health>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Health) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Health::default))
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: Vec::new(), buttons }])
}

fn link(text: String, detail: String, path: &str, line: usize) -> FileLink {
    FileLink { text, detail, path: path.to_owned(), line: line as u32 }
}

fn has(files: &[String], name: &str) -> bool {
    files.iter().any(|f| f == name)
}

fn last_line(text: &str) -> String {
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned()
}

// ----- reading the project ---------------------------------------------------------------

fn check() -> Result<(), String> {
    let files = host::list_files("", MAX_FILES)?;
    with(|h| {
        let (tab, flagged_only) = (h.tab, h.flagged_only);
        *h = Health { files, reading: true, tab, flagged_only, ..Default::default() };
    });
    host::show_message("Reading the project's files…");
    render();
    host::continue_later("scan");
    Ok(())
}

fn step() {
    let done = with(|h| {
        let (mut bytes, mut count) = (0, 0);
        while h.next < h.files.len() && count < STEP && bytes < STEP_BYTES {
            let path = h.files[h.next].clone();
            h.next += 1;
            count += 1;
            let Ok(text) = host::read_file(&path) else { continue };
            bytes += text.len();
            let size = text.len() as u64;
            if secrets::worth_reading(Path::new(&path)) && size <= secrets::MAX_BYTES {
                h.secrets.extend(secrets::scan_text(&path, &text));
            }
            let ext = overview::extension(&path);
            if size <= security::MAX_BYTES {
                h.security.extend(security::scan_text(&path, &ext, &text));
            }
            if size <= debt::MAX_BYTES && secrets::worth_reading(Path::new(&path)) {
                h.debt.extend(debt::scan_text(&path, &text));
            }
            if PARSED.contains(&ext.as_str())
                && let Ok(found) = host::syntax_functions(&path)
            {
                h.functions.extend(found.into_iter().map(|f| Function {
                    path: path.clone(),
                    name: f.name,
                    line: f.line as usize,
                    lines: f.lines as usize,
                    complexity: f.complexity as usize,
                }));
            }
            // Tests aren't public API.
            let test = path.split('/').any(|c| c == "tests" || c == "test" || c == "__tests__");
            if !test && size <= undocumented::MAX_BYTES {
                h.undocumented.extend(undocumented::in_text(&ext, &path, &text));
            }
            if duplicates::is_code(&path) && size <= duplicates::MAX_BYTES {
                h.sources.push((path, text));
            }
        }
        h.next >= h.files.len()
    });
    if !done {
        let (n, total) = with(|h| (h.next, h.files.len()));
        host::show_message(&format!("{n} of {total} files read"));
        host::continue_later("scan");
        return;
    }
    // Code written twice, then the dependencies: a step of their own.
    with(|h| {
        h.duplicates = duplicates::find(&h.sources);
        h.sources = Vec::new();
        h.secrets.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        h.security.sort_by(|a, b| (a.category, &a.path, a.line).cmp(&(b.category, &b.path, b.line)));
        h.undocumented.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        h.functions.sort_by(|a, b| b.complexity.cmp(&a.complexity).then(b.lines.cmp(&a.lines)));
        h.functions.truncate(TOP_FUNCTIONS);
        for (file, eco) in lockfiles::LOCKFILES {
            let Ok(text) = host::read_file(file) else { continue };
            match lockfiles::read_lock(file, eco, &text) {
                Ok(lock) => h.locks.push(lock),
                Err(e) => h.lock_notes.push(format!("{file}: {e}")),
            }
        }
        h.licences = npm_licences();
        if has(&h.files, "Cargo.toml") {
            let args = ["metadata", "--format-version", "1", "--offline"].map(str::to_owned);
            match host::start_job("cargo", &args, "") {
                Ok(id) => {
                    h.jobs.insert(id, Job::Metadata);
                }
                Err(e) => h.licence_note = Some(format!("Rust dependencies aren't listed: {e}")),
            }
        }
        h.reading = false;
        let mut files: Vec<String> = h.debt.iter().map(|n| n.path.clone()).collect();
        files.dedup();
        h.debt_files = files;
    });
    blame_next();
    render_tree();
    host::show_message("Checked");
    render();
}

/// The packages in `node_modules` (scoped ones too), one level deep.
fn npm_licences() -> Vec<Licensed> {
    let Ok(top) = host::read_dir("node_modules") else { return Vec::new() };
    let mut dirs = Vec::new();
    for e in top.into_iter().filter(|e| e.is_dir) {
        if e.name.starts_with('@') {
            let scope = format!("node_modules/{}", e.name);
            for inner in host::read_dir(&scope).unwrap_or_default().into_iter().filter(|i| i.is_dir) {
                dirs.push(format!("{scope}/{}", inner.name));
            }
        } else if !e.name.starts_with('.') {
            dirs.push(format!("node_modules/{}", e.name));
        }
    }
    let mut out: Vec<Licensed> = dirs
        .iter()
        .filter_map(|d| host::read_file(&format!("{d}/package.json")).ok())
        .filter_map(|t| packages::npm_package(&t))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

// ----- asking the package managers ---------------------------------------------------------

fn start_jobs(list: &[(&str, &[&str], Job)]) -> Vec<String> {
    let mut notes = Vec::new();
    for (program, args, job) in list {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        match host::start_job(program, &args, "") {
            Ok(id) => {
                with(|h| h.jobs.insert(id, *job));
            }
            Err(e) => notes.push(format!("{program}: {e}")),
        }
    }
    notes
}

fn check_updates() {
    let files = with(|h| h.files.clone());
    let python =
        ["pyproject.toml", "requirements.txt", "setup.py", "poetry.lock", "uv.lock"].iter().any(|f| has(&files, f));
    let mut list: Vec<(&str, &[&str], Job)> = Vec::new();
    if has(&files, "Cargo.toml") {
        list.push(("cargo", &["update", "--dry-run", "--verbose"], Job::Outdated(Ecosystem::Cargo)));
    }
    if has(&files, "package.json") {
        list.push(("npm", &["outdated", "--json"], Job::Outdated(Ecosystem::Npm)));
    }
    if python {
        list.push(("python", &["-m", "pip", "list", "--outdated", "--format=json"], Job::Outdated(Ecosystem::Pip)));
    }
    let mut notes = start_jobs(&list);
    if list.is_empty() {
        notes.push("No Cargo.toml, package.json or Python project here".to_owned());
    }
    with(|h| h.updates = Some((Vec::new(), notes)));
    render();
}

fn run_audits() {
    let files = with(|h| h.files.clone());
    let mut list: Vec<(&str, &[&str], Job)> = Vec::new();
    if has(&files, "Cargo.lock") {
        list.push(("cargo", &["audit", "--json"], Job::Audit("cargo")));
    }
    if has(&files, "package-lock.json") {
        list.push(("npm", &["audit", "--json"], Job::Audit("npm")));
    }
    let mut notes = start_jobs(&list);
    if list.is_empty() {
        notes.push("No Cargo.lock or package-lock.json here: nothing to audit".to_owned());
    }
    with(|h| h.audit = Some((Vec::new(), notes)));
    render();
}

fn finished(job: JobResult) {
    let Some(kind) = with(|h| h.jobs.remove(&job.id)) else { return };
    match kind {
        Job::Metadata => with(|h| match job.exit_code {
            Some(0) => match packages::parse_cargo_metadata(&job.stdout) {
                Ok(list) => {
                    let mut all = list;
                    all.append(&mut h.licences);
                    h.licences = all;
                }
                Err(e) => h.licence_note = Some(format!("cargo metadata: {e}")),
            },
            _ => {
                let first = job.stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed");
                h.licence_note = Some(format!("cargo metadata: {first} (build once to download the dependencies)"));
            }
        }),
        Job::Outdated(eco) => with(|h| {
            let (list, notes) = h.updates.get_or_insert_with(Default::default);
            match eco {
                // Cargo writes these lines on standard error.
                Ecosystem::Cargo => {
                    let found = lockfiles::parse_cargo_dry_run(&job.stderr);
                    if found.is_empty() && job.stderr.contains("error") {
                        notes.push(format!("cargo update: {}", last_line(&job.stderr)));
                    }
                    list.extend(found);
                }
                // It exits 1 when something is out of date: the JSON says what.
                Ecosystem::Npm => match lockfiles::parse_npm_outdated(&job.stdout) {
                    Ok(found) => list.extend(found),
                    Err(e) => notes.push(format!("npm outdated: {e}: {}", last_line(&job.stderr))),
                },
                _ if job.exit_code == Some(0) => match lockfiles::parse_pip_outdated(&job.stdout) {
                    Ok(found) => list.extend(found),
                    Err(e) => notes.push(format!("pip list: {e}")),
                },
                _ => notes.push(format!("pip list: {}", last_line(&job.stderr))),
            }
            list.sort_by(|a, b| a.ecosystem.label().cmp(b.ecosystem.label()).then(a.name.cmp(&b.name)));
        }),
        Job::Audit(eco) => with(|h| {
            let (list, notes) = h.audit.get_or_insert_with(Default::default);
            if eco == "cargo" {
                if job.stderr.contains("no such command") {
                    notes.push("cargo audit isn't installed: run `cargo install cargo-audit` to add it".to_owned());
                } else {
                    match packages::parse_cargo_audit(&job.stdout) {
                        Ok(found) => list.extend(found),
                        Err(_) => notes.push(format!("cargo audit: {}", last_line(&job.stderr))),
                    }
                }
            } else {
                match packages::parse_npm_audit(&job.stdout) {
                    Ok(found) => list.extend(found),
                    Err(e) => notes.push(format!("npm audit: {e}")),
                }
            }
            list.sort_by(|a, b| {
                packages::severity_rank(&a.severity)
                    .cmp(&packages::severity_rank(&b.severity))
                    .then(a.package.cmp(&b.package))
            });
        }),
        Job::Git(which) => {
            overview_git(which, job);
            return;
        }
        Job::Blame(i) => {
            with(|h| {
                h.blamed = i + 1;
                if job.exit_code == Some(0) {
                    let times = debt::blame_times(&job.stdout);
                    let path = h.debt_files.get(i).cloned().unwrap_or_default();
                    for n in h.debt.iter_mut().filter(|n| n.path == path) {
                        n.written = times.get(&n.line).copied();
                    }
                }
            });
            blame_next();
        }
    }
    render();
}

/// The next file with notes, blamed for when its lines were written (git, if approved).
fn blame_next() {
    let (next, path) = with(|h| (h.blamed, h.debt_files.get(h.blamed).cloned()));
    let Some(path) = path else { return };
    let args = ["blame", "--line-porcelain", "-w", "--", &path].map(str::to_owned);
    match host::start_job("git", &args, "") {
        Ok(id) => with(|h| {
            h.jobs.insert(id, Job::Blame(next));
        }),
        Err(e) => with(|h| h.debt_note = Some(format!("Their ages come from git blame: {e}"))),
    }
}

// ----- the panel -----------------------------------------------------------------------

fn count(h: &Health, tab: Tab) -> usize {
    match tab {
        Tab::Secrets => h.secrets.len(),
        Tab::Security => h.security.len(),
        Tab::Debt => h.debt.len(),
        Tab::Complexity => h.functions.iter().filter(|f| f.complexity > 10).count(),
        Tab::Duplicates => h.duplicates.len(),
        Tab::Undocumented => h.undocumented.len(),
        Tab::Dependencies => {
            h.locks.iter().map(|l| l.conflicts().len()).sum::<usize>() + h.updates.as_ref().map_or(0, |u| u.0.len())
        }
        Tab::Licences => h.licences.iter().filter(|l| packages::kind(&l.license) != Kind::Permissive).count(),
        Tab::Audits => h.audit.as_ref().map_or(0, |a| a.0.len()),
    }
}

fn render() {
    let blocks = with(|h| {
        let mut tabs: Vec<Button> = TABS
            .iter()
            .map(|(tab, id, name)| {
                let n = count(h, *tab);
                let mark = if *tab == h.tab { "▸ " } else { "" };
                let label = if n > 0 { format!("{mark}{name}  {n}") } else { format!("{mark}{name}") };
                button(&label, "tab", id)
            })
            .collect();
        tabs.push(button("Scan again", "check", ""));
        let mut blocks = vec![bar(tabs)];
        if h.reading {
            blocks.push(Block::Text(format!("Reading the project's files: {} of {}…", h.next, h.files.len())));
            return blocks;
        }
        match h.tab {
            Tab::Secrets => secrets_tab(h, &mut blocks),
            Tab::Security => security_tab(h, &mut blocks),
            Tab::Debt => debt_tab(h, &mut blocks),
            Tab::Complexity => complexity_tab(h, &mut blocks),
            Tab::Duplicates => duplicates_tab(h, &mut blocks),
            Tab::Undocumented => undocumented_tab(h, &mut blocks),
            Tab::Dependencies => dependencies_tab(h, &mut blocks),
            Tab::Licences => licences_tab(h, &mut blocks),
            Tab::Audits => audits_tab(h, &mut blocks),
        }
        blocks
    });
    host::set_panel("health", &blocks);
}

fn secrets_tab(h: &Health, b: &mut Vec<Block>) {
    if h.secrets.is_empty() {
        b.push(Block::Text("No secrets found in the project's files (Git-ignored ones aren't read).".into()));
        return;
    }
    b.push(Block::Text(format!(
        "{} that look real. Remove them from the files, move them to settings or the environment, and replace any \
         that were committed: they stay in Git's history.",
        plural(h.secrets.len(), "secret")
    )));
    let links = h
        .secrets
        .iter()
        .map(|f| link(format!("{}:{}", f.path, f.line), format!("{}  {}", f.kind, f.masked), &f.path, f.line));
    b.push(Block::Links(links.collect()));
}

fn security_tab(h: &Health, b: &mut Vec<Block>) {
    if h.security.is_empty() {
        b.push(Block::Text(
            "Nothing found: no SQL put together from strings, no HTML made from data, no C calls that \
             can write past a buffer, no Rust unsafe."
                .into(),
        ));
        return;
    }
    b.push(Block::Text(
        "Read a line at a time, so these are places to look at, not proof. A line with nosec or \
         coiman:allow in it is left out."
            .into(),
    ));
    for category in security::Category::ALL {
        let found: Vec<&security::Finding> = h.security.iter().filter(|f| f.category == category).collect();
        if found.is_empty() {
            continue;
        }
        b.push(Block::Spans(vec![
            Span { text: category.title().into(), color: None, bold: true, monospace: false },
            span(format!("  {}", found.len()), Some(DIM)),
        ]));
        b.push(Block::Text(category.advice().into()));
        let links =
            found.iter().map(|f| link(format!("{}:{}  {}", f.path, f.line, f.code), f.why.clone(), &f.path, f.line));
        b.push(Block::Links(links.collect()));
    }
}

fn debt_tab(h: &Health, b: &mut Vec<Block>) {
    if h.debt.is_empty() {
        b.push(Block::Text("No TODO, FIXME, HACK or XXX notes in the project's comments.".into()));
        return;
    }
    let mut kinds: Vec<String> = Vec::new();
    for kind in debt::KINDS {
        let n = h.debt.iter().filter(|d| d.kind == kind).count();
        if n > 0 {
            kinds.push(format!("{n} {kind}"));
        }
    }
    b.push(Block::Text(format!("{} in comments: {}.", plural(h.debt.len(), "note"), kinds.join(", "))));
    if h.blamed < h.debt_files.len() && h.debt_note.is_none() {
        b.push(Block::Spans(vec![span(
            format!("Finding their ages: {} of {} files…", h.blamed, h.debt_files.len()),
            Some(DIM),
        )]));
    }
    if let Some(note) = &h.debt_note {
        b.push(Block::Spans(vec![span(note, Some(DIM))]));
    }
    let now = host::now().unix_millis / 1000;
    let dated: Vec<&debt::Note> = h.debt.iter().filter(|d| d.written.is_some()).collect();
    let row = |cells: Vec<Span>| Row { cells, buttons: Vec::new() };
    let bar_of = |n: usize, most: usize| "▇".repeat((n * 30).div_ceil(most.max(1)));
    if !dated.is_empty() {
        b.push(Block::Spans(vec![Span { text: "By age".into(), color: None, bold: true, monospace: false }]));
        let counts: Vec<(&str, usize)> = debt::AGES
            .iter()
            .map(|(name, _)| {
                (*name, dated.iter().filter(|d| debt::age_bucket(d.written.unwrap_or(0), now) == *name).count())
            })
            .collect();
        let most = counts.iter().map(|c| c.1).max().unwrap_or(1);
        b.push(Block::Rows(
            counts
                .iter()
                .map(|(name, n)| {
                    let color = if name.starts_with("over") || name.starts_with("1–2") { AMBER } else { DIM };
                    row(vec![span(*name, None), span(n.to_string(), None), span(bar_of(*n, most), Some(color))])
                })
                .collect(),
        ));
        b.push(Block::Spans(vec![Span {
            text: "When the ones still here were written".into(),
            color: None,
            bold: true,
            monospace: false,
        }]));
        let months = debt::by_month(&h.debt);
        let most = months.iter().map(|m| m.1).max().unwrap_or(1);
        b.push(Block::Rows(
            months
                .iter()
                .map(|(m, n)| row(vec![span(m, None), span(n.to_string(), None), span(bar_of(*n, most), Some(DIM))]))
                .collect(),
        ));
    }
    b.push(Block::Spans(vec![Span { text: "By area".into(), color: None, bold: true, monospace: false }]));
    b.push(Block::Rows(
        debt::by_area(&h.debt)
            .into_iter()
            .map(|(area, n, kinds)| {
                let detail = kinds.iter().map(|(k, c)| format!("{c} {k}")).collect::<Vec<_>>().join(", ");
                row(vec![span(area, None), span(n.to_string(), None), span(detail, Some(DIM))])
            })
            .collect(),
    ));
    // Oldest first: those have waited longest.
    let mut list: Vec<&debt::Note> = h.debt.iter().collect();
    list.sort_by_key(|d| (d.written.unwrap_or(i64::MAX), d.path.clone(), d.line));
    b.push(Block::Spans(vec![Span {
        text: "The notes, oldest first".into(),
        color: None,
        bold: true,
        monospace: false,
    }]));
    let links = list.iter().map(|d| {
        let when = d.written.map(|t| format!("{} · ", debt::month(t))).unwrap_or_default();
        link(
            format!("{}:{}  {} {}", d.path, d.line, d.kind, d.text),
            format!("{when}{}", debt::area(&d.path)),
            &d.path,
            d.line,
        )
    });
    b.push(Block::Links(links.collect()));
}

fn complexity_tab(h: &Health, b: &mut Vec<Block>) {
    b.push(Block::Text(
        "Cyclomatic complexity: 1, plus one for each branch (if, loop, case, catch, &&, ||, ?:). Up to 10 is easy to \
         follow and test; above 20, consider splitting the function."
            .into(),
    ));
    if h.functions.is_empty() {
        b.push(Block::Text(
            "No functions found in languages the editor measures (Rust, Python, JavaScript, TypeScript, Java, C, C++, \
             C#, Go)."
                .into(),
        ));
        return;
    }
    let mut rows = vec![Row {
        cells: ["Complexity", "Lines", "Function", "Where"].map(|t| span(t, Some(DIM))).to_vec(),
        buttons: vec![],
    }];
    for f in &h.functions {
        let color = match f.complexity {
            0..=10 => None,
            11..=20 => Some(AMBER),
            _ => Some(RED),
        };
        let mut c = span(f.complexity.to_string(), color);
        c.bold = true;
        let mut name = span(&*f.name, None);
        name.monospace = true;
        rows.push(Row {
            cells: vec![c, span(f.lines.to_string(), None), name, span(format!("{}:{}", f.path, f.line), Some(DIM))],
            buttons: vec![button("Open", "open", &format!("{}:{}", f.path, f.line))],
        });
    }
    b.push(Block::Rows(rows));
}

fn duplicates_tab(h: &Health, b: &mut Vec<Block>) {
    if h.duplicates.is_empty() {
        b.push(Block::Text(format!(
            "No code written twice ({} or more lines alike, blank lines and comments aside).",
            duplicates::MIN_LINES
        )));
        return;
    }
    let lines: usize = h.duplicates.iter().map(|d| d.lines * (d.places.len() - 1)).sum();
    b.push(Block::Text(format!(
        "{} of code written more than once, about {lines} lines that could be shared.",
        plural(h.duplicates.len(), "block")
    )));
    for d in &h.duplicates {
        b.push(Block::Spans(vec![Span {
            text: format!("{} lines, in {} places", d.lines, d.places.len()),
            color: None,
            bold: true,
            monospace: false,
        }]));
        let links =
            d.places.iter().map(|(p, first, last)| link(format!("{p}:{first}–{last}"), String::new(), p, *first));
        b.push(Block::Links(links.collect()));
    }
}

fn undocumented_tab(h: &Health, b: &mut Vec<Block>) {
    b.push(Block::Text(
        "Public items with no doc comment: Rust's pub items (/// above), Go's exported names (// above), Python's \
         public top-level def and class (a docstring), JavaScript's and TypeScript's exports (/** */ above). Tests \
         are left out."
            .into(),
    ));
    if h.undocumented.is_empty() {
        b.push(Block::Text("Every public item has a doc comment.".into()));
        return;
    }
    let mut files: Vec<&str> = h.undocumented.iter().map(|i| i.path.as_str()).collect();
    files.dedup();
    b.push(Block::Text(format!("{} in {}", plural(h.undocumented.len(), "item"), plural(files.len(), "file"))));
    let links = h
        .undocumented
        .iter()
        .map(|i| link(format!("{}:{}", i.path, i.line), format!("{} {}", i.kind, i.name), &i.path, i.line));
    b.push(Block::Links(links.collect()));
}

fn dependencies_tab(h: &Health, b: &mut Vec<Block>) {
    for note in &h.lock_notes {
        b.push(Block::Spans(vec![span(note, Some(AMBER))]));
    }
    let asking = h.jobs.values().any(|j| matches!(j, Job::Outdated(_)));
    b.push(bar(vec![button(if asking { "Checking…" } else { "Check for updates" }, "updates", "")]));
    b.push(Block::Text(
        "cargo update --dry-run, npm outdated and pip list --outdated: they ask their registries themselves.".into(),
    ));
    if let Some((list, notes)) = &h.updates {
        for note in notes {
            b.push(Block::Spans(vec![span(note, Some(AMBER))]));
        }
        if list.is_empty() && notes.is_empty() && !asking {
            b.push(Block::Text("Everything is up to date.".into()));
        }
        if !list.is_empty() {
            b.push(Block::Heading(format!("Out of date: {}", list.len())));
            let head = ["", "Package", "Now", "Update gets", "Newest"].map(|t| span(t, Some(DIM))).to_vec();
            let mut rows = vec![Row { cells: head, buttons: vec![] }];
            for o in list {
                let newest = match &o.latest {
                    Some(latest) => {
                        let size = lockfiles::bump(&o.current, latest);
                        let color = match size {
                            "major" => Some(RED),
                            "minor" => Some(AMBER),
                            _ => Some(DIM),
                        };
                        span(format!("{latest}  {size}"), color)
                    }
                    None => span("—", None),
                };
                let eco = o.ecosystem.label();
                let can_update = o.wanted.as_ref().is_some_and(|w| w != &o.current) || o.ecosystem == Ecosystem::Pip;
                let direct = o.ecosystem == Ecosystem::Pip
                    || h.locks
                        .iter()
                        .filter(|l| l.ecosystem == o.ecosystem)
                        .any(|l| l.direct().iter().any(|&d| l.packages[d].name == o.name));
                let mut buttons = Vec::new();
                if can_update {
                    buttons.push(button("Update", "update", &format!("{eco}|{}", o.name)));
                }
                if direct {
                    buttons.push(button("Remove", "remove", &format!("{eco}|{}", o.name)));
                }
                rows.push(Row {
                    cells: vec![
                        span(eco, Some(DIM)),
                        span(&*o.name, None),
                        span(&*o.current, None),
                        span(o.wanted.clone().unwrap_or_else(|| "—".into()), None),
                        newest,
                    ],
                    buttons,
                });
            }
            b.push(Block::Rows(rows));
        }
    }
    if h.locks.is_empty() {
        b.push(Block::Text("No lockfile here (Cargo.lock, package-lock.json, poetry.lock or uv.lock).".into()));
        return;
    }
    for lock in &h.locks {
        let conflicts = lock.conflicts();
        b.push(Block::Heading(format!(
            "{}: {} packages, {} pinned at more than one version",
            lock.file.display(),
            lock.packages.len(),
            conflicts.len()
        )));
        let lines = conflicts.iter().map(|c| {
            let versions: Vec<String> = c
                .versions
                .iter()
                .map(|(v, by)| {
                    const SHOWN: usize = 6;
                    let who = match by.len() {
                        0 => "the project".to_owned(),
                        n if n <= SHOWN => by.join(", "),
                        n => format!("{} and {} more", by[..SHOWN].join(", "), n - SHOWN),
                    };
                    format!("{v} (wanted by {who})")
                })
                .collect();
            format!("{}: {}", c.name, versions.join("; "))
        });
        b.push(Block::Bullets(lines.collect()));
    }
    b.push(Block::Text("The tree of each lockfile is under Dependencies in the Explorer.".into()));
}

fn licences_tab(h: &Health, b: &mut Vec<Block>) {
    if let Some(note) = &h.licence_note {
        b.push(Block::Spans(vec![span(note, Some(AMBER))]));
    }
    if h.licences.is_empty() {
        let waiting = h.jobs.values().any(|j| matches!(j, Job::Metadata));
        b.push(Block::Text(if waiting {
            "Asking cargo metadata…".into()
        } else {
            "No dependencies found (Cargo.toml, or npm's node_modules).".into()
        }));
        return;
    }
    let mut counts = BTreeMap::new();
    for l in &h.licences {
        *counts.entry(packages::kind(&l.license)).or_insert(0) += 1;
    }
    let summary: Vec<String> = counts.iter().map(|(k, n)| format!("{n} {}", k.label().to_lowercase())).collect();
    b.push(Block::Text(format!("{} dependencies: {}.", h.licences.len(), summary.join(", "))));
    let toggle = if h.flagged_only { "Show all" } else { "Only those to look at" };
    b.push(bar(vec![button(toggle, "flagged", "")]));
    let mut rows = Vec::new();
    for l in &h.licences {
        let kind = packages::kind(&l.license);
        if h.flagged_only && kind == Kind::Permissive {
            continue;
        }
        let color = match kind {
            Kind::Permissive => Some(DIM),
            Kind::WeakCopyleft => Some(AMBER),
            Kind::Copyleft | Kind::Unknown => Some(RED),
        };
        let licence = if l.license.is_empty() { "(none declared)".to_owned() } else { l.license.clone() };
        rows.push(Row {
            cells: vec![
                span(format!("{} {}", l.name, l.version), None),
                span(licence, None),
                span(kind.label(), color),
                span(l.ecosystem, Some(DIM)),
            ],
            buttons: vec![],
        });
    }
    b.push(Block::Rows(rows));
}

fn audits_tab(h: &Health, b: &mut Vec<Block>) {
    let running = h.jobs.values().any(|j| matches!(j, Job::Audit(_)));
    b.push(bar(vec![button(if running { "Running…" } else { "Run the audits" }, "audit", "")]));
    b.push(Block::Text(
        "cargo audit beside a Cargo.lock, npm audit beside a package-lock.json. They download the lists of known \
         vulnerabilities themselves."
            .into(),
    ));
    let Some((list, notes)) = &h.audit else { return };
    for note in notes {
        b.push(Block::Spans(vec![span(note, Some(AMBER))]));
    }
    if list.is_empty() && notes.is_empty() && !running {
        b.push(Block::Text("No known vulnerabilities in the dependencies.".into()));
    }
    let rows = list.iter().map(|a| {
        let color = match packages::severity_rank(&a.severity) {
            0 | 1 => Some(RED),
            2 => Some(AMBER),
            _ => Some(DIM),
        };
        let mut severity = span(if a.severity.is_empty() { "—" } else { &a.severity }, color);
        severity.bold = true;
        let id = if a.url.is_empty() { a.id.clone() } else { format!("{} {}", a.id, a.url) };
        Row {
            cells: vec![
                severity,
                span(format!("{} {}", a.package, a.version), None),
                span(&*a.title, None),
                span(&*a.fix, Some(DIM)),
                span(id, Some(DIM)),
            ],
            buttons: vec![],
        }
    });
    b.push(Block::Rows(rows.collect()));
}

// ----- the dependency tree ----------------------------------------------------------------

/// Each lockfile's packages, opened one level at a time by a click. A row's id is the
/// lockfile's number and the path of packages to it.
fn render_tree() {
    let items = with(|h| {
        let mut items = Vec::new();
        for (l, lock) in h.locks.iter().enumerate() {
            let top = format!("{l}");
            items.push(TreeItem {
                id: top.clone(),
                parent: String::new(),
                label: lock.file.display().to_string(),
                description: format!("{} packages", lock.packages.len()),
                tooltip: String::new(),
                command: None,
            });
            for &r in &lock.roots {
                tree_rows(h, lock, &top, r, &mut Vec::new(), &mut items);
            }
        }
        items
    });
    host::set_tree_items("deps", &items);
}

fn tree_rows(h: &Health, lock: &Lock, parent: &str, at: usize, path: &mut Vec<usize>, items: &mut Vec<TreeItem>) {
    let p = &lock.packages[at];
    let id = format!("{parent}/{at}");
    let seen = path.contains(&at);
    let open = h.open_rows.contains(&id);
    let more = !p.deps.is_empty() && !seen;
    items.push(TreeItem {
        id: id.clone(),
        parent: parent.to_owned(),
        label: format!("{} {}", p.name, p.version),
        description: if more { format!("{} deps", p.deps.len()) } else { String::new() },
        tooltip: if seen { "Already above, on the way here".into() } else { String::new() },
        command: more.then(|| "dep".to_owned()),
    });
    if open && more && items.len() < 20_000 {
        path.push(at);
        for &d in &p.deps {
            tree_rows(h, lock, &id, d, path, items);
        }
        path.pop();
    }
}

// ----- the overview -------------------------------------------------------------------------

fn start_overview() -> Result<(), String> {
    let files = host::list_files("", MAX_FILES)?;
    with(|h| {
        h.overview = Some(Overview {
            files,
            next: 0,
            counts: overview::Counts::default(),
            git: overview::GitFacts::default(),
            git_jobs: 0,
        })
    });
    host::show_message("Counting the files for the overview…");
    host::continue_later("overview");
    Ok(())
}

fn overview_step() {
    let done = with(|h| {
        let Some(o) = h.overview.as_mut() else { return false };
        let (mut bytes, mut count) = (0, 0);
        while o.next < o.files.len() && count < STEP * 2 && bytes < STEP_BYTES * 2 {
            let rel = o.files[o.next].clone();
            o.next += 1;
            count += 1;
            let text = if overview::counted(&rel) { host::read_file(&rel).ok() } else { None };
            bytes += text.as_ref().map_or(0, String::len);
            o.counts.add(&rel, text.as_deref());
        }
        o.next >= o.files.len()
    });
    if !done {
        host::continue_later("overview");
        return;
    }
    // What Git says, if the folder is a repository and git may run.
    let asks: [&[&str]; 6] = [
        &["rev-parse", "--abbrev-ref", "HEAD"],
        &["rev-list", "--count", "HEAD"],
        &["log", "--reverse", "--format=%as", "--max-parents=0", "HEAD"],
        &["log", "-1", "--format=%as"],
        &["shortlog", "-sn", "HEAD"],
        &["describe", "--tags", "--abbrev=0"],
    ];
    let mut started = 0;
    for (i, args) in asks.iter().enumerate() {
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        if let Ok(id) = host::start_job("git", &args, "") {
            with(|h| h.jobs.insert(id, Job::Git(i)));
            started += 1;
        }
    }
    with(|h| {
        if let Some(o) = h.overview.as_mut() {
            o.git_jobs = started;
        }
    });
    if started == 0 {
        finish_overview();
    }
}

fn overview_git(which: usize, job: JobResult) {
    let all_in = with(|h| {
        let Some(o) = h.overview.as_mut() else { return false };
        let out = (job.exit_code == Some(0)).then(|| job.stdout.trim().to_owned()).filter(|s| !s.is_empty());
        match which {
            0 => o.git.branch = out,
            1 => o.git.commits = out,
            2 => o.git.first = out.and_then(|s| s.lines().next().map(str::to_owned)),
            3 => o.git.last = out,
            4 => o.git.authors = out.map(|s| s.lines().count()),
            _ => o.git.tag = out,
        }
        o.git_jobs -= 1;
        o.git_jobs == 0
    });
    if all_in {
        finish_overview();
    }
}

fn finish_overview() {
    let Some(o) = with(|h| h.overview.take()) else { return };
    let root = host::workspace_path().unwrap_or_default();
    let name = root.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or("project").to_owned();
    let readme = overview::READMES.iter().find_map(|r| host::read_file(r).ok());
    let md = overview::overview(&name, readme.as_deref(), o.counts, &o.files, &o.git);
    host::show_markdown(&format!("{name}: overview"), &md);
    host::show_message("Overview ready");
}

// ----- commands -------------------------------------------------------------------------

fn open(argument: &str) -> Result<(), String> {
    let (path, line) = argument.rsplit_once(':').ok_or("no place")?;
    host::open_location(path, line.parse().unwrap_or(1))
}

fn package(argument: &str, remove: bool) -> Result<(), String> {
    let (eco, name) = argument.split_once('|').ok_or("no package")?;
    let eco = [Ecosystem::Cargo, Ecosystem::Npm, Ecosystem::Poetry, Ecosystem::Uv, Ecosystem::Pip]
        .into_iter()
        .find(|e| e.label() == eco)
        .ok_or("unknown package manager")?;
    let command = if remove { lockfiles::remove_command(eco, name) } else { lockfiles::update_command(eco, name) };
    let (program, args) = command.ok_or("nothing to run")?;
    let title = format!("{} {name}", if remove { "Remove" } else { "Update" });
    host::run_in_terminal(&title, &program, &args)?;
    host::show_message(&format!("{title}: when it's done, Scan again shows the new versions"));
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        match id.as_str() {
            "check" => check(),
            "overview" => start_overview(),
            "tab" => {
                if let Some((tab, ..)) = TABS.iter().find(|(_, id, _)| *id == argument) {
                    with(|h| h.tab = *tab);
                    render();
                }
                Ok(())
            }
            "updates" => {
                with(|h| h.tab = Tab::Dependencies);
                check_updates();
                Ok(())
            }
            "audit" => {
                with(|h| h.tab = Tab::Audits);
                run_audits();
                Ok(())
            }
            "flagged" => {
                with(|h| h.flagged_only = !h.flagged_only);
                render();
                Ok(())
            }
            "open" => open(&argument),
            "update" => package(&argument, false),
            "remove" => package(&argument, true),
            "dep" => {
                with(|h| {
                    if !h.open_rows.remove(&argument) {
                        h.open_rows.insert(argument);
                    }
                });
                render_tree();
                Ok(())
            }
            _ => Err(format!("unknown command {id}")),
        }
    }

    fn on_event(event: Event) {
        match event {
            Event::Resumed(token) if token == "scan" => step(),
            Event::Resumed(token) if token == "overview" => overview_step(),
            Event::JobFinished(job) => finished(job),
            _ => {}
        }
    }
}

export!(Plugin);
