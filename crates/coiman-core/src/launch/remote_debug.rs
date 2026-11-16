//! Remote debugging with gdb: a program on another machine, run under `gdbserver`, debugged
//! here. Over ssh, Coiman starts gdbserver itself (`ssh -L` carries its port here); otherwise
//! gdb connects to a gdbserver already listening. gdb reads symbols from a copy of the program
//! on this machine. Nothing here opens a connection: `ssh`, `scp` and gdb do.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Where and what to debug, as last filled in for a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteTarget {
    /// Start gdbserver over ssh (true), or connect to one already running (false).
    pub over_ssh: bool,
    /// Over ssh: the ssh destination (`user@host`, or a name in `~/.ssh/config`). Connecting:
    /// the host gdbserver listens on.
    pub host: String,
    /// The port gdbserver listens on.
    pub port: u16,
    /// Over ssh: the port here that leads to it (0: the same number).
    pub local_port: u16,
    /// Over ssh: the program on the remote machine, and its arguments.
    pub remote_program: String,
    pub args: String,
    /// Over ssh: gdbserver's name or path there.
    pub gdbserver: String,
    /// The same program on this machine, for its symbols.
    pub local_program: String,
    /// Over ssh: copy the local program to `remote_program` with scp first.
    pub copy_first: bool,
    /// More options for ssh and scp, as typed: `-F file`, `-i key`, `-o Port=2222`.
    pub ssh_options: String,
}

impl Default for RemoteTarget {
    fn default() -> Self {
        Self {
            over_ssh: true,
            host: String::new(),
            port: 2345,
            local_port: 0,
            remote_program: String::new(),
            args: String::new(),
            gdbserver: "gdbserver".to_owned(),
            local_program: String::new(),
            copy_first: false,
            ssh_options: String::new(),
        }
    }
}

/// ssh options for a run with no one to type a password: key login only, and a clear
/// failure when the port can't be carried.
const SSH_OPTIONS: [&str; 4] = ["-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes"];

impl RemoteTarget {
    /// What's missing before it can start, if anything.
    pub fn problem(&self) -> Option<&'static str> {
        if self.host.trim().is_empty() {
            return Some(if self.over_ssh {
                "Name the ssh host (user@host)"
            } else {
                "Name the host gdbserver runs on"
            });
        }
        if self.port == 0 {
            return Some("Choose gdbserver's port");
        }
        if self.over_ssh && self.remote_program.trim().is_empty() {
            return Some("Name the program on the remote machine");
        }
        if self.local_program.trim().is_empty() {
            return Some("Choose the program on this machine, for its symbols");
        }
        if self.copy_first && !Path::new(self.local_program.trim()).is_file() {
            return Some("The local program to copy doesn't exist");
        }
        None
    }

    /// `ssh` arguments that start gdbserver there and carry its port here.
    pub fn ssh_args(&self) -> Vec<String> {
        let port = self.port;
        let mut args: Vec<String> = SSH_OPTIONS.iter().map(|s| (*s).to_owned()).collect();
        args.extend(self.options());
        let here = self.here_port();
        args.extend(["-L".to_owned(), format!("{here}:localhost:{port}"), self.host.trim().to_owned()]);
        let gdbserver = if self.gdbserver.trim().is_empty() { "gdbserver" } else { self.gdbserver.trim() };
        args.extend([gdbserver.to_owned(), format!(":{port}"), self.remote_program.trim().to_owned()]);
        if !self.args.trim().is_empty() {
            args.push(self.args.trim().to_owned());
        }
        args
    }

    /// `scp` arguments that copy the local program to the remote one.
    pub fn scp_args(&self) -> Vec<String> {
        let mut args = vec!["-o".to_owned(), "BatchMode=yes".to_owned()];
        args.extend(self.options());
        args.push(self.local_program.trim().to_owned());
        args.push(format!("{}:{}", self.host.trim(), self.remote_program.trim()));
        args
    }

    /// Over ssh, the port gdb connects to here.
    pub fn here_port(&self) -> u16 {
        if self.local_port == 0 { self.port } else { self.local_port }
    }

    /// The user's own ssh options, split as a shell would.
    fn options(&self) -> Vec<String> {
        crate::build::compile_db::split_command(self.ssh_options.trim())
    }

    /// What gdb connects to (`target remote`).
    pub fn target(&self) -> String {
        if self.over_ssh {
            return format!("localhost:{}", self.here_port());
        }
        let host = self.host.trim();
        // A host given with its port is taken as it is.
        if host.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) {
            host.to_owned()
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// gdb's `attach` request for the remote program.
    pub fn attach_args(&self) -> Value {
        json!({
            "type": "gdb",
            "request": "attach",
            "name": format!("Remote {}", self.target()),
            "program": self.local_program.trim(),
            "target": self.target(),
        })
    }

    /// Where a project keeps its remote target.
    pub fn file(root: &Path) -> PathBuf {
        crate::settings::project_dir(root).join("remote_debug.toml")
    }

    /// A project's remote target, or a new one.
    pub fn load(root: &Path) -> Self {
        std::fs::read_to_string(Self::file(root)).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = Self::file(root);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }
}

/// Whether a line gdbserver printed says it's ready: "Listening on port 2345".
pub fn listening(line: &str) -> bool {
    line.trim_start().starts_with("Listening on port")
}

/// Whether a line says the remote side failed for good (ssh or gdbserver).
pub fn failed(line: &str) -> bool {
    let l = line.to_lowercase();
    [
        "permission denied",
        "could not resolve",
        "connection refused",
        "connection timed out",
        "command not found",
        "not recognized",
        "cannot exec",
        "no such file",
        "can't bind address",
        "port forwarding failed",
    ]
    .iter()
    .any(|m| l.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn over_ssh() -> RemoteTarget {
        RemoteTarget {
            host: "pi@board".into(),
            port: 2345,
            remote_program: "/home/pi/app".into(),
            args: "--fast 3".into(),
            local_program: "target/app".into(),
            ..RemoteTarget::default()
        }
    }

    #[test]
    fn ssh_starts_gdbserver_and_carries_its_port() {
        let t = over_ssh();
        assert_eq!(
            t.ssh_args(),
            [
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                "2345:localhost:2345",
                "pi@board",
                "gdbserver",
                ":2345",
                "/home/pi/app",
                "--fast 3"
            ]
        );
        assert_eq!(t.scp_args(), ["-o", "BatchMode=yes", "target/app", "pi@board:/home/pi/app"]);
        assert_eq!(t.target(), "localhost:2345");
        let a = t.attach_args();
        assert_eq!(a["request"], "attach");
        assert_eq!(a["target"], "localhost:2345");
        assert_eq!(a["program"], "target/app");
        assert_eq!(t.problem(), None);
        let own = RemoteTarget { ssh_options: "-F \"my config\" -o Port=2222".into(), ..t };
        assert_eq!(own.ssh_args()[4..8], ["-F", "my config", "-o", "Port=2222"]);
        assert_eq!(own.scp_args()[2..6], ["-F", "my config", "-o", "Port=2222"]);
        let moved = RemoteTarget { local_port: 2400, ..own };
        assert!(moved.ssh_args().contains(&"2400:localhost:2345".to_owned()));
        assert_eq!(moved.target(), "localhost:2400");
    }

    #[test]
    fn connects_to_a_gdbserver_running() {
        let mut t = RemoteTarget { over_ssh: false, host: "10.0.0.5".into(), port: 9000, ..over_ssh() };
        assert_eq!(t.target(), "10.0.0.5:9000");
        t.host = "board:1234".into();
        assert_eq!(t.target(), "board:1234");
        t.remote_program.clear();
        assert_eq!(t.problem(), None, "no remote program needed to connect");
        assert!(RemoteTarget { host: " ".into(), ..t.clone() }.problem().is_some());
        assert!(RemoteTarget { local_program: String::new(), ..t }.problem().is_some());
    }

    #[test]
    fn reads_gdbserver_and_ssh() {
        assert!(listening("Listening on port 2345"));
        assert!(!listening("Process /home/pi/app created; pid = 77"));
        assert!(failed("pi@board: Permission denied (publickey)."));
        assert!(failed("bash: gdbserver: command not found"));
        assert!(!failed("Remote debugging from host 127.0.0.1"));
    }

    #[test]
    fn kept_per_project() {
        let dir = std::env::temp_dir().join(format!("coiman-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(RemoteTarget::load(&dir), RemoteTarget::default());
        over_ssh().save(&dir).unwrap();
        assert_eq!(RemoteTarget::load(&dir), over_ssh());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
