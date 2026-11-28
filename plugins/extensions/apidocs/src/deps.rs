//! The project's dependencies, read from files already on disk (nothing fetched): Cargo.lock
//! and `cargo metadata --offline`, package.json and the packages in node_modules,
//! requirements.txt and pyproject.toml, and the packages installed in the project's virtual
//! environment.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dep {
    pub name: String,
    pub version: String,
    /// "" when not known.
    pub licence: String,
    /// "direct", "indirect", "dev", "optional", "build".
    pub kind: String,
    /// Its documentation on disk, relative to the open folder; "" for none.
    pub docs: String,
}

// ----- Rust ----------------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct LockPackage {
    name: String,
    version: String,
    source: String,
    dependencies: Vec<String>,
}

fn lock_packages(text: &str) -> Vec<LockPackage> {
    let mut out: Vec<LockPackage> = Vec::new();
    let mut in_deps = false;
    let mut in_package = false;
    for line in text.lines() {
        let t = line.trim();
        if t == "[[package]]" {
            out.push(LockPackage::default());
            in_deps = false;
            in_package = true;
            continue;
        }
        if !in_deps && t.starts_with('[') {
            // Another table ([metadata], [[patch.unused]]): not a package's.
            in_package = false;
            continue;
        }
        let Some(p) = out.last_mut().filter(|_| in_package) else { continue };
        if in_deps {
            if t.starts_with(']') {
                in_deps = false;
            } else if let Some(d) = unquote(t.trim_end_matches(',')) {
                p.dependencies.push(d);
            }
            continue;
        }
        let Some((key, value)) = t.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "name" => p.name = unquote(value).unwrap_or_default(),
            "version" => p.version = unquote(value).unwrap_or_default(),
            "source" => p.source = unquote(value).unwrap_or_default(),
            "dependencies" => {
                let inner = value.trim_start_matches('[');
                if let Some(list) = inner.strip_suffix(']') {
                    p.dependencies.extend(list.split(',').filter_map(|d| unquote(d.trim())));
                } else {
                    in_deps = true;
                    p.dependencies.extend(inner.split(',').filter_map(|d| unquote(d.trim())));
                }
            }
            _ => {}
        }
    }
    out.retain(|p| !p.name.is_empty());
    out
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.strip_prefix('"')?.strip_suffix('"')?;
    Some(s.to_owned())
}

/// The packages Cargo.lock pins, the project's own left out: each marked direct (a
/// dependency of one of the project's own packages) or indirect.
pub fn cargo_lock(text: &str) -> Vec<Dep> {
    let packages = lock_packages(text);
    // A dependency is written "name", or "name version" where two versions are locked.
    let direct: BTreeSet<(String, Option<String>)> = packages
        .iter()
        .filter(|p| p.source.is_empty())
        .flat_map(|p| p.dependencies.iter())
        .map(|d| {
            let mut w = d.split_whitespace();
            (w.next().unwrap_or("").to_owned(), w.next().map(str::to_owned))
        })
        .collect();
    packages
        .into_iter()
        .filter(|p| !p.source.is_empty())
        .map(|p| {
            let is_direct =
                direct.contains(&(p.name.clone(), None)) || direct.contains(&(p.name.clone(), Some(p.version.clone())));
            Dep {
                kind: if is_direct { "direct" } else { "indirect" }.into(),
                name: p.name,
                version: p.version,
                ..Dep::default()
            }
        })
        .collect()
}

/// What `cargo metadata --format-version 1` says: each package's licence, and where it
/// builds (its documentation goes to `doc` in there).
#[derive(Debug, Default, PartialEq)]
pub struct Metadata {
    /// By name and version.
    pub licences: HashMap<(String, String), String>,
    pub target_directory: String,
}

pub fn cargo_metadata(json: &str) -> Result<Metadata, String> {
    // Cargo may print a warning line before the JSON.
    let start = json.find('{').ok_or("cargo metadata printed no JSON")?;
    let v: Value = serde_json::from_str(json[start..].trim()).map_err(|e| format!("cargo metadata: {e}"))?;
    let mut out =
        Metadata { target_directory: v["target_directory"].as_str().unwrap_or("").to_owned(), ..Metadata::default() };
    for p in v["packages"].as_array().into_iter().flatten() {
        let name = p["name"].as_str().unwrap_or("").to_owned();
        let version = p["version"].as_str().unwrap_or("").to_owned();
        let licence = match (p["license"].as_str(), p["license_file"].as_str()) {
            (Some(l), _) => l.to_owned(),
            (None, Some(f)) => format!("see {}", f.rsplit(['/', '\\']).next().unwrap_or(f)),
            _ => String::new(),
        };
        out.licences.insert((name, version), licence);
    }
    Ok(out)
}

/// A crate's documentation folder name in `target/doc`.
pub fn crate_doc_dir(name: &str) -> String {
    name.replace('-', "_")
}

// ----- JavaScript ----------------------------------------------------------------------------

/// The dependencies package.json asks for: name, the versions wanted, and the kind.
pub fn package_json(text: &str) -> Result<Vec<(String, String, String)>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("package.json: {e}"))?;
    let mut out = Vec::new();
    for (key, kind) in [
        ("dependencies", "direct"),
        ("devDependencies", "dev"),
        ("optionalDependencies", "optional"),
        ("peerDependencies", "peer"),
    ] {
        for (name, range) in v[key].as_object().into_iter().flatten() {
            out.push((name.clone(), range.as_str().unwrap_or("").to_owned(), kind.to_owned()));
        }
    }
    Ok(out)
}

/// An installed package's own package.json: its version and licence.
pub fn installed_package(text: &str) -> (String, String) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Default::default() };
    let licence = match &v["license"] {
        Value::String(s) => s.clone(),
        Value::Object(o) => o.get("type").and_then(Value::as_str).unwrap_or("").to_owned(),
        _ => v["licenses"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["type"].as_str()).collect::<Vec<_>>().join(" OR "))
            .unwrap_or_default(),
    };
    (v["version"].as_str().unwrap_or("").to_owned(), licence)
}

// ----- Python --------------------------------------------------------------------------------

/// A requirement line's name and version spec ("requests[socks]>=2.0; python_version>'3'").
pub fn requirement(line: &str) -> Option<(String, String)> {
    let t = line.split(" #").next().unwrap_or("").trim();
    if t.is_empty() || t.starts_with('#') || t.starts_with('-') || t.contains("://") {
        return None;
    }
    let t = t.split(';').next().unwrap_or("").trim();
    let end = t.find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))).unwrap_or(t.len());
    let name = &t[..end];
    if name.is_empty() {
        return None;
    }
    let mut spec = t[end..].trim();
    if spec.starts_with('[') {
        spec = spec.split_once(']').map_or("", |(_, s)| s.trim());
    }
    Some((name.to_owned(), spec.trim_start_matches('(').trim_end_matches(')').replace(' ', "")))
}

pub fn requirements(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(requirement).collect()
}

/// The dependencies pyproject.toml lists: PEP 621's (`[project]` and its optional ones) and
/// Poetry's.
pub fn pyproject(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut table = String::new();
    let mut array: Option<String> = None; // the kind, inside a list going on
    for line in text.lines() {
        let t = line.split(" #").next().unwrap_or("").trim();
        if let Some(kind) = &array {
            for item in t.split(',') {
                let item = item.trim().trim_end_matches(']').trim();
                if let Some((n, v)) = unquote(item).or_else(|| unquote_single(item)).and_then(|s| requirement(&s)) {
                    out.push((n, v, kind.clone()));
                }
            }
            if t.contains(']') {
                array = None;
            }
            continue;
        }
        if t.starts_with('[') {
            table = t.trim_matches(['[', ']']).trim().to_owned();
            continue;
        }
        let Some((key, value)) = t.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        let kind = match (table.as_str(), key) {
            ("project", "dependencies") => Some("direct".to_owned()),
            ("project.optional-dependencies", group) => Some(format!("optional ({group})")),
            ("dependency-groups", group) => Some(format!("group ({group})")),
            _ => None,
        };
        if let Some(kind) = kind {
            if let Some(rest) = value.strip_prefix('[') {
                for item in rest.split(',') {
                    let item = item.trim().trim_end_matches(']').trim();
                    if let Some((n, v)) = unquote(item).or_else(|| unquote_single(item)).and_then(|s| requirement(&s)) {
                        out.push((n, v, kind.clone()));
                    }
                }
                if !rest.contains(']') {
                    array = Some(kind);
                }
            }
            continue;
        }
        let poetry = match table.as_str() {
            "tool.poetry.dependencies" => Some("direct"),
            "tool.poetry.dev-dependencies" => Some("dev"),
            t if t.starts_with("tool.poetry.group.") && t.ends_with(".dependencies") => Some("dev"),
            _ => None,
        };
        if let Some(kind) = poetry {
            if key == "python" {
                continue;
            }
            let version = unquote(value).unwrap_or_else(|| {
                value.split("version").nth(1).and_then(|v| v.split('"').nth(1)).unwrap_or("").to_owned()
            });
            out.push((key.trim_matches('"').to_owned(), version, kind.to_owned()));
        }
    }
    out
}

fn unquote_single(s: &str) -> Option<String> {
    Some(s.strip_prefix('\'')?.strip_suffix('\'')?.to_owned())
}

/// A name as Python's packaging compares it: lower case, runs of `-_.` as one `-`.
pub fn normalize(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if matches!(c, '-' | '_' | '.') {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// An installed package's `<name>-<version>.dist-info` folder: its normalized name and version.
pub fn dist_info(folder: &str) -> Option<(String, String)> {
    let stem = folder.strip_suffix(".dist-info")?;
    let (name, version) = stem.rsplit_once('-')?;
    Some((normalize(name), version.to_owned()))
}

/// The licence a package's METADATA file gives.
pub fn wheel_licence(metadata: &str) -> String {
    let mut licence = String::new();
    let mut classifier = String::new();
    for line in metadata.lines() {
        if line.is_empty() {
            break; // the description starts
        }
        if let Some(l) = line.strip_prefix("License-Expression:") {
            return l.trim().to_owned();
        }
        if let Some(l) = line.strip_prefix("License:") {
            licence = l.trim().to_owned();
        }
        if let Some(c) = line.strip_prefix("Classifier: License ::") {
            classifier = c.rsplit("::").next().unwrap_or("").trim().to_owned();
        }
    }
    // A whole licence's text pasted into License: is no name.
    if !licence.is_empty() && licence.len() <= 40 && !licence.eq_ignore_ascii_case("UNKNOWN") {
        licence
    } else {
        classifier
    }
}

// ----- the page ------------------------------------------------------------------------------

fn cell(s: &str) -> String {
    if s.is_empty() { "—".into() } else { s.replace('|', "\\|") }
}

/// One Markdown page: a table for each ecosystem, then notes on what couldn't be read.
pub fn page(project: &str, groups: &BTreeMap<String, Vec<Dep>>, notes: &[String]) -> String {
    let total: usize = groups.values().map(Vec::len).sum();
    let mut md = format!("# Dependencies of {project}\n\n");
    if total == 0 {
        md.push_str(
            "No dependencies found: no Cargo.lock, package.json, requirements.txt or pyproject.toml lists any.\n",
        );
    } else {
        md.push_str(&format!(
            "{total} dependenc{}, read from the project's files and what is installed on this computer; \
             nothing was fetched from the network. Made by Coiman's API Docs.\n",
            if total == 1 { "y" } else { "ies" }
        ));
    }
    for (title, deps) in groups {
        if deps.is_empty() {
            continue;
        }
        let licences: BTreeSet<&str> = deps.iter().map(|d| d.licence.as_str()).filter(|l| !l.is_empty()).collect();
        md.push_str(&format!("\n## {title}\n\n"));
        md.push_str(&format!(
            "{} package{}; licences: {}.\n\n",
            deps.len(),
            if deps.len() == 1 { "" } else { "s" },
            if licences.is_empty() {
                "not known".to_owned()
            } else {
                licences.into_iter().collect::<Vec<_>>().join(", ")
            }
        ));
        md.push_str("| Package | Version | Licence | Kind | Local docs |\n|---|---|---|---|---|\n");
        for d in deps {
            let docs = if d.docs.is_empty() { "—".to_owned() } else { format!("[{}](/{})", d.docs, d.docs) };
            md.push_str(&format!(
                "| {} | {} | {} | {} | {docs} |\n",
                cell(&d.name),
                cell(&d.version),
                cell(&d.licence),
                cell(&d.kind)
            ));
        }
    }
    if !notes.is_empty() {
        md.push_str("\n## Notes\n\n");
        for n in notes {
            md.push_str(&format!("- {n}\n"));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "demo"
version = "0.1.0"
dependencies = [
 "itoa",
 "serde 1.0.200",
]

[[package]]
name = "itoa"
version = "1.0.19"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "abc"

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["serde_derive"]

[[package]]
name = "serde"
version = "0.9.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "serde_derive"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn cargo_lock_direct_and_indirect() {
        let deps = cargo_lock(LOCK);
        let got: Vec<(&str, &str, &str)> =
            deps.iter().map(|d| (d.name.as_str(), d.version.as_str(), d.kind.as_str())).collect();
        assert_eq!(
            got,
            [
                ("itoa", "1.0.19", "direct"),
                ("serde", "1.0.200", "direct"),
                ("serde", "0.9.0", "indirect"),
                ("serde_derive", "1.0.200", "indirect"),
            ]
        );
    }

    #[test]
    fn cargo_metadata_licences() {
        let json = "warning: x\n{\"packages\":[{\"name\":\"itoa\",\"version\":\"1.0.19\",\"license\":\"MIT OR Apache-2.0\",\"license_file\":null},\
                    {\"name\":\"odd\",\"version\":\"0.1.0\",\"license\":null,\"license_file\":\"/r/odd/LICENSE.txt\"}],\
                    \"target_directory\":\"/p/target\"}";
        let m = cargo_metadata(json).unwrap();
        assert_eq!(m.target_directory, "/p/target");
        assert_eq!(m.licences[&("itoa".into(), "1.0.19".into())], "MIT OR Apache-2.0");
        assert_eq!(m.licences[&("odd".into(), "0.1.0".into())], "see LICENSE.txt");
        assert!(cargo_metadata("error: no network").is_err());
        assert_eq!(crate_doc_dir("serde-json"), "serde_json");
    }

    #[test]
    fn npm_packages() {
        let deps = package_json(
            r#"{"name":"web","dependencies":{"left-pad":"^1.3.0","@scope/x":"2"},"devDependencies":{"jest":"~29"}}"#,
        )
        .unwrap();
        assert_eq!(
            deps,
            [
                ("left-pad".into(), "^1.3.0".into(), "direct".into()),
                ("@scope/x".into(), "2".into(), "direct".into()),
                ("jest".into(), "~29".into(), "dev".into())
            ]
        );
        assert_eq!(installed_package(r#"{"version":"1.3.0","license":"WTFPL"}"#), ("1.3.0".into(), "WTFPL".into()));
        assert_eq!(installed_package(r#"{"version":"1","license":{"type":"MIT"}}"#).1, "MIT");
        assert_eq!(installed_package(r#"{"version":"1","licenses":[{"type":"MIT"},{"type":"GPL"}]}"#).1, "MIT OR GPL");
        assert!(package_json("{").is_err());
    }

    #[test]
    fn python_requirements_and_pyproject() {
        let reqs = requirements(
            "# pinned\nrequests[socks]==2.31.0 ; python_version > '3'\n-r other.txt\nnumpy\nDjango >= 4.2, <5  # web\ngit+https://x/y.git\n",
        );
        assert_eq!(
            reqs,
            [("requests".into(), "==2.31.0".into()), ("numpy".into(), "".into()), ("Django".into(), ">=4.2,<5".into())]
        );
        let py = pyproject(
            "[project]\nname = \"demo\"\ndependencies = [\n  \"click>=8\",\n  'rich',\n]\n\n[project.optional-dependencies]\ntest = [\"pytest>=7\"]\n\n[tool.poetry.dependencies]\npython = \"^3.10\"\nhttpx = \"^0.27\"\npydantic = { version = \"^2.0\", extras = [\"email\"] }\n",
        );
        assert_eq!(
            py,
            [
                ("click".into(), ">=8".into(), "direct".into()),
                ("rich".into(), "".into(), "direct".into()),
                ("pytest".into(), ">=7".into(), "optional (test)".into()),
                ("httpx".into(), "^0.27".into(), "direct".into()),
                ("pydantic".into(), "^2.0".into(), "direct".into()),
            ]
        );
        assert_eq!(normalize("Foo_Bar.baz"), "foo-bar-baz");
        assert_eq!(dist_info("Foo_Bar-1.2.3.dist-info"), Some(("foo-bar".into(), "1.2.3".into())));
        assert_eq!(wheel_licence("Name: x\nLicense: MIT\nClassifier: License :: OSI Approved :: BSD License\n"), "MIT");
        assert_eq!(
            wheel_licence("Name: x\nLicense: UNKNOWN\nClassifier: License :: OSI Approved :: BSD License\n"),
            "BSD License"
        );
        assert_eq!(wheel_licence("Name: x\nLicense-Expression: Apache-2.0\nLicense: MIT\n"), "Apache-2.0");
        assert_eq!(wheel_licence("Name: x\n\nLicense: in the body\n"), "");
    }

    #[test]
    fn page_tables() {
        let mut groups = BTreeMap::new();
        groups.insert(
            "Rust (Cargo.lock)".to_owned(),
            vec![Dep {
                name: "itoa".into(),
                version: "1.0.19".into(),
                licence: "MIT".into(),
                kind: "direct".into(),
                docs: "target/doc/itoa/index.html".into(),
            }],
        );
        let md = page("demo", &groups, &["cargo metadata couldn't run".into()]);
        assert!(md.contains("1 dependency, read from"), "{md}");
        assert!(md.contains("\n## Rust (Cargo.lock)\n\n1 package; licences: MIT.\n"), "{md}");
        assert!(
            md.contains("| itoa | 1.0.19 | MIT | direct | [target/doc/itoa/index.html](/target/doc/itoa/index.html) |"),
            "{md}"
        );
        assert!(md.ends_with("## Notes\n\n- cargo metadata couldn't run\n"), "{md}");
        assert!(page("x", &BTreeMap::new(), &[]).contains("No dependencies found"));
    }
}
