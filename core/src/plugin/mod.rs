//! Plugins: sandboxed WebAssembly components with declared permissions and
//! contribution points, installed from signed registries and pinned by a
//! lockfile. See `wit/plugin.wit` for the API plugins are written against.

pub mod host;
pub mod manifest;
pub mod registry;
pub mod store;

pub use host::{Block, Button, Document, Event, PluginHost, TreeItem, Update};
pub use manifest::{EventKind, Manifest, Permission};
