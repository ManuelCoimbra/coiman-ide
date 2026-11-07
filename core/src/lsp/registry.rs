//! Languages described as data: adding one takes a few lines of TOML and no code.
//! Coiman IDE ships entries for common servers, and the user's `languages.toml`
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
    /// An interactive session to send code to.
    pub repl: Option<&'static str>,
    /// A debug adapter speaking DAP over stdio, and its `type` for `launch`.
    pub debugger: Option<&'static str>,
    pub debug_type: Option<&'static str>,
    /// What to do first, or how to install the server, for the doctor.
    pub hint: Option<&'static str>,
    /// Whether the server runs the project's own code, such as build scripts.
    pub runs_project_code: bool,
    /// Columns to draw a line after.
    pub rulers: Vec<usize>,
    /// Columns that don't count, from 1 and inclusive. An end of 0 runs on.
    pub dim_columns: Vec<(usize, usize)>,
    /// Backslash names turn into symbols as they're typed.
    pub abbreviations: bool,
    /// Characters to offer in a bar above the text.
    pub glyphs: Option<&'static str>,
    /// A pattern finding what an include statement names, in its one capture.
    pub include_pattern: Option<&'static str>,
    /// Where includes are, from the project's folder.
    pub include_paths: &'static [&'static str],
    /// What to try on a name written without its extension.
    pub include_extensions: &'static [&'static str],
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
    repl: Option<String>,
    #[serde(default)]
    debugger: Option<String>,
    #[serde(default)]
    debug_type: Option<String>,
    #[serde(default)]
    hint: Option<String>,
    #[serde(default = "assume_it_does")]
    runs_project_code: bool,
    #[serde(default)]
    rulers: Vec<usize>,
    #[serde(default)]
    dim_columns: Vec<(usize, usize)>,
    #[serde(default)]
    abbreviations: bool,
    #[serde(default)]
    glyphs: Option<String>,
    #[serde(default)]
    include_pattern: Option<String>,
    #[serde(default)]
    include_paths: Vec<String>,
    #[serde(default)]
    include_extensions: Vec<String>,
}

fn leak_list(list: Vec<String>) -> &'static [&'static str] {
    let leaked: Vec<&'static str> = list.into_iter().map(leak).collect();
    Box::leak(leaked.into_boxed_slice())
}

/// A server is asked to read a project; unless told otherwise, assume it may run it.
fn assume_it_does() -> bool {
    true
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
        repl: entry.repl.map(leak),
        debugger: entry.debugger.map(leak),
        debug_type: entry.debug_type.map(leak),
        hint: entry.hint.map(leak),
        runs_project_code: entry.runs_project_code,
        rulers: entry.rulers,
        dim_columns: entry.dim_columns,
        abbreviations: entry.abbreviations,
        glyphs: entry.glyphs.map(leak),
        include_pattern: entry.include_pattern.map(leak),
        include_paths: leak_list(entry.include_paths),
        include_extensions: leak_list(entry.include_extensions),
        from_user,
    }))
}

/// Reads a languages file, saying what's wrong with it.
///
/// One entry that's wrong doesn't take the others with it: they're kept, and it's
/// reported. Only text that isn't TOML at all gives nothing.
fn parse(text: &str, from_user: bool) -> (Vec<&'static Language>, Option<String>) {
    let table: toml::Table = match toml::from_str(text) {
        Ok(table) => table,
        Err(e) => return (Vec::new(), Some(format!("languages.toml: {}", e.message()))),
    };
    let mut problems = Vec::new();
    let mut found = Vec::new();
    let entries = table.get("language").and_then(toml::Value::as_array).cloned().unwrap_or_default();
    for (at, value) in entries.into_iter().enumerate() {
        let named = value.get("name").and_then(toml::Value::as_str).map(str::to_owned);
        let name = named.unwrap_or_else(|| format!("entry {}", at + 1));
        match value.try_into::<LanguageEntry>() {
            Ok(entry) if entry.extensions.is_empty() => {
                problems.push(format!("languages.toml: {name} names no extensions"));
            }
            Ok(entry) => found.push(into_language(entry, from_user)),
            Err(e) => problems.push(format!("languages.toml: {name}: {}", e.message())),
        }
    }
    (found, (!problems.is_empty()).then(|| problems.join("; ")))
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
        *list = parse(SHIPPED, false).0;
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
     # repl = \"gleam shell\"          # what Ctrl+Enter sends code to\n\
     # debugger = \"my-dap --stdio\"   # a debug adapter, for F5\n\
     # debug_type = \"my\"             # its `type` in the launch request\n\
     # hint = \"Install it from gleam.run\"   # what Language: Doctor suggests\n\
     # runs_project_code = true      # shown by the doctor; assumed unless false\n\
     # rulers = [72, 80]             # lines after these columns\n\
     # dim_columns = [[1, 6], [73, 0]] # columns that don't count; 0 runs on\n\
     # abbreviations = true          # \\\\to becomes an arrow as you type\n\
     # glyphs = \"∀∃λ→\"              # a bar of characters above the text\n\
     # include_pattern = \"^\\\\s*include\\\\s+'([^']+)'\"  # Go to Include at Cursor\n\
     # include_paths = [\"inc\"]       # where includes are\n\
     # include_extensions = [\"inc\"]  # tried on a name without one\n"
        .to_owned()
}

/// Reads the user's file again. Returns what's wrong with it, and whether the
/// languages changed in a way that running servers should restart for.
pub fn reload() -> (Option<String>, bool) {
    let text = user_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    let (mine, problem) = parse(&text, true);
    let mut list = LANGUAGES.write().unwrap_or_else(|e| e.into_inner());
    if list.is_empty() {
        *list = parse(SHIPPED, false).0;
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
        let (all, problem) = parse(SHIPPED, false);
        assert_eq!(problem, None);
        assert!(all.len() >= 10);
        for language in &all {
            assert!(!language.extensions.is_empty(), "{}", language.name);
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
        let found = parse(text, true).0;
        assert_eq!(found[0].language_id, "my-lang");
        assert_eq!(found[0].extensions, ["ml"]);
        assert!(found[0].runs_project_code, "assumed until told otherwise");
        assert!(found[0].from_user);
    }

    #[test]
    fn mistakes_are_reported() {
        let problem = |text: &str| parse(text, true).1.unwrap_or_default();
        assert!(problem("[[language]]\nname = \"x\"\nextensions = []\n").contains("no extensions"));
        assert!(problem("[[language]]\nname = \"x\"\n").contains("x:"));
        assert!(
            problem("[[language]]\nname=\"x\"\nextensions=[\"x\"]\nservr=\"y\"\n").contains("servr"),
            "typos are caught"
        );
        assert!(!problem("not toml =").is_empty());
    }

    #[test]
    fn a_mistake_keeps_the_other_entries() {
        let text = "[[language]]\nname=\"bad\"\nextensions=[\"b\"]\nservr=\"y\"\n\n[[language]]\nname=\"good\"\nextensions=[\"g\"]\n";
        let (found, problem) = parse(text, true);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "good");
        assert!(problem.unwrap().contains("bad"));
    }

    #[test]
    fn the_template_is_only_comments() {
        assert!(parse(&user_file_template(), true).0.is_empty());
    }
}
