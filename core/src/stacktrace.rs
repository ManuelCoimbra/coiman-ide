//! Stack traces pasted from a log, a ticket or a chat: puts back what wrapping and
//! logging did to them, finds the frames in any of the usual languages, and works
//! out which file of this project each one means.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// One place in a trace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub language: &'static str,
    pub symbol: Option<String>,
    /// The file as the trace names it.
    pub file: String,
    /// Folders the trace implies but doesn't name, like a Java package's.
    pub folder_hint: Option<String>,
    /// From 1.
    pub line: u32,
    pub column: Option<u32>,
}

/// Where a frame is in this project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// This file, by its path or the folders around it.
    Project(PathBuf),
    /// Only the file's name matches, so it may be another file: the best guess,
    /// and how many others there were.
    Maybe(PathBuf, usize),
    /// Not in this project.
    Elsewhere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub frame: Option<(Frame, Place)>,
}

/// A trace, read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    pub rows: Vec<Row>,
    /// Lines put back together or apart, for saying so.
    pub repaired: usize,
}

impl Analysis {
    pub fn frames(&self) -> usize {
        self.rows.iter().filter(|r| r.frame.is_some()).count()
    }

    pub fn in_project(&self) -> usize {
        self.rows.iter().filter(|r| matches!(r.frame, Some((_, Place::Project(_) | Place::Maybe(..))))).count()
    }
}

fn pattern(cell: &'static OnceLock<Regex>, text: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(text).expect("the pattern is valid"))
}

macro_rules! regex {
    ($text:expr) => {{
        static CELL: OnceLock<Regex> = OnceLock::new();
        pattern(&CELL, $text)
    }};
}

const SOURCE_EXTENSIONS: &[(&str, &str)] = &[
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("kts", "Kotlin"),
    ("scala", "Scala"),
    ("groovy", "Groovy"),
    ("clj", "Clojure"),
    ("cs", "C#"),
    ("fs", "F#"),
    ("vb", "Visual Basic"),
    ("py", "Python"),
    ("js", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("jsx", "JavaScript"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("vue", "Vue"),
    ("rs", "Rust"),
    ("go", "Go"),
    ("c", "C"),
    ("h", "C"),
    ("cc", "C++"),
    ("cpp", "C++"),
    ("cxx", "C++"),
    ("hpp", "C++"),
    ("m", "Objective-C"),
    ("mm", "Objective-C"),
    ("swift", "Swift"),
    ("dart", "Dart"),
    ("php", "PHP"),
    ("rb", "Ruby"),
    ("rake", "Ruby"),
    ("erb", "Ruby"),
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("erl", "Erlang"),
    ("lua", "Lua"),
    ("pl", "Perl"),
    ("zig", "Zig"),
    ("gd", "GDScript"),
];

fn language_of(file: &str) -> Option<&'static str> {
    let ext = file.rsplit('.').next()?.to_lowercase();
    SOURCE_EXTENSIONS.iter().find(|(e, _)| *e == ext).map(|(_, name)| *name)
}

/// The folders of a Java-like symbol: `java.base/com.foo.Bar$In.run` is `com/foo`.
fn package_folder(symbol: &str) -> Option<String> {
    let symbol = symbol.rsplit('/').next().unwrap_or(symbol);
    let parts: Vec<&str> = symbol.split('.').collect();
    (parts.len() > 2).then(|| parts[..parts.len() - 2].join("/"))
}

fn number(text: Option<regex::Match>) -> Option<u32> {
    text.and_then(|m| m.as_str().parse().ok())
}

/// The frame a line of a trace names, if any.
pub fn parse_frame(line: &str) -> Option<Frame> {
    let frame = |language, symbol: Option<&str>, file: &str, line, column| {
        Some(Frame {
            language,
            symbol: symbol.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()),
            file: file.trim().to_owned(),
            folder_hint: None,
            line,
            column,
        })
    };
    let text = line.trim_end();
    // at com.foo.Bar.baz(Bar.java:12) ~[app.jar:1.0]
    let java = regex!(r"^\s*at\s+([\w.$<>/@-]+)\(([^():]+\.\w+):(\d+)\)");
    if let Some(c) = java.captures(text) {
        let (symbol, file) = (&c[1], &c[2]);
        return Some(Frame {
            folder_hint: package_folder(symbol),
            ..frame(language_of(file).unwrap_or("Java"), Some(symbol), file, number(c.get(3))?, None)?
        });
    }
    // at Ns.Type.Method(Int32 a) in C:\src\File.cs:line 12
    let csharp = regex!(r"^\s*(?:at|à|bei|en)\s+(.+?)\s+(?:in|dans|à|en)\s+(.+?):(?:line|Zeile|ligne|línea)\s+(\d+)");
    if let Some(c) = csharp.captures(text) {
        return frame(language_of(&c[2]).unwrap_or("C#"), Some(&c[1]), &c[2], number(c.get(3))?, None);
    }
    // File "app/x.py", line 3, in main
    let python = regex!(r#"^\s*File "(.+?)", line (\d+)(?:, in (.+))?"#);
    if let Some(c) = python.captures(text) {
        return frame("Python", c.get(3).map(|m| m.as_str()), &c[1], number(c.get(2))?, None);
    }
    // #0 0x4004f4 in main /src/a.cpp:12:3 (sanitizers)
    let sanitizer = regex!(r"^\s*#\d+\s+0x[0-9a-fA-F]+\s+in\s+(.+?)\s+(\S+\.\w+):(\d+)(?::(\d+))?");
    if let Some(c) = sanitizer.captures(text) {
        return frame(language_of(&c[2]).unwrap_or("C++"), Some(&c[1]), &c[2], number(c.get(3))?, number(c.get(4)));
    }
    // #1 0x0000 in f (x=1) at a.cpp:12 (gdb); #1 f at a.cpp:12
    let gdb = regex!(r"^\s*#\d+\s+(?:0x[0-9a-fA-F]+\s+in\s+)?(.+?)\s+at\s+(\S+\.\w+):(\d+)");
    if let Some(c) = gdb.captures(text) {
        return frame(language_of(&c[2]).unwrap_or("C++"), Some(&c[1]), &c[2], number(c.get(3))?, None);
    }
    // #0 /var/www/a.php(12): f()
    let php = regex!(r"^\s*#\d+\s+(.+?\.\w+)\((\d+)\):\s*(.*)$");
    if let Some(c) = php.captures(text) {
        return frame("PHP", Some(&c[3]), &c[1], number(c.get(2))?, None);
    }
    // #0 main (package:app/main.dart:3:4)
    let dart = regex!(r"^\s*#\d+\s+(.+?)\s+\((.+?\.\w+):(\d+)(?::(\d+))?\)$");
    if let Some(c) = dart.captures(text) {
        let file = c[2].strip_prefix("package:").map(|rest| match rest.split_once('/') {
            Some((_, inner)) => format!("lib/{inner}"),
            None => rest.to_owned(),
        });
        let file = file.as_deref().unwrap_or(&c[2]);
        return frame(language_of(file).unwrap_or("Dart"), Some(&c[1]), file, number(c.get(3))?, number(c.get(4)));
    }
    // from a.rb:3:in 'f'
    let ruby = regex!(r"^\s*(?:from\s+)?(.+?\.(?:rb|rake|erb)):(\d+):in\s+[`'](.+?)['`]?$");
    if let Some(c) = ruby.captures(text) {
        return frame("Ruby", Some(&c[3]), &c[1], number(c.get(2))?, None);
    }
    // (app 1.0) lib/x.ex:12: M.f/1
    let elixir = regex!(r"^\s*(?:\(.+?\)\s+)?(\S+\.exs?):(\d+):\s*(.+)$");
    if let Some(c) = elixir.captures(text) {
        return frame("Elixir", Some(&c[3]), &c[1], number(c.get(2))?, None);
    }
    // /src/a.go:12 +0x1d
    let go = regex!(r"^\s*(\S.*\.go):(\d+)(?:\s+\+0x[0-9a-fA-F]+)?\s*$");
    if let Some(c) = go.captures(text) {
        return frame("Go", None, &c[1], number(c.get(2))?, None);
    }
    // thread 'main' panicked at src/main.rs:2:5:
    let panic = regex!(r"panicked at\s+(.+?\.rs):(\d+):(\d+)");
    if let Some(c) = panic.captures(text) {
        return frame("Rust", None, &c[1], number(c.get(2))?, number(c.get(3)));
    }
    // at async f (/app/x.js:1:2), at /app/x.js:1:2, at ./src/main.rs:3:5
    let at = regex!(
        r"^\s*at\s+(?:async\s+)?(?:(.+?)\s+\()?((?:file:/+)?(?:[A-Za-z]:)?[^():]+\.\w+):(\d+)(?::(\d+))?\)?\s*$"
    );
    if let Some(c) = at.captures(text) {
        let file = &c[2];
        let bare = file.strip_prefix("file:///").map_or(file, |rest| {
            // file:///C:/x on Windows, file:///x elsewhere.
            if rest.as_bytes().get(1) == Some(&b':') { rest } else { &file["file://".len()..] }
        });
        let language = language_of(bare).unwrap_or("JavaScript");
        return frame(language, c.get(1).map(|m| m.as_str()), bare, number(c.get(3))?, number(c.get(4)));
    }
    // Anything else naming a source file and a line.
    let plain = regex!(r"(?:^|[\s(])((?:[A-Za-z]:)?[\w./\\-]+\.(\w{1,6})):(\d+)(?::(\d+))?\b");
    let c = plain.captures(text)?;
    let language = language_of(&c[1])?;
    frame(language, None, &c[1], number(c.get(3))?, number(c.get(4)))
}

/// Takes off what logging adds before each line, and quoting and colors.
fn strip_decoration(line: &str) -> String {
    let ansi = regex!(r"\x1b\[[0-9;]*[A-Za-z]");
    let line = ansi.replace_all(line, "");
    let quote = regex!(r"^(?:>\s?)+");
    let line = quote.replace(&line, "");
    let stamp = regex!(
        r"^\[?\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?\]?\s*(?:\[[^\]]*\]\s*)?(?:(?:TRACE|DEBUG|INFO|WARN|WARNING|ERROR|FATAL|SEVERE)\b:?\s*)?(?:\[[^\]]*\]\s*)?"
    );
    stamp.replace(&line, "").into_owned()
}

/// Whether a line is a frame cut short by wrapping, which the next line finishes.
/// The flag says whether a space belongs between them.
fn cut_short(line: &str, next: &str) -> Option<bool> {
    let text = line.trim();
    let next = next.trim();
    if next.is_empty() {
        return None;
    }
    let opens = text.matches('(').count();
    let closes = text.matches(')').count();
    if text.starts_with("at ") {
        let csharp_path = text.contains(" in ") && !regex!(r":(?:line|Zeile|ligne|línea)\s+\d+").is_match(text);
        if opens > closes || csharp_path {
            return Some(false);
        }
        // The file was pushed onto the next line whole.
        if next.starts_with('(') && !text.contains('(') {
            return Some(false);
        }
    }
    if text.starts_with("File \"") {
        if text.matches('"').count() == 1 {
            return Some(false);
        }
        if text.ends_with(',') || text.ends_with(", in") || text.ends_with(", line") {
            return Some(true);
        }
    }
    None
}

/// Puts a trace back as it was printed. Returns its lines, and how many changed
/// beyond the decoration every line loses.
pub fn repair(text: &str) -> (Vec<String>, usize) {
    let mut repaired = 0;
    let text = text.replace("\r\n", "\n");
    // Escaped inside a JSON log line.
    let unescaped = if regex!(r"\\n\s*(?:\\t)?\s*(?:at |File )").is_match(&text) {
        repaired += 1;
        text.replace("\\r\\n", "\n").replace("\\n", "\n").replace("\\t", "\t").replace("\\\"", "\"")
    } else {
        text
    };
    let mut lines: Vec<String> = Vec::new();
    for line in unescaped.lines().map(strip_decoration) {
        // A trace flattened onto one line: a new frame at each ` at x(`.
        let breaks = regex!(r"\s+at\s+[\w.$<>/@-]+\(");
        let starts: Vec<usize> = breaks.find_iter(&line).map(|m| m.start()).collect();
        if starts.len() >= 2 {
            repaired += starts.len();
            let mut from = 0;
            for start in starts {
                lines.push(line[from..start].to_owned());
                from = start;
            }
            lines.push(line[from..].to_owned());
        } else {
            lines.push(line);
        }
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut joined = 0;
    for line in lines {
        let glue = match out.last() {
            Some(last) if joined < 4 => cut_short(last, &line),
            _ => None,
        };
        match (glue, out.last_mut()) {
            (Some(space), Some(last)) => {
                last.truncate(last.trim_end().len());
                if space {
                    last.push(' ');
                }
                last.push_str(line.trim());
                joined += 1;
                repaired += 1;
            }
            _ => {
                out.push(line);
                joined = 0;
            }
        }
    }
    (out, repaired)
}

/// Paths of code that isn't the project's, whatever a file there is called.
/// Where tools and their packages are installed. A project could be in one of
/// these too, so it's only library code if it isn't in the project that's open.
fn is_installed_path(file: &str) -> bool {
    let text = file.replace('\\', "/").to_lowercase();
    [
        "/.rustup/toolchains/",
        "/.m2/repository/",
        "/.gradle/caches/",
        "/.nuget/packages/",
        "/go/pkg/mod/",
        "/usr/local/go/src/",
        "/usr/local/lib/",
        "/.pyenv/versions/",
        "/program files/",
        "/program files (x86)/",
        "/appdata/local/programs/",
        "/library/frameworks/",
        "/jdk/",
    ]
    .iter()
    .any(|part| text.contains(part))
}

fn is_library_path(file: &str) -> bool {
    let text = file.replace('\\', "/");
    ["site-packages/", "dist-packages/", "node_modules/", "/usr/lib/", "/usr/include/", "/.cargo/registry/", "/rustc/"]
        .iter()
        .any(|part| text.contains(part))
        || text.starts_with('<')
        || text.starts_with("node:")
}

fn components(path: &str) -> Vec<String> {
    path.split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .map(|p| if cfg!(windows) { p.to_lowercase() } else { p.to_owned() })
        .collect()
}

/// Works out where a frame is, given this project's files.
pub fn locate(frame: &Frame, root: &Path, files: &[PathBuf]) -> Place {
    let named = Path::new(&frame.file);
    if is_library_path(&frame.file) || (is_installed_path(&frame.file) && !named.starts_with(root)) {
        return Place::Elsewhere;
    }
    if named.is_absolute() && named.is_file() {
        return Place::Project(named.to_path_buf());
    }
    if !named.is_absolute() && root.join(named).is_file() {
        return Place::Project(root.join(named));
    }
    // With the package as folders, a Java frame reads like any other path.
    let wanted = match &frame.folder_hint {
        Some(folder) => components(&format!("{folder}/{}", frame.file)),
        None => components(&frame.file),
    };
    let Some(name) = wanted.last() else { return Place::Elsewhere };
    let mut best: Option<(usize, &PathBuf)> = None;
    let mut same_name = 0;
    for file in files {
        let rel = file.strip_prefix(root).unwrap_or(file);
        let have = components(&rel.to_string_lossy());
        if have.last() != Some(name) {
            continue;
        }
        same_name += 1;
        let shared = have.iter().rev().zip(wanted.iter().rev()).take_while(|(a, b)| a == b).count();
        if best.is_none_or(|(most, _)| shared > most) {
            best = Some((shared, file));
        }
    }
    let Some((shared, file)) = best else { return Place::Elsewhere };
    // A folder in common, or the trace named no folders to disagree with.
    if shared >= 2 || wanted.len() == 1 && same_name == 1 {
        return Place::Project(file.clone());
    }
    Place::Maybe(file.clone(), same_name - 1)
}

/// Reads a trace, against a project's files.
pub fn analyze(text: &str, root: &Path, files: &[PathBuf]) -> Analysis {
    let (lines, repaired) = repair(text);
    let rows = lines
        .into_iter()
        .map(|text| {
            let frame = parse_frame(&text).map(|f| {
                let place = locate(&f, root, files);
                (f, place)
            });
            Row { text, frame }
        })
        .collect();
    Analysis { rows, repaired }
}

/// A project's files, for [`analyze`].
pub fn project_files(root: &Path) -> Vec<PathBuf> {
    crate::finder::walker(root)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(line: &str) -> Frame {
        parse_frame(line).unwrap_or_else(|| panic!("no frame in {line}"))
    }

    fn at(line: &str) -> (&'static str, String, u32) {
        let f = frame(line);
        (f.language, f.file, f.line)
    }

    #[test]
    fn reads_java_and_kotlin() {
        let f = frame("\tat com.foo.Bar$Inner.baz(Bar.java:12) ~[app.jar:1.0]");
        assert_eq!((f.language, f.file.as_str(), f.line), ("Java", "Bar.java", 12));
        assert_eq!(f.symbol.as_deref(), Some("com.foo.Bar$Inner.baz"));
        assert_eq!(f.folder_hint.as_deref(), Some("com/foo"));
        assert_eq!(frame("at java.base/java.util.List.of(List.java:3)").folder_hint.as_deref(), Some("java/util"));
        assert_eq!(at("    at com.foo.MainKt.main(Main.kt:7)"), ("Kotlin", "Main.kt".to_owned(), 7));
        assert_eq!(parse_frame("\tat java.lang.Thread.run(Native Method)"), None);
        assert_eq!(parse_frame("\tat sun.misc.Unsafe.park(Unknown Source)"), None);
    }

    #[test]
    fn reads_csharp() {
        let f = frame(r"   at App.Svc.Run(Int32 a) in C:\src\App\Svc.cs:line 42");
        assert_eq!((f.language, f.file.as_str(), f.line), ("C#", r"C:\src\App\Svc.cs", 42));
        assert_eq!(f.symbol.as_deref(), Some("App.Svc.Run(Int32 a)"));
        assert_eq!(at("   at A.B() in /src/B.cs:line 3"), ("C#", "/src/B.cs".to_owned(), 3));
        assert_eq!(parse_frame("   at System.Linq.Enumerable.First()"), None);
    }

    #[test]
    fn reads_python() {
        let f = frame(r#"  File "app/x.py", line 3, in main"#);
        assert_eq!((f.language, f.file.as_str(), f.line), ("Python", "app/x.py", 3));
        assert_eq!(f.symbol.as_deref(), Some("main"));
        assert_eq!(frame(r#"  File "C:\a b\x.py", line 9"#).file, r"C:\a b\x.py");
    }

    #[test]
    fn reads_javascript_and_typescript() {
        let f = frame("    at async run (/app/src/x.js:10:5)");
        assert_eq!((f.language, f.file.as_str(), f.line, f.column), ("JavaScript", "/app/src/x.js", 10, Some(5)));
        assert_eq!(f.symbol.as_deref(), Some("run"));
        assert_eq!(at("    at /app/y.ts:1:2"), ("TypeScript", "/app/y.ts".to_owned(), 1));
        assert_eq!(at("    at f (C:\\app\\z.js:1:2)"), ("JavaScript", "C:\\app\\z.js".to_owned(), 1));
        assert_eq!(frame("    at file:///C:/app/z.mjs:4:1").file, "C:/app/z.mjs");
        assert_eq!(frame("    at file:///app/z.mjs:4:1").file, "/app/z.mjs");
        assert_eq!(parse_frame("    at node:internal/main:1:2"), None);
    }

    #[test]
    fn reads_the_rest() {
        assert_eq!(at("      at ./src/main.rs:3:5"), ("Rust", "./src/main.rs".to_owned(), 3));
        assert_eq!(at("thread 'main' panicked at src/main.rs:2:5:"), ("Rust", "src/main.rs".to_owned(), 2));
        assert_eq!(at("\t/src/a/b.go:12 +0x1d"), ("Go", "/src/a/b.go".to_owned(), 12));
        assert_eq!(at("#0  0x0000 in f (x=1) at a.cpp:12"), ("C++", "a.cpp".to_owned(), 12));
        let asan = frame("    #1 0x4f4 in main /src/a.cc:12:3");
        assert_eq!((asan.file.as_str(), asan.symbol.as_deref(), asan.column), ("/src/a.cc", Some("main"), Some(3)));
        assert_eq!(at("#0 /var/www/a.php(12): f()"), ("PHP", "/var/www/a.php".to_owned(), 12));
        assert_eq!(at("#2 main (package:app/src/m.dart:3:4)"), ("Dart", "lib/src/m.dart".to_owned(), 3));
        assert_eq!(at("\tfrom /a/b.rb:3:in `f'"), ("Ruby", "/a/b.rb".to_owned(), 3));
        assert_eq!(at("a.rb:3:in 'Integer#f'"), ("Ruby", "a.rb".to_owned(), 3));
        assert_eq!(at("    (app 1.0) lib/x.ex:12: M.f/1"), ("Elixir", "lib/x.ex".to_owned(), 12));
        assert_eq!(at("error in src/thing.zig:4:2 oh no"), ("Zig", "src/thing.zig".to_owned(), 4));
    }

    #[test]
    fn leaves_ordinary_lines_alone() {
        for line in [
            "Traceback (most recent call last):",
            "java.lang.IllegalStateException: boom",
            "Caused by: java.io.IOException: nope",
            "    ... 12 more",
            "ValueError: invalid literal for int()",
            "See http://example.com:80/x",
            "version 1.2:3",
            "",
        ] {
            assert_eq!(parse_frame(line), None, "{line}");
        }
    }

    #[test]
    fn repairs_wrapped_frames() {
        let (lines, n) = repair("at com.foo.Bar.baz(Bar.ja\nva:12)\nat com.foo.Qux.run\n(Qux.java:3)");
        assert_eq!(lines, ["at com.foo.Bar.baz(Bar.java:12)", "at com.foo.Qux.run(Qux.java:3)"]);
        assert_eq!(n, 2);
        let (lines, _) = repair("   at A.B() in C:\\src\\long\\\n     File.cs:line 12");
        assert_eq!(lines, ["   at A.B() in C:\\src\\long\\File.cs:line 12"]);
        assert_eq!(frame(&lines[0]).line, 12);
        let (lines, _) = repair("  File \"app/x.py\", line 3,\n    in main\nnext");
        assert_eq!(lines, ["  File \"app/x.py\", line 3, in main", "next"]);
        let (lines, _) = repair("  File \"app/lo\nng.py\", line 3, in main");
        assert_eq!(frame(&lines[0]).file, "app/long.py");
    }

    #[test]
    fn does_not_glue_what_is_whole() {
        let text = "at a.B.c(B.java:1)\nat d.E.f(E.java:2)\nCaused by: x (y)\nsome (open text\nmore";
        let (lines, n) = repair(text);
        assert_eq!(lines.len(), 5);
        assert_eq!(n, 0);
    }

    #[test]
    fn takes_off_what_logging_adds() {
        let (lines, _) = repair(
            "2026-10-31 12:00:00,123 ERROR [main] boom\n\
             2026-10-31T12:00:00.5Z at a.B.c(B.java:1)\n\
             > > at d.E.f(E.java:2)\n\
             \x1b[31mat g.H.i(H.java:3)\x1b[0m",
        );
        assert_eq!(lines[0], "boom");
        assert_eq!(frame(&lines[1]).line, 1);
        assert_eq!(frame(&lines[2]).line, 2);
        assert_eq!(frame(&lines[3]).line, 3);
    }

    #[test]
    fn opens_up_flattened_traces() {
        let (lines, _) = repair(r#"{"msg":"boom\n\tat a.B.c(B.java:1)\n\tat d.E.f(E.java:2)"}"#);
        assert_eq!(lines.len(), 3);
        assert_eq!(frame(&lines[1]).line, 1);
        let (lines, _) = repair("Boom: x at a.B.c(B.java:1) at d.E.f(E.java:2)");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Boom: x");
        assert_eq!(frame(&lines[2]).line, 2);
    }

    fn project(files: &[&str]) -> (PathBuf, Vec<PathBuf>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "rc-trace-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut all = Vec::new();
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            all.push(path);
        }
        (root, all)
    }

    #[test]
    fn finds_frames_in_the_project() {
        let (root, files) = project(&[
            "src/main/java/com/foo/Bar.java",
            "src/other/Bar.java",
            "app/x.py",
            "web/util.js",
            "pkg/util.js",
        ]);
        let place = |line: &str| locate(&frame(line), &root, &files);
        assert_eq!(
            place("at com.foo.Bar.baz(Bar.java:1)"),
            Place::Project(root.join("src/main/java/com/foo/Bar.java"))
        );
        assert_eq!(place(r#"File "app/x.py", line 1"#), Place::Project(root.join("app/x.py")));
        // A path from the machine that ran it: the end of it still says which.
        assert_eq!(place(r#"File "/srv/deploy/app/x.py", line 1"#), Place::Project(root.join("app/x.py")));
        assert_eq!(place("at f (/other/web/util.js:1:1)"), Place::Project(root.join("web/util.js")));
        // Same name, nothing else in common: a guess, said to be one.
        assert_eq!(place("at f (/elsewhere/lib/x.py:1:1)"), Place::Maybe(root.join("app/x.py"), 0));
        assert!(matches!(place("at f (util.js:1:1)"), Place::Maybe(_, 1)));
        assert_eq!(place("at org.none.Nope.f(Nope.java:1)"), Place::Elsewhere);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn library_paths_stay_out_even_with_a_matching_name() {
        let (root, files) = project(&["json/decoder.py"]);
        let f = frame(r#"File "/usr/lib/python3/json/decoder.py", line 1"#);
        assert_eq!(locate(&f, &root, &files), Place::Elsewhere);
        let f = frame("at f (/a/node_modules/x/index.js:1:1)");
        assert_eq!(locate(&f, &root, &files), Place::Elsewhere);
        let f = frame(r#"File "C:\\Program Files\\Python\\json\\decoder.py", line 1"#);
        assert_eq!(locate(&f, &root, &files), Place::Elsewhere);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn analyzes_a_whole_trace() {
        let (root, files) = project(&["src/a.py"]);
        let text = "Traceback (most recent call last):\n  File \"src/a.py\", line 2, in f\n  File \"/usr/lib/python3/b.py\", line 3, in g\nValueError: x";
        let a = analyze(text, &root, &files);
        assert_eq!(a.rows.len(), 4);
        assert_eq!((a.frames(), a.in_project()), (2, 1));
        std::fs::remove_dir_all(root).unwrap();
    }
}
