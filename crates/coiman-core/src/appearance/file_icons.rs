//! Which icon a file gets, from the chosen icon pack, and whether it holds tests, which any
//! language's icon can show.
//!
//! Packs: Material Icon Theme (the default), Catppuccin and vscode-icons come with Coiman (all
//! MIT; see `data/file_icons_LICENSE.txt` and `data/icon_packs/LICENSES.md`). Others come from
//! VS Code icon-theme extensions whose icons are SVG files, converted by [`pack_from_vscode`]
//! into `<config>/icon_packs/<id>.txt`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use serde_json::Value;

/// The packs that come with Coiman: (id, label, packed text).
const BUNDLED: &[(&str, &str, &str)] = &[
    ("material", "Material Icon Theme", include_str!("../../data/file_icons.txt")),
    ("catppuccin", "Catppuccin", include_str!("../../data/icon_packs/catppuccin.txt")),
    ("vscode-icons", "vscode-icons", include_str!("../../data/icon_packs/vscode-icons.txt")),
];
/// The pack used when none is chosen.
pub const DEFAULT_PACK: &str = "material";
const DEFAULT_ICON: &str = "file";

#[derive(Default)]
struct Icons {
    /// The icon for a file nothing names, in dark and light themes.
    default: &'static str,
    light_default: Option<&'static str>,
    names: HashMap<&'static str, &'static str>,
    extensions: HashMap<&'static str, &'static str>,
    light_names: HashMap<&'static str, &'static str>,
    light_extensions: HashMap<&'static str, &'static str>,
    svgs: HashMap<&'static str, &'static str>,
}

/// Reads a packed pack: one line each, tab-separated. `N`/`E` name or extension to icon,
/// `n`/`e` the same in light themes, `D`/`d` the default icon, `I` an icon's SVG, `L` the
/// pack's label.
fn parse(text: &'static str) -> Icons {
    let mut all = Icons { default: DEFAULT_ICON, ..Default::default() };
    for line in text.lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(tag), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let table = match tag {
            "N" => &mut all.names,
            "E" => &mut all.extensions,
            "n" => &mut all.light_names,
            "e" => &mut all.light_extensions,
            "I" => &mut all.svgs,
            "D" => {
                all.default = value;
                continue;
            }
            "d" => {
                all.light_default = Some(value);
                continue;
            }
            _ => continue,
        };
        table.insert(key, value);
    }
    all
}

fn loaded() -> &'static Mutex<HashMap<String, &'static Icons>> {
    static LOADED: OnceLock<Mutex<HashMap<String, &'static Icons>>> = OnceLock::new();
    LOADED.get_or_init(Default::default)
}

fn current_cell() -> &'static RwLock<(&'static str, &'static Icons)> {
    static CURRENT: OnceLock<RwLock<(&'static str, &'static Icons)>> = OnceLock::new();
    CURRENT.get_or_init(|| RwLock::new((DEFAULT_PACK, load(DEFAULT_PACK).expect("the default pack is built in"))))
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Where packs converted from extensions are kept.
pub fn user_dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("icon_packs"))
}

/// A pack by id: a bundled one, or one in [`user_dir`]. Kept once loaded.
fn load(id: &str) -> Option<&'static Icons> {
    let mut cache = loaded().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(icons) = cache.get(id) {
        return Some(icons);
    }
    let text: &'static str = match BUNDLED.iter().find(|b| b.0 == id) {
        Some(b) => b.2,
        None => {
            let path = user_dir()?.join(format!("{id}.txt"));
            Box::leak(std::fs::read_to_string(path).ok()?.into_boxed_str())
        }
    };
    let icons: &'static Icons = Box::leak(Box::new(parse(text)));
    cache.insert(id.to_owned(), icons);
    Some(icons)
}

fn icons() -> &'static Icons {
    current_cell().read().unwrap_or_else(|e| e.into_inner()).1
}

/// A packed pack's label, from its `L` line.
fn label_of(text: &str) -> Option<&str> {
    text.lines().find_map(|l| l.strip_prefix("L\tlabel\t"))
}

/// The packs to choose from: (id, label), the bundled ones first.
pub fn packs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = BUNDLED.iter().map(|b| (b.0.to_owned(), b.1.to_owned())).collect();
    if let Some(Ok(rd)) = user_dir().map(std::fs::read_dir) {
        let mut mine: Vec<(String, String)> = rd
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "txt"))
            .filter_map(|p| {
                let id = p.file_stem()?.to_string_lossy().into_owned();
                let text = std::fs::read_to_string(&p).ok()?;
                let label = label_of(&text).unwrap_or(&id).to_owned();
                Some((id, label))
            })
            .filter(|(id, _)| !BUNDLED.iter().any(|b| b.0 == id))
            .collect();
        mine.sort_by_key(|(_, label)| label.to_lowercase());
        out.extend(mine);
    }
    out
}

/// Uses a pack from now on (an empty id is the default). An unknown id leaves the current one.
pub fn set_pack(id: &str) -> Result<(), String> {
    let id = if id.is_empty() { DEFAULT_PACK } else { id };
    let icons = load(id).ok_or_else(|| format!("No icon pack `{id}`"))?;
    let mut current = current_cell().write().unwrap_or_else(|e| e.into_inner());
    if current.0 != id {
        *current = (Box::leak(id.to_owned().into_boxed_str()), icons);
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

/// The pack in use.
pub fn current_pack() -> &'static str {
    current_cell().read().unwrap_or_else(|e| e.into_inner()).0
}

/// Changes each time the pack does, so icons drawn before can be thrown away.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// A path's last part, for either kind of slash.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The icon for a file, by its whole name, then its longest known extension:
/// `a.test.ts` tries `test.ts` before `ts`.
pub fn icon_for(path: &str, light: bool) -> &'static str {
    icon_in(icons(), path, light)
}

fn icon_in(all: &'static Icons, path: &str, light: bool) -> &'static str {
    let lower = file_name(path).to_lowercase();
    let look = |plain: &HashMap<&'static str, &'static str>, lit: &HashMap<&'static str, &'static str>, key: &str| {
        lit.get(key).filter(|_| light).or_else(|| plain.get(key)).copied()
    };
    if let Some(icon) = look(&all.names, &all.light_names, &lower) {
        return icon;
    }
    for (at, _) in lower.match_indices('.') {
        if let Some(icon) = look(&all.extensions, &all.light_extensions, &lower[at + 1..]) {
            return icon;
        }
    }
    all.light_default.filter(|_| light).unwrap_or(all.default)
}

/// The SVG of an icon from [`icon_for`].
pub fn svg(icon: &str) -> Option<&'static str> {
    icons().svgs.get(icon).copied()
}

/// Every icon of the pack in use, by name.
pub fn all_svgs() -> impl Iterator<Item = (&'static str, &'static str)> {
    icons().svgs.iter().map(|(name, svg)| (*name, *svg))
}

/// VS Code's ids of common languages and their extensions (the language list beside them
/// only holds the languages Coiman adds servers for).
const VSCODE_LANGUAGES: &[(&str, &[&str])] = &[
    ("rust", &["rs"]),
    ("python", &["py", "pyw", "pyi"]),
    ("javascript", &["js", "mjs", "cjs"]),
    ("javascriptreact", &["jsx"]),
    ("typescript", &["ts", "mts", "cts"]),
    ("typescriptreact", &["tsx"]),
    ("java", &["java"]),
    ("c", &["c", "h"]),
    ("cpp", &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]),
    ("csharp", &["cs"]),
    ("go", &["go"]),
    ("ruby", &["rb"]),
    ("php", &["php"]),
    ("lua", &["lua"]),
    ("shellscript", &["sh", "bash", "zsh"]),
    ("powershell", &["ps1", "psm1", "psd1"]),
    ("bat", &["bat", "cmd"]),
    ("json", &["json"]),
    ("jsonc", &["jsonc"]),
    ("yaml", &["yml", "yaml"]),
    ("toml", &["toml"]),
    ("markdown", &["md", "markdown"]),
    ("html", &["html", "htm"]),
    ("css", &["css"]),
    ("scss", &["scss"]),
    ("less", &["less"]),
    ("xml", &["xml"]),
    ("sql", &["sql"]),
    ("kotlin", &["kt", "kts"]),
    ("swift", &["swift"]),
    ("dart", &["dart"]),
    ("scala", &["scala", "sc"]),
    ("haskell", &["hs", "lhs"]),
    ("elixir", &["ex", "exs"]),
    ("erlang", &["erl", "hrl"]),
    ("clojure", &["clj", "cljs", "cljc", "edn"]),
    ("r", &["r"]),
    ("julia", &["jl"]),
    ("perl", &["pl", "pm"]),
    ("fsharp", &["fs", "fsx", "fsi"]),
    ("vb", &["vb"]),
    ("zig", &["zig"]),
    ("nim", &["nim"]),
    ("ocaml", &["ml", "mli"]),
    ("groovy", &["groovy", "gradle"]),
    ("latex", &["tex"]),
    ("vue", &["vue"]),
    ("svelte", &["svelte"]),
    ("plaintext", &["txt"]),
];

/// The extensions of each language Coiman knows, by the id VS Code gives it, for icon themes
/// that name languages rather than extensions.
fn language_extensions() -> HashMap<&'static str, Vec<&'static str>> {
    let mut out: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
    for (id, extensions) in VSCODE_LANGUAGES {
        out.entry(id).or_default().extend(extensions.iter().copied());
    }
    let mut index = 0;
    while let Some(language) = crate::language::lsp::registry::get(index) {
        out.entry(language.language_id).or_default().extend(language.extensions.iter().copied());
        index += 1;
    }
    out
}

/// One side (dark or light) of a VS Code icon theme: its tables, read from the JSON.
struct Side<'a> {
    file: Option<&'a str>,
    names: Vec<(String, &'a str)>,
    extensions: Vec<(String, &'a str)>,
}

fn side<'a>(json: &'a Value, languages: &HashMap<&'static str, Vec<&'static str>>) -> Side<'a> {
    let table = |key: &str| -> Vec<(String, &'a str)> {
        json.get(key)
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?))).collect())
            .unwrap_or_default()
    };
    let mut extensions = table("fileExtensions");
    let known: HashSet<String> = extensions.iter().map(|e| e.0.clone()).collect();
    for (language, icon) in table("languageIds") {
        for ext in languages.get(language.as_str()).into_iter().flatten() {
            if !known.contains(*ext) {
                extensions.push(((*ext).to_owned(), icon));
            }
        }
    }
    Side { file: json.get("file").and_then(Value::as_str), names: table("fileNames"), extensions }
}

/// Collects the SVGs a packed pack uses, by name.
struct Svgs(BTreeMap<String, String>);

impl Svgs {
    /// The packed name of `icon` from the theme `json` (whose icon paths are relative to
    /// `dir`), reading its SVG the first time; `None` when it isn't an SVG file.
    fn add(&mut self, json: &Value, dir: &Path, prefix: &str, icon: &str) -> Option<String> {
        let name = format!("{prefix}{icon}");
        if self.0.contains_key(&name) {
            return Some(name);
        }
        let path = json.get("iconDefinitions")?.get(icon)?.get("iconPath")?.as_str()?;
        if !path.to_lowercase().ends_with(".svg") {
            return None;
        }
        let text = std::fs::read_to_string(dir.join(path)).ok()?;
        self.0.insert(name.clone(), text.split_whitespace().collect::<Vec<_>>().join(" "));
        Some(name)
    }

    /// Lines for one table: `tag`, then each key and the icon's packed name.
    fn rows(&mut self, tag: char, table: &[(String, &str)], json: &Value, dir: &Path, prefix: &str) -> Vec<String> {
        let mut rows: Vec<String> = table
            .iter()
            .filter(|(key, _)| !key.contains(['\t', '\n']))
            .filter_map(|(key, icon)| Some(format!("{tag}\t{key}\t{}", self.add(json, dir, prefix, icon)?)))
            .collect();
        rows.sort();
        rows
    }
}

/// Packs a VS Code icon theme (its JSON file, with icons as SVG files beside it) into
/// Coiman's format. `light_theme` is a second theme for light color themes, as Catppuccin's
/// Latte; otherwise the theme's own `light` section is used. Font-based themes (such as
/// Seti) can't be packed.
pub fn pack_from_vscode(theme: &Path, light_theme: Option<&Path>, label: &str) -> Result<String, String> {
    let read = |path: &Path| -> Result<Value, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&crate::appearance::theme::strip_jsonc(&text))
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let languages = language_extensions();
    let dark = read(theme)?;
    let dir = theme.parent().unwrap_or(Path::new("."));
    let main = side(&dark, &languages);
    let mut svgs = Svgs(BTreeMap::new());
    let mut lines = vec![format!("L\tlabel\t{}", label.replace(['\t', '\n'], " "))];
    lines.extend(svgs.rows('N', &main.names, &dark, dir, ""));
    lines.extend(svgs.rows('E', &main.extensions, &dark, dir, ""));
    if let Some(file) = main.file.and_then(|f| svgs.add(&dark, dir, "", f)) {
        lines.push(format!("D\tfile\t{file}"));
    }
    match light_theme {
        Some(path) => {
            let json = read(path)?;
            let light = side(&json, &languages);
            let dir = path.parent().unwrap_or(Path::new("."));
            lines.extend(svgs.rows('n', &light.names, &json, dir, "light-"));
            lines.extend(svgs.rows('e', &light.extensions, &json, dir, "light-"));
            if let Some(file) = light.file.and_then(|f| svgs.add(&json, dir, "light-", f)) {
                lines.push(format!("d\tfile\t{file}"));
            }
        }
        None => {
            if let Some(section) = dark.get("light") {
                let light = side(section, &languages);
                lines.extend(svgs.rows('n', &light.names, &dark, dir, ""));
                lines.extend(svgs.rows('e', &light.extensions, &dark, dir, ""));
                if let Some(file) = light.file.and_then(|f| svgs.add(&dark, dir, "", f)) {
                    lines.push(format!("d\tfile\t{file}"));
                }
            }
        }
    }
    if svgs.0.is_empty() {
        return Err(if dark.get("fonts").is_some() {
            "This icon theme draws its icons with a font, which Coiman can't use".to_owned()
        } else {
            "No SVG icons found in this icon theme".to_owned()
        });
    }
    lines.extend(svgs.0.into_iter().map(|(name, svg)| format!("I\t{name}\t{svg}")));
    Ok(lines.join("\n") + "\n")
}

/// Keeps a packed pack among the user's; returns its id.
pub fn install_pack(label: &str, text: &str) -> Result<String, String> {
    let dir = user_dir().ok_or("No config directory available")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = label
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let id = if id.is_empty() || BUNDLED.iter().any(|b| b.0 == id) { format!("{id}-installed") } else { id };
    std::fs::write(dir.join(format!("{id}.txt")), text).map_err(|e| e.to_string())?;
    // A pack loaded before under this id is read again.
    loaded().lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
    Ok(id)
}

/// File types that hold code, so a folder of tests says something about them.
const CODE: &[&str] = &[
    "rs", "py", "pyw", "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "java", "kt", "kts", "scala", "groovy",
    "go", "c", "h", "cc", "cpp", "cxx", "hpp", "cs", "fs", "vb", "rb", "php", "lua", "pl", "pm", "r", "jl", "ex",
    "exs", "erl", "hs", "ml", "clj", "cljs", "cljc", "lisp", "swift", "dart", "zig", "nim", "sh", "ps1", "lean",
    "agda", "d", "pas", "f90", "f95", "f03", "f", "for", "cob", "cbl", "m", "mm", "hpp", "rkt", "cl", "nims", "pyx",
    "bash", "zsh", "fsx",
];
/// Whose class names say so: `AppTest`, `AppTests`, `TestApp`.
const CLASS_NAMED: &[&str] = &["java", "kt", "kts", "scala", "groovy", "cs", "fs", "vb", "swift", "php", "dart"];

/// Whether a file extension, in any case, is one of code.
pub fn is_code(extension: &str) -> bool {
    CODE.contains(&extension.to_lowercase().as_str())
}

/// Whether a file holds tests, going by what test tools themselves look for. `path` is
/// best relative to the project: a folder above it called `test` says nothing.
pub fn is_test_file(path: &str) -> bool {
    let name = file_name(path);
    let Some((stem, extension)) = name.rsplit_once('.') else { return false };
    let extension = extension.to_lowercase();
    if !CODE.contains(&extension.as_str()) {
        return false;
    }
    let lower = stem.to_lowercase();
    let by_name = lower.starts_with("test_")
        || lower.ends_with("_test")
        || lower.ends_with("_tests")
        || lower.ends_with("_spec")
        || lower.ends_with(".test")
        || lower.ends_with(".spec")
        || lower.ends_with("-test")
        || matches!(lower.as_str(), "test" | "tests" | "conftest");
    let by_class = CLASS_NAMED.contains(&extension.as_str()) && {
        let after_test = stem.strip_prefix("Test").and_then(|rest| rest.chars().next());
        stem.ends_with("Test") || stem.ends_with("Tests") || after_test.is_some_and(char::is_uppercase)
    };
    let folders = path[..path.len() - name.len()].split(['/', '\\']);
    by_name || by_class || folders.into_iter().any(crate::project::project_tree::is_test_folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_here_has_its_icon() {
        for (file, icon) in [
            ("main.rs", "rust"),
            ("a.py", "python"),
            ("A.java", "java"),
            ("a.go", "go"),
            ("a.ts", "typescript"),
            ("a.js", "javascript"),
            ("a.c", "c"),
            ("a.cpp", "cpp"),
            ("a.cs", "csharp"),
            ("a.zig", "zig"),
            ("a.lean", "lean"),
            ("a.cbl", "cobol"),
            ("a.agda", "rc-agda"),
            ("a.apl", "rc-apl"),
            ("a.for", "fortran"),
            ("a.ex", "elixir"),
            ("Cargo.toml", "rust"),
            ("Cargo.lock", "lock"),
            ("pom.xml", "maven"),
            ("build.gradle", "gradle"),
            ("Dockerfile", "docker"),
            (".gitignore", "git"),
            ("src\\deep/a.RS", "rust"),
        ] {
            assert_eq!(icon_for(file, false), icon, "{file}");
            assert!(svg(icon_for(file, false)).is_some_and(|s| s.contains("<svg")), "{file}");
        }
    }

    #[test]
    fn the_longest_extension_wins_and_unknown_gets_the_plain_icon() {
        assert_eq!(icon_for("a.test.ts", false), "test-ts");
        assert_eq!(icon_for("no-such.thing", false), "file");
        assert_eq!(icon_for("noextension", false), "file");
        assert!(svg("file").is_some());
    }

    #[test]
    fn light_themes_get_their_own_where_there_is_one() {
        // Whichever the data says; only that a light one exists and is packed.
        let found = icons().light_extensions.iter().next().unwrap();
        assert_eq!(icon_for(&format!("a.{}", found.0), true), *found.1);
        assert!(svg(found.1).is_some());
    }

    #[test]
    fn every_icon_named_is_packed() {
        let all = icons();
        let named = [&all.names, &all.extensions, &all.light_names, &all.light_extensions];
        for table in named {
            for (key, icon) in table {
                assert!(all.svgs.contains_key(icon), "{key} -> {icon}");
            }
        }
    }

    #[test]
    fn the_other_packs_have_icons_for_the_usual_files() {
        for id in ["catppuccin", "vscode-icons"] {
            let pack = load(id).unwrap();
            for file in ["main.rs", "a.py", "A.java", "a.ts", "Cargo.toml", "package.json", "README.md", "x.unknown"] {
                for light in [false, true] {
                    let icon = icon_in(pack, file, light);
                    let svg = pack.svgs.get(icon).unwrap_or_else(|| panic!("{id}: {file} -> {icon} isn't packed"));
                    assert!(svg.contains("<svg"), "{id}: {icon}");
                }
            }
            assert_ne!(icon_in(pack, "main.rs", false), icon_in(pack, "x.unknown", false), "{id} knows Rust");
            for table in [&pack.names, &pack.extensions, &pack.light_names, &pack.light_extensions] {
                assert!(table.values().all(|icon| pack.svgs.contains_key(icon)), "{id}");
            }
        }
        // Catppuccin's light flavour is its own set.
        let cat = load("catppuccin").unwrap();
        assert!(icon_in(cat, "main.rs", true).starts_with("light-"));
        let ids: Vec<String> = packs().into_iter().map(|p| p.0).collect();
        assert_eq!(ids[..3], ["material", "catppuccin", "vscode-icons"]);
    }

    #[test]
    fn a_vscode_icon_theme_packs_from_its_files() {
        let dir = std::env::temp_dir().join(format!("coiman-icon-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("icons")).unwrap();
        let svg = |c: &str| {
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 4 4\">
  <rect width=\"4\" height=\"4\" fill=\"{c}\"/></svg>"
            )
        };
        std::fs::write(dir.join("icons/file.svg"), svg("#888")).unwrap();
        std::fs::write(dir.join("icons/rust.svg"), svg("#d84")).unwrap();
        std::fs::write(dir.join("icons/rust-light.svg"), svg("#a52")).unwrap();
        std::fs::write(dir.join("icons/lock.png"), "not an svg").unwrap();
        std::fs::write(
            dir.join("theme.json"),
            r#"{ // comments are fine
                "iconDefinitions": {
                    "_file": { "iconPath": "./icons/file.svg" },
                    "_rust": { "iconPath": "./icons/rust.svg" },
                    "_rust_light": { "iconPath": "./icons/rust-light.svg" },
                    "_lock": { "iconPath": "./icons/lock.png" }
                },
                "file": "_file",
                "languageIds": { "rust": "_rust" },
                "fileNames": { "Cargo.lock": "_lock" },
                "light": { "languageIds": { "rust": "_rust_light" } },
            }"#,
        )
        .unwrap();
        let text = pack_from_vscode(&dir.join("theme.json"), None, "Test icons").unwrap();
        assert!(text.starts_with(
            "L	label	Test icons
"
        ));
        assert!(
            text.contains(
                "E	rs	_rust
"
            ) && text.contains(
                "e	rs	_rust_light
"
            ),
            "{text}"
        );
        assert!(text.contains(
            "D	file	_file
"
        ));
        assert!(!text.contains("cargo.lock"), "a PNG icon is left out");
        assert!(
            text.contains("I	_rust	<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 4 4\"> <rect"),
            "one line each"
        );
        let pack: &'static Icons = Box::leak(Box::new(parse(Box::leak(text.into_boxed_str()))));
        assert_eq!((icon_in(pack, "lib.rs", false), icon_in(pack, "lib.rs", true)), ("_rust", "_rust_light"));
        assert_eq!(icon_in(pack, "notes.txt", true), "_file");

        std::fs::write(
            dir.join("font.json"),
            r#"{ "fonts": [{ "id": "seti" }], "iconDefinitions": { "_x": { "fontCharacter": "a" } }, "file": "_x" }"#,
        )
        .unwrap();
        assert!(pack_from_vscode(&dir.join("font.json"), None, "Font").unwrap_err().contains("font"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn knows_tests_from_other_files() {
        for yes in [
            "foo_test.go",
            "test_foo.py",
            "foo_test.py",
            "conftest.py",
            "foo.test.ts",
            "foo.spec.js",
            "foo_spec.rb",
            "AppTest.java",
            "AppTests.cs",
            "TestApp.java",
            "tests/thing.rs",
            "src\\test\\java\\demo\\Helper.java",
            "__tests__/a.jsx",
        ] {
            assert!(is_test_file(yes), "{yes}");
        }
        for no in [
            "main.rs",
            "latest.py",
            "contest.py",
            "App.java",
            "Testament.java",
            "attest.go",
            "tests/data.json",
            "test.txt",
            "Makefile",
            "testing",
        ] {
            assert!(!is_test_file(no), "{no}");
        }
    }
}
