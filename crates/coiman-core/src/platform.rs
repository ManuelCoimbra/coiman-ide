//! Hooks for a port of Coiman to another platform (the Android build, say), so it doesn't have to
//! edit Coiman's source.
//!
//! A port sets them once, before it creates the app:
//!
//! - [`set_command_router`]: every program Coiman runs (git, cargo, compilers, language servers,
//!   formatters) goes through [`crate::language::lsp::discovery::quiet_command`]. The router may
//!   return a different command for a program: on Android, a shim that runs it inside Termux.
//! - [`set_tool_probe`]: [`crate::language::lsp::servers::which`] asks the probe first whether a
//!   program is there, for programs it can't see on `PATH` (Termux's folder belongs to another
//!   app).
//! - [`set_terminal_starter`]: a terminal's program (its shell, or a build run in a terminal)
//!   may be started by the port instead of in a pseudo-terminal: on Android, through Termux.
//!
//! The desktop sets neither.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Gives the command to run for a program, or `None` to run it as it is.
pub type CommandRouter = Box<dyn Fn(&Path) -> Option<Command> + Send + Sync>;

/// Says where a program is, or `None` to look on `PATH` as usual.
pub type ToolProbe = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// Starts a terminal's program, or `None` to start it in a pseudo-terminal as usual.
/// `Started::from_child` makes one from a child process with piped input and output.
pub type TerminalStarter = Box<
    dyn Fn(&crate::terminal::pty::TerminalRequest) -> Option<Result<crate::terminal::pty::Started, String>>
        + Send
        + Sync,
>;

static ROUTER: OnceLock<CommandRouter> = OnceLock::new();
static STARTER: OnceLock<TerminalStarter> = OnceLock::new();

/// Sets the terminal starter. Only the first call counts; it says whether this one did.
pub fn set_terminal_starter(starter: TerminalStarter) -> bool {
    STARTER.set(starter).is_ok()
}

/// The starter's session for `request`, when a starter is set and wants it.
pub(crate) fn terminal_started(
    request: &crate::terminal::pty::TerminalRequest,
) -> Option<Result<crate::terminal::pty::Started, String>> {
    STARTER.get().and_then(|starter| starter(request))
}
static PROBE: OnceLock<ToolProbe> = OnceLock::new();

/// Sets the command router. Only the first call counts; it says whether this one did.
pub fn set_command_router(router: CommandRouter) -> bool {
    ROUTER.set(router).is_ok()
}

/// Sets the tool probe. Only the first call counts; it says whether this one did.
pub fn set_tool_probe(probe: ToolProbe) -> bool {
    PROBE.set(probe).is_ok()
}

/// The router's command for `program`, when a router is set and wants it.
pub(crate) fn routed(program: &Path) -> Option<Command> {
    ROUTER.get().and_then(|router| router(program))
}

/// The probe's answer for `name`, when a probe is set and knows it.
pub(crate) fn probed(name: &str) -> Option<PathBuf> {
    PROBE.get().and_then(|probe| probe(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::lsp::{discovery::quiet_command, servers::which};

    /// A port's terminal: its program's output reaches the screen, its exit is seen, and what
    /// it was asked carries the shell flag and `TERM`. Others still get a pseudo-terminal.
    #[test]
    fn terminal_starter() {
        use crate::terminal::pty::{Notice, Session, Size, Started};
        use std::process::Stdio;
        use std::sync::Mutex;
        use std::time::{Duration, Instant};
        static ASKED: Mutex<Vec<(String, bool, bool)>> = Mutex::new(Vec::new());
        assert!(set_terminal_starter(Box::new(|request| {
            ASKED.lock().unwrap().push((
                request.program.to_owned(),
                request.shell,
                request.env.iter().any(|(k, v)| k == "TERM" && v == "xterm-256color"),
            ));
            if !request.program.starts_with("coiman-port") {
                return None;
            }
            let mut cmd = if cfg!(windows) { Command::new("cmd") } else { Command::new("sh") };
            if cfg!(windows) {
                cmd.args(["/c", "echo port-ok"]);
            } else {
                cmd.args(["-c", "echo port-ok"]);
            }
            cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
            Some(cmd.spawn().map_err(|e| e.to_string()).and_then(|c| Started::from_child(c, Some("port".into()))))
        })));
        let size = Size { columns: 40, lines: 5 };
        let mut session = Session::spawn("coiman-port-shell -l", &std::env::temp_dir(), &[], size, 10, || {}).unwrap();
        assert_eq!(session.name, "port");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut exited = false;
        while Instant::now() < deadline && !(exited && session.screen_text().contains("port-ok")) {
            exited |= session.poll().iter().any(|n| matches!(n, Notice::Exited(_)));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.screen_text().contains("port-ok"), "screen: {:?}", session.screen_text());
        assert!(exited);
        let asked = ASKED.lock().unwrap().clone();
        assert!(asked.contains(&("coiman-port-shell".to_owned(), true, true)), "{asked:?}");
        drop(Session::spawn_args("coiman-port-run", &[], &std::env::temp_dir(), &[], size, 10, || {}).unwrap());
        assert!(ASKED.lock().unwrap().contains(&("coiman-port-run".to_owned(), false, true)));
    }

    /// One test sets both hooks: they can be set once a process.
    #[test]
    fn router_and_probe() {
        let fake = std::env::temp_dir().join("coiman-routed-tool");
        let target = fake.clone();
        assert!(set_command_router(Box::new(move |program| {
            (program == Path::new("coiman-no-such-tool")).then(|| {
                let mut cmd = Command::new(&target);
                cmd.env("COIMAN_ROUTED_TOOL", "coiman-no-such-tool");
                cmd
            })
        })));
        assert!(!set_command_router(Box::new(|_| None)), "a second router is refused");
        let cmd = quiet_command(Path::new("coiman-no-such-tool"));
        assert_eq!(Path::new(cmd.get_program()), fake);
        assert!(cmd.get_envs().any(|(k, v)| k == "COIMAN_ROUTED_TOOL" && v.is_some()));
        // Others run as they are.
        assert_eq!(quiet_command(Path::new("git")).get_program(), "git");

        assert!(set_tool_probe(Box::new(|name| (name == "coiman-no-such-tool").then(|| PathBuf::from("/routed")))));
        assert_eq!(which("coiman-no-such-tool"), Some(PathBuf::from("/routed")));
        assert_eq!(which("coiman-really-no-such-tool"), None);
    }
}
