//! The project's own packages and how they depend on each other, read from the build files
//! without running a build: Cargo workspaces, npm/pnpm/yarn workspaces, Gradle projects and
//! Maven modules. Only dependencies between the project's own packages are kept.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    /// The build file, relative to the project.
    pub manifest: String,
    /// "Cargo", "npm", "Gradle" or "Maven".
    pub tool: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dep {
    pub from: usize,
    pub to: usize,
    /// "dev", "build" or "" for a normal dependency.
    pub kind: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildGraph {
    pub packages: Vec<Package>,
    pub deps: Vec<Dep>,
}

/// The build files worth reading.
pub fn is_build_file(path: &str) -> bool {
    if path.split('/').any(|p| p == "node_modules" || p == "target" || p == "build" || p == ".gradle") {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.toml"
            | "package.json"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "build.gradle"
            | "build.gradle.kts"
            | "pom.xml"
    )
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// The packages and their dependencies on each other, from the build files' text by path.
pub fn read(files: &BTreeMap<String, String>) -> BuildGraph {
    let files: BTreeMap<String, String> =
        files.iter().filter(|(p, _)| is_build_file(p)).map(|(p, t)| (p.clone(), t.clone())).collect();
    let files = &files;
    let mut g = BuildGraph::default();
    cargo(files, &mut g);
    npm(files, &mut g);
    gradle(files, &mut g);
    maven(files, &mut g);
    g.deps.sort();
    g.deps.dedup_by(|a, b| a.from == b.from && a.to == b.to);
    g
}

/// Adds dependencies of `from` on the packages of `tool` whose names are in `names`.
fn link(g: &mut BuildGraph, tool: &str, from: usize, names: &[(String, String)]) {
    for (name, kind) in names {
        if let Some(to) = g.packages.iter().position(|p| p.tool == tool && &p.name == name)
            && to != from
        {
            g.deps.push(Dep { from, to, kind: kind.clone() });
        }
    }
}

fn cargo(files: &BTreeMap<String, String>, g: &mut BuildGraph) {
    let mut wanted: Vec<(usize, Vec<(String, String)>)> = Vec::new();
    for (path, text) in files.iter().filter(|(p, _)| p.rsplit('/').next() == Some("Cargo.toml")) {
        let Ok(t) = text.parse::<toml::Table>() else { continue };
        let Some(name) = t.get("package").and_then(|p| p.get("name")).and_then(toml::Value::as_str) else { continue };
        let mut deps = Vec::new();
        let mut tables: Vec<(&str, &toml::Table)> = Vec::new();
        for (key, kind) in [("dependencies", ""), ("dev-dependencies", "dev"), ("build-dependencies", "build")] {
            if let Some(d) = t.get(key).and_then(toml::Value::as_table) {
                tables.push((kind, d));
            }
            if let Some(targets) = t.get("target").and_then(toml::Value::as_table) {
                for target in targets.values().filter_map(toml::Value::as_table) {
                    if let Some(d) = target.get(key).and_then(toml::Value::as_table) {
                        tables.push((kind, d));
                    }
                }
            }
        }
        for (kind, table) in tables {
            for (key, v) in table {
                // A renamed dependency names its package.
                let real = v.get("package").and_then(toml::Value::as_str).unwrap_or(key);
                deps.push((real.to_owned(), kind.to_owned()));
            }
        }
        g.packages.push(Package { name: name.to_owned(), manifest: path.clone(), tool: "Cargo" });
        wanted.push((g.packages.len() - 1, deps));
    }

    for (from, deps) in wanted {
        link(g, "Cargo", from, &deps);
    }
}

fn npm(files: &BTreeMap<String, String>, g: &mut BuildGraph) {
    let mut wanted = Vec::new();
    for (path, text) in files.iter().filter(|(p, _)| p.rsplit('/').next() == Some("package.json")) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { continue };
        let Some(name) = json.get("name").and_then(|n| n.as_str()) else { continue };
        let mut deps = Vec::new();
        for (key, kind) in [
            ("dependencies", ""),
            ("devDependencies", "dev"),
            ("peerDependencies", "peer"),
            ("optionalDependencies", ""),
        ] {
            if let Some(d) = json.get(key).and_then(|d| d.as_object()) {
                deps.extend(d.keys().map(|k| (k.clone(), kind.to_owned())));
            }
        }
        g.packages.push(Package { name: name.to_owned(), manifest: path.clone(), tool: "npm" });
        wanted.push((g.packages.len() - 1, deps));
    }
    for (from, deps) in wanted {
        link(g, "npm", from, &deps);
    }
}

/// `include ':app', ':lib:core'` and `include("app")` in a Gradle settings file.
fn gradle_includes(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("include") else { continue };
        if !rest.starts_with([' ', '(', '\t']) {
            continue;
        }
        for part in rest.split(['\'', '"']).skip(1).step_by(2) {
            let p = part.trim().trim_start_matches(':');
            if !p.is_empty() {
                out.push(p.to_owned());
            }
        }
    }
    out
}

fn gradle(files: &BTreeMap<String, String>, g: &mut BuildGraph) {
    let mut wanted = Vec::new();
    for (path, text) in
        files.iter().filter(|(p, _)| p.ends_with("settings.gradle") || p.ends_with("settings.gradle.kts"))
    {
        let root = dir_of(path);
        for project in gradle_includes(text) {
            let dir = project.replace(':', "/");
            let base = if root.is_empty() { dir.clone() } else { format!("{root}/{dir}") };
            let build = [format!("{base}/build.gradle.kts"), format!("{base}/build.gradle")]
                .into_iter()
                .find(|b| files.contains_key(b))
                .unwrap_or(format!("{base}/build.gradle"));
            let body = files.get(&build).cloned().unwrap_or_default();
            let mut deps = Vec::new();
            for (i, _) in body.match_indices("project(") {
                let rest = &body[i + 8..];
                if let Some(name) = rest.split(['\'', '"']).nth(1) {
                    let kind = if body[..i].rsplit('\n').next().unwrap_or("").contains("test") { "dev" } else { "" };
                    deps.push((name.trim_start_matches(':').to_owned(), kind.to_owned()));
                }
            }
            // Type-safe accessors: projects.lib.core
            for (i, _) in body.match_indices("projects.") {
                let name: String = body[i + 9..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '-' || *c == '_')
                    .collect();
                deps.push((name.replace('.', ":"), String::new()));
            }
            g.packages.push(Package { name: project.clone(), manifest: build, tool: "Gradle" });
            wanted.push((g.packages.len() - 1, deps));
        }
    }
    for (from, deps) in wanted {
        link(g, "Gradle", from, &deps);
    }
}

/// The text between each `<tag>` and its `</tag>`.
fn tags<'a>(text: &'a str, tag: &str) -> Vec<&'a str> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(&after[..j]);
        rest = &after[j + close.len()..];
    }
    out
}

fn without(text: &str, tag: &str) -> String {
    let mut s = text.to_owned();
    for inner in tags(text, tag) {
        s = s.replace(&format!("<{tag}>{inner}</{tag}>"), "");
    }
    s
}

fn maven(files: &BTreeMap<String, String>, g: &mut BuildGraph) {
    let mut wanted = Vec::new();
    for (path, text) in files.iter().filter(|(p, _)| p.rsplit('/').next() == Some("pom.xml")) {
        // Its own artifactId: not the parent's, a plugin's or a dependency's.
        let own = ["parent", "dependencies", "dependencyManagement", "build", "plugins", "profiles"]
            .iter()
            .fold(text.clone(), |t, tag| without(&t, tag));
        let Some(name) = tags(&own, "artifactId").first().map(|s| s.trim().to_owned()) else { continue };
        let mut deps = Vec::new();
        let managed: BTreeSet<&str> = tags(text, "dependencyManagement").into_iter().collect();
        for block in tags(text, "dependencies") {
            if managed.iter().any(|m| m.contains(block)) {
                continue;
            }
            for d in tags(block, "dependency") {
                if let Some(a) = tags(d, "artifactId").first() {
                    let test = tags(d, "scope").first().is_some_and(|s| s.trim() == "test");
                    deps.push((a.trim().to_owned(), if test { "dev" } else { "" }.to_owned()));
                }
            }
        }
        g.packages.push(Package { name, manifest: path.clone(), tool: "Maven" });
        wanted.push((g.packages.len() - 1, deps));
    }
    for (from, deps) in wanted {
        link(g, "Maven", from, &deps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, &str)]) -> BTreeMap<String, String> {
        list.iter().map(|(p, t)| ((*p).to_owned(), (*t).to_owned())).collect()
    }

    fn edges(g: &BuildGraph) -> Vec<String> {
        g.deps
            .iter()
            .map(|d| {
                let k = if d.kind.is_empty() { String::new() } else { format!(" ({})", d.kind) };
                format!("{} -> {}{k}", g.packages[d.from].name, g.packages[d.to].name)
            })
            .collect()
    }

    #[test]
    fn cargo_and_npm() {
        let g = read(&files(&[
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
            ("crates/core/Cargo.toml", "[package]\nname = \"core\"\n[dependencies]\nserde = \"1\"\n"),
            (
                "crates/app/Cargo.toml",
                "[package]\nname = \"app\"\n[dependencies]\nkern = { package = \"core\", path = \"../core\" }\n[dev-dependencies]\ntesty = { path = \"../testy\" }\n",
            ),
            (
                "crates/testy/Cargo.toml",
                "[package]\nname = \"testy\"\n[target.'cfg(unix)'.build-dependencies]\ncore = { path = \"../core\" }\n",
            ),
            ("web/package.json", r#"{"name":"@me/web","dependencies":{"@me/ui":"workspace:*","react":"18"}}"#),
            ("ui/package.json", r#"{"name":"@me/ui","devDependencies":{"@me/web":"*"}}"#),
            ("node_modules/x/package.json", r#"{"name":"x"}"#),
        ]));
        assert_eq!(
            g.packages.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["app", "core", "testy", "@me/ui", "@me/web"]
        );
        assert_eq!(
            edges(&g),
            [
                "app -> core",
                "app -> testy (dev)",
                "testy -> core (build)",
                "@me/ui -> @me/web (dev)",
                "@me/web -> @me/ui"
            ]
        );
        assert!(!is_build_file("node_modules/x/package.json") && is_build_file("pom.xml"));
    }

    #[test]
    fn gradle_and_maven() {
        let g = read(&files(&[
            (
                "settings.gradle.kts",
                "rootProject.name = \"shop\"\ninclude(\":app\", \":lib:core\")\ninclude ':lib:util'\n",
            ),
            (
                "app/build.gradle.kts",
                "dependencies {\n  implementation(project(\":lib:core\"))\n  testImplementation(projects.lib.util)\n}\n",
            ),
            ("lib/core/build.gradle", "dependencies { implementation project(':lib:util') }\n"),
            ("lib/util/build.gradle", ""),
            (
                "pom.xml",
                "<project><artifactId>parent</artifactId><modules><module>api</module></modules>\
                 <dependencyManagement><dependencies><dependency><artifactId>api</artifactId></dependency></dependencies></dependencyManagement></project>",
            ),
            (
                "api/pom.xml",
                "<project><parent><artifactId>parent</artifactId></parent><artifactId>api</artifactId>\
                 <dependencies><dependency><artifactId>model</artifactId><scope>test</scope></dependency>\
                 <dependency><artifactId>junit</artifactId></dependency></dependencies></project>",
            ),
            ("model/pom.xml", "<project><artifactId>model</artifactId></project>"),
        ]));
        let names: Vec<(&str, &str)> = g.packages.iter().map(|p| (p.tool, p.name.as_str())).collect();
        assert_eq!(
            names,
            [
                ("Gradle", "app"),
                ("Gradle", "lib:core"),
                ("Gradle", "lib:util"),
                ("Maven", "api"),
                ("Maven", "model"),
                ("Maven", "parent")
            ]
        );
        assert_eq!(g.packages[0].manifest, "app/build.gradle.kts");
        assert_eq!(edges(&g), ["app -> lib:core", "app -> lib:util", "lib:core -> lib:util", "api -> model (dev)"]);
    }
}
