//! Language Server Protocol client: talks to rust-analyzer (and later other
//! servers) over stdio. The editor-facing logic lives in the app; this module
//! is the protocol, process handling and position conversions.

mod client;
pub mod convert;
pub mod discovery;
pub mod transport;

pub use client::{Client, Event, language_id};
pub use lsp_types as types;
