//! The Project Metadata extension (`plugins/extensions/metadata`) in the real plugin host: a
//! folder's Cargo.toml and package.json read into forms, and saved back in place.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.metadata";

fn wait_panel(host: &PluginHost, until: &str) -> String {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(60) {
        for u in host.poll() {
            match u {
                Update::Stopped { error, .. } => panic!("the extension stopped: {error}"),
                Update::Panel { blocks, .. } => last = text(&blocks),
                _ => {}
            }
        }
        if last.contains(until) {
            return last;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(s) => {
                out.push_str(&s.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Form(f) => {
                out.push_str(&format!("form {}:", f.id));
                for x in f.fields.iter().filter(|x| !x.id.is_empty()) {
                    out.push_str(&format!(" {}={:?}", x.id, x.value));
                }
                out.push('\n');
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, form: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: "Save".into(), values }));
}

#[test]
fn metadata_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-metadata-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    std::fs::create_dir_all(&ws).unwrap();
    let cargo = "[package]\nname = \"demo\" # the crate\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = \"1\"\n";
    std::fs::write(ws.join("Cargo.toml"), cargo).unwrap();
    std::fs::write(ws.join("package.json"), "{\n  \"name\": \"demo-web\",\n  \"version\": \"1.0.0\"\n}\n").unwrap();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/metadata");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::WorkspaceWrite].into();
    host.load(pkg.manifest, pkg.wasm, granted);

    host.run_command(ID, "show", "");
    let shown = wait_panel(&host, "package.json");
    assert!(
        shown.contains(
            "form Cargo.toml#0: name=\"demo\" version=\"0.1.0\" description=\"\" license=\"\" repository=\"\""
        ),
        "{shown}"
    );
    assert!(shown.contains("form package.json#0: name=\"demo-web\" version=\"1.0.0\""), "{shown}");

    // A bad version is refused, and the file is left alone.
    submit(&host, "Cargo.toml#0", &[("name", "demo"), ("version", "1.0"), ("description", ""), ("license", "")]);
    let bad = wait_panel(&host, "three numbers");
    assert!(bad.contains("The version is three numbers, like 1.2.3"), "{bad}");
    assert_eq!(std::fs::read_to_string(ws.join("Cargo.toml")).unwrap(), cargo);

    submit(
        &host,
        "Cargo.toml#0",
        &[
            ("name", "demo"),
            ("version", "0.2.0"),
            ("description", "A small demo"),
            ("license", "MIT"),
            ("repository", ""),
        ],
    );
    let saved = wait_panel(&host, "Saved Cargo.toml");
    assert!(
        saved.contains("form Cargo.toml#1: name=\"demo\" version=\"0.2.0\" description=\"A small demo\""),
        "{saved}"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("Cargo.toml")).unwrap(),
        "[package]\nname = \"demo\" # the crate\nversion = \"0.2.0\"\nedition = \"2024\"\ndescription = \"A small demo\"\nlicense = \"MIT\"\n\n[dependencies]\nserde = \"1\"\n"
    );

    submit(&host, "package.json#1", &[("name", "demo-web"), ("version", "1.0.0"), ("license", "MIT")]);
    wait_panel(&host, "Saved package.json");
    assert_eq!(
        std::fs::read_to_string(ws.join("package.json")).unwrap(),
        "{\n  \"name\": \"demo-web\",\n  \"license\": \"MIT\",\n  \"version\": \"1.0.0\"\n}\n"
    );
    let _ = std::fs::remove_dir_all(&ws);
}
