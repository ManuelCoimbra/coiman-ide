//! A small Markdown reader for the preview: the common parts of CommonMark and
//! GitHub's tables. It never fetches anything: images show as their alt text.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Strike(String),
    Code(String),
    Link { text: String, url: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    Code {
        lang: String,
        text: String,
    },
    /// `marker` is what to draw: a bullet, `3.`, `[x]` or `[ ]`.
    Item {
        depth: usize,
        marker: String,
        text: Vec<Inline>,
    },
    Quote(Vec<Inline>),
    Rule,
    Table {
        header: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
}

/// Builds an emphasis span.
type MakeInline = fn(String) -> Inline;

/// Finds `close` at or after `from`, skipping backslash escapes.
fn find_close(chars: &[char], from: usize, close: &[char]) -> Option<usize> {
    let mut i = from;
    while i + close.len() <= chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i..i + close.len()] == *close {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn collect(chars: &[char]) -> String {
    chars.iter().collect()
}

/// Spans of one block of text.
pub fn parse_inlines(text: &str) -> Vec<Inline> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    let push = |plain: &mut String, out: &mut Vec<Inline>, inline: Inline| {
        if !plain.is_empty() {
            out.push(Inline::Text(std::mem::take(plain)));
        }
        out.push(inline);
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\\' && next.is_some_and(|n| n.is_ascii_punctuation()) {
            plain.extend(next);
            i += 2;
            continue;
        }
        if c == '`'
            && let Some(end) = find_close(&chars, i + 1, &['`'])
        {
            push(&mut plain, &mut out, Inline::Code(collect(&chars[i + 1..end])));
            i = end + 1;
            continue;
        }
        // ![alt](url) and [text](url).
        let image = c == '!' && next == Some('[');
        if c == '[' || image {
            let open = if image { i + 1 } else { i };
            if let Some(close) = find_close(&chars, open + 1, &[']'])
                && chars.get(close + 1) == Some(&'(')
                && let Some(end) = find_close(&chars, close + 2, &[')'])
            {
                let (label, url) = (collect(&chars[open + 1..close]), collect(&chars[close + 2..end]));
                if image {
                    plain.push_str(&format!("[image: {label}]"));
                } else {
                    // A title after the address isn't part of it.
                    let url = url.split_whitespace().next().unwrap_or("").to_owned();
                    push(&mut plain, &mut out, Inline::Link { text: label, url });
                }
                i = end + 1;
                continue;
            }
        }
        if c == '<'
            && let Some(end) = find_close(&chars, i + 1, &['>'])
        {
            let url = collect(&chars[i + 1..end]);
            if url.starts_with("http://") || url.starts_with("https://") {
                push(&mut plain, &mut out, Inline::Link { text: url.clone(), url });
                i = end + 1;
                continue;
            }
        }
        let spans: [(&[char], MakeInline); 5] = [
            (&['*', '*'], Inline::Bold),
            (&['_', '_'], Inline::Bold),
            (&['~', '~'], Inline::Strike),
            (&['*'], Inline::Italic),
            (&['_'], Inline::Italic),
        ];
        let mut matched = false;
        for (mark, make) in spans {
            if chars[i..].starts_with(mark) {
                let inner = i + mark.len();
                // `snake_case` and `2 * 3` aren't emphasis.
                let inside_word = c == '_' && i > 0 && chars[i - 1].is_alphanumeric();
                let opens = chars.get(inner).is_some_and(|n| !n.is_whitespace());
                if !inside_word
                    && opens
                    && let Some(end) = find_close(&chars, inner, mark)
                {
                    let closes_word = c == '_' && chars.get(end + mark.len()).is_some_and(|n| n.is_alphanumeric());
                    if end > inner && !closes_word && !chars[end - 1].is_whitespace() {
                        push(&mut plain, &mut out, make(collect(&chars[inner..end])));
                        i = end + mark.len();
                        matched = true;
                        break;
                    }
                }
            }
        }
        if !matched {
            plain.push(c);
            i += 1;
        }
    }
    if !plain.is_empty() {
        out.push(Inline::Text(plain));
    }
    out
}

fn table_cells(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let line = line.strip_suffix('|').unwrap_or(line);
    line.split('|').map(|c| c.trim().to_owned()).collect()
}

fn is_table_divider(line: &str) -> bool {
    line.contains('-') && line.contains('|') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn list_item(line: &str) -> Option<(usize, String, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = line.trim_start();
    let (marker, body) = if let Some(body) = ["- ", "* ", "+ "].iter().find_map(|m| rest.strip_prefix(m)) {
        ("•".to_owned(), body)
    } else {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let body =
            rest.get(digits..).filter(|_| digits > 0).and_then(|r| r.strip_prefix(". ").or(r.strip_prefix(") ")))?;
        (format!("{}.", &rest[..digits]), body)
    };
    let (marker, body) = match body.get(..4) {
        Some("[ ] ") => ("[ ]".to_owned(), &body[4..]),
        Some("[x] " | "[X] ") => ("[x]".to_owned(), &body[4..]),
        _ => (marker, body),
    };
    Some((indent / 2, marker, body))
}

fn is_rule(line: &str) -> bool {
    let t: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3 && ["-", "*", "_"].iter().any(|m| t.chars().all(|c| m.starts_with(c)))
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let t = line.trim_start();
    let level = t.chars().take_while(|c| *c == '#').count();
    let rest = t.get(level..)?;
    ((1..=6).contains(&level) && (rest.is_empty() || rest.starts_with(' ')))
        .then(|| (level as u8, rest.trim().trim_end_matches('#').trim_end()))
}

fn starts_block(line: &str) -> bool {
    let t = line.trim_start();
    heading(line).is_some()
        || t.starts_with("```")
        || t.starts_with("~~~")
        || t.starts_with('>')
        || is_rule(line)
        || list_item(line).is_some()
}

/// A Markdown document as blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim_start();
        if t.is_empty() {
            i += 1;
        } else if let Some(fence) = ["```", "~~~"].iter().find(|f| t.starts_with(**f)) {
            let lang = t[3..].trim().to_owned();
            let end = lines[i + 1..]
                .iter()
                .position(|l| l.trim_start().starts_with(fence))
                .map_or(lines.len(), |p| i + 1 + p);
            out.push(Block::Code { lang, text: lines[i + 1..end].join("\n") });
            i = end + 1;
        } else if let Some((level, title)) = heading(line) {
            out.push(Block::Heading(level, parse_inlines(title)));
            i += 1;
        } else if is_rule(line) {
            out.push(Block::Rule);
            i += 1;
        } else if t.starts_with('>') {
            let start = i;
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                i += 1;
            }
            let body: Vec<&str> =
                lines[start..i].iter().map(|l| l.trim_start().trim_start_matches('>').trim()).collect();
            out.push(Block::Quote(parse_inlines(&body.join(" "))));
        } else if let Some((depth, marker, body)) = list_item(line) {
            out.push(Block::Item { depth, marker, text: parse_inlines(body) });
            i += 1;
        } else if line.contains('|') && lines.get(i + 1).is_some_and(|l| is_table_divider(l)) {
            let cells = |l: &str| table_cells(l).iter().map(|c| parse_inlines(c)).collect::<Vec<_>>();
            let header = cells(line);
            i += 2;
            let mut rows = Vec::new();
            while i < lines.len() && lines[i].contains('|') && !lines[i].trim().is_empty() {
                rows.push(cells(lines[i]));
                i += 1;
            }
            out.push(Block::Table { header, rows });
        } else {
            let start = i;
            i += 1;
            while i < lines.len() && !lines[i].trim().is_empty() && !starts_block(lines[i]) {
                i += 1;
            }
            let body: Vec<&str> = lines[start..i].iter().map(|l| l.trim()).collect();
            out.push(Block::Paragraph(parse_inlines(&body.join(" "))));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::Inline::*;
    use super::*;

    fn text(s: &str) -> Inline {
        Text(s.to_owned())
    }

    #[test]
    fn reads_spans() {
        assert_eq!(
            parse_inlines("a **b** *c* `d*e` ~~f~~ [g](http://h \"t\")"),
            vec![
                text("a "),
                Bold("b".into()),
                text(" "),
                Italic("c".into()),
                text(" "),
                Code("d*e".into()),
                text(" "),
                Strike("f".into()),
                text(" "),
                Link { text: "g".into(), url: "http://h".into() },
            ]
        );
        assert_eq!(parse_inlines("snake_case_name and 2 * 3 * 4"), vec![text("snake_case_name and 2 * 3 * 4")]);
        assert_eq!(parse_inlines(r"\*not\* it"), vec![text("*not* it")]);
        assert_eq!(parse_inlines("![logo](x.png)"), vec![text("[image: logo]")]);
        assert_eq!(parse_inlines("unclosed **bold and `code"), vec![text("unclosed **bold and `code")]);
        assert_eq!(
            parse_inlines("<https://a.b> <div>")[0],
            Link { text: "https://a.b".into(), url: "https://a.b".into() }
        );
    }

    #[test]
    fn reads_blocks() {
        let doc = "# Title #\n\nOne\ntwo\n\n- a\n  - [x] b\n3. c\n\n> quote\n> more\n\n```rust\nlet x = 1;\n\n# not a heading\n```\n---\n| A | B |\n|---|:-:|\n| 1 | **2** |\n\nend";
        let blocks = parse(doc);
        assert_eq!(blocks[0], Block::Heading(1, vec![text("Title")]));
        assert_eq!(blocks[1], Block::Paragraph(vec![text("One two")]));
        assert_eq!(blocks[2], Block::Item { depth: 0, marker: "•".into(), text: vec![text("a")] });
        assert_eq!(blocks[3], Block::Item { depth: 1, marker: "[x]".into(), text: vec![text("b")] });
        assert_eq!(blocks[4], Block::Item { depth: 0, marker: "3.".into(), text: vec![text("c")] });
        assert_eq!(blocks[5], Block::Quote(vec![text("quote more")]));
        assert_eq!(blocks[6], Block::Code { lang: "rust".into(), text: "let x = 1;\n\n# not a heading".into() });
        assert_eq!(blocks[7], Block::Rule);
        assert_eq!(
            blocks[8],
            Block::Table {
                header: vec![vec![text("A")], vec![text("B")]],
                rows: vec![vec![vec![text("1")], vec![Bold("2".into())]]],
            }
        );
        assert_eq!(blocks[9], Block::Paragraph(vec![text("end")]));
        // An unclosed fence runs to the end; odd input doesn't panic.
        assert_eq!(parse("```\nx"), vec![Block::Code { lang: String::new(), text: "x".into() }]);
        assert_eq!(parse("#hashtag")[0], Block::Paragraph(vec![text("#hashtag")]));
        for odd in ["", "#", "- ", "|", "|\n|-|", "```", ">", "1.", "é - è", "[a](", "![", "*", "**", "`"] {
            let _ = parse(odd);
        }
    }
}
