//! Tree-sitter syntax highlighting (built-in grammars and Zed Wasm grammars).
//!
//! Parsing runs on a background thread: the UI sends each buffer version with
//! the exact edits since the last one, and the worker reparses incrementally.
//! Highlight queries run on the UI thread, only over the visible lines, against
//! the latest tree (adjusted for edits it hasn't seen yet). Zed language
//! extensions contribute Wasm grammars (`grammars/*.wasm`) plus `highlights.scm`,
//! `injections.scm`, and `outline.scm` queries. Brackets get rainbow colors by
//! nesting depth in the syntax tree, like VS Code.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock, RwLock};

use crate::appearance::theme::Color;
use crate::extensions::zed::languages::LoadedLanguage;
use ropey::Rope;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Range, Tree, WasmStore};

use crate::editor::buffer::{Buffer, SyntaxEdit};
use crate::language::highlight::Span;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Lang {
    Rust,
    Json,
    Toml,
    Yaml,
    Markdown,
    Python,
    JavaScript,
    Jsx,
    TypeScript,
    Tsx,
    Java,
    C,
    Cpp,
    CSharp,
    Html,
    Css,
    Go,
    Bash,
}

const LANG_COUNT: usize = 18;

impl Lang {
    pub const ALL: [Lang; LANG_COUNT] = [
        Lang::Rust,
        Lang::Json,
        Lang::Toml,
        Lang::Yaml,
        Lang::Markdown,
        Lang::Python,
        Lang::JavaScript,
        Lang::Jsx,
        Lang::TypeScript,
        Lang::Tsx,
        Lang::Java,
        Lang::C,
        Lang::Cpp,
        Lang::CSharp,
        Lang::Html,
        Lang::Css,
        Lang::Go,
        Lang::Bash,
    ];

    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "rs" => Self::Rust,
            "json" | "jsonc" => Self::Json,
            "toml" => Self::Toml,
            "yaml" | "yml" => Self::Yaml,
            "md" | "markdown" => Self::Markdown,
            "py" | "pyw" | "pyi" => Self::Python,
            "js" | "mjs" | "cjs" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "java" => Self::Java,
            "c" | "h" => Self::C,
            "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "inl" => Self::Cpp,
            "cs" => Self::CSharp,
            "html" | "htm" | "xhtml" => Self::Html,
            "css" => Self::Css,
            "go" => Self::Go,
            "sh" | "bash" | "zsh" => Self::Bash,
            _ => return None,
        })
    }

    pub fn from_grammar_name(name: &str) -> Option<Self> {
        let lower = name.trim().to_ascii_lowercase();
        Some(match lower.as_str() {
            "rust" => Self::Rust,
            "json" => Self::Json,
            "toml" => Self::Toml,
            "yaml" => Self::Yaml,
            "markdown" | "md" => Self::Markdown,
            "python" | "py" => Self::Python,
            "javascript" | "js" => Self::JavaScript,
            "jsx" => Self::Jsx,
            "typescript" | "ts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "java" => Self::Java,
            "c" => Self::C,
            "cpp" | "c++" => Self::Cpp,
            "c_sharp" | "c-sharp" | "csharp" => Self::CSharp,
            "html" => Self::Html,
            "css" | "scss" => Self::Css,
            "go" => Self::Go,
            "bash" | "sh" | "zsh" => Self::Bash,
            _ => return None,
        })
    }

    pub fn language(self) -> Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Toml => tree_sitter_toml_ng::LANGUAGE.into(),
            Self::Yaml => tree_sitter_yaml::LANGUAGE.into(),
            Self::Markdown => tree_sitter_md::LANGUAGE.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::JavaScript | Self::Jsx => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
        }
    }

    /// The highlights query. Grammars that extend another (C++ on C,
    /// TypeScript on JavaScript) list their own patterns first, since the
    /// earlier pattern wins.
    fn highlights(self) -> Cow<'static, str> {
        use tree_sitter_javascript as js;
        let join = |parts: &[&str]| Cow::Owned(parts.join("\n"));
        match self {
            Self::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
            Self::Json => tree_sitter_json::HIGHLIGHTS_QUERY.into(),
            Self::Toml => tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
            Self::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
            Self::Markdown => tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(),
            Self::Python => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
            Self::JavaScript => js::HIGHLIGHT_QUERY.into(),
            Self::Jsx => join(&[js::JSX_HIGHLIGHT_QUERY, js::HIGHLIGHT_QUERY]),
            Self::TypeScript => join(&[tree_sitter_typescript::HIGHLIGHTS_QUERY, js::HIGHLIGHT_QUERY]),
            Self::Tsx => {
                join(&[tree_sitter_typescript::HIGHLIGHTS_QUERY, js::JSX_HIGHLIGHT_QUERY, js::HIGHLIGHT_QUERY])
            }
            Self::Java => tree_sitter_java::HIGHLIGHTS_QUERY.into(),
            Self::C => tree_sitter_c::HIGHLIGHT_QUERY.into(),
            Self::Cpp => join(&[tree_sitter_cpp::HIGHLIGHT_QUERY, tree_sitter_c::HIGHLIGHT_QUERY]),
            Self::CSharp => tree_sitter_c_sharp::HIGHLIGHTS_QUERY.into(),
            Self::Html => tree_sitter_html::HIGHLIGHTS_QUERY.into(),
            Self::Css => tree_sitter_css::HIGHLIGHTS_QUERY.into(),
            Self::Go => tree_sitter_go::HIGHLIGHTS_QUERY.into(),
            Self::Bash => tree_sitter_bash::HIGHLIGHT_QUERY.into(),
        }
    }

    /// The tags query (definitions of classes, functions…) for the outline,
    /// for grammars that ship one.
    pub fn tags(self) -> Option<Cow<'static, str>> {
        use tree_sitter_javascript as js;
        let join = |parts: &[&str]| Some(Cow::Owned(parts.join("\n")));
        match self {
            Self::Python => Some(tree_sitter_python::TAGS_QUERY.into()),
            Self::JavaScript | Self::Jsx => Some(js::TAGS_QUERY.into()),
            Self::TypeScript | Self::Tsx => join(&[tree_sitter_typescript::TAGS_QUERY, js::TAGS_QUERY]),
            Self::Java => Some(tree_sitter_java::TAGS_QUERY.into()),
            Self::C => Some(tree_sitter_c::TAGS_QUERY.into()),
            Self::Cpp => join(&[tree_sitter_cpp::TAGS_QUERY, tree_sitter_c::TAGS_QUERY]),
            Self::CSharp => Some(tree_sitter_c_sharp::TAGS_QUERY.into()),
            Self::Go => Some(tree_sitter_go::TAGS_QUERY.into()),
            _ => None,
        }
    }
}

/// Compiled Tree-sitter grammar and queries loaded from a Zed extension.
pub struct ZedSyntax {
    pub id: String,
    pub name: String,
    pub grammar: String,
    pub path_suffixes: Vec<String>,
    pub first_line_pattern: Option<String>,
    pub language: Language,
    pub query: Query,
    literal: Vec<bool>,
    pub injections: Option<Query>,
    pub outline: Option<Query>,
}

/// Language grammar used by [`TsState`]: either built-in or loaded from a Zed extension.
#[derive(Clone)]
pub enum SyntaxLang {
    Builtin(Lang),
    Zed(Arc<ZedSyntax>),
}

impl SyntaxLang {
    fn key(&self) -> String {
        match self {
            Self::Builtin(l) => format!("builtin:{l:?}"),
            Self::Zed(z) => format!("zed:{}", z.id),
        }
    }

    fn language(&self) -> Language {
        match self {
            Self::Builtin(l) => l.language(),
            Self::Zed(z) => z.language.clone(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Style {
    color: Color,
    italic: bool,
}

struct LangConfig {
    query: Query,
    /// Captures whose text is a string or comment: brackets inside stay uncolored.
    literal: Vec<bool>,
}

fn literal_flags(query: &Query) -> Vec<bool> {
    query.capture_names().iter().map(|n| n.starts_with("string") || n.starts_with("comment")).collect()
}

fn config(lang: Lang) -> Option<&'static LangConfig> {
    static CONFIGS: [OnceLock<Option<LangConfig>>; LANG_COUNT] = [const { OnceLock::new() }; LANG_COUNT];
    CONFIGS[lang as usize]
        .get_or_init(|| {
            let query = Query::new(&lang.language(), &lang.highlights()).ok()?;
            let literal = literal_flags(&query);
            Some(LangConfig { query, literal })
        })
        .as_ref()
}

// ----- Zed Wasm grammar registry --------------------------------------------------

static ZED_SYNTAXES: RwLock<Vec<Arc<ZedSyntax>>> = RwLock::new(Vec::new());
static ZED_SIG: RwLock<Vec<(String, String, Option<std::path::PathBuf>)>> = RwLock::new(Vec::new());
static ZED_GENERATION: AtomicU64 = AtomicU64::new(0);

fn wasm_engine() -> &'static tree_sitter::wasmtime::Engine {
    static ENGINE: OnceLock<tree_sitter::wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(tree_sitter::wasmtime::Engine::default)
}

fn make_parser(language: &Language) -> Option<Parser> {
    let mut parser = Parser::new();
    if language.is_wasm() {
        let store = WasmStore::new(wasm_engine()).ok()?;
        parser.set_wasm_store(store).ok()?;
    }
    parser.set_language(language).ok()?;
    Some(parser)
}

fn load_wasm_language(grammar: &str, wasm: &[u8]) -> Option<Language> {
    let mut store = WasmStore::new(wasm_engine()).ok()?;
    if let Ok(lang) = store.load_language(grammar, wasm) {
        return Some(lang);
    }
    let snake = grammar.replace('-', "_");
    if snake != grammar {
        return store.load_language(&snake, wasm).ok();
    }
    None
}

/// Generation counter bumped whenever active Zed grammars change.
pub fn zed_syntax_generation() -> u64 {
    ZED_GENERATION.load(Ordering::Relaxed)
}

/// Compiles and installs Tree-sitter grammars and queries from active Zed extensions.
pub fn sync_zed_languages(langs: &[LoadedLanguage]) {
    let sig: Vec<(String, String, Option<std::path::PathBuf>)> =
        langs.iter().map(|l| (l.extension_id.clone(), l.config.name.clone(), l.grammar_wasm.clone())).collect();
    if ZED_SIG.read().is_ok_and(|cur| *cur == sig) {
        return;
    }

    let mut compiled = Vec::new();
    for l in langs {
        let Some(hl_src) = l.highlights_scm.as_deref() else { continue };
        let grammar_name = l.config.grammar.as_deref().unwrap_or(&l.config.name);
        let language = if let Some(wasm_path) = &l.grammar_wasm {
            let Ok(bytes) = std::fs::read(wasm_path) else {
                crate::applog::info(&format!("Zed grammar read failed: {}", wasm_path.display()));
                continue;
            };
            match load_wasm_language(grammar_name, &bytes) {
                Some(lang) => lang,
                None => {
                    crate::applog::info(&format!(
                        "Zed grammar `{grammar_name}` failed to load from {}",
                        wasm_path.display()
                    ));
                    continue;
                }
            }
        } else if let Some(builtin) = Lang::from_grammar_name(grammar_name) {
            builtin.language()
        } else {
            continue;
        };

        let query = match Query::new(&language, hl_src) {
            Ok(q) => q,
            Err(e) => {
                crate::applog::info(&format!("Zed highlights.scm for `{}` failed: {e}", l.config.name));
                continue;
            }
        };
        let literal = literal_flags(&query);
        let injections = l.injections_scm.as_deref().and_then(|s| Query::new(&language, s).ok());
        let outline = l.outline_scm.as_deref().and_then(|s| Query::new(&language, s).ok());
        compiled.push(Arc::new(ZedSyntax {
            id: format!("{}::{}", l.extension_id, l.config.name),
            name: l.config.name.clone(),
            grammar: grammar_name.to_owned(),
            path_suffixes: l.config.path_suffixes.clone(),
            first_line_pattern: l.config.first_line_pattern.clone(),
            language,
            query,
            literal,
            injections,
            outline,
        }));
    }

    if let Ok(mut guard) = ZED_SYNTAXES.write() {
        *guard = compiled;
    }
    if let Ok(mut guard) = ZED_SIG.write() {
        *guard = sig;
    }
    ZED_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Finds a compiled Zed Tree-sitter grammar matching `path`, `ext`, or `first_line`.
pub fn zed_syntax_for_document(path: Option<&Path>, ext: &str, first_line: Option<&str>) -> Option<Arc<ZedSyntax>> {
    let list = ZED_SYNTAXES.read().ok()?;
    let file_name = path.and_then(|p| p.file_name()).and_then(|n| n.to_str()).unwrap_or_default();
    if let Some(found) = list.iter().find(|z| {
        z.path_suffixes.iter().any(|s| {
            (!file_name.is_empty() && s.eq_ignore_ascii_case(file_name))
                || (!ext.is_empty() && s.eq_ignore_ascii_case(ext))
        })
    }) {
        return Some(Arc::clone(found));
    }
    let line = first_line.map(str::trim_end).filter(|s| !s.is_empty())?;
    list.iter()
        .find(|z| {
            z.first_line_pattern
                .as_deref()
                .and_then(|pat| regex::Regex::new(pat).ok())
                .is_some_and(|re| re.is_match(line))
        })
        .cloned()
}

fn find_zed_by_name(lang_name: &str) -> Option<Arc<ZedSyntax>> {
    let list = ZED_SYNTAXES.read().ok()?;
    list.iter()
        .find(|z| {
            z.name.eq_ignore_ascii_case(lang_name)
                || z.grammar.eq_ignore_ascii_case(lang_name)
                || z.path_suffixes.iter().any(|s| s.eq_ignore_ascii_case(lang_name))
        })
        .cloned()
}

// ----- background parser ---------------------------------------------------------

enum Msg {
    Parse { uid: u64, version: u64, lang: SyntaxLang, rope: Rope, edits: Vec<SyntaxEdit> },
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
    /// Spawns the worker; `notify` is called whenever a parse finishes.
    pub fn spawn(notify: impl Fn() + Send + 'static) -> Self {
        let (tx, jobs) = channel();
        let (done, rx) = channel();
        std::thread::Builder::new()
            .name("syntax".to_owned())
            .spawn(move || worker_loop(&jobs, &done, &notify))
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

fn worker_loop(jobs: &Receiver<Msg>, done: &Sender<Parsed>, notify: &dyn Fn()) {
    let mut docs: HashMap<u64, (Parser, Option<Tree>, String)> = HashMap::new();
    while let Ok(first) = jobs.recv() {
        // Coalesce everything queued: per document, concatenate the edits and
        // keep only the newest text, so a slow parse never falls further behind.
        let mut batch: Vec<Msg> = vec![first];
        batch.extend(jobs.try_iter());
        let mut pending: Vec<(u64, u64, SyntaxLang, Rope, Vec<SyntaxEdit>)> = Vec::new();
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
            let key = lang.key();
            let entry = docs.entry(uid).or_insert_with(|| (Parser::new(), None, String::new()));
            let (parser, tree, current_key) = entry;
            if *current_key != key || parser.language().is_none() {
                let language = lang.language();
                if let Some(fresh) = make_parser(&language) {
                    *parser = fresh;
                    *current_key = key;
                    *tree = None;
                } else {
                    continue;
                }
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
            notify();
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
    pub syntax: SyntaxLang,
    /// Last buffer version sent to the worker.
    sent_version: Option<u64>,
    /// Edits sent to the worker, tagged with the version they produce, that
    /// `tree` doesn't include yet.
    inflight: Vec<(u64, Vec<SyntaxEdit>)>,
    /// Latest parsed tree and the buffer version it matches.
    tree: Option<(u64, Tree)>,
    /// (buffer version, tree version, first line, last line, theme generation,
    /// rainbow brackets) that `lines` was computed for.
    computed_for: Option<(u64, u64, usize, usize, u64, bool)>,
    /// Highlighted spans for the lines last prepared.
    pub lines: HashMap<usize, Vec<Span>>,
    /// The same for the minimap's lines, and what they were computed for.
    minimap_for: Option<(u64, u64, usize, usize, u64, bool)>,
    pub minimap_lines: HashMap<usize, Vec<Span>>,
}

impl TsState {
    /// Built-in [`Lang`] if this document uses one of the compiled grammars.
    pub fn builtin_lang(&self) -> Option<Lang> {
        match &self.syntax {
            SyntaxLang::Builtin(l) => Some(*l),
            SyntaxLang::Zed(_) => None,
        }
    }

    /// Zed `outline.scm` query if this document uses a Zed language extension.
    pub fn zed_outline(&self) -> Option<&Query> {
        match &self.syntax {
            SyntaxLang::Zed(z) => z.outline.as_ref(),
            SyntaxLang::Builtin(_) => None,
        }
    }

    /// Whether char `pos` is text rather than code: in a string, character literal or
    /// comment, but not in code interpolated into a string. `None` while the tree is
    /// behind the text, so positions in it can't be trusted.
    pub fn is_literal_at(&self, buf: &Buffer, pos: usize) -> Option<bool> {
        let (version, tree) = self.tree.as_ref()?;
        if *version != buf.version() || pos >= buf.len_chars() {
            return None;
        }
        let byte = buf.rope().char_to_byte(pos);
        let mut node = tree.root_node().descendant_for_byte_range(byte, byte + 1);
        while let Some(n) = node {
            let kind = n.kind();
            if kind.contains("interpolation") || kind.contains("substitution") {
                return Some(false);
            }
            if kind.contains("comment")
                || kind.contains("string")
                || kind.contains("char_literal")
                || kind.contains("character_literal")
                || kind == "rune_literal"
                || kind == "regex"
            {
                return Some(true);
            }
            node = n.parent();
        }
        Some(false)
    }
}

impl TsState {
    pub fn new(lang: Lang) -> Option<Self> {
        config(lang)?;
        Some(Self::from_syntax(SyntaxLang::Builtin(lang)))
    }

    pub fn new_zed(zed: Arc<ZedSyntax>) -> Self {
        Self::from_syntax(SyntaxLang::Zed(zed))
    }

    fn from_syntax(syntax: SyntaxLang) -> Self {
        Self {
            syntax,
            sent_version: None,
            inflight: Vec::new(),
            tree: None,
            computed_for: None,
            lines: HashMap::new(),
            minimap_for: None,
            minimap_lines: HashMap::new(),
        }
    }

    /// Sends the buffer to the worker if it changed since the last send.
    pub fn sync(&mut self, uid: u64, buf: &mut Buffer, worker: &SyntaxWorker) {
        if self.sent_version == Some(buf.version()) {
            return;
        }
        let edits = buf.take_syntax_edits();
        let version = buf.version();
        let msg = Msg::Parse { uid, version, lang: self.syntax.clone(), rope: buf.rope().clone(), edits: edits.clone() };
        if worker.tx.send(msg).is_ok() {
            self.inflight.push((version, edits));
            self.sent_version = Some(version);
        }
    }

    /// Parses on the calling thread instead of the worker (benchmarks, tools).
    /// Applies the buffer's pending edits so the reparse is incremental.
    pub fn parse_sync(&mut self, buf: &mut Buffer) {
        let mut tree = self.tree.take().map(|(_, t)| t);
        for edit in self.inflight.drain(..).flat_map(|(_, e)| e).chain(buf.take_syntax_edits()) {
            match (edit, tree.as_mut()) {
                (SyntaxEdit::Reset, _) => tree = None,
                (SyntaxEdit::Edit(e), Some(t)) => t.edit(&e),
                (SyntaxEdit::Edit(_), None) => {}
            }
        }
        let Some(mut parser) = make_parser(&self.syntax.language()) else { return };
        let rope = buf.rope();
        if let Some(t) = parser.parse_with_options(&mut |byte, _| chunk_at(rope, byte), tree.as_ref(), None) {
            self.tree = Some((buf.version(), t));
            self.sent_version = Some(buf.version());
        }
    }

    pub fn accept(&mut self, parsed: Parsed) {
        // Versions only increase within a session except through undo, so match
        // by position in the in-flight list rather than by comparing numbers.
        let Some(idx) = self.inflight.iter().position(|(v, _)| *v == parsed.version) else { return };
        self.inflight.drain(..=idx);
        self.tree = Some((parsed.version, parsed.tree));
    }

    /// Version of the latest parsed tree.
    pub fn tree_version(&self) -> Option<u64> {
        self.tree.as_ref().map(|(v, _)| *v)
    }

    /// The latest tree with edits it hasn't seen applied, so node positions
    /// match the buffer (new text may not be reflected in its structure yet).
    pub fn current_tree(&self, buf: &Buffer) -> Option<Tree> {
        let (_, tree) = self.tree.as_ref()?;
        let mut tree = tree.clone();
        for e in self.inflight.iter().flat_map(|(_, e)| e.iter()).chain(buf.peek_syntax_edits()) {
            match e {
                SyntaxEdit::Reset => return None,
                SyntaxEdit::Edit(e) => tree.edit(e),
            }
        }
        Some(tree)
    }

    /// A tree exactly matching the buffer, reparsed incrementally on this
    /// thread without disturbing the background worker.
    pub fn fresh_tree(&self, buf: &Buffer) -> Option<Tree> {
        let old = self.current_tree(buf);
        let mut parser = make_parser(&self.syntax.language())?;
        let rope = buf.rope();
        parser.parse_with_options(&mut |byte, _| chunk_at(rope, byte), old.as_ref(), None)
    }

    /// Computes highlighted spans for `first..=last` (char-based, per line).
    pub fn prepare(&mut self, buf: &Buffer, first: usize, last: usize, rainbow: bool) {
        let Some((tree_version, _)) = &self.tree else {
            self.lines.clear();
            return;
        };
        let key = (buf.version(), *tree_version, first, last, crate::appearance::theme::generation(), rainbow);
        if self.computed_for == Some(key) {
            return;
        }
        self.computed_for = Some(key);
        self.lines = self.compute(buf, first, last, rainbow).unwrap_or_default();
    }

    /// Spans for `first..=last` kept apart from the view's, for the minimap.
    pub fn prepare_minimap(&mut self, buf: &Buffer, first: usize, last: usize) {
        let Some((tree_version, _)) = &self.tree else {
            self.minimap_lines.clear();
            return;
        };
        let key = (buf.version(), *tree_version, first, last, crate::appearance::theme::generation(), false);
        if self.minimap_for == Some(key) {
            return;
        }
        self.minimap_for = Some(key);
        self.minimap_lines = self.compute(buf, first, last, false).unwrap_or_default();
    }

    fn compute(&self, buf: &Buffer, first: usize, last: usize, rainbow: bool) -> Option<HashMap<usize, Vec<Span>>> {
        let (_, tree) = self.tree.as_ref()?;
        let (query, literal, injections): (&Query, &[bool], Option<&Query>) = match &self.syntax {
            SyntaxLang::Builtin(lang) => {
                let cfg = config(*lang)?;
                (&cfg.query, &cfg.literal, None)
            }
            SyntaxLang::Zed(zed) => (&zed.query, &zed.literal, zed.injections.as_ref()),
        };
        let mut out = HashMap::new();

        // Bring the tree up to date with edits it hasn't seen, so unchanged
        // regions keep the right positions until the worker catches up.
        let pending: Vec<&SyntaxEdit> =
            self.inflight.iter().flat_map(|(_, e)| e.iter()).chain(buf.peek_syntax_edits()).collect();
        if pending.iter().any(|e| matches!(e, SyntaxEdit::Reset)) {
            return None;
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
        let spec = QuerySpec { query, literal, injections };
        let styles = highlight_bytes(spec, &tree, rope, (start, end), rainbow);

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
            out.insert(line, spans);
        }
        Some(out)
    }
}

#[derive(Clone, Copy)]
struct QuerySpec<'a> {
    query: &'a Query,
    literal: &'a [bool],
    injections: Option<&'a Query>,
}

struct HighlightOut<'a> {
    theme: &'a crate::appearance::theme::Theme,
    styles: &'a mut [Style],
    literal: &'a mut [bool],
}

/// Paints query captures from `tree` within `[start, end)` into `out`.
fn paint_query(query: &Query, literal_flags: &[bool], tree: &Tree, rope: &Rope, (start, end): (usize, usize), out: &mut HighlightOut<'_>) {
    let capture_styles: Vec<Option<Style>> = query
        .capture_names()
        .iter()
        .map(|n| crate::appearance::theme::capture_style(out.theme, n).map(|(color, italic)| Style { color, italic }))
        .collect();

    // Collect captures, then paint larger nodes first so inner nodes win;
    // for the same node the earlier pattern wins (tree-sitter convention).
    let mut caps: Vec<(usize, usize, usize, u32)> = Vec::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start..end);
    let text = |node: Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes);
    let mut matches = cursor.matches(query, tree.root_node(), text);
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
        if let Some(style) = capture_styles[idx as usize] {
            out.styles[s..e].fill(style);
        }
        if literal_flags[idx as usize] {
            out.literal[s..e].fill(true);
        }
    }
}

enum InjectedSpec {
    Builtin(Lang),
    Zed(Arc<ZedSyntax>),
}

fn find_injected_spec(name: &str) -> Option<InjectedSpec> {
    if name.eq_ignore_ascii_case("comment") {
        return None;
    }
    if let Some(lang) = Lang::from_grammar_name(name) {
        config(lang)?;
        return Some(InjectedSpec::Builtin(lang));
    }
    find_zed_by_name(name).map(InjectedSpec::Zed)
}

/// Paints embedded language regions matching `injections` (`injections.scm`) within `[start, end)`.
fn paint_injections(
    inj_query: &Query,
    tree: &Tree,
    rope: &Rope,
    (start, end): (usize, usize),
    out: &mut HighlightOut<'_>,
) {
    let names = inj_query.capture_names();
    let mut by_lang: HashMap<String, Vec<Range>> = HashMap::new();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(start..end);
    let text = |node: Node| rope.byte_slice(node.byte_range()).chunks().map(str::as_bytes);
    let mut matches = cursor.matches(inj_query, tree.root_node(), text);
    while let Some(m) = matches.next() {
        let mut lang_name: Option<String> = None;
        let mut ranges: Vec<Range> = Vec::new();
        for c in m.captures() {
            let cap = names[c.index as usize];
            if cap == "injection.language" || cap == "language" {
                lang_name = Some(rope.byte_slice(c.node.byte_range()).to_string().trim().to_owned());
            } else if cap == "injection.content" || cap == "content" {
                let r = c.node.range();
                if r.end_byte > r.start_byte {
                    ranges.push(r);
                }
            }
        }
        if lang_name.is_none() {
            for prop in inj_query.property_settings(m.pattern_index) {
                if matches!(prop.key.as_ref(), "language" | "injection.language")
                    && let Some(val) = prop.value.as_ref()
                {
                    lang_name = Some(val.as_ref().to_owned());
                    break;
                }
            }
        }
        if let Some(lang) = lang_name.filter(|s| !s.is_empty())
            && !ranges.is_empty()
        {
            by_lang.entry(lang).or_default().extend(ranges);
        }
    }

    for (lang_name, mut ranges) in by_lang {
        let Some(spec) = find_injected_spec(&lang_name) else { continue };
        ranges.sort_by_key(|r| (r.start_byte, r.end_byte));
        // Tree-sitter requires non-overlapping sorted ranges.
        let mut clean = Vec::with_capacity(ranges.len());
        let mut prev_end = 0usize;
        for r in ranges {
            if r.start_byte >= prev_end && r.end_byte > r.start_byte {
                prev_end = r.end_byte;
                clean.push(r);
            }
        }
        if clean.is_empty() {
            continue;
        }
        let language = match &spec {
            InjectedSpec::Builtin(l) => l.language(),
            InjectedSpec::Zed(z) => z.language.clone(),
        };
        let Some(mut parser) = make_parser(&language) else { continue };
        if parser.set_included_ranges(&clean).is_err() {
            continue;
        }
        let Some(sub_tree) = parser.parse_with_options(&mut |byte, _| chunk_at(rope, byte), None, None) else {
            continue;
        };
        match &spec {
            InjectedSpec::Builtin(l) => {
                if let Some(cfg) = config(*l) {
                    paint_query(&cfg.query, &cfg.literal, &sub_tree, rope, (start, end), out);
                }
            }
            InjectedSpec::Zed(z) => {
                paint_query(&z.query, &z.literal, &sub_tree, rope, (start, end), out);
            }
        }
    }
}

/// Style of every byte in `[start, end)`.
fn highlight_bytes(
    spec: QuerySpec<'_>,
    tree: &Tree,
    rope: &Rope,
    (start, end): (usize, usize),
    rainbow: bool,
) -> Vec<Style> {
    let theme = crate::appearance::theme::current();
    let len = end - start;
    let mut styles = vec![Style { color: theme.palette.editor_fg, italic: false }; len];
    let mut literal = vec![false; len];

    {
        let mut out = HighlightOut { theme: &theme, styles: &mut styles, literal: &mut literal };
        paint_query(spec.query, spec.literal, tree, rope, (start, end), &mut out);
        if let Some(inj) = spec.injections {
            paint_injections(inj, tree, rope, (start, end), &mut out);
        }
    }

    if rainbow {
        let palette = &theme.palette;
        paint_brackets(
            tree.root_node(),
            0,
            start,
            end,
            &mut styles,
            &literal,
            (palette.rainbow, palette.bracket_unexpected),
        );
    }
    styles
}

/// Rainbow brackets in one walk over the nodes overlapping `[start, end)`.
/// A bracket's depth is the number of enclosing nodes that open with a bracket.
fn paint_brackets(
    node: Node,
    depth: usize,
    start: usize,
    end: usize,
    styles: &mut [Style],
    literal: &[bool],
    (colors, unexpected): ([Color; 3], Color),
) {
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
            paint_brackets(child, depth, start, end, styles, literal, (colors, unexpected));
        } else if !child.is_named() && !child.is_missing() && matches!(child.kind(), "(" | ")" | "[" | "]" | "{" | "}")
        {
            let i = child.start_byte();
            if (start..end).contains(&i) && !literal[i - start] {
                // A closing bracket needs its opener before it among its siblings.
                let opener = match child.kind() {
                    ")" => Some("("),
                    "]" => Some("["),
                    "}" => Some("{"),
                    _ => None,
                };
                let stray = opener.is_some_and(|o| {
                    let mut prev = child.prev_sibling();
                    while let Some(p) = prev {
                        if p.kind() == o && !p.is_missing() {
                            return false;
                        }
                        prev = p.prev_sibling();
                    }
                    true
                });
                let color =
                    if stray { Some(unexpected) } else { (depth > 0).then(|| colors[(depth - 1) % colors.len()]) };
                if let Some(color) = color {
                    styles[i - start] = Style { color, italic: false };
                }
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

/// Interns a dynamic language display name for callers requiring `&'static str`.
pub fn intern_language_name(s: &str) -> &'static str {
    static INTERN: OnceLock<std::sync::Mutex<HashSet<&'static str>>> = OnceLock::new();
    let set = INTERN.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    let mut guard = set.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&existing) = guard.get(s) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    guard.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(lang: Lang, src: &str, line: usize) -> Vec<(String, Color)> {
        let mut buf = Buffer::new(src);
        let mut ts = TsState::new(lang).expect("query compiles");
        let mut parser = Parser::new();
        parser.set_language(&lang.language()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        buf.take_syntax_edits();
        ts.tree = Some((buf.version(), tree));
        ts.prepare(&buf, 0, buf.len_lines() - 1, true);
        let text: Vec<char> = buf.line_text(line).chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        for s in &ts.lines[&line] {
            out.push((text[i..i + s.len].iter().collect(), s.color));
            i += s.len;
        }
        out
    }

    /// A tree for `src`, as the worker would have parsed it.
    fn parsed(lang: Lang, src: &str) -> (Buffer, TsState) {
        let mut buf = Buffer::new(src);
        let mut ts = TsState::new(lang).expect("query compiles");
        let mut parser = Parser::new();
        parser.set_language(&lang.language()).unwrap();
        buf.take_syntax_edits();
        ts.tree = Some((buf.version(), parser.parse(src, None).unwrap()));
        (buf, ts)
    }

    #[test]
    fn brackets_in_strings_are_text() {
        let (buf, ts) = parsed(Lang::Rust, "fn a() { let s = \"}\"; b(s) }");
        assert_eq!(ts.is_literal_at(&buf, 18), Some(true), "the brace in the string");
        assert_eq!(ts.is_literal_at(&buf, 7), Some(false));
        let code = &mut |p| !ts.is_literal_at(&buf, p).unwrap_or(false);
        assert_eq!(buf.matching_bracket_where(7, 1000, code), Some((7, 27)));
        assert_eq!(buf.matching_bracket(7, 1000), Some((7, 18)), "text alone is fooled");
        assert_eq!(buf.enclosing_brackets_where(21, 1000, code), Some((7, 27)));
        // Code interpolated into a string is code; a comment's brackets are text.
        let (buf, ts) = parsed(Lang::JavaScript, "x = `a${(b)}`; // (");
        assert_eq!(ts.is_literal_at(&buf, 8), Some(false));
        assert_eq!(ts.is_literal_at(&buf, 5), Some(true));
        assert_eq!(ts.is_literal_at(&buf, 18), Some(true));
    }

    #[test]
    fn stray_closing_brackets_stand_out() {
        let palette = crate::appearance::theme::current().palette;
        let line = colors(Lang::Rust, "fn a() { b()); }", 0);
        let of = |s: &str| line.iter().filter(|(t, _)| t.contains(s)).map(|(_, c)| *c).collect::<Vec<_>>();
        // The second `)` after `b(` closes nothing.
        assert!(of(")").contains(&palette.bracket_unexpected), "{line:?}");
        let good = colors(Lang::Rust, "fn a() { b(); }", 0);
        assert!(good.iter().all(|(_, c)| *c != palette.bracket_unexpected), "{good:?}");
    }

    #[test]
    fn all_queries_compile() {
        for lang in Lang::ALL {
            if let Err(e) = Query::new(&lang.language(), &lang.highlights()) {
                panic!("{lang:?} highlights query: {e}");
            }
            if let Some(tags) = lang.tags()
                && let Err(e) = Query::new(&lang.language(), &tags)
            {
                panic!("{lang:?} tags query: {e}");
            }
        }
    }

    #[test]
    fn rust_keywords_functions_and_strings() {
        let spans = colors(Lang::Rust, "fn main() { let s = \"hi\"; }", 0);
        let color_of = |t: &str| spans.iter().find(|(s, _)| s.contains(t)).map(|(_, c)| *c);
        assert_eq!(color_of("fn"), Some(Color::from_rgb(0x56, 0x9c, 0xd6)));
        assert_eq!(color_of("main"), Some(Color::from_rgb(0xdc, 0xdc, 0xaa)));
        assert_eq!(color_of("\"hi\""), Some(Color::from_rgb(0xce, 0x91, 0x78)));
    }

    #[test]
    fn rainbow_brackets_by_depth_and_not_in_strings() {
        let src = "fn f() { g((1), \"(\"); }";
        // Expand spans to one color per char.
        let per_char: Vec<Color> =
            colors(Lang::Rust, src, 0).iter().flat_map(|(s, c)| std::iter::repeat_n(*c, s.chars().count())).collect();
        let at = |i: usize| per_char[i];
        let rainbow = crate::appearance::theme::current().palette.rainbow;
        // f( ) at 4,5 and { } at 7,22: depth 1. g( ) at 10,19: depth 2. ( ) at 11,13: depth 3.
        assert_eq!((at(4), at(5), at(7), at(22)), (rainbow[0], rainbow[0], rainbow[0], rainbow[0]));
        assert_eq!((at(10), at(19)), (rainbow[1], rainbow[1]));
        assert_eq!((at(11), at(13)), (rainbow[2], rainbow[2]));
        // The "(" inside the string literal keeps the string color.
        assert_eq!(at(17), Color::from_rgb(0xce, 0x91, 0x78));
    }

    #[test]
    fn json_keys_and_values_differ() {
        let spans = colors(Lang::Json, "{\"a\": 1}", 0);
        let key = spans.iter().find(|(s, _)| s.contains("\"a\"")).unwrap().1;
        let num = spans.iter().find(|(s, _)| s.contains('1')).unwrap().1;
        assert_eq!(key, Color::from_rgb(0x9c, 0xdc, 0xfe));
        assert_eq!(num, Color::from_rgb(0xb5, 0xce, 0xa8));
    }

    #[test]
    fn stale_tree_is_adjusted_for_unparsed_edits() {
        let src = "fn a() {}\nfn b() {}\n";
        let mut buf = Buffer::new(src);
        buf.track_syntax_edits();
        let mut ts = TsState::new(Lang::Rust).unwrap();
        let mut parser = Parser::new();
        parser.set_language(&Lang::Rust.language()).unwrap();
        buf.take_syntax_edits();
        ts.tree = Some((buf.version(), parser.parse(src, None).unwrap()));
        // Insert at the top; the tree is now stale but `prepare` applies the edit.
        buf.set_single(crate::editor::buffer::Selection::cursor(0));
        buf.insert("// x\n");
        ts.prepare(&buf, 0, buf.len_lines() - 1, true);
        let line2: usize = ts.lines[&2].iter().map(|s| s.len).sum();
        assert_eq!(line2, buf.line_text(2).chars().count());
        // "fn" on the shifted line is still a keyword.
        assert_eq!(ts.lines[&2][0].color, Color::from_rgb(0x56, 0x9c, 0xd6));
    }

    #[test]
    fn zed_wasm_grammar_highlights_and_injects_when_present() {
        let dir = crate::extensions::zed::installed_dir().map(|d| d.join("svelte"));
        let Some(ext_dir) = dir.filter(|d| d.join("grammars/svelte.wasm").is_file()) else {
            return;
        };
        let ext = crate::extensions::zed::registry::InstalledExtension::from_folder(&ext_dir, "test", true).unwrap();
        let langs = crate::extensions::zed::languages::collect_languages(&[ext]);
        sync_zed_languages(&langs);
        let zed = zed_syntax_for_document(Some(Path::new("App.svelte")), "svelte", None)
            .expect("svelte wasm grammar should load");

        let src = "<script>\n  const x = 1;\n</script>\n<style>\n  .btn { color: red; }\n</style>\n<Button label=\"hi\" />\n";
        let mut buf = Buffer::new(src);
        let mut ts = TsState::new_zed(zed);
        ts.parse_sync(&mut buf);
        ts.prepare(&buf, 0, buf.len_lines() - 1, true);
        // Line 1: `const` inside <script> should be highlighted as JS keyword, not default fg.
        let default_fg = crate::appearance::theme::current().palette.editor_fg;
        let line1 = &ts.lines[&1];
        assert!(line1.iter().any(|s| s.color != default_fg), "JS injection inside <script> should color tokens: {line1:?}");
        // Line 6: <Button /> should highlight tag/component.
        let line6 = &ts.lines[&6];
        assert!(line6.iter().any(|s| s.color != default_fg), "Svelte tag line should color tokens: {line6:?}");
        let syms = crate::editor::structure::zed_symbols(
            &ts.current_tree(&buf).unwrap(),
            src,
            ts.zed_outline().expect("svelte outline.scm"),
        );
        assert!(!syms.is_empty(), "Svelte outline should contain items");
    }
}
