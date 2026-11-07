//! Recent projects and files: Open Recent (Ctrl+R), the Welcome page's list and the File
//! menu's, each able to open, remove and clear them, and each marking those that are gone.

use std::path::{Path, PathBuf};

use coiman_core::recent::{self, Entry};
use eframe::egui::{self, RichText};

use super::CodeApp;
use crate::palette::PickerKind;
use crate::theme;

/// The last row of Open Recent.
pub(super) const CLEAR: &str = "Clear Recently Opened…";

/// What a click in a recent list asks for.
pub(super) enum RecentClick {
    Open(PathBuf),
    Remove(PathBuf),
    Clear,
    More,
}

/// "Folder · 3 days ago · not found", for an entry.
fn detail(kind: &str, entry: &Entry, now: u64) -> String {
    let mut parts = vec![kind.to_owned()];
    let when = recent::ago(entry.opened, now);
    if !when.is_empty() {
        parts.push(when);
    }
    if !entry.exists() {
        parts.push("not found".to_owned());
    }
    parts.join(" · ")
}

/// Where an entry is, shortened under the home folder.
fn place(path: &Path) -> String {
    let parent = path.parent().unwrap_or(path);
    match std::env::var_os("USERPROFILE").map(PathBuf::from) {
        Some(home) if parent.starts_with(&home) => {
            format!("~\\{}", parent.strip_prefix(&home).unwrap_or(parent).display())
        }
        _ => parent.display().to_string(),
    }
}

impl CodeApp {
    /// A file dialog that opens where dialog `kind` was last left, else in the open folder.
    pub(super) fn file_dialog(&self, kind: &str) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match self.dialog_dirs.get(kind).map(Path::to_path_buf).or_else(|| self.explorer.root.clone()) {
            Some(dir) => dialog.set_directory(dir),
            None => dialog,
        }
    }

    /// Remembers where dialog `kind` chose `chosen`, for next time.
    pub(super) fn dialog_chose(&mut self, kind: &str, chosen: &Path) {
        self.dialog_dirs.remember(kind, chosen);
        if let Err(e) = coiman_core::dialog_dirs::save(&self.dialog_dirs) {
            self.status = Some(format!("Could not save dialog folders: {e}"));
        }
    }

    /// Saves the recent list, saying so if it can't be.
    pub(super) fn remember_recent(&mut self) {
        if let Err(e) = recent::save(&self.recent) {
            self.status = Some(format!("Could not save the recent list: {e}"));
        }
    }

    /// Open Recent's rows: projects, then files, then Clear.
    fn recent_rows(&self) -> (Vec<String>, Vec<String>) {
        let now = recent::now();
        let mut items = Vec::new();
        let mut details = Vec::new();
        for (kind, list) in [("Folder", &self.recent.projects), ("File", &self.recent.files)] {
            for entry in list {
                items.push(entry.path.display().to_string());
                details.push(detail(kind, entry, now));
            }
        }
        if !self.recent.is_empty() {
            items.push(CLEAR.to_owned());
            details.push(String::new());
        }
        (items, details)
    }

    /// Opens the Open Recent picker.
    pub(super) fn show_recent(&mut self) {
        if self.recent.is_empty() {
            self.status = Some("Nothing has been opened recently".to_owned());
            return;
        }
        let (items, details) = self.recent_rows();
        self.palette.show_removable_picker(
            PickerKind::Recent,
            items,
            details,
            "Open a recent folder or file (× or Shift+Delete removes one)",
        );
    }

    /// Opens a recent folder or file, or asks before clearing them all. One that is gone
    /// stays listed, as its drive may only be disconnected.
    pub(super) fn open_recent(&mut self, label: &str) {
        if label == CLEAR {
            self.ask_clear_recent();
            return;
        }
        let path = PathBuf::from(label);
        if path.is_dir() {
            self.open_folder(path);
        } else if path.is_file() {
            self.open_file(&path);
        } else {
            self.status = Some(format!(
                "{} isn't there: moved, deleted or on a drive that isn't connected. Its × in Open Recent removes it.",
                path.display()
            ));
        }
    }

    /// Takes one entry out of the list, and out of Open Recent if it's showing.
    pub(super) fn remove_recent(&mut self, path: &Path) {
        if self.recent.remove(path) {
            self.remember_recent();
            self.status = Some(format!("Removed {} from Recent", path.display()));
        }
        if self.palette.picker_kind() == Some(PickerKind::Recent) {
            if self.recent.is_empty() {
                self.palette.open = false;
            } else {
                let (items, details) = self.recent_rows();
                self.palette.replace_picker_items(items, details);
            }
        }
    }

    /// Asks before forgetting every recent folder and file.
    pub(super) fn ask_clear_recent(&mut self) {
        if self.recent.is_empty() {
            self.status = Some("Nothing has been opened recently".to_owned());
            return;
        }
        let count = self.recent.projects.len() + self.recent.files.len();
        let items = vec![format!("Clear all {count} recent folders and files"), "Cancel".to_owned()];
        self.palette.show_picker(PickerKind::ClearRecent, items, "", "Clear Recently Opened? This can't be undone.");
    }

    pub(super) fn clear_recent(&mut self) {
        self.recent.clear();
        self.remember_recent();
        self.status = Some("Cleared Recently Opened".to_owned());
    }

    /// Forgets the folders and files that are gone.
    pub(super) fn remove_missing_recent(&mut self) {
        let gone = self.recent.remove_missing();
        self.remember_recent();
        self.status = Some(match gone {
            0 => "Every recent folder and file is still there".to_owned(),
            1 => "Removed 1 missing entry from Recent".to_owned(),
            n => format!("Removed {n} missing entries from Recent"),
        });
    }

    /// File > Open Recent: a few projects and files, then More and Clear.
    pub(super) fn recent_menu(&self, ui: &mut egui::Ui) -> Option<RecentClick> {
        let mut click = None;
        for list in [&self.recent.projects, &self.recent.files] {
            for entry in list.iter().take(8) {
                // The name, and where it is in a few words; all of it on hover.
                let mut where_ = place(&entry.path);
                if where_.chars().count() > 40 {
                    let tail: String = where_.chars().rev().take(38).collect::<Vec<_>>().into_iter().rev().collect();
                    where_ = format!("…{tail}");
                }
                let label = if entry.exists() {
                    RichText::new(format!("{}   {where_}", entry.name()))
                } else {
                    RichText::new(format!("{}   not found · {where_}", entry.name())).color(theme::colors().text_dim)
                };
                if ui.button(label).on_hover_text(entry.path.display().to_string()).clicked() {
                    click = Some(RecentClick::Open(entry.path.clone()));
                    ui.close();
                }
            }
            if !list.is_empty() {
                ui.separator();
            }
        }
        if ui.button("More...").clicked() {
            click = Some(RecentClick::More);
            ui.close();
        }
        if ui.add_enabled(!self.recent.is_empty(), egui::Button::new("Clear Recently Opened...")).clicked() {
            click = Some(RecentClick::Clear);
            ui.close();
        }
        click
    }

    /// Does what a click in a recent list asked.
    pub(super) fn recent_click(&mut self, click: RecentClick) {
        match click {
            RecentClick::Open(path) => self.open_recent(&path.display().to_string()),
            RecentClick::Remove(path) => self.remove_recent(&path),
            RecentClick::Clear => self.ask_clear_recent(),
            RecentClick::More => self.show_recent(),
        }
    }
}
