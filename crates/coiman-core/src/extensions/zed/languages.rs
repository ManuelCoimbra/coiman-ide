//! Zed extension language integration: reads `languages/<dir>/config.toml`,
//! queries (`highlights.scm`, `injections.scm`, `outline.scm`, `indents.scm`, `brackets.scm`),
//! and `grammars/*.wasm` for dynamic language detection, commenting, indentation, brackets,
//! outline/folding, and Tree-sitter syntax highlighting.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::editor::auto_tags::TagMode;
use crate::extensions::plugin::manifest::LanguageDecl;

use super::manifest::{LanguageConfig, ZedBracketPair};
use super::registry::InstalledExtension;

/// Loaded language definition from an enabled Zed extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedLanguage {
    pub extension_id: String,
    pub config: LanguageConfig,
    pub dir: PathBuf,
    pub grammar_wasm: Option<PathBuf>,
    pub highlights_scm: Option<String>,
    pub injections_scm: Option<String>,
    pub outline_scm: Option<String>,
    pub indents_scm: Option<String>,
    pub brackets_scm: Option<String>,
}

static ACTIVE_LANGUAGES: RwLock<Vec<LoadedLanguage>> = RwLock::new(Vec::new());

fn read_scm(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name)).ok().filter(|s| !s.trim().is_empty())
}

/// Loads all language configs and queries declared by active Zed extensions.
pub fn collect_languages(extensions: &[InstalledExtension]) -> Vec<LoadedLanguage> {
    // Index all grammars across active extensions so languages referencing shared grammars resolve too.
    let mut grammar_map: HashMap<String, PathBuf> = HashMap::new();
    for ext in extensions {
        let gdir = ext.path.join("grammars");
        if let Ok(entries) = std::fs::read_dir(&gdir) {
            for entry in entries.filter_map(Result::ok) {
                let p = entry.path();
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wasm"))
                    && p.is_file()
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    grammar_map.entry(stem.to_owned()).or_insert(p);
                }
            }
        }
    }

    let mut out = Vec::new();
    for ext in extensions {
        for rel in &ext.manifest.languages {
            let dir = ext.path.join(rel);
            let Ok(config) = LanguageConfig::load(&dir) else { continue };
            if config.hidden || config.name.trim().is_empty() {
                continue;
            }
            let grammar_wasm = config.grammar.as_deref().and_then(|g| {
                let local = ext.path.join("grammars").join(format!("{g}.wasm"));
                if local.is_file() { Some(local) } else { grammar_map.get(g).cloned() }
            });
            out.push(LoadedLanguage {
                extension_id: ext.id.clone(),
                highlights_scm: read_scm(&dir, "highlights.scm"),
                injections_scm: read_scm(&dir, "injections.scm"),
                outline_scm: read_scm(&dir, "outline.scm"),
                indents_scm: read_scm(&dir, "indents.scm"),
                brackets_scm: read_scm(&dir, "brackets.scm"),
                config,
                dir,
                grammar_wasm,
            });
        }
    }
    out
}

/// Synchronizes active Zed languages globally and compiles their Tree-sitter Wasm grammars.
pub fn sync_all_languages(enabled: bool, active_extensions: &[InstalledExtension]) {
    let langs = if enabled { collect_languages(active_extensions) } else { Vec::new() };
    let unchanged = ACTIVE_LANGUAGES.read().is_ok_and(|cur| *cur == langs);
    if !unchanged
        && let Ok(mut guard) = ACTIVE_LANGUAGES.write()
    {
        *guard = langs.clone();
    }
    crate::language::syntax::sync_zed_languages(&langs);
    crate::language::languages::set(&to_language_decls(&langs));
}

/// Snapshot of currently active Zed languages.
pub fn active_languages() -> Vec<LoadedLanguage> {
    ACTIVE_LANGUAGES.read().map(|g| g.clone()).unwrap_or_default()
}

/// Converts loaded Zed languages into Coiman [`LanguageDecl`] entries for `languages::set`.
pub fn to_language_decls(languages: &[LoadedLanguage]) -> Vec<LanguageDecl> {
    languages
        .iter()
        .filter(|l| !l.config.path_suffixes.is_empty())
        .map(|l| LanguageDecl {
            name: l.config.name.clone(),
            extensions: l.config.path_suffixes.clone(),
            line_comment: l
                .config
                .line_comments
                .first()
                .map(|c| c.trim_end().to_owned())
                .filter(|c| !c.is_empty()),
        })
        .collect()
}

/// Returns auto-close bracket pairs contributed by a Zed language (if any match `ext`).
pub fn brackets_for_extension(languages: &[LoadedLanguage], ext: &str) -> Vec<ZedBracketPair> {
    languages
        .iter()
        .find(|l| l.config.path_suffixes.iter().any(|s| s.eq_ignore_ascii_case(ext)))
        .map(|l| l.config.brackets.clone())
        .unwrap_or_default()
}

/// Returns `(tab_size, hard_tabs)` configured by a Zed extension language matching `path` or `ext`.
pub fn indent_for(languages: &[LoadedLanguage], path: Option<&Path>, ext: &str) -> (Option<usize>, Option<bool>) {
    let Some(lang) = match_file_in(languages, path, ext, None) else {
        return (None, None);
    };
    (lang.config.tab_size.map(|n| n as usize), lang.config.hard_tabs)
}

/// Returns `(tab_size, hard_tabs)` configured by a Zed extension language, if specified.
pub fn indent_for_extension(languages: &[LoadedLanguage], ext: &str) -> (Option<usize>, Option<bool>) {
    indent_for(languages, None, ext)
}

/// Returns the block comment `(open, close)` from a Zed language matching `path` or `ext`.
pub fn active_block_comment(path: Option<&Path>, ext: &str) -> Option<(String, String)> {
    let list = ACTIVE_LANGUAGES.read().ok()?;
    match_file_in(&list, path, ext, None).and_then(|l| l.config.block_comment.clone())
}

/// Returns the line comment token from active Zed languages matching `path` or `ext`.
pub fn active_line_comment(path: Option<&Path>, ext: &str) -> Option<String> {
    let list = ACTIVE_LANGUAGES.read().ok()?;
    match_file_in(&list, path, ext, None)
        .and_then(|l| l.config.line_comments.first())
        .map(|c| c.trim_end().to_owned())
        .filter(|c| !c.is_empty())
}

/// Returns auto-tag closing mode inferred from Zed language config (`jsx_tag_auto_close` or HTML `</` wrap).
pub fn active_auto_tags(path: Option<&Path>, ext: &str) -> Option<TagMode> {
    let list = ACTIVE_LANGUAGES.read().ok()?;
    let lang = match_file_in(&list, path, ext, None)?;
    if lang.config.jsx_tag_auto_close.is_some()
        || lang.config.wrap_characters.as_ref().is_some_and(|w| w.end_prefix == "</")
    {
        Some(TagMode::Html)
    } else {
        None
    }
}

/// Whether single-quote pairing should be enabled for `path`/`ext` based on Zed `brackets`.
pub fn active_pair_single_quote(path: Option<&Path>, ext: &str) -> Option<bool> {
    let list = ACTIVE_LANGUAGES.read().ok()?;
    let lang = match_file_in(&list, path, ext, None)?;
    if lang.config.brackets.is_empty() {
        return None;
    }
    Some(lang.config.brackets.iter().any(|b| b.start == "'" && b.close))
}

/// Returns active Zed language name matching `path`, `ext`, or `first_line`.
pub fn active_language_name(path: Option<&Path>, ext: &str, first_line: Option<&str>) -> Option<String> {
    let list = ACTIVE_LANGUAGES.read().ok()?;
    match_file_in(&list, path, ext, first_line).map(|l| l.config.name.clone())
}

/// Maps a file path, extension, or first line to a matching Zed language.
pub fn match_file_in<'a>(
    languages: &'a [LoadedLanguage],
    path: Option<&Path>,
    ext: &str,
    first_line: Option<&str>,
) -> Option<&'a LoadedLanguage> {
    let file_name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or_default();
    if let Some(found) = languages.iter().find(|l| {
        l.config.path_suffixes.iter().any(|s| {
            (!file_name.is_empty() && s.eq_ignore_ascii_case(file_name))
                || (!ext.is_empty() && s.eq_ignore_ascii_case(ext))
        })
    }) {
        return Some(found);
    }
    let line = first_line.map(str::trim_end).filter(|s| !s.is_empty())?;
    languages.iter().find(|l| {
        l.config
            .first_line_pattern
            .as_deref()
            .and_then(|pat| regex::Regex::new(pat).ok())
            .is_some_and(|re| re.is_match(line))
    })
}

/// Maps a file path (including exact filenames in Zed `path_suffixes` like `Cargo.lock`)
/// to a matching Zed language.
pub fn match_path<'a>(languages: &'a [LoadedLanguage], path: &Path) -> Option<&'a LoadedLanguage> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    match_file_in(languages, Some(path), ext, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_real_toml_language_config_if_present() {
        let dir = std::env::temp_dir().join(format!("coiman-zed-manifest-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("languages/demo")).unwrap();
        std::fs::write(
            dir.join("extension.toml"),
            "id = \"demo\"\nname = \"Demo\"\nversion = \"0.1.0\"\nschema_version = 1\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("languages/demo/config.toml"),
            "name = \"Demo\"\npath_suffixes = [\"demo\", \"Demofile\"]\nline_comments = [\"# \"]\nfirst_line_pattern = \"^#!/usr/bin/env demo$\"\n",
        )
        .unwrap();
        let ext = InstalledExtension::from_folder(&dir, "test", true).unwrap();
        let langs = collect_languages(&[ext]);
        assert_eq!(langs.len(), 1);
        assert!(match_path(&langs, Path::new("/a/Demofile")).is_some());
        assert!(match_file_in(&langs, Some(Path::new("/a/script")), "", Some("#!/usr/bin/env demo")).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
