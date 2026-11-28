//! Zed extension manifest (`extension.toml` / legacy `extension.json`), language
//! configuration (`languages/*/config.toml`), and Wasm API version section reader.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Supported Zed extension manifest schema versions (`0` and `1`).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SchemaVersion(pub i32);

impl SchemaVersion {
    pub const ZERO: Self = Self(0);
    pub const CURRENT: Self = Self(1);

    pub fn is_v0(&self) -> bool {
        self.0 == 0
    }
}

/// Legacy v0 JSON manifest (`extension.json`).
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct OldExtensionManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub themes: BTreeMap<String, String>,
    #[serde(default)]
    pub languages: BTreeMap<String, String>,
    #[serde(default)]
    pub grammars: BTreeMap<String, serde_json::Value>,
}

/// Snippet path(s) field in `extension.toml`: a single path or array of paths.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExtensionSnippets {
    Single(PathBuf),
    Multiple(Vec<PathBuf>),
}

impl ExtensionSnippets {
    pub fn paths(&self) -> Vec<&PathBuf> {
        match self {
            Self::Single(path) => vec![path],
            Self::Multiple(paths) => paths.iter().collect(),
        }
    }
}

/// Capability declarations from `[capabilities]`.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtensionCapability {
    #[serde(rename = "process:exec")]
    ProcessExec {
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    #[serde(rename = "download_file")]
    DownloadFile {
        #[serde(default)]
        host: String,
        #[serde(default)]
        path: Vec<String>,
    },
    #[serde(rename = "npm:install")]
    NpmInstall {
        #[serde(default)]
        package: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LibManifestEntry {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct GrammarManifestEntry {
    #[serde(default)]
    pub repository: String,
    #[serde(default, alias = "commit")]
    pub rev: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LanguageServerManifestEntry {
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub language_ids: BTreeMap<String, String>,
    #[serde(default)]
    pub code_action_kinds: Option<Vec<String>>,
    #[serde(default)]
    pub opt_in_languages: BTreeSet<String>,
}

impl LanguageServerManifestEntry {
    /// All language names associated with this server (handling both `languages` and legacy `language`).
    pub fn all_languages(&self) -> Vec<&str> {
        if self.languages.is_empty() {
            self.language.as_deref().into_iter().collect()
        } else {
            self.languages.iter().map(String::as_str).collect()
        }
    }

    /// Returns whether the server is opt-in for `language_name`.
    pub fn is_opt_in_for(&self, language_name: &str) -> bool {
        self.opt_in_languages.contains(language_name)
    }
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct ContextServerManifestEntry {}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct SlashCommandManifestEntry {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub requires_argument: bool,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugAdapterManifestEntry {
    #[serde(default)]
    pub schema_path: Option<PathBuf>,
}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct DebugLocatorManifestEntry {}

#[derive(Clone, Default, PartialEq, Eq, Debug, Deserialize, Serialize)]
pub struct LanguageModelProviderManifestEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Parsed Zed `extension.toml` (or converted legacy `extension.json`).
#[derive(Debug, PartialEq, Eq, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub schema_version: SchemaVersion,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub lib: LibManifestEntry,
    #[serde(default)]
    pub themes: Vec<PathBuf>,
    #[serde(default)]
    pub icon_themes: Vec<PathBuf>,
    #[serde(default)]
    pub languages: Vec<PathBuf>,
    #[serde(default)]
    pub grammars: BTreeMap<String, GrammarManifestEntry>,
    #[serde(default)]
    pub language_servers: BTreeMap<String, LanguageServerManifestEntry>,
    #[serde(default)]
    pub context_servers: BTreeMap<String, ContextServerManifestEntry>,
    #[serde(default)]
    pub slash_commands: BTreeMap<String, SlashCommandManifestEntry>,
    #[serde(default)]
    pub snippets: Option<ExtensionSnippets>,
    #[serde(default)]
    pub capabilities: Vec<ExtensionCapability>,
    #[serde(default)]
    pub debug_adapters: BTreeMap<String, DebugAdapterManifestEntry>,
    #[serde(default)]
    pub debug_locators: BTreeMap<String, DebugLocatorManifestEntry>,
    #[serde(default)]
    pub language_model_providers: BTreeMap<String, LanguageModelProviderManifestEntry>,
}

impl ExtensionManifest {
    /// Loads `extension.toml` (or legacy `extension.json`) from `extension_dir` and fills defaults.
    pub fn load(extension_dir: &Path) -> Result<Self, String> {
        let dir_name = extension_dir
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("Invalid extension directory: {}", extension_dir.display()))?;

        let toml_path = extension_dir.join("extension.toml");
        let json_path = extension_dir.join("extension.json");
        let mut manifest = if toml_path.is_file() {
            let content = std::fs::read_to_string(&toml_path)
                .map_err(|e| format!("Could not read {}: {e}", toml_path.display()))?;
            toml::from_str::<Self>(&content)
                .map_err(|e| format!("Invalid {}: {e}", toml_path.display()))?
        } else if json_path.is_file() {
            let content = std::fs::read_to_string(&json_path)
                .map_err(|e| format!("Could not read {}: {e}", json_path.display()))?;
            let old: OldExtensionManifest = serde_json::from_str(&content)
                .map_err(|e| format!("Invalid {}: {e}", json_path.display()))?;
            from_old_manifest(old, dir_name)
        } else {
            return Err(format!("No extension.toml or extension.json in {}", extension_dir.display()));
        };

        manifest.populate_defaults(extension_dir);
        Ok(manifest)
    }

    /// Populates themes, icon_themes, languages, snippets, grammars, and lib kind from directory layout.
    pub fn populate_defaults(&mut self, extension_dir: &Path) {
        if self.schema_version.is_v0() {
            self.languages.clear();
            self.grammars.clear();
            self.themes.clear();
        }

        if extension_dir.join("Cargo.toml").is_file() && self.lib.kind.is_none() {
            self.lib.kind = Some("Rust".to_owned());
        }

        let languages_dir = extension_dir.join("languages");
        if languages_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&languages_dir)
        {
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.join("config.toml").is_file())
                .filter_map(|p| p.strip_prefix(extension_dir).ok().map(Path::to_path_buf))
                .collect();
            dirs.sort();
            for rel in dirs {
                if !self.languages.contains(&rel) {
                    self.languages.push(rel);
                }
            }
        }

        collect_json_files(extension_dir, "themes", &mut self.themes);
        collect_json_files(extension_dir, "icon_themes", &mut self.icon_themes);

        if self.snippets.is_none() && extension_dir.join("snippets.json").is_file() {
            self.snippets = Some(ExtensionSnippets::Single(PathBuf::from("snippets.json")));
        }

        // Discover prebuilt .wasm grammars in `grammars/` as well as v0 `grammars/*.toml`.
        let grammars_dir = extension_dir.join("grammars");
        if grammars_dir.is_dir()
            && let Ok(entries) = std::fs::read_dir(&grammars_dir)
        {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str());
                if matches!(ext, Some("wasm") | Some("toml"))
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                {
                    self.grammars.entry(stem.to_owned()).or_default();
                }
            }
        }
    }

    /// Feature tags matching Zed's registry `provides` categories.
    pub fn provides(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.themes.is_empty() {
            out.push("themes".to_owned());
        }
        if !self.icon_themes.is_empty() {
            out.push("icon-themes".to_owned());
        }
        if !self.languages.is_empty() {
            out.push("languages".to_owned());
        }
        if !self.grammars.is_empty() {
            out.push("grammars".to_owned());
        }
        if !self.language_servers.is_empty() {
            out.push("language-servers".to_owned());
        }
        if !self.context_servers.is_empty() {
            out.push("context-servers".to_owned());
        }
        if !self.slash_commands.is_empty() {
            out.push("slash-commands".to_owned());
        }
        if self.snippets.is_some() {
            out.push("snippets".to_owned());
        }
        if !self.debug_adapters.is_empty() {
            out.push("debug-adapters".to_owned());
        }
        out
    }

    /// Resolves `debug_adapter_schemas/<name>.json` or custom `schema_path`.
    pub fn debug_adapter_schema_path(&self, adapter: &str) -> PathBuf {
        self.debug_adapters
            .get(adapter)
            .and_then(|e| e.schema_path.clone())
            .unwrap_or_else(|| PathBuf::from("debug_adapter_schemas").join(format!("{adapter}.json")))
    }

    /// Checks whether a capability entry allows running `command` with `args`.
    pub fn allows_exec(&self, command: &str, args: &[String]) -> bool {
        if self.capabilities.is_empty() {
            // Extensions compiled prior to capability enforcement or without declared
            // capabilities still work, unless restricted explicitly.
            return false;
        }
        self.capabilities.iter().any(|cap| match cap {
            ExtensionCapability::ProcessExec { command: allowed_cmd, args: allowed_args } => {
                command_matches(allowed_cmd, command) && args_match(allowed_args, args)
            }
            _ => false,
        })
    }
}

fn command_matches(pattern: &str, candidate: &str) -> bool {
    pattern == "*" || pattern == "**" || pattern == candidate
}

fn args_match(pattern: &[String], actual: &[String]) -> bool {
    let mut i = 0;
    for pat in pattern {
        if pat == "**" {
            return true;
        }
        let Some(arg) = actual.get(i) else {
            return false;
        };
        if pat != "*" && pat != arg {
            return false;
        }
        i += 1;
    }
    i == actual.len()
}

fn collect_json_files(root: &Path, subdir: &str, out: &mut Vec<PathBuf>) {
    let dir = root.join(subdir);
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .filter_map(|p| p.strip_prefix(root).ok().map(Path::to_path_buf))
        .collect();
    paths.sort();
    for rel in paths {
        if !out.contains(&rel) {
            out.push(rel);
        }
    }
}

fn from_old_manifest(old: OldExtensionManifest, id: &str) -> ExtensionManifest {
    let mut themes: Vec<PathBuf> = old.themes.into_values().map(PathBuf::from).collect();
    themes.sort();
    themes.dedup();
    let mut languages: Vec<PathBuf> = old.languages.into_values().map(PathBuf::from).collect();
    languages.sort();
    languages.dedup();
    let grammars = old.grammars.into_keys().map(|k| (k, GrammarManifestEntry::default())).collect();
    ExtensionManifest {
        id: id.to_owned(),
        name: old.name,
        version: old.version,
        schema_version: SchemaVersion::ZERO,
        description: old.description,
        repository: old.repository,
        authors: old.authors,
        themes,
        languages,
        grammars,
        ..Default::default()
    }
}

/// Bracket rule inside a Zed language `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ZedBracketPair {
    pub start: String,
    pub end: String,
    #[serde(default = "default_true")]
    pub close: bool,
    #[serde(default)]
    pub newline: bool,
    #[serde(default)]
    pub not_in: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Wrap characters configured inside a Zed language `config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct WrapCharacters {
    #[serde(default)]
    pub start_prefix: String,
    #[serde(default)]
    pub start_suffix: String,
    #[serde(default)]
    pub end_prefix: String,
    #[serde(default)]
    pub end_suffix: String,
}

/// JSX/HTML tag auto-close configuration (`[jsx_tag_auto_close]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct JsxTagAutoClose {}

/// Parsed Zed `languages/<lang>/config.toml`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct LanguageConfig {
    pub name: String,
    #[serde(default)]
    pub grammar: Option<String>,
    #[serde(default)]
    pub path_suffixes: Vec<String>,
    #[serde(default)]
    pub first_line_pattern: Option<String>,
    #[serde(default)]
    pub line_comments: Vec<String>,
    #[serde(default)]
    pub block_comment: Option<(String, String)>,
    #[serde(default)]
    pub brackets: Vec<ZedBracketPair>,
    #[serde(default)]
    pub tab_size: Option<u32>,
    #[serde(default)]
    pub hard_tabs: Option<bool>,
    #[serde(default)]
    pub autoclose_before: Option<String>,
    #[serde(default)]
    pub word_characters: Vec<String>,
    #[serde(default)]
    pub wrap_characters: Option<WrapCharacters>,
    #[serde(default)]
    pub jsx_tag_auto_close: Option<JsxTagAutoClose>,
    #[serde(default)]
    pub hidden: bool,
}

impl LanguageConfig {
    pub fn load(language_dir: &Path) -> Result<Self, String> {
        let path = language_dir.join("config.toml");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("Could not read {}: {e}", path.display()))?;
        toml::from_str(&text).map_err(|e| format!("Invalid {}: {e}", path.display()))
    }
}

/// Reads Zed's `zed:api-version` custom section from a `.wasm` component or core module.
/// The payload is 6 bytes = three big-endian `u16` numbers `(major, minor, patch)`.
/// Components built for `wasm32-wasip2` embed the `zed:api-version` custom section inside an
/// inner core-module section (`id == 1`), so we scan recursively into embedded modules.
pub fn read_wasm_api_version(wasm: &[u8]) -> Option<(u16, u16, u16)> {
    // WebAssembly binary header is `\0asm` (4 bytes) + 4-byte version/layer.
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return None;
    }
    let is_component = wasm[4..8] == [0x0d, 0x00, 0x01, 0x00];
    let mut pos = 8usize;
    while pos < wasm.len() {
        let section_id = *wasm.get(pos)?;
        pos += 1;
        let payload_len = read_leb128(&wasm[pos..], &mut pos)?;
        let end = pos.checked_add(payload_len)?;
        if end > wasm.len() {
            return None;
        }
        if section_id == 0 {
            let mut inner = pos;
            let name_len = read_leb128(&wasm[inner..end], &mut inner)?;
            let name_end = inner.checked_add(name_len)?;
            if name_end <= end
                && let Ok(name) = std::str::from_utf8(&wasm[inner..name_end])
                && name == "zed:api-version"
                && end - name_end >= 6
            {
                let b = &wasm[name_end..name_end + 6];
                return Some((
                    u16::from_be_bytes([b[0], b[1]]),
                    u16::from_be_bytes([b[2], b[3]]),
                    u16::from_be_bytes([b[4], b[5]]),
                ));
            }
        } else if is_component
            && section_id == 1
            && let Some(ver) = read_wasm_api_version(&wasm[pos..end])
        {
            return Some(ver);
        }
        pos = end;
    }
    None
}

fn read_leb128(slice: &[u8], cursor: &mut usize) -> Option<usize> {
    let mut result = 0usize;
    let mut shift = 0u32;
    for (idx, &byte) in slice.iter().enumerate() {
        if shift >= 32 {
            return None;
        }
        result |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            *cursor += idx + 1;
            return Some(result);
        }
        shift += 7;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toml_manifest_and_populates_defaults() {
        let dir = std::env::temp_dir().join(format!("coiman-zed-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("languages/demo")).unwrap();
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::create_dir_all(dir.join("icon_themes")).unwrap();
        std::fs::create_dir_all(dir.join("grammars")).unwrap();
        std::fs::write(
            dir.join("extension.toml"),
            r#"
id = "demo-ext"
name = "Demo Extension"
version = "1.2.3"
schema_version = 1
description = "Demo"
authors = ["Alice"]
repository = "https://github.com/example/demo"

[language_servers.demo-ls]
languages = ["Demo"]
"#,
        )
        .unwrap();
        std::fs::write(dir.join("languages/demo/config.toml"), "name = \"Demo\"\npath_suffixes = [\"demo\"]\n")
            .unwrap();
        std::fs::write(dir.join("themes/demo.json"), "{}").unwrap();
        std::fs::write(dir.join("icon_themes/icons.json"), "{}").unwrap();
        std::fs::write(dir.join("snippets.json"), "{}").unwrap();
        std::fs::write(dir.join("grammars/demo.wasm"), b"").unwrap();

        let m = ExtensionManifest::load(&dir).unwrap();
        assert_eq!(m.id, "demo-ext");
        assert_eq!(m.themes, vec![PathBuf::from("themes/demo.json")]);
        assert_eq!(m.icon_themes, vec![PathBuf::from("icon_themes/icons.json")]);
        assert_eq!(m.languages, vec![PathBuf::from("languages/demo")]);
        assert!(m.grammars.contains_key("demo"));
        assert!(m.snippets.is_some());
        assert!(m.provides().contains(&"language-servers".to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_zed_api_version_custom_section() {
        let mut wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];
        let name = b"zed:api-version";
        let payload_len = 1 + name.len() + 6;
        wasm.push(0x00); // custom section
        wasm.push(payload_len as u8);
        wasm.push(name.len() as u8);
        wasm.extend_from_slice(name);
        wasm.extend_from_slice(&0u16.to_be_bytes());
        wasm.extend_from_slice(&7u16.to_be_bytes());
        wasm.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(read_wasm_api_version(&wasm), Some((0, 7, 0)));
    }

    #[test]
    fn parses_real_toml_extension_if_present() {
        let path = Path::new("/tmp/toml_ext");
        if path.join("extension.toml").is_file() {
            let m = ExtensionManifest::load(path).unwrap();
            assert!(!m.id.is_empty());
            if path.join("extension.wasm").is_file() {
                let wasm = std::fs::read(path.join("extension.wasm")).unwrap();
                assert!(read_wasm_api_version(&wasm).is_some());
            }
        }
    }
}
