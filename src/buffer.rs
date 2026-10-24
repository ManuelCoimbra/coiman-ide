//! Rope-backed text buffer with multiple selections and undo/redo.
//!
//! Positions are char indices into the rope. Lines are split on `\n` only
//! (ropey's `unicode_lines`/`cr_lines` features are off), so a CRLF line keeps
//! its `\r` as the last visible-but-skipped char; see [`Buffer::line_end`].

use ropey::Rope;
use tree_sitter::{InputEdit, Point};

pub const TAB_WIDTH: usize = 4;
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

pub struct Buffer {
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
    pub fn from_str(text: &str) -> Self {
        let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Self {
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

    /// Visual column of `pos`, expanding tabs to [`TAB_WIDTH`].
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

    fn find_chars(&self, needle: &[char], from: usize, to: usize) -> Option<usize> {
        if needle.is_empty() || to < from + needle.len() {
            return None;
        }
        // Rope chars iterator is O(1) amortized per step; fine for interactive use.
        let hay: Vec<char> = self.rope.slice(from..to).chars().collect();
        hay.windows(needle.len()).position(|w| w == needle).map(|i| from + i)
    }

    /// Bracket next to `pos` and its partner, searching at most `limit` chars away.
    pub fn matching_bracket(&self, pos: usize, limit: usize) -> Option<(usize, usize)> {
        const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];
        let len = self.len_chars();
        for at in [pos, pos.wrapping_sub(1)] {
            if at >= len {
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
        Snapshot { rope: self.rope.clone(), selections: self.selections.clone(), primary: self.primary, version: self.version }
    }

    /// Records an undo step unless this edit continues the previous one.
    fn begin_edit(&mut self, kind: EditKind) {
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
        self.version = self.next_version;
        self.next_version += 1;
        self.edited_from_line = Some(self.edited_from_line.map_or(first_line, |l| l.min(first_line)));
    }

    /// Applies non-overlapping changes (one per selection) and moves each
    /// selection to its change's cursor position.
    pub fn apply_changes(&mut self, mut changes: Vec<Change>, kind_typing: bool) {
        if changes.is_empty() {
            return;
        }
        let kind = if kind_typing {
            EditKind::Typing
        } else if changes.iter().all(|c| c.text.is_empty()) {
            EditKind::Deleting
        } else {
            EditKind::Other
        };
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
            let tracking = self.syntax_edits.is_some();
            let before = tracking.then(|| {
                let (sb, eb) = (self.rope.char_to_byte(start), self.rope.char_to_byte(end));
                (sb, eb, self.point_at_byte(sb), self.point_at_byte(eb))
            });
            if end > start {
                self.rope.remove(start..end);
            }
            let inserted = c.text.chars().count();
            if inserted > 0 {
                self.rope.insert(start, &c.text);
            }
            if let Some((start_byte, old_end_byte, start_position, old_end_position)) = before {
                let new_end_byte = start_byte + c.text.len();
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

    /// Enter: new line keeping the current indentation, indenting after an
    /// opening bracket and splitting `{|}` onto three lines.
    pub fn insert_newline(&mut self) {
        let nl = self.line_ending;
        let changes = self
            .selections
            .iter()
            .map(|s| {
                let line = self.char_to_line(s.start());
                let indent: String = self.line_text(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let before = (s.start() > 0).then(|| self.rope.char(s.start() - 1));
                let after = (s.end() < self.len_chars()).then(|| self.rope.char(s.end()));
                let opens = matches!(before, Some('{' | '(' | '['));
                let closes = matches!((before, after), (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']')));
                let inner = if opens { format!("{indent}{}", " ".repeat(TAB_WIDTH)) } else { indent.clone() };
                let mut text = format!("{nl}{inner}");
                let cursor = text.chars().count();
                if closes {
                    text.push_str(nl);
                    text.push_str(&indent);
                }
                Change { start: s.start(), end: s.end(), text, cursor_offset: Some(cursor) }
            })
            .collect();
        self.apply_changes(changes, false);
    }

    /// Tab: spaces up to the next tab stop at each cursor.
    pub fn insert_tab(&mut self) {
        let changes = self
            .selections
            .iter()
            .map(|s| {
                let col = self.visual_col(s.start());
                let n = TAB_WIDTH - col % TAB_WIDTH;
                Change { start: s.start(), end: s.end(), text: " ".repeat(n), cursor_offset: None }
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

    /// Replaces the whole text (used by line commands) and sets one selection.
    pub fn replace_all(&mut self, text: &str, sel: Selection) {
        let old = self.text();
        if old == text {
            self.set_single(sel);
            return;
        }
        self.begin_edit(EditKind::Other);
        let common = old.chars().zip(text.chars()).take_while(|(a, b)| a == b).count();
        let first_line = self.char_to_line(common);
        self.rope = Rope::from_str(text);
        self.record_reset();
        self.set_single(sel);
        self.last_edit = EditKind::Other;
        self.finish_edit(first_line);
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
    if c == '\t' { TAB_WIDTH - col % TAB_WIDTH } else { 1 }
}

pub fn visual_width(chars: impl Iterator<Item = char>) -> usize {
    chars.fold(0, |col, c| col + char_width(c, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_groups_into_one_undo_step_until_space() {
        let mut b = Buffer::from_str("");
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
    fn undo_restores_version_for_dirty_tracking() {
        let mut b = Buffer::from_str("x");
        let saved = b.version();
        b.insert("y");
        assert_ne!(b.version(), saved);
        b.undo();
        assert_eq!(b.version(), saved);
    }

    #[test]
    fn multi_cursor_insert_and_backspace() {
        let mut b = Buffer::from_str("ab\ncd\nef");
        b.set_selections(vec![Selection::cursor(1), Selection::cursor(4), Selection::cursor(7)], 0);
        b.insert("X");
        assert_eq!(b.text(), "aXb\ncXd\neXf");
        b.delete_backward(false);
        assert_eq!(b.text(), "ab\ncd\nef");
        assert_eq!(b.selections().len(), 3);
    }

    #[test]
    fn ctrl_d_adds_next_occurrences() {
        let mut b = Buffer::from_str("foo bar foo baz foo");
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
    fn newline_keeps_indent_and_splits_braces() {
        let mut b = Buffer::from_str("    fn a() {}");
        b.set_single(Selection::cursor(12));
        b.insert_newline();
        assert_eq!(b.text(), "    fn a() {\n        \n    }");
        assert_eq!(b.primary().head, 21);
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut b = Buffer::from_str("abcdef\nab\nabcdef");
        b.set_single(Selection::cursor(5));
        b.move_vertical(1, false);
        assert_eq!(b.primary().head, 9);
        b.move_vertical(1, false);
        assert_eq!(b.primary().head, 15);
    }

    #[test]
    fn crlf_line_end_and_backspace() {
        let mut b = Buffer::from_str("ab\r\ncd");
        assert_eq!(b.line_end(0), 2);
        assert_eq!(b.line_ending, "\r\n");
        b.set_single(Selection::cursor(4));
        b.delete_backward(false);
        assert_eq!(b.text(), "abcd");
    }

    #[test]
    fn word_movement() {
        let b = Buffer::from_str("let foo_bar = 1;");
        assert_eq!(b.next_word_boundary(0), 3);
        assert_eq!(b.next_word_boundary(3), 11);
        assert_eq!(b.prev_word_boundary(11), 4);
    }

    #[test]
    fn recorded_syntax_edits_match_a_fresh_parse() {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let mut b = Buffer::from_str("fn a() {\n    let x = 1;\n}\n");
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
    fn bracket_matching_both_directions() {
        let b = Buffer::from_str("f(a[1], {b})");
        assert_eq!(b.matching_bracket(1, 1000), Some((1, 11)));
        assert_eq!(b.matching_bracket(12, 1000), Some((1, 11)));
        assert_eq!(b.matching_bracket(3, 1000), Some((3, 5)));
        assert_eq!(b.matching_bracket(0, 1000), None);
    }

    #[test]
    fn tabs_map_to_visual_columns() {
        let b = Buffer::from_str("\tx");
        assert_eq!(b.visual_col(1), 4);
        assert_eq!(b.pos_at_visual_col(0, 4), 1);
        assert_eq!(b.pos_at_visual_col(0, 1), 0);
    }
}
