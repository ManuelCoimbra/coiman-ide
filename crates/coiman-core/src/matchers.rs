//! Problem matchers: finding errors in what a program printed, so a compiler or
//! a crash in any language lands in the Problems panel. Nothing here touches the
//! disk: callers decide which of the file names really are files.

use std::sync::LazyLock;

pub use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

impl Severity {
    fn from_word(word: &str) -> Self {
        let word = word.to_lowercase();
        if word.contains("warn") {
            Self::Warning
        } else if word.contains("note") || word.contains("info") {
            Self::Info
        } else {
            Self::Error
        }
    }
}

/// One problem a program reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// As printed: absolute, or relative to somewhere the caller must work out.
    pub file: String,
    /// From 1.
    pub line: u32,
    /// From 1; 0 when the program didn't say.
    pub col: u32,
    pub severity: Severity,
    pub message: String,
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in pattern")
}

/// gcc, clang, Zig: `file:1:2: error: text`.
static GCC: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^(.+?):(\d+):(\d+):\s*(fatal error|error|warning|note):\s*(.+)$"));
/// javac: `File.java:1: error: text`.
static JAVAC: LazyLock<Regex> = LazyLock::new(|| regex(r"^(.+?\.java):(\d+):\s*(error|warning):\s*(.+)$"));
/// Go: `./file.go:1:2: text`.
static GO: LazyLock<Regex> = LazyLock::new(|| regex(r"^(.+?\.go):(\d+):(\d+):\s*(.+)$"));
/// tsc, MSBuild, dotnet: `file(1,2): error CODE: text [project]`.
static MSBUILD: LazyLock<Regex> = LazyLock::new(|| {
    regex(r"^\s*(.+?)\((\d+),(\d+)\):\s*(error|warning)\s+(?:[A-Za-z]+\d+:\s*)?(.+?)(?:\s+\[[^\]]+\])?$")
});
/// rustc: `error[E0308]: text`, placed by the ` --> ` line after it.
static RUST_HEAD: LazyLock<Regex> = LazyLock::new(|| regex(r"^(error|warning)(?:\[E\d+\])?:\s*(.+)$"));
static RUST_AT: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*--> (.+?):(\d+):(\d+)\s*$"));
/// A Rust panic: `thread 'main' panicked at file:1:2:`, the text on the next line.
static PANIC: LazyLock<Regex> = LazyLock::new(|| regex(r"^thread '.*' .*panicked at (.+?):(\d+):(\d+):?\s*$"));
/// A frame of a Python traceback.
static PY_FRAME: LazyLock<Regex> = LazyLock::new(|| regex(r#"^\s+File "(.+?)", line (\d+)"#));
/// What ends a Python traceback.
static PY_END: LazyLock<Regex> = LazyLock::new(|| regex(r"^[A-Za-z_][\w.]*(Error|Exception|Warning|Interrupt|Exit)\b"));
/// JavaScript: `TypeError: text`, placed by the `at` lines after it.
static JS_HEAD: LazyLock<Regex> = LazyLock::new(|| regex(r"^(?:Uncaught )?([A-Za-z]*Error(?:: .*)?)$"));
static JS_FRAME: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s+at (?:.*\()?(.+?):(\d+):(\d+)\)?\s*$"));
/// Java: `Exception in thread "main" java.lang.X: text`.
static JAVA_HEAD: LazyLock<Regex> = LazyLock::new(|| regex(r#"^Exception in thread ".*?" (.+)$"#));
static JAVA_FRAME: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s+at [\w.$<>]+\(([\w$]+\.(?:java|kt)):(\d+)\)\s*$"));
/// .NET: `Unhandled exception. System.X: text`, then `at A.B() in file:line 1`.
static DOTNET_HEAD: LazyLock<Regex> = LazyLock::new(|| regex(r"^Unhandled exception\.\s*(.+)$"));
static DOTNET_FRAME: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s+at .+ in (.+):line (\d+)\s*$"));

/// A web address in terminal text.
static URL: LazyLock<Regex> = LazyLock::new(|| regex(r"https?://[^\s<>\x22'`]+"));

/// Which kind of message waits for the line that says where it happened.
enum Waiting {
    Nothing,
    RustPlace(Severity, String),
    Stack(String, StackKind),
    PanicText(String, u32, u32),
}

#[derive(Clone, Copy)]
enum StackKind {
    JavaScript,
    Java,
    DotNet,
}

/// Whether a Python frame is someone else's code, not the user's.
fn is_library_path(file: &str) -> bool {
    let file = file.replace('\\', "/").to_lowercase();
    file.starts_with('<') || file.contains("/site-packages/") || file.contains("/lib/python") || file.contains("/lib/")
}

/// The problems a program printed, by the built-in patterns.
pub fn scan<S: AsRef<str>>(lines: &[S]) -> Vec<Problem> {
    let mut found: Vec<Problem> = Vec::new();
    let mut waiting = Waiting::Nothing;
    let mut frames: Vec<(String, u32)> = Vec::new();
    let number = |text: &str| text.parse::<u32>().unwrap_or(0);
    for line in lines {
        let line = line.as_ref().trim_end();
        match std::mem::replace(&mut waiting, Waiting::Nothing) {
            Waiting::RustPlace(severity, message) => {
                if let Some(c) = RUST_AT.captures(line) {
                    let (file, l, col) = (c[1].to_owned(), number(&c[2]), number(&c[3]));
                    found.push(Problem { file, line: l, col, severity, message });
                    continue;
                }
            }
            Waiting::Stack(message, kind) => {
                let place = match kind {
                    StackKind::JavaScript => {
                        JS_FRAME.captures(line).map(|c| (c[1].to_owned(), c[2].to_owned(), number(&c[3])))
                    }
                    StackKind::Java => JAVA_FRAME.captures(line).map(|c| (c[1].to_owned(), c[2].to_owned(), 0)),
                    StackKind::DotNet => DOTNET_FRAME.captures(line).map(|c| (c[1].to_owned(), c[2].to_owned(), 0)),
                };
                match place {
                    // Runtime internals come first in some stacks: keep looking.
                    Some((file, _, _)) if file.starts_with("node:") => {
                        waiting = Waiting::Stack(message, kind);
                        continue;
                    }
                    Some((file, l, col)) => {
                        let severity = Severity::Error;
                        found.push(Problem { file, line: number(&l), col, severity, message });
                        continue;
                    }
                    // A blank or a code line can come before the first frame.
                    None if line.is_empty() => {
                        waiting = Waiting::Stack(message, kind);
                        continue;
                    }
                    None => {}
                }
            }
            Waiting::PanicText(file, l, col) => {
                let message = line.to_owned();
                found.push(Problem { file, line: l, col, severity: Severity::Error, message });
                continue;
            }
            Waiting::Nothing => {}
        }
        if let Some(c) = PY_FRAME.captures(line) {
            frames.push((c[1].to_owned(), number(&c[2])));
            continue;
        }
        if !frames.is_empty() && PY_END.is_match(line) {
            let own = frames.iter().rev().find(|(f, _)| !is_library_path(f));
            if let Some((file, l)) = own.or(frames.last()).cloned() {
                let severity = Severity::Error;
                found.push(Problem { file, line: l, col: 0, severity, message: line.to_owned() });
            }
            frames.clear();
            continue;
        }
        if let Some(c) = GCC.captures(line) {
            let severity = Severity::from_word(&c[4]);
            let (file, l, col, message) = (c[1].to_owned(), number(&c[2]), number(&c[3]), c[5].to_owned());
            found.push(Problem { file, line: l, col, severity, message });
        } else if let Some(c) = JAVAC.captures(line) {
            let severity = Severity::from_word(&c[3]);
            found.push(Problem {
                file: c[1].to_owned(),
                line: number(&c[2]),
                col: 0,
                severity,
                message: c[4].to_owned(),
            });
        } else if let Some(c) = MSBUILD.captures(line) {
            let severity = Severity::from_word(&c[4]);
            let (file, l, col, message) = (c[1].to_owned(), number(&c[2]), number(&c[3]), c[5].to_owned());
            found.push(Problem { file, line: l, col, severity, message });
        } else if let Some(c) = GO.captures(line) {
            let file = c[1].strip_prefix("./").unwrap_or(&c[1]).to_owned();
            let (l, col) = (number(&c[2]), number(&c[3]));
            found.push(Problem { file, line: l, col, severity: Severity::Error, message: c[4].to_owned() });
        } else if let Some(c) = PANIC.captures(line) {
            waiting = Waiting::PanicText(c[1].to_owned(), number(&c[2]), number(&c[3]));
        } else if let Some(c) = RUST_HEAD.captures(line) {
            waiting = Waiting::RustPlace(Severity::from_word(&c[1]), c[2].to_owned());
        } else if let Some(c) = DOTNET_HEAD.captures(line) {
            waiting = Waiting::Stack(c[1].to_owned(), StackKind::DotNet);
        } else if let Some(c) = JAVA_HEAD.captures(line) {
            waiting = Waiting::Stack(c[1].to_owned(), StackKind::Java);
        } else if let Some(c) = JS_HEAD.captures(line) {
            waiting = Waiting::Stack(c[1].to_owned(), StackKind::JavaScript);
        }
    }
    dedup(found)
}

/// The same report can print twice, as compilers and build tools do.
fn dedup(found: Vec<Problem>) -> Vec<Problem> {
    let mut seen = std::collections::HashSet::new();
    found.into_iter().filter(|p| seen.insert((p.file.clone(), p.line, p.col, p.message.clone()))).collect()
}

/// The problems a program printed, by a pattern from `launch.toml`. It names its
/// groups: `file`, `line`, `message`, and optionally `col` and `severity`.
pub fn scan_custom<S: AsRef<str>>(pattern: &Regex, lines: &[S]) -> Vec<Problem> {
    let found = lines
        .iter()
        .filter_map(|line| {
            let c = pattern.captures(line.as_ref().trim_end())?;
            Some(Problem {
                file: c.name("file")?.as_str().to_owned(),
                line: c.name("line")?.as_str().parse().ok()?,
                col: c.name("col").and_then(|m| m.as_str().parse().ok()).unwrap_or(0),
                severity: c.name("severity").map_or(Severity::Error, |m| Severity::from_word(m.as_str())),
                message: c.name("message")?.as_str().to_owned(),
            })
        })
        .collect();
    dedup(found)
}

/// Checks a pattern from `launch.toml`, saying what's wrong with it.
pub fn compile_custom(pattern: &str) -> Result<Regex, String> {
    // Its messages draw the pattern over several lines; the last says what's wrong.
    let compiled = Regex::new(pattern)
        .map_err(|e| format!("Bad matcher: {}", e.to_string().lines().last().unwrap_or("").trim()))?;
    let names: Vec<&str> = compiled.capture_names().flatten().collect();
    for needed in ["file", "line", "message"] {
        if !names.contains(&needed) {
            return Err(format!("Bad matcher: it needs a group named `{needed}`"));
        }
    }
    Ok(compiled)
}

/// The web address around character `col` of `text`, without trailing punctuation.
pub fn url_at(text: &str, col: usize) -> Option<String> {
    URL.find_iter(text).find_map(|m| {
        let start = text[..m.start()].chars().count();
        let url = m.as_str().trim_end_matches(['.', ',', ';', ':', ')', ']', '!', '?']);
        let end = start + url.chars().count();
        (start..end).contains(&col).then(|| url.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Problem {
        let lines: Vec<&str> = text.lines().collect();
        let mut found = scan(&lines);
        assert_eq!(found.len(), 1, "{found:?}");
        found.remove(0)
    }

    fn at(p: &Problem) -> (&str, u32, u32, Severity) {
        (p.file.as_str(), p.line, p.col, p.severity)
    }

    #[test]
    fn compilers() {
        let p = one("src/a.c:3:5: error: 'x' undeclared");
        assert_eq!(at(&p), ("src/a.c", 3, 5, Severity::Error));
        assert_eq!(p.message, "'x' undeclared");
        let p = one(r"C:\w\a.cpp:7:1: warning: unused");
        assert_eq!(at(&p), (r"C:\w\a.cpp", 7, 1, Severity::Warning));
        assert_eq!(at(&one("a.c:1:1: fatal error: nope.h")), ("a.c", 1, 1, Severity::Error));
        assert_eq!(at(&one("Main.java:4: error: ';' expected")), ("Main.java", 4, 0, Severity::Error));
        assert_eq!(at(&one("./m.go:9:2: undefined: q")), ("m.go", 9, 2, Severity::Error));
        let p = one(r"C:\w\P.cs(3,9): error CS0103: The name 'q' does not exist [C:\w\App.csproj]");
        assert_eq!(at(&p), (r"C:\w\P.cs", 3, 9, Severity::Error));
        assert_eq!(p.message, "The name 'q' does not exist");
        let p = one("src/a.ts(2,7): error TS2322: Type 'string' is not assignable");
        assert_eq!(at(&p), ("src/a.ts", 2, 7, Severity::Error));
        let p = one("error[E0308]: mismatched types\n --> src/main.rs:2:18\n  |\n");
        assert_eq!(at(&p), ("src/main.rs", 2, 18, Severity::Error));
        assert_eq!(p.message, "mismatched types");
        assert_eq!(at(&one("warning: unused variable\n  --> a.rs:1:2")), ("a.rs", 1, 2, Severity::Warning));
    }

    #[test]
    fn crashes() {
        let p = one(
            "Traceback (most recent call last):\n  File \"C:\\w\\a.py\", line 5, in <module>\n    f()\n  File \"C:\\w\\b.py\", line 2, in f\n    1/0\nZeroDivisionError: division by zero",
        );
        assert_eq!(at(&p), (r"C:\w\b.py", 2, 0, Severity::Error));
        assert_eq!(p.message, "ZeroDivisionError: division by zero");
        // A library at the top of the stack points back at the user's call.
        let p = one(
            "Traceback (most recent call last):\n  File \"/w/a.py\", line 5, in <module>\n  File \"/u/lib/python3/json/x.py\", line 2, in f\nValueError: bad",
        );
        assert_eq!(at(&p), ("/w/a.py", 5, 0, Severity::Error));
        let p = one("  File \"a.py\", line 1\n    def\n       ^\nSyntaxError: invalid syntax");
        assert_eq!((p.file.as_str(), p.line), ("a.py", 1));
        let p = one(
            "C:\\w\\a.js:1\nthrow new Error('boom');\n\nError: boom\n    at f (C:\\w\\a.js:1:7)\n    at node:internal/main:1:1",
        );
        assert_eq!(at(&p), (r"C:\w\a.js", 1, 7, Severity::Error));
        assert_eq!(p.message, "Error: boom");
        let p = one("TypeError: x is not a function\n    at node:internal/foo:1:1\n    at /w/a.js:3:2");
        assert_eq!(at(&p), ("/w/a.js", 3, 2, Severity::Error));
        let p = one("Exception in thread \"main\" java.lang.RuntimeException: boom\n\tat Main.main(Main.java:3)");
        assert_eq!(at(&p), ("Main.java", 3, 0, Severity::Error));
        let p = one(
            "Unhandled exception. System.Exception: boom\n   at Program.<Main>$(String[] args) in C:\\w\\Program.cs:line 1",
        );
        assert_eq!(at(&p), (r"C:\w\Program.cs", 1, 0, Severity::Error));
        let p = one("thread 'main' (1) panicked at src/main.rs:2:5:\nboom\nnote: run with RUST_BACKTRACE");
        assert_eq!(at(&p), ("src/main.rs", 2, 5, Severity::Error));
        assert_eq!(p.message, "boom");
    }

    #[test]
    fn ordinary_output_is_not_a_problem() {
        let lines = [
            "Compiling app v0.1.0 (C:\\w)",
            "Finished `dev` profile in 0.5s",
            "RAN 42",
            "12:30:45: server started",
            "http://localhost:3000/x:1:2",
            "error: could not compile `app`",
            "warning: `app` generated 1 warning",
            "Error handling is fine",
        ];
        assert_eq!(scan(&lines), []);
    }

    #[test]
    fn repeats_count_once() {
        assert_eq!(scan(&["a.c:1:1: error: x", "a.c:1:1: error: x"]).len(), 1);
    }

    #[test]
    fn custom_patterns() {
        assert!(compile_custom("(").unwrap_err().contains("Bad matcher"));
        assert!(compile_custom(r"(?P<file>\S+)").unwrap_err().contains("`line`"));
        let pattern = compile_custom(r"^LINT (?P<file>\S+) @(?P<line>\d+) (?P<severity>\w+) (?P<message>.+)$").unwrap();
        let found = scan_custom(&pattern, &["LINT a.txt @4 warn too long", "other"]);
        assert_eq!(at(&found[0]), ("a.txt", 4, 0, Severity::Warning));
        assert_eq!(found[0].message, "too long");
    }

    #[test]
    fn finds_the_url_under_the_pointer() {
        let text = "Listening on http://localhost:3000/a?b=1, see https://x.dev.";
        assert_eq!(url_at(text, 20).as_deref(), Some("http://localhost:3000/a?b=1"));
        assert_eq!(url_at(text, 50).as_deref(), Some("https://x.dev"));
        assert_eq!(url_at(text, 2), None);
        assert_eq!(url_at(text, 40), None);
    }
}
