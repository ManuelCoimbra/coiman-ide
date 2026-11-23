//! GitHub Actions as `gh` tells them: a repository's workflow runs, their jobs and steps, and the
//! log of what failed. `gh` talks to GitHub with the user's own login; the editor opens no
//! connection itself. A failed step is found in its workflow file by its name.

use serde::Deserialize;

/// Runs listed.
pub const RUNS: usize = 30;

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

/// `gh run list`'s arguments.
pub fn runs_args(failed_only: bool) -> Vec<String> {
    let mut a: Vec<String> = ["run", "list", "--limit"].map(str::to_owned).into();
    a.push(RUNS.to_string());
    a.push("--json".into());
    a.push("databaseId,displayTitle,workflowName,headBranch,status,conclusion,event,createdAt,url".into());
    if failed_only {
        a.extend(["--status".into(), "failure".into()]);
    }
    a
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

/// Which lines of a log to show: the errors and the few lines before each, or all of them
/// (when asked, or when none is an error).
pub fn shown_lines(lines: &[LogLine], whole: bool) -> Vec<bool> {
    const BEFORE: usize = 4;
    if whole || !lines.iter().any(|l| l.error) {
        return vec![true; lines.len()];
    }
    let mut keep = vec![false; lines.len()];
    for (i, _) in lines.iter().enumerate().filter(|(_, l)| l.error) {
        keep[i.saturating_sub(BEFORE)..=i].fill(true);
    }
    keep
}

/// A log line without its color codes: terminal escapes (`ESC[31;1m`), and the `^[[31;1m`
/// PowerShell writes in their place.
pub fn strip_colors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let esc = rest.find('\u{1b}').map(|i| (i, 1));
        let caret = rest.find("^[").map(|i| (i, 2));
        let start = match (esc, caret) {
            (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
            (a, b) => a.or(b),
        };
        let Some((at, len)) = start else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let after = &rest[at + len..];
        // `[`, then digits, `;` and `?`, then a letter.
        if let Some(body) = after.strip_prefix('[') {
            let params = body.find(|c: char| !(c.is_ascii_digit() || c == ';' || c == '?')).unwrap_or(body.len());
            if body[params..].starts_with(|c: char| c.is_ascii_alphabetic()) {
                rest = &body[params + 1..];
                continue;
            }
        }
        out.push_str(&rest[at..at + len]);
        rest = after;
    }
}

/// Unix seconds of an RFC 3339 time as GitHub gives it (`2026-11-14T14:27:00Z`).
pub fn unix_seconds(at: &str) -> Option<i64> {
    let (date, time) = at.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let (clock, offset) = match time.find(['+', '-']) {
        Some(i) => (&time[..i], &time[i..]),
        None => (time, ""),
    };
    let mut t = clock.split(':');
    let (h, mi) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let s = t.next().and_then(|s| s.split('.').next()?.parse::<i64>().ok()).unwrap_or(0);
    // Days from 1970 (Howard Hinnant's days_from_civil).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut secs = days * 86_400 + h * 3600 + mi * 60 + s;
    if !offset.is_empty() {
        let sign = if offset.starts_with('-') { -1 } else { 1 };
        let mut o = offset[1..].split(':');
        let (oh, om) = (o.next()?.parse::<i64>().ok()?, o.next().and_then(|m| m.parse::<i64>().ok()).unwrap_or(0));
        secs -= sign * (oh * 3600 + om * 60);
    }
    Some(secs)
}

/// "3 hours ago".
pub fn relative_time(secs: i64) -> String {
    let units = [
        (365 * 86_400, "year"),
        (30 * 86_400, "month"),
        (7 * 86_400, "week"),
        (86_400, "day"),
        (3600, "hour"),
        (60, "minute"),
    ];
    for (size, name) in units {
        let n = secs / size;
        if n >= 1 {
            return format!("{n} {name}{} ago", if n == 1 { "" } else { "s" });
        }
    }
    "just now".to_owned()
}

/// The workflow name at a file's top (`name: CI`).
pub fn top_name(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix("name:")).map(|n| n.trim().trim_matches(['"', '\'']).to_owned())
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

/// What a failed `gh` call means, in a sentence.
pub fn explain(stderr: &str) -> String {
    let e = stderr.trim();
    let l = e.to_lowercase();
    if l.contains("gh auth login") || l.contains("not logged in") || l.contains("authentication") {
        "gh isn't logged in to GitHub: run gh auth login in a terminal".to_owned()
    } else if l.contains("no git remotes")
        || l.contains("none of the git remotes")
        || l.contains("not a git repository")
    {
        "This folder isn't a GitHub repository (no GitHub remote)".to_owned()
    } else if e.is_empty() {
        "gh failed".to_owned()
    } else {
        e.lines().last().unwrap_or(e).to_owned()
    }
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
        assert_eq!(runs_args(true)[runs_args(true).len() - 2..], ["--status", "failure"]);
    }

    #[test]
    fn times() {
        // 2026-11-14T14:27:00Z is 1 794 666 420.
        assert_eq!(unix_seconds("2026-11-14T14:27:00Z"), Some(1_794_666_420));
        assert_eq!(unix_seconds("2026-11-14T15:27:00.5+01:00"), Some(1_794_666_420));
        assert_eq!(unix_seconds("not a time"), None);
        assert_eq!(relative_time(90), "1 minute ago");
        assert_eq!(relative_time(7200), "2 hours ago");
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
        assert_eq!(strip_colors("a ^[ b \u{1b}x"), "a ^[ b \u{1b}x");
        let many: Vec<LogLine> = (0..10)
            .map(|i| LogLine { job: "j".into(), step: "s".into(), text: i.to_string(), error: i == 7 })
            .collect();
        let shown = shown_lines(&many, false);
        assert_eq!(shown.iter().filter(|s| **s).count(), 5);
        assert!(shown[3] && shown[7] && !shown[8] && !shown[2]);
        assert!(shown_lines(&many, true).iter().all(|s| *s));
    }

    #[test]
    fn steps_found_in_the_workflow() {
        let yml = "name: CI\non: push\njobs:\n  check:\n    steps:\n      - uses: actions/checkout@v4\n      - name: Rust toolchain\n        run: rustup show\n      - name: \"Checks\"\n        run: ./check\n";
        assert_eq!(top_name(yml).as_deref(), Some("CI"));
        assert_eq!(step_line(yml, "Checks"), Some(9));
        assert_eq!(step_line(yml, "Rust toolchain"), Some(7));
        assert_eq!(step_line(yml, "Run actions/checkout@v4"), Some(6));
        assert_eq!(step_line(yml, "Nope"), None);
        assert!(explain("To get started with GitHub CLI, please run:  gh auth login").contains("isn't logged in"));
    }
}
