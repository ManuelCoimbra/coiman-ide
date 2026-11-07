//! Tree-sitter queries as search, and as lint rules a project keeps in
//! `.coiman/rules.toml`. There are no types here: a rule sees syntax only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor, Tree};

use crate::language::syntax::Lang;
use crate::project::finder;

pub const RULES_FILE: &str = ".coiman/rules.toml";

/// Where a project's rules are: `.coiman/rules.toml`, or `.rust-code`'s from before.
pub fn rules_path(root: &std::path::Path) -> std::path::PathBuf {
    crate::settings::project_dir(root).join("rules.toml")
}
/// Larger files aren't read when walking a project.
const MAX_FILE: u64 = 2 * 1024 * 1024;
const MATCH_LIMIT: u32 = 1 << 16;
/// A search stops here: more isn't something to read through.
pub const MAX_HITS: usize = 5000;

/// A language by name (`rust`, `csharp`) or by extension (`rs`).
pub fn lang_named(name: &str) -> Option<Lang> {
    let named = Lang::ALL.iter().find(|l| format!("{l:?}").eq_ignore_ascii_case(name));
    named.copied().or_else(|| Lang::from_extension(&name.to_ascii_lowercase()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Severity {
    Error,
    #[default]
    Warning,
    Info,
}

#[derive(Deserialize)]
struct RawRule {
    id: String,
    language: String,
    query: String,
    message: String,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    fix: Option<String>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    rule: Vec<RawRule>,
}

/// A rule ready to run. `message` and `fix` can use `$capture`.
#[derive(Clone)]
pub struct Rule {
    pub id: String,
    pub lang: Lang,
    pub query: Arc<Query>,
    pub message: String,
    pub severity: Severity,
    pub fix: Option<String>,
}

/// Reads rules. One that's wrong is reported and the others still work.
pub fn parse_rules(text: &str) -> (Vec<Rule>, Vec<String>) {
    let file: RawFile = match toml::from_str(text) {
        Ok(file) => file,
        Err(why) => return (Vec::new(), vec![format!("{RULES_FILE}: {why}")]),
    };
    let (mut rules, mut problems) = (Vec::new(), Vec::new());
    for raw in file.rule {
        let Some(lang) = lang_named(&raw.language) else {
            problems.push(format!("Rule {}: no grammar for \"{}\"", raw.id, raw.language));
            continue;
        };
        let severity = match raw.severity.as_deref() {
            None | Some("warning") => Severity::Warning,
            Some("error") => Severity::Error,
            Some("info") => Severity::Info,
            Some(other) => {
                problems.push(format!("Rule {}: \"{other}\" isn't error, warning or info", raw.id));
                continue;
            }
        };
        match compile(lang, &raw.query) {
            Ok(query) => rules.push(Rule {
                id: raw.id,
                lang,
                query: Arc::new(query),
                message: raw.message,
                severity,
                fix: raw.fix,
            }),
            Err(why) => problems.push(format!("Rule {}: {why}", raw.id)),
        }
    }
    (rules, problems)
}

/// A query, for those holding on to one.
pub type CompiledQuery = Query;

pub fn compile(lang: Lang, query: &str) -> Result<Query, String> {
    Query::new(&lang.language(), query).map_err(|e| format!("line {}: {}", e.row + 1, e.message.trim()))
}

/// A place a query matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub start_byte: usize,
    pub end_byte: usize,
    /// Line and UTF-16 column, as language servers count.
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub captures: Vec<(String, String)>,
    pub line_text: String,
}

fn column(source: &str, row_start: usize, byte: usize) -> u32 {
    source[row_start..byte].encode_utf16().count() as u32
}

fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

/// What a query finds. It reports the capture named `match`, else the first.
pub fn scan(query: &Query, tree: &Tree, source: &str) -> Vec<Hit> {
    let names = query.capture_names();
    let main = names.iter().position(|n| *n == "match");
    let mut cursor = QueryCursor::new();
    cursor.set_match_limit(MATCH_LIMIT);
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    let mut hits: Vec<Hit> = Vec::new();
    while let Some(found) = matches.next() {
        let chosen = found.captures().iter().find(|c| main.is_none_or(|m| c.index as usize == m));
        let Some(chosen) = chosen else { continue };
        let node = chosen.node;
        if hits.iter().any(|h| h.start_byte == node.start_byte() && h.end_byte == node.end_byte()) {
            continue;
        }
        let (from, to) = (node.start_position(), node.end_position());
        let row_start = node.start_byte() - from.column;
        let row_end = node.end_byte() - to.column;
        let line_end = source[row_start..].find('\n').map_or(source.len(), |at| row_start + at);
        hits.push(Hit {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start: (from.row as u32, column(source, row_start, node.start_byte())),
            end: (to.row as u32, column(source, row_end, node.end_byte())),
            captures: found
                .captures()
                .iter()
                .map(|c| (names[c.index as usize].to_owned(), node_text(c.node, source).to_owned()))
                .collect(),
            line_text: source[row_start..line_end].trim().to_owned(),
        });
        if hits.len() >= MAX_HITS {
            break;
        }
    }
    hits
}

/// Puts captures in a message or a fix. Longer names first, so `$ab` isn't read as `$a`.
pub fn fill(template: &str, captures: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = captures.iter().collect();
    sorted.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        rest = &rest[at + 1..];
        match sorted.iter().find(|(name, _)| rest.starts_with(name.as_str())) {
            Some((name, value)) => {
                out.push_str(value);
                rest = &rest[name.len()..];
            }
            None => out.push('$'),
        }
    }
    out.push_str(rest);
    out
}

/// What replacing each match with `template` changes, as byte ranges in order. Where
/// matches overlap the outermost wins; a match already as it should be is left alone.
pub fn replacements(query: &Query, tree: &Tree, source: &str, template: &str) -> Vec<(usize, usize, String)> {
    let mut hits = scan(query, tree, source);
    hits.sort_by_key(|h| (h.start_byte, std::cmp::Reverse(h.end_byte)));
    let mut end = 0;
    let mut edits = Vec::new();
    for hit in hits {
        if hit.start_byte < end {
            continue;
        }
        end = hit.end_byte;
        let text = fill(template, &hit.captures);
        if source.get(hit.start_byte..hit.end_byte) != Some(text.as_str()) {
            edits.push((hit.start_byte, hit.end_byte, text));
        }
    }
    edits
}

pub fn parse(lang: Lang, source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.language()).ok()?;
    parser.parse(source, None)
}

/// A query to run over a project's files of a language.
#[derive(Clone)]
pub struct Search {
    pub lang: Lang,
    pub query: Arc<Query>,
}

/// What the searches found in a file: per hit, which search.
pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<(usize, Hit)>,
}

/// Runs queries over a project. Meant for a background thread.
///
/// `open` gives a file's unsaved text. Returns what was found, whether it stopped at
/// the cap, and how many files were too large to read.
pub fn scan_project(
    root: &Path,
    open: &dyn Fn(&Path) -> Option<String>,
    searches: &[Search],
    cancel: &AtomicBool,
) -> (Vec<FileHits>, bool, usize) {
    let (mut files, mut total, mut skipped) = (Vec::new(), 0, 0);
    for entry in finder::walker(root).build().flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let path = entry.path();
        let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        let Some(lang) = extension.and_then(|e| Lang::from_extension(&e)) else { continue };
        if !searches.iter().any(|s| s.lang == lang) {
            continue;
        }
        let source = match open(path) {
            Some(source) => source,
            None => {
                if entry.metadata().map_or(true, |m| m.len() > MAX_FILE) {
                    skipped += 1;
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(path) else { continue };
                source
            }
        };
        let Some(tree) = parse(lang, &source) else { continue };
        let mut hits = Vec::new();
        for (at, search) in searches.iter().enumerate().filter(|(_, s)| s.lang == lang) {
            hits.extend(scan(&search.query, &tree, &source).into_iter().map(|h| (at, h)));
        }
        if !hits.is_empty() {
            total += hits.len();
            files.push(FileHits { path: path.to_path_buf(), hits });
            if total >= MAX_HITS {
                return (files, true, skipped);
            }
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    (files, false, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNWRAP: &str = r#"(call_expression function: (field_expression value: (_) @on field: (field_identifier) @f) (#eq? @f "unwrap")) @match"#;

    #[test]
    fn finds_and_fills() {
        let source = "fn m() {\n    let é = a.unwrap(); b.unwrap_or(1); c.d().unwrap();\n}\n";
        let query = compile(Lang::Rust, UNWRAP).unwrap();
        let tree = parse(Lang::Rust, source).unwrap();
        let hits = scan(&query, &tree, source);
        assert_eq!(hits.len(), 2, "unwrap_or isn't unwrap");
        assert_eq!(hits[0].start, (1, 12), "columns count UTF-16, not bytes");
        assert_eq!(hits[0].end, (1, 22));
        assert!(hits[0].line_text.starts_with("let é"));
        assert_eq!(fill("$on? ($f) $x $", &hits[0].captures), "a? (unwrap) $x $");
        assert_eq!(fill("$on", &hits[1].captures), "c.d()");
    }

    #[test]
    fn works_out_replacements() {
        let source = "fn m() { a.unwrap(); b.c.unwrap(); }";
        let query = compile(Lang::Rust, UNWRAP).unwrap();
        let tree = parse(Lang::Rust, source).unwrap();
        let edits = replacements(&query, &tree, source, "$on?");
        assert_eq!(edits.len(), 2);
        assert_eq!(&source[edits[0].0..edits[0].1], "a.unwrap()");
        assert_eq!(edits[0].2, "a?");
        assert_eq!(edits[1].2, "b.c?");
        // Nested: the outer one is replaced, and its text holds the inner one as written.
        let nested = "fn m() { a.unwrap().unwrap(); }";
        let tree = parse(Lang::Rust, nested).unwrap();
        let edits = replacements(&query, &tree, nested, "$on?");
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].2, "a.unwrap()?");
        // Already so: nothing to do.
        let query = compile(Lang::Rust, "(identifier) @match").unwrap();
        let tree = parse(Lang::Rust, "fn m() { a; }").unwrap();
        assert!(replacements(&query, &tree, "fn m() { a; }", "$match").is_empty());
    }

    #[test]
    fn reads_rules_and_reports_the_bad_ones() {
        let text = format!(
            "[[rule]]\nid = \"ok\"\nlanguage = \"rust\"\nquery = '{UNWRAP}'\nmessage = \"m\"\nfix = \"$on?\"\n\
             [[rule]]\nid = \"typed\"\nlanguage = \"rs\"\nquery = '(nonsense) @x'\nmessage = \"m\"\n\
             [[rule]]\nid = \"unknown\"\nlanguage = \"cobol\"\nquery = '(x)'\nmessage = \"m\"\n\
             [[rule]]\nid = \"level\"\nlanguage = \"go\"\nquery = '(identifier) @x'\nmessage = \"m\"\nseverity = \"loud\"\n\
             [[rule]]\nid = \"err\"\nlanguage = \"Python\"\nquery = '(identifier) @x'\nmessage = \"m\"\nseverity = \"error\"\n"
        );
        let (rules, problems) = parse_rules(&text);
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["ok", "err"]);
        assert_eq!(rules[1].severity, Severity::Error);
        assert_eq!(rules[1].lang, Lang::Python);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("typed"));
        assert!(problems[1].contains("cobol"));
        assert!(problems[2].contains("loud"));
        let (none, problems) = parse_rules("not toml [");
        assert!(none.is_empty() && problems[0].contains(RULES_FILE));
    }

    #[test]
    fn walks_a_project() {
        let root = std::env::temp_dir().join(format!("rc-rules-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn m() { x.unwrap(); }").unwrap();
        std::fs::write(root.join("src/b.rs"), "fn m() {}").unwrap();
        std::fs::write(root.join("c.py"), "x.unwrap()").unwrap();
        let search = Search { lang: Lang::Rust, query: Arc::new(compile(Lang::Rust, UNWRAP).unwrap()) };
        let (files, capped, _) = scan_project(&root, &|_| None, std::slice::from_ref(&search), &AtomicBool::new(false));
        assert!(!capped);
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("src/a.rs"));
        let (files, _, _) = scan_project(
            &root,
            &|p| p.ends_with("src/b.rs").then(|| "fn m() { y.unwrap(); }".to_owned()),
            &[search],
            &AtomicBool::new(true),
        );
        assert!(files.is_empty(), "cancelled");
        let search = Search { lang: Lang::Rust, query: Arc::new(compile(Lang::Rust, UNWRAP).unwrap()) };
        let unsaved = |p: &Path| p.ends_with("src/b.rs").then(|| "fn m() { y.unwrap(); }".to_owned());
        let (files, _, _) = scan_project(&root, &unsaved, &[search], &AtomicBool::new(false));
        assert_eq!(files.len(), 2, "unsaved text counts");
        std::fs::remove_dir_all(root).unwrap();
    }
}
