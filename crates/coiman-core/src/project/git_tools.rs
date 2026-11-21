//! More of Git, all through `git` itself: searching the history (messages, authors, files,
//! and changes that add or remove a text), submodules, hooks, and what each `.gitignore`
//! rule hides.

use super::git::Repo;

// ----- Searching the history ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchKind {
    /// Commit messages containing the text (any case).
    #[default]
    Message,
    /// Commits that add or remove the text (`git log -S`): where something came or went.
    Change,
    /// Commits whose changed lines match a regular expression (`git log -G`).
    Regex,
    Author,
    /// Commits touching a path (a file or folder, `*` allowed).
    Path,
}

impl SearchKind {
    pub const ALL: [SearchKind; 5] =
        [SearchKind::Message, SearchKind::Change, SearchKind::Regex, SearchKind::Author, SearchKind::Path];

    pub fn label(self) -> &'static str {
        match self {
            SearchKind::Message => "Message",
            SearchKind::Change => "Added or removed text",
            SearchKind::Regex => "Changed lines matching (regex)",
            SearchKind::Author => "Author",
            SearchKind::Path => "File or folder",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub hash: String,
    pub short: String,
    pub author: String,
    /// Seconds since 1970.
    pub when: i64,
    pub subject: String,
    /// What it did to each file (`M`, `A`, `D`, `R`) and the file.
    pub files: Vec<(char, String)>,
}

/// How many commits a search shows.
pub const FOUND_MAX: usize = 300;

/// The commits matching `query` (newest first), on every branch when `all`, and only those
/// touching `within` when given.
pub fn search(
    repo: &Repo,
    kind: SearchKind,
    query: &str,
    all: bool,
    within: Option<&str>,
) -> Result<Vec<Found>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let max = format!("-n{FOUND_MAX}");
    let mut args: Vec<String> = vec![
        "log".into(),
        max,
        "--format=%x1e%H%x1f%h%x1f%an%x1f%at%x1f%s".into(),
        "--name-status".into(),
        "--no-renames".into(),
    ];
    if all {
        args.push("--all".into());
    }
    match kind {
        SearchKind::Message => args.extend(["-i".into(), format!("--grep={query}"), "--fixed-strings".into()]),
        SearchKind::Change => args.push(format!("-S{query}")),
        SearchKind::Regex => args.push(format!("-G{query}")),
        SearchKind::Author => args.extend(["-i".into(), format!("--author={query}")]),
        SearchKind::Path => {}
    }
    args.push("--".into());
    if kind == SearchKind::Path {
        args.push(query.to_owned());
    } else if let Some(w) = within.filter(|w| !w.trim().is_empty()) {
        args.push(w.trim().to_owned());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(parse_log(&repo.run(&refs)?))
}

fn parse_log(out: &str) -> Vec<Found> {
    out.split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut lines = record.lines();
            let head: Vec<&str> = lines.next()?.split('\u{1f}').collect();
            let [hash, short, author, at, subject] = head.as_slice() else { return None };
            let files = lines
                .filter_map(|l| {
                    let (status, path) = l.split_once('\t')?;
                    Some((status.chars().next()?, path.to_owned()))
                })
                .collect();
            Some(Found {
                hash: (*hash).to_owned(),
                short: (*short).to_owned(),
                author: (*author).to_owned(),
                when: at.parse().unwrap_or(0),
                subject: (*subject).to_owned(),
                files,
            })
        })
        .collect()
}

/// The lines a commit added (`+`) or removed (`-`) that contain the text (or match the regex),
/// by file: what made it a match.
pub fn matching_lines(
    repo: &Repo,
    hash: &str,
    query: &str,
    regex: bool,
) -> Result<Vec<(String, char, String)>, String> {
    let diff = repo.run(&["show", "--format=", "-U0", "--no-renames", hash])?;
    let re = if regex { regex::Regex::new(query).ok() } else { None };
    let lower = query.to_lowercase();
    let mut file = String::new();
    let mut out = Vec::new();
    for line in diff.lines() {
        if let Some(f) = line.strip_prefix("+++ b/") {
            file = f.to_owned();
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        let (mark, body) = match line.chars().next() {
            Some(c @ ('+' | '-')) => (c, &line[1..]),
            _ => continue,
        };
        let hit = match &re {
            Some(re) => re.is_match(body),
            None => body.to_lowercase().contains(&lower),
        };
        if hit {
            out.push((file.clone(), mark, body.to_owned()));
            if out.len() >= 200 {
                break;
            }
        }
    }
    Ok(out)
}

// ----- Submodules --------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
    /// The commit the repository records for it.
    pub commit: String,
    pub state: SubState,
    /// `git describe` of what's checked out, when git gives it.
    pub describe: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubState {
    /// Checked out at the recorded commit.
    Ok,
    /// Not fetched yet (`git submodule update --init` brings it).
    NotInitialized,
    /// Checked out at another commit than the one recorded.
    Moved,
    Conflict,
}

impl SubState {
    pub fn label(self) -> &'static str {
        match self {
            SubState::Ok => "up to date",
            SubState::NotInitialized => "not fetched",
            SubState::Moved => "at another commit",
            SubState::Conflict => "in conflict",
        }
    }
}

pub fn submodules(repo: &Repo) -> Result<Vec<Submodule>, String> {
    if !repo.root.join(".gitmodules").exists() {
        return Ok(Vec::new());
    }
    // From .gitmodules: each one's path, url and branch.
    let config = repo
        .run(&["config", "-f", ".gitmodules", "--get-regexp", r"^submodule\..*\.(path|url|branch)$"])
        .unwrap_or_default();
    let mut found: Vec<(String, String, String, Option<String>)> = Vec::new();
    for line in config.lines() {
        let Some((key, value)) = line.split_once(' ') else { continue };
        let Some(rest) = key.strip_prefix("submodule.") else { continue };
        let Some((name, field)) = rest.rsplit_once('.') else { continue };
        let at = match found.iter().position(|(n, ..)| n == name) {
            Some(i) => i,
            None => {
                found.push((name.to_owned(), String::new(), String::new(), None));
                found.len() - 1
            }
        };
        match field {
            "path" => found[at].1 = value.to_owned(),
            "url" => found[at].2 = value.to_owned(),
            "branch" => found[at].3 = Some(value.to_owned()),
            _ => {}
        }
    }
    // `git submodule status`: " <sha> path (describe)", '-' not initialized, '+' moved, 'U' conflict.
    let status = repo.run(&["submodule", "status"]).unwrap_or_default();
    Ok(found
        .into_iter()
        .map(|(_, path, url, branch)| {
            let line =
                status.lines().find(|l| l.get(1..).and_then(|r| r.split_whitespace().nth(1)) == Some(path.as_str()));
            let (state, commit, describe) = match line {
                Some(l) => {
                    let state = match l.chars().next() {
                        Some('-') => SubState::NotInitialized,
                        Some('+') => SubState::Moved,
                        Some('U') => SubState::Conflict,
                        _ => SubState::Ok,
                    };
                    let mut parts = l[1..].split_whitespace();
                    let commit = parts.next().unwrap_or("").to_owned();
                    let describe =
                        l.split_once('(').map(|(_, d)| d.trim_end_matches(')').to_owned()).unwrap_or_default();
                    (state, commit, describe)
                }
                None => (SubState::NotInitialized, String::new(), String::new()),
            };
            Submodule { path, url, branch, commit, state, describe }
        })
        .collect())
}

/// Fetches and checks out submodules at their recorded commits (all, or one).
pub fn submodule_update(repo: &Repo, path: Option<&str>) -> Result<String, String> {
    let mut args = vec!["submodule", "update", "--init", "--recursive"];
    if let Some(p) = path {
        args.extend(["--", p]);
    }
    repo.run(&args).map(|_| "Submodules fetched and checked out".to_owned()).map_err(explain)
}

/// Git's submodule error, with what to do when the cause is one Git doesn't spell out.
fn explain(e: String) -> String {
    let e = e.trim().replace(" fatal: Unable", "\nfatal: Unable");
    if e.contains("transport 'file' not allowed") {
        format!(
            "{e}\nGit fetches submodules from a folder on this computer only when allowed: \
             git config protocol.file.allow always (in this repository, or --global)."
        )
    } else {
        e
    }
}

/// Moves a submodule to the newest commit of its branch (the change shows in Source Control).
pub fn submodule_to_latest(repo: &Repo, path: &str) -> Result<String, String> {
    repo.run(&["submodule", "update", "--remote", "--", path])
        .map(|_| format!("{path} is at its branch's newest commit: commit to record it"))
        .map_err(explain)
}

pub fn submodule_add(repo: &Repo, url: &str, path: &str) -> Result<String, String> {
    if url.trim().is_empty() || path.trim().is_empty() {
        return Err("Give the repository's address and the folder to put it in".to_owned());
    }
    repo.run(&["submodule", "add", "--", url.trim(), path.trim()])
        .map(|_| format!("Added {path}: commit to record it"))
        .map_err(explain)
}

/// Takes a submodule out: unregisters it, removes its folder and its `.gitmodules` entry
/// (staged, for a commit). Its own history stays in `.git/modules` until cleaned.
pub fn submodule_remove(repo: &Repo, path: &str) -> Result<String, String> {
    repo.run(&["submodule", "deinit", "-f", "--", path])?;
    repo.run(&["rm", "-f", "--", path])?;
    Ok(format!("Removed {path}: commit to record it"))
}

// ----- Hooks -------------------------------------------------------------------------------

/// The hooks Git knows, in the order they matter to most people.
pub const HOOK_NAMES: [&str; 18] = [
    "pre-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-push",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-merge-commit",
    "post-rewrite",
    "pre-auto-gc",
    "reference-transaction",
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "push-to-checkout",
    "sendemail-validate",
    "fsmonitor-watchman",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub name: String,
    pub path: std::path::PathBuf,
    pub state: HookState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookState {
    /// It runs.
    On,
    /// Turned off here (renamed `.off`).
    Off,
    /// Only Git's example (`.sample`) is there.
    Sample,
    None,
}

/// What does the hooks: Git's own folder or one set by `core.hooksPath` (Husky, Lefthook…).
pub fn hooks_dir(repo: &Repo) -> Result<(std::path::PathBuf, Option<String>), String> {
    let custom =
        repo.run(&["config", "--get", "core.hooksPath"]).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let dir = repo.run(&["rev-parse", "--git-path", "hooks"])?;
    let dir = std::path::PathBuf::from(dir.trim());
    let dir = if dir.is_absolute() { dir } else { repo.root.join(dir) };
    Ok((dir, custom))
}

pub fn hooks(repo: &Repo) -> Result<Vec<Hook>, String> {
    let (dir, _) = hooks_dir(repo)?;
    Ok(HOOK_NAMES
        .iter()
        .map(|name| {
            let path = dir.join(name);
            let state = if path.is_file() {
                HookState::On
            } else if dir.join(format!("{name}.off")).is_file() {
                HookState::Off
            } else if dir.join(format!("{name}.sample")).is_file() {
                HookState::Sample
            } else {
                HookState::None
            };
            Hook { name: (*name).to_owned(), path, state }
        })
        .collect())
}

/// A starting script for a hook, saying what Git gives it.
pub fn hook_template(name: &str) -> String {
    let what = match name {
        "pre-commit" => {
            "Runs before each commit; stopping (exit 1) stops the commit.\n# Example: cargo fmt --check || exit 1"
        }
        "commit-msg" => {
            "Gets the message's file as $1; stopping rejects the message.\n# Example: grep -qE '^(feat|fix|docs):' \"$1\" || { echo 'Start with feat:, fix: or docs:'; exit 1; }"
        }
        "prepare-commit-msg" => "Gets the message's file as $1, before the editor opens; it can fill it in.",
        "pre-push" => {
            "Runs before a push (remote name $1, address $2); stopping stops the push.\n# Example: cargo test || exit 1"
        }
        "post-checkout" => "Runs after a checkout or switch ($1 old, $2 new, $3 = 1 for a branch).",
        "post-merge" => "Runs after a merge or a pull.",
        _ => "See `git help hooks` for what this hook gets.",
    };
    format!("#!/bin/sh\n# {name}: {what}\n\nexit 0\n")
}

/// Makes a hook from its template (or from Git's sample when there is one), runnable.
pub fn hook_create(repo: &Repo, name: &str) -> Result<std::path::PathBuf, String> {
    let (dir, _) = hooks_dir(repo)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(name);
    if !path.exists() {
        let off = dir.join(format!("{name}.off"));
        if off.exists() {
            std::fs::rename(&off, &path).map_err(|e| e.to_string())?;
        } else {
            std::fs::write(&path, hook_template(name)).map_err(|e| e.to_string())?;
        }
    }
    executable(&path)?;
    Ok(path)
}

#[cfg(unix)]
fn executable(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut p = std::fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    p.set_mode(p.mode() | 0o755);
    std::fs::set_permissions(path, p).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn executable(_: &std::path::Path) -> Result<(), String> {
    // Git for Windows runs hooks through its own shell, by their #! line.
    Ok(())
}

/// Turns a hook off (renamed `name.off`, which Git doesn't run) or on again.
pub fn hook_toggle(repo: &Repo, name: &str) -> Result<HookState, String> {
    let (dir, _) = hooks_dir(repo)?;
    let (on, off) = (dir.join(name), dir.join(format!("{name}.off")));
    if on.exists() {
        std::fs::rename(&on, &off).map_err(|e| e.to_string())?;
        Ok(HookState::Off)
    } else if off.exists() {
        std::fs::rename(&off, &on).map_err(|e| e.to_string())?;
        Ok(HookState::On)
    } else {
        Err(format!("There's no {name} hook to turn on or off"))
    }
}

/// Runs a hook now, as Git would (`git hook run`), with what it printed.
pub fn hook_run(repo: &Repo, name: &str) -> Result<String, String> {
    // A repository's hooks are its own programs: only in a trusted folder.
    crate::project::trust::check(&repo.root)?;
    // Git sends a hook's output to stderr; both are what it printed.
    let out = crate::language::lsp::discovery::quiet_command(std::path::Path::new("git"))
        .current_dir(&repo.root)
        .args(["hook", "run", "--ignore-missing", name])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let text = if text.trim().is_empty() { "(it printed nothing)".to_owned() } else { text.trim_end().to_owned() };
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("{name} stopped ({}): {text}", out.status.code().map_or("killed".into(), |c| format!("exit {c}"))))
    }
}

// ----- .gitignore --------------------------------------------------------------------------

/// A rule of an ignore file and the files it hides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The file it's in (`.gitignore`, `sub/.gitignore`, `.git/info/exclude`, the global one).
    pub source: String,
    /// 1-based.
    pub line: usize,
    pub pattern: String,
    /// What it hides (folders end with `/`), up to [`HIDDEN_MAX`].
    pub hidden: Vec<String>,
    pub count: usize,
}

pub const HIDDEN_MAX: usize = 200;

/// Every rule of the repository's `.gitignore` files with what it hides now, the rules that
/// hide nothing included (as they're written, in order).
pub fn ignore_rules(repo: &Repo) -> Result<Vec<Rule>, String> {
    // What's ignored (folders as one entry), and which rule did it.
    let ignored = repo.run(&["ls-files", "--others", "--ignored", "--exclude-standard", "--directory"])?;
    let paths: Vec<&str> = ignored.lines().filter(|l| !l.is_empty()).collect();
    let mut rules: Vec<Rule> = Vec::new();
    if !paths.is_empty() {
        let verbose = super::git::git_stdin(
            &repo.root,
            &["check-ignore", "-v", "--stdin", "--no-index"],
            paths.join("\n").as_bytes(),
        )?;
        for line in verbose.lines() {
            // source:line:pattern<TAB>path
            let Some((rule, path)) = line.split_once('\t') else { continue };
            let mut parts = rule.splitn(3, ':');
            let (source, number, pattern) =
                (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
            // A Windows path's drive has a colon too: C:/Users/… .
            let (source, number, pattern) = if source.len() == 1 && number.starts_with('/') {
                let mut rest = rule.splitn(4, ':');
                let drive = rest.next().unwrap_or("");
                let path = rest.next().unwrap_or("");
                (format!("{drive}:{path}"), rest.next().unwrap_or("").to_owned(), rest.next().unwrap_or("").to_owned())
            } else {
                (source.to_owned(), number.to_owned(), pattern.to_owned())
            };
            let line_no: usize = number.parse().unwrap_or(0);
            let rel = source
                .strip_prefix(&format!("{}/", repo.root.display().to_string().replace('\\', "/")))
                .map(str::to_owned)
                .unwrap_or(source);
            match rules.iter_mut().find(|r| r.source == rel && r.line == line_no) {
                Some(r) => {
                    r.count += 1;
                    if r.hidden.len() < HIDDEN_MAX {
                        r.hidden.push(path.to_owned());
                    }
                }
                None => {
                    rules.push(Rule { source: rel, line: line_no, pattern, hidden: vec![path.to_owned()], count: 1 })
                }
            }
        }
    }
    // The rules of the top .gitignore that hide nothing, so they show (and can go).
    if let Ok(text) = std::fs::read_to_string(repo.root.join(".gitignore")) {
        for (i, l) in text.lines().enumerate() {
            let p = l.trim();
            if p.is_empty() || p.starts_with('#') {
                continue;
            }
            if !rules.iter().any(|r| r.source == ".gitignore" && r.line == i + 1) {
                rules.push(Rule {
                    source: ".gitignore".into(),
                    line: i + 1,
                    pattern: p.to_owned(),
                    hidden: Vec::new(),
                    count: 0,
                });
            }
        }
    }
    rules.sort_by(|a, b| {
        (a.source != ".gitignore", &a.source, a.line).cmp(&(b.source != ".gitignore", &b.source, b.line))
    });
    Ok(rules)
}

/// Why a path is (or isn't) ignored: the rule that decides, or None.
pub fn why_ignored(repo: &Repo, path: &str) -> Result<Option<(String, usize, String)>, String> {
    let out = super::git::git_stdin(
        &repo.root,
        &["check-ignore", "-v", "-n", "--stdin", "--no-index"],
        path.trim().as_bytes(),
    );
    // It exits 1 when nothing is ignored: the same answer, not a failure.
    let out = match out {
        Ok(out) => out,
        Err(e) if e.starts_with("::") || e.trim().is_empty() => return Ok(None),
        Err(e) => return Err(e),
    };
    let line = out.lines().next().unwrap_or("");
    let Some((rule, _)) = line.split_once('\t') else { return Ok(None) };
    if rule == "::" {
        return Ok(None);
    }
    // The pattern is after the last ':' that follows the line number.
    let mut parts = rule.rsplitn(3, ':');
    let pattern = parts.next().unwrap_or("").to_owned();
    let number = parts.next().unwrap_or("").parse().unwrap_or(0);
    let source = parts.next().unwrap_or("").to_owned();
    // A `!pattern` that matches un-ignores it.
    if pattern.starts_with('!') {
        return Ok(None);
    }
    Ok(Some((source, number, pattern)))
}

/// What a new pattern would hide: untracked files it matches, and tracked ones it can't hide
/// (Git keeps tracking those until they're removed from the index).
pub fn preview_pattern(repo: &Repo, pattern: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let p = format!("--exclude={}", pattern.trim());
    let untracked = repo.run(&["ls-files", "--others", "--ignored", "--directory", &p])?;
    let tracked = repo.run(&["ls-files", "--cached", "--ignored", &p])?;
    let take = |s: &str| s.lines().filter(|l| !l.is_empty()).take(HIDDEN_MAX).map(str::to_owned).collect();
    Ok((take(&untracked), take(&tracked)))
}

/// Adds a pattern to the top `.gitignore` (made if missing).
pub fn add_pattern(repo: &Repo, pattern: &str) -> Result<(), String> {
    let path = repo.root.join(".gitignore");
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern.trim());
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_in(dir: &std::path::Path) -> Repo {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let repo = super::super::git::init(dir, "main").unwrap();
        repo.set_identity("Ann", "ann@example.com").unwrap();
        repo
    }

    fn commit(repo: &Repo, file: &str, text: &str, message: &str) {
        let path = repo.root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        repo.stage(&["."]).unwrap();
        repo.commit(message).unwrap();
    }

    #[test]
    fn searches_history() {
        let dir = std::env::temp_dir().join(format!("coiman-gittools-search-{}", std::process::id()));
        let repo = repo_in(&dir);
        commit(&repo, "src/a.rs", "fn main() {}\n", "Start");
        commit(&repo, "src/a.rs", "fn main() {\n    let retries = 3;\n}\n", "Retry on failure");
        commit(&repo, "docs/x.md", "# Docs\n", "docs: Write the docs");
        commit(&repo, "src/a.rs", "fn main() {\n    let attempts = 3;\n}\n", "Rename");
        let msg = search(&repo, SearchKind::Message, "retry", false, None).unwrap();
        assert_eq!(msg.len(), 1);
        assert_eq!(msg[0].subject, "Retry on failure");
        assert_eq!(msg[0].files, [('M', "src/a.rs".to_owned())]);
        // Where `retries` came in and went out.
        let change = search(&repo, SearchKind::Change, "retries", false, None).unwrap();
        assert_eq!(change.iter().map(|f| f.subject.as_str()).collect::<Vec<_>>(), ["Rename", "Retry on failure"]);
        let lines = matching_lines(&repo, &change[0].hash, "retries", false).unwrap();
        assert_eq!(lines, [("src/a.rs".to_owned(), '-', "    let retries = 3;".to_owned())]);
        assert_eq!(search(&repo, SearchKind::Regex, r"let \w+ = 3", false, None).unwrap().len(), 2);
        assert_eq!(search(&repo, SearchKind::Path, "docs", false, None).unwrap()[0].subject, "docs: Write the docs");
        assert_eq!(search(&repo, SearchKind::Author, "ann", false, None).unwrap().len(), 4);
        assert_eq!(search(&repo, SearchKind::Message, "", false, None).unwrap(), []);
        // Only within a folder.
        assert!(search(&repo, SearchKind::Message, "docs", false, Some("src")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hooks_and_ignores() {
        let dir = std::env::temp_dir().join(format!("coiman-gittools-hooks-{}", std::process::id()));
        let repo = repo_in(&dir);
        let all = hooks(&repo).unwrap();
        assert_eq!(all.len(), HOOK_NAMES.len());
        let pre = all.iter().find(|h| h.name == "pre-commit").unwrap();
        assert!(matches!(pre.state, HookState::Sample | HookState::None));
        // Make one that prints, run it, turn it off and on.
        let path = hook_create(&repo, "pre-commit").unwrap();
        std::fs::write(&path, "#!/bin/sh\necho checked by the hook\n").unwrap();
        assert!(hook_run(&repo, "pre-commit").unwrap().contains("checked by the hook"));
        assert_eq!(hook_toggle(&repo, "pre-commit").unwrap(), HookState::Off);
        assert_eq!(hooks(&repo).unwrap().iter().find(|h| h.name == "pre-commit").unwrap().state, HookState::Off);
        assert_eq!(hook_run(&repo, "pre-commit").unwrap(), "(it printed nothing)");
        assert_eq!(hook_toggle(&repo, "pre-commit").unwrap(), HookState::On);
        assert!(hook_template("commit-msg").starts_with("#!/bin/sh"));

        // Ignore rules: what each hides, and one that hides nothing.
        commit(&repo, "keep.txt", "k\n", "keep");
        std::fs::write(dir.join(".gitignore"), "# build\ntarget/\n*.log\nnothing-here/\n").unwrap();
        std::fs::create_dir_all(dir.join("target").join("debug")).unwrap();
        std::fs::write(dir.join("target").join("debug").join("app"), "x").unwrap();
        std::fs::write(dir.join("a.log"), "x").unwrap();
        std::fs::write(dir.join("b.log"), "x").unwrap();
        let rules = ignore_rules(&repo).unwrap();
        let rule = |p: &str| rules.iter().find(|r| r.pattern == p).unwrap();
        assert_eq!((rule("target/").line, rule("target/").hidden.as_slice()), (2, &["target/".to_owned()][..]));
        assert_eq!(rule("*.log").count, 2);
        assert_eq!(rule("nothing-here/").count, 0);
        assert_eq!(why_ignored(&repo, "c.log").unwrap(), Some((".gitignore".into(), 3, "*.log".into())));
        assert_eq!(why_ignored(&repo, "keep.txt").unwrap(), None);
        // A new pattern: what it would hide, and the tracked file it can't.
        let (untracked, tracked) = preview_pattern(&repo, "*.txt").unwrap();
        assert!(untracked.is_empty() && tracked == ["keep.txt"]);
        add_pattern(&repo, "*.tmp").unwrap();
        assert!(std::fs::read_to_string(dir.join(".gitignore")).unwrap().ends_with("nothing-here/\n*.tmp\n"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn explains_a_local_submodule_refused() {
        let e = explain(
            "fatal: transport 'file' not allowed fatal: Unable to fetch in submodule path 'vendor/lib'".to_owned(),
        );
        assert!(e.starts_with("fatal: transport 'file' not allowed\nfatal: Unable"), "{e}");
        assert!(e.contains("protocol.file.allow always"), "{e}");
        assert_eq!(explain("fatal: other".to_owned()), "fatal: other");
    }

    #[test]
    fn submodules_of_a_repository() {
        let base = std::env::temp_dir().join(format!("coiman-gittools-sub-{}", std::process::id()));
        let lib = repo_in(&base.join("lib"));
        commit(&lib, "lib.txt", "v1\n", "lib v1");
        let app = repo_in(&base.join("app"));
        commit(&app, "app.txt", "a\n", "app");
        assert!(submodules(&app).unwrap().is_empty());
        // Local paths need the file protocol allowed (off by default since Git 2.38).
        let url = lib.root.display().to_string().replace('\\', "/");
        app.run(&["-c", "protocol.file.allow=always", "submodule", "add", "--", &url, "vendor/lib"]).unwrap();
        app.commit("add lib").unwrap();
        let subs = submodules(&app).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(
            (subs[0].path.as_str(), subs[0].url.as_str(), subs[0].state),
            ("vendor/lib", url.as_str(), SubState::Ok)
        );
        // Moved inside: the parent sees it at another commit.
        commit(&Repo { root: app.root.join("vendor").join("lib") }, "lib.txt", "v2\n", "lib v2 here");
        assert_eq!(submodules(&app).unwrap()[0].state, SubState::Moved);
        assert!(submodule_remove(&app, "vendor/lib").unwrap().contains("Removed"));
        assert!(!app.root.join("vendor").join("lib").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
