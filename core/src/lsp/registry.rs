//! Languages described as data: adding one takes a few lines of TOML and no code.
//! Rust Code ships entries for common servers, and the user's `languages.toml`
//! adds to or replaces them, including the languages built into the editor.

use std::path::PathBuf;
use std::sync::RwLock;

use serde::Deserialize;

/// Entries that ship with the editor. A server here is found on PATH, never
/// installed: each one's `hint` says how to get it.
const SHIPPED: &str = include_str!("languages.toml");

/// One language, leaked so it can be named by `&'static` like the built-in ones.
#[derive(Debug, PartialEq, Eq)]
pub struct Language {
    pub name: &'static str,
    /// The id language servers know it by.
    pub language_id: &'static str,
    pub extensions: Vec<&'static str>,
    pub comment: Option<&'static str>,
    /// A command line, or `tcp://127.0.0.1:port` for a server already running.
    pub server: Option<&'static str>,
    pub root_markers: &'static [&'static str],
    /// How to run a file, with `{file}` for its path.
    pub run: Option<&'static str>,
    /// What to do first, or how to install the server, for the doctor.
    pub hint: Option<&'static str>,
    /// Whether the server runs the project's own code, such as build scripts.
    pub runs_project_code: bool,
    /// From the user's file, not shipped.
    pub from_user: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageEntry {
    name: String,
    #[serde(default)]
    language_id: Option<String>,
    extensions: Vec<String>,
    #[serde(default)]
    comment: Option<String>,
    #[serde(default)]
    server: Option<String>,
    #[serde(default)]
    root_markers: Vec<String>,
    #[serde(default)]
    run: Option<String>,
    #[serde(default)]
    hint: Option<String>,
    #[serde(default = "assume_it_does")]
    runs_project_code: bool,
}

/// A server is asked to read a project; unless told otherwise, assume it may run it.
fn assume_it_does() -> bool {
    true
}

#[derive(Deserialize)]
struct LanguagesFile {
    #[serde(default)]
    language: Vec<LanguageEntry>,
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn into_language(entry: LanguageEntry, from_user: bool) -> &'static Language {
    let language_id = entry.language_id.unwrap_or_else(|| entry.name.to_lowercase().replace(' ', "-"));
    let markers: Vec<&'static str> = entry.root_markers.into_iter().map(leak).collect();
    Box::leak(Box::new(Language {
        name: leak(entry.name),
        language_id: leak(language_id),
        extensions: entry.extensions.into_iter().map(|e| leak(e.trim_start_matches('.').to_lowercase())).collect(),
        comment: entry.comment.map(leak),
        server: entry.server.filter(|s| !s.trim().is_empty()).map(leak),
        root_markers: Box::leak(markers.into_boxed_slice()),
        run: entry.run.map(leak),
        hint: entry.hint.map(leak),
        runs_project_code: entry.runs_project_code,
        from_user,
    }))
}

/// Reads a languages file, saying what's wrong with it.
fn parse(text: &str, from_user: bool) -> Result<Vec<&'static Language>, String> {
    let file: LanguagesFile = toml::from_str(text).map_err(|e| format!("languages.toml: {}", e.message()))?;
    let mut problems = Vec::new();
    let mut found = Vec::new();
    for entry in file.language {
        if entry.extensions.is_empty() {
            problems.push(format!("languages.toml: {} names no extensions", entry.name));
        } else {
            found.push(into_language(entry, from_user));
        }
    }
    if problems.is_empty() { Ok(found) } else { Err(problems.join("; ")) }
}

/// Every language: the user's first, so they win.
static LANGUAGES: RwLock<Vec<&'static Language>> = RwLock::new(Vec::new());

fn with_languages<T>(read: impl FnOnce(&[&'static Language]) -> T) -> T {
    {
        let list = LANGUAGES.read().unwrap_or_else(|e| e.into_inner());
        if !list.is_empty() {
            return read(&list);
        }
    }
    let mut list = LANGUAGES.write().unwrap_or_else(|e| e.into_inner());
    if list.is_empty() {
        *list = parse(SHIPPED, false).expect("shipped languages parse");
    }
    read(&list)
}

/// The user's languages file.
pub fn user_file() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("languages.toml"))
}

/// Text for a new languages file.
pub fn user_file_template() -> String {
    "# Add a language with a few lines, no code. Saving applies them.\n\
     # An entry here replaces a language with the same extension.\n\
     #\n\
     # [[language]]\n\
     # name = \"Gleam\"\n\
     # extensions = [\"gleam\"]\n\
     # comment = \"//\"                # for Toggle Comment\n\
     # server = \"gleam lsp\"          # or \"tcp://127.0.0.1:5757\" for one already running\n\
     # root_markers = [\"gleam.toml\"] # files that mark a project\n\
     # run = \"gleam run\"             # what Run does; {file} is the file's path\n\
     # hint = \"Install it from gleam.run\"   # what Language: Doctor suggests\n\
     # runs_project_code = true      # shown by the doctor; assumed unless false\n"
        .to_owned()
}

/// Reads the user's file again. Returns what's wrong with it, and whether the
/// languages changed in a way that running servers should restart for.
pub fn reload() -> (Option<String>, bool) {
    let text = user_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let (mine, problem) = match parse(&text, true) {
        Ok(mine) => (mine, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let mut list = LANGUAGES.write().unwrap_or_else(|e| e.into_inner());
    if list.is_empty() {
        *list = parse(SHIPPED, false).expect("shipped languages parse");
    }
    let before: Vec<&'static Language> = list.iter().filter(|l| l.from_user).copied().collect();
    let changed = before.len() != mine.len() || before.iter().zip(&mine).any(|(a, b)| a != b);
    let shipped: Vec<&'static Language> = list.iter().filter(|l| !l.from_user).copied().collect();
    *list = mine.into_iter().chain(shipped).collect();
    (problem, changed)
}

pub fn get(index: u16) -> Option<&'static Language> {
    with_languages(|list| list.get(index as usize).copied())
}

/// The language for a file extension. `from_user` limits it to the user's.
pub fn by_extension(ext: &str, from_user: bool) -> Option<&'static Language> {
    with_languages(|list| list.iter().find(|l| (!from_user || l.from_user) && l.extensions.contains(&ext)).copied())
}

/// Where the language with this server id is in the list.
pub fn index_of(language_id: &str, from_user: bool) -> Option<u16> {
    with_languages(|list| {
        list.iter().position(|l| (!from_user || l.from_user) && l.language_id == language_id).map(|i| i as u16)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_entries_are_sound() {
        let all = parse(SHIPPED, false).unwrap();
        assert!(all.len() >= 10);
        for language in &all {
            assert!(!language.extensions.is_empty(), "{}", language.name);
            assert!(language.server.is_some(), "{} has no server", language.name);
            assert!(language.hint.is_some(), "{} has no hint", language.name);
        }
        let mut ids: Vec<&str> = all.iter().map(|l| l.language_id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids must be unique");
    }

    #[test]
    fn entries_read_with_defaults() {
        let text = "[[language]]\nname = \"My Lang\"\nextensions = [\".ML\"]\nserver = \"mls --stdio\"\n";
        let found = parse(text, true).unwrap();
        assert_eq!(found[0].language_id, "my-lang");
        assert_eq!(found[0].extensions, ["ml"]);
        assert!(found[0].runs_project_code, "assumed until told otherwise");
        assert!(found[0].from_user);
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse("[[language]]\nname = \"x\"\nextensions = []\n", true).unwrap_err().contains("no extensions"));
        assert!(parse("[[language]]\nname = \"x\"\n", true).is_err());
        assert!(
            parse("[[language]]\nname=\"x\"\nextensions=[\"x\"]\nservr=\"y\"\n", true).is_err(),
            "typos are caught"
        );
        assert!(parse("not toml =", true).is_err());
    }

    #[test]
    fn the_template_is_only_comments() {
        assert!(parse(&user_file_template(), true).unwrap().is_empty());
    }
}
