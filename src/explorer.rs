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

/// What the Explorer's context menu asks for (`None`: the open folder).
pub enum ExplorerAction {
    NewFile(Option<PathBuf>),
    NewFolder(Option<PathBuf>),
    /// In this folder, of this one of `project_tree::TEMPLATES`.
    NewFromTemplate(Option<PathBuf>, usize),
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
    /// What files are, cleared on refresh.
    roles: HashMap<PathBuf, FileRole>,
    reads_left: u32,
    /// Show a chain of packages holding only each other as one row.
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
        self.roles.clear();
        self.expanded.clear();
        self.expanded.insert(root.clone());
        self.root = Some(root);
    }

    pub fn refresh(&mut self) {
        self.cache.clear();
        self.roles.clear();
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

    /// A folder's row: how far down it reaches when packages are compact, what to call it
    /// and how it looks.
    fn folder_row(&mut self, folder: &Path, name: &str) -> (PathBuf, String, FolderLook) {
        let (mut end, mut label) = (folder.to_path_buf(), name.to_owned());
        let root = self.root.clone().unwrap_or_default();
        let relative_of = |path: &Path| path.strip_prefix(&root).unwrap_or(path).to_string_lossy().into_owned();
        let relative = relative_of(folder);
        let names = self.children_of(folder);
        let children: Vec<(&str, bool)> = names.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        let kind = project_tree::folder_kind(&relative, &children);
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
        let mut order: Vec<usize> = (0..project_tree::TEMPLATES.len()).collect();
        order.sort_by_key(|&t| !seen.contains(project_tree::TEMPLATES[t].extension));
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            for template in order {
                let t = &project_tree::TEMPLATES[template];
                let picked = ui.horizontal(|ui| {
                    let size = ui.text_style_height(&egui::TextStyle::Body);
                    icons::show_file(ui, &format!("x.{}", t.extension), size);
                    ui.button(t.label).clicked()
                });
                if picked.inner {
                    self.action = Some(ExplorerAction::NewFromTemplate(dir.clone(), template));
                    ui.close();
                }
            }
        });
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
            self.reads_left = 16;
            let name =
                root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
            let module = self.listing(&root).iter().any(|e| !e.is_dir && project_tree::is_module_file(&e.name));
            let expanded = self.expanded.contains(&root);
            let look = FolderLook { expanded, kind: FolderKind::Plain, module };
            let response = row(ui, 0, &name, &home_relative(&root), false, Row::Folder(look));
            response.context_menu(|ui| {
                ui.menu_button("New", |ui| self.new_menu(ui, None));
            });
            if response.clicked() {
                if expanded {
                    self.expanded.remove(&root);
                } else {
                    self.expanded.insert(root.clone());
                }
            }
            if expanded {
                self.dir_ui(ui, &root, 1, active_file, &mut clicked);
            }
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
            let response = match look {
                Some(look) => row(ui, depth, &name, "", selected, Row::Folder(look)),
                None => {
                    let role = self.role_in_tree(ui.ctx(), &path, &place);
                    // A type goes by its name, as it does in the language.
                    let shown = role
                        .class
                        .filter(|_| project_tree::hides_extension(&name))
                        .map_or(name.as_str(), |_| name.rsplit_once('.').map_or(name.as_str(), |n| n.0));
                    row(ui, depth, shown, "", selected, Row::File(&place, role))
                }
            };
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
                ui.menu_button("New", |ui| self.new_menu(ui, Some(dir_of)));
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

/// What a row shows beside its name.
enum Row<'a> {
    Folder(FolderLook),
    /// Its path in the project, and what it is.
    File(&'a str, FileRole),
}

/// A full-width clickable row with indentation, hover/selection highlight, an icon and
/// after the name a dimmer `hint`.
fn row(ui: &mut egui::Ui, depth: usize, text: &str, hint: &str, selected: bool, what: Row) -> egui::Response {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let height = (font.size * 1.7).round();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 0.0)), 4.0, theme::colors().selection);
    } else if response.hovered() {
        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 0.0)), 4.0, theme::colors().hover_bg);
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
        Row::File(place, role) => icons::paint_source(&painter, icon_box(x + size), place, role),
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
    let code = path.extension().is_some_and(|e| file_icons::is_code(&e.to_string_lossy()));
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
