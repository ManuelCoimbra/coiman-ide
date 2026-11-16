//! Writing plugins: a new plugin's folder from a template, and dev mode's build, which
//! compiles a plugin folder for `wasm32-unknown-unknown` (Cargo does it) and turns the
//! module into the component Coiman loads (here, with `wit-component`).

use std::path::{Path, PathBuf};

/// The plugin interface, copied into each new plugin so its folder builds on its own.
pub const WIT: &str = include_str!("../../../wit/plugin.wit");

/// What a new plugin starts with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    /// A command and a status bar item (the active file's lines).
    Command,
    /// A panel with text, a list and a button.
    Panel,
    /// Problems on save: lines longer than a limit, in the Problems panel.
    Problems,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::Command, Template::Panel, Template::Problems];

    pub fn title(self) -> &'static str {
        match self {
            Template::Command => "Command and status bar item",
            Template::Panel => "Panel",
            Template::Problems => "Problems on save",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Template::Command => "A command that shows the active file's line count, and a status bar item with it",
            Template::Panel => "A panel with a heading, text, a list of the open folder's files and a Refresh button",
            Template::Problems => {
                "Checks each file when it's saved and lists lines longer than 100 characters as problems"
            }
        }
    }
}

/// A plugin id's last part from a name: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// Whether `dir` holds a plugin to build: its `plugin.toml` and `Cargo.toml`.
pub fn is_plugin_folder(dir: &Path) -> bool {
    dir.join("plugin.toml").is_file() && dir.join("Cargo.toml").is_file()
}

/// Writes a new plugin named `name` into `folder` (which must be empty or not there yet).
/// Returns the file to open first (`src/lib.rs`).
pub fn create(folder: &Path, name: &str, template: Template) -> Result<PathBuf, String> {
    let slug = slug(name);
    if slug.is_empty() {
        return Err("Give the plugin a name with letters or digits".to_owned());
    }
    if folder.exists() && std::fs::read_dir(folder).map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} isn't empty", folder.display()));
    }
    let name = name.trim();
    let files: [(&str, String); 6] = [
        ("Cargo.toml", cargo_toml(&slug)),
        ("plugin.toml", plugin_toml(&slug, name, template)),
        ("src/lib.rs", lib_rs(name, template)),
        ("wit/plugin.wit", WIT.to_owned()),
        (".gitignore", "/target\n".to_owned()),
        ("README.md", readme(name, template)),
    ];
    for (rel, text) in files {
        let path = folder.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(folder.join("src").join("lib.rs"))
}

fn cargo_toml(slug: &str) -> String {
    format!(
        r#"[package]
name = "{slug}"
version = "0.1.0"
edition = "2024"
publish = false

# A Coiman IDE plugin: built for wasm32-unknown-unknown and loaded as a component.
# In Coiman, "Plugins: Start Dev Mode" builds it and loads it again on every save.

[lib]
crate-type = ["cdylib"]

[dependencies]
wit-bindgen = "0.63"

[profile.release]
opt-level = "s"
strip = true

# Built on its own, outside any workspace around it.
[workspace]
"#
    )
}

fn plugin_toml(slug: &str, name: &str, template: Template) -> String {
    let head = format!("id = \"dev.{slug}\"\nname = \"{name}\"\nversion = \"0.1.0\"\n");
    let rest = match template {
        Template::Command => format!(
            r#"description = "Shows the active file's line count."
permissions = ["editor-read"]
events = ["active-changed", "edited"]

[[commands]]
id = "count"
title = "{name}: Count Lines"

[[status_items]]
id = "lines"
alignment = "right"
command = "count"
"#
        ),
        Template::Panel => format!(
            r#"description = "A panel about the open folder."
permissions = ["workspace-read"]
events = ["saved"]

[[commands]]
id = "refresh"
title = "{name}: Refresh"

[[panels]]
id = "main"
title = "{name}"
"#
        ),
        Template::Problems => r#"description = "Lists lines longer than 100 characters as problems, on save."
permissions = ["workspace-read"]
events = ["saved", "opened"]
"#
        .to_owned(),
    };
    head + &rest
}

fn lib_rs(name: &str, template: Template) -> String {
    let head = format!(
        r#"//! {name}: a Coiman IDE plugin.

wit_bindgen::generate!({{
    path: "wit/plugin.wit",
    world: "plugin",
}});

"#
    );
    let body = match template {
        Template::Command => r#"use coiman::plugin::host;

struct Plugin;

/// The active file's line count, in the status bar.
fn update() {
    match host::active_document() {
        Ok(Some(doc)) => {
            let n = doc.text.lines().count();
            host::set_status_item("lines", &format!("{n} lines"), "Click to count again");
        }
        Ok(None) => host::set_status_item("lines", "", ""),
        Err(e) => host::log(&e),
    }
}

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        update();
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        match id.as_str() {
            "count" => {
                let doc = host::active_document()?.ok_or("Open a file first")?;
                host::show_message(&format!("{}: {} lines", doc.path, doc.text.lines().count()));
                Ok(())
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    fn on_event(_event: Event) {
        update();
    }
}

export!(Plugin);
"#
        .to_owned(),
        Template::Panel => r#"use coiman::plugin::host;
use coiman::plugin::types::{Block, Button};

struct Plugin;

/// Fills the panel: a heading, some text, the folder's first files and a button.
fn refresh() -> Result<(), String> {
    let files = host::list_files("", 20)?;
    let blocks = vec![
        Block::Heading("The open folder".to_owned()),
        Block::Text(format!("{} files (the first 20):", files.len())),
        Block::Bullets(files),
        Block::Separator,
        Block::Button(Button { label: "Refresh".to_owned(), command: "refresh".to_owned(), argument: String::new() }),
    ];
    host::set_panel("main", &blocks);
    Ok(())
}

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        refresh()
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        match id.as_str() {
            "refresh" => refresh(),
            other => Err(format!("unknown command {other}")),
        }
    }

    fn on_event(event: Event) {
        if matches!(event, Event::Saved(_)) {
            let _ = refresh();
        }
    }
}

export!(Plugin);
"#
        .to_owned(),
        Template::Problems => r#"use coiman::plugin::host;
use coiman::plugin::types::{Problem, Severity};

struct Plugin;

const LIMIT: usize = 100;

/// Lines of `path` longer than LIMIT characters, as problems.
fn check(path: &str) -> Result<(), String> {
    let text = host::read_file(path)?;
    let problems: Vec<Problem> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| line.chars().count() > LIMIT)
        .map(|(i, line)| Problem {
            line: i as u32 + 1,
            start: LIMIT as u32,
            end: line.chars().count() as u32,
            severity: Severity::Warning,
            message: format!("Line longer than {LIMIT} characters"),
        })
        .collect();
    host::set_problems(path, &problems);
    Ok(())
}

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, _argument: String) -> Result<(), String> {
        Err(format!("unknown command {id}"))
    }

    fn on_event(event: Event) {
        if let Event::Saved(path) | Event::Opened(path) = event
            && !path.is_empty()
            && let Err(e) = check(&path)
        {
            host::log(&format!("{path}: {e}"));
        }
    }
}

export!(Plugin);
"#
        .to_owned(),
    };
    head + &body
}

fn readme(name: &str, template: Template) -> String {
    format!(
        "# {name}\n\nA Coiman IDE plugin ({}).\n\n- `plugin.toml`: what it is, the permissions it asks for, the events it hears, and its \
         commands, status bar items, panels and tree views.\n- `src/lib.rs`: the code, against the interface in \
         `wit/plugin.wit`.\n\nIn Coiman, open this folder and run **Plugins: Start Dev Mode**: it builds the plugin \
         (`cargo build --release --target wasm32-unknown-unknown`, which needs `rustup target add \
         wasm32-unknown-unknown`), loads it, and builds and loads it again whenever a file here is saved. Its log is \
         in the Output panel.\n",
        template.title().to_lowercase()
    )
}

/// Cargo's arguments for a plugin's build.
pub fn build_args() -> Vec<String> {
    ["build", "--release", "--target", "wasm32-unknown-unknown", "--message-format=short"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
}

/// Where Cargo puts the plugin's module, from its package name.
pub fn module_path(folder: &Path) -> Result<PathBuf, String> {
    let text = std::fs::read_to_string(folder.join("Cargo.toml")).map_err(|e| format!("Cargo.toml: {e}"))?;
    let doc: toml::Table = toml::from_str(&text).map_err(|e| format!("Cargo.toml: {}", e.message()))?;
    let name = doc
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .ok_or("Cargo.toml has no package name")?;
    let file = format!("{}.wasm", name.replace('-', "_"));
    Ok(folder.join("target").join("wasm32-unknown-unknown").join("release").join(file))
}

/// Turns a built module into the component Coiman loads.
pub fn componentize(module: &[u8]) -> Result<Vec<u8>, String> {
    wit_component::ComponentEncoder::default()
        .module(module)
        .and_then(|e| e.validate(true).encode())
        .map_err(|e| format!("{e:#}"))
}

/// Builds `folder`'s module into its `plugin.wasm` (after Cargo has built it).
pub fn write_component(folder: &Path) -> Result<usize, String> {
    let module_file = module_path(folder)?;
    let module = std::fs::read(&module_file).map_err(|e| format!("{}: {e}", module_file.display()))?;
    let component = componentize(&module)?;
    std::fs::write(folder.join("plugin.wasm"), &component).map_err(|e| format!("plugin.wasm: {e}"))?;
    Ok(component.len())
}

/// Whether a saved file calls for a new build: the plugin's own sources, not its builds.
pub fn needs_build(folder: &Path, saved: &Path) -> bool {
    let Ok(rel) = saved.strip_prefix(folder) else { return false };
    if rel.components().next().is_some_and(|c| c.as_os_str() == "target") {
        return false;
    }
    let ext = saved.extension().and_then(|e| e.to_str()).unwrap_or_default();
    matches!(ext, "rs" | "toml" | "wit")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_ids() {
        assert_eq!(slug("Line Counter!"), "line-counter");
        assert_eq!(slug("  My  2nd plugin "), "my-2nd-plugin");
        assert_eq!(slug("¿?"), "");
    }

    #[test]
    fn every_template_makes_a_plugin_folder() {
        for t in Template::ALL {
            let dir = std::env::temp_dir().join(format!("coiman-new-plugin-{}-{:?}", std::process::id(), t));
            let _ = std::fs::remove_dir_all(&dir);
            let first = create(&dir, "Line Counter", t).unwrap();
            assert!(first.ends_with("src/lib.rs") && first.is_file());
            assert!(is_plugin_folder(&dir));
            let manifest =
                crate::extensions::plugin::manifest::parse(&std::fs::read_to_string(dir.join("plugin.toml")).unwrap())
                    .unwrap();
            assert_eq!(manifest.id, "dev.line-counter");
            assert_eq!(std::fs::read_to_string(dir.join("wit/plugin.wit")).unwrap(), WIT);
            assert!(module_path(&dir).unwrap().ends_with("wasm32-unknown-unknown/release/line_counter.wasm"));
            assert!(create(&dir, "Again", t).unwrap_err().contains("isn't empty"));
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn builds_follow_sources() {
        let f = Path::new("/p");
        assert!(needs_build(f, Path::new("/p/src/lib.rs")));
        assert!(needs_build(f, Path::new("/p/plugin.toml")));
        assert!(!needs_build(f, Path::new("/p/target/x.rs")));
        assert!(!needs_build(f, Path::new("/p/README.md")));
        assert!(!needs_build(f, Path::new("/elsewhere/lib.rs")));
        assert!(componentize(b"not wasm").is_err());
    }
}
