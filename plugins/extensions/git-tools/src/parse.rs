//! What `git` prints, read: the log of a history search, the lines a commit changed, the
//! submodules, `check-ignore`'s rules. No `git` runs here: the glue runs it as jobs.

pub const FOUND_MAX: usize = 300;
pub const HIDDEN_MAX: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchKind {
    /// Commit messages containing the text (any case).
    #[default]
    Message,
    /// Commits that add or remove the text (`git log -S`).
    Change,
    /// Commits whose changed lines match a regular expression (`git log -G`).
    Regex,
    Author,
    /// Commits touching a path.
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

    pub fn from_label(label: &str) -> SearchKind {
        SearchKind::ALL.into_iter().find(|k| k.label() == label).unwrap_or_default()
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
    pub files: Vec<(char, String)>,
}

/// `git log`'s arguments for a search, or None for an empty query.
pub fn search_args(kind: SearchKind, query: &str, all: bool, within: &str) -> Option<Vec<String>> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    let mut args: Vec<String> = vec![
        "log".into(),
        format!("-n{FOUND_MAX}"),
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
    } else if !within.trim().is_empty() {
        args.push(within.trim().to_owned());
    }
    Some(args)
}

pub fn parse_log(out: &str) -> Vec<Found> {
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

/// The lines of `git show -U0` that hold the text (or match the regex): (file, + or -, line).
pub fn matching_lines(diff: &str, query: &str, regex: bool) -> Vec<(String, char, String)> {
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
    out
}

/// A date (UTC) from seconds since 1970: `2026-11-22`.
pub fn date(secs: i64) -> String {
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y })
}

// ----- submodules ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubState {
    Ok,
    NotInitialized,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub path: String,
    pub url: String,
    pub branch: Option<String>,
    pub commit: String,
    pub state: SubState,
    pub describe: String,
}

/// From `git config -f .gitmodules --get-regexp …` and `git submodule status`.
pub fn submodules(config: &str, status: &str) -> Vec<Submodule> {
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
    found
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
                    let commit = l[1..].split_whitespace().next().unwrap_or("").to_owned();
                    let describe =
                        l.split_once('(').map(|(_, d)| d.trim_end_matches(')').to_owned()).unwrap_or_default();
                    (state, commit, describe)
                }
                None => (SubState::NotInitialized, String::new(), String::new()),
            };
            Submodule { path, url, branch, commit, state, describe }
        })
        .collect()
}

/// Git's submodule error, with what to do when the cause is one Git doesn't spell out.
pub fn explain(e: &str) -> String {
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

// ----- hooks ------------------------------------------------------------------------------

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookState {
    On,
    Off,
    Sample,
    None,
}

/// Each hook's state, from the names in the hooks folder.
pub fn hook_states(names: &[String]) -> Vec<(&'static str, HookState)> {
    let has = |n: &str| names.iter().any(|x| x == n);
    HOOK_NAMES
        .iter()
        .map(|name| {
            let state = if has(name) {
                HookState::On
            } else if has(&format!("{name}.off")) {
                HookState::Off
            } else if has(&format!("{name}.sample")) {
                HookState::Sample
            } else {
                HookState::None
            };
            (*name, state)
        })
        .collect()
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

// ----- ignore rules -------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub source: String,
    pub line: usize,
    pub pattern: String,
    pub hidden: Vec<String>,
    pub count: usize,
}

/// `check-ignore -v` lines' rule part (`source:line:pattern`), a Windows drive kept whole.
fn split_rule(rule: &str) -> (String, usize, String) {
    let mut parts = rule.splitn(3, ':');
    let (source, number, pattern) =
        (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let (source, number, pattern) = if source.len() == 1 && number.starts_with('/') {
        let mut rest = rule.splitn(4, ':');
        let drive = rest.next().unwrap_or("");
        let path = rest.next().unwrap_or("");
        (format!("{drive}:{path}"), rest.next().unwrap_or("").to_owned(), rest.next().unwrap_or("").to_owned())
    } else {
        (source.to_owned(), number.to_owned(), pattern.to_owned())
    };
    (source, number.parse().unwrap_or(0), pattern)
}

/// The rules, from `check-ignore -v`'s output for what's ignored, and the top `.gitignore`
/// (its rules that hide nothing show too). `root` is the repository's folder, with `/`.
pub fn rules(verbose: &str, gitignore: Option<&str>, root: &str) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    for line in verbose.lines() {
        let Some((rule, path)) = line.split_once('\t') else { continue };
        let (source, line_no, pattern) = split_rule(rule);
        let rel = source.strip_prefix(&format!("{}/", root.trim_end_matches('/'))).map(str::to_owned).unwrap_or(source);
        match rules.iter_mut().find(|r| r.source == rel && r.line == line_no) {
            Some(r) => {
                r.count += 1;
                if r.hidden.len() < HIDDEN_MAX {
                    r.hidden.push(path.to_owned());
                }
            }
            None => rules.push(Rule { source: rel, line: line_no, pattern, hidden: vec![path.to_owned()], count: 1 }),
        }
    }
    if let Some(text) = gitignore {
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
                    hidden: vec![],
                    count: 0,
                });
            }
        }
    }
    rules.sort_by(|a, b| {
        (a.source != ".gitignore", &a.source, a.line).cmp(&(b.source != ".gitignore", &b.source, b.line))
    });
    rules
}

/// `check-ignore -v -n`'s answer for one path: the rule that hides it, or None.
pub fn why(out: &str) -> Option<(String, usize, String)> {
    let line = out.lines().next()?;
    let (rule, _) = line.split_once('\t')?;
    if rule == "::" {
        return None;
    }
    let mut parts = rule.rsplitn(3, ':');
    let pattern = parts.next().unwrap_or("").to_owned();
    let number = parts.next().unwrap_or("").parse().unwrap_or(0);
    let source = parts.next().unwrap_or("").to_owned();
    // A `!pattern` that matches un-ignores it.
    (!pattern.starts_with('!')).then_some((source, number, pattern))
}

/// `.gitignore`'s text with a pattern added at its end.
pub fn with_pattern(text: &str, pattern: &str) -> String {
    let mut text = text.to_owned();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern.trim());
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_and_lines() {
        let out = "\u{1e}abc123\u{1f}abc\u{1f}Ann\u{1f}1700000000\u{1f}Add parse\n\nM\tsrc/main.rs\nA\tsrc/new.rs\n\u{1e}def\u{1f}de\u{1f}Bo\u{1f}0\u{1f}x\n";
        let found = parse_log(out);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].files, [('M', "src/main.rs".to_owned()), ('A', "src/new.rs".to_owned())]);
        assert_eq!(date(found[0].when), "2023-11-14");
        assert_eq!(date(0), "1970-01-01");
        let diff = "diff --git a/m b/m\n--- a/m\n+++ b/m\n@@ -1 +1 @@\n-fn old() {}\n+fn parse_config() {}\n+other\n";
        assert_eq!(matching_lines(diff, "PARSE", false), [("m".to_owned(), '+', "fn parse_config() {}".to_owned())]);
        assert_eq!(matching_lines(diff, r"fn \w+\(", true).len(), 2);
        let args = search_args(SearchKind::Change, "x", true, "src").unwrap();
        assert!(
            args.contains(&"-Sx".to_owned()) && args.contains(&"--all".to_owned()) && args.last().unwrap() == "src"
        );
        assert!(search_args(SearchKind::Message, "  ", false, "").is_none());
        assert_eq!(SearchKind::from_label("Author"), SearchKind::Author);
    }

    #[test]
    fn submodules_hooks_and_rules() {
        let config = "submodule.lib.path vendor/lib\nsubmodule.lib.url ../lib\nsubmodule.lib.branch main\n";
        let status = " 99aad19 vendor/lib (heads/main)\n";
        let s = submodules(config, status);
        assert_eq!(
            (s[0].path.as_str(), s[0].state, s[0].describe.as_str()),
            ("vendor/lib", SubState::Ok, "heads/main")
        );
        assert_eq!(submodules(config, "-99aad19 vendor/lib\n")[0].state, SubState::NotInitialized);
        assert!(explain("fatal: transport 'file' not allowed fatal: Unable to fetch").contains("protocol.file.allow"));

        let names = ["pre-commit".to_owned(), "commit-msg.off".to_owned(), "pre-push.sample".to_owned()];
        let states = hook_states(&names);
        assert_eq!(states[0], ("pre-commit", HookState::On));
        assert_eq!(states.iter().find(|s| s.0 == "commit-msg").unwrap().1, HookState::Off);
        assert_eq!(states.iter().find(|s| s.0 == "pre-push").unwrap().1, HookState::Sample);
        assert!(hook_template("pre-commit").starts_with("#!/bin/sh\n"));

        let verbose = ".gitignore:2:*.log\ta.log\n.gitignore:2:*.log\tb.log\nC:/root/sub/.gitignore:1:out/\tsub/out/\n";
        let r = rules(verbose, Some("# build\n*.log\n*.bak\n"), "C:/root");
        let rows: Vec<(&str, usize, usize)> = r.iter().map(|r| (r.source.as_str(), r.line, r.count)).collect();
        assert_eq!(rows, [(".gitignore", 2, 2), (".gitignore", 3, 0), ("sub/.gitignore", 1, 1)]);
        assert_eq!(why(".gitignore:2:*.log\trun.log\n"), Some((".gitignore".into(), 2, "*.log".into())));
        assert_eq!(why("::\tsrc/x\n"), None);
        assert_eq!(why(".gitignore:3:!keep.log\tkeep.log\n"), None);
        assert_eq!(with_pattern("a", " *.tmp "), "a\n*.tmp\n");
    }
}
