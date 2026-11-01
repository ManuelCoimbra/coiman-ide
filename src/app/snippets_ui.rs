//! Your own snippets: insert them by prefix and Tab or from a list, and manage
//! them in a window.

use eframe::egui::{self, RichText};
use rust_code_core::snippets::{self, Snippet};

use super::CodeApp;
use crate::buffer::Selection;
use crate::palette::PickerKind;
use crate::theme;

#[derive(Default)]
pub(super) struct SnippetsUi {
    pub(super) open: bool,
    list: Vec<Snippet>,
    selected: usize,
    /// There are changes to save.
    dirty: bool,
    /// The file couldn't be read: saving would lose what's in it.
    broken: Option<String>,
}

impl SnippetsUi {
    pub(super) fn show(&mut self) {
        self.open = true;
        self.dirty = false;
        self.selected = 0;
        match snippets::load() {
            Ok(list) => {
                self.list = list;
                self.broken = None;
            }
            Err(e) => {
                self.list.clear();
                self.broken = Some(e);
            }
        }
    }
}

enum Action {
    Save,
    OpenFile,
}

/// Why these snippets can't be saved yet, if so.
fn problem(list: &[Snippet]) -> Option<String> {
    list.iter().find_map(|s| {
        let name = if s.name.is_empty() { "A snippet" } else { s.name.as_str() };
        if s.name.trim().is_empty() {
            Some("A snippet needs a name".to_owned())
        } else if s.body.trim().is_empty() {
            Some(format!("{name} needs a body"))
        } else if s.prefix.contains(char::is_whitespace) {
            Some(format!("{name}: the prefix can't contain spaces"))
        } else {
            None
        }
    })
}

/// The places of a snippet just inserted, to go through with Tab.
pub(super) struct SnippetStops {
    uid: u64,
    /// Char ranges, each as it stands now or will once what's typed is counted.
    list: Vec<(usize, usize)>,
    at: usize,
    /// The document's length when these were last brought up to date.
    len: usize,
}

impl CodeApp {
    /// Inserts a body at the selection of the active document, replacing `start..` it.
    pub(super) fn insert_snippet(&mut self, body: &str, start: Option<usize>) {
        let Some(doc) = self.docs.get_mut(self.active) else { return };
        let primary = doc.buffer.primary();
        let start = start.unwrap_or(primary.start());
        let end = primary.end();
        let line = doc.buffer.char_to_line(start);
        let text = doc.buffer.line_text(line);
        let indent: String = text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        let (text, stops) = snippets::prepare_stops(body, &indent, doc.buffer.line_ending);
        let (a, b) = stops.first().copied().unwrap_or_else(|| {
            let end = text.chars().count();
            (end, end)
        });
        doc.buffer.apply_edits(vec![(start, end, text)]);
        doc.buffer.set_single(Selection::new(start + a, start + b));
        doc.reveal = true;
        let list: Vec<(usize, usize)> = stops.into_iter().map(|(a, b)| (start + a, start + b)).collect();
        self.snippet_stops =
            (list.len() > 1).then(|| SnippetStops { uid: doc.uid, list, at: 0, len: doc.buffer.len_chars() });
    }

    /// Tab or Shift+Tab inside a snippet: on to its next place, or back. Says whether
    /// it moved. Moving the cursor out of the place, or leaving the last, ends this.
    pub(super) fn snippet_step(&mut self, back: bool) -> bool {
        let Some(mut stops) = self.snippet_stops.take() else { return false };
        let Some(doc) = self.docs.get_mut(self.active).filter(|d| d.uid == stops.uid) else { return false };
        let head = doc.buffer.primary().head;
        let len = doc.buffer.len_chars();
        // What was typed went into the place the cursor is in.
        let grown = len as isize - stops.len as isize;
        let (a, b) = stops.list[stops.at];
        let end = b.saturating_add_signed(grown);
        if head < a || head > end || end < a {
            return false;
        }
        stops.list[stops.at].1 = end;
        for later in &mut stops.list[stops.at + 1..] {
            later.0 = later.0.saturating_add_signed(grown);
            later.1 = later.1.saturating_add_signed(grown);
        }
        stops.len = len;
        let next = if back { stops.at.checked_sub(1) } else { Some(stops.at + 1) };
        let Some(next) = next.filter(|n| *n < stops.list.len()) else { return false };
        let (a, b) = stops.list[next];
        doc.buffer.set_single(Selection::new(a, b));
        doc.reveal = true;
        stops.at = next;
        // At the last, one more Tab is an ordinary Tab.
        if next + 1 < stops.list.len() || back {
            self.snippet_stops = Some(stops);
        }
        true
    }

    /// Tab: if the word before the cursor is a snippet's prefix, inserts it.
    pub(super) fn expand_snippet_prefix(&mut self) -> bool {
        let Some(doc) = self.docs.get(self.active) else { return false };
        let primary = doc.buffer.primary();
        if !primary.is_empty() || doc.buffer.selections().len() != 1 {
            return false;
        }
        let Ok(list) = snippets::load() else { return false };
        if list.is_empty() {
            return false;
        }
        let line_start = doc.buffer.line_start(doc.buffer.char_to_line(primary.head));
        let before: String = doc.buffer.rope().slice(line_start..primary.head).chars().collect();
        let Some(snippet) = snippets::matching(&list, doc.language_name(), &before) else { return false };
        let start = primary.head - snippet.prefix.chars().count();
        let body = snippet.body.clone();
        self.insert_snippet(&body, Some(start));
        true
    }

    /// A picker over the snippets for this language.
    pub(super) fn pick_snippet(&mut self) {
        let Some(doc) = self.docs.get(self.active) else {
            self.status = Some("Open a file to insert a snippet".to_owned());
            return;
        };
        let list = match snippets::load() {
            Ok(list) => list,
            Err(e) => {
                self.status = Some(e);
                return;
            }
        };
        let language = doc.language_name();
        let items: Vec<String> = list.iter().filter(|s| s.applies_to(language)).map(|s| s.name.clone()).collect();
        if items.is_empty() {
            self.status = Some("No snippets for this language. Add some with Snippets: Manage.".to_owned());
            return;
        }
        self.palette.show_picker(PickerKind::Snippet, items, "", "Insert a snippet");
    }

    pub(super) fn picked_snippet(&mut self, name: &str) {
        let body = snippets::load().ok().and_then(|l| l.into_iter().find(|s| s.name == name)).map(|s| s.body);
        if let Some(body) = body {
            self.insert_snippet(&body, None);
        }
    }

    pub(super) fn snippets_window(&mut self, ctx: &egui::Context) {
        if !self.snippets_ui.open {
            return;
        }
        let mut open = true;
        let mut action = None;
        let state = &mut self.snippets_ui;
        egui::Window::new("Snippets")
            .open(&mut open)
            .collapsible(false)
            .default_size([720.0, 480.0])
            .min_height(360.0)
            .default_pos(ctx.content_rect().center() - egui::vec2(360.0, 240.0))
            .show(ctx, |ui| {
                if let Some(e) = &state.broken {
                    ui.label(RichText::new(e).color(theme::colors().modified));
                    ui.label("Fix the file before changing snippets here.");
                    if ui.button("Edit as File").clicked() {
                        action = Some(Action::OpenFile);
                    }
                    return;
                }
                let error = problem(&state.list);
                ui.horizontal(|ui| {
                    if ui.button("New Snippet").clicked() {
                        state.list.push(Snippet {
                            name: "New snippet".to_owned(),
                            body: "$0".to_owned(),
                            ..Default::default()
                        });
                        state.selected = state.list.len() - 1;
                        state.dirty = true;
                    }
                    if ui.add_enabled(state.dirty && error.is_none(), egui::Button::new("Save")).clicked() {
                        action = Some(Action::Save);
                    }
                    if ui.button("Edit as File").clicked() {
                        action = Some(Action::OpenFile);
                    }
                    if let Some(e) = &error {
                        ui.label(RichText::new(e).color(theme::colors().modified));
                    }
                });
                ui.separator();
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical().id_salt("snippet_list").show(&mut columns[0], |ui| {
                        if state.list.is_empty() {
                            ui.label(RichText::new("No snippets yet.").color(theme::colors().text_dim));
                        }
                        let mut delete = None;
                        for (i, s) in state.list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let text = if s.prefix.is_empty() {
                                    s.name.clone()
                                } else {
                                    format!("{}  ({})", s.name, s.prefix)
                                };
                                if ui.selectable_label(i == state.selected, text).clicked() {
                                    state.selected = i;
                                }
                                if ui.small_button("Delete").clicked() {
                                    delete = Some(i);
                                }
                            });
                        }
                        if let Some(i) = delete {
                            state.list.remove(i);
                            state.selected = state.selected.min(state.list.len().saturating_sub(1));
                            state.dirty = true;
                        }
                    });
                    let ui = &mut columns[1];
                    let Some(s) = state.list.get_mut(state.selected) else { return };
                    let mut changed = false;
                    ui.label("Name");
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(f32::INFINITY)).changed();
                    ui.label("Prefix (type it and press Tab)");
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.prefix).desired_width(f32::INFINITY)).changed();
                    ui.label("Languages (Rust, Python; empty for all)");
                    changed |=
                        ui.add(egui::TextEdit::singleline(&mut s.language).desired_width(f32::INFINITY)).changed();
                    ui.label("Body ($1 and ${1:text} are selected first, $0 is where the cursor ends)");
                    let body = egui::TextEdit::multiline(&mut s.body)
                        .code_editor()
                        .desired_rows(10)
                        .desired_width(f32::INFINITY);
                    changed |= ui.add(body).changed();
                    state.dirty |= changed;
                });
            });
        self.snippets_ui.open = open;
        match action {
            Some(Action::Save) => match snippets::save(&self.snippets_ui.list) {
                Ok(()) => {
                    self.snippets_ui.dirty = false;
                    self.status = Some("Saved snippets".to_owned());
                }
                Err(e) => self.status = Some(e),
            },
            Some(Action::OpenFile) => {
                self.snippets_ui.open = false;
                let empty = "# Your snippets. Bodies use $1, ${1:text} and $0.\n".to_owned();
                self.open_config_file(snippets::file_path(), empty);
            }
            None => {}
        }
    }
}
