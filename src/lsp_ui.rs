//! Widgets for language features: completion rows, hover Markdown, and the
//! glyphs and colors they use.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, vec2};
use rust_code_core::lsp::types::CompletionItemKind;

use crate::theme;

/// A one-letter glyph and color for a completion kind (like VS Code's icons).
pub fn kind_glyph(kind: Option<CompletionItemKind>) -> (&'static str, Color32) {
    const PURPLE: Color32 = Color32::from_rgb(0xB1, 0x80, 0xD7);
    const BLUE: Color32 = Color32::from_rgb(0x75, 0xBE, 0xFF);
    const ORANGE: Color32 = Color32::from_rgb(0xEE, 0x9D, 0x28);
    const GRAY: Color32 = Color32::from_gray(0xAA);
    let Some(k) = kind else { return ("·", GRAY) };
    match k {
        CompletionItemKind::FUNCTION => ("f", PURPLE),
        CompletionItemKind::METHOD => ("m", PURPLE),
        CompletionItemKind::CONSTRUCTOR => ("c", PURPLE),
        CompletionItemKind::FIELD => ("F", BLUE),
        CompletionItemKind::PROPERTY => ("p", BLUE),
        CompletionItemKind::VARIABLE => ("v", BLUE),
        CompletionItemKind::CONSTANT => ("C", BLUE),
        CompletionItemKind::STRUCT | CompletionItemKind::CLASS => ("S", ORANGE),
        CompletionItemKind::ENUM => ("E", ORANGE),
        CompletionItemKind::ENUM_MEMBER => ("e", BLUE),
        CompletionItemKind::INTERFACE => ("T", BLUE),
        CompletionItemKind::TYPE_PARAMETER => ("t", ORANGE),
        CompletionItemKind::MODULE => ("M", GRAY),
        CompletionItemKind::KEYWORD => ("k", GRAY),
        CompletionItemKind::SNIPPET => ("s", GRAY),
        _ => ("·", GRAY),
    }
}

/// A small painted icon for a problem severity (the fonts lack these symbols):
/// a circled cross for errors, a triangle for warnings, a dot otherwise.
pub fn severity_icon(ui: &mut egui::Ui, severity: u8, color: Color32) -> egui::Response {
    let size = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint_severity(ui.painter(), rect, severity, color);
    resp
}

fn paint_severity(p: &egui::Painter, rect: Rect, severity: u8, color: Color32) {
    let c = rect.center();
    let r = rect.height() * 0.36;
    let stroke = Stroke::new(1.3, color);
    match severity {
        1 => {
            p.circle_stroke(c, r, stroke);
            let d = r * 0.45;
            p.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
            p.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
        }
        2 => {
            let top = Pos2::new(c.x, c.y - r * 1.05);
            let points = vec![top, Pos2::new(c.x + r * 1.1, c.y + r * 0.85), Pos2::new(c.x - r * 1.1, c.y + r * 0.85)];
            p.add(egui::Shape::closed_line(points, stroke));
            p.line_segment([Pos2::new(c.x, c.y - r * 0.4), Pos2::new(c.x, c.y + r * 0.2)], stroke);
            p.circle_filled(Pos2::new(c.x, c.y + r * 0.55), 0.8, color);
        }
        _ => {
            p.circle_filled(c, r * 0.7, color);
        }
    }
}

/// Renders the subset of Markdown servers use in hovers: fenced code blocks,
/// headings, rules, and paragraphs (inline markup is stripped).
pub fn markdown(ui: &mut egui::Ui, text: &str, code_size: f32) {
    let pal = theme::colors();
    let mut code: Option<String> = None;
    let mut paragraph = String::new();
    let flush = |ui: &mut egui::Ui, paragraph: &mut String| {
        if !paragraph.is_empty() {
            ui.label(strip_inline(paragraph.trim()));
            paragraph.clear();
        }
    };
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            match code.take() {
                Some(block) => {
                    egui::Frame::new().fill(pal.editor_bg).inner_margin(4.0).corner_radius(3.0).show(ui, |ui| {
                        ui.label(RichText::new(block.trim_end()).monospace().size(code_size).color(pal.editor_fg));
                    });
                }
                None => {
                    flush(ui, &mut paragraph);
                    code = Some(String::new());
                }
            }
            continue;
        }
        if let Some(block) = &mut code {
            block.push_str(line);
            block.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            flush(ui, &mut paragraph);
            continue;
        }
        if matches!(trimmed, "---" | "***" | "___") {
            flush(ui, &mut paragraph);
            ui.separator();
            continue;
        }
        if trimmed.starts_with('#') {
            flush(ui, &mut paragraph);
            ui.label(RichText::new(strip_inline(trimmed.trim_start_matches('#').trim())).strong());
            continue;
        }
        if !paragraph.is_empty() {
            paragraph.push(' ');
        }
        paragraph.push_str(trimmed);
    }
    flush(ui, &mut paragraph);
    if let Some(block) = code {
        ui.label(RichText::new(block.trim_end()).monospace().size(code_size));
    }
}

/// Drops inline Markdown: `**bold**`, `_em_`, `` `code` `` and `[text](link)` → text,
/// and images altogether. Decodes the entities servers escape text with.
fn strip_inline(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' | '`' => {}
            '!' if chars.get(i + 1) == Some(&'[') => {
                let close = chars[i..].iter().position(|&c| c == ']').map(|p| p + i);
                let end = close
                    .filter(|c| chars.get(c + 1) == Some(&'('))
                    .and_then(|c| chars[c..].iter().position(|&x| x == ')').map(|p| p + c));
                match end {
                    Some(end) => i = end,
                    None => out.push('!'),
                }
            }
            // Emphasis, but not the middle of snake_case.
            '_' if !(i > 0
                && chars[i - 1].is_alphanumeric()
                && chars.get(i + 1).is_some_and(|c| c.is_alphanumeric())) => {}
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 1;
            }
            '[' => {
                // [text](target): keep the text.
                let close = chars[i..].iter().position(|&c| c == ']').map(|p| p + i);
                if let Some(close) = close
                    && chars.get(close + 1) == Some(&'(')
                    && let Some(end) = chars[close..].iter().position(|&c| c == ')').map(|p| p + close)
                {
                    out.extend(&chars[i + 1..close]);
                    i = end;
                } else {
                    out.push('[');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::strip_inline;

    #[test]
    fn strips_inline_markdown() {
        assert_eq!(strip_inline("**Note**: use `Vec::new` [docs](https://x/y)"), "Note: use Vec::new docs");
        assert_eq!(strip_inline("a \\* b [not a link"), "a * b [not a link");
        assert_eq!(strip_inline("![icon](data:x) _Widely_ snake_case"), " Widely snake_case");
        assert_eq!(strip_inline("&lt;color&gt; &amp;lt;"), "<color> &lt;");
        assert_eq!(strip_inline("wow! [a](b)"), "wow! a");
    }
}
