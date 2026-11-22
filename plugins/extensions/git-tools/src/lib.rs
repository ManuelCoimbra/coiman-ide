//! Git Tools: searching the history (by message, author, file, or a change that adds or
//! removes a text), submodules, hooks, and what each `.gitignore` rule hides, in a panel with a
//! tab for each. Everything goes through `git`, run as approved jobs.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod parse;

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FieldKind, FileLink, Form, FormField, FormValues, JobResult, Row, Span};
use parse::{Found, HookState, Rule, SearchKind, SubState, Submodule};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;
const AMBER: u32 = 0xF29E4C;
const DIM: u32 = 0x8B949E;
const ACCENT: u32 = 0x3B8EEA;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Search,
    Submodules,
    Hooks,
    Ignore,
}

const TABS: [(Tab, &str, &str); 4] = [
    (Tab::Search, "search", "Search History"),
    (Tab::Submodules, "submodules", "Submodules"),
    (Tab::Hooks, "hooks", "Hooks"),
    (Tab::Ignore, "ignore", ".gitignore"),
];

/// What a running `git` is for.
enum Then {
    Search,
    Lines(String),
    SubConfig,
    SubStatus(String),
    /// A submodule action: what to say when it worked, and a second command to run after it.
    SubAction(String, Option<Vec<String>>),
    HooksPath,
    HooksDir(Option<String>),
    HookRun(String),
    IgnoreList,
    IgnoreVerbose,
    Why(String),
    PreviewUntracked(String),
    PreviewTracked(String, Vec<String>),
}

/// A line a commit added or removed that matches the search: file, + or -, text.
type MatchLine = (String, char, String);
/// The rule that hides a path: file, line, pattern.
type Because = (String, usize, String);

#[derive(Default)]
struct State {
    tab: Tab,
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    // Search.
    kind: SearchKind,
    query: String,
    found: Option<Vec<Found>>,
    open_commit: Option<usize>,
    lines: Option<(String, Vec<MatchLine>)>,
    // Submodules.
    subs: Option<Vec<Submodule>>,
    confirm_remove: Option<String>,
    // Hooks.
    hooks_dir: Option<String>,
    custom_hooks: Option<String>,
    hooks: Option<Vec<(&'static str, HookState)>>,
    hook_output: Option<(String, bool, String)>,
    // .gitignore.
    rules: Option<Vec<Rule>>,
    open_rules: HashSet<(String, usize)>,
    why: Option<(String, Option<Because>)>,
    preview: Option<(String, Vec<String>, Vec<String>)>,
    /// Asks a form's field to be emptied when the panel is next drawn.
    clear: HashSet<&'static str>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

fn git(args: Vec<String>, stdin: &str, then: Then, busy: &str) {
    // Paths as they are (not quoted with octal escapes), and no colors in what is read.
    let mut all: Vec<String> = ["-c", "core.quotepath=off", "-c", "color.ui=false"].map(str::to_owned).into();
    all.extend(args);
    match host::start_job("git", &all, stdin) {
        Ok(id) => with(|s| {
            s.jobs.insert(id, then);
            s.busy = Some(busy.to_owned());
            s.error = None;
        }),
        Err(e) => with(|s| s.error = Some(e)),
    }
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| (*a).to_owned()).collect()
}

// ----- loading each tab -------------------------------------------------------------------

fn load(tab: Tab) {
    match tab {
        Tab::Search => {}
        Tab::Submodules => {
            if host::read_file(".gitmodules").is_err() {
                with(|s| s.subs = Some(Vec::new()));
                return;
            }
            let a = args(&["config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.(path|url|branch)$"]);
            git(a, "", Then::SubConfig, "Reading…");
        }
        Tab::Hooks => git(args(&["config", "--get", "core.hooksPath"]), "", Then::HooksPath, "Reading…"),
        Tab::Ignore => {
            let a = args(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory"]);
            git(a, "", Then::IgnoreList, "Reading the rules…");
        }
    }
}

fn read_hooks() {
    let dir = with(|s| s.hooks_dir.clone());
    let Some(dir) = dir else { return };
    let names: Vec<String> = host::read_dir(&dir).map(|e| e.into_iter().map(|e| e.name).collect()).unwrap_or_default();
    with(|s| s.hooks = Some(parse::hook_states(&names)));
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let ok = job.exit_code == Some(0);
    let said = || {
        let t = if job.stderr.trim().is_empty() { job.stdout.trim() } else { job.stderr.trim() };
        if t.is_empty() { "git failed".to_owned() } else { t.to_owned() }
    };
    with(|s| s.busy = None);
    match then {
        Then::Search => with(|s| {
            if ok {
                s.found = Some(parse::parse_log(&job.stdout));
                s.open_commit = None;
                s.lines = None;
            } else {
                s.error = Some(said());
            }
        }),
        Then::Lines(hash) => with(|s| {
            let regex = s.kind == SearchKind::Regex;
            s.lines = Some((hash, parse::matching_lines(&job.stdout, &s.query, regex)));
        }),
        Then::SubConfig => git(args(&["submodule", "status"]), "", Then::SubStatus(job.stdout), "Reading…"),
        Then::SubStatus(config) => with(|s| s.subs = Some(parse::submodules(&config, &job.stdout))),
        Then::SubAction(done, next) => {
            if !ok {
                with(|s| s.error = Some(parse::explain(&said())));
            } else if let Some(next) = next {
                git(next, "", Then::SubAction(done, None), "Removing…");
                return;
            } else {
                host::show_message(&done);
            }
            load(Tab::Submodules);
        }
        Then::HooksPath => {
            let custom = Some(job.stdout.trim().to_owned()).filter(|c| ok && !c.is_empty());
            git(args(&["rev-parse", "--git-path", "hooks"]), "", Then::HooksDir(custom), "Reading…");
            return;
        }
        Then::HooksDir(custom) => {
            if ok {
                let root = host::workspace_path().unwrap_or_default().replace('\\', "/");
                let dir = job.stdout.trim().replace('\\', "/");
                let dir =
                    dir.strip_prefix(&format!("{}/", root.trim_end_matches('/'))).map(str::to_owned).unwrap_or(dir);
                with(|s| {
                    s.hooks_dir = Some(dir);
                    s.custom_hooks = custom;
                });
                read_hooks();
            } else {
                with(|s| s.error = Some(said()));
            }
        }
        Then::HookRun(name) => with(|s| {
            // Git sends a hook's output to stderr; both are what it printed.
            let text = format!("{}{}", job.stdout, job.stderr);
            let text =
                if text.trim().is_empty() { "(it printed nothing)".to_owned() } else { text.trim_end().to_owned() };
            s.hook_output = Some((name, ok, text));
        }),
        Then::IgnoreList => {
            let paths: Vec<&str> = job.stdout.lines().filter(|l| !l.is_empty()).collect();
            if !ok {
                with(|s| s.error = Some(said()));
            } else if paths.is_empty() {
                set_rules("");
            } else {
                let a = args(&["check-ignore", "-v", "--stdin", "--no-index"]);
                git(a, &paths.join("\n"), Then::IgnoreVerbose, "Reading the rules…");
                return;
            }
        }
        Then::IgnoreVerbose => set_rules(&job.stdout),
        // It exits 1 when nothing is ignored: the same answer, not a failure.
        Then::Why(path) => with(|s| s.why = Some((path, parse::why(&job.stdout)))),
        Then::PreviewUntracked(pattern) => {
            let take: Vec<String> =
                job.stdout.lines().filter(|l| !l.is_empty()).take(parse::HIDDEN_MAX).map(str::to_owned).collect();
            let a =
                vec!["ls-files".into(), "--cached".into(), "--ignored".into(), format!("--exclude={}", pattern.trim())];
            git(a, "", Then::PreviewTracked(pattern, take), "Looking…");
            return;
        }
        Then::PreviewTracked(pattern, untracked) => with(|s| {
            let tracked =
                job.stdout.lines().filter(|l| !l.is_empty()).take(parse::HIDDEN_MAX).map(str::to_owned).collect();
            s.preview = Some((pattern, untracked, tracked));
        }),
    }
    render();
}

fn set_rules(verbose: &str) {
    let root = host::workspace_path().unwrap_or_default().replace('\\', "/");
    let gitignore = host::read_file(".gitignore").ok();
    let rules = parse::rules(verbose, gitignore.as_deref(), &root);
    with(|s| s.rules = Some(rules));
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn mono(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: true }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn text_field(id: &str, label: &str, hint: &str, value: &str) -> FormField {
    FormField {
        id: id.to_owned(),
        label: label.to_owned(),
        kind: FieldKind::Text,
        value: value.to_owned(),
        options: vec![],
        hint: hint.to_owned(),
    }
}

fn render() {
    let blocks = with(|s| {
        let mut tabs: Vec<Button> = TABS
            .iter()
            .map(|(tab, id, name)| {
                let label = if *tab == s.tab { format!("▸ {name}") } else { (*name).to_owned() };
                button(&label, "tab", id)
            })
            .collect();
        if s.tab != Tab::Search {
            tabs.push(button("Refresh", "refresh", ""));
        }
        let mut b = vec![Block::Rows(vec![Row { cells: vec![], buttons: tabs }])];
        if let Some(busy) = &s.busy {
            b.push(Block::Spans(vec![span(busy, Some(DIM))]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        match s.tab {
            Tab::Search => search_tab(s, &mut b),
            Tab::Submodules => subs_tab(s, &mut b),
            Tab::Hooks => hooks_tab(s, &mut b),
            Tab::Ignore => ignore_tab(s, &mut b),
        }
        s.clear.clear();
        b
    });
    host::set_panel("git", &blocks);
}

fn search_tab(s: &State, b: &mut Vec<Block>) {
    let hint = match s.kind {
        SearchKind::Message => "words in the message",
        SearchKind::Change => "a name or text that came or went",
        SearchKind::Regex => r"a regular expression: fn \w+_test",
        SearchKind::Author => "a name or e-mail",
        SearchKind::Path => "src/parser.rs, docs/, *.toml",
    };
    b.push(Block::Form(Form {
        id: "search".into(),
        fields: vec![
            FormField {
                id: "kind".into(),
                label: String::new(),
                kind: FieldKind::Choice,
                value: s.kind.label().into(),
                options: SearchKind::ALL.iter().map(|k| k.label().to_owned()).collect(),
                hint: String::new(),
            },
            text_field("query", "", hint, ""),
            text_field("within", "only in", "a file or folder", ""),
            FormField {
                id: "all".into(),
                label: "All branches".into(),
                kind: FieldKind::Check,
                value: "false".into(),
                options: vec![],
                hint: String::new(),
            },
        ],
        buttons: vec!["Search".into()],
    }));
    let Some(found) = &s.found else {
        b.push(Block::Spans(vec![span(
            "Added or removed text finds where something came in or went out (git log -S).",
            Some(DIM),
        )]));
        return;
    };
    let more = if found.len() >= parse::FOUND_MAX { " (the newest; narrow the search for older ones)" } else { "" };
    b.push(Block::Spans(vec![span(format!("{} commit{}{more}", found.len(), if found.len() == 1 { "" } else { "s" }), Some(DIM))]));
    let mut rows = Vec::new();
    for (i, f) in found.iter().enumerate() {
        let open = s.open_commit == Some(i);
        rows.push(Row {
            cells: vec![
                mono(&*f.short, Some(ACCENT)),
                span(&*f.subject, None),
                span(format!("{} · {}", f.author, parse::date(f.when)), Some(DIM)),
            ],
            buttons: vec![button(if open { "Hide files" } else { "Files" }, "commit", &i.to_string())],
        });
        if open {
            for (status, file) in &f.files {
                let color = match status {
                    'A' => GREEN,
                    'D' => RED,
                    _ => AMBER,
                };
                rows.push(Row {
                    cells: vec![mono(format!("   {status}"), Some(color)), mono(file, None), span("", None)],
                    buttons: vec![button("Open change", "diff", &format!("{}|{file}", f.hash))],
                });
            }
        }
    }
    b.push(Block::Rows(rows));
    if let (Some(i), Some((hash, lines))) = (s.open_commit, &s.lines)
        && found.get(i).is_some_and(|f| &f.hash == hash)
    {
        b.push(Block::Heading("The lines that match".into()));
        for (file, mark, text) in lines {
            let color = if *mark == '+' { GREEN } else { RED };
            b.push(Block::Spans(vec![
                mono(format!("{mark} {text}"), Some(color)),
                span(format!("   {file}"), Some(DIM)),
            ]));
        }
    }
}

fn subs_tab(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Rows(vec![Row { cells: vec![], buttons: vec![button("Fetch all", "sub-fetch", "")] }]));
    match &s.subs {
        None => b.push(Block::Spans(vec![span("Reading…", Some(DIM))])),
        Some(subs) if subs.is_empty() => b.push(Block::Text("No submodules in this repository.".into())),
        Some(subs) => {
            let rows = subs.iter().map(|m| {
                let color = match m.state {
                    SubState::Ok => GREEN,
                    SubState::NotInitialized | SubState::Moved => AMBER,
                    SubState::Conflict => RED,
                };
                let branch = m.branch.as_ref().map(|b| format!(" ({b})")).unwrap_or_default();
                let mut buttons = vec![button("Fetch", "sub-fetch", &m.path), button("Newest", "sub-latest", &m.path)];
                if s.confirm_remove.as_deref() == Some(m.path.as_str()) {
                    buttons.push(button("Remove it", "sub-remove", &m.path));
                    buttons.push(button("Keep", "sub-keep", ""));
                } else {
                    buttons.push(button("Remove…", "sub-ask", &m.path));
                }
                Row {
                    cells: vec![
                        Span { text: m.path.clone(), color: None, bold: true, monospace: true },
                        span(m.state.label(), Some(color)),
                        span(format!("{}{branch}", m.url), Some(DIM)),
                    ],
                    buttons,
                }
            });
            b.push(Block::Rows(rows.collect()));
        }
    }
    b.push(Block::Heading("Add a submodule".into()));
    b.push(Block::Form(Form {
        id: "add".into(),
        fields: vec![
            text_field("url", "", "https://github.com/owner/lib.git", if s.clear.contains("add") { " " } else { "" }),
            text_field("path", "into", "vendor/lib", if s.clear.contains("add") { " " } else { "" }),
        ],
        buttons: vec!["Add".into()],
    }));
}

fn hooks_tab(s: &State, b: &mut Vec<Block>) {
    if let Some(c) = &s.custom_hooks {
        b.push(Block::Spans(vec![span(
            format!("core.hooksPath is {c}: a tool such as Husky or Lefthook runs these."),
            Some(AMBER),
        )]));
    }
    b.push(Block::Spans(vec![span(
        "Scripts Git runs at points of its work. Off renames one so Git skips it; Run runs it now, as Git would.",
        Some(DIM),
    )]));
    if let Some((name, ok, out)) = &s.hook_output {
        b.push(Block::Spans(vec![Span {
            text: if *ok { format!("{name} ran:") } else { format!("{name} stopped:") },
            color: Some(if *ok { GREEN } else { RED }),
            bold: true,
            monospace: false,
        }]));
        b.push(Block::Code(out.clone()));
    }
    let Some(hooks) = &s.hooks else {
        b.push(Block::Spans(vec![span("Reading…", Some(DIM))]));
        return;
    };
    let rows = hooks.iter().map(|(name, state)| {
        let (label, color) = match state {
            HookState::On => ("on", Some(GREEN)),
            HookState::Off => ("off", Some(AMBER)),
            HookState::Sample => ("example only", Some(DIM)),
            HookState::None => ("", None),
        };
        let buttons = match state {
            HookState::On => vec![
                button("Edit", "hook-edit", name),
                button("Run", "hook-run", name),
                button("Off", "hook-toggle", name),
            ],
            HookState::Off => vec![button("On", "hook-toggle", name)],
            _ => vec![button("Create", "hook-create", name)],
        };
        Row { cells: vec![mono(*name, None), span(label, color)], buttons }
    });
    b.push(Block::Rows(rows.collect()));
}

fn ignore_tab(s: &State, b: &mut Vec<Block>) {
    b.push(Block::Form(Form {
        id: "why".into(),
        fields: vec![text_field("path", "Why is", "a path: target/debug/app", "")],
        buttons: vec!["Check".into()],
    }));
    if let Some((path, answer)) = &s.why {
        match answer {
            Some((source, line, pattern)) => {
                b.push(Block::Spans(vec![mono(format!("{path}: {pattern}"), Some(AMBER))]));
                b.push(Block::Links(vec![FileLink {
                    text: format!("{source}:{line}"),
                    detail: "the rule that hides it".into(),
                    path: source.clone(),
                    line: *line as u32,
                }]));
            }
            None => b.push(Block::Spans(vec![span(format!("{path} isn't ignored"), Some(GREEN))])),
        }
    }
    b.push(Block::Form(Form {
        id: "rule".into(),
        fields: vec![text_field(
            "pattern",
            "New rule",
            "*.log, build/, /secret.env",
            if s.clear.contains("rule") { " " } else { "" },
        )],
        buttons: vec!["What would it hide?".into(), "Add to .gitignore".into()],
    }));
    if let Some((p, untracked, tracked)) = &s.preview {
        let n = untracked.len();
        b.push(Block::Spans(vec![span(
            format!("{p} would hide {n} untracked {}", if n == 1 { "entry" } else { "entries" }),
            Some(DIM),
        )]));
        if !untracked.is_empty() {
            b.push(Block::Code(untracked.iter().take(12).cloned().collect::<Vec<_>>().join("   ")));
        }
        if !tracked.is_empty() {
            let t = tracked.len();
            b.push(Block::Spans(vec![span(
                format!(
                    "{t} tracked {} too, and {} tracked: git rm --cached to stop tracking ({})",
                    if t == 1 { "file matches" } else { "files match" },
                    if t == 1 { "it stays" } else { "they stay" },
                    tracked.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                ),
                Some(AMBER),
            )]));
        }
    }
    let Some(rules) = &s.rules else {
        b.push(Block::Spans(vec![span("Reading…", Some(DIM))]));
        return;
    };
    b.push(Block::Spans(vec![span(
        "Each rule and what it hides now (folders count once); a rule that hides nothing can go.",
        Some(DIM),
    )]));
    let mut rows = Vec::new();
    for r in rules {
        let key = (r.source.clone(), r.line);
        let open = s.open_rules.contains(&key);
        let place = format!("{}|{}", r.source, r.line);
        let mut buttons = vec![button("Open", "open-rule", &place)];
        let what = if r.count == 0 {
            span("hides nothing", Some(DIM))
        } else {
            buttons.push(button(if open { "Hide" } else { "Show" }, "rule-toggle", &place));
            span(format!("{} {}", r.count, if r.count == 1 { "entry" } else { "entries" }), None)
        };
        rows.push(Row {
            cells: vec![
                span(format!("{}:{}", r.source, r.line), Some(DIM)),
                Span { text: r.pattern.clone(), color: None, bold: true, monospace: true },
                what,
            ],
            buttons,
        });
        if open {
            rows.push(Row {
                cells: vec![span("", None), mono(r.hidden.join("\n"), Some(DIM)), span("", None)],
                buttons: vec![],
            });
        }
    }
    b.push(Block::Rows(rows));
}

// ----- what the user does ---------------------------------------------------------------------

fn submitted(f: FormValues) {
    let get = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    match f.form.as_str() {
        "search" => {
            let kind = SearchKind::from_label(&get("kind"));
            let query = get("query");
            with(|s| {
                s.kind = kind;
                s.query = query.clone();
            });
            match parse::search_args(kind, &query, get("all") == "true", &get("within")) {
                Some(a) => git(a, "", Then::Search, "Searching…"),
                None => with(|s| s.found = None),
            }
        }
        "add" => {
            let (url, path) = (get("url"), get("path"));
            if url.is_empty() || path.is_empty() {
                with(|s| s.error = Some("Give the repository's address and the folder to put it in".into()));
            } else {
                let a = vec!["submodule".into(), "add".into(), "--".into(), url, path.clone()];
                git(a, "", Then::SubAction(format!("Added {path}: commit to record it"), None), "Adding…");
                with(|s| {
                    s.clear.insert("add");
                });
            }
        }
        "why" => {
            let path = get("path");
            if !path.is_empty() {
                let a = args(&["check-ignore", "-v", "-n", "--stdin", "--no-index"]);
                git(a, &path, Then::Why(path.clone()), "Checking…");
            }
        }
        "rule" => {
            let pattern = get("pattern");
            if pattern.is_empty() {
                return render();
            }
            if f.button.starts_with("Add") {
                let text = host::read_file(".gitignore").unwrap_or_default();
                match host::write_file(".gitignore", &parse::with_pattern(&text, &pattern)) {
                    Ok(()) => {
                        host::show_message(&format!("Added {pattern} to .gitignore"));
                        with(|s| {
                            s.preview = None;
                            s.clear.insert("rule");
                        });
                        load(Tab::Ignore);
                    }
                    Err(e) => with(|s| s.error = Some(e)),
                }
            } else {
                let a = vec![
                    "ls-files".into(),
                    "--others".into(),
                    "--ignored".into(),
                    "--directory".into(),
                    format!("--exclude={pattern}"),
                ];
                git(a, "", Then::PreviewUntracked(pattern), "Looking…");
            }
        }
        _ => {}
    }
    render();
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let show = |tab: Tab| {
        with(|s| s.tab = tab);
        load(tab);
    };
    match id {
        "search" => show(Tab::Search),
        "submodules" => show(Tab::Submodules),
        "hooks" => show(Tab::Hooks),
        "ignore" => show(Tab::Ignore),
        "tab" => {
            if let Some((tab, ..)) = TABS.iter().find(|(_, i, _)| *i == arg) {
                with(|s| {
                    s.tab = *tab;
                    s.error = None;
                });
                load(*tab);
            }
        }
        "refresh" => load(with(|s| s.tab)),
        "commit" => {
            let i: usize = arg.parse().unwrap_or(0);
            let (open, hash, kind) = with(|s| {
                let open = s.open_commit != Some(i);
                s.open_commit = open.then_some(i);
                s.lines = None;
                (open, s.found.as_ref().and_then(|f| f.get(i)).map(|f| f.hash.clone()), s.kind)
            });
            if let (true, Some(hash)) = (open, hash)
                && matches!(kind, SearchKind::Change | SearchKind::Regex)
            {
                let a = vec!["show".into(), "--format=".into(), "-U0".into(), "--no-renames".into(), hash.clone()];
                git(a, "", Then::Lines(hash), "Reading…");
            }
        }
        "diff" => {
            let (hash, file) = arg.split_once('|').ok_or("no change")?;
            host::open_commit_diff(hash, file)?;
        }
        "sub-fetch" => {
            let mut a = args(&["submodule", "update", "--init", "--recursive"]);
            if !arg.is_empty() {
                a.extend(["--".into(), arg.to_owned()]);
            }
            git(a, "", Then::SubAction("Submodules fetched and checked out".into(), None), "Fetching submodules…");
        }
        "sub-latest" => {
            let a = vec!["submodule".into(), "update".into(), "--remote".into(), "--".into(), arg.to_owned()];
            let done = format!("{arg} is at its branch's newest commit: commit to record it");
            git(a, "", Then::SubAction(done, None), "Updating…");
        }
        "sub-ask" => with(|s| s.confirm_remove = Some(arg.to_owned())),
        "sub-keep" => with(|s| s.confirm_remove = None),
        "sub-remove" => {
            with(|s| s.confirm_remove = None);
            // Unregistered, then its folder and .gitmodules entry removed (staged, for a commit).
            let first = vec!["submodule".into(), "deinit".into(), "-f".into(), "--".into(), arg.to_owned()];
            let then = vec!["rm".into(), "-f".into(), "--".into(), arg.to_owned()];
            git(first, "", Then::SubAction(format!("Removed {arg}: commit to record it"), Some(then)), "Removing…");
        }
        "hook-edit" | "hook-create" | "hook-toggle" => {
            let dir = with(|s| s.hooks_dir.clone()).ok_or("The hooks folder isn't known yet")?;
            let (on, off) = (format!("{dir}/{arg}"), format!("{dir}/{arg}.off"));
            let there = |p: &str| host::read_file(p).is_ok();
            match id {
                "hook-edit" => host::open_location(&on, 1)?,
                "hook-create" => {
                    if !there(&on) {
                        if there(&off) {
                            host::rename_file(&off, &on)?;
                        } else {
                            host::write_file(&on, &parse::hook_template(arg))?;
                        }
                    }
                    host::set_executable(&on)?;
                    host::open_location(&on, 1)?;
                }
                _ if there(&on) => host::rename_file(&on, &off)?,
                _ if there(&off) => host::rename_file(&off, &on)?,
                _ => return Err(format!("There's no {arg} hook to turn on or off")),
            }
            read_hooks();
        }
        "hook-run" => {
            let a = vec!["hook".into(), "run".into(), "--ignore-missing".into(), arg.to_owned()];
            git(a, "", Then::HookRun(arg.to_owned()), "Running the hook…");
        }
        "rule-toggle" | "open-rule" => {
            let (source, line) = arg.rsplit_once('|').ok_or("no rule")?;
            let line: usize = line.parse().unwrap_or(1);
            if id == "open-rule" {
                host::open_location(source, line as u32)?;
            } else {
                with(|s| {
                    let key = (source.to_owned(), line);
                    if !s.open_rules.remove(&key) {
                        s.open_rules.insert(key);
                    }
                });
            }
        }
        _ => return Err(format!("unknown command {id}")),
    }
    render();
    Ok(())
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        command(&id, &argument)
    }

    fn on_event(event: Event) {
        match event {
            Event::JobFinished(job) => finished(job),
            Event::FormSubmitted(f) => submitted(f),
            _ => {}
        }
    }
}

export!(Plugin);
