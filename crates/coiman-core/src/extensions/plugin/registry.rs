//! Plugin registries and signatures.
//!
//! A registry is a folder (or a URL serving the same files) with a
//! `registry.json` index. Each entry names a plugin version, the SHA-256 of
//! its `plugin.toml` and `plugin.wasm`, and an Ed25519 signature by the
//! publisher's key over those. Nothing is installed unless the files match
//! their hashes and the signature verifies. URLs are fetched with the
//! system's `curl` (Coiman IDE itself opens no connections), and only when
//! the user asks.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::manifest::{self, Manifest};
use crate::language::lsp::discovery::quiet_command;

/// Largest plugin file accepted (manifest or WebAssembly).
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Index {
    #[serde(default)]
    pub plugins: Vec<Entry>,
}

/// One published version of a plugin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The publisher's Ed25519 public key, base64.
    pub publisher_key: String,
    /// File paths relative to the registry.
    pub manifest: String,
    pub wasm: String,
    pub manifest_sha256: String,
    pub wasm_sha256: String,
    #[serde(default)]
    pub wasm_size: u64,
    /// Ed25519 signature (base64) over [`statement`].
    pub signature: String,
    #[serde(default)]
    pub changelog: String,
}

/// A plugin's two files, checked against each other.
#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub manifest_bytes: Vec<u8>,
    pub wasm: Vec<u8>,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// What a publisher signs.
pub fn statement(id: &str, version: &str, manifest_sha256: &str, wasm_sha256: &str) -> Vec<u8> {
    format!("rust-code-plugin-v1\n{id}\n{version}\n{manifest_sha256}\n{wasm_sha256}\n").into_bytes()
}

/// A short, readable form of a publisher key for prompts: `1a2b 3c4d 5e6f 7a8b`.
pub fn fingerprint(public_key_b64: &str) -> String {
    let hex = sha256_hex(public_key_b64.trim().as_bytes());
    hex.as_bytes()[..16].chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join(" ")
}

/// A key pair from 32 random bytes: (secret, public), both base64.
pub fn key_pair(seed: [u8; 32]) -> (String, String) {
    let key = SigningKey::from_bytes(&seed);
    (B64.encode(seed), B64.encode(key.verifying_key().to_bytes()))
}

fn signing_key(secret_b64: &str) -> Result<SigningKey, String> {
    let bytes = B64.decode(secret_b64.trim()).map_err(|_| "the secret key isn't base64".to_owned())?;
    let seed: [u8; 32] = bytes.try_into().map_err(|_| "the secret key must be 32 bytes".to_owned())?;
    Ok(SigningKey::from_bytes(&seed))
}

fn verifying_key(public_b64: &str) -> Result<VerifyingKey, String> {
    let bytes = B64.decode(public_b64.trim()).map_err(|_| "the publisher key isn't base64".to_owned())?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| "the publisher key must be 32 bytes".to_owned())?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "the publisher key isn't a valid Ed25519 key".to_owned())
}

impl Entry {
    /// Checks the signature over this entry's id, version and hashes.
    pub fn verify_signature(&self) -> Result<(), String> {
        let key = verifying_key(&self.publisher_key)?;
        let sig = B64.decode(self.signature.trim()).map_err(|_| "the signature isn't base64".to_owned())?;
        let sig = Signature::from_slice(&sig).map_err(|_| "the signature is malformed".to_owned())?;
        key.verify_strict(&statement(&self.id, &self.version, &self.manifest_sha256, &self.wasm_sha256), &sig)
            .map_err(|_| format!("{} {}: the signature doesn't match the publisher key", self.id, self.version))
    }
}

// ----- fetching --------------------------------------------------------------------

fn is_url(base: &str) -> bool {
    ["http://", "https://", "file://"].iter().any(|p| base.starts_with(p))
}

/// Reads `rel` from a registry (folder or URL).
pub fn fetch(base: &str, rel: &str) -> Result<Vec<u8>, String> {
    if rel.contains("..") || rel.starts_with(['/', '\\']) || rel.contains(':') {
        return Err(format!("registry path \"{rel}\" isn't allowed"));
    }
    if is_url(base) {
        let url = format!("{}/{}", base.trim_end_matches('/'), rel);
        return curl(&url);
    }
    let path = Path::new(base).join(rel);
    let len = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    if len > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MB", path.display(), MAX_FILE_BYTES >> 20));
    }
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

fn curl(url: &str) -> Result<Vec<u8>, String> {
    let program = std::env::var_os("SystemRoot")
        .map(|r| Path::new(&r).join("System32").join("curl.exe"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("curl"));
    let out = quiet_command(&program)
        .args(["-L", "--fail", "--silent", "--show-error", "--max-filesize"])
        .arg(MAX_FILE_BYTES.to_string())
        .arg(url)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{url}: {}", err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed").trim()))
    }
}

pub fn load_index(base: &str) -> Result<Index, String> {
    let bytes = fetch(base, "registry.json")?;
    serde_json::from_slice(&bytes).map_err(|e| format!("registry.json: {e}"))
}

fn check(bytes: &[u8], expected: &str, what: &str) -> Result<(), String> {
    let actual = sha256_hex(bytes);
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(format!("{what} doesn't match its published hash (expected {expected}, got {actual})"))
    }
}

/// Fetches and checks an entry's `plugin.toml` only (for update previews).
pub fn fetch_manifest(base: &str, entry: &Entry) -> Result<(Manifest, Vec<u8>), String> {
    entry.verify_signature()?;
    let bytes = fetch(base, &entry.manifest)?;
    check(&bytes, &entry.manifest_sha256, "plugin.toml")?;
    let m = manifest::parse(&String::from_utf8_lossy(&bytes))?;
    if m.id != entry.id || m.version != entry.version {
        return Err(format!(
            "plugin.toml says {} {}, the registry says {} {}",
            m.id, m.version, entry.id, entry.version
        ));
    }
    Ok((m, bytes))
}

/// Fetches an entry's files and checks hashes, signature and identity.
pub fn download(base: &str, entry: &Entry) -> Result<Package, String> {
    let (manifest, manifest_bytes) = fetch_manifest(base, entry)?;
    let wasm = fetch(base, &entry.wasm)?;
    check(&wasm, &entry.wasm_sha256, "plugin.wasm")?;
    Ok(Package { manifest, manifest_bytes, wasm })
}

/// A plugin folder with `plugin.toml` and `plugin.wasm` (unsigned, for development).
pub fn read_folder(dir: &Path) -> Result<Package, String> {
    let manifest_bytes = std::fs::read(dir.join("plugin.toml")).map_err(|e| format!("plugin.toml: {e}"))?;
    let manifest = manifest::parse(&String::from_utf8_lossy(&manifest_bytes))?;
    let wasm = std::fs::read(dir.join("plugin.wasm")).map_err(|e| format!("plugin.wasm: {e}"))?;
    if wasm.len() as u64 > MAX_FILE_BYTES {
        return Err("plugin.wasm is too large".to_owned());
    }
    Ok(Package { manifest, manifest_bytes, wasm })
}

/// Adds a plugin folder to a registry folder: copies its files to
/// `<id>/<version>/`, signs them with `secret_b64`, and updates `registry.json`
/// (replacing the same version if it was published before).
pub fn publish(registry: &Path, plugin_dir: &Path, secret_b64: &str, changelog: &str) -> Result<Entry, String> {
    let key = signing_key(secret_b64)?;
    let pkg = read_folder(plugin_dir)?;
    let m = &pkg.manifest;
    let rel = format!("{}/{}", m.id, m.version);
    let dir = registry.join(&rel);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(dir.join("plugin.toml"), &pkg.manifest_bytes).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("plugin.wasm"), &pkg.wasm).map_err(|e| e.to_string())?;
    let (manifest_sha256, wasm_sha256) = (sha256_hex(&pkg.manifest_bytes), sha256_hex(&pkg.wasm));
    let signature = B64.encode(key.sign(&statement(&m.id, &m.version, &manifest_sha256, &wasm_sha256)).to_bytes());
    let entry = Entry {
        id: m.id.clone(),
        version: m.version.clone(),
        name: m.name.clone(),
        description: m.description.clone(),
        publisher_key: B64.encode(key.verifying_key().to_bytes()),
        manifest: format!("{rel}/plugin.toml"),
        wasm: format!("{rel}/plugin.wasm"),
        manifest_sha256,
        wasm_sha256,
        wasm_size: pkg.wasm.len() as u64,
        signature,
        changelog: changelog.to_owned(),
    };
    let index_path = registry.join("registry.json");
    let mut index: Index = match std::fs::read(&index_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("registry.json: {e}"))?,
        Err(_) => Index::default(),
    };
    index.plugins.retain(|e| !(e.id == entry.id && e.version == entry.version));
    index.plugins.push(entry.clone());
    index.plugins.sort_by(|a, b| {
        a.id.cmp(&b.id).then_with(|| manifest::parse_version(&a.version).cmp(&manifest::parse_version(&b.version)))
    });
    let json = serde_json::to_string_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(&index_path, json).map_err(|e| format!("{}: {e}", index_path.display()))?;
    Ok(entry)
}

/// The latest version of each plugin in an index.
pub fn latest(index: &Index) -> Vec<&Entry> {
    let mut out: Vec<&Entry> = Vec::new();
    for e in &index.plugins {
        match out.iter_mut().find(|o| o.id == e.id) {
            Some(o) if manifest::is_newer(&e.version, &o.version) => *o = e,
            Some(_) => {}
            None => out.push(e),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_signs_verifies_and_rejects_tampering() {
        let tmp = std::env::temp_dir().join(format!("rust-code-registry-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (plugin, registry) = (tmp.join("plugin"), tmp.join("registry"));
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("plugin.toml"), "id = \"me.demo\"\nname = \"Demo\"\nversion = \"1.0.0\"\n").unwrap();
        std::fs::write(plugin.join("plugin.wasm"), b"\0asm-not-really").unwrap();
        let (secret, public) = key_pair([7; 32]);

        let entry = publish(&registry, &plugin, &secret, "first").unwrap();
        assert_eq!(entry.publisher_key, public);
        let base = registry.to_string_lossy().into_owned();
        let index = load_index(&base).unwrap();
        assert_eq!(latest(&index)[0].version, "1.0.0");
        let pkg = download(&base, &index.plugins[0]).unwrap();
        assert_eq!(pkg.manifest.name, "Demo");

        // Changed file: hash mismatch.
        std::fs::write(registry.join("me.demo/1.0.0/plugin.wasm"), b"evil").unwrap();
        assert!(download(&base, &index.plugins[0]).unwrap_err().contains("published hash"));
        // Edited entry (hash swapped to match the evil file): signature fails.
        let mut forged = index.plugins[0].clone();
        forged.wasm_sha256 = sha256_hex(b"evil");
        assert!(download(&base, &forged).unwrap_err().contains("signature"));
        // Someone else's key.
        let (_, other) = key_pair([9; 32]);
        forged = index.plugins[0].clone();
        forged.publisher_key = other;
        assert!(forged.verify_signature().is_err());
        assert!(fetch(&base, "../secret").is_err());
        assert_eq!(fingerprint(&public).len(), 19);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
