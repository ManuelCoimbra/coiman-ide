//! An open file (or untitled buffer) shown in an editor tab.

use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Selection};
use crate::highlight::Highlighter;
use crate::syntax::{Lang, TsState};

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

pub struct Document {
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
}

impl Document {
    fn from_text(path: Option<PathBuf>, untitled_index: usize, text: &str) -> Self {
        let buffer = Buffer::new(text);
        let large = text.len() > LARGE_FILE_BYTES || buffer.len_lines() > LARGE_FILE_LINES;
        let ext = extension_of(path.as_deref());
        let mut doc = Self {
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
        let text = String::from_utf8_lossy(&bytes);
        Ok(Self::from_text(Some(path.to_path_buf()), 0, &text))
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
    pub fn save(&mut self) -> std::io::Result<bool> {
        let Some(path) = &self.path else {
            return Ok(false);
        };
        let file = std::fs::File::create(path)?;
        let mut writer = std::io::BufWriter::new(file);
        self.buffer.rope().write_to(&mut writer)?;
        std::io::Write::flush(&mut writer)?;
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
                let text = String::from_utf8_lossy(&bytes);
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
        self.load_text(&String::from_utf8_lossy(&bytes));
        Ok(())
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
    match ext {
        "rs" => "Rust",
        "toml" => "TOML",
        "md" => "Markdown",
        "py" => "Python",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" => "TypeScript",
        "tsx" => "TypeScript React",
        "jsx" => "JavaScript React",
        "json" => "JSON",
        "html" | "htm" => "HTML",
        "css" => "CSS",
        "c" | "h" => "C",
        "cpp" | "cc" | "hpp" | "cxx" => "C++",
        "cs" => "C#",
        "java" => "Java",
        "go" => "Go",
        "sh" | "bash" => "Shell Script",
        "ps1" => "PowerShell",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "sql" => "SQL",
        "lua" => "Lua",
        "sii" | "sui" => "SCS Unit",
        _ => "Plain Text",
    }
}
