//! Installed plugins: `<config>/plugins/<id>/` holds each plugin's
//! files, and `plugins.lock` records exactly what was installed (version,
//! source, publisher key, file hashes) and what the user granted. A plugin
//! whose files no longer match the lockfile is not loaded.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::manifest::{self, Manifest, ManifestDiff, Permission};
use super::registry::{self, Entry, Package};

/// `<config>/plugins`.
pub fn plugins_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("plugins"))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    /// Publisher keys (base64) the user chose to trust.
    #[serde(default)]
    pub trusted_keys: Vec<String>,
    #[serde(default, rename = "plugin")]
    pub plugins: Vec<Locked>,
}

/// One installed plugin, as installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locked {
    pub id: String,
    pub version: String,
    /// The registry it came from, or `folder:<path>` for an unsigned install.
    pub source: String,
    /// Empty for unsigned (folder) installs.
    #[serde(default)]
    pub publisher_key: String,
    pub manifest_sha256: String,
    pub wasm_sha256: String,
    /// What the user granted (a subset of what the plugin asks for).
    #[serde(default)]
    pub granted: BTreeSet<Permission>,
    /// Programs the user approved it to start (a subset of what it declares).
    #[serde(default)]
    pub granted_programs: BTreeSet<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The version this one replaced, kept (in `<id>/previous/`) so an update can be undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Box<Locked>>,
}

fn yes() -> bool {
    true
}

impl Locked {
    pub fn signed(&self) -> bool {
        !self.publisher_key.is_empty()
    }
}

impl Lockfile {
    pub fn load(dir: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(dir.join("plugins.lock")) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("plugins.lock: {}", e.message())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("plugins.lock: {e}")),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = format!(
            "# Installed plugins. Written by Coiman IDE; a plugin whose files don't match\n\
             # these hashes is not loaded.\n\n{}",
            toml::to_string_pretty(self).map_err(|e| e.to_string())?
        );
        write_atomic(&dir.join("plugins.lock"), text.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&Locked> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Locked> {
        self.plugins.iter_mut().find(|p| p.id == id)
    }

    pub fn trusts(&self, key: &str) -> bool {
        self.trusted_keys.iter().any(|k| k == key)
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn plugin_dir(dir: &Path, id: &str) -> PathBuf {
    dir.join(id)
}

/// Copies a checked package into the store and records it in the lockfile
/// (replacing an earlier version). Does not save the lockfile.
pub fn install(
    dir: &Path,
    lock: &mut Lockfile,
    pkg: &Package,
    source: &str,
    publisher_key: &str,
    granted: BTreeSet<Permission>,
    programs: BTreeSet<String>,
) -> Result<(), String> {
    let id = &pkg.manifest.id;
    let target = plugin_dir(dir, id);
    std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
    let wasm_sha256 = registry::sha256_hex(&pkg.wasm);
    let manifest_sha256 = registry::sha256_hex(&pkg.manifest_bytes);
    // The version being replaced is kept, once, unless it's the same files again.
    let mut previous = None;
    if let Some(old) = lock.get(id).cloned()
        && (old.wasm_sha256 != wasm_sha256 || old.manifest_sha256 != manifest_sha256)
    {
        let keep = target.join(PREVIOUS);
        let _ = std::fs::remove_dir_all(&keep);
        std::fs::create_dir_all(&keep).map_err(|e| format!("{}: {e}", keep.display()))?;
        for f in ["plugin.wasm", "plugin.toml"] {
            if target.join(f).is_file() {
                std::fs::copy(target.join(f), keep.join(f)).map_err(|e| format!("{f}: {e}"))?;
            }
        }
        previous = Some(Box::new(Locked { previous: None, ..old }));
    } else if let Some(old) = lock.get(id) {
        previous = old.previous.clone();
    }
    write_atomic(&target.join("plugin.wasm"), &pkg.wasm)?;
    write_atomic(&target.join("plugin.toml"), &pkg.manifest_bytes)?;
    let enabled = lock.get(id).is_none_or(|l| l.enabled);
    lock.plugins.retain(|p| &p.id != id);
    lock.plugins.push(Locked {
        id: id.clone(),
        version: pkg.manifest.version.clone(),
        source: source.to_owned(),
        publisher_key: publisher_key.to_owned(),
        manifest_sha256,
        wasm_sha256,
        granted: granted.intersection(&pkg.manifest.permissions).copied().collect(),
        granted_programs: programs.intersection(&pkg.manifest.programs).cloned().collect(),
        enabled,
        previous,
    });
    lock.plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(())
}

/// Where an installed plugin's previous version is kept.
const PREVIOUS: &str = "previous";

/// Puts back the version an update replaced, with what was granted to it; the version rolled
/// back from is kept in its place, so the rollback can itself be undone. Its files are checked
/// against the lockfile first. Does not save the lockfile.
pub fn rollback(dir: &Path, lock: &mut Lockfile, id: &str) -> Result<String, String> {
    let current = lock.get(id).cloned().ok_or(format!("{id} isn't installed"))?;
    let old = current.previous.clone().ok_or("there's no earlier version to go back to")?;
    let target = plugin_dir(dir, id);
    let keep = target.join(PREVIOUS);
    let wasm = std::fs::read(keep.join("plugin.wasm")).map_err(|e| format!("the earlier plugin.wasm: {e}"))?;
    let toml = std::fs::read(keep.join("plugin.toml")).map_err(|e| format!("the earlier plugin.toml: {e}"))?;
    if registry::sha256_hex(&wasm) != old.wasm_sha256 || registry::sha256_hex(&toml) != old.manifest_sha256 {
        return Err("the earlier version's files changed since it was kept: not put back".to_owned());
    }
    let now_wasm = std::fs::read(target.join("plugin.wasm"));
    let now_toml = std::fs::read(target.join("plugin.toml"));
    write_atomic(&target.join("plugin.wasm"), &wasm)?;
    write_atomic(&target.join("plugin.toml"), &toml)?;
    // The one rolled back from becomes the one to go forward to.
    if let (Ok(w), Ok(t)) = (now_wasm, now_toml) {
        write_atomic(&keep.join("plugin.wasm"), &w)?;
        write_atomic(&keep.join("plugin.toml"), &t)?;
    }
    let version = old.version.clone();
    let enabled = current.enabled;
    let entry = lock.get_mut(id).ok_or("lost")?;
    *entry = Locked { enabled, previous: Some(Box::new(Locked { previous: None, ..current })), ..*old };
    Ok(version)
}

/// Removes a plugin's files and lockfile entry. Does not save the lockfile.
pub fn uninstall(dir: &Path, lock: &mut Lockfile, id: &str) -> Result<(), String> {
    lock.plugins.retain(|p| p.id != id);
    match std::fs::remove_dir_all(plugin_dir(dir, id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Reads an installed plugin, refusing files that changed since install.
pub fn load(dir: &Path, locked: &Locked) -> Result<Package, String> {
    let target = plugin_dir(dir, &locked.id);
    let manifest_bytes = std::fs::read(target.join("plugin.toml")).map_err(|e| format!("plugin.toml: {e}"))?;
    let wasm = std::fs::read(target.join("plugin.wasm")).map_err(|e| format!("plugin.wasm: {e}"))?;
    if registry::sha256_hex(&manifest_bytes) != locked.manifest_sha256
        || registry::sha256_hex(&wasm) != locked.wasm_sha256
    {
        return Err("its files changed since it was installed (not loaded; reinstall it)".to_owned());
    }
    let manifest = manifest::parse(&String::from_utf8_lossy(&manifest_bytes))?;
    if manifest.id != locked.id || manifest.version != locked.version {
        return Err("plugin.toml doesn't match the lockfile".to_owned());
    }
    Ok(Package { manifest, manifest_bytes, wasm })
}

/// What an update changes, shown before it is applied.
#[derive(Clone, Debug)]
pub struct UpdateDiff {
    pub id: String,
    pub name: String,
    pub from: String,
    pub to: String,
    pub manifest: ManifestDiff,
    /// The new version is signed by a different key than the installed one.
    pub key_changed: bool,
    /// Size of `plugin.wasm`, before and after.
    pub size: (u64, u64),
    pub changelog: String,
    pub entry: Entry,
}

pub fn update_diff(locked: &Locked, installed: &Package, entry: &Entry, next: &Manifest) -> UpdateDiff {
    UpdateDiff {
        id: locked.id.clone(),
        name: next.name.clone(),
        from: locked.version.clone(),
        to: entry.version.clone(),
        manifest: manifest::diff(&installed.manifest, next),
        key_changed: locked.signed() && locked.publisher_key != entry.publisher_key,
        size: (installed.wasm.len() as u64, entry.wasm_size),
        changelog: entry.changelog.clone(),
        entry: entry.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_can_be_rolled_back() {
        let dir = std::env::temp_dir().join(format!("coiman-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pkg = |version: &str, wasm: &[u8], perms: &str| {
            let text = format!("id = \"me.tool\"\nname = \"Tool\"\nversion = \"{version}\"\n{perms}");
            Package {
                manifest: manifest::parse(&text).unwrap(),
                manifest_bytes: text.into_bytes(),
                wasm: wasm.to_vec(),
            }
        };
        let none = BTreeSet::new;
        let read: BTreeSet<Permission> = [Permission::WorkspaceRead].into();
        let perms = "permissions = [\"workspace-read\"]";
        let mut lock = Lockfile::default();
        install(&dir, &mut lock, &pkg("1.0.0", b"one", perms), "folder:x", "", read.clone(), none()).unwrap();
        assert!(lock.get("me.tool").unwrap().previous.is_none(), "nothing before the first");
        assert!(rollback(&dir, &mut lock, "me.tool").unwrap_err().contains("no earlier version"));
        // The same files again keep no copy of themselves.
        install(&dir, &mut lock, &pkg("1.0.0", b"one", perms), "folder:x", "", read.clone(), none()).unwrap();
        assert!(lock.get("me.tool").unwrap().previous.is_none());
        install(&dir, &mut lock, &pkg("2.0.0", b"two", ""), "folder:x", "", BTreeSet::new(), none()).unwrap();
        lock.get_mut("me.tool").unwrap().enabled = false;
        // It goes through the lockfile on disk.
        lock.save(&dir).unwrap();
        let mut lock = Lockfile::load(&dir).unwrap();
        assert_eq!(lock.get("me.tool").unwrap().previous.as_ref().unwrap().version, "1.0.0");
        assert_eq!(rollback(&dir, &mut lock, "me.tool").unwrap(), "1.0.0");
        let now = lock.get("me.tool").unwrap().clone();
        assert_eq!((now.version.as_str(), now.granted.clone(), now.enabled), ("1.0.0", read, false));
        assert_eq!(load(&dir, &now).unwrap().wasm, b"one");
        // And forward again.
        assert_eq!(rollback(&dir, &mut lock, "me.tool").unwrap(), "2.0.0");
        assert_eq!(load(&dir, lock.get("me.tool").unwrap()).unwrap().wasm, b"two");
        // Kept files that changed aren't put back.
        std::fs::write(dir.join("me.tool").join(PREVIOUS).join("plugin.wasm"), b"evil").unwrap();
        assert!(rollback(&dir, &mut lock, "me.tool").unwrap_err().contains("changed"));
        uninstall(&dir, &mut lock, "me.tool").unwrap();
        assert!(!dir.join("me.tool").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installs_locks_detects_tampering_and_diffs_updates() {
        let dir = std::env::temp_dir().join(format!("coiman-store-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let toml = "id = \"me.demo\"\nname = \"Demo\"\nversion = \"1.0.0\"\npermissions = [\"editor-read\"]\n";
        let pkg =
            Package { manifest: manifest::parse(toml).unwrap(), manifest_bytes: toml.into(), wasm: b"wasm".to_vec() };
        let mut lock = Lockfile::default();
        let granted = BTreeSet::from([Permission::EditorRead, Permission::WorkspaceWrite]);
        install(&dir, &mut lock, &pkg, "C:/reg", "KEY", granted, BTreeSet::new()).unwrap();
        // Only what the plugin asks for is recorded.
        assert_eq!(lock.plugins[0].granted, BTreeSet::from([Permission::EditorRead]));
        lock.save(&dir).unwrap();
        let lock = Lockfile::load(&dir).unwrap();
        let locked = lock.get("me.demo").unwrap();
        assert!(load(&dir, locked).is_ok());

        let mut next = pkg.manifest.clone();
        next.version = "1.1.0".into();
        next.permissions.insert(Permission::WorkspaceRead);
        let entry = Entry {
            id: "me.demo".into(),
            version: "1.1.0".into(),
            name: "Demo".into(),
            description: String::new(),
            publisher_key: "OTHER".into(),
            manifest: String::new(),
            wasm: String::new(),
            manifest_sha256: String::new(),
            wasm_sha256: String::new(),
            wasm_size: 10,
            signature: String::new(),
            changelog: "faster".into(),
        };
        let d = update_diff(locked, &pkg, &entry, &next);
        assert!(d.key_changed);
        assert_eq!(d.manifest.added_permissions, vec![Permission::WorkspaceRead]);
        assert_eq!(d.size, (4, 10));

        std::fs::write(dir.join("me.demo/plugin.wasm"), b"changed").unwrap();
        assert!(load(&dir, locked).unwrap_err().contains("changed since"));
        let mut lock = lock;
        uninstall(&dir, &mut lock, "me.demo").unwrap();
        assert!(lock.plugins.is_empty() && !dir.join("me.demo").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
