//! Incremental syntax highlighting with syntect.
//!
//! Parser state is checkpointed every [`CHECKPOINT`] lines, so an edit only
//! invalidates state from the edited line down, and only lines that are
//! actually drawn get highlighted.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::appearance::theme::Color;
use std::sync::{Arc, RwLock};

use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme};
use syntect::parsing::{ParseState, Scope, ScopeStack, ScopeStackOp, SyntaxSet};

use crate::editor::buffer::Buffer;

const CHECKPOINT: usize = 32;
/// Lines longer than this (e.g. minified code) are drawn without highlighting.
const MAX_LINE_CHARS: usize = 5_000;
/// Cached highlighted lines are dropped past this count to bound memory.
const MAX_CACHED_LINES: usize = 20_000;

#[derive(Clone, Debug)]
pub struct Span {
    pub len: usize,
    pub color: Color,
    pub italic: bool,
}

/// Brackets still open at a line's start, innermost last, for rainbow colors.
type Open = Vec<char>;
type State = (ParseState, HighlightState, Open);
/// Open brackets kept track of; past this, more are colored but not remembered.
const MAX_OPEN: usize = 4096;

pub struct Highlighter {
    syntax_name: Option<String>,
    /// `checkpoints[i]` is the state at the start of line `i * CHECKPOINT`.
    checkpoints: Vec<State>,
    /// State at the start of `tail.0`, from the last highlighting pass.
    tail: Option<(usize, State)>,
    cache: HashMap<usize, Vec<Span>>,
    /// Char ranges of each highlighted line that are strings or comments.
    literals: HashMap<usize, Vec<(usize, usize)>>,
    /// Theme generation the cached colors were computed with.
    theme_generation: u64,
    /// Brackets colored by how deeply they nest (`editor.bracketPairColorization`).
    rainbow: bool,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The current color theme as a syntect theme, rebuilt when the theme changes.
fn theme() -> Arc<Theme> {
    static CACHE: RwLock<Option<(u64, Arc<Theme>)>> = RwLock::new(None);
    let generation = crate::appearance::theme::generation();
    if let Some((g, t)) = CACHE.read().ok().and_then(|c| c.clone())
        && g == generation
    {
        return t;
    }
    let t = Arc::new(crate::appearance::theme::syntect_theme(&crate::appearance::theme::current()));
    if let Ok(mut c) = CACHE.write() {
        *c = Some((generation, t.clone()));
    }
    t
}

impl Highlighter {
    /// `enabled: false` (large-file mode) never highlights.
    pub fn new(extension: &str, enabled: bool) -> Self {
        let syntax_name = enabled
            .then(|| {
                syntaxes().find_syntax_by_extension(extension).or_else(|| syntaxes().find_syntax_by_token(extension))
            })
            .flatten()
            .filter(|s| s.name != "Plain Text")
            .map(|s| s.name.clone());
        Self {
            syntax_name,
            checkpoints: Vec::new(),
            tail: None,
            cache: HashMap::new(),
            literals: HashMap::new(),
            theme_generation: crate::appearance::theme::generation(),
            rainbow: false,
        }
    }

    /// Turns rainbow brackets on or off, highlighting again if that changes them.
    pub fn set_rainbow(&mut self, on: bool) {
        if self.rainbow != on {
            self.rainbow = on;
            self.checkpoints.clear();
            self.cache.clear();
            self.literals.clear();
            self.tail = None;
        }
    }

    #[cfg(test)]
    pub fn is_active(&self) -> bool {
        self.syntax_name.is_some()
    }

    /// Forgets everything computed for `line` and below.
    pub fn invalidate_from(&mut self, line: usize) {
        self.checkpoints.truncate(line / CHECKPOINT + 1);
        self.cache.retain(|&l, _| l < line);
        if self.tail.as_ref().is_some_and(|(next, _)| *next > line) {
            self.tail = None;
        }
    }

    /// Highlighted spans for `line` (char lengths, excluding the line ending).
    pub fn spans(&mut self, buf: &Buffer, line: usize) -> Option<&[Span]> {
        let name = self.syntax_name.as_ref()?;
        // Highlight states embed theme styles; start over after a theme change.
        if self.theme_generation != crate::appearance::theme::generation() {
            self.theme_generation = crate::appearance::theme::generation();
            self.checkpoints.clear();
            self.cache.clear();
            self.literals.clear();
            self.tail = None;
        }
        if !self.cache.contains_key(&line) {
            let set = syntaxes();
            let syntax = set.find_syntax_by_name(name)?;
            let theme = theme();
            let highlighter = SynHighlighter::new(&theme);
            if self.checkpoints.is_empty() {
                self.checkpoints.push((
                    ParseState::new(syntax),
                    HighlightState::new(&highlighter, ScopeStack::new()),
                    Vec::new(),
                ));
            }
            if self.cache.len() > MAX_CACHED_LINES {
                self.cache.clear();
                self.literals.clear();
                self.literals.clear();
            }

            // Resume from the nearest known state at or before `line`.
            let cp = (line / CHECKPOINT).min(self.checkpoints.len() - 1);
            let (mut next, mut state) = (cp * CHECKPOINT, self.checkpoints[cp].clone());
            if let Some((t, s)) = &self.tail
                && *t <= line
                && *t > next
            {
                (next, state) = (*t, s.clone());
            }

            while next <= line {
                if next.is_multiple_of(CHECKPOINT) && next / CHECKPOINT == self.checkpoints.len() {
                    self.checkpoints.push(state.clone());
                }
                let (spans, literals) = highlight_line(buf, next, &mut state, &highlighter, set, self.rainbow);
                self.cache.insert(next, spans);
                self.literals.insert(next, literals);
                next += 1;
            }
            self.tail = Some((next, state));
        }
        self.cache.get(&line).map(Vec::as_slice)
    }

    /// Whether char `col` of `line` is in a string or comment, highlighting the line if
    /// it hasn't been yet. `None` without a grammar.
    pub fn is_literal_at(&mut self, buf: &Buffer, line: usize, col: usize) -> Option<bool> {
        self.spans(buf, line)?;
        let ranges = self.literals.get(&line)?;
        Some(ranges.iter().any(|&(a, b)| (a..b).contains(&col)))
    }
}

fn highlight_line(
    buf: &Buffer,
    line: usize,
    state: &mut State,
    highlighter: &SynHighlighter,
    set: &SyntaxSet,
    rainbow: bool,
) -> (Vec<Span>, Vec<(usize, usize)>) {
    let mut text = buf.line_text(line);
    let plain_len = text.chars().count();
    let fallback =
        || (vec![Span { len: plain_len, color: Color::from_rgb(0xd0, 0xd0, 0xd0), italic: false }], Vec::new());
    if plain_len > MAX_LINE_CHARS {
        return fallback();
    }
    // The "newlines" syntax set expects each line to end with '\n'.
    text.push('\n');
    let (parse, hl, open) = state;
    let Ok(ops) = parse.parse_line(&text, set) else { return fallback() };
    let path = hl.path.clone();
    let spans: Vec<Span> = HighlightIterator::new(hl, &ops, &text, highlighter)
        .filter_map(|(style, piece)| {
            let len = piece.trim_end_matches('\n').chars().count();
            (len > 0).then(|| Span {
                len,
                color: Color::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b),
                italic: style.font_style.contains(FontStyle::ITALIC),
            })
        })
        .collect();
    let literals = literal_ranges(&text, &ops, path);
    if !rainbow {
        return (spans, literals);
    }
    let marks = bracket_colors(&text, &literals, open);
    (recolor(spans, &marks), literals)
}

/// The char ranges of `text` inside strings, comments and character literals, from the
/// scope stack `path` at its start and the line's scope changes `ops`.
fn literal_ranges(text: &str, ops: &[(usize, ScopeStackOp)], mut path: ScopeStack) -> Vec<(usize, usize)> {
    let literal = literal_scopes();
    let is_literal = |path: &ScopeStack| path.as_slice().iter().any(|s| literal.iter().any(|l| l.is_prefix_of(*s)));
    let mut ops = ops.iter().peekable();
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut inside = is_literal(&path);
    for (ci, (bi, _)) in text.char_indices().enumerate() {
        let mut changed = false;
        while let Some((at, op)) = ops.peek()
            && *at <= bi
        {
            let _ = path.apply(op);
            ops.next();
            changed = true;
        }
        if changed {
            inside = is_literal(&path);
        }
        if inside {
            match out.last_mut() {
                Some((_, end)) if *end == ci => *end = ci + 1,
                _ => out.push((ci, ci + 1)),
            }
        }
    }
    out
}

/// Scopes whose brackets are text, not code.
fn literal_scopes() -> &'static [Scope] {
    static SCOPES: OnceLock<Vec<Scope>> = OnceLock::new();
    SCOPES.get_or_init(|| {
        ["string", "comment", "constant.character", "constant.other.character", "markup.raw"]
            .iter()
            .filter_map(|s| Scope::new(s).ok())
            .collect()
    })
}

/// Each code bracket in `text` (char index) with its rainbow color: by how many brackets
/// are open around it, carried in `open` from line to line. Brackets in `literals` (char
/// ranges) are text. A closing bracket that closes nothing, or closes the wrong kind,
/// gets the unexpected-bracket color.
fn bracket_colors(text: &str, literals: &[(usize, usize)], open: &mut Open) -> Vec<(usize, Color)> {
    let palette = crate::appearance::theme::current().palette;
    let mut out = Vec::new();
    for (ci, c) in text.chars().enumerate() {
        if !matches!(c, '(' | ')' | '[' | ']' | '{' | '}') || literals.iter().any(|&(a, b)| (a..b).contains(&ci)) {
            continue;
        }
        let opener = match c {
            ')' => '(',
            ']' => '[',
            '}' => '{',
            _ => {
                out.push((ci, palette.rainbow[open.len() % palette.rainbow.len()]));
                if open.len() < MAX_OPEN {
                    open.push(c);
                }
                continue;
            }
        };
        if open.last() == Some(&opener) {
            open.pop();
            out.push((ci, palette.rainbow[open.len() % palette.rainbow.len()]));
        } else {
            out.push((ci, palette.bracket_unexpected));
        }
    }
    out
}

/// `spans` with the chars in `marks` (sorted char indices) given their own color.
fn recolor(spans: Vec<Span>, marks: &[(usize, Color)]) -> Vec<Span> {
    if marks.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + marks.len() * 2);
    let mut marks = marks.iter().peekable();
    let mut at = 0;
    for span in spans {
        let end = at + span.len;
        let mut from = at;
        while let Some(&&(i, color)) = marks.peek()
            && i < end
        {
            marks.next();
            if i < from {
                continue;
            }
            if i > from {
                out.push(Span { len: i - from, ..span.clone() });
            }
            out.push(Span { len: 1, color, italic: false });
            from = i + 1;
        }
        if end > from {
            out.push(Span { len: end - from, ..span });
        }
        at = end;
    }
    out
}

/// A code block's lines as highlighted spans, by its language's name or file extension.
/// `None` when no grammar knows the language.
pub fn snippet(language: &str, text: &str) -> Option<Vec<Vec<Span>>> {
    let mut highlighter = Highlighter::new(&language.to_lowercase(), true);
    highlighter.syntax_name.as_ref()?;
    let buf = Buffer::new(text);
    Some(
        (0..buf.len_lines())
            .map(|line| highlighter.spans(&buf, line).map(<[Span]>::to_vec).unwrap_or_default())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rainbow_brackets_without_a_tree() {
        // Ruby has no tree-sitter grammar here; brackets nest across lines, and those in
        // strings and comments are left alone.
        let buf = Buffer::new("a = [foo(1), {\n  b: \"(x\"\n}] # (c)\n)");
        let mut hl = Highlighter::new("rb", true);
        assert!(hl.is_active());
        hl.set_rainbow(true);
        let palette = crate::appearance::theme::current().palette;
        let at = |hl: &mut Highlighter, line: usize, col: usize| {
            let mut i = 0;
            for s in hl.spans(&buf, line).unwrap() {
                if col < i + s.len {
                    return s.color;
                }
                i += s.len;
            }
            panic!("no char {col} on line {line}");
        };
        let [r0, r1, r2] = palette.rainbow;
        // `[` depth 0, `(` and `)` depth 1, `{` depth 1.
        assert_eq!((at(&mut hl, 0, 4), at(&mut hl, 0, 8), at(&mut hl, 0, 10), at(&mut hl, 0, 13)), (r0, r1, r1, r1));
        assert_ne!(at(&mut hl, 1, 6), r2, "the ( in a string is text");
        // `}` closes the `{`, `]` the `[`; the comment's brackets are text.
        assert_eq!((at(&mut hl, 2, 0), at(&mut hl, 2, 1)), (r1, r0));
        assert_ne!(at(&mut hl, 2, 5), r0);
        // A `)` with nothing open.
        assert_eq!(at(&mut hl, 3, 0), palette.bracket_unexpected);
        // Off: plain again.
        hl.set_rainbow(false);
        assert_ne!(at(&mut hl, 3, 0), palette.bracket_unexpected);
    }

    #[test]
    fn block_comment_state_carries_across_lines_and_edits() {
        let mut buf = Buffer::new("let a = 1;\n/* start\nstill comment\n*/\nlet b = 2;");
        let mut hl = Highlighter::new("rs", true);
        assert!(hl.is_active());
        let comment_color = hl.spans(&buf, 2).unwrap()[0].color;
        let code_color = hl.spans(&buf, 0).unwrap()[0].color;
        assert_ne!(comment_color, code_color, "line 2 is inside the block comment");

        // Remove the comment opener: line 2 must be re-highlighted as code.
        buf.replace_all(
            "let a = 1;\n// start\nstill comment\n*/\nlet b = 2;",
            crate::editor::buffer::Selection::cursor(0),
        );
        hl.invalidate_from(buf.take_edited_from_line().unwrap());
        let after = hl.spans(&buf, 2).unwrap()[0].color;
        assert_ne!(after, comment_color);
    }

    #[test]
    fn spans_cover_the_whole_line() {
        let buf = Buffer::new("fn main() { println!(\"hi\"); }");
        let mut hl = Highlighter::new("rs", true);
        let total: usize = hl.spans(&buf, 0).unwrap().iter().map(|s| s.len).sum();
        assert_eq!(total, buf.line_text(0).chars().count());
    }

    #[test]
    fn unknown_or_disabled_language_is_inactive() {
        assert!(!Highlighter::new("zzz-unknown", true).is_active());
        assert!(!Highlighter::new("rs", false).is_active());
    }
}
