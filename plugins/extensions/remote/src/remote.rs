//! A folder on another machine, worked on here: `ssh host tar` brings it into a local mirror,
//! which opens as the project, and each file saved, deleted or renamed in the mirror goes back
//! with ssh. The editor opens no connection itself; `ssh` and `tar` do.

use serde::{Deserialize, Serialize};

/// The file in a mirror's root that says where it comes from (never sent back). The editor
/// reads it too: paths a program there prints stand for the mirror's files.
pub const MARKER: &str = ".coiman-remote.toml";

/// Folders left out of the download: built or downloaded things, rebuilt there.
pub const SKIPPED: &[&str] = &["target", "node_modules", ".venv", "venv", "__pycache__", "build", "dist"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteFolder {
    /// The ssh destination: `user@host`, or a name in `~/.ssh/config`.
    pub host: String,
    /// The folder there.
    pub path: String,
    /// More options for ssh: `-i key`, `-o Port=2222`, `-F file`.
    pub ssh_options: String,
}

/// `s` in single quotes for a POSIX shell.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A command line split into words: quotes group, a backslash escapes a quote or itself.
pub fn split_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quote) {
            ('\\', q) if matches!(chars.peek(), Some('"' | '\'' | '\\')) && q != Some('\'') => {
                word.extend(chars.next());
                in_word = true;
            }
            (q @ ('"' | '\''), None) => {
                quote = Some(q);
                in_word = true;
            }
            (q, Some(open)) if q == open => quote = None,
            (w, None) if w.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            (other, _) => {
                word.push(other);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(word);
    }
    out
}

impl RemoteFolder {
    /// What's missing before it can be opened, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if self.host.trim().is_empty() {
            Some("Name the ssh host (user@host)")
        } else if self.path.trim().is_empty() {
            Some("Name the folder there (/home/me/project)")
        } else {
            None
        }
    }

    fn options(&self) -> Vec<String> {
        let mut args = vec!["-o".to_owned(), "BatchMode=yes".to_owned()];
        args.extend(split_command(self.ssh_options.trim()));
        args
    }

    /// The folder's own name, for the mirror and the window title.
    pub fn name(&self) -> String {
        let p = self.path.trim().trim_end_matches('/');
        p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("root").to_owned()
    }

    /// The mirror's folder, in the extension's own folder: `mirrors/<host>_<name>`.
    pub fn mirror(&self) -> String {
        let safe = |s: &str| {
            s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect::<String>()
        };
        format!("mirrors/{}_{}", safe(self.host.trim()), safe(&self.name()))
    }

    /// `ssh` arguments that write the folder to stdout as a tar stream.
    pub fn download_args(&self) -> Vec<String> {
        let mut args = self.options();
        args.push(self.host.trim().to_owned());
        let excludes: Vec<String> = SKIPPED.iter().map(|d| format!("--exclude={}", quote(&format!("./{d}")))).collect();
        args.push(format!("tar -C {} {} -cf - .", quote(self.path.trim()), excludes.join(" ")));
        args
    }

    /// `ssh` arguments that write stdin to `rel` (with `/`) in the folder there, making its
    /// folders first.
    pub fn upload_args(&self, rel: &str) -> Vec<String> {
        let full = self.full(rel);
        let dir = full.rsplit_once('/').map_or(".", |(d, _)| d).to_owned();
        self.run(format!("mkdir -p {} && cat > {}", quote(&dir), quote(&full)))
    }

    /// The remote path of `rel` (with `/`).
    fn full(&self, rel: &str) -> String {
        format!("{}/{}", self.path.trim().trim_end_matches('/'), rel.trim_start_matches('/'))
    }

    /// `ssh` arguments that run a shell command there.
    fn run(&self, command: String) -> Vec<String> {
        let mut args = self.options();
        args.push(self.host.trim().to_owned());
        args.push(command);
        args
    }

    /// `ssh` arguments that delete files or folders there.
    pub fn delete_args(&self, rels: &[String]) -> Vec<String> {
        let paths: Vec<String> = rels.iter().map(|r| quote(&self.full(r))).collect();
        self.run(format!("rm -rf -- {}", paths.join(" ")))
    }

    /// `ssh` arguments that rename `old` to `new` there.
    pub fn rename_args(&self, old: &str, new: &str) -> Vec<String> {
        self.run(format!("mv -- {} {}", quote(&self.full(old)), quote(&self.full(new))))
    }

    /// `ssh` arguments for a terminal in the folder there: an interactive shell, or `command` run
    /// there (then the shell stays, so what it printed can be read and more typed). MSYS2's
    /// shell goes home unless `CHERE_INVOKING` is set; elsewhere it means nothing.
    pub fn terminal_args(&self, command: Option<&str>) -> Vec<String> {
        let mut args = self.options();
        args.extend(["-t".to_owned(), self.host.trim().to_owned()]);
        let cd = format!("cd {}", quote(self.path.trim()));
        args.push(match command.map(str::trim).filter(|c| !c.is_empty()) {
            Some(c) => format!("{cd} && {c}; CHERE_INVOKING=1 exec \"${{SHELL:-sh}}\" -i"),
            None => format!("{cd} && CHERE_INVOKING=1 exec \"${{SHELL:-sh}}\" -i"),
        });
        args
    }

    /// The marker's text.
    pub fn marker(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The remote folder a marker's text stands for.
    pub fn from_marker(text: &str) -> Option<Self> {
        toml::from_str(text).ok()
    }

    /// Whether a file of the mirror (relative, with `/`) goes back: not the marker, not a
    /// skipped folder, not Git's.
    pub fn goes_back(rel: &str) -> bool {
        let rel = rel.replace('\\', "/");
        let first = rel.split('/').next().unwrap_or_default();
        !rel.is_empty() && rel != MARKER && !SKIPPED.contains(&first) && first != ".git"
    }
}

/// The hosts an ssh config names (`Host a b`), patterns (`*`, `?`, `!`) left out, each once, in
/// order.
pub fn ssh_hosts(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, rest)) = line.split_once(|c: char| c.is_whitespace() || c == '=') else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').split_whitespace() {
            let name = name.trim_matches('"');
            if !name.is_empty() && !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> RemoteFolder {
        RemoteFolder { host: "pi@board".into(), path: "/home/pi/my app/".into(), ssh_options: "-p 2222".into() }
    }

    #[test]
    fn commands_for_ssh() {
        let r = board();
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(r.name(), "my app");
        let down = r.download_args();
        assert_eq!(down[..5], ["-o", "BatchMode=yes", "-p", "2222", "pi@board"]);
        assert!(down[5].starts_with("tar -C '/home/pi/my app/' --exclude='./target'"), "{}", down[5]);
        assert!(down[5].ends_with(" -cf - ."));
        let up = r.upload_args("src/main.rs");
        assert_eq!(up[5], "mkdir -p '/home/pi/my app/src' && cat > '/home/pi/my app/src/main.rs'");
        assert_eq!(r.mirror(), "mirrors/pi_board_my_app");
        assert_eq!(
            r.delete_args(&["a.txt".into(), "old dir".into()])[5],
            "rm -rf -- '/home/pi/my app/a.txt' '/home/pi/my app/old dir'"
        );
        assert_eq!(
            r.rename_args("src/a.rs", "src/b.rs")[5],
            "mv -- '/home/pi/my app/src/a.rs' '/home/pi/my app/src/b.rs'"
        );
        let t = r.terminal_args(Some("cargo build"));
        assert_eq!(t[4..6], ["-t", "pi@board"]);
        assert_eq!(t[6], "cd '/home/pi/my app/' && cargo build; CHERE_INVOKING=1 exec \"${SHELL:-sh}\" -i");
        assert_eq!(r.terminal_args(None)[6], "cd '/home/pi/my app/' && CHERE_INVOKING=1 exec \"${SHELL:-sh}\" -i");
        assert_eq!(RemoteFolder::default().problem(), Some("Name the ssh host (user@host)"));
        assert_eq!(r.problem(), None);
        assert_eq!(split_command(r#"-i "C:/my keys/id" -o Port=2"#), ["-i", "C:/my keys/id", "-o", "Port=2"]);
    }

    #[test]
    fn marker_hosts_and_what_goes_back() {
        assert_eq!(RemoteFolder::from_marker(&board().marker()), Some(board()));
        assert!(RemoteFolder::goes_back("src/a.rs"));
        assert!(!RemoteFolder::goes_back(MARKER) && !RemoteFolder::goes_back("target/x"));
        assert!(!RemoteFolder::goes_back(".git/config") && !RemoteFolder::goes_back("node_modules\\a"));
        let text = "# boards\nHost pi board\n  HostName 10.0.0.5\nHost *.lan\nHost=build\n\nhost \"nas\" pi\nMatch host x\nHost !bad ok?\n";
        assert_eq!(ssh_hosts(text), ["pi", "board", "build", "nas"]);
        assert!(ssh_hosts("").is_empty());
    }
}
