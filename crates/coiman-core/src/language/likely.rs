//! Likely problems, found by reading code without running it: loops that look
//! endless and memory that looks leaked. Whether a program ends can't be decided in
//! general, so these are warnings about what's likely. When unsure, they stay quiet.

use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::language::syntax::Lang;

/// How sure a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    Likely,
    /// Depends on code that can't be seen from here.
    Possible,
}

/// A place in a file: 0-based, with the column in UTF-16 units as language servers count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub message: String,
    pub certainty: Certainty,
}

/// Larger files aren't worth holding up for.
const MAX_SIZE: usize = 1024 * 1024;

const ALLOCATORS: &[&str] = &["malloc", "calloc", "realloc", "strdup", "strndup", "aligned_alloc", "fopen"];
const FREERS: &[&str] = &["free", "fclose"];
/// Functions known to only look at memory passed to them.
const BORROWERS: &[&str] = &[
    "strlen", "printf", "fprintf", "sprintf", "snprintf", "memcpy", "memmove", "memset", "memcmp", "strcpy", "strncpy",
    "strcat", "strncat", "strcmp", "strncmp", "strchr", "strrchr", "strstr", "fgets", "fputs", "puts", "fread",
    "fwrite", "scanf", "sscanf", "fscanf", "fseek", "ftell", "rewind", "feof", "ferror", "fflush", "getc", "fgetc",
    "fputc", "atoi", "atof", "strtol",
];
/// Calls that can't end a loop unless the whole program fails.
const HARMLESS: &[&str] = &[
    "print",
    "printf",
    "puts",
    "println",
    "eprintln",
    "eprint",
    "fprintf",
    "sprintf",
    "log",
    "info",
    "debug",
    "warn",
    "sleep",
    "usleep",
    "Sleep",
    "WriteLine",
    "Write",
    "Println",
    "Printf",
    "Print",
    "Sprintf",
    "format",
];
/// Calls that end the program or leave the loop for good.
const ENDINGS: &[&str] = &[
    "exit",
    "_exit",
    "Exit",
    "abort",
    "panic",
    "quit",
    "terminate",
    "unreachable",
    "todo",
    "unimplemented",
    "longjmp",
    "Fatal",
    "Fatalf",
    "Abort",
];

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn visit<'t>(node: Node<'t>, f: &mut dyn FnMut(Node<'t>)) {
    f(node);
    for child in children(node) {
        visit(child, f);
    }
}

fn is_function(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "function_expression"
            | "generator_function"
            | "generator_function_declaration"
            | "arrow_function"
            | "lambda"
            | "lambda_expression"
            | "closure_expression"
            | "func_literal"
            | "function_item"
            | "method_declaration"
            | "method_definition"
            | "local_function_statement"
    )
}

fn is_call(kind: &str) -> bool {
    matches!(
        kind,
        "call"
            | "call_expression"
            | "method_invocation"
            | "invocation_expression"
            | "object_creation_expression"
            | "new_expression"
            | "macro_invocation"
            | "await"
            | "await_expression"
    )
}

/// What's called, without its object or path.
fn call_name<'a>(node: Node, source: &'a str) -> &'a str {
    let callee = ["function", "name", "macro", "type"].iter().find_map(|f| node.child_by_field_name(f));
    let whole = callee.map_or("", |c| text(c, source));
    whole.rsplit(['.', ':', '>']).next().unwrap_or(whole).trim_end_matches('!').trim()
}

fn place(node: Node, source: &str, message: String, certainty: Certainty) -> Finding {
    let start = node.start_position();
    let line = source.lines().nth(start.row).unwrap_or("");
    let units = |bytes: usize| line.get(..bytes.min(line.len())).unwrap_or("").encode_utf16().count() as u32;
    let end_col = if node.end_position().row == start.row { node.end_position().column } else { line.len() };
    Finding { line: start.row as u32, col: units(start.column), end_col: units(end_col), message, certainty }
}

// ----- loops ---------------------------------------------------------------------------

struct LoopParts<'t> {
    condition: Option<Node<'t>>,
    body: Node<'t>,
    /// Nothing in the header stops it.
    endless_header: bool,
}

fn is_constant_true(condition: &str) -> bool {
    let bare = condition.trim().trim_end_matches(';').trim();
    let bare = bare.strip_prefix('(').and_then(|b| b.strip_suffix(')')).unwrap_or(bare).trim();
    matches!(bare, "true" | "True" | "TRUE" | "1")
}

fn loop_parts<'t>(node: Node<'t>, source: &str) -> Option<LoopParts<'t>> {
    let body = node.child_by_field_name("body")?;
    let plain = |condition: Option<Node<'t>>| {
        let endless_header = condition.is_none_or(|c| is_constant_true(text(c, source)));
        Some(LoopParts { condition, body, endless_header })
    };
    match node.kind() {
        "loop_expression" => plain(None),
        "while_statement" | "while_expression" | "do_statement" => {
            let condition = node.child_by_field_name("condition")?;
            // `while let` ends when the pattern stops matching.
            if condition.kind() == "let_condition" || condition.kind() == "let_chain" {
                return None;
            }
            plain(Some(condition))
        }
        "for_statement" => {
            // Go's header is a child: a clause, a range, or just a condition.
            let inner = children(node).into_iter().find(|c| c.id() != body.id());
            match inner.map(|c| (c, c.kind())) {
                _ if node.child_by_field_name("initializer").is_some()
                    || node.child_by_field_name("update").is_some()
                    || node.child_by_field_name("increment").is_some() =>
                {
                    let condition = node.child_by_field_name("condition").filter(|c| c.kind() != "empty_statement");
                    let empty = condition.is_none();
                    let has_step =
                        node.child_by_field_name("update").is_some() || node.child_by_field_name("increment").is_some();
                    (empty && !has_step).then_some(LoopParts { condition: None, body, endless_header: true })
                }
                Some((_, "range_clause")) => None,
                Some((clause, "for_clause")) => {
                    let open = clause.child_by_field_name("condition").is_none();
                    let step = clause.child_by_field_name("update").is_some();
                    (open && !step).then_some(LoopParts { condition: None, body, endless_header: true })
                }
                Some((condition, _)) if node.child_by_field_name("condition").is_none() => {
                    // A Python `for` has `left` and `right`, so it isn't this.
                    if node.child_by_field_name("left").is_some() {
                        return None;
                    }
                    plain(Some(condition))
                }
                _ => {
                    let condition = node.child_by_field_name("condition").filter(|c| c.kind() != "empty_statement");
                    plain(condition)
                }
            }
        }
        _ => None,
    }
}

fn is_inner_block_for_break(kind: &str) -> bool {
    kind.contains("while")
        || kind.contains("for_")
        || kind.contains("foreach")
        || kind.contains("loop")
        || kind.contains("switch")
        || kind.contains("select")
        || kind == "do_statement"
}

/// What a loop's body can do.
#[derive(Default)]
struct BodyFacts {
    leaves: bool,
    harmless_calls: bool,
    other_calls: bool,
}

fn read_body(node: Node, source: &str, inner: bool, facts: &mut BodyFacts) {
    let kind = node.kind();
    if is_function(kind) {
        return;
    }
    match kind {
        "break_statement" | "break_expression" if !inner || node.named_child_count() > 0 => facts.leaves = true,
        "return_statement" | "return_expression" | "raise_statement" | "throw_statement" | "throw_expression"
        | "goto_statement" | "yield" | "yield_statement" | "yield_expression" | "try_expression" => facts.leaves = true,
        _ if is_call(kind) => {
            let name = call_name(node, source);
            if ENDINGS.contains(&name) {
                facts.leaves = true;
            } else if HARMLESS.contains(&name) {
                facts.harmless_calls = true;
            } else {
                facts.other_calls = true;
            }
        }
        _ => {}
    }
    let inner = inner || is_inner_block_for_break(kind);
    for child in children(node) {
        read_body(child, source, inner, facts);
    }
}

/// Whether evaluating this changes anything.
fn has_effects(node: Node) -> bool {
    let mut found = false;
    visit(node, &mut |n| {
        let kind = n.kind();
        found |= is_call(kind) || writes(kind);
    });
    found
}

fn writes(kind: &str) -> bool {
    kind.contains("assign")
        || kind.contains("delete")
        || kind.contains("update")
        || kind.contains("unary")
        || kind.contains("declaration")
        || kind.contains("declarator")
        || kind.contains("for_")
        || matches!(kind, "inc_statement" | "dec_statement" | "named_expression" | "reference_expression")
        || kind == "pointer_expression"
        || kind == "for_statement"
}

fn names_in(node: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    visit(node, &mut |n| {
        if n.kind() == "identifier" {
            names.push(text(n, source).to_owned());
        }
    });
    names
}

/// Names something in the body could be changing.
fn written_names(body: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    visit(body, &mut |n| {
        if writes(n.kind()) {
            names.extend(names_in(n, source));
        }
    });
    names
}

fn check_loops(root: Node, source: &str, out: &mut Vec<Finding>) {
    let mut found = Vec::new();
    visit(root, &mut |node| {
        // Half-typed code, or a grammar that doesn't fit: nothing can be told from it.
        if node.has_error() {
            return;
        }
        let Some(parts) = loop_parts(node, source) else { return };
        let mut facts = BodyFacts::default();
        read_body(parts.body, source, false, &mut facts);
        if facts.leaves {
            return;
        }
        let head = node.child(0).unwrap_or(node);
        if parts.endless_header {
            if facts.other_calls {
                let message = "This loop only ends if something it calls stops the program: it has no break or return";
                found.push(place(head, source, message.to_owned(), Certainty::Possible));
            } else {
                let message = "This loop never ends: nothing in it can leave it";
                found.push(place(head, source, message.to_owned(), Certainty::Likely));
            }
            return;
        }
        // A condition nothing can change stays as it was. A call could change it, unseen.
        let Some(condition) = parts.condition else { return };
        if facts.other_calls || facts.harmless_calls || has_effects(condition) {
            return;
        }
        let mut names = names_in(condition, source);
        names.sort();
        names.dedup();
        if names.is_empty() {
            return;
        }
        let written = written_names(parts.body, source);
        if names.iter().any(|n| written.contains(n)) {
            return;
        }
        // Another thread or a signal may set these.
        if source.contains("volatile") || source.contains("Atomic") {
            return;
        }
        let message = format!("This loop may never end: nothing in it changes {}", names.join(" or "));
        found.push(place(head, source, message, Certainty::Likely));
    });
    out.append(&mut found);
}

// ----- leaks ---------------------------------------------------------------------------

fn strip_wrappers(node: Node) -> Node {
    let mut at = node;
    loop {
        match at.kind() {
            "cast_expression" => match at.child_by_field_name("value") {
                Some(value) => at = value,
                None => return at,
            },
            "parenthesized_expression" => match at.named_child(0) {
                Some(inner) => at = inner,
                None => return at,
            },
            _ => return at,
        }
    }
}

fn allocates(node: Node, source: &str) -> Option<&'static str> {
    let value = strip_wrappers(node);
    match value.kind() {
        "new_expression" => Some("new"),
        "call_expression" => {
            let callee = value.child_by_field_name("function")?;
            let name = text(callee, source);
            ALLOCATORS.iter().find(|a| **a == name).copied()
        }
        _ => None,
    }
}

fn declared_name<'a>(declarator: Node, source: &'a str) -> Option<&'a str> {
    let mut at = declarator;
    loop {
        match at.kind() {
            "identifier" => return Some(text(at, source)),
            "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" => {
                at = at.child_by_field_name("declarator").or_else(|| children(at).pop())?;
            }
            _ => return None,
        }
    }
}

struct Site<'t> {
    node: Node<'t>,
    name: String,
    how: &'static str,
}

#[derive(Default)]
struct Uses {
    frees: Vec<usize>,
    escapes: bool,
}

/// How one mention of a variable treats its memory.
fn read_use(name_node: Node, source: &str, uses: &mut Uses) {
    let mut at = name_node;
    while let Some(parent) = at.parent() {
        match parent.kind() {
            "cast_expression" | "parenthesized_expression" => at = parent,
            "argument_list" => {
                let Some(call) = parent.parent() else { return };
                if call.kind() != "call_expression" {
                    uses.escapes = true;
                    return;
                }
                let name = call.child_by_field_name("function").map_or("", |f| text(f, source));
                if FREERS.contains(&name) {
                    uses.frees.push(call.start_byte());
                } else if !BORROWERS.contains(&name) {
                    // It may keep it, or free it.
                    uses.escapes = true;
                }
                return;
            }
            "delete_expression" => {
                uses.frees.push(parent.start_byte());
                return;
            }
            "return_statement" | "init_declarator" | "initializer_list" | "compound_literal_expression" => {
                uses.escapes = true;
                return;
            }
            "assignment_expression" => {
                let on_the_right = parent.child_by_field_name("right").is_some_and(|r| r.id() == at.id());
                uses.escapes |= on_the_right;
                return;
            }
            "pointer_expression" => {
                // `&p` hands it on; `*p` only reads.
                let address = parent.child(0).is_some_and(|op| text(op, source) == "&");
                uses.escapes |= address;
                return;
            }
            "lambda_capture_specifier" | "conditional_expression" | "comma_expression" => {
                uses.escapes = true;
                return;
            }
            _ => return,
        }
    }
}

fn is_static(init: Node, source: &str) -> bool {
    init.parent().is_some_and(|d| {
        children(d).iter().any(|c| c.kind() == "storage_class_specifier" && text(*c, source) == "static")
    })
}

fn is_null_guard(ret: Node, name: &str, source: &str) -> bool {
    let mut at = ret;
    while let Some(parent) = at.parent() {
        if parent.kind() == "if_statement"
            && let Some(condition) = parent.child_by_field_name("condition")
        {
            let squeezed: String = text(condition, source).chars().filter(|c| !c.is_whitespace()).collect();
            let inner = squeezed.trim_start_matches('(').trim_end_matches(')');
            let tests_it = inner == format!("!{name}")
                || ["NULL", "nullptr", "0"]
                    .iter()
                    .any(|null| inner == format!("{name}=={null}") || inner == format!("{null}=={name}"));
            if tests_it {
                return true;
            }
        }
        if is_function(parent.kind()) {
            return false;
        }
        at = parent;
    }
    false
}

/// The function or block holding `node`, for "was it freed just before this return?".
fn enclosing_block(node: Node) -> Option<Node> {
    let mut at = node;
    while let Some(parent) = at.parent() {
        if parent.kind() == "compound_statement" {
            return Some(parent);
        }
        at = parent;
    }
    None
}

fn check_function_leaks(function: Node, source: &str, out: &mut Vec<Finding>) {
    let Some(body) = function.child_by_field_name("body") else { return };
    if function.has_error() {
        return;
    }
    let mut sites: Vec<Site> = Vec::new();
    let mut locals: Vec<String> = Vec::new();
    let mut returns: Vec<Node> = Vec::new();
    let mut has_goto = false;
    visit(body, &mut |node| match node.kind() {
        "init_declarator" => {
            let Some(name) = node.child_by_field_name("declarator").and_then(|d| declared_name(d, source)) else {
                return;
            };
            locals.push(name.to_owned());
            if is_static(node, source) {
                return;
            }
            if let Some(how) = node.child_by_field_name("value").and_then(|v| allocates(v, source)) {
                sites.push(Site { node, name: name.to_owned(), how });
            }
        }
        "assignment_expression" => {
            let (Some(left), Some(right)) = (node.child_by_field_name("left"), node.child_by_field_name("right"))
            else {
                return;
            };
            if left.kind() == "identifier"
                && let Some(how) = allocates(right, source)
            {
                sites.push(Site { node, name: text(left, source).to_owned(), how });
            }
        }
        "declaration" => {
            for child in children(node) {
                if let Some(name) = declared_name(child, source) {
                    locals.push(name.to_owned());
                }
            }
        }
        "return_statement" => returns.push(node),
        "goto_statement" => has_goto = true,
        _ => {}
    });
    for (i, site) in sites.iter().enumerate() {
        if !locals.contains(&site.name) {
            continue;
        }
        // Each allocation answers for the mentions up to the next one of the same name.
        let until = sites[i + 1..].iter().find(|s| s.name == site.name).map(|s| s.node.start_byte());
        let mut uses = Uses::default();
        visit(body, &mut |node| {
            let after = node.start_byte() >= site.node.end_byte();
            if after
                && until.is_none_or(|u| node.start_byte() < u)
                && node.kind() == "identifier"
                && text(node, source) == site.name
            {
                read_use(node, source, &mut uses);
            }
        });
        if uses.escapes {
            continue;
        }
        let what = if site.how == "fopen" { "opened" } else { "allocated" };
        let undo = if site.how == "fopen" { "closed" } else { "freed" };
        if uses.frees.is_empty() {
            let line = site.node.start_position().row + 1;
            let message = match until {
                Some(_) => {
                    format!("{} is {what} again before this memory is {undo}, so it may leak (line {line})", site.name)
                }
                None => format!("{} is {what} here and never {undo}: this may leak", site.name),
            };
            out.push(place(site.node, source, message, Certainty::Likely));
            continue;
        }
        if has_goto {
            continue;
        }
        let last_free = uses.frees.iter().copied().max().unwrap_or(0);
        for ret in &returns {
            let at = ret.start_byte();
            if at < site.node.end_byte() || at > last_free || is_null_guard(*ret, &site.name, source) {
                continue;
            }
            let block = enclosing_block(*ret).map_or(0, |b| b.start_byte());
            if uses.frees.iter().any(|f| *f >= block && *f < at) {
                continue;
            }
            let line = site.node.start_position().row + 1;
            let message = format!("This returns without releasing {}, {what} on line {line}: this may leak", site.name);
            out.push(place(*ret, source, message, Certainty::Likely));
        }
    }
}

fn check_leaks(root: Node, source: &str, out: &mut Vec<Finding>) {
    let mut functions = Vec::new();
    visit(root, &mut |node| {
        if node.kind() == "function_definition" {
            functions.push(node);
        }
    });
    for function in functions {
        check_function_leaks(function, source, out);
    }
}

fn check_rust_leaks(root: Node, source: &str, out: &mut Vec<Finding>) {
    let mut found = Vec::new();
    visit(root, &mut |node| {
        if node.kind() != "call_expression" {
            return;
        }
        let name = node.child_by_field_name("function").map_or("", |f| text(f, source));
        if name.ends_with("mem::forget") || name == "forget" || name.ends_with("Box::leak") {
            let message = format!("{name} leaks on purpose: its memory is never released");
            found.push(place(node, source, message, Certainty::Possible));
        }
    });
    out.append(&mut found);
}

// ----- Zig, read as text ---------------------------------------------------------------

fn zig_code(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

fn line_finding(line: usize, source_line: &str, message: String, certainty: Certainty) -> Finding {
    let indent = source_line.len() - source_line.trim_start().len();
    let units = |s: &str| s.encode_utf16().count() as u32;
    Finding {
        line: line as u32,
        col: units(&source_line[..indent]),
        end_col: units(source_line.trim_end()),
        message,
        certainty,
    }
}

fn check_zig(source: &str, out: &mut Vec<Finding>) {
    let lines: Vec<&str> = source.lines().collect();
    // An arena frees everything together.
    let arena = source.contains("ArenaAllocator");
    for (i, raw) in lines.iter().enumerate() {
        let code = zig_code(raw);
        let trimmed = code.trim_start();
        if !arena
            && let Some(rest) = trimmed.strip_prefix("const ").or_else(|| trimmed.strip_prefix("var "))
            && let Some((name, value)) = rest.split_once('=')
        {
            let name = name.split(':').next().unwrap_or(name).trim();
            let allocates = [".alloc(", ".create(", ".dupe(", ".allocPrint(", ".dupeZ(", ".realloc("]
                .iter()
                .any(|a| value.contains(a));
            let rest_of_file = lines[i + 1..].iter().map(|l| zig_code(l)).collect::<Vec<_>>().join("\n");
            let released = [
                format!("free({name}"),
                format!("destroy({name}"),
                format!("return {name}"),
                format!(", {name})"),
                format!("({name})"),
            ]
            .iter()
            .any(|p| rest_of_file.contains(p.as_str()));
            if allocates && !released {
                out.push(line_finding(
                    i,
                    raw,
                    format!("{name} is allocated here and never freed: this may leak"),
                    Certainty::Likely,
                ));
            }
        }
        let endless = trimmed.starts_with("while (true)");
        if endless && trimmed.contains('{') {
            let mut depth = 0i32;
            let mut leaves = false;
            for later in &lines[i..] {
                let code = zig_code(later);
                depth += code.matches('{').count() as i32 - code.matches('}').count() as i32;
                leaves |=
                    ["break", "return", "unreachable", "exit(", "@panic", "try "].iter().any(|w| code.contains(w));
                if depth <= 0 {
                    break;
                }
            }
            if !leaves {
                out.push(line_finding(
                    i,
                    raw,
                    "This loop never ends: nothing in it can leave it".to_owned(),
                    Certainty::Likely,
                ));
            }
        }
    }
}

// ----- all together --------------------------------------------------------------------

/// Whether files of this type get checked.
pub fn checks(extension: &str) -> bool {
    extension == "zig" || lang_of(extension).is_some()
}

fn lang_of(extension: &str) -> Option<Lang> {
    Lang::from_extension(extension).filter(|lang| {
        matches!(
            lang,
            Lang::Rust
                | Lang::Python
                | Lang::JavaScript
                | Lang::Jsx
                | Lang::TypeScript
                | Lang::Tsx
                | Lang::Java
                | Lang::C
                | Lang::Cpp
                | Lang::CSharp
                | Lang::Go
        )
    })
}

/// Reads `source`, a file of this type. Takes long enough to keep off the UI thread.
pub fn check(extension: &str, source: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    if source.len() > MAX_SIZE {
        return out;
    }
    if extension == "zig" {
        check_zig(source, &mut out);
        return out;
    }
    let Some(lang) = lang_of(extension) else { return out };
    let mut parser = Parser::new();
    if parser.set_language(&lang.language()).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else { return out };
    let root = tree.root_node();
    check_loops(root, source, &mut out);
    match lang {
        Lang::C | Lang::Cpp => {
            check_leaks(root, source, &mut out);
            // Pointers that may be NULL, read before a check.
            for found in crate::language::null_check::find(root, source, lang == Lang::Cpp, "    ", "\n") {
                if let Some(node) = root.descendant_for_byte_range(found.use_start, found.use_end) {
                    out.push(place(node, source, found.message(), Certainty::Possible));
                }
            }
        }
        Lang::Rust => check_rust_leaks(root, source, &mut out),
        _ => {}
    }
    out
}

// ----- the Clang analyzer --------------------------------------------------------------

/// Whether the Clang analyzer applies to this type of file.
pub fn analyzer_reads(extension: &str) -> bool {
    matches!(Lang::from_extension(extension), Some(Lang::C | Lang::Cpp))
}

/// Arguments that ask `clang-tidy` only about leaks. With no compile database, the
/// final `--` says to use default flags, rather than complain about its absence.
pub fn analyzer_args(file: &Path, has_database: bool) -> Vec<String> {
    let checks = "-*,clang-analyzer-unix.Malloc,clang-analyzer-cplusplus.NewDeleteLeaks,clang-analyzer-unix.Stream";
    let mut args = vec!["--quiet".to_owned(), format!("--checks={checks}"), file.to_string_lossy().into_owned()];
    if !has_database {
        args.push("--".to_owned());
    }
    args
}

/// Reads what `clang-tidy` printed about `file`.
pub fn parse_analyzer<S: AsRef<str>>(lines: &[S], file: &Path) -> Vec<Finding> {
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Vec::new();
    for line in lines {
        let line = line.as_ref();
        let Some((place, rest)) = line.split_once(": warning: ") else { continue };
        let Some(message) = rest.split(" [clang-analyzer").next().filter(|_| rest.contains("[clang-analyzer")) else {
            continue;
        };
        let mut parts = place.rsplitn(3, ':');
        let (Some(col), Some(row), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let (Ok(col), Ok(row)) = (col.parse::<u32>(), row.parse::<u32>()) else { continue };
        if !path.replace('\\', "/").ends_with(&name) {
            continue;
        }
        let col = col.saturating_sub(1);
        out.push(Finding {
            line: row.saturating_sub(1),
            col,
            end_col: col + 1,
            message: message.to_owned(),
            certainty: Certainty::Likely,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(ext: &str, source: &str) -> Vec<String> {
        check(ext, source).into_iter().map(|f| f.message).collect()
    }

    fn lines(ext: &str, source: &str) -> Vec<u32> {
        check(ext, source).into_iter().map(|f| f.line).collect()
    }

    #[test]
    fn finds_loops_that_never_end() {
        for (ext, source) in [
            ("py", "while True:\n    x = 1\n"),
            ("py", "while True:\n    print('hi')\n"),
            ("js", "while (true) { n++; }"),
            ("ts", "for (;;) { n++; }"),
            ("java", "class A { void f() { while (true) { n++; } } }"),
            ("c", "void f(void) { for (;;) { n++; } }"),
            ("c", "void f(void) { while (1) { n++; } }"),
            ("cpp", "void f() { do { n++; } while (true); }"),
            ("cs", "class A { void F() { while (true) { n++; } } }"),
            ("go", "package a\nfunc f() { for { n++ } }"),
            ("rs", "fn f() { loop { n += 1; } }"),
            ("rs", "fn f() { loop { println!(\"x\"); } }"),
            ("zig", "pub fn main() void {\n    while (true) {\n        n += 1;\n    }\n}\n"),
        ] {
            let found = messages(ext, source);
            assert!(found.iter().any(|m| m.contains("never ends")), "{ext}: {source}: {found:?}");
        }
    }

    #[test]
    fn leaves_loops_that_can_end_alone() {
        for (ext, source) in [
            ("py", "while True:\n    if x:\n        break\n"),
            ("py", "def f():\n    while True:\n        return 1\n"),
            ("py", "def f():\n    while True:\n        yield 1\n"),
            ("py", "while True:\n    raise E()\n"),
            ("py", "import sys\nwhile True:\n    sys.exit(0)\n"),
            ("py", "n = 3\nwhile n > 0:\n    n -= 1\n"),
            ("py", "for x in y:\n    print(x)\n"),
            ("py", "while a[1] == 2:\n    del a[1]\n"),
            ("c", "void f(void) { while (true) { template <x> y; return; } }"),
            ("js", "while (true) { if (x) break; }"),
            ("js", "for (let i = 0; i < 3; i++) { n++; }"),
            ("js", "for (;; i++) { if (i > 3) break; }"),
            ("c", "void f(int i) { for (i = 0; i < 3; i++) { n++; } }"),
            ("c", "void f(int n) { while (n) { n--; } }"),
            ("c", "void f(int *p) { while (*p) { p++; } }"),
            ("c", "volatile int go; void f(void) { while (go) { } }"),
            ("go", "package a\nfunc f() { for i := 0; i < 3; i++ { n++ } }"),
            ("go", "package a\nfunc f() { for _, x := range y { n += x } }"),
            ("go", "package a\nfunc f(n int) { for n > 0 { n-- } }"),
            ("rs", "fn f() { loop { break; } }"),
            ("rs", "fn f() -> R { loop { g()?; } }"),
            ("rs", "fn f() { while let Some(x) = i.next() { n += x; } }"),
            ("zig", "pub fn main() void {\n    while (true) {\n        break;\n    }\n}\n"),
        ] {
            let found = messages(ext, source);
            assert!(
                found.iter().all(|m| !m.contains("never ends") && !m.contains("may never")),
                "{ext}: {source}: {found:?}"
            );
        }
    }

    #[test]
    fn a_break_of_an_inner_loop_does_not_count() {
        let found = messages("py", "while True:\n    for x in y:\n        break\n");
        assert!(found.iter().any(|m| m.contains("never ends")), "{found:?}");
        let found = messages("c", "void f(void) { while (1) { switch (a) { case 1: break; } } }");
        assert!(found.iter().any(|m| m.contains("never ends")), "{found:?}");
        // One inside a function it defines isn't its own either.
        let found = messages("py", "while True:\n    def g():\n        return 1\n");
        assert!(found.iter().any(|m| m.contains("never ends")), "{found:?}");
    }

    #[test]
    fn a_condition_nothing_changes_is_a_warning() {
        let found = messages("py", "n = 3\nwhile n > 0:\n    m = 1\n");
        assert!(found.iter().any(|m| m.contains("nothing in it changes n")), "{found:?}");
        let found = messages("c", "void f(int n) { while (n > 0) { m = 1; } }");
        assert!(found.iter().any(|m| m.contains("nothing in it changes n")), "{found:?}");
    }

    #[test]
    fn calls_make_it_only_a_possibility() {
        let found = check("py", "while True:\n    serve()\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].certainty, Certainty::Possible);
        // A call could be what changes the condition, so nothing is said.
        assert!(messages("py", "while n > 0:\n    step()\n").is_empty());
    }

    #[test]
    fn finds_memory_never_freed() {
        for (ext, source) in [
            ("c", "void f(void) { char *p = malloc(4); }"),
            ("c", "void f(void) { char *p = (char *)malloc(4); p[0] = 1; printf(\"%s\", p); }"),
            ("c", "void f(void) { char *p; p = strdup(\"a\"); strlen(p); }"),
            ("cpp", "void f() { int *p = new int[4]; p[0] = 1; }"),
        ] {
            let found = messages(ext, source);
            assert!(found.iter().any(|m| m.contains("never freed")), "{source}: {found:?}");
        }
        let found = messages("c", "void f(void) { FILE *f = fopen(\"a\", \"r\"); }");
        assert!(found.iter().any(|m| m.contains("opened here and never closed")), "{found:?}");
    }

    #[test]
    fn leaves_memory_that_is_released_or_handed_on_alone() {
        for (ext, source) in [
            ("c", "void f(void) { char *p = malloc(4); free(p); }"),
            ("c", "char *f(void) { char *p = malloc(4); return p; }"),
            ("c", "void f(void) { char *p = malloc(4); keep(p); }"),
            ("c", "void f(struct s *o) { char *p = malloc(4); o->p = p; }"),
            ("c", "void f(char **o) { char *p = malloc(4); *o = p; }"),
            ("c", "char *g; void f(void) { g = malloc(4); }"),
            ("c", "void f(void) { static char *p = malloc(4); }"),
            ("c", "void f(void) { FILE *f = fopen(\"a\", \"r\"); fclose(f); }"),
            ("cpp", "void f() { int *p = new int; delete p; }"),
            ("cpp", "void f() { int *p = new int[3]; delete[] p; }"),
            ("cpp", "void f() { auto p = std::make_unique<int>(1); }"),
            ("cpp", "void f() { std::unique_ptr<int> p(new int); }"),
            ("cpp", "void f(std::vector<int*> &v) { int *p = new int; v.push_back(p); }"),
        ] {
            // Leaks only: `fclose(f)` unchecked is the NULL check's to say.
            let found: Vec<String> = messages(ext, source).into_iter().filter(|m| !m.contains("may be NULL")).collect();
            assert!(found.is_empty(), "{source}: {found:?}");
        }
        let unchecked = messages("c", "void f(void) { FILE *f = fopen(\"a\", \"r\"); fclose(f); }");
        assert!(unchecked.iter().any(|m| m.contains("may be NULL")), "{unchecked:?}");
    }

    #[test]
    fn finds_a_return_that_skips_the_free() {
        let source = "int f(int e) {\n  char *p = malloc(4);\n  if (!p) return 1;\n  if (e) return 2;\n  free(p);\n  return 0;\n}\n";
        assert_eq!(lines("c", source), [3], "{:?}", messages("c", source));
        let freed_first =
            "int f(int e) {\n  char *p = malloc(4);\n  if (e) { free(p); return 2; }\n  free(p);\n  return 0;\n}\n";
        assert_eq!(messages("c", freed_first), Vec::<String>::new());
        let cleanup = "int f(int e) {\n  char *p = malloc(4);\n  if (e) goto done;\n  return 1;\ndone:\n  free(p);\n  return 0;\n}\n";
        assert_eq!(messages("c", cleanup), Vec::<String>::new());
    }

    #[test]
    fn finds_memory_lost_by_allocating_again() {
        let source = "void f(void) {\n  char *p = malloc(4);\n  p = malloc(8);\n  free(p);\n}\n";
        assert_eq!(lines("c", source), [1], "{:?}", messages("c", source));
    }

    #[test]
    fn reads_rust_and_zig_leaks() {
        let found = check("rs", "fn f(b: Box<i32>) { std::mem::forget(b); }");
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].certainty, Certainty::Possible);
        assert!(check("rs", "fn f() { let b = Box::new(1); }").is_empty());

        let leak = "pub fn f(a: Allocator) !void {\n    const buf = try a.alloc(u8, 4);\n    _ = buf;\n}\n";
        assert_eq!(lines("zig", leak), [1]);
        let freed = "pub fn f(a: Allocator) !void {\n    const buf = try a.alloc(u8, 4);\n    defer a.free(buf);\n}\n";
        assert!(check("zig", freed).is_empty());
        let arena = "var arena = std.heap.ArenaAllocator.init(a);\nconst buf = try arena.allocator().alloc(u8, 4);\n";
        assert!(check("zig", arena).is_empty());
    }

    #[test]
    fn stays_out_of_other_files() {
        assert!(check("json", "{}").is_empty());
        assert!(!checks("json") && checks("py") && checks("zig"));
        assert!(check("py", "while True:\n").iter().all(|f| f.line == 0));
        assert!(check("py", &"x = 1\n".repeat(300_000)).is_empty());
    }

    #[test]
    fn columns_count_as_language_servers_do() {
        let found = check("py", "é = 1\nif é: \n    while True: pass\n");
        assert_eq!(found[0].line, 2);
        assert_eq!(found[0].col, 4);
    }

    #[test]
    fn reads_the_analyzer() {
        let file = Path::new("C:/p/a.c");
        let out = [
            r"C:\p\a.c:3:1: warning: Potential leak of memory pointed to by 'p' [clang-analyzer-unix.Malloc]",
            r"C:\p\a.c:2:13: note: Memory is allocated",
            r"C:\p\other.c:9:1: warning: Potential leak [clang-analyzer-unix.Malloc]",
            r"C:\p\a.c:5:1: warning: unrelated [readability-x]",
        ];
        let found = parse_analyzer(&out, file);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!((found[0].line, found[0].col), (2, 0));
        assert_eq!(found[0].message, "Potential leak of memory pointed to by 'p'");
        assert!(analyzer_args(file, false).last().is_some_and(|a| a == "--"));
        assert!(analyzer_args(file, true).last().is_some_and(|a| a != "--"));
        assert!(analyzer_reads("cpp") && !analyzer_reads("py"));
    }
}
