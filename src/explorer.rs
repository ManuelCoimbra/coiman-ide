//! File explorer side bar: a lazily loaded folder tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{Color32, RichText, Sense};

use crate::icons::{self, Icon};
use crate::theme;

use rust_code_core::workspace::IGNORED_DIRS;

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

#[derive(Default)]
pub struct Explorer {
    pub root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    /// Cached directory listings, cleared on refresh.
    cache: HashMap<PathBuf, Vec<Entry>>,
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
        if !self.cache.contains_key(dir) {
            self.cache.insert(dir.to_path_buf(), read_dir_sorted(dir));
        }
        // Collect what we need so `self` can be borrowed mutably while recursing.
        let entries: Vec<(PathBuf, String, bool)> =
            self.cache[dir].iter().map(|e| (e.path.clone(), e.name.clone(), e.is_dir)).collect();

        for (path, name, is_dir) in entries {
            let expanded = self.expanded.contains(&path);
            let selected = active_file == Some(path.as_path());
            if row(ui, depth, &name, selected, is_dir.then_some(expanded)).clicked() {
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
/// an icon (`folder`: `Some(expanded)` for folders, `None` for files).
fn row(ui: &mut egui::Ui, depth: usize, text: &str, selected: bool, folder: Option<bool>) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let height = (font.size * 1.7).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, theme::colors().selection);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::colors().hover_bg);
    }
    let size = (font.size * 1.1).round();
    let x = rect.left() + 4.0 + depth as f32 * 12.0;
    let icon_box =
        |x: f32| egui::Rect::from_center_size(egui::pos2(x + size / 2.0, rect.center().y), egui::vec2(size, size));
    let dim = theme::colors().text_dim;
    match folder {
        Some(expanded) => {
            let chevron = if expanded { Icon::ChevronDown } else { Icon::ChevronRight };
            icons::paint(ui.painter(), icon_box(x).shrink(size * 0.1), chevron, dim);
            let folder = if expanded { Icon::FolderOpen } else { Icon::Folder };
            icons::paint(ui.painter(), icon_box(x + size), folder, Color32::from_rgb(0xC0, 0x9A, 0x53));
        }
        None => icons::paint_file(ui.painter(), icon_box(x + size), text),
    }
    let pos = egui::pos2(x + 2.0 * size + 6.0, rect.center().y);
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, text, font, theme::colors().text);
    response
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
