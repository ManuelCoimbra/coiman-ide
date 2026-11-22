//! Docker: the containers, images and the project's Compose services, in a panel, with what
//! can be done to them. Everything goes through the `docker` program, run as an approved job;
//! logs, shells and `compose up` run in a terminal tab.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

mod docker;

use std::collections::HashMap;
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, JobResult, Row, Span};
use docker::{Container, ContainerAction, Image, Service};

const RED: u32 = 0xE5484D;
const GREEN: u32 = 0x57C06A;
const YELLOW: u32 = 0xCCA700;
const DIM: u32 = 0x8B949E;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Containers,
    Images,
    Compose,
}

/// What a running `docker` is for.
enum Then {
    Containers,
    Images,
    Services,
    /// A change: what to say when it worked, and what was tried (for a failure).
    Done(String, String),
}

#[derive(Default)]
struct State {
    tab: Tab,
    jobs: HashMap<u64, Then>,
    busy: Option<String>,
    error: Option<String>,
    containers: Option<Result<Vec<Container>, String>>,
    images: Option<Result<Vec<Image>, String>>,
    services: Option<Result<Vec<Service>, String>>,
    compose: Option<String>,
    /// A container or image to remove, once confirmed.
    confirm: Option<String>,
    /// The tab was chosen by the user: a refresh doesn't change it.
    chosen: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// What starting `docker` failing means.
fn not_started(e: &str) -> String {
    if e.contains("isn't on PATH") {
        "Docker isn't installed (no docker program on PATH)".to_owned()
    } else {
        e.to_owned()
    }
}

fn docker(args: Vec<String>, then: Then, busy: &str) -> Result<(), String> {
    let id = host::start_job("docker", &args, "").map_err(|e| not_started(&e))?;
    with(|s| {
        s.jobs.insert(id, then);
        s.busy = Some(busy.to_owned());
    });
    Ok(())
}

fn refresh() {
    let compose = docker::COMPOSE_FILES.iter().find(|f| host::read_file(f).is_ok()).map(|f| (*f).to_owned());
    with(|s| {
        if compose.is_some() && s.compose.is_none() && !s.chosen {
            s.tab = Tab::Compose;
        }
        s.compose = compose;
        s.error = None;
    });
    if let Err(e) = docker(docker::ps_args(), Then::Containers, "Asking Docker…") {
        with(|s| {
            s.containers = Some(Err(e.clone()));
            s.images = Some(Err(e.clone()));
            s.services = s.compose.as_ref().map(|_| Err(e));
        });
    }
}

fn finished(job: JobResult) {
    let Some(then) = with(|s| s.jobs.remove(&job.id)) else { return };
    let ok = job.exit_code == Some(0);
    let said = docker::explain(&job.stderr);
    if with(|s| s.jobs.is_empty()) {
        with(|s| s.busy = None);
    }
    match then {
        Then::Containers => {
            if ok {
                with(|s| s.containers = Some(Ok(docker::parse_containers(&job.stdout))));
                let _ = docker(docker::images_args(), Then::Images, "Asking Docker…");
            } else {
                // Without an engine, the rest would only say so again.
                with(|s| {
                    s.containers = Some(Err(said.clone()));
                    s.images = Some(Err(said.clone()));
                    s.services = s.compose.as_ref().map(|_| Err(said));
                });
            }
        }
        Then::Images => {
            with(|s| s.images = Some(if ok { Ok(docker::parse_images(&job.stdout)) } else { Err(said) }));
            if let Some(file) = with(|s| s.compose.clone()) {
                let _ = docker(docker::compose_ps_args(&file), Then::Services, "Asking Docker…");
            }
        }
        Then::Services => {
            with(|s| s.services = Some(if ok { Ok(docker::parse_services(&job.stdout)) } else { Err(said) }));
        }
        Then::Done(done, what) => {
            if ok {
                host::show_message(&done);
                refresh();
            } else {
                with(|s| s.error = Some(format!("Couldn't {what}: {said}")));
            }
        }
    }
    render();
}

// ----- the panel ----------------------------------------------------------------------------

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn mono(text: impl Into<String>) -> Span {
    Span { text: text.into(), color: None, bold: false, monospace: true }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn bar(buttons: Vec<Button>) -> Block {
    Block::Rows(vec![Row { cells: vec![], buttons }])
}

fn state_color(state: &str) -> Option<u32> {
    match state {
        "running" => Some(GREEN),
        "paused" | "restarting" | "created" => Some(YELLOW),
        _ => Some(DIM),
    }
}

fn count<T>(found: &Option<Result<Vec<T>, String>>, label: &str) -> String {
    match found {
        Some(Ok(list)) => format!("{label}  {}", list.len()),
        _ => label.to_owned(),
    }
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        let tab =
            |t: Tab, label: String, id: &str| button(&if s.tab == t { format!("▸ {label}") } else { label }, "tab", id);
        b.push(bar(vec![
            tab(Tab::Containers, count(&s.containers, "Containers"), "containers"),
            tab(Tab::Images, count(&s.images, "Images"), "images"),
            tab(Tab::Compose, count(&s.services, "Compose"), "compose"),
            button("Refresh", "refresh", ""),
        ]));
        if let Some(busy) = &s.busy {
            b.push(Block::Spans(vec![span(busy, Some(DIM))]));
        }
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e, Some(RED))]));
        }
        match s.tab {
            Tab::Containers => match &s.containers {
                None => {}
                Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
                Some(Ok(list)) if list.is_empty() => b.push(Block::Text("No containers.".into())),
                Some(Ok(list)) => {
                    let rows = list
                        .iter()
                        .map(|c| {
                            let name = &c.name;
                            let mut buttons = if c.running() {
                                vec![
                                    button("Stop", "stop", name),
                                    button("Restart", "restart", name),
                                    button("Shell", "shell", name),
                                ]
                            } else if s.confirm.as_deref() == Some(name.as_str()) {
                                vec![button("Remove it", "remove", name), button("Keep", "keep", "")]
                            } else {
                                vec![button("Start", "start", name), button("Remove…", "ask", name)]
                            };
                            buttons.push(button("Logs", "logs", name));
                            Row {
                                cells: vec![
                                    Span {
                                        text: name.clone(),
                                        color: state_color(&c.state),
                                        bold: true,
                                        monospace: false,
                                    },
                                    mono(&*c.image),
                                    span(&*c.status, None),
                                    span(&*c.ports, Some(DIM)),
                                ],
                                buttons,
                            }
                        })
                        .collect();
                    b.push(Block::Rows(rows));
                }
            },
            Tab::Images => match &s.images {
                None => {}
                Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
                Some(Ok(list)) if list.is_empty() => b.push(Block::Text("No images.".into())),
                Some(Ok(list)) => {
                    let rows = list
                        .iter()
                        .map(|i| {
                            let r = i.reference();
                            let buttons = if s.confirm.as_deref() == Some(r.as_str()) {
                                vec![button("Remove it", "remove-image", &r), button("Keep", "keep", "")]
                            } else {
                                vec![button("Remove…", "ask", &r)]
                            };
                            Row { cells: vec![mono(&*r), span(&*i.size, None), span(&*i.created, Some(DIM))], buttons }
                        })
                        .collect();
                    b.push(Block::Rows(rows));
                }
            },
            Tab::Compose => match &s.compose {
                None => {
                    b.push(Block::Text("No Compose file in the open folder (compose.yaml, docker-compose.yml).".into()))
                }
                Some(file) => {
                    b.push(Block::Spans(vec![mono(&**file)]));
                    b.push(bar(vec![
                        button("Up", "compose", "up"),
                        button("Down", "compose", "down"),
                        button("Build", "compose", "build"),
                        button("Logs", "compose", "logs"),
                    ]));
                    b.push(Block::Spans(vec![span(
                        "Up, Build and Logs run in a terminal (Ctrl+C stops them); Down stops and removes the services.",
                        Some(DIM),
                    )]));
                    match &s.services {
                        None => {}
                        Some(Err(e)) => b.push(Block::Spans(vec![span(e, Some(RED))])),
                        Some(Ok(list)) if list.is_empty() => {
                            b.push(Block::Text("No services running: Up starts them.".into()))
                        }
                        Some(Ok(list)) => {
                            let rows = list
                                .iter()
                                .map(|v| Row {
                                    cells: vec![
                                        Span {
                                            text: v.service.clone(),
                                            color: state_color(&v.state),
                                            bold: true,
                                            monospace: false,
                                        },
                                        mono(&*v.name),
                                        span(&*v.status, None),
                                        span(&*v.ports, Some(DIM)),
                                    ],
                                    buttons: vec![],
                                })
                                .collect();
                            b.push(Block::Rows(rows));
                        }
                    }
                }
            },
        }
        b
    });
    host::set_panel("docker", &blocks);
}

// ----- commands -----------------------------------------------------------------------------

fn act(action: ContainerAction, id: &str) -> Result<(), String> {
    let verb = action.args(id)[0].replace("rm", "remove");
    let done = format!("{} {id}", action.done());
    docker(action.args(id), Then::Done(done, format!("{verb} {id}")), &format!("docker {verb} {id}…"))
}

fn terminal(title: &str, args: Vec<String>) -> Result<(), String> {
    host::run_in_terminal(title, "docker", &args).map_err(|e| not_started(&e))
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    let result = match id {
        "show" | "refresh" => {
            refresh();
            Ok(())
        }
        "tab" => {
            with(|s| {
                s.chosen = true;
                s.tab = match arg {
                    "images" => Tab::Images,
                    "compose" => Tab::Compose,
                    _ => Tab::Containers,
                }
            });
            Ok(())
        }
        "start" => act(ContainerAction::Start, arg),
        "stop" => act(ContainerAction::Stop, arg),
        "restart" => act(ContainerAction::Restart, arg),
        "ask" => {
            with(|s| s.confirm = Some(arg.to_owned()));
            Ok(())
        }
        "keep" => {
            with(|s| s.confirm = None);
            Ok(())
        }
        "remove" => {
            with(|s| s.confirm = None);
            act(ContainerAction::Remove, arg)
        }
        "remove-image" => {
            with(|s| s.confirm = None);
            let a = vec!["rmi".to_owned(), arg.to_owned()];
            docker(a, Then::Done(format!("Removed {arg}"), format!("remove {arg}")), &format!("docker rmi {arg}…"))
        }
        "logs" => terminal(&format!("logs {arg}"), docker::logs_args(arg)),
        "shell" => terminal(&format!("shell {arg}"), docker::shell_args(arg)),
        "compose" => {
            let file = with(|s| s.compose.clone()).ok_or("No Compose file in the open folder")?;
            let mut a = docker::compose(&file);
            match arg {
                "up" => a.extend(["up".to_owned(), "--build".to_owned()]),
                "logs" => a.extend(["logs", "-f", "--tail", "200"].map(str::to_owned)),
                "down" => a.push("down".to_owned()),
                _ => a.push("build".to_owned()),
            }
            if arg == "down" {
                docker(a, Then::Done("Compose: down".into(), "run compose down".into()), "docker compose down…")
            } else {
                terminal(&format!("compose {arg}"), a)
            }
        }
        _ => return Err(format!("unknown command {id}")),
    };
    if let Err(e) = result {
        with(|s| s.error = Some(e));
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
        if let Event::JobFinished(job) = event {
            finished(job);
        }
    }
}

export!(Plugin);
