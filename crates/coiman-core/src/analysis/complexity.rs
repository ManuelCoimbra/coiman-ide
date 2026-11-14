//! How complex each function is: its cyclomatic complexity (1, plus one for each branch:
//! if, loop, case, catch, `&&`, `||`, `?:`), and its length, read with tree-sitter.

use std::path::Path;

use tree_sitter::{Node, Parser};

use crate::language::syntax::Lang;

/// Files larger than this aren't parsed.
const MAX_BYTES: u64 = 512 * 1024;

/// One function and how complex it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// Relative to the project, with `/`.
    pub path: String,
    pub name: String,
    /// 1-based line where it starts.
    pub line: usize,
    pub lines: usize,
    pub complexity: usize,
}

/// What a language calls a function, and what's a branch in it.
struct Kinds {
    functions: &'static [&'static str],
    branches: &'static [&'static str],
    /// Binary operators that branch (`&&`, `||`, `??`), checked on binary expressions.
    logical: &'static [&'static str],
}

fn kinds(lang: Lang) -> Option<Kinds> {
    let c_like_logic: &[&str] = &["&&", "||"];
    Some(match lang {
        Lang::Rust => Kinds {
            functions: &["function_item", "closure_expression"],
            branches: &[
                "if_expression",
                "while_expression",
                "for_expression",
                "loop_expression",
                "match_arm",
                "try_expression",
            ],
            logical: c_like_logic,
        },
        Lang::Python => Kinds {
            functions: &["function_definition", "lambda"],
            branches: &[
                "if_statement",
                "elif_clause",
                "for_statement",
                "while_statement",
                "except_clause",
                "conditional_expression",
                "boolean_operator",
                "case_clause",
                "for_in_clause",
                "if_clause",
            ],
            logical: &[],
        },
        Lang::JavaScript | Lang::Jsx | Lang::TypeScript | Lang::Tsx => Kinds {
            functions: &[
                "function_declaration",
                "function_expression",
                "arrow_function",
                "method_definition",
                "generator_function_declaration",
            ],
            branches: &[
                "if_statement",
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_case",
                "catch_clause",
                "ternary_expression",
            ],
            logical: &["&&", "||", "??"],
        },
        Lang::Java => Kinds {
            functions: &["method_declaration", "constructor_declaration", "lambda_expression"],
            branches: &[
                "if_statement",
                "for_statement",
                "enhanced_for_statement",
                "while_statement",
                "do_statement",
                "switch_label",
                "catch_clause",
                "ternary_expression",
            ],
            logical: c_like_logic,
        },
        Lang::C | Lang::Cpp => Kinds {
            functions: &["function_definition", "lambda_expression"],
            branches: &[
                "if_statement",
                "for_statement",
                "for_range_loop",
                "while_statement",
                "do_statement",
                "case_statement",
                "catch_clause",
                "conditional_expression",
            ],
            logical: c_like_logic,
        },
        Lang::CSharp => Kinds {
            functions: &[
                "method_declaration",
                "constructor_declaration",
                "local_function_statement",
                "lambda_expression",
            ],
            branches: &[
                "if_statement",
                "for_statement",
                "foreach_statement",
                "while_statement",
                "do_statement",
                "switch_section",
                "catch_clause",
                "conditional_expression",
            ],
            logical: &["&&", "||", "??"],
        },
        Lang::Go => Kinds {
            functions: &["function_declaration", "method_declaration", "func_literal"],
            branches: &["if_statement", "for_statement", "expression_case", "type_case", "communication_case"],
            logical: c_like_logic,
        },
        _ => return None,
    })
}

/// A function's name: its `name` field, else "(anonymous)" with what holds it.
fn name_of(node: Node, src: &[u8]) -> String {
    let named = node.child_by_field_name("name").or_else(|| {
        // C and C++: the name is inside the declarator.
        let mut d = node.child_by_field_name("declarator")?;
        while let Some(inner) = d.child_by_field_name("declarator") {
            d = inner;
        }
        Some(d)
    });
    match named.and_then(|n| n.utf8_text(src).ok()) {
        Some(text) => text.to_owned(),
        None => {
            // A function given to a variable: `const f = () => …`.
            let parent = node.parent();
            parent
                .and_then(|p| p.child_by_field_name("name"))
                .and_then(|n| n.utf8_text(src).ok())
                .map_or_else(|| "(anonymous)".to_owned(), |n| format!("{n} (anonymous)"))
        }
    }
}

/// Branches in `node`, not counting functions inside it (they're measured on their own).
fn branches(node: Node, k: &Kinds, top: bool) -> usize {
    if !top && k.functions.contains(&node.kind()) {
        return 0;
    }
    let mut count = usize::from(k.branches.contains(&node.kind()));
    if !k.logical.is_empty()
        && node.kind().contains("binary")
        && let Some(op) = node.child_by_field_name("operator")
        && k.logical.contains(&op.kind())
    {
        count += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += branches(child, k, false);
    }
    count
}

fn collect(node: Node, k: &Kinds, src: &[u8], path: &str, out: &mut Vec<Function>) {
    if k.functions.contains(&node.kind()) {
        let (start, end) = (node.start_position().row, node.end_position().row);
        out.push(Function {
            path: path.to_owned(),
            name: name_of(node, src),
            line: start + 1,
            lines: end - start + 1,
            complexity: 1 + branches(node, k, true),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, k, src, path, out);
    }
}

/// The functions of one file's text, in `lang`.
pub fn functions_in(lang: Lang, path: &str, text: &str) -> Vec<Function> {
    let Some(k) = kinds(lang) else { return Vec::new() };
    let mut parser = Parser::new();
    if parser.set_language(&lang.language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else { return Vec::new() };
    let mut out = Vec::new();
    collect(tree.root_node(), &k, text.as_bytes(), path, &mut out);
    // A one-line closure or lambda says little: only those with branches stay.
    out.retain(|f| f.complexity > 1 || f.lines > 1);
    out
}

/// Every function in the project, most complex first.
pub fn scan(root: &Path) -> Vec<Function> {
    let mut out = Vec::new();
    for entry in super::project_files(root, false) {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) || entry.metadata().map_or(true, |m| m.len() > MAX_BYTES) {
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(lang) = Lang::from_extension(&ext) else { continue };
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        out.extend(functions_in(lang, &rel, &text));
    }
    out.sort_by(|a, b| b.complexity.cmp(&a.complexity).then(b.lines.cmp(&a.lines)).then(a.path.cmp(&b.path)));
    out
}

/// How a complexity reads: up to 10 is fine, to 20 is complex, above is very complex.
pub fn rating(complexity: usize) -> &'static str {
    match complexity {
        0..=10 => "simple",
        11..=20 => "complex",
        _ => "very complex",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(lang: Lang, text: &str) -> Vec<(String, usize)> {
        functions_in(lang, "f", text).into_iter().map(|f| (f.name, f.complexity)).collect()
    }

    #[test]
    fn rust_branches_count() {
        let text = "fn plain() -> i32 {\n    1\n}\n\nfn busy(x: i32, y: bool) -> Option<i32> {\n    if x > 0 && y {\n        return Some(1);\n    }\n    for _ in 0..3 {}\n    match x {\n        0 => None,\n        1 => Some(2),\n        _ => Some(3),\n    }\n}\n";
        assert_eq!(measure(Lang::Rust, text), [("plain".to_owned(), 1), ("busy".to_owned(), 7)]);
    }

    #[test]
    fn python_javascript_and_c_count_their_own_way() {
        let py = "def f(a, b):\n    if a and b:\n        return 1\n    elif a:\n        return 2\n    for x in b:\n        pass\n    return 3\n";
        assert_eq!(measure(Lang::Python, py), [("f".to_owned(), 5)]);
        let js = "function f(a) {\n  if (a ?? false) { return 1 }\n  const g = (b) => {\n    return b ? 1 : 2;\n  };\n  return a || 0;\n}\n";
        assert_eq!(measure(Lang::JavaScript, js), [("f".to_owned(), 4), ("g (anonymous)".to_owned(), 2)]);
        let c = "int max(int a, int b) {\n    if (a > b) return a;\n    return b;\n}\n";
        assert_eq!(measure(Lang::C, c), [("max".to_owned(), 2)]);
    }

    #[test]
    fn ratings() {
        assert_eq!((rating(4), rating(15), rating(30)), ("simple", "complex", "very complex"));
    }
}
