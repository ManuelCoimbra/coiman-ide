//! Incremental syntax highlighting with syntect.
//!
//! Parser state is checkpointed every [`CHECKPOINT`] lines, so an edit only
//! invalidates state from the edited line down, and only lines that are
//! actually drawn get highlighted.

use std::collections::HashMap;
use std::sync::OnceLock;

use egui::Color32;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

use crate::buffer::Buffer;

const CHECKPOINT: usize = 32;
/// Lines longer than this (e.g. minified code) are drawn without highlighting.
const MAX_LINE_CHARS: usize = 5_000;
/// Cached highlighted lines are dropped past this count to bound memory.
const MAX_CACHED_LINES: usize = 20_000;

pub struct Span {
    pub len: usize,
    pub color: Color32,
    pub italic: bool,
}

type State = (ParseState, HighlightState);

pub struct Highlighter {
    syntax_name: Option<String>,
    /// `checkpoints[i]` is the state at the start of line `i * CHECKPOINT`.
    checkpoints: Vec<State>,
    /// State at the start of `tail.0`, from the last highlighting pass.
    tail: Option<(usize, State)>,
    cache: HashMap<usize, Vec<Span>>,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        set.themes.remove("base16-mocha.dark").unwrap_or_default()
    })
}

impl Highlighter {
    /// `enabled: false` (large-file mode) never highlights.
    pub fn new(extension: &str, enabled: bool) -> Self {
        let syntax_name = enabled
            .then(|| syntaxes().find_syntax_by_extension(extension).or_else(|| syntaxes().find_syntax_by_token(extension)))
            .flatten()
            .filter(|s| s.name != "Plain Text")
            .map(|s| s.name.clone());
        Self { syntax_name, checkpoints: Vec::new(), tail: None, cache: HashMap::new() }
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
        if !self.cache.contains_key(&line) {
            let set = syntaxes();
            let syntax = set.find_syntax_by_name(name)?;
            let highlighter = SynHighlighter::new(theme());
            if self.checkpoints.is_empty() {
                self.checkpoints.push((ParseState::new(syntax), HighlightState::new(&highlighter, ScopeStack::new())));
            }
            if self.cache.len() > MAX_CACHED_LINES {
                self.cache.clear();
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
                let spans = highlight_line(buf, next, &mut state, &highlighter, set);
                self.cache.insert(next, spans);
                next += 1;
            }
            self.tail = Some((next, state));
        }
        self.cache.get(&line).map(Vec::as_slice)
    }
}

fn highlight_line(buf: &Buffer, line: usize, state: &mut State, highlighter: &SynHighlighter, set: &SyntaxSet) -> Vec<Span> {
    let mut text = buf.line_text(line);
    let plain_len = text.chars().count();
    let fallback = || vec![Span { len: plain_len, color: Color32::from_rgb(0xd0, 0xd0, 0xd0), italic: false }];
    if plain_len > MAX_LINE_CHARS {
        return fallback();
    }
    // The "newlines" syntax set expects each line to end with '\n'.
    text.push('\n');
    let (parse, hl) = state;
    let Ok(ops) = parse.parse_line(&text, set) else { return fallback() };
    HighlightIterator::new(hl, &ops, &text, highlighter)
        .filter_map(|(style, piece)| {
            let len = piece.trim_end_matches('\n').chars().count();
            (len > 0).then(|| Span {
                len,
                color: Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b),
                italic: style.font_style.contains(FontStyle::ITALIC),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_comment_state_carries_across_lines_and_edits() {
        let mut buf = Buffer::from_str("let a = 1;\n/* start\nstill comment\n*/\nlet b = 2;");
        let mut hl = Highlighter::new("rs", true);
        assert!(hl.is_active());
        let comment_color = hl.spans(&buf, 2).unwrap()[0].color;
        let code_color = hl.spans(&buf, 0).unwrap()[0].color;
        assert_ne!(comment_color, code_color, "line 2 is inside the block comment");

        // Remove the comment opener: line 2 must be re-highlighted as code.
        buf.replace_all("let a = 1;\n// start\nstill comment\n*/\nlet b = 2;", crate::buffer::Selection::cursor(0));
        hl.invalidate_from(buf.take_edited_from_line().unwrap());
        let after = hl.spans(&buf, 2).unwrap()[0].color;
        assert_ne!(after, comment_color);
    }

    #[test]
    fn spans_cover_the_whole_line() {
        let buf = Buffer::from_str("fn main() { println!(\"hi\"); }");
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
