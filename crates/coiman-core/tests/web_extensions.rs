//! The Web pack's extensions (`plugins/extensions/http-client` and `openapi`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: a `.http` file's lenses, requests sent
//! through the real curl to a server made here (an environment, `.env`, a process variable and a
//! named response filled in), history and comparison; a spec's lens, endpoints and models, an
//! endpoint written to a `.http` file.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{
    Block, Document, Event, EventKind, FormValues, Permission, PluginHost, Update, registry,
};

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
                }
                if want(&u) {
                    return Some(u);
                }
            }
            None
        });
        if let Some(u) = found {
            return u;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

fn panel(host: &PluginHost, until: &str) -> String {
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            if last.contains(until) {
                return last;
            }
        }
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn message(host: &PluginHost, part: &str) -> String {
    match wait(host, |u| matches!(u, Update::Message { text, .. } if text.contains(part))) {
        Update::Message { text, .. } => text,
        _ => unreachable!(),
    }
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
            Block::Diff(t) => out.push_str(&format!("diff:\n{t}\n")),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Form(f) => out.push_str(&format!("form {}\n", f.id)),
            _ => {}
        }
    }
    out
}

fn load(host: &PluginHost, name: &str, granted: &[Permission], programs: &[&str]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions").join(name);
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let programs = programs.iter().map(|p| (*p).to_owned()).collect();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted.iter().copied().collect::<BTreeSet<_>>(), programs);
    wait(host, |u| matches!(u, Update::Activated { .. }));
}

fn document(path: &str, text: &str, line: u32) -> Document {
    Document { path: path.into(), language: String::new(), text: text.into(), line, column: 1, dirty: false }
}

/// A server for `n` requests: each answered by what it asked, as JSON; what came is sent back
/// on the channel.
fn server(n: usize) -> (u16, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..n {
            let Ok((mut s, _)) = listener.accept() else { return };
            let mut got = Vec::new();
            let mut buf = vec![0u8; 8192];
            loop {
                let k = s.read(&mut buf).unwrap_or(0);
                got.extend_from_slice(&buf[..k]);
                let text = String::from_utf8_lossy(&got).into_owned();
                if let Some(at) = text.find("\r\n\r\n") {
                    let length = text[..at]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().to_owned())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if got.len() >= at + 4 + length {
                        break;
                    }
                }
                if k == 0 {
                    break;
                }
            }
            let asked = String::from_utf8_lossy(&got).into_owned();
            let (status, body) = if asked.starts_with("POST /login") {
                ("201 Created", r#"{"token": "t1", "n": 1}"#.to_owned())
            } else {
                let auth = asked.lines().find(|l| l.starts_with("Authorization:")).unwrap_or("").to_owned();
                ("200 OK", format!(r#"{{"auth": "{auth}", "n": 2}}"#))
            };
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body.as_bytes());
            let _ = tx.send(asked);
        }
    });
    (port, rx)
}

#[test]
fn web_extensions() {
    let tmp = std::env::temp_dir().join(format!("coiman-web-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("project");
    std::fs::create_dir_all(ws.join("api")).unwrap();
    // The plugins' storage goes in a profile of the test's own.
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        std::env::set_var("APPDATA", tmp.join("appdata"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("appdata"));
        std::env::set_var("COIMAN_WEB_TEST", "from-the-process");
    }
    let (port, asked) = server(2);
    let http = format!(
        "@host = 127.0.0.1:{port}\n\n### Log in\n# @name login\nPOST http://{{{{host}}}}/login\nContent-Type: application/json\n\n{{\"user\": \"{{{{user}}}}\", \"key\": \"{{{{$dotenv KEY}}}}\"}}\n\n### Orders\nGET http://{{{{host}}}}/orders\nAuthorization: Bearer {{{{login.response.body.$.token}}}}\nX-Home: {{{{$processEnv COIMAN_WEB_TEST}}}}\n\n### Broken\nGET http://{{{{nope}}}}/x\n"
    );
    std::fs::write(ws.join("api/demo.http"), &http).unwrap();
    std::fs::write(ws.join("http-client.env.json"), r#"{"dev": {"user": "ann"}, "prod": {"user": "bob"}}"#).unwrap();
    std::fs::write(ws.join("api/.env"), "KEY=k1\n").unwrap();

    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    use Permission::*;
    load(&host, "http-client", &[WorkspaceRead, EditorRead, Environment], &["curl"]);
    const HTTP: &str = "coiman.http-client";

    // Lenses above each request.
    host.set_document(Some(document("api/demo.http", &http, 1)));
    host.event(EventKind::Opened, Event::Opened("api/demo.http".into()));
    match wait(&host, |u| matches!(u, Update::CodeLenses { lenses, .. } if !lenses.is_empty())) {
        Update::CodeLenses { path, lenses, .. } => {
            assert_eq!(path, "api/demo.http");
            let at: Vec<(u32, &str, &str)> =
                lenses.iter().map(|l| (l.line, l.title.as_str(), l.argument.as_str())).collect();
            assert_eq!(&at[..2], [(5, "▶ Send", "4"), (5, "Copy as curl", "4")]);
            assert_eq!((at[2].0, at[4].0), (11, 16));
        }
        _ => unreachable!(),
    }

    // The environment chosen, then the login sent: its body filled from it and from .env.
    host.run_command(HTTP, "show", "");
    let shown = panel(&host, "[▸ No environment env ]");
    assert!(shown.contains("[dev env dev]") && shown.contains("[prod env prod]"), "{shown}");
    host.run_command(HTTP, "env", "dev");
    panel(&host, "[▸ dev env dev]");
    host.run_command(HTTP, "send", "4");
    message(&host, "201 Created in");
    let login = asked.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(login.starts_with("POST /login HTTP/1.1"), "{login}");
    assert!(login.ends_with(r#"{"user": "ann", "key": "k1"}"#), "{login}");
    let body = panel(&host, "\"token\": \"t1\"");
    assert!(body.contains("201 Created") && body.contains("POST http://127.0.0.1:"), "{body}");

    // The next uses the named response and a variable of the editor's process.
    host.run_command(HTTP, "send", "10");
    message(&host, "200 OK in");
    let orders = asked.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(orders.contains("Authorization: Bearer t1") && orders.contains("X-Home: from-the-process"), "{orders}");

    // A value nobody defined is said plainly; nothing is sent.
    host.run_command(HTTP, "send", "15");
    panel(&host, "{{nope}} isn't defined");

    // History, and two responses compared.
    host.run_command(HTTP, "tab", "history");
    let history = panel(&host, "History  2");
    assert!(history.contains("| 200 | GET http://127.0.0.1:") && history.contains("| 201 | POST "), "{history}");
    host.run_command(HTTP, "pick-a", "0");
    host.run_command(HTTP, "pick-b", "1");
    host.run_command(HTTP, "tab", "compare");
    let diff = panel(&host, "diff:");
    assert!(diff.contains("-200 OK\n+201 Created") && diff.contains("+  \"token\": \"t1\""), "{diff}");
    let kept = std::fs::read_to_string(tmp.join("appdata/coiman-ide/plugins/data/coiman.http-client.txt")).unwrap();
    assert!(kept.contains("api/demo.http") && kept.contains("t1"));

    host.run_command(HTTP, "curl", "4");
    match wait(&host, |u| matches!(u, Update::CopyText(_))) {
        Update::CopyText(t) => {
            assert!(t.starts_with("curl -X POST 'http://127.0.0.1:") && t.contains("\"user\": \"ann\""), "{t}")
        }
        _ => unreachable!(),
    }
    host.run_command(HTTP, "open-body", "");
    wait(&host, |u| matches!(u, Update::OpenText(_)));

    // OpenAPI: the spec's lens, its endpoints, one written to a .http file beside it.
    load(&host, "openapi", &[WorkspaceRead, WorkspaceWrite, EditorRead], &[]);
    const API: &str = "coiman.openapi";
    let spec = "openapi: 3.0.3\ninfo:\n  title: Pets\n  version: \"1.2\"\nservers:\n  - url: https://pets.example.com/v1\npaths:\n  /pets/{petId}:\n    get:\n      summary: Get a pet\n      operationId: getPet\n      tags: [pets]\n      parameters:\n        - { name: petId, in: path, required: true, schema: { type: integer } }\n      responses:\n        '200': { description: The pet }\n  /pets:\n    post:\n      summary: Add a pet\n      operationId: addPet\n      tags: [pets]\n      requestBody:\n        content:\n          application/json:\n            schema: { $ref: '#/components/schemas/Pet' }\n      responses:\n        '201': { description: Added }\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      required: [name]\n      properties:\n        name: { type: string, example: Rex }\n";
    std::fs::write(ws.join("api/pets.yaml"), spec).unwrap();
    host.set_document(Some(document("api/pets.yaml", spec, 1)));
    host.event(EventKind::ActiveChanged, Event::ActiveChanged("api/pets.yaml".into()));
    match wait(&host, |u| matches!(u, Update::CodeLenses { plugin, .. } if plugin == API)) {
        Update::CodeLenses { lenses, .. } => {
            assert_eq!((lenses[0].line, lenses[0].title.as_str()), (1, "▸ 2 endpoints, 1 models: show them"));
        }
        _ => unreachable!(),
    }
    host.run_command(API, "show", "");
    let list = panel(&host, "Endpoints  2");
    assert!(list.contains("Pets 1.2   OpenAPI 3.0.3   https://pets.example.com/v1"), "{list}");
    assert!(list.contains("GET    | /pets/{petId} | Get a pet [Details select 0]"), "{list}");
    host.deliver(
        API,
        Event::FormSubmitted(FormValues {
            form: "filter".into(),
            button: "Filter".into(),
            values: vec![("filter".into(), "add".into())],
        }),
    );
    let filtered = panel(&host, "Add a pet");
    assert!(!filtered.contains("Get a pet"), "{filtered}");
    host.run_command(API, "select", "1");
    let details = panel(&host, "[Open as request open-request 1]");
    assert!(details.contains("Body: Pet (application/json)") && details.contains("\"name\": \"Rex\""), "{details}");
    host.run_command(API, "open-request", "1");
    match wait(&host, |u| matches!(u, Update::Open { .. })) {
        Update::Open { path, line } => {
            assert!(path.ends_with("pets.http"), "{}", path.display());
            let written = std::fs::read_to_string(&path).unwrap();
            assert!(written.starts_with("@baseUrl = https://pets.example.com/v1\n"), "{written}");
            let at = written.lines().nth(line as usize - 1).unwrap();
            assert!(at.starts_with("POST {{baseUrl}}/pets"), "{at} in\n{written}");
        }
        _ => unreachable!(),
    }
    // Asked again: shown, not added twice.
    host.run_command(API, "open-request", "1");
    wait(&host, |u| matches!(u, Update::Open { .. }));
    let written = std::fs::read_to_string(ws.join("api/pets.http")).unwrap();
    assert_eq!(written.matches("POST {{baseUrl}}/pets").count(), 1, "{written}");
    host.run_command(API, "whole-file", "");
    message(&host, "is there already");
    host.deliver(
        API,
        Event::FormSubmitted(FormValues {
            form: "filter".into(),
            button: "Filter".into(),
            values: vec![("filter".into(), String::new())],
        }),
    );
    host.run_command(API, "tab", "models");
    host.run_command(API, "model", "Pet");
    let models = panel(&host, "[Hide model Pet]");
    assert!(models.contains("Pet | 1 fields") && models.contains("name * | string"), "{models}");

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
