//! File explorer side bar: a lazily loaded folder tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Sense};

use crate::icons::{self, Icon};
use crate::theme;

use rust_code_core::file_icons;
use rust_code_core::project_tree::{self, FileRole, FolderKind};

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

/// What the Explorer's context menu or keys ask for (`None`: the open folder).
pub enum ExplorerAction {
    NewFile(Option<PathBuf>),
    NewFolder(Option<PathBuf>),
    /// In this folder, of this one of `project_tree::TEMPLATES`.
    NewFromTemplate(Option<PathBuf>, usize),
    Rename(PathBuf),
    /// These files and folders: to the Recycle Bin, or for good (`true`).
    Delete(Vec<PathBuf>, bool),
    /// The copied or cut files into this folder.
    Paste(PathBuf),
}

/// A row as last drawn, for the keyboard to move through.
#[derive(Clone, Debug, PartialEq)]
pub struct RowInfo {
    pub path: PathBuf,
    /// A folder, or a file with others nested under it.
    pub opens: bool,
    pub expanded: bool,
    pub is_dir: bool,
    pub depth: usize,
}

/// Where the keyboard goes from row `at` of `rows`, for an arrow, Home, End or Page key:
/// the row to move to, or a folder to open or close instead (`Some(true)`: open).
pub fn key_target(rows: &[RowInfo], at: Option<usize>, key: egui::Key, page: usize) -> (Option<usize>, Option<bool>) {
    use egui::Key;
    let last = rows.len().checked_sub(1);
    let Some(at) = at.filter(|&a| a < rows.len()) else {
        return (if matches!(key, Key::End | Key::ArrowUp) { last } else { last.map(|_| 0) }, None);
    };
    let row = &rows[at];
    match key {
        Key::ArrowUp => (Some(at.saturating_sub(1)), None),
        Key::ArrowDown => (Some((at + 1).min(rows.len() - 1)), None),
        Key::Home => (Some(0), None),
        Key::End => (last, None),
        Key::PageUp => (Some(at.saturating_sub(page)), None),
        Key::PageDown => (Some((at + page).min(rows.len() - 1)), None),
        // Left closes an open folder, else goes up to the one holding the row.
        Key::ArrowLeft if row.opens && row.expanded => (Some(at), Some(false)),
        Key::ArrowLeft => (Some((0..at).rev().find(|&i| rows[i].depth < row.depth).unwrap_or(at)), None),
        // Right opens a closed folder, else goes into it.
        Key::ArrowRight if row.opens && !row.expanded => (Some(at), Some(true)),
        Key::ArrowRight if row.opens && rows.get(at + 1).is_some_and(|r| r.depth > row.depth) => (Some(at + 1), None),
        _ => (Some(at), None),
    }
}

/// The row after `from` (wrapping) whose name starts with `typed`, ignoring case.
pub fn row_named(rows: &[RowInfo], from: Option<usize>, typed: &str, same_row_ok: bool) -> Option<usize> {
    let typed = typed.to_lowercase();
    let n = rows.len();
    let start = from.map_or(0, |f| if same_row_ok { f } else { f + 1 });
    (0..n).map(|k| (start + k) % n).find(|&i| {
        rows[i].path.file_name().is_some_and(|name| name.to_string_lossy().to_lowercase().starts_with(&typed))
    })
}

#[derive(Default)]
pub struct Explorer {
    /// Chosen in a context menu, for the app to carry out.
    pub action: Option<ExplorerAction>,
    pub root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    /// Cached directory listings, cleared on refresh.
    cache: HashMap<PathBuf, Vec<Entry>>,
    /// What files are, cleared on refresh.
    roles: HashMap<PathBuf, FileRole>,
    reads_left: u32,
    /// Show a chain of packages holding only each other as one row.
    pub compact: bool,
    /// Show a file's tests, styles and what's built from it under it.
    pub nest: bool,
    /// The open folder's `package.json`, as far as asked for.
    manifest: Option<String>,
    /// Bring this row into view when it's next drawn.
    scroll_to: Option<PathBuf>,
    /// Rows picked by click, Ctrl or Shift+click, or the keyboard.
    selection: Vec<PathBuf>,
    /// The row the keyboard is on, and the one Shift ranges from.
    cursor: Option<PathBuf>,
    anchor: Option<PathBuf>,
    /// The rows as last drawn, top to bottom.
    rows: Vec<RowInfo>,
    /// Rows drawn this frame, becoming `rows`.
    drawing: Vec<RowInfo>,
    /// Files copied (`false`) or cut (`true`) for pasting.
    pub clipboard: Option<(Vec<PathBuf>, bool)>,
    /// Letters typed to jump to a row, and when the last came.
    typed: (String, f64),
    /// The open file when last drawn: the selection follows it when it changes.
    last_active: Option<PathBuf>,
    /// A row picked this frame: (path, whether the editor takes the keyboard).
    picked: Option<(PathBuf, bool)>,
}

/// How a row is marked.
#[derive(Clone, Copy, Default)]
struct Marks {
    selected: bool,
    /// The keyboard's row, while the Explorer has the keyboard.
    cursor: bool,
    /// Cut, waiting to be pasted.
    cut: bool,
}

/// Takes `event` out of this frame's input.
fn consume(ui: &egui::Ui, event: &egui::Event) {
    ui.input_mut(|i| i.events.retain(|e| e != event));
}

/// The Explorer's keyboard focus.
pub fn focus_id() -> egui::Id {
    egui::Id::unique("explorer_keyboard")
}

/// How a folder's row looks.
#[derive(Clone, Copy)]
struct FolderLook {
    expanded: bool,
    kind: FolderKind,
    module: bool,
}

impl Explorer {
    pub fn set_root(&mut self, root: PathBuf) {
        self.cache.clear();
        self.roles.clear();
        self.manifest = None;
        self.expanded.clear();
        self.expanded.insert(root.clone());
        self.root = Some(root);
    }

    pub fn refresh(&mut self) {
        self.cache.clear();
        self.roles.clear();
        self.manifest = None;
    }

    /// Whether the Explorer has the keyboard.
    pub fn has_focus(ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(focus_id()))
    }

    /// The rows the next action is about: the selection, else the keyboard's row. Never
    /// the open folder itself, which Delete, Cut and Rename must not touch.
    pub fn picked_rows(&self) -> Vec<PathBuf> {
        let rows =
            if self.selection.is_empty() { self.cursor.iter().cloned().collect() } else { self.selection.clone() };
        rows.into_iter().filter(|p| self.root.as_ref() != Some(p)).collect()
    }

    /// The folder new or pasted things go in: the keyboard's row if a folder, else the
    /// one holding it, else the open folder.
    pub fn target_folder(&self) -> Option<PathBuf> {
        let row = self.cursor.as_ref().and_then(|c| self.rows.iter().find(|r| &r.path == c));
        match row {
            Some(r) if r.is_dir => Some(r.path.clone()),
            Some(r) => r.path.parent().map(Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    /// Forgets rows that are gone, after files were deleted or moved.
    pub fn forget(&mut self, gone: &[PathBuf]) {
        let alive = |p: &PathBuf| !gone.iter().any(|g| p.starts_with(g));
        self.selection.retain(alive);
        if self.cursor.as_ref().is_some_and(|c| !alive(c)) {
            self.cursor = None;
        }
        if let Some((files, _)) = &mut self.clipboard {
            files.retain(alive);
        }
    }

    fn select_only(&mut self, path: &Path) {
        self.selection = vec![path.to_path_buf()];
        self.cursor = Some(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
    }

    /// Selects the rows from the anchor to `path`.
    fn select_range(&mut self, path: &Path) {
        let index = |p: &Path| self.rows.iter().position(|r| r.path == p);
        let to = index(path);
        let from = self.anchor.as_deref().and_then(index).or(to);
        if let (Some(a), Some(b)) = (from, to) {
            let (a, b) = (a.min(b), a.max(b));
            self.selection = self.rows[a..=b].iter().map(|r| r.path.clone()).collect();
        }
        self.cursor = Some(path.to_path_buf());
    }

    /// A click on a row, with the modifiers held: what VS Code's lists do.
    fn click_row(&mut self, ui: &egui::Ui, path: &Path) -> bool {
        let m = ui.input(|i| i.modifiers);
        ui.memory_mut(|mem| mem.request_focus(focus_id()));
        if m.shift {
            self.select_range(path);
            false
        } else if m.command || m.ctrl {
            if let Some(i) = self.selection.iter().position(|p| p == path) {
                self.selection.remove(i);
            } else {
                self.selection.push(path.to_path_buf());
            }
            self.cursor = Some(path.to_path_buf());
            self.anchor = Some(path.to_path_buf());
            false
        } else {
            self.select_only(path);
            true
        }
    }

    /// Opens the keyboard's row: a folder opens or closes, a file opens, in the editor
    /// with the keyboard (`to_editor`) or staying here.
    pub fn open_keyboard_row(&mut self, to_editor: bool) {
        let Some(row) = self.cursor.as_ref().and_then(|c| self.rows.iter().find(|r| &r.path == c)).cloned() else {
            return;
        };
        if row.is_dir {
            if row.expanded {
                self.expanded.remove(&row.path);
            } else {
                self.expanded.insert(row.path);
            }
        } else {
            self.picked = Some((row.path, to_editor));
        }
    }

    /// Renames the keyboard's row, unless it's the open folder.
    pub fn rename_picked(&mut self) {
        if let Some(path) = self.cursor.clone().filter(|c| self.root.as_ref() != Some(c)) {
            self.action = Some(ExplorerAction::Rename(path));
        }
    }

    /// Deletes the picked rows, once confirmed: to the Recycle Bin, or for good.
    pub fn delete_picked(&mut self, permanent: bool) {
        let rows = self.picked_rows();
        if !rows.is_empty() {
            self.action = Some(ExplorerAction::Delete(rows, permanent));
        }
    }

    /// Copies or cuts the picked rows for Paste, with their paths on the clipboard too.
    pub fn copy_picked(&mut self, ctx: &egui::Context, cut: bool) {
        let rows = self.picked_rows();
        if !rows.is_empty() {
            let text: Vec<String> = rows.iter().map(|p| p.display().to_string()).collect();
            ctx.copy_text(text.join("\n"));
            self.clipboard = Some((rows, cut));
        }
    }

    /// Puts the picked rows' paths on the clipboard, full or from the open folder.
    pub fn copy_paths(&self, ctx: &egui::Context, relative: bool) {
        let root = self.root.clone().unwrap_or_default();
        let text: Vec<String> = self
            .picked_rows()
            .iter()
            .map(|p| {
                if relative {
                    p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/")
                } else {
                    p.display().to_string()
                }
            })
            .collect();
        if !text.is_empty() {
            ctx.copy_text(text.join("\n"));
        }
    }

    /// Selects every row but the open folder.
    pub fn select_all(&mut self) {
        let root = self.root.clone();
        self.selection = self.rows.iter().map(|r| r.path.clone()).filter(|p| Some(p) != root.as_ref()).collect();
    }

    /// Pastes what was copied or cut into the keyboard's folder.
    pub fn paste_here(&mut self) {
        if self.clipboard.is_some()
            && let Some(dir) = self.target_folder()
        {
            self.action = Some(ExplorerAction::Paste(dir));
        }
    }

    /// Keys while the Explorer has the keyboard. A key it acts on is taken out of the
    /// input, so the editor it hands the keyboard to doesn't act on it too.
    fn keys(&mut self, ui: &egui::Ui) {
        use egui::Key;
        if !Self::has_focus(ui.ctx()) || self.rows.is_empty() {
            return;
        }
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                focus_id(),
                egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true },
            )
        });
        let at = self.cursor.as_ref().and_then(|c| self.rows.iter().position(|r| &r.path == c));
        let events = ui.input(|i| i.events.clone());
        let now = ui.input(|i| i.time);
        for event in events {
            match event {
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::ArrowUp
                    | Key::ArrowDown
                    | Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::Home
                    | Key::End
                    | Key::PageUp
                    | Key::PageDown
                        if !modifiers.ctrl && !modifiers.alt =>
                    {
                        let (to, open) = key_target(&self.rows, at, key, 10);
                        if let Some(open) = open {
                            let path = self.rows[at.unwrap_or(0)].path.clone();
                            if open {
                                self.expanded.insert(path);
                            } else {
                                self.expanded.remove(&path);
                            }
                        }
                        if let Some(to) = to {
                            let path = self.rows[to].path.clone();
                            if modifiers.shift {
                                self.select_range(&path);
                            } else {
                                self.select_only(&path);
                            }
                            self.scroll_to = Some(path);
                        }
                        return consume(ui, &event);
                    }
                    Key::Escape => {
                        self.selection = self.cursor.iter().cloned().collect();
                        if self.clipboard.as_ref().is_some_and(|(_, cut)| *cut) {
                            self.clipboard = None;
                        }
                        return consume(ui, &event);
                    }
                    _ => {}
                },
                // Typing a name jumps to it; letters within a second add to it.
                egui::Event::Text(ref text) if !text.chars().any(char::is_control) && text != " " => {
                    let fresh = now - self.typed.1 > 1.0;
                    if fresh {
                        self.typed.0.clear();
                    }
                    self.typed.0.push_str(text);
                    self.typed.1 = now;
                    let one_letter = self.typed.0.chars().count() == 1;
                    if let Some(to) = row_named(&self.rows, at, &self.typed.0, !one_letter) {
                        let path = self.rows[to].path.clone();
                        self.select_only(&path);
                        self.scroll_to = Some(path);
                    }
                    return consume(ui, &event);
                }
                _ => {}
            }
        }
    }

    pub fn collapse_all(&mut self) {
        self.expanded.retain(|path| self.root.as_ref() == Some(path));
    }

    /// Opens the folders down to a file, and scrolls to it.
    pub fn reveal(&mut self, file: &Path) {
        let Some(root) = self.root.clone() else { return };
        if !file.starts_with(&root) {
            return;
        }
        for folder in file.ancestors().skip(1).take_while(|f| f.starts_with(&root)) {
            self.expanded.insert(folder.to_path_buf());
        }
        if let (Some(dir), Some(name)) = (file.parent(), file.file_name()) {
            let top = self.nested_in(dir).remove(name.to_string_lossy().as_ref());
            self.expanded.extend(top.map(|top| dir.join(top)));
        }
        self.scroll_to = Some(file.to_path_buf());
    }

    fn listing(&mut self, dir: &Path) -> &[Entry] {
        self.cache.entry(dir.to_path_buf()).or_insert_with(|| read_dir_sorted(dir))
    }

    /// A folder's row: how far down it reaches when packages are compact, what to call it
    /// and how it looks.
    fn folder_row(&mut self, folder: &Path, name: &str) -> (PathBuf, String, FolderLook) {
        let (mut end, mut label) = (folder.to_path_buf(), name.to_owned());
        let root = self.root.clone().unwrap_or_default();
        let relative_of = |path: &Path| path.strip_prefix(&root).unwrap_or(path).to_string_lossy().into_owned();
        let relative = relative_of(folder);
        let names = self.children_of(folder);
        let children: Vec<(&str, bool)> = names.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let beside_manifest =
            folder.parent().is_some_and(|up| self.listing(up).iter().any(|e| e.name == "package.json"));
        let kind = project_tree::folder_kind_beside(&relative, &children, beside_manifest);
        let mut module = children.iter().any(|(child, dir)| !dir && project_tree::is_module_file(child));
        while self.compact && project_tree::is_root_package(&relative_of(&end)) {
            let (path, name) = match self.listing(&end) {
                [only] if only.is_dir => (only.path.clone(), only.name.clone()),
                _ => break,
            };
            module |= self.listing(&path).iter().any(|e| !e.is_dir && project_tree::is_module_file(&e.name));
            end = path;
            label = format!("{label}.{name}");
        }
        let expanded = self.expanded.contains(&end);
        (end, label, FolderLook { expanded, kind, module })
    }

    /// Which files of a folder go under which.
    fn nested_in(&mut self, dir: &Path) -> HashMap<String, String> {
        if !self.nest {
            return HashMap::new();
        }
        let entries = self.listing(dir);
        let names: Vec<&str> = entries.iter().filter(|e| !e.is_dir).map(|e| e.name.as_str()).collect();
        project_tree::nested_files(&names)
    }

    fn children_of(&mut self, dir: &Path) -> Vec<(String, bool)> {
        self.listing(dir).iter().map(|e| (e.name.clone(), e.is_dir)).collect()
    }

    /// What a file is, once read. Never more than a few new ones a frame, in case a folder
    /// holds thousands: the rest show what their names say, and catch up.
    fn role_in_tree(&mut self, ctx: &egui::Context, path: &Path, place: &str) -> FileRole {
        if self.roles.contains_key(path) {
            return self.roles[path];
        }
        if self.reads_left == 0 {
            ctx.request_repaint();
            return FileRole::of_name(place);
        }
        self.reads_left -= 1;
        self.role_of(path, place)
    }

    /// What a file is, reading it the first time it's asked for.
    pub fn role_of(&mut self, path: &Path, place: &str) -> FileRole {
        *self.roles.entry(path.to_path_buf()).or_insert_with(|| read_role(path, place))
    }

    /// What can be made in a folder, the languages already seen in the project first.
    pub fn new_menu(&mut self, ui: &mut egui::Ui, dir: Option<PathBuf>) {
        if ui.button("File").clicked() {
            self.action = Some(ExplorerAction::NewFile(dir.clone()));
            ui.close();
        }
        if ui.button("Folder").clicked() {
            self.action = Some(ExplorerAction::NewFolder(dir.clone()));
            ui.close();
        }
        ui.separator();
        let seen: HashSet<String> =
            self.cache.values().flatten().filter_map(|e| e.name.rsplit_once('.').map(|n| n.1.to_lowercase())).collect();
        let manifest = self.manifest.get_or_insert_with(|| {
            let file = self.root.as_ref().map(|r| r.join("package.json"));
            file.and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default()
        });
        let listed = |t: &project_tree::Template| t.needs.is_empty() || manifest.contains(&format!("\"{}\"", t.needs));
        let mut order: Vec<usize> =
            (0..project_tree::TEMPLATES.len()).filter(|&t| listed(&project_tree::TEMPLATES[t])).collect();
        order.sort_by_key(|&t| {
            let template = &project_tree::TEMPLATES[t];
            (template.needs.is_empty(), !seen.contains(template.extension))
        });
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for template in order {
                let t = &project_tree::TEMPLATES[template];
                let picked = ui.horizontal(|ui| {
                    let size = ui.text_style_height(&egui::TextStyle::Body);
                    let sample = t.fixed_name().map_or_else(|| format!("x.{}", t.extension), str::to_owned);
                    icons::show_file(ui, &sample, size);
                    ui.button(t.label).clicked()
                });
                if picked.inner {
                    self.action = Some(ExplorerAction::NewFromTemplate(dir.clone(), template));
                    ui.close();
                }
            }
        });
    }

    /// Draws the tree. Returns a file to open, and whether the editor takes the keyboard
    /// (a double click or Enter) or the Explorer keeps it (a click or Space).
    pub fn ui(&mut self, ui: &mut egui::Ui, active_file: Option<&Path>) -> Option<(PathBuf, bool)> {
        let Some(root) = self.root.clone() else {
            ui.add_space(8.0);
            ui.label(RichText::new("You have not yet opened a folder.").color(theme::colors().text_dim));
            ui.add_space(6.0);
            return None;
        };
        // The selection follows the open file, as VS Code's does.
        if self.last_active.as_deref() != active_file {
            self.last_active = active_file.map(Path::to_path_buf);
            if let Some(file) = active_file {
                self.selection = vec![file.to_path_buf()];
                self.cursor = Some(file.to_path_buf());
                self.anchor = Some(file.to_path_buf());
            }
        }
        self.keys(ui);
        let from_keys = self.picked.take();
        let (mut clicked, mut to_editor) = (None, false);
        let focused = Self::has_focus(ui.ctx());
        let area = ui.available_rect_before_wrap();
        ui.interact(area, focus_id(), Sense::focusable_noninteractive());
        self.drawing.clear();

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.reads_left = 16;
            let name =
                root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
            let module = self.listing(&root).iter().any(|e| !e.is_dir && project_tree::is_module_file(&e.name));
            let expanded = self.expanded.contains(&root);
            let look = FolderLook { expanded, kind: FolderKind::Plain, module };
            let marks = self.marks(&root, focused);
            let response = row(ui, 0, &name, &home_relative(&root), marks, Row::Folder(look));
            self.drawing.push(RowInfo { path: root.clone(), opens: true, expanded, is_dir: true, depth: 0 });
            response.context_menu(|ui| {
                ui.menu_button("New", |ui| self.new_menu(ui, None));
                if self.clipboard.is_some() && ui.button("Paste\tCtrl+V").clicked() {
                    self.action = Some(ExplorerAction::Paste(root.clone()));
                    ui.close();
                }
            });
            if response.clicked() && self.click_row(ui, &root) {
                if expanded {
                    self.expanded.remove(&root);
                } else {
                    self.expanded.insert(root.clone());
                }
            }
            if expanded {
                self.dir_ui(ui, &root, 1, focused, &mut clicked);
            }
        });
        self.rows = std::mem::take(&mut self.drawing);
        // A double click takes the file to the editor; a single one leaves the keyboard here.
        if let Some((path, editor)) = self.picked.take().or(from_keys) {
            clicked = Some(path);
            to_editor = editor;
        } else if clicked.is_some() && !to_editor {
            ui.memory_mut(|m| m.request_focus(focus_id()));
        }
        clicked.map(|path| (path, to_editor))
    }

    /// How row `path` is marked: selected, the keyboard's, cut.
    fn marks(&self, path: &Path, focused: bool) -> Marks {
        Marks {
            selected: self.selection.iter().any(|p| p == path),
            cursor: focused && self.cursor.as_deref() == Some(path),
            cut: self.clipboard.as_ref().is_some_and(|(files, cut)| *cut && files.iter().any(|f| f == path)),
        }
    }

    fn dir_ui(&mut self, ui: &mut egui::Ui, dir: &Path, depth: usize, focused: bool, clicked: &mut Option<PathBuf>) {
        // Collect what we need so `self` can be borrowed mutably while recursing.
        let entries: Vec<(PathBuf, String, bool)> =
            self.listing(dir).iter().map(|e| (e.path.clone(), e.name.clone(), e.is_dir)).collect();
        let nested = self.nested_in(dir);
        for (path, name, is_dir) in &entries {
            if nested.contains_key(name) {
                continue;
            }
            let under: Vec<(PathBuf, String)> = entries
                .iter()
                .filter(|(_, child, _)| nested.get(child) == Some(name))
                .map(|(path, child, _)| (path.clone(), child.clone()))
                .collect();
            self.entry_ui(ui, (path.clone(), name.clone(), *is_dir), &under, depth, focused, clicked);
        }
    }

    /// A row, and whatever is open under it.
    fn entry_ui(
        &mut self,
        ui: &mut egui::Ui,
        (path, name, is_dir): (PathBuf, String, bool),
        under: &[(PathBuf, String)],
        depth: usize,
        focused: bool,
        clicked: &mut Option<PathBuf>,
    ) {
        // A compact row stands for its last folder.
        let (path, name, look) = if is_dir {
            let (end, label, look) = self.folder_row(&path, &name);
            (end, label, Some(look))
        } else {
            (path, name, None)
        };
        let expanded = match look {
            Some(look) => look.expanded,
            None => !under.is_empty() && self.expanded.contains(&path),
        };
        let marks = self.marks(&path, focused);
        self.drawing.push(RowInfo { path: path.clone(), opens: is_dir || !under.is_empty(), expanded, is_dir, depth });
        let inside = self.root.as_deref().and_then(|r| path.strip_prefix(r).ok()).unwrap_or(&path);
        let place = inside.to_string_lossy().into_owned();
        let response = match look {
            Some(look) => row(ui, depth, &name, "", marks, Row::Folder(look)),
            None => {
                let role = self.role_in_tree(ui.ctx(), &path, &place);
                // A type goes by its name, as it does in the language.
                let named = role.class.is_some() && project_tree::hides_extension(&name);
                let shown = if named { name.rsplit_once('.').map_or(name.as_str(), |n| n.0) } else { name.as_str() };
                let parent = (!under.is_empty()).then_some(expanded);
                row(ui, depth, shown, "", marks, Row::File(&place, role, parent))
            }
        };
        if self.scroll_to.as_deref() == Some(path.as_path()) {
            response.scroll_to_me(Some(egui::Align::Center));
            self.scroll_to = None;
        }
        let tip = look.and_then(|l| l.kind.label());
        let on_chevron = response.interact_pointer_pos().is_some_and(|at| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            at.x < response.rect.left() + 4.0 + depth as f32 * INDENT + (font.size * 1.1).round()
        });
        let response = match tip {
            Some(label) => response.on_hover_text(label),
            None => response,
        };
        // A right click on a row outside the selection picks that row alone.
        if response.secondary_clicked() && !self.selection.contains(&path) {
            self.select_only(&path);
        }
        response.context_menu(|ui| {
            // New items go in the folder, or next to the file.
            let dir_of = if is_dir { path.clone() } else { path.parent().unwrap_or(&path).to_path_buf() };
            ui.menu_button("New", |ui| self.new_menu(ui, Some(dir_of.clone())));
            ui.separator();
            let rows = self.picked_rows();
            if ui.button("Cut\tCtrl+X").clicked() {
                self.clipboard = Some((rows.clone(), true));
                ui.close();
            }
            if ui.button("Copy\tCtrl+C").clicked() {
                self.clipboard = Some((rows.clone(), false));
                ui.close();
            }
            if self.clipboard.is_some() && ui.button("Paste\tCtrl+V").clicked() {
                self.action = Some(ExplorerAction::Paste(dir_of));
                ui.close();
            }
            ui.separator();
            if ui.button("Copy Path\tShift+Alt+C").clicked() {
                let text: Vec<String> = rows.iter().map(|p| p.display().to_string()).collect();
                ui.ctx().copy_text(text.join("\n"));
                ui.close();
            }
            if ui.button("Copy Relative Path").clicked() {
                let root = self.root.clone().unwrap_or_default();
                let text: Vec<String> = rows
                    .iter()
                    .map(|p| p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/"))
                    .collect();
                ui.ctx().copy_text(text.join("\n"));
                ui.close();
            }
            ui.separator();
            if ui.button("Rename...\tF2").clicked() {
                self.action = Some(ExplorerAction::Rename(path.clone()));
                ui.close();
            }
            if ui.button("Delete\tDelete").clicked() {
                self.action = Some(ExplorerAction::Delete(rows, false));
                ui.close();
            }
        });
        let plain_click = response.clicked() && self.click_row(ui, &path);
        if response.double_clicked() && !is_dir {
            self.picked = Some((path.clone(), true));
        }
        if plain_click {
            if is_dir || (!under.is_empty() && on_chevron) {
                if expanded {
                    self.expanded.remove(&path);
                } else {
                    self.expanded.insert(path.clone());
                }
            } else {
                *clicked = Some(path.clone());
            }
        }
        if is_dir && expanded {
            self.dir_ui(ui, &path, depth + 1, focused, clicked);
        } else if expanded {
            for (child, name) in under {
                self.entry_ui(ui, (child.clone(), name.clone(), false), &[], depth + 1, focused, clicked);
            }
        }
    }
}

/// What a row shows beside its name.
enum Row<'a> {
    Folder(FolderLook),
    /// Its path in the project, what it is, and whether what's under it is open.
    File(&'a str, FileRole, Option<bool>),
}

/// A full-width clickable row with indentation, hover/selection highlight, an icon and
/// after the name a dimmer `hint`.
fn row(ui: &mut egui::Ui, depth: usize, text: &str, hint: &str, marks: Marks, what: Row) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let height = (font.size * 1.7).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter_at(rect);
    let band = rect.shrink2(egui::vec2(2.0, 0.0));
    if marks.selected {
        painter.rect_filled(band, 4.0, theme::colors().selection);
    } else if response.hovered() {
        painter.rect_filled(band, 4.0, theme::colors().hover_bg);
    }
    if marks.cursor {
        painter.rect_stroke(
            band.shrink(0.5),
            4.0,
            egui::Stroke::new(1.0, theme::colors().accent),
            egui::StrokeKind::Inside,
        );
    }
    let size = (font.size * 1.1).round();
    let x = rect.left() + 4.0 + depth as f32 * INDENT;
    let icon_box =
        |x: f32| egui::Rect::from_center_size(egui::pos2(x + size / 2.0, rect.center().y), egui::vec2(size, size));
    let dim = theme::colors().text_dim;
    let mut color = theme::colors().text;
    match what {
        Row::Folder(look) => {
            let chevron = if look.expanded { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(&painter, icon_box(x).shrink(size * 0.1), chevron, dim);
            let place = icon_box(x + size);
            icons::paint(&painter, place, Icon::Folder, folder_color(look.kind));
            let at = |px: f32, py: f32| place.min + egui::vec2(px, py) * size;
            match look.kind {
                FolderKind::Package => {
                    painter.circle_filled(at(0.5, 0.58), size * 0.09, folder_color(look.kind));
                }
                FolderKind::Resources | FolderKind::TestResources => {
                    let stroke = egui::Stroke::new(1.0, Color32::from_rgb(0xE0, 0xB0, 0x40));
                    for (y, from) in [(0.46, 0.46), (0.59, 0.34), (0.72, 0.46)] {
                        painter.line_segment([at(from, y), at(0.8, y)], stroke);
                    }
                }
                FolderKind::Excluded => color = dim,
                _ => {}
            }
            if look.module {
                let mark = egui::Rect::from_center_size(at(0.84, 0.8), egui::Vec2::splat(size * 0.36));
                painter.rect_filled(mark, 1.0, SOURCE_BLUE);
            }
        }
        Row::File(place, role, parent) => {
            if let Some(open) = parent {
                let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
                icons::paint(&painter, icon_box(x).shrink(size * 0.1), chevron, dim);
            }
            icons::paint_source(&painter, icon_box(x + size), place, role);
        }
    }
    if marks.cut {
        color = dim;
    }
    let pos = egui::pos2(x + 2.0 * size + 6.0, rect.center().y);
    let name = painter.text(pos, egui::Align2::LEFT_CENTER, text, font.clone(), color);
    if !hint.is_empty() {
        let pos = egui::pos2(name.right() + 8.0, rect.center().y);
        painter.text(pos, egui::Align2::LEFT_CENTER, hint, font, dim);
    }
    response
}

const INDENT: f32 = 18.0;
const SOURCE_BLUE: Color32 = Color32::from_rgb(0x54, 0x8A, 0xF7);

/// Like JetBrains IDEs: blue sources, green tests, orange output, the rest in plain outline.
fn folder_color(kind: FolderKind) -> Color32 {
    match kind {
        FolderKind::Plain | FolderKind::Package | FolderKind::Resources => theme::colors().text.gamma_multiply(0.8),
        FolderKind::Excluded => Color32::from_rgb(0xD0, 0x7F, 0x4A),
        FolderKind::Source => SOURCE_BLUE,
        FolderKind::Test | FolderKind::TestResources => Color32::from_rgb(0x57, 0xB0, 0x6A),
    }
}

/// `~` for the home folder, as a place to show.
fn home_relative(path: &Path) -> String {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(inside) => format!("~{}{}", std::path::MAIN_SEPARATOR, inside.display()),
        None => path.display().to_string(),
    }
}

/// Reads as much of a file as says what it is, if it holds code.
fn read_role(path: &Path, place: &str) -> FileRole {
    use std::io::Read;
    const ENOUGH: u64 = 128 * 1024;
    let code = path.extension().is_some_and(|e| {
        let extension = e.to_string_lossy();
        file_icons::is_code(&extension) || matches!(extension.to_lowercase().as_str(), "html" | "htm")
    });
    let mut bytes = Vec::new();
    let read = code && std::fs::File::open(path).is_ok_and(|f| f.take(ENOUGH).read_to_end(&mut bytes).is_ok());
    if read { FileRole::of(place, &String::from_utf8_lossy(&bytes)) } else { FileRole::of_name(place) }
}

fn read_dir_sorted(dir: &Path) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().ok()?.is_dir();
            if is_dir && name == ".git" {
                return None;
            }
            Some(Entry { path: e.path(), name, is_dir })
        })
        .collect();
    // Folders first, then case-insensitive by name.
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    fn rows() -> Vec<RowInfo> {
        // proj/  src/  main.rs  lib.rs  README.md, with src open.
        let r = |p: &str, opens: bool, expanded: bool, is_dir: bool, depth: usize| RowInfo {
            path: PathBuf::from(p),
            opens,
            expanded,
            is_dir,
            depth,
        };
        vec![
            r("proj", true, true, true, 0),
            r("proj/src", true, true, true, 1),
            r("proj/src/main.rs", false, false, false, 2),
            r("proj/src/lib.rs", false, false, false, 2),
            r("proj/README.md", false, false, false, 1),
        ]
    }

    #[test]
    fn arrows_move_open_and_close_like_vs_code() {
        let rows = rows();
        let go = |at: usize, key: Key| key_target(&rows, Some(at), key, 2);
        assert_eq!(go(2, Key::ArrowDown), (Some(3), None));
        assert_eq!(go(4, Key::ArrowDown), (Some(4), None), "stays on the last row");
        assert_eq!(go(0, Key::ArrowUp), (Some(0), None));
        assert_eq!(go(3, Key::Home), (Some(0), None));
        assert_eq!(go(0, Key::End), (Some(4), None));
        assert_eq!(go(0, Key::PageDown), (Some(2), None));
        // Left on an open folder closes it; on a file, goes to its folder.
        assert_eq!(go(1, Key::ArrowLeft), (Some(1), Some(false)));
        assert_eq!(go(3, Key::ArrowLeft), (Some(1), None));
        // Right on an open folder goes in; on a closed one, opens it.
        assert_eq!(go(1, Key::ArrowRight), (Some(2), None));
        let mut closed = rows.clone();
        closed[1].expanded = false;
        assert_eq!(key_target(&closed, Some(1), Key::ArrowRight, 2), (Some(1), Some(true)));
        // Nothing picked yet: Down starts at the top, Up at the bottom.
        assert_eq!(key_target(&rows, None, Key::ArrowDown, 2), (Some(0), None));
        assert_eq!(key_target(&rows, None, Key::ArrowUp, 2), (Some(4), None));
    }

    #[test]
    fn the_open_folder_is_never_picked() {
        let mut explorer = Explorer { root: Some(PathBuf::from("proj")), ..Default::default() };
        explorer.rows = rows();
        explorer.selection = vec![PathBuf::from("proj"), PathBuf::from("proj/README.md")];
        assert_eq!(explorer.picked_rows(), [PathBuf::from("proj/README.md")]);
        explorer.selection.clear();
        explorer.cursor = Some(PathBuf::from("proj"));
        assert!(explorer.picked_rows().is_empty());
        // New and pasted things go in the keyboard's folder, or the one holding its file.
        assert_eq!(explorer.target_folder(), Some(PathBuf::from("proj")));
        explorer.cursor = Some(PathBuf::from("proj/src/lib.rs"));
        assert_eq!(explorer.target_folder(), Some(PathBuf::from("proj/src")));
        // Gone files leave the selection and the clipboard.
        explorer.selection = vec![PathBuf::from("proj/src/lib.rs"), PathBuf::from("proj/README.md")];
        explorer.clipboard = Some((vec![PathBuf::from("proj/src/main.rs")], true));
        explorer.forget(&[PathBuf::from("proj/src")]);
        assert_eq!(explorer.selection, [PathBuf::from("proj/README.md")]);
        assert_eq!(explorer.clipboard, Some((vec![], true)));
        assert_eq!(explorer.cursor, None);
    }

    #[test]
    fn typing_jumps_to_a_name() {
        let rows = rows();
        assert_eq!(row_named(&rows, Some(0), "l", false), Some(3));
        assert_eq!(row_named(&rows, Some(3), "r", false), Some(4));
        // The same letter again moves on, wrapping.
        assert_eq!(row_named(&rows, Some(4), "r", false), Some(4));
        assert_eq!(row_named(&rows, Some(2), "ma", true), Some(2));
        assert_eq!(row_named(&rows, Some(0), "zz", false), None);
    }
}
