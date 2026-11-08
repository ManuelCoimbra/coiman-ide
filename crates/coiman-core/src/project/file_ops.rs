//! Copying and moving files and folders, as the Explorer's Paste does.

use std::io;
use std::path::{Path, PathBuf};

/// Where a pasted copy of `name` goes in `dir`: the name itself when it's free, else
/// "name copy.ext", then "name copy 2.ext" and on, as VS Code names them.
pub fn free_copy_name(dir: &Path, name: &str) -> PathBuf {
    let plain = dir.join(name);
    if !plain.exists() {
        return plain;
    }
    // ".env" has no extension; "a.tar.gz" keeps only ".gz" apart.
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    (1..)
        .map(|n| if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") })
        .map(|candidate| dir.join(candidate))
        .find(|path| !path.exists())
        .expect("some name is free")
}

/// Whether `path` is `folder` or inside it: a folder can't go into itself.
pub fn is_within(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
}

/// Copies a file, or a folder with everything in it, to `to`.
pub fn copy_all(from: &Path, to: &Path) -> io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_all(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Moves a file or folder to `to`; across drives, copies it and removes the original.
pub fn move_to(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_all(from, to)?;
    if from.is_dir() { std::fs::remove_dir_all(from) } else { std::fs::remove_file(from) }
}

/// Deletes for good: a file, or a folder with everything in it.
pub fn remove_all(path: &Path) -> io::Result<()> {
    if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coiman_file_ops_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copies_get_free_names() {
        let dir = scratch("names");
        assert_eq!(free_copy_name(&dir, "a.rs"), dir.join("a.rs"));
        std::fs::write(dir.join("a.rs"), "").unwrap();
        assert_eq!(free_copy_name(&dir, "a.rs"), dir.join("a copy.rs"));
        std::fs::write(dir.join("a copy.rs"), "").unwrap();
        assert_eq!(free_copy_name(&dir, "a.rs"), dir.join("a copy 2.rs"));
        std::fs::write(dir.join(".env"), "").unwrap();
        assert_eq!(free_copy_name(&dir, ".env"), dir.join(".env copy"));
        std::fs::create_dir(dir.join("src")).unwrap();
        assert_eq!(free_copy_name(&dir, "src"), dir.join("src copy"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folders_copy_and_move_whole() {
        let dir = scratch("copy");
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::write(dir.join("src/inner/x.txt"), "x").unwrap();
        std::fs::write(dir.join("src/y.txt"), "y").unwrap();
        copy_all(&dir.join("src"), &dir.join("copy")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("copy/inner/x.txt")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(dir.join("copy/y.txt")).unwrap(), "y");
        move_to(&dir.join("copy"), &dir.join("moved")).unwrap();
        assert!(!dir.join("copy").exists());
        assert!(dir.join("moved/inner/x.txt").exists());
        assert!(is_within(&dir.join("src/inner"), &dir.join("src")));
        assert!(!is_within(&dir.join("srcx"), &dir.join("src")));
        remove_all(&dir.join("moved")).unwrap();
        assert!(!dir.join("moved").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
