//! Colors languages with no grammar here, by reading their words.

use std::collections::HashMap;
use std::sync::Arc;

use coiman_core::light_syntax::{self, Kind};
use coiman_core::theme;

use super::CodeApp;
use crate::editor_view::{Decorations, Highlight};

type Lines = Arc<HashMap<usize, Vec<Highlight>>>;

#[derive(Default)]
pub(super) struct LightColors {
    /// By tab: the text's version and theme it's for, and its colors.
    done: HashMap<u64, (u64, u64, Lines)>,
}

fn capture(kind: Kind) -> &'static str {
    match kind {
        Kind::Comment => "comment",
        Kind::Text => "string",
        Kind::Number => "number",
        Kind::Keyword => "keyword",
        Kind::Operator => "operator",
    }
}

impl CodeApp {
    pub(super) fn light_colors_decorate(&mut self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i) else { return };
        let extension = doc.extension();
        if doc.large || !light_syntax::colors(&extension) {
            return;
        }
        let (uid, version, generation) = (doc.uid, doc.buffer.version(), theme::generation());
        let known = self.light_colors.done.get(&uid).filter(|(v, g, _)| (*v, *g) == (version, generation));
        if let Some((.., lines)) = known {
            deco.highlights.push(lines.clone());
            return;
        }
        let active = theme::current();
        let color =
            |kind: Kind| theme::capture_style(&active, capture(kind)).map(|(c, italic)| (crate::theme::c32(c), italic));
        let mut lines: HashMap<usize, Vec<Highlight>> = HashMap::new();
        for token in light_syntax::tokens(&extension, &doc.text()) {
            let Some((foreground, italic)) = color(token.kind) else { continue };
            lines.entry(token.line).or_default().push(Highlight {
                start: token.start,
                end: token.end,
                foreground: Some(foreground),
                background: None,
                italic,
                strike: false,
                frame: None,
            });
        }
        let lines = Arc::new(lines);
        self.light_colors.done.insert(uid, (version, generation, lines.clone()));
        deco.highlights.push(lines);
    }
}
