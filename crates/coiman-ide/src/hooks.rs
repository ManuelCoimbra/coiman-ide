//! Hooks for a port of Coiman to another platform (the Android build), so it can add to the
//! window without editing Coiman's source. Each is set once, before the app is created; only the
//! first call counts, and it says whether it did. The desktop sets none.
//!
//! The rest of what a port needs is in `coiman_core`: `platform` (running programs another way)
//! and `selection` (where the selected text is, for a platform text bar).

use std::path::PathBuf;
use std::sync::OnceLock;

/// Draws something of the port's own.
pub type UiHook = Box<dyn Fn(&mut egui::Ui) + Send + Sync>;

/// Called when something happens.
pub type Notify = Box<dyn Fn() + Send + Sync>;

static STATUS_BAR: OnceLock<UiHook> = OnceLock::new();
static TEXT_TAPPED: OnceLock<Notify> = OnceLock::new();

/// Adds a widget at the right end of the status bar (on Android, the soft keyboard's button).
/// It's drawn right to left, so it's the item furthest right.
pub fn set_status_bar_item(draw: UiHook) -> bool {
    STATUS_BAR.set(draw).is_ok()
}

/// Called when the editor or a terminal is clicked or tapped to type in (on Android, to bring
/// the soft keyboard back).
pub fn set_text_tapped(notify: Notify) -> bool {
    TEXT_TAPPED.set(notify).is_ok()
}

/// What a file dialog asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickKind {
    /// A file to open.
    #[default]
    File,
    /// A folder to open.
    Folder,
    /// Where to save a file.
    Save,
}

/// A file dialog's request, for a port's picker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickRequest {
    pub kind: PickKind,
    /// The dialog's title.
    pub title: Option<String>,
    /// The folder it opens in.
    pub directory: Option<PathBuf>,
    /// The name a save starts with.
    pub file_name: Option<String>,
    /// Kinds of file to show: a name and its extensions (`*` for all).
    pub filters: Vec<(String, Vec<String>)>,
}

/// Answers a file dialog: the path chosen, or `None` when cancelled. It waits for the answer.
pub type FilePicker = Box<dyn Fn(&PickRequest) -> Option<PathBuf> + Send + Sync>;

static FILE_PICKER: OnceLock<FilePicker> = OnceLock::new();

/// Shows the port's own file dialogs in place of the system's (on Android, the system's
/// document picker). Android has no other.
pub fn set_file_picker(picker: FilePicker) -> bool {
    FILE_PICKER.set(picker).is_ok()
}

/// The port's file picker, if it set one.
pub(crate) fn file_picker() -> Option<&'static FilePicker> {
    FILE_PICKER.get()
}

/// Draws the port's status bar item, if it set one.
pub(crate) fn status_bar_item(ui: &mut egui::Ui) {
    if let Some(draw) = STATUS_BAR.get() {
        draw(ui);
    }
}

/// Tells the port the text was clicked to type in.
pub(crate) fn text_tapped() {
    if let Some(notify) = TEXT_TAPPED.get() {
        notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn hooks_are_set_once_and_called() {
        static TAPS: AtomicUsize = AtomicUsize::new(0);
        static DRAWN: AtomicUsize = AtomicUsize::new(0);
        assert!(set_text_tapped(Box::new(|| {
            TAPS.fetch_add(1, Ordering::Relaxed);
        })));
        assert!(!set_text_tapped(Box::new(|| {})), "a second one is refused");
        text_tapped();
        // Other tests may tap the editor meanwhile: at least this one counted.
        assert!(TAPS.load(Ordering::Relaxed) >= 1);

        assert!(set_status_bar_item(Box::new(|_ui| {
            DRAWN.fetch_add(1, Ordering::Relaxed);
        })));
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), status_bar_item);
        out.textures_delta.clear();
        assert!(DRAWN.load(Ordering::Relaxed) >= 1);
    }
}
