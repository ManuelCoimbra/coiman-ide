//! Finds characters that change how code reads without showing up: bidi
//! controls (Trojan Source), invisible characters and Private Use Area
//! characters (GlassWorm hid its payload in these).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bidi,
    Invisible,
    PrivateUse,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Bidi => "bidirectional control",
            Kind::Invisible => "invisible character",
            Kind::PrivateUse => "private use character",
        }
    }
}

/// What `c` is, whatever surrounds it.
fn always(c: char) -> Option<Kind> {
    match c {
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' => Some(Kind::Bidi),
        '\u{034F}'
        | '\u{115F}'
        | '\u{1160}'
        | '\u{17B4}'
        | '\u{17B5}'
        | '\u{180E}'
        | '\u{200B}'
        | '\u{2028}'
        | '\u{2029}'
        | '\u{2060}'..='\u{2064}'
        | '\u{3164}'
        | '\u{FEFF}'
        | '\u{FFA0}'
        | '\u{E0000}'..='\u{E007F}'
        | '\u{E0100}'..='\u{E01EF}' => Some(Kind::Invisible),
        '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}' => Some(Kind::PrivateUse),
        _ => None,
    }
}

/// What `c` is when only ASCII surrounds it. These do real work in emoji and
/// in Arabic, Persian and Indic text, so they count only in plain code.
fn beside_ascii(c: char) -> Option<Kind> {
    match c {
        '\u{061C}' | '\u{200E}' | '\u{200F}' => Some(Kind::Bidi),
        '\u{00AD}' | '\u{200C}' | '\u{200D}' => Some(Kind::Invisible),
        _ => None,
    }
}

/// The suspicious characters of one line: (char column, character, kind).
pub fn scan_line(line: &str) -> Vec<(usize, char, Kind)> {
    let mut found = Vec::new();
    if line.is_ascii() {
        return found;
    }
    let chars: Vec<char> = line.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let plain = |n: Option<&char>| n.is_none_or(char::is_ascii);
        let kind = always(c).or_else(|| {
            let around = plain(i.checked_sub(1).and_then(|p| chars.get(p))) && plain(chars.get(i + 1));
            beside_ascii(c).filter(|_| around)
        });
        if let Some(kind) = kind {
            found.push((i, c, kind));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<(usize, Kind)> {
        scan_line(line).into_iter().map(|(i, _, k)| (i, k)).collect()
    }

    #[test]
    fn finds_what_hides() {
        assert_eq!(kinds("if a\u{202E} {"), [(4, Kind::Bidi)]);
        assert_eq!(kinds("é\u{2066}"), [(1, Kind::Bidi)]);
        assert_eq!(kinds("a\u{200B}b"), [(1, Kind::Invisible)]);
        assert_eq!(kinds("x\u{E0101}"), [(1, Kind::Invisible)]);
        assert_eq!(kinds("\u{E000}\u{10FFFD}"), [(0, Kind::PrivateUse), (1, Kind::PrivateUse)]);
        assert_eq!(kinds("a\u{200D}b"), [(1, Kind::Invisible)]);
        assert_eq!(kinds("a\u{200F}"), [(1, Kind::Bidi)]);
    }

    #[test]
    fn leaves_real_text_alone() {
        assert!(kinds("plain ascii\r\n").is_empty());
        assert!(kinds("olá, 日本語, Привет").is_empty());
        // Emoji sequences and Persian joiners.
        assert!(kinds("👩\u{200D}💻 ❤\u{FE0F}").is_empty());
        assert!(kinds("می\u{200C}خواهم").is_empty());
        assert!(kinds("שלום\u{200F}").is_empty());
    }
}
