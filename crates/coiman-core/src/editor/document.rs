//! An open file (or untitled buffer) shown in an editor tab.

use std::path::{Path, PathBuf};

use encoding_rs::{Encoding, UTF_8};

use crate::editor::buffer::{Buffer, Change, Selection};
use crate::editor::encoding;
use crate::editor::structure::{self, FoldRange, Symbol};
use crate::language::highlight::Highlighter;
use crate::language::syntax::{Lang, TsState};

/// Files above either limit open in large-file mode: no syntax highlighting.
const LARGE_FILE_BYTES: usize = 16 * 1024 * 1024;
const LARGE_FILE_LINES: usize = 100_000;

/// How the file on disk relates to the buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiskState {
    InSync,
    /// Changed by another program while the buffer had unsaved edits.
    Changed,
    Deleted,
}

/// Folds and outline of a document, for one buffer and tree version.
#[derive(Default)]
pub struct Structure {
    key: Option<(u64, Option<u64>)>,
    pub folds: Vec<FoldRange>,
    pub symbols: Vec<Symbol>,
}

pub struct Document {
    /// Remove spaces at line ends when saving.
    pub trim_trailing_whitespace: bool,
    /// End the file with a newline when saving.
    pub insert_final_newline: bool,
    /// Unique id for the editor widget state; assigned by the app.
    pub uid: u64,
    pub path: Option<PathBuf>,
    pub buffer: Buffer,
    /// Buffer version at the last save/load, used to compute the dirty flag.
    saved_version: u64,
    /// Index used to name untitled buffers ("Untitled-1", …).
    untitled_index: usize,
    /// syntect highlighting, used for languages without a tree-sitter grammar.
    pub highlighter: Highlighter,
    /// Tree-sitter highlighting for supported languages.
    pub ts: Option<TsState>,
    /// Large-file mode: highlighting off to keep huge files responsive.
    pub large: bool,
    /// Give the editor keyboard focus the next time it is shown.
    pub request_focus: bool,
    /// Scroll the primary cursor into view the next time it is shown.
    pub reveal: bool,
    pub disk: DiskState,
    /// Encoding the file is read and saved with.
    pub encoding: &'static Encoding,
    /// Whether the file starts with a byte order mark.
    pub bom: bool,
    /// Folds and outline, recomputed when the text or syntax tree changes.
    structure: Structure,
}

/// Whether char `pos` is code, by the syntax tree where there is one, else by the
/// grammar's scopes. Without either, everything counts.
fn is_code(buffer: &Buffer, ts: Option<&TsState>, highlighter: &mut Highlighter, pos: usize) -> bool {
    let literal = match ts {
        Some(ts) => ts.is_literal_at(buffer, pos),
        None => {
            let line = buffer.char_to_line(pos);
            highlighter.is_literal_at(buffer, line, pos - buffer.line_start(line))
        }
    };
    !literal.unwrap_or(false)
}

impl Document {
    /// The bracket next to `pos` and its partner, passing over brackets in strings and
    /// comments.
    pub fn matching_bracket(&mut self, pos: usize, limit: usize) -> Option<(usize, usize)> {
        let Document { buffer, ts, highlighter, .. } = self;
        buffer.matching_bracket_where(pos, limit, &mut |p| is_code(buffer, ts.as_ref(), highlighter, p))
    }

    /// The innermost bracket pair around `pos`, passing over brackets in strings and
    /// comments.
    pub fn enclosing_brackets(&mut self, pos: usize, limit: usize) -> Option<(usize, usize)> {
        let Document { buffer, ts, highlighter, .. } = self;
        buffer.enclosing_brackets_where(pos, limit, &mut |p| is_code(buffer, ts.as_ref(), highlighter, p))
    }

    /// The first code bracket from `pos` to the end of its line, as Vim's `%` looks for.
    pub fn bracket_ahead(&mut self, pos: usize) -> Option<usize> {
        let Document { buffer, ts, highlighter, .. } = self;
        let end = buffer.line_end(buffer.char_to_line(pos.min(buffer.len_chars())));
        (pos..end).find(|&q| "()[]{}".contains(buffer.rope().char(q)) && is_code(buffer, ts.as_ref(), highlighter, q))
    }

    fn from_text(path: Option<PathBuf>, untitled_index: usize, text: &str) -> Self {
        let buffer = Buffer::new(text);
        let large = text.len() > LARGE_FILE_BYTES || buffer.len_lines() > LARGE_FILE_LINES;
        let ext = extension_of(path.as_deref());
        let mut doc = Self {
            trim_trailing_whitespace: false,
            insert_final_newline: false,
            uid: 0,
            saved_version: buffer.version(),
            buffer,
            highlighter: Highlighter::new(&ext, false),
            ts: None,
            path,
            untitled_index,
            large,
            request_focus: false,
            reveal: false,
            disk: DiskState::InSync,
            encoding: UTF_8,
            bom: false,
            structure: Structure::default(),
        };
        doc.set_language(&ext);
        doc
    }

    /// Picks tree-sitter for supported languages, syntect otherwise; nothing in large-file mode.
    fn set_language(&mut self, ext: &str) {
        self.ts = Lang::from_extension(ext).filter(|_| !self.large).and_then(TsState::new);
        if self.ts.is_some() {
            self.buffer.track_syntax_edits();
        }
        self.highlighter = Highlighter::new(ext, !self.large && self.ts.is_none());
    }

    pub fn untitled(index: usize) -> Self {
        Self::from_text(None, index, "")
    }

    pub fn open(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let decoded = encoding::detect_and_decode(bytes);
        let mut doc = Self::from_text(Some(path.to_path_buf()), 0, &decoded.text);
        doc.encoding = decoded.encoding;
        doc.bom = decoded.bom;
        Ok(doc)
    }

    /// Rebuilds a document from a hot-exit backup. `saved_text` is what is on disk
    /// (empty for untitled buffers); `text` is the backed-up buffer content.
    pub fn restored(path: Option<PathBuf>, untitled_index: usize, saved_text: String, text: String) -> Self {
        let mut doc = Self::from_text(path, untitled_index, &text);
        if saved_text != text {
            // Mark dirty: no buffer version will ever equal this.
            doc.saved_version = u64::MAX;
        }
        doc
    }

    pub fn untitled_index(&self) -> usize {
        self.untitled_index
    }

    pub fn title(&self) -> String {
        match &self.path {
            Some(p) => {
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
            }
            None => format!("Untitled-{}", self.untitled_index),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.version() != self.saved_version
    }

    pub fn text(&self) -> String {
        self.buffer.text()
    }

    /// Saves to the current path. Returns `Ok(false)` if the document has no path yet.
    /// What `.editorconfig` or the settings ask of the text when saving.
    fn prepare_for_save(&mut self) {
        let mut changes = Vec::new();
        if self.trim_trailing_whitespace {
            for line in 0..self.buffer.len_lines() {
                let (start, end) = (self.buffer.line_start(line), self.buffer.line_end(line));
                let text = self.buffer.slice_string(start, end);
                let kept = text.trim_end_matches([' ', '\t']).chars().count();
                if start + kept < end {
                    changes.push(Change { start: start + kept, end, text: String::new(), cursor_offset: None });
                }
            }
        }
        let len = self.buffer.len_chars();
        if self.insert_final_newline && len > 0 && !self.buffer.rope().char(len - 1).eq(&'\n') {
            changes.push(Change {
                start: len,
                end: len,
                text: self.buffer.line_ending.to_owned(),
                cursor_offset: None,
            });
        }
        if !changes.is_empty() {
            let selections = self.buffer.selections().to_vec();
            self.buffer.apply_changes(changes, false);
            let len = self.buffer.len_chars();
            let kept = selections.iter().map(|s| Selection::new(s.anchor.min(len), s.head.min(len))).collect();
            self.buffer.set_selections(kept, 0);
        }
    }

    /// Replaces every match; returns how many (one undo step).
    pub fn replace_all(&mut self, opts: &crate::editor::find::FindOptions, with: &str) -> Result<usize, String> {
        let found = crate::editor::find::replacements(&self.text(), opts, with)?;
        let count = found.len();
        let changes = found
            .into_iter()
            .map(|r| Change { start: r.start, end: r.end, text: r.text, cursor_offset: None })
            .collect();
        self.buffer.apply_changes(changes, false);
        Ok(count)
    }

    pub fn save(&mut self) -> std::io::Result<bool> {
        if self.path.is_none() {
            return Ok(false);
        }
        self.prepare_for_save();
        let Some(path) = &self.path else {
            return Ok(false);
        };
        if self.encoding == UTF_8 && !self.bom {
            // Common case: stream the rope straight to disk without a copy.
            let file = std::fs::File::create(path)?;
            let mut writer = std::io::BufWriter::new(file);
            self.buffer.rope().write_to(&mut writer)?;
            std::io::Write::flush(&mut writer)?;
        } else {
            let bytes = encoding::encode(&self.text(), self.encoding, self.bom)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            std::fs::write(path, bytes)?;
        }
        self.saved_version = self.buffer.version();
        self.disk = DiskState::InSync;
        Ok(true)
    }

    /// Reacts to the file changing on disk: an unmodified buffer reloads
    /// (keeping the cursor and undo history); a modified one is flagged so
    /// the user can choose. Our own saves read back identical and are ignored.
    pub fn check_disk(&mut self) {
        let Some(path) = &self.path else { return };
        match std::fs::read(path) {
            Ok(bytes) => {
                let text = encoding::decode_with(&bytes, self.encoding).text;
                if *self.buffer.rope() == *text {
                    self.disk = DiskState::InSync;
                } else if self.is_dirty() {
                    self.disk = DiskState::Changed;
                } else {
                    self.load_text(&text);
                }
            }
            Err(_) if !path.exists() => self.disk = DiskState::Deleted,
            Err(_) => {}
        }
    }

    /// Replaces the buffer with the file's current contents (as one undo step).
    pub fn reload_from_disk(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let bytes = std::fs::read(path)?;
        self.load_text(&encoding::decode_with(&bytes, self.encoding).text);
        Ok(())
    }

    /// Re-reads the file as `enc` ("Reopen with Encoding"). Refuses when there
    /// are unsaved changes, since they would be lost.
    pub fn reopen_with_encoding(&mut self, enc: &'static Encoding) -> Result<(), String> {
        let Some(path) = &self.path else { return Err("The file has not been saved yet".to_owned()) };
        if self.is_dirty() {
            return Err("Save or revert your changes before reopening with another encoding".to_owned());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let decoded = encoding::decode_with(&bytes, enc);
        self.load_text(&decoded.text);
        self.encoding = enc;
        self.bom = decoded.bom;
        Ok(())
    }

    /// Display name of the current encoding, e.g. "UTF-8 with BOM".
    pub fn encoding_name(&self) -> String {
        encoding::display_name(self.encoding, self.bom)
    }

    /// Converts every line ending to `nl` ("\n" or "\r\n") as one undo step;
    /// new lines typed afterwards use it too.
    pub fn set_line_ending(&mut self, nl: &'static str) {
        self.buffer.convert_line_endings(nl);
        self.reveal = true;
    }

    /// Fold ranges and outline symbols for the current text.
    pub fn structure(&mut self) -> &Structure {
        let key = (self.buffer.version(), self.ts.as_ref().and_then(TsState::tree_version));
        if self.structure.key != Some(key) {
            let (folds, symbols) = match &self.ts {
                Some(ts) => match ts.current_tree(&self.buffer) {
                    Some(tree) => {
                        (structure::folds_from_tree(&tree, ts.lang), structure::symbols(&tree, &self.text(), ts.lang))
                    }
                    None => (Vec::new(), Vec::new()),
                },
                None if !self.large => (structure::indent_folds(self.buffer.rope()), Vec::new()),
                None => (Vec::new(), Vec::new()),
            };
            self.structure = Structure { key: Some(key), folds, symbols };
        }
        &self.structure
    }

    /// A syntax tree exactly matching the text (for selection commands).
    pub fn fresh_tree(&self) -> Option<tree_sitter::Tree> {
        self.ts.as_ref()?.fresh_tree(&self.buffer)
    }

    fn load_text(&mut self, text: &str) {
        let (anchor, head) = self.selection();
        let len = text.chars().count();
        self.buffer.replace_all(text, Selection::new(anchor.min(len), head.min(len)));
        self.saved_version = self.buffer.version();
        self.disk = DiskState::InSync;
    }

    pub fn save_as(&mut self, path: PathBuf) -> std::io::Result<()> {
        self.set_language(&extension_of(Some(&path)));
        self.path = Some(path);
        self.save().map(|_| ())
    }

    /// File extension, used as the syntax-highlighting language token.
    pub fn extension(&self) -> String {
        extension_of(self.path.as_deref())
    }

    pub fn language_name(&self) -> &'static str {
        language_name(&self.extension())
    }

    pub fn line_ending(&self) -> &'static str {
        if self.buffer.line_ending == "\r\n" { "CRLF" } else { "LF" }
    }

    /// 1-based line and column of the primary cursor.
    pub fn cursor(&self) -> (usize, usize) {
        self.buffer.line_col(self.buffer.primary().head)
    }

    /// Total selected characters across all selections.
    pub fn selection_len(&self) -> usize {
        self.buffer.selections().iter().map(|s| s.end() - s.start()).sum()
    }

    /// Primary selection as `(anchor, cursor)`.
    pub fn selection(&self) -> (usize, usize) {
        let p = self.buffer.primary();
        (p.anchor, p.head)
    }

    /// Selects `[anchor, head)`, focuses the editor and scrolls there.
    pub fn select(&mut self, anchor: usize, head: usize) {
        self.buffer.set_single(Selection::new(anchor, head));
        self.request_focus = true;
        self.reveal = true;
    }

    /// Selects the given 1-based line and scrolls to it.
    pub fn goto_line(&mut self, line: usize) {
        let line = line.saturating_sub(1).min(self.buffer.len_lines() - 1);
        let (start, end) = (self.buffer.line_start(line), self.buffer.line_end(line));
        self.select(start, end);
    }
}

fn extension_of(path: Option<&Path>) -> String {
    path.and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "txt".to_owned())
}

pub fn language_name(ext: &str) -> &'static str {
    use crate::language::lsp::registry::by_extension;
    if let Some(language) = by_extension(ext, true) {
        return language.name;
    }
    let name = built_in_language_name(ext);
    if name == "Plain Text"
        && let Some(language) = by_extension(ext, false)
    {
        return language.name;
    }
    name
}

fn built_in_language_name(ext: &str) -> &'static str {
    match ext {
        "rs" => "Rust",
        "toml" => "TOML",
        "md" => "Markdown",
        "py" | "pyw" | "pyi" => "Python",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "mts" | "cts" => "TypeScript",
        "tsx" => "TypeScript React",
        "jsx" => "JavaScript React",
        "json" => "JSON",
        "csv" => "CSV",
        "tsv" | "tab" => "TSV",
        "html" | "htm" | "xhtml" => "HTML",
        "css" => "CSS",
        "scss" => "SCSS",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "inl" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "go" => "Go",
        "sh" | "bash" | "zsh" => "Shell Script",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "rb" => "Ruby",
        "php" => "PHP",
        "dart" => "Dart",
        "scala" => "Scala",
        "r" => "R",
        "ps1" => "PowerShell",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "sql" => "SQL",
        "lua" => "Lua",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "svg" => "SVG",
        "xaml" => "XAML",
        "csproj" | "fsproj" | "vbproj" | "props" | "targets" => "MSBuild",
        "jinja" | "jinja2" | "j2" => "Jinja",
        "njk" => "Nunjucks",
        "hbs" | "handlebars" => "Handlebars",
        "ejs" => "EJS",
        "twig" => "Twig",
        "liquid" => "Liquid",
        _ => crate::language::languages::name(ext)
            .or_else(|| crate::language::lsp::registry::by_extension(ext, false).map(|l| l.name))
            .unwrap_or("Plain Text"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_in_strings_neither_match_nor_count() {
        // Ruby is highlighted by syntect here: its scopes say where strings are.
        let mut doc = Document::from_text(Some(PathBuf::from("t.rb")), 0, "f(\")\", [1])");
        assert!(doc.ts.is_none());
        assert_eq!(doc.matching_bracket(1, 1000), Some((1, 10)));
        assert_eq!(doc.matching_bracket(3, 1000), None, "the ) in the string");
        assert_eq!(doc.enclosing_brackets(8, 1000), Some((7, 9)));
        assert_eq!(doc.enclosing_brackets(6, 1000), Some((1, 10)));
        assert_eq!(doc.bracket_ahead(2), Some(7));
    }
}
