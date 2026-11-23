//! The Remote over SSH extension (`plugins/extensions/remote`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, against a stand-in `ssh` whose remote
//! folder is a folder here (its tar stream made by the real `tar`), and the real `tar` unpacking
//! it: the mirror made with no folder open and opened, saves, deletes and renames sent back, a
//! download again, terminals there, the hosts of an ssh config, and ssh refusing.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{
    Block, Event, EventKind, FormValues, Permission, PickAction, PickChoice, PluginHost, Update, registry,
};

const ID: &str = "coiman.remote";

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

fn message(host: &PluginHost, part: &str) {
    wait(host, |u| matches!(u, Update::Message { text, .. } if text.contains(part)));
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{}\n", buttons.join(" ")));
                }
            }
            Block::Form(f) => {
                let fields: Vec<String> = f.fields.iter().map(|f| format!("{}={}", f.id, f.value)).collect();
                out.push_str(&format!("form {} ({})\n", f.id, fields.join(", ")));
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

/// An `ssh` that keeps each call in `calls.log` and, for the command it's given: `tar … -cf -`
/// prints `there` as a tar stream; `cat > …` keeps what comes in in `got.txt`; the rest does
/// nothing. With `COIMAN_FAKE_SSH_DOWN` set to 1 it refuses, as ssh does.
fn fake_ssh(dir: &Path) {
    std::fs::write(
        dir.join("ssh.cmd"),
        "@echo off\r\n>>\"%~dp0calls.log\" echo %*\r\n\
         if \"%COIMAN_FAKE_SSH_DOWN%\"==\"1\" (\r\n  >&2 echo pi@board: Permission denied ^(publickey^).\r\n  exit /b 255\r\n)\r\n\
         echo %* | findstr /c:\"-cf -\" >nul && (tar -C \"%~dp0there\" -cf - . & exit /b 0)\r\n\
         echo %* | findstr /c:\"cat >\" >nul && (findstr \"^\" > \"%~dp0got.txt\" & exit /b 0)\r\n\
         exit /b 0\r\n",
    )
    .unwrap();
}

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

#[test]
fn remote_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-remote-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    let there = bin.join("there");
    std::fs::create_dir_all(there.join("src")).unwrap();
    std::fs::write(there.join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(there.join("old.txt"), "old\n").unwrap();
    fake_ssh(&bin);
    let home = tmp.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/config"), "Host pi\n  HostName 10.0.0.5\nHost *.lan\nHost build\n").unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("appdata"));
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/remote");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(None);
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::Terminal].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["ssh".to_owned(), "tar".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The form, with the ssh config's hosts; opening a folder with no folder open.
    host.run_command(ID, "open", "");
    let form = panel(&host, "Hosts in ~/.ssh/config:");
    assert!(form.contains("[New SSH terminal… ssh-terminal ] [pi use-host pi] [build use-host build]"), "{form}");
    submit(&host, "open", "Open", &[("host", ""), ("path", "/x"), ("options", "")]);
    panel(&host, "Name the ssh host (user@host)");
    submit(&host, "open", "Open", &[("host", "pi@board"), ("path", "/home/pi/app"), ("options", "-p 2222")]);
    let mirror = match wait(&host, |u| {
        if let Update::Panel { blocks, .. } = u {
            let t = text(blocks);
            assert!(!t.contains("Couldn't"), "{t}");
        }
        matches!(u, Update::OpenFolder(_))
    }) {
        Update::OpenFolder(p) => p,
        _ => unreachable!(),
    };
    message(&host, "pi@board:/home/pi/app is here");
    assert!(
        mirror.ends_with("mirrors/pi_board_app") || mirror.ends_with("mirrors\\pi_board_app"),
        "{}",
        mirror.display()
    );
    assert!(mirror.starts_with(tmp.join("appdata")), "in the extension's own folder: {}", mirror.display());
    assert_eq!(std::fs::read_to_string(mirror.join("src/a.rs")).unwrap(), "fn a() {}\n");
    let marker = std::fs::read_to_string(mirror.join(".coiman-remote.toml")).unwrap();
    assert!(marker.contains("host = \"pi@board\"") && marker.contains("ssh_options = \"-p 2222\""), "{marker}");
    assert!(calls(&bin).contains("BatchMode=yes\" -p 2222 pi@board \"tar -C '/home/pi/app' --exclude='./target'"));
    let editor_reads = coiman_core::project::remote_folder::RemoteFolder::of_mirror(&mirror).unwrap();
    assert_eq!(editor_reads.path, "/home/pi/app");

    // The mirror open: saves, deletes and renames go there; skipped folders don't.
    host.set_workspace(Some(mirror.clone()));
    std::fs::write(mirror.join("src/a.rs"), "fn a() { b() }\n").unwrap();
    host.event(EventKind::Saved, Event::Saved("src/a.rs".into()));
    message(&host, "Sent src/a.rs");
    assert!(calls(&bin).contains("mkdir -p '/home/pi/app/src' && cat > '/home/pi/app/src/a.rs'"));
    assert_eq!(std::fs::read_to_string(bin.join("got.txt")).unwrap().trim(), "fn a() { b() }");
    host.event(EventKind::Saved, Event::Saved("target/debug/x".into()));
    host.event(EventKind::Deleted, Event::Deleted(vec!["old.txt".into(), "node_modules/y".into()]));
    message(&host, "Deleted old.txt there");
    host.event(EventKind::Renamed, Event::Renamed(("src/a.rs".into(), "src/b.rs".into())));
    message(&host, "Renamed src/a.rs to src/b.rs there");
    let log = calls(&bin);
    assert!(log.contains("rm -rf -- '/home/pi/app/old.txt'\"") && !log.contains("target/debug"), "{log}");
    assert!(log.contains("mv -- '/home/pi/app/src/a.rs' '/home/pi/app/src/b.rs'"), "{log}");
    let lately = panel(&host, "Lately");
    assert!(lately.contains("This folder is a copy of pi@board:/home/pi/app"), "{lately}");

    // Terminals there, and a command run there.
    host.run_command(ID, "terminal", "");
    match wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) {
        Update::RunInTerminal { title, args, folder, .. } => {
            assert_eq!(title, "ssh pi@board");
            assert_eq!(folder, mirror);
            assert_eq!(args.last().unwrap(), "cd '/home/pi/app' && CHERE_INVOKING=1 exec \"${SHELL:-sh}\" -i");
        }
        _ => unreachable!(),
    }
    submit(&host, "run", "Run", &[("command", "cargo build")]);
    match wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) {
        Update::RunInTerminal { title, args, .. } => {
            assert_eq!(title, "pi@board: cargo build");
            assert!(args.last().unwrap().starts_with("cd '/home/pi/app' && cargo build; "));
        }
        _ => unreachable!(),
    }

    // A terminal on a host of the ssh config, picked from a list.
    host.run_command(ID, "ssh-terminal", "");
    match wait(&host, |u| matches!(u, Update::Pick { .. })) {
        Update::Pick { pick, .. } => {
            assert_eq!(pick.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(), ["pi", "build"]);
        }
        _ => unreachable!(),
    }
    host.deliver(ID, Event::Picked(PickChoice { pick: "hosts".into(), text: "pi".into(), action: PickAction::Run }));
    match wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) {
        Update::RunInTerminal { title, args, .. } => {
            assert_eq!((title.as_str(), args), ("ssh pi", vec!["pi".to_owned()]))
        }
        _ => unreachable!(),
    }

    // Again: the same folder stays open, with what came down.
    std::fs::write(there.join("new.txt"), "new\n").unwrap();
    host.run_command(ID, "download", "");
    message(&host, "is here");
    assert!(mirror.join("new.txt").is_file() && mirror.join(".coiman-remote.toml").is_file());
    BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
    assert!(!BACKLOG.with_borrow(|b| b.iter().any(|u| matches!(u, Update::OpenFolder(_)))), "not opened again");

    // ssh refusing: said, and the mirror as it was.
    // SAFETY: as above.
    unsafe { std::env::set_var("COIMAN_FAKE_SSH_DOWN", "1") };
    host.run_command(ID, "download", "");
    let refused = panel(&host, "Couldn't download pi@board:/home/pi/app");
    assert!(refused.contains("Permission denied (publickey)."), "{refused}");
    assert!(mirror.join("new.txt").is_file());

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
