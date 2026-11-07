//! Line-oriented editing commands modelled on VS Code's built-in editor actions
//! (`editor.action.*`). Each operation takes the text and a selection as char
//! indices and returns the new text and selection.

/// A selection as `(anchor, cursor)` char indices. They may be in either order.
pub type Sel = (usize, usize);

pub struct Edit {
    pub text: String,
    pub sel: Sel,
}

/// Char offsets where each line starts, plus a final entry for the end of text.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let mut idx = 0;
    for c in text.chars() {
        idx += 1;
        if c == '\n' {
            starts.push(idx);
        }
    }
    starts
}

fn line_of(starts: &[usize], char_idx: usize) -> usize {
    match starts.binary_search(&char_idx) {
        Ok(i) => i,
        Err(i) => i - 1,
    }
}

/// Lines as owned strings without their `\n` (a trailing `\r` stays with the line).
fn split_lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_owned).collect()
}

/// Inclusive range of lines touched by the selection. A selection ending at
/// column 0 of a line does not include that line, as in VS Code.
fn selected_lines(text: &str, sel: Sel) -> (usize, usize) {
    let starts = line_starts(text);
    let (lo, hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    let first = line_of(&starts, lo);
    let mut last = line_of(&starts, hi);
    if last > first && starts[last] == hi {
        last -= 1;
    }
    (first, last)
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Char index of the start of `line` within `lines` joined by `\n`.
fn line_start_in(lines: &[String], line: usize) -> usize {
    lines[..line].iter().map(|l| char_len(l) + 1).sum()
}

/// Keeps the selection at the same columns after lines moved by `delta`.
fn shift_sel(text: &str, sel: Sel, lines: &[String], delta: isize) -> Sel {
    let starts = line_starts(text);
    let map = |i: usize| {
        let line = line_of(&starts, i);
        let col = i - starts[line];
        let new_line = (line as isize + delta) as usize;
        line_start_in(lines, new_line) + col.min(char_len(&lines[new_line]))
    };
    (map(sel.0), map(sel.1))
}

/// `editor.action.moveLinesUpAction` / `moveLinesDownAction` (Alt+Up / Alt+Down).
pub fn move_lines(text: &str, sel: Sel, down: bool) -> Option<Edit> {
    let (first, last) = selected_lines(text, sel);
    let mut lines = split_lines(text);
    if down {
        if last + 1 >= lines.len() {
            return None;
        }
        let moved = lines.remove(last + 1);
        lines.insert(first, moved);
    } else {
        if first == 0 {
            return None;
        }
        let moved = lines.remove(first - 1);
        lines.insert(last, moved);
    }
    let sel = shift_sel(text, sel, &lines, if down { 1 } else { -1 });
    Some(Edit { text: lines.join("\n"), sel })
}

/// `editor.action.copyLinesUpAction` / `copyLinesDownAction` (Shift+Alt+Up / Down).
pub fn copy_lines(text: &str, sel: Sel, down: bool) -> Edit {
    let (first, last) = selected_lines(text, sel);
    let mut lines = split_lines(text);
    let block: Vec<String> = lines[first..=last].to_vec();
    let count = block.len();
    for (i, l) in block.into_iter().enumerate() {
        lines.insert(last + 1 + i, l);
    }
    // Copying down selects the new copy; copying up keeps the original (now the upper copy).
    let sel = if down { shift_sel(text, sel, &lines, count as isize) } else { sel };
    Edit { text: lines.join("\n"), sel }
}

/// `editor.action.deleteLines` (Ctrl+Shift+K).
pub fn delete_lines(text: &str, sel: Sel) -> Edit {
    let (first, last) = selected_lines(text, sel);
    let mut lines = split_lines(text);
    let col = {
        let starts = line_starts(text);
        sel.1 - starts[line_of(&starts, sel.1)]
    };
    lines.drain(first..=last);
    if lines.is_empty() {
        lines.push(String::new());
    }
    let line = first.min(lines.len() - 1);
    let pos = line_start_in(&lines, line) + col.min(char_len(&lines[line]));
    Edit { text: lines.join("\n"), sel: (pos, pos) }
}

/// `editor.action.indentLines` / `outdentLines` (Ctrl+] / Ctrl+[).
pub fn indent_lines(text: &str, sel: Sel, outdent: bool, tab: &str) -> Edit {
    let (first, last) = selected_lines(text, sel);
    let starts = line_starts(text);
    let mut lines = split_lines(text);
    // Per-line char delta, used to keep the selection on the same characters.
    let mut deltas = vec![0isize; lines.len()];
    for (i, line) in lines.iter_mut().enumerate().take(last + 1).skip(first) {
        if outdent {
            let n = if line.starts_with('\t') {
                1
            } else {
                line.chars().take(char_len(tab)).take_while(|&c| c == ' ').count()
            };
            line.drain(..n);
            deltas[i] = -(n as isize);
        } else if !line.trim().is_empty() || first == last {
            line.insert_str(0, tab);
            deltas[i] = char_len(tab) as isize;
        }
    }
    let map = |idx: usize| {
        let line = line_of(&starts, idx);
        let col = (idx - starts[line]) as isize;
        let new_col = (col + deltas[line]).max(0) as usize;
        line_start_in(&lines, line) + new_col.min(char_len(&lines[line]))
    };
    Edit { sel: (map(sel.0), map(sel.1)), text: lines.join("\n") }
}

/// Line comment token for a file extension, if the language has one.
pub fn comment_token(ext: &str) -> Option<&'static str> {
    use crate::language::lsp::registry::by_extension;
    if let Some(token) = by_extension(ext, true).and_then(|l| l.comment) {
        return Some(token);
    }
    built_in_comment_token(ext).or_else(|| by_extension(ext, false).and_then(|l| l.comment))
}

fn built_in_comment_token(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" | "c" | "h" | "cpp" | "cc" | "hpp" | "hh" | "hxx" | "cxx" | "c++" | "inl" | "cs" | "java" | "js"
        | "mjs" | "cjs" | "ts" | "mts" | "cts" | "tsx" | "jsx" | "go" | "swift" | "kt" | "kts" | "dart" | "scala"
        | "php" | "scss" | "json" | "jsonc" => "//",
        "py" | "pyw" | "pyi" | "sh" | "bash" | "zsh" | "toml" | "yaml" | "yml" | "ps1" | "rb" | "pl" | "r" | "cfg"
        | "conf" | "ini" | "txt" => "#",
        "sql" | "lua" | "hs" => "--",
        "bat" | "cmd" => "REM",
        _ => return crate::language::languages::line_comment(ext),
    })
}

/// `editor.action.commentLine` (Ctrl+/): comments the lines at the smallest
/// indentation, or uncomments them if every non-blank line is already commented.
pub fn toggle_line_comment(text: &str, sel: Sel, token: &str) -> Edit {
    let (first, last) = selected_lines(text, sel);
    let starts = line_starts(text);
    let mut lines = split_lines(text);
    let range = first..=last;
    let non_blank = |l: &String| !l.trim().is_empty();

    let all_commented = lines[range.clone()].iter().filter(|l| non_blank(l)).all(|l| l.trim_start().starts_with(token))
        && lines[range.clone()].iter().any(non_blank);
    let indent = lines[range.clone()]
        .iter()
        .filter(|l| non_blank(l))
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);

    // (column where the edit happened, chars added (+) or removed (-)) per line.
    let mut edits: Vec<Option<(usize, isize)>> = vec![None; lines.len()];
    for i in range {
        let line = &mut lines[i];
        if all_commented {
            if !non_blank(line) {
                continue;
            }
            let lead = line.chars().take_while(|c| c.is_whitespace()).count();
            let byte = line.char_indices().nth(lead).map_or(line.len(), |(b, _)| b);
            let mut remove = char_len(token);
            if line[byte + token.len()..].starts_with(' ') {
                remove += 1;
            }
            let end = line.char_indices().nth(lead + remove).map_or(line.len(), |(b, _)| b);
            line.replace_range(byte..end, "");
            edits[i] = Some((lead, -(remove as isize)));
        } else if non_blank(line) || first == last {
            let byte = line.char_indices().nth(indent).map_or(line.len(), |(b, _)| b);
            let insert = format!("{token} ");
            line.insert_str(byte, &insert);
            edits[i] = Some((indent, char_len(&insert) as isize));
        }
    }
    let map = |idx: usize| {
        let line = line_of(&starts, idx);
        let col = idx - starts[line];
        let new_col = match edits[line] {
            Some((at, d)) if col >= at => ((col as isize + d).max(at as isize)) as usize,
            _ => col,
        };
        line_start_in(&lines, line) + new_col.min(char_len(&lines[line]))
    };
    Edit { sel: (map(sel.0), map(sel.1)), text: lines.join("\n") }
}

/// Replaces the selected lines (or all lines if the selection is empty and
/// `whole_doc_if_empty`) using `f`, then selects the result.
fn transform_lines(text: &str, sel: Sel, whole_doc_if_empty: bool, f: impl FnOnce(Vec<String>) -> Vec<String>) -> Edit {
    let mut lines = split_lines(text);
    let (first, last) =
        if sel.0 == sel.1 && whole_doc_if_empty { (0, lines.len() - 1) } else { selected_lines(text, sel) };
    let new_block = f(lines[first..=last].to_vec());
    lines.splice(first..=last, new_block.iter().cloned());
    let start = line_start_in(&lines, first);
    let end = start + new_block.iter().map(|l| char_len(l) + 1).sum::<usize>().saturating_sub(1);
    Edit { text: lines.join("\n"), sel: (start, end) }
}

/// `editor.action.sortLinesAscending` / `sortLinesDescending`.
pub fn sort_lines(text: &str, sel: Sel, descending: bool) -> Edit {
    transform_lines(text, sel, true, |mut block| {
        block.sort();
        if descending {
            block.reverse();
        }
        block
    })
}

/// `editor.action.removeDuplicateLines`.
pub fn remove_duplicate_lines(text: &str, sel: Sel) -> Edit {
    transform_lines(text, sel, true, |block| {
        let mut seen = std::collections::HashSet::new();
        block.into_iter().filter(|l| seen.insert(l.clone())).collect()
    })
}

/// `editor.action.reverseLines`.
pub fn reverse_lines(text: &str, sel: Sel) -> Edit {
    transform_lines(text, sel, true, |mut block| {
        block.reverse();
        block
    })
}

/// `editor.action.joinLines`: joins the selected lines (or the next line) with single spaces.
pub fn join_lines(text: &str, sel: Sel) -> Option<Edit> {
    let (first, mut last) = selected_lines(text, sel);
    let mut lines = split_lines(text);
    if first == last {
        last += 1;
    }
    if last >= lines.len() {
        return None;
    }
    let mut joined = lines[first].trim_end().to_owned();
    let mut cursor = 0;
    for l in &lines[first + 1..=last] {
        let t = l.trim();
        cursor = char_len(&joined);
        if !t.is_empty() {
            joined.push(' ');
            joined.push_str(t);
        }
    }
    lines.splice(first..=last, [joined]);
    let pos = line_start_in(&lines, first) + cursor;
    Some(Edit { text: lines.join("\n"), sel: (pos, pos) })
}

/// `editor.action.duplicateSelection`: duplicates the selection, or the line if empty.
pub fn duplicate_selection(text: &str, sel: Sel) -> Edit {
    let (lo, hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    if lo == hi {
        return copy_lines(text, sel, true);
    }
    let chars: Vec<char> = text.chars().collect();
    let piece: String = chars[lo..hi].iter().collect();
    let mut out: String = chars[..hi].iter().collect();
    out.push_str(&piece);
    out.extend(&chars[hi..]);
    Edit { text: out, sel: (hi, hi + (hi - lo)) }
}

pub enum CaseTransform {
    Upper,
    Lower,
    Title,
}

/// `editor.action.transformToUppercase` / `Lowercase` / `Titlecase`. With an empty
/// selection the word under the cursor is transformed, as in VS Code.
pub fn transform_case(text: &str, sel: Sel, mode: CaseTransform) -> Edit {
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (sel.0.min(sel.1), sel.0.max(sel.1));
    if lo == hi {
        (lo, hi) = word_at(&chars, lo);
    }
    let piece: String = chars[lo..hi].iter().collect();
    let replaced = match mode {
        CaseTransform::Upper => piece.to_uppercase(),
        CaseTransform::Lower => piece.to_lowercase(),
        CaseTransform::Title => {
            let mut out = String::new();
            let mut start = true;
            for c in piece.chars() {
                if start && c.is_alphanumeric() {
                    out.extend(c.to_uppercase());
                    start = false;
                } else {
                    out.extend(c.to_lowercase());
                }
                if c.is_whitespace() {
                    start = true;
                }
            }
            out
        }
    };
    let mut out: String = chars[..lo].iter().collect();
    out.push_str(&replaced);
    out.extend(&chars[hi..]);
    let new_hi = lo + char_len(&replaced);
    Edit { text: out, sel: (lo, new_hi) }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `[start, end)` of the word touching `idx`, or `(idx, idx)` if there is none.
pub fn word_at(chars: &[char], idx: usize) -> (usize, usize) {
    let mut lo = idx.min(chars.len());
    let mut hi = lo;
    while lo > 0 && is_word(chars[lo - 1]) {
        lo -= 1;
    }
    while hi < chars.len() && is_word(chars[hi]) {
        hi += 1;
    }
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_line_down_and_up() {
        let e = move_lines("a\nb\nc", (0, 0), true).unwrap();
        assert_eq!(e.text, "b\na\nc");
        assert_eq!(e.sel, (2, 2));
        let e = move_lines(&e.text, e.sel, false).unwrap();
        assert_eq!(e.text, "a\nb\nc");
        assert_eq!(e.sel, (0, 0));
        assert!(move_lines("a\nb", (0, 0), false).is_none());
    }

    #[test]
    fn copy_line_down_selects_copy() {
        let e = copy_lines("ab\ncd", (1, 1), true);
        assert_eq!(e.text, "ab\nab\ncd");
        assert_eq!(e.sel, (4, 4));
    }

    #[test]
    fn delete_line_keeps_column() {
        let e = delete_lines("abc\ndef\nghi", (5, 5));
        assert_eq!(e.text, "abc\nghi");
        assert_eq!(e.sel, (5, 5));
    }

    #[test]
    fn indent_and_outdent() {
        let e = indent_lines("a\nb", (0, 3), false, "    ");
        assert_eq!(e.text, "    a\n    b");
        let e = indent_lines(&e.text, e.sel, true, "    ");
        assert_eq!(e.text, "a\nb");
    }

    #[test]
    fn toggle_comment_round_trip() {
        let src = "fn x() {\n    a();\n\n    b();\n}";
        let sel = (9, 28);
        let e = toggle_line_comment(src, sel, "//");
        assert_eq!(e.text, "fn x() {\n    // a();\n\n    // b();\n}");
        let e = toggle_line_comment(&e.text, e.sel, "//");
        assert_eq!(e.text, src);
    }

    #[test]
    fn sort_and_dedupe() {
        assert_eq!(sort_lines("b\na\nc", (0, 0), false).text, "a\nb\nc");
        assert_eq!(sort_lines("b\na\nc", (0, 0), true).text, "c\nb\na");
        assert_eq!(remove_duplicate_lines("a\nb\na", (0, 0)).text, "a\nb");
    }

    #[test]
    fn join_two_lines() {
        let e = join_lines("foo\n   bar", (0, 0)).unwrap();
        assert_eq!(e.text, "foo bar");
        assert_eq!(e.sel, (3, 3));
    }

    #[test]
    fn case_transforms() {
        assert_eq!(transform_case("hello world", (0, 0), CaseTransform::Upper).text, "HELLO world");
        assert_eq!(transform_case("hello world", (0, 11), CaseTransform::Title).text, "Hello World");
    }
}
