//! Hooks for a port of Coiman to another platform (the Android build), so it can add to the
//! window without editing Coiman's source. Each is set once, before the app is created; only the
//! first call counts, and it says whether it did. The desktop sets none.
//!
//! The rest of what a port needs is in `coiman_core`: `platform` (running programs another way)
//! and `selection` (where the selected text is, for a platform text bar).

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
