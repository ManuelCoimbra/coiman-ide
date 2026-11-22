//! The Docker extension (`plugins/extensions/docker`, rebuilt with `scripts/build-plugins.ps1`)
//! in the real plugin host, against a stand-in `docker` program that answers as Docker does
//! (this machine has no Docker): containers, images, Compose services, the actions on them,
//! what runs in a terminal, and an engine that isn't running.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.docker";

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
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
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
            _ => {}
        }
    }
    out
}

/// A `docker` that prints what Docker prints for `ps`, `images` and `compose ps`, keeps each
/// call in `calls.log`, refuses to remove the image `busy:1`, and says the engine is down when
/// `COIMAN_FAKE_DOCKER_DOWN` is 1.
fn fake_docker(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    w(
        "ps.json",
        "{\"ID\":\"a1b2\",\"Image\":\"nginx:1.27\",\"Names\":\"web\",\"Ports\":\"0.0.0.0:8080->80/tcp\",\"State\":\"running\",\"Status\":\"Up 2 hours\"}\n{\"ID\":\"c3d4\",\"Image\":\"postgres:17\",\"Names\":\"db\",\"Ports\":\"\",\"State\":\"exited\",\"Status\":\"Exited (0) 3 days ago\"}\n",
    );
    w(
        "images.json",
        "{\"ID\":\"sha1\",\"Repository\":\"nginx\",\"Tag\":\"1.27\",\"Size\":\"187MB\",\"CreatedSince\":\"2 weeks ago\"}\n{\"ID\":\"sha9\",\"Repository\":\"busy\",\"Tag\":\"1\",\"Size\":\"5MB\",\"CreatedSince\":\"1 day ago\"}\n",
    );
    w(
        "services.json",
        "{\"Service\":\"web\",\"Name\":\"app-web-1\",\"State\":\"running\",\"Status\":\"Up 5 minutes\",\"Publishers\":[{\"TargetPort\":80,\"PublishedPort\":8080}]}\n",
    );
    if cfg!(windows) {
        w(
            "docker.cmd",
            "@echo off\r\n>>\"%~dp0calls.log\" echo %*\r\n\
             if \"%COIMAN_FAKE_DOCKER_DOWN%\"==\"1\" (\r\n  >&2 echo error during connect: this error may indicate that the docker daemon is not running\r\n  exit /b 1\r\n)\r\n\
             if \"%1\"==\"ps\" type \"%~dp0ps.json\" & exit /b 0\r\n\
             if \"%1\"==\"images\" type \"%~dp0images.json\" & exit /b 0\r\n\
             if \"%1\"==\"compose\" if \"%4\"==\"ps\" type \"%~dp0services.json\" & exit /b 0\r\n\
             if \"%1\"==\"rmi\" if \"%2\"==\"busy:1\" (\r\n  >&2 echo Error response from daemon: conflict: image is being used by a container\r\n  exit /b 1\r\n)\r\n\
             exit /b 0\r\n",
        );
    } else {
        let script = "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$*\" >> \"$d/calls.log\"\n\
            if [ \"$COIMAN_FAKE_DOCKER_DOWN\" = 1 ]; then echo 'error during connect: this error may indicate that the docker daemon is not running' >&2; exit 1; fi\n\
            case \"$1\" in ps) cat \"$d/ps.json\";; images) cat \"$d/images.json\";;\n\
            compose) [ \"$4\" = ps ] && cat \"$d/services.json\";;\n\
            rmi) [ \"$2\" = busy:1 ] && { echo 'Error response from daemon: conflict: image is being used by a container' >&2; exit 1; };;\n\
            esac\nexit 0\n";
        w("docker", script);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("docker"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

fn calls(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

#[test]
fn docker_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-docker-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    fake_docker(&bin);
    let ws = tmp.join("project");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("compose.yaml"), "services:\n  web:\n    image: nginx\n").unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/docker");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["docker".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // With a Compose file, its tab first; every list read.
    host.run_command(ID, "show", "");
    let compose = panel(&host, "Compose  1");
    assert!(compose.contains("[▸ Compose  1 tab compose]") && compose.contains("compose.yaml"), "{compose}");
    assert!(compose.contains("web | app-web-1 | Up 5 minutes | 8080->80"), "{compose}");
    assert!(compose.contains("[Containers  2 tab containers]") && compose.contains("[Images  2 tab images]"));
    let asked = calls(&bin);
    assert!(asked.contains("ps -a --no-trunc --format json") && asked.contains("images --format json"), "{asked}");
    assert!(asked.contains("compose -f compose.yaml ps -a --format json"), "{asked}");

    host.run_command(ID, "tab", "containers");
    let list = panel(&host, "[▸ Containers  2");
    assert!(list.contains("web | nginx:1.27 | Up 2 hours | 0.0.0.0:8080->80/tcp [Stop stop web] [Restart restart web] [Shell shell web] [Logs logs web]"), "{list}");
    assert!(
        list.contains("db | postgres:17 | Exited (0) 3 days ago |  [Start start db] [Remove… ask db] [Logs logs db]"),
        "{list}"
    );

    // Actions: done, said, and the lists read again.
    host.run_command(ID, "stop", "web");
    message(&host, "Stopped web");
    host.run_command(ID, "ask", "db");
    panel(&host, "[Remove it remove db] [Keep keep ]");
    host.run_command(ID, "remove", "db");
    message(&host, "Removed db");
    let asked = calls(&bin);
    assert!(asked.contains("stop web") && asked.contains("rm db"), "{asked}");

    // An image in use: Docker's refusal is said.
    host.run_command(ID, "tab", "images");
    panel(&host, "busy:1 | 5MB");
    host.run_command(ID, "ask", "busy:1");
    host.run_command(ID, "remove-image", "busy:1");
    panel(&host, "Couldn't remove busy:1: Error response from daemon: conflict: image is being used by a container");

    // Long ones in a terminal.
    host.run_command(ID, "shell", "web");
    match wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) {
        Update::RunInTerminal { title, args, .. } => {
            assert_eq!(title, "shell web");
            assert_eq!(args[..3], ["exec", "-it", "web"]);
        }
        _ => unreachable!(),
    }
    host.run_command(ID, "compose", "up");
    match wait(&host, |u| matches!(u, Update::RunInTerminal { .. })) {
        Update::RunInTerminal { args, .. } => assert_eq!(args, ["compose", "-f", "compose.yaml", "up", "--build"]),
        _ => unreachable!(),
    }
    host.run_command(ID, "compose", "down");
    message(&host, "Compose: down");

    // No engine: said once, plainly.
    // SAFETY: as above.
    unsafe { std::env::set_var("COIMAN_FAKE_DOCKER_DOWN", "1") };
    host.run_command(ID, "refresh", "");
    let down = panel(&host, "Docker's engine isn't running");
    assert!(down.contains("start Docker Desktop"), "{down}");

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
