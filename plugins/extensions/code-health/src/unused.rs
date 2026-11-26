//! Dependencies a manifest names that the code never uses: a crate in `Cargo.toml` no Rust file
//! under it mentions (`use name`, `name::`, `extern crate name`), and a package in
//! `package.json` nothing imports or requires and no script or config file names. Read from
//! the text, so a dependency used another way (a macro's own crate, a plugin a tool loads by
//! name) can show here: each says what was looked for.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unused {
    /// The manifest, relative to the project.
    pub manifest: String,
    pub name: String,
    /// "dependencies", "dev-dependencies", "devDependencies"…
    pub section: String,
    /// What was looked for, and where.
    pub why: String,
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// Whether `path` is under `dir` ("" is the project's top).
fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path.starts_with(&format!("{dir}/"))
}

fn rust_roots() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(?:use\s+|extern\s+crate\s+)(?:::)?([A-Za-z_][A-Za-z0-9_]*)|\b([A-Za-z_][A-Za-z0-9_]*)\s*::")
            .expect("compiles")
    })
}

fn js_imports() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*|\bimport\s+)["']([^"'./][^"']*)["']|@import\s+["']~?([^"'./][^"']*)["']"#,
        )
        .expect("compiles")
    })
}

/// The package an import names: `lodash/fp` is `lodash`, `@scope/pkg/x` is `@scope/pkg`.
fn package_of(spec: &str) -> String {
    let mut parts = spec.split('/');
    let first = parts.next().unwrap_or("");
    if first.starts_with('@') { format!("{first}/{}", parts.next().unwrap_or("")) } else { first.to_owned() }
}

/// The names a Rust file uses as the first part of a path.
pub fn rust_names(text: &str, into: &mut BTreeSet<String>) {
    for c in rust_roots().captures_iter(text) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)) {
            into.insert(m.as_str().to_owned());
        }
    }
}

/// The packages a JavaScript, TypeScript, Vue, Svelte or CSS file imports.
pub fn js_packages(text: &str, into: &mut BTreeSet<String>) {
    for c in js_imports().captures_iter(text) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)) {
            let spec = m.as_str();
            // `node:fs` and the like are Node's own.
            if !spec.starts_with("node:") {
                into.insert(package_of(spec));
            }
        }
    }
}

/// The crates `Cargo.toml` (at `path`) names, by section, with the name the code uses for each
/// (a renamed one's key; `-` as `_`).
fn cargo_deps(path: &str, text: &str) -> Vec<(String, String, String)> {
    let Ok(value) = text.parse::<toml::Table>() else { return Vec::new() };
    let mut out = Vec::new();
    let mut sections: Vec<(String, &toml::Table)> = Vec::new();
    for s in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(t) = value.get(s).and_then(toml::Value::as_table) {
            sections.push((s.to_owned(), t));
        }
    }
    // [target.'cfg(…)'.dependencies]
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        for t in targets.values().filter_map(toml::Value::as_table) {
            for s in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(d) = t.get(s).and_then(toml::Value::as_table) {
                    sections.push((s.to_owned(), d));
                }
            }
        }
    }
    let _ = path;
    for (section, table) in sections {
        for key in table.keys() {
            out.push((section.clone(), key.clone(), key.replace('-', "_")));
        }
    }
    out
}

/// What the manifests name that the files never use. `files` holds each file's text, by its
/// path relative to the project (manifests, code, configuration).
pub fn find(files: &BTreeMap<String, String>) -> Vec<Unused> {
    let mut out = Vec::new();
    for (path, text) in files {
        let name = path.rsplit('/').next().unwrap_or(path);
        let dir = dir_of(path);
        if name == "Cargo.toml" {
            let deps = cargo_deps(path, text);
            if deps.is_empty() {
                continue;
            }
            // Features may switch a dependency on: those count as used.
            let features = text
                .parse::<toml::Table>()
                .ok()
                .and_then(|t| t.get("features").cloned())
                .map(|f| f.to_string())
                .unwrap_or_default();
            let mut used = BTreeSet::new();
            let mut build = BTreeSet::new();
            for (p, t) in files.iter().filter(|(p, _)| p.ends_with(".rs") && under(p, dir)) {
                if p.rsplit('/').next() == Some("build.rs") {
                    rust_names(t, &mut build);
                } else {
                    rust_names(t, &mut used);
                }
            }
            for (section, key, ident) in deps {
                let seen = if section == "build-dependencies" { &build } else { &used };
                let in_features = features.contains(&format!("dep:{key}")) || features.contains(&format!("\"{key}/"));
                if !seen.contains(&ident) && !in_features {
                    out.push(Unused {
                        manifest: path.clone(),
                        name: key.clone(),
                        why: format!(
                            "no `use {ident}`, `{ident}::` or `extern crate {ident}` in {}",
                            if section == "build-dependencies" {
                                "build.rs".to_owned()
                            } else {
                                format!("the .rs files under {}", if dir.is_empty() { "the project" } else { dir })
                            }
                        ),
                        section,
                    });
                }
            }
        } else if name == "package.json" && !path.contains("node_modules/") {
            let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { continue };
            let scripts = json.get("scripts").map(|s| s.to_string()).unwrap_or_default();
            let mut used = BTreeSet::new();
            let mut configs = String::new();
            for (p, t) in files.iter().filter(|(p, _)| under(p, dir) && !p.contains("node_modules/")) {
                let ext = p.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
                if matches!(
                    ext,
                    "js" | "jsx"
                        | "mjs"
                        | "cjs"
                        | "ts"
                        | "tsx"
                        | "mts"
                        | "cts"
                        | "vue"
                        | "svelte"
                        | "astro"
                        | "css"
                        | "scss"
                        | "less"
                ) {
                    js_packages(t, &mut used);
                }
                let file = p.rsplit('/').next().unwrap_or("");
                // Tools name their plugins in their own settings.
                if file.starts_with('.')
                    || file.contains(".config.")
                    || file.ends_with("rc.json")
                    || file == "tsconfig.json"
                    || file == "babel.config.json"
                {
                    configs.push_str(t);
                }
            }
            for section in ["dependencies", "devDependencies", "optionalDependencies"] {
                let Some(deps) = json.get(section).and_then(serde_json::Value::as_object) else { continue };
                for dep in deps.keys() {
                    // Types come with what they describe; a tool is used by its command.
                    let typed = dep.strip_prefix("@types/").is_some_and(|t| {
                        used.iter().any(|u| u == t || u.ends_with(&format!("/{t}")))
                            || used.contains(t)
                            || !used.is_empty()
                    });
                    let short = dep.rsplit('/').next().unwrap_or(dep);
                    let named =
                        scripts.contains(dep.as_str()) || scripts.contains(short) || configs.contains(dep.as_str());
                    if !used.contains(dep) && !typed && !named {
                        out.push(Unused {
                            manifest: path.clone(),
                            name: dep.clone(),
                            section: section.to_owned(),
                            why: format!(
                                "nothing under {} imports or requires it, and no script or config file names it",
                                if dir.is_empty() { "the project" } else { dir }
                            ),
                        });
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> BTreeMap<String, String> {
        list.iter().map(|(p, t)| ((*p).to_owned(), (*t).to_owned())).collect()
    }

    #[test]
    fn rust_crates() {
        let f = files(&[
            (
                "Cargo.toml",
                "[package]\nname = \"app\"\n[dependencies]\nserde_json = \"1\"\nregex = \"1\"\nonce-cell = \"1\"\nlog = \"0.4\"\nrayon = { version = \"1\", optional = true }\n\
                 [features]\nfast = [\"dep:rayon\"]\n[dev-dependencies]\ntempfile = \"3\"\n[build-dependencies]\ncc = \"1\"\n",
            ),
            (
                "src/main.rs",
                "use regex::Regex;\nfn main() { let v = serde_json::json!({}); once_cell::sync::Lazy::new(|| 1); }\n// log is mentioned only in a comment: log\n",
            ),
            ("build.rs", "fn main() {}\n"),
        ]);
        let got: Vec<(String, String)> = find(&f).into_iter().map(|u| (u.section, u.name)).collect();
        assert_eq!(
            got,
            [
                ("dependencies".to_owned(), "log".to_owned()),
                ("dev-dependencies".to_owned(), "tempfile".to_owned()),
                ("build-dependencies".to_owned(), "cc".to_owned()),
            ]
        );
    }

    #[test]
    fn workspace_members_look_at_their_own_code() {
        let f = files(&[
            ("crates/a/Cargo.toml", "[dependencies]\nregex = \"1\"\n"),
            ("crates/b/Cargo.toml", "[dependencies]\nregex = \"1\"\n"),
            ("crates/a/src/lib.rs", "pub fn f() { regex::Regex::new(\"x\"); }\n"),
            ("crates/b/src/lib.rs", "pub fn g() {}\n"),
        ]);
        let got: Vec<String> = find(&f).into_iter().map(|u| u.manifest).collect();
        assert_eq!(got, ["crates/b/Cargo.toml"]);
    }

    #[test]
    fn npm_packages() {
        let f = files(&[
            (
                "package.json",
                r#"{"scripts":{"lint":"eslint ."},"dependencies":{"react":"18","lodash":"4","@scope/ui":"1","left-pad":"1"},
                   "devDependencies":{"eslint":"9","eslint-plugin-react":"7","@types/react":"18","vitest":"1"}}"#,
            ),
            (
                "src/app.tsx",
                "import React from 'react';\nimport map from \"lodash/map\";\nconst ui = await import('@scope/ui/button');\nimport fs from 'node:fs';\n",
            ),
            (".eslintrc.json", r#"{"plugins":["eslint-plugin-react"]}"#),
            ("node_modules/left-pad/index.js", "require('left-pad')"),
        ]);
        let got: Vec<String> = find(&f).into_iter().map(|u| format!("{} {}", u.section, u.name)).collect();
        assert_eq!(got, ["dependencies left-pad", "devDependencies vitest"]);
    }

    #[test]
    fn import_names() {
        let mut s = BTreeSet::new();
        js_packages(
            "const a = require( 'x/y' );\nexport * from '@a/b/c';\nimport './local';\n@import '~bootstrap/scss/x';\n",
            &mut s,
        );
        assert_eq!(s.into_iter().collect::<Vec<_>>(), ["@a/b", "bootstrap", "x"]);
    }
}

#[cfg(test)]
mod this_repo {
    /// Prints what the repository's own manifests name and its code never uses.
    /// `cargo test --lib this_repo -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn this_repo() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut files = std::collections::BTreeMap::new();
        fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut std::collections::BTreeMap<String, String>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    if !matches!(name.as_str(), "target" | ".git" | "node_modules") {
                        walk(&p, root, out);
                    }
                } else if name.ends_with(".rs") || name == "Cargo.toml" || name == "package.json" {
                    let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                    if let Ok(t) = std::fs::read_to_string(&p) {
                        out.insert(rel, t);
                    }
                }
            }
        }
        walk(&root, &root, &mut files);
        for u in super::find(&files) {
            println!("{} [{}] {}", u.manifest, u.section, u.name);
        }
    }
}
