//! A review's data: its base, comments (with replies) anchored to lines, and the verdict;
//! kept as text in the plugin's storage, one line per record. No host calls: tested alone.

#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
    pub id: u64,
    pub path: String,
    /// 1-based line, as last found.
    pub line: u32,
    /// The line's text when the comment was made (trimmed): it follows the code when lines move.
    pub anchor: String,
    pub text: String,
    pub time: i64,
    pub resolved: bool,
    pub replies: Vec<(i64, String)>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Review {
    /// What the changes are compared with: a branch, a commit, or "" for the last commit.
    pub base: String,
    /// "approve", "changes" or "comment".
    pub verdict: String,
    pub summary: String,
    pub comments: Vec<Comment>,
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n").replace('\r', "")
}

fn unesc(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Every project's review from the storage text: (project, review).
pub fn read_all(text: &str) -> Vec<(String, Review)> {
    let mut all: Vec<(String, Review)> = Vec::new();
    let get = |all: &mut Vec<(String, Review)>, p: &str| -> usize {
        match all.iter().position(|(q, _)| q == p) {
            Some(i) => i,
            None => {
                all.push((p.to_owned(), Review::default()));
                all.len() - 1
            }
        }
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.first().copied() {
            Some("review") if f.len() == 5 => {
                let i = get(&mut all, f[1]);
                let r = &mut all[i].1;
                r.base = unesc(f[2]);
                r.verdict = unesc(f[3]);
                r.summary = unesc(f[4]);
            }
            Some("comment") if f.len() == 9 => {
                let (Ok(id), Ok(line), Ok(time)) = (f[2].parse(), f[4].parse(), f[7].parse()) else { continue };
                let i = get(&mut all, f[1]);
                all[i].1.comments.push(Comment {
                    id,
                    path: unesc(f[3]),
                    line,
                    resolved: f[5] == "1",
                    anchor: unesc(f[6]),
                    time,
                    text: unesc(f[8]),
                    replies: Vec::new(),
                });
            }
            Some("reply") if f.len() == 5 => {
                let (Ok(id), Ok(time)) = (f[2].parse::<u64>(), f[3].parse()) else { continue };
                let i = get(&mut all, f[1]);
                if let Some(c) = all[i].1.comments.iter_mut().find(|c| c.id == id) {
                    c.replies.push((time, unesc(f[4])));
                }
            }
            _ => {}
        }
    }
    all
}

pub fn write_all(all: &[(String, Review)]) -> String {
    let mut out = String::new();
    for (p, r) in all {
        out.push_str(&format!("review\t{p}\t{}\t{}\t{}\n", esc(&r.base), esc(&r.verdict), esc(&r.summary)));
        for c in &r.comments {
            out.push_str(&format!(
                "comment\t{p}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                c.id,
                esc(&c.path),
                c.line,
                if c.resolved { 1 } else { 0 },
                esc(&c.anchor),
                c.time,
                esc(&c.text)
            ));
            for (t, text) in &c.replies {
                out.push_str(&format!("reply\t{p}\t{}\t{t}\t{}\n", c.id, esc(text)));
            }
        }
    }
    out
}

/// Where the comment's line is now in `text`: the same line if it still reads the same, else
/// the nearest line that does; None when it's gone (the comment is outdated).
pub fn relocate(c: &Comment, text: &str) -> Option<u32> {
    let lines: Vec<&str> = text.lines().collect();
    let at = c.line.saturating_sub(1) as usize;
    if lines.get(at).is_some_and(|l| l.trim() == c.anchor) {
        return Some(c.line);
    }
    if c.anchor.is_empty() {
        return (at < lines.len()).then_some(c.line);
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == c.anchor)
        .min_by_key(|(i, _)| i.abs_diff(at))
        .map(|(i, _)| i as u32 + 1)
}

fn verdict_words(v: &str) -> &'static str {
    match v {
        "approve" => "Approved",
        "changes" => "Changes requested",
        _ => "Comments",
    }
}

/// The review as Markdown, for a pull request or an email.
pub fn markdown(r: &Review, files: &[(String, u32, u32)], when: impl Fn(i64) -> String) -> String {
    let base = if r.base.is_empty() { "the last commit".to_owned() } else { format!("`{}`", r.base) };
    let mut out = format!("## Review: {}\n\nCompared with {base}.\n\n", verdict_words(&r.verdict));
    if !r.summary.trim().is_empty() {
        out.push_str(r.summary.trim());
        out.push_str("\n\n");
    }
    if !files.is_empty() {
        out.push_str(&format!("{} file{} changed:\n\n", files.len(), if files.len() == 1 { "" } else { "s" }));
        for (p, a, d) in files {
            out.push_str(&format!("- `{p}` (+{a} −{d})\n"));
        }
        out.push('\n');
    }
    let open: Vec<&Comment> = r.comments.iter().filter(|c| !c.resolved).collect();
    if !open.is_empty() {
        out.push_str("### Comments\n\n");
        for c in open {
            out.push_str(&format!("- **`{}:{}`**", c.path, c.line));
            if !c.anchor.is_empty() {
                out.push_str(&format!(" `{}`", c.anchor.replace('`', "'")));
            }
            out.push_str(&format!("  \n  {}\n", c.text.trim().replace('\n', "\n  ")));
            for (t, text) in &c.replies {
                out.push_str(&format!("  - {} ({})\n", text.trim().replace('\n', " "), when(*t)));
            }
        }
    }
    let done = r.comments.iter().filter(|c| c.resolved).count();
    if done > 0 {
        out.push_str(&format!("\n{done} comment{} resolved.\n", if done == 1 { "" } else { "s" }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(line: u32, anchor: &str) -> Comment {
        Comment {
            id: 7,
            path: "src/a.rs".into(),
            line,
            anchor: anchor.into(),
            text: "Use a\tconstant\nhere".into(),
            time: 5,
            resolved: false,
            replies: vec![(6, "Done \\o/".into())],
        }
    }

    #[test]
    fn round_trip() {
        let r = Review {
            base: "main".into(),
            verdict: "changes".into(),
            summary: "Two\nthings".into(),
            comments: vec![comment(3, "let x = 42;")],
        };
        let other = Review { base: String::new(), ..Default::default() };
        let all = vec![("p1".to_owned(), r), ("p2".to_owned(), other)];
        assert_eq!(read_all(&write_all(&all)), all);
    }

    #[test]
    fn comments_follow_their_line() {
        let c = comment(2, "let x = 42;");
        assert_eq!(relocate(&c, "fn f() {\n    let x = 42;\n}\n"), Some(2));
        assert_eq!(relocate(&c, "// new\n// lines\nfn f() {\n    let x = 42;\n}\n"), Some(4));
        assert_eq!(relocate(&c, "fn f() {\n    let x = 43;\n}\n"), None, "the line is gone: outdated");
    }

    #[test]
    fn markdown_lists_open_comments() {
        let mut done = comment(9, "");
        done.resolved = true;
        done.id = 8;
        let r = Review {
            base: "main".into(),
            verdict: "approve".into(),
            summary: "Looks good.".into(),
            comments: vec![comment(3, "let x = 42;"), done],
        };
        let md = markdown(&r, &[("src/a.rs".into(), 4, 1)], |t| format!("t{t}"));
        assert!(
            md.starts_with("## Review: Approved\n\nCompared with `main`.\n\nLooks good.\n\n1 file changed:"),
            "{md}"
        );
        assert!(
            md.contains("- **`src/a.rs:3`** `let x = 42;`  \n  Use a\tconstant\n  here\n  - Done \\o/ (t6)\n"),
            "{md}"
        );
        assert!(md.contains("1 comment resolved."), "{md}");
    }
}
