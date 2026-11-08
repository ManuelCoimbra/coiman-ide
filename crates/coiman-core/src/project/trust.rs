//! Workspace trust. Opening a folder must not run anything from it: its
//! settings can name programs, language servers run its build scripts and
//! proc macros, Cargo reads its `.cargo/config.toml` and `rust-toolchain.toml`,
//! Git reads hooks and `core.fsmonitor` from `.git/config`.
//!
//! Until the user trusts a folder (or a parent of it), it is in Restricted
//! Mode: every place that would run something checks [`is_trusted`] first.
//! The list lives in `<config>/rust-code/trusted_folders.toml`, outside any
//! project, so a project can't trust itself. An unreadable list trusts nothing.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// `None` until [`enforce`] is called: library users and tests aren't gated.
static TRUSTED: RwLock<Option<Vec<PathBuf>>> = RwLock::new(None);

#[derive(Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    folders: Vec<PathBuf>,
}

fn file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("trusted_folders.toml"))
}

/// Turns the gate on and loads the list. The editor calls this first thing.
pub fn enforce() {
    let text = file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let folders = toml::from_str::<TrustFile>(&text).map(|t| t.folders).unwrap_or_default();
    *TRUSTED.write().unwrap_or_else(|e| e.into_inner()) = Some(folders);
}

/// A path resolved as far as it exists: links, junctions, `..` and Windows short names
/// (`RUNNER~1`) become the real names, and the part not made yet is added back as written.
fn resolve(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = at.canonicalize() {
            return rest.iter().rev().fold(real, |p, part| p.join(part));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The form paths are compared in: resolved, no `\\?\`, case-insensitive on Windows.
fn normalize(path: &Path) -> PathBuf {
    let path = resolve(path);
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(if cfg!(windows) { text.to_lowercase() } else { text.to_owned() })
}

fn covered(list: &[PathBuf], path: &Path) -> bool {
    let path = normalize(path);
    list.iter().any(|t| path.starts_with(normalize(t)))
}

/// Toolchain sources and downloaded crates, opened through go to definition.
fn is_tool_folder(path: &Path) -> bool {
    let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) else { return false };
    let home = Path::new(&home);
    let rustup = std::env::var_os("RUSTUP_HOME").map_or_else(|| home.join(".rustup"), PathBuf::from);
    let cargo = std::env::var_os("CARGO_HOME").map_or_else(|| home.join(".cargo"), PathBuf::from);
    covered(&[rustup.join("toolchains"), cargo.join("registry")], path)
}

/// Whether things may run for this folder.
pub fn is_trusted(path: &Path) -> bool {
    match &*TRUSTED.read().unwrap_or_else(|e| e.into_inner()) {
        None => true,
        Some(list) => covered(list, path) || is_tool_folder(path),
    }
}

/// [`is_trusted`] as an error, for code that returns one.
pub fn check(path: &Path) -> Result<(), String> {
    if is_trusted(path) { Ok(()) } else { Err(format!("{} isn't trusted (Restricted Mode)", path.display())) }
}

fn save(list: &[PathBuf]) -> Result<(), String> {
    let path = file().ok_or("no config folder")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = toml::to_string_pretty(&TrustFile { folders: list.to_vec() }).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Trusts a folder and everything under it, and remembers that.
pub fn trust(folder: &Path) -> Result<(), String> {
    let mut guard = TRUSTED.write().unwrap_or_else(|e| e.into_inner());
    let list = guard.get_or_insert_with(Vec::new);
    if !covered(list, folder) {
        list.push(folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf()));
        save(list)?;
    }
    Ok(())
}

/// Stops trusting a folder: every entry that covers it is removed.
pub fn revoke(folder: &Path) -> Result<(), String> {
    let mut guard = TRUSTED.write().unwrap_or_else(|e| e.into_inner());
    let list = guard.get_or_insert_with(Vec::new);
    let path = normalize(folder);
    list.retain(|t| !path.starts_with(normalize(t)));
    save(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path that doesn't exist yet, written another way than the trusted folder was (through
    /// `..`, a junction, or a Windows short name like `RUNNER~1`), is still under it.
    #[test]
    fn a_missing_path_under_a_folder_written_another_way_is_covered() {
        let base = std::env::temp_dir().join(format!("rust-code-trust-alias-{}", std::process::id()));
        let repo = base.join("repo");
        std::fs::create_dir_all(base.join("x")).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let list = vec![repo.canonicalize().unwrap()];
        let roundabout = base.join("x").join("..").join("repo").join("not-made-yet").join("file.rs");
        assert!(covered(&list, &roundabout));
        assert!(!covered(&list, &base.join("x").join("..").join("other").join("file.rs")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_folder_covers_what_is_under_it_only() {
        let base = std::env::temp_dir().join(format!("rust-code-trust-{}", std::process::id()));
        let (a, a_sub, ab) = (base.join("a"), base.join("a").join("sub"), base.join("ab"));
        for d in [&a_sub, &ab] {
            std::fs::create_dir_all(d).unwrap();
        }
        let list = vec![a.clone()];
        assert!(covered(&list, &a) && covered(&list, &a_sub));
        // Same prefix, different folder.
        assert!(!covered(&list, &ab));
        // `..` can't climb out of a trusted folder.
        assert!(!covered(&list, &a.join("..").join("ab")));
        assert!(!covered(&[], &a));
        let _ = std::fs::remove_dir_all(&base);
    }
}
