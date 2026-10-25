//! Text encodings: detection when a file is opened and encoding when it is
//! saved, so files keep the encoding (and BOM) they were written with.

use encoding_rs::{
    BIG5, EUC_JP, EUC_KR, Encoding, GBK, ISO_8859_2, ISO_8859_15, KOI8_R, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE,
    WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1253, WINDOWS_1254,
};

/// Encodings offered in the pickers: (display name, VS Code setting id, encoding, BOM).
pub const CHOICES: &[(&str, &str, &Encoding, bool)] = &[
    ("UTF-8", "utf8", UTF_8, false),
    ("UTF-8 with BOM", "utf8bom", UTF_8, true),
    ("UTF-16 LE", "utf16le", UTF_16LE, true),
    ("UTF-16 BE", "utf16be", UTF_16BE, true),
    ("Western (Windows 1252)", "windows1252", WINDOWS_1252, false),
    ("Western (ISO 8859-15)", "iso885915", ISO_8859_15, false),
    ("Central European (Windows 1250)", "windows1250", WINDOWS_1250, false),
    ("Central European (ISO 8859-2)", "iso88592", ISO_8859_2, false),
    ("Cyrillic (Windows 1251)", "windows1251", WINDOWS_1251, false),
    ("Cyrillic (KOI8-R)", "koi8r", KOI8_R, false),
    ("Greek (Windows 1253)", "windows1253", WINDOWS_1253, false),
    ("Turkish (Windows 1254)", "windows1254", WINDOWS_1254, false),
    ("Japanese (Shift JIS)", "shiftjis", SHIFT_JIS, false),
    ("Japanese (EUC-JP)", "eucjp", EUC_JP, false),
    ("Simplified Chinese (GBK)", "gbk", GBK, false),
    ("Traditional Chinese (Big5)", "big5", BIG5, false),
    ("Korean (EUC-KR)", "euckr", EUC_KR, false),
];

/// Text decoded from bytes, with the encoding it was read as.
pub struct Decoded {
    pub text: String,
    pub encoding: &'static Encoding,
    pub bom: bool,
}

/// Detects the encoding: a BOM wins, then valid UTF-8, otherwise Windows-1252
/// (which maps every byte, so nothing is lost and the file can be reopened in
/// the right encoding).
/// Takes the bytes by value so valid UTF-8 (the common case) needs no copy.
pub fn detect_and_decode(bytes: Vec<u8>) -> Decoded {
    if let Some((enc, bom_len)) = Encoding::for_bom(&bytes) {
        let (text, _) = enc.decode_without_bom_handling(&bytes[bom_len..]);
        return Decoded { text: text.into_owned(), encoding: enc, bom: true };
    }
    match String::from_utf8(bytes) {
        Ok(text) => Decoded { text, encoding: UTF_8, bom: false },
        Err(e) => {
            let (text, _) = WINDOWS_1252.decode_without_bom_handling(e.as_bytes());
            Decoded { text: text.into_owned(), encoding: WINDOWS_1252, bom: false }
        }
    }
}

/// Decodes as `enc`, stripping (and recording) a BOM for that encoding.
pub fn decode_with(bytes: &[u8], enc: &'static Encoding) -> Decoded {
    let bom = Encoding::for_bom(bytes).filter(|(e, _)| *e == enc);
    let body = bom.map_or(bytes, |(_, len)| &bytes[len..]);
    let (text, _) = enc.decode_without_bom_handling(body);
    Decoded { text: text.into_owned(), encoding: enc, bom: bom.is_some() }
}

/// Encodes `text`. Fails (naming the first offending character) if the text
/// contains characters the encoding cannot represent, instead of silently
/// writing replacements.
pub fn encode(text: &str, enc: &'static Encoding, bom: bool) -> Result<Vec<u8>, String> {
    // encoding_rs only decodes UTF-16 (per the WHATWG spec), so encode it here.
    if enc == UTF_16LE || enc == UTF_16BE {
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        let le = enc == UTF_16LE;
        if bom {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if le { unit.to_le_bytes() } else { unit.to_be_bytes() });
        }
        return Ok(out);
    }
    if enc == UTF_8 {
        let mut out = Vec::with_capacity(text.len() + 3);
        if bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.extend_from_slice(text.as_bytes());
        return Ok(out);
    }
    let (bytes, _, had_errors) = enc.encode(text);
    if had_errors {
        let bad = text.chars().find(|c| enc.encode(&c.to_string()).2).unwrap_or('?');
        let name = display_name(enc, false);
        return Err(format!("“{bad}” (U+{:04X}) can't be saved in {name}; choose another encoding", bad as u32));
    }
    Ok(bytes.into_owned())
}

/// Status-bar name, e.g. "UTF-8", "UTF-8 with BOM", "Western (Windows 1252)".
pub fn display_name(enc: &'static Encoding, bom: bool) -> String {
    CHOICES
        .iter()
        .find(|(_, _, e, b)| *e == enc && (*b == bom || (enc != UTF_8)))
        .map_or_else(|| enc.name().to_owned(), |(name, _, _, _)| (*name).to_owned())
}

pub fn from_display_name(name: &str) -> Option<(&'static Encoding, bool)> {
    CHOICES.iter().find(|(n, ..)| *n == name).map(|&(_, _, e, b)| (e, b))
}

/// Parses a `files.encoding` setting value (VS Code ids like "utf8bom").
pub fn from_setting(id: &str) -> Option<(&'static Encoding, bool)> {
    let id = id.to_ascii_lowercase().replace(['-', '_', ' '], "");
    CHOICES.iter().find(|(_, i, ..)| *i == id).map(|&(_, _, e, b)| (e, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_bom_utf8_and_falls_back_to_1252() {
        let d = detect_and_decode("héllo".as_bytes().to_vec());
        assert_eq!((d.text.as_str(), d.encoding, d.bom), ("héllo", UTF_8, false));

        let d = detect_and_decode(b"\xEF\xBB\xBFhi".to_vec());
        assert_eq!((d.text.as_str(), d.encoding, d.bom), ("hi", UTF_8, true));

        let d = detect_and_decode(b"\xFF\xFEh\0i\0".to_vec());
        assert_eq!((d.text.as_str(), d.encoding, d.bom), ("hi", UTF_16LE, true));

        let d = detect_and_decode(b"caf\xE9".to_vec());
        assert_eq!((d.text.as_str(), d.encoding), ("café", WINDOWS_1252));
    }

    #[test]
    fn round_trips_every_choice_for_representable_text() {
        for &(name, _, enc, bom) in CHOICES {
            let text = "plain ascii\r\nline two";
            let bytes = encode(text, enc, bom).unwrap();
            let back = decode_with(&bytes, enc);
            assert_eq!(back.text, text, "{name}");
            assert_eq!(back.bom, bom, "{name}");
        }
    }

    #[test]
    fn unrepresentable_characters_are_refused() {
        let err = encode("snowman ☃", WINDOWS_1252, false).unwrap_err();
        assert!(err.contains('☃') && err.contains("Windows 1252"), "{err}");
        assert!(encode("café", WINDOWS_1252, false).is_ok());
    }

    #[test]
    fn names_and_settings() {
        assert_eq!(display_name(UTF_8, true), "UTF-8 with BOM");
        assert_eq!(display_name(UTF_16LE, true), "UTF-16 LE");
        assert_eq!(from_display_name("UTF-8 with BOM"), Some((UTF_8, true)));
        assert_eq!(from_setting("utf8"), Some((UTF_8, false)));
        assert_eq!(from_setting("UTF-16LE"), Some((UTF_16LE, true)));
        assert_eq!(from_setting("nope"), None);
    }
}
