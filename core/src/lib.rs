//! Rust Code's editor core: text buffer, documents, syntax highlighting,
//! settings, themes, sessions and workspace services. No UI dependencies, so
//! everything here can be tested and benchmarked headlessly.

pub mod bookmarks;
pub mod buffer;
pub mod cargo;
pub mod commands;
pub mod dap;
pub mod dbgout;
pub mod document;
pub mod edit_ops;
pub mod editorconfig;
pub mod encoding;
pub mod find;
pub mod finder;
pub mod fonts;
pub mod git;
pub mod highlight;
pub mod history;
pub mod languages;
pub mod lsp;
pub mod markdown;
pub mod plugin;
pub mod pty;
pub mod recent;
pub mod run;
pub mod search;
pub mod session;
pub mod settings;
pub mod snippets;
pub mod structure;
pub mod syntax;
pub mod termfind;
pub mod theme;
pub mod trust;
pub mod unicode;
pub mod vim;
pub mod watcher;
pub mod workspace;
