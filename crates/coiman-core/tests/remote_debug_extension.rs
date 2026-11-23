//! The Remote Debugging extension (`plugins/extensions/remote-debug`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, against stand-ins for `ssh` (it prints
//! what gdbserver prints, then runs on) and `scp`: connecting to a gdbserver running, starting
//! one over ssh after a copy and attaching once it listens, the target kept for the project, the
//! session's end stopping ssh, a restart, and ssh refusing.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.remote-debug";

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

fn message(host: &PluginHost, part: &str) {
    wait(host, |u| matches!(u, Update::Message { text, .. } if text.contains(part)));
}

fn console(host: &PluginHost, part: &str) {
    wait(host, |u| matches!(u, Update::DebugConsole(t) if t.contains(part)));
}

fn attached(host: &PluginHost) -> (String, String, String) {
    match wait(host, |u| matches!(u, Update::DebugAttach { .. })) {
        Update::DebugAttach { attach, .. } => (attach.name, attach.program, attach.target),
        _ => unreachable!(),
    }
}

fn form_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            Block::Form(f) => Some(format!(
                "{} {:?}",
                f.fields
                    .iter()
                    .filter(|f| !f.id.is_empty())
                    .map(|f| format!("{}={}", f.id, f.value))
                    .collect::<Vec<_>>()
                    .join(", "),
                f.buttons
            )),
            _ => None,
        })
        .collect()
}

fn submit(host: &PluginHost, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: "target".into(), button: button.into(), values }));
}

/// `ssh` printing what gdbserver prints, then running on (or refusing, with
/// `COIMAN_FAKE_SSH_DOWN`); `scp` doing nothing. Both keep their calls in `calls.log`.
fn fakes(dir: &Path) {
    std::fs::write(
        dir.join("ssh.cmd"),
        "@echo off\r\n>>\"%~dp0calls.log\" echo ssh %*\r\n\
         if \"%COIMAN_FAKE_SSH_DOWN%\"==\"1\" (\r\n  >&2 echo pi@board: Permission denied ^(publickey^).\r\n  exit /b 255\r\n)\r\n\
         echo Process /home/pi/app created; pid = 77\r\necho Listening on port 2345\r\n\
         ping -n 60 127.0.0.1 >nul\r\n",
    )
    .unwrap();
    std::fs::write(dir.join("scp.cmd"), "@echo off\r\n>>\"%~dp0calls.log\" echo scp %*\r\nexit /b 0\r\n").unwrap();
}

#[test]
fn remote_debug_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-rdebug-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    fakes(&bin);
    let ws = tmp.join("project");
    std::fs::create_dir_all(ws.join("target")).unwrap();
    std::fs::write(ws.join("target/app"), "ELF").unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/remote-debug");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::WorkspaceWrite].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["ssh".to_owned(), "scp".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The form, with what a new target starts with.
    host.run_command(ID, "show", "");
    match wait(&host, |u| matches!(u, Update::Panel { .. })) {
        Update::Panel { blocks, .. } => {
            let f = form_text(&blocks);
            assert!(
                f.contains("mode=Start gdbserver over ssh") && f.contains("port=2345") && f.contains("[\"Start\"]"),
                "{f}"
            );
        }
        _ => unreachable!(),
    }

    // A gdbserver running already: gdb connects to it; the target is kept.
    submit(
        &host,
        "Start",
        &[("mode", "Connect to a gdbserver running"), ("host", "10.0.0.5"), ("port", "9000"), ("local", "target/app")],
    );
    assert_eq!(attached(&host), ("app on 10.0.0.5:9000".into(), "target/app".into(), "10.0.0.5:9000".into()));
    let kept = std::fs::read_to_string(ws.join(".coiman/remote_debug.toml")).unwrap();
    assert!(kept.contains("over_ssh = false") && kept.contains("host = \"10.0.0.5\""), "{kept}");

    // Over ssh, after a copy: gdb attaches once gdbserver listens, through the carried port.
    let ssh_values = [
        ("mode", "Start gdbserver over ssh"),
        ("host", "pi@board"),
        ("port", "2345"),
        ("local", "target/app"),
        ("remote", "/home/pi/app"),
        ("args", "--fast"),
        ("copy", "true"),
        ("gdbserver", "gdbserver"),
        ("local_port", "0"),
        ("ssh", ""),
    ];
    submit(&host, "Start", &ssh_values);
    message(&host, "Copying the program there");
    console(&host, "Listening on port 2345");
    assert_eq!(attached(&host).2, "localhost:2345");
    let calls = std::fs::read_to_string(bin.join("calls.log")).unwrap();
    assert!(calls.contains("scp -o \"BatchMode=yes\" target/app pi@board:/home/pi/app"), "{calls}");
    assert!(calls.contains("-L 2345:localhost:2345 pi@board gdbserver :2345 /home/pi/app --fast"), "{calls}");

    // The session's end stops ssh; a restart starts it all again.
    host.run_command(ID, "debug-ended", "");
    host.run_command(ID, "restart", "");
    console(&host, "Listening on port 2345");
    attached(&host);
    host.run_command(ID, "debug-ended", "");

    // ssh refusing: said, and no session.
    // SAFETY: as above.
    unsafe { std::env::set_var("COIMAN_FAKE_SSH_DOWN", "1") };
    submit(&host, "Start", &[("copy", "false")]);
    console(&host, "Permission denied (publickey).");
    message(&host, "Remote debugging couldn't start");
    message(&host, "ssh ended before gdbserver was ready (exit 255)");
    BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
    assert!(!BACKLOG.with_borrow(|b| b.iter().any(|u| matches!(u, Update::DebugAttach { .. }))));

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
