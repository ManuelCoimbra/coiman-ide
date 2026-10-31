//! Print debugging: reads what `dbg!` printed, and takes the calls back out.

/// One value printed by `dbg!`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// As the compiler wrote it, usually relative to the workspace.
    pub file: String,
    /// Counted from 1.
    pub line: usize,
    /// Counted from 1; 0 if the output has none (old compilers).
    pub column: usize,
    /// Empty for a bare `dbg!()`.
    pub expr: String,
    /// `{:#?}` output, so it can span lines.
    pub value: String,
}

impl Entry {
    /// The value on a single line, cut to `max` characters.
    pub fn short_value(&self, max: usize) -> String {
        let joined = self.value.lines().map(str::trim).collect::<Vec<_>>().join(" ");
        if joined.chars().count() <= max {
            return joined;
        }
        let mut cut: String = joined.chars().take(max).collect();
        cut.push('…');
        cut
    }
}

/// Lines a multi-line value may take before it's cut off.
const VALUE_LIMIT: usize = 5000;

/// Finds `dbg!` output in lines of text, fed one at a time.
#[derive(Default)]
pub struct Parser {
    open: Option<Entry>,
    lines: usize,
    done: Vec<Entry>,
}

impl Parser {
    /// Whether the next line may continue a multi-line value.
    pub fn in_value(&self) -> bool {
        self.open.is_some()
    }

    pub fn feed(&mut self, line: &str) {
        if let Some(mut entry) = self.open.take() {
            let closes = line.starts_with(['}', ']', ')']);
            if closes || line.starts_with([' ', '\t']) {
                entry.value.push('\n');
                entry.value.push_str(line);
                self.lines += 1;
                if closes || self.lines >= VALUE_LIMIT {
                    self.done.push(entry);
                } else {
                    self.open = Some(entry);
                }
                return;
            }
            // Something else printed: the value ends here.
            self.done.push(entry);
        }
        let Some(entry) = parse_line(line) else { return };
        if entry.value.ends_with(['{', '[', '(']) {
            self.lines = 0;
            self.open = Some(entry);
        } else {
            self.done.push(entry);
        }
    }

    /// Everything found, in the order it was printed.
    pub fn finish(mut self) -> Vec<Entry> {
        self.done.extend(self.open.take());
        self.done
    }
}

/// `[src/main.rs:3:5] x = 1`, or `[src/main.rs:3:5]` for a bare `dbg!()`.
fn parse_line(line: &str) -> Option<Entry> {
    let rest = line.strip_prefix('[')?;
    let at = rest.find(".rs:")? + 3;
    let (file, rest) = rest.split_at(at);
    let (place, rest) = rest[1..].split_once(']')?;
    let (line_no, column) = match place.split_once(':') {
        Some((l, c)) => (l.parse().ok()?, c.parse().ok()?),
        None => (place.parse().ok()?, 0),
    };
    let (expr, value) = match rest.strip_prefix(' ') {
        Some(printed) => split_expression(printed)?,
        None if rest.is_empty() => ("", ""),
        None => return None,
    };
    Some(Entry { file: file.to_owned(), line: line_no, column, expr: expr.to_owned(), value: value.to_owned() })
}

/// Splits at the first ` = ` outside brackets and strings: the expression may
/// hold one, like `{ let a = 1; a }`.
fn split_expression(text: &str) -> Option<(&str, &str)> {
    let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
    for (i, c) in text.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ' ' if depth == 0 && text[i..].starts_with(" = ") => return Some((&text[..i], &text[i + 3..])),
            _ => {}
        }
    }
    None
}

/// What it takes to take the `dbg!` calls out of a text.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// Replacements in character offsets, not overlapping, in order.
    pub edits: Vec<(usize, usize, String)>,
    /// Calls left alone: they hold comments or aren't well formed.
    pub skipped: usize,
}

/// Plans removing `dbg!`, keeping what the calls evaluate: `dbg!(f(x))` becomes
/// `f(x)`. A statement that only prints a variable, `dbg!(x);`, goes entirely.
pub fn remove(text: &str) -> Removal {
    let mut removal = Removal::default();
    if text.contains("dbg") {
        let chars: Vec<char> = text.chars().collect();
        walk(&chars, 0, chars.len(), &mut removal);
    }
    removal
}

#[derive(PartialEq)]
enum Token {
    Space,
    Comment,
    Literal,
    Word,
    Other,
}

/// The token at `i`, and where it ends. Strings, characters and comments are
/// whole tokens, so what's inside them is never taken for code.
fn token(c: &[char], i: usize, end: usize) -> (Token, usize) {
    let at = |n: usize| if n < end { c[n] } else { '\0' };
    let first = c[i];
    if first.is_whitespace() {
        return (Token::Space, i + 1);
    }
    if first == '/' && at(i + 1) == '/' {
        let mut j = i;
        while j < end && c[j] != '\n' {
            j += 1;
        }
        return (Token::Comment, j);
    }
    if first == '/' && at(i + 1) == '*' {
        let (mut j, mut depth) = (i + 2, 1);
        while j < end && depth > 0 {
            if c[j] == '/' && at(j + 1) == '*' {
                depth += 1;
                j += 2;
            } else if c[j] == '*' && at(j + 1) == '/' {
                depth -= 1;
                j += 2;
            } else {
                j += 1;
            }
        }
        return (Token::Comment, j);
    }
    if first == '"' {
        return (Token::Literal, quoted(c, i, end, '"'));
    }
    if first == '\'' {
        if at(i + 1) == '\\' {
            return (Token::Literal, quoted(c, i, end, '\''));
        }
        if at(i + 2) == '\'' {
            return (Token::Literal, (i + 3).min(end));
        }
        // A lifetime.
        return (Token::Other, i + 1);
    }
    if first.is_alphanumeric() || first == '_' {
        let mut j = i;
        while j < end && (c[j].is_alphanumeric() || c[j] == '_') {
            j += 1;
        }
        let word: String = c[i..j].iter().collect();
        if matches!(word.as_str(), "b" | "c") && at(j) == '"' {
            return (Token::Literal, quoted(c, j, end, '"'));
        }
        if matches!(word.as_str(), "r" | "br" | "cr") {
            let mut k = j;
            while at(k) == '#' {
                k += 1;
            }
            if at(k) == '"' {
                let hashes = k - j;
                let mut e = k + 1;
                while e < end {
                    if c[e] == '"' && (1..=hashes).all(|h| at(e + h) == '#') {
                        return (Token::Literal, e + 1 + hashes);
                    }
                    e += 1;
                }
                return (Token::Literal, end);
            }
        }
        return (Token::Word, j);
    }
    (Token::Other, i + 1)
}

/// Where a string or character opened at `i` ends.
fn quoted(c: &[char], i: usize, end: usize, quote: char) -> usize {
    let mut j = i + 1;
    while j < end {
        match c[j] {
            '\\' => j += 2,
            q if q == quote => return j + 1,
            _ => j += 1,
        }
    }
    end
}

fn skip_space(c: &[char], mut i: usize, end: usize) -> usize {
    while i < end && c[i].is_whitespace() {
        i += 1;
    }
    i
}

/// If a `dbg!` call starts at `i`, the index of its opening bracket.
fn call_at(c: &[char], i: usize, end: usize) -> Option<usize> {
    let starts = |at: usize, s: &str| s.chars().enumerate().all(|(n, ch)| at + n < end && c[at + n] == ch);
    let mut at = i;
    if starts(at, "::std::") {
        at += 7;
    } else if starts(at, "std::") {
        at += 5;
    }
    if !starts(at, "dbg") {
        return None;
    }
    let after_name = at + 3;
    if after_name < end && (c[after_name].is_alphanumeric() || c[after_name] == '_') {
        return None;
    }
    let bang = skip_space(c, after_name, end);
    if bang >= end || c[bang] != '!' {
        return None;
    }
    let open = skip_space(c, bang + 1, end);
    (open < end && matches!(c[open], '(' | '[' | '{')).then_some(open)
}

struct Call {
    close: usize,
    /// Argument ranges, without the empty one a trailing comma leaves.
    args: Vec<(usize, usize)>,
}

/// Reads the call whose bracket opens at `open`. `None` if it can't be taken
/// out safely: it doesn't close, or a comment in it could swallow what follows.
fn read_call(c: &[char], open: usize, end: usize) -> Option<Call> {
    let mut depth = 0usize;
    let mut args = Vec::new();
    let mut start = open + 1;
    let mut i = open + 1;
    while i < end {
        let (kind, next) = token(c, i, end);
        if kind == Token::Comment {
            return None;
        }
        if kind == Token::Other {
            match c[i] {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => {
                    args.push((start, i));
                    args.retain(|&(a, b)| c[a..b].iter().any(|ch| !ch.is_whitespace()));
                    return Some(Call { close: i, args });
                }
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    args.push((start, i));
                    start = i + 1;
                }
                _ => {}
            }
        }
        i = next;
    }
    None
}

/// The text of `a..b` with any `dbg!` inside it taken out.
fn cleaned(c: &[char], a: usize, b: usize, removal: &mut Removal) -> String {
    let mut inner = Removal::default();
    walk(c, a, b, &mut inner);
    removal.skipped += inner.skipped;
    let mut out = String::new();
    let mut at = a;
    for (s, e, text) in inner.edits {
        out.extend(&c[at..s]);
        out.push_str(&text);
        at = e;
    }
    out.extend(&c[at..b]);
    out.trim().to_owned()
}

/// Reading it again has no effect, so a statement of only this does nothing.
fn only_reads(arg: &str) -> bool {
    arg.chars().all(|ch| ch.is_alphanumeric() || ch.is_whitespace() || matches!(ch, '_' | '.' | '&' | ':'))
}

fn walk(c: &[char], start: usize, end: usize, removal: &mut Removal) {
    let mut i = start;
    // The last character of code seen: tells a statement from part of an expression.
    let mut prev: Option<char> = None;
    while i < end {
        let (kind, next) = token(c, i, end);
        match kind {
            Token::Space | Token::Comment => {
                i = next;
                continue;
            }
            Token::Word | Token::Other => {
                if let Some(open) =
                    (kind == Token::Word || c[i] == ':' && prev != Some(':')).then(|| call_at(c, i, end)).flatten()
                {
                    let Some(call) = read_call(c, open, end) else {
                        removal.skipped += 1;
                        i = open + 1;
                        prev = Some(c[open]);
                        continue;
                    };
                    let args: Vec<String> = call.args.iter().map(|&(a, b)| cleaned(c, a, b, removal)).collect();
                    let after = skip_space(c, call.close + 1, end);
                    let semicolon = after < end && c[after] == ';';
                    let statement = semicolon && matches!(prev, None | Some(';' | '{' | '}'));
                    if statement && args.iter().all(|a| only_reads(a)) {
                        let (from, to) = whole_statement(c, i, after + 1, end);
                        removal.edits.push((from, to, String::new()));
                        i = to;
                        prev = Some(';');
                        continue;
                    }
                    let text = match args.as_slice() {
                        [] => "()".to_owned(),
                        [one] => one.clone(),
                        many => format!("({})", many.join(", ")),
                    };
                    removal.edits.push((i, call.close + 1, text));
                    i = call.close + 1;
                    prev = Some(')');
                    continue;
                }
            }
            Token::Literal => {}
        }
        prev = Some(c[next - 1]);
        i = next;
    }
}

/// The span to delete for a statement: its whole line if nothing else is on it.
fn whole_statement(c: &[char], from: usize, to: usize, end: usize) -> (usize, usize) {
    let mut line_start = from;
    while line_start > 0 && matches!(c[line_start - 1], ' ' | '\t') {
        line_start -= 1;
    }
    let mut line_end = to;
    while line_end < end && matches!(c[line_end], ' ' | '\t') {
        line_end += 1;
    }
    if line_end < end && c[line_end] == '\r' {
        line_end += 1;
    }
    let starts_line = line_start == 0 || c[line_start - 1] == '\n';
    if starts_line && line_end < end && c[line_end] == '\n' {
        (line_start, line_end + 1)
    } else if starts_line && line_end == end {
        (line_start, end)
    } else {
        (from, to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<Entry> {
        let mut parser = Parser::default();
        for line in text.lines() {
            parser.feed(line);
        }
        parser.finish()
    }

    fn removed(text: &str) -> String {
        let removal = remove(text);
        let chars: Vec<char> = text.chars().collect();
        let (mut out, mut at) = (String::new(), 0);
        for (s, e, t) in removal.edits {
            out.extend(&chars[at..s]);
            out.push_str(&t);
            at = e;
        }
        out.extend(&chars[at..]);
        out
    }

    #[test]
    fn reads_a_value() {
        let found = parse("hello\n[src\\main.rs:3:5] x + 1 = 42\nbye");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file, "src\\main.rs");
        assert_eq!((found[0].line, found[0].column), (3, 5));
        assert_eq!((found[0].expr.as_str(), found[0].value.as_str()), ("x + 1", "42"));
    }

    #[test]
    fn reads_a_multi_line_value_and_what_follows() {
        let found = parse("[a.rs:1:1] v = [\n    1,\n    Foo {\n        a: 2,\n    },\n]\n[a.rs:2:1] y = 1");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].value, "[\n    1,\n    Foo {\n        a: 2,\n    },\n]");
        assert_eq!(found[0].short_value(100), "[ 1, Foo { a: 2, }, ]");
        assert_eq!(found[0].short_value(3), "[ 1…");
        assert_eq!(found[1].value, "1");
    }

    #[test]
    fn output_interrupting_a_value_ends_it() {
        let found = parse("[a.rs:1:1] v = [\n    1,\nother\n[a.rs:2:1] y = 1");
        assert_eq!(found.iter().map(|e| e.line).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(found[0].value, "[\n    1,");
    }

    #[test]
    fn reads_forms_without_expression_or_column() {
        assert_eq!(parse("[a.rs:7:2]")[0].expr, "");
        assert_eq!(parse("[a.rs:7] x = 1")[0].column, 0);
        let spaced = parse("[C:\\my code\\a.rs:7:2] { let a = 1; a } = \"p = q\"");
        assert_eq!(spaced[0].file, "C:\\my code\\a.rs");
        assert_eq!(spaced[0].expr, "{ let a = 1; a }");
        assert_eq!(spaced[0].value, "\"p = q\"");
    }

    #[test]
    fn ignores_what_only_looks_like_it() {
        for line in
            ["[INFO] x = 1", "[a.txt:1:1] x = 1", "[a.rs:x:1] y = 1", "[a.rs:1:1]x", " [a.rs:1:1] x = 1", "[a.rs:"]
        {
            assert!(parse(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn keeps_what_a_call_evaluates() {
        assert_eq!(removed("let y = dbg!(f(x)) + 1;"), "let y = f(x) + 1;");
        assert_eq!(removed("foo(dbg!(a, b), std::dbg!(c));"), "foo((a, b), c);");
        assert_eq!(removed("let y = dbg!(dbg!(x) + 1);"), "let y = x + 1;");
        assert_eq!(removed("let y = dbg![x,];"), "let y = x;");
        assert_eq!(removed("let u = dbg!();"), "let u = ();");
        assert_eq!(removed("fn f() -> i32 { dbg!(1) }"), "fn f() -> i32 { 1 }");
    }

    #[test]
    fn removes_statements_that_only_print() {
        assert_eq!(removed("{\n    dbg!(x);\n    y();\n}\n"), "{\n    y();\n}\n");
        assert_eq!(removed("{\r\n    dbg!(&a.b);\r\n}\r\n"), "{\r\n}\r\n");
        assert_eq!(removed("{ dbg!(x); y(); }"), "{  y(); }");
        assert_eq!(removed("a();\n    dbg!();\nb();"), "a();\nb();");
    }

    #[test]
    fn keeps_side_effects_and_expressions_across_lines() {
        assert_eq!(removed("{\n    dbg!(f());\n}"), "{\n    f();\n}");
        assert_eq!(removed("let y =\n    dbg!(x);\n"), "let y =\n    x;\n");
        assert_eq!(removed("match a { _ => dbg!(x), }"), "match a { _ => x, }");
    }

    #[test]
    fn leaves_comments_strings_and_lookalikes() {
        for text in [
            "// dbg!(x);\n",
            "/* dbg!(x); */",
            "let s = \"dbg!(x)\";",
            "let s = r#\"dbg!(\"x\")\"#;",
            "let c = '\"'; // dbg!(x)",
            "mydbg!(x);",
            "dbg_x!(y);",
            "let dbg = 1;",
        ] {
            assert_eq!(removed(text), text);
        }
        assert_eq!(removed("let c = ')'; let s = dbg!(')');"), "let c = ')'; let s = ')';");
        assert_eq!(removed("fn f<'a>(x: &'a u8) { g(dbg!(x)); }"), "fn f<'a>(x: &'a u8) { g(x); }");
    }

    #[test]
    fn skips_calls_it_cant_take_out_safely() {
        let with_comment = "let y = dbg!(x // why\n);";
        let plan = remove(with_comment);
        assert_eq!((plan.edits.len(), plan.skipped), (0, 1));
        let open = remove("let y = dbg!(x;");
        assert_eq!((open.edits.len(), open.skipped), (0, 1));
    }

    #[test]
    fn edits_count_characters() {
        let plan = remove("let é = dbg!(1);");
        assert_eq!(plan.edits, vec![(8, 15, "1".to_owned())]);
    }
}
