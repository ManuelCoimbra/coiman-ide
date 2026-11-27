//! Terminal Commands: keeps the commands run in Coiman's shells and suggests them again,
//! with the project's own commands and paths.
//!
//! - Ctrl+Space in a shell (`terminal-suggest`): a list for the line being typed: paths,
//!   the project's commands, commands run before, and the shells' own history.
//! - Terminal: Search Command History... and Terminal: Run Project Command... open the
//!   same list with one kind of row each.
//!
//! A choice replaces what was typed on the command line (erased with DEL) and, when run,
//! ends with Enter. The history is kept in the plugin's storage.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod history;
mod project;
mod routines;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Pick, PickAction, PickItem, TerminalLine};
use history::History;
use project::{Files, ProjectCommand};

/// Rows shown at most.
const SHOWN: usize = 300;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Suggest,
    History,
    Project,
}

/// The list on screen.
struct Open {
    mode: Mode,
    line: TerminalLine,
    query: String,
    project: Vec<ProjectCommand>,
    shell: Vec<String>,
}

struct State {
    history: History,
    open: Option<Open>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| State { history: History::parse(&host::storage_read()), open: None });
    f(state)
}

fn save(history: &History) {
    if let Err(e) = host::storage_write(&history.to_text()) {
        host::log(&format!("could not keep the history: {e}"));
    }
}

fn project_path() -> String {
    host::workspace_path().unwrap_or_default()
}

fn list_dir(path: &str) -> history::Listing {
    host::read_dir(path).ok().map(|entries| entries.into_iter().map(|e| (e.name, e.is_dir)).collect())
}

fn items(open: &Open, history: &History, project: &str) -> Vec<PickItem> {
    let q = open.query.as_str();
    let mut items: Vec<PickItem> = Vec::new();
    let mut push = |text: String, kind: &str, detail: String, can_forget: bool| {
        if items.len() < SHOWN && !text.is_empty() && !items.iter().any(|i| i.text == text) {
            items.push(PickItem { text, kind: kind.to_owned(), detail, can_forget });
        }
    };
    // A path once the command has a word after it, or the word looks like one.
    if open.mode == Mode::Suggest {
        let word = history::last_word(q);
        if q.contains(' ') || word.contains(['/', '\\']) {
            let head = &q[..q.len() - word.len()];
            for p in history::complete_path(&open.line.dir, word, None, &list_dir) {
                push(format!("{head}{p}"), "path", String::new(), false);
            }
        }
    }
    if open.mode != Mode::History {
        for c in open.project.iter().filter(|c| history::words_fit(q, &c.command)) {
            push(c.command.clone(), c.source, c.detail.clone(), false);
        }
    }
    if open.mode != Mode::Project {
        for e in history.search(q, project) {
            let detail = if e.count > 1 { format!("{} times", e.count) } else { String::new() };
            push(e.command.clone(), "history", detail, true);
        }
        for c in open.shell.iter().rev().filter(|c| history::words_fit(q, c)) {
            push(c.clone(), "shell history", String::new(), false);
        }
    }
    items
}

fn show() {
    let project = project_path();
    with(|s| {
        let Some(open) = &s.open else { return };
        let title = match open.mode {
            Mode::Suggest => "Commands",
            Mode::History => "Command History",
            Mode::Project => "Project Commands",
        };
        host::show_pick(&Pick {
            id: "commands".to_owned(),
            title: title.to_owned(),
            query: open.query.clone(),
            items: items(open, &s.history, &project),
            at_terminal_cursor: open.mode == Mode::Suggest,
            run_first: open.mode == Mode::Project,
            filter: false,
        });
    });
}

fn open(mode: Mode, line: TerminalLine) {
    let project = if mode == Mode::History {
        Vec::new()
    } else {
        let read = |p: &str| host::read_file(p).ok();
        project::project_commands(&Files { read: &read, list: &list_dir })
    };
    let shell = if mode == Mode::Project { Vec::new() } else { host::shell_history().unwrap_or_default() };
    let query = if mode == Mode::Suggest { line.typed.clone() } else { String::new() };
    with(|s| s.open = Some(Open { mode, line, query, project, shell }));
    show();
}

fn chosen(text: String, action: PickAction) {
    if action == PickAction::Forget {
        with(|s| {
            s.history.forget(&text);
            save(&s.history);
        });
        show();
        return;
    }
    let run = action == PickAction::Run;
    let project = project_path();
    let line = with(|s| s.open.take().map(|o| o.line));
    let Some(line) = line else { return };
    // What was typed goes, then the choice.
    let erase = "\u{7f}".repeat(line.typed.chars().count());
    let enter = if run { "\r" } else { "" };
    if let Err(e) = host::terminal_type(line.terminal, &format!("{erase}{text}{enter}")) {
        host::show_message(&e);
    }
    if run {
        with(|s| {
            s.history.record(&text, &line.dir, &project);
            save(&s.history);
        });
    }
}

/// The project's routines, offered to run.
fn pick_routine() -> Result<(), String> {
    let text = host::read_file(routines::FILE).map_err(|_| {
        format!(
            "No routines in this project: name lists of commands in {} ([[routine]] with name and steps)",
            routines::FILE
        )
    })?;
    let all = routines::parse(&text).map_err(|e| format!("{}: {e}", routines::FILE))?;
    if all.is_empty() {
        return Err(format!("{} has no [[routine]]", routines::FILE));
    }
    let items = all
        .iter()
        .map(|r| PickItem {
            text: r.name.clone(),
            kind: "routine".into(),
            detail: if r.description.is_empty() {
                r.steps.join(" → ")
            } else {
                format!("{} · {} steps", r.description, r.steps.len())
            },
            can_forget: false,
        })
        .collect();
    host::show_pick(&Pick {
        id: "routines".into(),
        title: "Run which routine?".into(),
        query: String::new(),
        items,
        at_terminal_cursor: false,
        run_first: true,
        filter: true,
    });
    Ok(())
}

/// Runs a routine in a terminal tab of its own, with the first shell that's here.
fn run_routine(name: &str) -> Result<(), String> {
    let text = host::read_file(routines::FILE).map_err(|e| format!("{}: {e}", routines::FILE))?;
    let all = routines::parse(&text)?;
    let steps = routines::expand(&all, name)?;
    let routine = all.iter().find(|r| r.name == name).ok_or("gone")?;
    let mut last = String::new();
    for (program, args) in routines::commands(routine, &steps) {
        match host::run_in_terminal(&format!("Routine: {name}"), program, &args) {
            Ok(()) => return Ok(()),
            // Not here, or not approved: the next shell.
            Err(e) if e.contains("isn't on PATH") || e.contains("not approved") => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(if last.contains("not approved") {
        format!(
            "No shell for routines is approved: allow pwsh, powershell or sh for Terminal Commands in the Extensions view ({last})"
        )
    } else {
        last
    })
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        with(|_| ());
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        match id.as_str() {
            "routine" if argument.is_empty() => return pick_routine(),
            "routine" => return run_routine(&argument),
            _ => {}
        }
        let mode = match id.as_str() {
            "suggest" => Mode::Suggest,
            "history" => Mode::History,
            "project" => Mode::Project,
            _ => return Err(format!("unknown command {id}")),
        };
        match host::active_terminal_line()? {
            Some(line) => {
                open(mode, line);
                Ok(())
            }
            None => Err("Open a terminal with a prompt first".to_owned()),
        }
    }

    fn on_event(event: Event) {
        match event {
            Event::TerminalCommand(line) => {
                let project = project_path();
                with(|s| {
                    s.history.record(&line.typed, &line.dir, &project);
                    save(&s.history);
                });
            }
            Event::TerminalSuggest(line) => open(Mode::Suggest, line),
            Event::Picked(choice) if choice.pick == "commands" => chosen(choice.text, choice.action),
            Event::Picked(choice) if choice.pick == "routines" => {
                if let Err(e) = run_routine(&choice.text) {
                    host::show_message(&e);
                }
            }
            Event::PickQueryChanged(q) if q.pick == "commands" => {
                with(|s| {
                    if let Some(open) = &mut s.open {
                        open.query = q.query;
                    }
                });
                show();
            }
            _ => {}
        }
    }
}

export!(Plugin);
