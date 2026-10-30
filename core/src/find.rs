//! Find and replace in a text: plain or regex, case and whole-word options.
//! Offsets are in chars, like the buffer's.

use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub query: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// One match and what replacing it gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

fn compile(opts: &FindOptions) -> Result<Regex, String> {
    let pattern = if opts.regex { opts.query.clone() } else { regex::escape(&opts.query) };
    let pattern = if opts.whole_word { format!(r"\b(?:{pattern})\b") } else { pattern };
    RegexBuilder::new(&pattern)
        .case_insensitive(!opts.match_case)
        .multi_line(true)
        .crlf(true)
        .build()
        .map_err(|e| e.to_string().lines().last().unwrap_or("Invalid regular expression").trim().to_owned())
}

/// Matches as char ranges, at most `limit`. Empty matches are skipped.
pub fn find_all(text: &str, opts: &FindOptions, limit: usize) -> Result<Vec<(usize, usize)>, String> {
    if opts.query.is_empty() {
        return Ok(Vec::new());
    }
    let regex = compile(opts)?;
    let mut out = Vec::new();
    let (mut byte, mut char_at) = (0, 0);
    for m in regex.find_iter(text).filter(|m| !m.is_empty()).take(limit) {
        char_at += text[byte..m.start()].chars().count();
        let end = char_at + m.as_str().chars().count();
        out.push((char_at, end));
        byte = m.end();
        char_at = end;
    }
    Ok(out)
}

/// Every match with its replacement. In regex mode, `$1` and `${name}` expand;
/// otherwise the replacement is literal.
pub fn replacements(text: &str, opts: &FindOptions, with: &str) -> Result<Vec<Replacement>, String> {
    if opts.query.is_empty() {
        return Ok(Vec::new());
    }
    let regex = compile(opts)?;
    let mut out = Vec::new();
    let (mut byte, mut char_at) = (0, 0);
    for caps in regex.captures_iter(text) {
        let m = caps.get(0).expect("group 0 always exists");
        if m.is_empty() {
            continue;
        }
        char_at += text[byte..m.start()].chars().count();
        let end = char_at + m.as_str().chars().count();
        let mut replaced = String::new();
        if opts.regex {
            caps.expand(with, &mut replaced);
        } else {
            replaced.push_str(with);
        }
        out.push(Replacement { start: char_at, end, text: replaced });
        byte = m.end();
        char_at = end;
    }
    Ok(out)
}

/// The whole text with every match replaced, and how many there were.
pub fn replace_in_text(text: &str, opts: &FindOptions, with: &str) -> Result<(String, usize), String> {
    let found = replacements(text, opts, with)?;
    let mut out = String::with_capacity(text.len());
    let mut chars = text.char_indices();
    let mut at = 0;
    let mut byte_of = |target: usize, at: &mut usize| {
        let mut byte = text.len();
        while *at <= target {
            match chars.next() {
                Some((b, _)) => {
                    if *at == target {
                        byte = b;
                    }
                    *at += 1;
                }
                None => {
                    *at += 1;
                    break;
                }
            }
        }
        byte
    };
    let mut last = 0;
    for r in &found {
        let start = byte_of(r.start, &mut at);
        let end = if r.end > r.start { byte_of(r.end, &mut at) } else { start };
        out.push_str(&text[last..start]);
        out.push_str(&r.text);
        last = end;
    }
    out.push_str(&text[last..]);
    Ok((out, found.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(query: &str) -> FindOptions {
        FindOptions { query: query.to_owned(), ..Default::default() }
    }

    #[test]
    fn finds_with_each_option() {
        assert_eq!(find_all("Foo foo", &opts("foo"), 10).unwrap(), [(0, 3), (4, 7)]);
        let case = FindOptions { match_case: true, ..opts("foo") };
        assert_eq!(find_all("Foo foo", &case, 10).unwrap(), [(4, 7)]);
        let word = FindOptions { whole_word: true, ..opts("foo") };
        assert_eq!(find_all("foobar foo", &word, 10).unwrap(), [(7, 10)]);
        // Plain text isn't a pattern.
        assert_eq!(find_all("a.b axb", &opts("a.b"), 10).unwrap(), [(0, 3)]);
        let re = FindOptions { regex: true, ..opts("a.b") };
        assert_eq!(find_all("a.b axb", &re, 10).unwrap(), [(0, 3), (4, 7)]);
        // Offsets count chars, not bytes.
        assert_eq!(find_all("é é x", &opts("x"), 10).unwrap(), [(4, 5)]);
        let bad = FindOptions { regex: true, ..opts("(") };
        assert!(find_all("x", &bad, 10).is_err());
        assert_eq!(find_all("aaa", &opts("a"), 2).unwrap().len(), 2);
    }

    #[test]
    fn replaces_literally_or_with_groups() {
        assert_eq!(replace_in_text("a é a", &opts("a"), "$1b").unwrap(), ("$1b é $1b".to_owned(), 2));
        let re = FindOptions { regex: true, ..opts(r"(\w+)=(\w+)") };
        assert_eq!(replace_in_text("x=1, é=2", &re, "$2=$1").unwrap(), ("1=x, 2=é".to_owned(), 2));
        // Lines end with CRLF: `$` still matches, the \r stays.
        let end = FindOptions { regex: true, ..opts("b$") };
        assert_eq!(replace_in_text("ab\r\nb\r\n", &end, "c").unwrap().0, "ac\r\nc\r\n");
        assert_eq!(replace_in_text("xyz", &opts("q"), "r").unwrap(), ("xyz".to_owned(), 0));
    }
}
