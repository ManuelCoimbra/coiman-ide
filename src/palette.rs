//! Quick Open (Ctrl+P) and Command Palette (Ctrl+Shift+P) overlay.
//! As in VS Code, a query starting with '>' lists commands; otherwise files.

use std::path::{Path, PathBuf};

use egui::{Key, RichText};

use crate::commands::{COMMANDS, Command};
use crate::finder::FileFinder;
use crate::keymap::Keymap;
use crate::theme;

pub enum PaletteResult {
    OpenFile(PathBuf),
    Run(Command),
    /// A plugin command (its palette key).
    RunPlugin(String),
    GoToLine(usize),
    /// A symbol from `#`, by its place in `symbols`.
    WorkspaceSymbol(usize),
    /// An item chosen in a picker.
    Picked(PickerKind, String),
    /// Text entered in an input box.
    Input(InputKind, String),
}

/// What an input box asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputKind {
    Rename,
    NewFile,
    NewFolder,
    RenamePath,
    NewBranch,
    BreakpointCondition,
    Logpoint,
}

/// What a picker list is choosing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerKind {
    Theme,
    LineEnding,
    Bookmark,
    Snippet,
    Indentation,
    RecentFolder,
    /// "Reopen with Encoding" or "Save with Encoding".
    EncodingAction,
    ReopenWithEncoding,
    SaveWithEncoding,
    CodeAction,
    CodeLens,
    RunMode,
    Python,
    Branch,
    RunConfig,
    Symbol,
    History,
    DebugTarget,
    RunTarget,
}

struct Picker {
    kind: PickerKind,
    items: Vec<String>,
    placeholder: &'static str,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    just_opened: bool,
    /// When set, the palette is a picker over these items instead of files/commands.
    picker: Option<Picker>,
    /// When set, the palette is a plain input box.
    input: Option<(InputKind, &'static str)>,
    /// Commands of running plugins: (title, key).
    pub plugin_commands: Vec<(String, String)>,
    /// Results for a `#` query: (name, container).
    pub symbols: Vec<(String, String)>,
    /// Commands that don't apply here.
    pub hidden_commands: &'static [Command],
}

impl Palette {
    pub fn show_with(&mut self, prefix: &str) {
        self.open = true;
        self.query = prefix.to_owned();
        self.selected = 0;
        self.just_opened = true;
        self.picker = None;
        self.input = None;
    }

    /// Opens an input box prefilled with `initial` (all of it selected).
    pub fn show_input(&mut self, kind: InputKind, initial: &str, placeholder: &'static str) {
        self.show_with(initial);
        self.input = Some((kind, placeholder));
    }

    /// Opens the Color Theme picker with `current` preselected.
    /// Opens a picker over `items` with `current` preselected.
    pub fn show_picker(&mut self, kind: PickerKind, items: Vec<String>, current: &str, placeholder: &'static str) {
        self.show_with("");
        self.selected = items.iter().position(|l| l.eq_ignore_ascii_case(current)).unwrap_or(0);
        self.picker = Some(Picker { kind, items, placeholder });
    }

    /// What's being searched after `#`, while the palette shows symbols.
    pub fn symbol_query(&self) -> Option<&str> {
        if !self.open || self.picker.is_some() || self.input.is_some() {
            return None;
        }
        self.query.strip_prefix('#').map(str::trim)
    }

    pub fn ui(&mut self, ctx: &egui::Context, finder: &mut FileFinder, keymap: &Keymap) -> Option<PaletteResult> {
        if !self.open {
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.open = false;
            return None;
        }

        if let Some((kind, placeholder)) = self.input {
            return self.input_ui(ctx, kind, placeholder);
        }

        // Build the candidate list: (label, detail, result, highlighted label chars).
        let mut items: Vec<(String, String, PaletteResult, Vec<usize>)> = Vec::new();
        let mut empty_text = "No matching results";
        if let Some(picker) = &self.picker {
            let q = self.query.trim();
            let mut matching: Vec<&String> = picker.items.iter().filter(|t| fuzzy_match(t, q)).collect();
            // Long labels match nearly anything fuzzily: the text itself comes first.
            let lower = q.to_lowercase();
            matching.sort_by_key(|t| t.to_lowercase().find(&lower).unwrap_or(usize::MAX));
            for t in matching {
                items.push((t.clone(), String::new(), PaletteResult::Picked(picker.kind, t.clone()), Vec::new()));
            }
        } else if let Some(q) = self.query.strip_prefix('>') {
            let q = q.trim();
            for (cmd, name) in COMMANDS {
                if fuzzy_match(name, q) && !self.hidden_commands.contains(cmd) {
                    items.push((name.to_string(), keymap.shortcut_text(*cmd), PaletteResult::Run(*cmd), Vec::new()));
                }
            }
            for (title, key) in &self.plugin_commands {
                if fuzzy_match(title, q) {
                    items.push((title.clone(), "plugin".to_owned(), PaletteResult::RunPlugin(key.clone()), Vec::new()));
                }
            }
        } else if self.query.starts_with('#') {
            empty_text = "Type to search symbols in the project (needs a language server)";
            for (i, (name, detail)) in self.symbols.iter().enumerate() {
                items.push((name.clone(), detail.clone(), PaletteResult::WorkspaceSymbol(i), Vec::new()));
            }
        } else if let Some(q) = self.query.strip_prefix(':') {
            let line = q.trim().parse::<usize>().ok();
            let label = match line {
                Some(n) => format!("Go to line {n}"),
                None => "Type a line number to go to".to_owned(),
            };
            items.push((label, String::new(), PaletteResult::GoToLine(line.unwrap_or(0)), Vec::new()));
        } else {
            finder.set_query(self.query.trim());
            for m in finder.results(200) {
                let name = m.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dir = Path::new(&m.rel).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                // Matched chars of the relative path that fall inside the file name.
                let name_start = m.rel.chars().count() - name.chars().count();
                let hl = m.indices.iter().filter_map(|&i| (i as usize).checked_sub(name_start)).collect();
                items.push((name, dir, PaletteResult::OpenFile(m.path), hl));
            }
            if !finder.has_root() {
                empty_text = "Open a folder to search files (> for commands, : for line)";
            } else if finder.is_scanning() {
                empty_text = "Indexing files…";
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }

        let (up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.key_pressed(Key::Enter),
            )
        });
        if !items.is_empty() {
            if up {
                self.selected = self.selected.checked_sub(1).unwrap_or(items.len() - 1);
            }
            if down {
                self.selected = (self.selected + 1) % items.len();
            }
            self.selected = self.selected.min(items.len() - 1);
        }

        let mut result = None;
        let width = (ctx.content_rect().width() * 0.5).clamp(400.0, 700.0);
        let area = egui::Area::new(egui::Id::unique("command_palette"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(theme::colors().sidebar_bg).inner_margin(6.0).show(ui, |ui| {
                    ui.set_width(width);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(
                                self.picker
                                    .as_ref()
                                    .map_or("Search files by name (> commands, : line, # symbols)", |p| p.placeholder),
                            )
                            .desired_width(f32::INFINITY),
                    );
                    if self.just_opened {
                        edit.request_focus();
                        // Put the cursor at the end of the prefilled text.
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                            let end = egui::text::CCursor::new(self.query.chars().count());
                            state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                            state.store(ui.ctx(), edit.id);
                        }
                        self.just_opened = false;
                    }
                    if edit.changed() {
                        self.selected = 0;
                    }
                    ui.add_space(4.0);

                    // Room for every item up to the cap (the list otherwise keeps its first, smaller size).
                    let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                    let wanted = (items.len().max(1) as f32 * row_h).min(360.0);
                    egui::ScrollArea::vertical().max_height(360.0).min_scrolled_height(wanted).show(ui, |ui| {
                        for (i, (label, detail, outcome, hl)) in items.iter().enumerate() {
                            let selected = i == self.selected;
                            let resp = ui.horizontal(|ui| {
                                if matches!(outcome, PaletteResult::OpenFile(_)) {
                                    let size = ui.text_style_height(&egui::TextStyle::Body);
                                    crate::icons::show_file(ui, label, size);
                                }
                                let text = highlight_label(ui, label, hl);
                                let r = ui.add(egui::Button::selectable(selected, text).truncate());
                                ui.label(RichText::new(detail).small().color(theme::colors().text_dim));
                                r
                            });
                            if selected && (up || down) {
                                resp.inner.scroll_to_me(None);
                            }
                            if resp.inner.clicked() {
                                self.selected = i;
                                result = Some(i);
                            }
                        }
                        if items.is_empty() {
                            ui.label(RichText::new(empty_text).color(theme::colors().text_dim));
                        }
                    });
                });
            });

        if enter && !items.is_empty() {
            result = Some(self.selected);
        }
        self.close_on_click_outside(ctx, area.response.rect);
        let picked = result.map(|i| items.swap_remove(i).2);
        if picked.is_some() {
            self.open = false;
        }
        picked
    }
}

impl Palette {
    fn close_on_click_outside(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        let outside =
            ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
        if outside {
            self.open = false;
        }
    }

    fn input_ui(&mut self, ctx: &egui::Context, kind: InputKind, placeholder: &str) -> Option<PaletteResult> {
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let width = (ctx.content_rect().width() * 0.4).clamp(300.0, 500.0);
        let area = egui::Area::new(egui::Id::unique("palette_input"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(theme::colors().sidebar_bg).inner_margin(6.0).show(ui, |ui| {
                    ui.set_width(width);
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.query).desired_width(f32::INFINITY));
                    if self.just_opened {
                        edit.request_focus();
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                            let all = egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.query.chars().count()),
                            );
                            state.cursor.set_char_range(Some(all));
                            state.store(ui.ctx(), edit.id);
                        }
                        self.just_opened = false;
                    }
                    ui.label(RichText::new(placeholder).small().color(theme::colors().text_dim));
                });
            });
        self.close_on_click_outside(ctx, area.response.rect);
        if enter && !self.query.trim().is_empty() {
            self.open = false;
            return Some(PaletteResult::Input(kind, self.query.trim().to_owned()));
        }
        None
    }
}

/// The label with fuzzy-matched characters in the accent color.
fn highlight_label(ui: &egui::Ui, label: &str, highlights: &[usize]) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let pal = theme::colors();
    let mut job = egui::text::LayoutJob::default();
    for (i, c) in label.chars().enumerate() {
        let color = if highlights.contains(&i) { pal.accent } else { pal.text };
        job.append(&c.to_string(), 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
    }
    job
}

/// Case-insensitive subsequence match ("mnrs" matches "main.rs").
fn fuzzy_match(text: &str, query: &str) -> bool {
    let mut chars = text.chars().flat_map(char::to_lowercase);
    query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).all(|q| chars.any(|c| c == q))
}
