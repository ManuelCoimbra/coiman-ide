//! Zed icon theme (`https://zed.dev/schema/icon_themes/v0.3.0.json`) loader and packer
//! into Coiman [`file_icons`] packs.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::appearance::file_icons;
use crate::appearance::theme::strip_jsonc;

use super::registry::InstalledExtension;

#[derive(Debug, Deserialize)]
struct ZedIconThemeFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    themes: Vec<ZedIconThemeVariant>,
}

#[derive(Debug, Deserialize)]
struct ZedIconThemeVariant {
    name: String,
    #[serde(default)]
    appearance: String,
    #[serde(default)]
    directory_icons: Option<ZedDirectoryIcons>,
    #[serde(default)]
    file_stems: BTreeMap<String, String>,
    #[serde(default)]
    file_suffixes: BTreeMap<String, String>,
    #[serde(default)]
    file_icons: BTreeMap<String, ZedIconAsset>,
}

#[derive(Debug, Deserialize)]
struct ZedDirectoryIcons {
    #[serde(default)]
    collapsed: Option<String>,
    #[serde(default)]
    expanded: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ZedIconAsset {
    path: String,
}

/// Checks whether a parsed JSON value matches Zed's icon theme schema.
pub fn looks_like_zed_icon_theme(value: &Value) -> bool {
    value
        .get("$schema")
        .and_then(Value::as_str)
        .is_some_and(|s| s.contains("zed.dev/schema/icon_themes"))
        || value
            .get("themes")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|t| t.get("file_icons").is_some() || t.get("file_suffixes").is_some()))
}

/// Converts a Zed `icon_themes/*.json` file into Coiman's packed icon-pack format.
/// Returns a single `(label, packed_text)` entry (combining dark + light variants when both exist).
pub fn pack_from_zed(json_path: &Path, ext_root: &Path) -> Result<(String, String), String> {
    let text = std::fs::read_to_string(json_path).map_err(|e| format!("{}: {e}", json_path.display()))?;
    let file: ZedIconThemeFile = serde_json::from_str(&strip_jsonc(&text))
        .map_err(|e| format!("{}: {e}", json_path.display()))?;
    if file.themes.is_empty() {
        return Err(format!("{}: no icon theme entries", json_path.display()));
    }

    let dark = file
        .themes
        .iter()
        .find(|t| !t.appearance.eq_ignore_ascii_case("light"))
        .unwrap_or(&file.themes[0]);
    let light = file.themes.iter().find(|t| t.appearance.eq_ignore_ascii_case("light"));

    let label = if dark.name.is_empty() { file.name.clone() } else { dark.name.clone() };
    let label = if label.is_empty() {
        json_path.file_stem().and_then(|s| s.to_str()).unwrap_or("Zed Icons").to_owned()
    } else {
        label
    };

    let mut svgs: BTreeMap<String, String> = BTreeMap::new();
    let mut lines = vec![format!("L\tlabel\t{}", label.replace(['\t', '\n'], " "))];

    let base_dir = json_path.parent().unwrap_or(ext_root);
    let resolve_svg = |rel: &str| -> Option<String> {
        let trimmed = rel.trim_start_matches("./");
        let candidates = [ext_root.join(trimmed), base_dir.join(trimmed)];
        let path = candidates.into_iter().find(|p| p.is_file())?;
        let raw = std::fs::read_to_string(path).ok()?;
        let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        compact.contains("<svg").then_some(compact)
    };

    let mut add_variant = |variant: &ZedIconThemeVariant, prefix: &str, name_tag: char, ext_tag: char, def_tag: char| {
        let mut icon_keys: BTreeMap<String, String> = BTreeMap::new();
        for (key, asset) in &variant.file_icons {
            if let Some(svg) = resolve_svg(&asset.path) {
                let pack_key = format!("{prefix}{key}");
                svgs.insert(pack_key.clone(), svg);
                icon_keys.insert(key.clone(), pack_key);
            }
        }
        if let Some(default_key) = icon_keys.get("default") {
            lines.push(format!("{def_tag}\tfile\t{default_key}"));
        }
        if let Some(folder) = variant
            .directory_icons
            .as_ref()
            .and_then(|d| d.collapsed.as_deref().or(d.expanded.as_deref()))
            && let Some(svg) = resolve_svg(folder)
        {
            let folder_key = format!("{prefix}__folder__");
            svgs.insert(folder_key.clone(), svg);
            lines.push(format!("{def_tag}\tfolder\t{folder_key}"));
        }
        for (stem, key) in &variant.file_stems {
            if let Some(pack_key) = icon_keys.get(key) {
                lines.push(format!("{name_tag}\t{}\t{pack_key}", stem.to_ascii_lowercase()));
            }
        }
        for (suffix, key) in &variant.file_suffixes {
            if let Some(pack_key) = icon_keys.get(key) {
                lines.push(format!("{ext_tag}\t{}\t{pack_key}", suffix.to_ascii_lowercase()));
            }
        }
    };

    add_variant(dark, "", 'N', 'E', 'D');
    if let Some(light) = light
        && !std::ptr::eq(dark, light)
    {
        add_variant(light, "light-", 'n', 'e', 'd');
    }

    if svgs.is_empty() {
        return Err(format!("{}: no SVG icon assets found", json_path.display()));
    }
    for (k, svg) in svgs {
        lines.push(format!("I\t{k}\t{svg}"));
    }
    Ok((label, lines.join("\n") + "\n"))
}

/// Syncs all icon packs belonging to active Zed extensions into Coiman's `icon_packs/` folder.
/// Removes any `zed-*` packs whose extension is missing or disabled.
pub fn sync_all_icon_packs(enabled: bool, extensions: &[InstalledExtension]) {
    file_icons::remove_packs_with_prefix("zed-");
    if !enabled {
        return;
    }
    for ext in extensions {
        for (idx, rel) in ext.manifest.icon_themes.iter().enumerate() {
            let path = ext.path.join(rel);
            if let Ok((_, packed)) = pack_from_zed(&path, &ext.path) {
                let pack_id = format!("zed-{}-{idx}", sanitize_id(&ext.id));
                let _ = file_icons::install_pack_with_id(&pack_id, &packed);
            }
        }
    }
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_zed_icon_theme_with_svgs() {
        let dir = std::env::temp_dir().join(format!("coiman-zed-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("icon_themes")).unwrap();
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        std::fs::write(dir.join("icons/rust.svg"), "<svg viewBox=\"0 0 16 16\"><path d=\"M0 0\"/></svg>").unwrap();
        std::fs::write(dir.join("icons/file.svg"), "<svg viewBox=\"0 0 16 16\"><rect width=\"8\" height=\"8\"/></svg>").unwrap();
        let json = dir.join("icon_themes/test.json");
        std::fs::write(
            &json,
            r#"{
                "$schema": "https://zed.dev/schema/icon_themes/v0.3.0.json",
                "name": "Test Icons",
                "themes": [{
                    "name": "Test Icons",
                    "appearance": "dark",
                    "file_stems": { "Makefile": "rust" },
                    "file_suffixes": { "rs": "rust" },
                    "file_icons": {
                        "default": { "path": "./icons/file.svg" },
                        "rust": { "path": "./icons/rust.svg" }
                    }
                }]
            }"#,
        )
        .unwrap();

        let (label, packed) = pack_from_zed(&json, &dir).unwrap();
        assert_eq!(label, "Test Icons");
        assert!(packed.contains("E\trs\trust"));
        assert!(packed.contains("N\tmakefile\trust"));
        assert!(packed.contains("I\trust\t<svg"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
