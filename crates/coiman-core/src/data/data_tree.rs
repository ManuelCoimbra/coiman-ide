//! JSON, YAML and TOML as a tree of keys and values, in the file's own order, and tidying JSON
//! without reordering or rewriting anything in it.

use tree_sitter::{Node, Parser};

use crate::language::syntax::Lang;

#[derive(Debug, PartialEq, Eq)]
pub struct DataNode {
    pub key: String,
    /// The value as written; empty for one with children.
    pub value: String,
    /// 0-based line where it starts.
    pub line: usize,
    pub children: Vec<DataNode>,
}

fn named<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).filter(|c| c.kind() != "comment").collect()
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn unquoted(key: &str) -> String {
    key.trim().trim_matches(['"', '\'']).to_owned()
}

fn leaf(key: String, value: String, line: usize) -> DataNode {
    DataNode { key, value, line, children: Vec::new() }
}

/// What's in a file with one of those formats; `None` for any other.
pub fn parse(extension: &str, source: &str) -> Option<Vec<DataNode>> {
    let lang = Lang::from_extension(extension).filter(|l| matches!(l, Lang::Json | Lang::Toml | Lang::Yaml))?;
    let mut parser = Parser::new();
    parser.set_language(&lang.language()).ok()?;
    let tree = parser.parse(source, None)?;
    let root = tree.root_node();
    Some(match lang {
        Lang::Json => named(root).first().map_or_else(Vec::new, |top| json_children(*top, source)),
        Lang::Toml => toml_entries(root, source),
        _ => yaml_children(root, source),
    })
}

fn json_children(node: Node, source: &str) -> Vec<DataNode> {
    match node.kind() {
        "object" => named(node)
            .into_iter()
            .filter(|n| n.kind() == "pair")
            .filter_map(|pair| {
                let key = unquoted(text(pair.child_by_field_name("key")?, source));
                Some(json_node(key, pair.child_by_field_name("value")?, source))
            })
            .collect(),
        "array" => named(node).into_iter().enumerate().map(|(i, v)| json_node(format!("[{i}]"), v, source)).collect(),
        _ => vec![json_node(String::new(), node, source)],
    }
}

fn json_node(key: String, value: Node, source: &str) -> DataNode {
    let line = value.start_position().row;
    if matches!(value.kind(), "object" | "array") {
        DataNode { key, value: String::new(), line, children: json_children(value, source) }
    } else {
        leaf(key, text(value, source).to_owned(), line)
    }
}

/// A YAML node without what wraps it: documents, block and flow nodes, tags and anchors.
fn yaml_inner(node: Node) -> Node {
    let mut at = node;
    while matches!(at.kind(), "stream" | "document" | "block_node" | "flow_node") {
        let Some(next) = named(at).into_iter().rfind(|c| !matches!(c.kind(), "tag" | "anchor")) else { break };
        at = next;
    }
    at
}

fn yaml_children(node: Node, source: &str) -> Vec<DataNode> {
    let inner = yaml_inner(node);
    match inner.kind() {
        "block_mapping" | "flow_mapping" => named(inner)
            .into_iter()
            .filter_map(|pair| {
                let key = unquoted(text(pair.child_by_field_name("key")?, source));
                Some(yaml_node(key, pair.child_by_field_name("value"), pair, source))
            })
            .collect(),
        "block_sequence" | "flow_sequence" => named(inner)
            .into_iter()
            .enumerate()
            .map(|(i, item)| {
                let value =
                    if item.kind() == "block_sequence_item" { named(item).first().copied() } else { Some(item) };
                yaml_node(format!("[{i}]"), value, item, source)
            })
            .collect(),
        "stream" | "document" | "ERROR" => Vec::new(),
        _ => vec![leaf(String::new(), text(inner, source).to_owned(), inner.start_position().row)],
    }
}

fn yaml_node(key: String, value: Option<Node>, place: Node, source: &str) -> DataNode {
    let line = place.start_position().row;
    let Some(value) = value else { return leaf(key, String::new(), line) };
    let inner = yaml_inner(value);
    if matches!(inner.kind(), "block_mapping" | "flow_mapping" | "block_sequence" | "flow_sequence") {
        DataNode { key, value: String::new(), line, children: yaml_children(inner, source) }
    } else {
        leaf(key, text(inner, source).to_owned(), line)
    }
}

fn toml_entries(node: Node, source: &str) -> Vec<DataNode> {
    named(node)
        .into_iter()
        .filter_map(|entry| {
            let line = entry.start_position().row;
            match entry.kind() {
                "pair" => toml_pair(entry, source),
                "table" | "table_array_element" => {
                    let name = named(entry).first().map_or("", |key| text(*key, source));
                    let key = if entry.kind() == "table" { name.to_owned() } else { format!("[[{name}]]") };
                    Some(DataNode { key, value: String::new(), line, children: toml_entries(entry, source) })
                }
                _ => None,
            }
        })
        .collect()
}

fn toml_pair(pair: Node, source: &str) -> Option<DataNode> {
    let parts = named(pair);
    let (key, value) = (parts.first()?, parts.get(1)?);
    Some(toml_node(unquoted(text(*key, source)), *value, source))
}

fn toml_node(key: String, value: Node, source: &str) -> DataNode {
    let line = value.start_position().row;
    match value.kind() {
        "inline_table" => DataNode { key, value: String::new(), line, children: toml_entries(value, source) },
        "array" => {
            let items = named(value).into_iter().enumerate();
            let children = items.map(|(i, v)| toml_node(format!("[{i}]"), v, source)).collect();
            DataNode { key, value: String::new(), line, children }
        }
        _ => leaf(key, text(value, source).to_owned(), line),
    }
}

/// Lays JSON out with two spaces, keeping the order of keys and each value exactly as written.
pub fn format_json(source: &str) -> Result<String, String> {
    serde_json::from_str::<serde::de::IgnoredAny>(source).map_err(|e| format!("Not valid JSON: {e}"))?;
    let mut out = String::with_capacity(source.len() + source.len() / 4);
    let mut depth = 0usize;
    let mut chars = source.chars().peekable();
    let newline = |out: &mut String, depth: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(depth));
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                out.push('"');
                while let Some(inner) = chars.next() {
                    out.push(inner);
                    match inner {
                        '\\' => out.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '{' | '[' => {
                out.push(c);
                while chars.peek().is_some_and(|n| n.is_whitespace()) {
                    chars.next();
                }
                if matches!(chars.peek(), Some('}' | ']')) {
                    out.extend(chars.next());
                } else {
                    depth += 1;
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(',');
                newline(&mut out, depth);
            }
            ':' => out.push_str(": "),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(nodes: &[DataNode]) -> Vec<&str> {
        nodes.iter().map(|n| n.key.as_str()).collect()
    }

    #[test]
    fn reads_json_in_its_own_order() {
        let tree = parse("json", "{\n  \"z\": 1,\n  \"a\": { \"b\": [true, \"x\"] }\n}\n").unwrap();
        assert_eq!(keys(&tree), ["z", "a"]);
        assert_eq!((tree[0].value.as_str(), tree[0].line), ("1", 1));
        let list = &tree[1].children[0];
        assert_eq!((list.key.as_str(), keys(&list.children)), ("b", vec!["[0]", "[1]"]));
        assert_eq!(list.children[1].value, "\"x\"");
    }

    #[test]
    fn reads_yaml() {
        let source =
            "# note\nname: web\nlist:\n  - one\n  - two: 2\nflow: {a: 1, b: [x, y]}\n'quoted': \"v\"\nempty:\n";
        let tree = parse("yaml", source).unwrap();
        assert_eq!(keys(&tree), ["name", "list", "flow", "quoted", "empty"]);
        assert_eq!(tree[0].value, "web");
        assert_eq!(keys(&tree[1].children), ["[0]", "[1]"]);
        assert_eq!(tree[1].children[0].value, "one");
        assert_eq!(tree[1].children[1].children[0].key, "two");
        assert_eq!(keys(&tree[2].children), ["a", "b"]);
        assert_eq!(keys(&tree[2].children[1].children), ["[0]", "[1]"]);
        assert_eq!(tree[1].line, 2);
    }

    #[test]
    fn reads_toml() {
        let source =
            "top = 1\n[package]\nname = \"x\" # note\nlist = [1, 2]\ninline = { a = 1 }\n[[bin]]\nname = \"y\"\n";
        let tree = parse("toml", source).unwrap();
        assert_eq!(keys(&tree), ["top", "package", "[[bin]]"]);
        assert_eq!(keys(&tree[1].children), ["name", "list", "inline"]);
        assert_eq!(tree[1].children[0].value, "\"x\"");
        assert_eq!(keys(&tree[1].children[1].children), ["[0]", "[1]"]);
        assert_eq!(tree[1].children[2].children[0].key, "a");
        assert_eq!(tree[2].line, 5);
    }

    #[test]
    fn other_files_have_no_tree() {
        assert!(parse("rs", "fn main() {}").is_none());
        assert_eq!(parse("json", ""), Some(Vec::new()));
    }

    #[test]
    fn tidies_json_without_changing_it() {
        let messy = "{\"z\":1,  \"a\":[ true,{\"k\": \"s , : { \\\" \"} ], \"e\":{ }, \"f\":[]}";
        let tidy = format_json(messy).unwrap();
        assert_eq!(
            tidy,
            "{\n  \"z\": 1,\n  \"a\": [\n    true,\n    {\n      \"k\": \"s , : { \\\" \"\n    }\n  ],\n  \"e\": {},\n  \"f\": []\n}\n"
        );
        assert_eq!(format_json(&tidy).unwrap(), tidy);
        // Numbers stay as written, where a parser would change them.
        assert!(format_json("[1.10, 1e5]").unwrap().contains("1.10"));
        assert!(format_json("{\"a\":}").unwrap_err().contains("Not valid JSON"));
    }
}
