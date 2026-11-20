//! A folder on another machine, worked on here: `ssh host tar` brings it into a local mirror,
//! which opens as the project, and each file saved in the mirror goes back with ssh. Coiman
//! opens no connection itself; `ssh` and `tar` do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file in a mirror's root that says where it comes from (never sent back).
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
        args.extend(crate::build::compile_db::split_command(self.ssh_options.trim()));
        args
    }

    /// The folder's own name, for the mirror and the window title.
    pub fn name(&self) -> String {
        let p = self.path.trim().trim_end_matches('/');
        p.rsplit('/').next().filter(|n| !n.is_empty()).unwrap_or("root").to_owned()
    }

    /// Where the mirror is kept: `<config>/remote/<host>_<name>`.
    pub fn mirror_dir(&self, config: &Path) -> PathBuf {
        let safe = |s: &str| {
            s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect::<String>()
        };
        config.join("remote").join(format!("{}_{}", safe(self.host.trim()), safe(&self.name())))
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

    /// Writes the marker into a mirror.
    pub fn mark(&self, mirror: &Path) -> Result<(), String> {
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(mirror.join(MARKER), text).map_err(|e| e.to_string())
    }

    /// The remote folder a mirror stands for, if `root` is one.
    pub fn of_mirror(root: &Path) -> Option<Self> {
        toml::from_str(&std::fs::read_to_string(root.join(MARKER)).ok()?).ok()
    }

    /// The relative path to send for `file` saved under `root`, if it should go (not the marker,
    /// not a skipped folder).
    pub fn relative(root: &Path, file: &Path) -> Option<String> {
        let rel = file.strip_prefix(root).ok()?;
        let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        let first = rel.split('/').next().unwrap_or_default();
        (rel != MARKER && !SKIPPED.contains(&first) && first != ".git").then_some(rel)
    }
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
        assert!(r.mirror_dir(Path::new("/c")).ends_with("remote/pi_board_my_app"));
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
    }

    #[test]
    fn marker_and_what_goes_back() {
        let dir = std::env::temp_dir().join(format!("coiman-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(RemoteFolder::of_mirror(&dir).is_none());
        board().mark(&dir).unwrap();
        assert_eq!(RemoteFolder::of_mirror(&dir).unwrap(), board());
        assert_eq!(RemoteFolder::relative(&dir, &dir.join("src").join("a.rs")).as_deref(), Some("src/a.rs"));
        assert!(RemoteFolder::relative(&dir, &dir.join(MARKER)).is_none());
        assert!(RemoteFolder::relative(&dir, &dir.join("target").join("x")).is_none());
        assert!(RemoteFolder::relative(Path::new("/elsewhere"), &dir.join("a")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
