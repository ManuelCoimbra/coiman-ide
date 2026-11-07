//! Rope-backed text buffer with multiple selections and undo/redo.
//!
//! Positions are char indices into the rope. Lines are split on `\n` only
//! (ropey's `unicode_lines`/`cr_lines` features are off), so a CRLF line keeps
//! its `\r` as the last visible-but-skipped char; see [`Buffer::line_end`].

use ropey::Rope;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::auto_tags::{self, TagMode};
use crate::edit_ops::{Edit, Sel};
use crate::indent_rules::{self, IndentRules};
use tree_sitter::{InputEdit, Point};

pub const TAB_WIDTH: usize = 4;

/// Columns a tab takes on screen (`editor.tab_size`).
static SHOWN_TAB_WIDTH: AtomicUsize = AtomicUsize::new(TAB_WIDTH);

pub fn tab_width() -> usize {
    SHOWN_TAB_WIDTH.load(Ordering::Relaxed)
}

pub fn set_tab_width(columns: usize) {
    SHOWN_TAB_WIDTH.store(columns.clamp(1, 16), Ordering::Relaxed);
}
const MAX_UNDO: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Visual column to aim for when moving vertically through shorter lines.
    pub goal: Option<usize>,
}

impl Selection {
    pub fn cursor(pos: usize) -> Self {
        Self { anchor: pos, head: pos, goal: None }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self { anchor, head, goal: None }
    }

    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

/// What kind of edit happened last, so consecutive typing can share one undo step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditKind {
    None,
    Typing,
    Deleting,
    Other,
}

#[derive(Clone)]
struct Snapshot {
    rope: Rope,
    selections: Vec<Selection>,
    primary: usize,
    version: u64,
}

/// A change for the tree-sitter parser: an exact edit, or "reparse from scratch".
#[derive(Clone, Debug)]
pub enum SyntaxEdit {
    Edit(InputEdit),
    Reset,
}

/// One replacement: `[start, end)` becomes `text`, and the selection that
/// produced it lands at `start + cursor_offset` (defaults to the end of `text`).
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub cursor_offset: Option<usize>,
}

/// How a document indents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Indent {
    pub use_tabs: bool,
    /// Columns per level.
    pub size: usize,
}

impl Default for Indent {
    fn default() -> Self {
        Self { use_tabs: false, size: TAB_WIDTH }
    }
}

impl Indent {
    /// The text of one level.
    pub fn unit(self) -> String {
        if self.use_tabs { "\t".to_owned() } else { " ".repeat(self.size) }
    }

    /// `indent` one level shallower: a tab, or spaces back to the tab stop before.
    pub fn outdented(self, indent: &str) -> String {
        if let Some(rest) = indent.strip_suffix('\t') {
            return rest.to_owned();
        }
        let spaces = indent.len() - indent.trim_end_matches(' ').len();
        let size = self.size.max(1);
        let drop = match spaces % size {
            0 => size.min(spaces),
            partial => partial,
        };
        indent[..indent.len() - drop].to_owned()
    }
}

fn leading_whitespace(line: &str) -> String {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Typed characters that get a partner.
fn closer_of(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '"' => '"',
        '\'' => '\'',
        '`' => '`',
        _ => return None,
    })
}

pub struct Buffer {
    /// Refuse every edit, such as in a decompiled class from a language server.
    pub read_only: bool,
    /// How Enter and Tab indent.
    pub indent: Indent,
    /// Type brackets and quotes in pairs.
    pub auto_close: bool,
    /// Whether `'` pairs too (not in Rust, where it starts lifetimes).
    pub pair_single_quote: bool,
    /// What opens and closes a level for Enter and typing, by language.
    pub indent_rules: IndentRules,
    /// Which markup gets its end tags typed for it.
    pub auto_tags: TagMode,
    /// Whether `=` after an attribute's name adds `""`.
    pub auto_quotes: bool,
    /// Enter after `/**` above a function writes its Doxygen comment (`@brief`, `@param`,
    /// `@return`); otherwise it only closes the comment.
    pub doc_comments: bool,
    rope: Rope,
    selections: Vec<Selection>,
    primary: usize,
    /// Identifies the current content; undo restores old versions, so
    /// "dirty" can be a cheap comparison against the saved version.
    version: u64,
    next_version: u64,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last_edit: EditKind,
    /// Whether the last typed text was whitespace (for undo grouping).
    last_typed_space: bool,
    /// Smallest line touched since the last call to [`Buffer::take_edited_from_line`].
    edited_from_line: Option<usize>,
    /// Line ending used for new lines ("\n" or "\r\n").
    pub line_ending: &'static str,
    /// Edits not yet sent to the syntax parser; only recorded when enabled.
    syntax_edits: Option<Vec<SyntaxEdit>>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        // Like VS Code, decide from the start of the file instead of scanning all of it.
        let probe = &text.as_bytes()[..text.len().min(64 * 1024)];
        let line_ending = if probe.windows(2).any(|w| w == b"\r\n") { "\r\n" } else { "\n" };
        Self {
            indent: Indent::default(),
            auto_close: true,
            read_only: false,
            pair_single_quote: true,
            indent_rules: IndentRules::default(),
            auto_tags: TagMode::Off,
            auto_quotes: false,
            doc_comments: false,
            rope: Rope::from_str(text),
            selections: vec![Selection::cursor(0)],
            primary: 0,
            version: 0,
            next_version: 1,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: EditKind::None,
            last_typed_space: false,
            edited_from_line: None,
            line_ending,
            syntax_edits: None,
        }
    }

    // ----- read access -----------------------------------------------------------

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn selections(&self) -> &[Selection] {
        &self.selections
    }

    pub fn primary(&self) -> Selection {
        self.selections[self.primary]
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn slice_string(&self, start: usize, end: usize) -> String {
        self.rope.slice(start..end).to_string()
    }

    pub fn char_to_line(&self, pos: usize) -> usize {
        self.rope.char_to_line(pos.min(self.len_chars()))
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.rope.line_to_char(line)
    }

    /// Char index just before the line's `\n` (and `\r` for CRLF).
    pub fn line_end(&self, line: usize) -> usize {
        let start = self.line_start(line);
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        start + len
    }

    /// The line's text without its line ending.
    pub fn line_text(&self, line: usize) -> String {
        self.rope.slice(self.line_start(line)..self.line_end(line)).to_string()
    }

    /// 1-based (line, column) of a position.
    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let line = self.char_to_line(pos);
        (line + 1, pos - self.line_start(line) + 1)
    }

    /// Visual column of `pos`, expanding tabs to [`tab_width`].
    pub fn visual_col(&self, pos: usize) -> usize {
        let line = self.char_to_line(pos);
        let start = self.line_start(line);
        visual_width(self.rope.slice(start..pos).chars())
    }

    /// Position on `line` closest to visual column `vcol`.
    pub fn pos_at_visual_col(&self, line: usize, vcol: usize) -> usize {
        let start = self.line_start(line);
        let end = self.line_end(line);
        let mut col = 0;
        for (i, c) in self.rope.slice(start..end).chars().enumerate() {
            let w = char_width(c, col);
            if vcol < col + w {
                // Inside a wide char (tab): snap to whichever side is nearer.
                return start + i + usize::from(w > 1 && (vcol - col) * 2 >= w);
            }
            col += w;
        }
        end
    }

    pub fn selected_texts(&self) -> Vec<String> {
        self.selections.iter().map(|s| self.slice_string(s.start(), s.end())).collect()
    }

    /// Starts recording [`SyntaxEdit`]s for an incremental parser.
    pub fn track_syntax_edits(&mut self) {
        self.syntax_edits = Some(vec![SyntaxEdit::Reset]);
    }

    /// Edits recorded since the last call (oldest first).
    pub fn take_syntax_edits(&mut self) -> Vec<SyntaxEdit> {
        self.syntax_edits.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// Edits recorded but not yet taken.
    pub fn peek_syntax_edits(&self) -> &[SyntaxEdit] {
        self.syntax_edits.as_deref().unwrap_or(&[])
    }

    fn record_reset(&mut self) {
        if let Some(edits) = &mut self.syntax_edits {
            edits.clear();
            edits.push(SyntaxEdit::Reset);
        }
    }

    /// Tree-sitter position of a byte offset (row, byte column).
    pub fn point_at_byte(&self, byte: usize) -> Point {
        let row = self.rope.byte_to_line(byte);
        Point { row, column: byte - self.rope.line_to_byte(row) }
    }

    /// Smallest edited line since the last call, for incremental re-highlighting.
    pub fn take_edited_from_line(&mut self) -> Option<usize> {
        self.edited_from_line.take()
    }

    // ----- selection management --------------------------------------------------

    pub fn set_selections(&mut self, mut selections: Vec<Selection>, primary: usize) {
        if selections.is_empty() {
            selections.push(Selection::cursor(0));
        }
        let len = self.len_chars();
        for s in &mut selections {
            s.anchor = s.anchor.min(len);
            s.head = s.head.min(len);
        }
        self.primary = primary.min(selections.len() - 1);
        self.selections = selections;
        self.normalize();
        self.last_edit = EditKind::None;
    }

    pub fn set_single(&mut self, sel: Selection) {
        self.set_selections(vec![sel], 0);
    }

    pub fn add_selection(&mut self, sel: Selection) {
        let mut sels = self.selections.clone();
        sels.push(sel);
        let primary = sels.len() - 1;
        self.set_selections(sels, primary);
    }

    pub fn collapse_to_primary(&mut self) {
        let p = self.primary();
        self.set_single(p);
    }

    /// Sorts selections and merges overlapping ones, keeping track of the primary.
    fn normalize(&mut self) {
        let primary_sel = self.selections[self.primary];
        self.selections.sort_by_key(|s| (s.start(), s.end()));
        let mut merged: Vec<Selection> = Vec::with_capacity(self.selections.len());
        for s in self.selections.drain(..) {
            if let Some(last) = merged.last_mut() {
                // Overlapping selections, or two at the same start, become one.
                if s.start() < last.end() || s.start() == last.start() {
                    let (start, end) = (last.start(), last.end().max(s.end()));
                    let forward = last.head >= last.anchor;
                    *last = if forward { Selection::new(start, end) } else { Selection::new(end, start) };
                    continue;
                }
            }
            merged.push(s);
        }
        self.primary = merged
            .iter()
            .position(|s| s.start() <= primary_sel.start() && primary_sel.start() <= s.end())
            .unwrap_or(merged.len() - 1);
        self.selections = merged;
    }

    pub fn select_all(&mut self) {
        let len = self.len_chars();
        self.set_single(Selection::new(0, len));
    }

    /// Moves every selection's head with `f`. With `extend`, anchors stay put.
    pub fn move_heads(&mut self, extend: bool, f: impl Fn(&Buffer, Selection) -> (usize, Option<usize>)) {
        let moved: Vec<Selection> = self
            .selections
            .iter()
            .map(|&s| {
                let (head, goal) = f(self, s);
                Selection { anchor: if extend { s.anchor } else { head }, head, goal }
            })
            .collect();
        let primary = self.primary;
        self.set_selections(moved, primary);
    }

    pub fn move_horizontal(&mut self, forward: bool, word: bool, extend: bool) {
        self.move_heads(extend, |b, s| {
            // Without shift, an arrow collapses a selection to its edge (like VS Code).
            if !extend && !s.is_empty() && !word {
                return (if forward { s.end() } else { s.start() }, None);
            }
            let pos = match (forward, word) {
                (true, false) => (s.head + 1).min(b.len_chars()),
                (false, false) => s.head.saturating_sub(1),
                (true, true) => b.next_word_boundary(s.head),
                (false, true) => b.prev_word_boundary(s.head),
            };
            (b.skip_cr(pos, forward), None)
        });
    }

    pub fn move_vertical(&mut self, lines: isize, extend: bool) {
        self.move_heads(extend, |b, s| {
            let line = b.char_to_line(s.head) as isize;
            let goal = s.goal.unwrap_or_else(|| b.visual_col(s.head));
            let target = line + lines;
            if target < 0 {
                return (0, Some(goal));
            }
            if target as usize >= b.len_lines() {
                return (b.len_chars(), Some(goal));
            }
            (b.pos_at_visual_col(target as usize, goal), Some(goal))
        });
    }

    /// Home toggles between the first non-blank char and column 0 (smart home).
    pub fn move_home(&mut self, extend: bool) {
        self.move_heads(extend, |b, s| {
            let line = b.char_to_line(s.head);
            let start = b.line_start(line);
            let indent_end = start + b.line_text(line).chars().take_while(|c| c.is_whitespace()).count();
            (if s.head == indent_end { start } else { indent_end }, None)
        });
    }

    pub fn move_end(&mut self, extend: bool) {
        self.move_heads(extend, |b, s| (b.line_end(b.char_to_line(s.head)), None));
    }

    pub fn move_doc_edge(&mut self, end: bool, extend: bool) {
        self.move_heads(extend, |b, _| (if end { b.len_chars() } else { 0 }, None));
    }

    /// Moves left past a `\r` that belongs to a CRLF so the cursor never sits between them.
    fn skip_cr(&self, pos: usize, forward: bool) -> usize {
        if pos > 0 && pos < self.len_chars() && self.rope.char(pos) == '\n' && self.rope.char(pos - 1) == '\r' {
            return if forward { pos + 1 } else { pos - 1 };
        }
        pos
    }

    pub fn next_word_boundary(&self, pos: usize) -> usize {
        let len = self.len_chars();
        let mut chars = self.rope.chars_at(pos).peekable();
        let mut i = pos;
        while let Some(&c) = chars.peek() {
            if c == '\n' || !c.is_whitespace() {
                break;
            }
            chars.next();
            i += 1;
        }
        if let Some(&c) = chars.peek() {
            if c == '\n' && i == pos {
                return (i + 1).min(len);
            }
            let class = char_class(c);
            while let Some(&c) = chars.peek() {
                if c == '\n' || char_class(c) != class {
                    break;
                }
                chars.next();
                i += 1;
            }
        }
        i
    }

    pub fn prev_word_boundary(&self, pos: usize) -> usize {
        let mut chars = self.rope.chars_at(pos);
        let mut i = pos;
        let mut prev = chars.prev();
        while let Some(c) = prev {
            if c == '\n' || !c.is_whitespace() {
                break;
            }
            i -= 1;
            prev = chars.prev();
        }
        if let Some(c) = prev {
            if c == '\n' && i == pos {
                return i - 1;
            }
            let class = char_class(c);
            while let Some(c) = prev {
                if c == '\n' || char_class(c) != class {
                    break;
                }
                i -= 1;
                prev = chars.prev();
            }
        }
        i
    }

    /// `[start, end)` of the word at `pos`, if any.
    pub fn word_at(&self, pos: usize) -> Option<(usize, usize)> {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let len = self.len_chars();
        let mut start = pos;
        while start > 0 && is_word(self.rope.char(start - 1)) {
            start -= 1;
        }
        let mut end = pos;
        while end < len && is_word(self.rope.char(end)) {
            end += 1;
        }
        (start < end).then_some((start, end))
    }

    /// `editor.action.addSelectionToNextFindMatch` (Ctrl+D): selects the word at
    /// the cursor, then adds a selection at each next occurrence (wrapping).
    pub fn add_next_occurrence(&mut self) {
        let primary = self.primary();
        if primary.is_empty() {
            if let Some((s, e)) = self.word_at(primary.head) {
                let mut sels = self.selections.clone();
                sels[self.primary] = Selection::new(s, e);
                let p = self.primary;
                self.set_selections(sels, p);
            }
            return;
        }
        let needle: Vec<char> = self.rope.slice(primary.start()..primary.end()).chars().collect();
        let from = self.selections.iter().map(Selection::end).max().unwrap_or(0);
        let found = self.find_chars(&needle, from, self.len_chars()).or_else(|| self.find_chars(&needle, 0, from));
        if let Some(start) = found {
            let sel = Selection::new(start, start + needle.len());
            if !self.selections.iter().any(|s| s.start() == sel.start()) {
                self.add_selection(sel);
            }
        }
    }

    /// Selects every occurrence of the primary selection. With no selection,
    /// takes the word at the cursor and matches it as a whole word.
    pub fn select_all_occurrences(&mut self) {
        let primary = self.primary();
        let whole_word = primary.is_empty();
        let (start, end) = if whole_word {
            let Some(word) = self.word_at(primary.head) else { return };
            word
        } else {
            (primary.start(), primary.end())
        };
        let needle: Vec<char> = self.rope.slice(start..end).chars().collect();
        let hay: Vec<char> = self.rope.chars().collect();
        let mut sels = Vec::new();
        let mut at = 0;
        while at + needle.len() <= hay.len() {
            let hit = hay[at..at + needle.len()] == needle[..]
                && (!whole_word || self.word_at(at) == Some((at, at + needle.len())));
            if hit {
                sels.push(Selection::new(at, at + needle.len()));
                at += needle.len();
            } else {
                at += 1;
            }
        }
        let primary = sels.iter().position(|s| s.start() == start).unwrap_or(0);
        self.set_selections(sels, primary);
    }

    /// Column selection: one selection per line in the block between two
    /// (line, visual column) corners. Lines too short to reach it are left out.
    pub fn select_column(&mut self, anchor: (usize, usize), head: (usize, usize)) {
        let last = self.len_lines() - 1;
        let (anchor_line, head_line) = (anchor.0.min(last), head.0.min(last));
        let left = anchor.1.min(head.1);
        let mut sels = Vec::new();
        let mut primary = 0;
        for line in anchor_line.min(head_line)..=anchor_line.max(head_line) {
            if self.visual_col(self.line_end(line)) < left && line != head_line {
                continue;
            }
            if line == head_line {
                primary = sels.len();
            }
            let from = self.pos_at_visual_col(line, anchor.1);
            sels.push(Selection::new(from, self.pos_at_visual_col(line, head.1)));
        }
        self.set_selections(sels, primary);
    }

    fn find_chars(&self, needle: &[char], from: usize, to: usize) -> Option<usize> {
        if needle.is_empty() || to < from + needle.len() {
            return None;
        }
        // Rope chars iterator is O(1) amortized per step; fine for interactive use.
        let hay: Vec<char> = self.rope.slice(from..to).chars().collect();
        hay.windows(needle.len()).position(|w| w == needle).map(|i| from + i)
    }

    /// The innermost bracket pair around `pos`: an opener before it and its closer at or
    /// after it, searching at most `limit` chars each way.
    pub fn enclosing_brackets(&self, pos: usize, limit: usize) -> Option<(usize, usize)> {
        self.enclosing_brackets_where(pos, limit, &mut |_| true)
    }

    /// [`Buffer::enclosing_brackets`] counting only brackets where `is_code` says so, so
    /// those in strings and comments are passed over.
    pub fn enclosing_brackets_where(
        &self,
        pos: usize,
        limit: usize,
        is_code: &mut dyn FnMut(usize) -> bool,
    ) -> Option<(usize, usize)> {
        let pos = pos.min(self.len_chars());
        let mut closers: Vec<char> = Vec::new();
        let mut chars = self.rope.chars_at(pos);
        let mut i = pos;
        while let Some(c) = chars.prev() {
            i -= 1;
            if pos - i > limit {
                return None;
            }
            if !matches!(c, '(' | ')' | '[' | ']' | '{' | '}') || !is_code(i) {
                continue;
            }
            let close = match c {
                ')' | ']' | '}' => {
                    closers.push(c);
                    continue;
                }
                '(' => ')',
                '[' => ']',
                '{' => '}',
                _ => continue,
            };
            match closers.last() {
                Some(&last) if last == close => {
                    closers.pop();
                }
                Some(_) => {}
                None => {
                    let (open, end) = self.matching_bracket_where(i, limit, is_code)?;
                    return (open == i && end >= pos).then_some((open, end));
                }
            }
        }
        None
    }

    /// Bracket next to `pos` and its partner, searching at most `limit` chars away.
    pub fn matching_bracket(&self, pos: usize, limit: usize) -> Option<(usize, usize)> {
        self.matching_bracket_where(pos, limit, &mut |_| true)
    }

    /// [`Buffer::matching_bracket`] counting only brackets where `is_code` says so: a
    /// bracket in a string or comment neither matches nor is matched.
    pub fn matching_bracket_where(
        &self,
        pos: usize,
        limit: usize,
        is_code: &mut dyn FnMut(usize) -> bool,
    ) -> Option<(usize, usize)> {
        const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
        let len = self.len_chars();
        for at in [pos, pos.wrapping_sub(1)] {
            if at >= len || !"()[]{}".contains(self.rope.char(at)) || !is_code(at) {
                continue;
            }
            let c = self.rope.char(at);
            for (open, close) in PAIRS {
                let forward = c == open;
                if !forward && c != close {
                    continue;
                }
                let mut depth = 0usize;
                if forward {
                    for (i, ch) in self.rope.chars_at(at).enumerate().take(limit) {
                        if (ch == open || ch == close) && !is_code(at + i) {
                            continue;
                        }
                        if ch == open {
                            depth += 1;
                        } else if ch == close {
                            depth -= 1;
                            if depth == 0 {
                                return Some((at, at + i));
                            }
                        }
                    }
                } else {
                    let mut chars = self.rope.chars_at(at + 1);
                    let mut i = at + 1;
                    while let Some(ch) = chars.prev() {
                        i -= 1;
                        if at - i > limit {
                            break;
                        }
                        if (ch == open || ch == close) && !is_code(i) {
                            continue;
                        }
                        if ch == close {
                            depth += 1;
                        } else if ch == open {
                            depth -= 1;
                            if depth == 0 {
                                return Some((i, at));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Adds a cursor on the line above/below each existing cursor (Ctrl+Alt+Up/Down).
    pub fn add_cursor_vertical(&mut self, down: bool) {
        let p = self.primary();
        let line = self.char_to_line(p.head);
        let goal = p.goal.unwrap_or_else(|| self.visual_col(p.head));
        let target = if down { line + 1 } else { line.wrapping_sub(1) };
        if target >= self.len_lines() {
            return;
        }
        let pos = self.pos_at_visual_col(target, goal);
        let mut sel = Selection::cursor(pos);
        sel.goal = Some(goal);
        self.add_selection(sel);
    }

    // ----- editing ---------------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            rope: self.rope.clone(),
            selections: self.selections.clone(),
            primary: self.primary,
            version: self.version,
        }
    }

    /// Records an undo step unless this edit continues the previous one.
    fn begin_edit(&mut self, kind: EditKind) {
        if self.read_only {
            return;
        }
        let continues = kind == self.last_edit && matches!(kind, EditKind::Typing | EditKind::Deleting);
        if !continues {
            self.undo.push(self.snapshot());
            if self.undo.len() > MAX_UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last_edit = kind;
    }

    fn finish_edit(&mut self, first_line: usize) {
        if self.read_only {
            return;
        }
        self.version = self.next_version;
        self.next_version += 1;
        self.edited_from_line = Some(self.edited_from_line.map_or(first_line, |l| l.min(first_line)));
    }

    /// Applies non-overlapping changes (one per selection) and moves each
    /// selection to its change's cursor position.
    pub fn apply_changes(&mut self, changes: Vec<Change>, kind_typing: bool) {
        let kind = if kind_typing {
            EditKind::Typing
        } else if changes.iter().all(|c| c.text.is_empty()) {
            EditKind::Deleting
        } else {
            EditKind::Other
        };
        self.apply_changes_as(changes, kind);
    }

    fn apply_changes_as(&mut self, mut changes: Vec<Change>, kind: EditKind) {
        if changes.is_empty() || self.read_only {
            return;
        }
        // A word typed after whitespace starts a new undo step, like VS Code.
        let typed_space = changes.iter().all(|c| c.text.chars().all(char::is_whitespace));
        if kind == EditKind::Typing && self.last_typed_space && !typed_space {
            self.last_edit = EditKind::None;
        }
        self.last_typed_space = kind == EditKind::Typing && typed_space;
        self.begin_edit(kind);

        changes.sort_by_key(|c| c.start);
        let first_line = self.char_to_line(changes[0].start);
        let primary_start = self.selections[self.primary].start();
        let mut delta: isize = 0;
        let mut new_sels = Vec::with_capacity(changes.len());
        let mut new_primary = 0;
        for c in changes {
            let start = (c.start as isize + delta) as usize;
            let end = (c.end as isize + delta) as usize;
            let inserted = self.splice(start, end, &c.text);
            // An offset past the inserted text points into the text that followed it.
            let cursor = (start + c.cursor_offset.unwrap_or(inserted)).min(self.rope.len_chars());
            if c.start <= primary_start {
                new_primary = new_sels.len();
            }
            new_sels.push(Selection::cursor(cursor));
            delta += inserted as isize - (c.end - c.start) as isize;
        }
        let kind_before = self.last_edit;
        self.set_selections(new_sels, new_primary);
        self.last_edit = kind_before;
        self.finish_edit(first_line);
    }

    /// Replaces `[start, end)` with `text`, recording the edit for the syntax
    /// parser. Returns the number of chars inserted.
    fn splice(&mut self, start: usize, end: usize, text: &str) -> usize {
        if self.read_only {
            return 0;
        }
        let before = self.syntax_edits.is_some().then(|| {
            let (sb, eb) = (self.rope.char_to_byte(start), self.rope.char_to_byte(end));
            (sb, eb, self.point_at_byte(sb), self.point_at_byte(eb))
        });
        if end > start {
            self.rope.remove(start..end);
        }
        let inserted = text.chars().count();
        if inserted > 0 {
            self.rope.insert(start, text);
        }
        if let Some((start_byte, old_end_byte, start_position, old_end_position)) = before {
            let new_end_byte = start_byte + text.len();
            let edit = InputEdit {
                start_byte,
                old_end_byte,
                new_end_byte,
                start_position,
                old_end_position,
                new_end_position: self.point_at_byte(new_end_byte),
            };
            if let Some(edits) = &mut self.syntax_edits {
                edits.push(SyntaxEdit::Edit(edit));
            }
        }
        inserted
    }

    /// Applies non-overlapping edits given in the current text's coordinates
    /// (e.g. from a language server) as one undo step. Selections move with
    /// the text around them; one inside a replaced span ends up after it.
    pub fn apply_edits(&mut self, edits: Vec<(usize, usize, String)>) {
        self.apply_edits_in(edits, false);
    }

    /// `apply_edits` as part of the last undo step, which goes on: a change that follows
    /// from what was just typed (a tag's partner renamed) is undone with it.
    pub fn apply_edits_joined(&mut self, edits: Vec<(usize, usize, String)>) {
        self.apply_edits_in(edits, true);
    }

    fn apply_edits_in(&mut self, mut edits: Vec<(usize, usize, String)>, joined: bool) {
        if self.read_only {
            return;
        }
        let len = self.len_chars();
        for e in &mut edits {
            e.1 = e.1.min(len);
            e.0 = e.0.min(e.1);
        }
        // Servers may replace much more than changes: only what differs is an edit, so
        // cursors and undo stay where they make sense.
        for e in &mut edits {
            let old: Vec<char> = self.rope().slice(e.0..e.1).chars().collect();
            let new: Vec<char> = e.2.chars().collect();
            let same = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
            let tail = old[same..].iter().rev().zip(new[same..].iter().rev()).take_while(|(a, b)| a == b).count();
            e.0 += same;
            e.1 -= tail;
            e.2 = new[same..new.len() - tail].iter().collect();
        }
        edits.retain(|(s, e, t)| e > s || !t.is_empty());
        if edits.is_empty() {
            return;
        }
        edits.sort_by_key(|(s, e, _)| (*s, *e));
        let map = |pos: usize| {
            let mut delta = 0isize;
            for (s, e, t) in &edits {
                let n = t.chars().count() as isize;
                if pos < *s || (pos == *s && pos < *e) {
                    break;
                }
                if pos < *e {
                    return (*s as isize + delta + n) as usize;
                }
                delta += n - (e - s) as isize;
            }
            (pos as isize + delta) as usize
        };
        let sels: Vec<Selection> = self
            .selections
            .iter()
            .map(|s| {
                // A selection an edit cuts into no longer means anything: a cursor remains.
                let cut = |pos: usize| edits.iter().any(|(a, b, _)| *a < pos && pos < *b);
                if !s.is_empty() && (cut(s.anchor) || cut(s.head)) {
                    return Selection { anchor: map(s.head), head: map(s.head), goal: None };
                }
                Selection { anchor: map(s.anchor), head: map(s.head), goal: None }
            })
            .collect();
        let kind_before = self.last_edit;
        if joined && !self.undo.is_empty() {
            self.redo.clear();
        } else {
            self.begin_edit(EditKind::Other);
        }
        let first_line = self.char_to_line(edits[0].0);
        // Back to front, so earlier offsets stay valid.
        for (s, e, t) in edits.iter().rev() {
            self.splice(*s, *e, t);
        }
        let primary = self.primary;
        self.set_selections(sels, primary);
        self.last_edit = if joined { kind_before } else { EditKind::Other };
        self.finish_edit(first_line);
    }

    /// Replaces every selection with `text` (typing, paste).
    pub fn insert(&mut self, text: &str) {
        let typing = text.chars().count() == 1;
        let changes = self
            .selections
            .iter()
            .map(|s| Change { start: s.start(), end: s.end(), text: text.to_owned(), cursor_offset: None })
            .collect();
        self.apply_changes(changes, typing);
    }

    /// Replaces each selection with its own text (multi-cursor paste).
    pub fn insert_each(&mut self, texts: &[String]) {
        let changes = self
            .selections
            .iter()
            .zip(texts)
            .map(|(s, t)| Change { start: s.start(), end: s.end(), text: t.clone(), cursor_offset: None })
            .collect();
        self.apply_changes(changes, false);
    }

    /// Enter: new line keeping the current indentation, a level in after what opens one
    /// in the file's language (`{`, Python's `:`, an HTML tag, Ruby's `do`), a level out
    /// after Python's `return`, splitting `{|}` and `<p>|</p>` onto three lines, and
    /// going on with a `/** */` comment. A line left blank keeps no spaces.
    pub fn insert_newline(&mut self) {
        let nl = self.line_ending;
        let rules = self.indent_rules;
        let changes = self
            .selections
            .iter()
            .map(|s| {
                let line = self.char_to_line(s.start());
                let line_start = self.line_start(line);
                let text = self.line_text(line);
                let before: String = text.chars().take(s.start() - line_start).collect();
                let end_line = self.char_to_line(s.end());
                let after: String =
                    self.line_text(end_line).chars().skip(s.end() - self.line_start(end_line)).collect();
                // Enter inside the indentation keeps only what's before the cursor.
                let blank_before = before.trim().is_empty();
                let mut indent = if blank_before { before.clone() } else { leading_whitespace(&text) };
                // Enter after `end` or `fi` finishes the word: its line moves back first.
                let mut moved_back = None;
                if !rules.dedents_on_colon()
                    && rules.dedents(before.trim())
                    && after.trim().is_empty()
                    && let Some(target) = self.block_end_indent(line)
                    && crate::buffer::visual_width(target.chars()) < crate::buffer::visual_width(indent.chars())
                {
                    moved_back = Some(format!("{target}{}", before.trim()));
                    indent = target;
                }
                let trimmed = before.trim_end();
                let code = trimmed.trim_start();
                let (prev, next) = (trimmed.chars().last(), after.chars().next());
                let brackets =
                    matches!((prev, next), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')));
                let tags = rules == IndentRules::Markup
                    && indent_rules::open_tag(trimmed).is_some()
                    && after.trim_start().starts_with("</");
                // `/**` and Enter: the comment closed, and for a function its Doxygen lines.
                if matches!(rules, IndentRules::Brackets)
                    && s.is_empty()
                    && matches!(code, "/**" | "/*!")
                    && after.trim().is_empty()
                    && !self.comment_goes_on(line)
                {
                    let last = (line + 9).min(self.len_lines());
                    let following: Vec<String> = (line + 1..last).map(|l| self.line_text(l)).collect();
                    let (text, cursor) =
                        crate::doc_comment::doc_block(&following.join(" "), &indent, nl, self.doc_comments);
                    return Change { start: s.start(), end: s.end(), text, cursor_offset: Some(cursor) };
                }
                // A `*` line only goes on inside a `/*` comment, not `*ptr = 1;`.
                let comment = matches!(rules, IndentRules::Brackets)
                    && ((code.starts_with("/*") && !code.contains("*/"))
                        || (code.starts_with('*') && !code.starts_with("*/") && self.in_block_comment(line)));
                let inner = if comment {
                    if code.starts_with("/*") { format!("{indent} * ") } else { format!("{indent}* ") }
                } else if rules.opens(trimmed) {
                    format!("{indent}{}", self.indent.unit())
                } else if rules.ends_block(trimmed) {
                    self.indent.outdented(&indent)
                } else {
                    indent.clone()
                };
                let mut new = format!("{nl}{inner}");
                // A line with only spaces before the cursor and nothing after keeps none.
                let mut start =
                    if blank_before && after.trim().is_empty() && s.is_empty() { line_start } else { s.start() };
                if let Some(line) = moved_back {
                    new = format!("{line}{new}");
                    start = line_start;
                }
                let cursor = new.chars().count();
                if brackets || tags {
                    new.push_str(nl);
                    new.push_str(&indent);
                }
                Change { start, end: s.end(), text: new, cursor_offset: Some(cursor) }
            })
            .collect();
        self.apply_changes(changes, false);
    }

    /// Whether a `/*` on `line` is already followed by its comment's lines (` * ...`).
    fn comment_goes_on(&self, line: usize) -> bool {
        (line + 1..(line + 50).min(self.len_lines()))
            .map(|l| self.line_text(l))
            .find(|t| !t.trim().is_empty())
            .is_some_and(|t| t.trim_start().starts_with('*'))
    }

    /// Whether `line`, starting with `*`, carries on a `/*` comment from the lines above.
    fn in_block_comment(&self, line: usize) -> bool {
        for above in (line.saturating_sub(500)..line).rev() {
            let text = self.line_text(above);
            let code = text.trim();
            if code.contains("*/") {
                return false;
            }
            if code.starts_with("/*") {
                return true;
            }
            if !code.starts_with('*') {
                return false;
            }
        }
        false
    }

    /// The indentation of the line holding the bracket that `closer` at `pos` closes.
    fn opener_indent(&self, closer: char, pos: usize) -> Option<String> {
        let opener = match closer {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            _ => return None,
        };
        let mut depth = 0usize;
        let first = pos.saturating_sub(200_000);
        for at in (first..pos).rev() {
            let c = self.rope.char(at);
            if c == closer {
                depth += 1;
            } else if c == opener {
                if depth == 0 {
                    return Some(leading_whitespace(&self.line_text(self.char_to_line(at))));
                }
                depth -= 1;
            }
        }
        None
    }

    /// Where a line that ends a block (`else:`, `end`) belongs: a level out from the line
    /// above, or level with it when that line opened the block.
    fn block_end_indent(&self, line: usize) -> Option<String> {
        let above = (0..line).rev().map(|l| self.line_text(l)).find(|l| !l.trim().is_empty())?;
        let indent = leading_whitespace(&above);
        Some(if self.indent_rules.opens(above.trim_end()) { indent } else { self.indent.outdented(&indent) })
    }

    /// `>`, `</` and `=` in markup: the end tag, the rest of the end tag, or `""`.
    /// Says whether it typed `c`.
    fn type_tag(&mut self, c: char) -> bool {
        if self.auto_tags == TagMode::Off || !matches!(c, '>' | '/' | '=') {
            return false;
        }
        let [s] = self.selections.as_slice() else { return false };
        if !s.is_empty() {
            return false;
        }
        let head = s.head;
        let before = self.rope.slice(head.saturating_sub(65_536)..head).to_string();
        let after = self.rope.slice(head..(head + 256).min(self.len_chars())).to_string();
        let Some(typed) = auto_tags::on_type(self.auto_tags, c, &before, &after, self.auto_quotes) else {
            return false;
        };
        // `</` alone on its line also moves it back a level, as `/` there does without this.
        let line_start = self.line_start(self.char_to_line(head));
        let line_before = self.rope.slice(line_start..head).to_string();
        let (start, prefix) = if c == '/' && self.indent_rules == IndentRules::Markup && line_before.trim_start() == "<"
        {
            (line_start, format!("{}<", self.indent.outdented(&leading_whitespace(&line_before))))
        } else {
            (head, String::new())
        };
        let cursor = prefix.chars().count() + typed.cursor;
        let text = format!("{prefix}{}", typed.text);
        self.apply_changes(vec![Change { start, end: head, text, cursor_offset: Some(cursor) }], true);
        true
    }

    /// Typing that moves its line back a level: a closing bracket or `</` alone on a
    /// line, or a word like `else:`, `end` or `fi` that ends a block. Says whether it typed `c`.
    fn type_outdenting(&mut self, c: char) -> bool {
        let [s] = self.selections.as_slice() else { return false };
        if !s.is_empty() {
            return false;
        }
        let head = s.head;
        let line = self.char_to_line(head);
        let line_start = self.line_start(line);
        let text = self.line_text(line);
        let before: String = text.chars().take(head - line_start).collect();
        let after: String = text.chars().skip(head - line_start).collect();
        let lead = leading_whitespace(&text);
        let rules = self.indent_rules;
        let target = if matches!(c, ')' | ']' | '}') && before.trim().is_empty() {
            // Stepping over a closer already there isn't typing one.
            if self.auto_close && after.starts_with(c) {
                return false;
            }
            self.opener_indent(c, head)
        } else if c == '/' && rules == IndentRules::Markup && before.trim_start() == "<" {
            Some(self.indent.outdented(&lead))
        } else if after.trim().is_empty() && {
            // Python's line is whole with its `:`; a word, once a space or `;` ends it.
            // Not on a bracket or quote, which would then lose its pair.
            let word_ended = c.is_whitespace() || c == ';';
            let finished = if rules.dedents_on_colon() {
                format!("{}{c}", before.trim())
            } else if word_ended {
                before.trim().to_owned()
            } else {
                String::new()
            };
            !finished.is_empty() && rules.dedents(&finished)
        } {
            self.block_end_indent(line)
        } else {
            None
        };
        let Some(target) = target else { return false };
        let width = |s: &str| crate::buffer::visual_width(s.chars());
        // Only ever back: never deeper than it was.
        if width(&target) >= width(&lead) || !before.starts_with(&lead) {
            return false;
        }
        let rest = &before[lead.len()..];
        let text = format!("{target}{rest}{c}");
        let cursor = text.chars().count();
        let change = Change { start: line_start, end: head, text, cursor_offset: Some(cursor) };
        self.apply_changes(vec![change], true);
        true
    }

    /// Tab: spaces up to the next tab stop at each cursor.
    pub fn insert_tab(&mut self) {
        let changes = self
            .selections
            .iter()
            .map(|s| {
                let text = if self.indent.use_tabs {
                    "\t".to_owned()
                } else {
                    let size = self.indent.size.max(1);
                    " ".repeat(size - self.visual_col(s.start()) % size)
                };
                Change { start: s.start(), end: s.end(), text, cursor_offset: None }
            })
            .collect();
        self.apply_changes(changes, false);
    }

    /// Typing: like [`insert`](Self::insert), with brackets and quotes in pairs.
    /// Typing a closer that is already next steps over it, a selection gets
    /// surrounded, and a pair only opens where it can't swallow following text.
    pub fn type_text(&mut self, text: &str) {
        if let (Some(c), 1) = (text.chars().next(), text.chars().count())
            && (self.type_tag(c) || self.type_outdenting(c))
        {
            return;
        }
        let mut chars = text.chars();
        let (Some(c), None, true) = (chars.next(), chars.next(), self.auto_close) else {
            self.insert(text);
            return;
        };
        let at = |pos: usize| (pos < self.len_chars()).then(|| self.rope.char(pos));
        let is_closer = matches!(c, ')' | ']' | '}' | '"' | '`') || (c == '\'' && self.pair_single_quote);
        if is_closer && self.selections.iter().all(|s| s.is_empty() && at(s.head) == Some(c)) {
            let moved = self.selections.iter().map(|s| Selection::cursor(s.head + 1)).collect();
            let primary = self.primary;
            self.set_selections(moved, primary);
            self.last_edit = EditKind::Other;
            return;
        }
        let Some(close) = closer_of(c).filter(|_| c != '\'' || self.pair_single_quote) else {
            self.insert(text);
            return;
        };
        let quote = c == close;
        let changes: Vec<Change> = self
            .selections
            .iter()
            .map(|s| {
                let (start, end) = (s.start(), s.end());
                if start != end {
                    let inner = self.slice_string(start, end);
                    let cursor = inner.chars().count() + 1;
                    return Change { start, end, text: format!("{c}{inner}{close}"), cursor_offset: Some(cursor) };
                }
                let next_ok =
                    at(end).is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';' | ':'));
                let before = start.checked_sub(1).map(|p| self.rope.char(p));
                let prev_ok = !quote || before.is_none_or(|p| !p.is_alphanumeric() && p != c);
                if next_ok && prev_ok {
                    Change { start, end, text: format!("{c}{close}"), cursor_offset: Some(1) }
                } else {
                    Change { start, end, text: c.to_string(), cursor_offset: None }
                }
            })
            .collect();
        self.apply_changes(changes, false);
    }

    pub fn delete_backward(&mut self, word: bool) {
        let changes = self
            .selections
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    return Some(Change { start: s.start(), end: s.end(), text: String::new(), cursor_offset: None });
                }
                if s.head == 0 {
                    return None;
                }
                // Between an empty pair: both go.
                if self.auto_close && !word && s.head < self.len_chars() {
                    let (before, after) = (self.rope.char(s.head - 1), self.rope.char(s.head));
                    let pairs = closer_of(before) == Some(after) && (before != '\'' || self.pair_single_quote);
                    if pairs {
                        return Some(Change {
                            start: s.head - 1,
                            end: s.head + 1,
                            text: String::new(),
                            cursor_offset: None,
                        });
                    }
                }
                // In spaces that indent, back to the tab stop before, a level at a time.
                if !word && !self.indent.use_tabs {
                    let line_start = self.line_start(self.char_to_line(s.head));
                    let before = self.rope.slice(line_start..s.head);
                    let size = self.indent.size.max(1);
                    if before.len_chars() > 1 && before.chars().all(|c| c == ' ') {
                        let col = before.len_chars();
                        let start = line_start + (col - 1) / size * size;
                        return Some(Change { start, end: s.head, text: String::new(), cursor_offset: None });
                    }
                }
                let start = if word { self.prev_word_boundary(s.head) } else { self.skip_cr(s.head - 1, false) };
                Some(Change { start, end: s.head, text: String::new(), cursor_offset: None })
            })
            .collect();
        self.apply_changes(changes, false);
    }

    pub fn delete_forward(&mut self, word: bool) {
        let len = self.len_chars();
        let changes = self
            .selections
            .iter()
            .filter_map(|s| {
                if !s.is_empty() {
                    return Some(Change { start: s.start(), end: s.end(), text: String::new(), cursor_offset: None });
                }
                if s.head >= len {
                    return None;
                }
                let end = if word { self.next_word_boundary(s.head) } else { self.skip_cr(s.head + 1, true) };
                Some(Change { start: s.head, end, text: String::new(), cursor_offset: None })
            })
            .collect();
        self.apply_changes(changes, false);
    }

    /// Replaces the whole text (e.g. reload from disk) and sets one selection.
    /// Only the span that actually differs is edited, so undo and incremental
    /// parsing stay cheap.
    pub fn replace_all(&mut self, text: &str, sel: Selection) {
        let old = self.text();
        self.replace_range_minimal(0, &old, text);
        self.set_single(sel);
    }

    /// Replaces `old` (which starts at char `start`) with `new`, editing only
    /// the part between their common prefix and suffix, as one command step.
    fn replace_range_minimal(&mut self, start: usize, old: &str, new: &str) {
        let o: Vec<char> = old.chars().collect();
        let n: Vec<char> = new.chars().collect();
        let prefix = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
        let max_suffix = o.len().min(n.len()) - prefix;
        let suffix = o.iter().rev().zip(n.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();
        if prefix == o.len() && prefix == n.len() {
            return;
        }
        let text: String = n[prefix..n.len() - suffix].iter().collect();
        let change = Change { start: start + prefix, end: start + o.len() - suffix, text, cursor_offset: None };
        self.apply_changes_as(vec![change], EditKind::Other);
    }

    /// Rewrites every line ending as `nl` and uses it for new lines.
    pub fn convert_line_endings(&mut self, nl: &'static str) {
        let head = self.primary().head;
        let line = self.char_to_line(head);
        let col = head - self.line_start(line);
        let text = self.text().replace("\r\n", "\n");
        let converted = if nl == "\r\n" { text.replace('\n', "\r\n") } else { text };
        self.line_ending = nl;
        self.replace_all(&converted, Selection::cursor(0));
        let pos = (self.line_start(line) + col).min(self.line_end(line));
        self.set_single(Selection::cursor(pos));
    }

    /// Runs a line command from [`crate::edit_ops`] on just the lines around
    /// the primary selection (one line of context on each side, or the whole
    /// document when `whole_doc`), then applies the difference as one small
    /// edit. Returns false if the command did nothing.
    pub fn apply_line_op(&mut self, whole_doc: bool, op: impl FnOnce(&str, Sel) -> Option<Edit>) -> bool {
        let p = self.primary();
        let last_line = self.len_lines() - 1;
        let (first, last) = if whole_doc {
            (0, last_line)
        } else {
            let first = self.char_to_line(p.start());
            let mut last = self.char_to_line(p.end());
            if last > first && self.line_start(last) == p.end() {
                last -= 1;
            }
            (first.saturating_sub(1), (last + 1).min(last_line))
        };
        let start = self.line_start(first);
        let end = self.line_end(last).max(p.end());
        let window = self.slice_string(start, end);
        let Some(edit) = op(&window, (p.anchor - start, p.head - start)) else { return false };
        self.replace_range_minimal(start, &window, &edit.text);
        self.set_single(Selection::new(start + edit.sel.0, start + edit.sel.1));
        true
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rope = snap.rope;
        self.selections = snap.selections;
        self.primary = snap.primary;
        self.version = snap.version;
        self.last_edit = EditKind::None;
        // Undo can change any line; re-highlight everything after the first line.
        self.edited_from_line = Some(0);
        self.record_reset();
    }

    /// Number of undo steps available (Vim groups changes by these).
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else { return false };
        self.redo.push(self.snapshot());
        self.restore(snap);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else { return false };
        self.undo.push(self.snapshot());
        self.restore(snap);
        true
    }
}

fn char_class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

/// Display width of `c` at visual column `col` (tabs jump to the next stop).
pub fn char_width(c: char, col: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    if c == '\t' {
        let width = tab_width();
        width - col % width
    } else if crate::unicode::always(c).is_some() {
        // Shown with a mark of its own, so it keeps a cell.
        1
    } else {
        // Chinese, Japanese and Korean take two cells; combining marks sit on the one before.
        c.width().unwrap_or(1)
    }
}

pub fn visual_width(chars: impl Iterator<Item = char>) -> usize {
    chars.fold(0, |col, c| col + char_width(c, col))
}

/// What differs between two texts, line by line, as edits in chars. A server's own edits
/// can move far more than changes, which takes cursors along for the ride.
pub fn minimal_edits(old: &str, new: &str) -> Vec<(usize, usize, String)> {
    let diff = similar::TextDiff::from_lines(old, new);
    let char_starts = |lines: &[&str]| {
        let mut at = 0;
        let mut starts = Vec::with_capacity(lines.len() + 1);
        for line in lines {
            starts.push(at);
            at += line.chars().count();
        }
        starts.push(at);
        starts
    };
    let old_starts = char_starts(diff.old_slices());
    let new_lines = diff.new_slices();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for op in diff.ops() {
        if matches!(op, similar::DiffOp::Equal { .. }) {
            continue;
        }
        let (from, to) = (op.old_range(), op.new_range());
        let (start, end, text) = (old_starts[from.start], old_starts[from.end], new_lines[to].concat());
        // A removal and an insertion side by side are one change.
        match edits.last_mut() {
            Some((_, last_end, last_text)) if *last_end == start => {
                *last_end = end;
                last_text.push_str(&text);
            }
            _ => edits.push((start, end, text)),
        }
    }
    edits
}

#[cfg(test)]
mod edit_selection_tests {
    use super::*;

    #[test]
    fn finds_what_really_changed() {
        let old = "use a;\n\nfn main() {\n    x;\n}\n\nfn b() {}\n";
        let new = "\n\nfn main() {\n    x;\n}\n\nfn b() {}\n";
        let edits = minimal_edits(old, new);
        assert_eq!(edits, [(0, 7, "\n".to_owned())]);
        // Lines of many bytes count as chars.
        assert_eq!(minimal_edits("é\nb\n", "é\nc\n"), [(2, 4, "c\n".to_owned())]);
        assert!(minimal_edits("same\n", "same\n").is_empty());
        // Applied, they make the new text, and a cursor further down keeps its line.
        let mut b = Buffer::new(old);
        b.set_single(Selection::cursor(14));
        b.apply_edits(edits);
        assert_eq!(b.rope().to_string(), new);
        assert_eq!(b.char_to_line(b.primary().head), 2);
    }

    #[test]
    fn joined_edits_undo_with_the_typing() {
        // `<div></div>`: typing over the first name, the second follows in the same step.
        let mut b = Buffer::new("<div></div>");
        b.set_single(Selection { anchor: 1, head: 4, goal: None });
        b.insert("p");
        b.apply_edits_joined(vec![(5, 8, "p".to_owned())]);
        b.insert("re");
        b.apply_edits_joined(vec![(7, 8, "pre".to_owned())]);
        assert_eq!(b.rope().to_string(), "<pre></pre>");
        assert_eq!(b.primary().head, 4, "the cursor stays in the name typed");
        // Each undo step takes both names back together: they never disagree.
        assert!(b.undo());
        assert_eq!(b.rope().to_string(), "<p></p>");
        assert!(b.undo());
        assert_eq!(b.rope().to_string(), "<div></div>");
        assert!(b.redo());
        assert_eq!(b.rope().to_string(), "<p></p>");
    }

    #[test]
    fn an_edit_cutting_into_a_selection_leaves_a_cursor() {
        let mut b = Buffer::new("use a;\nfn main() {}\n");
        b.set_single(Selection::new(0, 3));
        // The whole file replaced, as a server's fix may do.
        let len = b.len_chars();
        b.apply_edits(vec![(0, len, "fn main() {}\n".to_owned())]);
        let s = b.primary();
        assert!(s.is_empty(), "{s:?}");
        assert_eq!(s.head, 0, "it stays by what changed");
        assert_eq!(b.rope().to_string(), "fn main() {}\n");
        // A cursor past what changed follows the text it was in.
        let mut b = Buffer::new("one\ntwo\nthree\n");
        b.set_single(Selection::cursor(9));
        let len = b.len_chars();
        b.apply_edits(vec![(0, len, "ONE\ntwo\nthree\n".to_owned())]);
        assert_eq!(b.primary().head, 9);
        // Nothing different is no edit at all.
        let version = b.version();
        let len = b.len_chars();
        b.apply_edits(vec![(0, len, b.rope().to_string())]);
        assert_eq!(b.version(), version);
        // One the edit doesn't touch stays selected, moved along.
        let mut b = Buffer::new("abc def");
        b.set_single(Selection::new(4, 7));
        b.apply_edits(vec![(0, 1, "xx".to_owned())]);
        assert_eq!((b.primary().start(), b.primary().end()), (5, 8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_comments_on_enter_after_slash_star_star() {
        let mut b = Buffer::new("#pragma once\n\n/**\nint vector3dDestroy(PtVector3d *pVec);\n");
        b.indent_rules = IndentRules::Brackets;
        b.doc_comments = true;
        // `/**` above the function, and Enter.
        b.set_single(Selection::cursor(b.line_start(2) + 3));
        b.insert_newline();
        let want = "#pragma once\n\n/**\n * @brief \n * \n * @param pVec \n * @return int \n */\nint vector3dDestroy(PtVector3d *pVec);\n";
        assert_eq!(b.text(), want);
        // The cursor waits after `@brief `.
        assert_eq!(b.primary().head, b.line_start(3) + " * @brief ".len());
        // Inside a comment already written, Enter only carries it on.
        let mut b = Buffer::new("/**\n * @brief Old\n */\nint f(void);\n");
        b.indent_rules = IndentRules::Brackets;
        b.doc_comments = true;
        b.set_single(Selection::cursor(3));
        b.insert_newline();
        assert_eq!(b.text(), "/**\n * \n * @brief Old\n */\nint f(void);\n");
        // Off: closed, nothing more.
        let mut b = Buffer::new("/**\nint f(int a);\n");
        b.indent_rules = IndentRules::Brackets;
        b.set_single(Selection::cursor(3));
        b.insert_newline();
        assert_eq!(b.text(), "/**\n * \n */\nint f(int a);\n");
    }

    #[test]
    fn enclosing_bracket_pairs() {
        let b = Buffer::new("f(a, [b], {\n  c(d)\n}) x");
        let pair = |pos: usize| b.enclosing_brackets(pos, 1000);
        // In `{ ... }`, past the inner `c(d)`.
        assert_eq!(pair(18), Some((10, 19)));
        // Inside `[b]`, and right after its opener.
        assert_eq!(pair(6), Some((5, 7)));
        // Inside `c(d)`.
        assert_eq!(pair(16), Some((15, 17)));
        // Between `[b]` and `{`: the call's parentheses.
        assert_eq!(pair(8), Some((1, 20)));
        // Outside everything.
        assert_eq!(pair(22), None);
        assert_eq!(pair(0), None);
    }

    #[test]
    fn types_brackets_and_quotes_in_pairs() {
        let typed = |start: &str, cursor: usize, keys: &[&str]| {
            let mut b = Buffer::new(start);
            b.set_single(Selection::cursor(cursor));
            for k in keys {
                b.type_text(k);
            }
            (b.text(), b.primary().head)
        };
        assert_eq!(typed("", 0, &["{"]), ("{}".to_owned(), 1));
        assert_eq!(typed("", 0, &["(", "[", "\""]), ("([\"\"])".to_owned(), 3));
        // A closer that's already next is stepped over.
        assert_eq!(typed("", 0, &["(", "x", ")"]), ("(x)".to_owned(), 3));
        assert_eq!(typed("", 0, &["\"", "\""]), ("\"\"".to_owned(), 2));
        // Nothing swallows the word after the cursor; no pair right after a word for quotes.
        assert_eq!(typed("word", 0, &["("]), ("(word".to_owned(), 1));
        assert_eq!(typed("it", 2, &["'"]), ("it'".to_owned(), 3));
        // Selections get surrounded.
        let mut b = Buffer::new("word");
        b.set_single(Selection::new(0, 4));
        b.type_text("[");
        assert_eq!(b.text(), "[word]");
        // Rust lifetimes stay alone.
        let mut b = Buffer::new("");
        b.pair_single_quote = false;
        b.type_text("'");
        assert_eq!(b.text(), "'");
        // Backspace removes an empty pair, but not a closer with text between.
        let mut b = Buffer::new("");
        b.type_text("(");
        b.delete_backward(false);
        assert_eq!(b.text(), "");
        // Off means plain typing.
        let mut b = Buffer::new("");
        b.auto_close = false;
        b.type_text("(");
        assert_eq!(b.text(), "(");
    }

    #[test]
    fn enter_and_tab_follow_the_indent_style() {
        let mut b = Buffer::new("{");
        b.indent = Indent { use_tabs: true, size: 4 };
        b.set_single(Selection::cursor(1));
        b.insert_newline();
        assert_eq!(
            b.text(),
            "{
	"
        );
        let mut b = Buffer::new("");
        b.indent = Indent { use_tabs: false, size: 2 };
        b.insert_tab();
        assert_eq!(b.text(), "  ");
    }

    #[test]
    fn typing_groups_into_one_undo_step_until_space() {
        let mut b = Buffer::new("");
        for c in "hello world".chars() {
            b.insert(&c.to_string());
        }
        assert_eq!(b.text(), "hello world");
        b.undo();
        assert_eq!(b.text(), "hello ");
        b.undo();
        assert_eq!(b.text(), "");
        b.redo();
        assert_eq!(b.text(), "hello ");
    }

    #[test]
    fn read_only_refuses_every_kind_of_edit() {
        let mut b = Buffer::new("x\ny\n");
        b.read_only = true;
        let version = b.version();
        b.insert("z");
        b.insert_newline();
        b.delete_backward(false);
        b.apply_edits(vec![(0, 1, "q".to_owned())]);
        b.replace_all("other", b.primary());
        assert!(!b.undo());
        assert_eq!((b.text().as_str(), b.version()), ("x\ny\n", version));
    }

    #[test]
    fn undo_restores_version_for_dirty_tracking() {
        let mut b = Buffer::new("x");
        let saved = b.version();
        b.insert("y");
        assert_ne!(b.version(), saved);
        b.undo();
        assert_eq!(b.version(), saved);
    }

    #[test]
    fn multi_cursor_insert_and_backspace() {
        let mut b = Buffer::new("ab\ncd\nef");
        b.set_selections(vec![Selection::cursor(1), Selection::cursor(4), Selection::cursor(7)], 0);
        b.insert("X");
        assert_eq!(b.text(), "aXb\ncXd\neXf");
        b.delete_backward(false);
        assert_eq!(b.text(), "ab\ncd\nef");
        assert_eq!(b.selections().len(), 3);
    }

    #[test]
    fn ctrl_d_adds_next_occurrences() {
        let mut b = Buffer::new("foo bar foo baz foo");
        b.set_single(Selection::cursor(1));
        b.add_next_occurrence();
        assert_eq!(b.selections(), &[Selection::new(0, 3)]);
        b.add_next_occurrence();
        b.add_next_occurrence();
        assert_eq!(b.selections().len(), 3);
        b.insert("x");
        assert_eq!(b.text(), "x bar x baz x");
    }

    #[test]
    fn wide_and_combining_characters_take_their_cells() {
        assert_eq!(char_width('a', 0), 1);
        assert_eq!(char_width('日', 0), 2);
        assert_eq!(char_width('\u{301}', 0), 0);
        assert_eq!(char_width('├', 0), 1);
        // Invisible characters keep the cell their mark is drawn in.
        assert_eq!(char_width('\u{200B}', 0), 1);
        assert_eq!(visual_width("日本 x".chars()), 6);
        let b = Buffer::new("日本語x");
        assert_eq!(b.visual_col(2), 4);
        // Inside a wide char, the nearer side.
        assert_eq!(b.pos_at_visual_col(0, 5), 3);
        assert_eq!(b.pos_at_visual_col(0, 4), 2);
    }

    #[test]
    fn newline_keeps_indent_and_splits_braces() {
        let mut b = Buffer::new("    fn a() {}");
        b.set_single(Selection::cursor(12));
        b.insert_newline();
        assert_eq!(b.text(), "    fn a() {\n        \n    }");
        assert_eq!(b.primary().head, 21);
    }

    /// A buffer of `text` with the cursor at `|`, in a language's rules, indented by 4 spaces.
    fn at(text: &str, rules: IndentRules) -> Buffer {
        // The last `|`, so that Ruby's `|item|` can be written too.
        let cursor = text.rfind('|').expect("a cursor");
        let mut b = Buffer::new(&format!("{}{}", &text[..cursor], &text[cursor + 1..]));
        b.indent_rules = rules;
        b.indent = Indent { use_tabs: false, size: 4 };
        b.set_single(Selection::cursor(text[..cursor].chars().count()));
        b
    }

    /// The text with `|` where the cursor is.
    fn shown(b: &Buffer) -> String {
        let mut text = b.text();
        let at = text.char_indices().nth(b.primary().head).map_or(text.len(), |(i, _)| i);
        text.insert(at, '|');
        text
    }

    #[test]
    fn enter_follows_each_languages_blocks() {
        let cases = [
            (IndentRules::Python, "def f(x):|", "def f(x):\n    |"),
            (IndentRules::Python, "    if x:  # yes|", "    if x:  # yes\n        |"),
            (IndentRules::Python, "    return x|", "    return x\n|"),
            (IndentRules::Python, "        pass|", "        pass\n    |"),
            (IndentRules::Markup, "<ul>|", "<ul>\n    |"),
            (IndentRules::Markup, "  <div class=\"a\">|</div>", "  <div class=\"a\">\n      |\n  </div>"),
            (IndentRules::Markup, "<br>|", "<br>\n|"),
            (IndentRules::Ruby, "  items.each do |item||", "  items.each do |item|\n      |"),
            (IndentRules::Lua, "if x then|", "if x then\n    |"),
            (IndentRules::Shell, "for f in *; do|", "for f in *; do\n    |"),
            (IndentRules::Yaml, "server:|", "server:\n    |"),
            (IndentRules::Brackets, "    call(|)", "    call(\n        |\n    )"),
            // A new `/**` comment is closed as it opens.
            (IndentRules::Brackets, "/**|", "/**\n * |\n */"),
            (IndentRules::Brackets, "/*\n    * text|", "/*\n    * text\n    * |"),
            (IndentRules::Brackets, "    *ptr = 1;|", "    *ptr = 1;\n    |"),
            (IndentRules::Brackets, "/* a */\n*p = 1;|", "/* a */\n*p = 1;\n|"),
            (IndentRules::Brackets, "let x = 1;|", "let x = 1;\n|"),
        ];
        for (rules, before, after) in cases {
            let mut b = at(before, rules);
            b.insert_newline();
            assert_eq!(shown(&b), after, "{rules:?} {before:?}");
        }
    }

    #[test]
    fn a_line_left_blank_keeps_no_spaces() {
        let mut b = at("{\n    |\n}", IndentRules::Brackets);
        b.insert_newline();
        assert_eq!(shown(&b), "{\n\n    |\n}");
        // Enter inside the indentation doesn't double it.
        let mut b = at("  |  x", IndentRules::Brackets);
        b.insert_newline();
        assert_eq!(shown(&b), "  \n  |  x");
    }

    #[test]
    fn tabs_files_indent_with_tabs() {
        let mut b = at("\tif (x) {|}", IndentRules::Brackets);
        b.indent = Indent { use_tabs: true, size: 4 };
        b.insert_newline();
        assert_eq!(shown(&b), "\tif (x) {\n\t\t|\n\t}");
    }

    #[test]
    fn typing_a_closer_or_end_word_moves_its_line_back() {
        let typed = |text: &str, rules, typing: &str| {
            let mut b = at(text, rules);
            b.auto_close = false;
            for c in typing.chars() {
                b.type_text(&c.to_string());
            }
            shown(&b)
        };
        assert_eq!(typed("fn a() {\n    x();\n    |", IndentRules::Brackets, "}"), "fn a() {\n    x();\n}|");
        assert_eq!(typed("  call(\n      a,\n      |", IndentRules::Brackets, ")"), "  call(\n      a,\n  )|");
        assert_eq!(typed("if x:\n    a()\n    |", IndentRules::Python, "else:"), "if x:\n    a()\nelse:|");
        assert_eq!(typed("if x:\n    a()\n|", IndentRules::Python, "else:"), "if x:\n    a()\nelse:|", "already out");
        assert_eq!(typed("def a\n  b\n  |", IndentRules::Ruby, "end "), "def a\n  b\nend |");
        assert_eq!(typed("if [ x ]; then\n    y\n    |", IndentRules::Shell, "fi;"), "if [ x ]; then\n    y\nfi;|");
        assert_eq!(typed("<ul>\n    <li>a</li>\n    |", IndentRules::Markup, "</"), "<ul>\n    <li>a</li>\n</|");
        // Words that only start like one, and closers after text, stay.
        assert_eq!(typed("def a\n  |", IndentRules::Ruby, "ending"), "def a\n  ending|");
        assert_eq!(typed("    x|", IndentRules::Brackets, "}"), "    x}|");
        // Never deeper than it was.
        assert_eq!(typed("{\n}\n|", IndentRules::Brackets, "}"), "{\n}\n}|");
    }

    #[test]
    fn enter_finishes_an_end_word() {
        let mut b = at("def a\n    b\n    end|", IndentRules::Ruby);
        b.insert_newline();
        assert_eq!(shown(&b), "def a\n    b\nend\n|");
        let mut b = at("if x\n    y\n    else|", IndentRules::Ruby);
        b.insert_newline();
        assert_eq!(shown(&b), "if x\n    y\nelse\n    |");
    }

    #[test]
    fn typing_a_tag_closes_it_and_quotes_its_values() {
        let mut b = at("|", IndentRules::Markup);
        b.auto_tags = TagMode::Html;
        b.auto_quotes = true;
        // `=` adds the quotes; the closing one is stepped over.
        for c in "<div class=a\">".chars() {
            b.type_text(&c.to_string());
        }
        assert_eq!(shown(&b), "<div class=\"a\">|</div>");
        b.insert_newline();
        assert_eq!(shown(&b), "<div class=\"a\">\n    |\n</div>");
        // `</` finishes the open element, moving its line back.
        let mut b = at("<ul>\n    <li>a</li>\n    |", IndentRules::Markup);
        b.auto_tags = TagMode::Html;
        for c in "</".chars() {
            b.type_text(&c.to_string());
        }
        assert_eq!(shown(&b), "<ul>\n    <li>a</li>\n</ul>|");
    }

    #[test]
    fn backspace_in_indentation_goes_to_the_tab_stop() {
        let mut b = at("        |x", IndentRules::Brackets);
        b.delete_backward(false);
        assert_eq!(shown(&b), "    |x");
        let mut b = at("      |x", IndentRules::Brackets);
        b.delete_backward(false);
        assert_eq!(shown(&b), "    |x");
        // After text, one character at a time.
        let mut b = at("a   |", IndentRules::Brackets);
        b.delete_backward(false);
        assert_eq!(shown(&b), "a  |");
        assert_eq!(Indent { use_tabs: false, size: 4 }.outdented("      "), "    ");
        assert_eq!(Indent { use_tabs: true, size: 4 }.outdented("\t\t"), "\t");
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut b = Buffer::new("abcdef\nab\nabcdef");
        b.set_single(Selection::cursor(5));
        b.move_vertical(1, false);
        assert_eq!(b.primary().head, 9);
        b.move_vertical(1, false);
        assert_eq!(b.primary().head, 15);
    }

    #[test]
    fn crlf_line_end_and_backspace() {
        let mut b = Buffer::new("ab\r\ncd");
        assert_eq!(b.line_end(0), 2);
        assert_eq!(b.line_ending, "\r\n");
        b.set_single(Selection::cursor(4));
        b.delete_backward(false);
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn word_movement() {
        let b = Buffer::new("let foo_bar = 1;");
        assert_eq!(b.next_word_boundary(0), 3);
        assert_eq!(b.next_word_boundary(3), 11);
        assert_eq!(b.prev_word_boundary(11), 4);
    }

    #[test]
    fn recorded_syntax_edits_match_a_fresh_parse() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let mut b = Buffer::new("fn a() {\n    let x = 1;\n}\n");
        b.track_syntax_edits();
        b.take_syntax_edits();
        let mut tree = parser.parse(b.text(), None).unwrap();

        b.set_selections(vec![Selection::cursor(13), Selection::cursor(24)], 0);
        b.insert("é");
        b.delete_backward(false);
        b.insert("yy");
        for e in b.take_syntax_edits() {
            match e {
                SyntaxEdit::Edit(e) => tree.edit(&e),
                SyntaxEdit::Reset => panic!("unexpected reset"),
            }
        }
        let text = b.text();
        let incremental = parser.parse(&text, Some(&tree)).unwrap();
        let fresh = parser.parse(&text, None).unwrap();
        assert_eq!(incremental.root_node().to_sexp(), fresh.root_node().to_sexp());
    }

    #[test]
    fn line_ops_edit_only_the_changed_lines() {
        let text: String = (0..200)
            .map(|i| {
                format!(
                    "let x{i} = {i};
"
                )
            })
            .collect();
        let mut b = Buffer::new(&text);
        b.track_syntax_edits();
        b.take_syntax_edits();
        let pos = b.line_start(100) + 2;
        b.set_single(Selection::cursor(pos));
        assert!(b.apply_line_op(false, |t, s| Some(crate::edit_ops::toggle_line_comment(t, s, "//"))));
        assert_eq!(b.line_text(100), "// let x100 = 100;");
        assert_eq!(b.line_text(99), "let x99 = 99;");
        assert_eq!(b.primary().head, pos + 3, "cursor stays on the same character");
        let edits = b.take_syntax_edits();
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            SyntaxEdit::Edit(e) => assert_eq!(e.new_end_byte - e.start_byte, 3, "only `// ` was inserted"),
            SyntaxEdit::Reset => panic!("line commands must not force a full reparse"),
        }
        b.undo();
        assert_eq!(b.text(), text);
    }

    #[test]
    fn line_ops_at_document_edges_and_whole_document() {
        let mut b = Buffer::new(
            "a
b
c",
        );
        b.set_single(Selection::cursor(4));
        assert!(b.apply_line_op(false, |t, s| Some(crate::edit_ops::delete_lines(t, s))));
        assert_eq!(
            b.text(),
            "a
b"
        );
        b.set_single(Selection::cursor(0));
        assert!(!b.apply_line_op(false, |t, s| crate::edit_ops::move_lines(t, s, false)), "nothing above line 1");
        assert!(b.apply_line_op(false, |t, s| crate::edit_ops::move_lines(t, s, true)));
        assert_eq!(
            b.text(),
            "b
a"
        );
        let mut b = Buffer::new(
            "c
a
b",
        );
        assert!(b.apply_line_op(true, |t, s| Some(crate::edit_ops::sort_lines(t, s, false))));
        assert_eq!(
            b.text(),
            "a
b
c"
        );
    }

    #[test]
    fn delete_line_is_its_own_undo_step() {
        let mut b = Buffer::new(
            "one
two
three",
        );
        b.set_single(Selection::cursor(5));
        b.apply_line_op(false, |t, s| Some(crate::edit_ops::delete_lines(t, s)));
        b.delete_backward(false);
        b.undo();
        assert_eq!(
            b.text(),
            "one
three",
            "backspace undone on its own"
        );
    }

    #[test]
    fn line_ending_conversion_keeps_cursor_and_is_undoable() {
        let mut b = Buffer::new("one\ntwo\r\nthree");
        b.set_single(Selection::cursor(b.line_start(2) + 2));
        b.convert_line_endings("\r\n");
        assert_eq!(b.text(), "one\r\ntwo\r\nthree");
        assert_eq!(b.line_col(b.primary().head), (3, 3));
        b.insert("\n");
        b.undo();
        b.convert_line_endings("\n");
        assert_eq!(b.text(), "one\ntwo\nthree");
        assert_eq!(b.line_ending, "\n");
        b.undo();
        assert_eq!(b.text(), "one\r\ntwo\r\nthree");
    }

    #[test]
    fn bracket_matching_both_directions() {
        let b = Buffer::new("f(a[1], {b})");
        assert_eq!(b.matching_bracket(1, 1000), Some((1, 11)));
        assert_eq!(b.matching_bracket(12, 1000), Some((1, 11)));
        assert_eq!(b.matching_bracket(3, 1000), Some((3, 5)));
        assert_eq!(b.matching_bracket(0, 1000), None);
    }

    #[test]
    fn tabs_map_to_visual_columns() {
        let b = Buffer::new("\tx");
        assert_eq!(b.visual_col(1), 4);
        assert_eq!(b.pos_at_visual_col(0, 4), 1);
        assert_eq!(b.pos_at_visual_col(0, 1), 0);
    }

    fn selected(b: &Buffer) -> Vec<String> {
        b.selections().iter().map(|s| b.rope().slice(s.start()..s.end()).to_string()).collect()
    }

    #[test]
    fn selects_all_occurrences() {
        let mut b = Buffer::new("foo food foo\nfoo");
        // From the cursor: whole words only, and the word stays primary.
        b.set_single(Selection::cursor(10));
        b.select_all_occurrences();
        assert_eq!(selected(&b), ["foo", "foo", "foo"]);
        assert_eq!(b.primary().start(), 9);
        // From a selection: any match, without overlaps.
        b.set_single(Selection::new(0, 3));
        b.select_all_occurrences();
        assert_eq!(selected(&b).len(), 4);
        let mut b = Buffer::new("aaaa");
        b.set_single(Selection::new(0, 2));
        b.select_all_occurrences();
        assert_eq!(selected(&b), ["aa", "aa"]);
        // Nothing to take: nothing changes.
        let mut b = Buffer::new("  ");
        b.set_single(Selection::cursor(1));
        b.select_all_occurrences();
        assert_eq!(b.selections(), [Selection::cursor(1)]);
    }

    #[test]
    fn selects_a_column_block() {
        let mut b = Buffer::new("abcdef\nab\n\nabcdef");
        b.select_column((0, 1), (3, 4));
        // The empty line can't reach the block; the short one gives what it has.
        assert_eq!(selected(&b), ["bcd", "b", "bcd"]);
        assert_eq!(b.primary().head, 11 + 4);
        // Dragging up and left keeps the head at the pointer.
        b.select_column((3, 4), (0, 1));
        assert_eq!(selected(&b), ["bcd", "b", "bcd"]);
        assert_eq!(b.primary().head, 1);
        // Zero width: a cursor per line.
        b.select_column((0, 2), (1, 2));
        assert_eq!(b.selections().iter().map(|s| s.head).collect::<Vec<_>>(), [2, 9]);
        // Past the end of the text.
        b.select_column((3, 0), (99, 2));
        assert_eq!(selected(&b), ["ab"]);
    }

    #[test]
    fn apply_edits_is_one_undo_step_and_moves_cursors() {
        let mut b = Buffer::new(
            "let x=1;
let y=2;
",
        );
        b.set_single(Selection::cursor(13)); // before the "y" on line 2
        b.apply_edits(vec![
            (5, 6, " = ".into()),
            (14, 15, " = ".into()),
            (
                0,
                0,
                "// fmt
"
                .into(),
            ),
        ]);
        assert_eq!(
            b.text(),
            "// fmt
let x = 1;
let y = 2;
"
        );
        assert_eq!(b.primary().head, 13 + 7 + 2);
        assert!(b.undo());
        assert_eq!(
            b.text(),
            "let x=1;
let y=2;
"
        );
    }
}
