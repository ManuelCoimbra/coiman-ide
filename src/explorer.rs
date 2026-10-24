//! File explorer side bar: a lazily loaded folder tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{RichText, Sense};

use crate::theme;

/// Folders that are never shown or indexed.
pub const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", ".idea", ".vs"];

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
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
        let entries: Vec<(PathBuf, String, bool)> = self.cache[dir]
            .iter()
            .map(|e| (e.path.clone(), e.name.clone(), e.is_dir))
            .collect();

        for (path, name, is_dir) in entries {
            let expanded = self.expanded.contains(&path);
            let selected = active_file == Some(path.as_path());
            let icon = match (is_dir, expanded) {
                (true, true) => "▾ 📂",
                (true, false) => "▸ 📁",
                (false, _) => "   📄",
            };
            if row(ui, depth, &format!("{icon} {name}"), selected).clicked() {
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

/// A full-width clickable row with indentation and hover/selection highlight.
fn row(ui: &mut egui::Ui, depth: usize, text: &str, selected: bool) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let height = (font.size * 1.7).round();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter()
            .rect_filled(rect, 0.0, theme::colors().selection);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 0.0, theme::colors().hover_bg);
    }
    let pos = rect.left_center() + egui::vec2(8.0 + depth as f32 * 12.0, 0.0);
    ui.painter().text(
        pos,
        egui::Align2::LEFT_CENTER,
        text,
        font,
        theme::colors().text,
    );
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
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}
