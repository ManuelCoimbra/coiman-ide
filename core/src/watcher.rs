//! Watches the open folder (recursively) and open files outside it for
//! changes made by other programs.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::workspace::IGNORED_DIRS;

/// What changed since the last [`FileWatcher::drain`].
#[derive(Default, Debug)]
pub struct Changes {
    /// Files whose content may have changed (or that were created/deleted).
    pub paths: HashSet<PathBuf>,
    /// Files or folders were created, deleted or renamed.
    pub structure: bool,
    /// Git's index, HEAD or refs changed (a commit, checkout or stage made
    /// outside the editor).
    pub git: bool,
}

pub struct FileWatcher {
    watcher: Option<RecommendedWatcher>,
    rx: Receiver<notify::Result<Event>>,
    root: Option<PathBuf>,
    /// Folders watched non-recursively for open files outside the root.
    extra_dirs: HashSet<PathBuf>,
}

impl FileWatcher {
    /// Starts the OS watcher; `notify` is called on every event.
    pub fn new(notify: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
            notify();
        })
        .ok();
        Self { watcher, rx, root: None, extra_dirs: HashSet::new() }
    }

    pub fn set_root(&mut self, root: Option<&Path>) {
        if self.root.as_deref() == root {
            return;
        }
        let Some(w) = &mut self.watcher else { return };
        if let Some(old) = self.root.take() {
            let _ = w.unwatch(&old);
        }
        if let Some(root) = root
            && w.watch(root, RecursiveMode::Recursive).is_ok()
        {
            self.root = Some(root.to_path_buf());
        }
    }

    /// Makes sure changes to `path` are reported, also outside the open folder.
    pub fn watch_file(&mut self, path: &Path) {
        if self.root.as_deref().is_some_and(|r| path.starts_with(r)) {
            return;
        }
        let Some(dir) = path.parent() else { return };
        if self.extra_dirs.contains(dir) {
            return;
        }
        if let Some(w) = &mut self.watcher
            && w.watch(dir, RecursiveMode::NonRecursive).is_ok()
        {
            self.extra_dirs.insert(dir.to_path_buf());
        }
    }

    /// Collects events since the last call. Changes inside ignored folders
    /// (`target`, `.git`, …) are dropped so builds don't cause churn.
    pub fn drain(&mut self) -> Changes {
        let mut changes = Changes::default();
        for event in self.rx.try_iter().filter_map(Result::ok) {
            if !matches!(event.kind, EventKind::Access(_)) && event.paths.iter().any(|p| is_git_state(p)) {
                changes.git = true;
            }
            let paths: Vec<PathBuf> = event.paths.into_iter().filter(|p| !in_ignored_dir(p)).collect();
            if paths.is_empty() {
                continue;
            }
            match event.kind {
                EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) => {
                    changes.structure = true;
                }
                EventKind::Access(_) => continue,
                _ => {}
            }
            changes.paths.extend(paths);
        }
        changes
    }
}

/// `.git/index`, `.git/HEAD` or a file under `.git/refs`.
fn is_git_state(path: &Path) -> bool {
    let mut parts = path.components().map(|c| c.as_os_str().to_string_lossy());
    let Some(_) = parts.by_ref().find(|c| c == ".git") else { return false };
    matches!(parts.next().as_deref(), Some("index" | "HEAD" | "refs"))
}

fn in_ignored_dir(path: &Path) -> bool {
    path.components().any(|c| IGNORED_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
}

/// Path equality that ignores case on Windows, where the file system does.
pub fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_for(w: &mut FileWatcher, pred: impl Fn(&Changes) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut all = Changes::default();
        while Instant::now() < deadline {
            let c = w.drain();
            all.structure |= c.structure;
            all.paths.extend(c.paths);
            if pred(&all) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn reports_modify_create_and_ignores_target() {
        let dir = std::env::temp_dir().join(format!("rust-code-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target")).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "one").unwrap();

        let mut w = FileWatcher::new(|| {});
        w.set_root(Some(&dir));
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&file, "two").unwrap();
        assert!(wait_for(&mut w, |c| c.paths.iter().any(|p| same_path(p, &file))), "modify reported");

        std::fs::write(dir.join("target/build.o"), "x").unwrap();
        std::fs::write(dir.join("b.txt"), "new").unwrap();
        let mut seen = Changes::default();
        assert!(wait_for(&mut w, |c| { c.structure && c.paths.iter().any(|p| p.ends_with("b.txt")) }));
        seen.paths.extend(w.drain().paths);
        assert!(!seen.paths.iter().any(|p| in_ignored_dir(p)), "target/ events dropped");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ignored_dir_and_path_compare() {
        assert!(in_ignored_dir(Path::new("C:/p/target/debug/x")));
        assert!(!in_ignored_dir(Path::new("C:/p/src/targets.rs")));
        if cfg!(windows) {
            assert!(same_path(Path::new("C:\\A\\b.rs"), Path::new("c:\\a\\B.rs")));
        }
    }
}
