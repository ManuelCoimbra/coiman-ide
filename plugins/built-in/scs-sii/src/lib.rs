//! SCS Units: `.sii` and `.sui` files have the shape
//!
//! ```text
//! SiiNunit
//! {
//! sign_board_template : _nameless.206.92b6.2258 {
//!  board_name: "fr a20 a89 far 1"
//!  width: &40d00000
//!  items[0]: _nameless.206.37bb.3ab8
//! }
//! }
//! ```
//!
//! This colors them, lists their units in a tree view, and reports broken
//! structure: unbalanced braces, a missing `SiiNunit` (not needed in `.sui`
//! include files), duplicate unit names and unterminated strings.

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

use std::collections::HashMap;

use coiman::plugin::host;
use coiman::plugin::types::{Decoration, Problem, Severity, TreeItem};

const MAX_BYTES: usize = 8_000_000;
const CLASS: u32 = 0x4EC9B0;
const NAME: u32 = 0xDCDCAA;
const KEY: u32 = 0x9CDCFE;
const STRING: u32 = 0xCE9178;
const NUMBER: u32 = 0xB5CEA8;
const KEYWORD: u32 = 0x569CD6;
const COMMENT: u32 = 0x6A9955;
const LINK: u32 = 0xC586C0;

fn color(line: u32, start: usize, end: usize, foreground: u32) -> Decoration {
    Decoration {
        line,
        start: start as u32,
        end: end as u32,
        foreground: Some(foreground),
        background: None,
        italic: false,
    }
}

fn problem(line: u32, start: usize, end: usize, severity: Severity, message: String) -> Problem {
    Problem { line, start: start as u32, end: end as u32, severity, message }
}

fn chars_len(text: &str) -> usize {
    text.chars().count()
}

/// Column where a comment starts, outside strings.
fn comment_at(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut previous = ' ';
    for (i, c) in line.chars().enumerate() {
        match c {
            '"' if previous != '\\' => in_string = !in_string,
            '#' if !in_string => return Some(i),
            '/' if !in_string && previous == '/' => return Some(i - 1),
            _ => {}
        }
        previous = c;
    }
    None
}

fn value_color(value: &str) -> u32 {
    if value.starts_with('"') {
        STRING
    } else if value.starts_with('&') || value.starts_with('(') || value.parse::<f64>().is_ok() {
        NUMBER
    } else if matches!(value, "true" | "false" | "nil") {
        KEYWORD
    } else {
        LINK
    }
}

#[derive(Default)]
struct Scan {
    items: Vec<Decoration>,
    problems: Vec<Problem>,
    tree: Vec<TreeItem>,
    /// `class : name` at the top level, which a `{` on the next line makes a unit.
    pending: Option<(u32, String, usize, String, String)>,
    open: Vec<u32>,
    seen: HashMap<String, u32>,
    counts: HashMap<String, usize>,
    has_header: bool,
    block_comment: bool,
}

impl Scan {
    fn unit(&mut self, number: u32, code: &str, indent: usize, class: &str, name: &str) {
        self.open.push(number);
        self.items.push(color(number, indent, indent + chars_len(class), CLASS));
        if let Some(byte) = code.rfind(name).filter(|_| !name.is_empty()) {
            let start = chars_len(&code[..byte]);
            let end = start + chars_len(name);
            self.items.push(color(number, start, end, NAME));
            // `.` names are local and may repeat.
            if !name.starts_with('.') {
                if let Some(first) = self.seen.get(name) {
                    let message = format!("Unit {name} is already defined on line {first}");
                    self.problems.push(problem(number, start, end, Severity::Warning, message));
                } else {
                    self.seen.insert(name.to_owned(), number);
                }
            }
        }
        if !self.counts.contains_key(class) {
            self.tree.push(TreeItem {
                id: class.to_owned(),
                parent: String::new(),
                label: class.to_owned(),
                description: String::new(),
                tooltip: String::new(),
                command: None,
            });
        }
        *self.counts.entry(class.to_owned()).or_default() += 1;
        self.tree.push(TreeItem {
            id: number.to_string(),
            parent: class.to_owned(),
            label: name.to_owned(),
            description: format!("line {number}"),
            tooltip: String::new(),
            command: Some("open".to_owned()),
        });
    }

    fn line(&mut self, number: u32, raw: &str) {
        let total = chars_len(raw);
        if self.block_comment || raw.trim_start().starts_with("/*") {
            self.items.push(color(number, 0, total, COMMENT));
            self.block_comment = !raw.contains("*/");
            return;
        }
        let code_end = comment_at(raw).unwrap_or(total);
        if code_end < total {
            self.items.push(color(number, code_end, total, COMMENT));
        }
        let code: String = raw.chars().take(code_end).collect();
        let trimmed = code.trim();
        let indent = chars_len(&code) - chars_len(code.trim_start());
        if trimmed.is_empty() {
            return;
        }
        let pending = self.pending.take();
        if trimmed == "SiiNunit" {
            self.has_header = true;
            self.items.push(color(number, indent, indent + 8, KEYWORD));
            return;
        }
        if trimmed.starts_with("@include") {
            self.items.push(color(number, indent, indent + 8, KEYWORD));
            if let Some(quote) = code.find('"') {
                self.items.push(color(number, chars_len(&code[..quote]), code_end, STRING));
            }
            return;
        }
        if trimmed.chars().filter(|c| *c == '"').count() % 2 == 1 {
            let message = "This string has no closing quote".to_owned();
            self.problems.push(problem(number, indent, code_end, Severity::Error, message));
        }
        if trimmed == "{" {
            match pending {
                Some((line, code, indent, class, name)) => self.unit(line, &code, indent, &class, &name),
                None => self.open.push(number),
            }
            return;
        }
        if trimmed == "}" {
            if self.open.pop().is_none() {
                let message = "This } closes nothing".to_owned();
                self.problems.push(problem(number, indent, indent + 1, Severity::Error, message));
            }
            return;
        }
        if let Some(body) = trimmed.strip_suffix('{')
            && let Some((class, name)) = body.split_once(':')
        {
            self.unit(number, &code, indent, class.trim(), name.trim());
            return;
        }
        if let Some(byte) = code.find(':') {
            let key = code[..byte].trim();
            if self.open.len() == 1 {
                let name = code[byte + 1..].trim().to_owned();
                self.pending = Some((number, code.clone(), indent, key.to_owned(), name));
            }
            self.items.push(color(number, indent, indent + chars_len(key), KEY));
            let after = &code[byte + 1..];
            let value = after.trim();
            if !value.is_empty() {
                let start = chars_len(&code[..=byte]) + chars_len(after) - chars_len(after.trim_start());
                self.items.push(color(number, start, start + chars_len(value), value_color(value)));
            }
        } else if self.open.len() >= 2 {
            let message = "Expected `name: value`".to_owned();
            self.problems.push(problem(number, indent, code_end, Severity::Warning, message));
        }
    }
}

fn update() {
    let Ok(Some(doc)) = host::active_document() else { return };
    if doc.language != "SCS Unit" || doc.text.len() > MAX_BYTES {
        host::set_decorations("", &[]);
        host::set_problems("", &[]);
        host::set_tree_items("units", &[]);
        return;
    }
    let mut scan = Scan::default();
    for (n, raw) in doc.text.lines().enumerate() {
        scan.line(n as u32 + 1, raw);
    }
    if !scan.has_header && !doc.path.ends_with(".sui") {
        scan.problems.push(problem(1, 0, 1, Severity::Warning, "A .sii file starts with SiiNunit".to_owned()));
    }
    if let Some(open) = scan.open.last() {
        let message = format!("The {{ on line {open} is never closed");
        scan.problems.push(problem(*open, 0, 1, Severity::Error, message));
    }
    for item in scan.tree.iter_mut().filter(|t| t.parent.is_empty()) {
        item.description = scan.counts.get(&item.id).copied().unwrap_or(0).to_string();
    }
    host::set_decorations("", &scan.items);
    host::set_problems("", &scan.problems);
    host::set_tree_items("units", &scan.tree);
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        update();
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        if id != "open" {
            return Err(format!("unknown command {id}"));
        }
        let Some(doc) = host::active_document()? else { return Ok(()) };
        host::open_location(&doc.path, argument.parse().unwrap_or(1))
    }

    fn on_event(_: Event) {
        update();
    }
}

export!(Plugin);
