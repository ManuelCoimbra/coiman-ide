//! Zed extension registry (`api.zed.dev`), archive installer, and local extension store.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use tar::Archive;

use super::manifest::{ExtensionManifest, read_wasm_api_version};

const API_BASE: &str = "https://api.zed.dev/extensions";
pub const MAX_SCHEMA_VERSION: i32 = 1;

/// An extension entry returned by `api.zed.dev/extensions`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub schema_version: i32,
    #[serde(default)]
    pub wasm_api_version: Option<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub download_count: u64,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    data: Vec<RegistryExtension>,
}

/// An extension installed on disk under `<config_dir>/zed_extensions/installed/<id>`.
#[derive(Clone, Debug)]
pub struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub authors: Vec<String>,
    pub repository: String,
    pub wasm_api_version: Option<(u16, u16, u16)>,
    pub has_wasm: bool,
    pub enabled: bool,
    pub source: String,
    pub path: PathBuf,
    pub manifest: ExtensionManifest,
}

impl InstalledExtension {
    pub fn from_folder(dir: &Path, source: &str, enabled: bool) -> Result<Self, String> {
        let manifest = ExtensionManifest::load(dir)?;
        let wasm_path = dir.join("extension.wasm");
        let has_wasm = wasm_path.is_file();
        let wasm_api_version = has_wasm
            .then(|| std::fs::read(&wasm_path).ok().and_then(|b| read_wasm_api_version(&b)))
            .flatten();
        Ok(Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone().unwrap_or_default(),
            authors: manifest.authors.clone(),
            repository: manifest.repository.clone().unwrap_or_default(),
            wasm_api_version,
            has_wasm,
            enabled,
            source: source.to_owned(),
            path: dir.to_path_buf(),
            manifest,
        })
    }

    pub fn provides(&self) -> Vec<String> {
        self.manifest.provides()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ExtensionRecord {
    pub version: String,
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ZedExtensionState {
    #[serde(default)]
    pub disabled: BTreeSet<String>,
    #[serde(default)]
    pub records: BTreeMap<String, ExtensionRecord>,
}

impl ZedExtensionState {
    pub fn load() -> Self {
        let Some(path) = super::state_path() else { return Self::default() };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = super::state_path() else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

/// Searches or browses the Zed extensions registry (`https://api.zed.dev/extensions`).
pub fn search(filter: Option<&str>, provides: Option<&str>) -> Result<Vec<RegistryExtension>, String> {
    let mut url = format!("{API_BASE}?max_schema_version={MAX_SCHEMA_VERSION}");
    if let Some(q) = filter.map(str::trim).filter(|q| !q.is_empty()) {
        url.push_str("&filter=");
        url.push_str(&encode_query(q));
    }
    if let Some(cat) = provides.map(str::trim).filter(|c| !c.is_empty()) {
        url.push_str("&provides=");
        url.push_str(&encode_query(cat));
    }
    let body = curl_get(&url)?;
    let parsed: ApiResponse = serde_json::from_slice(&body)
        .map_err(|e| format!("Invalid registry response from api.zed.dev: {e}"))?;
    Ok(parsed.data)
}

/// Fetches all versions of `id` from the Zed registry.
pub fn versions(id: &str) -> Result<Vec<RegistryExtension>, String> {
    let url = format!("{API_BASE}/{}", encode_query(id));
    let body = curl_get(&url)?;
    let parsed: ApiResponse = serde_json::from_slice(&body)
        .map_err(|e| format!("Invalid extension details from api.zed.dev: {e}"))?;
    Ok(parsed.data)
}

/// Fetches README markdown for a GitHub repository URL (`https://github.com/owner/repo`).
pub fn fetch_readme(repository: &str) -> Result<String, String> {
    let slug = repository
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .strip_prefix("https://github.com/")
        .or_else(|| repository.trim().trim_end_matches('/').strip_prefix("http://github.com/"))
        .ok_or_else(|| "README preview is only supported for GitHub repositories".to_owned())?;
    let mut parts = slug.split('/').filter(|p| !p.is_empty());
    let (Some(owner), Some(repo)) = (parts.next(), parts.next()) else {
        return Err("Invalid GitHub repository URL".to_owned());
    };
    for branch in ["HEAD", "main", "master"] {
        for file in ["README.md", "readme.md", "README.markdown"] {
            let url = format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{file}");
            if let Ok(bytes) = curl_get(&url)
                && let Ok(text) = String::from_utf8(bytes)
                && !text.trim().is_empty()
            {
                return Ok(text);
            }
        }
    }
    Err("README not found".to_owned())
}

/// Downloads a Zed extension `.tar.gz` archive from `api.zed.dev` and installs it.
pub fn download_and_install(ext: &RegistryExtension) -> Result<InstalledExtension, String> {
    let id = sanitize_ext_id(&ext.id)?;
    let installed_root = super::installed_dir().ok_or("No config directory available")?;
    let url = format!("{API_BASE}/{}/{}/download", encode_query(&id), encode_query(&ext.version));
    let bytes = curl_get(&url)?;
    install_tar_gz_into(&installed_root, &id, &ext.version, &bytes, "zed-registry")
}

/// Installs an extension from raw `.tar.gz` bytes into `<installed_root>/<id>`.
pub fn install_tar_gz_into(
    installed_root: &Path,
    id: &str,
    version: &str,
    archive_bytes: &[u8],
    source: &str,
) -> Result<InstalledExtension, String> {
    let id = sanitize_ext_id(id)?;
    std::fs::create_dir_all(installed_root).map_err(|e| e.to_string())?;
    let stage = installed_root.join(format!(".staging-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    let unpack_result = unpack_tar_gz(archive_bytes, &stage);
    if let Err(err) = unpack_result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(err);
    }

    // Some tarballs wrap files inside a single top-level directory; unwrap if extension.toml is there.
    let content_root = unwrap_single_subdir(&stage);
    let target = installed_root.join(&id);
    let backup = installed_root.join(format!(".backup-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&backup);
    if target.exists() {
        std::fs::rename(&target, &backup).map_err(|e| e.to_string())?;
    }
    let move_res = if content_root == stage {
        std::fs::rename(&stage, &target)
    } else {
        let r = std::fs::rename(&content_root, &target);
        let _ = std::fs::remove_dir_all(&stage);
        r
    };
    if let Err(err) = move_res {
        let _ = std::fs::remove_dir_all(&stage);
        if backup.exists() {
            let _ = std::fs::rename(&backup, &target);
        }
        return Err(format!("Could not finalize extension install: {err}"));
    }
    let _ = std::fs::remove_dir_all(&backup);

    let mut state = ZedExtensionState::load();
    state.disabled.remove(&id);
    state.records.insert(
        id.clone(),
        ExtensionRecord {
            version: version.to_owned(),
            source: source.to_owned(),
        },
    );
    let _ = state.save();

    let installed = InstalledExtension::from_folder(&target, source, true)?;
    super::sync_assets();
    Ok(installed)
}

/// Installs an extension from a local folder (dev extension or unpacked bundle).
pub fn install_from_folder(folder: &Path) -> Result<InstalledExtension, String> {
    let manifest = ExtensionManifest::load(folder)?;
    let id = sanitize_ext_id(&manifest.id)?;
    let installed_root = super::installed_dir().ok_or("No config directory available")?;
    std::fs::create_dir_all(&installed_root).map_err(|e| e.to_string())?;
    let target = installed_root.join(&id);
    let stage = installed_root.join(format!(".staging-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    copy_dir_filtered(folder, &stage).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&stage);
    })?;
    if target.exists() {
        let _ = std::fs::remove_dir_all(&target);
    }
    std::fs::rename(&stage, &target).map_err(|e| e.to_string())?;

    let source = format!("folder:{}", folder.display());
    let mut state = ZedExtensionState::load();
    state.disabled.remove(&id);
    state.records.insert(
        id.clone(),
        ExtensionRecord {
            version: manifest.version,
            source: source.clone(),
        },
    );
    let _ = state.save();

    let installed = InstalledExtension::from_folder(&target, &source, true)?;
    super::sync_assets();
    Ok(installed)
}

/// Enables or disables an installed Zed extension.
pub fn set_enabled(id: &str, enabled: bool) -> Result<(), String> {
    let mut state = ZedExtensionState::load();
    if enabled {
        state.disabled.remove(id);
    } else {
        state.disabled.insert(id.to_owned());
    }
    state.save()?;
    super::sync_assets();
    Ok(())
}

/// Uninstalls an installed Zed extension and cleans up its working folder & icon packs.
pub fn uninstall(id: &str) -> Result<(), String> {
    let id = sanitize_ext_id(id)?;
    if let Some(dir) = super::installed_dir() {
        let target = dir.join(&id);
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| format!("Could not remove {}: {e}", target.display()))?;
        }
    }
    if let Some(work) = super::work_dir(&id)
        && work.exists()
    {
        let _ = std::fs::remove_dir_all(work);
    }
    let mut state = ZedExtensionState::load();
    state.disabled.remove(&id);
    state.records.remove(&id);
    state.save()?;
    super::sync_assets();
    Ok(())
}

/// Lists all installed Zed extensions (including disabled ones).
pub fn list_installed() -> Vec<InstalledExtension> {
    let Some(dir) = super::installed_dir() else { return Vec::new() };
    list_installed_in(&dir, &ZedExtensionState::load())
}

pub fn list_installed_in(dir: &Path, state: &ZedExtensionState) -> Vec<InstalledExtension> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let disabled = state.disabled.contains(name);
        let source = state
            .records
            .get(name)
            .map(|r| r.source.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("zed-registry");
        if let Ok(ext) = InstalledExtension::from_folder(&path, source, !disabled) {
            out.push(ext);
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    out
}

/// Lists only active (enabled) installed Zed extensions, returning empty if Zed extensions are disabled globally.
pub fn list_active() -> Vec<InstalledExtension> {
    if !super::is_enabled_global() {
        return Vec::new();
    }
    list_installed().into_iter().filter(|e| e.enabled).collect()
}

fn sanitize_ext_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.starts_with('.')
    {
        return Err(format!("Invalid Zed extension id: {id:?}"));
    }
    Ok(id.to_owned())
}

fn unwrap_single_subdir(stage: &Path) -> PathBuf {
    if stage.join("extension.toml").is_file() || stage.join("extension.json").is_file() {
        return stage.to_path_buf();
    }
    let Ok(entries) = std::fs::read_dir(stage) else {
        return stage.to_path_buf();
    };
    let children: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    if children.len() == 1
        && children[0].is_dir()
        && (children[0].join("extension.toml").is_file() || children[0].join("extension.json").is_file())
    {
        return children[0].clone();
    }
    stage.to_path_buf()
}

fn unpack_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let gz = GzDecoder::new(bytes);
    let mut archive = Archive::new(gz);
    let entries = archive.entries().map_err(|e| format!("Invalid .tar.gz archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Corrupt tar entry: {e}"))?;
        let rel = entry.path().map_err(|e| format!("Invalid tar entry path: {e}"))?.into_owned();
        if !is_safe_rel(&rel) {
            return Err(format!("Unsafe archive entry path: {}", rel.display()));
        }
        let out_path = dest.join(&rel);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            std::fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else if kind.is_file() {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| format!("Failed reading {}: {e}", rel.display()))?;
            std::fs::write(&out_path, buf).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn is_safe_rel(path: &Path) -> bool {
    !path.is_absolute()
        && path.components().all(|c| !matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_)))
}

fn copy_dir_filtered(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    let entries = std::fs::read_dir(src).map_err(|e| e.to_string())?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        if matches!(name, "target" | ".git" | "node_modules") {
            continue;
        }
        let target = dst.join(name);
        if path.is_dir() {
            copy_dir_filtered(&path, &target)?;
        } else if path.is_file() {
            std::fs::copy(&path, &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub(crate) fn curl_get(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-sSLf", "--max-time", "30", "-A", "CoimanIDE/0.8.0", url])
        .output()
        .map_err(|e| format!("Could not run curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        return Err(if err.is_empty() {
            format!("curl exited with {}", out.status)
        } else {
            format!("curl: {err}")
        });
    }
    Ok(out.stdout)
}

fn encode_query(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push_str("%20");
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    #[test]
    fn unpacks_real_extension_tar_gz_safely() {
        let mut tar_data = Vec::new();
        {
            let gz = GzEncoder::new(&mut tar_data, Compression::default());
            let mut builder = tar::Builder::new(gz);
            let manifest = b"id = \"hello\"\nname = \"Hello\"\nversion = \"0.1.0\"\nschema_version = 1\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(manifest.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, "extension.toml", &manifest[..]).unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("coiman-zed-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let stage = dir.join("stage");
        std::fs::create_dir_all(&stage).unwrap();
        unpack_tar_gz(&tar_data, &stage).unwrap();
        let ext = InstalledExtension::from_folder(&stage, "test", true).unwrap();
        assert_eq!(ext.id, "hello");
        assert_eq!(ext.version, "0.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
