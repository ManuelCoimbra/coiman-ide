//! A project's dependencies as its lockfiles pin them (`Cargo.lock`, `package-lock.json`,
//! `poetry.lock`, `uv.lock`): the tree of what needs what, packages pinned at more than one
//! version, and which are out of date, as the package managers themselves say
//! (`cargo update --dry-run`, `npm outdated`, `pip list --outdated`). Updating or removing one
//! is the package manager's own command, run in a terminal.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::language::lsp::discovery::quiet_command;
use crate::language::lsp::servers::which;

/// Which package manager a lockfile is of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ecosystem {
    Cargo,
    Npm,
    Poetry,
    Uv,
    /// The project's Python environment, through pip (no lockfile).
    Pip,
}

impl Ecosystem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo",
            Self::Npm => "npm",
            Self::Poetry => "Poetry",
            Self::Uv => "uv",
            Self::Pip => "pip",
        }
    }
}

/// A pinned package, and the packages it needs (indices into [`Lock::packages`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub deps: Vec<usize>,
}

/// What a lockfile pins.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lock {
    pub ecosystem: Ecosystem,
    pub file: PathBuf,
    pub packages: Vec<Package>,
    /// Where the tree starts: the project's own packages (or, without them, what nothing needs).
    pub roots: Vec<usize>,
}

impl Lock {
    /// Packages a root needs directly: the ones its manifest names.
    pub fn direct(&self) -> HashSet<usize> {
        self.roots.iter().flat_map(|&r| self.packages[r].deps.iter().copied()).collect()
    }

    /// Packages pinned at more than one version, with what wants each.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, p) in self.packages.iter().enumerate() {
            by_name.entry(p.name.as_str()).or_default().push(i);
        }
        let mut wanted_by: HashMap<usize, Vec<String>> = HashMap::new();
        for p in &self.packages {
            for &d in &p.deps {
                wanted_by.entry(d).or_default().push(format!("{} {}", p.name, p.version));
            }
        }
        let mut out: Vec<Conflict> = by_name
            .into_iter()
            .filter_map(|(name, at)| {
                let mut versions: Vec<(String, Vec<String>)> = at
                    .iter()
                    .map(|&i| {
                        let mut by = wanted_by.get(&i).cloned().unwrap_or_default();
                        by.sort();
                        by.dedup();
                        (self.packages[i].version.clone(), by)
                    })
                    .collect();
                versions.sort_by(|a, b| compare_versions(&a.0, &b.0));
                versions.dedup_by(|a, b| a.0 == b.0);
                (versions.len() > 1).then(|| Conflict { name: name.to_owned(), versions })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

/// A package pinned at several versions: each version with the packages that want it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub name: String,
    pub versions: Vec<(String, Vec<String>)>,
}

/// The lockfiles in the project's folder, read.
pub fn read_locks(root: &Path) -> (Vec<Lock>, Vec<String>) {
    let mut locks = Vec::new();
    let mut notes = Vec::new();
    for (file, eco) in [
        ("Cargo.lock", Ecosystem::Cargo),
        ("package-lock.json", Ecosystem::Npm),
        ("poetry.lock", Ecosystem::Poetry),
        ("uv.lock", Ecosystem::Uv),
    ] {
        let path = root.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let read = match eco {
            Ecosystem::Cargo => parse_cargo_lock(&text),
            Ecosystem::Npm => parse_npm_lock(&text),
            Ecosystem::Poetry | Ecosystem::Uv => parse_python_lock(&text, eco),
            Ecosystem::Pip => continue,
        };
        match read {
            Ok((packages, roots)) => locks.push(Lock { ecosystem: eco, file: path, packages, roots }),
            Err(e) => notes.push(format!("{file}: {e}")),
        }
    }
    (locks, notes)
}

type Parsed = (Vec<Package>, Vec<usize>);

/// `Cargo.lock`: `[[package]]` with `name`, `version`, `dependencies = ["name", "name 1.0"]`;
/// the project's own packages have no `source`.
pub fn parse_cargo_lock(text: &str) -> Result<Parsed, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| e.message().to_owned())?;
    let list = doc.get("package").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let field = |t: &toml::Value, k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let mut packages: Vec<Package> = list
        .iter()
        .map(|t| Package { name: field(t, "name"), version: field(t, "version"), deps: Vec::new() })
        .collect();
    let mut roots = Vec::new();
    for (i, t) in list.iter().enumerate() {
        if t.get("source").is_none() {
            roots.push(i);
        }
        let deps = t.get("dependencies").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        for d in deps.iter().filter_map(|d| d.as_str()) {
            // "name", or "name version" when several versions are pinned, maybe with "(source)".
            let mut parts = d.split_whitespace();
            let name = parts.next().unwrap_or("");
            let version = parts.next();
            let found = packages.iter().position(|p| p.name == name && version.is_none_or(|v| p.version == v));
            if let Some(j) = found {
                packages[i].deps.push(j);
            }
        }
    }
    Ok((packages, roots))
}

/// `package-lock.json` (version 2 or 3): `packages` keyed by folder, `""` the project itself.
/// A package's dependency is found as Node finds it: in its own `node_modules`, then each
/// folder above.
pub fn parse_npm_lock(text: &str) -> Result<Parsed, String> {
    let doc: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let map = doc.get("packages").and_then(|p| p.as_object()).ok_or("no `packages` (lockfile version 1 isn't read)")?;
    let keys: Vec<&String> = map.keys().collect();
    let index: HashMap<&str, usize> = keys.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let name_of = |key: &str, v: &serde_json::Value| -> String {
        if key.is_empty() {
            return v.get("name").and_then(|n| n.as_str()).unwrap_or("(project)").to_owned();
        }
        key.rsplit_once("node_modules/").map_or(key, |(_, n)| n).to_owned()
    };
    let mut packages: Vec<Package> = keys
        .iter()
        .map(|k| {
            let v = &map[k.as_str()];
            Package {
                name: name_of(k, v),
                version: v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_owned(),
                deps: Vec::new(),
            }
        })
        .collect();
    for (i, key) in keys.iter().enumerate() {
        let v = &map[key.as_str()];
        let mut names: Vec<&str> = Vec::new();
        for field in ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"] {
            if let Some(o) = v.get(field).and_then(|d| d.as_object()) {
                names.extend(o.keys().map(String::as_str));
            }
        }
        for name in names {
            // From this package's folder up to the project's.
            let mut base = key.to_string();
            let found = loop {
                let candidate = if base.is_empty() {
                    format!("node_modules/{name}")
                } else {
                    format!("{base}/node_modules/{name}")
                };
                if let Some(&j) = index.get(candidate.as_str()) {
                    break Some(j);
                }
                if base.is_empty() {
                    break None;
                }
                base = base.rfind("/node_modules/").map_or(String::new(), |at| base[..at].to_owned());
                if base == "node_modules" {
                    base.clear();
                }
            };
            if let Some(j) = found
                && !packages[i].deps.contains(&j)
            {
                packages[i].deps.push(j);
            }
        }
    }
    let roots = index.get("").copied().into_iter().collect();
    Ok((packages, roots))
}

/// `poetry.lock` (`[package.dependencies]` tables) and `uv.lock` (`dependencies = [{ name }]`).
/// The roots: uv's project itself (an editable or virtual source), else what nothing needs.
pub fn parse_python_lock(text: &str, eco: Ecosystem) -> Result<Parsed, String> {
    let doc: toml::Table = toml::from_str(text).map_err(|e| e.message().to_owned())?;
    let list = doc.get("package").and_then(|p| p.as_array()).cloned().unwrap_or_default();
    let norm = |n: &str| n.to_ascii_lowercase().replace(['_', '.'], "-");
    let field = |t: &toml::Value, k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let mut packages: Vec<Package> = list
        .iter()
        .map(|t| Package { name: field(t, "name"), version: field(t, "version"), deps: Vec::new() })
        .collect();
    let mut roots = Vec::new();
    for (i, t) in list.iter().enumerate() {
        let names: Vec<String> = match eco {
            Ecosystem::Uv => {
                let source = t.get("source");
                if source.is_some_and(|s| s.get("editable").is_some() || s.get("virtual").is_some()) {
                    roots.push(i);
                }
                let mut all: Vec<String> = Vec::new();
                for key in ["dependencies", "dev-dependencies"] {
                    match t.get(key) {
                        Some(toml::Value::Array(a)) => all
                            .extend(a.iter().filter_map(|d| d.get("name").and_then(|n| n.as_str())).map(str::to_owned)),
                        // uv's dev-dependencies: groups of lists.
                        Some(toml::Value::Table(groups)) => all.extend(
                            groups
                                .values()
                                .filter_map(|g| g.as_array())
                                .flatten()
                                .filter_map(|d| d.get("name").and_then(|n| n.as_str()))
                                .map(str::to_owned),
                        ),
                        _ => {}
                    }
                }
                all
            }
            _ => t
                .get("dependencies")
                .and_then(|d| d.as_table())
                .map(|d| d.keys().cloned().collect())
                .unwrap_or_default(),
        };
        for name in names {
            if let Some(j) = packages.iter().position(|p| norm(&p.name) == norm(&name))
                && !packages[i].deps.contains(&j)
            {
                packages[i].deps.push(j);
            }
        }
    }
    if roots.is_empty() {
        let needed: HashSet<usize> = packages.iter().flat_map(|p| p.deps.iter().copied()).collect();
        roots = (0..packages.len()).filter(|i| !needed.contains(i)).collect();
    }
    Ok((packages, roots))
}

/// Versions compared part by part, as numbers where they are (`1.10.0` after `1.9.3`).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<(u64, String)> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|p| {
                let digits: String = p.chars().take_while(char::is_ascii_digit).collect();
                (digits.parse().unwrap_or(0), p[digits.len()..].to_owned())
            })
            .collect()
    };
    parts(a).cmp(&parts(b))
}

/// How big a move from `from` to `to` is: "major", "minor" or "patch" (a 0.x minor counts as
/// major, as Cargo and npm treat it).
pub fn bump(from: &str, to: &str) -> &'static str {
    let nums = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split('.')
            .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (nums(from), nums(to));
    let at = |v: &Vec<u64>, i: usize| v.get(i).copied().unwrap_or(0);
    if at(&a, 0) != at(&b, 0) || (at(&a, 0) == 0 && at(&a, 1) != at(&b, 1)) {
        "major"
    } else if at(&a, 1) != at(&b, 1) {
        "minor"
    } else {
        "patch"
    }
}

/// A package with a newer version: the one pinned, the newest its requirement allows, and
/// the newest there is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outdated {
    pub ecosystem: Ecosystem,
    pub name: String,
    pub current: String,
    /// The newest the project's requirement allows (what an update gets), when known.
    pub wanted: Option<String>,
    pub latest: Option<String>,
}

/// `cargo update --dry-run --verbose`: "Updating x v1.0.0 -> v1.0.1", with "(available:
/// v2.0.0)" when a newer one is out of the requirement's reach, and "Unchanged x v0.3.0
/// (available: v0.4.0)" for those that stay.
pub fn parse_cargo_dry_run(out: &str) -> Vec<Outdated> {
    let mut found = Vec::new();
    for line in out.lines().map(str::trim) {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        let available =
            rest.split_once("(available: ").map(|(_, v)| v.trim_end_matches(')').trim_start_matches('v').to_owned());
        let rest = rest.split(" (").next().unwrap_or(rest);
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(current)) = (parts.next(), parts.next()) else { continue };
        let current = current.trim_start_matches('v').to_owned();
        match word {
            "Updating" if parts.next() == Some("->") => {
                let wanted = parts.next().map(|v| v.trim_start_matches('v').to_owned());
                let latest = available.or_else(|| wanted.clone());
                found.push(Outdated { ecosystem: Ecosystem::Cargo, name: name.to_owned(), current, wanted, latest });
            }
            "Unchanged" if available.is_some() => found.push(Outdated {
                ecosystem: Ecosystem::Cargo,
                name: name.to_owned(),
                current,
                wanted: None,
                latest: available,
            }),
            _ => {}
        }
    }
    found
}

/// `npm outdated --json`: `{ "name": { "current", "wanted", "latest" } }`.
pub fn parse_npm_outdated(json: &str) -> Result<Vec<Outdated>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let map = doc.as_object().ok_or("not an object")?;
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
    Ok(map
        .iter()
        .map(|(name, v)| Outdated {
            ecosystem: Ecosystem::Npm,
            name: name.clone(),
            current: s(v, "current").unwrap_or_else(|| "not installed".to_owned()),
            wanted: s(v, "wanted"),
            latest: s(v, "latest"),
        })
        .collect())
}

/// `pip list --outdated --format=json`: `[{ "name", "version", "latest_version" }]`.
pub fn parse_pip_outdated(json: &str) -> Result<Vec<Outdated>, String> {
    let doc: serde_json::Value = serde_json::from_str(json.trim()).map_err(|e| e.to_string())?;
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned();
    Ok(doc
        .as_array()
        .ok_or("not a list")?
        .iter()
        .map(|v| Outdated {
            ecosystem: Ecosystem::Pip,
            name: s(v, "name"),
            current: s(v, "version"),
            wanted: None,
            latest: Some(s(v, "latest_version")),
        })
        .collect())
}

/// Asks each package manager the project uses what's out of date. They ask their registries
/// themselves. `python` is the project's interpreter, when it has one.
pub fn outdated(root: &Path, python: Option<&Path>) -> (Vec<Outdated>, Vec<String>) {
    let mut out = Vec::new();
    let mut notes = Vec::new();
    let run = |program: &Path, args: &[&str]| {
        quiet_command(program).args(args).current_dir(root).stdin(Stdio::null()).output()
    };
    let last_line = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned()
    };
    if root.join("Cargo.toml").is_file() {
        match which("cargo") {
            None => notes.push("Cargo isn't installed".to_owned()),
            Some(cargo) => match run(&cargo, &["update", "--dry-run", "--verbose"]) {
                Ok(o) => {
                    // Cargo writes these lines on standard error.
                    let text = String::from_utf8_lossy(&o.stderr);
                    let found = parse_cargo_dry_run(&text);
                    if found.is_empty() && text.contains("error") {
                        notes.push(format!("cargo update: {}", last_line(&o.stderr)));
                    }
                    out.extend(found);
                }
                Err(e) => notes.push(format!("cargo: {e}")),
            },
        }
    }
    if root.join("package.json").is_file() {
        match which("npm") {
            None => notes.push("npm isn't installed".to_owned()),
            Some(npm) => match run(&npm, &["outdated", "--json"]) {
                // It exits 1 when something is out of date: the JSON says what.
                Ok(o) => match parse_npm_outdated(&String::from_utf8_lossy(&o.stdout)) {
                    Ok(list) => out.extend(list),
                    Err(e) => notes.push(format!("npm outdated: {e}: {}", last_line(&o.stderr))),
                },
                Err(e) => notes.push(format!("npm: {e}")),
            },
        }
    }
    let python_project = ["pyproject.toml", "requirements.txt", "setup.py", "poetry.lock", "uv.lock"]
        .iter()
        .any(|f| root.join(f).is_file());
    if python_project {
        match python {
            None => notes.push("No Python environment found for this project".to_owned()),
            Some(py) => match run(py, &["-m", "pip", "list", "--outdated", "--format=json"]) {
                Ok(o) if o.status.success() => match parse_pip_outdated(&String::from_utf8_lossy(&o.stdout)) {
                    Ok(list) => out.extend(list),
                    Err(e) => notes.push(format!("pip list: {e}")),
                },
                Ok(o) => notes.push(format!("pip list: {}", last_line(&o.stderr))),
                Err(e) => notes.push(format!("{}: {e}", py.display())),
            },
        }
    }
    if out.is_empty()
        && notes.is_empty()
        && !python_project
        && !root.join("Cargo.toml").is_file()
        && !root.join("package.json").is_file()
    {
        notes.push("No Cargo.toml, package.json or Python project here".to_owned());
    }
    out.sort_by(|a, b| a.ecosystem.label().cmp(b.ecosystem.label()).then(a.name.cmp(&b.name)));
    (out, notes)
}

/// The command that updates a package (as far as the project's requirement allows), run in
/// the project's folder: a program and its arguments.
pub fn update_command(eco: Ecosystem, name: &str, python: Option<&Path>) -> Option<(String, Vec<String>)> {
    let own = |p: &str, args: &[&str]| Some((p.to_owned(), args.iter().map(|a| a.to_string()).collect()));
    match eco {
        Ecosystem::Cargo => own("cargo", &["update", "-p", name]),
        Ecosystem::Npm => own("npm", &["update", name]),
        Ecosystem::Poetry => own("poetry", &["update", name]),
        Ecosystem::Uv => own("uv", &["lock", "--upgrade-package", name]),
        Ecosystem::Pip => {
            let py = python?.to_string_lossy().into_owned();
            Some((py, ["-m", "pip", "install", "--upgrade", name].map(str::to_owned).to_vec()))
        }
    }
}

/// The command that removes a dependency the project names itself.
pub fn remove_command(eco: Ecosystem, name: &str, python: Option<&Path>) -> Option<(String, Vec<String>)> {
    let own = |p: &str, args: &[&str]| Some((p.to_owned(), args.iter().map(|a| a.to_string()).collect()));
    match eco {
        Ecosystem::Cargo => own("cargo", &["remove", name]),
        Ecosystem::Npm => own("npm", &["uninstall", name]),
        Ecosystem::Poetry => own("poetry", &["remove", name]),
        Ecosystem::Uv => own("uv", &["remove", name]),
        Ecosystem::Pip => {
            let py = python?.to_string_lossy().into_owned();
            Some((py, ["-m", "pip", "uninstall", "-y", name].map(str::to_owned).to_vec()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_lock_tree_and_conflicts() {
        let text = r#"
version = 4

[[package]]
name = "app"
version = "0.1.0"
dependencies = ["serde", "rand 0.9.0", "old"]

[[package]]
name = "serde"
version = "1.0.200"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "rand"
version = "0.8.5"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "rand"
version = "0.9.0"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "old"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["rand 0.8.5"]
"#;
        let (packages, roots) = parse_cargo_lock(text).unwrap();
        assert_eq!(roots, [0]);
        let names: Vec<String> =
            packages[0].deps.iter().map(|&d| format!("{} {}", packages[d].name, packages[d].version)).collect();
        assert_eq!(names, ["serde 1.0.200", "rand 0.9.0", "old 1.0.0"]);
        let lock = Lock { ecosystem: Ecosystem::Cargo, file: PathBuf::new(), packages, roots };
        let c = lock.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "rand");
        assert_eq!(c[0].versions[0], ("0.8.5".to_owned(), vec!["old 1.0.0".to_owned()]));
        assert_eq!(c[0].versions[1], ("0.9.0".to_owned(), vec!["app 0.1.0".to_owned()]));
        assert_eq!(lock.direct().len(), 3);
    }

    #[test]
    fn npm_lock_finds_nested_and_hoisted() {
        let text = r#"{
  "name": "web", "lockfileVersion": 3,
  "packages": {
    "": { "name": "web", "dependencies": { "a": "^1" }, "devDependencies": { "b": "^2" } },
    "node_modules/a": { "version": "1.2.0", "dependencies": { "b": "^1" } },
    "node_modules/a/node_modules/b": { "version": "1.0.0" },
    "node_modules/b": { "version": "2.1.0", "dependencies": { "c": "*" } },
    "node_modules/c": { "version": "3.0.0" }
  }
}"#;
        let (p, roots) = parse_npm_lock(text).unwrap();
        let show = |i: usize| format!("{} {}", p[i].name, p[i].version);
        assert_eq!(roots.len(), 1);
        let root = roots[0];
        assert_eq!(p[root].deps.iter().map(|&d| show(d)).collect::<Vec<_>>(), ["a 1.2.0", "b 2.1.0"]);
        let a = p.iter().position(|x| x.name == "a").unwrap();
        assert_eq!(p[a].deps.iter().map(|&d| show(d)).collect::<Vec<_>>(), ["b 1.0.0"], "its own nested b");
        let b2 = p.iter().position(|x| x.name == "b" && x.version == "2.1.0").unwrap();
        assert_eq!(p[b2].deps.iter().map(|&d| show(d)).collect::<Vec<_>>(), ["c 3.0.0"]);
        let lock = Lock { ecosystem: Ecosystem::Npm, file: PathBuf::new(), packages: p, roots };
        assert_eq!(lock.conflicts()[0].name, "b");
    }

    #[test]
    fn python_locks() {
        let uv = r#"
[[package]]
name = "app"
version = "0.1.0"
source = { editable = "." }
dependencies = [{ name = "requests" }]

[[package]]
name = "requests"
version = "2.32.0"
dependencies = [{ name = "idna" }]

[[package]]
name = "idna"
version = "3.7"
"#;
        let (p, roots) = parse_python_lock(uv, Ecosystem::Uv).unwrap();
        assert_eq!(roots, [0]);
        assert_eq!(p[1].deps, [2]);
        let poetry = r#"
[[package]]
name = "requests"
version = "2.32.0"

[package.dependencies]
idna = ">=2.5"

[[package]]
name = "idna"
version = "3.7"
"#;
        let (p, roots) = parse_python_lock(poetry, Ecosystem::Poetry).unwrap();
        assert_eq!(roots, [0], "what nothing needs");
        assert_eq!(p[0].deps, [1]);
    }

    #[test]
    fn outdated_outputs() {
        let cargo = "    Updating crates.io index\n     Locking 2 packages to latest compatible versions\n    Updating serde v1.0.200 -> v1.0.210\n    Updating rand v0.8.5 -> v0.8.6 (available: v0.9.0)\n   Unchanged egui v0.36.0 (available: v0.37.0)\n   Unchanged toml v1.1.8\nwarning: aborting update due to dry run\n";
        let got = parse_cargo_dry_run(cargo);
        assert_eq!(got.len(), 3);
        assert_eq!(
            (got[0].current.as_str(), got[0].wanted.as_deref(), got[0].latest.as_deref()),
            ("1.0.200", Some("1.0.210"), Some("1.0.210"))
        );
        assert_eq!((got[1].wanted.as_deref(), got[1].latest.as_deref()), (Some("0.8.6"), Some("0.9.0")));
        assert_eq!(
            (got[2].name.as_str(), got[2].wanted.as_deref(), got[2].latest.as_deref()),
            ("egui", None, Some("0.37.0"))
        );
        let npm =
            parse_npm_outdated(r#"{"left-pad":{"current":"1.0.0","wanted":"1.3.0","latest":"2.0.0","location":"x"}}"#)
                .unwrap();
        assert_eq!(
            (npm[0].current.as_str(), npm[0].wanted.as_deref(), npm[0].latest.as_deref()),
            ("1.0.0", Some("1.3.0"), Some("2.0.0"))
        );
        assert!(parse_npm_outdated("").unwrap().is_empty());
        let pip = parse_pip_outdated(
            r#"[{"name": "pip", "version": "24.0", "latest_version": "26.3", "latest_filetype": "wheel"}]"#,
        )
        .unwrap();
        assert_eq!((pip[0].name.as_str(), pip[0].latest.as_deref()), ("pip", Some("26.3")));
    }

    #[test]
    fn versions_and_bumps() {
        assert!(compare_versions("1.10.0", "1.9.3").is_gt());
        assert!(compare_versions("v2.0.0", "1.99.0").is_gt());
        assert_eq!(bump("1.2.3", "1.2.9"), "patch");
        assert_eq!(bump("1.2.3", "1.4.0"), "minor");
        assert_eq!(bump("1.2.3", "2.0.0"), "major");
        assert_eq!(bump("0.8.5", "0.9.0"), "major", "a 0.x minor breaks");
    }

    #[test]
    fn commands() {
        assert_eq!(update_command(Ecosystem::Cargo, "serde", None).unwrap().1, ["update", "-p", "serde"]);
        assert_eq!(
            remove_command(Ecosystem::Npm, "a", None).unwrap(),
            ("npm".to_owned(), vec!["uninstall".to_owned(), "a".to_owned()])
        );
        assert!(update_command(Ecosystem::Pip, "x", None).is_none(), "pip needs the project's Python");
        assert_eq!(
            update_command(Ecosystem::Pip, "x", Some(Path::new("py"))).unwrap().1,
            ["-m", "pip", "install", "--upgrade", "x"]
        );
    }

    /// This repository's own Cargo.lock reads, with its packages as roots.
    #[test]
    fn reads_this_repositorys_lock() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (locks, notes) = read_locks(&root);
        assert!(notes.is_empty(), "{notes:?}");
        let cargo = locks.iter().find(|l| l.ecosystem == Ecosystem::Cargo).expect("Cargo.lock");
        let roots: Vec<&str> = cargo.roots.iter().map(|&r| cargo.packages[r].name.as_str()).collect();
        assert!(roots.contains(&"coiman-ide") && roots.contains(&"coiman-core"), "{roots:?}");
        let ide = cargo.packages.iter().find(|p| p.name == "coiman-ide").unwrap();
        assert!(ide.deps.iter().any(|&d| cargo.packages[d].name == "eframe"));
    }
}
