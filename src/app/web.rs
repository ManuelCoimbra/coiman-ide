//! The web authoring kit: Emmet on Tab, the color picker, Baseline and specificity
//! on hover, Baseline problems against a project's target, a guard against
//! formatters that fight.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use rust_code_core::baseline::{self, Target};
use rust_code_core::css::{self, Color, Format, Gamut};
use rust_code_core::emmet::{self, Syntax};
use rust_code_core::formatters::{self, WebTools};
use rust_code_core::lsp::{convert, types as lt};

use super::CodeApp;
use super::lsp::path_key;
use crate::buffer::Selection;

/// The picker's window.
struct Picker {
    uid: u64,
    version: u64,
    start: usize,
    end: usize,
    /// There was no color here: it will be inserted.
    inserting: bool,
    place: egui::Pos2,
    color: Color,
    against: Color,
}

/// Baseline problems of a file, for the version of it they were found in.
struct Checked {
    uid: u64,
    version: u64,
    target: Target,
    path: PathBuf,
    problems: Vec<lt::Diagnostic>,
}

#[derive(Default)]
pub(super) struct Web {
    picker: Option<Picker>,
    checked: HashMap<PathBuf, Checked>,
}

fn to_color32(c: &Color) -> egui::Color32 {
    let [r, g, b, a] = c.srgb8();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn from_color32(c: egui::Color32, format: Format) -> Color {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color::from_srgb8(r, g, b, a, format)
}

fn gamut_note(gamut: Gamut) -> Option<&'static str> {
    match gamut {
        Gamut::Srgb => None,
        Gamut::DisplayP3 => Some("Outside sRGB, inside Display P3: most other screens show a nearby color."),
        Gamut::Wider => Some("Outside Display P3: nearly every screen shows a nearby color instead."),
    }
}

impl CodeApp {
    // ----- Emmet -----------------------------------------------------------------------

    /// Replaces the abbreviation before the cursor with what it stands for.
    pub(super) fn expand_emmet(&mut self, on_tab: bool) -> bool {
        let Some(doc) = self.docs.get(self.active) else { return false };
        let ext = doc.extension();
        let primary = doc.buffer.primary();
        let usable = primary.is_empty() && doc.buffer.selections().len() == 1 && !doc.large && !doc.buffer.read_only;
        let syntax = Syntax::for_extension(&ext);
        let syntax = syntax.filter(|_| usable && (!on_tab || Syntax::expands_on_tab(&ext)));
        let Some(syntax) = syntax else {
            if !on_tab {
                self.status = Some("Emmet works in HTML, CSS and JSX files".to_owned());
            }
            return false;
        };
        let before: String = doc.buffer.rope().slice(..primary.head).chars().collect();
        let Some(expansion) = emmet::expand(&before, syntax, on_tab) else {
            if !on_tab {
                self.status = Some("No abbreviation before the cursor".to_owned());
            }
            return false;
        };
        self.insert_snippet(&expansion.body, Some(primary.head - expansion.length));
        true
    }

    // ----- the color picker ------------------------------------------------------------

    /// Opens the picker on the color under the cursor, or to insert one there.
    pub(super) fn pick_color(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.status = Some("Open a file to pick a color".to_owned());
            return;
        };
        if doc.large || doc.buffer.read_only {
            self.status = Some("This file can't be edited here".to_owned());
            return;
        }
        let head = doc.buffer.primary().head;
        let text = doc.text();
        let place = self.cursor_on_screen().map_or(egui::pos2(200.0, 100.0), |r| r.left_bottom());
        let found = css::color_at(&text, head);
        let white = Color::from_srgb8(255, 255, 255, 255, Format::Hex);
        let (start, end, color) = found.map_or_else(
            || (head, head, Color::from_srgb8(0x33, 0x66, 0x99, 255, Format::Hex)),
            |f| (f.start, f.end, f.color),
        );
        self.web.picker = Some(Picker {
            uid: doc.uid,
            version: doc.buffer.version(),
            start,
            end,
            inserting: found.is_none(),
            place,
            color,
            against: css::counterpart(&text, head).unwrap_or(white),
        });
    }

    fn apply_picked(&mut self) {
        let Some(picker) = &mut self.web.picker else { return };
        let Some(doc) = self.docs.iter_mut().find(|d| d.uid == picker.uid) else {
            self.web.picker = None;
            return;
        };
        if doc.buffer.version() != picker.version {
            self.status = Some("The file changed: pick the color again".to_owned());
            self.web.picker = None;
            return;
        }
        let text = picker.color.to_css();
        let length = text.chars().count();
        doc.buffer.apply_edits(vec![(picker.start, picker.end, text)]);
        picker.end = picker.start + length;
        picker.inserting = false;
        picker.version = doc.buffer.version();
        doc.buffer.set_single(Selection::new(picker.end, picker.end));
        doc.reveal = true;
    }

    fn picker_window(&mut self, ctx: &egui::Context) {
        // It edits one file, so it doesn't follow to another.
        let active = self.docs.get(self.active).map(|d| d.uid);
        if self.web.picker.as_ref().is_some_and(|p| Some(p.uid) != active) {
            self.web.picker = None;
        }
        let Some(picker) = &mut self.web.picker else { return };
        let (mut open, mut apply) = (true, false);
        // Beside the color, not over it.
        let place = picker.place + egui::vec2(0.0, 24.0);
        let window = egui::Window::new("Color").open(&mut open).resizable(false).collapsible(false);
        window.default_pos(place).show(ctx, |ui| {
            let format = picker.color.format;
            let mut shown = to_color32(&picker.color);
            if egui::color_picker::color_picker_color32(ui, &mut shown, egui::color_picker::Alpha::OnlyBlend) {
                picker.color = from_color32(shown, format);
            }
            let [mut l, mut c, mut h] = picker.color.oklch();
            ui.horizontal(|ui| {
                ui.label("OKLCH");
                let mut changed = ui.add(egui::DragValue::new(&mut l).range(0.0..=1.0).speed(0.002)).changed();
                changed |= ui.add(egui::DragValue::new(&mut c).range(0.0..=0.5).speed(0.001)).changed();
                changed |= ui.add(egui::DragValue::new(&mut h).range(0.0..=360.0).speed(0.5)).changed();
                if changed {
                    picker.color = Color::from_oklch([l, c, h], picker.color.alpha, format);
                }
            });
            ui.label(RichText::new(picker.color.to_css()).monospace());
            if let Some(note) = gamut_note(picker.color.gamut()) {
                ui.label(RichText::new(note).color(egui::Color32::from_rgb(0xCC, 0xA7, 0x00)));
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Against");
                let mut against = to_color32(&picker.against);
                if egui::color_picker::color_edit_button_srgba(ui, &mut against, egui::color_picker::Alpha::Opaque)
                    .changed()
                {
                    picker.against = from_color32(against, Format::Hex);
                }
                egui::Frame::new().fill(to_color32(&picker.against)).inner_margin(6.0).show(ui, |ui| {
                    ui.label(RichText::new("Sample text").color(to_color32(&picker.color)));
                });
            });
            let ratio = picker.color.contrast(&picker.against);
            let grade = |needed: f64| if ratio >= needed { "pass" } else { "fail" };
            ui.label(format!("Contrast {ratio:.2}:1. AA {}, AA large {}, AAA {}", grade(4.5), grade(3.0), grade(7.0)));
            ui.add_space(4.0);
            apply = ui.button(if picker.inserting { "Insert" } else { "Apply" }).clicked();
        });
        if apply {
            self.apply_picked();
        }
        if !open {
            self.web.picker = None;
        }
    }

    // ----- Baseline and specificity ----------------------------------------------------

    fn baseline_target(&self) -> Option<Target> {
        Target::parse(&self.settings.web.baseline)
    }

    /// Finds what the active file uses that its project's target doesn't allow.
    pub(super) fn check_baseline(&mut self) {
        let Some(target) = self.baseline_target() else {
            self.web.checked.clear();
            return;
        };
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(path) = doc.path.clone() else { return };
        let key = path_key(&path);
        let (uid, version) = (doc.uid, doc.buffer.version());
        let current =
            self.web.checked.get(&key).is_some_and(|c| c.uid == uid && c.version == version && c.target == target);
        if current || doc.large {
            return;
        }
        let rope = doc.buffer.rope();
        let found = baseline::below_target(&doc.extension(), &doc.text(), target);
        let problems = found
            .into_iter()
            .map(|(used, entry)| lt::Diagnostic {
                range: lt::Range::new(convert::position_of(rope, used.start), convert::position_of(rope, used.end)),
                severity: Some(lt::DiagnosticSeverity::WARNING),
                source: Some("Baseline".to_owned()),
                message: format!("{}. {} is below this project's {}.", entry.describe(), entry.name, target.label()),
                ..lt::Diagnostic::default()
            })
            .collect();
        self.web.checked.insert(key, Checked { uid, version, target, path, problems });
    }

    pub(super) fn baseline_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        self.web.checked.get(key).map(|c| c.problems.clone()).unwrap_or_default()
    }

    pub(super) fn baseline_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        let with_problems = self.web.checked.iter().filter(|(_, c)| !c.problems.is_empty());
        with_problems.map(|(k, c)| (k.clone(), c.path.clone())).collect()
    }

    /// What the editor itself knows about the place under the pointer, as Markdown.
    /// `server` is what the language server said, which may have some of this already.
    pub(super) fn web_hover(&self, uid: u64, pos: usize, server: &str) -> String {
        let Some(doc) = self.docs.iter().find(|d| d.uid == uid) else { return String::new() };
        let ext = doc.extension();
        let style_sheet = matches!(ext.as_str(), "css" | "scss" | "less");
        if doc.large || !(style_sheet || matches!(ext.as_str(), "html" | "htm" | "xhtml")) {
            return String::new();
        }
        let text = doc.text();
        let mut parts = Vec::new();
        if let Some(found) = css::color_at(&text, pos)
            && let Some(note) = gamut_note(found.color.gamut())
        {
            parts.push(note.to_owned());
        }
        // The CSS server says this itself.
        let selector = if style_sheet { css::selector_at(&text, pos) } else { css::selector_in_page(&text, pos) };
        if !server.contains("Specificity")
            && let Some(selector) = selector
        {
            let (a, b, c) = css::specificity(&selector);
            parts.push(format!("Specificity ({a}, {b}, {c}) of `{selector}`"));
        }
        let used = baseline::uses_for(&ext, &text).into_iter().find(|u| u.start <= pos && pos <= u.end);
        if let Some(entry) = used.and_then(|u| baseline::lookup(&u.key)) {
            let target = self.baseline_target().filter(|t| !entry.meets(*t));
            let below = target.map_or_else(String::new, |t| format!(" Below this project's {}.", t.label()));
            if !server.contains("Baseline") || !below.is_empty() {
                parts.push(format!("{}: {}.{below}", entry.name, entry.describe()));
            }
        }
        parts.join("\n\n")
    }

    // ----- formatters ------------------------------------------------------------------

    fn web_project_root(&self, path: &Path) -> Option<PathBuf> {
        let root = self.explorer.root.as_deref().filter(|r| path.starts_with(r));
        root.or_else(|| path.parent()).map(Path::to_path_buf)
    }

    /// The tool that owns formatting for the active file, if not its language server.
    pub(super) fn formatter_owner(&self) -> Option<&'static str> {
        let doc = self.docs.get(self.active)?;
        if !formatters::is_web_file(&doc.extension()) {
            return None;
        }
        WebTools::of(&self.web_project_root(doc.path.as_deref()?)?).owner()
    }

    /// For the doctor: who formats this file, and what will fight over it.
    pub(super) fn formatter_report(&self) -> Option<(String, bool)> {
        let doc = self.docs.get(self.active)?;
        if !formatters::is_web_file(&doc.extension()) {
            return None;
        }
        let tools = WebTools::of(&self.web_project_root(doc.path.as_deref()?)?);
        let conflicts = tools.conflicts();
        if !conflicts.is_empty() {
            return Some((conflicts.join(" "), false));
        }
        Some(match tools.owner() {
            Some(owner) => (format!("{owner}. Format on save leaves these files alone, so run it yourself"), true),
            None => ("The language server, if it can".to_owned(), true),
        })
    }

    pub(super) fn web_windows(&mut self, ctx: &egui::Context) {
        self.picker_window(ctx);
    }
}
