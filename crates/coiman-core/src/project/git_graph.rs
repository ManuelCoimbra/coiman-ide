//! The commit graph and what's done from it: lanes for drawing branches and merges,
//! cherry-pick and revert, the operation in progress (merge, cherry-pick, revert, rebase),
//! conflict blocks in a file, and interactive rebase from a plan.

use std::path::PathBuf;

use crate::project::git::Repo;

/// One commit, with its parents, for the graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCommit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Unix time.
    pub time: i64,
    /// Branches and tags pointing here, as git prints them (`HEAD -> main, origin/main`).
    pub refs: String,
    pub subject: String,
}

/// How to draw one row: the commit's lane, and which lane leads where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub lane: usize,
    /// Lines in the row's top half: (lane at the top, lane at the middle). The commit's own
    /// lane meets its node.
    pub top: Vec<(usize, usize)>,
    /// Lines in the bottom half: (lane at the middle, lane at the bottom).
    pub bottom: Vec<(usize, usize)>,
    /// Lanes in use, for the row's width.
    pub width: usize,
}

/// Lays the commits (newest first, each after its children) out in lanes.
pub fn lanes(commits: &[GraphCommit]) -> Vec<Row> {
    // What each lane waits for: the hash of the next commit drawn in it.
    let mut active: Vec<Option<&str>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let hash = commit.hash.as_str();
        let lane = match active.iter().position(|h| *h == Some(hash)) {
            Some(l) => l,
            None => match active.iter().position(Option::is_none) {
                Some(free) => free,
                None => {
                    active.push(None);
                    active.len() - 1
                }
            },
        };
        let before = active.clone();
        // Lanes that waited for this commit end at its node; the rest go straight down.
        for slot in active.iter_mut() {
            if *slot == Some(hash) {
                *slot = None;
            }
        }
        active[lane] = commit.parents.first().map(String::as_str);
        let mut extra = Vec::new();
        for parent in commit.parents.iter().skip(1) {
            let at = match active.iter().position(|h| *h == Some(parent.as_str())) {
                Some(at) => at,
                None => match active.iter().position(Option::is_none) {
                    Some(free) => {
                        active[free] = Some(parent.as_str());
                        free
                    }
                    None => {
                        active.push(Some(parent.as_str()));
                        active.len() - 1
                    }
                },
            };
            extra.push(at);
        }
        let mut top = Vec::new();
        let mut bottom = Vec::new();
        for (i, h) in before.iter().enumerate() {
            match h {
                Some(h) if *h == hash => top.push((i, lane)),
                Some(h) => {
                    // Still waiting: on through the row, in the same lane.
                    if let Some(j) = active.iter().position(|a| *a == Some(*h)) {
                        top.push((i, j));
                        if j != lane || i != lane {
                            bottom.push((j, j));
                        }
                    }
                }
                None => {}
            }
        }
        if !commit.parents.is_empty() {
            bottom.push((lane, lane));
        }
        for at in extra {
            bottom.push((lane, at));
        }
        // Lanes nobody waits for at the right are dropped.
        while active.last().is_some_and(Option::is_none) {
            active.pop();
        }
        let width = before.len().max(active.len()).max(lane + 1);
        rows.push(Row { lane, top, bottom, width });
    }
    rows
}

/// What's half done in the repository, waiting for conflicts to be resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Merge,
    CherryPick,
    Revert,
    Rebase,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
            Operation::Rebase => "rebase",
        }
    }
}

/// One conflict in a file: what the two sides (and, with diff3, their common base) have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Lines from `<<<<<<<` to `>>>>>>>`, 0-based, end exclusive.
    pub start: usize,
    pub end: usize,
    pub ours_label: String,
    pub theirs_label: String,
    pub ours: Vec<String>,
    pub base: Option<Vec<String>>,
    pub theirs: Vec<String>,
}

/// How a conflict is settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Ours,
    Theirs,
    /// Ours, then theirs.
    Both,
    /// Theirs, then ours.
    BothTheirsFirst,
}

/// The conflicts marked in a file's text.
pub fn conflicts(text: &str) -> Vec<Conflict> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(label) = lines[i].strip_prefix("<<<<<<<") else {
            i += 1;
            continue;
        };
        let start = i;
        let (mut ours, mut base, mut theirs) = (Vec::new(), None::<Vec<String>>, Vec::new());
        let mut part = 0; // 0 ours, 1 base, 2 theirs
        let mut theirs_label = String::new();
        let mut j = i + 1;
        let mut closed = false;
        while j < lines.len() {
            let line = lines[j];
            if line.starts_with("|||||||") && part == 0 {
                part = 1;
                base = Some(Vec::new());
            } else if line.starts_with("=======") && part < 2 {
                part = 2;
            } else if let Some(l) = line.strip_prefix(">>>>>>>").filter(|_| part == 2) {
                theirs_label = l.trim().to_owned();
                closed = true;
                break;
            } else {
                let line = line.to_owned();
                match part {
                    0 => ours.push(line),
                    1 => base.get_or_insert_default().push(line),
                    _ => theirs.push(line),
                }
            }
            j += 1;
        }
        if !closed {
            break;
        }
        out.push(Conflict { start, end: j + 1, ours_label: label.trim().to_owned(), theirs_label, ours, base, theirs });
        i = j + 1;
    }
    out
}

/// The lines a choice keeps for a conflict.
pub fn chosen(c: &Conflict, choice: Choice) -> Vec<String> {
    match choice {
        Choice::Ours => c.ours.clone(),
        Choice::Theirs => c.theirs.clone(),
        Choice::Both => c.ours.iter().chain(&c.theirs).cloned().collect(),
        Choice::BothTheirsFirst => c.theirs.iter().chain(&c.ours).cloned().collect(),
    }
}

/// The text with conflict `index` settled by `choice`, keeping the file's line endings.
pub fn resolve(text: &str, index: usize, choice: Choice) -> Option<String> {
    let c = conflicts(text).into_iter().nth(index)?;
    resolve_with(text, index, &chosen(&c, choice))
}

/// The text with conflict `index` replaced by `kept`: lines of one's own, as the result pane
/// edits them.
pub fn resolve_with(text: &str, index: usize, kept: &[String]) -> Option<String> {
    let found = conflicts(text);
    let c = found.get(index)?;
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let lines: Vec<&str> = text.lines().collect();
    let kept: Vec<&str> = kept.iter().map(String::as_str).collect();
    let mut out: Vec<&str> = lines[..c.start].to_vec();
    out.extend(kept);
    out.extend(&lines[c.end..]);
    let mut joined = out.join(nl);
    if text.ends_with('\n') {
        joined.push_str(nl);
    }
    Some(joined)
}

/// One step of an interactive rebase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Pick,
    /// Pick with a new message.
    Reword(String),
    /// Melt into the commit before, keeping both messages.
    Squash,
    /// Melt into the commit before, keeping its message.
    Fixup,
    Drop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub action: Action,
    pub hash: String,
    pub short: String,
    pub subject: String,
}

/// Quoted for `sh`, where git runs `exec` lines.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The todo list git reads, from a plan. A reword is a pick then an amend of its message.
pub fn todo(steps: &[Step]) -> String {
    let mut out = String::new();
    for step in steps {
        match &step.action {
            Action::Pick => out.push_str(&format!("pick {}\n", step.hash)),
            Action::Squash => out.push_str(&format!("squash {}\n", step.hash)),
            Action::Fixup => out.push_str(&format!("fixup {}\n", step.hash)),
            Action::Drop => out.push_str(&format!("drop {}\n", step.hash)),
            Action::Reword(message) => {
                out.push_str(&format!("pick {}\n", step.hash));
                out.push_str(&format!("exec git commit --amend --only --allow-empty -m {}\n", sh_quote(message)));
            }
        }
    }
    out
}

/// A working tree of the repository: the main one, or one added with `git worktree add`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: std::path::PathBuf,
    pub head: String,
    /// Its branch (`main`); `None` when it's on a commit of its own (detached).
    pub branch: Option<String>,
    /// The repository's own folder.
    pub main: bool,
    pub locked: bool,
    /// Its folder is gone: `git worktree prune` would drop it.
    pub prunable: bool,
}

/// `git worktree list --porcelain`: a block per working tree, the main one first.
pub fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "worktree" => out.push(Worktree {
                path: std::path::PathBuf::from(value),
                head: String::new(),
                branch: None,
                main: out.is_empty(),
                locked: false,
                prunable: false,
            }),
            "HEAD" => {
                if let Some(w) = out.last_mut() {
                    w.head = value.to_owned();
                }
            }
            "branch" => {
                if let Some(w) = out.last_mut() {
                    w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_owned());
                }
            }
            "locked" => {
                if let Some(w) = out.last_mut() {
                    w.locked = true;
                }
            }
            "prunable" => {
                if let Some(w) = out.last_mut() {
                    w.prunable = true;
                }
            }
            _ => {}
        }
    }
    out
}

impl Repo {
    /// The repository's working trees, its own first.
    pub fn worktrees(&self) -> Result<Vec<Worktree>, String> {
        Ok(parse_worktrees(&self.run(&["worktree", "list", "--porcelain"])?))
    }

    /// A new working tree in `path` on `branch`: a new branch from the current commit when
    /// `create`, else one that exists (git refuses a branch another working tree has).
    pub fn add_worktree(&self, path: &std::path::Path, branch: &str, create: bool) -> Result<(), String> {
        let path = path.to_string_lossy().into_owned();
        let args: Vec<&str> =
            if create { vec!["worktree", "add", "-b", branch, &path] } else { vec!["worktree", "add", &path, branch] };
        self.run(&args).map(drop)
    }

    /// Removes a working tree and its folder. Without `force`, git refuses one with changes.
    pub fn remove_worktree(&self, path: &std::path::Path, force: bool) -> Result<(), String> {
        let path = path.to_string_lossy().into_owned();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path);
        self.run(&args).map(drop)
    }

    /// Forgets working trees whose folders are gone.
    pub fn prune_worktrees(&self) -> Result<(), String> {
        self.run(&["worktree", "prune"]).map(drop)
    }

    /// Commits of every branch, newest first and each after its children, for the graph.
    pub fn graph_log(&self, limit: usize) -> Result<Vec<GraphCommit>, String> {
        if self.run(&["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
            return Ok(Vec::new());
        }
        let count = format!("-n{limit}");
        let out =
            self.run(&["log", "--all", "--topo-order", &count, "--format=%H%x1f%h%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut f = line.splitn(7, '\x1f').map(str::to_owned);
                Some(GraphCommit {
                    hash: f.next()?,
                    short: f.next()?,
                    parents: f.next()?.split_whitespace().map(str::to_owned).collect(),
                    author: f.next()?,
                    time: f.next()?.parse().ok()?,
                    refs: f.next()?,
                    subject: f.next()?,
                })
            })
            .collect())
    }

    fn git_dir(&self) -> Option<PathBuf> {
        let dir = self.run(&["rev-parse", "--absolute-git-dir"]).ok()?;
        Some(PathBuf::from(dir.trim()))
    }

    /// The merge, cherry-pick, revert or rebase in progress, if any.
    pub fn operation(&self) -> Option<Operation> {
        let dir = self.git_dir()?;
        if dir.join("rebase-merge").is_dir() || dir.join("rebase-apply").is_dir() {
            Some(Operation::Rebase)
        } else if dir.join("CHERRY_PICK_HEAD").is_file() {
            Some(Operation::CherryPick)
        } else if dir.join("REVERT_HEAD").is_file() {
            Some(Operation::Revert)
        } else if dir.join("MERGE_HEAD").is_file() {
            Some(Operation::Merge)
        } else {
            None
        }
    }

    /// Goes on with the operation once conflicts are resolved and staged, keeping the
    /// messages git prepared.
    pub fn continue_operation(&self, op: Operation) -> Result<(), String> {
        let args: &[&str] = match op {
            Operation::Merge => &["commit", "--no-edit"],
            _ => &[op.name(), "--continue"],
        };
        self.run_env(args, &[("GIT_EDITOR", "true")]).map(drop)
    }

    /// Gives the operation up, back to how things were before it.
    pub fn abort_operation(&self, op: Operation) -> Result<(), String> {
        self.run(&[op.name(), "--abort"]).map(drop)
    }

    /// Applies a commit's change on top of the current branch.
    pub fn cherry_pick(&self, hash: &str) -> Result<(), String> {
        self.run(&["cherry-pick", hash]).map(drop)
    }

    /// Makes a commit that undoes another, with git's own message.
    pub fn revert(&self, hash: &str) -> Result<(), String> {
        self.run_env(&["revert", "--no-edit", hash], &[("GIT_EDITOR", "true")]).map(drop)
    }

    /// A new branch at a commit (not switched to).
    pub fn branch_at(&self, name: &str, hash: &str) -> Result<(), String> {
        self.run(&["branch", "--", name, hash]).map(drop)
    }

    /// Marks a file's conflicts as resolved: it's staged as it is now.
    pub fn mark_resolved(&self, rel: &str) -> Result<(), String> {
        self.run(&["add", "--", rel]).map(drop)
    }

    /// The commits after `base` up to HEAD, oldest first: what a rebase onto `base` replays.
    pub fn rebase_plan(&self, base: &str) -> Result<Vec<Step>, String> {
        let range = format!("{base}..HEAD");
        let out = self.run(&["log", "--reverse", "--no-merges", "--format=%H%x1f%h%x1f%s", &range])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut f = line.splitn(3, '\x1f').map(str::to_owned);
                Some(Step { action: Action::Pick, hash: f.next()?, short: f.next()?, subject: f.next()? })
            })
            .collect())
    }

    /// Replays the commits after `base` as the plan says: reordered, reworded, squashed,
    /// fixed up or dropped. Stops at a conflict, as git does.
    pub fn rebase(&self, base: &str, steps: &[Step]) -> Result<(), String> {
        if steps.first().is_some_and(|s| matches!(s.action, Action::Squash | Action::Fixup)) {
            return Err("The first commit can't be squashed or fixed up: there's nothing before it to go into".into());
        }
        let file = std::env::temp_dir().join(format!("coiman-rebase-todo-{}.txt", std::process::id()));
        std::fs::write(&file, todo(steps)).map_err(|e| e.to_string())?;
        // Git starts the sequence editor through its own `sh`, with the todo file's path after.
        let path = file.display().to_string().replace('\\', "/");
        let editor = format!("cp {}", sh_quote(&path));
        let result = self
            .run_env(&["rebase", "-i", base], &[("GIT_SEQUENCE_EDITOR", &editor), ("GIT_EDITOR", "true")])
            .map(drop);
        let _ = std::fs::remove_file(&file);
        result
    }

    /// Merges `branch` into the current one, with git's own message. Stops at conflicts.
    pub fn merge(&self, branch: &str) -> Result<(), String> {
        self.run_env(&["merge", "--no-edit", branch], &[("GIT_EDITOR", "true")]).map(drop)
    }

    /// Replays the current branch's own commits on top of `onto`. Stops at a conflict.
    pub fn rebase_onto(&self, onto: &str) -> Result<(), String> {
        self.run_env(&["rebase", onto], &[("GIT_EDITOR", "true")]).map(drop)
    }

    /// How `other` differs from HEAD: the commits each has that the other hasn't, and the
    /// files `other` changed since the two parted.
    pub fn compare(&self, other: &str) -> Result<Comparison, String> {
        let base = self.run(&["merge-base", "HEAD", other])?.trim().to_owned();
        let side = |range: String| -> Result<Vec<GraphCommit>, String> {
            let out = self.run(&["log", &range, "--format=%H%x1f%h%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s"])?;
            Ok(out
                .lines()
                .filter_map(|line| {
                    let mut f = line.splitn(7, '\x1f').map(str::to_owned);
                    Some(GraphCommit {
                        hash: f.next()?,
                        short: f.next()?,
                        parents: f.next()?.split_whitespace().map(str::to_owned).collect(),
                        author: f.next()?,
                        time: f.next()?.parse().ok()?,
                        refs: f.next()?,
                        subject: f.next()?,
                    })
                })
                .collect())
        };
        let files = self.run(&["diff", "--name-status", "--no-renames", "-z", &format!("{base}..{other}")])?;
        let mut parts = files.split('\0').filter(|p| !p.is_empty());
        let mut changed = Vec::new();
        while let (Some(kind), Some(rel)) = (parts.next(), parts.next()) {
            changed.push((kind.trim().chars().next().unwrap_or('M'), rel.to_owned()));
        }
        Ok(Comparison {
            base,
            theirs: side(format!("HEAD..{other}"))?,
            ours: side(format!("{other}..HEAD"))?,
            files: changed,
        })
    }
}

/// How a branch differs from the current one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Comparison {
    /// Where the two parted.
    pub base: String,
    /// Commits only on the other branch.
    pub theirs: Vec<GraphCommit>,
    /// Commits only on the current branch.
    pub ours: Vec<GraphCommit>,
    /// Files the other branch changed since `base`: (kind, path).
    pub files: Vec<(char, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(hash: &str, parents: &[&str]) -> GraphCommit {
        GraphCommit {
            hash: hash.into(),
            short: hash.into(),
            parents: parents.iter().map(|p| (*p).into()).collect(),
            author: String::new(),
            time: 0,
            refs: String::new(),
            subject: String::new(),
        }
    }

    #[test]
    fn worktree_list_is_read() {
        let text = "worktree C:/src/app\nHEAD 1111\nbranch refs/heads/main\n\nworktree C:/src/app-fix\nHEAD 2222\nbranch refs/heads/fix\nlocked\n\nworktree C:/src/old\nHEAD 3333\ndetached\nprunable gitdir file points to non-existent location\n";
        let w = parse_worktrees(text);
        assert_eq!(w.len(), 3);
        assert!(w[0].main && !w[1].main);
        assert_eq!(w[0].branch.as_deref(), Some("main"));
        assert_eq!((w[1].branch.as_deref(), w[1].locked), (Some("fix"), true));
        assert_eq!((w[2].branch.as_deref(), w[2].prunable, w[2].head.as_str()), (None, true, "3333"));
    }

    /// Against the real `git`: a worktree on a new branch, listed, then removed.
    #[test]
    fn worktrees_with_real_git() {
        let dir = std::env::temp_dir().join(format!("coiman-worktree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("repo")).unwrap();
        let repo = crate::project::git::init(&dir.join("repo"), "main").unwrap();
        repo.set_identity("Test", "test@example.com").unwrap();
        std::fs::write(dir.join("repo/a.txt"), "a\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("first").unwrap();
        let other = dir.join("repo-feature");
        repo.add_worktree(&other, "feature", true).unwrap();
        assert!(other.join("a.txt").is_file(), "the new tree has the files");
        let list = repo.worktrees().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].branch.as_deref(), Some("feature"));
        assert!(repo.add_worktree(&dir.join("again"), "feature", false).is_err(), "a branch in one tree only");
        std::fs::write(other.join("b.txt"), "b\n").unwrap();
        assert!(repo.remove_worktree(&other, false).is_err(), "changes keep it");
        repo.remove_worktree(&other, true).unwrap();
        assert!(!other.exists());
        assert_eq!(repo.worktrees().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_branch_and_its_merge_take_two_lanes() {
        // m merges b into a; both come from root.
        let commits = [c("m", &["a", "b"]), c("a", &["root"]), c("b", &["root"]), c("root", &[])];
        let rows = lanes(&commits);
        let at: Vec<usize> = rows.iter().map(|r| r.lane).collect();
        assert_eq!(at, [0, 0, 1, 0]);
        assert!(rows[0].bottom.contains(&(0, 1)), "the merge reaches out to the second lane");
        assert!(rows[3].top.contains(&(1, 0)), "the branch comes back in at the root");
        assert_eq!(rows[1].width, 2);
        assert!(rows[3].bottom.is_empty(), "nothing below the root");
    }

    #[test]
    fn conflicts_are_found_and_settled() {
        let text = "a\n<<<<<<< HEAD\nmine\n||||||| base\nold\n=======\ntheirs 1\ntheirs 2\n>>>>>>> feature\nz\n";
        let found = conflicts(text);
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!((c.start, c.end, c.ours_label.as_str(), c.theirs_label.as_str()), (1, 9, "HEAD", "feature"));
        assert_eq!((c.ours.clone(), c.base.clone()), (vec!["mine".to_owned()], Some(vec!["old".to_owned()])));
        assert_eq!(resolve(text, 0, Choice::Ours).unwrap(), "a\nmine\nz\n");
        assert_eq!(resolve(text, 0, Choice::Theirs).unwrap(), "a\ntheirs 1\ntheirs 2\nz\n");
        assert_eq!(resolve(text, 0, Choice::Both).unwrap(), "a\nmine\ntheirs 1\ntheirs 2\nz\n");
        assert_eq!(resolve_with(text, 0, &["my own".to_owned()]).unwrap(), "a\nmy own\nz\n");
        assert_eq!(
            resolve("x\r\n<<<<<<< a\r\n1\r\n=======\r\n2\r\n>>>>>>> b\r\n", 0, Choice::Theirs).unwrap(),
            "x\r\n2\r\n"
        );
        assert!(conflicts("<<<<<<< never closed\nx\n").is_empty());
        let reword = Step {
            action: Action::Reword("it's new".into()),
            hash: "abc".into(),
            short: "abc".into(),
            subject: String::new(),
        };
        assert_eq!(todo(&[reword]), "pick abc\nexec git commit --amend --only --allow-empty -m 'it'\\''s new'\n");
    }

    /// Against the real `git`: cherry-pick with a conflict, its resolution, revert, and an
    /// interactive rebase that reorders, rewords, squashes and drops.
    #[test]
    fn graph_cherry_pick_conflicts_and_rebase_with_real_git() {
        let dir = std::env::temp_dir().join(format!("coiman-git-graph-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = crate::project::git::init(&dir, "main").unwrap();
        repo.set_identity("Test", "test@example.com").unwrap();
        let commit = |file: &str, text: &str, msg: &str| {
            std::fs::write(dir.join(file), text).unwrap();
            repo.stage(&["."]).unwrap();
            repo.commit(msg).unwrap();
        };
        commit("a.txt", "one\n", "first");
        repo.switch("feature", true).unwrap();
        commit("a.txt", "feature\n", "on feature");
        let feature = repo.run(&["rev-parse", "HEAD"]).unwrap().trim().to_owned();
        repo.switch("main", false).unwrap();
        commit("a.txt", "main\n", "on main");

        let graph = repo.graph_log(50).unwrap();
        assert_eq!(graph.len(), 3);
        assert!(graph.iter().any(|g| g.refs.contains("feature")));
        assert_eq!(lanes(&graph).iter().map(|r| r.width).max(), Some(2), "two branches, two lanes");

        // Cherry-picking the feature's change conflicts with main's.
        assert!(repo.cherry_pick(&feature).is_err());
        assert_eq!(repo.operation(), Some(Operation::CherryPick));
        let text = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        assert_eq!(conflicts(&text).len(), 1);
        std::fs::write(dir.join("a.txt"), resolve(&text, 0, Choice::Both).unwrap()).unwrap();
        repo.mark_resolved("a.txt").unwrap();
        repo.continue_operation(Operation::CherryPick).unwrap();
        assert_eq!(repo.operation(), None);
        // Git for Windows may check files out with CRLF.
        let merged = std::fs::read_to_string(dir.join("a.txt")).unwrap().replace("\r\n", "\n");
        assert_eq!(merged, "main\nfeature\n");

        // Abort brings things back.
        commit("b.txt", "1\n", "b one");
        repo.revert("HEAD").unwrap();
        assert!(!dir.join("b.txt").exists(), "the revert undid it");

        // Rebase: four commits after a base, reordered, reworded, squashed and dropped.
        let base = repo.run(&["rev-parse", "HEAD"]).unwrap().trim().to_owned();
        commit("c.txt", "c\n", "add c");
        commit("d.txt", "d\n", "add d");
        commit("e.txt", "e\n", "add e");
        commit("f.txt", "f\n", "add f");
        let mut plan = repo.rebase_plan(&base).unwrap();
        assert_eq!(plan.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(), ["add c", "add d", "add e", "add f"]);
        plan.swap(0, 1); // d, c, e, f
        plan[0].action = Action::Reword("add d (it's reworded)".into());
        plan[2].action = Action::Squash; // e into c
        plan[3].action = Action::Drop; // no f
        repo.rebase(&base, &plan).unwrap();
        let log = repo.log(10).unwrap();
        let subjects: Vec<&str> = log.iter().take(2).map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, ["add c", "add d (it's reworded)"]);
        assert!(dir.join("e.txt").exists() && !dir.join("f.txt").exists());
        assert!(repo.rebase(&base, &[Step { action: Action::Squash, ..plan[0].clone() }]).is_err());

        // A branch compared, then merged; another rebased onto main.
        let main_tip = repo.run(&["rev-parse", "HEAD"]).unwrap().trim().to_owned();
        repo.switch("side", true).unwrap();
        commit("side.txt", "s\n", "side work");
        repo.switch("main", false).unwrap();
        commit("main2.txt", "m\n", "main work");
        let cmp = repo.compare("side").unwrap();
        assert_eq!(cmp.theirs.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["side work"]);
        assert_eq!(cmp.ours.iter().map(|c| c.subject.as_str()).collect::<Vec<_>>(), ["main work"]);
        assert_eq!(cmp.files, [('A', "side.txt".to_owned())]);
        assert_eq!(cmp.base, main_tip);
        repo.merge("side").unwrap();
        assert!(dir.join("side.txt").exists());
        repo.switch("later", true).unwrap();
        commit("later.txt", "l\n", "later work");
        repo.switch("main", false).unwrap();
        commit("main3.txt", "m3\n", "more main");
        repo.switch("later", false).unwrap();
        repo.rebase_onto("main").unwrap();
        assert!(dir.join("main3.txt").exists() && dir.join("later.txt").exists(), "later now sits on main");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
