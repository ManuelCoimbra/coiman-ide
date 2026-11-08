//! Quick Open (Ctrl+P) and Command Palette (Ctrl+Shift+P) overlay.
//! As in VS Code, a query starting with '>' lists commands; otherwise files.

use std::path::{Path, PathBuf};

use coiman_core::project::search::{Msg, Query, SearchHandle};
use egui::{Key, RichText};

use crate::commands::{COMMANDS, Command};
use crate::finder::{FileFinder, rank};
use crate::ui::keymap::Keymap;
use crate::ui::theme;

pub enum PaletteResult {
    OpenFile(PathBuf),
    /// A file at a 1-based line, selecting these columns.
    OpenAt(PathBuf, usize, (usize, usize)),
    /// Settings, on one setting.
    OpenSetting(String),
    Run(Command),
    /// A plugin command (its palette key).
    RunPlugin(String),
    GoToLine(usize),
    /// A symbol from `#`, by its place in `symbols`.
    WorkspaceSymbol(usize),
    /// An item chosen in a picker.
    Picked(PickerKind, String),
    /// An item to take out of a picker's list (its ×, or Shift+Delete).
    Remove(PickerKind, String),
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
    /// A repository's URL, to clone.
    CloneUrl,
    /// The name of a repository to create on GitHub.
    GitHubName,
    /// Goals and options to run with mvn.
    MavenGoal,
    /// What to look for on Maven Central.
    MavenSearch,
    MavenGroupId,
    MavenArtifactId,
}

/// What a picker list is choosing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickerKind {
    Glyph,
    Theme,
    LineEnding,
    Bookmark,
    Snippet,
    Indentation,
    /// Recent projects and files, and Clear.
    Recent,
    /// Yes or no to clearing the recent list.
    ClearRecent,
    /// Private or public, for a new GitHub repository.
    GitHubVisibility,
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
    /// A running process, to attach the debugger to.
    AttachProcess,
    /// Which debugger attaches.
    AttachDebugger,
    RunTarget,
    VsCodeImport,
    MavenGoal,
    MavenFound,
    MavenScope,
    MavenArchetype,
    SampleProject,
}

struct Picker {
    kind: PickerKind,
    items: Vec<String>,
    placeholder: &'static str,
    /// A dim note after each item, when there are any.
    details: Vec<String>,
    /// Items can be taken out of the list.
    removable: bool,
}

/// What Search Everywhere is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EverywhereTab {
    All,
    Files,
    Symbols,
    Commands,
    Settings,
    Text,
}

const EVERYWHERE_TABS: [(EverywhereTab, &str); 6] = [
    (EverywhereTab::All, "All"),
    (EverywhereTab::Files, "Files"),
    (EverywhereTab::Symbols, "Symbols"),
    (EverywhereTab::Commands, "Commands"),
    (EverywhereTab::Settings, "Settings"),
    (EverywhereTab::Text, "Text"),
];

/// Seconds of no typing before searching the text of a project.
const TEXT_SEARCH_DELAY: f64 = 0.2;
/// Lines of text kept from a search.
const TEXT_RESULTS: usize = 200;

/// Search Everywhere's search of the project's text, running behind it.
#[derive(Default)]
struct TextSearch {
    handle: Option<SearchHandle>,
    /// What was last searched for.
    searched: String,
    /// What's being typed, and when it last changed.
    typed: String,
    typed_at: f64,
    /// (file, 1-based line, its text, columns).
    results: Vec<(PathBuf, usize, String, (usize, usize))>,
}

impl TextSearch {
    /// Follows what's typed, once typing pauses, and collects what's found so far.
    fn pump(&mut self, ctx: &egui::Context, query: &str, root: Option<&Path>) {
        let now = ctx.input(|i| i.time);
        if query != self.typed {
            self.typed = query.to_owned();
            self.typed_at = now;
        }
        if self.searched != self.typed {
            if now - self.typed_at < TEXT_SEARCH_DELAY {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            } else {
                self.searched = self.typed.clone();
                self.results.clear();
                self.handle = match root.filter(|_| self.searched.chars().count() >= 2) {
                    Some(root) => {
                        let wake = ctx.clone();
                        let query = Query { text: self.searched.clone(), ..Query::default() };
                        Some(SearchHandle::spawn(query, root.to_path_buf(), move || wake.request_repaint()))
                    }
                    None => None,
                };
            }
        }
        let mut finished = false;
        if let Some(handle) = &self.handle {
            while let Some(message) = handle.try_recv() {
                match message {
                    Msg::File(file) => {
                        for found in file.matches {
                            if self.results.len() < TEXT_RESULTS {
                                self.results.push((file.path.clone(), found.line, found.text, found.cols));
                            }
                        }
                    }
                    Msg::Done { .. } | Msg::Error(_) => finished = true,
                }
            }
        }
        if finished || self.results.len() >= TEXT_RESULTS {
            self.handle = None;
        }
    }
}

/// Tells Shift pressed twice alone from Shift used for anything else.
#[derive(Default)]
pub struct DoubleShift {
    down: bool,
    clean: bool,
    pressed_at: f64,
    last_tap: Option<f64>,
}

impl DoubleShift {
    /// Longest press that counts as a tap, and longest wait between the two.
    const TAP: f64 = 0.4;
    const BETWEEN: f64 = 0.45;

    /// Each frame: whether Shift is down, and whether anything else was pressed or held.
    /// Returns true on the release that completes a double press.
    pub fn update(&mut self, shift: bool, other: bool, now: f64) -> bool {
        if other {
            self.clean = false;
            self.last_tap = None;
        }
        if shift && !self.down {
            self.down = true;
            self.clean = !other;
            self.pressed_at = now;
        } else if !shift && self.down {
            self.down = false;
            if self.clean && now - self.pressed_at <= Self::TAP {
                if self.last_tap.is_some_and(|last| now - last <= Self::BETWEEN) {
                    self.last_tap = None;
                    return true;
                }
                self.last_tap = Some(now);
            }
        }
        false
    }
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
    /// When set, the palette is Search Everywhere, on this tab.
    everywhere: Option<EverywhereTab>,
    /// The folder Search Everywhere searches.
    everywhere_root: Option<PathBuf>,
    /// Every setting: (label, section).
    setting_rows: Vec<(String, String)>,
    text_search: TextSearch,
}

impl Palette {
    pub fn show_with(&mut self, prefix: &str) {
        self.open = true;
        self.query = prefix.to_owned();
        self.selected = 0;
        self.just_opened = true;
        self.picker = None;
        self.input = None;
        self.everywhere = None;
    }

    /// Opens Search Everywhere over `root`, which also knows these settings.
    pub fn show_everywhere(&mut self, root: Option<PathBuf>, settings: Vec<(String, String)>) {
        self.show_with("");
        self.everywhere = Some(EverywhereTab::All);
        self.everywhere_root = root;
        self.setting_rows = settings;
    }

    /// In a word or two, for the log: what's showing. Never what's typed.
    pub fn describe(&self) -> String {
        if !self.open {
            return "closed".to_owned();
        }
        match (&self.picker, self.input, self.everywhere_tab()) {
            (Some(picker), ..) => format!("picker {:?}", picker.kind),
            (_, Some((kind, _)), _) => format!("input {kind:?}"),
            (_, _, Some(tab)) => format!("everywhere {tab:?}"),
            _ if self.query.starts_with('>') => "commands".to_owned(),
            _ => "quick open".to_owned(),
        }
    }

    /// Search Everywhere's tab, unless a prefix has taken the query over.
    fn everywhere_tab(&self) -> Option<EverywhereTab> {
        self.everywhere.filter(|_| !self.query.starts_with(['>', '#', ':']))
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
        self.picker = Some(Picker { kind, items, placeholder, details: Vec::new(), removable: false });
    }

    /// A picker whose items have notes after them and can be removed (a × on each row,
    /// or Shift+Delete), as Open Recent's are.
    pub fn show_removable_picker(
        &mut self,
        kind: PickerKind,
        items: Vec<String>,
        details: Vec<String>,
        placeholder: &'static str,
    ) {
        self.show_with("");
        self.picker = Some(Picker { kind, items, placeholder, details, removable: true });
    }

    /// A picker whose items have notes after them.
    pub fn show_detailed_picker(
        &mut self,
        kind: PickerKind,
        items: Vec<String>,
        details: Vec<String>,
        placeholder: &'static str,
    ) {
        self.show_with("");
        self.picker = Some(Picker { kind, items, placeholder, details, removable: false });
    }

    /// New items for the picker showing, keeping what's typed and the row picked.
    pub fn replace_picker_items(&mut self, items: Vec<String>, details: Vec<String>) {
        if let Some(picker) = &mut self.picker {
            picker.items = items;
            picker.details = details;
            self.open = true;
        }
    }

    /// The picker showing, if any.
    pub fn picker_kind(&self) -> Option<PickerKind> {
        self.picker.as_ref().filter(|_| self.open).map(|p| p.kind)
    }

    /// What's being searched after `#`, while the palette shows symbols.
    pub fn symbol_query(&self) -> Option<&str> {
        if !self.open || self.picker.is_some() || self.input.is_some() {
            return None;
        }
        if let Some(tab) = self.everywhere_tab() {
            let q = self.query.trim();
            return (matches!(tab, EverywhereTab::All | EverywhereTab::Symbols) && !q.is_empty()).then_some(q);
        }
        self.query.strip_prefix('#').map(str::trim)
    }

    pub fn ui(&mut self, ctx: &egui::Context, finder: &mut FileFinder, keymap: &Keymap) -> Option<PaletteResult> {
        // Arrives from a little higher up.
        let arrived = theme::eased(ctx, egui::Id::unique("palette_arrival"), self.open);
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
            for (i, hl) in rank(picker.items.iter().map(String::as_str), &self.query) {
                let t = &picker.items[i];
                let detail = picker.details.get(i).cloned().unwrap_or_default();
                items.push((t.clone(), detail, PaletteResult::Picked(picker.kind, t.clone()), hl));
            }
        } else if let Some(tab) = self.everywhere_tab() {
            let q = self.query.trim().to_owned();
            let all = tab == EverywhereTab::All;
            // In All each kind gets a few; a tab shows everything it has.
            let cap = |few: usize, many: usize| if all { few } else { many };
            let wanted = |one: EverywhereTab| all || tab == one;
            empty_text = if q.is_empty() { "Type to search" } else { "No matching results" };
            if wanted(EverywhereTab::Files) {
                finder.set_query(&q);
                for m in finder.results(cap(8, 200) as u32) {
                    let name = m.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let dir = Path::new(&m.rel).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                    let name_start = m.rel.chars().count() - name.chars().count();
                    let hl = m.indices.iter().filter_map(|&i| (i as usize).checked_sub(name_start)).collect();
                    items.push((name, dir, PaletteResult::OpenFile(m.path), hl));
                }
                if finder.is_scanning() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            if wanted(EverywhereTab::Symbols) && !q.is_empty() {
                for (i, (name, detail)) in self.symbols.iter().enumerate().take(cap(6, 200)) {
                    items.push((name.clone(), detail.clone(), PaletteResult::WorkspaceSymbol(i), Vec::new()));
                }
                if !all && self.symbols.is_empty() {
                    empty_text = "No symbols (they need a language server)";
                }
            }
            if wanted(EverywhereTab::Commands) && (!all || !q.is_empty()) {
                let commands: Vec<&(Command, &str)> = COMMANDS
                    .iter()
                    .filter(|(cmd, name)| !self.hidden_commands.contains(cmd) && (!all || words_match(name, &q)))
                    .collect();
                for (i, hl) in rank(commands.iter().map(|(_, name)| *name), &q).into_iter().take(cap(6, 500)) {
                    let (cmd, name) = commands[i];
                    items.push((name.to_string(), keymap.shortcut_text(*cmd), PaletteResult::Run(*cmd), hl));
                }
            }
            if wanted(EverywhereTab::Settings) && (!all || !q.is_empty()) {
                let rows: Vec<&(String, String)> =
                    self.setting_rows.iter().filter(|(label, _)| words_match(label, &q)).collect();
                for (i, hl) in rank(rows.iter().map(|(label, _)| label.as_str()), &q).into_iter().take(cap(4, 200)) {
                    let (label, section) = rows[i];
                    items.push((label.clone(), section.clone(), PaletteResult::OpenSetting(label.clone()), hl));
                }
            }
            if wanted(EverywhereTab::Text) {
                let root = self.everywhere_root.clone();
                self.text_search.pump(ctx, &q, root.as_deref());
                for (path, line, text, cols) in self.text_search.results.iter().take(cap(8, TEXT_RESULTS)) {
                    let inside = root.as_deref().and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
                    let detail = format!("{}:{line}", inside.display());
                    items.push((text.clone(), detail, PaletteResult::OpenAt(path.clone(), *line, *cols), Vec::new()));
                }
            }
        } else if let Some(q) = self.query.strip_prefix('>') {
            // Built-in and plugin commands, ranked together.
            let commands: Vec<(&str, String, PaletteResult)> =
                COMMANDS
                    .iter()
                    .filter(|(cmd, _)| !self.hidden_commands.contains(cmd))
                    .map(|(cmd, name)| (*name, keymap.shortcut_text(*cmd), PaletteResult::Run(*cmd)))
                    .chain(self.plugin_commands.iter().map(|(title, key)| {
                        (title.as_str(), "plugin".to_owned(), PaletteResult::RunPlugin(key.clone()))
                    }))
                    .collect();
            let order = rank(commands.iter().map(|c| c.0), q);
            let mut commands: Vec<Option<(&str, String, PaletteResult)>> = commands.into_iter().map(Some).collect();
            for (i, hl) in order {
                if let Some((name, detail, outcome)) = commands[i].take() {
                    items.push((name.to_owned(), detail, outcome, hl));
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

        if let Some(tab) = self.everywhere_tab() {
            let (next, back) = ctx.input_mut(|i| {
                (i.consume_key(egui::Modifiers::NONE, Key::Tab), i.consume_key(egui::Modifiers::SHIFT, Key::Tab))
            });
            if next || back {
                let at = EVERYWHERE_TABS.iter().position(|t| t.0 == tab).unwrap_or(0);
                let count = EVERYWHERE_TABS.len();
                self.everywhere = Some(EVERYWHERE_TABS[(at + if next { 1 } else { count - 1 }) % count].0);
                self.selected = 0;
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
        // Shift+Delete takes the picked item out of a removable list. Windows sends it as
        // Cut, which would cut the query instead.
        let removable = self.picker.as_ref().is_some_and(|p| p.removable);
        let mut remove = None;
        if removable && !items.is_empty() {
            let shift_delete = ctx.input_mut(|i| {
                let cut = i.modifiers.shift && !i.modifiers.ctrl;
                let at = i.events.iter().position(|e| {
                    matches!(e, egui::Event::Key { key: Key::Delete, pressed: true, modifiers, .. } if modifiers.shift)
                        || (cut && matches!(e, egui::Event::Cut))
                });
                at.map(|at| i.events.remove(at)).is_some()
            });
            if shift_delete {
                remove = Some(self.selected);
            }
        }
        let shown_tab = self.everywhere_tab();
        let width = if shown_tab.is_some() {
            (ctx.content_rect().width() * 0.6).clamp(500.0, 860.0)
        } else {
            (ctx.content_rect().width() * 0.5).clamp(400.0, 700.0)
        };
        let area = egui::Area::new(egui::Id::unique("command_palette"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 40.0 - 10.0 * (1.0 - arrived)))
            .show(ctx, |ui| {
                palette_frame(ui).show(ui, |ui| {
                    ui.set_width(width);
                    let hint = self.picker.as_ref().map_or(
                        if self.everywhere.is_some() {
                            "Search everything (Tab changes what)"
                        } else {
                            "Search files by name (> commands, : line, # symbols)"
                        },
                        |p| p.placeholder,
                    );
                    let edit = search_box(ui, &mut self.query, hint);
                    // A click's release can hand focus back to the editor, and Enter would go there too.
                    if !self.just_opened && !edit.has_focus() {
                        edit.request_focus();
                    }
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
                    ui.add_space(6.0);
                    if let Some(tab) = shown_tab {
                        ui.horizontal(|ui| {
                            for (one, name) in EVERYWHERE_TABS {
                                if ui.selectable_label(one == tab, name).clicked() {
                                    self.everywhere = Some(one);
                                    self.selected = 0;
                                }
                            }
                        });
                        ui.separator();
                    }

                    // Room for every item up to the cap (the list otherwise keeps its first, smaller size).
                    let row_h = ROW_HEIGHT + ui.spacing().item_spacing.y;
                    let wanted = (items.len().max(1) as f32 * row_h).min(360.0);
                    let list = egui::ScrollArea::vertical().auto_shrink([false, true]);
                    list.max_height(360.0).min_scrolled_height(wanted).show(ui, |ui| {
                        let mut group = "";
                        for (i, (label, detail, outcome, hl)) in items.iter().enumerate() {
                            if shown_tab == Some(EverywhereTab::All) && group_name(outcome) != group {
                                group = group_name(outcome);
                                ui.label(RichText::new(group).small().strong().color(theme::colors().text_dim));
                            }
                            let selected = i == self.selected;
                            // The whole row is one target, with its background painted under it.
                            let background = ui.painter().add(egui::Shape::Noop);
                            let mut cross = None;
                            let resp = ui.horizontal(|ui| {
                                ui.set_min_width(ui.available_width());
                                ui.set_min_height(ROW_HEIGHT);
                                ui.add_space(8.0);
                                // A removable row: its × at the far right, then the note, then
                                // the label in what's left.
                                if removable {
                                    let size = ui.text_style_height(&egui::TextStyle::Body);
                                    let right_to_left = egui::Layout::right_to_left(egui::Align::Center);
                                    ui.with_layout(right_to_left, |ui| {
                                        let (spot, _) =
                                            ui.allocate_exact_size(egui::vec2(size * 1.4, size), egui::Sense::hover());
                                        cross = Some(egui::Rect::from_center_size(
                                            spot.center(),
                                            egui::vec2(size * 1.2, size * 1.2),
                                        ));
                                        ui.label(RichText::new(detail).small().color(theme::colors().text_dim));
                                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                            let text = highlight_label(ui, label, hl);
                                            ui.add(egui::Label::new(text).truncate().selectable(false));
                                        });
                                    });
                                    return;
                                }
                                let size = ui.text_style_height(&egui::TextStyle::Body);
                                match outcome {
                                    PaletteResult::OpenFile(_) => {
                                        crate::ui::icons::show_file(ui, &format!("{detail}/{label}"), size);
                                    }
                                    PaletteResult::OpenAt(path, ..) => {
                                        crate::ui::icons::show_file(ui, &path.to_string_lossy(), size);
                                    }
                                    PaletteResult::OpenSetting(_) => {
                                        let color = theme::colors().text_dim;
                                        crate::ui::icons::show(ui, crate::ui::icons::Icon::Gear, size, color);
                                    }
                                    _ => {}
                                }
                                let shortcut = matches!(outcome, PaletteResult::Run(_) | PaletteResult::RunPlugin(_));
                                let right_to_left = egui::Layout::right_to_left(egui::Align::Center);
                                ui.with_layout(right_to_left, |ui| {
                                    ui.add_space(6.0);
                                    if shortcut {
                                        key_caps(ui, detail);
                                    } else {
                                        let note = RichText::new(detail).small().color(theme::colors().text_dim);
                                        ui.add(egui::Label::new(note).truncate().selectable(false));
                                    }
                                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                        let text = highlight_label(ui, label, hl);
                                        ui.add(egui::Label::new(text).truncate().selectable(false));
                                    });
                                });
                            });
                            let row = resp.response.rect.expand2(egui::vec2(0.0, ui.spacing().item_spacing.y / 2.0));
                            let row = ui.interact(row, ui.scope_id().with(("palette_row", i)), egui::Sense::click());
                            let pal = theme::colors();
                            let fill = if selected {
                                Some(pal.accent.gamma_multiply(0.2))
                            } else {
                                row.hovered().then_some(pal.hover_bg)
                            };
                            if let Some(fill) = fill {
                                let mut shapes = vec![egui::Shape::rect_filled(row.rect, 6.0, fill)];
                                if selected {
                                    // A bar in the accent color marks the row Enter takes.
                                    let bar = egui::Rect::from_min_size(
                                        row.rect.left_top() + egui::vec2(0.0, 5.0),
                                        egui::vec2(3.0, row.rect.height() - 10.0),
                                    );
                                    shapes.push(egui::Shape::rect_filled(bar, 1.5, pal.accent));
                                }
                                ui.painter().set(background, egui::Shape::Vec(shapes));
                            }
                            if selected && (up || down) {
                                row.scroll_to_me(None);
                            }
                            if let Some(cross) = cross.filter(|_| selected || row.hovered()) {
                                let on = row.hover_pos().is_some_and(|p| cross.contains(p));
                                let color = if on { pal.text } else { pal.text_dim };
                                if on {
                                    ui.painter().rect_filled(cross, 3.0, pal.hover_bg);
                                }
                                ui.painter().text(
                                    cross.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "×",
                                    egui::FontId::proportional(cross.height() * 0.9),
                                    color,
                                );
                            }
                            let on_cross =
                                cross.is_some_and(|c| row.interact_pointer_pos().is_some_and(|p| c.contains(p)));
                            if row.clicked() && on_cross {
                                remove = Some(i);
                            } else if row.clicked() {
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
        // Taken out: the app redraws the list without it, and the palette stays.
        if let Some(i) = remove.filter(|&i| i < items.len())
            && let PaletteResult::Picked(kind, label) = items.swap_remove(i).2
        {
            self.selected = i.min(items.len().saturating_sub(1));
            return Some(PaletteResult::Remove(kind, label));
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
                palette_frame(ui).show(ui, |ui| {
                    ui.set_width(width);
                    let edit = search_box(ui, &mut self.query, "");
                    if !self.just_opened && !edit.has_focus() {
                        edit.request_focus();
                    }
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
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.add_space(4.0);
                        ui.label(RichText::new(placeholder).small().color(theme::colors().text_dim));
                    });
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

/// Height of a row in the list.
const ROW_HEIGHT: f32 = 28.0;

/// The palette's card: rounded, a little padding, a faint border.
fn palette_frame(ui: &egui::Ui) -> egui::Frame {
    let pal = theme::colors();
    egui::Frame::popup(ui.style())
        .fill(pal.sidebar_bg)
        .corner_radius(12.0)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .inner_margin(8.0)
}

/// The query box: a magnifier, the text, and a border that lights up in the accent color.
fn search_box(ui: &mut egui::Ui, query: &mut String, hint: &str) -> egui::Response {
    let pal = theme::colors();
    let id = ui.next_auto_id();
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = egui::Stroke::new(1.0, if focused { pal.accent } else { pal.border });
    let frame = egui::Frame::new().fill(pal.widget_bg).corner_radius(8.0).stroke(stroke).inner_margin(egui::Margin {
        left: 10,
        right: 10,
        top: 7,
        bottom: 7,
    });
    frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let size = ui.text_style_height(&egui::TextStyle::Body);
                crate::ui::icons::show(ui, crate::ui::icons::Icon::Search, size, pal.text_dim);
                ui.add_space(4.0);
                let edit = egui::TextEdit::singleline(query)
                    .id(id)
                    .hint_text(RichText::new(hint).color(pal.text_dim))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY);
                ui.add(edit)
            })
            .inner
        })
        .inner
}

/// A shortcut such as `Ctrl+K Ctrl+S`, each key in its own small box.
fn key_caps(ui: &mut egui::Ui, keys: &str) {
    if keys.is_empty() {
        return;
    }
    let pal = theme::colors();
    let font = egui::FontId::proportional(11.5);
    ui.spacing_mut().item_spacing.x = 3.0;
    // Right to left: the last key first.
    let chords: Vec<&str> = keys.split(' ').collect();
    for (n, chord) in chords.iter().rev().enumerate() {
        if n > 0 {
            ui.add_space(4.0);
        }
        for key in chord.split('+').rev() {
            let galley = ui.painter().layout_no_wrap(key.to_owned(), font.clone(), pal.text_dim);
            let size = egui::vec2((galley.size().x + 10.0).max(20.0), 19.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 4.0, pal.widget_bg);
            p.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            p.galley(rect.center() - galley.size() / 2.0, galley, pal.text_dim);
        }
    }
}

/// The label with fuzzy-matched characters in the accent color. A command's category
/// (`File:` in `File: Save`) is dimmed, so the name stands out.
fn highlight_label(ui: &egui::Ui, label: &str, highlights: &[usize]) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let pal = theme::colors();
    let category = label
        .find(": ")
        .filter(|&at| at <= 24 && !label[..at].contains(['/', '\\']))
        .map_or(0, |at| label[..at + 1].chars().count());
    let mut job = egui::text::LayoutJob::default();
    for (i, c) in label.chars().enumerate() {
        let plain = if i < category { pal.text_dim } else { pal.text };
        let color = if highlights.contains(&i) { pal.accent } else { plain };
        job.append(&c.to_string(), 0.0, egui::TextFormat { font_id: font.clone(), color, ..Default::default() });
    }
    job
}

/// Which part of Search Everywhere's All tab a result is in.
fn group_name(outcome: &PaletteResult) -> &'static str {
    match outcome {
        PaletteResult::OpenFile(_) => "FILES",
        PaletteResult::WorkspaceSymbol(_) => "SYMBOLS",
        PaletteResult::Run(_) | PaletteResult::RunPlugin(_) => "COMMANDS",
        PaletteResult::OpenSetting(_) => "SETTINGS",
        PaletteResult::OpenAt(..) => "TEXT",
        _ => "",
    }
}

/// Every word of the query is somewhere in the text, in any case. Short queries match too
/// much loosely, when mixed in with other results.
fn words_match(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.to_lowercase().split_whitespace().all(|word| text.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs (shift, other, time) frames; returns the times it fired.
    fn fired(frames: &[(bool, bool, f64)]) -> Vec<f64> {
        let mut double = DoubleShift::default();
        frames.iter().filter(|(shift, other, now)| double.update(*shift, *other, *now)).map(|f| f.2).collect()
    }

    #[test]
    fn two_quick_presses_of_shift_alone() {
        let quick = [(true, false, 0.0), (false, false, 0.1), (true, false, 0.2), (false, false, 0.3)];
        assert_eq!(fired(&quick), [0.3]);
    }

    #[test]
    fn typing_or_waiting_is_not_a_double_press() {
        // Too far apart.
        assert!(fired(&[(true, false, 0.0), (false, false, 0.1), (true, false, 2.0), (false, false, 2.1)]).is_empty());
        // Held.
        assert!(fired(&[(true, false, 0.0), (false, false, 1.0), (true, false, 1.1), (false, false, 1.2)]).is_empty());
        // Capitals.
        let typing =
            [(true, false, 0.0), (true, true, 0.05), (false, false, 0.1), (true, false, 0.2), (false, false, 0.3)];
        assert!(fired(&typing).is_empty());
        // A key between the two.
        let between =
            [(true, false, 0.0), (false, false, 0.1), (false, true, 0.15), (true, false, 0.2), (false, false, 0.3)];
        assert!(fired(&between).is_empty());
        // With Ctrl held.
        assert!(fired(&[(true, true, 0.0), (false, true, 0.1), (true, true, 0.2), (false, true, 0.3)]).is_empty());
    }

    #[test]
    fn three_presses_fire_once() {
        let frames = [
            (true, false, 0.0),
            (false, false, 0.1),
            (true, false, 0.2),
            (false, false, 0.3),
            (true, false, 0.4),
            (false, false, 0.5),
        ];
        assert_eq!(fired(&frames), [0.3]);
    }

    #[test]
    fn words_in_any_order() {
        assert!(words_match("View: Toggle Terminal", "terminal toggle"));
        assert!(!words_match("View: Toggle Terminal", "tgl"));
    }
}
