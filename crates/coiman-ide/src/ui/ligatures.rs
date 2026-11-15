//! Font ligatures in the editor (`->`, `!=`, `=>` drawn as the font joins them), for fonts that
//! have them: Fira Code, JetBrains Mono, Cascadia Code and the like.
//!
//! egui shapes each section of a laid-out row with the font's own rules, so the font's
//! ligatures are drawn by egui itself, and each character still keeps its cell. Turning them
//! off, or leaving some out, is done by ending a section between the characters that would
//! join: the shaper never joins across sections.

use std::sync::RwLock;

/// Character runs never joined (`editor.font_ligatures_off`).
static OFF: RwLock<Vec<Vec<char>>> = RwLock::new(Vec::new());

/// The ligatures to leave out, by the characters they join: `["==", "!="]`.
pub fn set_off(off: &[String]) {
    let off = off.iter().filter(|s| s.chars().count() > 1).map(|s| s.chars().collect()).collect();
    if let Ok(mut slot) = OFF.write() {
        *slot = off;
    }
}

/// Where a section has to start (before which characters of `chars`) so the font joins only
/// what it should: nothing when `enabled` is off, else nothing that's left off.
pub fn breaks(chars: &[char], enabled: bool) -> Vec<usize> {
    let off = OFF.read().map(|o| o.clone()).unwrap_or_default();
    breaks_with(chars, enabled, &off)
}

fn breaks_with(chars: &[char], enabled: bool, off: &[Vec<char>]) -> Vec<usize> {
    let mut at: Vec<usize> = Vec::new();
    if !enabled {
        // Code fonts join punctuation (`->`, `!=`, `/*`) and a few letter runs (`www`): a
        // break between any two marks, and inside every run of a repeated letter.
        for i in 1..chars.len() {
            let (a, b) = (chars[i - 1], chars[i]);
            let marks = !a.is_alphanumeric() && !a.is_whitespace() && !b.is_alphanumeric() && !b.is_whitespace();
            if marks || (a == b && a.is_alphabetic()) {
                at.push(i);
            }
        }
        return at;
    }
    for seq in off {
        for start in (0..chars.len()).filter(|&s| chars[s..].starts_with(seq)) {
            at.extend(start + 1..start + seq.len());
        }
    }
    at.sort_unstable();
    at.dedup();
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn off_breaks_between_marks() {
        // "a -> b != c": marks at 2,3 and 7,8.
        assert_eq!(breaks_with(&chars("a -> b != c"), false, &[]), [3, 8]);
        assert_eq!(breaks_with(&chars("www.x"), false, &[]), [1, 2]);
        assert!(breaks_with(&chars("plain words"), false, &[]).is_empty());
    }

    #[test]
    fn left_off_ligatures_break_inside() {
        let off = [chars("=="), chars("!=")];
        assert_eq!(breaks_with(&chars("a == b"), true, &off), [3]);
        // `===` holds `==` twice: every character apart, so no half is joined.
        assert_eq!(breaks_with(&chars("a === b"), true, &off), [3, 4]);
        assert_eq!(breaks_with(&chars("a != b -> c"), true, &off), [3]);
        assert!(breaks_with(&chars("a -> b"), true, &off).is_empty());
        assert!(breaks_with(&chars("a == b"), true, &[]).is_empty());
    }
}
