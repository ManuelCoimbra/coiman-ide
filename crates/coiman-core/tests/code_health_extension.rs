//! The Code Health extension (`plugins/extensions/code-health`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: the checks of a small project read in
//! steps, its tabs, complexity from the editor's parser, the overview page, and programs it may
//! only run once approved.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.code-health";

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

/// The panel's text, when it shows `until`.
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

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Bullets(items) => out.push_str(&format!("{}\n", items.join("\n"))),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Links(links) => {
                for l in links {
                    out.push_str(&format!("{} {} -> {}:{}\n", l.text, l.detail, l.path, l.line));
                }
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn code_health_extension() {
    let ws = std::env::temp_dir().join(format!("coiman-healthext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&ws);
    let w = |p: &str, t: &str| {
        let path = ws.join(p);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, t).unwrap();
    };
    // Put together at run time: a key-shaped literal here would trip push protection.
    let stripe = ["sk", "live", "abcdefghijklmnopqrstuvwx"].join("_");
    w(".env", &format!("STRIPE={stripe}\n"));
    w("README.md", "# Demo\n\nA small project.\n");
    w("Cargo.toml", "[package]\nname = \"demo\"\n[dependencies]\nlog = \"0.4\"\n");
    let busy = "pub fn busy(x: i32) -> i32 {\n    if x > 1 { 1 } else if x > 2 { 2 } else { 3 }\n}\n";
    let block = "    let a = 1;\n    let b = a + 2;\n    let c = b * 3;\n    let d = c - 4;\n    let e = d / 5;\n    let f = e % 6;\n    let g = f + 7;\n";
    w("src/lib.rs", &format!("/// Said.\npub fn said() {{}}\n\n{busy}\nfn one() {{\n{block}}}\n"));
    w("src/other.rs", &format!("fn two() {{\n{block}}}\n"));
    w(
        "Cargo.lock",
        "version = 3\n\n[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\ndependencies = [\"serde\"]\n\n\
         [[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+x\"\n",
    );
    // Code an attacker could use, one of each kind; and a note left for later.
    w(
        "app.py",
        "cur.execute(f\"SELECT * FROM users WHERE id = {uid}\")\ncur.execute(\"SELECT 1 FROM t WHERE id = ?\", (uid,))\n",
    );
    w("web/page.js", "el.innerHTML = userText;\nel.textContent = userText;\n// FIXME: escape it first\n");
    w("native/copy.c", "#include <string.h>\nvoid copy(char *d, const char *s) { strcpy(d, s); }\n");
    w("src/raw.rs", "fn raw(p: *const u8) -> u8 {\n    unsafe { *p }\n}\n");
    w("node_modules/left-pad/package.json", r#"{"name":"left-pad","version":"1.3.0","license":"WTFPL"}"#);

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/code-health");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    // Reads files; no program approved.
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load(pkg.manifest, pkg.wasm, granted);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    host.run_command(ID, "check", "");
    let secrets = panel(&host, "1 secret that look");
    assert!(secrets.contains(".env:1 Stripe secret key") && secrets.contains("-> .env:1"), "{secrets}");
    assert!(secrets.contains("[▸ Secrets  1 tab secrets]"), "{secrets}");

    host.run_command(ID, "tab", "complexity");
    let complexity = panel(&host, "Cyclomatic");
    assert!(complexity.contains("3 | 3 | busy | src/lib.rs:4 [Open open src/lib.rs:4]"), "{complexity}");

    host.run_command(ID, "tab", "duplicates");
    let dups = panel(&host, "written more than once");
    assert!(dups.contains("src/lib.rs:") && dups.contains("src/other.rs:"), "{dups}");

    host.run_command(ID, "tab", "undocumented");
    let docs = panel(&host, "doc comment");
    assert!(docs.contains("busy") && !docs.contains("function said"), "{docs}");

    host.run_command(ID, "tab", "dependencies");
    let deps = panel(&host, "Cargo.lock: 2 packages");
    assert!(deps.contains("[Check for updates updates ]"), "{deps}");
    // log is named but nothing uses it.
    assert!(deps.contains("Not used by the code: 1"), "{deps}");
    assert!(deps.contains("Cargo.toml | dependencies | log | no `use log`"), "{deps}");
    // Comparing with another revision reads the lockfile through git, not approved here.
    let values = vec![("since".to_owned(), "HEAD~1".to_owned())];
    host.deliver(
        ID,
        Event::FormSubmitted(coiman_core::extensions::plugin::FormValues {
            form: "since".into(),
            button: "Compare".into(),
            values,
        }),
    );
    let since = panel(&host, "Cargo.lock: git: program not approved: git");
    assert!(since.contains("Not used by the code: 1"), "{since}");

    host.run_command(ID, "tab", "licences");
    let lic = panel(&host, "left-pad 1.3.0");
    // cargo isn't approved: said, and npm's are still listed.
    assert!(lic.contains("program not approved: cargo"), "{lic}");

    host.run_command(ID, "tab", "security");
    let sec = panel(&host, "Rust unsafe code");
    assert!(sec.contains("[▸ Security  4 tab security]"), "{sec}");
    assert!(sec.contains("app.py:1  cur.execute(f\"SELECT * FROM users WHERE id = {uid}\") a value put into SQL text with {…} -> app.py:1"), "{sec}");
    assert!(!sec.contains("app.py:2"), "a parameter is fine: {sec}");
    assert!(sec.contains("web/page.js:1  el.innerHTML = userText; innerHTML set from a value"), "{sec}");
    assert!(sec.contains("native/copy.c:2") && sec.contains("strcpy doesn't know how big its target is"), "{sec}");
    assert!(sec.contains("src/raw.rs:2  unsafe { *p } unsafe block with no SAFETY note"), "{sec}");
    // Grouped, with what to do about each.
    assert!(sec.find("SQL built from strings").unwrap() < sec.find("HTML made from data").unwrap(), "{sec}");
    assert!(sec.contains("Pass it as a parameter"), "{sec}");

    host.run_command(ID, "tab", "debt");
    let debt = panel(&host, "The notes, oldest first");
    assert!(debt.contains("1 note in comments: 1 FIXME."), "{debt}");
    assert!(debt.contains("web | 1 | 1 FIXME"), "{debt}");
    assert!(debt.contains("web/page.js:3  FIXME escape it first web -> web/page.js:3"), "{debt}");
    // git isn't approved: no ages, and it says why.
    assert!(debt.contains("Their ages come from git blame: program not approved: git"), "{debt}");

    // Updating runs only an approved program.
    host.run_command(ID, "update", "Cargo|serde");
    match wait(&host, |u| matches!(u, Update::Message { text, .. } if text.contains("approved"))) {
        Update::Message { text, .. } => assert!(text.contains("program not approved: cargo"), "{text}"),
        _ => unreachable!(),
    }

    // Without settings, the Gates tab says how to write them.
    host.run_command(ID, "tab", "gates");
    panel(&host, "No settings for this project.");

    // The project's settings: native/ left out, raw.rs's unsafe allowed, two gates.
    w(
        ".coiman/health.toml",
        "ignore = [\"native/**\"]\n\
         [[allow]]\ncheck = \"security\"\npath = \"src/raw.rs\"\nrule = \"unsafe\"\nwhy = \"reads a pointer it owns\"\n\
         [gates]\nsecrets = 0\nsecurity = 2\n",
    );
    host.run_command(ID, "check", "");
    let gates = panel(&host, "gates fail");
    // The .env secret fails one gate; security is down to app.py and page.js: 2, which passes.
    assert!(gates.contains("1 of 2 gates fail"), "{gates}");
    assert!(gates.contains("fail | secrets | 1 (at most 0) | secrets"), "{gates}");
    assert!(gates.contains("pass | security | 2 (at most 2)"), "{gates}");
    assert!(gates.contains("Allowed on purpose (1)"), "{gates}");
    assert!(gates.contains("security src/raw.rs:2  unsafe block") && gates.contains("reads a pointer it owns"), "{gates}");
    assert!(gates.contains("[▸ Gates  1 tab gates]"), "{gates}");
    host.run_command(ID, "tab", "security");
    let sec = panel(&host, "[▸ Security  2 tab security]");
    assert!(!sec.contains("native/copy.c") && !sec.contains("src/raw.rs"), "{sec}");
    // A mistake in the file is said, not ignored.
    w(".coiman/health.toml", "[gates]\nspeed = 1\n");
    host.run_command(ID, "check", "");
    host.run_command(ID, "tab", "gates");
    panel(&host, "speed isn't one of");
    std::fs::remove_file(ws.join(".coiman/health.toml")).unwrap();

    host.run_command(ID, "overview", "");
    match wait(&host, |u| matches!(u, Update::Markdown { .. })) {
        Update::Markdown { title, text, .. } => {
            assert!(title.ends_with(": overview"), "{title}");
            assert!(text.contains("**Demo**: A small project."), "{text}");
            assert!(text.contains("| Rust | 3 |") && text.contains("Cargo (`Cargo.toml`)"), "{text}");
            assert!(!text.contains("## Git"), "git isn't approved: {text}");
        }
        _ => unreachable!(),
    }
    let _ = std::fs::remove_dir_all(&ws);
}
