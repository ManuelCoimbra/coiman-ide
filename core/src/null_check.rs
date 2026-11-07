//! Pointers that can be NULL, used before they're checked: what `malloc`, `fopen`,
//! `strchr` and the like return, read through (`*p`, `p->x`, `p[i]`) or handed to a
//! library function that reads it. Only what's sure to be such a use is reported, and
//! the fix (a check after the pointer is set) is offered, never made unasked.

use tree_sitter::Node;

/// Library functions that return NULL on failure (or when nothing is found).
const NULLABLE: &[&str] = &[
    "malloc",
    "calloc",
    "realloc",
    "aligned_alloc",
    "strdup",
    "strndup",
    "fopen",
    "freopen",
    "tmpfile",
    "fdopen",
    "popen",
    "opendir",
    "getenv",
    "strchr",
    "strrchr",
    "strstr",
    "strpbrk",
    "memchr",
    "bsearch",
    "localtime",
    "gmtime",
];

/// Of those, the ones whose NULL means it failed (rather than found nothing): their
/// check says why with `perror`.
const FAILING: &[&str] = &[
    "malloc",
    "calloc",
    "realloc",
    "aligned_alloc",
    "strdup",
    "strndup",
    "fopen",
    "freopen",
    "tmpfile",
    "fdopen",
    "popen",
    "opendir",
];

/// Library functions that read through a pointer they're given (NULL there is
/// undefined behaviour).
const READERS: &[&str] = &[
    "strlen", "strcpy", "strncpy", "strcat", "strncat", "strcmp", "strncmp", "strchr", "strrchr", "strstr", "strtok",
    "memcpy", "memmove", "memset", "memcmp", "printf", "puts", "fputs", "fgets", "fprintf", "fscanf", "sscanf",
    "sprintf", "snprintf", "fread", "fwrite", "fclose", "fseek", "ftell", "rewind", "feof", "ferror", "fgetc", "getc",
    "fputc", "putc", "ungetc", "atoi", "atol", "atof", "strtol", "strtoul", "strtod", "readdir", "closedir", "qsort",
    "toupper", "tolower",
];

/// One pointer used before a check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NullUse {
    pub name: String,
    /// What gave it: `malloc`, `fopen`, `dynamic_cast`.
    pub from: String,
    /// The statement that set it: its line, and the byte after it.
    pub set_line: usize,
    pub set_end: usize,
    /// The use: bytes, and its line.
    pub use_start: usize,
    pub use_end: usize,
    pub use_line: usize,
    /// The check to put after the statement that set it, ready to insert there.
    pub fix: String,
}

impl NullUse {
    pub fn message(&self) -> String {
        if FAILING.contains(&self.from.as_str()) {
            format!("`{}` may be NULL here: {} returns NULL when it fails. Check it first", self.name, self.from)
        } else if self.from == "dynamic_cast" {
            format!(
                "`{}` may be null here: dynamic_cast to a pointer gives null when the type doesn't match",
                self.name
            )
        } else {
            format!(
                "`{}` may be NULL here: {} returns NULL when it finds nothing. Check it first",
                self.name, self.from
            )
        }
    }
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn named_children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

/// Casts and parentheses taken off.
fn bare(mut node: Node) -> Node {
    loop {
        match node.kind() {
            "parenthesized_expression" => match node.named_child(0) {
                Some(inner) => node = inner,
                None => return node,
            },
            "cast_expression" => match node.child_by_field_name("value") {
                Some(inner) => node = inner,
                None => return node,
            },
            _ => return node,
        }
    }
}

/// The NULL-returning function a value comes from, if it does.
fn nullable_call(value: Node, source: &str, cpp: bool) -> Option<String> {
    let value = bare(value);
    if value.kind() != "call_expression" {
        return None;
    }
    let function = value.child_by_field_name("function")?;
    let name = text(function, source);
    if cpp && name.starts_with("dynamic_cast") && name.contains('*') {
        return Some("dynamic_cast".to_owned());
    }
    let name = name.trim_start_matches("std::").trim_start_matches("::");
    NULLABLE.contains(&name).then(|| name.to_owned())
}

/// A pointer set in a statement: `T *p = f(..);`, `auto p = f(..);` or `p = f(..);`.
fn set_by(statement: Node, source: &str, cpp: bool) -> Option<(String, String)> {
    match statement.kind() {
        "declaration" => {
            let init = named_children(statement).into_iter().find(|c| c.kind() == "init_declarator")?;
            let mut declarator = init.child_by_field_name("declarator")?;
            let mut pointer = false;
            while matches!(declarator.kind(), "pointer_declarator" | "reference_declarator") {
                pointer |= declarator.kind() == "pointer_declarator";
                declarator = declarator.child_by_field_name("declarator").or_else(|| declarator.named_child(0))?;
            }
            if declarator.kind() != "identifier" {
                return None;
            }
            let ty = statement.child_by_field_name("type").map_or("", |t| text(t, source));
            if !pointer && ty != "auto" {
                return None;
            }
            let from = nullable_call(init.child_by_field_name("value")?, source, cpp)?;
            Some((text(declarator, source).to_owned(), from))
        }
        "expression_statement" => {
            let assign = statement.named_child(0)?;
            if assign.kind() != "assignment_expression" || text(assign.child(1)?, source) != "=" {
                return None;
            }
            let left = assign.child_by_field_name("left")?;
            if left.kind() != "identifier" {
                return None;
            }
            let from = nullable_call(assign.child_by_field_name("right")?, source, cpp)?;
            Some((text(left, source).to_owned(), from))
        }
        _ => None,
    }
}

fn mentions(node: Node, name: &str, source: &str) -> bool {
    if node.kind() == "identifier" && text(node, source) == name {
        return true;
    }
    named_children(node).into_iter().any(|c| mentions(c, name, source))
}

fn is_name(node: Option<Node>, name: &str, source: &str) -> bool {
    node.is_some_and(|n| {
        let n = bare(n);
        n.kind() == "identifier" && text(n, source) == name
    })
}

enum Seen<'t> {
    Guarded,
    Use(Node<'t>),
}

/// The first thing in `node`, in reading order, that checks `name` or reads through it.
fn first_seen<'t>(node: Node<'t>, name: &str, source: &str) -> Option<Seen<'t>> {
    match node.kind() {
        // Not evaluated.
        "sizeof_expression" | "alignof_expression" | "offsetof_expression" => return None,
        "if_statement"
        | "while_statement"
        | "do_statement"
        | "for_statement"
        | "switch_statement"
        | "conditional_expression" => {
            let condition = node.child_by_field_name("condition");
            if condition.is_some_and(|c| mentions(c, name, source)) {
                return Some(Seen::Guarded);
            }
        }
        "binary_expression" => {
            let op = node.child_by_field_name("operator").map_or("", |o| text(o, source));
            let left = node.child_by_field_name("left");
            // `p && p->x`, `p == NULL || ...`: the left side checks it first.
            if matches!(op, "&&" | "||") && left.is_some_and(|l| mentions(l, name, source)) {
                return Some(Seen::Guarded);
            }
            if matches!(op, "==" | "!=")
                && (is_name(left, name, source) || is_name(node.child_by_field_name("right"), name, source))
            {
                return Some(Seen::Guarded);
            }
        }
        "unary_expression" => {
            let op = node.child_by_field_name("operator").map_or("", |o| text(o, source));
            if op == "!" && is_name(node.child_by_field_name("argument"), name, source) {
                return Some(Seen::Guarded);
            }
        }
        "assignment_expression" => {
            // Set again: what's after is about the new value.
            if is_name(node.child_by_field_name("left"), name, source)
                && text(node.child(1).unwrap_or(node), source) == "="
            {
                return match node.child_by_field_name("right").and_then(|r| first_seen(r, name, source)) {
                    Some(seen) => Some(seen),
                    None => Some(Seen::Guarded),
                };
            }
        }
        "pointer_expression" => {
            let op = node.child(0).map_or("", |o| text(o, source));
            if op == "*" && is_name(node.child_by_field_name("argument"), name, source) {
                return Some(Seen::Use(node));
            }
        }
        "field_expression" => {
            let op = node.child_by_field_name("operator").map_or("", |o| text(o, source));
            if op == "->" && is_name(node.child_by_field_name("argument"), name, source) {
                return Some(Seen::Use(node));
            }
        }
        "subscript_expression" => {
            if is_name(node.child_by_field_name("argument"), name, source) {
                return Some(Seen::Use(node));
            }
        }
        "call_expression" => {
            let function = node.child_by_field_name("function").map_or("", |f| text(f, source));
            let function = function.trim_start_matches("std::");
            if matches!(function, "assert" | "ASSERT" | "Expects" | "assume" | "_ASSERTE" | "g_assert" | "REQUIRE")
                && mentions(node, name, source)
            {
                return Some(Seen::Guarded);
            }
            if READERS.contains(&function)
                && let Some(args) = node.child_by_field_name("arguments")
                && let Some(arg) = named_children(args).into_iter().find(|a| is_name(Some(*a), name, source))
            {
                // Its own arguments are read first.
                for a in named_children(args) {
                    if a.id() == arg.id() {
                        break;
                    }
                    if let Some(seen) = first_seen(a, name, source) {
                        return Some(seen);
                    }
                }
                return Some(Seen::Use(arg));
            }
        }
        _ => {}
    }
    for child in named_children(node) {
        if let Some(seen) = first_seen(child, name, source) {
            return Some(seen);
        }
    }
    None
}

/// What a failed check does, as the function it's in returns.
fn give_up(function: Option<Node>, source: &str, cpp: bool) -> String {
    let stdlib = source.contains("stdlib.h") || source.contains("<cstdlib>");
    let exit_failure = if stdlib { "EXIT_FAILURE" } else { "1" };
    let Some(function) = function else { return format!("exit({exit_failure});") };
    let ty = function.child_by_field_name("type").map_or("", |t| text(t, source)).trim();
    let declarator = function.child_by_field_name("declarator");
    let returns_pointer = declarator.is_some_and(|d| d.kind() == "pointer_declarator");
    let name = declarator
        .and_then(|mut d| {
            while d.kind() != "function_declarator" {
                d = d.child_by_field_name("declarator")?;
            }
            d.child_by_field_name("declarator")
        })
        .map_or("", |n| text(n, source));
    if returns_pointer {
        return if cpp { "return nullptr;".to_owned() } else { "return NULL;".to_owned() };
    }
    if name == "main" {
        return format!("return {exit_failure};");
    }
    match ty {
        "void" => "return;".to_owned(),
        "bool" | "_Bool" => "return false;".to_owned(),
        "int" | "long" | "short" | "ssize_t" | "long long" | "signed" => "return -1;".to_owned(),
        _ => format!("exit({exit_failure});"),
    }
}

fn line_indent(source: &str, byte: usize) -> String {
    let start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
    source[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

fn enclosing_function(node: Node) -> Option<Node> {
    let mut here = node.parent();
    while let Some(n) = here {
        if matches!(n.kind(), "function_definition" | "lambda_expression") {
            return Some(n);
        }
        here = n.parent();
    }
    None
}

/// Pointers in a C or C++ tree used before a check, with their fixes.
pub fn find(root: Node, source: &str, cpp: bool, unit: &str, nl: &str) -> Vec<NullUse> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        stack.extend(named_children(node));
        if node.kind() != "compound_statement" {
            continue;
        }
        let statements = named_children(node);
        for (i, statement) in statements.iter().enumerate() {
            let Some((name, from)) = set_by(*statement, source, cpp) else { continue };
            // What's after a return (or break, continue, goto) in this block isn't reached.
            let Some(Seen::Use(used)) = statements[i + 1..].iter().find_map(|s| {
                first_seen(*s, &name, source).or_else(|| {
                    matches!(s.kind(), "return_statement" | "break_statement" | "continue_statement" | "goto_statement")
                        .then_some(Seen::Guarded)
                })
            }) else {
                continue;
            };
            let indent = line_indent(source, statement.start_byte());
            let null = if cpp { "nullptr" } else { "NULL" };
            let mut body = String::new();
            if FAILING.contains(&from.as_str()) {
                body.push_str(&format!("{indent}{unit}perror(\"{from}\");{nl}"));
            }
            body.push_str(&format!("{indent}{unit}{}{nl}", give_up(enclosing_function(*statement), source, cpp)));
            let fix = format!("{nl}{indent}if ({name} == {null}) {{{nl}{body}{indent}}}");
            out.push(NullUse {
                name,
                from,
                set_line: statement.start_position().row,
                set_end: statement.end_byte(),
                use_start: used.start_byte(),
                use_end: used.end_byte(),
                use_line: used.start_position().row,
                fix,
            });
        }
    }
    out.sort_by_key(|u| u.use_start);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uses(source: &str, cpp: bool) -> Vec<NullUse> {
        let lang = if cpp { crate::syntax::Lang::Cpp } else { crate::syntax::Lang::C };
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&lang.language()).unwrap();
        let tree = parser.parse(source, None).unwrap();
        find(tree.root_node(), source, cpp, "    ", "\n")
    }

    #[test]
    fn uses_before_a_check_are_found() {
        let src = "#include <stdlib.h>\n#include <stdio.h>\nint main(void) {\n    char *p = malloc(10);\n    struct node *n;\n    n = calloc(1, sizeof *n);\n    p[0] = 'a';\n    if (!n) return 1;\n    n->x = 1;\n    FILE *fp = fopen(\"x\", \"r\");\n    fgets(p, 10, fp);\n    char *dot = strchr(p, '.');\n    printf(\"%s\", dot);\n    return 0;\n}\n";
        let found = uses(src, false);
        let names: Vec<(&str, &str)> = found.iter().map(|u| (u.name.as_str(), u.from.as_str())).collect();
        // n is checked before n->x; sizeof *n isn't a use.
        assert_eq!(names, [("p", "malloc"), ("fp", "fopen"), ("dot", "strchr")]);
        assert_eq!(&src[found[0].use_start..found[0].use_end], "p[0]");
        assert_eq!(
            found[0].fix,
            "\n    if (p == NULL) {\n        perror(\"malloc\");\n        return EXIT_FAILURE;\n    }"
        );
        assert_eq!(&src[found[1].use_start..found[1].use_end], "fp");
        assert!(found[2].fix.contains("return EXIT_FAILURE;") && !found[2].fix.contains("perror"));
        assert!(found[0].message().contains("malloc returns NULL when it fails"));
        assert!(found[2].message().contains("finds nothing"));
    }

    #[test]
    fn checks_in_many_shapes_count() {
        for check in [
            "if (p == NULL) return;",
            "if (!p) { return; }",
            "if (p) { p[0] = 1; }",
            "if (NULL != p) p[0] = 1;",
            "assert(p != NULL);",
            "while (p && p[0]) p++;",
            "p = other;",
            "return;",
        ] {
            let src = format!("void f(void) {{\n    char *p = (char *) malloc(4);\n    {check}\n    p[1] = 0;\n}}\n");
            assert!(uses(&src, false).is_empty(), "{check}");
        }
        // A user function might check it: left alone. A library one reads it.
        assert!(uses("void f(void) {\n    char *p = malloc(4);\n    fill(p);\n}\n", false).is_empty());
        assert_eq!(uses("void f(void) {\n    char *p = malloc(4);\n    strcpy(p, \"x\");\n}\n", false).len(), 1);
        // Not a pointer: not ours.
        assert!(uses("void f(void) {\n    int n = strlen(s);\n    g(n);\n}\n", false).is_empty());
    }

    #[test]
    fn what_a_failed_check_does() {
        let fix = |src: &str| uses(src, false).remove(0).fix;
        assert!(fix("void f(void) {\n    int *p = malloc(4);\n    *p = 1;\n}\n").contains("        return;\n"));
        assert!(
            fix("int *f(void) {\n    int *p = malloc(4);\n    *p = 1;\n    return p;\n}\n").contains("return NULL;")
        );
        assert!(fix("int f(void) {\n    int *p = malloc(4);\n    *p = 1;\n    return 0;\n}\n").contains("return -1;"));
        assert!(fix("int main() {\n    int *p = malloc(4);\n    *p = 1;\n}\n").contains("return 1;"));
        // C++: nullptr, and dynamic_cast.
        let cpp = uses(
            "void f(Base *b) {\n    auto d = dynamic_cast<Derived *>(b);\n    d->run();\n    char *s = std::strdup(\"x\");\n    s[0] = 0;\n}\n",
            true,
        );
        assert_eq!(cpp.len(), 2);
        assert_eq!(cpp[0].from, "dynamic_cast");
        assert!(cpp[0].fix.contains("if (d == nullptr)"));
        assert_eq!(cpp[1].from, "strdup");
    }
}
