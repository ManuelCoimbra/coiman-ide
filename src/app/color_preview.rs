//! Colors written in a file shown on themselves: the text of `#e53935`, `rgb(...)` or
//! `tomato` sits on that color, in black or white, whichever reads.

use std::collections::HashMap;
use std::sync::Arc;

use egui::Color32;
use rust_code_core::{css, theme};

use super::CodeApp;
use crate::editor_view::{Decorations, Highlight};

type Lines = Arc<HashMap<usize, Vec<Highlight>>>;

#[derive(Default)]
pub(super) struct ColorPreview {
    /// By tab: the text's version and theme it's for, and its chips.
    done: HashMap<u64, (u64, u64, Lines)>,
}

/// A color as drawn: see-through colors over the editor's background.
fn shown(color: &css::Color, under: Color32) -> Color32 {
    let [r, g, b, a] = color.srgb8();
    let alpha = f32::from(a) / 255.0;
    let mix = |c: u8, u: u8| (f32::from(c) * alpha + f32::from(u) * (1.0 - alpha)).round() as u8;
    Color32::from_rgb(mix(r, under.r()), mix(g, under.g()), mix(b, under.b()))
}

/// Whether two colors are too alike to tell apart (contrast under 1.5:1), so a chip
/// needs a frame: black on a dark editor, white on a light one.
fn near(a: Color32, b: Color32) -> bool {
    let luminance = |c: Color32| {
        let linear = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
    };
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05) < 1.5
}

/// Chips by line, as char columns within the line.
fn chips(extension: &str, text: &str, under: Color32) -> HashMap<usize, Vec<Highlight>> {
    let found = css::colors_for(extension, text);
    let mut lines: HashMap<usize, Vec<Highlight>> = HashMap::new();
    if found.is_empty() {
        return lines;
    }
    // Char offset of each line's start.
    let mut starts = vec![0];
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    for color in found {
        let line = starts.partition_point(|&s| s <= color.start) - 1;
        let line_end = starts.get(line + 1).map_or(usize::MAX, |s| s - 1);
        let background = shown(&color.color, under);
        let [r, g, b] = css::readable_on([background.r(), background.g(), background.b()]);
        lines.entry(line).or_default().push(Highlight {
            start: color.start - starts[line],
            end: color.end.min(line_end) - starts[line],
            foreground: Some(Color32::from_rgb(r, g, b)),
            background: Some(background),
            italic: false,
            strike: false,
            frame: near(background, under).then(|| crate::theme::colors().text_dim),
        });
    }
    lines
}

impl CodeApp {
    pub(super) fn color_preview_decorate(&mut self, i: usize, deco: &mut Decorations) {
        if !self.settings.editor.color_decorators {
            return;
        }
        let Some(doc) = self.docs.get(i) else { return };
        if doc.large {
            return;
        }
        let (uid, version, generation) = (doc.uid, doc.buffer.version(), theme::generation());
        let known = self.color_preview.done.get(&uid).filter(|(v, g, _)| (*v, *g) == (version, generation));
        if let Some((.., lines)) = known {
            deco.highlights.push(lines.clone());
            return;
        }
        let lines = Arc::new(chips(&doc.extension(), &doc.text(), crate::theme::colors().editor_bg));
        self.color_preview.done.insert(uid, (version, generation, lines.clone()));
        deco.highlights.push(lines);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_sit_on_their_color_in_readable_text() {
        let under = Color32::from_rgb(30, 30, 30);
        let lines = chips("css", "a {\n  color: red;\n  background: #ffff00;\n}\n", under);
        let red = &lines[&1][0];
        assert_eq!((red.start, red.end), (9, 12));
        assert_eq!(red.background, Some(Color32::from_rgb(255, 0, 0)));
        assert_eq!(red.foreground, Some(Color32::WHITE), "white on red");
        let yellow = &lines[&2][0];
        assert_eq!(yellow.foreground, Some(Color32::BLACK), "black on yellow");
        // Half see-through black over a dark editor is drawn as it shows.
        let faint = chips("css", "a { color: rgb(0 0 0 / 50%) }", under);
        assert_eq!(faint[&0][0].background, Some(Color32::from_rgb(15, 15, 15)));
        // Code: colors in strings only.
        let code = chips("ts", "const a = '#ff0000'; // #00ff00\nconst b = \"blue\";\n", under);
        assert_eq!(code[&0].len(), 1, "not the one in the comment");
        assert_eq!(code[&1][0].background, Some(Color32::from_rgb(0, 0, 255)));
        assert!(chips("rs", "#[derive(Debug)]\nfn add() {}\n", under).is_empty());
        // Black on a dark editor gets a frame; red doesn't need one.
        let black = chips("css", "a { color: black }", under);
        assert!(black[&0][0].frame.is_some());
        assert!(lines[&1][0].frame.is_none());
    }
}
