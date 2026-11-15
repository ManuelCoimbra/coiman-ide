//! Snapshot tests: the snapshots insta (Rust) leaves to review, as `.snap.new` files beside the
//! accepted `.snap`, each to accept (it becomes the snapshot) or reject (it's deleted). Jest and
//! Vitest keep theirs in `__snapshots__` and update them with their own `-u`.

use std::path::{Path, PathBuf};

/// A snapshot waiting for review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    /// The new one (`x.snap.new`).
    pub new: PathBuf,
    /// The accepted one it would replace (`x.snap`), which may not exist yet.
    pub old: PathBuf,
}

impl Pending {
    /// The snapshot's name: the file's, without `.snap.new`.
    pub fn name(&self) -> String {
        let file = self.new.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        file.trim_end_matches(".snap.new").to_owned()
    }
}

/// The pending snapshots under `root` (git-ignored folders and `target` left out).
pub fn find_pending(root: &Path) -> Vec<Pending> {
    let mut found: Vec<Pending> = crate::project::finder::walker(root)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|e| {
            let new = e.into_path();
            let name = new.file_name()?.to_str()?.to_owned();
            let base = name.strip_suffix(".new")?;
            base.ends_with(".snap").then(|| Pending { old: new.with_file_name(base), new })
        })
        .collect();
    found.sort_by(|a, b| a.new.cmp(&b.new));
    found
}

/// A snapshot file's contents without insta's header (`---` lines with `source:` and the like).
pub fn body(text: &str) -> &str {
    let text = text.trim_start_matches('\u{feff}');
    if let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n"))
        && let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\r\n---\r\n"))
    {
        let after = &rest[end..];
        return after.trim_start_matches(['\r', '\n']).trim_start_matches("---").trim_start_matches(['\r', '\n']);
    }
    text
}

/// One line of a snapshot's change: `-` (only the old), `+` (only the new) or ` ` (both).
pub type DiffLine = (char, String);

/// How the new snapshot differs from the old, line by line (bodies only).
pub fn diff(old: &str, new: &str) -> Vec<DiffLine> {
    similar::TextDiff::from_lines(body(old), body(new))
        .iter_all_changes()
        .map(|c| {
            let tag = match c.tag() {
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
                similar::ChangeTag::Equal => ' ',
            };
            (tag, c.value().trim_end_matches(['\n', '\r']).to_owned())
        })
        .collect()
}

/// Accepts a pending snapshot: it becomes the snapshot.
pub fn accept(p: &Pending) -> Result<(), String> {
    std::fs::rename(&p.new, &p.old).map_err(|e| format!("Couldn't accept {}: {e}", p.name()))
}

/// Rejects a pending snapshot: it's deleted, the old one stays.
pub fn reject(p: &Pending) -> Result<(), String> {
    std::fs::remove_file(&p.new).map_err(|e| format!("Couldn't reject {}: {e}", p.name()))
}

/// The command that updates a JavaScript project's snapshots, by the runner it uses
/// (`package.json`'s dependencies): `npx jest -u` or `npx vitest run -u`.
pub fn js_update_command(root: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let has =
        |name: &str| ["dependencies", "devDependencies"].iter().any(|k| doc.get(k).and_then(|d| d.get(name)).is_some());
    let args: &[&str] = if has("vitest") {
        &["npx", "vitest", "run", "-u"]
    } else if has("jest") {
        &["npx", "jest", "-u"]
    } else {
        return None;
    };
    Some(args.iter().map(|a| a.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_found_accepted_and_rejected() {
        let dir = std::env::temp_dir().join(format!("coiman-snaps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let snaps = dir.join("tests").join("snapshots");
        std::fs::create_dir_all(&snaps).unwrap();
        let head = "---\nsource: tests/a.rs\nexpression: value\n---\n";
        std::fs::write(snaps.join("a__one.snap"), format!("{head}hello\nworld\n")).unwrap();
        std::fs::write(snaps.join("a__one.snap.new"), format!("{head}hello\nthere\n")).unwrap();
        std::fs::write(snaps.join("a__two.snap.new"), format!("{head}brand new\n")).unwrap();
        let found = find_pending(&dir);
        assert_eq!(found.iter().map(Pending::name).collect::<Vec<_>>(), ["a__one", "a__two"]);
        let old = std::fs::read_to_string(&found[0].old).unwrap();
        let new = std::fs::read_to_string(&found[0].new).unwrap();
        assert_eq!(diff(&old, &new), [(' ', "hello".to_owned()), ('-', "world".to_owned()), ('+', "there".to_owned())]);
        accept(&found[0]).unwrap();
        assert_eq!(std::fs::read_to_string(snaps.join("a__one.snap")).unwrap(), new);
        reject(&found[1]).unwrap();
        assert!(!snaps.join("a__two.snap.new").exists() && !snaps.join("a__two.snap").exists());
        assert!(find_pending(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn header_left_out_and_js_runners() {
        assert_eq!(body("---\nsource: x\n---\nbody\n"), "body\n");
        assert_eq!(body("no header\n"), "no header\n");
        let dir = std::env::temp_dir().join(format!("coiman-snaps-js-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("package.json"), r#"{"devDependencies":{"vitest":"^2"}}"#).unwrap();
        assert_eq!(js_update_command(&dir).unwrap(), ["npx", "vitest", "run", "-u"]);
        std::fs::write(dir.join("package.json"), r#"{"devDependencies":{"jest":"^29"}}"#).unwrap();
        assert_eq!(js_update_command(&dir).unwrap(), ["npx", "jest", "-u"]);
        std::fs::write(dir.join("package.json"), r#"{}"#).unwrap();
        assert!(js_update_command(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
