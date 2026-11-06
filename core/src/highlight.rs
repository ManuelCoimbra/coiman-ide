//! Incremental syntax highlighting with syntect.
//!
//! Parser state is checkpointed every [`CHECKPOINT`] lines, so an edit only
//! invalidates state from the edited line down, and only lines that are
//! actually drawn get highlighted.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::theme::Color;
use std::sync::{Arc, RwLock};

use syntect::highlighting::{FontStyle, HighlightIterator, HighlightState, Highlighter as SynHighlighter, Theme};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

use crate::buffer::Buffer;

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

type State = (ParseState, HighlightState);

pub struct Highlighter {
    syntax_name: Option<String>,
    /// `checkpoints[i]` is the state at the start of line `i * CHECKPOINT`.
    checkpoints: Vec<State>,
    /// State at the start of `tail.0`, from the last highlighting pass.
    tail: Option<(usize, State)>,
    cache: HashMap<usize, Vec<Span>>,
    /// Theme generation the cached colors were computed with.
    theme_generation: u64,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// The current color theme as a syntect theme, rebuilt when the theme changes.
fn theme() -> Arc<Theme> {
    static CACHE: RwLock<Option<(u64, Arc<Theme>)>> = RwLock::new(None);
    let generation = crate::theme::generation();
    if let Some((g, t)) = CACHE.read().ok().and_then(|c| c.clone())
        && g == generation
    {
        return t;
    }
    let t = Arc::new(crate::theme::syntect_theme(&crate::theme::current()));
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
            theme_generation: crate::theme::generation(),
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
        if self.theme_generation != crate::theme::generation() {
            self.theme_generation = crate::theme::generation();
            self.checkpoints.clear();
            self.cache.clear();
            self.tail = None;
        }
        if !self.cache.contains_key(&line) {
            let set = syntaxes();
            let syntax = set.find_syntax_by_name(name)?;
            let theme = theme();
            let highlighter = SynHighlighter::new(&theme);
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

fn highlight_line(
    buf: &Buffer,
    line: usize,
    state: &mut State,
    highlighter: &SynHighlighter,
    set: &SyntaxSet,
) -> Vec<Span> {
    let mut text = buf.line_text(line);
    let plain_len = text.chars().count();
    let fallback = || vec![Span { len: plain_len, color: Color::from_rgb(0xd0, 0xd0, 0xd0), italic: false }];
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
                color: Color::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b),
                italic: style.font_style.contains(FontStyle::ITALIC),
            })
        })
        .collect()
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
    fn block_comment_state_carries_across_lines_and_edits() {
        let mut buf = Buffer::new("let a = 1;\n/* start\nstill comment\n*/\nlet b = 2;");
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
