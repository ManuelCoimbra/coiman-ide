//! What test and build runs did in a project, kept between sessions: each run's results, so a
//! test that passes and fails by turns can be told apart (flaky), two runs compared, and a
//! failed build's first error found again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::launch::testing::Outcome;

/// Runs kept for each kind; older ones are dropped.
pub const KEEP: usize = 100;
/// A test is flaky when, among its last this many results, it both passed and failed.
pub const FLAKY_WINDOW: usize = 10;

/// One test's result in a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestEntry {
    /// The suite and name together, as the runner names the test (`a::b::c`, `Suite.name`).
    pub id: String,
    pub outcome: Outcome,
    #[serde(default)]
    pub millis: Option<u64>,
    /// The failure's first line.
    #[serde(default)]
    pub message: String,
}

/// One run of tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TestRun {
    /// Seconds since 1970.
    pub at: u64,
    /// Which runner: "cargo test", "pytest", "go test"…
    pub runner: String,
    /// The whole suite ran (not one test or a few).
    pub whole: bool,
    pub seconds: f32,
    pub tests: Vec<TestEntry>,
}

impl TestRun {
    pub fn count(&self, outcome: Outcome) -> usize {
        self.tests.iter().filter(|t| t.outcome == outcome).count()
    }

    pub fn failed(&self) -> Vec<&TestEntry> {
        self.tests.iter().filter(|t| t.outcome == Outcome::Failed).collect()
    }
}

/// One build, check or run of a program.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BuildRun {
    pub at: u64,
    /// What ran: "cargo check", a run configuration's name…
    pub what: String,
    pub success: bool,
    #[serde(default)]
    pub exit: Option<i64>,
    #[serde(default)]
    pub seconds: Option<f32>,
    pub errors: usize,
    pub warnings: usize,
    /// The first error: file, 1-based line, message.
    #[serde(default)]
    pub first_error: Option<(PathBuf, u32, String)>,
}

/// A project's history.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub tests: Vec<TestRun>,
    #[serde(default)]
    pub builds: Vec<BuildRun>,
}

/// A test that both passed and failed lately.
#[derive(Clone, Debug, PartialEq)]
pub struct Flaky {
    pub id: String,
    pub passed: usize,
    pub failed: usize,
    /// Its results, oldest first, among the last [`FLAKY_WINDOW`].
    pub recent: Vec<Outcome>,
}

/// What changed from one run to the next.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comparison {
    /// Passed before, failed now.
    pub broke: Vec<String>,
    /// Failed before, passed now.
    pub fixed: Vec<String>,
    /// Took at least half as long again (and 100 ms more): id, then before and now in ms.
    pub slower: Vec<(String, u64, u64)>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

fn file(root: &Path) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().to_lowercase().replace('\\', "/").trim_end_matches('/').hash(&mut hash);
    Some(crate::settings::config_dir()?.join("run_history").join(format!("{:016x}.json", hash.finish())))
}

impl History {
    /// The project's history, or an empty one.
    pub fn load(root: &Path) -> Self {
        file(root)
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        let Some(path) = file(root) else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    pub fn add_tests(&mut self, run: TestRun) {
        self.tests.push(run);
        let over = self.tests.len().saturating_sub(KEEP);
        self.tests.drain(..over);
    }

    pub fn add_build(&mut self, run: BuildRun) {
        self.builds.push(run);
        let over = self.builds.len().saturating_sub(KEEP);
        self.builds.drain(..over);
    }

    /// A test's results, oldest first, across every run that had it.
    pub fn results_of(&self, id: &str) -> Vec<Outcome> {
        self.tests.iter().filter_map(|r| r.tests.iter().find(|t| t.id == id)).map(|t| t.outcome).collect()
    }

    /// Tests that both passed and failed among their last [`FLAKY_WINDOW`] results, those
    /// that flip most first.
    pub fn flaky(&self) -> Vec<Flaky> {
        let mut by_test: BTreeMap<&str, Vec<Outcome>> = BTreeMap::new();
        for run in &self.tests {
            for t in &run.tests {
                by_test.entry(t.id.as_str()).or_default().push(t.outcome);
            }
        }
        let mut out: Vec<Flaky> = by_test
            .into_iter()
            .filter_map(|(id, all)| {
                let recent: Vec<Outcome> = all[all.len().saturating_sub(FLAKY_WINDOW)..]
                    .iter()
                    .copied()
                    .filter(|o| *o != Outcome::Skipped)
                    .collect();
                let passed = recent.iter().filter(|o| **o == Outcome::Passed).count();
                let failed = recent.iter().filter(|o| **o == Outcome::Failed).count();
                (passed > 0 && failed > 0).then(|| Flaky { id: id.to_owned(), passed, failed, recent })
            })
            .collect();
        let flips = |f: &Flaky| f.recent.windows(2).filter(|w| w[0] != w[1]).count();
        out.sort_by(|a, b| flips(b).cmp(&flips(a)).then(a.id.cmp(&b.id)));
        out
    }
}

/// How `after` differs from `before`.
pub fn compare(before: &TestRun, after: &TestRun) -> Comparison {
    let old: BTreeMap<&str, &TestEntry> = before.tests.iter().map(|t| (t.id.as_str(), t)).collect();
    let new: BTreeMap<&str, &TestEntry> = after.tests.iter().map(|t| (t.id.as_str(), t)).collect();
    let mut out = Comparison::default();
    for (id, now) in &new {
        let Some(was) = old.get(id) else {
            out.added.push((*id).to_owned());
            continue;
        };
        match (was.outcome, now.outcome) {
            (Outcome::Passed, Outcome::Failed) => out.broke.push((*id).to_owned()),
            (Outcome::Failed, Outcome::Passed) => out.fixed.push((*id).to_owned()),
            _ => {}
        }
        if let (Some(a), Some(b)) = (was.millis, now.millis)
            && b >= a + 100
            && b * 2 >= a * 3
        {
            out.slower.push(((*id).to_owned(), a, b));
        }
    }
    let new_ids: BTreeSet<&str> = new.keys().copied().collect();
    out.removed = old.keys().filter(|id| !new_ids.contains(*id)).map(|id| (*id).to_owned()).collect();
    out.slower.sort_by_key(|a| std::cmp::Reverse(a.2 - a.1));
    out
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(at: u64, results: &[(&str, Outcome, Option<u64>)]) -> TestRun {
        TestRun {
            at,
            runner: "cargo test".to_owned(),
            whole: true,
            seconds: 1.0,
            tests: results
                .iter()
                .map(|(id, outcome, millis)| TestEntry {
                    id: (*id).to_owned(),
                    outcome: *outcome,
                    millis: *millis,
                    message: String::new(),
                })
                .collect(),
        }
    }

    use Outcome::{Failed as F, Passed as P, Skipped as S};

    #[test]
    fn flaky_tests_pass_and_fail_by_turns() {
        let mut h = History::default();
        h.add_tests(run(1, &[("steady", P, None), ("flips", P, None), ("broken", F, None), ("once", F, None)]));
        h.add_tests(run(2, &[("steady", P, None), ("flips", F, None), ("broken", F, None), ("once", F, None)]));
        h.add_tests(run(3, &[("steady", P, None), ("flips", P, None), ("broken", F, None), ("once", P, None)]));
        h.add_tests(run(4, &[("steady", S, None), ("flips", F, None)]));
        let flaky = h.flaky();
        let ids: Vec<&str> = flaky.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["flips", "once"], "flips most first; steady and always-broken aren't flaky");
        assert_eq!((flaky[0].passed, flaky[0].failed), (2, 2));
        assert_eq!(h.results_of("once"), [F, F, P]);
    }

    #[test]
    fn only_recent_results_count_for_flakiness() {
        let mut h = History::default();
        h.add_tests(run(0, &[("t", F, None)]));
        for at in 1..=FLAKY_WINDOW as u64 {
            h.add_tests(run(at, &[("t", P, None)]));
        }
        assert!(h.flaky().is_empty(), "the failure is older than the window");
    }

    #[test]
    fn runs_compare() {
        let before = run(1, &[("a", P, Some(100)), ("b", F, Some(10)), ("c", P, Some(1000)), ("gone", P, None)]);
        let after = run(2, &[("a", F, Some(120)), ("b", P, Some(10)), ("c", P, Some(1600)), ("new", P, None)]);
        let c = compare(&before, &after);
        assert_eq!(c.broke, ["a"]);
        assert_eq!(c.fixed, ["b"]);
        assert_eq!(c.slower, [("c".to_owned(), 1000, 1600)]);
        assert_eq!((c.added, c.removed), (vec!["new".to_owned()], vec!["gone".to_owned()]));
    }

    #[test]
    fn history_keeps_the_last_runs_and_reads_back() {
        let mut h = History::default();
        for at in 0..(KEEP as u64 + 5) {
            h.add_tests(run(at, &[("t", P, None)]));
            h.add_build(BuildRun {
                at,
                what: "cargo check".to_owned(),
                success: at % 2 == 0,
                exit: Some(0),
                seconds: Some(2.5),
                errors: 0,
                warnings: 1,
                first_error: None,
            });
        }
        assert_eq!((h.tests.len(), h.builds.len()), (KEEP, KEEP));
        assert_eq!(h.tests[0].at, 5);
        let text = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<History>(&text).unwrap(), h);
        assert_eq!(serde_json::from_str::<History>("{}").unwrap(), History::default());
    }
}
