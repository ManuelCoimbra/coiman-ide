//! Shared workspace conventions.

/// Folders that are never shown, indexed, searched or watched.
pub const IGNORED_DIRS: &[&str] = &[".git", "target", "node_modules", ".idea", ".vs"];
