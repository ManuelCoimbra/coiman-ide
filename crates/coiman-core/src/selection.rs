//! Where the selected text is on screen, for a platform text bar (Cut/Copy/Paste on a phone or a
//! tablet).
//!
//! Coiman's editor and terminal draw their own text, so neither is an egui `TextEdit` and a
//! platform's text bar can't find their selection through egui's own state. The one with the
//! keyboard publishes the selection's rectangle here every frame, and the app clears it at the
//! start of the frame, so a stale one never stays.
//!
//! The rectangle is `[min.x, min.y, max.x, max.y]` in egui points, or `None` when nothing is
//! selected.

use std::sync::RwLock;

static RECT: RwLock<Option<[f32; 4]>> = RwLock::new(None);

/// Publishes where the selected text is, or `None` when there is no selection.
pub fn set(rect: Option<[f32; 4]>) {
    if let Ok(mut slot) = RECT.write() {
        *slot = rect;
    }
}

/// Where the selected text is, if there is one.
pub fn get() -> Option<[f32; 4]> {
    RECT.read().ok().and_then(|slot| *slot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get() {
        set(Some([1.0, 2.0, 3.0, 4.0]));
        assert_eq!(get(), Some([1.0, 2.0, 3.0, 4.0]));
        set(None);
        assert_eq!(get(), None);
    }
}
