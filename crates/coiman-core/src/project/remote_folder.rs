//! A folder on another machine, worked on here through the Remote over SSH extension, which
//! keeps a mirror of it and opens the mirror as the project. The mirror's marker says where it
//! comes from, so the editor can show a path a program there prints as the mirror's file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file in a mirror's root that says where it comes from.
pub const MARKER: &str = ".coiman-remote.toml";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteFolder {
    /// The ssh destination: `user@host`, or a name in `~/.ssh/config`.
    pub host: String,
    /// The folder there.
    pub path: String,
    /// More options for ssh.
    pub ssh_options: String,
}

impl RemoteFolder {
    /// The remote folder a mirror stands for, if `root` is one.
    pub fn of_mirror(root: &Path) -> Option<Self> {
        toml::from_str(&std::fs::read_to_string(root.join(MARKER)).ok()?).ok()
    }

    /// `file` as written by a program there (`/home/pi/app/src/a.rs`), as the file in the
    /// mirror at `root`; None when it isn't inside the remote folder.
    pub fn local_path(&self, root: &Path, file: &str) -> Option<PathBuf> {
        let base = self.path.trim().trim_end_matches('/');
        let rest = file.strip_prefix(base)?.strip_prefix('/')?;
        Some(rest.split('/').filter(|s| !s.is_empty()).fold(root.to_path_buf(), |p, s| p.join(s)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_and_remote_paths() {
        let dir = std::env::temp_dir().join(format!("coiman-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(RemoteFolder::of_mirror(&dir).is_none());
        std::fs::write(dir.join(MARKER), "host = \"pi@board\"\npath = \"/home/pi/my app/\"\nssh_options = \"\"\n")
            .unwrap();
        let r = RemoteFolder::of_mirror(&dir).unwrap();
        assert_eq!(r.host, "pi@board");
        let local = r.local_path(Path::new("/m"), "/home/pi/my app/src/main.rs").unwrap();
        assert_eq!(local, Path::new("/m").join("src").join("main.rs"));
        assert!(r.local_path(Path::new("/m"), "/home/pi/other/x.rs").is_none());
        assert!(r.local_path(Path::new("/m"), "/home/pi/my apple/x.rs").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
