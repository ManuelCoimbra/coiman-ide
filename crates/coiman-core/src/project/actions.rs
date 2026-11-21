//! GitHub Actions through the `gh` program: a repository's workflow runs, their jobs and
//! steps, and the log of what failed. `gh` talks to GitHub with the user's own login; Coiman
//! opens no connection itself. A failed step is found in its workflow file by its name.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;

use crate::language::lsp::discovery::quiet_command;
use crate::language::lsp::servers::which;

/// A workflow run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub database_id: u64,
    pub display_title: String,
    pub workflow_name: String,
    pub head_branch: String,
    /// `queued`, `in_progress`, `completed`…
    pub status: String,
    /// When completed: `success`, `failure`, `cancelled`, `skipped`…
    #[serde(default)]
    pub conclusion: String,
    pub event: String,
    pub created_at: String,
    pub url: String,
}

impl Run {
    /// One word for how it went: the conclusion once it's done, else its status.
    pub fn state(&self) -> &str {
        if self.status == "completed" { &self.conclusion } else { &self.status }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Step {
    pub name: String,
    pub number: u32,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    #[serde(default)]
    pub database_id: u64,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub conclusion: String,
    #[serde(default)]
    pub steps: Vec<Step>,
}

/// A line of a failed job's log: its job, step and text (without the time stamp).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub job: String,
    pub step: String,
    pub text: String,
    /// An error line (`##[error]`, or `error:`/`error[` from a compiler).
    pub error: bool,
}

pub fn parse_runs(json: &str) -> Result<Vec<Run>, String> {
    serde_json::from_str(json).map_err(|e| format!("couldn't read gh's answer: {e}"))
}

pub fn parse_jobs(json: &str) -> Result<Vec<Job>, String> {
    #[derive(Deserialize)]
    struct View {
        jobs: Vec<Job>,
    }
    serde_json::from_str::<View>(json).map(|v| v.jobs).map_err(|e| format!("couldn't read gh's answer: {e}"))
}

/// `gh run view --log-failed`: `job<TAB>step<TAB>2026-11-14T14:27:48.1692873Z text`.
pub fn parse_failed_log(text: &str) -> Vec<LogLine> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (job, step, rest) = (parts.next()?, parts.next()?, parts.next()?);
            let rest = rest.trim_start_matches('\u{feff}');
            // The time stamp, up to its `Z `.
            let text = match rest.split_once("Z ") {
                Some((stamp, t)) if stamp.len() >= 19 && stamp.as_bytes()[4] == b'-' => t,
                _ => rest,
            };
            let text = strip_colors(text);
            let lower = text.trim_start().to_ascii_lowercase();
            let error = text.contains("##[error]") || lower.starts_with("error:") || lower.starts_with("error[");
            Some(LogLine { job: job.to_owned(), step: step.to_owned(), text, error })
        })
        .collect()
}

/// Seconds from `created_at` (RFC 3339, as GitHub gives it) to `now` (Unix seconds).
pub fn age_seconds(created_at: &str, now: i64) -> Option<i64> {
    let at = chrono::DateTime::parse_from_rfc3339(created_at).ok()?;
    Some((now - at.timestamp()).max(0))
}

/// A log line without its color codes: terminal escapes, and the `^[[31;1m` PowerShell
/// writes in their place.
pub fn strip_colors(text: &str) -> String {
    static CODES: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = CODES.get_or_init(|| regex::Regex::new(r"(?:\x1b|\^\[)\[[0-9;?]*[A-Za-z]").expect("valid pattern"));
    re.replace_all(text, "").into_owned()
}

/// `gh`, if installed.
pub fn gh() -> Option<PathBuf> {
    which("gh")
}

fn run_gh(root: &Path, args: &[&str]) -> Result<String, String> {
    let program =
        gh().ok_or("The gh program isn't installed: get GitHub CLI (cli.github.com), then `gh auth login`")?;
    let out = quiet_command(&program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("couldn't start gh: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if err.is_empty() { format!("gh stopped ({})", out.status) } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The repository's latest workflow runs, newest first; only those that failed when asked.
pub fn runs(root: &Path, limit: usize, failed_only: bool) -> Result<Vec<Run>, String> {
    let limit = limit.to_string();
    let mut args = vec![
        "run",
        "list",
        "--limit",
        &limit,
        "--json",
        "databaseId,displayTitle,workflowName,headBranch,status,conclusion,event,createdAt,url",
    ];
    if failed_only {
        args.extend(["--status", "failure"]);
    }
    parse_runs(&run_gh(root, &args)?)
}

/// A run's jobs and their steps.
pub fn jobs(root: &Path, run: u64) -> Result<Vec<Job>, String> {
    parse_jobs(&run_gh(root, &["run", "view", &run.to_string(), "--json", "jobs"])?)
}

/// The log of a run's failed steps.
pub fn failed_log(root: &Path, run: u64) -> Result<Vec<LogLine>, String> {
    Ok(parse_failed_log(&run_gh(root, &["run", "view", &run.to_string(), "--log-failed"])?))
}

/// Runs a run's failed jobs again (or all of it): GitHub starts them, which uses its minutes.
pub fn rerun(root: &Path, run: u64, failed_only: bool) -> Result<(), String> {
    let id = run.to_string();
    let mut args = vec!["run", "rerun", id.as_str()];
    if failed_only {
        args.push("--failed");
    }
    run_gh(root, &args).map(drop)
}

/// The workflow file of a workflow, by the `name:` at its top (or its file name, which GitHub
/// shows when there's none).
pub fn workflow_file(root: &Path, workflow: &str) -> Option<PathBuf> {
    let dir = root.join(".github").join("workflows");
    let files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    let top_name = |text: &str| {
        text.lines().find_map(|l| l.strip_prefix("name:")).map(|n| n.trim().trim_matches(['"', '\'']).to_owned())
    };
    files
        .iter()
        .find(|p| std::fs::read_to_string(p).ok().and_then(|t| top_name(&t)).as_deref() == Some(workflow))
        .or_else(|| files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy() == workflow)))
        .cloned()
}

/// The line (1-based) where a step is named in a workflow file: `- name: Step` or `name: Step`.
pub fn step_line(text: &str, step: &str) -> Option<usize> {
    // GitHub names a `uses:` step without a name "Run owner/action@v1".
    let uses = step.strip_prefix("Run ");
    text.lines()
        .position(|l| {
            let t = l.trim_start().trim_start_matches("- ").trim();
            let value = |key: &str| t.strip_prefix(key).map(|v| v.trim().trim_matches(['"', '\'']));
            value("name:") == Some(step) || (uses.is_some() && value("uses:") == uses)
        })
        .map(|i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_runs_and_jobs() {
        let runs = parse_runs(r#"[{"conclusion":"failure","createdAt":"2026-11-14T14:27:00Z","databaseId":7,"displayTitle":"Fix","event":"push","headBranch":"main","status":"completed","url":"https://x","workflowName":"CI"},{"conclusion":"","createdAt":"2026-11-15T10:00:00Z","databaseId":8,"displayTitle":"Next","event":"push","headBranch":"main","status":"in_progress","url":"https://y","workflowName":"CI"}]"#).unwrap();
        assert_eq!((runs[0].database_id, runs[0].state()), (7, "failure"));
        assert_eq!(runs[1].state(), "in_progress");
        let jobs = parse_jobs(r#"{"jobs":[{"databaseId":1,"name":"build","status":"completed","conclusion":"failure","steps":[{"name":"Set up job","number":1,"status":"completed","conclusion":"success"},{"name":"Checks","number":2,"status":"completed","conclusion":"failure"}]}]}"#).unwrap();
        assert_eq!(jobs[0].steps[1].conclusion, "failure");
    }

    #[test]
    fn ages() {
        // 2026-11-14T14:27:00Z is 1 794 666 420.
        assert_eq!(age_seconds("2026-11-14T14:27:00Z", 1_794_666_420 + 90), Some(90));
        assert_eq!(age_seconds("not a time", 0), None);
    }

    #[test]
    fn failed_log_lines() {
        let text = "build\tChecks\t\u{feff}2026-11-14T14:27:48.1692873Z Compiling x\nbuild\tChecks\t2026-11-14T14:27:49.0000000Z error[E0425]: cannot find value\nbuild\tChecks\t2026-11-14T14:27:50.0000000Z ##[error]Process completed with exit code 1.\n";
        let lines = parse_failed_log(text);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].text, "Compiling x");
        assert!(!lines[0].error && lines[1].error && lines[2].error);
        assert_eq!((lines[2].job.as_str(), lines[2].step.as_str()), ("build", "Checks"));
        let colored =
            parse_failed_log("b\ts\t2026-11-14T14:27:48.1Z ^[[31;1m^[[0m^[[36;1mLine |^[[0m \u{1b}[1mx\u{1b}[0m\n");
        assert_eq!(colored[0].text, "Line | x");
    }

    #[test]
    fn steps_found_in_the_workflow() {
        let yml = "name: CI\non: push\njobs:\n  check:\n    steps:\n      - uses: actions/checkout@v4\n      - name: Rust toolchain\n        run: rustup show\n      - name: \"Checks\"\n        run: ./check\n";
        assert_eq!(step_line(yml, "Checks"), Some(9));
        assert_eq!(step_line(yml, "Rust toolchain"), Some(7));
        assert_eq!(step_line(yml, "Run actions/checkout@v4"), Some(6));
        assert_eq!(step_line(yml, "Nope"), None);
    }

    /// This repository's own workflows are found by their names.
    #[test]
    fn this_repositorys_workflows() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let ci = workflow_file(&root, "CI").expect("ci.yml");
        assert!(ci.ends_with("ci.yml"));
        let text = std::fs::read_to_string(ci).unwrap();
        assert!(step_line(&text, "Checks").is_some());
    }
}
