//! Code written twice: runs of lines that appear again elsewhere, whitespace aside. Blank
//! lines, comments and lines of only brackets don't count, so neither do they break a run.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::language::syntax::Lang;

/// A run needs this many lines that count to be reported.
pub const MIN_LINES: usize = 6;
const MAX_BYTES: u64 = 512 * 1024;

/// One run of lines and where it is again.
#[derive(Clone, Debug, PartialEq)]
pub struct Duplicate {
    /// Lines that count, in the run.
    pub lines: usize,
    /// Each place: path relative to the project, first and last line (1-based).
    pub places: Vec<(String, usize, usize)>,
}

/// A line that counts: its text, whitespace squeezed, and where it is.
struct Line {
    text: String,
    at: usize,
}

/// The lines of a file that count.
fn significant(text: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut in_block_comment = false;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if in_block_comment {
            if line.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if line.starts_with("/*") {
            in_block_comment = !line.contains("*/");
            continue;
        }
        let comment = ["//", "#", "--", "*", "<!--"].iter().any(|c| line.starts_with(c));
        let only_brackets = line.chars().all(|c| "{}()[];,".contains(c));
        // Imports are alike by nature.
        let import = ["use ", "import ", "from ", "#include", "package "].iter().any(|p| line.starts_with(p));
        if line.is_empty() || comment || only_brackets || import {
            continue;
        }
        out.push(Line { text: line.split_whitespace().collect::<Vec<_>>().join(" "), at: i + 1 });
    }
    out
}

fn hash_of(lines: &[Line]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for l in lines {
        l.text.hash(&mut h);
    }
    h.finish()
}

/// Runs of at least [`MIN_LINES`] lines found in two places or more, among `files`
/// (path, text). Longest first.
pub fn find(files: &[(String, String)]) -> Vec<Duplicate> {
    let lines: Vec<Vec<Line>> = files.iter().map(|(_, text)| significant(text)).collect();
    // Every window of MIN_LINES lines, by its hash: (file, first line's index).
    let mut windows: HashMap<u64, Vec<(usize, usize)>> = HashMap::new();
    for (f, file_lines) in lines.iter().enumerate() {
        for start in 0..file_lines.len().saturating_sub(MIN_LINES - 1) {
            windows.entry(hash_of(&file_lines[start..start + MIN_LINES])).or_default().push((f, start));
        }
    }
    // A window seen twice starts a run: grow it while the lines keep matching.
    let mut covered: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut starts: Vec<&Vec<(usize, usize)>> = windows.values().filter(|v| v.len() > 1).collect();
    starts.sort_by_key(|v| v[0]);
    for places in starts {
        if places.iter().all(|p| covered.contains(p)) {
            continue;
        }
        // Different windows can share a hash: keep the places that truly match the first.
        let (f0, s0) = places[0];
        let same = |&(f, s): &(usize, usize)| (0..MIN_LINES).all(|k| lines[f][s + k].text == lines[f0][s0 + k].text);
        let mut places: Vec<(usize, usize)> = places.iter().copied().filter(same).collect();
        // A run overlapping itself (repeated lines in one file) isn't a copy.
        places.dedup_by(|a, b| a.0 == b.0 && a.1 < b.1 + MIN_LINES);
        if places.len() < 2 {
            continue;
        }
        let mut len = MIN_LINES;
        loop {
            let next = |&(f, s): &(usize, usize)| lines[f].get(s + len).map(|l| l.text.as_str());
            let first = next(&places[0]);
            if first.is_none() || !places.iter().all(|p| next(p) == first) {
                break;
            }
            len += 1;
        }
        for &(f, s) in &places {
            for k in 0..=len - MIN_LINES {
                covered.insert((f, s + k));
            }
        }
        let spots =
            places.iter().map(|&(f, s)| (files[f].0.clone(), lines[f][s].at, lines[f][s + len - 1].at)).collect();
        out.push(Duplicate { lines: len, places: spots });
    }
    out.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.places.cmp(&b.places)));
    out
}

/// The project's source files, read, then [`find`].
pub fn scan(root: &Path) -> Vec<Duplicate> {
    let mut files = Vec::new();
    for entry in super::project_files(root, false) {
        let path = entry.path();
        if !entry.file_type().is_some_and(|t| t.is_file()) || entry.metadata().map_or(true, |m| m.len() > MAX_BYTES) {
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        // Code only: data and docs repeat themselves by nature.
        let code = Lang::from_extension(&ext).is_some_and(|l| {
            !matches!(l, Lang::Json | Lang::Toml | Lang::Yaml | Lang::Markdown | Lang::Html | Lang::Css)
        });
        if !code {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        files.push((rel, text));
    }
    find(&files)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "let a = load(path);\nlet b = parse(a);\nif b.ok() {\n    save(b);\n    log(\"saved\");\n    count += 1;\n    notify(user);\n}\n";

    #[test]
    fn a_block_in_two_files_is_found_whole() {
        let one = format!("fn one() {{\n{BLOCK}}}\n");
        let two = format!("fn two() {{\n    // the same again\n{}}}\n", BLOCK.replace('\n', "\n  "));
        let found = find(&[("one.rs".to_owned(), one), ("two.rs".to_owned(), two)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lines, 7);
        assert_eq!(found[0].places, [("one.rs".to_owned(), 2, 8), ("two.rs".to_owned(), 3, 9)]);
    }

    #[test]
    fn short_runs_imports_and_blank_lines_dont_count() {
        let short = "let a = 1;\nlet b = 2;\nlet c = 3;\n";
        let imports = "use a::b;\nuse c::d;\nuse e::f;\nuse g::h;\nuse i::j;\nuse k::l;\nuse m::n;\n";
        let files =
            [("a.rs".to_owned(), format!("{short}{imports}")), ("b.rs".to_owned(), format!("{short}\n\n{imports}"))];
        assert!(find(&files).is_empty());
    }

    #[test]
    fn the_same_file_can_repeat_itself() {
        let text = format!("fn a() {{\n{BLOCK}}}\nfn b() {{\n{BLOCK}}}\n");
        let found = find(&[("x.rs".to_owned(), text)]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].places.len(), 2);
    }
}
