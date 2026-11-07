//! Color Preview: paints color literals in their own color (like Color Highlight).

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

use rust_code::plugin::host;
use rust_code::plugin::types::Decoration;

const MAX_BYTES: usize = 2_000_000;

fn hex(chars: &[char], at: usize) -> Option<(usize, u32)> {
    let digits = chars[at + 1..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
    let text: String = chars[at + 1..at + 1 + digits].iter().collect();
    // Only web color forms; `#` also starts comments and Rust attributes.
    let value = match digits {
        3 | 4 => {
            let v: Vec<u32> = text.chars().take(3).filter_map(|c| c.to_digit(16)).collect();
            (v[0] * 17) << 16 | (v[1] * 17) << 8 | (v[2] * 17)
        }
        6 | 8 => u32::from_str_radix(&text[..6], 16).ok()?,
        _ => return None,
    };
    Some((digits + 1, value))
}

fn rgb(chars: &[char], at: usize) -> Option<(usize, u32)> {
    let text: String = chars[at..].iter().take(40).collect();
    let open = if text.starts_with("rgb(") {
        4
    } else if text.starts_with("rgba(") {
        5
    } else {
        return None;
    };
    let close = text.find(')')?;
    let parts: Vec<u32> = text[open..close]
        .split([',', ' ', '/'])
        .filter(|p| !p.is_empty())
        .take(3)
        .filter_map(|p| p.parse().ok())
        .collect();
    if parts.len() != 3 || parts.iter().any(|p| *p > 255) {
        return None;
    }
    Some((text[..=close].chars().count(), parts[0] << 16 | parts[1] << 8 | parts[2]))
}

/// Text color readable on `background`.
fn contrast(background: u32) -> u32 {
    let (r, g, b) = ((background >> 16) & 255, (background >> 8) & 255, background & 255);
    if r * 299 + g * 587 + b * 114 > 128_000 { 0x000000 } else { 0xFFFFFF }
}

fn update() {
    let Ok(Some(doc)) = host::active_document() else { return };
    let mut items = Vec::new();
    if doc.text.len() <= MAX_BYTES {
        for (n, line) in doc.text.lines().enumerate() {
            if !line.contains('#') && !line.contains("rgb") {
                continue;
            }
            let chars: Vec<char> = line.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let boundary = i == 0 || !chars[i - 1].is_alphanumeric();
                let found = match chars[i] {
                    '#' if boundary => hex(&chars, i),
                    'r' if boundary => rgb(&chars, i),
                    _ => None,
                };
                let Some((len, color)) = found else {
                    i += 1;
                    continue;
                };
                items.push(Decoration {
                    line: n as u32 + 1,
                    start: i as u32,
                    end: (i + len) as u32,
                    foreground: Some(contrast(color)),
                    background: Some(color),
                    italic: false,
                });
                i += len;
            }
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
