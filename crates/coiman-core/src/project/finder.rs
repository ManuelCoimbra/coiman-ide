//! Quick Open file index: a background walk of the folder (respecting
//! `.gitignore`, like ripgrep) feeds nucleo, which ranks matches on its own
//! threads. The UI only reads the latest ranked snapshot.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ignore::{WalkBuilder, WalkState};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Nucleo, Utf32Str};

use crate::project::workspace::IGNORED_DIRS;

pub struct Entry {
    pub path: PathBuf,
    /// Path relative to the folder, as shown and matched.
    pub rel: String,
}

pub struct FileMatch {
    pub path: PathBuf,
    pub rel: String,
    /// Char indices into `rel` that matched the query.
    pub indices: Vec<u32>,
}

pub struct FileFinder {
    nucleo: Nucleo<Entry>,
    root: Option<PathBuf>,
    /// Bumped on every new walk so older walks stop early.
    walk_generation: Arc<AtomicU64>,
    walking: Arc<AtomicBool>,
    query: String,
    /// Set when files were created or deleted; the next Quick Open rescans.
    stale: bool,
    matcher: Matcher,
}

/// Builds a walker over `root` that honours .gitignore and skips heavy folders.
pub fn walker(root: &Path) -> WalkBuilder {
    let mut b = WalkBuilder::new(root);
    b.hidden(false).require_git(false).filter_entry(|e| {
        let is_dir = e.file_type().is_some_and(|t| t.is_dir());
        !(is_dir && IGNORED_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
    });
    b
}

impl FileFinder {
    /// `notify` is called (from worker threads) when new results are ready.
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        let notify = Arc::new(notify);
        Self {
            nucleo: Nucleo::new(Config::DEFAULT.match_paths(), notify, None, 1),
            root: None,
            walk_generation: Arc::new(AtomicU64::new(0)),
            walking: Arc::new(AtomicBool::new(false)),
            query: String::new(),
            stale: false,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    pub fn set_root(&mut self, root: Option<PathBuf>) {
        if self.root == root {
            return;
        }
        self.root = root;
        self.rescan();
    }

    /// Marks the index out of date (files were created or deleted).
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// Rescans if files changed since the last walk.
    pub fn refresh_if_stale(&mut self) {
        if self.stale {
            self.rescan();
        }
    }

    pub fn rescan(&mut self) {
        self.stale = false;
        self.nucleo.restart(true);
        let generation = self.walk_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(root) = self.root.clone() else { return };
        let injector = self.nucleo.injector();
        let current = self.walk_generation.clone();
        let walking = self.walking.clone();
        walking.store(true, Ordering::SeqCst);
        std::thread::spawn(move || {
            walker(&root).build_parallel().run(|| {
                let injector = injector.clone();
                let root = root.clone();
                let current = current.clone();
                Box::new(move |entry| {
                    if current.load(Ordering::Relaxed) != generation {
                        return WalkState::Quit;
                    }
                    if let Ok(e) = entry
                        && e.file_type().is_some_and(|t| t.is_file())
                    {
                        let path = e.into_path();
                        let rel = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().into_owned();
                        injector.push(Entry { path, rel }, |e, cols| cols[0] = e.rel.as_str().into());
                    }
                    WalkState::Continue
                })
            });
            if current.load(Ordering::SeqCst) == generation {
                walking.store(false, Ordering::SeqCst);
            }
        });
    }

    /// Adds a path directly, bypassing the folder walk (benchmarks, tests).
    pub fn inject(&self, path: PathBuf, rel: String) {
        self.nucleo.injector().push(Entry { path, rel }, |e, cols| cols[0] = e.rel.as_str().into());
    }

    pub fn set_query(&mut self, query: &str) {
        if query == self.query {
            return;
        }
        let append = query.starts_with(&self.query);
        self.nucleo.pattern.reparse(0, query, CaseMatching::Smart, Normalization::Smart, append);
        self.query = query.to_owned();
    }

    /// Lets nucleo merge new results (waiting at most 2 ms, so a frame is never
    /// held up) and returns the top `max` matches.
    pub fn results(&mut self, max: u32) -> Vec<FileMatch> {
        self.nucleo.tick(2);
        let snapshot = self.nucleo.snapshot();
        let n = snapshot.matched_item_count().min(max);
        let pattern = self.nucleo.pattern.column_pattern(0);
        let mut buf = Vec::new();
        snapshot
            .matched_items(..n)
            .map(|item| {
                let mut indices = Vec::new();
                pattern.indices(Utf32Str::new(&item.data.rel, &mut buf), &mut self.matcher, &mut indices);
                indices.sort_unstable();
                indices.dedup();
                FileMatch { path: item.data.path.clone(), rel: item.data.rel.clone(), indices }
            })
            .collect()
    }

    pub fn is_scanning(&self) -> bool {
        self.walking.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn file_count(&self) -> u32 {
        self.nucleo.snapshot().item_count()
    }

    pub fn has_root(&self) -> bool {
        self.root.is_some()
    }
}

/// Labels that match `query`, best first, each with the indices of its matched chars.
/// Words match in any order. Equal scores keep the labels' order.
pub fn rank<'a>(labels: impl IntoIterator<Item = &'a str>, query: &str) -> Vec<(usize, Vec<usize>)> {
    let query = query.trim();
    if query.is_empty() {
        return labels.into_iter().enumerate().map(|(i, _)| (i, Vec::new())).collect();
    }
    thread_local! {
        // A matcher holds a sizeable scratch space: one per thread, not one per call.
        static MATCHER: std::cell::RefCell<Matcher> = std::cell::RefCell::new(Matcher::new(Config::DEFAULT));
    }
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let (mut buf, mut found) = (Vec::new(), Vec::new());
    MATCHER.with_borrow_mut(|matcher| {
        for (i, label) in labels.into_iter().enumerate() {
            let mut indices = Vec::new();
            if let Some(score) = pattern.indices(Utf32Str::new(label, &mut buf), matcher, &mut indices) {
                indices.sort_unstable();
                indices.dedup();
                found.push((score, i, indices.into_iter().map(|c| c as usize).collect()));
            }
        }
    });
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, i, indices)| (i, indices)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn ranks_the_closest_label_first() {
        let labels = ["Change Indentation", "Go to Definition", "Git: Commit", "Git: Push", "View: Toggle Terminal"];
        let order: Vec<usize> = rank(labels, "git").into_iter().map(|(i, _)| i).collect();
        assert_eq!(&order[..2], [2, 3], "{order:?}");
        let words = rank(labels, "terminal toggle");
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].0, 4);
        assert!(rank(labels, "zzz").is_empty());
        assert_eq!(rank(labels, " ").len(), labels.len());
        assert_eq!(rank(["Git: Commit"], "gc")[0].1, [0, 5]);
    }

    #[test]
    fn indexes_folder_respecting_gitignore_and_ranks_matches() {
        let dir = std::env::temp_dir().join(format!("coiman-finder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        std::fs::create_dir_all(dir.join("ignored")).unwrap();
        std::fs::write(dir.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("src/editor_view.rs"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();
        std::fs::write(dir.join("target/debug/junk.rs"), "").unwrap();
        std::fs::write(dir.join("ignored/secret.rs"), "").unwrap();

        let mut f = FileFinder::new(|| {});
        f.set_root(Some(dir.clone()));
        let start = Instant::now();
        while (f.is_scanning() || f.file_count() < 4) && start.elapsed() < Duration::from_secs(5) {
            f.results(10);
            std::thread::sleep(Duration::from_millis(10));
        }
        f.set_query("edview");
        let mut results = Vec::new();
        for _ in 0..100 {
            results = f.results(10);
            // Under load the first answers can still be from before the query.
            if results.len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(results.len(), 1, "fuzzy match finds editor_view.rs only");
        assert!(results[0].rel.ends_with("editor_view.rs"));
        assert!(!results[0].indices.is_empty());

        f.set_query("");
        // Matching runs on worker threads; poll until every indexed file is listed.
        let mut all: Vec<String> = Vec::new();
        for _ in 0..200 {
            all = f.results(100).into_iter().map(|m| m.rel.replace('\\', "/")).collect();
            if all.len() >= 4 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(all.iter().any(|r| r == ".gitignore"), "hidden files are included");
        assert!(!all.iter().any(|r| r.starts_with("target/")), "target/ is skipped");
        assert!(!all.iter().any(|r| r.starts_with("ignored/")), ".gitignore is honoured");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
