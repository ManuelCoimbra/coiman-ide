//! The file dialog: the system's own (through `rfd`), or a port's picker when it set one (see
//! `crate::hooks::set_file_picker`). Android has no `rfd`, so there it's the port's or none.

use crate::hooks::{PickKind, PickRequest};
use std::path::{Path, PathBuf};

/// A file dialog to set up, then show with `pick_file`, `pick_folder` or `save_file`. Each
/// waits for the user's answer.
#[derive(Clone, Debug, Default)]
pub struct FileDialog {
    request: PickRequest,
}

impl FileDialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// The folder the dialog opens in.
    pub fn set_directory(mut self, dir: impl AsRef<Path>) -> Self {
        self.request.directory = Some(dir.as_ref().to_path_buf());
        self
    }

    /// The name a save starts with.
    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.request.file_name = Some(name.into());
        self
    }

    /// The dialog's title.
    pub fn set_title(mut self, title: impl Into<String>) -> Self {
        self.request.title = Some(title.into());
        self
    }

    /// A kind of file to show: a name and its extensions (`*` for all).
    pub fn add_filter(mut self, name: impl Into<String>, extensions: &[impl ToString]) -> Self {
        self.request.filters.push((name.into(), extensions.iter().map(ToString::to_string).collect()));
        self
    }

    /// A file to open.
    pub fn pick_file(self) -> Option<PathBuf> {
        self.show(PickKind::File)
    }

    /// A folder to open.
    pub fn pick_folder(self) -> Option<PathBuf> {
        self.show(PickKind::Folder)
    }

    /// Where to save.
    pub fn save_file(self) -> Option<PathBuf> {
        self.show(PickKind::Save)
    }

    fn show(mut self, kind: PickKind) -> Option<PathBuf> {
        self.request.kind = kind;
        if let Some(picker) = crate::hooks::file_picker() {
            return picker(&self.request);
        }
        system(&self.request)
    }
}

/// The system's dialog.
#[cfg(not(target_os = "android"))]
fn system(request: &PickRequest) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(dir) = &request.directory {
        dialog = dialog.set_directory(dir);
    }
    if let Some(name) = &request.file_name {
        dialog = dialog.set_file_name(name);
    }
    if let Some(title) = &request.title {
        dialog = dialog.set_title(title);
    }
    for (name, extensions) in &request.filters {
        dialog = dialog.add_filter(name, extensions);
    }
    match request.kind {
        PickKind::File => dialog.pick_file(),
        PickKind::Folder => dialog.pick_folder(),
        PickKind::Save => dialog.save_file(),
    }
}

/// Android has no dialog of its own here: only a port's picker.
#[cfg(target_os = "android")]
fn system(_request: &PickRequest) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_request() {
        let d = FileDialog::new()
            .set_directory("/tmp")
            .set_file_name("a.txt")
            .set_title("Save")
            .add_filter("Text", &["txt", "md"]);
        assert_eq!(d.request.directory, Some(PathBuf::from("/tmp")));
        assert_eq!(d.request.file_name.as_deref(), Some("a.txt"));
        assert_eq!(d.request.title.as_deref(), Some("Save"));
        assert_eq!(d.request.filters, [("Text".to_owned(), vec!["txt".to_owned(), "md".to_owned()])]);
    }

    /// A port's picker answers every dialog, and sees what it asked for.
    #[test]
    fn port_picker_answers() {
        assert!(crate::hooks::set_file_picker(Box::new(|request: &PickRequest| {
            let name = request.file_name.clone().unwrap_or_default();
            Some(PathBuf::from(format!("/picked/{:?}/{name}", request.kind)))
        })));
        assert_eq!(FileDialog::new().pick_file(), Some(PathBuf::from("/picked/File/")));
        assert_eq!(FileDialog::new().pick_folder(), Some(PathBuf::from("/picked/Folder/")));
        assert_eq!(FileDialog::new().set_file_name("a.txt").save_file(), Some(PathBuf::from("/picked/Save/a.txt")));
    }
}
