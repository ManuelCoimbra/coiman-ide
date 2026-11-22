//! The commands run in terminals, kept in the plugin's storage as JSON, newest last, and
//! completion of paths. No clock in the sandbox: the order is the age.

use serde::{Deserialize, Serialize};

/// A folder's entries (name, is a folder), or `None` when it can't be read.
pub type Listing = Option<Vec<(String, bool)>>;

/// Commands kept; the oldest go first.
pub const KEPT: usize = 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub command: String,
    /// The folder the shell was in.
    pub cwd: String,
    /// The project open then.
    pub project: String,
    /// Times run.
    pub count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    /// Oldest first.
    pub entries: Vec<Entry>,
}

impl History {
    pub fn parse(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn to_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Notes a command run: moved to the newest end when it was there already.
    pub fn record(&mut self, command: &str, cwd: &str, project: &str) {
        let command = command.trim();
        if command.is_empty() || command.len() > 2000 {
            return;
        }
        let count = match self.entries.iter().position(|e| e.command == command && e.project == project) {
            Some(i) => self.entries.remove(i).count + 1,
            None => 1,
        };
        self.entries.push(Entry {
            command: command.to_owned(),
            cwd: cwd.to_owned(),
            project: project.to_owned(),
            count,
        });
        if self.entries.len() > KEPT {
            let extra = self.entries.len() - KEPT;
            self.entries.drain(..extra);
        }
    }

    /// Takes a command out (every project's copy).
    pub fn forget(&mut self, command: &str) {
        self.entries.retain(|e| e.command != command);
    }

    /// The commands holding every word of `query` (any case), this project's first, newest
    /// first, those starting with the query before the others; each command once.
    pub fn search(&self, query: &str, project: &str) -> Vec<&Entry> {
        let fits = |e: &&Entry| words_fit(query, &e.command);
        let mut found: Vec<&Entry> = self.entries.iter().rev().filter(|e| e.project == project).filter(fits).collect();
        for e in self.entries.iter().rev().filter(|e| e.project != project).filter(fits) {
            if !found.iter().any(|f| f.command == e.command) {
                found.push(e);
            }
        }
        let (mut starts, rest): (Vec<_>, Vec<_>) = found.into_iter().partition(|e| e.command.starts_with(query));
        starts.extend(rest);
        starts
    }
}

/// Whether `text` holds every word of `query`, in any case.
pub fn words_fit(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|w| text.contains(&w.to_lowercase()))
}

/// The last word of a command line, where a path would be completed.
pub fn last_word(line: &str) -> &str {
    let mut start = 0;
    let mut quote: Option<char> = None;
    for (i, ch) in line.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'') => {
                quote = Some(ch);
                start = i;
            }
            (Some(q), c) if c == q => quote = None,
            (None, ' ' | '\t' | '=' | '|' | ';' | '(' | '>' | '<') => start = i + ch.len_utf8(),
            _ => {}
        }
    }
    &line[start..]
}

/// Whether a path is absolute, on Windows or elsewhere (the sandbox's own `Path` can't tell
/// for Windows paths).
pub fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/') || path.starts_with("\\\\") || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// `dir` and `rest` joined with `dir`'s own separator.
pub fn join(dir: &str, rest: &str) -> String {
    if rest.is_empty() {
        return dir.to_owned();
    }
    let sep = if dir.contains('\\') { '\\' } else { '/' };
    let rest = rest.trim_matches(['/', '\\']);
    format!("{}{sep}{rest}", dir.trim_end_matches(['/', '\\']))
}

/// The files and folders `word` may go on to name, from `dir` (absolute): each the whole word
/// as it would read completed, folders ending in a separator, folders first, at most 200.
/// `list` reads a folder: (name, is a folder).
pub fn complete_path(dir: &str, word: &str, home: Option<&str>, list: &dyn Fn(&str) -> Listing) -> Vec<String> {
    let quote = word.chars().next().filter(|c| *c == '"' || *c == '\'');
    let bare = quote.map_or(word, |q| &word[q.len_utf8()..]);
    let cut = bare.rfind(['/', '\\']).map_or(0, |i| i + 1);
    let (folder, start) = bare.split_at(cut);
    let windows = dir.contains('\\') || dir.as_bytes().get(1) == Some(&b':');
    let sep = if folder.contains('\\') || (windows && !folder.contains('/')) { '\\' } else { '/' };
    let look_in = if folder.is_empty() {
        dir.to_owned()
    } else if let (Some(rest), Some(h)) = (folder.strip_prefix('~'), home) {
        join(h, rest)
    } else if is_absolute(folder) {
        // "C:\" and "/" stay whole.
        let trimmed = folder.trim_end_matches(['/', '\\']);
        if trimmed.is_empty() || trimmed.ends_with(':') { folder.to_owned() } else { trimmed.to_owned() }
    } else {
        join(dir, folder)
    };
    let Some(entries) = list(&look_in) else { return Vec::new() };
    let lower = start.to_lowercase();
    let mut found: Vec<(bool, String)> = entries
        .into_iter()
        .filter(|(name, _)| {
            let fits = if windows { name.to_lowercase().starts_with(&lower) } else { name.starts_with(start) };
            fits && (start.starts_with('.') || !name.starts_with('.'))
        })
        .map(|(name, is_dir)| (is_dir, name))
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    found
        .into_iter()
        .take(200)
        .map(|(is_dir, name)| {
            let needs_quote = quote.is_none() && name.contains(' ');
            let q = quote.map(String::from).unwrap_or_else(|| if needs_quote { "\"".into() } else { String::new() });
            let tail = if is_dir { sep.to_string() } else { q.clone() };
            format!("{q}{folder}{name}{tail}")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_keeps_each_command_once() {
        let mut h = History::default();
        h.record("cargo build", "/a", "/a");
        h.record("npm test", "/b", "/b");
        h.record("cargo build", "/a", "/a");
        h.record("  ", "/a", "/a");
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[1].command, "cargo build");
        assert_eq!(h.entries[1].count, 2);
        let found: Vec<_> = h.search("", "/b").iter().map(|e| e.command.clone()).collect();
        assert_eq!(found, ["npm test", "cargo build"]);
        assert_eq!(h.search("BUILD", "/a").len(), 1);
        h.record("cargo test", "/a", "/a");
        let found: Vec<_> = h.search("cargo b", "/a").iter().map(|e| e.command.clone()).collect();
        assert_eq!(found, ["cargo build"]);
        let again = History::parse(&h.to_text());
        assert_eq!(again.entries, h.entries);
        h.forget("npm test");
        assert_eq!(h.entries.len(), 2);
        assert!(History::parse("not json").entries.is_empty());
    }

    #[test]
    fn paths_complete() {
        let list = |dir: &str| -> Listing {
            let e = |n: &str, d: bool| (n.to_owned(), d);
            match dir {
                "/p" => Some(vec![e("src", true), e("Cargo.toml", false), e(".git", true)]),
                "/p/src" => Some(vec![e("main.rs", false), e("my file.rs", false), e("bin", true)]),
                "/home/me/docs" => Some(vec![e("a.md", false)]),
                r"C:\p" => Some(vec![e("Src", true)]),
                _ => None,
            }
        };
        assert_eq!(last_word("cat \"src/my"), "\"src/my");
        assert_eq!(last_word("ls src/ma"), "src/ma");
        assert_eq!(complete_path("/p", "Ca", None, &list), ["Cargo.toml"]);
        assert_eq!(complete_path("/p", "", None, &list), ["src/", "Cargo.toml"]);
        assert_eq!(complete_path("/p", "src/", None, &list), ["src/bin/", "src/main.rs", "\"src/my file.rs\""]);
        assert_eq!(complete_path("/p", "\"src/my", None, &list), ["\"src/my file.rs\""]);
        assert_eq!(complete_path("/p", "/p/src/b", None, &list), ["/p/src/bin/"]);
        assert_eq!(complete_path("/p", "~/docs/", Some("/home/me"), &list), ["~/docs/a.md"]);
        assert_eq!(complete_path(r"C:\p", "sr", None, &list), [r"Src\"]);
        assert_eq!(complete_path("/p", "nothing/", None, &list), Vec::<String>::new());
        assert!(is_absolute(r"C:\x") && is_absolute("/x") && !is_absolute("x/y"));
    }
}
