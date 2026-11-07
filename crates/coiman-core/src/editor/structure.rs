//! Code structure from tree-sitter: fold ranges, outline symbols, expanding a
//! selection to the enclosing syntax node, and text objects (function, type,
//! argument, comment). Files without a grammar fold by indentation.

use ropey::Rope;
use tree_sitter::{Node, Tree};

use crate::language::syntax::Lang;

/// A foldable region: folding hides lines `start + 1 ..= end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoldRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Constant,
    TypeAlias,
    Macro,
    Field,
    Variant,
    Heading,
    Table,
    Key,
    Class,
    Interface,
    Method,
}

impl SymbolKind {
    /// Short label, e.g. "function".
    pub fn word(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "module",
            SymbolKind::Constant => "constant",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Macro => "macro",
            SymbolKind::Field => "field",
            SymbolKind::Variant => "variant",
            SymbolKind::Heading => "#",
            SymbolKind::Table => "table",
            SymbolKind::Key => "key",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Method => "method",
        }
    }
}

/// An outline entry. The outline is a flat list in document order; `depth`
/// gives the nesting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub depth: usize,
    /// Line of the name (where "go to symbol" lands).
    pub line: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// Last line a node really covers (a node ending at column 0 ends the line before).
fn last_row(node: Node) -> usize {
    let end = node.end_position();
    if end.column == 0 && end.row > node.start_position().row { end.row - 1 } else { end.row }
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

/// Fold ranges from a syntax tree: bracketed blocks (`{}` `[]` `()`), runs of
/// comments, Markdown sections, TOML tables.
pub fn folds_from_tree(tree: &Tree, lang: Lang) -> Vec<FoldRange> {
    let mut out: Vec<FoldRange> = Vec::new();
    let mut cursor = tree.walk();
    let mut comment_run: Option<(usize, usize)> = None;
    'walk: loop {
        let node = cursor.node();
        let (start, end) = (node.start_position().row, last_row(node));
        let kind = node.kind();
        let is_line_comment = kind == "line_comment" || (kind == "comment" && start == end);
        if is_line_comment {
            comment_run = match comment_run {
                Some((s, e)) if e + 1 == start => Some((s, start)),
                Some((s, e)) => {
                    if e > s {
                        out.push(FoldRange { start: s, end: e });
                    }
                    Some((start, start))
                }
                None => Some((start, start)),
            };
        } else if end > start {
            let n = node.child_count();
            let first = node.child(0).map(|c| c.kind());
            let last = n.checked_sub(1).and_then(|i| node.child(i)).map(|c| c.kind());
            let bracketed = n >= 2 && matches!(first, Some("{" | "[" | "(")) && matches!(last, Some("}" | "]" | ")"));
            let folding = match lang {
                // Keep the closing bracket's line visible, like VS Code.
                _ if bracketed => (end > start + 1).then_some(end - 1),
                _ if kind == "block_comment" || kind == "comment" => Some(end),
                Lang::Markdown if kind == "section" || kind == "fenced_code_block" => Some(end),
                Lang::Toml if kind == "table" || kind == "table_array_element" => Some(end),
                Lang::Yaml if kind == "block_mapping_pair" || kind == "block_sequence_item" => Some(end),
                _ => None,
            };
            if let Some(e) = folding.filter(|e| *e > start) {
                out.push(FoldRange { start, end: e });
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    if let Some((s, e)) = comment_run.filter(|(s, e)| e > s) {
        out.push(FoldRange { start: s, end: e });
    }
    normalize(out)
}

/// One fold per start line (the largest), sorted.
fn normalize(mut folds: Vec<FoldRange>) -> Vec<FoldRange> {
    folds.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    folds.dedup_by_key(|f| f.start);
    folds
}

/// Fold ranges by indentation: a line followed by more-indented lines.
pub fn indent_folds(rope: &Rope) -> Vec<FoldRange> {
    let indent = |line: usize| -> Option<usize> {
        let mut cols = 0;
        for c in rope.line(line).chars() {
            match c {
                ' ' => cols += 1,
                '\t' => cols += 4,
                '\n' | '\r' => return None,
                _ => return Some(cols),
            }
        }
        None // blank
    };
    let mut out = Vec::new();
    // Open regions: (indent of the header line, header line, last non-blank line inside).
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();
    let mut prev: Option<(usize, usize)> = None;
    for line in 0..rope.len_lines() {
        let Some(ind) = indent(line) else { continue };
        while let Some(&(h_ind, h_line, last)) = stack.last() {
            if ind > h_ind {
                break;
            }
            stack.pop();
            if last > h_line {
                out.push(FoldRange { start: h_line, end: last });
            }
        }
        if let Some((p_ind, p_line)) = prev
            && ind > p_ind
            && stack.last().is_none_or(|s| s.1 != p_line)
        {
            stack.push((p_ind, p_line, line));
        }
        for s in &mut stack {
            s.2 = line;
        }
        prev = Some((ind, line));
    }
    for (_, h_line, last) in stack {
        if last > h_line {
            out.push(FoldRange { start: h_line, end: last });
        }
    }
    normalize(out)
}

/// The outline of a document.
pub fn symbols(tree: &Tree, src: &str, lang: Lang) -> Vec<Symbol> {
    let mut out = Vec::new();
    match lang {
        Lang::Rust => rust_symbols(tree.root_node(), src, 0, &mut out),
        Lang::Markdown => markdown_symbols(tree.root_node(), src, &mut out),
        Lang::Toml | Lang::Json | Lang::Yaml => key_symbols(tree.root_node(), src, lang, 0, &mut out),
        _ => tag_symbols(tree, src, lang, &mut out),
    }
    out
}

/// The outline from a grammar's tags query: each `@definition.*` capture with
/// its `@name`, nested by which definitions contain which.
fn tag_symbols(tree: &Tree, src: &str, lang: Lang, out: &mut Vec<Symbol>) {
    use std::sync::OnceLock;
    use streaming_iterator::StreamingIterator;
    static QUERIES: [OnceLock<Option<tree_sitter::Query>>; Lang::ALL.len()] =
        [const { OnceLock::new() }; Lang::ALL.len()];
    let Some(query) = QUERIES[lang as usize]
        .get_or_init(|| lang.tags().and_then(|q| tree_sitter::Query::new(&lang.language(), &q).ok()))
        .as_ref()
    else {
        return;
    };
    let names = query.capture_names();
    let mut found: Vec<(Node, Node, SymbolKind)> = Vec::new();
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), src.as_bytes());
    while let Some(m) = matches.next() {
        let mut def = None;
        let mut name = None;
        for c in m.captures() {
            let capture = names[c.index as usize];
            if capture == "name" {
                name = Some(c.node);
            } else if let Some(kind) = capture.strip_prefix("definition.") {
                let kind = match kind {
                    "class" => SymbolKind::Class,
                    "interface" => SymbolKind::Interface,
                    "method" => SymbolKind::Method,
                    "module" => SymbolKind::Module,
                    "constant" => SymbolKind::Constant,
                    "type" => SymbolKind::TypeAlias,
                    "macro" => SymbolKind::Macro,
                    _ => SymbolKind::Function,
                };
                def = Some((c.node, kind));
            }
        }
        if let (Some((node, kind)), Some(name)) = (def, name)
            && !found.iter().any(|(n, _, _)| n.id() == node.id())
        {
            found.push((node, name, kind));
        }
    }
    found.sort_by_key(|(n, _, _)| (n.start_byte(), std::cmp::Reverse(n.end_byte())));
    // Ends of the definitions enclosing the current one.
    let mut open: Vec<usize> = Vec::new();
    for (node, name, kind) in found {
        while open.last().is_some_and(|&end| end <= node.start_byte()) {
            open.pop();
        }
        push(out, node, text(name, src).to_owned(), kind, open.len(), Some(name));
        open.push(node.end_byte());
    }
}

fn push(out: &mut Vec<Symbol>, node: Node, name: String, kind: SymbolKind, depth: usize, name_node: Option<Node>) {
    let line = name_node.unwrap_or(node).start_position().row;
    out.push(Symbol { name, kind, depth, line, start_line: node.start_position().row, end_line: last_row(node) });
}

fn rust_symbols(node: Node, src: &str, depth: usize, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let name_node = child.child_by_field_name("name");
        let name = name_node.map(|n| text(n, src).to_owned()).unwrap_or_default();
        let kind = match child.kind() {
            "function_item" | "function_signature_item" => SymbolKind::Function,
            "struct_item" | "union_item" => SymbolKind::Struct,
            "enum_item" => SymbolKind::Enum,
            "trait_item" => SymbolKind::Trait,
            "mod_item" => SymbolKind::Module,
            "const_item" | "static_item" => SymbolKind::Constant,
            "type_item" | "associated_type" => SymbolKind::TypeAlias,
            "macro_definition" => SymbolKind::Macro,
            "field_declaration" => SymbolKind::Field,
            "enum_variant" => SymbolKind::Variant,
            "impl_item" => {
                let ty = child.child_by_field_name("type").map_or("", |n| text(n, src));
                let name = match child.child_by_field_name("trait") {
                    Some(t) => format!("{} for {ty}", text(t, src)),
                    None => ty.to_owned(),
                };
                push(out, child, name, SymbolKind::Impl, depth, None);
                if let Some(body) = child.child_by_field_name("body") {
                    rust_symbols(body, src, depth + 1, out);
                }
                continue;
            }
            // Look inside wrappers like `declaration_list` of a mod without a symbol of their own.
            _ => continue,
        };
        push(out, child, name, kind, depth, name_node);
        // Members: struct fields, enum variants, trait items, module contents.
        if matches!(kind, SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait | SymbolKind::Module)
            && let Some(body) = child.child_by_field_name("body")
        {
            rust_symbols(body, src, depth + 1, out);
        }
    }
}

fn markdown_symbols(node: Node, src: &str, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "atx_heading" || child.kind() == "setext_heading" {
            let raw = text(child, src).lines().next().unwrap_or("");
            let level = raw.chars().take_while(|c| *c == '#').count().max(1);
            let name = raw.trim_start_matches('#').trim().to_owned();
            let section = child.parent().filter(|p| p.kind() == "section").unwrap_or(child);
            out.push(Symbol {
                name,
                kind: SymbolKind::Heading,
                depth: level - 1,
                line: child.start_position().row,
                start_line: section.start_position().row,
                end_line: last_row(section),
            });
        }
        if child.named_child_count() > 0 {
            markdown_symbols(child, src, out);
        }
    }
}

/// Tables and keys (TOML), object keys (JSON), mapping keys (YAML), two levels deep.
fn key_symbols(node: Node, src: &str, lang: Lang, depth: usize, out: &mut Vec<Symbol>) {
    if depth > 1 {
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match (lang, child.kind()) {
            (Lang::Toml, "table" | "table_array_element") => {
                let header = text(child, src).lines().next().unwrap_or("").trim().to_owned();
                push(out, child, header, SymbolKind::Table, 0, None);
                key_symbols(child, src, lang, 1, out);
            }
            (Lang::Toml, "pair") => {
                let key = child.named_child(0).map_or("", |k| text(k, src)).to_owned();
                push(out, child, key, SymbolKind::Key, depth, None);
            }
            (Lang::Json, "pair") => {
                let key = child.child_by_field_name("key").map_or("", |k| text(k, src)).trim_matches('"').to_owned();
                push(out, child, key, SymbolKind::Key, depth, None);
                if let Some(value) = child.child_by_field_name("value") {
                    key_symbols(value, src, lang, depth + 1, out);
                }
            }
            (Lang::Yaml, "block_mapping_pair") => {
                let key = child.child_by_field_name("key").map_or("", |k| text(k, src)).to_owned();
                push(out, child, key, SymbolKind::Key, depth, None);
                if let Some(value) = child.child_by_field_name("value") {
                    key_symbols(value, src, lang, depth + 1, out);
                }
            }
            (Lang::Json, "object" | "document") | (Lang::Yaml, _) => key_symbols(child, src, lang, depth, out),
            _ => {}
        }
    }
}

fn to_bytes(rope: &Rope, start: usize, end: usize) -> (usize, usize) {
    (rope.char_to_byte(start.min(rope.len_chars())), rope.char_to_byte(end.min(rope.len_chars())))
}

fn to_chars(rope: &Rope, node: Node) -> (usize, usize) {
    let len = rope.len_bytes();
    (rope.byte_to_char(node.start_byte().min(len)), rope.byte_to_char(node.end_byte().min(len)))
}

/// The smallest syntax node strictly larger than the selection `start..end` (chars).
pub fn expand_selection(tree: &Tree, rope: &Rope, start: usize, end: usize) -> Option<(usize, usize)> {
    let (sb, eb) = to_bytes(rope, start, end);
    let mut node = tree.root_node().named_descendant_for_byte_range(sb, eb)?;
    while node.start_byte() == sb && node.end_byte() == eb {
        node = node.parent()?;
    }
    Some(to_chars(rope, node))
}

/// Syntax-aware selections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextObject {
    /// The whole function, with its attributes and doc comments.
    Function,
    /// The inside of the function's body, without the braces.
    FunctionBody,
    /// A struct, enum, union, trait, impl or type alias.
    Type,
    /// One parameter or argument.
    Argument,
    /// A comment (a run of line comments together).
    Comment,
}

const FUNCTIONS: &[&str] = &["function_item", "closure_expression", "function_signature_item"];
const TYPES: &[&str] = &["struct_item", "enum_item", "union_item", "trait_item", "impl_item", "type_item"];

fn ancestor<'t>(mut node: Node<'t>, kinds: &[&str]) -> Option<Node<'t>> {
    loop {
        if kinds.contains(&node.kind()) {
            return Some(node);
        }
        node = node.parent()?;
    }
}

/// Start of `node` extended over the attributes and comments right above it.
fn with_leading_attributes(node: Node) -> usize {
    let mut start = node.start_byte();
    let mut prev = node.prev_sibling();
    while let Some(p) = prev.filter(|p| matches!(p.kind(), "attribute_item" | "line_comment" | "block_comment")) {
        start = p.start_byte();
        prev = p.prev_sibling();
    }
    start
}

/// The range of `object` around char `pos` (Rust only).
pub fn text_object(tree: &Tree, rope: &Rope, pos: usize, object: TextObject) -> Option<(usize, usize)> {
    let (b, _) = to_bytes(rope, pos, pos);
    let leaf = tree.root_node().descendant_for_byte_range(b, b)?;
    let chars = |s: usize, e: usize| (rope.byte_to_char(s), rope.byte_to_char(e));
    match object {
        TextObject::Function => {
            let f = ancestor(leaf, FUNCTIONS)?;
            let start = if f.kind() == "closure_expression" { f.start_byte() } else { with_leading_attributes(f) };
            Some(chars(start, f.end_byte()))
        }
        TextObject::FunctionBody => {
            let f = ancestor(leaf, FUNCTIONS)?;
            let body = f.child_by_field_name("body")?;
            if body.kind() != "block" {
                return Some(to_chars(rope, body));
            }
            // Between the braces, without the surrounding blank space.
            let (mut s, mut e) = (body.start_byte() + 1, body.end_byte().saturating_sub(1));
            let src = rope.byte_slice(s..e).to_string();
            s += src.len() - src.trim_start().len();
            e -= src.len() - src.trim_end().len();
            Some(chars(s, e.max(s)))
        }
        TextObject::Type => {
            let t = ancestor(leaf, TYPES)?;
            Some(chars(with_leading_attributes(t), t.end_byte()))
        }
        TextObject::Argument => {
            let mut node = leaf;
            loop {
                let parent = node.parent()?;
                if matches!(
                    parent.kind(),
                    "parameters" | "arguments" | "type_arguments" | "type_parameters" | "tuple_expression"
                ) && node.is_named()
                {
                    return Some(to_chars(rope, node));
                }
                node = parent;
            }
        }
        TextObject::Comment => {
            let c = ancestor(leaf, &["line_comment", "block_comment"])?;
            let (mut first, mut last) = (c, c);
            while let Some(p) =
                first.prev_sibling().filter(|p| p.kind() == "line_comment" && c.kind() == "line_comment")
            {
                if p.end_position().row + 1 < first.start_position().row {
                    break;
                }
                first = p;
            }
            while let Some(n) = last.next_sibling().filter(|n| n.kind() == "line_comment" && c.kind() == "line_comment")
            {
                if last.end_position().row + 1 < n.start_position().row {
                    break;
                }
                last = n;
            }
            // Line comments include their newline; leave it out of the selection.
            let end = rope.byte_slice(..last.end_byte()).to_string().trim_end().len();
            Some(chars(first.start_byte(), end))
        }
    }
}

/// Start (char) of the next/previous function after/before char `pos`.
pub fn adjacent_function(tree: &Tree, rope: &Rope, pos: usize, forward: bool) -> Option<usize> {
    let mut starts = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        if node.kind() == "function_item" {
            let name = node.child_by_field_name("name").unwrap_or(node);
            starts.push(rope.byte_to_char(name.start_byte()));
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    starts.sort_unstable();
    if forward { starts.into_iter().find(|s| *s > pos) } else { starts.into_iter().rev().find(|s| *s < pos) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str, lang: Lang) -> Tree {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&lang.language()).unwrap();
        p.parse(src, None).unwrap()
    }

    const SRC: &str = "/// Docs\n#[inline]\nfn add(a: i32, b: i32) -> i32 {\n    let s = a + b;\n    s\n}\n\nstruct P {\n    x: i32,\n    y: i32,\n}\n\nimpl Display for P {\n    fn fmt(&self) {}\n}\n";

    #[test]
    fn outline_from_tags_queries_nests_definitions() {
        let outline = |src: &str, lang: Lang| -> Vec<(String, usize)> {
            symbols(&parse(src, lang), src, lang).into_iter().map(|s| (s.name, s.depth)).collect()
        };
        let py = "class Shape:\n    def area(self):\n        pass\n\ndef main():\n    pass\n";
        assert_eq!(outline(py, Lang::Python), [("Shape".into(), 0), ("area".into(), 1), ("main".into(), 0)]);
        let java = "class App {\n  void run() {}\n  interface Job {}\n}\n";
        let names = outline(java, Lang::Java);
        assert_eq!(names[0], ("App".into(), 0));
        assert!(names.contains(&("run".into(), 1)), "{names:?}");
        let cpp = "namespace n { class V { void f(); }; }\nint main() { return 0; }\n";
        assert!(outline(cpp, Lang::Cpp).contains(&("main".into(), 0)), "{:?}", outline(cpp, Lang::Cpp));
    }

    #[test]
    fn folds_blocks_keeping_closing_lines() {
        let folds = folds_from_tree(&parse(SRC, Lang::Rust), Lang::Rust);
        assert!(folds.contains(&FoldRange { start: 2, end: 4 }), "{folds:?}");
        assert!(folds.contains(&FoldRange { start: 7, end: 9 }), "{folds:?}");
        let rope = Rope::from_str("a:\n  b\n  c:\n    d\ne\n");
        assert_eq!(indent_folds(&rope), vec![FoldRange { start: 0, end: 3 }, FoldRange { start: 2, end: 3 }]);
    }

    #[test]
    fn outlines_rust() {
        let syms = symbols(&parse(SRC, Lang::Rust), SRC, Lang::Rust);
        let names: Vec<(String, usize)> =
            syms.iter().map(|s| (format!("{} {}", s.kind.word(), s.name), s.depth)).collect();
        assert_eq!(
            names,
            vec![
                ("function add".into(), 0),
                ("struct P".into(), 0),
                ("field x".into(), 1),
                ("field y".into(), 1),
                ("impl Display for P".into(), 0),
                ("function fmt".into(), 1),
            ]
        );
        assert_eq!((syms[0].line, syms[0].start_line, syms[0].end_line), (2, 2, 5));
    }

    #[test]
    fn expands_selection_outward() {
        let tree = parse(SRC, Lang::Rust);
        let rope = Rope::from_str(SRC);
        let a = SRC.find("a + b").unwrap();
        let step1 = expand_selection(&tree, &rope, a, a).unwrap();
        assert_eq!(&SRC[step1.0..step1.1], "a");
        let step2 = expand_selection(&tree, &rope, step1.0, step1.1).unwrap();
        assert_eq!(&SRC[step2.0..step2.1], "a + b");
    }

    #[test]
    fn selects_text_objects() {
        let tree = parse(SRC, Lang::Rust);
        let rope = Rope::from_str(SRC);
        let inside = SRC.find("let s").unwrap();
        let get = |pos, obj| text_object(&tree, &rope, pos, obj).map(|(s, e)| SRC[s..e].to_owned());
        assert!(get(inside, TextObject::Function).unwrap().starts_with("/// Docs\n#[inline]\nfn add"));
        assert_eq!(get(inside, TextObject::FunctionBody).unwrap(), "let s = a + b;\n    s");
        assert_eq!(get(SRC.find("b: i32").unwrap(), TextObject::Argument).unwrap(), "b: i32");
        assert!(get(SRC.find("y: i32").unwrap(), TextObject::Type).unwrap().starts_with("struct P {"));
        assert_eq!(get(2, TextObject::Comment).unwrap(), "/// Docs");
        let next = adjacent_function(&tree, &rope, 0, true).unwrap();
        assert_eq!(&SRC[next..next + 3], "add");
    }
}
