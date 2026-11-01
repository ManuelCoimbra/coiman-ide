//! File explorer side bar: a lazily loaded folder tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Sense};

use crate::icons::{self, Icon};
use crate::theme;

use rust_code_core::project_tree::{self, FolderKind};
use rust_code_core::workspace::IGNORED_DIRS;

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

/// What the Explorer's context menu asks for (`None`: the open folder).
pub enum ExplorerAction {
    NewFile(Option<PathBuf>),
    NewFolder(Option<PathBuf>),
    Rename(PathBuf),
    Delete(PathBuf),
}

#[derive(Default)]
pub struct Explorer {
    /// Chosen in a context menu, for the app to carry out.
    pub action: Option<ExplorerAction>,
    pub root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    /// Cached directory listings, cleared on refresh.
    cache: HashMap<PathBuf, Vec<Entry>>,
    /// Show a chain of folders holding only each other as one row.
    pub compact: bool,
    /// Bring this row into view when it's next drawn.
    scroll_to: Option<PathBuf>,
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
        self.expanded.clear();
        self.expanded.insert(root.clone());
        self.root = Some(root);
    }

    pub fn refresh(&mut self) {
        self.cache.clear();
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
        self.scroll_to = Some(file.to_path_buf());
    }

    fn listing(&mut self, dir: &Path) -> &[Entry] {
        self.cache.entry(dir.to_path_buf()).or_insert_with(|| read_dir_sorted(dir))
    }

    /// A folder's row: how far down it reaches when compact, what to call it and how it looks.
    fn folder_row(&mut self, folder: &Path, name: &str) -> (PathBuf, String, FolderLook) {
        let (mut end, mut label) = (folder.to_path_buf(), name.to_owned());
        let (mut kind, mut module) = (FolderKind::Plain, false);
        let root = self.root.clone().unwrap_or_default();
        loop {
            let relative = end.strip_prefix(&root).unwrap_or(&end).to_string_lossy().into_owned();
            let entries = self.listing(&end);
            let children: Vec<(&str, bool)> = entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect();
            kind = kind.max(project_tree::folder_kind(&relative, &children));
            module |= children.iter().any(|(child, dir)| !dir && project_tree::is_module_file(child));
            let only = match entries {
                [only] if only.is_dir => Some((only.path.clone(), only.name.clone())),
                _ => None,
            };
            match only.filter(|_| self.compact) {
                Some((path, name)) => {
                    end = path;
                    label = format!("{label}/{name}");
                }
                None => break,
            }
        }
        let expanded = self.expanded.contains(&end);
        (end, label, FolderLook { expanded, kind, module })
    }

    /// Draws the tree. Returns a file the user clicked, if any.
    pub fn ui(&mut self, ui: &mut egui::Ui, active_file: Option<&Path>) -> Option<PathBuf> {
        let Some(root) = self.root.clone() else {
            ui.add_space(8.0);
            ui.label(RichText::new("You have not yet opened a folder.").color(theme::colors().text_dim));
            ui.add_space(6.0);
            return None;
        };

        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            self.dir_ui(ui, &root, 0, active_file, &mut clicked);
        });
        clicked
    }

    fn dir_ui(
        &mut self,
        ui: &mut egui::Ui,
        dir: &Path,
        depth: usize,
        active_file: Option<&Path>,
        clicked: &mut Option<PathBuf>,
    ) {
        // Collect what we need so `self` can be borrowed mutably while recursing.
        let entries: Vec<(PathBuf, String, bool)> =
            self.listing(dir).iter().map(|e| (e.path.clone(), e.name.clone(), e.is_dir)).collect();

        for (path, name, is_dir) in entries {
            // A compact row stands for its last folder.
            let (path, name, look) = if is_dir {
                let (end, label, look) = self.folder_row(&path, &name);
                (end, label, Some(look))
            } else {
                (path, name, None)
            };
            let expanded = look.is_some_and(|l| l.expanded);
            let selected = active_file == Some(path.as_path());
            let inside = self.root.as_deref().and_then(|r| path.strip_prefix(r).ok()).unwrap_or(&path);
            let place = inside.to_string_lossy().into_owned();
            let response = row(ui, depth, &name, &place, selected, look);
            if self.scroll_to.as_deref() == Some(path.as_path()) {
                response.scroll_to_me(Some(egui::Align::Center));
                self.scroll_to = None;
            }
            let response = match look.and_then(|l| l.kind.label()) {
                Some(label) => response.on_hover_text(label),
                None => response,
            };
            response.context_menu(|ui| {
                // New items go in the folder, or next to the file.
                let dir_of = if is_dir { path.clone() } else { path.parent().unwrap_or(&path).to_path_buf() };
                if ui.button("New File...").clicked() {
                    self.action = Some(ExplorerAction::NewFile(Some(dir_of.clone())));
                    ui.close();
                }
                if ui.button("New Folder...").clicked() {
                    self.action = Some(ExplorerAction::NewFolder(Some(dir_of)));
                    ui.close();
                }
                ui.separator();
                if ui.button("Rename...").clicked() {
                    self.action = Some(ExplorerAction::Rename(path.clone()));
                    ui.close();
                }
                if ui.button("Delete").clicked() {
                    self.action = Some(ExplorerAction::Delete(path.clone()));
                    ui.close();
                }
            });
            if response.clicked() {
                if is_dir {
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
                self.dir_ui(ui, &path, depth + 1, active_file, clicked);
            }
        }
    }
}

/// A full-width clickable row with indentation, hover/selection highlight and
/// an icon (`folder`: `None` for files).
fn row(
    ui: &mut egui::Ui,
    depth: usize,
    text: &str,
    place: &str,
    selected: bool,
    folder: Option<FolderLook>,
) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let height = (font.size * 1.7).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, theme::colors().selection);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::colors().hover_bg);
    }
    let size = (font.size * 1.1).round();
    let x = rect.left() + 4.0 + depth as f32 * INDENT;
    let icon_box =
        |x: f32| egui::Rect::from_center_size(egui::pos2(x + size / 2.0, rect.center().y), egui::vec2(size, size));
    let dim = theme::colors().text_dim;
    // A guide down from each folder above, under its chevron.
    for level in 0..depth {
        let guide = rect.left() + 4.0 + level as f32 * INDENT + size / 2.0;
        ui.painter().vline(guide, rect.y_range(), egui::Stroke::new(1.0, dim.gamma_multiply(0.35)));
    }
    match folder {
        Some(look) => {
            let chevron = if look.expanded { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(ui.painter(), icon_box(x).shrink(size * 0.1), chevron, dim);
            let icon = if look.expanded { Icon::FolderOpen } else { Icon::Folder };
            let place = icon_box(x + size);
            icons::paint(ui.painter(), place, icon, folder_color(look.kind));
            if look.kind.is_test() {
                icons::paint_test_mark(ui.painter(), place);
            } else if look.module {
                let mark = egui::Rect::from_center_size(place.right_bottom(), egui::Vec2::splat(size * 0.36));
                ui.painter().rect_filled(mark.translate(-egui::Vec2::splat(size * 0.14)), 1.0, SOURCE_BLUE);
            }
        }
        None => icons::paint_file(ui.painter(), icon_box(x + size), place),
    }
    let pos = egui::pos2(x + 2.0 * size + 6.0, rect.center().y);
    let excluded = folder.is_some_and(|f| f.kind == FolderKind::Excluded);
    let color = match folder.map(|f| f.kind) {
        Some(kind) if kind.is_test() => folder_color(kind),
        _ if excluded => dim,
        _ => theme::colors().text,
    };
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, text, font, color);
    response
}

const INDENT: f32 = 16.0;
const SOURCE_BLUE: Color32 = Color32::from_rgb(0x4A, 0x90, 0xD9);

/// Like JetBrains IDEs: blue sources, green tests.
fn folder_color(kind: FolderKind) -> Color32 {
    match kind {
        FolderKind::Plain => Color32::from_rgb(0xC0, 0x9A, 0x53),
        FolderKind::Excluded => Color32::from_rgb(0xB0, 0x70, 0x50),
        FolderKind::Source => SOURCE_BLUE,
        FolderKind::Resources => Color32::from_rgb(0x98, 0x76, 0xAA),
        FolderKind::Test => Color32::from_rgb(0x3F, 0xA8, 0x6B),
        FolderKind::TestResources => Color32::from_rgb(0x7F, 0xA8, 0x3F),
    }
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
            if is_dir && IGNORED_DIRS.contains(&name.as_str()) {
                return None;
            }
            Some(Entry { path: e.path(), name, is_dir })
        })
        .collect();
    // Folders first, then case-insensitive by name, like VS Code.
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}
