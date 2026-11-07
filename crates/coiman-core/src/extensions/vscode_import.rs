//! Takes what's plain data out of a VS Code extension: color themes, snippets and languages.
//! Its code can't run here, and is never read. Nothing here runs a program from the extension.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::appearance::theme::strip_jsonc;
use crate::editor::snippets::Snippet;

/// A language an extension defines. Never with a server or a command: only names.
#[derive(Debug, PartialEq, Eq)]
pub struct ImportedLanguage {
    pub name: String,
    pub extensions: Vec<String>,
    pub comment: Option<String>,
}

/// What an extension holds.
#[derive(Debug, Default)]
pub struct Contents {
    pub name: String,
    pub themes: Vec<(String, Value)>,
    pub snippets: Vec<Snippet>,
    pub languages: Vec<ImportedLanguage>,
    /// Text colors for languages, which need an engine that isn't here.
    pub grammars: usize,
    /// It has a program of its own, which can't run here.
    pub has_code: bool,
}

impl Contents {
    pub fn is_useful(&self) -> bool {
        !self.themes.is_empty() || !self.snippets.is_empty() || !self.languages.is_empty()
    }

    /// `2 themes, 40 snippets, 1 language`.
    pub fn describe(&self) -> String {
        let count = |n: usize, what: &str| match n {
            0 => None,
            1 => Some(format!("1 {what}")),
            n => Some(format!("{n} {what}s")),
        };
        let parts = [
            count(self.themes.len(), "theme"),
            count(self.snippets.len(), "snippet"),
            count(self.languages.len(), "language"),
        ];
        let parts: Vec<String> = parts.into_iter().flatten().collect();
        if parts.is_empty() { "nothing that can come across".to_owned() } else { parts.join(", ") }
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&strip_jsonc(text.trim_start_matches('\u{feff}'))).ok()
}

/// A path an extension names, if it stays inside the extension.
fn inside(dir: &Path, relative: &str) -> Option<PathBuf> {
    let root = dir.canonicalize().ok()?;
    let path = dir.join(relative).canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

/// A name that may be `%key%`, looked up in the extension's own strings.
fn localized(text: &str, strings: Option<&Value>) -> Option<String> {
    match text.strip_prefix('%').and_then(|t| t.strip_suffix('%')) {
        Some(key) => strings?.get(key)?.as_str().map(str::to_owned),
        None => Some(text.to_owned()),
    }
}

/// A theme with what it includes merged in, so that it stands alone.
fn whole_theme(dir: &Path, path: &Path, depth: usize) -> Option<Value> {
    let mut theme = read_json(path)?;
    let include = theme.get("include").and_then(Value::as_str).map(str::to_owned);
    let Some(include) = include.filter(|_| depth < 8) else { return Some(theme) };
    let folder = path.parent()?.strip_prefix(dir.canonicalize().ok()?).ok()?;
    let base = inside(dir, &folder.join(&include).to_string_lossy())?;
    let mut merged = whole_theme(dir, &base, depth + 1)?;
    let (object, base_object) = (theme.as_object_mut()?, merged.as_object_mut()?);
    object.remove("include");
    for (key, value) in std::mem::take(object) {
        match (key.as_str(), base_object.get_mut(&key), value) {
            ("colors", Some(Value::Object(old)), Value::Object(new)) => old.extend(new),
            ("tokenColors", Some(Value::Array(old)), Value::Array(new)) => old.extend(new),
            (_, _, value) => {
                base_object.insert(key, value);
            }
        }
    }
    Some(merged)
}

fn themes_of(dir: &Path, package: &Value, strings: Option<&Value>) -> Vec<(String, Value)> {
    let list = package.pointer("/contributes/themes").and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|entry| {
            let label = entry.get("label").or_else(|| entry.get("id")).and_then(Value::as_str)?;
            let label = localized(label, strings)?;
            let path = inside(dir, entry.get("path")?.as_str()?)?;
            let mut theme = whole_theme(dir, &path, 0)?;
            // The type a theme goes by is sometimes only in the extension's list.
            if theme.get("type").is_none()
                && let (Some(object), Some(ui)) = (theme.as_object_mut(), entry.get("uiTheme").and_then(Value::as_str))
            {
                let kind = if ui == "vs" || ui.contains("light") { "light" } else { "dark" };
                object.insert("type".to_owned(), Value::String(kind.to_owned()));
            }
            Some((label, theme))
        })
        .collect()
}

fn snippets_of(dir: &Path, package: &Value) -> Vec<Snippet> {
    // The same file is often listed once per language.
    let mut files: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for entry in package.pointer("/contributes/snippets").and_then(Value::as_array).into_iter().flatten() {
        let (Some(path), Some(language)) =
            (entry.get("path").and_then(Value::as_str), entry.get("language").and_then(Value::as_str))
        else {
            continue;
        };
        files.entry(path.to_owned()).or_default().push(language.to_owned());
    }
    let mut out = Vec::new();
    for (path, languages) in files {
        let Some(json) = inside(dir, &path).and_then(|p| read_json(&p)) else { continue };
        let Some(entries) = json.as_object() else { continue };
        out.extend(entries.iter().flat_map(|(name, entry)| snippet_entries(name, entry, &languages.join(","))));
    }
    out
}

/// One snippet for each prefix an entry has.
fn snippet_entries(name: &str, entry: &Value, language: &str) -> Vec<Snippet> {
    let body = match entry.get("body") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(lines)) => lines.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"),
        _ => return Vec::new(),
    };
    let prefixes: Vec<&str> = match entry.get("prefix") {
        Some(Value::String(one)) => vec![one],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let make = |prefix: &&str| Snippet {
        name: name.to_owned(),
        prefix: (*prefix).to_owned(),
        language: language.to_owned(),
        body: body.clone(),
    };
    prefixes.iter().map(make).collect()
}

fn languages_of(dir: &Path, package: &Value) -> Vec<ImportedLanguage> {
    let list = package.pointer("/contributes/languages").and_then(Value::as_array);
    list.into_iter()
        .flatten()
        .filter_map(|entry| {
            let extensions: Vec<String> = entry
                .get("extensions")?
                .as_array()?
                .iter()
                .filter_map(Value::as_str)
                .map(|e| e.trim_start_matches('.').to_lowercase())
                .collect();
            if extensions.is_empty() {
                return None;
            }
            let id = entry.get("id")?.as_str()?;
            let alias = entry.pointer("/aliases/0").and_then(Value::as_str);
            let comment = entry
                .get("configuration")
                .and_then(Value::as_str)
                .and_then(|p| inside(dir, p))
                .and_then(|p| read_json(&p))
                .and_then(|c| match c.pointer("/comments/lineComment") {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Object(o)) => o.get("comment").and_then(Value::as_str).map(str::to_owned),
                    _ => None,
                });
            Some(ImportedLanguage { name: alias.unwrap_or(id).to_owned(), extensions, comment })
        })
        .collect()
}

/// Reads an unpacked extension.
pub fn read(dir: &Path) -> Result<Contents, String> {
    let package = read_json(&dir.join("package.json")).ok_or("This isn't a VS Code extension: no package.json")?;
    let strings = read_json(&dir.join("package.nls.json"));
    let folder = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let title = package.get("displayName").or_else(|| package.get("name")).and_then(Value::as_str);
    Ok(Contents {
        name: title.and_then(|t| localized(t, strings.as_ref())).unwrap_or(folder),
        themes: themes_of(dir, &package, strings.as_ref()),
        snippets: snippets_of(dir, &package),
        languages: languages_of(dir, &package),
        grammars: package.pointer("/contributes/grammars").and_then(Value::as_array).map_or(0, Vec::len),
        has_code: package.get("main").is_some() || package.get("browser").is_some(),
    })
}

/// The extensions VS Code has installed that hold something that can come across.
pub fn installed() -> Vec<(PathBuf, Contents)> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    let Some(entries) = home.and_then(|h| std::fs::read_dir(h.join(".vscode").join("extensions")).ok()) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Contents)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter_map(|dir| read(&dir).ok().filter(Contents::is_useful).map(|contents| (dir, contents)))
        .collect();
    found.sort_by_key(|(_, c)| c.name.to_lowercase());
    found
}

/// Unpacks a `.vsix`, which is a zip file, and returns the extension's folder in `into`.
pub fn unpack_vsix(file: &Path, into: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    // Git's tar may come first on the path, and can't read zip files.
    let system = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32").join("tar.exe"));
    let tar = system.filter(|p| p.exists()).unwrap_or_else(|| PathBuf::from("tar"));
    let status = crate::language::lsp::discovery::quiet_command(&tar).arg("-xf").arg(file).arg("-C").arg(into).status();
    match status {
        Ok(done) if done.success() => {}
        Ok(done) => return Err(format!("Couldn't unpack {}: tar said {done}", file.display())),
        Err(e) => return Err(format!("Couldn't unpack {}: {e}", file.display())),
    }
    let folder = into.join("extension");
    if folder.join("package.json").exists() { Ok(folder) } else { Err("That file has no extension in it".to_owned()) }
}

/// A file name for a theme's label.
fn file_name_for(label: &str) -> String {
    let safe: String =
        label.chars().map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_') { c } else { '_' }).collect();
    format!("{}.json", safe.trim())
}

/// Adds snippets not there yet: the same prefix for the same languages stays as it is.
fn merged_snippets(mut existing: Vec<Snippet>, new: Vec<Snippet>) -> (Vec<Snippet>, usize) {
    let mut added = 0;
    for snippet in new {
        if !existing.iter().any(|s| s.prefix == snippet.prefix && s.language == snippet.language) {
            existing.push(snippet);
            added += 1;
        }
    }
    (existing, added)
}

/// The entry for `languages.toml`.
fn language_entry(language: &ImportedLanguage) -> String {
    let text = |s: &str| toml::Value::String(s.to_owned()).to_string();
    let extensions: Vec<String> = language.extensions.iter().map(|e| text(e)).collect();
    let mut entry =
        format!("\n[[language]]\nname = {}\nextensions = [{}]\n", text(&language.name), extensions.join(", "));
    if let Some(comment) = &language.comment {
        entry.push_str(&format!("comment = {}\n", text(comment)));
    }
    entry
}

/// What was added, for saying so.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Imported {
    pub themes: usize,
    pub snippets: usize,
    pub languages: usize,
}

/// Puts an extension's data in the config folder, leaving what's already there alone.
pub fn install(contents: Contents) -> Result<Imported, String> {
    let dir = crate::settings::config_dir().ok_or("No config directory available")?;
    let mut done = Imported::default();
    if !contents.themes.is_empty() {
        let folder = dir.join("themes");
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        for (label, theme) in &contents.themes {
            let text = serde_json::to_string_pretty(theme).map_err(|e| e.to_string())?;
            std::fs::write(folder.join(file_name_for(label)), text).map_err(|e| e.to_string())?;
            done.themes += 1;
        }
    }
    if !contents.snippets.is_empty() {
        // A snippets file that's broken says so, and stays untouched.
        let existing = crate::editor::snippets::load()?;
        let (all, added) = merged_snippets(existing, contents.snippets);
        crate::editor::snippets::save(&all)?;
        done.snippets = added;
    }
    // A language that's known already keeps what it has.
    let fresh: Vec<&ImportedLanguage> = contents
        .languages
        .iter()
        .filter(|l| !l.extensions.iter().any(|e| crate::language::lsp::registry::by_extension(e, false).is_some()))
        .collect();
    if !fresh.is_empty() {
        let path = crate::language::lsp::registry::user_file().ok_or("No config directory available")?;
        let mut text =
            std::fs::read_to_string(&path).unwrap_or_else(|_| crate::language::lsp::registry::user_file_template());
        for language in fresh {
            text.push_str(&language_entry(language));
            done.languages += 1;
        }
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes files into a new folder, removed at the end of a test.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("rc_vsc_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for (file, text) in files {
                let path = dir.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            Self(dir)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const PACKAGE: &str = r#"{
        "displayName": "%title%", "main": "./x.js",
        "contributes": {
            "themes": [{ "label": "Mine", "uiTheme": "vs", "path": "./themes/mine.json" },
                       { "label": "Escapes", "path": "../../outside.json" }],
            "snippets": [{ "language": "javascript", "path": "./s.code-snippets" },
                         { "language": "typescript", "path": "./s.code-snippets" }],
            "languages": [{ "id": "scs", "aliases": ["SCS Script"], "extensions": [".SCS2"], "configuration": "./c.json" },
                          { "id": "noext" }],
            "grammars": [{ "language": "scs", "path": "./g.json" }]
        }
    }"#;

    #[test]
    fn reads_what_can_come_across() {
        let folder = Folder::new(
            "read",
            &[
                ("package.json", PACKAGE),
                ("package.nls.json", r#"{ "title": "My Pack" }"#),
                (
                    "themes/mine.json",
                    r##"{ "include": "./base.json", "colors": { "a": "#1", "b": "#2" }, "tokenColors": [2] } // note"##,
                ),
                ("themes/base.json", r##"{ "name": "Base", "colors": { "a": "#0", "c": "#3" }, "tokenColors": [1] }"##),
                (
                    "s.code-snippets",
                    r#"{
                    "Log": { "prefix": ["log", "lg"], "body": ["a(", "$1)"] },
                    "One": { "prefix": "one", "body": "b" },
                    "Broken": { "body": "no prefix" } }"#,
                ),
                ("c.json", r#"{ "comments": { "lineComment": "//" } } // trailing"#),
            ],
        );
        let found = read(&folder.0).unwrap();
        assert_eq!(found.name, "My Pack");
        assert!(found.has_code);
        assert_eq!(found.grammars, 1);
        assert_eq!(found.describe(), "1 theme, 3 snippets, 1 language");
        let (label, theme) = &found.themes[0];
        assert_eq!(label, "Mine");
        assert_eq!(theme["colors"]["a"], "#1");
        assert_eq!(theme["colors"]["c"], "#3");
        assert_eq!(theme["tokenColors"], serde_json::json!([1, 2]));
        assert_eq!(theme["type"], "light");
        assert!(theme.get("include").is_none());
        let log = found.snippets.iter().find(|s| s.prefix == "lg").unwrap();
        assert_eq!((log.body.as_str(), log.language.as_str()), ("a(\n$1)", "javascript,typescript"));
        assert_eq!(
            found.languages,
            [ImportedLanguage {
                name: "SCS Script".into(),
                extensions: vec!["scs2".into()],
                comment: Some("//".into())
            }]
        );
    }

    #[test]
    fn stays_inside_the_extension() {
        let folder = Folder::new("inside", &[("package.json", "{}"), ("a/b.json", "{}")]);
        assert!(inside(&folder.0, "a/b.json").is_some());
        assert!(inside(&folder.0, "../../../../Windows/win.ini").is_none());
        assert!(inside(&folder.0, "missing.json").is_none());
    }

    #[test]
    fn says_so_when_it_is_no_extension() {
        let folder = Folder::new("none", &[("a.txt", "x")]);
        assert!(read(&folder.0).unwrap_err().contains("package.json"));
        let code_only = Folder::new("code", &[("package.json", r#"{ "name": "c", "main": "x.js" }"#)]);
        let found = read(&code_only.0).unwrap();
        assert!(!found.is_useful() && found.has_code);
        assert_eq!(found.describe(), "nothing that can come across");
    }

    #[test]
    fn snippets_already_there_stay() {
        let snippet = |prefix: &str, body: &str| Snippet {
            name: "n".into(),
            prefix: prefix.into(),
            language: "js".into(),
            body: body.into(),
        };
        let (all, added) =
            merged_snippets(vec![snippet("a", "mine")], vec![snippet("a", "theirs"), snippet("b", "new")]);
        assert_eq!(added, 1);
        assert_eq!(all[0].body, "mine");
        assert_eq!(all[1].prefix, "b");
    }

    #[test]
    fn language_entries_are_valid_and_hold_no_commands() {
        let entry = language_entry(&ImportedLanguage {
            name: "Quote \" Lang".into(),
            extensions: vec!["q".into(), "qq".into()],
            comment: Some("#".into()),
        });
        let parsed: toml::Table = toml::from_str(&entry).unwrap();
        let language = &parsed["language"][0];
        assert_eq!(language["name"].as_str(), Some("Quote \" Lang"));
        assert_eq!(language["extensions"].as_array().unwrap().len(), 2);
        assert!(language.get("server").is_none() && language.get("run").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn unpacks_a_vsix() {
        let folder = Folder::new("vsix", &[("src/extension/package.json", r#"{ "name": "z" }"#)]);
        let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join("tar.exe");
        let file = folder.0.join("z.vsix");
        let made = std::process::Command::new(tar)
            .args(["-a", "--format=zip", "-cf"])
            .arg(&file)
            .arg("-C")
            .arg(folder.0.join("src"))
            .arg("extension")
            .status()
            .unwrap();
        assert!(made.success());
        let unpacked = unpack_vsix(&file, &folder.0.join("out")).unwrap();
        assert_eq!(read(&unpacked).unwrap().name, "z");
        let not_one = folder.0.join("src").join("extension").join("package.json");
        assert!(unpack_vsix(&not_one, &folder.0.join("out2")).is_err());
    }

    #[test]
    fn theme_files_get_safe_names() {
        assert_eq!(file_name_for("GitHub Dark / Default"), "GitHub Dark _ Default.json");
        assert_eq!(file_name_for("..\\up"), "___up.json");
    }
}
