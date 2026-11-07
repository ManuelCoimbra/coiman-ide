//! Golden-file tests for the Vim and Helix modes (`tests/golden/*.txt`).
//!
//! Every Vim case runs through Rust Code's engine and, when Vim is installed
//! (on PATH, in Git for Windows, or at `RUST_CODE_VIM`), through real Vim with
//! `vim -u NONE`, so the expected results are checked against Vim itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use coiman_core::editor::buffer::{Buffer, Selection};
use coiman_core::editor::vim::{Flavor, Vim, keys_to_vim, parse_keys};

struct Case {
    name: String,
    input: String,
    keys: String,
    expected: String,
}

fn cases(src: &str) -> Vec<Case> {
    let mut out = Vec::new();
    let mut lines = src.lines().peekable();
    while let Some(line) = lines.next() {
        let Some(name) = line.strip_prefix("=== ") else { continue };
        let mut input = Vec::new();
        let mut keys = String::new();
        for l in lines.by_ref() {
            if let Some(k) = l.strip_prefix("--- ") {
                keys = k.to_owned();
                break;
            }
            input.push(l);
        }
        let mut expected = Vec::new();
        while let Some(l) = lines.peek() {
            if l.starts_with("=== ") {
                break;
            }
            expected.push(*l);
            lines.next();
        }
        while expected.last().is_some_and(|l| l.is_empty()) {
            expected.pop();
        }
        out.push(Case {
            name: name.to_owned(),
            input: input.join("\n") + "\n",
            keys,
            expected: expected.join("\n") + "\n",
        });
    }
    out
}

/// Text without its `|` marker, and the marker's char position.
fn split_cursor(marked: &str) -> (String, usize) {
    let pos = marked.chars().position(|c| c == '|').expect("a | cursor marker");
    (marked.replacen('|', "", 1), pos)
}

fn mark_cursor(text: &str, pos: usize) -> String {
    let mut s: String = text.chars().take(pos).collect();
    s.push('|');
    s.extend(text.chars().skip(pos));
    s
}

fn mark_selection(text: &str, start: usize, end_inclusive: usize) -> String {
    let mut s = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == start {
            s.push('[');
        }
        s.push(c);
        if i == end_inclusive {
            s.push(']');
        }
    }
    s
}

fn run_engine(flavor: Flavor, case: &Case) -> Buffer {
    let (text, cursor) = split_cursor(&case.input);
    let mut buf = Buffer::new(&text);
    buf.set_single(Selection::cursor(cursor));
    let mut vim = Vim::new(flavor);
    vim.autoindent = false;
    vim.reset(&mut buf);
    for key in parse_keys(&case.keys.replace("<sync>", "")) {
        vim.handle(&mut buf, key);
    }
    buf
}

fn find_vim() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("RUST_CODE_VIM").map(PathBuf::from),
        Some(PathBuf::from("vim")),
        Some(PathBuf::from(r"C:\Program Files\Git\usr\bin\vim.exe")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|v| Command::new(v).arg("--version").output().is_ok_and(|o| o.status.success()))
}

/// Runs the case in real Vim; returns the text and the cursor marked with |.
fn run_real_vim(vim: &Path, dir: &Path, n: usize, case: &Case) -> Result<String, String> {
    let (text, cursor) = split_cursor(&case.input);
    let file = dir.join(format!("case{n}.txt"));
    let pos_file = dir.join(format!("case{n}.pos"));
    std::fs::write(&file, &text).map_err(|e| e.to_string())?;
    let before: String = text.chars().take(cursor).collect();
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().unwrap_or("").len() + 1;
    let pos_path = pos_file.to_string_lossy().replace('\\', "/");
    let mut cmd = Command::new(vim);
    cmd.args(["-N", "-u", "NONE", "-i", "NONE", "-n", "-Es"]).arg("-c").arg(format!("call cursor({line}, {col})"));
    // One `:normal!` per <sync> part: each is its own undo step in Vim.
    for (i, part) in case.keys.split("<sync>").enumerate() {
        if i > 0 {
            // Setting 'undolevels' to itself makes an undo break (`:help undo-break`).
            cmd.arg("-c").arg("let &undolevels = &undolevels");
        }
        cmd.arg("-c").arg(format!("exe \"normal! {}\"", keys_to_vim(&parse_keys(part))));
    }
    let status = cmd
        .arg("-c")
        .arg(format!("call writefile([line('.') . ',' . col('.')], '{pos_path}')"))
        .arg("-c")
        .arg("wq")
        .arg(&file)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("vim exited with {status}"));
    }
    let out = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let pos = std::fs::read_to_string(&pos_file).map_err(|e| e.to_string())?;
    let (l, c) = pos.trim().split_once(',').ok_or("bad position")?;
    let (l, c): (usize, usize) = (l.parse().map_err(|_| "line")?, c.parse().map_err(|_| "col")?);
    // Vim's column is in bytes; turn it into a char position.
    let line_start: usize = out.split('\n').take(l - 1).map(|s| s.chars().count() + 1).sum();
    let line_text = out.split('\n').nth(l - 1).unwrap_or("");
    let byte = (c - 1).min(line_text.len());
    let char_col = line_text[..line_text.floor_char_boundary(byte)].chars().count();
    Ok(mark_cursor(&out, line_start + char_col))
}

#[test]
fn vim_golden() {
    let all = cases(include_str!("golden/vim.txt"));
    assert!(all.len() > 40, "cases parsed: {}", all.len());
    let mut failures = Vec::new();
    for case in &all {
        let buf = run_engine(Flavor::Vim, case);
        let got = mark_cursor(&buf.text(), buf.primary().head);
        if got != case.expected {
            failures.push(format!(
                "{}\n  keys:     {}\n  expected: {:?}\n  got:      {:?}",
                case.name, case.keys, case.expected, got
            ));
        }
    }

    // The expectations themselves must match real Vim.
    match find_vim() {
        Some(vim) => {
            let dir = std::env::temp_dir().join(format!("rust-code-vim-golden-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            for (n, case) in all.iter().enumerate() {
                match run_real_vim(&vim, &dir, n, case) {
                    Ok(real) if real == case.expected => {}
                    Ok(real) => failures.push(format!(
                        "{} (real Vim disagrees with the golden file)\n  keys:     {}\n  expected: {:?}\n  vim:      {:?}",
                        case.name, case.keys, case.expected, real
                    )),
                    Err(e) => failures.push(format!("{}: could not run Vim: {e}", case.name)),
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
        None => eprintln!("Vim not found: golden expectations not cross-checked (set RUST_CODE_VIM)"),
    }
    assert!(failures.is_empty(), "{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
}

#[test]
fn helix_golden() {
    let all = cases(include_str!("golden/helix.txt"));
    assert!(all.len() > 15);
    let mut failures = Vec::new();
    for case in &all {
        let buf = run_engine(Flavor::Helix, case);
        let sel = buf.primary();
        let (s, e) = (sel.start(), sel.end().max(sel.start() + 1) - 1);
        let got = mark_selection(&buf.text(), s, e);
        if got != case.expected {
            failures.push(format!(
                "{}\n  keys:     {}\n  expected: {:?}\n  got:      {:?}",
                case.name, case.keys, case.expected, got
            ));
        }
    }
    assert!(failures.is_empty(), "{} failure(s):\n\n{}", failures.len(), failures.join("\n\n"));
}
