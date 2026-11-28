//! The API Docs extension (`plugins/extensions/apidocs`) in the real plugin host, on a small
//! project with Rust, Python, Java, TypeScript and C files: the dependency page (Cargo.lock with
//! licences from a stand-in `cargo metadata`, package.json with node_modules, requirements.txt
//! with a virtual environment), saved as docs/DEPENDENCIES.md; the page from doc comments; and
//! the real `cargo doc`, pydoc and javadoc (the last two when installed).

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.apidocs";

thread_local! {
    static BACKLOG: std::cell::RefCell<VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, what: &str, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    let mut last_panel = String::new();
    while start.elapsed() < Duration::from_secs(180) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
                }
                if let Update::Panel { blocks, .. } = &u {
                    last_panel = text(blocks);
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
    panic!("timed out waiting for {what}; the panel last said:\n{last_panel}");
}

fn wait_panel(host: &PluginHost, until: &str) -> String {
    match wait(host, until, |u| matches!(u, Update::Panel { blocks, .. } if text(blocks).contains(until))) {
        Update::Panel { blocks, .. } => text(&blocks),
        _ => unreachable!(),
    }
}

fn wait_page(host: &PluginHost, title: &str) -> String {
    match wait(host, title, |u| matches!(u, Update::Markdown { title: t, .. } if t == title)) {
        Update::Markdown { text, .. } => text,
        _ => unreachable!(),
    }
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
            Block::Rows(rows) => {
                for r in rows {
                    let buttons: Vec<&str> = r.buttons.iter().map(|b| b.label.as_str()).collect();
                    out.push_str(&format!("[{}]\n", buttons.join("] [")));
                }
            }
            Block::Log(log) => {
                for l in &log.lines {
                    out.push_str(&format!("log: {}\n", l.text));
                }
            }
            _ => {}
        }
    }
    out
}

/// The front page's path the panel shows after a run.
fn out_path(panel: &str) -> PathBuf {
    let line = panel.lines().find(|l| l.ends_with("index.html") && !l.starts_with("Done")).expect(panel);
    PathBuf::from(line.trim())
}

fn installed(program: &str, arg: &str) -> bool {
    Command::new(program).arg(arg).output().is_ok_and(|o| o.status.success())
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

const LOCK: &str = r#"version = 4

[[package]]
name = "demo"
version = "0.1.0"
dependencies = [
 "itoa",
]

[[package]]
name = "itoa"
version = "1.0.19"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = [
 "memchr",
]

[[package]]
name = "memchr"
version = "2.8.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

#[test]
fn apidocs_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-apidocs-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    let ws = tmp.join("project");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&ws).unwrap();
    let real_cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let target = ws.join("target").to_string_lossy().replace('\\', "/");
    // `cargo metadata` answers from a file (no crates need be downloaded); the rest is real cargo.
    write(
        &bin.join("metadata.json"),
        &format!(
            "{{\"packages\":[{{\"name\":\"itoa\",\"version\":\"1.0.19\",\"license\":\"MIT OR Apache-2.0\",\"license_file\":null}},\
             {{\"name\":\"memchr\",\"version\":\"2.8.3\",\"license\":\"Unlicense OR MIT\",\"license_file\":null}}],\
             \"workspace_members\":[],\"target_directory\":\"{target}\"}}"
        ),
    );
    if cfg!(windows) {
        write(
            &bin.join("cargo.cmd"),
            &format!(
                "@echo off\r\nif \"%1\"==\"metadata\" (\r\n  type \"%~dp0metadata.json\"\r\n  exit /b 0\r\n)\r\n\"{real_cargo}\" %*\r\n"
            ),
        );
    } else {
        write(
            &bin.join("cargo"),
            &format!(
                "#!/bin/sh\nif [ \"$1\" = metadata ]; then cat \"$(dirname \"$0\")/metadata.json\"; exit 0; fi\nexec \"{real_cargo}\" \"$@\"\n"
            ),
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(bin.join("cargo"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("config"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
        // cargo doc writes into the project's own target folder.
        std::env::remove_var("CARGO_TARGET_DIR");
    }

    // The project.
    write(&ws.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n");
    write(&ws.join("Cargo.lock"), LOCK);
    write(
        &ws.join("src/lib.rs"),
        "//! The demo crate.\n\n/// Adds two numbers.\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\n/// A point.\npub struct Point {\n    pub x: i32,\n}\n",
    );
    write(&ws.join("tests/it.rs"), "/// Not part of the API.\nfn helper() {}\n");
    write(
        &ws.join("tool.py"),
        "\"\"\"Tools for the demo.\"\"\"\n\n\ndef greet(name):\n    \"\"\"Says hello to name.\"\"\"\n    return 'hello ' + name\n",
    );
    write(
        &ws.join("src/main/java/demo/Calc.java"),
        "package demo;\n\n/** A calculator. */\npublic class Calc {\n    /**\n     * Multiplies.\n     * @param a the first\n     * @return the product\n     */\n    public int times(int a, int b) {\n        return a * b;\n    }\n}\n",
    );
    write(&ws.join("web/app.ts"), "/** Shows the page. */\nexport function show(id: string): void {\n}\n");
    write(&ws.join("native/io.h"), "#pragma once\n/// Opens a file.\nint open_file(const char *path);\n");
    write(
        &ws.join("package.json"),
        r#"{"name":"web","dependencies":{"left-pad":"^1.3.0"},"devDependencies":{"jest":"^29"}}"#,
    );
    write(&ws.join("node_modules/left-pad/package.json"), r#"{"name":"left-pad","version":"1.3.0","license":"WTFPL"}"#);
    write(&ws.join("node_modules/left-pad/README.md"), "# left-pad\n");
    write(&ws.join("requirements.txt"), "requests==2.31.0\nnumpy>=1.26\n");
    write(
        &ws.join(".venv/Lib/site-packages/requests-2.31.0.dist-info/METADATA"),
        "Metadata-Version: 2.1\nName: requests\nVersion: 2.31.0\nLicense: Apache 2.0\n\nRequests is a library.\n",
    );
    std::fs::create_dir_all(ws.join("target/doc/itoa")).unwrap();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/apidocs");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::WorkspaceWrite].into();
    let programs = ["cargo", "python", "javadoc"].map(str::to_owned).into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, programs);
    host.run_command(ID, "show", "");
    wait_panel(&host, "Nothing is fetched from the network");

    // Dependencies: three ecosystems, with versions, licences and local docs.
    host.run_command(ID, "deps", "");
    let deps = wait_page(&host, "Dependencies of project");
    assert!(
        deps.contains("## Rust (Cargo.lock)\n\n2 packages; licences: MIT OR Apache-2.0, Unlicense OR MIT."),
        "{deps}"
    );
    assert!(
        deps.contains(
            "| itoa | 1.0.19 | MIT OR Apache-2.0 | direct | [target/doc/itoa/index.html](/target/doc/itoa/index.html) |"
        ),
        "{deps}"
    );
    assert!(deps.contains("| memchr | 2.8.3 | Unlicense OR MIT | indirect | — |"), "{deps}");
    assert!(
        deps.contains("| left-pad | 1.3.0 | WTFPL | direct | [node_modules/left-pad/README.md](/node_modules/left-pad/README.md) |"),
        "{deps}"
    );
    assert!(deps.contains("| jest | ^29 (wanted) | — | dev | — |"), "{deps}");
    assert!(deps.contains("## Python (requirements.txt)"), "{deps}");
    assert!(deps.contains("| requests | 2.31.0 | Apache 2.0 | direct | — |"), "{deps}");
    assert!(deps.contains("| numpy | >=1.26 (wanted) | — | direct | — |"), "{deps}");
    assert!(
        deps.contains("1 npm package is not installed") && deps.contains("1 Python package is not installed"),
        "{deps}"
    );
    let panel = wait_panel(&host, "dependencies listed, opened as a page");
    assert!(panel.contains("[Show the page again] [Save as docs/DEPENDENCIES.md]"), "{panel}");

    // Saved, and saving again asks before replacing.
    host.run_command(ID, "save", "");
    wait_panel(&host, "Saved as docs/DEPENDENCIES.md");
    assert_eq!(std::fs::read_to_string(ws.join("docs/DEPENDENCIES.md")).unwrap(), deps);
    std::fs::write(ws.join("docs/DEPENDENCIES.md"), "old").unwrap();
    host.run_command(ID, "save", "");
    wait_panel(&host, "docs/DEPENDENCIES.md is there already. Replace it?");
    assert_eq!(std::fs::read_to_string(ws.join("docs/DEPENDENCIES.md")).unwrap(), "old");
    host.run_command(ID, "replace", "");
    wait_panel(&host, "Saved as docs/DEPENDENCIES.md");
    assert_eq!(std::fs::read_to_string(ws.join("docs/DEPENDENCIES.md")).unwrap(), deps);

    // The page from doc comments, in five languages, tests left out.
    host.run_command(ID, "comments", "");
    let api = wait_page(&host, "API of project");
    assert!(api.contains("From the doc comments in 5 files: 7 documented items."), "{api}");
    assert!(api.contains("\n## src/lib.rs\n\nThe demo crate.\n"), "{api}");
    assert!(
        api.contains("\n### add\n\n```rust\npub fn add(a: i32, b: i32) -> i32\n```\n\nAdds two numbers.\n\n[src/lib.rs, line 4](/src/lib.rs#L4)\n"),
        "{api}"
    );
    assert!(api.contains("\n### Point\n"), "{api}");
    assert!(
        api.contains("```python\ndef greet(name)\n```\n\nSays hello to name.\n\n[tool.py, line 4](/tool.py#L4)"),
        "{api}"
    );
    assert!(api.contains("\n### times\n\n```java\npublic int times(int a, int b)\n```\n\nMultiplies.\n\n- `a`: the first\n- Returns: the product"), "{api}");
    assert!(api.contains("\n### Calc\n"), "{api}");
    assert!(api.contains("```ts\nexport function show(id: string): void\n```"), "{api}");
    assert!(api.contains("\n### open_file\n\n```cpp\nint open_file(const char *path)\n```\n\nOpens a file."), "{api}");
    assert!(!api.contains("Not part of the API"), "{api}");
    let panel = wait_panel(&host, "opened as a page");
    assert!(panel.contains("[Save as docs/API.md]"), "{panel}");
    host.run_command(ID, "save", "");
    wait_panel(&host, "Saved as docs/API.md");
    assert_eq!(std::fs::read_to_string(ws.join("docs/API.md")).unwrap(), api);

    // The real cargo doc, private items too: the editor won't open file:// in the browser, so
    // the path is shown to copy.
    host.run_command(ID, "generate", "rust");
    wait_panel(&host, "Running cargo doc --no-deps");
    let done = wait_panel(&host, "Done");
    assert!(done.contains("copy the path below"), "{done}");
    let index = out_path(&done);
    assert!(index.ends_with("target/doc/demo/index.html") || index.ends_with(r"target\doc\demo\index.html"), "{done}");
    assert!(index.is_file(), "{done}");
    host.run_command(ID, "copy", "");
    match wait(&host, "the path copied", |u| matches!(u, Update::CopyText(_))) {
        Update::CopyText(t) => assert_eq!(PathBuf::from(t), index),
        _ => unreachable!(),
    }
    // A failure shows the tool's output.
    write(&ws.join("src/lib.rs"), "pub fn broken( {\n");
    host.run_command(ID, "generate", "rust");
    let failed = wait_panel(&host, "Rust: cargo doc failed (exit code");
    assert!(failed.contains("log: $ cargo doc --no-deps"), "{failed}");

    if installed("python", "--version") {
        host.run_command(ID, "generate", "python");
        let done = wait_panel(&host, "Done");
        let index = out_path(&done);
        let html = std::fs::read_to_string(&index).unwrap();
        assert!(html.contains("<a href=\"tool.html\">tool</a>"), "{html}");
        // pydoc writes the docstring's spaces as &nbsp;.
        let page = std::fs::read_to_string(index.with_file_name("tool.html")).unwrap().replace("&nbsp;", " ");
        assert!(page.contains("Says hello to name."), "pydoc's page lacks the docstring");
    } else {
        eprintln!("python isn't installed: pydoc not checked");
    }
    if installed("javadoc", "--version") {
        host.run_command(ID, "generate", "java");
        let done = wait_panel(&host, "Done");
        let index = out_path(&done);
        assert!(index.is_file(), "{done}");
        let calc = index.with_file_name("demo").join("Calc.html");
        assert!(std::fs::read_to_string(&calc).unwrap().contains("Multiplies."), "javadoc's page lacks the comment");
    } else {
        eprintln!("javadoc isn't installed: javadoc not checked");
    }
    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
