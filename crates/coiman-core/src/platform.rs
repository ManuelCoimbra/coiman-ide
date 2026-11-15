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
//!
//! The desktop sets neither.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// Gives the command to run for a program, or `None` to run it as it is.
pub type CommandRouter = Box<dyn Fn(&Path) -> Option<Command> + Send + Sync>;

/// Says where a program is, or `None` to look on `PATH` as usual.
pub type ToolProbe = Box<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

static ROUTER: OnceLock<CommandRouter> = OnceLock::new();
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
