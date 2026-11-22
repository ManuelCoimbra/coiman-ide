//! Plugins: sandboxed WebAssembly components with declared permissions and
//! contribution points, installed from signed registries and pinned by a
//! lockfile. See `wit/plugin.wit` for the API plugins are written against.

pub mod dev;
pub mod host;
pub mod manifest;
pub mod registry;
pub mod store;

pub use host::{
    Block, Button, Decoration, DirEntry, Document, Event, Pick, PickAction, PickChoice, PickItem, PickQuery,
    PluginHost, Problem, Severity, TerminalLine, TreeItem, Update,
};
pub use manifest::{EventKind, Manifest, Permission};
