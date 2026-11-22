//! The Terminal Commands extension (`plugins/extensions/terminal-commands`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: pick lists, typing into a terminal,
//! its own storage, folder listing, and the `terminal` permission.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{
    Event, EventKind, Permission, PickAction, PickChoice, PickQuery, PluginHost, TerminalLine, Update, registry,
};

const ID: &str = "coiman.terminal-commands";

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait_for(host: &PluginHost, within: Duration, want: impl Fn(&Update) -> bool) -> Option<Update> {
    let start = Instant::now();
    while start.elapsed() < within {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            let i = b.iter().position(&want)?;
            b.remove(i)
        });
        if found.is_some() {
            return found;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

fn wait(host: &PluginHost, want: impl Fn(&Update) -> bool) -> Update {
    wait_for(host, Duration::from_secs(20), want).expect("timed out waiting for an update")
}

/// The rows of the next pick list shown.
fn pick_rows(host: &PluginHost) -> (Vec<String>, bool) {
    match wait(host, |u| matches!(u, Update::Pick { .. })) {
        Update::Pick { pick, .. } => (pick.items.into_iter().map(|i| i.text).collect(), pick.run_first),
        _ => unreachable!(),
    }
}

fn line(terminal: u64, typed: &str, dir: &std::path::Path) -> TerminalLine {
    TerminalLine { terminal, typed: typed.to_owned(), dir: dir.to_string_lossy().into_owned() }
}

#[test]
fn terminal_commands_extension() {
    let root = std::env::temp_dir().join(format!("coiman-termext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(ws.join("package.json"), r#"{"scripts": {"dev": "vite"}}"#).unwrap();
    std::fs::write(ws.join("Makefile"), "test:\n\techo hi\n").unwrap();
    // This test binary keeps its config (and so the extension's storage) to itself.
    let config = root.join("config");
    // SAFETY: the only test in this binary, set before anything reads the environment.
    unsafe {
        std::env::set_var("APPDATA", &config);
        std::env::set_var("HOME", &config);
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/terminal-commands");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let all: BTreeSet<Permission> = [Permission::Terminal, Permission::WorkspaceRead].into();
    host.load(pkg.manifest.clone(), pkg.wasm.clone(), all);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The project's commands, run first.
    host.set_terminal_line(Some(line(7, "np", &ws)));
    host.run_command(ID, "project", "");
    let (rows, run_first) = pick_rows(&host);
    assert!(run_first);
    assert!(rows.contains(&"npm run dev".to_owned()) && rows.contains(&"make test".to_owned()), "{rows:?}");

    // A choice erases what was typed and runs.
    let choice = PickChoice { pick: "commands".into(), text: "npm run dev".into(), action: PickAction::Run };
    host.deliver(ID, Event::Picked(choice));
    match wait(&host, |u| matches!(u, Update::TerminalType { .. })) {
        Update::TerminalType { terminal, text, .. } => {
            assert_eq!(terminal, 7);
            assert_eq!(text, "\u{7f}\u{7f}npm run dev\r");
        }
        _ => unreachable!(),
    }

    // A command run in a shell is kept, in the extension's own storage.
    host.event(EventKind::TerminalCommand, Event::TerminalCommand(line(7, "cargo build", &ws)));
    host.set_terminal_line(Some(line(7, "", &ws)));
    host.run_command(ID, "history", "");
    let (rows, run_first) = pick_rows(&host);
    assert!(!run_first);
    assert_eq!(rows[..2], ["cargo build".to_owned(), "npm run dev".to_owned()], "{rows:?}");
    let stored = config.join("coiman-ide/plugins/data").join(format!("{ID}.txt"));
    assert!(std::fs::read_to_string(&stored).unwrap().contains("cargo build"));

    // Ctrl+Space: paths for the word being typed, and narrowing as the box changes.
    host.event(EventKind::TerminalSuggest, Event::TerminalSuggest(line(7, "ls sr", &ws)));
    let (rows, _) = pick_rows(&host);
    let sep = if cfg!(windows) { '\\' } else { '/' };
    assert_eq!(rows.first(), Some(&format!("ls src{sep}")), "{rows:?}");
    host.deliver(ID, Event::PickQueryChanged(PickQuery { pick: "commands".into(), query: "cargo".into() }));
    let (rows, _) = pick_rows(&host);
    assert_eq!(rows, ["cargo build"]);

    // Forgetting takes it out of the history.
    let forget = PickChoice { pick: "commands".into(), text: "cargo build".into(), action: PickAction::Forget };
    host.deliver(ID, Event::Picked(forget));
    let (rows, _) = pick_rows(&host);
    assert!(rows.is_empty(), "{rows:?}");

    // Without the `terminal` permission it hears nothing typed, and can't read the line.
    let read_only: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load(pkg.manifest, pkg.wasm, read_only);
    wait(&host, |u| matches!(u, Update::Activated { .. }));
    host.event(EventKind::TerminalSuggest, Event::TerminalSuggest(line(7, "ls", &ws)));
    assert!(wait_for(&host, Duration::from_millis(800), |u| matches!(u, Update::Pick { .. })).is_none());
    host.run_command(ID, "history", "");
    match wait(&host, |u| matches!(u, Update::Message { .. })) {
        Update::Message { text, .. } => assert!(text.contains("permission denied: terminal"), "{text}"),
        _ => unreachable!(),
    }
    let _ = std::fs::remove_dir_all(&root);
}
