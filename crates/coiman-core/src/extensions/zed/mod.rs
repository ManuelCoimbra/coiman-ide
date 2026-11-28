//! Zed extensions support: registry client, manifest/asset loaders (themes, icon themes,
//! snippets, languages/grammars), and Wasmtime component host (`extension.wit`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub mod icons;
pub mod languages;
pub mod manifest;
pub mod registry;
pub mod snippets;
pub mod themes;
pub mod wasm_host;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Updates whether Zed extensions are active globally (driven by `settings.plugins.zed_extensions`).
pub fn set_enabled_global(enabled: bool) {
    let prev = ENABLED.swap(enabled, Ordering::Relaxed);
    if prev != enabled {
        sync_assets();
    }
}

/// Whether Zed extensions are currently enabled globally.
pub fn is_enabled_global() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Root directory for Zed extensions under Coiman's config directory.
pub fn root_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("zed_extensions"))
}

/// Directory containing installed Zed extensions (`<root>/installed/<id>`).
pub fn installed_dir() -> Option<PathBuf> {
    root_dir().map(|d| d.join("installed"))
}

/// Sandbox work directory for a given Zed extension (`<root>/work/<id>`).
pub fn work_dir(id: &str) -> Option<PathBuf> {
    root_dir().map(|d| d.join("work").join(id))
}

/// Persisted enable/disable & origin metadata file (`<root>/state.json`).
pub fn state_path() -> Option<PathBuf> {
    root_dir().map(|d| d.join("state.json"))
}

/// Synchronizes declarative assets (icon packs and languages/grammars) from active Zed extensions.
pub fn sync_assets() {
    let enabled = is_enabled_global();
    let active = if enabled { registry::list_active() } else { Vec::new() };
    icons::sync_all_icon_packs(enabled, &active);
    languages::sync_all_languages(enabled, &active);
}
