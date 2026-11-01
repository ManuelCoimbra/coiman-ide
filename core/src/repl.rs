//! Sending code to an interactive session in a terminal: which program to start,
//! where a cell begins and ends (`# %%` in a plain file), how to pass many lines,
//! and what came back. It works by typing into the REPL and reading its output, so
//! any language with a REPL can take part, without a kernel or a protocol.

use std::path::{Path, PathBuf};

/// How a REPL takes more than one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// From a file, through a small helper, so that the last value still prints.
    Python,
    /// From a file, with `.load`.
    Node,
    /// Typed line by line.
    Plain,
}

/// A REPL for a kind of file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repl {
    pub title: String,
    pub command: String,
    pub kind: Kind,
}

/// The REPL for a file extension. Languages described as data name their own.
pub fn for_extension(ext: &str) -> Option<Repl> {
    let (title, command, kind) = match ext {
        "py" | "pyw" => ("Python", "python -q -i", Kind::Python),
        "js" | "mjs" | "cjs" => ("Node", "node -i", Kind::Node),
        "r" => ("R", "R --quiet", Kind::Plain),
        "jl" => ("Julia", "julia", Kind::Plain),
        _ => {
            let language = crate::lsp::registry::by_extension(ext, false)?;
            return Some(Repl {
                title: language.name.to_owned(),
                command: language.repl?.to_owned(),
                kind: Kind::Plain,
            });
        }
    };
    Some(Repl { title: title.to_owned(), command: command.to_owned(), kind })
}

/// The helper Python runs a block through: like the REPL, it prints the value of
/// a last expression, which a plain `exec` would lose.
const PYTHON_HELPER: &str = "\
import ast as _rc_ast
def __rc_run(path):
    tree = _rc_ast.parse(open(path, encoding='utf-8').read())
    last = tree.body.pop() if tree.body and isinstance(tree.body[-1], _rc_ast.Expr) else None
    scope = globals()
    exec(compile(tree, path, 'exec'), scope)
    if last is not None:
        value = eval(compile(_rc_ast.Expression(last.value), path, 'eval'), scope)
        if value is not None:
            print(repr(value))
";

/// What to type to give a new Python REPL its helper: one short line, so it
/// doesn't fill the terminal.
pub fn python_setup() -> std::io::Result<String> {
    let file = temp_file("helper.py")?;
    std::fs::write(&file, PYTHON_HELPER)?;
    Ok(format!("exec(open(r\"{}\").read())\r", file.display()))
}

/// Whether a line starts a cell: `# %%`, `// %%`, `-- %%`.
fn is_marker(line: &str) -> bool {
    let line = line.trim_start();
    ["#", "//", "--", "%"]
        .iter()
        .any(|comment| line.strip_prefix(comment).is_some_and(|rest| rest.trim_start().starts_with("%%")))
}

/// The lines (from 0) that start cells.
pub fn cell_lines(source: &str) -> Vec<usize> {
    source.lines().enumerate().filter(|(_, line)| is_marker(line)).map(|(i, _)| i).collect()
}

/// A cell's code, as lines `start..end`, with where the next cell's code starts.
#[derive(Debug, PartialEq, Eq)]
pub struct Cell {
    pub start: usize,
    pub end: usize,
    pub next: Option<usize>,
}

/// The cell holding `line`, when the file has cells.
pub fn cell_at(source: &str, line: usize) -> Option<Cell> {
    let markers = cell_lines(source);
    if markers.is_empty() {
        return None;
    }
    let total = source.lines().count();
    let before = markers.iter().rposition(|m| *m <= line);
    let start = before.map_or(0, |i| markers[i] + 1);
    let after = before.map_or(0, |i| i + 1);
    let end = markers.get(after).copied().unwrap_or(total);
    Some(Cell { start, end, next: markers.get(after).map(|m| m + 1) })
}

/// The last line with code in `start..end`, where an answer belongs.
pub fn last_code_line(source: &str, start: usize, end: usize) -> usize {
    let lines: Vec<&str> = source.lines().collect();
    (start..end.min(lines.len())).rev().find(|i| !lines[*i].trim().is_empty()).unwrap_or(start)
}

/// Next line with code after `line`, for moving on once it's sent.
pub fn next_code_line(source: &str, line: usize) -> Option<usize> {
    source.lines().enumerate().skip(line + 1).find(|(_, l)| !l.trim().is_empty()).map(|(i, _)| i)
}

/// Removes what's directly in `dir` that hasn't changed for `age`: files, or folders
/// going by their contents' folder time. Anything in use is left to fail quietly.
pub fn remove_older_than(dir: &std::path::Path, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(info) = entry.metadata() else { continue };
        let old = info.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|e| e > age);
        if old {
            let path = entry.path();
            let _ = if info.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
        }
    }
}

/// Clears up after earlier sessions: code sent to REPLs, and documents servers supplied.
pub fn clean_old_files() {
    const DAY: u64 = 24 * 60 * 60;
    remove_older_than(&std::env::temp_dir().join("rust-code-repl"), std::time::Duration::from_secs(DAY));
    if let Some(dir) = crate::lsp::convert::virtual_folder() {
        remove_older_than(&dir, std::time::Duration::from_secs(30 * DAY));
    }
}

fn temp_file(ext: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("rust-code-repl");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(format!("block-{}.{ext}", std::process::id())))
}

/// What to type into a REPL so that it runs `code`.
pub fn keystrokes(kind: Kind, code: &str) -> std::io::Result<String> {
    let code = code.trim_end().replace("\r\n", "\n");
    let one_line = !code.contains('\n');
    Ok(match kind {
        _ if one_line => format!("{code}\r"),
        Kind::Plain => format!("{}\r", code.replace('\n', "\r")),
        Kind::Python => {
            let file = temp_file("py")?;
            std::fs::write(&file, &code)?;
            format!("__rc_run(r\"{}\")\r", file.display())
        }
        Kind::Node => {
            let file = temp_file("js")?;
            std::fs::write(&file, &code)?;
            format!(".load {}\r", file.display())
        }
    })
}

/// Whether a line of output is a REPL's prompt, with or without what was typed.
fn is_prompt(line: &str) -> bool {
    let line = line.trim_start();
    line == ">"
        || line == ">>>"
        || line == "..."
        || line == "|"
        || ["> ", ">>> ", "... ", "| "].iter().any(|p| line.starts_with(p))
        || line.starts_with("irb(")
        || line.starts_with("iex(")
        || line.starts_with("julia>")
        || line.starts_with("ghci>")
}

/// Whether the REPL is waiting for input again: its last line is an empty prompt.
pub fn is_ready<S: AsRef<str>>(lines: &[S]) -> bool {
    lines.iter().rev().map(|l| l.as_ref().trim_end()).find(|l| !l.is_empty()).is_some_and(|last| {
        matches!(last, ">" | ">>>" | "julia>" | "ghci>")
            || (last.starts_with("irb(") || last.starts_with("iex(")) && last.ends_with('>')
    })
}

/// What to show beside the code: the last thing the REPL said, shortened. An
/// error's last line is its message, and a value's is the value.
pub fn result_text<S: AsRef<str>>(lines: &[S], sent: &str) -> Option<String> {
    const SHOWN: usize = 80;
    let sent: Vec<&str> = sent.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let last =
        lines.iter().map(|l| l.as_ref().trim()).rfind(|l| !l.is_empty() && !is_prompt(l) && !sent.contains(l))?;
    let short: String = last.chars().take(SHOWN).collect();
    Some(if short.len() < last.len() { format!("{short}…") } else { short })
}

/// The folder a REPL for `file` starts in.
pub fn start_folder(file: &Path, root: Option<&Path>) -> PathBuf {
    root.filter(|r| file.starts_with(r)).or_else(|| file.parent()).unwrap_or(file).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_cells_in_plain_files() {
        let source = "setup = 1\n# %%\na = 1\nb = 2\n# %% [title]\nc = 3\n";
        assert_eq!(cell_lines(source), [1, 4]);
        assert_eq!(cell_at(source, 2), Some(Cell { start: 2, end: 4, next: Some(5) }));
        assert_eq!(cell_at(source, 1), Some(Cell { start: 2, end: 4, next: Some(5) }), "its marker belongs to it");
        assert_eq!(cell_at(source, 5), Some(Cell { start: 5, end: 6, next: None }));
        assert_eq!(cell_at(source, 0), Some(Cell { start: 0, end: 1, next: Some(2) }), "code before the first");
        assert_eq!(cell_at("a = 1\nb = 2\n", 0), None);
        assert_eq!(cell_lines("#%%\n// %% x\n-- %%\n% %%\n# 100%% sure\n#x\n"), [0, 1, 2, 3]);
    }

    #[test]
    fn answers_go_on_the_last_line_with_code() {
        assert_eq!(last_code_line("a\nb\n\n\n# %%\n", 0, 4), 1);
        assert_eq!(last_code_line("\n\n", 0, 2), 0);
    }

    #[test]
    fn moves_to_the_next_code() {
        assert_eq!(next_code_line("a\n\n  \nb\n", 0), Some(3));
        assert_eq!(next_code_line("a\n\n", 0), None);
    }

    #[test]
    fn types_what_each_repl_takes() {
        assert_eq!(keystrokes(Kind::Python, "1 + 1\n").unwrap(), "1 + 1\r");
        assert_eq!(keystrokes(Kind::Plain, "a\nb").unwrap(), "a\rb\r");
        let python = keystrokes(Kind::Python, "a = 1\na").unwrap();
        assert!(python.starts_with("__rc_run(r\"") && python.ends_with("\")\r"), "{python}");
        assert!(keystrokes(Kind::Node, "a\nb").unwrap().starts_with(".load "));
    }

    #[test]
    fn reads_what_came_back() {
        let lines = [">>> 1 + 1", "2", ">>> "];
        assert_eq!(result_text(&lines, "1 + 1").as_deref(), Some("2"));
        assert!(is_ready(&lines));
        assert!(!is_ready(&[">>> 1 + 1", "2"]));
        let error = [
            ">>> 1/0",
            "Traceback (most recent call last):",
            "  File \"<stdin>\", line 1",
            "ZeroDivisionError: division by zero",
            ">>>",
        ];
        assert_eq!(result_text(&error, "1/0").as_deref(), Some("ZeroDivisionError: division by zero"));
        assert_eq!(result_text(&[">>> a = 1", ">>> "], "a = 1"), None, "nothing to show");
        let node = ["> 40 + 2", "42", "> "];
        assert_eq!(result_text(&node, "40 + 2").as_deref(), Some("42"));
        assert!(is_ready(&node));
        let long = "x".repeat(200);
        assert_eq!(result_text(&[long.as_str()], "").unwrap().chars().count(), 81);
    }

    #[test]
    fn knows_the_repls() {
        assert_eq!(for_extension("py").unwrap().kind, Kind::Python);
        assert_eq!(for_extension("js").unwrap().kind, Kind::Node);
        assert_eq!(for_extension("rb").unwrap().command, "irb");
        assert_eq!(for_extension("toml"), None);
    }
}
