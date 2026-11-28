//! Zed snippet loader (`snippets.json` / `snippets/*.json` declared in `extension.toml`).

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::appearance::theme::strip_jsonc;
use crate::editor::snippets::Snippet;

use super::registry::InstalledExtension;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    One(String),
    Many(Vec<String>),
}

impl StringOrVec {
    fn first(&self) -> String {
        match self {
            Self::One(s) => s.clone(),
            Self::Many(v) => v.first().cloned().unwrap_or_default(),
        }
    }

    fn joined(&self) -> String {
        match self {
            Self::One(s) => s.clone(),
            Self::Many(v) => v.join("\n"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ZedSnippetEntry {
    prefix: StringOrVec,
    body: StringOrVec,
}

/// Loads all snippets from enabled Zed extensions.
pub fn collect_snippets(extensions: &[InstalledExtension]) -> Vec<Snippet> {
    let mut out = Vec::new();
    for ext in extensions {
        let Some(snippets) = &ext.manifest.snippets else { continue };
        for rel in snippets.paths() {
            let path = ext.path.join(rel);
            let lang = language_from_snippet_path(&path);
            if let Ok(items) = parse_snippet_file(&path, &lang) {
                out.extend(items);
            }
        }
    }
    out
}

/// Parses a single Zed snippet JSON/JSONC file into Coiman [`Snippet`] items.
pub fn parse_snippet_file(path: &Path, language: &str) -> Result<Vec<Snippet>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_snippet_json(&text, language).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn parse_snippet_json(text: &str, language: &str) -> Result<Vec<Snippet>, String> {
    let stripped = strip_jsonc(text);
    let map: BTreeMap<String, ZedSnippetEntry> = serde_json::from_str(&stripped).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(map.len());
    for (name, entry) in map {
        let prefix = entry.prefix.first();
        let body = entry.body.joined();
        if prefix.trim().is_empty() || body.is_empty() {
            continue;
        }
        out.push(Snippet { name, prefix, language: language.to_owned(), body });
    }
    Ok(out)
}

fn language_from_snippet_path(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if stem.is_empty() || stem.eq_ignore_ascii_case("snippets") {
        String::new()
    } else {
        stem.replace('_', " ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_zed_snippets_json() {
        let json = r#"{
            "Print Line": {
                "prefix": "pln",
                "body": ["println!(\"{$1}\");", "$0"],
                "description": "Print with newline"
            }
        }"#;
        let list = parse_snippet_json(json, "Rust").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].prefix, "pln");
        assert_eq!(list[0].language, "Rust");
        assert!(list[0].applies_to("Rust"));
        assert!(list[0].body.contains("$1"));
    }
}
