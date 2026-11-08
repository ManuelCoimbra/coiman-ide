//! What a running language server reports about itself, and what it costs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::discovery::quiet_command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    Warning,
    Error,
}

/// rust-analyzer's `experimental/serverStatus`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerStatus {
    pub health: Health,
    /// Nothing left to do: indexing and loading are finished.
    pub quiescent: bool,
    pub message: Option<String>,
}

impl ServerStatus {
    pub fn parse(params: &Value) -> Self {
        let health = match params["health"].as_str() {
            Some("warning") => Health::Warning,
            Some("error") => Health::Error,
            _ => Health::Ok,
        };
        let message = params["message"].as_str().filter(|m| !m.is_empty()).map(str::to_owned);
        Self { health, quiescent: params["quiescent"].as_bool().unwrap_or(true), message }
    }
}

/// "5s", "2m 05s", "1h 03m".
pub fn format_duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// "512 MB", "1.2 GB".
pub fn format_bytes(bytes: u64) -> String {
    const MB: u64 = 1024 * 1024;
    if bytes < 1024 * MB {
        format!("{} MB", bytes / MB)
    } else {
        format!("{:.1} GB", bytes as f64 / (1024 * MB) as f64)
    }
}

/// Lines of "pid parent bytes" into a table. Bad lines are skipped.
fn parse_processes(output: &str, unit: u64) -> Vec<(u32, u32, u64)> {
    let parse = |line: &str| {
        let mut f = line.split_whitespace();
        Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?, f.next()?.parse::<u64>().ok()? * unit))
    };
    output.lines().filter_map(parse).collect()
}

/// What `root` and everything it started use together. Servers are often
/// launched through a small wrapper (rustup's proxy, an npm shim), so the
/// process itself says little.
fn tree_memory(table: &[(u32, u32, u64)], root: u32) -> Option<u64> {
    let mut seen = vec![root];
    let mut total = table.iter().find(|p| p.0 == root)?.2;
    let mut i = 0;
    while let Some(&parent) = seen.get(i) {
        i += 1;
        for &(pid, _, bytes) in table.iter().filter(|p| p.1 == parent) {
            // Process ids get reused, which can make a loop.
            if !seen.contains(&pid) {
                seen.push(pid);
                total += bytes;
            }
        }
    }
    Some(total)
}

/// Memory each of `roots` uses with its children, in bytes. Runs a program:
/// call it off the UI thread.
pub fn process_memory(roots: &[u32]) -> HashMap<u32, u64> {
    let run = |program: &str, args: &[&str]| {
        let out = quiet_command(Path::new(program)).args(args).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let table = if cfg!(windows) {
        const LIST: &str = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.WorkingSetSize)\" }";
        run("powershell", &["-NoProfile", "-NonInteractive", "-Command", LIST]).map(|o| parse_processes(&o, 1))
    } else {
        run("ps", &["-A", "-o", "pid=,ppid=,rss="]).map(|o| parse_processes(&o, 1024))
    };
    let table = table.unwrap_or_default();
    roots.iter().filter_map(|&root| Some((root, tree_memory(&table, root)?))).collect()
}

/// Whether a line of cargo's output says it is waiting for the build folder.
/// Package cache locks are another matter: a separate build folder won't help.
pub fn waits_for_build_lock(line: &str) -> bool {
    let line = line.to_lowercase();
    line.contains("waiting for file lock") && !line.contains("package cache")
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(m) if m.is_dir() => dir_size(&entry.path()),
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        })
        .sum()
}

/// Bytes in a workspace's target folder, and how many of those are
/// rust-analyzer's own copy. Runs cargo and walks the folder: call it off the
/// UI thread, and only for trusted folders.
pub fn build_folder_sizes(root: &Path) -> (u64, u64) {
    // Cargo knows where it really is (`CARGO_TARGET_DIR`, `build.target-dir`).
    let reported = quiet_command(Path::new("cargo"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .and_then(|v| v["target_directory"].as_str().map(PathBuf::from));
    let target = reported.unwrap_or_else(|| root.join("target"));
    (dir_size(&target), dir_size(&target.join("rust-analyzer")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognizes_lock_waits() {
        assert!(waits_for_build_lock("    Blocking waiting for file lock on build directory"));
        assert!(!waits_for_build_lock("    Blocking waiting for file lock on package cache"));
        assert!(!waits_for_build_lock("    Checking foo v0.1.0"));
    }

    #[test]
    fn sizes_a_folder_with_its_subfolders() {
        let dir = std::env::temp_dir().join(format!("coiman-size-test-{}", std::process::id()));
        let inner = dir.join("target").join("rust-analyzer");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(dir.join("target").join("a"), [0u8; 10]).unwrap();
        std::fs::write(inner.join("b"), [0u8; 5]).unwrap();
        assert_eq!(build_folder_sizes(&dir), (15, 5));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_server_status() {
        let s = ServerStatus::parse(&json!({"health": "warning", "quiescent": false, "message": "no Cargo.toml"}));
        assert_eq!(s.health, Health::Warning);
        assert!(!s.quiescent);
        assert_eq!(s.message.as_deref(), Some("no Cargo.toml"));
        let s = ServerStatus::parse(&json!({"health": "ok", "quiescent": true, "message": ""}));
        assert_eq!((s.health, s.quiescent, s.message), (Health::Ok, true, None));
    }

    #[test]
    fn formats_for_people() {
        assert_eq!(format_duration(5), "5s");
        assert_eq!(format_duration(125), "2m 05s");
        assert_eq!(format_duration(3780), "1h 03m");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512 MB");
        assert_eq!(format_bytes(1288490188), "1.2 GB");
    }

    #[test]
    fn adds_up_a_process_tree() {
        let table = parse_processes("1 0 100\n2 1 10\n3 2 5\n4 9 7\nnonsense\n5 5 1\n", 1);
        assert_eq!(tree_memory(&table, 1), Some(115));
        assert_eq!(tree_memory(&table, 2), Some(15));
        assert_eq!(tree_memory(&table, 42), None);
        // A process that seems to be its own parent doesn't count twice.
        assert_eq!(tree_memory(&table, 5), Some(1));
    }

    #[test]
    fn measures_this_process() {
        let me = std::process::id();
        assert!(process_memory(&[me]).get(&me).is_some_and(|b| *b > 0));
    }
}
