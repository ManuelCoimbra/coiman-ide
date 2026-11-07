//! Tools a build needs that Coiman IDE can fetch when asked: pinned releases, checked
//! against their hash, kept in its config folder and used by what it runs. They don't
//! touch the system: nothing outside Coiman IDE sees them.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::lsp::discovery::quiet_command;
use crate::lsp::servers::{system_tool, which};
use crate::plugin::registry::sha256_hex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildTool {
    Ninja,
    MinGw,
    JsDebug,
    Maven,
}

impl BuildTool {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ninja => "Ninja 1.13.2",
            Self::MinGw => "GCC 16.2 for Windows (winlibs MinGW-w64)",
            Self::JsDebug => "js-debug 1.140.0",
            Self::Maven => "Apache Maven 3.9.16",
        }
    }

    /// Why it's wanted, and what it takes, for someone deciding.
    pub fn offer(self) -> String {
        match self {
            Self::Ninja => "This CMake project needs a build tool, and none was found. Download Ninja 1.13.2 from \
                            github.com/ninja-build (under 1 MB) into Coiman IDE's config folder?"
                .to_owned(),
            Self::MinGw => "This CMake project needs a C++ compiler, and none was found. Download GCC 16.2 and its \
                            build tools (winlibs MinGW-w64) from github.com/brechtsanders (264 MB, about 900 MB unpacked) \
                            into Coiman IDE's config folder?"
                .to_owned(),
            Self::JsDebug => "Debugging JavaScript needs js-debug, and it wasn't found. Download js-debug 1.140.0 from \
                              github.com/microsoft (about 1 MB) into Coiman IDE's config folder?"
                .to_owned(),
            Self::Maven => "This Maven project needs Maven, and it wasn't found (no mvnw, no mvn on PATH). Download \
                            Apache Maven 3.9.16 from repo.maven.apache.org (9 MB) into Coiman IDE's config folder? It \
                            needs Java."
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
            Self::JsDebug => {
                "https://github.com/microsoft/vscode-js-debug/releases/download/v1.140.0/js-debug-dap-v1.140.0.tar.gz"
            }
            Self::Maven => {
                "https://repo.maven.apache.org/maven2/org/apache/maven/apache-maven/3.9.16/apache-maven-3.9.16-bin.zip"
            }
        }
    }

    /// The archive's SHA-256, or SHA-512 where that's what its publisher gives.
    fn digest(self) -> &'static str {
        match self {
            Self::Ninja => "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65",
            Self::MinGw => "3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1",
            Self::JsDebug => "27dab92937ec1ab35821ae955aac867544fe06a1b6307229049f2d789af10968",
            Self::Maven => {
                "ed41650d42485cfc243fad22158caf9cbb5dc408ce7a09ddb94dd42a019de929ca43065bfa450612cf12bf78b5cafa3884b96c090de326ff590448c933454af3"
            }
        }
    }

    fn folder(self) -> &'static str {
        match self {
            Self::Ninja => "ninja",
            Self::MinGw => "mingw",
            Self::JsDebug => "js-debug",
            Self::Maven => "maven",
        }
    }

    /// Where its programs end up, under its folder.
    fn bin(self) -> &'static str {
        match self {
            Self::Ninja | Self::JsDebug => "",
            Self::MinGw => "mingw64/bin",
            Self::Maven => "apache-maven-3.9.16/bin",
        }
    }
}

fn tools_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("tools"))
}

/// Folders of the tools that have been fetched, for finding and running them.
pub fn dirs() -> Vec<PathBuf> {
    let Some(tools) = tools_dir() else { return Vec::new() };
    [BuildTool::Ninja, BuildTool::MinGw, BuildTool::Maven]
        .into_iter()
        .map(|t| tools.join(t.folder()).join(t.bin()))
        .filter(|d| d.is_dir())
        .collect()
}

/// The server program of js-debug, if it's been fetched.
pub fn js_debug_server() -> Option<PathBuf> {
    let dir = tools_dir()?.join(BuildTool::JsDebug.folder());
    Some(dir.join("js-debug").join("src").join("dapDebugServer.js")).filter(|p| p.is_file())
}

/// Maven's launcher, if it's been fetched.
pub fn maven_program() -> Option<PathBuf> {
    let dir = tools_dir()?.join(BuildTool::Maven.folder()).join(BuildTool::Maven.bin());
    Some(dir.join(if cfg!(windows) { "mvn.cmd" } else { "mvn" })).filter(|p| p.is_file())
}

/// The digest a pin is compared with: by its length, SHA-256 or SHA-512.
fn digest_hex(pin: &str, bytes: &[u8]) -> String {
    use sha2::Digest;
    if pin.len() == 128 {
        sha2::Sha512::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    } else {
        sha256_hex(bytes)
    }
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
        if digest_hex(tool.digest(), &bytes) != tool.digest() {
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
    fn digests_by_their_length() {
        assert_eq!(
            digest_hex(&"0".repeat(64), b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(digest_hex(&"0".repeat(128), b"abc").starts_with("ddaf35a193617aba"));
    }

    #[test]
    fn pins_are_whole() {
        for tool in [BuildTool::Ninja, BuildTool::MinGw, BuildTool::JsDebug, BuildTool::Maven] {
            assert!(matches!(tool.digest().len(), 64 | 128));
            assert!(tool.digest().chars().all(|c| c.is_ascii_hexdigit()));
            let host = tool.url().starts_with("https://github.com/")
                || tool.url().starts_with("https://repo.maven.apache.org/");
            assert!(host && !tool.url().contains(' '));
        }
    }
}
