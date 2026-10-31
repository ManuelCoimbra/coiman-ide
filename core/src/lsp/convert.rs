//! Conversions between the editor's char offsets and LSP positions (line +
//! UTF-16 column), file paths and URIs, text diffs and snippets.

use std::path::{Path, PathBuf};

use lsp_types::{Position, Range, Url};
use ropey::Rope;

/// LSP position of char offset `pos`.
pub fn position_of(rope: &Rope, pos: usize) -> Position {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let col: usize = rope.slice(rope.line_to_char(line)..pos).chars().map(char::len_utf16).sum();
    Position::new(line as u32, col as u32)
}

/// Char offset of an LSP position, clamped to the line and the document.
pub fn char_of(rope: &Rope, p: Position) -> usize {
    let line = p.line as usize;
    if line >= rope.len_lines() {
        return rope.len_chars();
    }
    let start = rope.line_to_char(line);
    let mut units = 0;
    let mut offset = 0;
    for c in rope.line(line).chars() {
        if units >= p.character as usize || c == '\n' || c == '\r' {
            break;
        }
        units += c.len_utf16();
        offset += 1;
    }
    start + offset
}

pub fn range_of(rope: &Rope, start: usize, end: usize) -> Range {
    Range::new(position_of(rope, start), position_of(rope, end))
}

/// Char range of an LSP range.
pub fn chars_of(rope: &Rope, r: Range) -> (usize, usize) {
    let (a, b) = (char_of(rope, r.start), char_of(rope, r.end));
    (a.min(b), a.max(b))
}

pub fn uri_from_path(path: &Path) -> Option<Url> {
    Url::from_file_path(path).ok()
}

pub fn path_from_uri(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

/// The places named by a call or type hierarchy answer: `field` picks the item
/// out of each entry ("from", "to"), or `None` when the entries are items.
pub fn hierarchy_locations(value: &serde_json::Value, field: Option<&str>) -> Vec<lsp_types::Location> {
    let entries = value.as_array().map_or(&[][..], Vec::as_slice);
    entries
        .iter()
        .filter_map(|entry| {
            let item = field.map_or(entry, |f| &entry[f]);
            Some(lsp_types::Location {
                uri: serde_json::from_value(item["uri"].clone()).ok()?,
                range: serde_json::from_value(item["selectionRange"].clone()).ok()?,
            })
        })
        .collect()
}

/// Folder of the documents language servers supply, like decompiled classes.
fn virtual_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("virtual"))
}

/// Whether a file is a server-supplied document, which stays read-only.
pub fn is_virtual_path(path: &Path) -> bool {
    virtual_dir().is_some_and(|dir| path.starts_with(dir))
}

/// A safe file name for a document only a server can read: its last segment,
/// with a class named as the source the server gives for it.
fn virtual_file_name(uri: &Url) -> String {
    let last = uri.path().rsplit(['/', '\\']).find(|s| !s.is_empty()).unwrap_or_default();
    let name: String = last.chars().filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c)).collect();
    let name = name.trim_matches('.');
    match name.strip_suffix(".class") {
        Some(class) if !class.is_empty() => format!("{class}.java"),
        _ if name.is_empty() => "document.txt".to_owned(),
        _ => name.to_owned(),
    }
}

/// Where the text of the server-only document `uri` is kept.
pub fn virtual_path(uri: &Url) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    uri.as_str().hash(&mut hasher);
    Some(virtual_dir()?.join(format!("{:016x}", hasher.finish())).join(virtual_file_name(uri)))
}

/// Whether a web address stays on this computer.
pub fn is_loopback_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// LSP position at byte offset `byte` of `text`.
fn position_in_str(text: &str, byte: usize) -> Position {
    let before = &text[..byte];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let col: usize = before[line_start..].chars().map(char::len_utf16).sum();
    Position::new(line as u32, col as u32)
}

/// The smallest single edit turning `old` into `new`, as an LSP range in
/// `old` plus the replacement text. `None` if they are equal.
pub fn diff_change(old: &str, new: &str) -> Option<(Range, String)> {
    if old == new {
        return None;
    }
    let (o, n) = (old.as_bytes(), new.as_bytes());
    let mut prefix = o.iter().zip(n).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    // Never split a CRLF pair: positions inside it don't exist in LSP.
    if prefix > 0 && o[prefix - 1] == b'\r' {
        prefix -= 1;
    }
    let max_suffix = o.len().min(n.len()) - prefix;
    let mut suffix = o.iter().rev().zip(n.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(o.len() - suffix) || !new.is_char_boundary(n.len() - suffix) {
        suffix -= 1;
    }
    if suffix > 0 && o.len() - suffix > prefix && o[o.len() - suffix] == b'\n' && o[o.len() - suffix - 1] == b'\r' {
        suffix -= 1;
    }
    let range = Range::new(position_in_str(old, prefix), position_in_str(old, o.len() - suffix));
    Some((range, new[prefix..n.len() - suffix].to_owned()))
}

/// Expands an LSP snippet to plain text. Returns the text and the char range
/// to select afterwards: the first tab stop (with its placeholder), else
/// `$0`, else the end.
pub fn expand_snippet(snippet: &str) -> (String, (usize, usize)) {
    let mut out = String::new();
    let mut stops: Vec<(u32, usize, usize)> = Vec::new();
    expand_into(snippet, &mut out, &mut stops);
    let end = out.chars().count();
    let pick = stops
        .iter()
        .filter(|s| s.0 > 0)
        .min_by_key(|s| (s.0, s.1))
        .or_else(|| stops.iter().find(|s| s.0 == 0))
        .map_or((end, end), |s| (s.1, s.2));
    (out, pick)
}

fn expand_into(s: &str, out: &mut String, stops: &mut Vec<(u32, usize, usize)>) {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '$' | '}' | '\\') {
            out.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        // $1
        let digits: String = chars[i + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let at = out.chars().count();
            stops.push((digits.parse().unwrap_or(0), at, at));
            i += 1 + digits.len();
            continue;
        }
        // ${1:placeholder}, ${1|one,two|}, ${name}
        if chars.get(i + 1) == Some(&'{') {
            let Some(close) = matching_brace(&chars, i + 1) else {
                out.push(c);
                i += 1;
                continue;
            };
            let inner: String = chars[i + 2..close].iter().collect();
            let digits: String = inner.chars().take_while(char::is_ascii_digit).collect();
            let rest = &inner[digits.len()..];
            let start = out.chars().count();
            if let Some(placeholder) = rest.strip_prefix(':') {
                expand_into(placeholder, out, stops);
            } else if let Some(choices) = rest.strip_prefix('|') {
                out.push_str(choices.trim_end_matches('|').split(',').next().unwrap_or(""));
            }
            if !digits.is_empty() {
                stops.push((digits.parse().unwrap_or(0), start, out.chars().count()));
            }
            i = close + 1;
            continue;
        }
        out.push(c);
        i += 1;
    }
}

/// Index of the `}` closing the `{` at `open`, honoring nesting and escapes.
fn matching_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn virtual_file_names_are_safe() {
        let name = |s: &str| super::virtual_file_name(&lsp_types::Url::parse(s).unwrap());
        assert_eq!(name("jdt://contents/rt.jar/java.util/ArrayList.class?=x%2Fy"), "ArrayList.java");
        assert_eq!(name("custom://host/dir/lib.rs"), "lib.rs");
        assert_eq!(name("custom://host/..%2f..%2fevil.txt"), "2f..2fevil.txt");
        assert_eq!(name("custom://host/"), "document.txt");
        assert_eq!(name("custom://host/.class"), "class");
    }

    #[test]
    fn hierarchy_answers_become_locations() {
        let item = serde_json::json!({
            "uri": "file:///a.rs",
            "selectionRange": { "start": { "line": 2, "character": 0 }, "end": { "line": 2, "character": 1 } },
        });
        let calls = serde_json::json!([{ "from": item, "fromRanges": [] }, { "from": {} }]);
        let found = super::hierarchy_locations(&calls, Some("from"));
        assert_eq!(found.len(), 1, "an entry without a place is skipped");
        assert_eq!(found[0].range.start.line, 2);
        assert_eq!(super::hierarchy_locations(&serde_json::json!([item]), None).len(), 1);
        assert!(super::hierarchy_locations(&serde_json::Value::Null, None).is_empty());
    }

    #[test]
    fn only_local_web_addresses_are_loopback() {
        let local = |s: &str| super::is_loopback_url(&lsp_types::Url::parse(s).unwrap());
        assert!(local("http://localhost:8080/x") && local("http://127.0.0.1/") && local("https://[::1]:1/"));
        assert!(!local("https://example.com/") && !local("http://localhost.example.com/"));
        assert!(!local("file:///localhost"));
    }

    use super::*;

    #[test]
    fn positions_count_utf16_and_ignore_crlf() {
        let rope = Rope::from_str("a😀b\r\nxyz");
        assert_eq!(position_of(&rope, 2), Position::new(0, 3));
        assert_eq!(char_of(&rope, Position::new(0, 3)), 2);
        // Past the end of a line clamps before the CRLF.
        assert_eq!(char_of(&rope, Position::new(0, 99)), 3);
        assert_eq!(char_of(&rope, Position::new(1, 2)), 7);
        assert_eq!(char_of(&rope, Position::new(5, 0)), rope.len_chars());
    }

    #[test]
    fn diff_finds_the_minimal_range() {
        let (r, t) = diff_change("fn main() {}\n", "fn mainx() {}\n").unwrap();
        assert_eq!((r, t.as_str()), (Range::new(Position::new(0, 7), Position::new(0, 7)), "x"));
        let (r, t) = diff_change("a\nb\nc", "a\nc").unwrap();
        assert_eq!((r.start, r.end, t.as_str()), (Position::new(1, 0), Position::new(2, 0), ""));
        // Typing before a CRLF must not split it.
        let (r, t) = diff_change("ab\r\n", "abc\r\n").unwrap();
        assert_eq!((r.start, r.end, t.as_str()), (Position::new(0, 2), Position::new(0, 2), "c"));
        let (r, t) = diff_change("é", "è").unwrap();
        assert_eq!((r.end, t.as_str()), (Position::new(0, 1), "è"));
        assert!(diff_change("same", "same").is_none());
    }

    #[test]
    fn snippets_expand_with_the_first_tab_stop_selected() {
        assert_eq!(expand_snippet("foo(${1:a}, ${2:b})$0"), ("foo(a, b)".to_owned(), (4, 5)));
        assert_eq!(expand_snippet("vec![$0]"), ("vec![]".to_owned(), (5, 5)));
        assert_eq!(expand_snippet("println!(\"$1\")"), ("println!(\"\")".to_owned(), (10, 10)));
        assert_eq!(expand_snippet("plain"), ("plain".to_owned(), (5, 5)));
        assert_eq!(expand_snippet("${1|one,two|} \\$x"), ("one $x".to_owned(), (0, 3)));
        assert_eq!(expand_snippet("${1:outer ${2:inner}}"), ("outer inner".to_owned(), (0, 11)));
    }
}
