//! Modal editing: a native Vim mode (normal, insert, visual, command line,
//! counts, operators with motions and text objects, registers, `.` repeat,
//! search), and a Helix-style selection-first flavor. Works directly on a
//! [`Buffer`]; the editor feeds it keys. Positions are chars; Vim's cursor
//! and visual selections are inclusive, so a cursor is a one-char block.

use std::collections::HashMap;

use crate::buffer::{Buffer, Selection};

/// A key press, in Vim's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Esc,
    Enter,
    Backspace,
    Tab,
    Left,
    Right,
    Up,
    Down,
}

/// Parses Vim key notation: `dw`, `ihi<Esc>`, `<C-r>`, `<CR>`, `<BS>`, `<lt>`.
pub fn parse_keys(s: &str) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(end) = rest.find('>')
        {
            let name = &rest[1..end];
            let key = match name.to_ascii_lowercase().as_str() {
                "esc" => Some(Key::Esc),
                "cr" | "enter" => Some(Key::Enter),
                "bs" => Some(Key::Backspace),
                "tab" => Some(Key::Tab),
                "left" => Some(Key::Left),
                "right" => Some(Key::Right),
                "up" => Some(Key::Up),
                "down" => Some(Key::Down),
                "lt" => Some(Key::Char('<')),
                n if n.starts_with("c-") && n.len() == 3 => n.chars().nth(2).map(Key::Ctrl),
                _ => None,
            };
            if let Some(key) = key {
                keys.push(key);
                rest = &rest[end + 1..];
                continue;
            }
        }
        keys.push(Key::Char(c));
        rest = &rest[c.len_utf8()..];
    }
    keys
}

/// Keys as a Vim `:exe "normal! …"` string body.
pub fn keys_to_vim(keys: &[Key]) -> String {
    keys.iter()
        .map(|k| match k {
            Key::Char('"') => "\\\"".to_owned(),
            Key::Char('\\') => "\\\\".to_owned(),
            Key::Char('<') => "\\<lt>".to_owned(),
            Key::Char(c) => c.to_string(),
            Key::Ctrl(c) => format!("\\<C-{c}>"),
            Key::Esc => "\\<Esc>".to_owned(),
            Key::Enter => "\\<CR>".to_owned(),
            Key::Backspace => "\\<BS>".to_owned(),
            Key::Tab => "\\<Tab>".to_owned(),
            Key::Left => "\\<Left>".to_owned(),
            Key::Right => "\\<Right>".to_owned(),
            Key::Up => "\\<Up>".to_owned(),
            Key::Down => "\\<Down>".to_owned(),
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Vim,
    /// Select first, then act (Helix/Kakoune).
    Helix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    /// Helix: motions extend the selection.
    Select,
    /// Typing a `:` command or a `/` `?` search.
    Command,
}

/// Something the editor around the engine must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Save,
    Quit {
        force: bool,
    },
    SaveQuit,
    /// Copy to the system clipboard (the `+` and `*` registers).
    Clipboard(String),
}

#[derive(Clone, Default, Debug)]
struct Register {
    text: String,
    linewise: bool,
}

/// How a motion's range is taken for an operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Exclusive,
    Inclusive,
    Linewise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordStart(bool),
    WordBack(bool),
    WordEnd(bool),
    LineStart,
    FirstNonBlank,
    LineEnd,
    FileStart,
    FileEnd,
    Find(char, char),
    RepeatFind(bool),
    MatchBracket,
    ParagraphForward,
    ParagraphBack,
    SearchNext(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    ToggleCase,
    Lower,
    Upper,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Motion(Motion, Option<usize>),
    /// The operator doubled (`dd`, `cc`, `>>`): whole lines.
    Lines,
    /// `iw`, `a(`, `i"`…
    Object(bool, char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Move(Motion),
    Operate(Op, Target),
    /// A single-key command (`x`, `p`, `i`, `u`, `.`…).
    Simple(char),
    /// `r{c}`
    Replace(char),
    Ctrl(char),
}

#[derive(Clone, Copy, Debug)]
struct Cmd {
    register: Option<char>,
    count: Option<usize>,
    action: Action,
}

enum Parsed {
    More,
    Invalid,
    Done(Cmd),
}

// ----- text helpers ------------------------------------------------------------------

/// Vim character classes: 0 blank, 1 word, 2 punctuation (WORD: 1 for any non-blank).
fn class(c: char, big: bool) -> u8 {
    if c.is_whitespace() {
        0
    } else if big || c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

fn ch(b: &Buffer, p: usize) -> char {
    if p < b.len_chars() { b.rope().char(p) } else { '\n' }
}

fn line_of(b: &Buffer, p: usize) -> usize {
    b.char_to_line(p.min(b.len_chars()))
}

fn last_line(b: &Buffer) -> usize {
    // A final newline doesn't start a real line.
    let n = b.len_lines();
    if n > 1 && b.line_start(n - 1) == b.len_chars() { n - 2 } else { n - 1 }
}

/// The last char of a line, where Vim's normal-mode cursor may go.
fn last_col(b: &Buffer, line: usize) -> usize {
    let (s, e) = (b.line_start(line), b.line_end(line));
    if e > s { e - 1 } else { s }
}

fn clamp_normal(b: &Buffer, p: usize) -> usize {
    let line = line_of(b, p).min(last_line(b));
    p.clamp(b.line_start(line), last_col(b, line))
}

fn first_non_blank(b: &Buffer, line: usize) -> usize {
    let (s, e) = (b.line_start(line), b.line_end(line));
    (s..e).find(|&p| !matches!(ch(b, p), ' ' | '\t')).unwrap_or(if e > s { e - 1 } else { s })
}

/// Start of the line after `line` (or the end of the text).
fn next_line_start(b: &Buffer, line: usize) -> usize {
    if line + 1 < b.len_lines() { b.line_start(line + 1) } else { b.len_chars() }
}

fn is_empty_line(b: &Buffer, line: usize) -> bool {
    b.line_start(line) == b.line_end(line)
}

fn word_forward(b: &Buffer, mut p: usize, big: bool) -> usize {
    let len = b.len_chars();
    if p >= len {
        return len;
    }
    let start_line = line_of(b, p);
    let c = class(ch(b, p), big);
    if c != 0 {
        while p < len && class(ch(b, p), big) == c && ch(b, p) != '\n' {
            p += 1;
        }
    }
    // Skip blanks; an empty line counts as a word.
    while p < len && class(ch(b, p), big) == 0 {
        if ch(b, p) == '\n' {
            let line = line_of(b, p) + 1;
            if line > start_line && line <= last_line(b) && is_empty_line(b, line) {
                return b.line_start(line);
            }
        }
        p += 1;
    }
    p
}

fn word_back(b: &Buffer, mut p: usize, big: bool) -> usize {
    if p == 0 {
        return 0;
    }
    p -= 1;
    while p > 0 && class(ch(b, p), big) == 0 {
        if ch(b, p) == '\n' && is_empty_line(b, line_of(b, p)) && b.line_start(line_of(b, p)) == p {
            return p;
        }
        p -= 1;
    }
    let c = class(ch(b, p), big);
    while p > 0 && class(ch(b, p - 1), big) == c && c != 0 {
        p -= 1;
    }
    p
}

fn word_end(b: &Buffer, mut p: usize, big: bool) -> usize {
    let len = b.len_chars();
    if p + 1 >= len {
        return p;
    }
    p += 1;
    while p + 1 < len && class(ch(b, p), big) == 0 {
        p += 1;
    }
    let c = class(ch(b, p), big);
    while p + 1 < len && class(ch(b, p + 1), big) == c && ch(b, p + 1) != '\n' {
        p += 1;
    }
    p
}

// ----- the engine --------------------------------------------------------------------

pub struct Vim {
    pub flavor: Flavor,
    mode: Mode,
    /// Normal-mode keys of the command being typed.
    pending: Vec<Key>,
    registers: HashMap<char, Register>,
    /// Keys of the last change, for `.`.
    last_change: Vec<Key>,
    /// Keys of the change in progress (until it leaves insert mode).
    recording: Option<Vec<Key>>,
    last_find: Option<(char, char)>,
    last_search: Option<(String, bool)>,
    /// The command line, with its `:` `/` or `?`.
    cmdline: String,
    /// Selection ends (inclusive); `head` is the cursor.
    anchor: usize,
    head: usize,
    /// Wanted visual column for up/down (`usize::MAX`: end of line).
    goal: Option<usize>,
    /// The selection last put in the buffer, to notice moves made elsewhere.
    expected: Option<Selection>,
    /// Buffer undo depths before/after each change, so `u` undoes a whole change.
    undo_groups: Vec<(usize, usize)>,
    redo_groups: Vec<(usize, usize)>,
    change_depth: Option<usize>,
    /// Count of the insert in progress and the keys typed in it (`3ihi<Esc>`).
    insert_count: usize,
    insert_keys: Vec<Key>,
    replaying: bool,
    /// Auto-indent new lines (off for tests matching `vim -u NONE`).
    pub autoindent: bool,
    effects: Vec<Effect>,
    message: Option<String>,
    /// The code brackets of one line and their partners, worked out by the editor, which
    /// knows strings and comments: `%` uses them on that line.
    bracket_hint: Option<BracketHint>,
}

/// A line, and each code bracket on it with its partner (`None`: unmatched).
pub type BracketHint = (usize, Vec<(usize, Option<(usize, usize)>)>);

impl Vim {
    /// Brackets for `%` on one line, from what the editor knows is code. Cleared by
    /// [`Vim::clear_bracket_hint`]; other lines search the text.
    pub fn set_bracket_hint(&mut self, hint: BracketHint) {
        self.bracket_hint = Some(hint);
    }

    pub fn clear_bracket_hint(&mut self) {
        self.bracket_hint = None;
    }

    pub fn new(flavor: Flavor) -> Self {
        Self {
            flavor,
            mode: Mode::Normal,
            pending: Vec::new(),
            registers: HashMap::new(),
            last_change: Vec::new(),
            recording: None,
            last_find: None,
            last_search: None,
            cmdline: String::new(),
            anchor: 0,
            head: 0,
            goal: None,
            expected: None,
            undo_groups: Vec::new(),
            redo_groups: Vec::new(),
            change_depth: None,
            insert_count: 1,
            insert_keys: Vec::new(),
            replaying: false,
            autoindent: true,
            effects: Vec::new(),
            message: None,
            bracket_hint: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether the cursor is drawn as a block (everything but insert mode).
    pub fn block_cursor(&self) -> bool {
        self.mode != Mode::Insert
    }

    /// Status bar text: the mode, a pending command, or the command line.
    pub fn status(&self) -> String {
        if self.mode == Mode::Command {
            return self.cmdline.clone();
        }
        if let Some(m) = &self.message {
            return m.clone();
        }
        let mode = match (self.flavor, self.mode) {
            (_, Mode::Insert) => "-- INSERT --",
            (_, Mode::Visual) => "-- VISUAL --",
            (_, Mode::VisualLine) => "-- VISUAL LINE --",
            (_, Mode::Select) => "-- SELECT --",
            (Flavor::Helix, _) => "NORMAL (helix)",
            _ => "NORMAL",
        };
        let pending: String = self
            .pending
            .iter()
            .map(|k| match k {
                Key::Char(c) => c.to_string(),
                Key::Ctrl(c) => format!("^{}", c.to_ascii_uppercase()),
                _ => String::new(),
            })
            .collect();
        if pending.is_empty() { mode.to_owned() } else { format!("{mode}  {pending}") }
    }

    /// Back to normal mode with a cursor at the buffer's (e.g. on file switch).
    pub fn reset(&mut self, buf: &mut Buffer) {
        self.mode = Mode::Normal;
        self.pending.clear();
        self.expected = None;
        self.sync(buf);
        self.apply(buf);
    }

    /// Insert-mode keys typed by the editor itself: remembered for `.` and counts.
    pub fn observe(&mut self, key: Key) {
        if self.mode == Mode::Insert {
            self.insert_keys.push(key);
            if let Some(r) = &mut self.recording {
                r.push(key);
            }
        }
    }

    /// Handles one key; returns what the editor must do.
    pub fn handle(&mut self, buf: &mut Buffer, key: Key) -> Vec<Effect> {
        self.message = None;
        self.sync(buf);
        match self.mode {
            Mode::Insert => self.insert_key(buf, key),
            Mode::Command => self.cmdline_key(buf, key),
            _ => {
                self.pending.push(key);
                match self.flavor {
                    Flavor::Vim => self.run_vim(buf),
                    Flavor::Helix => self.run_helix(buf),
                }
            }
        }
        self.apply(buf);
        std::mem::take(&mut self.effects)
    }

    /// Adopts a cursor/selection the editor changed (mouse, undo, go to line).
    fn sync(&mut self, buf: &Buffer) {
        let sel = buf.primary();
        if self.expected == Some(sel) || self.mode == Mode::Insert {
            return;
        }
        if self.flavor == Flavor::Helix && !sel.is_empty() {
            self.anchor = sel.anchor.min(buf.len_chars().saturating_sub(1));
            self.head = if sel.head > sel.anchor { sel.head - 1 } else { sel.head };
        } else {
            self.head = clamp_normal(buf, sel.head);
            self.anchor = self.head;
            if matches!(self.mode, Mode::Visual | Mode::VisualLine | Mode::Select) {
                self.mode = Mode::Normal;
            }
        }
        self.goal = None;
    }

    /// Puts the engine's cursor/selection into the buffer.
    fn apply(&mut self, buf: &mut Buffer) {
        let len = buf.len_chars();
        let sel = match self.mode {
            Mode::Insert => buf.primary(),
            Mode::Normal | Mode::Command if self.flavor == Flavor::Vim => {
                self.head = clamp_normal(buf, self.head);
                Selection::cursor(self.head)
            }
            Mode::VisualLine => {
                let (a, h) = (line_of(buf, self.anchor), line_of(buf, self.head));
                if h >= a {
                    Selection::new(buf.line_start(a), buf.line_end(h))
                } else {
                    Selection::new(buf.line_end(a), buf.line_start(h))
                }
            }
            _ => {
                self.anchor = self.anchor.min(len);
                self.head = self.head.min(len);
                if self.head >= self.anchor {
                    Selection::new(self.anchor, (self.head + 1).min(len))
                } else {
                    Selection::new((self.anchor + 1).min(len), self.head)
                }
            }
        };
        if buf.primary() != sel || buf.selections().len() > 1 {
            buf.set_single(sel);
        }
        self.expected = Some(buf.primary());
    }

    // ----- vim: parsing ---------------------------------------------------------------

    fn parse(keys: &[Key], visual: bool) -> Parsed {
        let mut i = 0;
        let next = |i: &mut usize| -> Option<Key> {
            let k = keys.get(*i).copied();
            *i += 1;
            k
        };
        let mut register = None;
        if keys.first() == Some(&Key::Char('"')) {
            i = 1;
            match next(&mut i) {
                Some(Key::Char(c)) => register = Some(c),
                Some(_) => return Parsed::Invalid,
                None => return Parsed::More,
            }
        }
        let read_count = |i: &mut usize| -> Option<usize> {
            let mut n: Option<usize> = None;
            while let Some(Key::Char(c)) = keys.get(*i) {
                match c.to_digit(10) {
                    Some(d) if d > 0 || n.is_some() => {
                        n = Some(n.unwrap_or(0) * 10 + d as usize);
                        *i += 1;
                    }
                    _ => break,
                }
            }
            n
        };
        let count = read_count(&mut i);
        let Some(key) = next(&mut i) else { return Parsed::More };
        let done = |action| Parsed::Done(Cmd { register, count, action });

        // Motion keys (also targets of operators).
        fn motion(key: Key, keys: &[Key], i: &mut usize) -> Result<Option<Motion>, bool> {
            let mut next = || {
                let k = keys.get(*i).copied();
                *i += 1;
                k
            };
            Ok(Some(match key {
                Key::Char('h') | Key::Left | Key::Backspace => Motion::Left,
                Key::Char('l') | Key::Right | Key::Char(' ') => Motion::Right,
                Key::Char('j') | Key::Down => Motion::Down,
                Key::Char('k') | Key::Up => Motion::Up,
                Key::Char('w') => Motion::WordStart(false),
                Key::Char('W') => Motion::WordStart(true),
                Key::Char('b') => Motion::WordBack(false),
                Key::Char('B') => Motion::WordBack(true),
                Key::Char('e') => Motion::WordEnd(false),
                Key::Char('E') => Motion::WordEnd(true),
                Key::Char('0') => Motion::LineStart,
                Key::Char('^') => Motion::FirstNonBlank,
                Key::Char('$') => Motion::LineEnd,
                Key::Char('G') => Motion::FileEnd,
                Key::Char('%') => Motion::MatchBracket,
                Key::Char('}') => Motion::ParagraphForward,
                Key::Char('{') => Motion::ParagraphBack,
                Key::Char(';') => Motion::RepeatFind(false),
                Key::Char(',') => Motion::RepeatFind(true),
                Key::Char('n') => Motion::SearchNext(false),
                Key::Char('N') => Motion::SearchNext(true),
                Key::Char(k @ ('f' | 'F' | 't' | 'T')) => match next() {
                    Some(Key::Char(c)) => Motion::Find(k, c),
                    Some(_) => return Err(false),
                    None => return Err(true),
                },
                Key::Char('g') => match next() {
                    Some(Key::Char('g')) => Motion::FileStart,
                    Some(_) => return Ok(None),
                    None => return Err(true),
                },
                _ => return Ok(None),
            }))
        }

        // Operators.
        let op = match key {
            Key::Char('d') => Some(Op::Delete),
            Key::Char('c') => Some(Op::Change),
            Key::Char('y') => Some(Op::Yank),
            Key::Char('>') => Some(Op::Indent),
            Key::Char('<') => Some(Op::Outdent),
            Key::Char('g') => match keys.get(i) {
                Some(Key::Char('~')) => Some(Op::ToggleCase),
                Some(Key::Char('u')) => Some(Op::Lower),
                Some(Key::Char('U')) => Some(Op::Upper),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            if key == Key::Char('g') {
                i += 1;
            }
            if visual {
                return done(Action::Operate(op, Target::Lines));
            }
            let count2 = read_count(&mut i);
            let Some(k2) = next(&mut i) else { return Parsed::More };
            // Doubled: dd, cc, yy, >>, <<, g~~, guu, gUU.
            let doubled = match (op, k2) {
                (Op::ToggleCase, Key::Char('~')) | (Op::Lower, Key::Char('u')) | (Op::Upper, Key::Char('U')) => true,
                (_, k) => k == key,
            };
            if doubled {
                let total = count.unwrap_or(1) * count2.unwrap_or(1);
                return Parsed::Done(Cmd { register, count: Some(total), action: Action::Operate(op, Target::Lines) });
            }
            if let Key::Char(a @ ('i' | 'a')) = k2 {
                return match next(&mut i) {
                    Some(Key::Char(o)) => done(Action::Operate(op, Target::Object(a == 'a', o))),
                    Some(_) => Parsed::Invalid,
                    None => Parsed::More,
                };
            }
            return match motion(k2, keys, &mut i) {
                Ok(Some(m)) => done(Action::Operate(op, Target::Motion(m, count2))),
                Ok(None) | Err(false) => Parsed::Invalid,
                Err(true) => Parsed::More,
            };
        }
        match motion(key, keys, &mut i) {
            Ok(Some(m)) => return done(Action::Move(m)),
            Err(true) => return Parsed::More,
            Err(false) => return Parsed::Invalid,
            Ok(None) => {}
        }
        match key {
            Key::Char('r') => match next(&mut i) {
                Some(Key::Char(c)) => done(Action::Replace(c)),
                Some(Key::Enter) => done(Action::Replace('\n')),
                Some(_) => Parsed::Invalid,
                None => Parsed::More,
            },
            Key::Char(c) if "xXpPJ~uUiaIAoOvVDCYsS.:/?*#".contains(c) => done(Action::Simple(c)),
            Key::Ctrl(c) => done(Action::Ctrl(c)),
            Key::Esc => done(Action::Simple('\u{1b}')),
            Key::Enter => done(Action::Move(Motion::Down)),
            _ => Parsed::Invalid,
        }
    }

    fn run_vim(&mut self, buf: &mut Buffer) {
        let visual = matches!(self.mode, Mode::Visual | Mode::VisualLine);
        let cmd = match Self::parse(&self.pending, visual) {
            Parsed::More => return,
            Parsed::Invalid => {
                self.pending.clear();
                return;
            }
            Parsed::Done(cmd) => cmd,
        };
        let keys = std::mem::take(&mut self.pending);
        let is_change = match cmd.action {
            Action::Operate(Op::Yank, _) | Action::Move(_) => false,
            Action::Operate(..) | Action::Replace(_) => true,
            Action::Simple(c) => "xXpPJ~iaIAoODCsS".contains(c),
            Action::Ctrl(_) => false,
        };
        if is_change && !self.replaying {
            self.recording = Some(keys);
        }
        if is_change {
            self.change_depth = Some(buf.undo_depth());
            self.redo_groups.clear();
        }
        if visual {
            self.run_visual(buf, cmd);
        } else {
            self.run_normal(buf, cmd);
        }
        // A change that didn't enter insert mode is complete.
        if is_change && self.mode != Mode::Insert {
            self.finish_change(buf);
        }
    }

    fn finish_change(&mut self, buf: &Buffer) {
        if let Some(before) = self.change_depth.take() {
            let after = buf.undo_depth();
            if after > before {
                self.undo_groups.push((before, after));
            }
        }
        if let Some(keys) = self.recording.take() {
            self.last_change = keys;
        }
    }

    // ----- vim: motions ---------------------------------------------------------------

    /// Where `motion` (repeated `count` times) leads from `p`, and how it ranges.
    fn target(&mut self, buf: &Buffer, p: usize, motion: Motion, count: usize, for_op: bool) -> Option<(usize, Kind)> {
        let line = line_of(buf, p);
        let col_goal = |vim: &mut Vim| *vim.goal.get_or_insert_with(|| buf.visual_col(p));
        Some(match motion {
            Motion::Left => (p.saturating_sub(count).max(buf.line_start(line)), Kind::Exclusive),
            Motion::Right => {
                let max = if for_op { buf.line_end(line) } else { last_col(buf, line) };
                ((p + count).min(max), Kind::Exclusive)
            }
            Motion::Down | Motion::Up => {
                let target =
                    if motion == Motion::Down { (line + count).min(last_line(buf)) } else { line.checked_sub(count)? };
                if target == line && !for_op {
                    return None;
                }
                let goal = col_goal(self);
                let pos = if goal == usize::MAX {
                    last_col(buf, target)
                } else {
                    buf.pos_at_visual_col(target, goal).min(last_col(buf, target))
                };
                (pos, Kind::Linewise)
            }
            Motion::WordStart(big) => {
                let mut q = p;
                for _ in 0..count {
                    q = word_forward(buf, q, big);
                }
                (q, Kind::Exclusive)
            }
            Motion::WordBack(big) => {
                let mut q = p;
                for _ in 0..count {
                    q = word_back(buf, q, big);
                }
                (q, Kind::Exclusive)
            }
            Motion::WordEnd(big) => {
                let mut q = p;
                for _ in 0..count {
                    q = word_end(buf, q, big);
                }
                (q, Kind::Inclusive)
            }
            Motion::LineStart => (buf.line_start(line), Kind::Exclusive),
            Motion::FirstNonBlank => (first_non_blank(buf, line), Kind::Exclusive),
            Motion::LineEnd => {
                let l = (line + count - 1).min(last_line(buf));
                self.goal = Some(usize::MAX);
                (last_col(buf, l), Kind::Inclusive)
            }
            Motion::FileStart | Motion::FileEnd => {
                let l = match (motion, count) {
                    (_, n) if self_count_given(n) => (n - 1).min(last_line(buf)),
                    (Motion::FileStart, _) => 0,
                    _ => last_line(buf),
                };
                (first_non_blank(buf, l), Kind::Linewise)
            }
            Motion::Find(kind, c) => {
                self.last_find = Some((kind, c));
                (find_in_line(buf, p, kind, c, count, false)?, find_kind(kind))
            }
            Motion::RepeatFind(reverse) => {
                let (kind, c) = self.last_find?;
                let kind = if reverse { flip_find(kind) } else { kind };
                (find_in_line(buf, p, kind, c, count, true)?, find_kind(kind))
            }
            Motion::MatchBracket => {
                let (s, e) = (p, buf.line_end(line));
                let (at, pair) = match &self.bracket_hint {
                    Some((l, brackets)) if *l == line => *brackets.iter().find(|(at, _)| (s..e).contains(at))?,
                    _ => {
                        let at = (s..e).find(|&q| "()[]{}".contains(ch(buf, q)))?;
                        (at, buf.matching_bracket(at, 1_000_000))
                    }
                };
                let (open, close) = pair?;
                (if at == open { close } else { open }, Kind::Inclusive)
            }
            Motion::ParagraphForward | Motion::ParagraphBack => {
                let mut l = line;
                for _ in 0..count {
                    let forward = motion == Motion::ParagraphForward;
                    // Skip blank lines, then go to the next blank line.
                    loop {
                        let next = if forward { l + 1 } else { l.wrapping_sub(1) };
                        if next > last_line(buf) {
                            break;
                        }
                        l = next;
                        if !is_empty_line(buf, l) {
                            break;
                        }
                    }
                    loop {
                        if is_empty_line(buf, l) {
                            break;
                        }
                        let next = if forward { l + 1 } else { l.wrapping_sub(1) };
                        if next > last_line(buf) {
                            break;
                        }
                        l = next;
                    }
                }
                let pos = if is_empty_line(buf, l) {
                    buf.line_start(l)
                } else if motion == Motion::ParagraphForward {
                    last_col(buf, l)
                } else {
                    buf.line_start(l)
                };
                (pos, Kind::Exclusive)
            }
            Motion::SearchNext(reverse) => {
                let (pat, forward) = self.last_search.clone()?;
                let mut q = p;
                for _ in 0..count {
                    q = search(buf, &pat, q, forward != reverse)?;
                }
                (q, Kind::Exclusive)
            }
        })
    }

    // ----- vim: normal mode -----------------------------------------------------------

    fn run_normal(&mut self, buf: &mut Buffer, cmd: Cmd) {
        let n = cmd.count.unwrap_or(1);
        let p = self.head;
        let reg = cmd.register;
        match cmd.action {
            Action::Move(m) => {
                let vertical = matches!(m, Motion::Up | Motion::Down);
                if !vertical && m != Motion::LineEnd {
                    self.goal = None;
                }
                let count = if matches!(m, Motion::FileStart | Motion::FileEnd) { cmd.count.unwrap_or(0) } else { n };
                if let Some((q, _)) = self.target(buf, p, m, count, false) {
                    self.head = q;
                }
            }
            Action::Operate(op, target) => {
                if let Some((s, e, linewise)) = self.op_range(buf, p, op, target, cmd.count) {
                    self.operate(buf, op, s, e, linewise, reg);
                }
            }
            Action::Replace(c) => {
                let line = line_of(buf, p);
                if p + n <= buf.line_end(line) {
                    if c == '\n' {
                        buf.apply_edits(vec![(p, p + n, buf.line_ending.to_owned())]);
                        self.head = buf.line_start(line + 1);
                    } else {
                        buf.apply_edits(vec![(p, p + n, c.to_string().repeat(n))]);
                        self.head = p + n - 1;
                    }
                }
            }
            Action::Ctrl('r') => {
                for _ in 0..n {
                    self.redo(buf);
                }
            }
            Action::Ctrl(_) => {}
            Action::Simple(c) => self.simple(buf, c, n, cmd.count, reg),
        }
    }

    fn simple(&mut self, buf: &mut Buffer, c: char, n: usize, count: Option<usize>, reg: Option<char>) {
        let p = self.head;
        let line = line_of(buf, p);
        match c {
            'x' | 'X' | 's' => {
                let target = if c == 'X' { Motion::Left } else { Motion::Right };
                let op = if c == 's' { Op::Change } else { Op::Delete };
                if let Some((s, e, l)) = self.op_range(buf, p, op, Target::Motion(target, None), Some(n)) {
                    self.operate(buf, op, s, e, l, reg);
                }
            }
            'D' | 'C' | 'Y' => {
                let op = match c {
                    'D' => Op::Delete,
                    'C' => Op::Change,
                    _ => Op::Yank,
                };
                let target = if c == 'Y' { Target::Lines } else { Target::Motion(Motion::LineEnd, None) };
                if let Some((s, e, l)) = self.op_range(buf, p, op, target, Some(n)) {
                    self.operate(buf, op, s, e, l, reg);
                }
            }
            'S' => {
                if let Some((s, e, l)) = self.op_range(buf, p, Op::Change, Target::Lines, Some(n)) {
                    self.operate(buf, Op::Change, s, e, l, reg);
                }
            }
            'p' | 'P' => self.put(buf, c == 'p', n, reg),
            'J' => {
                let lines = n.max(2) - 1;
                for _ in 0..lines {
                    self.join(buf, line);
                }
            }
            '~' => {
                let e = (p + n).min(buf.line_end(line));
                if e > p {
                    let text: String = buf.slice_string(p, e).chars().map(toggle_case).collect();
                    buf.apply_edits(vec![(p, e, text)]);
                    self.head = e.min(last_col(buf, line));
                }
            }
            'u' => {
                for _ in 0..n {
                    self.undo(buf);
                }
            }
            '.' => self.repeat(buf, count),
            'i' => self.start_insert(buf, p, n),
            'a' => {
                let at = if buf.line_end(line) > buf.line_start(line) { p + 1 } else { p };
                self.start_insert(buf, at, n);
            }
            'I' => {
                let at = first_non_blank(buf, line);
                let at = if is_empty_line(buf, line) { buf.line_start(line) } else { at };
                self.start_insert(buf, at, n);
            }
            'A' => self.start_insert(buf, buf.line_end(line), n),
            'o' | 'O' => {
                let nl = buf.line_ending;
                let indent = if self.autoindent { leading_ws(buf, line) } else { String::new() };
                if c == 'o' {
                    let at = buf.line_end(line);
                    buf.apply_edits(vec![(at, at, format!("{nl}{indent}"))]);
                    let pos = buf.line_start(line + 1) + indent.chars().count();
                    self.start_insert(buf, pos, n);
                } else {
                    let at = buf.line_start(line);
                    buf.apply_edits(vec![(at, at, format!("{indent}{nl}"))]);
                    self.start_insert(buf, at + indent.chars().count(), n);
                }
            }
            'v' | 'V' => {
                self.mode = if c == 'v' { Mode::Visual } else { Mode::VisualLine };
                self.anchor = p;
            }
            ':' | '/' | '?' => {
                self.mode = Mode::Command;
                self.cmdline = c.to_string();
            }
            '*' | '#' => {
                if let Some((s, e)) = buf.word_at(p) {
                    let word = buf.slice_string(s, e);
                    self.last_search = Some((word.clone(), c == '*'));
                    if let Some(q) = search(buf, &word, if c == '*' { p } else { s }, c == '*') {
                        self.head = q;
                    }
                }
            }
            _ => {} // Esc and unknown keys: nothing
        }
    }

    /// The char range (and linewise flag) an operator covers.
    fn op_range(
        &mut self,
        buf: &Buffer,
        p: usize,
        op: Op,
        target: Target,
        count: Option<usize>,
    ) -> Option<(usize, usize, bool)> {
        let n = count.unwrap_or(1);
        let line = line_of(buf, p);
        match target {
            Target::Lines => {
                let last = (line + n - 1).min(last_line(buf));
                Some((buf.line_start(line), next_line_start(buf, last), true))
            }
            Target::Object(around, obj) => text_object(buf, p, around, obj).map(|(s, e)| (s, e, false)),
            Target::Motion(m, count2) => {
                let total = n * count2.unwrap_or(1);
                // `cw` on a word changes to its end, like `ce`.
                let m = match m {
                    Motion::WordStart(big) if op == Op::Change && class(ch(buf, p), big) != 0 => Motion::WordEnd(big),
                    m => m,
                };
                let total = if matches!(m, Motion::FileStart | Motion::FileEnd) {
                    if count.is_some() || count2.is_some() { total } else { 0 }
                } else {
                    total
                };
                // `cw`/`ce` on the last char of a word stays on it.
                let (q, kind) = if matches!(m, Motion::WordEnd(_)) && op == Op::Change && total == 1 {
                    let big = matches!(m, Motion::WordEnd(true));
                    let c = class(ch(buf, p), big);
                    if c != 0 && class(ch(buf, p + 1), big) != c {
                        (p, Kind::Inclusive)
                    } else {
                        self.target(buf, p, m, total, true)?
                    }
                } else {
                    self.target(buf, p, m, total, true)?
                };
                let (mut s, mut e) = (p.min(q), p.max(q));
                match kind {
                    Kind::Linewise => {
                        let (l1, l2) = (line_of(buf, s), line_of(buf, e));
                        Some((buf.line_start(l1), next_line_start(buf, l2), true))
                    }
                    Kind::Inclusive => Some((s, (e + 1).min(buf.len_chars()), false)),
                    Kind::Exclusive => {
                        // An exclusive motion ending at the start of a later line
                        // stops at the end of the line before (`dw` on the last word).
                        if e > s && line_of(buf, e) > line_of(buf, s) && buf.line_start(line_of(buf, e)) == e {
                            let prev = line_of(buf, e) - 1;
                            e = buf.line_end(prev);
                            if s <= first_non_blank(buf, line_of(buf, s)) && prev > line_of(buf, s) {
                                s = buf.line_start(line_of(buf, s));
                                return Some((s, next_line_start(buf, prev), true));
                            }
                        }
                        Some((s, e, false))
                    }
                }
            }
        }
    }

    fn set_register(&mut self, reg: Option<char>, text: String, linewise: bool, yank: bool) {
        let r = Register { text: text.clone(), linewise };
        match reg {
            Some('+' | '*') => self.effects.push(Effect::Clipboard(text)),
            Some('_') => return,
            Some(c) if c.is_ascii_alphabetic() => {
                let key = c.to_ascii_lowercase();
                if c.is_ascii_uppercase() {
                    let e = self.registers.entry(key).or_default();
                    e.text.push_str(&r.text);
                } else {
                    self.registers.insert(key, r.clone());
                }
            }
            _ => {}
        }
        if yank && reg.is_none() {
            self.registers.insert('0', r.clone());
        }
        self.registers.insert('"', r);
    }

    fn operate(&mut self, buf: &mut Buffer, op: Op, s: usize, e: usize, linewise: bool, reg: Option<char>) {
        let len = buf.len_chars();
        let (s, e) = (s.min(len), e.min(len));
        let line = line_of(buf, s);
        match op {
            Op::Yank => {
                let mut text = buf.slice_string(s, e);
                if linewise && !text.ends_with('\n') {
                    text.push('\n');
                }
                self.set_register(reg, text, linewise, true);
                if !linewise {
                    self.head = s;
                } else {
                    self.head = self.head.min(s).max(buf.line_start(line)).max(self.head.min(e));
                    if line_of(buf, self.head) != line {
                        self.head = s;
                    }
                }
            }
            Op::Delete => {
                let mut text = buf.slice_string(s, e);
                let (mut s, mut e) = (s, e);
                if linewise {
                    let had_break = text.ends_with('\n');
                    if !had_break {
                        text.push('\n');
                    }
                    // Deleting the last lines removes the line break before them
                    // instead, so the text keeps (or keeps lacking) its final one.
                    if e >= len && s > 0 {
                        let back = if s >= 2 && ch(buf, s - 2) == '\r' { 2 } else { 1 };
                        s -= back;
                        if had_break {
                            e -= back;
                        }
                    }
                }
                self.set_register(reg, text, linewise, false);
                buf.apply_edits(vec![(s, e, String::new())]);
                if linewise {
                    let l = line_of(buf, s.min(buf.len_chars())).min(last_line(buf));
                    let l = if s > 0 && e >= len { line_of(buf, s) } else { l };
                    self.head = first_non_blank(buf, l);
                } else {
                    self.head = s;
                }
            }
            Op::Change => {
                if linewise {
                    let last = line_of(buf, e.saturating_sub(1).max(s));
                    let (cs, ce) = (buf.line_start(line), buf.line_end(last.max(line)));
                    let indent = if self.autoindent { leading_ws(buf, line) } else { String::new() };
                    let mut text = buf.slice_string(cs, ce);
                    text.push('\n');
                    self.set_register(reg, text, true, false);
                    buf.apply_edits(vec![(cs, ce, indent.clone())]);
                    self.start_insert(buf, cs + indent.chars().count(), 1);
                } else {
                    self.set_register(reg, buf.slice_string(s, e), false, false);
                    buf.apply_edits(vec![(s, e, String::new())]);
                    self.start_insert(buf, s, 1);
                }
            }
            Op::Indent | Op::Outdent => {
                let (l1, l2) = (line, line_of(buf, e.saturating_sub(1).max(s)));
                let mut edits = Vec::new();
                for l in l1..=l2.max(l1) {
                    let ls = buf.line_start(l);
                    if op == Op::Indent {
                        if !is_empty_line(buf, l) {
                            edits.push((ls, ls, "\t".to_owned()));
                        }
                    } else {
                        let ws = leading_ws(buf, l);
                        let cut =
                            if ws.starts_with('\t') { 1 } else { ws.chars().take(8).take_while(|c| *c == ' ').count() };
                        if cut > 0 {
                            edits.push((ls, ls + cut, String::new()));
                        }
                    }
                }
                buf.apply_edits(edits);
                self.head = first_non_blank(buf, l1);
            }
            Op::ToggleCase | Op::Lower | Op::Upper => {
                let text: String = buf
                    .slice_string(s, e)
                    .chars()
                    .map(|c| match op {
                        Op::Lower => c.to_lowercase().next().unwrap_or(c),
                        Op::Upper => c.to_uppercase().next().unwrap_or(c),
                        _ => toggle_case(c),
                    })
                    .collect();
                buf.apply_edits(vec![(s, e, text)]);
                self.head = if linewise { self.head.min(buf.len_chars()) } else { s };
            }
        }
    }

    fn put(&mut self, buf: &mut Buffer, after: bool, n: usize, reg: Option<char>) {
        let r = self.registers.get(&reg.map_or('"', |c| c.to_ascii_lowercase())).cloned().unwrap_or_default();
        if r.text.is_empty() {
            return;
        }
        let p = self.head;
        let line = line_of(buf, p);
        let text = r.text.repeat(n);
        if r.linewise {
            let text = text.replace('\n', buf.line_ending);
            let (at, insert) = if after {
                let at = next_line_start(buf, line);
                if at >= buf.len_chars() && !buf.rope().chars_at(at.saturating_sub(1)).next().is_some_and(|c| c == '\n')
                {
                    // After a last line without a line break.
                    let t = format!("{}{}", buf.line_ending, text.strip_suffix(buf.line_ending).unwrap_or(&text));
                    (at, t)
                } else {
                    (at, text)
                }
            } else {
                (buf.line_start(line), text)
            };
            buf.apply_edits(vec![(at, at, insert)]);
            let new_line = if after { line + 1 } else { line };
            self.head = first_non_blank(buf, new_line.min(last_line(buf)));
        } else {
            let at = if after && buf.line_end(line) > buf.line_start(line) { p + 1 } else { p };
            let count = text.chars().count();
            buf.apply_edits(vec![(at, at, text)]);
            self.head = at + count - 1;
        }
    }

    fn join(&mut self, buf: &mut Buffer, line: usize) {
        if line >= last_line(buf) {
            return;
        }
        let e = buf.line_end(line);
        let next = buf.line_start(line + 1);
        let next_text = buf.line_text(line + 1);
        let ws = next_text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let rest = next_text.trim_start();
        let this_ends_blank = e == buf.line_start(line) || matches!(ch(buf, e.saturating_sub(1)), ' ' | '\t');
        let sep = if rest.is_empty() || rest.starts_with(')') || this_ends_blank { "" } else { " " };
        buf.apply_edits(vec![(e, next + ws, sep.to_owned())]);
        self.head = e;
    }

    fn start_insert(&mut self, buf: &mut Buffer, at: usize, count: usize) {
        self.mode = Mode::Insert;
        self.insert_count = count.max(1);
        self.insert_keys.clear();
        buf.set_single(Selection::cursor(at.min(buf.len_chars())));
        self.head = at;
    }

    // ----- insert mode --------------------------------------------------------------------

    fn insert_key(&mut self, buf: &mut Buffer, key: Key) {
        if key == Key::Esc {
            self.leave_insert(buf);
            return;
        }
        self.observe(key);
        type_key(buf, key, self.autoindent);
    }

    fn leave_insert(&mut self, buf: &mut Buffer) {
        // `3ihi<Esc>` types the text two more times.
        let keys = std::mem::take(&mut self.insert_keys);
        for _ in 1..self.insert_count {
            for k in &keys {
                type_key(buf, *k, self.autoindent);
            }
        }
        self.insert_count = 1;
        if let Some(r) = &mut self.recording {
            r.push(Key::Esc);
        }
        let head = buf.primary().head;
        let line = line_of(buf, head);
        self.head = if head > buf.line_start(line) { head - 1 } else { head };
        self.mode = Mode::Normal;
        self.goal = None;
        if self.flavor == Flavor::Helix {
            self.anchor = self.head;
        }
        self.finish_change(buf);
    }

    // ----- visual mode --------------------------------------------------------------------

    fn run_visual(&mut self, buf: &mut Buffer, cmd: Cmd) {
        let n = cmd.count.unwrap_or(1);
        let linewise = self.mode == Mode::VisualLine;
        let (s, e) = (self.anchor.min(self.head), self.anchor.max(self.head));
        let (rs, re) = if linewise {
            (buf.line_start(line_of(buf, s)), next_line_start(buf, line_of(buf, e)))
        } else {
            (s, (e + 1).min(buf.len_chars()))
        };
        match cmd.action {
            Action::Move(m) => {
                let count = if matches!(m, Motion::FileStart | Motion::FileEnd) { cmd.count.unwrap_or(0) } else { n };
                if let Some((q, _)) = self.target(buf, self.head, m, count, false) {
                    self.head = q;
                }
            }
            Action::Operate(op, _) => {
                self.mode = Mode::Normal;
                self.operate(buf, op, rs, re, linewise, cmd.register);
                if op == Op::Yank {
                    self.head = rs.min(self.head).max(if linewise { rs } else { s });
                }
            }
            Action::Replace(c) => {
                let text: String =
                    buf.slice_string(rs, re).chars().map(|x| if x == '\n' || x == '\r' { x } else { c }).collect();
                buf.apply_edits(vec![(rs, re, text)]);
                self.mode = Mode::Normal;
                self.head = rs;
            }
            Action::Simple(c) => match c {
                'o' => std::mem::swap(&mut self.anchor, &mut self.head),
                'v' | 'V' | '\u{1b}' => {
                    let wanted = if c == 'v' { Mode::Visual } else { Mode::VisualLine };
                    self.mode = if c != '\u{1b}' && self.mode != wanted { wanted } else { Mode::Normal };
                }
                'x' | 'D' | 'X' => {
                    self.mode = Mode::Normal;
                    let lw = linewise || c != 'x';
                    let (rs, re) = if lw {
                        (buf.line_start(line_of(buf, s)), next_line_start(buf, line_of(buf, e)))
                    } else {
                        (rs, re)
                    };
                    self.operate(buf, Op::Delete, rs, re, lw, cmd.register);
                }
                's' => {
                    self.mode = Mode::Normal;
                    self.operate(buf, Op::Change, rs, re, linewise, cmd.register);
                }
                'J' => {
                    self.mode = Mode::Normal;
                    let (l1, l2) = (line_of(buf, s), line_of(buf, e));
                    for _ in 0..(l2 - l1).max(1) {
                        self.join(buf, l1);
                    }
                }
                '~' | 'u' | 'U' => {
                    self.mode = Mode::Normal;
                    let op = match c {
                        'u' => Op::Lower,
                        'U' => Op::Upper,
                        _ => Op::ToggleCase,
                    };
                    self.operate(buf, op, rs, re, linewise, None);
                    self.head = rs;
                }
                'p' | 'P' => {
                    self.mode = Mode::Normal;
                    let r = self.registers.get(&'"').cloned().unwrap_or_default();
                    buf.apply_edits(vec![(rs, re, r.text.clone())]);
                    self.registers.insert('"', Register { text: buf.slice_string(rs, rs), linewise: false });
                    self.head = rs + r.text.chars().count().saturating_sub(1);
                }
                ':' => {
                    self.mode = Mode::Command;
                    self.cmdline = ":".into();
                }
                _ => {}
            },
            Action::Ctrl(_) => {}
        }
    }

    // ----- undo / repeat --------------------------------------------------------------

    fn undo(&mut self, buf: &mut Buffer) {
        match self.undo_groups.pop() {
            Some((before, after)) => {
                while buf.undo_depth() > before && buf.undo() {}
                self.redo_groups.push((before, after));
            }
            None => {
                buf.undo();
            }
        }
        self.head = clamp_normal(buf, buf.primary().start());
    }

    fn redo(&mut self, buf: &mut Buffer) {
        match self.redo_groups.pop() {
            Some((before, after)) => {
                while buf.undo_depth() < after && buf.redo() {}
                self.undo_groups.push((before, after));
            }
            None => {
                buf.redo();
            }
        }
        self.head = clamp_normal(buf, buf.primary().start());
    }

    fn repeat(&mut self, buf: &mut Buffer, count: Option<usize>) {
        let mut keys = self.last_change.clone();
        if keys.is_empty() {
            return;
        }
        if let Some(n) = count {
            // A new count replaces the recorded one.
            let start = usize::from(keys.first() == Some(&Key::Char('"'))) * 2;
            let digits = keys[start..].iter().take_while(|k| matches!(k, Key::Char(c) if c.is_ascii_digit())).count();
            keys.splice(start..start + digits, n.to_string().chars().map(Key::Char));
        }
        self.replaying = true;
        for k in &keys {
            self.message = None;
            match self.mode {
                Mode::Insert => self.insert_key(buf, *k),
                Mode::Command => self.cmdline_key(buf, *k),
                _ => {
                    self.pending.push(*k);
                    self.run_vim(buf);
                }
            }
        }
        self.replaying = false;
        self.last_change = keys;
    }

    // ----- command line -------------------------------------------------------------------

    fn cmdline_key(&mut self, buf: &mut Buffer, key: Key) {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.cmdline.clear();
            }
            Key::Backspace => {
                self.cmdline.pop();
                if self.cmdline.is_empty() {
                    self.mode = Mode::Normal;
                }
            }
            Key::Char(c) => self.cmdline.push(c),
            Key::Enter => {
                let line = std::mem::take(&mut self.cmdline);
                self.mode = Mode::Normal;
                self.execute(buf, &line);
            }
            _ => {}
        }
    }

    fn execute(&mut self, buf: &mut Buffer, line: &str) {
        let (kind, body) = line.split_at(1);
        if kind == "/" || kind == "?" {
            let forward = kind == "/";
            let pat = if body.is_empty() {
                self.last_search.as_ref().map(|s| s.0.clone()).unwrap_or_default()
            } else {
                body.to_owned()
            };
            self.last_search = Some((pat.clone(), forward));
            match search(buf, &pat, self.head, forward) {
                Some(q) => self.head = q,
                None => self.message = Some(format!("Pattern not found: {pat}")),
            }
            return;
        }
        let cmd = body.trim();
        match cmd {
            "w" => self.effects.push(Effect::Save),
            "q" => self.effects.push(Effect::Quit { force: false }),
            "q!" => self.effects.push(Effect::Quit { force: true }),
            "wq" | "x" => self.effects.push(Effect::SaveQuit),
            "noh" | "nohlsearch" | "" => {}
            c if c.chars().all(|c| c.is_ascii_digit()) => {
                let l = c.parse::<usize>().unwrap_or(1).saturating_sub(1).min(last_line(buf));
                self.head = first_non_blank(buf, l);
            }
            c if c.starts_with("s/") || c.starts_with("%s/") => {
                self.change_depth = Some(buf.undo_depth());
                if !self.substitute(buf, c) {
                    self.message = Some("Pattern not found".into());
                }
                self.finish_change(buf);
            }
            c => self.message = Some(format!("Not an editor command: {c}")),
        }
    }

    /// `:s/a/b/` on the cursor line, or `:%s/a/b/g` on every line (plain text).
    fn substitute(&mut self, buf: &mut Buffer, cmd: &str) -> bool {
        let all_lines = cmd.starts_with('%');
        let spec = cmd.trim_start_matches('%').trim_start_matches('s');
        let parts: Vec<&str> = spec.split('/').collect();
        if parts.len() < 3 || parts[1].is_empty() {
            return false;
        }
        let (pat, rep, global) = (parts[1], parts[2], parts.get(3).is_some_and(|f| f.contains('g')));
        let lines: Vec<usize> = if all_lines { (0..=last_line(buf)).collect() } else { vec![line_of(buf, self.head)] };
        let mut edits = Vec::new();
        let mut last_changed = None;
        for l in lines {
            let text = buf.line_text(l);
            let new = if global { text.replace(pat, rep) } else { text.replacen(pat, rep, 1) };
            if new != text {
                edits.push((buf.line_start(l), buf.line_end(l), new));
                last_changed = Some(l);
            }
        }
        let Some(l) = last_changed else { return false };
        buf.apply_edits(edits);
        self.head = first_non_blank(buf, l);
        true
    }

    // ----- helix ----------------------------------------------------------------------

    fn run_helix(&mut self, buf: &mut Buffer) {
        let keys = self.pending.clone();
        let mut i = 0;
        let mut count = None;
        while let Some(Key::Char(c)) = keys.get(i) {
            match c.to_digit(10) {
                Some(d) if d > 0 || count.is_some() => {
                    count = Some(count.unwrap_or(0) * 10 + d as usize);
                    i += 1;
                }
                _ => break,
            }
        }
        let Some(&key) = keys.get(i) else { return };
        let arg = keys.get(i + 1).copied();
        let n = count.unwrap_or(1);
        let extend = self.mode == Mode::Select;
        let len = buf.len_chars();
        let (s, e) = (self.anchor.min(self.head), self.anchor.max(self.head));
        let mut done = true;
        let mut change = false;
        let depth = buf.undo_depth();
        let move_to = |vim: &mut Vim, q: usize| {
            if !extend {
                vim.anchor = q;
            }
            vim.head = q;
        };
        match key {
            Key::Char('h') | Key::Left => {
                let q = self.head.saturating_sub(n);
                move_to(self, q);
            }
            Key::Char('l') | Key::Right => {
                let q = (self.head + n).min(len.saturating_sub(1));
                move_to(self, q);
            }
            Key::Char('j') | Key::Down | Key::Char('k') | Key::Up => {
                let down = matches!(key, Key::Char('j') | Key::Down);
                let line = line_of(buf, self.head);
                let target = if down { (line + n).min(last_line(buf)) } else { line.saturating_sub(n) };
                let goal = *self.goal.get_or_insert_with(|| buf.visual_col(self.head));
                let q = buf.pos_at_visual_col(target, goal).min(last_col(buf, target));
                move_to(self, q);
            }
            Key::Char(c @ ('w' | 'W' | 'e' | 'E' | 'b' | 'B')) => {
                let big = c.is_ascii_uppercase();
                for _ in 0..n {
                    let h = self.head;
                    match c.to_ascii_lowercase() {
                        'w' | 'e' => {
                            // Start after a word boundary, select up to the next word (w) or word end (e).
                            let start = if h + 1 < len && class(ch(buf, h), big) != class(ch(buf, h + 1), big) {
                                h + 1
                            } else {
                                h
                            };
                            let end = if c.eq_ignore_ascii_case(&'w') {
                                word_forward(buf, start, big).saturating_sub(1).max(start)
                            } else {
                                word_end(buf, start.saturating_sub(1).max(if start == h { h } else { start - 1 }), big)
                            };
                            if !extend {
                                self.anchor = start;
                            }
                            self.head = end;
                        }
                        _ => {
                            let start =
                                if h > 0 && class(ch(buf, h), big) != class(ch(buf, h - 1), big) { h - 1 } else { h };
                            let back = word_back(buf, start + 1, big);
                            if !extend {
                                self.anchor = start;
                            }
                            self.head = back;
                        }
                    }
                }
                self.goal = None;
            }
            Key::Char('x') => {
                let (l1, l2) = (line_of(buf, s), line_of(buf, e));
                let whole = s == buf.line_start(l1) && e + 1 == next_line_start(buf, l2);
                let l2 = if whole { (l2 + n).min(last_line(buf)) } else { (l2 + n - 1).min(last_line(buf)) };
                self.anchor = buf.line_start(l1);
                self.head = next_line_start(buf, l2).saturating_sub(1);
            }
            Key::Char('%') => {
                self.anchor = 0;
                self.head = len.saturating_sub(1);
            }
            Key::Char(';') => self.anchor = self.head,
            Key::Char('v') => self.mode = if extend { Mode::Normal } else { Mode::Select },
            Key::Esc => {
                if extend {
                    self.mode = Mode::Normal;
                } else {
                    self.anchor = self.head;
                }
            }
            Key::Char('d') | Key::Char('c') => {
                let text = buf.slice_string(s, (e + 1).min(len));
                self.set_register(None, text, false, false);
                buf.apply_edits(vec![(s, (e + 1).min(len), String::new())]);
                change = true;
                if key == Key::Char('c') {
                    self.start_insert(buf, s, 1);
                    self.change_depth = Some(depth);
                } else {
                    self.anchor = s.min(buf.len_chars().saturating_sub(1));
                    self.head = self.anchor;
                }
                self.mode = if key == Key::Char('c') { Mode::Insert } else { Mode::Normal };
            }
            Key::Char('y') => {
                let text = buf.slice_string(s, (e + 1).min(len));
                self.set_register(None, text, false, true);
            }
            Key::Char(c @ ('p' | 'P')) => {
                let r = self.registers.get(&'"').cloned().unwrap_or_default();
                if !r.text.is_empty() {
                    let at = if c == 'p' { (e + 1).min(len) } else { s };
                    let count = r.text.chars().count();
                    buf.apply_edits(vec![(at, at, r.text.repeat(n))]);
                    self.anchor = at;
                    self.head = at + count * n - 1;
                    change = true;
                }
            }
            Key::Char(c @ ('i' | 'a' | 'I' | 'A')) => {
                let at = match c {
                    'i' => s,
                    'a' => (e + 1).min(len),
                    'I' => first_non_blank(buf, line_of(buf, s)),
                    _ => buf.line_end(line_of(buf, e)),
                };
                self.change_depth = Some(depth);
                self.recording = None;
                self.start_insert(buf, at, 1);
            }
            Key::Char(c @ ('o' | 'O')) => {
                let line = line_of(buf, if c == 'o' { e } else { s });
                let nl = buf.line_ending;
                let at = if c == 'o' { buf.line_end(line) } else { buf.line_start(line) };
                buf.apply_edits(vec![(at, at, nl.to_owned())]);
                self.change_depth = Some(depth);
                let pos = if c == 'o' { buf.line_start(line + 1) } else { at };
                self.start_insert(buf, pos, 1);
            }
            Key::Char('u') => {
                self.undo(buf);
                self.anchor = self.head;
            }
            Key::Char('U') => {
                self.redo(buf);
                self.anchor = self.head;
            }
            Key::Char('r') => match arg {
                Some(Key::Char(c)) => {
                    let text: String = buf
                        .slice_string(s, (e + 1).min(len))
                        .chars()
                        .map(|x| if x == '\n' || x == '\r' { x } else { c })
                        .collect();
                    buf.apply_edits(vec![(s, (e + 1).min(len), text)]);
                    change = true;
                }
                Some(_) => {}
                None => done = false,
            },
            Key::Char('~') => {
                let text: String = buf.slice_string(s, (e + 1).min(len)).chars().map(toggle_case).collect();
                buf.apply_edits(vec![(s, (e + 1).min(len), text)]);
                change = true;
            }
            Key::Char(c @ ('>' | '<')) => {
                let (anchor, head) = (self.anchor, self.head);
                self.operate(buf, if c == '>' { Op::Indent } else { Op::Outdent }, s, (e + 1).min(len), true, None);
                (self.anchor, self.head) = (anchor, head);
                change = true;
            }
            Key::Char('J') => {
                let (l1, l2) = (line_of(buf, s), line_of(buf, e));
                for _ in 0..(l2 - l1).max(1) {
                    self.join(buf, l1);
                }
                self.anchor = self.head;
                change = true;
            }
            Key::Char('g') => match arg {
                Some(Key::Char(c)) => {
                    let line = line_of(buf, self.head);
                    let q = match c {
                        'g' => Some(if count.is_some() { buf.line_start((n - 1).min(last_line(buf))) } else { 0 }),
                        'e' => Some(buf.line_start(last_line(buf))),
                        'h' => Some(buf.line_start(line)),
                        'l' => Some(last_col(buf, line)),
                        's' => Some(first_non_blank(buf, line)),
                        _ => None,
                    };
                    if let Some(q) = q {
                        move_to(self, q);
                    }
                }
                Some(_) => {}
                None => done = false,
            },
            Key::Char(c @ ('f' | 't' | 'F' | 'T')) => match arg {
                Some(Key::Char(t)) => {
                    if let Some(q) = find_in_line(buf, self.head, c, t, n, false) {
                        if !extend {
                            self.anchor = self.head;
                        }
                        self.head = q;
                    }
                }
                Some(_) => {}
                None => done = false,
            },
            Key::Char(c @ ('/' | '?' | ':')) => {
                self.mode = Mode::Command;
                self.cmdline = c.to_string();
            }
            Key::Char(c @ ('n' | 'N')) => {
                if let Some((pat, forward)) = self.last_search.clone()
                    && let Some(q) = search(buf, &pat, self.head, forward == (c == 'n'))
                {
                    self.anchor = q;
                    self.head = q + pat.chars().count() - 1;
                }
            }
            _ => {}
        }
        if done {
            self.pending.clear();
            if change && self.mode != Mode::Insert {
                let after = buf.undo_depth();
                if after > depth {
                    self.undo_groups.push((depth, after));
                    self.redo_groups.clear();
                }
            }
        }
        if self.mode == Mode::Command {
            return;
        }
        let max = buf.len_chars().saturating_sub(1);
        self.anchor = self.anchor.min(max);
        self.head = self.head.min(max);
    }
}

// ----- shared helpers ------------------------------------------------------------------

fn self_count_given(n: usize) -> bool {
    n > 0
}

fn toggle_case(c: char) -> char {
    if c.is_lowercase() { c.to_uppercase().next().unwrap_or(c) } else { c.to_lowercase().next().unwrap_or(c) }
}

fn leading_ws(b: &Buffer, line: usize) -> String {
    b.line_text(line).chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

fn find_kind(kind: char) -> Kind {
    if kind == 'f' || kind == 't' { Kind::Inclusive } else { Kind::Exclusive }
}

fn flip_find(kind: char) -> char {
    match kind {
        'f' => 'F',
        'F' => 'f',
        't' => 'T',
        _ => 't',
    }
}

/// `f`/`F`/`t`/`T` within the cursor line. `repeat`: `;` after `t` skips the adjacent match.
fn find_in_line(b: &Buffer, p: usize, kind: char, c: char, count: usize, repeat: bool) -> Option<usize> {
    let line = line_of(b, p);
    let (ls, le) = (b.line_start(line), b.line_end(line));
    let forward = kind == 'f' || kind == 't';
    let mut q = p;
    for i in 0..count {
        let skip = usize::from(repeat && i == 0 && (kind == 't' || kind == 'T'));
        if forward {
            q = (q + 1 + skip..le).find(|&x| ch(b, x) == c)?;
        } else {
            q = (ls..q.saturating_sub(skip)).rev().find(|&x| ch(b, x) == c)?;
        }
    }
    Some(match kind {
        't' => q - 1,
        'T' => q + 1,
        _ => q,
    })
}

/// Next (or previous) occurrence of `pat` from `p`, wrapping around.
fn search(b: &Buffer, pat: &str, p: usize, forward: bool) -> Option<usize> {
    if pat.is_empty() {
        return None;
    }
    let text = b.text();
    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
    let to_char = |i: usize| text[..i].chars().count();
    let from = byte(p);
    let found = if forward {
        let start = text[from..].chars().next().map_or(from, |c| from + c.len_utf8());
        text[start.min(text.len())..].find(pat).map(|i| start + i).or_else(|| text.find(pat))
    } else {
        text[..from].rfind(pat).or_else(|| text.rfind(pat))
    };
    found.map(to_char)
}

/// `iw`, `aw`, `iW`, `aW`, quotes, and brackets (`(` `)` `b`, `[` `]`, `{` `}` `B`, `<` `>`).
fn text_object(b: &Buffer, p: usize, around: bool, obj: char) -> Option<(usize, usize)> {
    let line = line_of(b, p);
    let (ls, le) = (b.line_start(line), b.line_end(line));
    match obj {
        'w' | 'W' => {
            let big = obj == 'W';
            if le == ls {
                return None;
            }
            let c = class(ch(b, p), big);
            let same = |x: usize| class(ch(b, x), big) == c;
            let mut s = p;
            while s > ls && same(s - 1) {
                s -= 1;
            }
            let mut e = p + 1;
            while e < le && same(e) {
                e += 1;
            }
            if around {
                if c != 0 && e < le && class(ch(b, e), big) == 0 {
                    while e < le && class(ch(b, e), big) == 0 {
                        e += 1;
                    }
                } else if c != 0 {
                    while s > ls && class(ch(b, s - 1), big) == 0 {
                        s -= 1;
                    }
                } else {
                    // On blanks: the blanks plus the following word.
                    if e < le {
                        let c2 = class(ch(b, e), big);
                        while e < le && class(ch(b, e), big) == c2 {
                            e += 1;
                        }
                    }
                }
            }
            Some((s, e))
        }
        '"' | '\'' | '`' => {
            let quotes: Vec<usize> =
                (ls..le).filter(|&x| ch(b, x) == obj && (x == ls || ch(b, x - 1) != '\\')).collect();
            // The pair around the cursor, else the first pair after it.
            let pair = quotes
                .chunks(2)
                .filter(|c| c.len() == 2)
                .map(|c| (c[0], c[1]))
                .find(|&(a, z)| a <= p && p <= z)
                .or_else(|| quotes.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0], c[1])).find(|&(a, _)| a > p))?;
            let (a, z) = pair;
            if around {
                let mut e = z + 1;
                while e < le && matches!(ch(b, e), ' ' | '\t') {
                    e += 1;
                }
                Some((a, e))
            } else {
                Some((a + 1, z))
            }
        }
        _ => {
            let (open, close) = match obj {
                '(' | ')' | 'b' => ('(', ')'),
                '[' | ']' => ('[', ']'),
                '{' | '}' | 'B' => ('{', '}'),
                '<' | '>' => ('<', '>'),
                _ => return None,
            };
            // Walk back to the unmatched opening bracket, then forward to its partner.
            let mut depth = 0;
            let mut q = p;
            let start = loop {
                let c = ch(b, q);
                if c == close && q != p {
                    depth += 1;
                } else if c == open {
                    if depth == 0 {
                        break q;
                    }
                    depth -= 1;
                }
                if q == 0 {
                    return None;
                }
                q -= 1;
            };
            let mut depth = 0;
            let end = (start + 1..b.len_chars()).find(|&x| {
                let c = ch(b, x);
                if c == open {
                    depth += 1;
                } else if c == close {
                    if depth == 0 {
                        return true;
                    }
                    depth -= 1;
                }
                false
            })?;
            Some(if around { (start, end + 1) } else { (start + 1, end) })
        }
    }
}

/// Types one insert-mode key into the buffer (replays and tests; the editor
/// types live keys itself).
fn type_key(buf: &mut Buffer, key: Key, autoindent: bool) {
    match key {
        Key::Char(c) => buf.insert(&c.to_string()),
        Key::Tab => buf.insert("\t"),
        Key::Enter => {
            if autoindent {
                buf.insert_newline();
            } else {
                let nl = buf.line_ending;
                buf.insert(nl);
            }
        }
        Key::Backspace => buf.delete_backward(false),
        Key::Left => buf.move_horizontal(false, false, false),
        Key::Right => buf.move_horizontal(true, false, false),
        Key::Up => buf.move_vertical(-1, false),
        Key::Down => buf.move_vertical(1, false),
        Key::Esc | Key::Ctrl(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_uses_the_editors_brackets() {
        // `f("(", x)`: the `(` in the string isn't code; the editor says so through the hint.
        let mut buf = Buffer::new("f(\"(\", x)");
        let mut vim = Vim::new(Flavor::Vim);
        vim.reset(&mut buf);
        vim.set_bracket_hint((0, vec![(1, Some((1, 8))), (8, Some((1, 8)))]));
        vim.handle(&mut buf, Key::Char('%'));
        assert_eq!(buf.primary().head, 8);
        vim.handle(&mut buf, Key::Char('%'));
        assert_eq!(buf.primary().head, 1);
        // Without it, the text search is fooled by the string.
        vim.clear_bracket_hint();
        vim.handle(&mut buf, Key::Char('0'));
        vim.handle(&mut buf, Key::Char('%'));
        assert_ne!(buf.primary().head, 8);
    }
}
