//! Git through the `git` command line, as VS Code does: status, file contents
//! at HEAD and in the index, line diffs, staging single hunks by writing index
//! blobs, blame, commit and branches. Fetch, pull and push are the only commands
//! that reach the network, and only run when asked for.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp};

use crate::lsp::discovery::quiet_command;

/// A repository, by its top-level folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
}

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    /// Path relative to the repository root, with `/` separators.
    pub rel: String,
    /// `M` modified, `A` added, `D` deleted, `R` renamed, `U` untracked, `!` conflict.
    pub kind: char,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
    pub conflicts: Vec<FileChange>,
}

/// A changed run of lines: `old` lines were replaced by `new` lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// Who last changed a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub author: String,
    /// Unix time of the commit.
    pub time: i64,
    pub summary: String,
    pub uncommitted: bool,
}

fn native(path: &str) -> PathBuf {
    if cfg!(windows) { PathBuf::from(path.replace('/', "\\")) } else { PathBuf::from(path) }
}

/// Runs git in `dir`. Errors carry git's own message.
fn git(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    // A repository's config can run programs (core.fsmonitor, hooks, diff drivers).
    crate::trust::check(dir)?;
    let mut cmd = quiet_command(Path::new("git"));
    cmd.current_dir(dir)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not run git: {e}"))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        let input = input.to_vec();
        // Write from another thread so a large input can't deadlock with git's output.
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = pipe.write_all(&input);
        });
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" ");
        Err(if msg.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { msg })
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

impl Repo {
    /// The repository containing `dir`, if any (and if git is installed).
    pub fn discover(dir: &Path) -> Option<Repo> {
        let out = git(dir, &["rev-parse", "--show-toplevel"], None).ok()?;
        let root = text(out).trim().to_owned();
        (!root.is_empty()).then(|| Repo { root: native(&root) })
    }

    fn run(&self, args: &[&str]) -> Result<String, String> {
        git(&self.root, args, None).map(text)
    }

    /// Path of `path` relative to the root, with `/` separators.
    pub fn rel(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok().or_else(|| {
            // Case-insensitive match on Windows.
            let (p, r) = (path.to_string_lossy().to_lowercase(), self.root.to_string_lossy().to_lowercase());
            let tail = path.to_str()?.get(r.len()..)?;
            p.starts_with(&r).then(|| Path::new(tail))
        })?;
        Some(rel.to_string_lossy().trim_start_matches(['\\', '/']).replace('\\', "/"))
    }

    pub fn status(&self) -> Result<Status, String> {
        let out = self.run(&["status", "--porcelain=v1", "-z", "-b", "--untracked-files=all"])?;
        Ok(parse_status(&self.root, &out))
    }

    /// File contents in the index (`stage` true) or at HEAD; `None` if absent.
    pub fn show(&self, rel: &str, stage: bool) -> Option<String> {
        let spec = if stage { format!(":{rel}") } else { format!("HEAD:{rel}") };
        git(&self.root, &["show", &spec], None).ok().map(text)
    }

    pub fn stage(&self, rels: &[&str]) -> Result<(), String> {
        let mut args = vec!["add", "-A", "--"];
        args.extend(rels);
        self.run(&args).map(drop)
    }

    pub fn unstage(&self, rels: &[&str]) -> Result<(), String> {
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(rels);
        self.run(&args).map(drop).or_else(|_| {
            // No commits yet: there is no HEAD to restore from.
            let mut args = vec!["rm", "--cached", "-r", "-q", "--"];
            args.extend(rels);
            self.run(&args).map(drop)
        })
    }

    /// Throws away working-tree changes; untracked files are deleted.
    pub fn discard(&self, changes: &[FileChange]) -> Result<(), String> {
        let tracked: Vec<&str> = changes.iter().filter(|c| c.kind != 'U').map(|c| c.rel.as_str()).collect();
        if !tracked.is_empty() {
            let mut args = vec!["restore", "--"];
            args.extend(&tracked);
            self.run(&args)?;
        }
        for c in changes.iter().filter(|c| c.kind == 'U') {
            std::fs::remove_file(&c.path).map_err(|e| format!("{}: {e}", c.rel))?;
        }
        Ok(())
    }

    /// Replaces the index version of `rel` with `content` (stages exactly that text).
    pub fn set_index_content(&self, rel: &str, content: &str) -> Result<(), String> {
        let listed = self.run(&["ls-files", "-s", "--", rel])?;
        let mode = listed.split_whitespace().next().unwrap_or("100644").to_owned();
        let path_arg = format!("--path={rel}");
        let hash = git(&self.root, &["hash-object", "-w", "--stdin", &path_arg], Some(content.as_bytes())).map(text)?;
        let info = format!("{mode},{},{rel}", hash.trim());
        self.run(&["update-index", "--add", "--cacheinfo", &info]).map(drop)
    }

    fn first_remote(&self) -> Result<String, String> {
        let remotes = self.run(&["remote"])?;
        remotes.lines().next().map(str::to_owned).ok_or_else(|| "This repository has no remote".to_owned())
    }

    pub fn fetch(&self) -> Result<(), String> {
        self.first_remote()?;
        self.run(&["fetch"]).map(drop)
    }

    pub fn pull(&self) -> Result<(), String> {
        self.first_remote()?;
        self.run(&["pull"]).map(drop)
    }

    /// Pushes the current branch, publishing it if it isn't on a remote yet.
    pub fn push(&self) -> Result<(), String> {
        if self.run(&["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).is_ok() {
            return self.run(&["push"]).map(drop);
        }
        let remote = self.first_remote()?;
        self.run(&["push", "-u", &remote, "HEAD"]).map(drop)
    }

    pub fn commit(&self, message: &str) -> Result<(), String> {
        git(&self.root, &["commit", "-q", "-F", "-"], Some(message.as_bytes())).map(drop)
    }

    /// Local branch names.
    pub fn branches(&self) -> Result<Vec<String>, String> {
        let out = self.run(&["branch", "--format=%(refname:short)"])?;
        Ok(out.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
    }

    pub fn switch(&self, branch: &str, create: bool) -> Result<(), String> {
        let args: &[&str] = if create { &["switch", "-q", "-c", branch] } else { &["switch", "-q", branch] };
        self.run(args).map(drop)
    }

    pub fn user_name(&self) -> Option<String> {
        self.run(&["config", "user.name"]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    }

    /// Blame for one 0-based line of `rel`, as it is in `contents` (unsaved
    /// edits included).
    pub fn blame_line(&self, rel: &str, line: usize, contents: &str) -> Result<BlameLine, String> {
        let range = format!("{},+1", line + 1);
        let out = git(
            &self.root,
            &["blame", "--porcelain", "-L", &range, "--contents", "-", "--", rel],
            Some(contents.as_bytes()),
        )
        .map(text)?;
        parse_blame(&out).ok_or_else(|| "no blame information".to_owned())
    }
}

fn parse_status(root: &Path, out: &str) -> Status {
    let mut status = Status::default();
    let mut entries = out.split('\0').filter(|e| !e.is_empty());
    while let Some(entry) = entries.next() {
        if let Some(branch) = entry.strip_prefix("## ") {
            parse_branch(branch, &mut status);
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (xy, rel) = entry.split_at(3);
        let (x, y) = (xy.as_bytes()[0] as char, xy.as_bytes()[1] as char);
        if x == 'R' || x == 'C' {
            entries.next(); // the original path
        }
        let change = |kind| FileChange { path: root.join(native(rel)), rel: rel.to_owned(), kind };
        let conflict = matches!((x, y), ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D'));
        if conflict {
            status.conflicts.push(change('!'));
        } else if x == '?' {
            status.unstaged.push(change('U'));
        } else {
            if x != ' ' {
                status.staged.push(change(x));
            }
            if y != ' ' {
                status.unstaged.push(change(y));
            }
        }
    }
    status
}

/// "main...origin/main [ahead 1, behind 2]", "No commits yet on main", "HEAD (no branch)".
fn parse_branch(line: &str, status: &mut Status) {
    let line = line.strip_prefix("No commits yet on ").unwrap_or(line);
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    status.branch =
        if name == "HEAD" { "(detached)".to_owned() } else { name.split("...").next().unwrap_or(name).to_owned() };
    let number = |key: &str| {
        rest.split([',', '[', ']']).find_map(|part| part.trim().strip_prefix(key)?.trim().parse().ok()).unwrap_or(0)
    };
    status.ahead = number("ahead");
    status.behind = number("behind");
}

fn parse_blame(out: &str) -> Option<BlameLine> {
    let mut lines = out.lines();
    let sha = lines.next()?.split_whitespace().next()?;
    let mut blame = BlameLine {
        author: String::new(),
        time: 0,
        summary: String::new(),
        uncommitted: sha.bytes().all(|b| b == b'0'),
    };
    for line in lines {
        if let Some(v) = line.strip_prefix("author ") {
            blame.author = v.to_owned();
        } else if let Some(v) = line.strip_prefix("author-time ") {
            blame.time = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("summary ") {
            blame.summary = v.to_owned();
        }
    }
    Some(blame)
}

/// "You, 3 days ago • Fix the parser", or "You • Uncommitted changes".
pub fn format_blame(b: &BlameLine, now: i64, me: Option<&str>) -> String {
    if b.uncommitted {
        return "You • Uncommitted changes".to_owned();
    }
    let who = if me == Some(b.author.as_str()) { "You" } else { b.author.as_str() };
    format!("{who}, {} • {}", relative_time(now - b.time), b.summary)
}

/// "3 days ago", "just now".
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

/// Lines of `text` without line endings (a final newline adds no empty line).
pub fn lines(text: &str) -> Vec<&str> {
    let mut v: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if v.last() == Some(&"") {
        v.pop();
    }
    v
}

/// The changed runs between two versions. With `ignore_whitespace`, lines
/// that differ only in whitespace count as equal.
pub fn diff_lines(old: &[&str], new: &[&str], ignore_whitespace: bool) -> Vec<Hunk> {
    let deadline = Some(Instant::now() + Duration::from_millis(500));
    let ops = if ignore_whitespace {
        let squash = |l: &&str| l.split_whitespace().collect::<String>();
        let (o, n): (Vec<String>, Vec<String>) = (old.iter().map(squash).collect(), new.iter().map(squash).collect());
        similar::capture_diff_slices_deadline(Algorithm::Myers, &o, &n, deadline)
    } else {
        similar::capture_diff_slices_deadline(Algorithm::Myers, old, new, deadline)
    };
    let mut hunks: Vec<Hunk> = Vec::new();
    for op in ops {
        let (o, n) = match op {
            DiffOp::Equal { .. } => continue,
            DiffOp::Delete { old_index, old_len, new_index } => (old_index..old_index + old_len, new_index..new_index),
            DiffOp::Insert { old_index, new_index, new_len } => (old_index..old_index, new_index..new_index + new_len),
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                (old_index..old_index + old_len, new_index..new_index + new_len)
            }
        };
        match hunks.last_mut() {
            Some(h) if h.old.end == o.start && h.new.end == n.start => {
                h.old.end = o.end;
                h.new.end = n.end;
            }
            _ => hunks.push(Hunk { old: o, new: n }),
        }
    }
    hunks
}

/// `base` with lines `range` replaced by `with`, joined with `nl`.
pub fn replace_lines(base: &[&str], range: Range<usize>, with: &[&str], nl: &str) -> String {
    let mut out: Vec<&str> = Vec::with_capacity(base.len() + with.len());
    out.extend(&base[..range.start]);
    out.extend(with);
    out.extend(&base[range.end..]);
    let mut text = out.join(nl);
    if !out.is_empty() {
        text.push_str(nl);
    }
    text
}

/// The parts of `hunks` on lines `selected` of the new text, for staging or
/// reverting only those lines. In a replaced run, new lines pair with old ones
/// by position, and a selection reaching its end takes all the old lines left.
/// A deletion counts when the selection touches the place it happened.
pub fn hunks_in_lines(hunks: &[Hunk], selected: Range<usize>) -> Vec<Hunk> {
    let mut out = Vec::new();
    for h in hunks {
        if h.new.is_empty() {
            if (selected.start..=selected.end).contains(&h.new.start) {
                out.push(h.clone());
            }
            continue;
        }
        let (from, to) = (selected.start.max(h.new.start), selected.end.min(h.new.end));
        if from >= to {
            continue;
        }
        let at = |line: usize| (h.old.start + (line - h.new.start)).min(h.old.end);
        let old_end = if to == h.new.end { h.old.end } else { at(to) };
        out.push(Hunk { old: at(from)..old_end, new: from..to });
    }
    out
}

/// `base` with `hunks` (ascending, from a diff against `new`) applied.
pub fn apply_hunks(base: &str, new: &str, hunks: &[Hunk], nl: &str) -> String {
    let new_lines = lines(new);
    let mut text = base.to_owned();
    for h in hunks.iter().rev() {
        text = replace_lines(&lines(&text), h.old.clone(), &new_lines[h.new.clone()], nl);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_fetches_and_pulls() {
        let dir = std::env::temp_dir().join(format!("rust-code-git-remote-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (a, b, bare) = (dir.join("a"), dir.join("b"), dir.join("bare.git"));
        for d in [&a, &b, &bare] {
            std::fs::create_dir_all(d).unwrap();
        }
        let sh = |d: &Path, args: &[&str]| {
            let out = std::process::Command::new("git").current_dir(d).args(args).output().unwrap();
            assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        let setup = |d: &Path| {
            sh(d, &["config", "user.name", "t"]);
            sh(d, &["config", "user.email", "t@t"]);
        };
        sh(&bare, &["init", "--bare", "-q"]);
        sh(&a, &["init", "-q"]);
        setup(&a);
        let repo = Repo { root: a.clone() };

        std::fs::write(a.join("f"), "1\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("one").unwrap();
        assert_eq!(repo.push().unwrap_err(), "This repository has no remote");
        assert_eq!(repo.fetch().unwrap_err(), "This repository has no remote");
        assert_eq!(repo.pull().unwrap_err(), "This repository has no remote");

        // The first push publishes the branch.
        sh(&a, &["remote", "add", "origin", bare.to_str().unwrap()]);
        repo.push().unwrap();

        // A second copy sees new commits after fetch, and gets them with pull.
        sh(&dir, &["clone", "-q", bare.to_str().unwrap(), "b"]);
        setup(&b);
        let other = Repo { root: b.clone() };
        std::fs::write(a.join("f"), "2\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("two").unwrap();
        assert_eq!(repo.status().unwrap().ahead, 1);
        repo.push().unwrap();
        assert_eq!(repo.status().unwrap().ahead, 0);
        other.fetch().unwrap();
        assert_eq!(other.status().unwrap().behind, 1);
        other.pull().unwrap();
        assert_eq!(other.status().unwrap().behind, 0);
        assert_eq!(std::fs::read_to_string(b.join("f")).unwrap().trim(), "2");

        // A push the remote refuses reports git's reason.
        std::fs::write(b.join("f"), "3\n").unwrap();
        other.stage(&["."]).unwrap();
        other.commit("three").unwrap();
        other.push().unwrap();
        std::fs::write(a.join("g"), "x\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("four").unwrap();
        assert!(repo.push().unwrap_err().contains("rejected"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stages_only_the_selected_lines() {
        let base = "a\nb\nc\nd\n";
        let new = "a\nB\nC\nd\nx\ny\n";
        let hunks = diff_lines(&lines(base), &lines(new), false);
        let staged = |selected| apply_hunks(base, new, &hunks_in_lines(&hunks, selected), "\n");
        // One line of a replaced run: the old line at its place goes.
        assert_eq!(staged(1..2), "a\nB\nc\nd\n");
        assert_eq!(staged(2..3), "a\nb\nC\nd\n");
        // One added line; both runs at once.
        assert_eq!(staged(4..5), "a\nb\nc\nd\nx\n");
        assert_eq!(staged(1..6), new);
        // An unchanged line selects nothing.
        assert_eq!(staged(0..1), base);

        // More old lines than new: reaching the end takes the rest.
        let hunks = [Hunk { old: 1..4, new: 1..2 }];
        assert_eq!(hunks_in_lines(&hunks, 1..2), [Hunk { old: 1..4, new: 1..2 }]);
        // More new lines than old: the extras only add.
        let hunks = [Hunk { old: 1..2, new: 1..4 }];
        assert_eq!(hunks_in_lines(&hunks, 2..3), [Hunk { old: 2..2, new: 2..3 }]);
        // A deletion, from either line beside it.
        let hunks = [Hunk { old: 1..2, new: 1..1 }];
        assert_eq!(hunks_in_lines(&hunks, 0..1).len(), 1);
        assert_eq!(hunks_in_lines(&hunks, 1..2).len(), 1);
        assert!(hunks_in_lines(&hunks, 2..3).is_empty());
    }

    #[test]
    fn diff_groups_changes_into_hunks() {
        let old = ["a", "b", "c", "d"];
        let new = ["a", "B", "c", "d", "e"];
        assert_eq!(diff_lines(&old, &new, false), vec![Hunk { old: 1..2, new: 1..2 }, Hunk { old: 4..4, new: 4..5 }]);
        assert!(diff_lines(&["fn x() {"], &["fn  x()  {"], true).is_empty());
        assert_eq!(diff_lines(&["a", "b"], &["a"], false), vec![Hunk { old: 1..2, new: 1..1 }]);
        assert_eq!(lines("x\r\ny\n"), vec!["x", "y"]);
        assert_eq!(replace_lines(&["a", "b", "c"], 1..2, &["B", "B2"], "\n"), "a\nB\nB2\nc\n");
    }

    #[test]
    fn parses_status_and_branch() {
        let out = "## main...origin/main [ahead 2, behind 1]\0M  staged.rs\0 M work.rs\0MM both.rs\0?? new.txt\0R  new.rs\0old.rs\0UU conflict.rs\0";
        let s = parse_status(Path::new("/r"), out);
        assert_eq!((s.branch.as_str(), s.ahead, s.behind), ("main", 2, 1));
        let kinds = |v: &[FileChange]| v.iter().map(|c| format!("{}{}", c.kind, c.rel)).collect::<Vec<_>>();
        assert_eq!(kinds(&s.staged), vec!["Mstaged.rs", "Mboth.rs", "Rnew.rs"]);
        assert_eq!(kinds(&s.unstaged), vec!["Mwork.rs", "Mboth.rs", "Unew.txt"]);
        assert_eq!(kinds(&s.conflicts), vec!["!conflict.rs"]);
        let mut s = Status::default();
        parse_branch("No commits yet on trunk", &mut s);
        assert_eq!(s.branch, "trunk");
    }

    #[test]
    fn formats_blame() {
        let b = BlameLine { author: "Ana".into(), time: 0, summary: "Fix".into(), uncommitted: false };
        assert_eq!(format_blame(&b, 3 * 86_400, Some("Ana")), "You, 3 days ago • Fix");
        assert_eq!(format_blame(&b, 90, None), "Ana, 1 minute ago • Fix");
    }

    /// Runs the real git CLI against a throwaway repository.
    #[test]
    fn stages_a_single_hunk_and_blames_lines() {
        let dir = std::env::temp_dir().join(format!("rust-code-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let setup = |args: &[&str]| git(&dir, args, None).unwrap();
        setup(&["init", "-q", "-b", "main"]);
        setup(&["config", "user.name", "Tester"]);
        setup(&["config", "user.email", "t@example.com"]);
        setup(&["config", "core.autocrlf", "false"]);
        std::fs::write(dir.join("f.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let repo = Repo::discover(&dir).expect("repo");
        repo.stage(&["f.txt"]).unwrap();
        repo.commit("first").unwrap();

        // Two separate changes; stage only the first.
        std::fs::write(dir.join("f.txt"), "ONE\ntwo\nthree\nFOUR\n").unwrap();
        let index = repo.show("f.txt", true).unwrap();
        let work = std::fs::read_to_string(dir.join("f.txt")).unwrap();
        let (old, new) = (lines(&index), lines(&work));
        let hunks = diff_lines(&old, &new, false);
        assert_eq!(hunks.len(), 2);
        let h = &hunks[0];
        repo.set_index_content("f.txt", &replace_lines(&old, h.old.clone(), &new[h.new.clone()], "\n")).unwrap();
        assert_eq!(repo.show("f.txt", true).unwrap(), "ONE\ntwo\nthree\nfour\n");
        let s = repo.status().unwrap();
        assert_eq!((s.branch.as_str(), s.staged.len(), s.unstaged.len()), ("main", 1, 1));

        let blame = repo.blame_line("f.txt", 1, &work).unwrap();
        assert_eq!((blame.author.as_str(), blame.summary.as_str(), blame.uncommitted), ("Tester", "first", false));
        assert!(repo.blame_line("f.txt", 0, &work).unwrap().uncommitted);

        repo.commit("second").unwrap();
        repo.switch("feature", true).unwrap();
        assert_eq!(repo.branches().unwrap(), vec!["feature", "main"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
