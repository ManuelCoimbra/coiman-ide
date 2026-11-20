//! A function's control flow as a Mermaid flowchart: its ifs (yes and no), loops (each time
//! round, and done), returns and breaks, with runs of plain statements as one box. Read with
//! tree-sitter, by the fields grammars share (`condition`, `consequence`, `alternative`,
//! `body`), for the languages Code Health reads.

use tree_sitter::{Node, Parser};

use crate::language::syntax::Lang;

/// Where control comes from into the next box: a node, and the label on that edge.
type Pred = (usize, Option<&'static str>);

#[derive(Default)]
struct Flow {
    /// (shape, label): shape `[`, `{` or `([`.
    nodes: Vec<(&'static str, String)>,
    edges: Vec<(usize, usize, Option<&'static str>)>,
    /// Loops being walked: (the loop's node, its breaks).
    loops: Vec<(usize, Vec<Pred>)>,
    end: usize,
}

const MAX_LABEL: usize = 48;

/// The first line of a node's text, short, for a box.
fn label(node: Node, src: &str) -> String {
    let text = node.utf8_text(src.as_bytes()).unwrap_or_default();
    let first = text.lines().next().unwrap_or_default().trim().trim_end_matches('{').trim();
    let mut out: String = first.chars().take(MAX_LABEL).collect();
    if first.chars().count() > MAX_LABEL {
        out.push('…');
    }
    out
}

fn is_if(kind: &str) -> bool {
    kind == "if_expression" || kind == "if_statement" || kind == "elif_clause"
}

fn is_loop(kind: &str) -> bool {
    [
        "for_expression",
        "while_expression",
        "loop_expression",
        "for_statement",
        "for_in_statement",
        "for_range_loop",
        "enhanced_for_statement",
        "foreach_statement",
        "while_statement",
        "do_statement",
    ]
    .contains(&kind)
}

fn is_return(kind: &str) -> bool {
    kind == "return_expression" || kind == "return_statement"
}

fn is_break(kind: &str) -> bool {
    kind == "break_expression" || kind == "break_statement"
}

fn is_comment(kind: &str) -> bool {
    kind.contains("comment")
}

/// The statements inside a block, or the node itself when it isn't one.
fn statements(node: Node) -> Vec<Node> {
    let kind = node.kind();
    if kind.contains("block") || kind == "compound_statement" || kind == "statement_block" || kind == "body" {
        let mut c = node.walk();
        node.named_children(&mut c).filter(|n| !is_comment(n.kind())).collect()
    } else if kind == "else_clause" {
        // `else { … }` / `else if …`: what follows the keyword.
        let mut c = node.walk();
        node.named_children(&mut c).filter(|n| !is_comment(n.kind())).flat_map(statements).collect()
    } else {
        vec![node]
    }
}

/// An expression statement that only wraps an if or a loop counts as that if or loop.
fn unwrap(node: Node) -> Node {
    if node.kind() == "expression_statement" && node.named_child_count() == 1 {
        let inner = node.named_child(0).unwrap_or(node);
        let k = inner.kind();
        if is_if(k) || is_loop(k) || is_return(k) || is_break(k) {
            return inner;
        }
    }
    node
}

impl Flow {
    fn add(&mut self, shape: &'static str, label: String) -> usize {
        self.nodes.push((shape, label));
        self.nodes.len() - 1
    }

    fn link(&mut self, preds: &[Pred], to: usize) {
        for &(from, label) in preds {
            self.edges.push((from, to, label));
        }
    }

    /// Walks statements in order; returns where control leaves them.
    fn seq(&mut self, stmts: &[Node], mut preds: Vec<Pred>, src: &str) -> Vec<Pred> {
        // Plain statements in a row share one box.
        let mut run: Vec<Node> = Vec::new();
        let flush = |f: &mut Flow, run: &mut Vec<Node>, preds: &mut Vec<Pred>| {
            if run.is_empty() {
                return;
            }
            let mut text = label(run[0], src);
            if run.len() > 1 {
                text.push_str(&format!(" (+{} more)", run.len() - 1));
            }
            let id = f.add("[", text);
            f.link(preds, id);
            *preds = vec![(id, None)];
            run.clear();
        };
        for &stmt in stmts {
            let node = unwrap(stmt);
            let kind = node.kind();
            if is_if(kind) || is_loop(kind) || is_return(kind) || is_break(kind) {
                flush(self, &mut run, &mut preds);
                if preds.is_empty() {
                    break; // unreachable after a return or break
                }
                preds = self.statement(node, preds, src);
            } else {
                run.push(node);
            }
        }
        if !preds.is_empty() {
            flush(self, &mut run, &mut preds);
        }
        preds
    }

    fn statement(&mut self, node: Node, preds: Vec<Pred>, src: &str) -> Vec<Pred> {
        let kind = node.kind();
        if is_if(kind) {
            let cond = node.child_by_field_name("condition").map_or_else(|| label(node, src), |c| label(c, src));
            let id = self.add("{", format!("{cond}?"));
            self.link(&preds, id);
            let then = node.child_by_field_name("consequence").map(statements).unwrap_or_default();
            let mut out = self.seq(&then, vec![(id, Some("yes"))], src);
            // Python lists elif and else as several `alternative`s; others have one.
            let mut c = node.walk();
            let alts: Vec<Node> = node.children_by_field_name("alternative", &mut c).collect();
            match alts.split_first() {
                None => out.push((id, Some("no"))),
                Some((first, rest)) if first.kind() == "elif_clause" => {
                    out.extend(self.elif_chain(*first, rest, vec![(id, Some("no"))], src));
                }
                Some((first, _)) => {
                    let body = if first.kind() == "else_clause" {
                        first.child_by_field_name("body").map_or_else(|| statements(*first), statements)
                    } else {
                        statements(*first)
                    };
                    out.extend(self.seq(&body, vec![(id, Some("no"))], src));
                }
            }
            out
        } else if is_loop(kind) {
            let id = self.add("{", label(node, src));
            self.link(&preds, id);
            self.loops.push((id, Vec::new()));
            let body = node.child_by_field_name("body").map(statements).unwrap_or_default();
            let back = self.seq(&body, vec![(id, Some("each"))], src);
            self.link(&back, id);
            let (_, breaks) = self.loops.pop().unwrap_or((id, Vec::new()));
            let mut out = breaks;
            // `loop { }` only ends by a break.
            if kind != "loop_expression" {
                out.push((id, Some("done")));
            }
            out
        } else if is_return(kind) {
            let id = self.add("[", label(node, src));
            self.link(&preds, id);
            self.edges.push((id, self.end, None));
            Vec::new()
        } else if is_break(kind) {
            let id = self.add("[", label(node, src));
            self.link(&preds, id);
            if let Some((_, breaks)) = self.loops.last_mut() {
                breaks.push((id, None));
            }
            Vec::new()
        } else {
            preds
        }
    }

    /// Python's `elif …: … elif …: … else: …`, one condition after another.
    fn elif_chain(&mut self, elif: Node, rest: &[Node], preds: Vec<Pred>, src: &str) -> Vec<Pred> {
        let cond = elif.child_by_field_name("condition").map_or_else(|| label(elif, src), |c| label(c, src));
        let id = self.add("{", format!("{cond}?"));
        self.link(&preds, id);
        let then = elif.child_by_field_name("consequence").map(statements).unwrap_or_default();
        let mut out = self.seq(&then, vec![(id, Some("yes"))], src);
        match rest.split_first() {
            None => out.push((id, Some("no"))),
            Some((next, more)) if next.kind() == "elif_clause" => {
                out.extend(self.elif_chain(*next, more, vec![(id, Some("no"))], src))
            }
            Some((next, _)) => {
                let body = next.child_by_field_name("body").map_or_else(|| statements(*next), statements);
                out.extend(self.seq(&body, vec![(id, Some("no"))], src));
            }
        }
        out
    }

    fn mermaid(&self) -> String {
        let mut out = String::from("flowchart TD\n");
        for (i, (shape, text)) in self.nodes.iter().enumerate() {
            let close = match *shape {
                "{" => "}",
                "([" => "])",
                _ => "]",
            };
            let text = text.replace('"', "#quot;");
            out.push_str(&format!("    n{i}{shape}\"{text}\"{close}\n"));
        }
        for (from, to, label) in &self.edges {
            match label {
                Some(l) => out.push_str(&format!("    n{from} -->|{l}| n{to}\n")),
                None => out.push_str(&format!("    n{from} --> n{to}\n")),
            }
        }
        out
    }
}

/// The function around byte `offset` of `src`: its name, 1-based line, and flowchart.
pub fn function_flow(lang: Lang, src: &str, offset: usize) -> Option<(String, usize, String)> {
    let functions = super::complexity::function_kinds(lang)?;
    let mut parser = Parser::new();
    parser.set_language(&lang.language()).ok()?;
    let tree = parser.parse(src, None)?;
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset)?;
    while !functions.contains(&node.kind()) {
        node = node.parent()?;
    }
    let name = super::complexity::name_of_function(node, src.as_bytes());
    let body = node.child_by_field_name("body")?;
    let mut flow = Flow::default();
    let start = flow.add("([", format!("{name}()"));
    flow.end = flow.add("([", "end".to_owned());
    let exits = flow.seq(&statements(body), vec![(start, None)], src);
    let end = flow.end;
    flow.link(&exits, end);
    Some((name, node.start_position().row + 1, flow.mermaid()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_if_loop_and_return() {
        let src = "fn pick(xs: &[i32]) -> i32 {\n    let mut best = 0;\n    for x in xs {\n        if *x > best {\n            best = *x;\n        } else {\n            continue;\n        }\n    }\n    if best == 0 {\n        return -1;\n    }\n    best\n}\n";
        let (name, line, chart) = function_flow(Lang::Rust, src, src.find("best = *x").unwrap()).unwrap();
        assert_eq!((name.as_str(), line), ("pick", 1));
        for want in [
            "n0([\"pick()\"])",
            "n2[\"let mut best = 0;\"]",
            "n3{\"for x in xs\"}",
            "n4{\"*x > best?\"}",
            "n0 --> n2",
            "n3 -->|each| n4",
            "n4 -->|yes| n5",
            "n3 -->|done| n7",
            "n8[\"return -1\"]",
            "n8 --> n1",
        ] {
            assert!(chart.contains(want), "{want} missing:\n{chart}");
        }
    }

    #[test]
    fn python_elif_chain_and_quotes() {
        let src = "def size(n):\n    if n < 10:\n        return \"small\"\n    elif n < 100:\n        x = 1\n    else:\n        x = 2\n    return x\n";
        let (_, _, chart) = function_flow(Lang::Python, src, 5).unwrap();
        assert!(chart.contains("{\"n < 10?\"}") && chart.contains("{\"n < 100?\"}"), "{chart}");
        assert!(chart.contains("return #quot;small#quot;"), "{chart}");
        assert!(chart.contains("-->|no|"), "{chart}");
        assert!(function_flow(Lang::Python, "x = 1\n", 0).is_none(), "not in a function");
    }
}
