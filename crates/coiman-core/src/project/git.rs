//! Git through the `git` command line, as VS Code does: status, file contents
//! at HEAD and in the index, line diffs, staging single hunks by writing index
//! blobs, blame, commit and branches. Fetch, pull and push are the only commands
//! that reach the network, and only run when asked for.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use similar::{Algorithm, DiffOp};

use crate::language::lsp::discovery::quiet_command;

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
    /// The remote branch this one follows (`origin/main`), if it follows one.
    pub upstream: Option<String>,
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

/// One commit in the history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Unix time of the commit.
    pub time: i64,
    /// Branches and tags pointing here.
    pub refs: String,
    pub subject: String,
}

/// One stash; `name` looks like `stash@{0}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub name: String,
    pub message: String,
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
    crate::project::trust::check(dir)?;
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

/// Who commits are made by: what Git writes into each one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl Identity {
    pub fn is_complete(&self) -> bool {
        self.name.is_some() && self.email.is_some()
    }
}

/// Runs git where no repository's settings apply: its version, global settings, clone.
/// Such commands read no repository config, so they need no trust.
fn git_outside(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = quiet_command(Path::new("git"))
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(text(out.stdout))
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" ");
        Err(if msg.is_empty() { format!("git {} failed", args.first().unwrap_or(&"")) } else { msg })
    }
}

/// Git's version, when it's installed ("git version 2.47.1.windows.1").
pub fn installed() -> Option<String> {
    git_outside(&std::env::temp_dir(), &["--version"]).ok().map(|v| v.trim().to_owned())
}

/// Whether the GitHub CLI (`gh`) is installed.
pub fn github_cli_installed() -> bool {
    quiet_command(Path::new("gh")).arg("--version").stdin(Stdio::null()).output().is_ok_and(|o| o.status.success())
}

/// Where global settings are read and written: `--global`, or a given file (for tests).
fn scope_args(file: Option<&Path>) -> Vec<String> {
    match file {
        Some(f) => vec!["--file".to_owned(), f.display().to_string()],
        None => vec!["--global".to_owned()],
    }
}

/// The name and email set for every repository (`git config --global`).
pub fn global_identity() -> Identity {
    global_identity_in(None)
}

fn global_identity_in(file: Option<&Path>) -> Identity {
    let get = |key: &str| {
        let mut args = vec!["config".to_owned()];
        args.extend(scope_args(file));
        args.push(key.to_owned());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git_outside(&std::env::temp_dir(), &args).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
    };
    Identity { name: get("user.name"), email: get("user.email") }
}

/// Sets the name and email for every repository.
pub fn set_global_identity(name: &str, email: &str) -> Result<(), String> {
    set_global_identity_in(None, name, email)
}

fn set_global_identity_in(file: Option<&Path>, name: &str, email: &str) -> Result<(), String> {
    check_identity(name, email)?;
    for (key, value) in [("user.name", name), ("user.email", email)] {
        let mut args = vec!["config".to_owned()];
        args.extend(scope_args(file));
        args.extend([key.to_owned(), value.to_owned()]);
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        git_outside(&std::env::temp_dir(), &args)?;
    }
    Ok(())
}

/// A name and an email that look like what Git expects.
pub fn check_identity(name: &str, email: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Enter a name: it goes into every commit.".to_owned());
    }
    let email = email.trim();
    let (user, host) = email.split_once('@').unwrap_or(("", ""));
    if user.is_empty() || host.is_empty() || email.contains(char::is_whitespace) {
        return Err("Enter an email like you@example.com. For GitHub, use one on your account, or your noreply address, so commits show as yours.".to_owned());
    }
    Ok(())
}

/// Makes `dir` a repository, its first branch named `branch`.
pub fn init(dir: &Path, branch: &str) -> Result<Repo, String> {
    git(dir, &["init", "-q", "-b", branch], None)?;
    Ok(Repo { root: dir.to_path_buf() })
}

/// The folder a clone of `url` goes into: the last part of its path, without `.git`.
pub fn repo_name_from_url(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches(['/', '\\']);
    let last = url.rsplit(['/', '\\', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && !name.contains(['?', '#'])).then(|| name.to_owned())
}

/// Clones `url` into a new folder in `parent`. Returns that folder.
pub fn clone(url: &str, parent: &Path) -> Result<PathBuf, String> {
    let name = repo_name_from_url(url).ok_or("That doesn't look like a repository URL.")?;
    let target = parent.join(&name);
    if target.exists() {
        return Err(format!("{} already exists. Choose another folder, or remove it first.", target.display()));
    }
    git_outside(parent, &["clone", "--", url.trim(), &name])?;
    Ok(target)
}

fn valid_remote_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && !name.starts_with('-')
        && name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok { Ok(()) } else { Err(format!("\"{name}\" can't be a remote's name: use letters, digits, - _ or .")) }
}

/// The GitHub CLI's arguments to create a repository from the current folder.
fn github_create_args(name: &str, visibility: &str) -> Vec<String> {
    ["repo", "create", name, visibility, "--source", ".", "--remote", "origin", "--push"].map(str::to_owned).to_vec()
}

/// Git's error in plain words, with what to do, for the failures people meet most:
/// no author set, signing in, no upstream, rejected pushes, a repository not found.
pub fn explain(error: &str) -> String {
    let e = error.to_lowercase();
    let why = if e.contains("please tell me who you are")
        || e.contains("empty ident")
        || e.contains("unable to auto-detect email")
    {
        "Git doesn't know your name and email, which it writes into each commit. Set them in Source Control (Author)."
    } else if e.contains("authentication failed")
        || e.contains("could not read username")
        || e.contains("terminal prompts disabled")
        || e.contains("invalid username or password")
    {
        "The remote wants you to sign in. Git uses your system's credential manager: sign in once with `git push` in the terminal, or use an SSH remote. Your commit author (name and email) is a separate setting."
    } else if e.contains("permission denied (publickey)") {
        "The SSH key isn't accepted. Add your public key to your GitHub account, or use the HTTPS URL."
    } else if e.contains("has no upstream branch") || e.contains("no upstream configured") {
        "This branch isn't on the remote yet. Push publishes it."
    } else if e.contains("[rejected]") || e.contains("fetch first") || e.contains("non-fast-forward") {
        "The remote has commits you don't have. Pull first, then push."
    } else if e.contains("repository not found") || e.contains("does not appear to be a git repository") {
        "The remote isn't there, or you can't see it: check the URL, and that you're signed in to an account that can."
    } else if e.contains("could not resolve host") || e.contains("unable to access") && e.contains("connect") {
        "Couldn't reach the server: check the connection and the URL."
    } else if e.contains("not a git repository") {
        "This folder isn't in a Git repository. Initialize one in Source Control."
    } else if e.contains("this repository has no remote") || e.contains("no configured push destination") {
        "There's no remote to send to. Add one in Source Control (Remotes), or publish to GitHub."
    } else if e.contains("already exists") && e.contains("remote") {
        "A remote with that name exists already."
    } else {
        return error.to_owned();
    };
    format!("{why} ({})", error.trim())
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

    /// Replaces the last commit: its message, if given, and what is staged.
    pub fn amend(&self, message: Option<&str>) -> Result<(), String> {
        match message {
            Some(m) => git(&self.root, &["commit", "-q", "--amend", "-F", "-"], Some(m.as_bytes())).map(drop),
            None => self.run(&["commit", "-q", "--amend", "--no-edit"]).map(drop),
        }
    }

    pub fn last_commit_message(&self) -> Result<String, String> {
        Ok(self.run(&["log", "-1", "--format=%B"])?.trim().to_owned())
    }

    /// The latest commits of the current branch. A repository with none has an empty log.
    pub fn log(&self, limit: usize) -> Result<Vec<Commit>, String> {
        if self.run(&["rev-parse", "--verify", "-q", "HEAD"]).is_err() {
            return Ok(Vec::new());
        }
        let count = format!("-n{limit}");
        let out = self.run(&["log", &count, "--format=%H%x1f%h%x1f%an%x1f%at%x1f%D%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut f = line.splitn(6, '\x1f').map(str::to_owned);
                Some(Commit {
                    hash: f.next()?,
                    short: f.next()?,
                    author: f.next()?,
                    time: f.next()?.parse().ok()?,
                    refs: f.next()?,
                    subject: f.next()?,
                })
            })
            .collect())
    }

    /// Files a commit changed: (kind, relative path). A merge lists what it brought in.
    pub fn commit_files(&self, hash: &str) -> Result<Vec<(char, String)>, String> {
        let out = self.run(&["show", "--first-parent", "--format=", "--name-status", "--no-renames", "-z", hash])?;
        let mut parts = out.split('\0').filter(|p| !p.is_empty());
        let mut files = Vec::new();
        while let (Some(kind), Some(rel)) = (parts.next(), parts.next()) {
            files.push((kind.trim().chars().next().unwrap_or('M'), rel.to_owned()));
        }
        Ok(files)
    }

    /// A file's text at a revision, or empty if it wasn't there.
    pub fn file_at(&self, rev: &str, rel: &str) -> String {
        self.show_rev(rev, rel).unwrap_or_default()
    }

    fn show_rev(&self, rev: &str, rel: &str) -> Option<String> {
        git(&self.root, &["show", &format!("{rev}:{rel}")], None).ok().map(text)
    }

    /// Puts all changes, untracked files too, away. False if there were none.
    pub fn stash_push(&self) -> Result<bool, String> {
        Ok(!self.run(&["stash", "push", "-u"])?.contains("No local changes"))
    }

    pub fn stash_list(&self) -> Result<Vec<Stash>, String> {
        let out = self.run(&["stash", "list", "--format=%gd%x1f%s"])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\x1f'))
            .map(|(name, message)| Stash { name: name.to_owned(), message: message.to_owned() })
            .collect())
    }

    /// Brings a stash back and keeps it (`pop` false), or removes it after.
    pub fn stash_restore(&self, name: &str, pop: bool) -> Result<(), String> {
        self.run(&["stash", if pop { "pop" } else { "apply" }, name]).map(drop)
    }

    pub fn stash_drop(&self, name: &str) -> Result<(), String> {
        self.run(&["stash", "drop", name]).map(drop)
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

    /// The name and email commits here are made with: this repository's, else the global ones.
    pub fn identity(&self) -> Identity {
        let get = |key| self.run(&["config", key]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        Identity { name: get("user.name"), email: get("user.email") }
    }

    /// Sets the name and email for this repository only.
    pub fn set_identity(&self, name: &str, email: &str) -> Result<(), String> {
        self.run(&["config", "user.name", name])?;
        self.run(&["config", "user.email", email]).map(drop)
    }

    /// The remotes and where they point: (name, fetch URL).
    pub fn remotes(&self) -> Result<Vec<(String, String)>, String> {
        let out = self.run(&["remote", "-v"])?;
        Ok(out
            .lines()
            .filter(|l| l.ends_with("(fetch)"))
            .filter_map(|l| {
                let mut parts = l.split_whitespace();
                Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
            })
            .collect())
    }

    pub fn add_remote(&self, name: &str, url: &str) -> Result<(), String> {
        valid_remote_name(name)?;
        self.run(&["remote", "add", "--", name, url]).map(drop)
    }

    pub fn remove_remote(&self, name: &str) -> Result<(), String> {
        self.run(&["remote", "remove", "--", name]).map(drop)
    }

    /// Creates a repository on GitHub for this one, through the GitHub CLI (`gh`), which
    /// does the signing in. Adds it as `origin` and pushes the current branch.
    pub fn create_on_github(&self, name: &str, private: bool) -> Result<String, String> {
        let visibility = if private { "--private" } else { "--public" };
        let args = github_create_args(name, visibility);
        let mut cmd = quiet_command(Path::new("gh"));
        crate::project::trust::check(&self.root)?;
        let out = cmd
            .current_dir(&self.root)
            .args(&args)
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run the GitHub CLI (gh): {e}"))?;
        let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(all.lines().find(|l| l.contains("github.com")).unwrap_or("").trim().to_owned())
        } else {
            Err(all.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" "))
        }
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
    status.upstream = name.split_once("...").map(|(_, up)| up.to_owned()).filter(|up| !up.is_empty());
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
    fn identities_remotes_init_and_clone() {
        let base = std::env::temp_dir().join(format!("rust-code-git-setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert!(installed().is_some_and(|v| v.starts_with("git version")));
        // Global settings go to a file of our own here, never the real ones.
        let global = base.join("gitconfig");
        assert_eq!(global_identity_in(Some(&global)), Identity::default());
        assert!(set_global_identity_in(Some(&global), "", "a@b.c").is_err());
        assert!(set_global_identity_in(Some(&global), "Ann", "not an email").is_err());
        set_global_identity_in(Some(&global), "Ann Lee", "ann@example.com").unwrap();
        let ann = global_identity_in(Some(&global));
        assert_eq!((ann.name.as_deref(), ann.email.as_deref()), (Some("Ann Lee"), Some("ann@example.com")));
        assert!(ann.is_complete());
        // A new repository, its own author, its remotes.
        let dir = base.join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        let repo = init(&dir, "main").unwrap();
        repo.set_identity("Bo", "bo@example.com").unwrap();
        assert_eq!(repo.identity().email.as_deref(), Some("bo@example.com"));
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("first").unwrap();
        assert_eq!(repo.status().unwrap().branch, "main");
        assert!(repo.add_remote("-x", "u").is_err());
        repo.add_remote("origin", "https://example.com/proj.git").unwrap();
        assert_eq!(repo.remotes().unwrap(), [("origin".to_owned(), "https://example.com/proj.git".to_owned())]);
        assert!(repo.add_remote("origin", "x").is_err());
        repo.remove_remote("origin").unwrap();
        assert!(repo.remotes().unwrap().is_empty());
        // Clone it, by path, into another folder.
        let into = base.join("clones");
        std::fs::create_dir_all(&into).unwrap();
        let cloned = clone(&dir.display().to_string(), &into).unwrap();
        assert_eq!(cloned, into.join("proj"));
        assert!(cloned.join("a.txt").exists());
        assert!(clone(&dir.display().to_string(), &into).unwrap_err().contains("already exists"));
        let status = Repo { root: cloned }.status().unwrap();
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn names_from_urls_and_plain_errors() {
        for (url, name) in [
            ("https://github.com/user/repo.git", "repo"),
            ("https://github.com/user/repo", "repo"),
            ("https://github.com/user/repo/", "repo"),
            ("git@github.com:user/my-lib.git", "my-lib"),
            ("C:\\work\\proj", "proj"),
        ] {
            assert_eq!(repo_name_from_url(url).as_deref(), Some(name), "{url}");
        }
        assert_eq!(repo_name_from_url(""), None);
        assert!(explain("Author identity unknown *** Please tell me who you are.").contains("name and email"));
        assert!(explain("fatal: Authentication failed for 'https://github.com/a/b.git/'").contains("sign in"));
        assert!(
            explain("fatal: could not read Username for 'https://github.com': terminal prompts disabled")
                .contains("credential manager")
        );
        assert!(explain("! [rejected] main -> main (fetch first)").contains("Pull first"));
        assert!(explain("remote: Repository not found.").contains("check the URL"));
        assert!(explain("This repository has no remote").contains("Add one"));
        assert_eq!(explain("something else"), "something else");
        assert_eq!(
            github_create_args("demo", "--private"),
            ["repo", "create", "demo", "--private", "--source", ".", "--remote", "origin", "--push"]
        );
        let mut s = Status::default();
        parse_branch("main...origin/main [ahead 2]", &mut s);
        assert_eq!((s.branch.as_str(), s.upstream.as_deref(), s.ahead), ("main", Some("origin/main"), 2));
        parse_branch("feature", &mut s);
        assert_eq!(s.upstream, None);
    }

    #[test]
    fn amends_stashes_and_reads_history() {
        let dir = std::env::temp_dir().join(format!("rust-code-git-history-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |args: &[&str]| {
            let out = std::process::Command::new("git").current_dir(&dir).args(args).output().unwrap();
            assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        sh(&["init", "-q"]);
        sh(&["config", "user.name", "Ann"]);
        sh(&["config", "user.email", "a@a"]);
        sh(&["config", "core.autocrlf", "false"]);
        let repo = Repo { root: dir.clone() };
        assert_eq!(repo.log(10).unwrap(), []);

        std::fs::write(dir.join("a.txt"), "1\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("first").unwrap();
        std::fs::write(dir.join("a.txt"), "2\n").unwrap();
        std::fs::write(dir.join("b c.txt"), "new\n").unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit("secnd | with: symbols").unwrap();

        // Amending changes the message, and with no message only the content.
        repo.amend(Some("second | with: symbols")).unwrap();
        assert_eq!(repo.last_commit_message().unwrap(), "second | with: symbols");
        std::fs::write(dir.join("c.txt"), "late\n").unwrap();
        repo.stage(&["c.txt"]).unwrap();
        repo.amend(None).unwrap();
        let log = repo.log(10).unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!((log[0].subject.as_str(), log[0].author.as_str()), ("second | with: symbols", "Ann"));
        assert!(log[0].refs.contains("HEAD"));
        assert!(log[0].hash.starts_with(&log[0].short));
        assert_eq!(repo.log(1).unwrap().len(), 1);

        let files = repo.commit_files(&log[0].hash).unwrap();
        assert_eq!(files, [('M', "a.txt".into()), ('A', "b c.txt".into()), ('A', "c.txt".into())]);
        // The first commit has no parent to compare with.
        assert_eq!(repo.commit_files(&log[1].hash).unwrap(), [('A', "a.txt".to_owned())]);
        assert_eq!(repo.file_at(&log[0].hash, "a.txt"), "2\n");
        assert_eq!(repo.file_at(&format!("{}^", log[0].hash), "a.txt"), "1\n");
        assert_eq!(repo.file_at(&format!("{}^", log[0].hash), "c.txt"), "");

        // Stash takes everything, untracked files too.
        assert!(!repo.stash_push().unwrap());
        std::fs::write(dir.join("a.txt"), "3\n").unwrap();
        std::fs::write(dir.join("u.txt"), "u\n").unwrap();
        assert!(repo.stash_push().unwrap());
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "2\n");
        assert!(!dir.join("u.txt").exists());
        let stashes = repo.stash_list().unwrap();
        assert_eq!(stashes.len(), 1);
        assert_eq!(stashes[0].name, "stash@{0}");
        repo.stash_restore("stash@{0}", false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "3\n");
        assert_eq!(repo.stash_list().unwrap().len(), 1);
        sh(&["checkout", "-q", "--", "a.txt"]);
        std::fs::remove_file(dir.join("u.txt")).unwrap();
        repo.stash_restore("stash@{0}", true).unwrap();
        assert!(dir.join("u.txt").exists());
        assert_eq!(repo.stash_list().unwrap(), []);
        assert!(repo.stash_push().unwrap());
        repo.stash_drop("stash@{0}").unwrap();
        assert_eq!(repo.stash_list().unwrap(), []);
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
