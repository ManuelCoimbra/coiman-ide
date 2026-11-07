//! Baseline: which web features work across the major browsers, and since when.
//! The table is generated from web-features (`scripts/gen-baseline.py`) and ships
//! with the editor, so nothing is looked up over the network.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::language::css::{self, SegmentKind};

const TABLE: &str = include_str!("../../data/baseline.tsv");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// Interoperable for long enough to count as widely available.
    Widely,
    /// Newly interoperable.
    Newly,
    /// Missing from at least one major browser.
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub level: Level,
    /// `YYYY-MM-DD` it became newly available, if it has.
    pub since: &'static str,
    /// `YYYY-MM-DD` it became widely available, if it has.
    pub widely_since: &'static str,
    pub name: &'static str,
}

fn table() -> &'static HashMap<&'static str, Entry> {
    static PARSED: OnceLock<HashMap<&'static str, Entry>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TABLE
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|line| {
                let mut f = line.split('\t');
                let key = f.next()?;
                let level = match f.next()? {
                    "high" => Level::Widely,
                    "low" => Level::Newly,
                    _ => Level::Limited,
                };
                Some((key, Entry { level, since: f.next()?, widely_since: f.next()?, name: f.next()? }))
            })
            .collect()
    })
}

pub fn lookup(key: &str) -> Option<Entry> {
    table().get(key).copied()
}

/// What a project promises to support, from `web.baseline`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Only what is widely available.
    Widely,
    /// Anything in all the major browsers.
    Newly,
    /// What was in all of them by the end of this year.
    Year(u32),
}

impl Target {
    /// `widely`, `newly` or a year. Anything else, including empty, is no target.
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "widely" => Some(Self::Widely),
            "newly" => Some(Self::Newly),
            year => year.parse().ok().filter(|y| (2015..=2100).contains(y)).map(Self::Year),
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Widely => "Baseline Widely available".to_owned(),
            Self::Newly => "Baseline Newly available".to_owned(),
            Self::Year(y) => format!("Baseline {y}"),
        }
    }
}

impl Entry {
    fn year(&self) -> Option<u32> {
        self.since.get(..4)?.parse().ok()
    }

    pub fn meets(&self, target: Target) -> bool {
        match target {
            Target::Widely => self.level == Level::Widely,
            Target::Newly => self.level != Level::Limited,
            Target::Year(y) => self.level != Level::Limited && self.year().is_some_and(|since| since <= y),
        }
    }

    /// One line for people.
    pub fn describe(&self) -> String {
        let month = |date: &'static str| date.get(..7).unwrap_or(date);
        match self.level {
            Level::Widely => format!("Baseline: widely available since {}", month(self.widely_since)),
            Level::Newly => format!("Baseline: newly available since {}", month(self.since)),
            Level::Limited => "Baseline: limited availability, missing from a major browser".to_owned(),
        }
    }
}

/// A feature used in a text, by char offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub key: String,
    pub start: usize,
    pub end: usize,
}

fn word_end(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
        i += 1;
    }
    i
}

fn css_uses_in(chars: &[char], base: usize, out: &mut Vec<Use>) {
    let mut add = |prefix: &str, start: usize, end: usize| {
        let name: String = chars[start..end].iter().collect::<String>().to_lowercase();
        // Vendor prefixes and custom properties aren't the standard feature.
        if !name.starts_with('-') && !name.is_empty() {
            out.push(Use { key: format!("{prefix}{name}"), start: base + start, end: base + end });
        }
    };
    for segment in css::segments(chars) {
        let (a, b) = (segment.start, segment.end);
        match segment.kind {
            SegmentKind::Prelude if chars[a] == '@' => add("css.at-rules.", a + 1, word_end(chars, a + 1)),
            SegmentKind::Prelude => {
                let mut i = a;
                while i < b {
                    if chars[i] == ':' {
                        while chars.get(i) == Some(&':') {
                            i += 1;
                        }
                        let end = word_end(chars, i);
                        add("css.selectors.", i, end);
                        i = end;
                    } else {
                        i += 1;
                    }
                }
            }
            SegmentKind::Declaration if chars[a] == '@' => add("css.at-rules.", a + 1, word_end(chars, a + 1)),
            SegmentKind::Declaration => {
                let name_end = word_end(chars, a);
                add("css.properties.", a, name_end);
                let property: String = chars[a..name_end].iter().collect::<String>().to_lowercase();
                let values = format!("css.properties.{property}.");
                let mut i = name_end;
                while i < b {
                    if chars[i].is_alphabetic() && !chars[i - 1].is_alphanumeric() && chars[i - 1] != '-' {
                        let end = word_end(chars, i);
                        if chars.get(end) == Some(&'(') {
                            add("css.types.", i, end);
                        } else {
                            add(&values, i, end);
                        }
                        i = end.max(i + 1);
                    } else {
                        i += 1;
                    }
                }
            }
        }
    }
}

/// The features a style sheet uses.
pub fn css_uses(text: &str) -> Vec<Use> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    css_uses_in(&chars, 0, &mut out);
    out
}

/// Where the text `needle` starts at or after `from`, ignoring ASCII case.
fn find_from(chars: &[char], from: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    (from..chars.len().saturating_sub(needle.len() - 1))
        .find(|&i| chars[i..i + needle.len()].iter().zip(&needle).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

/// The features a page uses: its elements, and the CSS in its `<style>` blocks.
pub fn html_uses(text: &str) -> Vec<Use> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(at) = find_from(&chars, i, "<") {
        i = at + 1;
        if chars[i..].starts_with(&['!', '-', '-']) {
            i = find_from(&chars, i, "-->").map_or(chars.len(), |e| e + 3);
            continue;
        }
        if !chars.get(i).is_some_and(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let end = word_end(&chars, i);
        let name: String = chars[i..end].iter().collect::<String>().to_lowercase();
        out.push(Use { key: format!("html.elements.{name}"), start: i, end });
        i = end;
        if name == "script" || name == "style" {
            let open_end = find_from(&chars, i, ">").map_or(chars.len(), |e| e + 1);
            let close = find_from(&chars, open_end, &format!("</{name}")).unwrap_or(chars.len());
            if name == "style" {
                css_uses_in(&chars[open_end..close], open_end, &mut out);
            }
            i = close;
        }
    }
    out
}

/// The features a file uses, if it is a kind the table covers.
pub fn uses_for(extension: &str, text: &str) -> Vec<Use> {
    match extension {
        "css" | "scss" | "less" => css_uses(text),
        "html" | "htm" | "xhtml" => html_uses(text),
        _ => Vec::new(),
    }
}

/// Uses in a file that a project's target doesn't allow.
pub fn below_target(extension: &str, text: &str, target: Target) -> Vec<(Use, Entry)> {
    uses_for(extension, text)
        .into_iter()
        .filter_map(|u| lookup(&u.key).map(|e| (u, e)))
        .filter(|(_, e)| !e.meets(target))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(uses: &[Use]) -> Vec<&str> {
        uses.iter().map(|u| u.key.as_str()).collect()
    }

    #[test]
    fn the_table_reads_and_has_the_kinds_it_should() {
        assert!(table().len() > 500, "{}", table().len());
        for key in ["css.properties.color", "css.selectors.has", "css.at-rules.media", "html.elements.dialog"] {
            assert!(lookup(key).is_some(), "{key}");
        }
        assert!(lookup("css.types.oklch").is_some());
        assert_eq!(lookup("css.properties.color").unwrap().level, Level::Widely);
        assert!(lookup("css.properties.not-a-thing").is_none());
    }

    #[test]
    fn targets_judge_entries() {
        let widely = Entry { level: Level::Widely, since: "2018-01-01", widely_since: "2020-07-01", name: "" };
        let newly = Entry { level: Level::Newly, since: "2024-05-01", widely_since: "", name: "" };
        let limited = Entry { level: Level::Limited, since: "", widely_since: "", name: "" };
        assert!(widely.meets(Target::Widely) && widely.meets(Target::Newly) && widely.meets(Target::Year(2018)));
        assert!(!widely.meets(Target::Year(2017)));
        assert!(!newly.meets(Target::Widely) && newly.meets(Target::Newly));
        assert!(newly.meets(Target::Year(2024)) && !newly.meets(Target::Year(2023)));
        assert!(!limited.meets(Target::Newly) && !limited.meets(Target::Year(2100)));
        assert!(limited.describe().contains("limited"));
        assert!(widely.describe().contains("2020-07"));
    }

    #[test]
    fn parses_targets() {
        assert_eq!(Target::parse("Widely"), Some(Target::Widely));
        assert_eq!(Target::parse(" newly "), Some(Target::Newly));
        assert_eq!(Target::parse("2023"), Some(Target::Year(2023)));
        assert_eq!(Target::parse(""), None);
        assert_eq!(Target::parse("soon"), None);
        assert_eq!(Target::parse("99"), None);
    }

    #[test]
    fn finds_uses_in_style_sheets() {
        let text = "/* grid: x; */\n@container (x) {\n  a:has(b)::before { --mine: 1; color: oklch(0.5 0.1 200); -webkit-foo: 1 }\n}\n";
        let uses = css_uses(text);
        assert_eq!(
            keys(&uses),
            [
                "css.at-rules.container",
                "css.selectors.has",
                "css.selectors.before",
                "css.properties.color",
                "css.types.oklch"
            ]
        );
        let chars: Vec<char> = text.chars().collect();
        let word: String = chars[uses[1].start..uses[1].end].iter().collect();
        assert_eq!(word, "has");
    }

    #[test]
    fn finds_uses_in_pages() {
        let text = "<!-- <dialog> --><p>a < b</p><style>a { gap: 1px }</style><script>let x = '<dialog>';</script><Dialog open>";
        let uses = html_uses(text);
        assert_eq!(
            keys(&uses),
            [
                "html.elements.p",
                "html.elements.style",
                "css.properties.gap",
                "html.elements.script",
                "html.elements.dialog"
            ]
        );
        let chars: Vec<char> = text.chars().collect();
        let gap = uses.iter().find(|u| u.key.ends_with("gap")).unwrap();
        assert_eq!(chars[gap.start..gap.end].iter().collect::<String>(), "gap");
    }
}
