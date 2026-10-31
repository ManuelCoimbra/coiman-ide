//! Backslash abbreviations: `\to` becomes `→` as soon as what's typed can't grow
//! into a longer name. Languages ask for this in their profile.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::RegexBuilder;

const TABLE: &str = include_str!("../data/symbols.tsv");

fn shipped() -> &'static BTreeMap<&'static str, &'static str> {
    static PARSED: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TABLE.lines().filter(|l| !l.starts_with('#')).filter_map(|line| line.split_once('\t')).collect()
    })
}

/// A name's symbol, from the user's table first.
fn lookup<'a>(name: &str, mine: &'a HashMap<String, String>) -> Option<&'a str> {
    mine.get(name).map(String::as_str).or_else(|| shipped().get(name).copied())
}

fn can_grow(start: &str, mine: &HashMap<String, String>) -> bool {
    shipped().range(start..).next().is_some_and(|(k, _)| k.starts_with(start) && *k != start)
        || shipped().keys().any(|k| k.starts_with(start) && *k != start)
        || mine.keys().any(|k| k.starts_with(start) && k != start)
}

/// A piece of the text to replace, counted back from the cursor.
#[derive(Debug, PartialEq, Eq)]
pub struct Replacement {
    /// Chars back from the cursor to the backslash.
    pub back: usize,
    /// Chars to replace, from there.
    pub length: usize,
    pub symbol: String,
}

/// The backslash of an abbreviation being typed: its place among `chars`.
fn backslash(chars: &[char]) -> Option<usize> {
    const LONGEST: usize = 16;
    let from = chars.len().saturating_sub(LONGEST + 2);
    let at = (from..chars.len()).rev().find(|&i| chars[i] == '\\')?;
    // A doubled backslash is a backslash, and white space ends a name.
    let escaped = at > 0 && chars[at - 1] == '\\';
    (!escaped && !chars[at + 1..chars.len().saturating_sub(1)].iter().any(|c| c.is_whitespace())).then_some(at)
}

/// After a character was typed: what `before`, the line up to the cursor, turns
/// into. A name becomes its symbol once the new character can't continue it.
pub fn after_typing(before: &str, mine: &HashMap<String, String>) -> Option<Replacement> {
    let chars: Vec<char> = before.chars().collect();
    let at = backslash(&chars)?;
    let name: String = chars[at + 1..chars.len() - 1].iter().collect();
    let with_new: String = chars[at + 1..].iter().collect();
    if name.is_empty() || lookup(&with_new, mine).is_some() || can_grow(&with_new, mine) {
        return None;
    }
    let symbol = lookup(&name, mine)?.to_owned();
    Some(Replacement { back: chars.len() - at, length: chars.len() - 1 - at, symbol })
}

/// On Tab: the whole name before the cursor, whatever could follow it.
pub fn at_cursor(before: &str, mine: &HashMap<String, String>) -> Option<Replacement> {
    let mut chars: Vec<char> = before.chars().collect();
    // `backslash` allows for one typed character after the name.
    chars.push(' ');
    let at = backslash(&chars)?;
    chars.pop();
    let name: String = chars[at + 1..].iter().collect();
    let symbol = lookup(&name, mine)?.to_owned();
    Some(Replacement { back: chars.len() - at, length: chars.len() - at, symbol })
}

/// Every symbol once, with all its names, for the picker.
pub fn all(mine: &HashMap<String, String>) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let table = shipped().iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned()));
    for (name, symbol) in table.chain(mine.iter().map(|(k, v)| (k.clone(), v.clone()))) {
        match out.iter_mut().find(|(s, _)| *s == symbol) {
            Some((_, names)) => names.push(name),
            None => out.push((symbol, vec![name])),
        }
    }
    out
}

/// The name a statement on `line` includes, by a pattern with one capture.
pub fn included_name(pattern: &str, line: &str) -> Option<String> {
    let regex = RegexBuilder::new(pattern).case_insensitive(true).build().ok()?;
    regex.captures(line)?.get(1).map(|m| m.as_str().to_owned())
}

/// Where an included name is: beside the file, then along the search paths, as
/// written and with each extension. Returns the places it looked if it isn't.
pub fn find_include(
    name: &str,
    file: &Path,
    root: &Path,
    paths: &[&str],
    extensions: &[&str],
) -> Result<PathBuf, Vec<PathBuf>> {
    let mut dirs: Vec<PathBuf> = file.parent().into_iter().map(Path::to_path_buf).collect();
    dirs.extend(paths.iter().map(|p| root.join(p)));
    for dir in &dirs {
        let bare = dir.join(name);
        let tries = std::iter::once(bare.clone()).chain(extensions.iter().map(|e| bare.with_extension(e)));
        for attempt in tries {
            if attempt.is_file() {
                return Ok(attempt);
            }
        }
    }
    Err(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn the_table_reads() {
        assert!(shipped().len() > 150, "{}", shipped().len());
        assert_eq!(shipped().get("forall"), Some(&"∀"));
        assert_eq!(shipped().get("_1"), Some(&"₁"));
    }

    #[test]
    fn a_name_turns_when_it_cannot_grow() {
        let typed = |before: &str| after_typing(before, &none());
        assert_eq!(typed("x \\to ").unwrap(), Replacement { back: 4, length: 3, symbol: "→".into() });
        assert_eq!(typed("\\forall(").unwrap().symbol, "∀");
        // Still growing, or not finished.
        assert_eq!(typed("\\fora"), None);
        assert_eq!(typed("\\a"), None);
        assert_eq!(typed("\\al"), None);
        // `\a` could have been `\alpha`, but a space ends it.
        assert_eq!(typed("\\a ").unwrap().symbol, "α");
        assert_eq!(typed("\\nope "), None);
        assert_eq!(typed("no backslash "), None);
        assert_eq!(typed("\\\\to "), None, "an escaped backslash");
        assert_eq!(typed("\\to and more "), None, "white space ended it");
        assert_eq!(typed("\\ "), None);
    }

    #[test]
    fn names_with_marks_work() {
        let typed = |before: &str| after_typing(before, &none());
        assert_eq!(typed("\\_1 ").unwrap().symbol, "₁");
        assert_eq!(typed("\\|- ").unwrap().symbol, "⊢");
        assert_eq!(typed("\\<- "), None);
        assert_eq!(typed("\\<-> ").unwrap().symbol, "↔");
        assert_eq!(at_cursor("\\<", &none()).unwrap().symbol, "⟨");
    }

    #[test]
    fn tab_takes_the_name_as_it_is() {
        let tab = |before: &str| at_cursor(before, &none());
        assert_eq!(tab("x \\a").unwrap(), Replacement { back: 2, length: 2, symbol: "α".into() });
        assert_eq!(tab("\\forall").unwrap().symbol, "∀");
        assert_eq!(tab("\\nope"), None);
        assert_eq!(tab("plain"), None);
        assert_eq!(tab("\\to x"), None);
    }

    #[test]
    fn the_user_table_adds_and_wins() {
        let mine: HashMap<String, String> =
            [("to".to_owned(), "⟶".to_owned()), ("yes".to_owned(), "✓".to_owned())].into();
        assert_eq!(after_typing("\\to ", &mine).unwrap().symbol, "⟶");
        assert_eq!(after_typing("\\yes ", &mine).unwrap().symbol, "✓");
        assert_eq!(after_typing("\\ye", &mine), None);
    }

    #[test]
    fn the_picker_lists_each_symbol_once() {
        let all = all(&none());
        let arrow = all.iter().find(|(s, _)| s == "→").unwrap();
        assert!(arrow.1.contains(&"to".to_owned()) && arrow.1.contains(&"r".to_owned()));
        assert_eq!(all.iter().filter(|(s, _)| s == "→").count(), 1);
    }

    #[test]
    fn finds_includes_along_the_paths() {
        let root = std::env::temp_dir().join(format!("rc-include-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("copy")).unwrap();
        let file = root.join("src/a.cbl");
        std::fs::write(&file, "").unwrap();
        std::fs::write(root.join("copy/CUST.cpy"), "").unwrap();
        std::fs::write(root.join("src/near.inc"), "").unwrap();
        let find = |name: &str| find_include(name, &file, &root, &["missing", "copy"], &["cpy", "cob"]);
        assert_eq!(find("CUST"), Ok(root.join("copy/CUST.cpy")));
        assert_eq!(find("near.inc"), Ok(root.join("src/near.inc")));
        let looked = find("NONE").unwrap_err();
        assert_eq!(looked.len(), 3);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_include_statements() {
        let cobol = r#"^.{0,7}\s*COPY\s+["']?([\w-]+)"#;
        assert_eq!(included_name(cobol, "000100     COPY CUSTREC.").as_deref(), Some("CUSTREC"));
        assert_eq!(included_name(cobol, "       copy 'x-y'.").as_deref(), Some("x-y"));
        assert_eq!(included_name(cobol, "       MOVE A TO B."), None);
        let fortran = r#"^\s*include\s+['"]([^'"]+)['"]"#;
        assert_eq!(included_name(fortran, "      INCLUDE 'common.inc'").as_deref(), Some("common.inc"));
        assert_eq!(included_name("(", "x"), None, "a bad pattern finds nothing");
    }
}
