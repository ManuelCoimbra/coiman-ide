//! Plugins: sandboxed WebAssembly components with declared permissions and
//! contribution points, installed from signed registries and pinned by a
//! lockfile. See `wit/plugin.wit` for the API plugins are written against.

pub mod dev;
pub mod host;
pub mod manifest;
pub mod registry;
pub mod store;
pub mod uml;

pub use host::{
    Block, Button, Decoration, DirEntry, Document, EdgeKind, Event, FileLink, Graph, GraphEdge, GraphNode, Pick,
    PickAction, PickChoice, PickItem, PickQuery, PluginHost, Problem, Row, Severity, Span, TerminalLine, TreeItem,
    Update,
};
pub use manifest::{EventKind, Manifest, Permission};
