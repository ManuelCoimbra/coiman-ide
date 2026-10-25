//! Rust Code's editor core: text buffer, documents, syntax highlighting,
//! settings, themes, sessions and workspace services. No UI dependencies, so
//! everything here can be tested and benchmarked headlessly.

pub mod buffer;
pub mod commands;
pub mod document;
pub mod edit_ops;
pub mod encoding;
pub mod finder;
pub mod highlight;
pub mod search;
pub mod session;
pub mod settings;
pub mod syntax;
pub mod theme;
pub mod watcher;
pub mod workspace;
