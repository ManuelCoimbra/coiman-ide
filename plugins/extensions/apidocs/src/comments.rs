//! Doc comments read from source files, without the languages' own tools: Rust `///` and `//!`,
//! Python docstrings, `/** */` in JavaScript, TypeScript and Java, and `///` and `/** */` in
//! C and C++. Each comment comes with the line of code it documents, and the page made from
//! them links back to that line.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Python,
    Script,
    Java,
    C,
}

impl Lang {
    /// The fence name for code blocks.
    pub fn fence(self) -> &'static str {
        match self {
            Lang::Rust => "rust",
            Lang::Python => "python",
            Lang::Script => "ts",
            Lang::Java => "java",
            Lang::C => "cpp",
        }
    }
}

/// The file name endings read, for `list-files`.
pub const SUFFIXES: [&str; 18] = [
    ".rs", ".py", ".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts", ".java", ".c", ".h", ".cc", ".cpp",
    ".cxx", ".hpp", ".hh",
];

pub fn lang_of(path: &str) -> Option<Lang> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "rs" => Lang::Rust,
        "py" => Lang::Python,
        "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" => Lang::Script,
        "java" => Lang::Java,
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Lang::C,
        _ => return None,
    })
}

/// Files that aren't the project's own API: tests, bundles, generated code.
pub fn skipped(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit('/').next().unwrap_or(&p);
    p.split('/').any(|d| matches!(d, "tests" | "test" | "__tests__" | "node_modules" | "dist" | "build" | "vendor"))
        || name.contains(".min.")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.contains(".test.")
        || name.contains(".spec.")
        || name == "conftest.py"
        || name == "setup.py"
}

/// A doc comment and what it documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// 1-based: the line of the code documented, or of the comment for a file's own.
    pub line: u32,
    /// The code documented, on one line; empty for the file's own comment.
    pub sig: String,
    /// The comment, as Markdown.
    pub doc: String,
}

pub fn items(lang: Lang, text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    match lang {
        Lang::Python => python(&lines),
        _ => braces(lang, &lines),
    }
}

// ----- C-like languages ----------------------------------------------------------------------

fn braces(lang: Lang, lines: &[&str]) -> Vec<Item> {
    let mut out = Vec::new();
    let mut file_doc: Vec<String> = Vec::new();
    let mut file_line = 0;
    let slashes = matches!(lang, Lang::Rust | Lang::C);
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if lang == Lang::Rust && t.starts_with("//!") {
            if file_doc.is_empty() {
                file_line = i as u32 + 1;
            }
            file_doc.push(strip_one(&t[3..]).to_owned());
            i += 1;
            continue;
        }
        let start = i;
        let mut doc: Vec<String> = Vec::new();
        if slashes && is_triple_slash(t) {
            while i < lines.len() && is_triple_slash(lines[i].trim_start()) {
                doc.push(strip_one(&lines[i].trim_start()[3..]).to_owned());
                i += 1;
            }
        } else if t.starts_with("/**") && !t.starts_with("/**/") && !(lang == Lang::Rust && t.starts_with("/***")) {
            let mut first = &t[3..];
            loop {
                if let Some(end) = first.find("*/") {
                    doc.push(block_line(&first[..end]));
                    i += 1;
                    break;
                }
                doc.push(block_line(first));
                i += 1;
                if i >= lines.len() {
                    break;
                }
                first = lines[i];
            }
        } else {
            i += 1;
            continue;
        }
        let (sig, at) = signature(lang, lines, i);
        let doc = if lang == Lang::Rust { doc.join("\n") } else { tags(&doc) };
        let doc = tidy(&doc);
        if doc.is_empty() {
            continue;
        }
        let file_level = sig.is_empty() || is_preamble(lang, &sig);
        if file_level && out.is_empty() && file_doc.is_empty() {
            file_doc.push(doc);
            file_line = start as u32 + 1;
        } else if !file_level {
            out.push(Item { line: at as u32 + 1, sig, doc });
        }
    }
    if !file_doc.is_empty() {
        let doc = tidy(&file_doc.join("\n"));
        if !doc.is_empty() {
            out.insert(0, Item { line: file_line, sig: String::new(), doc });
        }
    }
    out
}

fn is_triple_slash(t: &str) -> bool {
    t.starts_with("///") && !t.starts_with("////")
}

/// One space after the comment marker goes.
fn strip_one(s: &str) -> &str {
    s.strip_prefix(' ').unwrap_or(s)
}

/// A line inside `/** */`, its leading `*` gone.
fn block_line(s: &str) -> String {
    let t = s.trim_start();
    let t = t.strip_prefix('*').filter(|_| !t.starts_with("*/")).unwrap_or(t);
    strip_one(t).trim_end().to_owned()
}

/// Code a comment at the top documents the file with, not an item.
fn is_preamble(lang: Lang, sig: &str) -> bool {
    match lang {
        Lang::Java => sig.starts_with("package ") || sig.starts_with("import "),
        Lang::Script => {
            sig.starts_with("import ") || sig.starts_with("'use strict'") || sig.starts_with("\"use strict\"")
        }
        Lang::C => sig.starts_with('#'),
        Lang::Rust => sig.starts_with("use ") || sig.starts_with("#!["),
        Lang::Python => false,
    }
}

/// The code from line `from` on, as one line, and the line it starts at.
fn signature(lang: Lang, lines: &[&str], from: usize) -> (String, usize) {
    let mut i = from;
    // Attributes, annotations and decorators belong with the item: past them.
    while i < lines.len() {
        let t = lines[i].trim();
        let attribute = match lang {
            Lang::Rust => t.starts_with("#[") && !t.starts_with("#!["),
            Lang::Java | Lang::Script => t.starts_with('@') && !t.starts_with("@interface"),
            Lang::C => t.starts_with("[["),
            Lang::Python => false,
        };
        if t.is_empty() || attribute || (t.starts_with("//") && !is_triple_slash(t)) {
            i += 1;
            continue;
        }
        break;
    }
    let at = i;
    let mut parts: Vec<&str> = Vec::new();
    while i < lines.len() && parts.len() < 4 {
        let t = lines[i].trim();
        if t.is_empty() || t.starts_with("/**") || is_triple_slash(t) {
            break;
        }
        let code = t.split(" //").next().unwrap_or(t);
        if let Some(cut) = code.find('{') {
            parts.push(&code[..cut]);
            break;
        }
        parts.push(code);
        // A comma ends a field or variant, not a parameter list going on.
        let open: usize = parts.iter().map(|p| p.matches('(').count()).sum();
        let close: usize = parts.iter().map(|p| p.matches(')').count()).sum();
        if code.ends_with(';') || (code.ends_with(',') && open <= close) || code.contains("=>") || code.ends_with('}') {
            break;
        }
        i += 1;
    }
    let sig = parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
    (sig.trim_end_matches([';', ',', '{', ' ']).to_owned(), at)
}

/// Javadoc and Doxygen tags as Markdown: `@param x what` as a bullet, `{@code x}` as code.
fn tags(lines: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_tags = false;
    for raw in lines {
        let l = inline_tags(raw.trim_end());
        let t = l.trim_start();
        let tag = t.strip_prefix('@').or_else(|| t.strip_prefix('\\'));
        if let Some(rest) = tag.filter(|r| r.starts_with(|c: char| c.is_ascii_alphabetic())) {
            let (name, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let rest = rest.trim();
            if name == "brief" {
                out.push(rest.to_owned());
                continue;
            }
            if !in_tags {
                if out.last().is_some_and(|l| !l.is_empty()) {
                    out.push(String::new());
                }
                in_tags = true;
            }
            out.push(match name {
                "param" | "tparam" | "arg" | "property" => {
                    let rest = rest.trim_start_matches(['{', '[']);
                    let (p, what) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    format!("- `{}`: {}", p.trim_end_matches(['}', ']']), what.trim())
                }
                "return" | "returns" => format!("- Returns: {rest}"),
                "throws" | "exception" | "throw" => {
                    let (e, what) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                    format!("- Throws `{e}`: {}", what.trim())
                }
                "deprecated" => format!("- Deprecated: {rest}"),
                "see" => format!("- See {rest}"),
                "since" => format!("- Since {rest}"),
                _ => format!("- {name}: {rest}"),
            });
        } else if in_tags && !t.is_empty() {
            // A tag's text going on.
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(t);
            }
        } else {
            out.push(html(&l));
        }
    }
    out.join("\n")
}

fn inline_tags(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("{@") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = &after[..end];
        let text = inner.split_once(char::is_whitespace).map_or("", |(_, t)| t.trim());
        out.push('`');
        out.push_str(text);
        out.push('`');
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The few HTML tags Javadoc uses for layout.
fn html(s: &str) -> String {
    let t = s.trim();
    if matches!(t.to_ascii_lowercase().as_str(), "<p>" | "</p>" | "<br>" | "<br/>" | "<ul>" | "</ul>") {
        return String::new();
    }
    s.replace("<p>", "\n")
        .replace("</p>", "")
        .replace("<li>", "- ")
        .replace("</li>", "")
        .replace("<code>", "`")
        .replace("</code>", "`")
}

/// Blank lines at the ends gone, and headings inside a comment made smaller than the page's.
fn tidy(doc: &str) -> String {
    let mut out = Vec::new();
    let mut fence = false;
    for l in doc.lines() {
        if l.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && l.starts_with('#') {
            out.push(format!("#### {}", l.trim_start_matches('#').trim()));
        } else {
            out.push(l.trim_end().to_owned());
        }
    }
    out.join("\n").trim_matches('\n').to_owned()
}

// ----- Python --------------------------------------------------------------------------------

fn python(lines: &[&str]) -> Vec<Item> {
    let mut out = Vec::new();
    // The module's docstring: its first statement.
    if let Some(first) = lines.iter().position(|l| {
        let t = l.trim();
        !t.is_empty() && !t.starts_with('#')
    }) && let Some((doc, _)) = docstring(lines, first)
        && !doc.is_empty()
    {
        out.push(Item { line: first as u32 + 1, sig: String::new(), doc });
    }
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        let def = t.starts_with("def ") || t.starts_with("async def ") || t.starts_with("class ");
        if !def {
            i += 1;
            continue;
        }
        let at = i;
        let mut parts = Vec::new();
        while i < lines.len() && parts.len() < 12 {
            let code = lines[i].split(" #").next().unwrap_or("").trim();
            parts.push(code);
            i += 1;
            if code.ends_with(':') {
                break;
            }
        }
        let sig = parts.join(" ").split_whitespace().collect::<Vec<_>>().join(" ");
        let sig = sig.trim_end_matches(':').replace("( ", "(").replace(" )", ")").replace(", )", ")");
        let name = sig.split_whitespace().find(|w| !matches!(*w, "def" | "async" | "class")).unwrap_or("");
        let name = name.split(['(', ':']).next().unwrap_or("");
        let private = name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__"));
        let Some(body) = (i..lines.len()).find(|&j| !lines[j].trim().is_empty()) else { break };
        if private {
            continue;
        }
        if let Some((doc, end)) = docstring(lines, body) {
            if !doc.is_empty() {
                out.push(Item { line: at as u32 + 1, sig, doc });
            }
            i = end;
        }
    }
    out
}

/// A string literal starting line `at` (a docstring), and the line after it.
fn docstring(lines: &[&str], at: usize) -> Option<(String, usize)> {
    let t = lines[at].trim_start();
    let body = t.trim_start_matches(['r', 'R', 'u', 'U']);
    let quote = ["\"\"\"", "'''", "\"", "'"].into_iter().find(|q| body.starts_with(q))?;
    let rest = &body[quote.len()..];
    if quote.len() == 1 {
        let end = rest.find(quote)?;
        return Some((rest[..end].trim().to_owned(), at + 1));
    }
    if let Some(end) = rest.find(quote) {
        return Some((rest[..end].trim().to_owned(), at + 1));
    }
    let mut body: Vec<&str> = vec![rest];
    let mut j = at + 1;
    while j < lines.len() {
        if let Some(end) = lines[j].find(quote) {
            body.push(&lines[j][..end]);
            return Some((dedent(&body), j + 1));
        }
        body.push(lines[j]);
        j += 1;
    }
    Some((dedent(&body), j))
}

/// The docstring's own indentation (after its first line) gone, as `inspect.cleandoc` does.
fn dedent(lines: &[&str]) -> String {
    let indent =
        lines[1..].iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
    let mut out = vec![lines[0].trim().to_owned()];
    out.extend(lines[1..].iter().map(|l| l.get(indent..).unwrap_or(l.trim_start()).trim_end().to_owned()));
    let text = out.join("\n");
    let text = text.trim_matches('\n');
    tidy(&rst(text))
}

/// reStructuredText's `Args:` sections and `::` blocks read well as they are; only its
/// double-backtick code needs one less.
fn rst(s: &str) -> String {
    s.replace("``", "`")
}

// ----- the page ------------------------------------------------------------------------------

/// The documented items' name: from the editor's parser where it knows the line, else from
/// the code.
pub fn name_of(sig: &str) -> String {
    const KEYWORDS: [&str; 22] = [
        "fn",
        "struct",
        "enum",
        "trait",
        "type",
        "const",
        "static",
        "mod",
        "union",
        "macro_rules!",
        "class",
        "interface",
        "def",
        "function",
        "namespace",
        "typedef",
        "record",
        "let",
        "var",
        "impl",
        "module",
        "@interface",
    ];
    let head = sig.split(['(', '<', '=', ':', '[']).next().unwrap_or(sig);
    let words: Vec<&str> = head.split_whitespace().collect();
    if let Some(k) = words.iter().position(|w| KEYWORDS.contains(w))
        && let Some(n) = words.get(k + 1)
    {
        let n = n.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'));
        if !n.is_empty() {
            return n.to_owned();
        }
    }
    // A method or field: the last word before its parameters or type.
    let last = words.last().copied().unwrap_or("");
    let last = last.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '~'));
    if last.is_empty() { sig.chars().take(40).collect() } else { last.to_owned() }
}

pub struct FileDocs {
    pub path: String,
    pub lang: Lang,
    /// Each item with its name.
    pub items: Vec<(Item, String)>,
}

/// One Markdown page of every file's doc comments, each linking to its line.
pub fn page(project: &str, files: &[FileDocs]) -> String {
    let count: usize = files.iter().map(|f| f.items.iter().filter(|(i, _)| !i.sig.is_empty()).count()).sum();
    let mut md = format!("# API of {project}\n\n");
    if files.is_empty() {
        md.push_str("No doc comments found in the project's files.\n");
        return md;
    }
    md.push_str(&format!(
        "From the doc comments in {} file{}: {count} documented item{}. Made by Coiman's API Docs; \
         each link opens the line in the code.\n",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        if count == 1 { "" } else { "s" },
    ));
    for f in files {
        md.push_str(&format!("\n## {}\n", f.path));
        for (item, name) in &f.items {
            let link = format!("[{}, line {}](/{}#L{})", f.path, item.line, f.path, item.line);
            if item.sig.is_empty() {
                md.push_str(&format!("\n{}\n", item.doc));
                continue;
            }
            md.push_str(&format!(
                "\n### {name}\n\n```{}\n{}\n```\n\n{}\n\n{link}\n",
                f.lang.fence(),
                item.sig,
                item.doc
            ));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_outer_and_inner_comments() {
        let src = "//! The crate.\n//! Second line.\n\nuse std::io;\n\n/// Adds two numbers.\n///\n/// # Examples\n/// ```\n/// # let x = 1;\n/// ```\n#[inline]\npub fn add(a: i32,\n    b: i32) -> i32 {\n    a + b\n}\n\nfn plain() {}\n\n/// A point.\npub struct Point { x: i32 }\n";
        let items = items(Lang::Rust, src);
        assert_eq!(items.len(), 3, "{items:?}");
        assert_eq!(items[0], Item { line: 1, sig: String::new(), doc: "The crate.\nSecond line.".into() });
        assert_eq!(items[1].line, 13);
        assert_eq!(items[1].sig, "pub fn add(a: i32, b: i32) -> i32");
        assert!(items[1].doc.contains("#### Examples\n```\n# let x = 1;\n```"), "{}", items[1].doc);
        assert_eq!(items[2].sig, "pub struct Point");
        assert_eq!(name_of(&items[1].sig), "add");
        assert_eq!(name_of(&items[2].sig), "Point");
    }

    #[test]
    fn javadoc_with_tags() {
        let src = "/**\n * The package's tool.\n */\npackage demo;\n\npublic class Calc {\n    /**\n     * Adds {@code a} and b.\n     * <p>\n     * @param a the first\n     * @param b the second,\n     *        added\n     * @return the sum\n     */\n    @Override\n    public int add(int a, int b) {\n        return a + b;\n    }\n}\n";
        let items = items(Lang::Java, src);
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(items[0].sig, "");
        assert_eq!(items[0].doc, "The package's tool.");
        assert_eq!(items[1].line, 16);
        assert_eq!(items[1].sig, "public int add(int a, int b)");
        assert_eq!(items[1].doc, "Adds `a` and b.\n\n- `a`: the first\n- `b`: the second, added\n- Returns: the sum");
        assert_eq!(name_of(&items[1].sig), "add");
    }

    #[test]
    fn typescript_and_c() {
        let ts = "/** Greets. @param name who */\nexport function greet(name: string): string {\n}\n/// <reference path=\"x\" />\n/**\n * A shape.\n */\nexport interface Shape {\n}\nexport const area = (s: Shape) => 0;\n";
        let items_ts = items(Lang::Script, ts);
        assert_eq!(items_ts.len(), 2, "{items_ts:?}");
        assert_eq!(items_ts[0].sig, "export function greet(name: string): string");
        assert_eq!(items_ts[0].doc, "Greets. @param name who");
        assert_eq!(name_of(&items_ts[1].sig), "Shape");
        let c = "#include <stdio.h>\n\n/// Opens it.\n/// \\param path where\nint open_it(const char *path);\n/** The size. */\nsize_t size;\n";
        let items_c = items(Lang::C, c);
        assert_eq!(items_c.len(), 2, "{items_c:?}");
        assert_eq!(items_c[0].sig, "int open_it(const char *path)");
        assert_eq!(items_c[0].doc, "Opens it.\n\n- `path`: where");
        assert_eq!(name_of(&items_c[0].sig), "open_it");
        assert_eq!(items_c[1].sig, "size_t size");
        assert_eq!(name_of(&items_c[1].sig), "size");
    }

    #[test]
    fn python_docstrings() {
        let src = "#!/usr/bin/env python\n\"\"\"The module.\n\nMore.\n\"\"\"\nimport os\n\ndef add(a,\n        b):  # adds\n    \"\"\"Adds two numbers.\n\n    Uses ``+``.\n    \"\"\"\n    return a + b\n\ndef _hidden():\n    \"\"\"Not shown.\"\"\"\n\nclass Thing(Base):\n    '''A thing.'''\n\n    async def run(self) -> None:\n        r\"\"\"Runs it.\"\"\"\n\n    def __init__(self):\n        pass\n";
        let items = items(Lang::Python, src);
        assert_eq!(items.len(), 4, "{items:?}");
        assert_eq!(items[0], Item { line: 2, sig: String::new(), doc: "The module.\n\nMore.".into() });
        assert_eq!(items[1].line, 8);
        assert_eq!(items[1].sig, "def add(a, b)");
        assert_eq!(items[1].doc, "Adds two numbers.\n\nUses `+`.");
        assert_eq!(items[2].sig, "class Thing(Base)");
        assert_eq!(items[2].doc, "A thing.");
        assert_eq!(items[3].sig, "async def run(self) -> None");
        assert_eq!(name_of(&items[3].sig), "run");
    }

    #[test]
    fn languages_and_skipped_files() {
        assert_eq!(lang_of("src/a.RS"), Some(Lang::Rust));
        assert_eq!(lang_of("x.hpp"), Some(Lang::C));
        assert_eq!(lang_of("README.md"), None);
        assert!(skipped("tests/it.rs") && skipped("web/app.min.js") && skipped("pkg/test_x.py"));
        assert!(skipped("src/a.test.ts") && !skipped("src/lib.rs"));
    }

    #[test]
    fn page_links_to_lines() {
        let item = Item { line: 3, sig: "pub fn add()".into(), doc: "Adds.".into() };
        let module = Item { line: 1, sig: String::new(), doc: "The crate.".into() };
        let files = [FileDocs {
            path: "src/lib.rs".into(),
            lang: Lang::Rust,
            items: vec![(module, String::new()), (item, "add".into())],
        }];
        let md = page("demo", &files);
        assert!(md.starts_with("# API of demo\n\nFrom the doc comments in 1 file: 1 documented item."), "{md}");
        assert!(md.contains("\n## src/lib.rs\n\nThe crate.\n"), "{md}");
        assert!(
            md.contains("\n### add\n\n```rust\npub fn add()\n```\n\nAdds.\n\n[src/lib.rs, line 3](/src/lib.rs#L3)\n"),
            "{md}"
        );
        assert!(page("x", &[]).contains("No doc comments"));
    }
}
