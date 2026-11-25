//! Color and icon themes from Open VSX (open-vsx.org), the open registry of VS Code
//! extensions. Only themes: an extension's snippets, languages and code are never taken.
//!
//! The app opens no connections itself: `curl`, the program on the computer (Windows 10 and
//! later, macOS and Linux have it), fetches the search results and the package, from
//! open-vsx.org only and only over HTTPS.

use std::path::{Path, PathBuf};

use serde_json::Value;

const SITE: &str = "https://open-vsx.org/";

/// An extension the search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub namespace: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub downloads: u64,
    pub verified: bool,
    pub download: String,
    pub icon: String,
    pub rating: u64,
    pub categories: Vec<String>,
    pub url: String,
    pub timestamp: String,
}

/// Everything a detail page shows, from Open VSX's `latest` answer and its README.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub version: String,
    pub publisher: String,
    pub downloads: u64,
    pub verified: bool,
    pub rating: u64,
    pub tags: Vec<String>,
    pub categories: Vec<String>,
    pub repository: String,
    pub homepage: String,
    pub license: String,
    pub engines: String,
    pub dependencies: Vec<String>,
    pub versions: Vec<String>,
    pub readme: String,
    pub changelog: String,
    pub icon: String,
    pub download: String,
    pub sha256: String,
    pub timestamp: String,
    /// The extension's own package.json, for the Features and Pack pages.
    pub manifest: Value,
}

impl Found {
    pub fn id(&self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }
}

/// What an install added.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    /// Color themes, by label.
    pub themes: Vec<String>,
    /// Icon packs: (id, label).
    pub icon_packs: Vec<(String, String)>,
    /// Icon themes that couldn't be used, and why.
    pub skipped: Vec<String>,
}

/// Letters, digits, `-`, `_` and `.`: what Open VSX allows in names, and all a URL gets.
fn plain(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The search address for theme extensions.
pub fn search_url(query: &str) -> String {
    let encoded: String = query
        .trim()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_owned(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("{SITE}api/-/search?category=Themes&size=40&sortBy=downloadCount&sortOrder=desc&query={encoded}")
}

/// A search of all extensions, not just themes: what the marketplace browses.
pub fn browse_url(query: &str, category: Option<&str>, size: usize, offset: usize) -> String {
    let encoded: String = query
        .trim()
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            b' ' => "+".to_owned(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let category = category
        .filter(|c| {
            !c.trim().is_empty() && c.chars().all(|c| c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || c == '-')
        })
        .map(|c| format!("category={}&", c.trim().replace(' ', "+")))
        .unwrap_or_default();
    format!(
        "{SITE}api/-/search?{category}size={size}&offset={offset}&sortBy=downloadCount&sortOrder=desc&query={encoded}"
    )
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// Reads the search's answer, keeping only extensions with names and a package on open-vsx.org.
pub fn parse_search(json: &str) -> Result<Vec<Found>, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| format!("Open VSX answered something unreadable: {e}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("Open VSX: {error}"));
    }
    let text = |e: &Value, key: &str| e.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(value
        .get("extensions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|e| !e.get("deprecated").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|e| {
            let found = Found {
                namespace: text(e, "namespace"),
                name: text(e, "name"),
                display_name: text(e, "displayName"),
                description: text(e, "description"),
                version: text(e, "version"),
                downloads: e.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
                verified: e.get("verified").and_then(Value::as_bool).unwrap_or(false),
                download: e.pointer("/files/download").and_then(Value::as_str).unwrap_or_default().to_owned(),
                icon: e.pointer("/files/icon").and_then(Value::as_str).unwrap_or_default().to_owned(),
                rating: (e.get("averageRating").and_then(Value::as_f64).unwrap_or(0.0) * 10.0).round() as u64,
                categories: strings(e.get("categories")),
                url: text(e, "url"),
                timestamp: text(e, "timestamp"),
            };
            let ok = plain(&found.namespace)
                && plain(&found.name)
                && plain(&found.version)
                && found.download.starts_with(&format!("{SITE}api/"));
            ok.then_some(found)
        })
        .collect())
}

/// The `curl` program.
fn curl() -> Option<PathBuf> {
    // Windows' own, ahead of others on the path (Git's can lack the system's certificates).
    let system = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("curl.exe"));
    system.filter(|p| p.exists()).or_else(|| crate::language::lsp::servers::which("curl"))
}

/// Runs curl for an open-vsx.org address: into `out`, or its answer as text.
fn fetch(url: &str, out: Option<&Path>) -> Result<String, String> {
    if !url.starts_with(SITE) {
        return Err(format!("Only open-vsx.org is used, not {url}"));
    }
    let program = curl().ok_or("The curl program isn't on this computer, so Open VSX can't be reached")?;
    let mut cmd = crate::language::lsp::discovery::quiet_command(&program);
    cmd.args(["--silent", "--show-error", "--fail", "--location", "--proto", "=https", "--proto-redir", "=https"]);
    cmd.args(["--max-time", "120", "--max-filesize", "104857600"]);
    if let Some(out) = out {
        cmd.arg("--output").arg(out);
    }
    cmd.arg(url);
    let done = cmd.output().map_err(|e| format!("Couldn't start curl: {e}"))?;
    if !done.status.success() {
        let why = String::from_utf8_lossy(&done.stderr).trim().to_owned();
        return Err(if why.is_empty() { format!("curl stopped ({})", done.status) } else { format!("curl: {why}") });
    }
    Ok(String::from_utf8_lossy(&done.stdout).into_owned())
}

/// Searches Open VSX for theme extensions.
pub fn search(query: &str) -> Result<Vec<Found>, String> {
    parse_search(&fetch(&search_url(query), None)?)
}

/// Browses all extensions, with an optional Open VSX category such as `Linters`.
pub fn browse(query: &str, category: Option<&str>, size: usize, offset: usize) -> Result<Vec<Found>, String> {
    parse_search(&fetch(&browse_url(query, category, size, offset), None)?)
}

/// An image (an icon or a README picture) from open-vsx.org, as bytes.
pub fn fetch_image(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with(SITE) {
        return None;
    }
    // Kept on disk, so a picture is fetched once (`<config>/cache/open-vsx`).
    let cached = crate::settings::config_dir().map(|d| d.join("cache").join("open-vsx").join(cache_name(url)));
    if let Some(bytes) = cached.as_deref().and_then(|c| std::fs::read(c).ok()) {
        return Some(bytes);
    }
    let file = std::env::temp_dir().join(format!("coiman-image-{}-{}", std::process::id(), sanitize(url)));
    if fetch(url, Some(&file)).is_err() {
        return None;
    }
    let bytes = std::fs::read(&file).ok();
    let _ = std::fs::remove_file(&file);
    if let (Some(cached), Some(bytes)) = (&cached, &bytes)
        && let Some(dir) = cached.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = std::fs::write(cached, bytes);
    }
    bytes
}

/// A file name for a cached address: readable, and unique by a hash of the whole of it.
fn cache_name(url: &str) -> String {
    // FNV-1a: stable from run to run, unlike the standard hasher.
    let hash = url.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{}-{hash:016x}", sanitize(url))
}

fn sanitize(url: &str) -> String {
    url.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).take(80).collect()
}

/// A text file an extension's detail page points at, only on open-vsx.org.
fn fetch_file(value: &Value, key: &str) -> String {
    value
        .pointer(&format!("/files/{key}"))
        .and_then(Value::as_str)
        .filter(|url| url.starts_with(SITE))
        .and_then(|url| fetch(url, None).ok())
        .unwrap_or_default()
}

/// An extension's detail page, including the versions on Open VSX.
pub fn detail(namespace: &str, name: &str) -> Result<Detail, String> {
    if !plain(namespace) || !plain(name) {
        return Err("That isn't an Open VSX extension name".to_owned());
    }
    let url = format!("{SITE}api/{namespace}/{name}/latest");
    let value: Value = serde_json::from_str(&fetch(&url, None)?)
        .map_err(|e| format!("Open VSX answered something unreadable: {e}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("Open VSX: {error}"));
    }
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(Detail {
        id: format!("{namespace}.{name}"),
        display_name: text("displayName"),
        description: text("description"),
        version: text("version"),
        publisher: value
            .get("publishedBy")
            .and_then(|p| p.get("loginName"))
            .and_then(Value::as_str)
            .unwrap_or(namespace)
            .to_owned(),
        downloads: value.get("downloadCount").and_then(Value::as_u64).unwrap_or(0),
        verified: value.get("verified").and_then(Value::as_bool).unwrap_or(false),
        rating: (value.get("averageRating").and_then(Value::as_f64).unwrap_or(0.0) * 10.0).round() as u64,
        tags: strings(value.get("tags")),
        categories: strings(value.get("categories")),
        repository: text("repository"),
        homepage: text("homepage"),
        license: text("license"),
        engines: value.pointer("/engines/vscode").and_then(Value::as_str).unwrap_or_default().to_owned(),
        dependencies: strings(value.get("dependencies")),
        versions: value
            .get("allVersions")
            .and_then(Value::as_object)
            .map(|versions| {
                versions.keys().filter(|k| k.as_str() != "latest" && k.as_str() != "pre-release").cloned().collect()
            })
            .unwrap_or_default(),
        readme: fetch_file(&value, "readme"),
        changelog: fetch_file(&value, "changelog"),
        icon: value.pointer("/files/icon").and_then(Value::as_str).unwrap_or_default().to_owned(),
        download: value.pointer("/files/download").and_then(Value::as_str).unwrap_or_default().to_owned(),
        sha256: value.pointer("/files/sha256").and_then(Value::as_str).unwrap_or_default().to_owned(),
        timestamp: text("timestamp"),
        manifest: value
            .pointer("/files/manifest")
            .and_then(Value::as_str)
            .filter(|url| url.starts_with(SITE))
            .and_then(|url| fetch(url, None).ok())
            .and_then(|body| serde_json::from_str(&body).ok())
            .unwrap_or(Value::Null),
    })
}

/// What installing a full extension added: imported data and what it can do here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtensionInstalled {
    pub id: String,
    pub version: String,
    pub folder: std::path::PathBuf,
    pub data: Installed,
    pub capability: String,
}

/// The icon themes an unpacked extension declares: (label, theme file).
fn icon_themes(dir: &Path) -> Vec<(String, PathBuf)> {
    let Some(package) = std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&crate::appearance::theme::strip_jsonc(&t)).ok())
    else {
        return Vec::new();
    };
    package
        .pointer("/contributes/iconThemes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| {
            let path = dir.join(t.get("path")?.as_str()?);
            let label = t.get("label").or_else(|| t.get("id")).and_then(Value::as_str)?.to_owned();
            // Within the extension only.
            let root = dir.canonicalize().ok()?;
            path.canonicalize().ok().filter(|p| p.starts_with(&root)).map(|p| (label, p))
        })
        .collect()
}

/// Takes the color and icon themes out of an unpacked extension; nothing else in it.
pub fn take_themes(dir: &Path) -> Result<Installed, String> {
    let mut done = Installed::default();
    let mut contents = crate::extensions::vscode_import::read(dir)?;
    contents.snippets.clear();
    contents.languages.clear();
    done.themes = contents.themes.iter().map(|t| t.0.clone()).collect();
    if !contents.themes.is_empty() {
        crate::extensions::vscode_import::install(contents)?;
    }
    for (label, theme) in icon_themes(dir) {
        match crate::appearance::file_icons::pack_from_vscode(&theme, None, &label)
            .and_then(|text| crate::appearance::file_icons::install_pack(&label, &text))
        {
            Ok(id) => done.icon_packs.push((id, label)),
            Err(e) => done.skipped.push(format!("{label}: {e}")),
        }
    }
    if done.themes.is_empty() && done.icon_packs.is_empty() {
        return Err(match done.skipped.first() {
            Some(why) => why.clone(),
            None => "This extension has no color or icon themes".to_owned(),
        });
    }
    Ok(done)
}

/// Compares the package with the SHA-256 Open VSX publishes beside it, when it does.
fn check_sha256(download: &str, file: &Path) -> Result<(), String> {
    use sha2::Digest;
    let Some(url) = download.strip_suffix(".vsix").map(|base| format!("{base}.sha256")) else { return Ok(()) };
    // Older packages have none; the download itself came over HTTPS from open-vsx.org.
    let Ok(text) = fetch(&url, None) else { return Ok(()) };
    let Some(expected) = text.split_whitespace().next().map(str::to_lowercase) else { return Ok(()) };
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let actual: String = sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if actual == expected {
        Ok(())
    } else {
        Err("The package doesn't match its published SHA-256: not installed".to_owned())
    }
}

/// Downloads a full extension, keeps it in Coiman's own extensions folder, imports
/// whatever data it holds, and reports what of its code can run here.
pub fn install_extension(found: &Found) -> Result<ExtensionInstalled, String> {
    let folder_root =
        crate::extensions::vscode_import::coiman_extensions_dir().ok_or("Couldn't find Coiman's settings folder")?;
    std::fs::create_dir_all(&folder_root).map_err(|e| e.to_string())?;
    let work = std::env::temp_dir().join(format!("coiman-vsix-{}-{}", std::process::id(), found.id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let file = work.join("package.vsix");
    let result = (|| {
        fetch(&found.download, Some(&file))?;
        check_sha256(&found.download, &file)?;
        let unpacked = work.join("unpacked");
        let source = crate::extensions::vscode_import::unpack_vsix(&file, &unpacked)?;
        let id = found.id();
        let target = folder_root.join(format!("{id}-{}", found.version));
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
        }
        copy_dir(&source, &target)?;
        let data = crate::extensions::vscode_import::read(&target).unwrap_or_default();
        let imported = Installed { themes: data.themes.iter().map(|t| t.0.clone()).collect(), ..Default::default() };
        if !data.themes.is_empty() || !data.snippets.is_empty() || !data.languages.is_empty() {
            crate::extensions::vscode_import::install(data)?;
        }
        let capability = crate::extensions::vscode_compat::examine(&target)
            .map(|report| report.summary())
            .unwrap_or_else(|_| "Unknown".to_owned());
        Ok(ExtensionInstalled { id, version: found.version.clone(), folder: target, data: imported, capability })
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

/// Removes an extension downloaded from Open VSX. Data already imported into
/// Coiman (a theme or snippets) is left alone: it may be in use elsewhere.
pub fn uninstall_extension(id: &str) -> Result<(), String> {
    if !plain(id) {
        return Err("That isn't an extension id".to_owned());
    }
    let root = crate::extensions::vscode_import::coiman_extensions_dir().ok_or("No extensions folder")?;
    let prefix = format!("{id}-");
    let mut removed = false;
    for entry in std::fs::read_dir(&root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        // `<id>-<version>`: the version starts with a digit, so `a.b-extra-1.0` isn't `a.b`'s.
        let name = entry.file_name().to_string_lossy().into_owned();
        let versioned = name.strip_prefix(&prefix).is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()));
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) && versioned {
            std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            removed = true;
        }
    }
    if removed { Ok(()) } else { Err("That extension isn't installed in Coiman's folder".to_owned()) }
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let target = to.join(&name);
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Downloads an extension's package from open-vsx.org and installs its themes.
pub fn install(found: &Found) -> Result<Installed, String> {
    let work = std::env::temp_dir().join(format!("coiman-open-vsx-{}-{}", std::process::id(), found.id()));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let file = work.join("package.vsix");
    let result = fetch(&found.download, Some(&file)).and_then(|_| {
        check_sha256(&found.download, &file)?;
        let dir = crate::extensions::vscode_import::unpack_vsix(&file, &work.join("unpacked"))?;
        take_themes(&dir)
    });
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_files_are_read_only_from_open_vsx() {
        let other = serde_json::json!({ "files": { "readme": "https://example.com/README.md" } });
        assert_eq!(fetch_file(&other, "readme"), "", "another site is never fetched");
        assert_eq!(fetch_file(&serde_json::json!({}), "readme"), "");
    }

    #[test]
    fn browsing_all_extensions_keeps_categories_and_ratings() {
        assert_eq!(
            browse_url("rust analyzer", Some("Programming Languages"), 60, 20),
            "https://open-vsx.org/api/-/search?category=Programming+Languages&size=60&offset=20&sortBy=downloadCount&sortOrder=desc&query=rust+analyzer"
        );
        let json = r#"{"extensions":[{"namespace":"rust-lang","name":"rust-analyzer","displayName":"Rust Analyzer",
            "description":"Rust support","version":"1.0.0","downloadCount":42,"averageRating":4.5,"verified":true,
            "categories":["Programming Languages"],"url":"https://open-vsx.org/api/rust-lang/rust-analyzer",
            "timestamp":"2026-01-01T00:00:00Z","files":{"download":"https://open-vsx.org/api/a/b/1/file/a.b.vsix","icon":"https://open-vsx.org/api/a/b/1/file/icon.png"}}]}"#;
        let found = parse_search(json).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].categories, vec!["Programming Languages"]);
        assert_eq!(found[0].rating, 45);
        assert!(found[0].icon.ends_with("icon.png"));
    }

    #[test]
    fn only_themes_come_out_of_an_extension() {
        let dir = std::env::temp_dir().join(format!("coiman-open-vsx-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        std::fs::write(
            dir.join("package.json"),
            r#"{"name":"t","contributes":{
                "iconThemes":[{"id":"t","label":"Test Pack","path":"./icons.json"},{"id":"out","label":"Outside","path":"../elsewhere.json"}],
                "snippets":[{"language":"rust","path":"./s.json"}]}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("icons/f.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#).unwrap();
        std::fs::write(
            dir.join("icons.json"),
            r#"{"iconDefinitions":{"_f":{"iconPath":"./icons/f.svg"}},"file":"_f"}"#,
        )
        .unwrap();
        let themes = icon_themes(&dir);
        assert_eq!(themes.len(), 1, "a theme file outside the extension is ignored");
        assert_eq!(themes[0].0, "Test Pack");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
