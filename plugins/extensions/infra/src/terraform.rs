//! Terraform's `.tf` files, read offline: the blocks a folder (a module) declares, the
//! references its expressions make (`var.x`, `local.x`, `module.x`, `data.t.n`, `t.n`), blocks
//! and strings that are never closed, references to what isn't declared, and variables nothing
//! uses.

use std::collections::BTreeSet;

use crate::{Finding, Level};

/// A top-level block: `resource "aws_instance" "web" {`.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub file: String,
    /// `resource`, `data`, `variable`, `output`, `module`, `locals`, `provider`, `terraform`.
    pub kind: String,
    pub labels: Vec<String>,
    pub line: usize,
    /// Its closing line.
    pub end: usize,
    /// For `locals`, the names it sets; for `module`, its `source`.
    pub names: Vec<(String, usize)>,
    pub source: Option<String>,
}

impl Block {
    /// The address others use for it: `aws_instance.web`, `data.t.n`, `var.x`, `module.m`.
    pub fn address(&self) -> Option<String> {
        let l = &self.labels;
        match (self.kind.as_str(), l.len()) {
            ("resource", 2) => Some(format!("{}.{}", l[0], l[1])),
            ("data", 2) => Some(format!("data.{}.{}", l[0], l[1])),
            ("variable", 1) => Some(format!("var.{}", l[0])),
            ("module", 1) => Some(format!("module.{}", l[0])),
            ("output", 1) => Some(format!("output.{}", l[0])),
            _ => None,
        }
    }
}

/// A reference in an expression, with where it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Ref {
    /// The address it names: `var.x`, `aws_instance.web`, `data.t.n`, `module.m`, `local.x`.
    pub address: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub len: usize,
    /// The address of the top-level block it is in, if that has one.
    pub within: Option<String>,
}

/// What reading one file found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct File {
    pub blocks: Vec<Block>,
    pub refs: Vec<Ref>,
    pub problems: Vec<Finding>,
    /// Names bound by `for` expressions, `dynamic` blocks and `iterator`: not references.
    pub bound: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Open(char),
    Close(char),
    Eq,
    Dot,
    Other,
    Newline,
}

#[derive(Clone, Debug)]
struct T {
    tok: Tok,
    line: usize,
    col: usize,
}

const KEYWORDS: &[&str] = &[
    "var",
    "local",
    "module",
    "data",
    "count",
    "each",
    "self",
    "path",
    "terraform",
    "true",
    "false",
    "null",
    "for",
    "in",
    "if",
    "else",
    "endif",
    "endfor",
];

/// Splits a file into tokens; the inside of `${ }` and `%{ }` in strings counts as code.
fn tokens(file: &str, text: &str, problems: &mut Vec<Finding>) -> Vec<T> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1usize, 0usize);
    // Open string interpolations: the brace depth at which each began.
    let mut interp: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let err = |problems: &mut Vec<Finding>, line: usize, col: usize, message: &str| {
        problems.push(Finding {
            path: file.to_owned(),
            line,
            start: col,
            end: col + 1,
            level: Level::Error,
            message: message.to_owned(),
        });
    };
    // Reads a string body from `i` (after the quote) up to its closing quote or a `${`.
    // Returns true if it stopped at `${`.
    macro_rules! advance {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }
    let mut in_string: Option<(usize, usize)> = None;
    while i < chars.len() {
        if let Some((sl, sc)) = in_string {
            // Inside a quoted string.
            let c = chars[i];
            if c == '\\' {
                advance!();
                if i < chars.len() {
                    advance!();
                }
                continue;
            }
            if c == '"' {
                advance!();
                in_string = None;
                continue;
            }
            if (c == '$' || c == '%') && chars.get(i + 1) == Some(&c) && chars.get(i + 2) == Some(&'{') {
                // `$${` is a literal.
                advance!();
                advance!();
                advance!();
                continue;
            }
            if (c == '$' || c == '%') && chars.get(i + 1) == Some(&'{') {
                advance!();
                advance!();
                depth += 1;
                interp.push(depth);
                in_string = None;
                // Remember to return to the string when this brace closes.
                out.push(T { tok: Tok::Other, line, col });
                continue;
            }
            if c == '\n' {
                err(problems, sl, sc, "This string is not closed on its line");
                in_string = None;
            }
            advance!();
            continue;
        }
        let c = chars[i];
        let (tl, tc) = (line, col);
        match c {
            '\n' => {
                out.push(T { tok: Tok::Newline, line, col });
                advance!();
            }
            ' ' | '\t' | '\r' => advance!(),
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    advance!();
                }
            }
            '/' if chars.get(i + 1) == Some(&'/') => {
                while i < chars.len() && chars[i] != '\n' {
                    advance!();
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                advance!();
                advance!();
                let mut closed = false;
                while i < chars.len() {
                    if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        advance!();
                        advance!();
                        closed = true;
                        break;
                    }
                    advance!();
                }
                if !closed {
                    err(problems, tl, tc, "This comment is not closed: */ is missing");
                }
            }
            '"' => {
                advance!();
                // A whole simple string becomes one token, for block labels.
                let mut j = i;
                let mut s = String::new();
                let mut simple = true;
                while j < chars.len() && chars[j] != '"' && chars[j] != '\n' {
                    if chars[j] == '\\' || ((chars[j] == '$' || chars[j] == '%') && chars.get(j + 1) == Some(&'{')) {
                        simple = false;
                        break;
                    }
                    s.push(chars[j]);
                    j += 1;
                }
                if simple && chars.get(j) == Some(&'"') {
                    while i <= j {
                        advance!();
                    }
                    out.push(T { tok: Tok::Str(s), line: tl, col: tc });
                } else {
                    out.push(T { tok: Tok::Str(String::new()), line: tl, col: tc });
                    in_string = Some((tl, tc));
                }
            }
            '<' if chars.get(i + 1) == Some(&'<') => {
                // A heredoc: <<EOF or <<-EOF up to a line that is the marker.
                let mut j = i + 2;
                if chars.get(j) == Some(&'-') {
                    j += 1;
                }
                let mark: String = chars[j..].iter().take_while(|c| c.is_alphanumeric() || **c == '_').collect();
                if mark.is_empty() {
                    advance!();
                    continue;
                }
                while i < chars.len() && chars[i] != '\n' {
                    advance!();
                }
                let mut closed = false;
                while i < chars.len() {
                    // At the end of a line: on to the next.
                    advance!();
                    let rest: String = chars[i..].iter().take_while(|c| **c != '\n').collect();
                    if rest.trim() == mark {
                        while i < chars.len() && chars[i] != '\n' {
                            advance!();
                        }
                        closed = true;
                        break;
                    }
                    // `${ }` in a heredoc: its references count.
                    for (k, _) in rest.match_indices("${") {
                        let inner: String = rest[k + 2..].chars().take_while(|c| *c != '}').collect();
                        let mut sub = tokens(file, &inner, &mut Vec::new());
                        for t in &mut sub {
                            t.line = line;
                            t.col += col + k + 2;
                        }
                        out.extend(sub.into_iter().filter(|t| t.tok != Tok::Newline));
                    }
                    while i < chars.len() && chars[i] != '\n' {
                        advance!();
                    }
                }
                if !closed {
                    err(problems, tl, tc, &format!("This heredoc is not closed: no line {mark} after it"));
                }
                out.push(T { tok: Tok::Str(String::new()), line: tl, col: tc });
            }
            '{' | '[' | '(' => {
                if c == '{' {
                    depth += 1;
                }
                out.push(T { tok: Tok::Open(c), line, col });
                advance!();
            }
            '}' | ']' | ')' => {
                if c == '}' {
                    if interp.last() == Some(&depth) {
                        // The end of an interpolation: back into its string.
                        interp.pop();
                        depth -= 1;
                        advance!();
                        in_string = Some((tl, tc));
                        out.push(T { tok: Tok::Other, line: tl, col: tc });
                        continue;
                    }
                    depth = depth.saturating_sub(1);
                }
                out.push(T { tok: Tok::Close(c), line, col });
                advance!();
            }
            '=' if chars.get(i + 1) != Some(&'=') => {
                out.push(T { tok: Tok::Eq, line, col });
                advance!();
            }
            '.' => {
                out.push(T { tok: Tok::Dot, line, col });
                advance!();
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut s = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '-') {
                    s.push(chars[i]);
                    advance!();
                }
                out.push(T { tok: Tok::Ident(s), line: tl, col: tc });
            }
            _ => {
                out.push(T { tok: Tok::Other, line, col });
                advance!();
            }
        }
    }
    if let Some((sl, sc)) = in_string {
        err(problems, sl, sc, "This string is not closed");
    }
    out
}

/// Reads one `.tf` file.
pub fn read(file: &str, text: &str) -> File {
    let mut f = File::default();
    let toks = tokens(file, text, &mut f.problems);
    // Brackets: each close matches its open; one left open is reported where it opened.
    let mut stack: Vec<(char, usize, usize)> = Vec::new();
    // The top-level block being read: its index in f.blocks.
    let mut current: Option<usize> = None;
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match &t.tok {
            Tok::Open(c) => {
                if *c == '{' && stack.is_empty() {
                    // A top-level block: an identifier and labels before it, on its line.
                    let mut j = i;
                    let mut labels = Vec::new();
                    let mut kind = None;
                    while j > 0 {
                        j -= 1;
                        match &toks[j].tok {
                            Tok::Str(s) => labels.insert(0, s.clone()),
                            Tok::Ident(s) => {
                                labels.insert(0, s.clone());
                                if j == 0 || matches!(toks[j - 1].tok, Tok::Newline | Tok::Close('}')) {
                                    kind = Some((labels.remove(0), toks[j].line));
                                    break;
                                }
                            }
                            _ => break,
                        }
                    }
                    if let Some((kind, line)) = kind {
                        f.blocks.push(Block {
                            file: file.to_owned(),
                            kind,
                            labels,
                            line,
                            end: line,
                            names: Vec::new(),
                            source: None,
                        });
                        current = Some(f.blocks.len() - 1);
                    }
                }
                stack.push((*c, t.line, t.col));
            }
            Tok::Close(c) => {
                let want = match c {
                    '}' => '{',
                    ']' => '[',
                    _ => '(',
                };
                match stack.last() {
                    Some((o, ..)) if *o == want => {
                        stack.pop();
                    }
                    Some((o, l, cc)) => {
                        let (o, l, cc) = (*o, *l, *cc);
                        f.problems.push(Finding {
                            path: file.to_owned(),
                            line: t.line,
                            start: t.col,
                            end: t.col + 1,
                            level: Level::Error,
                            message: format!("{c} closes the {o} opened on line {l}"),
                        });
                        let _ = cc;
                        stack.pop();
                    }
                    None => f.problems.push(Finding {
                        path: file.to_owned(),
                        line: t.line,
                        start: t.col,
                        end: t.col + 1,
                        level: Level::Error,
                        message: format!("{c} closes nothing: there's no open block here"),
                    }),
                }
                if stack.is_empty()
                    && let Some(b) = current.take()
                {
                    f.blocks[b].end = t.line;
                }
            }
            Tok::Ident(name) => {
                // An attribute at the first level of `locals`, and `source` of a module.
                let next_eq = matches!(toks.get(i + 1).map(|t| &t.tok), Some(Tok::Eq));
                let line_start = i == 0 || matches!(toks[i - 1].tok, Tok::Newline | Tok::Open('{'));
                if let Some(idx) = current
                    && stack.len() == 1
                    && next_eq
                    && line_start
                {
                    let kind = f.blocks[idx].kind.clone();
                    if kind == "locals" {
                        f.blocks[idx].names.push((name.clone(), t.line));
                    }
                    if kind == "module"
                        && name == "source"
                        && let Some(Tok::Str(s)) = toks.get(i + 2).map(|t| &t.tok)
                    {
                        f.blocks[idx].source = Some(s.clone());
                    }
                }
                // Names bound here: `for k, v in`, `dynamic "x"`, `iterator = x`.
                if name == "for" {
                    let mut j = i + 1;
                    while let Some(T { tok, .. }) = toks.get(j) {
                        match tok {
                            Tok::Ident(n) if n == "in" => break,
                            Tok::Ident(n) => {
                                f.bound.insert(n.clone());
                            }
                            Tok::Other => {}
                            _ => break,
                        }
                        j += 1;
                    }
                }
                if name == "dynamic"
                    && let Some(Tok::Str(s)) = toks.get(i + 1).map(|t| &t.tok)
                {
                    f.bound.insert(s.clone());
                }
                if name == "iterator"
                    && next_eq
                    && let Some(Tok::Ident(s)) = toks.get(i + 2).map(|t| &t.tok)
                {
                    f.bound.insert(s.clone());
                }
                // A reference: a dotted chain not after a dot, not an attribute name.
                let after_dot = i > 0 && toks[i - 1].tok == Tok::Dot;
                if !after_dot && !stack.is_empty() {
                    let mut parts = vec![(name.clone(), t.col)];
                    let mut j = i + 1;
                    while let (Some(Tok::Dot), Some(Tok::Ident(n))) =
                        (toks.get(j).map(|t| &t.tok), toks.get(j + 1).map(|t| &t.tok))
                    {
                        parts.push((n.clone(), toks[j + 1].col));
                        j += 2;
                    }
                    if let Some(address) = address_of(&parts) {
                        let last = parts[address.split('.').count() - 1].clone();
                        let len = last.1 + last.0.len() - t.col;
                        f.refs.push(Ref {
                            address,
                            file: file.to_owned(),
                            line: t.line,
                            col: t.col,
                            len,
                            within: current.and_then(|b| f.blocks[b].address()),
                        });
                    }
                    i = j;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    for (c, l, col) in stack {
        let what = if c == '{' { "block" } else { "bracket" };
        f.problems.push(Finding {
            path: file.to_owned(),
            line: l,
            start: col,
            end: col + 1,
            level: Level::Error,
            message: format!("This {what} is not closed: {} is missing", closing(c)),
        });
    }
    f
}

fn closing(c: char) -> char {
    match c {
        '{' => '}',
        '[' => ']',
        _ => ')',
    }
}

/// Whether a name looks like a resource type: a provider, an underscore, a kind.
fn resource_type(name: &str) -> bool {
    let mut parts = name.split('_');
    let first = parts.next().unwrap_or_default();
    !first.is_empty()
        && first.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && parts.next().is_some_and(|p| !p.is_empty())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The address a dotted chain names, if it names one.
fn address_of(parts: &[(String, usize)]) -> Option<String> {
    let p: Vec<&str> = parts.iter().map(|(s, _)| s.as_str()).collect();
    match p.as_slice() {
        ["var" | "local" | "module", name, ..] => Some(format!("{}.{name}", p[0])),
        ["data", t, n, ..] => Some(format!("data.{t}.{n}")),
        [t, n, ..] if !KEYWORDS.contains(t) && resource_type(t) => Some(format!("{t}.{n}")),
        _ => None,
    }
}

/// Checks a folder's files together, as Terraform reads a module: references to what isn't
/// declared, and variables that nothing uses.
pub fn check_module(files: &[File], out: &mut Vec<Finding>) {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut bound: BTreeSet<&str> = BTreeSet::new();
    for f in files {
        bound.extend(f.bound.iter().map(String::as_str));
        for b in &f.blocks {
            if let Some(a) = b.address() {
                declared.insert(a);
            }
            for (n, _) in &b.names {
                declared.insert(format!("local.{n}"));
            }
        }
    }
    let used: BTreeSet<&str> = files.iter().flat_map(|f| &f.refs).map(|r| r.address.as_str()).collect();
    for r in files.iter().flat_map(|f| &f.refs) {
        let first = r.address.split('.').next().unwrap_or_default();
        if declared.contains(&r.address) || bound.contains(first) {
            continue;
        }
        let what = match first {
            "var" => "variable",
            "local" => "local value",
            "module" => "module",
            "data" => "data source",
            _ => "resource",
        };
        out.push(Finding {
            path: r.file.clone(),
            line: r.line,
            start: r.col,
            end: r.col + r.len,
            level: Level::Error,
            message: format!("{} is not declared: no {what} by that name in this folder", r.address),
        });
    }
    for f in files {
        for b in f.blocks.iter().filter(|b| b.kind == "variable") {
            let Some(a) = b.address() else { continue };
            if !used.contains(a.as_str()) {
                out.push(Finding {
                    path: b.file.clone(),
                    line: b.line,
                    start: 0,
                    end: 8,
                    level: Level::Hint,
                    message: format!("Variable {} is declared but nothing in this folder uses {a}", b.labels[0]),
                });
            }
        }
    }
}

/// The links between a module's blocks, for the graph: (from, to).
pub fn links(files: &[File]) -> Vec<(String, String)> {
    let declared: BTreeSet<String> = files.iter().flat_map(|f| &f.blocks).filter_map(Block::address).collect();
    let mut out: Vec<(String, String)> = Vec::new();
    for r in files.iter().flat_map(|f| &f.refs) {
        if let Some(w) = &r.within
            && w != &r.address
            && declared.contains(&r.address)
            && !out.iter().any(|(a, b)| a == w && b == &r.address)
        {
            out.push((w.clone(), r.address.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = r#"
variable "region" {
  type    = string
  default = "eu-west-1"
}

variable "unused" {}

locals {
  name = "app-${var.region}"
  tags = { Env = "prod" }
}

resource "aws_instance" "web" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = var.size
  subnet_id     = aws_subnet.main.id
  tags          = merge(local.tags, { Name = local.name })
  user_data     = <<-EOT
    #!/bin/sh
    echo ${aws_s3_bucket.logs.bucket}
  EOT
  dynamic "ebs_block_device" {
    for_each = var.disks
    content {
      device_name = ebs_block_device.value.name
    }
  }
}

data "aws_ami" "ubuntu" {
  most_recent = true
}

module "net" {
  source = "./modules/net"
  cidr   = "10.0.0.0/16"
}

output "ip" {
  value = [for s_x in module.net.subnets : s_x.id]
}
"#;

    fn messages(f: &[Finding]) -> Vec<String> {
        let mut f = f.to_vec();
        f.sort_by_key(|f| (f.line, f.start));
        f.iter().map(|f| format!("{}: {}", f.line, f.message)).collect()
    }

    #[test]
    fn blocks_and_refs() {
        let f = read("main.tf", MAIN);
        assert!(f.problems.is_empty(), "{:?}", messages(&f.problems));
        let addrs: Vec<String> = f.blocks.iter().filter_map(Block::address).collect();
        assert_eq!(
            addrs,
            vec!["var.region", "var.unused", "aws_instance.web", "data.aws_ami.ubuntu", "module.net", "output.ip"]
        );
        let locals = f.blocks.iter().find(|b| b.kind == "locals").unwrap();
        assert_eq!(locals.names, vec![("name".to_owned(), 10), ("tags".to_owned(), 11)]);
        assert_eq!(f.blocks[5].source.as_deref(), Some("./modules/net"));
        assert_eq!(f.blocks[3].line, 14);
        assert_eq!(f.blocks[3].end, 29);
        let refs: Vec<String> = f.refs.iter().map(|r| format!("{}:{}", r.line, r.address)).collect();
        assert_eq!(
            refs,
            vec![
                "10:var.region",
                "15:data.aws_ami.ubuntu",
                "16:var.size",
                "17:aws_subnet.main",
                "18:local.tags",
                "18:local.name",
                "21:aws_s3_bucket.logs",
                "24:var.disks",
                "26:ebs_block_device.value",
                "41:module.net",
                "41:s_x.id",
            ]
        );
        assert!(f.bound.contains("ebs_block_device") && f.bound.contains("s_x"));
    }

    #[test]
    fn undeclared_and_unused() {
        let f = read("main.tf", MAIN);
        let mut out = Vec::new();
        check_module(&[f], &mut out);
        assert_eq!(
            messages(&out),
            vec![
                "7: Variable unused is declared but nothing in this folder uses var.unused",
                "16: var.size is not declared: no variable by that name in this folder",
                "17: aws_subnet.main is not declared: no resource by that name in this folder",
                "21: aws_s3_bucket.logs is not declared: no resource by that name in this folder",
                "24: var.disks is not declared: no variable by that name in this folder",
            ]
        );
        // Declared in another file of the folder: fine.
        let other = read(
            "vars.tf",
            "variable \"size\" {}\nvariable \"disks\" {}\nresource \"aws_subnet\" \"main\" {}\nresource \"aws_s3_bucket\" \"logs\" {}\n",
        );
        let mut out = Vec::new();
        check_module(&[read("main.tf", MAIN), other], &mut out);
        assert_eq!(messages(&out), vec!["7: Variable unused is declared but nothing in this folder uses var.unused"]);
    }

    #[test]
    fn unclosed() {
        let f = read("a.tf", "resource \"a_b\" \"c\" {\n  x = [1, 2\n}\n");
        assert_eq!(
            messages(&f.problems),
            vec!["1: This block is not closed: } is missing", "3: } closes the [ opened on line 2"]
        );
        let f = read("a.tf", "resource \"a_b\" \"c\" {\n  x = 1\n");
        assert_eq!(messages(&f.problems), vec!["1: This block is not closed: } is missing"]);
        let f = read("a.tf", "x = 1\n}\n");
        assert_eq!(messages(&f.problems), vec!["2: } closes nothing: there's no open block here"]);
        let f = read("a.tf", "a = \"open\nb = 2\n");
        assert_eq!(messages(&f.problems), vec!["1: This string is not closed on its line"]);
        let f = read("a.tf", "a = <<EOF\nnever\n");
        assert_eq!(messages(&f.problems), vec!["1: This heredoc is not closed: no line EOF after it"]);
        let f = read("a.tf", "/* open\n");
        assert_eq!(messages(&f.problems), vec!["1: This comment is not closed: */ is missing"]);
    }

    #[test]
    fn strings_are_not_references() {
        let f = read(
            "a.tf",
            "resource \"a_b\" \"c\" {\n  x = \"aws_instance.web is text\"\n  y = \"${var.x}-${local.y}\"\n  z = \"$${var.not}\"\n  # aws_x.y\n}\n",
        );
        let refs: Vec<&str> = f.refs.iter().map(|r| r.address.as_str()).collect();
        assert_eq!(refs, vec!["var.x", "local.y"]);
        assert_eq!(f.refs[1].col, 18);
        assert!(f.problems.is_empty(), "{:?}", messages(&f.problems));
    }

    #[test]
    fn graph_links() {
        let f = read("main.tf", MAIN);
        let l = links(&[f]);
        assert!(l.contains(&("aws_instance.web".into(), "data.aws_ami.ubuntu".into())), "{l:?}");
        assert!(l.contains(&("output.ip".into(), "module.net".into())), "{l:?}");
        assert!(!l.iter().any(|(_, b)| b.starts_with("local.") || b.starts_with("ebs_block_device")));
    }

    #[test]
    fn resource_types() {
        assert!(resource_type("aws_instance"));
        assert!(resource_type("google_compute_instance"));
        assert!(!resource_type("each"));
        assert!(!resource_type("Foo_bar"));
        assert!(!resource_type("tags"));
    }
}
