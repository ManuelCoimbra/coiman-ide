//! Coiman IDE's editor core: text buffer, documents, syntax highlighting,
//! settings, themes, sessions and workspace services. No UI dependencies, so
//! everything here can be tested and benchmarked headlessly.

pub mod analysis;
pub mod appearance;
pub mod applog;
pub mod build;
pub mod data;
pub mod editor;
pub mod extensions;
pub mod language;
pub mod launch;
pub mod project;
pub mod settings;
pub mod terminal;
