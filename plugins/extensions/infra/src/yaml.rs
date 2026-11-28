//! A small YAML reader that keeps where everything is: block mappings and sequences, plain,
//! quoted and block scalars, one-line flow collections, several documents in a file, and the
//! keys a mapping has twice. What YAML itself rejects comes from yaml-rust's scanner, which
//! knows the whole grammar; this reader is for finding things by line.

/// A value and the 1-based line and 0-based column it starts at.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub value: Value,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Map(Vec<Entry>),
    Seq(Vec<Node>),
    Scalar(String),
    Null,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: String,
    pub line: usize,
    pub col: usize,
    pub value: Node,
}

/// A key a mapping has more than once: where it is the second time.
#[derive(Clone, Debug, PartialEq)]
pub struct Duplicate {
    pub key: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub root: Node,
    pub duplicates: Vec<Duplicate>,
}

/// What YAML rejects in a text: a message, its 1-based line and 0-based column.
#[derive(Clone, Debug, PartialEq)]
pub struct SyntaxError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl Node {
    fn null(line: usize, col: usize) -> Node {
        Node { value: Value::Null, line, col }
    }

    /// The value of a key, if this is a mapping that has it.
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entry(key).map(|e| &e.value)
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        match &self.value {
            // The last of a key that is there twice: YAML readers keep that one.
            Value::Map(entries) => entries.iter().rev().find(|e| e.key == key),
            _ => None,
        }
    }

    /// Follows keys down: `at(&["spec", "template"])`.
    pub fn at(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |n, k| n.get(k))
    }

    pub fn str(&self) -> Option<&str> {
        match &self.value {
            Value::Scalar(s) => Some(s),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        match &self.value {
            Value::Map(e) => e,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Node] {
        match &self.value {
            Value::Seq(i) => i,
            _ => &[],
        }
    }

    pub fn is_map(&self) -> bool {
        matches!(self.value, Value::Map(_))
    }
}

/// What YAML rejects in the text, if anything (the first problem only: the scanner stops there).
pub fn syntax_error(text: &str) -> Option<SyntaxError> {
    match yaml_rust::YamlLoader::load_from_str(text) {
        Ok(_) => None,
        Err(e) => Some(SyntaxError {
            message: e.to_string().split(" at line ").next().unwrap_or_default().to_owned(),
            line: e.marker().line().max(1),
            col: e.marker().col(),
        }),
    }
}

/// A line that holds something: its indentation, its text from there, and its number.
#[derive(Clone, Debug)]
struct Line {
    indent: usize,
    text: String,
    line: usize,
    /// The lines of a block scalar (`|`, `>`) that follow it.
    block: Option<String>,
}

/// Cuts a comment off a line: a `#` at its start or after a space, outside quotes.
fn strip_comment(s: &str) -> &str {
    let (mut single, mut double) = (false, false);
    let mut prev = ' ';
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single && prev != '\\' => double = !double,
            '#' if !single && !double && (prev == ' ' || prev == '\t' || i == 0) => return s[..i].trim_end(),
            _ => {}
        }
        prev = c;
    }
    s.trim_end()
}

/// Where a key's `:` is: the first one followed by a space or the end, outside quotes and
/// brackets.
fn key_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let (mut single, mut double, mut depth) = (false, false, 0i32);
    if s.starts_with('[') || s.starts_with('{') {
        return None;
    }
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single && (i == 0 || bytes[i - 1] != b'\\') => double = !double,
            '[' | '{' if !single && !double => depth += 1,
            ']' | '}' if !single && !double => depth -= 1,
            ':' if !single
                && !double
                && depth <= 0
                && (i + 1 == s.len() || bytes[i + 1] == b' ' || bytes[i + 1] == b'\t') =>
            {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return s[1..s.len() - 1].replace("''", "'");
    }
    s.to_owned()
}

/// Leaves out an anchor (`&a`) and a tag (`!!str`) before a value.
fn strip_props(mut s: &str) -> &str {
    loop {
        let t = s.trim_start();
        if (t.starts_with('&') || t.starts_with('!')) && !t.starts_with("!=") {
            match t.find([' ', '\t']) {
                Some(i) => s = &t[i..],
                None => return "",
            }
        } else {
            return t;
        }
    }
}

fn is_block_indicator(s: &str) -> bool {
    let s = s.trim();
    (s.starts_with('|') || s.starts_with('>')) && s[1..].chars().all(|c| matches!(c, '-' | '+' | '0'..='9'))
}

/// The value part of a line, after `- ` and `key: ` heads, to know if a block scalar starts.
fn tail(text: &str) -> &str {
    let mut t = text;
    loop {
        if let Some(rest) = t.strip_prefix("- ") {
            t = rest.trim_start();
        } else if let Some(c) = key_colon(t) {
            t = t[c + 1..].trim_start();
        } else {
            return t;
        }
    }
}

/// Splits a text into documents of lines that hold something.
fn lines(text: &str) -> Vec<Vec<Line>> {
    let mut docs = vec![Vec::new()];
    let raw: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < raw.len() {
        let l = raw[i].trim_end_matches('\r');
        i += 1;
        if l.starts_with("---") && (l.len() == 3 || l[3..].starts_with([' ', '\t'])) {
            docs.push(Vec::new());
            let rest = strip_comment(l[3..].trim());
            if !rest.is_empty() {
                docs.last_mut().expect("a document").push(Line {
                    indent: 4,
                    text: rest.to_owned(),
                    line: i,
                    block: None,
                });
            }
            continue;
        }
        if l == "..." || l.trim_start().starts_with('%') && l.starts_with('%') {
            continue;
        }
        let content = strip_comment(l);
        if content.trim().is_empty() {
            continue;
        }
        let indent = content.len() - content.trim_start().len();
        let text = content.trim_start().to_owned();
        let mut line = Line { indent, text, line: i, block: None };
        if is_block_indicator(tail(&line.text)) {
            // Its lines: blank ones and those indented more than it.
            let mut block = Vec::new();
            while i < raw.len() {
                let b = raw[i].trim_end_matches('\r');
                let bi = b.len() - b.trim_start().len();
                if !b.trim().is_empty() && bi <= indent {
                    break;
                }
                block.push(b);
                i += 1;
            }
            while block.last().is_some_and(|b| b.trim().is_empty()) {
                block.pop();
            }
            let cut = block.iter().filter(|b| !b.trim().is_empty()).map(|b| b.len() - b.trim_start().len()).min();
            let cut = cut.unwrap_or(0);
            line.block = Some(block.iter().map(|b| b.get(cut..).unwrap_or("")).collect::<Vec<_>>().join("\n"));
        } else if (tail(&line.text).starts_with('[') || tail(&line.text).starts_with('{'))
            && !balanced(tail(&line.text))
        {
            // A flow collection over several lines: one line here.
            while i < raw.len() && !balanced(tail(&line.text)) {
                line.text.push(' ');
                line.text.push_str(strip_comment(raw[i].trim()));
                i += 1;
            }
        }
        docs.last_mut().expect("a document").push(line);
    }
    // A first `---` leaves an empty document before it.
    if docs.len() > 1 && docs[0].is_empty() {
        docs.remove(0);
    }
    docs
}

fn balanced(s: &str) -> bool {
    let (mut depth, mut single, mut double) = (0i32, false, false);
    for c in s.chars() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' | '{' if !single && !double => depth += 1,
            ']' | '}' if !single && !double => depth -= 1,
            _ => {}
        }
    }
    depth <= 0
}

struct Parser {
    lines: Vec<Line>,
    i: usize,
    duplicates: Vec<Duplicate>,
}

impl Parser {
    fn peek(&self) -> Option<&Line> {
        self.lines.get(self.i)
    }

    /// A node from the line at `i`, which is indented at least `min`.
    fn node(&mut self, min: usize) -> Node {
        let Some(l) = self.peek().cloned() else { return Node::null(0, 0) };
        if l.indent < min {
            return Node::null(l.line, l.indent);
        }
        if l.text == "-" || l.text.starts_with("- ") {
            return self.seq(l.indent);
        }
        if key_colon(&l.text).is_some() {
            return self.map(l.indent);
        }
        // A scalar, maybe over several lines.
        self.i += 1;
        let mut text = l.text.clone();
        if let Some(b) = &l.block {
            text = b.clone();
        }
        while let Some(n) = self.peek() {
            if n.indent < min {
                break;
            }
            if key_colon(&n.text).is_some() || n.text.starts_with("- ") {
                break;
            }
            text.push(' ');
            text.push_str(&n.text);
            self.i += 1;
        }
        Node { value: scalar(&text), line: l.line, col: l.indent }
    }

    fn seq(&mut self, indent: usize) -> Node {
        let first = self.peek().map(|l| l.line).unwrap_or(0);
        let mut items = Vec::new();
        while let Some(l) = self.peek().cloned() {
            if l.indent != indent || !(l.text == "-" || l.text.starts_with("- ")) {
                break;
            }
            if l.text == "-" {
                self.i += 1;
                let child = match self.peek() {
                    Some(n) if n.indent > indent => self.node(indent + 1),
                    _ => Node::null(l.line, indent),
                };
                items.push(child);
                continue;
            }
            // What follows `- ` is read as a line of its own, further in.
            let rest = &l.text[2..];
            let skip = rest.len() - rest.trim_start().len();
            let col = indent + 2 + skip;
            self.lines[self.i] = Line { indent: col, text: rest.trim_start().to_owned(), line: l.line, block: l.block };
            items.push(self.node(col));
        }
        Node { value: Value::Seq(items), line: first, col: indent }
    }

    fn map(&mut self, indent: usize) -> Node {
        let first = self.peek().map(|l| l.line).unwrap_or(0);
        let mut entries: Vec<Entry> = Vec::new();
        while let Some(l) = self.peek().cloned() {
            if l.indent != indent {
                break;
            }
            let Some(c) = key_colon(&l.text) else { break };
            if l.text.starts_with("- ") {
                break;
            }
            let key = unquote(&l.text[..c]);
            let rest = strip_props(&l.text[c + 1..]).trim().to_owned();
            let vcol = indent + c + 1 + (l.text[c + 1..].len() - l.text[c + 1..].trim_start().len());
            self.i += 1;
            let value = if let Some(b) = &l.block {
                Node { value: Value::Scalar(b.clone()), line: l.line, col: vcol }
            } else if rest.is_empty() {
                match self.peek() {
                    Some(n) if n.indent > indent => self.node(indent + 1),
                    // A sequence may sit at its key's indentation.
                    Some(n) if n.indent == indent && (n.text == "-" || n.text.starts_with("- ")) => self.seq(indent),
                    _ => Node::null(l.line, vcol),
                }
            } else if key_colon(&rest).is_some() && !rest.starts_with(['"', '\'', '{', '[']) {
                // `a: b: c` is not YAML; yaml-rust says so. Keep the text.
                Node { value: Value::Scalar(rest.clone()), line: l.line, col: vcol }
            } else {
                // A plain scalar may go on over lines indented more.
                let mut text = rest.clone();
                while let Some(n) = self.peek() {
                    if n.indent <= indent || key_colon(&n.text).is_some() || n.text.starts_with("- ") {
                        break;
                    }
                    text.push(' ');
                    text.push_str(&n.text);
                    self.i += 1;
                }
                Node { value: scalar(&text), line: l.line, col: vcol }
            };
            if entries.iter().any(|e| e.key == key) && key != "<<" {
                self.duplicates.push(Duplicate { key: key.clone(), line: l.line, col: indent });
            }
            entries.push(Entry { key, line: l.line, col: indent, value });
        }
        Node { value: Value::Map(entries), line: first, col: indent }
    }
}

/// A scalar or a one-line flow collection.
fn scalar(text: &str) -> Value {
    let t = text.trim();
    if t.starts_with('[') || t.starts_with('{') {
        return flow(t);
    }
    if t.is_empty() || t == "~" || t == "null" {
        return Value::Null;
    }
    Value::Scalar(unquote(t))
}

/// Splits a flow collection's inside at its top-level commas.
fn split_flow(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let (mut depth, mut single, mut double) = (0i32, false, false);
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '\'' if !double => single = !single,
            '"' if !single => double = !double,
            '[' | '{' if !single && !double => depth += 1,
            ']' | '}' if !single && !double => depth -= 1,
            ',' if depth == 0 && !single && !double => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    parts.into_iter().map(|p| p.trim().to_owned()).filter(|p| !p.is_empty()).collect()
}

fn flow(t: &str) -> Value {
    let inner = &t[1..t.len().saturating_sub(1).max(1)];
    if t.starts_with('[') {
        Value::Seq(split_flow(inner).iter().map(|p| Node { value: scalar(p), line: 0, col: 0 }).collect())
    } else {
        Value::Map(
            split_flow(inner)
                .iter()
                .map(|p| {
                    let (k, v) = match p.find(':') {
                        Some(c) => (&p[..c], &p[c + 1..]),
                        None => (p.as_str(), ""),
                    };
                    Entry { key: unquote(k), line: 0, col: 0, value: Node { value: scalar(v), line: 0, col: 0 } }
                })
                .collect(),
        )
    }
}

/// Fills in lines for what a flow collection holds: the line of the collection itself.
fn place(node: &mut Node, line: usize, col: usize) {
    if node.line == 0 {
        node.line = line;
        node.col = col;
    }
    let (l, c) = (node.line, node.col);
    match &mut node.value {
        Value::Map(entries) => {
            for e in entries {
                if e.line == 0 {
                    e.line = l;
                    e.col = c;
                }
                place(&mut e.value, e.line, e.col);
            }
        }
        Value::Seq(items) => {
            for i in items {
                place(i, l, c);
            }
        }
        _ => {}
    }
}

/// Reads every document of a text. A document with nothing in it is left out.
pub fn read(text: &str) -> Vec<Doc> {
    lines(text)
        .into_iter()
        .filter(|l| !l.is_empty())
        .map(|lines| {
            let min = lines.iter().map(|l| l.indent).min().unwrap_or(0);
            let mut p = Parser { lines, i: 0, duplicates: Vec::new() };
            let mut root = p.node(min);
            // What is left (lines indented less than the first): more of the top level.
            while p.i < p.lines.len() {
                let (at, indent) = (p.i, p.lines[p.i].indent);
                let more = p.node(indent);
                if p.i == at {
                    p.i += 1;
                }
                if let (Value::Map(a), Value::Map(b)) = (&mut root.value, more.value) {
                    a.extend(b);
                }
            }
            place(&mut root, 1, 0);
            Doc { root, duplicates: p.duplicates }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Node {
        let mut d = read(text);
        assert_eq!(d.len(), 1, "{d:?}");
        d.remove(0).root
    }

    #[test]
    fn maps_and_lines() {
        let n = one("apiVersion: v1\nkind: Service\nmetadata:\n  name: web\n  labels:\n    app: web\n");
        assert_eq!(n.get("kind").and_then(Node::str), Some("Service"));
        assert_eq!(n.at(&["metadata", "name"]).and_then(Node::str), Some("web"));
        let labels = n.entry("metadata").unwrap().value.entry("labels").unwrap();
        assert_eq!(labels.line, 5);
        assert_eq!(n.at(&["metadata", "labels", "app"]).unwrap().line, 6);
    }

    #[test]
    fn sequences_of_maps() {
        let n = one(
            "spec:\n  containers:\n    - name: web\n      image: nginx:1.25\n    - name: side\n      image: busybox\n",
        );
        let c = n.at(&["spec", "containers"]).unwrap().items();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].get("image").and_then(Node::str), Some("busybox"));
        assert_eq!(c[1].get("image").unwrap().line, 6);
        assert_eq!(c[0].line, 3);
    }

    #[test]
    fn sequence_at_its_keys_indent() {
        let n = one("tasks:\n- name: a\n  debug: msg=hi\n- name: b\nhosts: all\n");
        assert_eq!(n.get("tasks").unwrap().items().len(), 2);
        assert_eq!(n.get("hosts").and_then(Node::str), Some("all"));
    }

    #[test]
    fn top_level_sequence() {
        let n = one("- hosts: web\n  tasks:\n    - name: x\n      ping:\n- import_playbook: other.yml\n");
        assert_eq!(n.items().len(), 2);
        assert_eq!(n.items()[0].get("tasks").unwrap().items()[0].get("name").and_then(Node::str), Some("x"));
        assert_eq!(n.items()[1].get("import_playbook").unwrap().line, 5);
    }

    #[test]
    fn scalars_quotes_comments() {
        let n = one("a: \"x # not a comment\"  # a comment\nb: 'it''s'\nc: plain # gone\nd: ~\ne:\n");
        assert_eq!(n.get("a").and_then(Node::str), Some("x # not a comment"));
        assert_eq!(n.get("b").and_then(Node::str), Some("it's"));
        assert_eq!(n.get("c").and_then(Node::str), Some("plain"));
        assert_eq!(n.get("d").unwrap().value, Value::Null);
        assert_eq!(n.get("e").unwrap().value, Value::Null);
    }

    #[test]
    fn block_scalars() {
        let n = one("data:\n  script: |\n    echo one\n      two\n\n    three\n  after: yes\n");
        assert_eq!(n.at(&["data", "script"]).and_then(Node::str), Some("echo one\n  two\n\nthree"));
        assert_eq!(n.at(&["data", "after"]).and_then(Node::str), Some("yes"));
        let n = one("- name: x\n  shell: >-\n    a\n    b\n- name: y\n");
        assert_eq!(n.items().len(), 2);
    }

    #[test]
    fn flow_collections() {
        let n = one("args: [\"--port\", 80]\nsel: {app: web, tier: \"front\"}\nlong: [a,\n  b]\nnext: 1\n");
        assert_eq!(n.get("args").unwrap().items().len(), 2);
        assert_eq!(n.at(&["sel", "tier"]).and_then(Node::str), Some("front"));
        assert_eq!(n.at(&["sel", "app"]).unwrap().line, 2);
        assert_eq!(n.get("long").unwrap().items().len(), 2);
        assert_eq!(n.get("next").unwrap().line, 5);
    }

    #[test]
    fn documents() {
        let d = read("---\na: 1\n---\n# only a comment\n---\nb: 2\n...\n");
        assert_eq!(d.len(), 2);
        assert_eq!(d[1].root.get("b").unwrap().line, 6);
    }

    #[test]
    fn duplicate_keys() {
        let d = read("a: 1\nb:\n  c: 1\n  c: 2\na: 3\n");
        let dups: Vec<(&str, usize)> = d[0].duplicates.iter().map(|x| (x.key.as_str(), x.line)).collect();
        assert_eq!(dups, vec![("c", 4), ("a", 5)]);
        // The same key in two mappings of a list is fine.
        assert!(read("- name: a\n- name: b\n")[0].duplicates.is_empty());
    }

    #[test]
    fn anchors_and_tags() {
        let n = one("base: &b\n  x: 1\nother:\n  <<: *b\n  <<: *b\nn: !!str 5\n");
        assert_eq!(n.at(&["base", "x"]).and_then(Node::str), Some("1"));
        assert_eq!(n.get("n").and_then(Node::str), Some("5"));
    }

    #[test]
    fn keys_with_colons_and_urls() {
        let n = one("image: registry.local:5000/app:1.2\nurl: http://x/y\n\"a: b\": c\n");
        assert_eq!(n.get("image").and_then(Node::str), Some("registry.local:5000/app:1.2"));
        assert_eq!(n.get("url").and_then(Node::str), Some("http://x/y"));
        assert_eq!(n.get("a: b").and_then(Node::str), Some("c"));
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(syntax_error("a: 1\nb:\n  c: 2\n"), None);
        let e = syntax_error("a:\n  b: 1\n c: 2\n").expect("bad indentation");
        assert_eq!(e.line, 3, "{e:?}");
        let e = syntax_error("a: [1, 2\nb: 3\n").expect("unclosed");
        assert!(e.line >= 1, "{e:?}");
        assert!(syntax_error("key: \"open\n").is_some());
    }

    #[test]
    fn crlf() {
        let n = one("a: 1\r\nb:\r\n  - x\r\n");
        assert_eq!(n.get("a").and_then(Node::str), Some("1"));
        assert_eq!(n.get("b").unwrap().items()[0].str(), Some("x"));
    }
}
