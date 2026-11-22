//! What Git prints, read: the log with parents and refs, lanes to draw it in, a commit's files,
//! a branch compared with the current one, worktrees, and an interactive rebase's plan.

/// Commits read for the graph.
pub const LIMIT: usize = 2000;

/// The log format [`parse_log`] reads.
pub const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%P%x1f%an%x1f%ar%x1f%D%x1f%s";

/// The pseudo-refs whose presence says what is half done, in the order asked.
pub const OPERATION_REFS: &str = "MERGE_HEAD\nCHERRY_PICK_HEAD\nREVERT_HEAD\nREBASE_HEAD\n";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub parents: Vec<String>,
    pub author: String,
    /// When, as Git says it ("3 days ago").
    pub when: String,
    /// Branches and tags here, as Git prints them (`HEAD -> main, origin/main, tag: v1`).
    pub refs: String,
    pub subject: String,
}

impl Commit {
    /// The first branch here: not a tag, not a remote's HEAD.
    pub fn branch(&self) -> Option<&str> {
        self.refs
            .split(", ")
            .map(|r| r.trim_start_matches("HEAD -> "))
            .find(|r| !r.is_empty() && *r != "HEAD" && !r.starts_with("tag: ") && !r.ends_with("/HEAD"))
    }

    /// A branch here, or the short hash: what merge, compare and rebase-onto name.
    pub fn target(&self) -> String {
        self.branch().map_or_else(|| self.short.clone(), str::to_owned)
    }

    /// Each branch and tag: (name, is a tag, is the branch checked out).
    pub fn ref_labels(&self) -> Vec<(String, bool, bool)> {
        self.refs
            .split(", ")
            .filter(|r| !r.is_empty())
            .map(|r| {
                let current = r.starts_with("HEAD -> ") || r == "HEAD";
                let tag = r.starts_with("tag: ");
                (r.trim_start_matches("HEAD -> ").trim_start_matches("tag: ").to_owned(), tag, current)
            })
            .collect()
    }

    pub fn head(&self) -> bool {
        self.refs.split(", ").any(|r| r == "HEAD" || r.starts_with("HEAD -> "))
    }
}

/// The commits of `git log` with [`LOG_FORMAT`].
pub fn parse_log(out: &str) -> Vec<Commit> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.splitn(7, '\x1f').map(str::to_owned);
            Some(Commit {
                hash: f.next()?,
                short: f.next()?,
                parents: f.next()?.split_whitespace().map(str::to_owned).collect(),
                author: f.next()?,
                when: f.next()?,
                refs: f.next()?,
                subject: f.next()?,
            })
        })
        .collect()
}

/// How to draw one row: the commit's lane, and which lane leads where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub lane: usize,
    /// Lines in the row's top half: (lane at the top, lane at the middle).
    pub top: Vec<(usize, usize)>,
    /// Lines in the bottom half: (lane at the middle, lane at the bottom).
    pub bottom: Vec<(usize, usize)>,
}

/// Lays the commits (newest first, each after its children) out in lanes.
pub fn lanes(commits: &[Commit]) -> Vec<Row> {
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
        rows.push(Row { lane, top, bottom });
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

    /// The arguments that go on with it once its conflicts are resolved.
    pub fn continue_args(self) -> Vec<String> {
        match self {
            Operation::Merge => vec!["commit".into(), "--no-edit".into()],
            _ => vec![self.name().into(), "--continue".into()],
        }
    }
}

/// What `git cat-file --batch-check` said of [`OPERATION_REFS`]: the first one there.
pub fn operation(out: &str) -> Option<Operation> {
    let ops = [Operation::Merge, Operation::CherryPick, Operation::Revert, Operation::Rebase];
    let found: Vec<bool> = out.lines().map(|l| !l.trim_end().ends_with(" missing")).collect();
    // A rebase is named first: a merge commit replayed in one has MERGE_HEAD too.
    if found.get(3) == Some(&true) {
        return Some(Operation::Rebase);
    }
    ops.into_iter().zip(found).find(|(_, there)| *there).map(|(op, _)| op)
}

/// `--name-status -z` output: (kind, path).
pub fn name_status(out: &str) -> Vec<(char, String)> {
    let mut parts = out.split('\0').map(|p| p.trim_start_matches('\n')).filter(|p| !p.is_empty());
    let mut files = Vec::new();
    while let (Some(kind), Some(rel)) = (parts.next(), parts.next()) {
        files.push((kind.trim().chars().next().unwrap_or('M'), rel.to_owned()));
    }
    files
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Worktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    /// The repository's own folder.
    pub main: bool,
    pub locked: bool,
    /// Its folder is gone.
    pub prunable: bool,
}

/// `git worktree list --porcelain`.
pub fn parse_worktrees(text: &str) -> Vec<Worktree> {
    let mut out: Vec<Worktree> = Vec::new();
    for line in text.lines() {
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key == "worktree" {
            out.push(Worktree { path: value.to_owned(), main: out.is_empty(), ..Default::default() });
            continue;
        }
        let Some(w) = out.last_mut() else { continue };
        match key {
            "HEAD" => w.head = value.to_owned(),
            "branch" => w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_owned()),
            "locked" => w.locked = true,
            "prunable" => w.prunable = true,
            _ => {}
        }
    }
    out
}

/// A folder beside the repository's, named after the branch: `C:/src/app` and `fix/x` give
/// `C:/src/app-fix-x`.
pub fn worktree_folder(main: &str, branch: &str) -> String {
    let safe: String =
        branch.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    if safe.is_empty() {
        return String::new();
    }
    let main = main.trim_end_matches(['/', '\\']);
    format!("{main}-{safe}")
}

/// One commit of an interactive rebase, oldest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub hash: String,
    pub short: String,
    pub subject: String,
    /// pick, reword, squash, fixup or drop.
    pub action: String,
    /// The message, when reworded.
    pub message: String,
}

pub const ACTIONS: [&str; 5] = ["pick", "reword", "squash", "fixup", "drop"];

/// `git log --reverse --format=%H%x1f%h%x1f%s base..HEAD`.
pub fn parse_plan(out: &str) -> Vec<Step> {
    out.lines()
        .filter_map(|line| {
            let mut f = line.splitn(3, '\x1f');
            let (hash, short, subject) = (f.next()?, f.next()?, f.next()?);
            Some(Step {
                hash: hash.into(),
                short: short.into(),
                subject: subject.into(),
                action: "pick".into(),
                message: subject.into(),
            })
        })
        .collect()
}

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The rebase's todo list. A picked commit whose message was changed is reworded; a reword is
/// a pick and an amend, so no editor opens.
pub fn todo(steps: &[Step]) -> Result<String, String> {
    let first = steps.iter().find(|s| s.action != "drop");
    if first.is_some_and(|s| s.action == "squash" || s.action == "fixup") {
        return Err("The first commit can't be squashed or fixed up: there's nothing before it to go into".into());
    }
    let mut out = String::new();
    for step in steps {
        let reword = step.action == "reword" || (step.action == "pick" && step.message.trim() != step.subject.trim());
        if reword && !step.message.trim().is_empty() {
            out.push_str(&format!("pick {}\n", step.hash));
            let message = sh_quote(step.message.trim());
            out.push_str(&format!("exec git commit --amend --only --allow-empty -m {message}\n"));
        } else {
            let action = if step.action == "reword" { "pick" } else { step.action.as_str() };
            out.push_str(&format!("{action} {}\n", step.hash));
        }
    }
    Ok(out)
}

/// `git -c sequence.editor=… rebase -i base`: Git runs the editor as `sh -c '<editor> "$@"'`
/// with the todo file, so the editor writes the plan into it. No file of ours is needed.
pub fn rebase_args(base: &str, todo: &str) -> Vec<String> {
    let editor = format!("sequence.editor=printf '%s' {} >", sh_quote(todo));
    vec!["-c".into(), editor, "rebase".into(), "-i".into(), base.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(hash: &str, parents: &[&str]) -> Commit {
        Commit {
            hash: hash.into(),
            short: hash.into(),
            parents: parents.iter().map(|p| (*p).into()).collect(),
            author: String::new(),
            when: String::new(),
            refs: String::new(),
            subject: String::new(),
        }
    }

    #[test]
    fn a_branch_and_its_merge_take_two_lanes() {
        let commits = [c("m", &["a", "b"]), c("a", &["root"]), c("b", &["root"]), c("root", &[])];
        let rows = lanes(&commits);
        let at: Vec<usize> = rows.iter().map(|r| r.lane).collect();
        assert_eq!(at, [0, 0, 1, 0]);
        assert!(rows[0].bottom.contains(&(0, 1)), "the merge reaches out to the second lane");
        assert!(rows[3].top.contains(&(1, 0)), "the branch comes back in at the root");
        assert!(rows[3].bottom.is_empty(), "nothing below the root");
    }

    #[test]
    fn log_refs_and_operations_are_read() {
        let line = "abc123\x1fabc\x1fp1 p2\x1fAda\x1f2 days ago\x1fHEAD -> main, origin/main, tag: v1\x1fMerge it";
        let commits = parse_log(line);
        let k = &commits[0];
        assert_eq!(k.parents, ["p1", "p2"]);
        assert_eq!(k.branch(), Some("main"));
        assert!(k.head());
        let labels = k.ref_labels();
        assert_eq!(labels[0], ("main".to_owned(), false, true));
        assert_eq!(labels[2], ("v1".to_owned(), true, false));
        let none = "MERGE_HEAD missing\nCHERRY_PICK_HEAD missing\nREVERT_HEAD missing\nREBASE_HEAD missing\n";
        assert_eq!(operation(none), None);
        let pick = "MERGE_HEAD missing\nabc commit 200\nREVERT_HEAD missing\nREBASE_HEAD missing\n";
        assert_eq!(operation(pick), Some(Operation::CherryPick));
        let rebase = "abc commit 1\nCHERRY_PICK_HEAD missing\nREVERT_HEAD missing\ndef commit 2\n";
        assert_eq!(operation(rebase), Some(Operation::Rebase));
    }

    #[test]
    fn worktrees_files_and_folders() {
        let text = "worktree C:/src/app\nHEAD 1111\nbranch refs/heads/main\n\nworktree C:/src/app-fix\nHEAD 2222\nbranch refs/heads/fix\nlocked\n\nworktree C:/src/old\nHEAD 3333\ndetached\nprunable gitdir file points to non-existent location\n";
        let w = parse_worktrees(text);
        assert_eq!(w.len(), 3);
        assert!(w[0].main && !w[1].main);
        assert_eq!((w[1].branch.as_deref(), w[1].locked), (Some("fix"), true));
        assert_eq!((w[2].branch.as_deref(), w[2].prunable), (None, true));
        assert_eq!(worktree_folder("C:/src/app/", "fix/x y"), "C:/src/app-fix-x-y");
        assert_eq!(name_status("M\0a.rs\0A\0b c.rs\0"), [('M', "a.rs".into()), ('A', "b c.rs".into())]);
    }

    #[test]
    fn the_plan_becomes_a_todo() {
        let mut steps = parse_plan("h1\x1fs1\x1fFirst\nh2\x1fs2\x1fIt's second\nh3\x1fs3\x1fThird\n");
        steps[1].message = "Second, it's".into();
        steps[2].action = "fixup".into();
        let todo = todo(&steps).unwrap();
        assert_eq!(
            todo,
            "pick h1\npick h2\nexec git commit --amend --only --allow-empty -m 'Second, it'\\''s'\nfixup h3\n"
        );
        steps[0].action = "squash".into();
        assert!(super::todo(&steps).is_err());
        let args = rebase_args("base", "pick h1\n");
        assert_eq!(args[1], "sequence.editor=printf '%s' 'pick h1\n' >");
    }
}
