//! Tools a build needs that Rust Code can fetch when asked: pinned releases, checked
//! against their hash, kept in its config folder and used by what it runs. They don't
//! touch the system: nothing outside Rust Code sees them.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::lsp::discovery::quiet_command;
use crate::lsp::servers::{system_tool, which};
use crate::plugin::registry::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildTool {
    Ninja,
    MinGw,
}

impl BuildTool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ninja => "Ninja 1.13.2",
            Self::MinGw => "GCC 16.2 for Windows (winlibs MinGW-w64)",
        }
    }

    /// Why it's wanted, and what it takes, for someone deciding.
    pub fn offer(self) -> String {
        match self {
            Self::Ninja => "This CMake project needs a build tool, and none was found. Download Ninja 1.13.2 from \
                            github.com/ninja-build (under 1 MB) into Rust Code's config folder?"
                .to_owned(),
            Self::MinGw => "This CMake project needs a C++ compiler, and none was found. Download GCC 16.2 and its \
                            build tools (winlibs MinGW-w64) from github.com/brechtsanders (264 MB, about 900 MB unpacked) \
                            into Rust Code's config folder?"
                .to_owned(),
        }
    }

    fn url(self) -> &'static str {
        match self {
            Self::Ninja => "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip",
            Self::MinGw => {
                "https://github.com/brechtsanders/winlibs_mingw/releases/download/16.2.0posix-23.1.3-14.0.0-ucrt-r3/\
                 winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip"
            }
        }
    }

    fn sha256(self) -> &'static str {
        match self {
            Self::Ninja => "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65",
            Self::MinGw => "3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1",
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Ninja => "ninja",
            Self::MinGw => "mingw",
        }
    }

    /// Where its programs end up, under its folder.
    fn bin(self) -> &'static str {
        match self {
            Self::Ninja => "",
            Self::MinGw => "mingw64/bin",
        }
    }
}

fn tools_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("tools"))
}

/// Folders of the tools that have been fetched, for finding and running them.
pub fn dirs() -> Vec<PathBuf> {
    let Some(tools) = tools_dir() else { return Vec::new() };
    [BuildTool::Ninja, BuildTool::MinGw]
        .into_iter()
        .map(|t| tools.join(t.folder()).join(t.bin()))
        .filter(|d| d.is_dir())
        .collect()
}

/// What's in the way of CMake building here, given what's installed.
fn missing(has_compiler: bool, has_generator: bool) -> Option<BuildTool> {
    match (has_compiler, has_generator) {
        (true, true) => None,
        // Brings its own make, so it's enough by itself.
        (false, _) => Some(BuildTool::MinGw),
        (true, false) => Some(BuildTool::Ninja),
    }
}

/// What CMake needs fetched before it can build the project in `dir`, if anything.
/// Only Windows lacks these by default, and Visual Studio provides for itself.
pub fn needed_for_cmake(dir: &Path) -> Option<BuildTool> {
    if !cfg!(windows) || dir.join("build").join("CMakeCache.txt").is_file() {
        return None;
    }
    let installer = std::env::var_os("ProgramFiles(x86)")
        .map(|p| Path::new(&p).join("Microsoft Visual Studio").join("Installer").join("vswhere.exe"));
    if installer.is_some_and(|p| p.is_file()) {
        return None;
    }
    let found = |names: &[&str]| names.iter().any(|n| which(n).is_some());
    missing(found(&["g++", "clang++", "cl"]), found(&["ninja", "mingw32-make"]))
}

fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let output = quiet_command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let why = String::from_utf8_lossy(&output.stderr);
    Err(format!("{}: {}", program.display(), why.trim()))
}

/// Downloads and unpacks a tool with the system's `curl` and `tar`. Only call this
/// after the user agreed. A file that isn't the one expected is removed, not used.
pub fn install(tool: BuildTool) -> Result<(), String> {
    let dir = tools_dir().ok_or("No config directory available")?.join(tool.folder());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join("download.zip");
    let archive_text = archive.to_string_lossy().into_owned();
    let result = (|| {
        let curl = ["-L", "--fail", "--silent", "--show-error", "-o", &archive_text, tool.url()];
        run(&system_tool("curl"), &curl)?;
        let bytes = std::fs::read(&archive).map_err(|e| e.to_string())?;
        if sha256_hex(&bytes) != tool.sha256() {
            return Err("The download isn't the expected file, so it wasn't used".to_owned());
        }
        drop(bytes);
        run(&system_tool("tar"), &["-xf", &archive_text, "-C", &dir.to_string_lossy()])
    })();
    let _ = std::fs::remove_file(&archive);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_for_what_is_missing() {
        assert_eq!(missing(true, true), None);
        assert_eq!(missing(true, false), Some(BuildTool::Ninja));
        assert_eq!(missing(false, true), Some(BuildTool::MinGw));
        assert_eq!(missing(false, false), Some(BuildTool::MinGw));
    }

    #[test]
    fn pins_are_whole() {
        for tool in [BuildTool::Ninja, BuildTool::MinGw] {
            assert_eq!(tool.sha256().len(), 64);
            assert!(tool.url().starts_with("https://github.com/") && !tool.url().contains(' '));
        }
    }
}
