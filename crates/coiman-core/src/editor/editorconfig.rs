//! `.editorconfig` files and indentation detection. The files are plain data:
//! nothing in them can run, so they apply in untrusted folders too.

use std::path::Path;

/// What `.editorconfig` says about a file; `None` where it says nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub use_tabs: Option<bool>,
    pub indent_size: Option<usize>,
    pub trim_trailing_whitespace: Option<bool>,
    pub insert_final_newline: Option<bool>,
}

fn apply_property(config: &mut Config, key: &str, value: &str) {
    let flag = || match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    match key {
        "indent_style" => {
            config.use_tabs = match value {
                "tab" => Some(true),
                "space" => Some(false),
                _ => None,
            }
        }
        // `indent_size = tab` means "as wide as a tab", which is the default here.
        "indent_size" => config.indent_size = value.parse().ok().filter(|n| *n > 0),
        "tab_width" => {
            if config.indent_size.is_none() {
                config.indent_size = value.parse().ok().filter(|n| *n > 0);
            }
        }
        "trim_trailing_whitespace" => config.trim_trailing_whitespace = flag(),
        "insert_final_newline" => config.insert_final_newline = flag(),
        _ => {}
    }
}

/// Expands the first `{a,b}` in a pattern, recursively.
fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else { return vec![pattern.to_owned()] };
    let Some(close) = pattern[open..].find('}').map(|c| c + open) else { return vec![pattern.to_owned()] };
    let (head, inner, tail) = (&pattern[..open], &pattern[open + 1..close], &pattern[close + 1..]);
    if !inner.contains(',') {
        return vec![pattern.to_owned()];
    }
    inner.split(',').flat_map(|option| expand_braces(&format!("{head}{option}{tail}"))).collect()
}

fn glob(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') if pattern.get(1) == Some(&'*') => (0..=text.len()).any(|i| glob(&pattern[2..], &text[i..])),
        Some('*') => {
            let stop = text.iter().position(|c| *c == '/').unwrap_or(text.len());
            (0..=stop).any(|i| glob(&pattern[1..], &text[i..]))
        }
        Some('?') => text.first().is_some_and(|c| *c != '/') && glob(&pattern[1..], &text[1..]),
        Some('[') => {
            let Some(close) = pattern.iter().position(|c| *c == ']') else {
                return text.first() == Some(&'[') && glob(&pattern[1..], &text[1..]);
            };
            let Some(c) = text.first() else { return false };
            let (negate, set) = match pattern.get(1) {
                Some('!') => (true, &pattern[2..close]),
                _ => (false, &pattern[1..close]),
            };
            set.contains(c) != negate && glob(&pattern[close + 1..], &text[1..])
        }
        Some('\\') if pattern.len() > 1 => text.first() == Some(&pattern[1]) && glob(&pattern[2..], &text[1..]),
        Some(p) => text.first() == Some(p) && glob(&pattern[1..], &text[1..]),
    }
}

/// Whether a section header matches a path relative to the config's folder.
/// Patterns without `/` match the file name at any depth.
fn section_matches(section: &str, relative: &str) -> bool {
    expand_braces(section).iter().any(|pattern| {
        let chars: Vec<char> = if let Some(rooted) = pattern.strip_prefix('/') {
            rooted.chars().collect()
        } else if pattern.contains('/') {
            pattern.chars().collect()
        } else {
            format!("**/{pattern}").chars().collect()
        };
        let text: Vec<char> = relative.chars().collect();
        // `**/x` also matches `x` at the top.
        glob(&chars, &text) || chars.starts_with(&['*', '*', '/']) && glob(&chars[3..], &text)
    })
}

/// Applies one file's sections to `config`. Returns whether it says `root = true`.
fn apply_file(text: &str, relative: &str, config: &mut Config) -> bool {
    let mut root = false;
    let mut active = false;
    let mut in_preamble = true;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_preamble = false;
            active = section_matches(section, relative);
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
        if in_preamble {
            root |= key == "root" && value == "true";
        } else if active {
            apply_property(config, &key, &value);
        }
    }
    root
}

/// The settings for `path` from every `.editorconfig` above it. Nearer files
/// win, and a file with `root = true` ends the search.
pub fn lookup(path: &Path) -> Config {
    let mut files = Vec::new();
    for dir in path.ancestors().skip(1) {
        let Ok(text) = std::fs::read_to_string(dir.join(".editorconfig")) else { continue };
        let is_root = text.lines().take_while(|l| !l.trim_start().starts_with('[')).any(|l| {
            l.split_once('=')
                .is_some_and(|(k, v)| k.trim().eq_ignore_ascii_case("root") && v.trim().eq_ignore_ascii_case("true"))
        });
        files.push((dir.to_path_buf(), text));
        if is_root {
            break;
        }
    }
    let mut config = Config::default();
    // Farthest first, so nearer ones overwrite.
    for (dir, text) in files.iter().rev() {
        let Ok(relative) = path.strip_prefix(dir) else { continue };
        apply_file(text, &relative.to_string_lossy().replace('\\', "/"), &mut config);
    }
    config
}

/// A file's indentation from its own lines: (uses tabs, width). `None` when
/// too few lines are indented to tell.
pub fn detect_indent(text: &str) -> Option<(bool, usize)> {
    let (mut tabs, mut spaces) = (0usize, 0usize);
    let mut widths = [0usize; 9];
    let mut previous = 0usize;
    for line in text.lines().take(5000) {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.chars().take_while(|c| *c == ' ').count();
        if line.starts_with('\t') {
            tabs += 1;
        } else if indent >= 2 {
            spaces += 1;
        }
        // How much each line goes in compared to the one before.
        if !line.starts_with('\t') {
            if indent > previous && (2..=8).contains(&(indent - previous)) {
                widths[indent - previous] += 1;
            }
            previous = indent;
        }
    }
    if tabs + spaces < 2 {
        return None;
    }
    if tabs > spaces {
        return Some((true, 4));
    }
    let width = (2..=8).max_by_key(|w| (widths[*w], std::cmp::Reverse(*w)))?;
    (widths[width] > 0).then_some((false, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_sections_like_editorconfig() {
        assert!(section_matches("*", "a/b.rs"));
        assert!(section_matches("*.rs", "b.rs") && section_matches("*.rs", "a/b.rs"));
        assert!(!section_matches("*.rs", "a/b.py"));
        assert!(section_matches("*.{js,ts}", "x/y.ts"));
        assert!(section_matches("src/*.rs", "src/a.rs") && !section_matches("src/*.rs", "src/x/a.rs"));
        assert!(section_matches("src/**.rs", "src/x/a.rs"));
        assert!(section_matches("/top.txt", "top.txt") && !section_matches("/top.txt", "a/top.txt"));
        assert!(section_matches("[Mm]akefile", "Makefile"));
    }

    #[test]
    fn nearer_files_win_and_root_stops() {
        let base = std::env::temp_dir().join(format!("coiman-ec-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let inner = base.join("proj/sub");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(base.join(".editorconfig"), "[*]\nindent_style = tab\ninsert_final_newline = true\n").unwrap();
        std::fs::write(base.join("proj/.editorconfig"), "root = true\n[*]\nindent_style = space\nindent_size = 2\n")
            .unwrap();
        std::fs::write(inner.join(".editorconfig"), "[*.py]\nindent_size = 4\ntrim_trailing_whitespace = true\n")
            .unwrap();

        let py = lookup(&inner.join("a.py"));
        assert_eq!((py.use_tabs, py.indent_size, py.trim_trailing_whitespace), (Some(false), Some(4), Some(true)));
        let rs = lookup(&inner.join("a.rs"));
        assert_eq!((rs.indent_size, rs.trim_trailing_whitespace), (Some(2), None));
        // The file above root = true doesn't count.
        assert_eq!(rs.insert_final_newline, None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn detects_indentation() {
        assert_eq!(detect_indent("a {\n  b {\n    c\n  }\n}\nd {\n  e\n}\n"), Some((false, 2)));
        assert_eq!(detect_indent("a {\n    b\n    c\n}\nd {\n    e\n}\n"), Some((false, 4)));
        assert_eq!(detect_indent("a {\n\tb\n\tc\n}\n"), Some((true, 4)));
        assert_eq!(detect_indent("no\nindent\nhere\n"), None);
    }
}
