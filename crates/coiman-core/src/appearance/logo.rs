//! Coiman's logo in the color theme's accent: the blue of `assets/icon/coiman.svg` becomes the
//! accent, and its navy hexagon a deep shade of it. The white C stays white.

use crate::appearance::theme::Color;

const SVG: &str = include_str!("../../../../assets/icon/coiman.svg");
const BLUE: &str = "#2F8CFF";
const NAVY_TOP: &str = "#13224A";
const NAVY_BOTTOM: &str = "#0A1430";

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// `c` taken toward black: `keep` of it is left.
fn darker(c: Color, keep: f32) -> Color {
    let f = |v: u8| (v as f32 * keep).round() as u8;
    Color::from_rgb(f(c.r), f(c.g), f(c.b))
}

/// The logo's SVG with `accent` in place of its blue.
pub fn svg(accent: Color) -> String {
    let accent = Color::from_rgb(accent.r, accent.g, accent.b);
    SVG.replace(BLUE, &hex(accent))
        .replace(NAVY_TOP, &hex(darker(accent, 0.26)))
        .replace(NAVY_BOTTOM, &hex(darker(accent, 0.13)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_takes_the_accent() {
        assert!(SVG.contains(BLUE) && SVG.contains(NAVY_TOP) && SVG.contains(NAVY_BOTTOM), "the icon changed");
        let green = svg(Color::from_rgb(0x57, 0xC0, 0x6A));
        assert!(!green.contains(BLUE) && green.contains("#57C06A"));
        assert!(green.contains("#17321C") && green.contains("#0B190E"), "a deep green hexagon");
        assert!(green.contains("#FFFFFF"), "the C stays white");
    }
}
