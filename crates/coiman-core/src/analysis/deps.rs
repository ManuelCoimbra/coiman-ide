//! A project's dependency graph: which source file imports which, read from the files' own
//! import lines, for Rust, Python, JavaScript and TypeScript, Java, C#, Go, and C and C++.
//! Only files of the project are nodes; libraries from outside it are left out.
//!
//! Nothing here runs a compiler or a language server: imports are read as text and resolved
//! against the files on disk, so a graph of a large project takes a moment, not a build.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// How many source files a graph takes at most.
pub const MAX_FILES: usize = 20_000;
/// Files bigger than this are read only for their first part, where imports are.
const MAX_READ: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    Python,
    Script,
    Java,
    CSharp,
    Go,
    C,
}

impl Lang {
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Self::Rust,
            "py" | "pyi" => Self::Python,
            "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte" => Self::Script,
            "java" => Self::Java,
            "cs" => Self::CSharp,
            "go" => Self::Go,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => Self::C,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Script => "JavaScript/TypeScript",
            Self::Java => "Java",
            Self::CSharp => "C#",
            Self::Go => "Go",
            Self::C => "C/C++",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Relative to the graph's root, with `/` between parts.
    pub path: String,
    pub lang: Lang,
    /// A test file, or (Rust) a file with tests in it.
    pub test: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    /// 1-based line of the first import that makes it.
    pub line: usize,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub root: PathBuf,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// More source files were there than [`MAX_FILES`].
    pub truncated: bool,
}

impl Graph {
    pub fn index_of(&self, path: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == path)
    }

    pub fn full_path(&self, node: usize) -> PathBuf {
        self.root.join(&self.nodes[node].path)
    }

    /// Each node's outgoing and incoming neighbours.
    pub fn adjacency(&self) -> (Vec<Vec<usize>>, Vec<Vec<usize>>) {
        let mut out = vec![Vec::new(); self.nodes.len()];
        let mut inc = vec![Vec::new(); self.nodes.len()];
        for e in &self.edges {
            out[e.from].push(e.to);
            inc[e.to].push(e.from);
        }
        (out, inc)
    }
}

/// Reads every source file under `root` (honouring `.gitignore`) and links each to the
/// project files it imports.
pub fn scan(root: &Path) -> Graph {
    let mut files = Vec::new();
    let mut truncated = false;
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(name.as_ref(), "target" | "node_modules" | "build" | "dist" | "out" | "bin" | "obj" | "venv")
                && !name.starts_with(".venv")
        })
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        if let Some(lang) = Lang::of(path) {
            if files.len() >= MAX_FILES {
                truncated = true;
                break;
            }
            files.push((path.to_path_buf(), lang));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let texts: Vec<String> = files.iter().map(|(p, _)| read_head(p)).collect();
    let mut graph = build(root, &files, &texts);
    graph.truncated = truncated;
    graph
}

fn read_head(path: &Path) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    let end = bytes.len().min(MAX_READ);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// The graph of these files, with their text: what [`scan`] does after reading the disk.
pub fn build(root: &Path, files: &[(PathBuf, Lang)], texts: &[String]) -> Graph {
    let index: HashMap<PathBuf, usize> = files.iter().enumerate().map(|(i, (p, _))| (normal(p), i)).collect();
    let nodes: Vec<Node> = files
        .iter()
        .zip(texts)
        .map(|((p, lang), text)| Node { path: rel(root, p), lang: *lang, test: is_test(p, *lang, text) })
        .collect();
    let ctx = Context::new(root, files, texts, &index);
    let mut seen = HashSet::new();
    let mut edges = Vec::new();
    for (from, ((path, lang), text)) in files.iter().zip(texts).enumerate() {
        let found = match lang {
            Lang::Rust => ctx.rust(path, text),
            Lang::Python => ctx.python(path, text),
            Lang::Script => ctx.script(path, text),
            Lang::Java => ctx.java(from, text),
            Lang::CSharp => ctx.csharp(from, text),
            Lang::Go => ctx.go(path, text),
            Lang::C => ctx.c(path, text),
        };
        for (to, line) in found {
            if to != from && seen.insert((from, to)) {
                edges.push(Edge { from, to, line });
            }
        }
    }
    Graph { root: root.to_path_buf(), nodes, edges, truncated: false }
}

/// A path as the index keeps it: `..` and `.` taken out, without resolving links.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn is_test(path: &Path, lang: Lang, text: &str) -> bool {
    let p = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let in_test_dir = p.split('/').any(|part| matches!(part, "test" | "tests" | "__tests__" | "spec" | "specs"));
    in_test_dir
        || match lang {
            Lang::Rust => text.contains("#[test]") || text.contains("#[tokio::test]"),
            Lang::Python => name.starts_with("test_") || name.ends_with("_test.py") || name == "conftest.py",
            Lang::Script => name.contains(".test.") || name.contains(".spec."),
            Lang::Java => name.ends_with("test.java") || name.ends_with("tests.java"),
            Lang::CSharp => name.ends_with("test.cs") || name.ends_with("tests.cs"),
            Lang::Go => name.ends_with("_test.go"),
            Lang::C => name.starts_with("test_") || name.ends_with("_test.c") || name.ends_with("_test.cpp"),
        }
}

/// Lines that are code: comment lines and block comments blanked, so imports in them don't count.
fn code_lines(text: &str, hash_comments: bool) -> impl Iterator<Item = (usize, &str)> {
    let mut in_block = false;
    text.lines().enumerate().map(move |(i, line)| {
        let t = line.trim_start();
        if in_block {
            if t.contains("*/") {
                in_block = false;
            }
            return (i + 1, "");
        }
        if t.starts_with("/*") && !t.contains("*/") {
            in_block = true;
            return (i + 1, "");
        }
        if t.starts_with("//") || (hash_comments && t.starts_with('#')) {
            return (i + 1, "");
        }
        (i + 1, line)
    })
}

static RUST_MOD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_]\w*)\s*;").expect("regex"));
static RUST_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(crate|super|self|[A-Za-z_]\w*)::").expect("regex"));
static PY_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*import\s+([\w., ]+)").expect("regex"));
static PY_FROM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*from\s+(\.*)([\w.]*)\s+import\s+\(?([\w, *]+)").expect("regex"));
static JS_IMPORT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*|^\s*import\s+)['"]([^'"]+)['"]"#).expect("regex")
});
static JAVA_PACKAGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*package\s+([\w.]+)\s*;").expect("regex"));
static JAVA_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*import\s+(?:static\s+)?([\w.]+?)(\.\*)?\s*;").expect("regex"));
static CS_NAMESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*namespace\s+([\w.]+)").expect("regex"));
static CS_USING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:global\s+)?using\s+(?:static\s+)?([\w.]+)\s*;").expect("regex"));
static TYPE_DECL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:class|interface|struct|enum|record)\s+([A-Z]\w*)").expect("regex"));
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Z]\w*\b").expect("regex"));
static GO_IMPORT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*(?:import\s+)?(?:[\w.]+\s+)?"([^"]+)""#).expect("regex"));
static C_INCLUDE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*#\s*include\s*([<"])([^>"]+)[>"]"#).expect("regex"));

struct Context<'a> {
    root: &'a Path,
    files: &'a [(PathBuf, Lang)],
    index: &'a HashMap<PathBuf, usize>,
    /// Rust crate names (with `_`) and their `src` folders, for imports between a workspace's crates.
    crates: HashMap<String, PathBuf>,
    /// Java: a class's full name to its file; a package to its files.
    java_classes: HashMap<String, usize>,
    java_packages: HashMap<String, Vec<usize>>,
    java_package_of: Vec<Option<String>>,
    /// C#: a type's name to the files and namespaces that declare it; each file's namespace.
    cs_types: HashMap<String, Vec<usize>>,
    cs_namespace_of: Vec<Option<String>>,
    /// Go: the module path in each go.mod, and its folder.
    go_modules: Vec<(String, PathBuf)>,
    go_dirs: HashMap<PathBuf, Vec<usize>>,
    texts: &'a [String],
}

impl<'a> Context<'a> {
    fn new(
        root: &'a Path,
        files: &'a [(PathBuf, Lang)],
        texts: &'a [String],
        index: &'a HashMap<PathBuf, usize>,
    ) -> Self {
        let mut crates = HashMap::new();
        let mut go_modules = Vec::new();
        let mut manifests = HashSet::new();
        for (path, lang) in files {
            let Some(mut dir) = path.parent() else { continue };
            loop {
                if *lang == Lang::Rust
                    && manifests.insert(dir.join("Cargo.toml"))
                    && let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml"))
                    && let Some(name) = package_name(&text)
                {
                    crates.insert(name.replace('-', "_"), dir.join("src"));
                }
                if *lang == Lang::Go
                    && manifests.insert(dir.join("go.mod"))
                    && let Ok(text) = std::fs::read_to_string(dir.join("go.mod"))
                    && let Some(line) = text.lines().find(|l| l.trim_start().starts_with("module "))
                {
                    let module = line.trim_start()["module ".len()..].trim().to_owned();
                    go_modules.push((module, dir.to_path_buf()));
                }
                if dir == root {
                    break;
                }
                match dir.parent() {
                    Some(up) if up.starts_with(root) => dir = up,
                    _ => break,
                }
            }
        }
        // Longest module paths first, so a nested module wins.
        go_modules.sort_by_key(|m| std::cmp::Reverse(m.0.len()));
        let mut java_classes = HashMap::new();
        let mut java_packages: HashMap<String, Vec<usize>> = HashMap::new();
        let mut java_package_of = vec![None; files.len()];
        let mut cs_types: HashMap<String, Vec<usize>> = HashMap::new();
        let mut cs_namespace_of = vec![None; files.len()];
        let mut go_dirs: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (i, ((path, lang), text)) in files.iter().zip(texts).enumerate() {
            match lang {
                Lang::Java => {
                    let package = JAVA_PACKAGE.captures(text).map(|c| c[1].to_owned()).unwrap_or_default();
                    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let full = if package.is_empty() { stem } else { format!("{package}.{stem}") };
                    java_classes.insert(full, i);
                    java_packages.entry(package.clone()).or_default().push(i);
                    java_package_of[i] = Some(package);
                }
                Lang::CSharp => {
                    cs_namespace_of[i] = CS_NAMESPACE.captures(text).map(|c| c[1].to_owned());
                    for c in TYPE_DECL.captures_iter(text) {
                        cs_types.entry(c[1].to_owned()).or_default().push(i);
                    }
                }
                Lang::Go => {
                    if let Some(dir) = path.parent() {
                        go_dirs.entry(normal(dir)).or_default().push(i);
                    }
                }
                _ => {}
            }
        }
        Self {
            root,
            files,
            index,
            crates,
            java_classes,
            java_packages,
            java_package_of,
            cs_types,
            cs_namespace_of,
            go_modules,
            go_dirs,
            texts,
        }
    }

    fn node(&self, path: &Path) -> Option<usize> {
        self.index.get(&normal(path)).copied()
    }

    // ---- Rust ----

    /// The crate a file is in: its `src` folder and the file that is the crate's root.
    fn rust_crate(&self, file: &Path) -> Option<(PathBuf, PathBuf)> {
        let mut dir = file.parent()?;
        loop {
            if dir.file_name().is_some_and(|n| n == "bin") && dir.parent().is_some_and(|p| p.ends_with("src")) {
                return Some((dir.parent()?.to_path_buf(), file.to_path_buf()));
            }
            if dir.join("Cargo.toml").exists() {
                let src = dir.join("src");
                let root_file = ["lib.rs", "main.rs"].iter().map(|f| src.join(f)).find(|f| f.exists())?;
                return Some((src, root_file));
            }
            if dir == self.root {
                return None;
            }
            dir = dir.parent()?;
        }
    }

    /// A file's module path inside its crate: `src/a/b.rs` is `[a, b]`.
    fn rust_module_of(src: &Path, crate_root: &Path, file: &Path) -> Vec<String> {
        if file == crate_root {
            return Vec::new();
        }
        let rel = file.strip_prefix(src).unwrap_or(file);
        let mut parts: Vec<String> = rel.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if let Some(last) = parts.pop() {
            let stem = last.trim_end_matches(".rs").to_owned();
            if stem != "mod" && stem != "lib" && stem != "main" {
                parts.push(stem);
            }
        }
        parts
    }

    /// The file of a module path, trying the longest prefix first (the rest are items in it).
    fn rust_resolve(&self, src: &Path, crate_root: &Path, path: &[String]) -> Option<usize> {
        self.rust_resolve_from(src, crate_root, path, 0)
    }

    /// [`Self::rust_resolve`], taking no prefix shorter than `min`.
    fn rust_resolve_from(&self, src: &Path, crate_root: &Path, path: &[String], min: usize) -> Option<usize> {
        for len in (min..=path.len()).rev() {
            if len == 0 {
                return self.node(crate_root);
            }
            let mut dir = src.to_path_buf();
            for part in &path[..len - 1] {
                dir.push(part);
            }
            let name = &path[len - 1];
            for candidate in [dir.join(format!("{name}.rs")), dir.join(name).join("mod.rs")] {
                if let Some(i) = self.node(&candidate) {
                    return Some(i);
                }
            }
        }
        None
    }

    fn rust(&self, file: &Path, text: &str) -> Vec<(usize, usize)> {
        let Some((src, crate_root)) = self.rust_crate(file) else { return Vec::new() };
        let here = Self::rust_module_of(&src, &crate_root, file);
        let mut out = Vec::new();
        // Inside `#[cfg(test)] mod tests`, `super` is the file itself.
        let tests_from =
            text.lines().position(|l| l.trim_start().starts_with("#[cfg(test)]")).map_or(usize::MAX, |i| i + 1);
        for (line, code) in code_lines(text, false) {
            // `mod x;` says a file contains another, not that it uses it: no edge.
            if RUST_MOD.is_match(code) {
                continue;
            }
            for start in RUST_PATH.find_iter(code) {
                // `a::b::c` is one path, found at `a`, not again at `b`.
                if code[..start.start()].ends_with("::") {
                    continue;
                }
                for path in rust_paths(&code[start.start()..]) {
                    let Some((head, rest)) = path.split_first() else { continue };
                    let (src, root, base): (&Path, &Path, Vec<String>) = match head.as_str() {
                        "crate" => (src.as_path(), crate_root.as_path(), Vec::new()),
                        "self" => (src.as_path(), crate_root.as_path(), here.clone()),
                        "super" => {
                            let mut base = here.clone();
                            if line <= tests_from {
                                base.pop();
                            }
                            let mut rest_iter = rest.iter().peekable();
                            while rest_iter.peek().is_some_and(|s| *s == "super") {
                                base.pop();
                                rest_iter.next();
                            }
                            let rest: Vec<String> = rest_iter.cloned().collect();
                            let full: Vec<String> = base.into_iter().chain(rest).collect();
                            if let Some(i) = self.rust_resolve(&src, &crate_root, &full) {
                                out.push((i, line));
                            }
                            continue;
                        }
                        name => match self.crates.get(name) {
                            Some(other_src) => {
                                let other_root =
                                    ["lib.rs", "main.rs"].iter().map(|f| other_src.join(f)).find(|f| f.exists());
                                let Some(other_root) = other_root else { continue };
                                if let Some(i) = self.rust_resolve(other_src, &other_root, rest) {
                                    out.push((i, line));
                                }
                                continue;
                            }
                            // A child module named without `crate::`: `use files::x` in a
                            // module that has `mod files;`. Only if that child is a file.
                            None => {
                                let full: Vec<String> = here.iter().cloned().chain(path.iter().cloned()).collect();
                                if let Some(i) = self.rust_resolve_from(&src, &crate_root, &full, here.len() + 1) {
                                    out.push((i, line));
                                }
                                continue;
                            }
                        },
                    };
                    let full: Vec<String> = base.into_iter().chain(rest.iter().cloned()).collect();
                    if let Some(i) = self.rust_resolve(src, root, &full) {
                        out.push((i, line));
                    }
                }
            }
        }
        out
    }

    // ---- Python ----

    fn python(&self, file: &Path, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let dir = file.parent().unwrap_or(self.root);
        // Where absolute imports start: the project, `src`, the file's folder, and the folder
        // above its outermost package (a package imports itself by its own name).
        let mut top = dir;
        while top.join("__init__.py").exists()
            && let Some(up) = top.parent()
        {
            top = up;
        }
        let bases = [self.root.to_path_buf(), self.root.join("src"), dir.to_path_buf(), top.to_path_buf()];
        for (line, code) in code_lines(text, true) {
            if let Some(c) = PY_FROM.captures(code) {
                let dots = c[1].len();
                let module: Vec<&str> = c[2].split('.').filter(|s| !s.is_empty()).collect();
                let names: Vec<&str> = c[3].split(',').map(|s| s.split_whitespace().next().unwrap_or("")).collect();
                let starts: Vec<PathBuf> = if dots > 0 {
                    let mut base = dir.to_path_buf();
                    for _ in 1..dots {
                        base.pop();
                    }
                    vec![base]
                } else {
                    bases.to_vec()
                };
                for base in &starts {
                    let mut found = false;
                    // `from a import b` may name a module `b` in package `a`.
                    for name in names.iter().filter(|n| !n.is_empty() && **n != "*") {
                        let parts: Vec<&str> = module.iter().copied().chain([*name]).collect();
                        if let Some(i) = self.python_module(base, &parts, false) {
                            out.push((i, line));
                            found = true;
                        }
                    }
                    if !found && let Some(i) = self.python_module(base, &module, true) {
                        out.push((i, line));
                        found = true;
                    }
                    if found {
                        break;
                    }
                }
            } else if let Some(c) = PY_IMPORT.captures(code) {
                for item in c[1].split(',') {
                    let name = item.split_whitespace().next().unwrap_or("");
                    let parts: Vec<&str> = name.split('.').filter(|s| !s.is_empty()).collect();
                    if let Some(i) = bases.iter().find_map(|b| self.python_module(b, &parts, true)) {
                        out.push((i, line));
                    }
                }
            }
        }
        out
    }

    /// `a.b.c` from `base`: `a/b/c.py` or `a/b/c/__init__.py`, or with `shorter`, the
    /// longest prefix that is a file.
    fn python_module(&self, base: &Path, parts: &[&str], shorter: bool) -> Option<usize> {
        let lens: Vec<usize> = if shorter { (1..=parts.len()).rev().collect() } else { vec![parts.len()] };
        for len in lens {
            if len == 0 {
                continue;
            }
            let mut p = base.to_path_buf();
            for part in &parts[..len] {
                p.push(part);
            }
            let as_file = p.with_extension("py");
            if let Some(i) = self.node(&as_file).or_else(|| self.node(&p.join("__init__.py"))) {
                return Some(i);
            }
        }
        None
    }

    // ---- JavaScript and TypeScript ----

    fn script(&self, file: &Path, text: &str) -> Vec<(usize, usize)> {
        const EXTS: [&str; 10] = ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "vue", "svelte"];
        let mut out = Vec::new();
        let dir = file.parent().unwrap_or(self.root);
        for (line, code) in code_lines(text, false) {
            for c in JS_IMPORT.captures_iter(code) {
                let spec = &c[1];
                let base = if let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) {
                    self.root.join("src").join(rest)
                } else if spec.starts_with('.') {
                    dir.join(spec)
                } else {
                    continue;
                };
                let mut candidates = vec![base.clone()];
                // `./x.js` in TypeScript means `./x.ts`.
                let stem = base.with_extension("");
                candidates.extend(EXTS.iter().map(|e| PathBuf::from(format!("{}.{e}", base.display()))));
                candidates.extend(EXTS.iter().map(|e| stem.with_extension(e)));
                candidates.extend(EXTS.iter().map(|e| base.join(format!("index.{e}"))));
                if let Some(i) = candidates.iter().find_map(|p| self.node(p)) {
                    out.push((i, line));
                }
            }
        }
        out
    }

    // ---- Java ----

    fn java(&self, from: usize, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (line, code) in code_lines(text, false) {
            if let Some(c) = JAVA_IMPORT.captures(code) {
                let name = &c[1];
                if c.get(2).is_some() {
                    for &i in self.java_packages.get(name).into_iter().flatten() {
                        out.push((i, line));
                    }
                    // `import a.B.*` brings in B's nested members.
                    if let Some(&i) = self.java_classes.get(name) {
                        out.push((i, line));
                    }
                    continue;
                }
                // `import static a.B.m` or a nested class: drop parts until a class is found.
                let mut name = name;
                loop {
                    if let Some(&i) = self.java_classes.get(name) {
                        out.push((i, line));
                        break;
                    }
                    match name.rfind('.') {
                        Some(dot) => name = &name[..dot],
                        None => break,
                    }
                }
            }
        }
        // Classes of the same package are used without an import.
        if let Some(package) = &self.java_package_of[from] {
            let words: HashSet<&str> = WORD.find_iter(text).map(|m| m.as_str()).collect();
            for &i in self.java_packages.get(package).into_iter().flatten() {
                let stem = self.files[i].0.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if i != from && words.contains(stem.as_str()) {
                    let line = first_line_with(text, &stem);
                    out.push((i, line));
                }
            }
        }
        out
    }

    // ---- C# ----

    fn csharp(&self, from: usize, text: &str) -> Vec<(usize, usize)> {
        let mut namespaces: HashSet<String> =
            code_lines(text, false).filter_map(|(_, c)| CS_USING.captures(c).map(|c| c[1].to_owned())).collect();
        if let Some(own) = &self.cs_namespace_of[from] {
            // A namespace sees its parents' types too.
            let mut ns = own.as_str();
            loop {
                namespaces.insert(ns.to_owned());
                match ns.rfind('.') {
                    Some(dot) => ns = &ns[..dot],
                    None => break,
                }
            }
        }
        let own_types: HashSet<String> = TYPE_DECL.captures_iter(text).map(|c| c[1].to_owned()).collect();
        let mut out = Vec::new();
        let words: HashSet<&str> = WORD.find_iter(text).map(|m| m.as_str()).collect();
        for word in words {
            if own_types.contains(word) {
                continue;
            }
            for &i in self.cs_types.get(word).into_iter().flatten() {
                let visible = self.cs_namespace_of[i].as_ref().is_none_or(|ns| namespaces.contains(ns));
                if i != from && visible {
                    out.push((i, first_line_with(self.texts[from].as_str(), word)));
                }
            }
        }
        out
    }

    // ---- Go ----

    fn go(&self, _file: &Path, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut in_block = false;
        for (line, code) in code_lines(text, false) {
            let t = code.trim();
            if t.starts_with("import (") {
                in_block = true;
                continue;
            }
            if in_block && t.starts_with(')') {
                in_block = false;
                continue;
            }
            if !(in_block || t.starts_with("import ")) {
                continue;
            }
            let Some(c) = GO_IMPORT.captures(code) else { continue };
            let spec = &c[1];
            for (module, dir) in &self.go_modules {
                let Some(rest) = spec.strip_prefix(module.as_str()) else { continue };
                let target = normal(&dir.join(rest.trim_start_matches('/')));
                for &i in self.go_dirs.get(&target).into_iter().flatten() {
                    if !self.files[i].0.to_string_lossy().ends_with("_test.go") {
                        out.push((i, line));
                    }
                }
                break;
            }
        }
        out
    }

    // ---- C and C++ ----

    fn c(&self, file: &Path, text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let dir = file.parent().unwrap_or(self.root);
        for (line, code) in text.lines().enumerate().map(|(i, l)| (i + 1, l)) {
            let Some(c) = C_INCLUDE.captures(code) else { continue };
            let quoted = &c[1] == "\"";
            let name = &c[2];
            let mut places = Vec::new();
            if quoted {
                places.push(dir.join(name));
            }
            places.extend([
                self.root.join(name),
                self.root.join("include").join(name),
                self.root.join("src").join(name),
            ]);
            if let Some(i) = places.iter().find_map(|p| self.node(p)) {
                out.push((i, line));
            }
        }
        out
    }
}

fn package_name(manifest: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(manifest).ok()?;
    value.get("package")?.get("name")?.as_str().map(str::to_owned)
}

fn first_line_with(text: &str, word: &str) -> usize {
    text.lines().position(|l| l.contains(word)).map_or(1, |i| i + 1)
}

/// The paths a Rust path expression names, from its start: `crate::a::{b, c::d}` gives
/// `[crate, a, b]` and `[crate, a, c, d]`.
fn rust_paths(text: &str) -> Vec<Vec<String>> {
    fn parse(chars: &[char], at: &mut usize, prefix: &[String], out: &mut Vec<Vec<String>>) {
        let mut path = prefix.to_vec();
        loop {
            while *at < chars.len() && chars[*at].is_whitespace() {
                *at += 1;
            }
            if *at < chars.len() && chars[*at] == '{' {
                *at += 1;
                loop {
                    parse(chars, at, &path, out);
                    while *at < chars.len() && chars[*at].is_whitespace() {
                        *at += 1;
                    }
                    match chars.get(*at) {
                        Some(',') => *at += 1,
                        Some('}') => {
                            *at += 1;
                            return;
                        }
                        _ => return,
                    }
                }
            }
            let start = *at;
            while *at < chars.len() && (chars[*at].is_alphanumeric() || chars[*at] == '_') {
                *at += 1;
            }
            if *at == start {
                if path.len() > prefix.len() {
                    out.push(path);
                }
                return;
            }
            let word: String = chars[start..*at].iter().collect();
            if word != "self" || path.is_empty() {
                path.push(word);
            }
            if chars.get(*at) == Some(&':') && chars.get(*at + 1) == Some(&':') {
                *at += 2;
            } else {
                out.push(path);
                return;
            }
        }
    }
    let chars: Vec<char> = text.chars().take(2000).collect();
    let mut out = Vec::new();
    parse(&chars, &mut 0, &[], &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(files: &[(&str, &str)]) -> (tempdir::Dir, Graph) {
        let dir = tempdir::Dir::new("deps");
        for (path, text) in files {
            let p = dir.0.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, text).unwrap();
        }
        let g = scan(&dir.0);
        (dir, g)
    }

    fn has(g: &Graph, from: &str, to: &str) -> bool {
        let (Some(a), Some(b)) = (g.index_of(from), g.index_of(to)) else { return false };
        g.edges.iter().any(|e| e.from == a && e.to == b)
    }

    mod tempdir {
        pub struct Dir(pub std::path::PathBuf);
        impl Dir {
            pub fn new(name: &str) -> Self {
                static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let p = std::env::temp_dir().join(format!("coiman-{name}-{}-{n}", std::process::id()));
                let _ = std::fs::remove_dir_all(&p);
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn rust_paths_expand_groups() {
        let got = rust_paths("crate::a::{b, c::{d, self}, e as f};");
        let want: Vec<Vec<&str>> = vec![
            vec!["crate", "a", "b"],
            vec!["crate", "a", "c", "d"],
            vec!["crate", "a", "c"],
            vec!["crate", "a", "e"],
        ];
        assert_eq!(got, want.iter().map(|p| p.iter().map(|s| s.to_string()).collect::<Vec<_>>()).collect::<Vec<_>>());
    }

    #[test]
    fn rust_mods_uses_and_other_crates() {
        let (_d, g) = graph(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"core\", \"app\"]\n"),
            ("core/Cargo.toml", "[package]\nname = \"my-core\"\n"),
            ("core/src/lib.rs", "pub mod buffer;\npub mod edit;\n"),
            ("core/src/buffer.rs", "pub struct Buffer;\n"),
            ("core/src/edit/mod.rs", "mod ops;\nuse crate::buffer::Buffer;\n"),
            ("core/src/edit/ops.rs", "use super::super::buffer;\n// use crate::nothing;\n#[test]\nfn t() {}\n"),
            ("app/Cargo.toml", "[package]\nname = \"app\"\n"),
            (
                "app/src/main.rs",
                "mod ui;\nuse my_core::{buffer::Buffer, edit};\nuse std::fs;\nfn main() { ui::draw(); }\n",
            ),
            ("app/src/ui.rs", "pub fn draw() {}\n"),
        ]);
        assert!(!has(&g, "core/src/lib.rs", "core/src/buffer.rs"), "mod declarations aren't uses");
        assert!(has(&g, "core/src/edit/mod.rs", "core/src/buffer.rs"));
        assert!(has(&g, "core/src/edit/ops.rs", "core/src/buffer.rs"));
        assert!(has(&g, "app/src/main.rs", "core/src/buffer.rs"));
        assert!(has(&g, "app/src/main.rs", "core/src/edit/mod.rs"));
        assert!(g.nodes[g.index_of("core/src/edit/ops.rs").unwrap()].test);
        assert!(has(&g, "app/src/main.rs", "app/src/ui.rs"), "a child module named without crate::");
        assert_eq!(g.edges.len(), 5, "{:?}", g.edges);
    }

    #[test]
    fn python_absolute_relative_and_from() {
        let (_d, g) = graph(&[
            ("app/__init__.py", ""),
            ("app/models.py", "class User: pass\n"),
            ("app/views.py", "from .models import User\nfrom . import util\nimport os\n"),
            ("app/util.py", "import app.models\n# import app.views\n"),
            ("tests/test_views.py", "from app.views import *\nfrom app import util\n"),
        ]);
        assert!(has(&g, "app/views.py", "app/models.py"));
        assert!(has(&g, "app/views.py", "app/util.py"));
        assert!(has(&g, "app/util.py", "app/models.py"));
        assert!(!has(&g, "app/util.py", "app/views.py"));
        assert!(has(&g, "tests/test_views.py", "app/views.py"));
        assert!(has(&g, "tests/test_views.py", "app/util.py"));
        assert!(g.nodes[g.index_of("tests/test_views.py").unwrap()].test);
    }

    #[test]
    fn scripts_relative_index_and_aliases() {
        let (_d, g) = graph(&[
            ("src/main.ts", "import { a } from './lib/a.js';\nimport b from \"./lib\";\nimport 'react';\n"),
            ("src/lib/a.ts", "export const a = require('../util');\n"),
            ("src/lib/index.ts", "export * from './a';\n"),
            ("src/util.js", "const x = import('@/lib/a');\n"),
            ("src/main.test.ts", "import { a } from './lib/a';\n"),
        ]);
        assert!(has(&g, "src/main.ts", "src/lib/a.ts"));
        assert!(has(&g, "src/main.ts", "src/lib/index.ts"));
        assert!(has(&g, "src/lib/a.ts", "src/util.js"));
        assert!(has(&g, "src/lib/index.ts", "src/lib/a.ts"));
        assert!(has(&g, "src/util.js", "src/lib/a.ts"));
        assert!(g.nodes[g.index_of("src/main.test.ts").unwrap()].test);
    }

    #[test]
    fn java_imports_wildcards_and_same_package() {
        let (_d, g) = graph(&[
            ("src/shop/Item.java", "package shop;\npublic class Item {}\n"),
            ("src/shop/Cart.java", "package shop;\nimport java.util.List;\npublic class Cart { List<Item> items; }\n"),
            ("src/app/Main.java", "package app;\nimport shop.*;\nimport static shop.Cart.empty;\nclass Main {}\n"),
        ]);
        assert!(has(&g, "src/shop/Cart.java", "src/shop/Item.java"));
        assert!(!has(&g, "src/shop/Item.java", "src/shop/Cart.java"));
        assert!(has(&g, "src/app/Main.java", "src/shop/Item.java"));
        assert!(has(&g, "src/app/Main.java", "src/shop/Cart.java"));
    }

    #[test]
    fn csharp_types_through_usings() {
        let (_d, g) = graph(&[
            ("Models/Item.cs", "namespace Shop.Models;\npublic class Item {}\n"),
            ("Services/Cart.cs", "using Shop.Models;\nnamespace Shop.Services { class Cart { Item i; } }\n"),
            ("Other/Thing.cs", "namespace Elsewhere { class Thing { Item i; } }\n"),
        ]);
        assert!(has(&g, "Services/Cart.cs", "Models/Item.cs"));
        assert!(!has(&g, "Other/Thing.cs", "Models/Item.cs"));
    }

    #[test]
    fn go_packages_of_the_module() {
        let (_d, g) = graph(&[
            ("go.mod", "module example.com/shop\n\ngo 1.22\n"),
            ("main.go", "package main\n\nimport (\n\t\"fmt\"\n\tc \"example.com/shop/cart\"\n)\n"),
            ("cart/cart.go", "package cart\n"),
            ("cart/cart_test.go", "package cart\nimport \"testing\"\n"),
        ]);
        assert!(has(&g, "main.go", "cart/cart.go"));
        assert!(!has(&g, "main.go", "cart/cart_test.go"));
        assert!(g.nodes[g.index_of("cart/cart_test.go").unwrap()].test);
    }

    #[test]
    fn c_quoted_and_include_folder() {
        let (_d, g) = graph(&[
            ("src/main.c", "#include <stdio.h>\n#include \"util.h\"\n#include <shop/cart.h>\n"),
            ("src/util.h", "int f(void);\n"),
            ("include/shop/cart.h", "struct cart;\n"),
        ]);
        assert!(has(&g, "src/main.c", "src/util.h"));
        assert!(has(&g, "src/main.c", "include/shop/cart.h"));
        assert_eq!(g.edges.len(), 2);
    }
}
