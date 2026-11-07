//! Language Server Protocol client: talks to language servers (rust-analyzer,
//! pyright, clangd…) over stdio. The editor-facing logic lives in the app; this module
//! is the protocol, process handling and position conversions.

mod client;
pub mod convert;
pub mod discovery;
pub mod registry;
pub mod servers;
pub mod status;
pub mod transport;

pub use client::{Client, Event, language_id};
pub use lsp_types as types;
pub mod companions;
