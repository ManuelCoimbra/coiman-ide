//! The plugin runtime against a real component (`fixtures/test-plugin`,
//! rebuilt with `scripts/build-plugins.ps1`): host calls, permission checks,
//! the folder sandbox, time limits and crashes.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rust_code_core::plugin::{Event, EventKind, Permission, PluginHost, Update, manifest, registry};

const ID: &str = "test.host-api";

fn fixture() -> (manifest::Manifest, Vec<u8>) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test-plugin");
    let pkg = registry::read_folder(&dir).expect("fixture plugin");
    (pkg.manifest, pkg.wasm)
}

thread_local! {
    /// Updates polled but not waited for yet.
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

/// Waits for an update matching `want` (earlier ones are skipped), failing
/// after a while.
fn wait(host: &PluginHost, want: impl Fn(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        if let Some(u) = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if want(&u) {
                    return Some(u);
                }
            }
            None
        }) {
            return u;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

fn message(host: &PluginHost, op: &str) -> String {
    host.run_command(ID, "run", op);
    match wait(host, |u| matches!(u, Update::Message { .. })) {
        Update::Message { text, .. } => text,
        _ => unreachable!(),
    }
}

#[test]
fn host_api_permissions_sandbox_and_limits() {
    let root = std::env::temp_dir().join(format!("rust-code-plugin-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("ws/src")).unwrap();
    std::fs::write(root.join("ws/src/a.txt"), "hello").unwrap();
    std::fs::write(root.join("outside.txt"), "secret").unwrap();
    let ws = root.join("ws");

    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let (manifest, wasm) = fixture();
    host.load(manifest.clone(), wasm.clone(), Permission::ALL.into_iter().collect());
    wait(&host, |u| matches!(u, Update::Status { text, .. } if text == "active"));
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // Reading, listing, writing inside the folder.
    assert_eq!(message(&host, "read src/a.txt"), "ok: hello");
    assert_eq!(message(&host, "list .txt"), "ok: src/a.txt");
    assert_eq!(message(&host, "write src/new.txt"), "ok: written");
    assert_eq!(std::fs::read_to_string(ws.join("src/new.txt")).unwrap(), "written by plugin");
    // Never outside it.
    assert!(message(&host, "read ../outside.txt").contains("isn't a path inside"));
    assert!(message(&host, &format!("read {}", root.join("outside.txt").display())).contains("isn't a path inside"));
    assert!(message(&host, "write ../evil.txt").contains("isn't a path inside"));
    assert!(!root.join("evil.txt").exists());

    // The active editor.
    assert_eq!(message(&host, "doc"), "ok: none");
    host.set_document(Some(rust_code_core::plugin::Document {
        path: "src/a.txt".into(),
        language: "Plain Text".into(),
        text: "hello".into(),
        line: 1,
        column: 1,
        dirty: false,
    }));
    assert_eq!(message(&host, "doc"), "ok: src/a.txt:5");
    host.run_command(ID, "run", "insert hi");
    assert!(matches!(wait(&host, |u| matches!(u, Update::Insert { .. })), Update::Insert { text, .. } if text == "hi"));

    // Declarative contributions, checked against plugin.toml.
    host.run_command(ID, "run", "panel Title");
    assert!(
        matches!(wait(&host, |u| matches!(u, Update::Panel { .. })), Update::Panel { blocks, .. } if blocks.len() == 2)
    );
    host.run_command(ID, "run", "undeclared");
    wait(&host, |u| matches!(u, Update::Log { line, .. } if line.contains("isn't declared")));

    // Coloring and problems, in the editor's terms.
    host.run_command(ID, "run", "decorate careful");
    let decorated = wait(&host, |u| matches!(u, Update::Decorations { .. }));
    assert!(matches!(&decorated, Update::Decorations { items, .. } if items[0].foreground == Some(0xFF0000)));
    let problems = wait(&host, |u| matches!(u, Update::Problems { .. }));
    assert!(matches!(&problems, Update::Problems { items, .. } if items[0].message == "careful"));

    // Events it subscribed to.
    host.event(EventKind::Saved, Event::Saved("src/a.txt".into()));
    wait(&host, |u| matches!(u, Update::Log { line, .. } if line == "saved src/a.txt"));

    // Permissions taken away while running.
    host.grant(ID, BTreeSet::from([Permission::EditorRead]));
    assert_eq!(message(&host, "read src/a.txt"), "err: permission denied: workspace-read");
    assert_eq!(message(&host, "write src/x.txt"), "err: permission denied: workspace-write");
    assert_eq!(message(&host, "insert x"), "err: permission denied: editor-write");

    // Errors a plugin returns are shown; it keeps running.
    assert_eq!(message(&host, "fail"), "failed on purpose");
    // An endless loop is stopped by the time limit.
    host.run_command(ID, "run", "spin");
    let stopped = wait(&host, |u| matches!(u, Update::Stopped { .. }));
    assert!(matches!(&stopped, Update::Stopped { error, .. } if error.contains("took longer")), "{stopped:?}");

    // A panic is a crash: the plugin is stopped too.
    host.load(manifest, wasm, BTreeSet::new());
    wait(&host, |u| matches!(u, Update::Activated { .. }));
    host.run_command(ID, "run", "trap");
    let crashed = wait(&host, |u| matches!(u, Update::Stopped { .. }));
    assert!(matches!(&crashed, Update::Stopped { error, .. } if error.contains("crashed")), "{crashed:?}");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rejects_modules_that_are_not_plugins() {
    let host = PluginHost::start(|| {}).unwrap();
    let (manifest, _) = fixture();
    host.load(manifest, b"\0asm\x01\0\0\0".to_vec(), BTreeSet::new());
    assert!(matches!(wait(&host, |u| matches!(u, Update::Stopped { .. })), Update::Stopped { .. }));
}
