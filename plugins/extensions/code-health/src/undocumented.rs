//! Public items without a doc comment: Rust's `pub` items (no `///` or `#[doc]` above),
//! Go's exported names (no `//` above), Python's public top-level `def` and `class` (no
//! docstring) and JavaScript's and TypeScript's `export`s (no `/** */` above). Read line by
//! line: what's public is told by the line the item starts on.

/// A public item with no documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub path: String,
    /// 1-based.
    pub line: usize,
    pub kind: &'static str,
    pub name: String,
}

pub const MAX_BYTES: u64 = 1024 * 1024;

/// The word after `prefix` at the start of `rest`, if `rest` starts with it.
fn after<'a>(rest: &'a str, prefix: &str) -> Option<&'a str> {
    rest.strip_prefix(prefix).map(str::trim_start)
}

/// The leading identifier of `s`.
fn ident(s: &str) -> String {
    s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect()
}

/// The line above `i` that isn't blank or an attribute/decorator, if any.
fn above<'a>(lines: &[&'a str], i: usize, skip: &[&str]) -> Option<&'a str> {
    lines[..i].iter().rev().map(|l| l.trim()).find(|l| !l.is_empty() && !skip.iter().any(|s| l.starts_with(s)))
}

fn rust(lines: &[&str], path: &str, out: &mut Vec<Item>) {
    for (i, line) in lines.iter().enumerate() {
        // Tests and what follows them aren't public API.
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        let Some(mut rest) = after(line.trim_start(), "pub ") else { continue };
        for q in ["async ", "unsafe ", "const fn", "extern \"C\" "] {
            if let Some(r) = rest.strip_prefix(q) {
                rest = if q == "const fn" { "fn" } else { r };
                if q == "const fn" {
                    rest = line.trim_start().split("const fn").nth(1).map_or("", |r| r).trim_start();
                    let name = ident(rest);
                    push_rust(lines, i, path, "fn", name, out);
                    rest = "";
                }
            }
        }
        for kind in ["fn", "struct", "enum", "trait", "type", "const", "static", "mod", "union"] {
            if let Some(r) = after(rest, &format!("{kind} ")) {
                let name = ident(r.trim_start_matches("mut "));
                // `pub mod x;` is documented in its own file.
                if kind == "mod" && r.trim_end().ends_with(';') {
                    break;
                }
                push_rust(lines, i, path, kind, name, out);
                break;
            }
        }
    }
}

fn push_rust(lines: &[&str], i: usize, path: &str, kind: &'static str, name: String, out: &mut Vec<Item>) {
    if name.is_empty() {
        return;
    }
    let documented = above(lines, i, &["#[", "#!["])
        .is_some_and(|l| l.starts_with("///") || l.starts_with("/**") || l.ends_with("*/"))
        || lines[..i].iter().rev().take_while(|l| l.trim().starts_with("#[")).any(|l| l.contains("#[doc"));
    if !documented {
        out.push(Item { path: path.to_owned(), line: i + 1, kind, name });
    }
}

fn go(lines: &[&str], path: &str, out: &mut Vec<Item>) {
    for (i, line) in lines.iter().enumerate() {
        let (kind, rest) = if let Some(r) = line.strip_prefix("func ") {
            // A method: `func (r *T) Name(`.
            let r = if r.starts_with('(') { r.split_once(')').map_or("", |(_, b)| b.trim_start()) } else { r };
            ("func", r)
        } else if let Some(r) = line.strip_prefix("type ") {
            ("type", r)
        } else {
            continue;
        };
        let name = ident(rest);
        if !name.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        if !(i > 0 && lines[i - 1].trim_start().starts_with("//")) {
            out.push(Item { path: path.to_owned(), line: i + 1, kind, name });
        }
    }
}

fn python(lines: &[&str], path: &str, out: &mut Vec<Item>) {
    for (i, line) in lines.iter().enumerate() {
        let (kind, rest) = if let Some(r) = line.strip_prefix("def ").or_else(|| line.strip_prefix("async def ")) {
            ("def", r)
        } else if let Some(r) = line.strip_prefix("class ") {
            ("class", r)
        } else {
            continue;
        };
        let name = ident(rest);
        if name.is_empty() || name.starts_with('_') {
            continue;
        }
        // The signature may go on for a few lines; the body starts after the line ending in ':'.
        let Some(end) = (i..lines.len().min(i + 12)).find(|&j| lines[j].trim_end().ends_with(':')) else { continue };
        let first = lines[end + 1..].iter().map(|l| l.trim()).find(|l| !l.is_empty());
        let documented = first.is_some_and(|l| {
            let l = l.trim_start_matches(['r', 'R', 'u', 'U']);
            l.starts_with("\"\"\"") || l.starts_with("'''") || l.starts_with('"') || l.starts_with('\'')
        });
        if !documented {
            out.push(Item { path: path.to_owned(), line: i + 1, kind, name });
        }
    }
}

fn javascript(lines: &[&str], path: &str, out: &mut Vec<Item>) {
    for (i, line) in lines.iter().enumerate() {
        let Some(mut rest) = after(line.trim_start(), "export ") else { continue };
        for q in ["default ", "async ", "declare ", "abstract "] {
            rest = rest.strip_prefix(q).unwrap_or(rest);
        }
        let Some((kind, r)) = ["function", "class", "const", "interface", "type", "enum"].iter().find_map(|k| {
            after(rest, &format!("{k} "))
                .or_else(|| (*k == "function").then(|| rest.strip_prefix("function*")).flatten())
                .map(|r| (*k, r))
        }) else {
            continue;
        };
        let name = ident(r.trim_start_matches('*').trim_start());
        if name.is_empty() {
            continue;
        }
        let documented = above(lines, i, &["@"]).is_some_and(|l| l.ends_with("*/"));
        if !documented {
            let kind = match kind {
                "function" => "function",
                "class" => "class",
                "const" => "const",
                "interface" => "interface",
                "type" => "type",
                _ => "enum",
            };
            out.push(Item { path: path.to_owned(), line: i + 1, kind, name });
        }
    }
}

/// The undocumented public items of one file's text, by its extension.
pub fn in_text(ext: &str, path: &str, text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    match ext {
        "rs" => rust(&lines, path, &mut out),
        "go" if !path.ends_with("_test.go") => go(&lines, path, &mut out),
        "py" if !path.rsplit('/').next().is_some_and(|n| n.starts_with("test_")) => python(&lines, path, &mut out),
        "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "jsx" | "tsx"
            if !path.contains(".test.") && !path.contains(".spec.") =>
        {
            javascript(&lines, path, &mut out)
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ext: &str, text: &str) -> Vec<String> {
        in_text(ext, "x", text).into_iter().map(|i| format!("{}:{}", i.kind, i.name)).collect()
    }

    #[test]
    fn rust_pub_items() {
        let text = "/// Said.\npub fn said() {}\n\npub fn bare() {}\n/// Kept.\n#[derive(Debug)]\npub struct Kept;\npub(crate) fn inner() {}\npub async fn run() {}\npub const fn make() {}\npub mod parts;\npub mod inline {}\n#[cfg(test)]\nmod tests { pub fn t() {} }\n";
        assert_eq!(names("rs", text), ["fn:bare", "fn:run", "fn:make", "mod:inline"]);
    }

    #[test]
    fn go_python_and_js() {
        assert_eq!(
            names(
                "go",
                "// Said says.\nfunc Said() {}\nfunc Bare() {}\nfunc private() {}\nfunc (s *T) Method() {}\ntype Thing struct{}\n"
            ),
            ["func:Bare", "func:Method", "type:Thing"]
        );
        assert_eq!(
            names(
                "py",
                "def said():\n    \"\"\"Says.\"\"\"\n\ndef bare(a,\n         b):\n    return a\n\ndef _hidden():\n    pass\n\nclass Thing:\n    x = 1\n"
            ),
            ["def:bare", "class:Thing"]
        );
        assert_eq!(
            names(
                "ts",
                "/** Said. */\nexport function said() {}\nexport function bare() {}\nexport default class App {}\n/**\n * Kept.\n */\nexport const kept = 1;\nexport interface Shape {}\nconst inner = 2;\n"
            ),
            ["function:bare", "class:App", "interface:Shape"]
        );
        assert!(in_text("py", "pkg/test_x.py", "def bare():\n    pass\n").is_empty());
    }
}
