//! What extensions are told about the terminals: the command line after a shell's prompt
//! (caught when Enter is pressed, or Ctrl+Space asks for suggestions), the folder the prompt
//! names, and the shells' own history files.

use std::path::{Path, PathBuf};

/// Splits a terminal line into the shell's prompt and the command after it. `None` when no
/// prompt shows, as in a program's output.
pub fn strip_prompt(line: &str) -> Option<(String, String)> {
    let line = line.trim_end();
    let split_at = |i: usize, len: usize| Some((line[..i + len].to_owned(), line[i + len..].trim().to_owned()));
    // PowerShell: "PS C:\folder> command".
    if let Some(rest) = line.strip_prefix("PS ")
        && let Some(i) = rest.find("> ")
    {
        return split_at(i + 3, 2);
    }
    if line.starts_with("PS ") && line.ends_with('>') {
        return Some((line.to_owned(), String::new()));
    }
    // cmd.exe: "C:\folder>command".
    let b = line.as_bytes();
    if b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'>')
        && let Some(i) = line.find('>')
    {
        return split_at(i, 1);
    }
    // Unix shells: the first "$ ", "# ", "% ", "> ", "❯ ", "➜ " or "λ ".
    let ends = ["$ ", "# ", "% ", "> ", "❯ ", "➜ ", "λ "];
    let first = ends.iter().filter_map(|e| line.find(e).map(|i| (i, e.len()))).min_by_key(|&(i, _)| i);
    if let Some((i, len)) = first {
        // A prompt is short; a "> " far along the line is a redirection in output.
        if line[..i].chars().count() <= 120 {
            return split_at(i, len);
        }
    }
    let bare = ["$", "#", "%", ">", "❯"];
    if bare.iter().any(|e| line.ends_with(e)) && line.chars().count() <= 120 {
        return Some((line.to_owned(), String::new()));
    }
    None
}

/// The folder a prompt shows, when it shows one: PowerShell's, cmd's, or `user@host:dir`.
pub fn prompt_dir(prompt: &str, home: Option<&Path>) -> Option<PathBuf> {
    let p = prompt.trim_end();
    if let Some(rest) = p.strip_prefix("PS ") {
        return Some(PathBuf::from(rest.trim_end_matches('>').trim()));
    }
    let b = p.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && p.ends_with('>') {
        return Some(PathBuf::from(p.trim_end_matches('>')));
    }
    let (_, dir) = p.split_once('@')?.1.split_once(':')?;
    let dir = dir.trim_end_matches(['$', '#', '%', ' ']).trim();
    if dir == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = dir.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    dir.starts_with('/').then(|| PathBuf::from(dir))
}

/// The text of the shells' own history files (PowerShell, bash, zsh, fish), newest last,
/// each command once.
pub fn shell_history(home: Option<&Path>) -> Vec<String> {
    let mut files: Vec<(PathBuf, u8)> = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let ps = PathBuf::from(appdata).join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt");
        files.push((ps, b'p'));
    }
    if let Some(h) = home {
        files.push((h.join(".local/share/powershell/PSReadLine/ConsoleHost_history.txt"), b'p'));
        files.push((h.join(".bash_history"), b'b'));
        files.push((h.join(".zsh_history"), b'z'));
        files.push((h.join(".local/share/fish/fish_history"), b'f'));
    }
    let mut out: Vec<String> = Vec::new();
    for (path, kind) in files {
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().skip(lines.len().saturating_sub(2000)) {
            let command = match kind {
                // ": 1700000000:0;command"
                b'z' => line.split_once(';').filter(|_| line.starts_with(": ")).map_or(*line, |(_, c)| c),
                b'f' => match line.strip_prefix("- cmd: ") {
                    Some(c) => c,
                    None => continue,
                },
                _ => line,
            };
            let command = command.trim();
            // PowerShell and bash continue a line with ` or \ at its end; those pieces are skipped.
            if command.is_empty() || command.ends_with('`') || command.starts_with('#') {
                continue;
            }
            out.push(command.to_owned());
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut unique: Vec<String> = out.into_iter().rev().filter(|c| seen.insert(c.clone())).collect();
    unique.reverse();
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_come_off() {
        let s = |l: &str| strip_prompt(l).map(|(_, c)| c);
        assert_eq!(s(r"PS C:\Users\me\app> cargo build --release"), Some("cargo build --release".into()));
        assert_eq!(s(r"C:\work>dir /b"), Some("dir /b".into()));
        assert_eq!(s("me@box:~/app$ ls -la > out.txt"), Some("ls -la > out.txt".into()));
        assert_eq!(s("$ make test"), Some("make test".into()));
        assert_eq!(s("❯ just build"), Some("just build".into()));
        assert_eq!(s("root@box:/srv# systemctl status"), Some("systemctl status".into()));
        assert_eq!(s(r"PS C:\x>"), Some(String::new()));
        assert_eq!(s("   Compiling serde v1.0"), None);
        let (p, _) = strip_prompt(r"PS C:\Users\me\app> x").unwrap();
        assert_eq!(prompt_dir(&p, None), Some(PathBuf::from(r"C:\Users\me\app")));
        let (p, _) = strip_prompt("me@box:~/app$ x").unwrap();
        assert_eq!(prompt_dir(&p, Some(Path::new("/home/me"))), Some(PathBuf::from("/home/me/app")));
        let (p, _) = strip_prompt(r"C:\work>x").unwrap();
        assert_eq!(prompt_dir(&p, None), Some(PathBuf::from(r"C:\work")));
    }

    #[test]
    fn shell_history_files_are_read() {
        let home = std::env::temp_dir().join(format!("coiman-shellhist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".local/share/fish")).unwrap();
        let write = |file: &str, text: &str| std::fs::write(home.join(file), text).unwrap();
        write(".bash_history", "ls -la\nmake build\nls -la\n");
        write(".zsh_history", ": 1700000000:0;git status\n");
        write(".local/share/fish/fish_history", "- cmd: just serve\n  when: 1700000000\n");
        let all = shell_history(Some(&home));
        let ours: Vec<&str> = all
            .iter()
            .map(String::as_str)
            .filter(|c| ["ls -la", "make build", "git status", "just serve"].contains(c))
            .collect();
        assert_eq!(ours, ["make build", "ls -la", "git status", "just serve"]);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
