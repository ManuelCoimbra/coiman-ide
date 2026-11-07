//! Your own snippets, kept in `<config dir>/snippets.toml`. Bodies use the same
//! `$1`, `${1:text}` and `$0` as VS Code snippets.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    /// Typing this and pressing Tab inserts the snippet.
    pub prefix: String,
    /// Language names, separated by commas. Empty: every language.
    pub language: String,
    pub body: String,
}

impl Snippet {
    pub fn applies_to(&self, language: &str) -> bool {
        self.language.trim().is_empty() || self.language.split(',').any(|l| l.trim().eq_ignore_ascii_case(language))
    }
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    snippet: Vec<Snippet>,
}

pub fn file_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("snippets.toml"))
}

/// The saved snippets. A broken file is an error, so it never gets overwritten.
pub fn load() -> Result<Vec<Snippet>, String> {
    let Some(path) = file_path().filter(|p| p.exists()) else { return Ok(Vec::new()) };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

fn parse(text: &str) -> Result<Vec<Snippet>, String> {
    toml::from_str::<File>(text).map(|f| f.snippet).map_err(|e| format!("snippets.toml: {}", e.message()))
}

pub fn save(snippets: &[Snippet]) -> Result<(), String> {
    let path = file_path().ok_or("No config directory available")?;
    let text = toml::to_string(&File { snippet: snippets.to_vec() }).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The snippet whose prefix ends `before_cursor` as a whole word; the longest wins.
pub fn matching<'a>(snippets: &'a [Snippet], language: &str, before_cursor: &str) -> Option<&'a Snippet> {
    snippets
        .iter()
        .filter(|s| !s.prefix.is_empty() && s.applies_to(language) && before_cursor.ends_with(&s.prefix))
        .filter(|s| {
            let rest = &before_cursor[..before_cursor.len() - s.prefix.len()];
            !rest.chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
        .max_by_key(|s| s.prefix.len())
}

/// A body ready to insert on a line indented with `indent`: the text, and the
/// char range to select in it.
pub fn prepare(body: &str, indent: &str, nl: &str) -> (String, (usize, usize)) {
    let indented = body.replace("\r\n", "\n").replace('\n', &format!("\n{indent}"));
    let (text, (a, b)) = crate::language::lsp::convert::expand_snippet(&indented);
    // Offsets count one char per line break, which CRLF isn't.
    let extra = nl.chars().count() - 1;
    let shift = |at: usize| at + extra * text.chars().take(at).filter(|c| *c == '\n').count();
    let range = (shift(a), shift(b));
    (text.replace('\n', nl), range)
}

/// Like [`prepare`], with every tab stop rather than only the first.
pub fn prepare_stops(body: &str, indent: &str, nl: &str) -> (String, Vec<(usize, usize)>) {
    let indented = body.replace("\r\n", "\n").replace('\n', &format!("\n{indent}"));
    let (text, stops) = crate::language::lsp::convert::expand_snippet_stops(&indented);
    let extra = nl.chars().count() - 1;
    let shift = |at: usize| at + extra * text.chars().take(at).filter(|c| *c == '\n').count();
    let stops = stops.into_iter().map(|(a, b)| (shift(a), shift(b))).collect();
    (text.replace('\n', nl), stops)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepares_every_stop_in_order() {
        let (text, stops) = prepare_stops("<a href=\"$1\" id=\"${2:x}\">$0</a>", "", "\n");
        assert_eq!(text, "<a href=\"\" id=\"x\"></a>");
        assert_eq!(stops, [(9, 9), (15, 16), (18, 18)]);
        // Line breaks count as written.
        let (_, stops) = prepare_stops("a\n$1", "", "\r\n");
        assert_eq!(stops, [(3, 3)]);
        // A number repeated is one stop.
        let (_, stops) = prepare_stops("$1 $1 $2", "", "\n");
        assert_eq!(stops, [(0, 0), (2, 2)]);
    }

    fn snippet(prefix: &str, language: &str) -> Snippet {
        Snippet { name: prefix.into(), prefix: prefix.into(), language: language.into(), body: "x".into() }
    }

    #[test]
    fn matches_whole_words_for_the_language() {
        let all = [snippet("fo", ""), snippet("for", "Rust, Python"), snippet("log", "Go")];
        let pick = |language, text| matching(&all, language, text).map(|s| s.prefix.as_str());
        assert_eq!(pick("Rust", "    for"), Some("for"));
        assert_eq!(pick("python", "(for"), Some("for"));
        assert_eq!(pick("C", "for"), None);
        assert_eq!(pick("C", "fo"), Some("fo"));
        assert_eq!(pick("Rust", "before"), None);
        assert_eq!(pick("Rust", "my_for"), None);
        assert_eq!(pick("Rust", "log"), None);
        assert_eq!(pick("Rust", ""), None);
    }

    #[test]
    fn prepares_indent_line_breaks_and_selection() {
        assert_eq!(prepare("a\nb$0", "  ", "\n"), ("a\n  b".to_owned(), (5, 5)));
        assert_eq!(prepare("if ${1:x} {\n    $0\n}", "\t", "\n").1, (3, 4));
        let (text, range) = prepare("one\ntwo ${1:three}", "", "\r\n");
        assert_eq!(text, "one\r\ntwo three");
        assert_eq!(text.chars().skip(range.0).take(range.1 - range.0).collect::<String>(), "three");
        assert_eq!(prepare("plain", "", "\n"), ("plain".to_owned(), (5, 5)));
    }

    #[test]
    fn file_round_trips_and_reports_breakage() {
        let list = vec![Snippet {
            name: "Loop".into(),
            prefix: "lp".into(),
            language: "".into(),
            body: "loop {\n    \"$0\"\n}".into(),
        }];
        let text = toml::to_string(&File { snippet: list.clone() }).unwrap();
        assert_eq!(parse(&text).unwrap(), list);
        assert_eq!(parse("").unwrap(), []);
        assert!(parse("[[[").is_err());
    }
}
