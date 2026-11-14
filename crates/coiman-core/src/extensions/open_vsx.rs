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
    fn searches_ask_for_themes_and_keep_only_safe_answers() {
        assert_eq!(
            search_url("tokyo night & co"),
            "https://open-vsx.org/api/-/search?category=Themes&size=40&sortBy=downloadCount&sortOrder=desc&query=tokyo+night+%26+co"
        );
        let json = r#"{"extensions":[
            {"namespace":"enkia","name":"tokyo-night","displayName":"Tokyo Night","description":"A clean theme","version":"1.1.2","downloadCount":5,"verified":true,
             "files":{"download":"https://open-vsx.org/api/enkia/tokyo-night/1.1.2/file/enkia.tokyo-night-1.1.2.vsix"}},
            {"namespace":"evil","name":"x","version":"1.0.0","files":{"download":"https://example.com/x.vsix"}},
            {"namespace":"a b","name":"x","version":"1","files":{"download":"https://open-vsx.org/api/x"}},
            {"namespace":"old","name":"x","version":"1","deprecated":true,"files":{"download":"https://open-vsx.org/api/x"}}
        ]}"#;
        let found = parse_search(json).unwrap();
        assert_eq!(found.len(), 1, "another site, an odd name and a deprecated one are left out");
        assert_eq!(found[0].id(), "enkia.tokyo-night");
        assert!(found[0].verified);
        assert!(parse_search(r#"{"error":"bad query"}"#).unwrap_err().contains("bad query"));
        assert!(fetch("https://example.com/", None).unwrap_err().contains("Only open-vsx.org"));
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
