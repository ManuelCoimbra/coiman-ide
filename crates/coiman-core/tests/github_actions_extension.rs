//! The GitHub Actions extension (`plugins/extensions/github-actions`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host, against a stand-in `gh` that answers as
//! GitHub CLI does (no GitHub login is used): runs, only the failed ones, a run's jobs and steps,
//! the log of what failed, a re-run asked first, a failed step found in its workflow file, and
//! GitHub opened in the browser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.github-actions";

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
                    let buttons: Vec<String> = r
                        .buttons
                        .iter()
                        .map(|b| format!("[{} {} {}]", b.label, b.command, b.argument.replace('\n', "|")))
                        .collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            _ => {}
        }
    }
    out
}

/// A `gh` that answers `run list` (all, or `--status failure`), `run view 7 --json jobs`,
/// `run view 7 --log-failed` and `run rerun`, keeping each call in `calls.log`.
fn fake_gh(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let w = |name: &str, text: &str| std::fs::write(dir.join(name), text).unwrap();
    let failed = r#"{"conclusion":"failure","createdAt":"2026-11-14T14:27:00Z","databaseId":7,"displayTitle":"Fix the parser","event":"push","headBranch":"main","status":"completed","url":"https://github.com/o/r/actions/runs/7","workflowName":"CI"}"#;
    let going = r#"{"conclusion":"","createdAt":"2026-11-15T10:00:00Z","databaseId":8,"displayTitle":"Next","event":"push","headBranch":"dev","status":"in_progress","url":"https://github.com/o/r/actions/runs/8","workflowName":"CI"}"#;
    w("all.json", &format!("[{going},{failed}]"));
    w("failed.json", &format!("[{failed}]"));
    w(
        "jobs.json",
        r#"{"jobs":[{"databaseId":1,"name":"build","status":"completed","conclusion":"failure","steps":[{"name":"Set up job","number":1,"status":"completed","conclusion":"success"},{"name":"Checks","number":2,"status":"completed","conclusion":"failure"}]},{"databaseId":2,"name":"docs","status":"completed","conclusion":"success","steps":[{"name":"Build","number":1,"status":"completed","conclusion":"success"}]}]}"#,
    );
    let mut log = String::new();
    for i in 0..10 {
        log.push_str(&format!("build\tChecks\t2026-11-14T14:27:4{i}.0000000Z line {i}\n"));
    }
    log.push_str("build\tChecks\t2026-11-14T14:27:50.0000000Z error[E0425]: cannot find value `x`\n");
    log.push_str("build\tChecks\t2026-11-14T14:27:51.0000000Z ##[error]Process completed with exit code 1.\n");
    w("log.txt", &log);
    if cfg!(windows) {
        w(
            "gh.cmd",
            "@echo off\r\n>>\"%~dp0calls.log\" echo %*\r\n\
             if \"%2\"==\"list\" (\r\n  echo %* | findstr /c:\"--status failure\" >nul && (type \"%~dp0failed.json\" & exit /b 0)\r\n  type \"%~dp0all.json\" & exit /b 0\r\n)\r\n\
             if \"%2\"==\"view\" if \"%4\"==\"--json\" type \"%~dp0jobs.json\" & exit /b 0\r\n\
             if \"%2\"==\"view\" if \"%4\"==\"--log-failed\" type \"%~dp0log.txt\" & exit /b 0\r\n\
             exit /b 0\r\n",
        );
    } else {
        w(
            "gh",
            "#!/bin/sh\nd=$(dirname \"$0\")\necho \"$*\" >> \"$d/calls.log\"\n\
             case \"$2\" in list) case \"$*\" in *'--status failure'*) cat \"$d/failed.json\";; *) cat \"$d/all.json\";; esac;;\n\
             view) [ \"$4\" = --json ] && cat \"$d/jobs.json\"; [ \"$4\" = --log-failed ] && cat \"$d/log.txt\";;\n\
             esac\nexit 0\n",
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("gh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[test]
fn github_actions_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-actions-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    fake_gh(&bin);
    let ws = tmp.join("project");
    std::fs::create_dir_all(ws.join(".github/workflows")).unwrap();
    std::fs::write(ws.join(".github/workflows/other.yml"), "name: Docs\njobs: {}\n").unwrap();
    std::fs::write(
        ws.join(".github/workflows/ci.yml"),
        "name: CI\non: push\njobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n      - name: Checks\n        run: ./check\n",
    )
    .unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    }

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/github-actions");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, ["gh".to_owned()].into());
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // The latest runs, then only those that failed.
    host.run_command(ID, "show", "");
    let runs = panel(&host, "Fix the parser");
    assert!(runs.contains("● | Next | CI · dev · in progress · "), "{runs}");
    assert!(runs.contains("● | Fix the parser | CI · main · failure · ") && runs.contains("[Jobs select 7]"), "{runs}");
    host.run_command(ID, "only-failed", "");
    let failed = panel(&host, "[▸ Only failed only-failed ]");
    let failed = if failed.contains("Next") { panel(&host, "Fix the parser") } else { failed };
    assert!(!failed.contains("| Next |"), "{failed}");

    // A run's jobs: the failed step, with its place in the workflow.
    host.run_command(ID, "select", "7");
    let jobs = panel(&host, "2. Checks");
    assert!(jobs.contains("CI: Fix the parser") && jobs.contains("● build"), "{jobs}");
    assert!(jobs.contains("[Open in workflow open-step CI|Checks]"), "{jobs}");
    assert!(jobs.contains("1. Set up job") && !jobs.contains("1. Build"), "only a failed job's steps: {jobs}");
    host.run_command(ID, "open-step", "CI\nChecks");
    match wait(&host, |u| matches!(u, Update::Open { .. })) {
        Update::Open { path, line } => {
            assert!(path.ends_with("ci.yml"), "{}", path.display());
            assert_eq!(line, 7);
        }
        _ => unreachable!(),
    }

    // The log: the errors and the lines before them, or all of it.
    host.run_command(ID, "log", "7");
    let log = panel(&host, "12 lines, 2 errors (in red)");
    assert!(log.contains("line 6\nline 7\nline 8\nline 9\nerror[E0425]"), "{log}");
    assert!(!log.contains("line 5\n") && !log.contains("⋯"), "{log}");
    host.run_command(ID, "whole-log", "");
    let whole = panel(&host, "[▸ Whole log whole-log ]");
    assert!(whole.contains("line 0\n") && whole.contains("##[error]Process completed"), "{whole}");

    // Re-running asks first: it uses GitHub's minutes.
    host.run_command(ID, "rerun-ask", "");
    let ask = panel(&host, "[Run them again rerun 7]");
    assert!(ask.contains("on its minutes"), "{ask}");
    assert!(!std::fs::read_to_string(bin.join("calls.log")).unwrap().contains("rerun"));
    host.run_command(ID, "rerun", "7");
    panel(&host, "GitHub is running the failed jobs again");
    let calls = std::fs::read_to_string(bin.join("calls.log")).unwrap();
    assert!(calls.contains("run rerun 7 --failed") && calls.contains("run view 7 --log-failed"), "{calls}");

    host.run_command(ID, "open", "https://github.com/o/r/actions/runs/7");
    wait(&host, |u| matches!(u, Update::OpenUrl(u) if u == "https://github.com/o/r/actions/runs/7"));

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
