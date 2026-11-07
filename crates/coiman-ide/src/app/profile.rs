//! What a language's profile asks of the editor: rulers and dim columns, typing
//! symbols, a bar of glyphs, and following includes along search paths.

use coiman_core::language::lsp::registry::{self, Language};
use coiman_core::{appearance::fonts, editor::symbols};
use eframe::egui::{self, RichText};

use super::CodeApp;
use crate::buffer::Selection;
use crate::editor_view::Decorations;
use crate::palette::PickerKind;
use crate::theme;

impl CodeApp {
    fn profile_of(&self, i: usize) -> Option<&'static Language> {
        registry::by_extension(&self.docs.get(i)?.extension(), false)
    }

    pub(super) fn profile_decorate(&self, i: usize, deco: &mut Decorations) {
        let profile = self.profile_of(i);
        deco.rulers = profile
            .filter(|p| !p.rulers.is_empty())
            .map_or_else(|| self.settings.editor.rulers.clone(), |p| p.rulers.clone());
        deco.dim_columns = profile.map(|p| p.dim_columns.clone()).unwrap_or_default();
    }

    /// Symbols, box drawing, Chinese, Japanese and Korean need system fonts: the built-in
    /// ones have none of them for UI text. Read once, off to the side, at start.
    pub(super) fn pump_symbol_font(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.fallback_started, true) {
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = super::repaint(ctx);
            std::thread::spawn(move || {
                let _ = tx.send(fonts::load_fallback_fonts());
                wake();
            });
            self.fallback_load = Some(rx);
        }
        let Some(found) = self.fallback_load.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.fallback_load = None;
        if !found.iter().any(|(name, ..)| *name == "symbols") {
            self.status = Some("No font with symbols found: some may show as boxes".to_owned());
        }
        let names: Vec<&str> = found.iter().map(|(name, ..)| *name).collect();
        coiman_core::applog::info(&format!("fallback fonts: {}", names.join(", ")));
        self.fallback_fonts = found
            .into_iter()
            .map(|(name, bytes, index)| {
                let mut font = egui::FontData::from_owned(bytes);
                font.index = index;
                (format!("{name} fallback"), std::sync::Arc::new(font))
            })
            .collect();
        self.install_fonts(ctx);
    }

    fn replace_before_cursor(&mut self, back: usize, length: usize, symbol: &str) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let head = doc.buffer.primary().head;
        let start = head - back;
        let rest = back - length;
        doc.buffer.apply_edits(vec![(start, start + length, symbol.to_owned())]);
        let at = start + symbol.chars().count() + rest;
        doc.buffer.set_single(Selection::new(at, at));
    }

    fn line_before_cursor(&self) -> Option<String> {
        let doc = self.docs.get(self.active)?;
        let primary = doc.buffer.primary();
        if !primary.is_empty() || doc.buffer.selections().len() != 1 || doc.buffer.read_only {
            return None;
        }
        let line_start = doc.buffer.line_start(doc.buffer.char_to_line(primary.head));
        Some(doc.buffer.rope().slice(line_start..primary.head).chars().collect())
    }

    /// `\to` plus a character that can't continue it becomes `→` plus that character.
    pub(super) fn symbols_after_typing(&mut self, typed: &str) {
        let count = typed.chars().count();
        if count == 0 || !self.profile_of(self.active).is_some_and(|p| p.abbreviations) {
            return;
        }
        // A paste, or several keys in one frame: all of it at once, from where a
        // name that it finishes could have started.
        if count > 1 {
            let Some(doc) = self.docs.get(self.active) else { return };
            let primary = doc.buffer.primary();
            if !primary.is_empty() || doc.buffer.selections().len() != 1 || doc.buffer.read_only {
                return;
            }
            let start = primary.head.saturating_sub(count + 17);
            let text = doc.buffer.slice_string(start, primary.head);
            let converted = symbols::convert_all(&text, &self.settings.symbols);
            if converted != text {
                let length = primary.head - start;
                self.replace_before_cursor(length, length, &converted);
            }
            return;
        }
        let Some(before) = self.line_before_cursor() else { return };
        if let Some(found) = symbols::after_typing(&before, &self.settings.symbols) {
            self.replace_before_cursor(found.back, found.length, &found.symbol);
        }
    }

    /// Tab right after a name.
    pub(super) fn symbol_on_tab(&mut self) -> bool {
        if !self.profile_of(self.active).is_some_and(|p| p.abbreviations) {
            return false;
        }
        let Some(before) = self.line_before_cursor() else { return false };
        let Some(found) = symbols::at_cursor(&before, &self.settings.symbols) else { return false };
        self.replace_before_cursor(found.back, found.length, &found.symbol);
        true
    }

    /// `Insert Symbol`: every symbol, searchable by name.
    pub(super) fn pick_glyph(&mut self) {
        let items = symbols::all(&self.settings.symbols)
            .into_iter()
            .map(|(symbol, names)| format!("{symbol}  \\{}", names.join("  \\")))
            .collect();
        self.palette.show_picker(PickerKind::Glyph, items, "", "Find a symbol by name");
    }

    pub(super) fn picked_glyph(&mut self, label: &str) {
        let symbol = label.split("  ").next().unwrap_or_default().to_owned();
        self.insert_glyph(&symbol);
    }

    fn insert_glyph(&mut self, symbol: &str) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        if doc.buffer.read_only {
            return;
        }
        let primary = doc.buffer.primary();
        let at = primary.start() + symbol.chars().count();
        doc.buffer.apply_edits(vec![(primary.start(), primary.end(), symbol.to_owned())]);
        doc.buffer.set_single(Selection::new(at, at));
        doc.request_focus = true;
    }

    /// A row of a language's glyphs above the text, for those without a keyboard for them.
    pub(super) fn glyph_bar(&mut self, ui: &mut egui::Ui) {
        if !self.settings.editor.glyph_bar {
            return;
        }
        let Some(glyphs) = self.profile_of(self.active).and_then(|p| p.glyphs) else { return };
        let mut picked = None;
        let dim = theme::colors().hover_bg;
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for glyph in glyphs.chars().filter(|c| !c.is_whitespace()) {
                    let text = RichText::new(glyph.to_string()).monospace().size(self.font_size);
                    if ui.add(egui::Button::new(text).fill(dim)).clicked() {
                        picked = Some(glyph);
                    }
                }
            });
        });
        if let Some(glyph) = picked {
            self.insert_glyph(&glyph.to_string());
        }
    }

    /// `Go to Include at Cursor`: the file a line's include statement names.
    pub(super) fn open_include(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let (Some(path), Some(profile)) = (doc.path.clone(), self.profile_of(self.active)) else {
            self.status = Some("This language's profile has no include statement".to_owned());
            return;
        };
        let Some(pattern) = profile.include_pattern else {
            self.status = Some("This language's profile has no include statement".to_owned());
            return;
        };
        let line = doc.buffer.line_text(doc.buffer.char_to_line(doc.buffer.primary().head));
        let Some(name) = symbols::included_name(pattern, &line) else {
            self.status = Some("No include statement on this line".to_owned());
            return;
        };
        let root = self.explorer.root.clone().or_else(|| path.parent().map(std::path::Path::to_path_buf));
        let root = root.unwrap_or_default();
        let mut paths: Vec<&str> = self.settings.editor.include_paths.iter().map(String::as_str).collect();
        paths.extend(profile.include_paths.iter().copied());
        match symbols::find_include(&name, &path, &root, &paths, profile.include_extensions) {
            Ok(found) => self.open_file(&found),
            Err(looked) => {
                let folders: Vec<String> = looked
                    .iter()
                    .map(|d| d.strip_prefix(&root).unwrap_or(d).to_string_lossy().replace('\\', "/"))
                    .map(|d| if d.is_empty() { ".".to_owned() } else { d })
                    .collect();
                self.status = Some(format!("{name} not found in {}", folders.join(", ")));
            }
        }
    }
}
