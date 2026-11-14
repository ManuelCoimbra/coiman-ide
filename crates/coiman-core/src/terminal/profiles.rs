//! Terminal profiles: the shells a new terminal can start, found on the computer and added
//! by the user (`[[terminal.profiles]]`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A shell to start: a name for the menu and a command line (quotes allowed).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub command: String,
}

impl Profile {
    fn new(name: &str, command: String) -> Self {
        Self { name: name.to_owned(), command }
    }
}

/// A command line naming `program`, quoted when its path has spaces.
fn line(program: &Path, args: &str) -> String {
    let p = program.display().to_string();
    let p = if p.contains(' ') { format!("\"{p}\"") } else { p };
    if args.is_empty() { p } else { format!("{p} {args}") }
}

/// The shells on this computer.
pub fn detected() -> Vec<Profile> {
    let mut out = Vec::new();
    if cfg!(windows) {
        let system = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32"));
        out.push(Profile::new("PowerShell", "powershell.exe".to_owned()));
        if let Some(pwsh) = crate::language::lsp::servers::which("pwsh") {
            out.push(Profile::new("PowerShell 7", line(&pwsh, "")));
        }
        out.push(Profile::new("Command Prompt", "cmd.exe".to_owned()));
        let programs =
            std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or(PathBuf::from(r"C:\Program Files"));
        let git_bash = programs.join("Git").join("bin").join("bash.exe");
        if git_bash.is_file() {
            out.push(Profile::new("Git Bash", line(&git_bash, "--login -i")));
        }
        let msys = PathBuf::from(r"C:\msys64\msys2_shell.cmd");
        if msys.is_file() {
            out.push(Profile::new("MSYS2 UCRT64", line(&msys, "-defterm -here -no-start -ucrt64")));
        }
        if system.map(|s| s.join("wsl.exe")).is_some_and(|w| w.is_file()) {
            out.push(Profile::new("WSL", "wsl.exe".to_owned()));
        }
    } else {
        let mut seen: Vec<String> = Vec::new();
        let login = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
        let listed = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let shells = login.into_iter().chain(listed.lines().map(str::trim).map(str::to_owned));
        for shell in shells.filter(|s| s.starts_with('/') && Path::new(s).is_file()) {
            let name = Path::new(&shell).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // `/bin/bash` and `/usr/bin/bash` are the same shell to a person.
            if name.is_empty() || seen.contains(&name) || matches!(name.as_str(), "nologin" | "false" | "git-shell") {
                continue;
            }
            seen.push(name.clone());
            out.push(Profile::new(&name, shell));
        }
    }
    out
}

/// The user's profiles first, then the shells found that they don't name again.
pub fn all(user: &[Profile]) -> Vec<Profile> {
    let mut out: Vec<Profile> =
        user.iter().filter(|p| !p.name.trim().is_empty() && !p.command.trim().is_empty()).cloned().collect();
    for found in detected() {
        if !out.iter().any(|p| p.name.eq_ignore_ascii_case(&found.name)) {
            out.push(found);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_profiles_come_first_and_win_by_name() {
        let found = detected();
        assert!(!found.is_empty(), "every computer has a shell");
        let first = found[0].name.clone();
        let mine = vec![
            Profile::new("Mine", "my-shell --fancy".to_owned()),
            Profile::new(&first.to_uppercase(), "other".to_owned()),
            Profile::new("", "nameless".to_owned()),
        ];
        let all = all(&mine);
        assert_eq!(all[0].name, "Mine");
        assert_eq!(all.iter().filter(|p| p.name.eq_ignore_ascii_case(&first)).count(), 1, "one per name");
        assert!(!all.iter().any(|p| p.command == "nameless"));
        assert_eq!(
            line(Path::new(r"C:\Program Files\Git\bin\bash.exe"), "-i"),
            r#""C:\Program Files\Git\bin\bash.exe" -i"#
        );
    }
}
