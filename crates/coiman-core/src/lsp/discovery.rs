//! Finding language server executables. Nothing here downloads anything:
//! installing is a separate, explicit step the user has to agree to.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What [`find_rust_analyzer`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    Server(PathBuf),
    /// Not installed. `rustup` is set when it can install it.
    Missing {
        rustup: Option<PathBuf>,
    },
}

/// Stops a process and what it started: runners and shells are often a chain of them.
pub fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        let _ = quiet_command(Path::new("taskkill"))
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// A command that never opens a console window (the app is a GUI program).
pub fn quiet_command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Whether `program --version` runs and prints something starting with `expect`.
fn responds(program: &Path, expect: &str) -> bool {
    quiet_command(program)
        .arg("--version")
        // Not the app's folder: its rust-toolchain.toml would choose what runs.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).starts_with(expect))
}

fn cargo_bin(name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("CARGO_HOME").map(PathBuf::from).or_else(|| {
        std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(|h| Path::new(&h).join(".cargo"))
    })?;
    let exe = home.join("bin").join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    exe.exists().then_some(exe)
}

/// Looks for a working rust-analyzer: the configured path, then `PATH`, then
/// `~/.cargo/bin`. The rustup proxy only counts if its component is installed
/// (otherwise it exits with an error), which `--version` checks.
pub fn find_rust_analyzer(configured: &str) -> Found {
    let mut candidates = Vec::new();
    if !configured.trim().is_empty() {
        candidates.push(PathBuf::from(configured.trim()));
    }
    candidates.push(PathBuf::from("rust-analyzer"));
    candidates.extend(cargo_bin("rust-analyzer"));
    if let Some(server) = candidates.into_iter().find(|c| responds(c, "rust-analyzer")) {
        return Found::Server(server);
    }
    let rustup = [PathBuf::from("rustup")].into_iter().chain(cargo_bin("rustup")).find(|r| responds(r, "rustup"));
    Found::Missing { rustup }
}

/// `rustup component add rust-analyzer`: downloads the server from the
/// official Rust distribution servers. Only run after the user agreed.
pub fn install_rust_analyzer(rustup: &Path) -> Result<(), String> {
    let out = quiet_command(rustup)
        .args(["component", "add", "rust-analyzer"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run rustup: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("rustup failed").trim().to_owned())
    }
}

#[cfg(all(test, windows))]
mod kill_tests {
    use super::*;
    use std::io::Read;
    use std::process::Stdio;
    use std::time::Duration;

    #[test]
    fn stops_what_a_process_started() {
        // The grandchild holds the pipe open, so it only ends if that dies too.
        let mut child = quiet_command(Path::new("cmd"))
            .args(["/c", "ping -n 60 127.0.0.1"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pipe = child.stdout.take().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        kill_tree(&mut child);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut all = Vec::new();
            let _ = pipe.read_to_end(&mut all);
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_secs(10)).is_ok(), "something was left running");
    }
}
