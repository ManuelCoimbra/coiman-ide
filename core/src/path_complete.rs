//! File and folder names offered while a path is typed in a string, as the Path
//! Intellisense extension does: `"./src/`, `src="img/`, `url(../`, `import x from './`,
//! `#include "`. Relative paths start at the file's folder; `/` at the open folder.

use std::path::{Path, PathBuf};

/// Names to offer, and where in the line (a char index) the part being typed starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub start: usize,
    /// Name, and whether it's a folder.
    pub entries: Vec<(String, bool)>,
}

/// Contexts where a string holds a path even without `./`.
const PATH_CONTEXTS: &[&str] = &[
    "src=", "href=", "url(", "from", "import", "require(", "#include", "@import", "@use", "@forward", "srcset=",
    "action=", "poster=",
];

/// The string being typed at the end of `before` (the line up to the cursor): its text,
/// where it starts, and what's before its quote.
fn open_string(before: &str) -> Option<(usize, &str, &str)> {
    let chars: Vec<(usize, char)> = before.char_indices().collect();
    let mut open: Option<(usize, char)> = None;
    for &(i, c) in &chars {
        match open {
            Some((_, q)) if c == q => open = None,
            Some(_) => {}
            None if matches!(c, '"' | '\'' | '`') => open = Some((i, c)),
            None => {}
        }
    }
    if let Some((at, q)) = open {
        let inner = &before[at + q.len_utf8()..];
        return Some((at + q.len_utf8(), inner, &before[..at]));
    }
    // `url(` without quotes.
    let at = before.rfind("url(")?;
    let inner = &before[at + 4..];
    (!inner.contains(')') && !inner.contains(' ')).then_some((at + 4, inner, &before[..at + 4]))
}

/// What to offer at the end of `before`, for a file in `dir`, in a project at `root`.
pub fn complete(before: &str, dir: &Path, root: Option<&Path>) -> Option<Offer> {
    let (start, typed, lead) = open_string(before)?;
    let lead = lead.trim_end();
    let pathlike = typed.starts_with("./") || typed.starts_with("../") || typed.starts_with('/');
    let in_context = PATH_CONTEXTS.iter().any(|c| lead.ends_with(c) || lead.ends_with(&format!("{c} ")));
    if !pathlike && !in_context {
        return None;
    }
    // Web addresses and the like aren't files.
    if typed.contains("://") || typed.starts_with("data:") || typed.starts_with("#") || typed.contains('\\') {
        return None;
    }
    let (folder, segment) = match typed.rfind('/') {
        Some(i) => (&typed[..=i], &typed[i + 1..]),
        None => ("", typed),
    };
    let base: PathBuf =
        if let Some(rest) = folder.strip_prefix('/') { root.unwrap_or(dir).join(rest) } else { dir.join(folder) };
    let entries = std::fs::read_dir(&base).ok()?;
    let mut found: Vec<(String, bool)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // Hidden ones only when asked for.
            if name.starts_with('.') && !segment.starts_with('.') {
                return None;
            }
            Some((name, e.path().is_dir()))
        })
        .take(2000)
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    let start_char = before[..start].chars().count() + folder.chars().count();
    Some(Offer { start: start_char, entries: found })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_in_strings() {
        let dir = std::env::temp_dir().join(format!("rc-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src").join("img")).unwrap();
        std::fs::write(dir.join("src").join("app.js"), "").unwrap();
        std::fs::write(dir.join("src").join(".env"), "").unwrap();
        std::fs::write(dir.join("index.html"), "").unwrap();
        let names =
            |o: Offer| o.entries.into_iter().map(|(n, d)| if d { format!("{n}/") } else { n }).collect::<Vec<_>>();
        // Relative, folders first.
        let o = complete("import { a } from './src/", &dir, None).unwrap();
        assert_eq!(o.start, "import { a } from './src/".chars().count());
        assert_eq!(names(o), ["img/", "app.js"]);
        // Hidden files when the name starts with a dot.
        assert_eq!(names(complete("x = \"./src/.", &dir, None).unwrap()), ["img/", ".env", "app.js"]);
        // An attribute holds a path without ./ ; / is the project's top.
        let o = complete("<img src=\"src/", &dir, None).unwrap();
        assert_eq!(names(o), ["img/", "app.js"]);
        let o = complete("<a href='/", &dir.join("src"), Some(&dir)).unwrap();
        assert_eq!(o.start, "<a href='/".chars().count());
        assert!(names(o).contains(&"index.html".to_owned()));
        assert!(complete("background: url(./src/", &dir, None).is_some());
        // Not paths: plain strings, closed strings, addresses.
        assert_eq!(complete("let s = \"hello", &dir, None), None);
        assert_eq!(complete("let s = \"./src\" + ", &dir, None), None);
        assert_eq!(complete("<a href=\"https://", &dir, None), None);
        assert_eq!(complete("x = './nowhere/", &dir, None), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
