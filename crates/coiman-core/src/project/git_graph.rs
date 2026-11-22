//! Merge conflicts: the operation in progress (merge, cherry-pick, revert, rebase), the conflict
//! blocks in a file and how each is settled, and going on with the operation or giving it up.
//! The commit graph itself is the Git Graph extension.

use std::path::PathBuf;

use crate::project::git::Repo;

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

impl Repo {
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

    /// Marks a file's conflicts as resolved: it's staged as it is now.
    pub fn mark_resolved(&self, rel: &str) -> Result<(), String> {
        self.run(&["add", "--", rel]).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    /// Against the real `git`: a cherry-pick that conflicts, read as in progress, its file
    /// settled and staged, then continued.
    #[test]
    fn an_operation_in_progress_with_real_git() {
        let dir = std::env::temp_dir().join(format!("coiman-conflicts-{}", std::process::id()));
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
        assert_eq!(repo.operation(), None);
        assert!(repo.run(&["cherry-pick", &feature]).is_err());
        assert_eq!(repo.operation(), Some(Operation::CherryPick));
        let text = std::fs::read_to_string(dir.join("a.txt")).unwrap();
        std::fs::write(dir.join("a.txt"), resolve(&text, 0, Choice::Both).unwrap()).unwrap();
        repo.mark_resolved("a.txt").unwrap();
        repo.continue_operation(Operation::CherryPick).unwrap();
        assert_eq!(repo.operation(), None);
        // Git for Windows may check files out with CRLF.
        let merged = std::fs::read_to_string(dir.join("a.txt")).unwrap().replace("\r\n", "\n");
        assert_eq!(merged, "main\nfeature\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
