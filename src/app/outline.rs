//! Code structure in the workbench: folding commands, the Outline view, the
//! symbol path in the breadcrumbs, Go to Symbol, expand/shrink selection and
//! syntax text objects (function, type, argument, comment).

use egui::{Color32, RichText, Sense};
use rust_code_core::structure::{self, Symbol, SymbolKind, TextObject};

use crate::buffer::Selection;
use crate::palette::PickerKind;
use crate::theme;

use super::CodeApp;

/// What a folding command does.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FoldAction {
    Fold,
    Unfold,
    Toggle,
    FoldAll,
    UnfoldAll,
}

/// Selections before each Expand Selection, to shrink back:
/// (doc uid, buffer version, stack).
pub(super) type SelectionStack = Option<(u64, u64, Vec<Vec<Selection>>)>;

/// A short colored badge for a symbol kind.
fn kind_badge(kind: SymbolKind) -> (&'static str, Color32) {
    const PURPLE: Color32 = Color32::from_rgb(0xB1, 0x80, 0xD7);
    const ORANGE: Color32 = Color32::from_rgb(0xEE, 0x9D, 0x28);
    const BLUE: Color32 = Color32::from_rgb(0x75, 0xBE, 0xFF);
    const GRAY: Color32 = Color32::from_gray(0xAA);
    match kind {
        SymbolKind::Function => ("f", PURPLE),
        SymbolKind::Struct => ("S", ORANGE),
        SymbolKind::Enum => ("E", ORANGE),
        SymbolKind::Trait => ("T", BLUE),
        SymbolKind::Impl => ("I", BLUE),
        SymbolKind::Module => ("M", GRAY),
        SymbolKind::Constant => ("C", BLUE),
        SymbolKind::TypeAlias => ("t", ORANGE),
        SymbolKind::Macro => ("!", PURPLE),
        SymbolKind::Field | SymbolKind::Key => ("F", BLUE),
        SymbolKind::Variant => ("e", BLUE),
        SymbolKind::Heading => ("#", GRAY),
        SymbolKind::Table => ("[", GRAY),
    }
}

/// The symbols enclosing `line`, outermost first.
fn symbol_path(symbols: &[Symbol], line: usize) -> Vec<&Symbol> {
    let mut path: Vec<&Symbol> = Vec::new();
    for s in symbols.iter().filter(|s| s.start_line <= line && line <= s.end_line) {
        path.retain(|p| p.depth < s.depth);
        path.push(s);
    }
    path
}

impl CodeApp {
    // ----- folding -----------------------------------------------------------------------

    pub(super) fn fold_command(&mut self, action: FoldAction) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let folds = doc.structure().folds.clone();
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let uid = doc.uid;
        let view = self.views.entry(uid).or_default();
        // Regions around the cursor, innermost last (the closing line counts as inside).
        let around: Vec<usize> =
            folds.iter().filter(|f| f.start <= line && line <= f.end + 1).map(|f| f.start).collect();
        match action {
            FoldAction::Fold => {
                if let Some(&start) = around.iter().rev().find(|s| !view.folded().contains(s)) {
                    view.set_folded(start, true);
                }
            }
            FoldAction::Unfold => {
                if let Some(&start) = around.iter().rev().find(|s| view.folded().contains(s)) {
                    view.set_folded(start, false);
                }
            }
            FoldAction::Toggle => {
                if let Some(&start) = around.last() {
                    let folded = view.folded().contains(&start);
                    view.set_folded(start, !folded);
                }
            }
            FoldAction::FoldAll => {
                for f in &folds {
                    view.set_folded(f.start, true);
                }
            }
            FoldAction::UnfoldAll => view.clear_folds(),
        }
        // Keep the cursor visible: move it out of a region that just folded.
        if let Some(f) = folds
            .iter()
            .filter(|f| view.folded().contains(&f.start) && f.start < line && line <= f.end)
            .min_by_key(|f| f.start)
        {
            let pos = doc_line_end(&self.docs[self.active], f.start);
            self.docs[self.active].buffer.set_single(Selection::cursor(pos));
        }
    }

    // ----- outline, breadcrumbs, go to symbol ------------------------------------------

    /// The Outline section of the Explorer: symbols of the active file.
    pub(super) fn outline_view(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let Some(doc) = self.docs.get_mut(self.active) else {
            ui.label(RichText::new("No file is open.").color(pal.text_dim));
            return;
        };
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let symbols = doc.structure().symbols.clone();
        if symbols.is_empty() {
            ui.label(RichText::new("No symbols in this file.").color(pal.text_dim));
            return;
        }
        let current = symbol_path(&symbols, line).last().map(|s| (s.line, s.depth));
        let mut jump = None;
        let font = egui::TextStyle::Body.resolve(ui.style());
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for s in &symbols {
                let height = (font.size * 1.6).round();
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
                if current == Some((s.line, s.depth)) {
                    ui.painter().rect_filled(rect, 0.0, pal.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, pal.hover_bg);
                }
                let x = rect.left() + 6.0 + s.depth as f32 * 12.0;
                let (badge, color) = kind_badge(s.kind);
                let center = rect.left_center();
                ui.painter().text(egui::pos2(x, center.y), egui::Align2::LEFT_CENTER, badge, font.clone(), color);
                let name_pos = egui::pos2(x + font.size, center.y);
                ui.painter().text(name_pos, egui::Align2::LEFT_CENTER, &s.name, font.clone(), pal.text);
                if resp.on_hover_text(format!("{} {} (line {})", s.kind.word(), s.name, s.line + 1)).clicked() {
                    jump = Some(s.line);
                }
            }
        });
        if let Some(line) = jump {
            self.go_to_line_start(line);
        }
    }

    fn go_to_line_start(&mut self, line: usize) {
        if let Some(doc) = self.docs.get_mut(self.active) {
            let line = line.min(doc.buffer.len_lines() - 1);
            let text = doc.buffer.line_text(line);
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let pos = doc.buffer.line_start(line) + indent;
            doc.select(pos, pos);
        }
    }

    /// Symbol path at the cursor for the breadcrumbs, as (label, line).
    pub(super) fn breadcrumb_symbols(&mut self) -> Vec<(String, usize)> {
        let Some(doc) = self.docs.get_mut(self.active) else { return Vec::new() };
        let line = doc.buffer.char_to_line(doc.buffer.primary().head);
        let symbols = doc.structure().symbols.clone();
        symbol_path(&symbols, line).into_iter().map(|s| (format!("{} {}", s.kind.word(), s.name), s.line)).collect()
    }

    pub(super) fn jump_to_symbol_line(&mut self, line: usize) {
        self.go_to_line_start(line);
    }

    /// Ctrl+Shift+O: pick a symbol of the active file.
    pub(super) fn go_to_symbol(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let symbols = doc.structure().symbols.clone();
        if symbols.is_empty() {
            self.status = Some("No symbols in this file".to_owned());
            return;
        }
        self.symbol_pick = symbols
            .iter()
            .map(|s| (format!("{}{} {}  ·  {}", "  ".repeat(s.depth), s.kind.word(), s.name, s.line + 1), s.line))
            .collect();
        let labels = self.symbol_pick.iter().map(|(l, _)| l.clone()).collect();
        self.palette.show_picker(PickerKind::Symbol, labels, "", "Go to symbol in editor");
    }

    pub(super) fn picked_symbol(&mut self, label: &str) {
        if let Some(&(_, line)) = self.symbol_pick.iter().find(|(l, _)| l == label) {
            self.go_to_line_start(line);
        }
    }

    // ----- smart selection and text objects --------------------------------------------

    /// Shift+Alt+Right: grow each selection to the enclosing syntax node.
    pub(super) fn expand_selection(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let Some(tree) = doc.fresh_tree() else {
            self.status = Some("Expand Selection needs a syntax tree (Rust, JSON, TOML, YAML, Markdown)".to_owned());
            return;
        };
        let rope = doc.buffer.rope();
        let old = doc.buffer.selections().to_vec();
        let new: Vec<Selection> = old
            .iter()
            .map(|s| match structure::expand_selection(&tree, rope, s.start(), s.end()) {
                Some((a, b)) => Selection::new(a, b),
                None => *s,
            })
            .collect();
        if new == old {
            return;
        }
        let (uid, primary) = (doc.uid, old.iter().position(|s| *s == doc.buffer.primary()).unwrap_or(0));
        doc.buffer.set_selections(new, primary);
        let version = doc.buffer.version();
        match &mut self.selection_stack {
            Some((u, v, stack)) if *u == uid && *v == version => stack.push(old),
            _ => self.selection_stack = Some((uid, version, vec![old])),
        }
    }

    /// Shift+Alt+Left: undo the last Expand Selection.
    pub(super) fn shrink_selection(&mut self) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let Some((uid, version, stack)) = &mut self.selection_stack else { return };
        if *uid != doc.uid || *version != doc.buffer.version() {
            self.selection_stack = None;
            return;
        }
        if let Some(previous) = stack.pop() {
            doc.buffer.set_selections(previous, 0);
        }
    }

    /// Selects a text object around the cursor (Rust).
    pub(super) fn select_text_object(&mut self, object: TextObject) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let found = doc
            .fresh_tree()
            .filter(|_| doc.extension() == "rs")
            .and_then(|tree| structure::text_object(&tree, doc.buffer.rope(), doc.buffer.primary().head, object));
        match found {
            Some((a, b)) => doc.select(a, b),
            None => self.status = Some("Nothing to select here".to_owned()),
        }
    }

    pub(super) fn adjacent_function(&mut self, forward: bool) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let found = doc.fresh_tree().filter(|_| doc.extension() == "rs").and_then(|tree| {
            structure::adjacent_function(&tree, doc.buffer.rope(), doc.buffer.primary().head, forward)
        });
        match found {
            Some(pos) => doc.select(pos, pos),
            None => self.status = Some(format!("No {} function", if forward { "next" } else { "previous" })),
        }
    }
}

fn doc_line_end(doc: &crate::document::Document, line: usize) -> usize {
    doc.buffer.line_end(line.min(doc.buffer.len_lines() - 1))
}
