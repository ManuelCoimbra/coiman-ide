//! Tree-sitter syntax highlighting.
//!
//! Parsing runs on a background thread: the UI sends each buffer version with
//! the exact edits since the last one, and the worker reparses incrementally.
//! Highlight queries run on the UI thread, only over the visible lines, against
//! the latest tree (adjusted for edits it hasn't seen yet). Brackets get rainbow
//! colors by nesting depth in the syntax tree, like VS Code.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::Color32;
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};

use crate::buffer::{Buffer, SyntaxEdit};
use crate::highlight::Span;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Rust,
    Json,
    Toml,
    Yaml,
    Markdown,
}

const LANG_COUNT: usize = 5;

impl Lang {
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "rs" => Self::Rust,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "md" | "markdown" => Self::Markdown,
            _ => return None,
        })
    }

    fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Self::Markdown => tree_sitter_md::LANGUAGE.into(),
        }
    }

    fn highlights(self) -> &'static str {
        match self {
            Self::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
            Self::Json => tree_sitter_json::HIGHLIGHTS_QUERY,
            Self::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            Self::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY,
            Self::Markdown => tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Style {
    color: Color32,
    italic: bool,
}

const DEFAULT_STYLE: Style = Style { color: Color32::from_rgb(0xd4, 0xd4, 0xd4), italic: false };

/// VS Code Dark+ colors for tree-sitter capture names. A capture uses the
/// longest matching dotted prefix (`function.method` falls back to `function`).
const CAPTURE_STYLES: &[(&str, Color32, bool)] = &[
    ("comment", Color32::from_rgb(0x6a, 0x99, 0x55), false),
    ("string", Color32::from_rgb(0xce, 0x91, 0x78), false),
    ("string.special.key", Color32::from_rgb(0x9c, 0xdc, 0xfe), false),
    ("string.escape", Color32::from_rgb(0xd7, 0xba, 0x7d), false),
    ("escape", Color32::from_rgb(0xd7, 0xba, 0x7d), false),
    ("character", Color32::from_rgb(0xce, 0x91, 0x78), false),
    ("number", Color32::from_rgb(0xb5, 0xce, 0xa8), false),
    ("boolean", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("keyword", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("function", Color32::from_rgb(0xdc, 0xdc, 0xaa), false),
    ("type", Color32::from_rgb(0x4e, 0xc9, 0xb0), false),
    ("constructor", Color32::from_rgb(0x4e, 0xc9, 0xb0), false),
    ("variable", Color32::from_rgb(0x9c, 0xdc, 0xfe), false),
    ("variable.builtin", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("property", Color32::from_rgb(0x9c, 0xdc, 0xfe), false),
    ("constant", Color32::from_rgb(0x4f, 0xc1, 0xff), false),
    ("constant.builtin", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("attribute", Color32::from_rgb(0xdc, 0xdc, 0xaa), false),
    ("label", Color32::from_rgb(0xc8, 0xc8, 0xc8), false),
    ("operator", Color32::from_rgb(0xd4, 0xd4, 0xd4), false),
    ("punctuation.special", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("text.title", Color32::from_rgb(0x56, 0x9c, 0xd6), false),
    ("text.literal", Color32::from_rgb(0xce, 0x91, 0x78), false),
    ("text.uri", Color32::from_rgb(0x37, 0x94, 0xff), false),
    ("text.reference", Color32::from_rgb(0x37, 0x94, 0xff), false),
];

/// Bracket colors by nesting depth (VS Code's default bracket pair colorization).
const RAINBOW: [Color32; 3] =
    [Color32::from_rgb(0xff, 0xd7, 0x00), Color32::from_rgb(0xda, 0x70, 0xd6), Color32::from_rgb(0x17, 0x9f, 0xff)];

fn style_for_capture(name: &str) -> Option<Style> {
    let mut name = name;
    loop {
        if let Some(&(_, color, italic)) = CAPTURE_STYLES.iter().find(|(n, _, _)| *n == name) {
            return Some(Style { color, italic });
        }
        name = &name[..name.rfind('.')?];
    }
}

struct LangConfig {
    query: Query,
    /// Style per capture index (`None`: leave the enclosing color).
    styles: Vec<Option<Style>>,
    /// Captures whose text is a string or comment: brackets inside stay uncolored.
    literal: Vec<bool>,
}

fn config(lang: Lang) -> Option<&'static LangConfig> {
    static CONFIGS: [OnceLock<Option<LangConfig>>; LANG_COUNT] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    CONFIGS[lang as usize]
        .get_or_init(|| {
            let query = Query::new(&lang.language(), lang.highlights()).ok()?;
            let names = query.capture_names();
            let styles = names.iter().map(|n| style_for_capture(n)).collect();
            let literal = names.iter().map(|n| n.starts_with("string") || n.starts_with("comment")).collect();
            Some(LangConfig { query, styles, literal })
        })
        .as_ref()
}

// ----- background parser ---------------------------------------------------------

enum Msg {
    Parse { uid: u64, version: u64, lang: Lang, rope: Rope, edits: Vec<SyntaxEdit> },
    Close(u64),
}

pub struct Parsed {
    pub uid: u64,
    version: u64,
    tree: Tree,
}

/// Handle to the background parsing thread.
pub struct SyntaxWorker {
    tx: Sender<Msg>,
    rx: Receiver<Parsed>,
}

impl SyntaxWorker {
    /// Spawns the worker; it requests a repaint of `ctx` whenever a parse finishes.
    pub fn spawn(ctx: egui::Context) -> Self {
        let (tx, jobs) = channel();
        let (done, rx) = channel();
        std::thread::Builder::new()
            .name("syntax".to_owned())
            .spawn(move || worker_loop(&jobs, &done, &ctx))
            .expect("failed to start syntax thread");
        Self { tx, rx }
    }

    pub fn try_recv(&self) -> Option<Parsed> {
        self.rx.try_recv().ok()
    }

    /// Drops the worker's parser state for a closed document.
    pub fn close(&self, uid: u64) {
        let _ = self.tx.send(Msg::Close(uid));
    }
}

fn worker_loop(jobs: &Receiver<Msg>, done: &Sender<Parsed>, ctx: &egui::Context) {
    let mut docs: HashMap<u64, (Parser, Option<Tree>, Lang)> = HashMap::new();
    while let Ok(first) = jobs.recv() {
        // Coalesce everything queued: per document, concatenate the edits and
        // keep only the newest text, so a slow parse never falls further behind.
        let mut batch: Vec<Msg> = vec![first];
        batch.extend(jobs.try_iter());
        let mut pending: Vec<(u64, u64, Lang, Rope, Vec<SyntaxEdit>)> = Vec::new();
        for msg in batch {
            match msg {
                Msg::Close(uid) => {
                    docs.remove(&uid);
                    pending.retain(|p| p.0 != uid);
                }
                Msg::Parse { uid, version, lang, rope, edits } => {
                    if let Some(p) = pending.iter_mut().find(|p| p.0 == uid) {
                        p.1 = version;
                        p.2 = lang;
                        p.3 = rope;
                        p.4.extend(edits);
                    } else {
                        pending.push((uid, version, lang, rope, edits));
                    }
                }
            }
        }

        for (uid, version, lang, rope, edits) in pending {
            let entry = docs.entry(uid).or_insert_with(|| (Parser::new(), None, lang));
            let (parser, tree, current_lang) = entry;
            if *current_lang != lang || parser.language().is_none() {
                if parser.set_language(&lang.language()).is_err() {
                    continue;
                }
                *current_lang = lang;
                *tree = None;
            }
            for edit in &edits {
                match edit {
                    SyntaxEdit::Reset => *tree = None,
                    SyntaxEdit::Edit(e) => {
                        if let Some(t) = tree.as_mut() {
                            t.edit(e);
                        }
                    }
                }
            }
            let parsed = parser.parse_with_options(&mut |byte, _| chunk_at(&rope, byte), tree.as_ref(), None);
            let Some(new_tree) = parsed else { continue };
            *tree = Some(new_tree.clone());
            if done.send(Parsed { uid, version, tree: new_tree }).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

/// The rope's text from `byte` to the end of its chunk.
fn chunk_at(rope: &Rope, byte: usize) -> &[u8] {
    if byte >= rope.len_bytes() {
        return &[];
    }
    let (chunk, chunk_start, _, _) = rope.chunk_at_byte(byte);
    &chunk.as_bytes()[byte - chunk_start..]
}

// ----- per-document state ----------------------------------------------------------

/// Tree-sitter state for one document, owned by the UI thread.
pub struct TsState {
    pub lang: Lang,
    /// Last buffer version sent to the worker.
    sent_version: Option<u64>,
    /// Edits sent to the worker, tagged with the version they produce, that
    /// `tree` doesn't include yet.
    inflight: Vec<(u64, Vec<SyntaxEdit>)>,
    /// Latest parsed tree and the buffer version it matches.
    tree: Option<(u64, Tree)>,
    /// (buffer version, tree version, first line, last line) of `lines`.
    computed_for: Option<(u64, u64, usize, usize)>,
    /// Highlighted spans for the lines last prepared.
    pub lines: HashMap<usize, Vec<Span>>,
}

impl TsState {
    pub fn new(lang: Lang) -> Option<Self> {
        config(lang)?;
        Some(Self { lang, sent_version: None, inflight: Vec::new(), tree: None, computed_for: None, lines: HashMap::new() })
    }

    /// Sends the buffer to the worker if it changed since the last send.
    pub fn sync(&mut self, uid: u64, buf: &mut Buffer, worker: &SyntaxWorker) {
        if self.sent_version == Some(buf.version()) {
            return;
        }
        let edits = buf.take_syntax_edits();
        let version = buf.version();
        let msg = Msg::Parse { uid, version, lang: self.lang, rope: buf.rope().clone(), edits: edits.clone() };
        if worker.tx.send(msg).is_ok() {
            self.inflight.push((version, edits));
            self.sent_version = Some(version);
        }
    }

    pub fn accept(&mut self, parsed: Parsed) {
        // Versions only increase within a session except through undo, so match
        // by position in the in-flight list rather than by comparing numbers.
        let Some(idx) = self.inflight.iter().position(|(v, _)| *v == parsed.version) else { return };
        self.inflight.drain(..=idx);
        self.tree = Some((parsed.version, parsed.tree));
    }

    /// Computes highlighted spans for `first..=last` (char-based, per line).
    pub fn prepare(&mut self, buf: &Buffer, first: usize, last: usize) {
        let Some((tree_version, tree)) = &self.tree else {
            self.lines.clear();
            return;
        };
        let key = (buf.version(), *tree_version, first, last);
        if self.computed_for == Some(key) {
            return;
        }
        self.computed_for = Some(key);
        self.lines.clear();
        let Some(cfg) = config(self.lang) else { return };

        // Bring the tree up to date with edits it hasn't seen, so unchanged
        // regions keep the right positions until the worker catches up.
        let pending: Vec<&SyntaxEdit> =
            self.inflight.iter().flat_map(|(_, e)| e.iter()).chain(buf.peek_syntax_edits()).collect();
        if pending.iter().any(|e| matches!(e, SyntaxEdit::Reset)) {
            return;
        }
        let tree: Cow<Tree> = if pending.is_empty() {
            Cow::Borrowed(tree)
        } else {
            let mut t = tree.clone();
            for e in pending {
                if let SyntaxEdit::Edit(e) = e {
                    t.edit(e);
                }
            }
            Cow::Owned(t)
        };

        let rope = buf.rope();
        let last = last.min(buf.len_lines() - 1);
        let start = rope.line_to_byte(first);
        let end = if last + 1 < rope.len_lines() { rope.line_to_byte(last + 1) } else { rope.len_bytes() };
        let styles = highlight_bytes(cfg, &tree, rope, start, end);

        for line in first..=last {
            let line_start = rope.line_to_byte(line);
            let mut spans: Vec<Span> = Vec::new();
            let mut byte = line_start;
            for c in rope.slice(buf.line_start(line)..buf.line_end(line)).chars() {
                let style = styles[byte - start];
                match spans.last_mut() {
                    Some(s) if s.color == style.color && s.italic == style.italic => s.len += 1,
                    _ => spans.push(Span { len: 1, color: style.color, italic: style.italic }),
                }
                byte += c.len_utf8();
            }
            self.lines.insert(line, spans);
        }
    }
}

/// Style of every byte in `[start, end)`.
fn highlight_bytes(cfg: &LangConfig, tree: &Tree, rope: &Rope, start: usize, end: usize) -> Vec<Style> {
    let len = end - start;
    let mut styles = vec![DEFAULT_STYLE; len];
    let mut literal = vec![false; len];

    // Collect captures, then paint larger nodes first so inner nodes win;
    // for the same node the earlier pattern wins (tree-sitter convention).
    let mut caps: Vec<(usize, usize, usize, u32)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start..end);
    let text = |node: Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes);
    let mut matches = cursor.matches(&cfg.query, tree.root_node(), text);
    while let Some(m) = matches.next() {
        for c in m.captures() {
            let r = c.node.byte_range();
            caps.push((r.start, r.end, m.pattern_index, c.index));
        }
    }
    caps.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)).then(b.2.cmp(&a.2)));
    for (s, e, _, idx) in caps {
        let (s, e) = (s.max(start) - start, e.min(end).saturating_sub(start));
        if s >= e {
            continue;
        }
        if let Some(style) = cfg.styles[idx as usize] {
            styles[s..e].fill(style);
        }
        if cfg.literal[idx as usize] {
            literal[s..e].fill(true);
        }
    }

    paint_brackets(tree.root_node(), 0, start, end, &mut styles, &literal);
    styles
}

/// Rainbow brackets in one walk over the nodes overlapping `[start, end)`.
/// A bracket's depth is the number of enclosing nodes that open with a bracket.
fn paint_brackets(node: Node, depth: usize, start: usize, end: usize, styles: &mut [Style], literal: &[bool]) {
    let opens = node.child(0).is_some_and(|c| matches!(c.kind(), "(" | "[" | "{"));
    let depth = depth + usize::from(opens);
    let mut cursor = node.walk();
    if cursor.goto_first_child_for_byte(start).is_none() {
        return;
    }
    loop {
        let child = cursor.node();
        if child.start_byte() >= end {
            break;
        }
        if child.child_count() > 0 {
            paint_brackets(child, depth, start, end, styles, literal);
        } else if depth > 0 && !child.is_named() && matches!(child.kind(), "(" | ")" | "[" | "]" | "{" | "}") {
            let i = child.start_byte();
            if (start..end).contains(&i) && !literal[i - start] {
                styles[i - start] = Style { color: RAINBOW[(depth - 1) % RAINBOW.len()], italic: false };
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(lang: Lang, src: &str, line: usize) -> Vec<(String, Color32)> {
        let mut buf = Buffer::from_str(src);
        let mut ts = TsState::new(lang).expect("query compiles");
        let mut parser = Parser::new();
        parser.set_language(&lang.language()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        buf.take_syntax_edits();
        ts.tree = Some((buf.version(), tree));
        ts.prepare(&buf, 0, buf.len_lines() - 1);
        let text: Vec<char> = buf.line_text(line).chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        for s in &ts.lines[&line] {
            out.push((text[i..i + s.len].iter().collect(), s.color));
            i += s.len;
        }
        out
    }

    #[test]
    fn all_queries_compile() {
        for lang in [Lang::Rust, Lang::Json, Lang::Toml, Lang::Yaml, Lang::Markdown] {
            assert!(config(lang).is_some(), "{lang:?} highlights query failed to compile");
        }
    }

    #[test]
    fn rust_keywords_functions_and_strings() {
        let spans = colors(Lang::Rust, "fn main() { let s = \"hi\"; }", 0);
        let color_of = |t: &str| spans.iter().find(|(s, _)| s.contains(t)).map(|(_, c)| *c);
        assert_eq!(color_of("fn"), Some(Color32::from_rgb(0x56, 0x9c, 0xd6)));
        assert_eq!(color_of("main"), Some(Color32::from_rgb(0xdc, 0xdc, 0xaa)));
        assert_eq!(color_of("\"hi\""), Some(Color32::from_rgb(0xce, 0x91, 0x78)));
    }

    #[test]
    fn rainbow_brackets_by_depth_and_not_in_strings() {
        let src = "fn f() { g((1), \"(\"); }";
        // Expand spans to one color per char.
        let per_char: Vec<Color32> =
            colors(Lang::Rust, src, 0).iter().flat_map(|(s, c)| std::iter::repeat_n(*c, s.chars().count())).collect();
        let at = |i: usize| per_char[i];
        // f( ) at 4,5 and { } at 7,22: depth 1. g( ) at 10,19: depth 2. ( ) at 11,13: depth 3.
        assert_eq!((at(4), at(5), at(7), at(22)), (RAINBOW[0], RAINBOW[0], RAINBOW[0], RAINBOW[0]));
        assert_eq!((at(10), at(19)), (RAINBOW[1], RAINBOW[1]));
        assert_eq!((at(11), at(13)), (RAINBOW[2], RAINBOW[2]));
        // The "(" inside the string literal keeps the string color.
        assert_eq!(at(17), Color32::from_rgb(0xce, 0x91, 0x78));
    }

    #[test]
    fn json_keys_and_values_differ() {
        let spans = colors(Lang::Json, "{\"a\": 1}", 0);
        let key = spans.iter().find(|(s, _)| s.contains("\"a\"")).unwrap().1;
        let num = spans.iter().find(|(s, _)| s.contains('1')).unwrap().1;
        assert_eq!(key, Color32::from_rgb(0x9c, 0xdc, 0xfe));
        assert_eq!(num, Color32::from_rgb(0xb5, 0xce, 0xa8));
    }

    #[test]
    fn stale_tree_is_adjusted_for_unparsed_edits() {
        let src = "fn a() {}\nfn b() {}\n";
        let mut buf = Buffer::from_str(src);
        buf.track_syntax_edits();
        let mut ts = TsState::new(Lang::Rust).unwrap();
        let mut parser = Parser::new();
        parser.set_language(&Lang::Rust.language()).unwrap();
        buf.take_syntax_edits();
        ts.tree = Some((buf.version(), parser.parse(src, None).unwrap()));
        // Insert at the top; the tree is now stale but `prepare` applies the edit.
        buf.set_single(crate::buffer::Selection::cursor(0));
        buf.insert("// x\n");
        ts.prepare(&buf, 0, buf.len_lines() - 1);
        let line2: usize = ts.lines[&2].iter().map(|s| s.len).sum();
        assert_eq!(line2, buf.line_text(2).chars().count());
        // "fn" on the shifted line is still a keyword.
        assert_eq!(ts.lines[&2][0].color, Color32::from_rgb(0x56, 0x9c, 0xd6));
    }
}
