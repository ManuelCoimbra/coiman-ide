//! The Benchmarks extension (`plugins/extensions/benchmarks`) in the real plugin host, with a
//! stand-in `cargo` on PATH printing Criterion's results and the real `git`: two runs recorded
//! with their commit, the slower benchmark flagged, and the history kept for the next start.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.benchmarks";

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
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
                    out.push_str(&format!("{}\n", cells.join(" | ")));
                }
            }
            Block::Plot(series) => {
                for s in series {
                    out.push_str(&format!("plot {}: {:?}\n", s.name, s.values));
                }
            }
            _ => {}
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.invalid", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(ok.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&ok.stderr));
}

fn results(fib: &str, sort: &str) -> String {
    format!(
        "   Compiling demo v0.1.0\nBenchmarking fib 20: Warming up for 3.0000 s\n\
         fib 20                  time:   [{fib}]\nsort 1000               time:   [{sort}]\n"
    )
}

#[test]
fn benchmarks_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-benchmarks-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let ws = tmp.join("project");
    std::fs::create_dir_all(&ws).unwrap();
    if cfg!(windows) {
        std::fs::write(bin.join("cargo.cmd"), "@echo off\r\ntype \"%~dp0out.txt\"\r\nexit /b 0\r\n").unwrap();
    } else {
        std::fs::write(bin.join("cargo"), "#!/bin/sh\ncat \"$(dirname \"$0\")/out.txt\"\n").unwrap();
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
        // The extension's saved runs go to a config folder of the test's own.
        std::env::set_var("APPDATA", tmp.join("config"));
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
    }
    std::fs::write(ws.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
    git(&ws, &["init", "-q"]);
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-q", "-m", "first"]);
    let head = String::from_utf8(
        Command::new("git").args(["rev-parse", "--short", "HEAD"]).current_dir(&ws).output().unwrap().stdout,
    )
    .unwrap()
    .trim()
    .to_owned();

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/benchmarks");
    let start = |ws: &Path| {
        let pkg = registry::read_folder(&dir).expect("the extension is built");
        let host = PluginHost::start(|| {}).unwrap();
        host.set_workspace(Some(ws.to_path_buf()));
        let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
        let programs = ["cargo".to_owned(), "git".to_owned()].into();
        host.load_with_programs(pkg.manifest, pkg.wasm, granted, programs);
        host
    };
    let host = start(&ws);
    host.run_command(ID, "show", "");
    wait_panel(&host, "No runs yet");

    std::fs::write(bin.join("out.txt"), results("26.0 µs 26.2 µs 26.5 µs", "1.10 ms 1.20 ms 1.30 ms")).unwrap();
    host.run_command(ID, "run", "");
    let first = wait_panel(&host, "2 benchmarks recorded: the next run is compared with this one");
    assert!(first.contains(&format!("Latest run, {head} · ")), "{first}");
    assert!(first.contains("fib 20 | 26.2 µs\n") && first.contains("sort 1000 | 1.20 ms\n"), "{first}");

    // A change not committed: the commit is marked, and fib got slower.
    std::fs::write(ws.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.2.0\"\n").unwrap();
    std::fs::write(bin.join("out.txt"), results("31.0 µs 31.44 µs 32.0 µs", "1.10 ms 1.19 ms 1.30 ms")).unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    host.run_command(ID, "run", "");
    let second = wait_panel(&host, "1 slower than the run before by more than 10%");
    assert!(second.contains(&format!("Latest run, {head}+ · ")), "{second}");
    assert!(second.contains("fib 20 | 31.4 µs | was 26.2 µs | +20.0%"), "{second}");
    assert!(second.contains("sort 1000 | 1.19 ms | was 1.20 ms | -0.8%"), "{second}");
    assert!(second.contains("plot fib 20: [100.0, 120.0"), "{second}");

    // Kept for the next start.
    drop(host);
    let host = start(&ws);
    host.run_command(ID, "show", "");
    let again = wait_panel(&host, "Over the last 2 runs");
    assert!(again.contains("+20.0%"), "{again}");
    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
