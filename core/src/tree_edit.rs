//! Moving code by its structure rather than its characters. Lisps get the whole set
//! (slurp, barf, raise, splice, wrap), read by a small reader of their own since they
//! have no grammar here. Other languages raise and wrap through their syntax tree.

use ropey::Rope;
use tree_sitter::{Node, Tree};

/// What to change, in the current text's char offsets.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub edits: Vec<(usize, usize, String)>,
    /// Where the cursor goes, in the new text. Otherwise it moves with what's around it.
    pub cursor: Option<usize>,
}

pub fn is_lisp(extension: &str) -> bool {
    matches!(extension, "clj" | "cljs" | "cljc" | "edn" | "bb" | "lisp" | "cl" | "el" | "scm" | "ss" | "rkt" | "fnl")
}

#[derive(Debug)]
struct Form {
    start: usize,
    end: usize,
    /// For a list: its opening, with any prefix such as `#` or `'`, ends here...
    open_end: usize,
    /// ...and its closing starts here. An atom has neither.
    close_start: usize,
    list: bool,
    children: Vec<Form>,
}

const TOO_DEEP: usize = 512;

struct Reader<'a> {
    chars: &'a [char],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn skip_blank(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == ',' {
                self.at += 1;
            } else if c == ';' {
                while self.peek().is_some_and(|c| c != '\n') {
                    self.at += 1;
                }
            } else if c == '#' && self.chars.get(self.at + 1) == Some(&'|') {
                self.at += 2;
                let mut depth = 1;
                while depth > 0 {
                    let Some(c) = self.peek() else { return };
                    match (c, self.chars.get(self.at + 1)) {
                        ('|', Some('#')) => (depth, self.at) = (depth - 1, self.at + 2),
                        ('#', Some('|')) => (depth, self.at) = (depth + 1, self.at + 2),
                        _ => self.at += 1,
                    }
                }
            } else {
                return;
            }
        }
    }

    /// Reads forms up to `closer`, or the end of the text if there's none to expect.
    fn forms(&mut self, closer: Option<char>, depth: usize) -> Option<Vec<Form>> {
        if depth > TOO_DEEP {
            return None;
        }
        let mut forms = Vec::new();
        loop {
            self.skip_blank();
            match self.peek() {
                None => return closer.is_none().then_some(forms),
                Some(c) if Some(c) == closer => return Some(forms),
                Some(')' | ']' | '}') => return None,
                Some(_) => forms.push(self.form(depth)?),
            }
        }
    }

    fn form(&mut self, depth: usize) -> Option<Form> {
        let start = self.at;
        while self.peek().is_some_and(|c| matches!(c, '\'' | '`' | '~' | '@' | '^' | '#')) {
            self.at += 1;
        }
        match self.peek() {
            Some(open @ ('(' | '[' | '{')) => {
                self.at += 1;
                let open_end = self.at;
                let closer = match open {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                let children = self.forms(Some(closer), depth + 1)?;
                let close_start = self.at;
                self.at += 1;
                Some(Form { start, end: self.at, open_end, close_start, list: true, children })
            }
            Some('"') => {
                self.at += 1;
                loop {
                    match self.peek()? {
                        '\\' => self.at += 2,
                        '"' => break,
                        _ => self.at += 1,
                    }
                }
                self.at += 1;
                Some(self.atom(start))
            }
            _ => {
                while let Some(c) = self.peek() {
                    match c {
                        '\\' => self.at += 2,
                        c if c.is_whitespace() || matches!(c, ',' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | ';') => {
                            break;
                        }
                        _ => self.at += 1,
                    }
                }
                self.at = self.at.min(self.chars.len());
                Some(self.atom(start))
            }
        }
    }

    fn atom(&self, start: usize) -> Form {
        Form { start, end: self.at, open_end: start, close_start: self.at, list: false, children: Vec::new() }
    }
}

fn read(chars: &[char]) -> Result<Form, String> {
    let mut reader = Reader { chars, at: 0 };
    let children = reader.forms(None, 0).ok_or("The brackets here don't balance")?;
    let end = chars.len();
    Ok(Form { start: 0, end, open_end: 0, close_start: end, list: true, children })
}

/// The lists around `pos`, outermost (the whole file) first.
fn lists_around(root: &Form, pos: usize) -> Vec<&Form> {
    let mut chain = vec![root];
    loop {
        let inner = chain[chain.len() - 1];
        match inner.children.iter().find(|c| c.list && c.open_end <= pos && pos <= c.close_start) {
            Some(child) => chain.push(child),
            None => return chain,
        }
    }
}

/// The form at `pos` in a list, else the next one, else the last.
fn form_at(list: &Form, pos: usize) -> Option<&Form> {
    let children = &list.children;
    let starting = children.iter().find(|c| c.start == pos);
    starting
        .or_else(|| children.iter().find(|c| c.start <= pos && pos <= c.end))
        .or_else(|| children.iter().find(|c| c.start > pos))
        .or_else(|| children.last())
}

fn text(chars: &[char], from: usize, to: usize) -> String {
    chars[from..to].iter().collect()
}

/// Which way a form moves in or out of a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    SlurpForward,
    SlurpBackward,
    BarfForward,
    BarfBackward,
    Raise,
    Splice,
}

/// Works out a move in Lisp text, for a cursor at `pos`.
pub fn lisp_move(source: &str, pos: usize, what: Move) -> Result<Change, String> {
    let chars: Vec<char> = source.chars().collect();
    let root = read(&chars)?;
    let chain = lists_around(&root, pos);
    let inner = chain[chain.len() - 1];
    let in_a_list = chain.len() > 1;
    let edits = match what {
        Move::SlurpForward => {
            // The nearest list that has something after it.
            let found = (1..chain.len()).rev().find_map(|at| {
                let next = chain[at - 1].children.iter().find(|c| c.start >= chain[at].end)?;
                Some((chain[at], next))
            });
            let (list, next) = found.ok_or("Nothing after this list to bring in")?;
            let close = text(&chars, list.close_start, list.end);
            let until = if list.children.is_empty() { next.start } else { list.end };
            vec![(list.close_start, until, String::new()), (next.end, next.end, close)]
        }
        Move::SlurpBackward => {
            let found = (1..chain.len()).rev().find_map(|at| {
                let before = chain[at - 1].children.iter().rfind(|c| c.end <= chain[at].start)?;
                Some((chain[at], before))
            });
            let (list, before) = found.ok_or("Nothing before this list to bring in")?;
            let open = text(&chars, list.start, list.open_end);
            let mut edits = vec![(before.start, before.start, open), (list.start, list.open_end, String::new())];
            if list.children.is_empty() {
                edits.push((list.open_end, list.close_start, String::new()));
            }
            edits
        }
        Move::BarfForward => {
            let list = in_a_list.then_some(inner).filter(|l| !l.children.is_empty());
            let list = list.ok_or("Nothing in a list to move out")?;
            let close = text(&chars, list.close_start, list.end);
            let last = &list.children[list.children.len() - 1];
            let mut edits = vec![(list.close_start, list.end, String::new())];
            match list.children.len().checked_sub(2).map(|at| &list.children[at]) {
                Some(before) => edits.push((before.end, before.end, close)),
                None => edits.push((list.open_end, last.start, format!("{close} "))),
            }
            edits
        }
        Move::BarfBackward => {
            let list = in_a_list.then_some(inner).filter(|l| !l.children.is_empty());
            let list = list.ok_or("Nothing in a list to move out")?;
            let open = text(&chars, list.start, list.open_end);
            let mut edits = vec![(list.start, list.open_end, String::new())];
            match list.children.get(1) {
                Some(next) => edits.push((next.start, next.start, open)),
                None => edits.push((list.close_start, list.close_start, format!(" {open}"))),
            }
            edits
        }
        Move::Raise => {
            let form = in_a_list.then(|| form_at(inner, pos)).flatten().ok_or("No form in a list to raise")?;
            let edits = vec![(inner.start, inner.end, text(&chars, form.start, form.end))];
            let cursor = inner.start + pos.clamp(form.start, form.end) - form.start;
            return Ok(Change { edits, cursor: Some(cursor) });
        }
        Move::Splice => {
            if !in_a_list {
                return Err("Not inside a list".to_owned());
            }
            vec![(inner.start, inner.open_end, String::new()), (inner.close_start, inner.end, String::new())]
        }
    };
    Ok(Change { edits, cursor: None })
}

/// Wraps a selection as it is, or the Lisp form at the cursor.
pub fn lisp_wrap(source: &str, start: usize, end: usize, open: &str, close: &str) -> Result<Change, String> {
    if start != end {
        return Ok(wrap_range(start, end, open, close));
    }
    let chars: Vec<char> = source.chars().collect();
    let root = read(&chars)?;
    let chain = lists_around(&root, start);
    Ok(match form_at(chain[chain.len() - 1], start) {
        Some(form) => wrap_range(form.start, form.end, open, close),
        None => wrap_range(start, start, open, close),
    })
}

/// Puts brackets around a range, leaving the cursor just inside the opening.
pub fn wrap_range(start: usize, end: usize, open: &str, close: &str) -> Change {
    let cursor = start + open.chars().count();
    let edits = vec![(start, start, open.to_owned()), (end, end, close.to_owned())];
    Change { edits, cursor: Some(cursor) }
}

/// Whether a node only holds others, such as an argument list.
fn holder(kind: &str) -> bool {
    kind.contains("argument") || kind.contains("parenthes") || kind.contains("list")
}

fn named_at<'a>(tree: &'a Tree, rope: &Rope, start: usize, end: usize) -> Option<Node<'a>> {
    let word = |at: usize| rope.get_char(at).is_some_and(|c| c.is_alphanumeric() || c == '_');
    // A cursor just after a word means that word, not what follows it.
    let (start, end) = match start == end && !word(start) && start > 0 && word(start - 1) {
        true => (start - 1, start),
        false => (start, end),
    };
    let (a, b) = (rope.char_to_byte(start), rope.char_to_byte(end));
    let mut node = tree.root_node().named_descendant_for_byte_range(a, b)?;
    while holder(node.kind()) {
        node = node.parent()?;
    }
    Some(node)
}

fn char_range(rope: &Rope, node: Node) -> (usize, usize) {
    (rope.byte_to_char(node.start_byte()), rope.byte_to_char(node.end_byte()))
}

/// What a selection, or the cursor, covers in a syntax tree.
pub fn node_range(tree: &Tree, rope: &Rope, start: usize, end: usize) -> Option<(usize, usize)> {
    named_at(tree, rope, start, end).map(|node| char_range(rope, node))
}

/// The kind of syntax node at a place, to start writing a query from.
pub fn node_kind(tree: &Tree, rope: &Rope, start: usize, end: usize) -> Option<String> {
    named_at(tree, rope, start, end).map(|node| node.kind().to_owned())
}

/// Grammars name what can be replaced by its part in much the same words. Blocks,
/// argument lists and statements don't count: a node raised over them wouldn't fit.
fn replaceable(kind: &str) -> bool {
    (kind.contains("expression") || kind.contains("call") || kind.contains("invocation")) && !kind.contains("statement")
}

/// Replaces the expression around a node with that node.
pub fn tree_raise(tree: &Tree, rope: &Rope, start: usize, end: usize) -> Result<Change, String> {
    let node = named_at(tree, rope, start, end).ok_or("Nothing here to raise")?;
    let mut around = node.parent();
    while let Some(parent) = around.filter(|p| !replaceable(p.kind())) {
        // Only through what merely holds it, such as an argument list.
        if !holder(parent.kind()) {
            around = None;
            break;
        }
        around = parent.parent();
    }
    let around = around.ok_or("This isn't inside an expression it could replace")?;
    let (a, b) = char_range(rope, around);
    let (from, to) = char_range(rope, node);
    let cursor = a + start.clamp(from, to) - from;
    Ok(Change { edits: vec![(a, b, rope.slice(from..to).to_string())], cursor: Some(cursor) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a move on text with `|` marking the cursor.
    fn moved(marked: &str, what: Move) -> String {
        let pos = marked.chars().position(|c| c == '|').unwrap();
        let source = marked.replacen('|', "", 1);
        apply(&source, lisp_move(&source, pos, what).unwrap())
    }

    fn apply(source: &str, change: Change) -> String {
        let mut chars: Vec<char> = source.chars().collect();
        let mut edits = change.edits;
        edits.sort_by_key(|e| e.0);
        for (a, b, new) in edits.into_iter().rev() {
            chars.splice(a..b, new.chars());
        }
        chars.into_iter().collect()
    }

    #[test]
    fn slurps() {
        assert_eq!(moved("(a |b) c d", Move::SlurpForward), "(a b c) d");
        assert_eq!(moved("a b (|c)", Move::SlurpBackward), "a (b c)");
        assert_eq!(moved("(|) c", Move::SlurpForward), "(c)");
        assert_eq!(moved("a (|)", Move::SlurpBackward), "(a )");
        assert_eq!(moved("[a (b|)] c", Move::SlurpForward), "[a (b)  c]".replace("  ", " "));
        assert_eq!(moved("#{a|} (b c)", Move::SlurpForward), "#{a (b c)}");
        assert_eq!(moved("x '(a|)", Move::SlurpBackward), "'(x a)");
    }

    #[test]
    fn barfs() {
        assert_eq!(moved("(a |b c) d", Move::BarfForward), "(a b) c d");
        assert_eq!(moved("(a |b c)", Move::BarfBackward), "a (b c)");
        assert_eq!(moved("(|a) d", Move::BarfForward), "() a d");
        assert_eq!(moved("(|a)", Move::BarfBackward), "a ()");
    }

    #[test]
    fn raises_and_splices() {
        assert_eq!(moved("(a (b |c) d)", Move::Raise), "(a c d)");
        assert_eq!(moved("(a |(b c) d)", Move::Raise), "(b c)");
        assert_eq!(moved("(a (b |c) d)", Move::Splice), "(a b c d)");
        let change = lisp_move("(a (b c))", 7, Move::Raise).unwrap();
        assert_eq!(change.cursor, Some(4));
    }

    #[test]
    fn reads_what_isnt_code_as_it_should() {
        assert_eq!(moved("(a \") ;\" |b) c", Move::SlurpForward), "(a \") ;\" b c)");
        assert_eq!(moved("(a ; )\n |b) c", Move::SlurpForward), "(a ; )\n b c)");
        assert_eq!(moved("(a \\) |b) c", Move::SlurpForward), "(a \\) b c)");
        let source = "(a #| ) |# b) c";
        assert_eq!(apply(source, lisp_move(source, 11, Move::SlurpForward).unwrap()), "(a #| ) |# b c)");
        assert_eq!(moved("(é |ü) ñ", Move::SlurpForward), "(é ü ñ)");
    }

    #[test]
    fn says_why_not() {
        assert!(lisp_move("(a", 1, Move::Raise).unwrap_err().contains("balance"));
        assert!(lisp_move("a)", 1, Move::Raise).unwrap_err().contains("balance"));
        assert!(lisp_move("(a]", 1, Move::Raise).is_err());
        assert!(lisp_move("\"open", 1, Move::Raise).is_err());
        assert!(lisp_move("(a) b", 4, Move::Raise).is_err(), "not in a list");
        assert!(lisp_move("(a|)", 1, Move::SlurpForward).is_err(), "nothing after");
        assert!(lisp_move(&"(".repeat(100_000), 0, Move::Raise).is_err(), "too deep isn't a crash");
    }

    #[test]
    fn wraps() {
        let wrapped = |s: &str, a, b| apply(s, lisp_wrap(s, a, b, "(", ")").unwrap());
        assert_eq!(wrapped("a bc d", 2, 2), "a (bc) d");
        assert_eq!(wrapped("a bc d", 0, 4), "(a bc) d");
        assert_eq!(wrapped("(f) x", 0, 0), "((f)) x");
        assert_eq!(wrapped("", 0, 0), "()");
    }

    fn rust_tree(source: &str) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn raises_through_a_tree() {
        let source = "fn m() { let x = f(g(1)); }";
        let (tree, rope) = (rust_tree(source), Rope::from_str(source));
        let at = source.find("g(1)").unwrap();
        let change = tree_raise(&tree, &rope, at, at + 4).unwrap();
        assert_eq!(apply(source, change), "fn m() { let x = g(1); }");
        // A cursor against a word takes the word, never the arguments after it.
        let at = source.find("g(1)").unwrap() + 1;
        assert_eq!(apply(source, tree_raise(&tree, &rope, at, at).unwrap()), "fn m() { let x = f(g); }");
        let at = at + 1;
        assert_eq!(apply(source, tree_raise(&tree, &rope, at, at).unwrap()), "fn m() { let x = f(1); }");
        // An expression inside a block isn't raised over its function.
        let at = source.find("let").unwrap();
        assert!(tree_raise(&tree, &rope, at, at + 3).is_err());
    }
}
