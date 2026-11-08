//! Comment Highlights: TODO-style tags (like TODO Highlight) and comment kinds
//! (like Better Comments), in any language with line comments.

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

use coiman::plugin::host;
use coiman::plugin::types::Decoration;

const MAX_BYTES: usize = 2_000_000;
const TAGS: [(&str, u32, u32); 6] = [
    ("TODO", 0x1E1E1E, 0xFFBD2A),
    ("FIXME", 0xFFFFFF, 0xF14C4C),
    ("BUG", 0xFFFFFF, 0xF14C4C),
    ("HACK", 0x1E1E1E, 0xE8A33D),
    ("XXX", 0x1E1E1E, 0xE8A33D),
    ("NOTE", 0xFFFFFF, 0x3794FF),
];

/// Where a line's comment starts, in chars, skipping strings.
fn comment_start(chars: &[char], tokens: &[&str]) -> Option<usize> {
    let mut quote = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None if c == '"' || c == '`' => quote = Some(c),
            None => {
                let starts = |t: &&str| t.chars().enumerate().all(|(n, tc)| chars.get(i + n) == Some(&tc));
                if tokens.iter().any(|t| starts(t)) {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

fn tokens_for(language: &str) -> &'static [&'static str] {
    match language {
        "Python" | "Shell Script" | "PowerShell" | "TOML" | "YAML" | "Ruby" | "R" => &["#"],
        "SQL" | "Lua" => &["--"],
        "HTML" | "XML" | "CSS" | "Markdown" | "Plain Text" | "JSON" => &[],
        _ => &["//"],
    }
}

fn decorate_comment(chars: &[char], start: usize, line: u32, items: &mut Vec<Decoration>) {
    let end = chars.len() as u32;
    let mut body = start;
    while body < chars.len() && matches!(chars[body], '/' | '#' | '-') {
        body += 1;
    }
    while body < chars.len() && chars[body].is_whitespace() {
        body += 1;
    }
    let kind = match chars.get(body) {
        Some('!') => Some(0xFF5C5C),
        Some('?') => Some(0x3794FF),
        Some('*') => Some(0x98C379),
        _ => None,
    };
    if let Some(color) = kind {
        items.push(Decoration {
            line,
            start: start as u32,
            end,
            foreground: Some(color),
            background: None,
            italic: false,
        });
    }
    let text: String = chars[start..].iter().collect();
    for (tag, fg, bg) in TAGS {
        let Some(byte) = text.find(tag) else { continue };
        let at = start + text[..byte].chars().count();
        let after = chars.get(at + tag.len()).copied().unwrap_or(' ');
        if !after.is_alphanumeric() && (at == 0 || !chars[at - 1].is_alphanumeric()) {
            items.push(Decoration {
                line,
                start: at as u32,
                end: (at + tag.len()) as u32,
                foreground: Some(fg),
                background: Some(bg),
                italic: false,
            });
        }
    }
}

fn update() {
    let Ok(Some(doc)) = host::active_document() else { return };
    let mut items = Vec::new();
    let tokens = tokens_for(&doc.language);
    if doc.text.len() <= MAX_BYTES && !tokens.is_empty() {
        for (n, line) in doc.text.lines().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let Some(start) = comment_start(&chars, tokens) else { continue };
            decorate_comment(&chars, start, n as u32 + 1, &mut items);
        }
    }
    host::set_decorations("", &items);
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        update();
        Ok(())
    }

    fn run_command(_: String, _: String) -> Result<(), String> {
        Ok(())
    }

    fn on_event(_: Event) {
        update();
    }
}

export!(Plugin);
