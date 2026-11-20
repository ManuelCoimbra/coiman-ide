//! Watches the open folder and the folders shown in the Explorer, plus open
//! files outside it, for changes made by other programs.
//!
//! Watches are added on a worker thread. On Linux, only for folders the editor has
//! listed (or an open file lives in). Watching a whole tree recursively up front
//! is fast on Windows but on Linux it means an `inotify` watch per directory, so
//! opening a large folder or a home directory used to block the first frame for
//! seconds.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::project::workspace::IGNORED_DIRS;

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

/// What the worker should watch.
enum WatchCmd {
    /// Watch a folder and its children (used for `.git`, which is small).
    Tree(PathBuf),
    /// Watch one folder, not its children.
    Dir(PathBuf),
    Unwatch(PathBuf),
}

pub struct FileWatcher {
    /// The worker owns the OS watcher: adding watches can take a while on a
    /// large tree, and must never hold up a frame.
    commands: Option<Sender<WatchCmd>>,
    rx: Receiver<notify::Result<Event>>,
    root: Option<PathBuf>,
    /// Folders watched one level deep: those the Explorer has listed, and the
    /// parents of open files.
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
        let commands = watcher.map(|watcher| {
            let (commands, worker) = channel();
            std::thread::spawn(move || watch_worker(watcher, worker));
            commands
        });
        Self { commands, rx, root: None, extra_dirs: HashSet::new() }
    }

    /// Watches `root` and its `.git` folder. The root itself is watched one
    /// level deep; folders below it are watched as the Explorer lists them.
    pub fn set_root(&mut self, root: Option<&Path>) {
        if self.root.as_deref() == root {
            return;
        }
        if let Some(old) = self.root.take()
            && let Some(commands) = &self.commands
        {
            let _ = commands.send(WatchCmd::Unwatch(old));
        }
        let Some(root) = root else { return };
        if let Some(commands) = &self.commands {
            // Linux: one inotify watch per folder, so the tree is watched as it's listed.
            // Windows and macOS watch a whole tree in one call, and then miss nothing deep
            // inside it (a checkout, a build writing sources).
            let whole = !cfg!(target_os = "linux");
            let _ = commands.send(if whole {
                WatchCmd::Tree(root.to_path_buf())
            } else {
                WatchCmd::Dir(root.to_path_buf())
            });
            // Git's refs are folders below `.git`; the index and HEAD are watched
            // with it. It is small, unlike the project around it.
            let git = root.join(".git");
            if git.is_dir() {
                let _ = commands.send(WatchCmd::Tree(git));
            }
            // The project's settings are read from `.coiman` (or `.rust-code`).
            for name in [".coiman", ".rust-code"] {
                let dir = root.join(name);
                if dir.is_dir() {
                    let _ = commands.send(WatchCmd::Dir(dir));
                }
            }
        }
        self.root = Some(root.to_path_buf());
        // The root is watched by `Dir` above; remember it so listing it doesn't
        // add the same watch again.
        self.extra_dirs.insert(root.to_path_buf());
    }

    /// Watches one folder (not its children) if it isn't watched already.
    /// The Explorer calls this for every folder it lists, so changes in the
    /// folders on screen are reported without watching the whole tree.
    pub fn watch_dir(&mut self, dir: &Path) {
        // Inside a root watched whole, it's watched already.
        if !cfg!(target_os = "linux") && self.root.as_deref().is_some_and(|r| dir.starts_with(r)) {
            return;
        }
        if !self.extra_dirs.insert(dir.to_path_buf()) {
            return;
        }
        if let Some(commands) = &self.commands {
            let _ = commands.send(WatchCmd::Dir(dir.to_path_buf()));
        }
    }

    /// Makes sure changes to `path` are reported, also outside the open folder.
    pub fn watch_file(&mut self, path: &Path) {
        let Some(dir) = path.parent() else { return };
        self.watch_dir(dir);
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

/// Adds watches as asked. `notify`'s recursive mode walks the whole tree, so it
/// is only used for `.git`; everything else is watched one folder at a time.
fn watch_worker(mut watcher: RecommendedWatcher, commands: Receiver<WatchCmd>) {
    for command in commands {
        let _ = match command {
            WatchCmd::Tree(dir) => watcher.watch(&dir, RecursiveMode::Recursive),
            WatchCmd::Dir(dir) => watcher.watch(&dir, RecursiveMode::NonRecursive),
            WatchCmd::Unwatch(dir) => watcher.unwatch(&dir),
        };
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
        let dir = std::env::temp_dir().join(format!("coiman-watch-{}", std::process::id()));
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
    fn watches_folders_the_explorer_lists() {
        let dir = std::env::temp_dir().join(format!("coiman-watch-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        // Joined a part at a time: "a/b/c.txt" would keep its slashes on Windows, unlike the
        // paths events carry.
        let nested = dir.join("a").join("b");
        let file = nested.join("c.txt");
        std::fs::write(&file, "one").unwrap();

        let mut w = FileWatcher::new(|| {});
        w.set_root(Some(&dir));
        // The Explorer listing a folder is what brings its watch in.
        w.watch_dir(&nested);
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(&file, "two").unwrap();
        assert!(wait_for(&mut w, |c| c.paths.iter().any(|p| same_path(p, &file))), "nested change reported");
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
