//! A project's Markdown documents read without a build: their titles and headings, the links
//! between them (and the broken ones), and the files they mention, to find documents older
//! than the code they describe.

use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    /// 1-based.
    pub line: usize,
    /// The anchor a link uses for it (`#getting-started`), as GitHub makes it.
    pub slug: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub line: usize,
    /// As written.
    pub target: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// Relative to the project, with `/`.
    pub path: String,
    pub title: String,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
    /// Paths named in inline code (`src/main.rs`), with their lines.
    pub mentions: Vec<(usize, String)>,
    pub words: usize,
}

pub fn is_doc(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md") || p.ends_with(".markdown") || p.ends_with(".mdx")
}

/// GitHub's anchor for a heading: lower case, punctuation dropped, spaces as `-`.
pub fn slug(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// A heading's text without its inline marks (`**`, `` ` ``, links' targets).
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            '[' => {
                let label: String = chars.by_ref().take_while(|c| *c != ']').collect();
                out.push_str(&label);
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
            }
            c => out.push(c),
        }
    }
    out.trim().trim_end_matches('#').trim().to_owned()
}

/// Inline code that names a file: no spaces, a `/` or a known extension.
fn path_like(code: &str) -> bool {
    !code.is_empty()
        && !code.contains([' ', '(', '<', '{', '=', '*', '$'])
        && !code.starts_with(['-', '.', '@', '#'])
        && !code.contains("://")
        && code.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty() && (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
        })
}

/// One document's text, read.
pub fn read(path: &str, text: &str) -> Doc {
    let mut doc = Doc { path: path.to_owned(), ..Doc::default() };
    let mut fence: Option<&str> = None;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut lines = text.lines().enumerate().peekable();
    // Front matter: a `title:` there names it.
    if text.starts_with("---") {
        lines.next();
        for (_, l) in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
            if let Some(t) = l.strip_prefix("title:") {
                doc.title = t.trim().trim_matches(['"', '\'']).to_owned();
            }
        }
    }
    while let Some((i, line)) = lines.next() {
        let t = line.trim_start();
        if let Some(f) = fence {
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            continue;
        }
        doc.words += line.split_whitespace().count();
        // `# Heading`, or a line underlined with === or ---.
        let hashes = t.chars().take_while(|c| *c == '#').count();
        let underline = lines.peek().map(|(_, n)| n.trim()).filter(|n| {
            !t.is_empty()
                && !n.is_empty()
                && (n.chars().all(|c| c == '=') || (n.chars().all(|c| c == '-') && n.len() >= 2))
        });
        let heading = if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            Some((hashes, plain(&t[hashes..])))
        } else if let Some(u) = underline {
            let level = if u.starts_with('=') { 1 } else { 2 };
            lines.next();
            Some((level, plain(t)))
        } else {
            None
        };
        if let Some((level, text)) = heading {
            let base = slug(&text);
            let n = seen.entry(base.clone()).or_insert(0);
            let slug = if *n == 0 { base.clone() } else { format!("{base}-{n}") };
            *n += 1;
            if doc.title.is_empty() && level == 1 {
                doc.title = text.clone();
            }
            doc.headings.push(Heading { level, text, line: i + 1, slug });
            continue;
        }
        scan_inline(line, i + 1, &mut doc);
    }
    if doc.title.is_empty() {
        doc.title =
            doc.headings.first().map_or_else(|| path.rsplit('/').next().unwrap_or(path).to_owned(), |h| h.text.clone());
    }
    doc
}

/// Links (`[x](target)`, `<a href>`, `[x]: target`) and inline code naming files.
fn scan_inline(line: &str, n: usize, doc: &mut Doc) {
    let t = line.trim_start();
    // A reference definition: `[name]: target`.
    if t.starts_with('[')
        && let Some(end) = t.find("]:")
        && !t[..end].contains(']')
    {
        if let Some(target) = t[end + 2..].split_whitespace().next() {
            doc.links.push(Link { line: n, target: target.trim_matches(['<', '>']).to_owned() });
        }
        return;
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => {
                let ticks = line[i..].bytes().take_while(|b| *b == b'`').count();
                let fence = &line[i..i + ticks];
                if let Some(end) = line[i + ticks..].find(fence) {
                    let code = line[i + ticks..i + ticks + end].trim();
                    if path_like(code) {
                        doc.mentions.push((n, code.trim_start_matches("./").to_owned()));
                    }
                    i += ticks + end + ticks;
                    continue;
                }
                i += ticks;
            }
            b']' if bytes.get(i + 1) == Some(&b'(') => {
                if let Some(end) = line[i + 2..].find(')') {
                    let inner = line[i + 2..i + 2 + end].trim();
                    let target = inner.split_whitespace().next().unwrap_or("").trim_matches(['<', '>']);
                    if !target.is_empty() {
                        doc.links.push(Link { line: n, target: target.to_owned() });
                    }
                    i += 2 + end;
                    continue;
                }
                i += 1;
            }
            b'h' if line[i..].starts_with("href=\"") => {
                let rest = &line[i + 6..];
                if let Some(end) = rest.find('"') {
                    doc.links.push(Link { line: n, target: rest[..end].to_owned() });
                    i += 6 + end;
                    continue;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
}

/// Where a link goes: a path relative to the project and an anchor, or None for the web,
/// mail and the like.
pub fn resolve(from: &str, target: &str) -> Option<(String, String)> {
    let t = target.trim();
    if t.is_empty() || t.contains("://") || t.starts_with("mailto:") || t.starts_with("tel:") || t.starts_with("data:")
    {
        return None;
    }
    let (path, anchor) = t.split_once('#').unwrap_or((t, ""));
    let path = path.split('?').next().unwrap_or("").replace("%20", " ");
    if path.is_empty() {
        return Some((from.to_owned(), anchor.to_owned()));
    }
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        from.split('/').collect::<Vec<_>>()[..from.matches('/').count()].to_vec()
    };
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some((parts.join("/"), anchor.to_owned()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Broken {
    pub doc: String,
    pub line: usize,
    pub target: String,
    pub why: String,
}

/// The links that lead nowhere: a file that isn't there, or an anchor the document hasn't.
pub fn broken(docs: &[Doc], files: &BTreeSet<String>) -> Vec<Broken> {
    let by_path: HashMap<&str, &Doc> = docs.iter().map(|d| (d.path.as_str(), d)).collect();
    let mut out = Vec::new();
    for d in docs {
        for l in &d.links {
            let Some((path, anchor)) = resolve(&d.path, &l.target) else { continue };
            let dir = format!("{path}/");
            let exists = files.contains(&path)
                || path.is_empty()
                || files.range(dir.clone()..).next().is_some_and(|f| f.starts_with(&dir));
            let why = if !exists {
                Some(format!("{path} isn't there"))
            } else if !anchor.is_empty() {
                by_path.get(path.as_str()).and_then(|t| {
                    let a = anchor.to_lowercase();
                    (!t.headings.iter().any(|h| h.slug == a)).then(|| format!("{} has no heading #{anchor}", t.path))
                })
            } else {
                None
            };
            if let Some(why) = why {
                out.push(Broken { doc: d.path.clone(), line: l.line, target: l.target.clone(), why });
            }
        }
    }
    out
}

/// The links between documents: (from, to) once each.
pub fn doc_links(docs: &[Doc]) -> BTreeSet<(String, String)> {
    let paths: BTreeSet<&str> = docs.iter().map(|d| d.path.as_str()).collect();
    let mut out = BTreeSet::new();
    for d in docs {
        for l in &d.links {
            if let Some((p, _)) = resolve(&d.path, &l.target)
                && p != d.path
                && paths.contains(p.as_str())
            {
                out.insert((d.path.clone(), p));
            }
        }
    }
    out
}

/// Documents no other document links to (READMEs aside: they're where reading starts).
pub fn orphans(docs: &[Doc]) -> Vec<String> {
    let linked: BTreeSet<String> = doc_links(docs).into_iter().map(|(_, to)| to).collect();
    docs.iter()
        .map(|d| d.path.clone())
        .filter(|p| !linked.contains(p) && !p.rsplit('/').next().unwrap_or(p).to_lowercase().starts_with("readme"))
        .collect()
}

/// `git log --name-only --no-renames --format=%x00%ct`: when each path last changed.
pub fn last_changes(log: &str) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let mut time = 0;
    for line in log.lines() {
        if let Some(t) = line.strip_prefix('\0') {
            time = t.trim().parse().unwrap_or(0);
        } else if !line.trim().is_empty() {
            out.entry(line.trim().to_owned()).or_insert(time);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stale {
    pub doc: String,
    pub line: usize,
    /// The file it mentions or links to.
    pub file: String,
    pub doc_changed: i64,
    pub file_changed: i64,
}

/// Documents that mention or link to a file (not another document) changed after they were.
pub fn outdated(docs: &[Doc], changed: &HashMap<String, i64>) -> Vec<Stale> {
    let mut out = Vec::new();
    for d in docs {
        let Some(&doc_time) = changed.get(&d.path) else { continue };
        let mut named: BTreeMap<String, usize> = BTreeMap::new();
        for (line, m) in &d.mentions {
            named.entry(m.clone()).or_insert(*line);
        }
        for l in &d.links {
            if let Some((p, _)) = resolve(&d.path, &l.target) {
                named.entry(p).or_insert(l.line);
            }
        }
        for (file, line) in named {
            if is_doc(&file) {
                continue;
            }
            // A mention may be relative to the document or to the project.
            let here = resolve(&d.path, &file).map(|(p, _)| p);
            let time = changed.get(&file).or_else(|| here.as_ref().and_then(|h| changed.get(h)));
            if let Some(&t) = time
                && t > doc_time
            {
                out.push(Stale { doc: d.path.clone(), line, file, doc_changed: doc_time, file_changed: t });
            }
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.file_changed - x.doc_changed));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUIDE: &str = "---\ntitle: The Guide\n---\n# Guide\n\nSee [setup](setup.md#install) and [api](../api/README.md).\n\
        ```sh\n# not a heading\n[not](a-link.md)\n```\n## Setting **it** up\nRun `src/main.rs` or `cargo run`.\n\
        ## Setting it up\n[top](#guide) [web](https://example.com) [gone](missing.md) [bad](#nowhere)\n\
        <a href=\"setup.md\">x</a>\n[ref]: ./setup.md\nOther\n-----\n";

    #[test]
    fn reads_a_document() {
        let d = read("docs/guide.md", GUIDE);
        assert_eq!(d.title, "The Guide");
        let h: Vec<(usize, &str, &str)> =
            d.headings.iter().map(|h| (h.level, h.text.as_str(), h.slug.as_str())).collect();
        assert_eq!(
            h,
            [
                (1, "Guide", "guide"),
                (2, "Setting it up", "setting-it-up"),
                (2, "Setting it up", "setting-it-up-1"),
                (2, "Other", "other")
            ]
        );
        let targets: Vec<&str> = d.links.iter().map(|l| l.target.as_str()).collect();
        assert_eq!(
            targets,
            [
                "setup.md#install",
                "../api/README.md",
                "#guide",
                "https://example.com",
                "missing.md",
                "#nowhere",
                "setup.md",
                "./setup.md"
            ]
        );
        assert_eq!(d.mentions, [(12, "src/main.rs".to_owned())]);
        assert_eq!(read("x/notes.md", "no heading").title, "notes.md");
    }

    #[test]
    fn links_resolve_and_break() {
        assert_eq!(resolve("docs/guide.md", "../api/README.md#x"), Some(("api/README.md".into(), "x".into())));
        assert_eq!(resolve("docs/guide.md", "#top"), Some(("docs/guide.md".into(), "top".into())));
        assert_eq!(resolve("docs/guide.md", "/README.md"), Some(("README.md".into(), String::new())));
        assert_eq!(resolve("README.md", "../outside.md"), None, "outside the project");
        assert_eq!(resolve("a.md", "https://x.org"), None);
        let docs = vec![
            read("docs/guide.md", GUIDE),
            read("docs/setup.md", "# Setup\n## Install\n"),
            read("api/README.md", "# API\n"),
        ];
        let files: BTreeSet<String> =
            ["docs/guide.md", "docs/setup.md", "api/README.md", "src/main.rs"].map(str::to_owned).into();
        let why: Vec<(usize, String)> = broken(&docs, &files).into_iter().map(|b| (b.line, b.why)).collect();
        assert_eq!(
            why,
            [(14, "docs/missing.md isn't there".to_owned()), (14, "docs/guide.md has no heading #nowhere".to_owned())]
        );
        let links = doc_links(&docs);
        assert!(links.contains(&("docs/guide.md".into(), "docs/setup.md".into())));
        assert!(links.contains(&("docs/guide.md".into(), "api/README.md".into())));
        assert_eq!(orphans(&docs), ["docs/guide.md"]);
    }

    #[test]
    fn older_than_the_code() {
        let log = "\x001700\nsrc/main.rs\ndocs/guide.md\n\x001500\nsrc/main.rs\ndocs/guide.md\nsrc/old.rs\n";
        let changed = last_changes(log);
        assert_eq!(changed["src/main.rs"], 1700);
        assert_eq!(changed["src/old.rs"], 1500);
        let mut changed = changed;
        changed.insert("docs/guide.md".into(), 1600);
        let docs = vec![read("docs/guide.md", GUIDE)];
        let stale = outdated(&docs, &changed);
        assert_eq!(stale.len(), 1);
        assert_eq!((stale[0].file.as_str(), stale[0].line, stale[0].file_changed), ("src/main.rs", 12, 1700));
    }
}

#[cfg(test)]
mod this_repo {
    /// This repository's documents: `cargo test --lib this_repo -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn this_repo() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let out = std::process::Command::new("git").args(["ls-files"]).current_dir(&root).output().unwrap();
        let files: std::collections::BTreeSet<String> =
            String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect();
        let docs: Vec<super::Doc> = files
            .iter()
            .filter(|f| super::is_doc(f))
            .map(|f| super::read(f, &std::fs::read_to_string(root.join(f)).unwrap_or_default()))
            .collect();
        println!("{} docs", docs.len());
        for b in super::broken(&docs, &files) {
            println!("broken {}:{} {} ({})", b.doc, b.line, b.target, b.why);
        }
        println!("orphans {:?}", super::orphans(&docs));
    }
}
