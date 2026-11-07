//! Maven without a UI: projects found from `pom.xml` (and their modules), the lifecycle
//! and plugin goals they offer, dependencies resolved from the local repository (as
//! Maven would: nearest wins, scopes, exclusions, managed versions, parents and BOMs),
//! `dependency:tree` and `help:describe` output, Maven Central search, and adding a
//! dependency to a pom. Maven itself runs elsewhere; this reads and writes text.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::Event;
use serde_json::Value;

use crate::lsp::servers::which;

/// Maven's default lifecycle phases, with `clean` and `site`, in the order the VS Code
/// Maven extension lists them.
pub const LIFECYCLE: &[&str] =
    &["clean", "validate", "compile", "test", "package", "verify", "install", "site", "deploy"];

/// A dependency as a pom declares it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dependency {
    pub group_id: String,
    pub artifact_id: String,
    pub version: Option<String>,
    pub scope: Option<String>,
    pub kind: Option<String>,
    pub classifier: Option<String>,
    pub optional: bool,
    /// `groupId:artifactId` pairs left out below this one (`*` matches any).
    pub exclusions: Vec<(String, String)>,
}

impl Dependency {
    /// What Maven tells dependencies apart by: group, then artifact with its classifier.
    pub fn key(&self) -> (String, String) {
        match self.classifier.as_deref().filter(|c| !c.is_empty()) {
            Some(c) => (self.group_id.clone(), format!("{}:{c}", self.artifact_id)),
            None => (self.group_id.clone(), self.artifact_id.clone()),
        }
    }
}

/// A build plugin a pom names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plugin {
    pub group_id: String,
    pub artifact_id: String,
    pub version: Option<String>,
}

impl Plugin {
    /// The short name goals are run by: `compiler` for `maven-compiler-plugin`, `exec`
    /// for `exec-maven-plugin`.
    pub fn prefix(&self) -> String {
        let a = self.artifact_id.as_str();
        a.strip_prefix("maven-")
            .and_then(|r| r.strip_suffix("-plugin"))
            .or_else(|| a.strip_suffix("-maven-plugin"))
            .unwrap_or(a)
            .to_owned()
    }

    /// Goals known without asking Maven; empty when Maven has to say (`help:describe`).
    pub fn known_goals(&self) -> &'static [&'static str] {
        match self.artifact_id.as_str() {
            "maven-clean-plugin" => &["clean"],
            "maven-resources-plugin" => &["resources", "testResources", "copy-resources"],
            "maven-compiler-plugin" => &["compile", "testCompile"],
            "maven-surefire-plugin" => &["test"],
            "maven-failsafe-plugin" => &["integration-test", "verify"],
            "maven-jar-plugin" => &["jar", "test-jar"],
            "maven-war-plugin" => &["war", "exploded"],
            "maven-install-plugin" => &["install", "install-file"],
            "maven-deploy-plugin" => &["deploy", "deploy-file"],
            "maven-site-plugin" => &["site", "run", "stage", "deploy"],
            "maven-dependency-plugin" => {
                &["tree", "resolve", "analyze", "copy-dependencies", "go-offline", "purge-local-repository"]
            }
            "maven-javadoc-plugin" => &["javadoc", "jar", "test-javadoc"],
            "maven-source-plugin" => &["jar", "test-jar"],
            "maven-shade-plugin" => &["shade"],
            "maven-assembly-plugin" => &["single"],
            "maven-antrun-plugin" => &["run"],
            "maven-enforcer-plugin" => &["enforce", "display-info"],
            "maven-help-plugin" => &["effective-pom", "describe", "active-profiles", "effective-settings"],
            "exec-maven-plugin" => &["java", "exec"],
            "versions-maven-plugin" => &["display-dependency-updates", "display-plugin-updates", "use-latest-releases"],
            "jacoco-maven-plugin" => &["prepare-agent", "report", "check"],
            "spring-boot-maven-plugin" => &["run", "repackage", "build-image", "start", "stop"],
            "javafx-maven-plugin" => &["run", "jlink"],
            "quarkus-maven-plugin" => &["dev", "build", "test"],
            "maven-checkstyle-plugin" => &["check", "checkstyle"],
            "spotbugs-maven-plugin" => &["check", "spotbugs", "gui"],
            _ => &[],
        }
    }
}

/// What Maven binds to every jar project without it being named: shown with the pom's own.
pub fn default_plugins(packaging: &str) -> Vec<Plugin> {
    let mut names =
        vec!["maven-clean-plugin", "maven-resources-plugin", "maven-compiler-plugin", "maven-surefire-plugin"];
    match packaging {
        "pom" => names = vec!["maven-clean-plugin"],
        "war" => names.push("maven-war-plugin"),
        _ => names.push("maven-jar-plugin"),
    }
    names.extend(["maven-install-plugin", "maven-deploy-plugin", "maven-site-plugin"]);
    names
        .into_iter()
        .map(|a| Plugin { group_id: "org.apache.maven.plugins".to_owned(), artifact_id: a.to_owned(), version: None })
        .collect()
}

/// A pom.xml, as written (nothing inherited or interpolated yet).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pom {
    pub group_id: Option<String>,
    pub artifact_id: String,
    pub version: Option<String>,
    pub packaging: Option<String>,
    pub name: Option<String>,
    pub parent: Option<Parent>,
    pub modules: Vec<String>,
    pub properties: BTreeMap<String, String>,
    pub dependencies: Vec<Dependency>,
    pub managed: Vec<Dependency>,
    pub plugins: Vec<Plugin>,
    pub profiles: Vec<Profile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parent {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub relative_path: Option<String>,
}

impl Pom {
    pub fn group(&self) -> &str {
        self.group_id.as_deref().or(self.parent.as_ref().map(|p| p.group_id.as_str())).unwrap_or("")
    }

    pub fn version_text(&self) -> &str {
        self.version.as_deref().or(self.parent.as_ref().map(|p| p.version.as_str())).unwrap_or("")
    }

    pub fn packaging(&self) -> &str {
        self.packaging.as_deref().unwrap_or("jar")
    }

    /// `group:artifact:version`.
    pub fn coordinates(&self) -> String {
        format!("{}:{}:{}", self.group(), self.artifact_id, self.version_text())
    }
}

/// A profile: when it applies, and what it adds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub active_by_default: bool,
    /// `name`, `family`, `arch` or `version`, with the value asked for (`!` negates).
    pub os: Vec<(String, String)>,
    /// A system property's name, and the value asked for.
    pub property: Option<(String, Option<String>)>,
    pub jdk: Option<String>,
    /// `exists` or `missing`, and the file.
    pub file: Option<(String, String)>,
    /// Its properties, dependencies and managed versions.
    pub body: Box<Pom>,
}

/// Reads a pom. Broken XML gives what was read before the break.
pub fn parse_pom(text: &str) -> Pom {
    let mut reader = Reader::from_str(text);
    let mut pom = Pom::default();
    let mut path: Vec<String> = Vec::new();
    let mut text_now = String::new();
    let mut parts = Parts::default();
    let mut profile = Profile::default();
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                path.push(String::from_utf8_lossy(tag.local_name().as_ref()).into_owned());
                text_now.clear();
                let (_, logical) = logical_path(&path);
                match logical.as_str() {
                    "project/dependencies/dependency" | "project/dependencyManagement/dependencies/dependency" => {
                        parts.dep = Dependency::default();
                    }
                    "project/build/plugins/plugin" => parts.plugin = Plugin::default(),
                    _ => {}
                }
                if joined(&path) == "project/profiles/profile" {
                    profile = Profile::default();
                }
            }
            Ok(Event::Text(t)) => {
                if let Ok(t) = t.decode() {
                    text_now.push_str(&quick_xml::escape::unescape(&t).map(|c| c.into_owned()).unwrap_or_default());
                }
            }
            Ok(Event::GeneralRef(entity)) => {
                if let Ok(name) = entity.decode() {
                    text_now.push_str(quick_xml::escape::resolve_predefined_entity(&name).unwrap_or(""));
                }
            }
            Ok(Event::CData(t)) => text_now.push_str(&String::from_utf8_lossy(&t)),
            Ok(Event::End(_)) => {
                let value = text_now.trim().to_owned();
                let actual = joined(&path);
                let leaf = path.last().cloned().unwrap_or_default();
                let actual_owner = path.len().checked_sub(1).map(|n| joined(&path[..n])).unwrap_or_default();
                let (in_profile, logical) = logical_path(&path);
                if actual == "project/profiles/profile" {
                    pom.profiles.push(std::mem::take(&mut profile));
                } else if in_profile && actual_owner == "project/profiles/profile" && leaf == "id" {
                    profile.id = value;
                } else if in_profile && actual_owner.starts_with("project/profiles/profile/activation") {
                    match (actual_owner.trim_start_matches("project/profiles/profile/activation"), leaf.as_str()) {
                        ("", "activeByDefault") => profile.active_by_default = value == "true",
                        ("", "jdk") => profile.jdk = Some(value),
                        ("/os", key) => profile.os.push((key.to_owned(), value)),
                        ("/property", "name") => {
                            profile.property = Some((value, profile.property.take().and_then(|p| p.1)));
                        }
                        ("/property", "value") => {
                            let name = profile.property.take().map(|p| p.0).unwrap_or_default();
                            profile.property = Some((name, Some(value)));
                        }
                        ("/file", key @ ("exists" | "missing")) => profile.file = Some((key.to_owned(), value)),
                        _ => {}
                    }
                } else {
                    let target = if in_profile { &mut *profile.body } else { &mut pom };
                    let owner = logical.rsplit_once('/').map(|(o, _)| o.to_owned()).unwrap_or_default();
                    parts.apply(target, &owner, &logical, &leaf, value);
                }
                path.pop();
                text_now.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    pom
}

/// The path as if a profile's contents were the project's: (inside a profile, path).
fn logical_path(path: &[String]) -> (bool, String) {
    if path.len() > 3 && path[0] == "project" && path[1] == "profiles" && path[2] == "profile" {
        (true, format!("project/{}", joined(&path[3..])))
    } else {
        (false, joined(path))
    }
}

/// What's being read inside a pom's elements.
#[derive(Default)]
struct Parts {
    dep: Dependency,
    exclusion: (String, String),
    plugin: Plugin,
    parent: Parent,
}

impl Parts {
    fn apply(&mut self, pom: &mut Pom, owner: &str, here: &str, leaf: &str, value: String) {
        match owner {
            "project" => match leaf {
                "groupId" => pom.group_id = Some(value),
                "artifactId" => pom.artifact_id = value,
                "version" => pom.version = Some(value),
                "packaging" => pom.packaging = Some(value),
                "name" => pom.name = Some(value),
                "parent" => pom.parent = Some(std::mem::take(&mut self.parent)),
                _ => {}
            },
            "project/parent" => match leaf {
                "groupId" => self.parent.group_id = value,
                "artifactId" => self.parent.artifact_id = value,
                "version" => self.parent.version = value,
                "relativePath" => self.parent.relative_path = Some(value),
                _ => {}
            },
            "project/modules" if leaf == "module" => pom.modules.push(value),
            "project/properties" => {
                pom.properties.insert(leaf.to_owned(), value);
            }
            "project/dependencies/dependency" | "project/dependencyManagement/dependencies/dependency" => match leaf {
                "groupId" => self.dep.group_id = value,
                "artifactId" => self.dep.artifact_id = value,
                "version" => self.dep.version = Some(value),
                "scope" => self.dep.scope = Some(value),
                "type" => self.dep.kind = Some(value),
                "classifier" => self.dep.classifier = Some(value),
                "optional" => self.dep.optional = value == "true",
                _ => {}
            },
            "project/dependencies/dependency/exclusions/exclusion"
            | "project/dependencyManagement/dependencies/dependency/exclusions/exclusion" => match leaf {
                "groupId" => self.exclusion.0 = value,
                "artifactId" => self.exclusion.1 = value,
                _ => {}
            },
            "project/build/plugins/plugin" => match leaf {
                "groupId" => self.plugin.group_id = value,
                "artifactId" => self.plugin.artifact_id = value,
                "version" => self.plugin.version = Some(value),
                _ => {}
            },
            _ => {}
        }
        match here {
            "project/dependencies/dependency" => pom.dependencies.push(std::mem::take(&mut self.dep)),
            "project/dependencyManagement/dependencies/dependency" => pom.managed.push(std::mem::take(&mut self.dep)),
            "project/dependencies/dependency/exclusions/exclusion"
            | "project/dependencyManagement/dependencies/dependency/exclusions/exclusion" => {
                self.dep.exclusions.push(std::mem::take(&mut self.exclusion));
            }
            "project/build/plugins/plugin" => {
                let mut p = std::mem::take(&mut self.plugin);
                if p.group_id.is_empty() {
                    p.group_id = "org.apache.maven.plugins".to_owned();
                }
                pom.plugins.push(p);
            }
            _ => {}
        }
    }
}

/// What a profile's activation is checked against.
#[derive(Clone, Debug)]
pub struct Environment {
    /// Java's `os.name` and `os.arch`, lower case.
    pub os_name: String,
    pub os_arch: String,
    pub family: &'static str,
    /// Java's version (`21.0.12`), when known.
    pub jdk: Option<String>,
    /// System properties given (`-D`): none for what's read here.
    pub properties: HashMap<String, String>,
}

impl Environment {
    /// This computer, as Java names it.
    pub fn here(jdk: Option<String>) -> Self {
        let os_name = if cfg!(windows) {
            "windows 11".to_owned()
        } else if cfg!(target_os = "macos") {
            "mac os x".to_owned()
        } else {
            std::env::consts::OS.to_owned()
        };
        let os_arch = match std::env::consts::ARCH {
            "x86_64" => "amd64",
            other => other,
        }
        .to_owned();
        let family = if cfg!(windows) {
            "windows"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "unix"
        };
        Self { os_name, os_arch, family, jdk, properties: HashMap::new() }
    }
}

/// The version of `java` on PATH (`21.0.12`), from `java -version`.
pub fn java_version() -> Option<String> {
    let java = which("java")?;
    let out =
        crate::lsp::discovery::quiet_command(&java).arg("-version").stdin(std::process::Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let start = text.find('"')? + 1;
    let end = start + text[start..].find('"')?;
    Some(text[start..end].to_owned())
}

/// `actual` against what's asked, `!` meaning "not".
fn matches(asked: &str, actual: &str) -> bool {
    match asked.strip_prefix('!') {
        Some(rest) => !rest.eq_ignore_ascii_case(actual),
        None => asked.eq_ignore_ascii_case(actual),
    }
}

/// Whether a Java version matches `asked`: a prefix (`17`, `!1.8`) or a range (`[17,)`).
fn jdk_matches(asked: &str, version: &str) -> bool {
    let asked = asked.trim();
    let number = |v: &str| -> Vec<u32> {
        let v = v.trim();
        let v = v.strip_prefix("1.").filter(|_| !v.starts_with("1.0")).unwrap_or(v);
        v.split(['.', '_', '-', '+']).map_while(|p| p.parse().ok()).collect()
    };
    if let Some(rest) = asked.strip_prefix('!') {
        return !jdk_matches(rest, version);
    }
    if (asked.starts_with('[') || asked.starts_with('(')) && asked.len() >= 2 {
        let Some((low, high)) = asked[1..asked.len() - 1].split_once(',') else { return false };
        let v = number(version);
        let lo_ok = low.trim().is_empty() || if asked.starts_with('[') { v >= number(low) } else { v > number(low) };
        let hi_ok = high.trim().is_empty() || if asked.ends_with(']') { v <= number(high) } else { v < number(high) };
        return lo_ok && hi_ok;
    }
    let (a, v) = (number(asked), number(version));
    !a.is_empty() && v.starts_with(&a)
}

impl Profile {
    /// Whether its activation holds here: every condition given, as from Maven 3.2.2.
    /// A profile with no conditions is only on by default.
    pub fn applies(&self, env: &Environment, dir: Option<&Path>) -> bool {
        let mut any = false;
        for (key, asked) in &self.os {
            any = true;
            let ok = match key.as_str() {
                "name" => matches(asked, &env.os_name),
                "family" => matches(asked, env.family) || (env.family == "mac" && matches(asked, "unix")),
                "arch" => matches(asked, &env.os_arch),
                _ => true,
            };
            if !ok {
                return false;
            }
        }
        if let Some((name, value)) = &self.property {
            any = true;
            let (negated, name) = match name.strip_prefix('!') {
                Some(n) => (true, n),
                None => (false, name.as_str()),
            };
            let set = env.properties.get(name);
            let ok = match (negated, value, set) {
                (true, _, s) => s.is_none(),
                (false, None, s) => s.is_some(),
                (false, Some(v), Some(s)) => matches(v, s),
                // Unset: `!value` holds, a value doesn't.
                (false, Some(v), None) => v.starts_with('!'),
            };
            if !ok {
                return false;
            }
        }
        if let Some(asked) = &self.jdk {
            any = true;
            if !env.jdk.as_deref().is_some_and(|v| jdk_matches(asked, v)) {
                return false;
            }
        }
        if let Some((kind, file)) = &self.file {
            any = true;
            let file = file.replace("${basedir}/", "").replace("${project.basedir}/", "");
            let exists = dir.is_some_and(|d| d.join(&file).exists());
            if (kind == "exists") != exists {
                return false;
            }
        }
        any
    }
}

/// The profiles of a pom that apply: those whose conditions hold, else those on by
/// default.
pub fn active_profiles<'a>(pom: &'a Pom, env: &Environment, dir: Option<&Path>) -> Vec<&'a Profile> {
    let on: Vec<&Profile> = pom.profiles.iter().filter(|p| p.applies(env, dir)).collect();
    if on.is_empty() { pom.profiles.iter().filter(|p| p.active_by_default).collect() } else { on }
}

fn joined(path: &[String]) -> String {
    path.join("/")
}

/// `${name}` replaced by its value, a few times over for values made of others.
pub fn interpolate(value: &str, props: &HashMap<String, String>) -> String {
    let mut out = value.to_owned();
    for _ in 0..6 {
        if !out.contains("${") {
            break;
        }
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(start) = rest.find("${") {
            next.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                next.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let name = &after[..end];
            match props.get(name) {
                Some(v) => next.push_str(v),
                None => next.push_str(&rest[start..start + 3 + end]),
            }
            rest = &after[end + 1..];
        }
        next.push_str(rest);
        if next == out {
            break;
        }
        out = next;
    }
    out
}

// ----- projects in a folder ------------------------------------------------------------

/// A Maven project in the workspace: its pom, and its modules' projects.
#[derive(Clone, Debug)]
pub struct Project {
    pub pom_path: PathBuf,
    pub pom: Pom,
    pub modules: Vec<Project>,
}

impl Project {
    pub fn dir(&self) -> PathBuf {
        self.pom_path.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    /// The name shown: the pom's `<name>`, else its artifactId.
    pub fn label(&self) -> String {
        self.pom
            .name
            .clone()
            .filter(|n| !n.is_empty() && !n.contains("${"))
            .unwrap_or_else(|| self.pom.artifact_id.clone())
    }

    /// Plugins: Maven's defaults for the packaging, then the pom's own (replacing a
    /// default of the same name).
    pub fn plugins(&self) -> Vec<Plugin> {
        let mut out = default_plugins(self.pom.packaging());
        for p in &self.pom.plugins {
            match out.iter_mut().find(|o| o.artifact_id == p.artifact_id) {
                Some(o) => *o = p.clone(),
                None => out.push(p.clone()),
            }
        }
        out
    }
}

fn read_project(pom_path: &Path, depth: usize) -> Option<Project> {
    let text = std::fs::read_to_string(pom_path).ok()?;
    let pom = parse_pom(&text);
    if pom.artifact_id.is_empty() {
        return None;
    }
    let dir = pom_path.parent()?.to_path_buf();
    let modules = if depth < 6 {
        pom.modules
            .iter()
            .filter_map(|m| {
                let target = dir.join(m);
                let file = if target.is_dir() { target.join("pom.xml") } else { target };
                read_project(&file, depth + 1)
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Project { pom_path: pom_path.to_path_buf(), pom, modules })
}

/// The Maven projects under `root`: its own pom with its modules, else poms found a few
/// folders down (several projects side by side). Build output and hidden folders are
/// skipped.
pub fn find_projects(root: &Path) -> Vec<Project> {
    if let Some(p) = read_project(&root.join("pom.xml"), 0) {
        return vec![p];
    }
    let mut out = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let walk = ignore::WalkBuilder::new(root).max_depth(Some(3)).build();
    for entry in walk.filter_map(Result::ok) {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == "pom.xml")
            && !path.components().any(|c| matches!(c.as_os_str().to_str(), Some("target" | "node_modules")))
            && !seen.iter().any(|s| path.starts_with(s))
            && let Some(p) = read_project(path, 0)
        {
            seen.insert(p.dir());
            out.push(p);
        }
        if out.len() >= 50 {
            break;
        }
    }
    out.sort_by_key(|p| p.pom_path.clone());
    out
}

/// Every project, modules included, in tree order.
pub fn all_projects(projects: &[Project]) -> Vec<&Project> {
    let mut out = Vec::new();
    for p in projects {
        out.push(p);
        out.extend(all_projects(&p.modules));
    }
    out
}

// ----- running Maven -------------------------------------------------------------------

/// How Maven is started for a project: its wrapper (`mvnw`) when it or a parent folder
/// has one, else `mvn` from PATH, else the Maven Rust Code fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mvn {
    pub program: PathBuf,
    pub wrapper: bool,
}

pub fn wrapper_name() -> &'static str {
    if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }
}

pub fn find_mvn(dir: &Path, root: Option<&Path>) -> Option<Mvn> {
    let mut here = Some(dir);
    while let Some(d) = here {
        let w = d.join(wrapper_name());
        if w.is_file() {
            return Some(Mvn { program: w, wrapper: true });
        }
        if root.is_some_and(|r| d == r) {
            break;
        }
        here = d.parent();
    }
    which("mvn").or_else(crate::build_tools::maven_program).map(|program| Mvn { program, wrapper: false })
}

impl Mvn {
    /// The command line for a terminal: PowerShell on Windows, sh elsewhere.
    pub fn command(&self, args: &str) -> String {
        let program = self.program.display().to_string();
        let name = self.program.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let call = if !self.wrapper && which(&name).as_deref() == Some(self.program.as_path()) {
            "mvn".to_owned()
        } else if cfg!(windows) {
            format!("& '{}'", program.replace('\'', "''"))
        } else {
            format!("'{}'", program.replace('\'', r"'\''"))
        };
        if args.is_empty() { call } else { format!("{call} {args}") }
    }
}

/// A goal line typed by someone, made safe to put after `mvn`: one line, no shell
/// operators that would start a second command.
pub fn clean_goal(text: &str) -> Result<String, String> {
    let text = text.trim();
    let text = text.strip_prefix("mvn ").unwrap_or(text).trim();
    if text.is_empty() {
        return Err("No goal given".to_owned());
    }
    if text.chars().any(|c| matches!(c, ';' | '&' | '|' | '`' | '\n' | '\r' | '<' | '>')) || text.contains("$(") {
        return Err("A goal line can't hold ; & | ` < > or $( )".to_owned());
    }
    Ok(text.to_owned())
}

/// The arguments for `help:describe` of a plugin, to list its goals. Not `-q`: the
/// description is what Maven logs.
pub fn describe_args(plugin: &Plugin) -> Vec<String> {
    vec!["-B".to_owned(), "help:describe".to_owned(), format!("-Dplugin={}:{}", plugin.group_id, plugin.artifact_id)]
}

/// Goals from `help:describe` output (`exec:exec`, `exec:java` ...), as their names.
pub fn parse_describe(output: &str) -> Vec<String> {
    let mut out = Vec::new();
    // The goals come after "This plugin has N goals:".
    let goals = output.find("This plugin has").map_or("", |i| &output[i..]);
    for line in goals.lines().skip(1) {
        let line = line.trim().trim_start_matches("[INFO]").trim();
        if let Some((prefix, goal)) = line.split_once(':')
            && !prefix.is_empty()
            && !goal.is_empty()
            && !line.contains(' ')
            && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
            && goal.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !out.iter().any(|g| g == goal)
        {
            out.push(goal.to_owned());
        }
    }
    out
}

// ----- dependencies --------------------------------------------------------------------

/// Where Maven keeps what it downloaded: `<localRepository>` in `~/.m2/settings.xml`,
/// else `~/.m2/repository`.
pub fn local_repository() -> Option<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
    let m2 = home.join(".m2");
    if let Ok(text) = std::fs::read_to_string(m2.join("settings.xml"))
        && let Some(start) = text.find("<localRepository>")
        && let Some(end) = text[start..].find("</localRepository>")
    {
        let value = text[start + 17..start + end].trim();
        let mut props = HashMap::new();
        props.insert("user.home".to_owned(), home.display().to_string());
        let value = interpolate(value, &props);
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    Some(m2.join("repository"))
}

/// The file of an artifact in a local repository.
pub fn artifact_path(repo: &Path, group: &str, artifact: &str, version: &str, ext: &str) -> PathBuf {
    let mut p = repo.to_path_buf();
    for part in group.split('.') {
        p.push(part);
    }
    p.push(artifact);
    p.push(version);
    p.push(format!("{artifact}-{version}.{ext}"));
    p
}

/// What became of a dependency when resolving.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepState {
    /// Used, and its pom is in the local repository.
    Resolved,
    /// Used, but its pom isn't downloaded yet (Maven fetches it on the next build).
    NotDownloaded,
    /// Left out: another version is nearer the project.
    Omitted { used: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepNode {
    pub group_id: String,
    pub artifact_id: String,
    /// A variant of the artifact (`win`, `natives-linux`), when it's one.
    pub classifier: Option<String>,
    pub version: String,
    pub scope: String,
    pub optional: bool,
    pub state: DepState,
    pub children: Vec<DepNode>,
}

impl DepNode {
    pub fn coordinates(&self) -> String {
        match &self.classifier {
            Some(c) => format!("{}:{}:{c}:{}", self.group_id, self.artifact_id, self.version),
            None => format!("{}:{}:{}", self.group_id, self.artifact_id, self.version),
        }
    }
}

/// A pom with what it inherits: properties, managed versions and dependencies from its
/// parents, `${...}` filled in, and BOMs (`<scope>import</scope>`) expanded.
#[derive(Clone, Debug, Default)]
struct Model {
    dependencies: Vec<Dependency>,
    managed: HashMap<(String, String), Dependency>,
}

type Props = HashMap<String, String>;

/// Reads and caches models from the local repository and the workspace.
pub struct Resolver {
    repo: PathBuf,
    env: Environment,
    cache: HashMap<String, Option<(Model, Props)>>,
}

impl Resolver {
    /// For this computer: its OS, and the version of the `java` on PATH.
    pub fn new(repo: PathBuf) -> Self {
        Self::with_env(repo, Environment::here(java_version()))
    }

    pub fn with_env(repo: PathBuf, env: Environment) -> Self {
        Self { repo, env, cache: HashMap::new() }
    }

    fn repo_pom(&self, g: &str, a: &str, v: &str) -> Option<(PathBuf, Pom)> {
        let path = artifact_path(&self.repo, g, a, v, "pom");
        let text = std::fs::read_to_string(&path).ok()?;
        Some((path, parse_pom(&text)))
    }

    /// The parent of a pom: from `relativePath` (default `../pom.xml`) when that file is
    /// the parent named, else from the local repository.
    fn parent_of(&self, pom_path: Option<&Path>, parent: &Parent) -> Option<(PathBuf, Pom)> {
        let local = pom_path.and_then(|p| {
            let dir = p.parent()?;
            let rel = parent.relative_path.clone().unwrap_or_else(|| "../pom.xml".to_owned());
            if rel.is_empty() {
                return None;
            }
            let target = dir.join(rel);
            let file = if target.is_dir() { target.join("pom.xml") } else { target };
            let found = parse_pom(&std::fs::read_to_string(&file).ok()?);
            (found.artifact_id == parent.artifact_id && found.group() == parent.group_id).then_some((file, found))
        });
        local.or_else(|| self.repo_pom(&parent.group_id, &parent.artifact_id, &parent.version))
    }

    /// The model of a pom, with the properties it ends with: its parents' first.
    fn full(&mut self, pom_path: Option<&Path>, pom: &Pom, depth: usize) -> (Model, Props) {
        let mut model = Model::default();
        let mut props = Props::new();
        if let Some(parent) = &pom.parent
            && depth < 12
            && let Some((file, ppom)) = self.parent_of(pom_path, parent)
        {
            (model, props) = self.cached(&ppom, Some(&file), depth + 1);
            props.insert("project.parent.version".to_owned(), parent.version.clone());
            props.insert("project.parent.groupId".to_owned(), parent.group_id.clone());
        }
        let (own, props) = self.own(pom, pom_path.and_then(Path::parent), props, depth);
        for (k, d) in own.managed {
            model.managed.insert(k, d);
        }
        for d in own.dependencies {
            model.dependencies.retain(|o| o.key() != d.key());
            model.dependencies.push(d);
        }
        (model, props)
    }

    /// `full`, kept by coordinates: parents and BOMs are shared by many.
    fn cached(&mut self, pom: &Pom, path: Option<&Path>, depth: usize) -> (Model, Props) {
        let key = pom.coordinates();
        if let Some(Some(found)) = self.cache.get(&key) {
            return found.clone();
        }
        let found = self.full(path, pom, depth);
        self.cache.insert(key, Some(found.clone()));
        found
    }

    fn model_of(&mut self, pom_path: Option<&Path>, pom: &Pom, depth: usize) -> Model {
        self.full(pom_path, pom, depth).0
    }

    /// This pom's own managed versions and dependencies, interpolated with `props` plus
    /// its own properties. Returns the properties after adding its own.
    fn own(&mut self, pom: &Pom, dir: Option<&Path>, mut props: Props, depth: usize) -> (Model, Props) {
        for (k, v) in &pom.properties {
            props.insert(k.clone(), v.clone());
        }
        // Profiles that apply add their properties, managed versions and dependencies.
        let profiles: Vec<Pom> = active_profiles(pom, &self.env, dir).into_iter().map(|p| (*p.body).clone()).collect();
        for body in &profiles {
            for (k, v) in &body.properties {
                props.insert(k.clone(), v.clone());
            }
        }
        let group = pom.group().to_owned();
        let version = pom.version_text().to_owned();
        for (k, v) in [
            ("project.groupId", group.clone()),
            ("pom.groupId", group),
            ("project.artifactId", pom.artifact_id.clone()),
            ("project.version", version.clone()),
            ("pom.version", version),
        ] {
            props.insert(k.to_owned(), v);
        }
        let fill = |d: &Dependency, props: &HashMap<String, String>| {
            let mut d = d.clone();
            d.group_id = interpolate(&d.group_id, props);
            d.artifact_id = interpolate(&d.artifact_id, props);
            d.version = d.version.as_deref().map(|v| interpolate(v, props));
            d.scope = d.scope.as_deref().map(|v| interpolate(v, props));
            d.classifier = d.classifier.as_deref().map(|v| interpolate(v, props));
            d
        };
        let mut model = Model::default();
        let managed = pom.managed.iter().chain(profiles.iter().flat_map(|b| b.managed.iter()));
        for m in managed {
            let m = fill(m, &props);
            if m.scope.as_deref() == Some("import") && m.kind.as_deref() == Some("pom") {
                if let Some(v) = m.version.clone()
                    && depth < 12
                    && let Some(bom) = self.load(&m.group_id, &m.artifact_id, &v, depth + 1)
                {
                    for (k, d) in bom.managed {
                        model.managed.entry(k).or_insert(d);
                    }
                }
                continue;
            }
            model.managed.insert(m.key(), m);
        }
        model.dependencies = pom
            .dependencies
            .iter()
            .chain(profiles.iter().flat_map(|b| b.dependencies.iter()))
            .map(|d| fill(d, &props))
            .collect();
        (model, props)
    }

    /// The model of `g:a:v` from the local repository; `None` when its pom isn't there.
    fn load(&mut self, g: &str, a: &str, v: &str, depth: usize) -> Option<Model> {
        let key = format!("{g}:{a}:{v}");
        if let Some(found) = self.cache.get(&key) {
            return found.as_ref().map(|f| f.0.clone());
        }
        // Marks it as being read, so a cycle ends here.
        self.cache.insert(key.clone(), None);
        let found = self.repo_pom(g, a, v).map(|(path, pom)| self.full(Some(&path), &pom, depth));
        let model = found.as_ref().map(|f| f.0.clone());
        self.cache.insert(key, found);
        model
    }

    /// The dependency tree of a workspace project, as Maven would resolve it.
    pub fn resolve(&mut self, project: &Project) -> Vec<DepNode> {
        let root = self.model_of(Some(&project.pom_path), &project.pom, 0);
        struct Pending {
            /// Where in the tree: indexes from the top.
            at: Vec<usize>,
            dep: Dependency,
            scope: String,
            exclusions: Vec<(String, String)>,
            depth: usize,
        }
        let mut tree: Vec<DepNode> = Vec::new();
        let mut used: HashMap<(String, String), String> = HashMap::new();
        let mut queue: VecDeque<Pending> = VecDeque::new();
        for d in &root.dependencies {
            let scope = d.scope.clone().unwrap_or_else(|| "compile".to_owned());
            queue.push_back(Pending {
                at: Vec::new(),
                dep: d.clone(),
                scope,
                exclusions: d.exclusions.clone(),
                depth: 0,
            });
        }
        let mut count = 0;
        while let Some(p) = queue.pop_front() {
            count += 1;
            if count > 3000 {
                break;
            }
            let key = p.dep.key();
            // The project's own management wins, even for what comes from further down.
            let managed = root.managed.get(&key);
            let version = if p.depth > 0 {
                managed.and_then(|m| m.version.clone()).or_else(|| p.dep.version.clone())
            } else {
                p.dep.version.clone().or_else(|| managed.and_then(|m| m.version.clone()))
            }
            .unwrap_or_default();
            let scope = if p.depth == 0 && p.dep.scope.is_none() {
                managed.and_then(|m| m.scope.clone()).unwrap_or(p.scope.clone())
            } else {
                p.scope.clone()
            };
            let mut node = DepNode {
                group_id: p.dep.group_id.clone(),
                artifact_id: p.dep.artifact_id.clone(),
                classifier: p.dep.classifier.clone().filter(|c| !c.is_empty()),
                version: version.clone(),
                scope: scope.clone(),
                optional: p.dep.optional,
                state: DepState::Resolved,
                children: Vec::new(),
            };
            let mut children = Vec::new();
            if let Some(v) = used.get(&key) {
                // The same version again isn't shown, as in Maven's tree; another is.
                if *v == version {
                    continue;
                }
                node.state = DepState::Omitted { used: v.clone() };
            } else {
                used.insert(key.clone(), version.clone());
                match self.load(&node.group_id, &node.artifact_id, &version, 1) {
                    None => node.state = DepState::NotDownloaded,
                    Some(model) => {
                        let mut managed_here = model.managed.clone();
                        for (k, d) in &root.managed {
                            managed_here.insert(k.clone(), d.clone());
                        }
                        for child in &model.dependencies {
                            let child_scope = child.scope.as_deref().unwrap_or("compile");
                            if child.optional || matches!(child_scope, "test" | "provided" | "system" | "import") {
                                continue;
                            }
                            if p.exclusions.iter().any(|(g, a)| {
                                (g == "*" || *g == child.group_id) && (a == "*" || *a == child.artifact_id)
                            }) {
                                continue;
                            }
                            let mut child = child.clone();
                            if child.version.is_none() {
                                child.version = managed_here.get(&child.key()).and_then(|m| m.version.clone());
                            }
                            // Scopes as Maven's table has them.
                            let scope = match (scope.as_str(), child_scope) {
                                ("compile", s) => s.to_owned(),
                                ("provided", _) => "provided".to_owned(),
                                ("runtime", _) => "runtime".to_owned(),
                                ("test", _) => "test".to_owned(),
                                (s, _) => s.to_owned(),
                            };
                            let mut exclusions = p.exclusions.clone();
                            exclusions.extend(child.exclusions.iter().cloned());
                            children.push((child, scope, exclusions));
                        }
                    }
                }
            }
            // Put the node in its place, then queue its children under it.
            let siblings = place(&mut tree, &p.at);
            siblings.push(node);
            let index = siblings.len() - 1;
            if p.depth < 25 {
                for (dep, scope, exclusions) in children {
                    let mut at = p.at.clone();
                    at.push(index);
                    queue.push_back(Pending { at, dep, scope, exclusions, depth: p.depth + 1 });
                }
            }
        }
        tree
    }
}

fn place<'a>(tree: &'a mut Vec<DepNode>, at: &[usize]) -> &'a mut Vec<DepNode> {
    let mut here = tree;
    for &i in at {
        here = &mut here[i].children;
    }
    here
}

/// Counts of a tree: (all nodes, not downloaded, omitted).
pub fn tree_counts(nodes: &[DepNode]) -> (usize, usize, usize) {
    let mut out = (0, 0, 0);
    for n in nodes {
        out.0 += 1;
        match n.state {
            DepState::NotDownloaded => out.1 += 1,
            DepState::Omitted { .. } => out.2 += 1,
            DepState::Resolved => {}
        }
        let c = tree_counts(&n.children);
        out = (out.0 + c.0, out.1 + c.1, out.2 + c.2);
    }
    out
}

/// The arguments for Maven's own dependency tree, written to `file`.
pub fn tree_args(file: &Path) -> Vec<String> {
    vec![
        "dependency:tree".to_owned(),
        "-DoutputType=text".to_owned(),
        format!("-DoutputFile={}", file.display()),
        "-DappendOutput=false".to_owned(),
    ]
}

/// Maven's `dependency:tree` text: the project's line, then one line per dependency,
/// drawn with `+- `, `\- ` and `|  `. Coordinates are `g:a:type[:classifier]:v:scope`.
pub fn parse_tree(text: &str) -> Vec<DepNode> {
    let mut roots: Vec<DepNode> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim_start_matches("[INFO] ").trim_end();
        let Some(pos) = line.find(|c: char| c.is_ascii_alphanumeric()) else { continue };
        let prefix = &line[..pos];
        if !prefix.ends_with("- ") {
            continue;
        }
        let depth = prefix.chars().count() / 3;
        let rest = &line[pos..];
        let (coords, note) = match rest.split_once(" (") {
            Some((c, n)) => (c, Some(n.trim_end_matches(')'))),
            None => (rest, None),
        };
        let parts: Vec<&str> = coords.split(':').collect();
        let (g, a, classifier, v, scope) = match parts.len() {
            5 => (parts[0], parts[1], None, parts[3], parts[4]),
            6 => (parts[0], parts[1], Some(parts[3]), parts[4], parts[5]),
            4 => (parts[0], parts[1], None, parts[3], ""),
            _ => continue,
        };
        let state = match note {
            Some(n) if n.starts_with("omitted for conflict with ") => {
                DepState::Omitted { used: n.trim_start_matches("omitted for conflict with ").to_owned() }
            }
            Some(n) if n.starts_with("omitted for duplicate") => DepState::Omitted { used: v.to_owned() },
            _ => DepState::Resolved,
        };
        let node = DepNode {
            group_id: g.to_owned(),
            artifact_id: a.to_owned(),
            classifier: classifier.map(str::to_owned),
            version: v.to_owned(),
            scope: scope.to_owned(),
            optional: false,
            state,
            children: Vec::new(),
        };
        stack.truncate(depth.saturating_sub(1));
        let siblings = place(&mut roots, &stack);
        siblings.push(node);
        stack.push(siblings.len() - 1);
    }
    roots
}

// ----- Maven Central -------------------------------------------------------------------

/// One artifact found on Maven Central.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
}

/// The search URL for some text (`junit`, `com.google.guava:guava`).
pub fn search_url(query: &str) -> String {
    let q = match query.trim().split_once(':') {
        Some((g, a)) if !g.is_empty() && !a.is_empty() => format!("g:{g} AND a:{a}"),
        _ => query.trim().to_owned(),
    };
    let encoded: String = q
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            b' ' => "+".to_owned(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("https://search.maven.org/solrsearch/select?q={encoded}&rows=20&wt=json")
}

pub fn parse_search(json: &str) -> Result<Vec<Found>, String> {
    let value: Value =
        serde_json::from_str(json).map_err(|_| "Maven Central gave an answer that isn't JSON".to_owned())?;
    let docs = value["response"]["docs"].as_array().ok_or("Maven Central's answer has no results list")?;
    Ok(docs
        .iter()
        .filter_map(|d| {
            Some(Found {
                group_id: d["g"].as_str()?.to_owned(),
                artifact_id: d["a"].as_str()?.to_owned(),
                version: d["latestVersion"].as_str().or(d["v"].as_str())?.to_owned(),
            })
        })
        .collect())
}

/// Searches Maven Central with the system's curl.
pub fn search_central(query: &str) -> Result<Vec<Found>, String> {
    let out = crate::lsp::discovery::quiet_command(&crate::lsp::servers::system_tool("curl"))
        .args(["-s", "--fail", "--max-time", "20", &search_url(query)])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("Maven Central didn't answer (no connection?)".to_owned());
    }
    parse_search(&String::from_utf8_lossy(&out.stdout))
}

// ----- editing a pom -------------------------------------------------------------------

/// The pom text with a dependency added to the project's `<dependencies>` (made when
/// there's none), indented as the file is. Returns the text and the line of the new
/// `<dependency>`. A dependency already there (same group and artifact) is an error.
pub fn add_dependency(text: &str, found: &Found, scope: Option<&str>) -> Result<(String, usize), String> {
    let pom = parse_pom(text);
    if pom.dependencies.iter().any(|d| d.group_id == found.group_id && d.artifact_id == found.artifact_id) {
        return Err(format!("{}:{} is already a dependency", found.group_id, found.artifact_id));
    }
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let unit = indent_unit(text);
    let block = |base: &str| {
        let i1 = format!("{base}{unit}");
        let i2 = format!("{i1}{unit}");
        let mut s = format!("{i1}<dependency>{nl}");
        s.push_str(&format!("{i2}<groupId>{}</groupId>{nl}", found.group_id));
        s.push_str(&format!("{i2}<artifactId>{}</artifactId>{nl}", found.artifact_id));
        s.push_str(&format!("{i2}<version>{}</version>{nl}", found.version));
        if let Some(scope) = scope.filter(|s| !s.is_empty() && *s != "compile") {
            s.push_str(&format!("{i2}<scope>{scope}</scope>{nl}"));
        }
        s.push_str(&format!("{i1}</dependency>{nl}"));
        s
    };
    // The project's own </dependencies>: at the project's level, not under
    // dependencyManagement, a profile or a plugin.
    if let Some(close) = top_level_close(text, "dependencies") {
        let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
        let base: String = text[line_start..close].chars().take_while(|c| c.is_whitespace()).collect();
        let at = if text[line_start..close].trim().is_empty() { line_start } else { close };
        let mut out = text[..at].to_owned();
        if at == close {
            out.push_str(nl);
        }
        let line = out.matches('\n').count();
        out.push_str(&block(&base));
        if at == close {
            out.push_str(&base);
        }
        out.push_str(&text[at..]);
        return Ok((out, line));
    }
    let close = text.rfind("</project>").ok_or("This pom has no </project>")?;
    let line_start = text[..close].rfind('\n').map_or(0, |i| i + 1);
    let at = if text[line_start..close].trim().is_empty() { line_start } else { close };
    let mut out = text[..at].to_owned();
    if !out.ends_with('\n') {
        out.push_str(nl);
    }
    out.push_str(nl);
    out.push_str(&format!("{unit}<dependencies>{nl}"));
    let line = out.matches('\n').count();
    out.push_str(&block(&unit));
    out.push_str(&format!("{unit}</dependencies>{nl}"));
    out.push_str(&text[at..]);
    Ok((out, line))
}

/// Where `</name>` closes at the project's own level (depth 1 under `<project>`).
fn top_level_close(text: &str, name: &str) -> Option<usize> {
    let mut reader = Reader::from_str(text);
    let mut depth = 0usize;
    let mut skip_depth: Option<usize> = None;
    loop {
        let before = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(tag)) => {
                depth += 1;
                let n = tag.local_name();
                if depth == 2 && skip_depth.is_none() && n.as_ref() != name.as_bytes() {
                    skip_depth = Some(depth);
                }
            }
            Ok(Event::End(tag)) => {
                if depth == 2 && skip_depth.is_none() && tag.local_name().as_ref() == name.as_bytes() {
                    return Some(before);
                }
                if skip_depth == Some(depth) {
                    skip_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

fn indent_unit(text: &str) -> String {
    for line in text.lines() {
        let lead: String = line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        if !lead.is_empty() && line.trim_start().starts_with('<') {
            return lead;
        }
    }
    "    ".to_owned()
}

// ----- new projects --------------------------------------------------------------------

/// Archetypes offered for a new project: (label, groupId, artifactId, version).
pub const ARCHETYPES: &[(&str, &str, &str, &str)] = &[
    (
        "Quickstart: a jar with a main class and a JUnit test",
        "org.apache.maven.archetypes",
        "maven-archetype-quickstart",
        "1.5",
    ),
    ("Web app: a war with a JSP page", "org.apache.maven.archetypes", "maven-archetype-webapp", "1.5"),
    ("Simple: a project with no code", "org.apache.maven.archetypes", "maven-archetype-simple", "1.5"),
];

/// Checks a groupId or artifactId: letters, digits, `.`, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !id.starts_with('.')
        && !id.ends_with('.')
}

/// `archetype:generate` arguments for a new project, asking nothing.
pub fn archetype_args(archetype: (&str, &str, &str), group: &str, artifact: &str) -> Vec<String> {
    vec![
        "-B".to_owned(),
        "archetype:generate".to_owned(),
        format!("-DarchetypeGroupId={}", archetype.0),
        format!("-DarchetypeArtifactId={}", archetype.1),
        format!("-DarchetypeVersion={}", archetype.2),
        format!("-DgroupId={group}"),
        format!("-DartifactId={artifact}"),
        "-Dversion=1.0-SNAPSHOT".to_owned(),
        format!("-Dpackage={group}"),
    ]
}

/// `help:effective-pom` arguments, written to `file`.
pub fn effective_pom_args(file: &Path) -> Vec<String> {
    vec!["-q".to_owned(), "help:effective-pom".to_owned(), format!("-Doutput={}", file.display())]
}

/// The last `[ERROR]` lines of Maven's output, for saying why it failed.
pub fn error_summary(output: &str) -> String {
    // Maven's first error says what failed; the rest is advice on reading logs.
    let first = output.lines().filter_map(|l| l.strip_prefix("[ERROR] ")).find(|l| {
        let l = l.trim();
        !l.is_empty()
            && !l.starts_with("-> [Help")
            && !l.starts_with("Re-run Maven")
            && !l.starts_with("To see the full")
    });
    match first {
        Some(line) => {
            let line = line.split(" -> [Help").next().unwrap_or(line).trim();
            // "Failed to execute goal g:a:v:goal (id) on project x: why" says it best by its why.
            match line.split_once(": ") {
                Some((head, why)) if head.starts_with("Failed to execute goal") && !why.trim().is_empty() => {
                    why.trim().to_owned()
                }
                _ => line.to_owned(),
            }
        }
        None => output.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Maven failed").trim().to_owned(),
    }
}

/// Why a new project can't be made in `dir`, if it can't: Maven adds it as a module of
/// a pom there, and only a pom with `<packaging>pom</packaging>` takes modules.
pub fn new_project_blocked(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join("pom.xml")).ok()?;
    let pom = parse_pom(&text);
    (pom.packaging() != "pom").then(|| {
        format!(
            "{} is a Maven project ({} packaging): choose a folder outside it, or one whose pom has <packaging>pom</packaging> to add a module",
            dir.display(),
            pom.packaging()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent>
    <groupId>com.example</groupId>
    <artifactId>parent</artifactId>
    <version>2.0</version>
  </parent>
  <artifactId>demo</artifactId>
  <name>Demo &amp; more</name>
  <properties>
    <junit.version>4.13.2</junit.version>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>junit</groupId>
      <artifactId>junit</artifactId>
      <version>${junit.version}</version>
      <scope>test</scope>
      <exclusions>
        <exclusion><groupId>org.hamcrest</groupId><artifactId>*</artifactId></exclusion>
      </exclusions>
    </dependency>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
    </dependency>
  </dependencies>
  <build>
    <plugins>
      <plugin>
        <groupId>org.codehaus.mojo</groupId>
        <artifactId>exec-maven-plugin</artifactId>
        <version>3.5.0</version>
        <dependencies><dependency><groupId>x</groupId><artifactId>y</artifactId></dependency></dependencies>
      </plugin>
      <plugin>
        <artifactId>maven-compiler-plugin</artifactId>
      </plugin>
    </plugins>
  </build>
</project>
"#;

    #[test]
    fn poms_are_read() {
        let pom = parse_pom(POM);
        assert_eq!(pom.artifact_id, "demo");
        assert_eq!(pom.group(), "com.example");
        assert_eq!(pom.version_text(), "2.0");
        assert_eq!(pom.name.as_deref(), Some("Demo & more"));
        assert_eq!(pom.coordinates(), "com.example:demo:2.0");
        assert_eq!(pom.dependencies.len(), 2, "a plugin's own dependencies aren't the project's");
        assert_eq!(pom.dependencies[0].version.as_deref(), Some("${junit.version}"));
        assert_eq!(pom.dependencies[0].exclusions, vec![("org.hamcrest".to_owned(), "*".to_owned())]);
        assert_eq!(pom.managed.len(), 1);
        assert_eq!(pom.plugins.len(), 2);
        assert_eq!(pom.plugins[0].prefix(), "exec");
        assert_eq!(pom.plugins[1].group_id, "org.apache.maven.plugins");
        assert_eq!(pom.plugins[1].prefix(), "compiler");
        assert_eq!(pom.properties["junit.version"], "4.13.2");
    }

    #[test]
    fn values_are_filled_in() {
        let mut props = HashMap::new();
        props.insert("a".to_owned(), "1".to_owned());
        props.insert("b".to_owned(), "${a}.2".to_owned());
        assert_eq!(interpolate("v${b}-x", &props), "v1.2-x");
        assert_eq!(interpolate("${missing}", &props), "${missing}");
        assert_eq!(interpolate("${a", &props), "${a");
    }

    /// A local repository in a temporary folder, with poms written as given.
    fn repo(name: &str, poms: &[(&str, &str, &str, &str)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rc-maven-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (g, a, v, body) in poms {
            let path = artifact_path(&dir, g, a, v, "pom");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let text = format!(
                "<project><groupId>{g}</groupId><artifactId>{a}</artifactId><version>{v}</version>{body}</project>"
            );
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn deps(list: &[(&str, &str, &str)]) -> String {
        let mut s = "<dependencies>".to_owned();
        for (g, a, v) in list {
            s.push_str(&format!(
                "<dependency><groupId>{g}</groupId><artifactId>{a}</artifactId><version>{v}</version></dependency>"
            ));
        }
        s.push_str("</dependencies>");
        s
    }

    #[test]
    fn dependencies_resolve_as_maven_does() {
        let repo = repo(
            "resolve",
            &[
                ("g", "a", "1", &deps(&[("g", "c", "1"), ("g", "d", "1")])),
                ("g", "b", "1", &deps(&[("g", "c", "2")])),
                ("g", "c", "1", ""),
                ("g", "c", "2", ""),
                (
                    "g",
                    "t",
                    "1",
                    "<dependencies><dependency><groupId>g</groupId><artifactId>c</artifactId><version>9</version><scope>test</scope></dependency><dependency><groupId>g</groupId><artifactId>o</artifactId><version>1</version><optional>true</optional></dependency></dependencies>",
                ),
            ],
        );
        let dir = repo.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        let pom = format!(
            "<project><groupId>w</groupId><artifactId>w</artifactId><version>1</version>{}</project>",
            deps(&[("g", "a", "1"), ("g", "b", "1"), ("g", "t", "1"), ("g", "missing", "3")])
        );
        std::fs::write(dir.join("pom.xml"), &pom).unwrap();
        let project = find_projects(&dir).remove(0);
        let tree = Resolver::new(repo.clone()).resolve(&project);
        assert_eq!(tree.len(), 4);
        // a -> c 1 (nearest first, breadth first), d not downloaded.
        assert_eq!(tree[0].children[0].version, "1");
        assert_eq!(tree[0].children[0].state, DepState::Resolved);
        assert_eq!(tree[0].children[1].state, DepState::NotDownloaded);
        // b -> c 2 loses to c 1.
        assert_eq!(tree[1].children[0].state, DepState::Omitted { used: "1".to_owned() });
        // t's test and optional dependencies aren't passed on.
        assert!(tree[2].children.is_empty());
        assert_eq!(tree[3].state, DepState::NotDownloaded);
        assert_eq!(tree_counts(&tree), (7, 2, 1));
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn parents_boms_management_and_exclusions() {
        let repo = repo(
            "parents",
            &[
                (
                    "p",
                    "parent",
                    "1",
                    "<properties><lib.version>5</lib.version></properties><dependencyManagement><dependencies><dependency><groupId>g</groupId><artifactId>lib</artifactId><version>${lib.version}</version></dependency><dependency><groupId>b</groupId><artifactId>bom</artifactId><version>1</version><type>pom</type><scope>import</scope></dependency></dependencies></dependencyManagement>",
                ),
                (
                    "b",
                    "bom",
                    "1",
                    "<dependencyManagement><dependencies><dependency><groupId>g</groupId><artifactId>fromBom</artifactId><version>7</version></dependency></dependencies></dependencyManagement>",
                ),
                ("g", "lib", "5", &deps(&[("g", "inner", "1"), ("x", "gone", "1")])),
                ("g", "inner", "1", ""),
                ("g", "inner", "2", ""),
                ("g", "fromBom", "7", ""),
            ],
        );
        let dir = repo.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        let pom = "<project><parent><groupId>p</groupId><artifactId>parent</artifactId><version>1</version></parent><artifactId>w</artifactId>\
            <dependencyManagement><dependencies><dependency><groupId>g</groupId><artifactId>inner</artifactId><version>2</version></dependency></dependencies></dependencyManagement>\
            <dependencies><dependency><groupId>g</groupId><artifactId>lib</artifactId><exclusions><exclusion><groupId>x</groupId><artifactId>*</artifactId></exclusion></exclusions></dependency>\
            <dependency><groupId>g</groupId><artifactId>fromBom</artifactId></dependency></dependencies></project>";
        std::fs::write(dir.join("pom.xml"), pom).unwrap();
        let project = find_projects(&dir).remove(0);
        let tree = Resolver::new(repo.clone()).resolve(&project);
        assert_eq!(tree[0].coordinates(), "g:lib:5", "version from the parent's management, with its property");
        assert_eq!(tree[0].children.len(), 1, "x:gone excluded");
        assert_eq!(tree[0].children[0].version, "2", "the project's management wins further down");
        assert_eq!(tree[1].coordinates(), "g:fromBom:7", "version from an imported BOM");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn modules_are_projects() {
        let dir = std::env::temp_dir().join(format!("rc-maven-modules-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("core")).unwrap();
        std::fs::write(
            dir.join("pom.xml"),
            "<project><groupId>g</groupId><artifactId>top</artifactId><version>1</version><packaging>pom</packaging><modules><module>core</module><module>gone</module></modules></project>",
        )
        .unwrap();
        std::fs::write(
            dir.join("core").join("pom.xml"),
            "<project><parent><groupId>g</groupId><artifactId>top</artifactId><version>1</version></parent><artifactId>core</artifactId></project>",
        )
        .unwrap();
        let projects = find_projects(&dir);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].modules.len(), 1);
        assert_eq!(projects[0].modules[0].pom.coordinates(), "g:core:1");
        assert_eq!(all_projects(&projects).len(), 2);
        let names: Vec<String> = projects[0].plugins().iter().map(Plugin::prefix).collect();
        assert_eq!(names, ["clean", "install", "deploy", "site"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn maven_tree_output() {
        let text = "com.example:demo:jar:1.0-SNAPSHOT\n\
                    +- junit:junit:jar:4.13.2:test\n\
                    |  \\- org.hamcrest:hamcrest-core:jar:1.3:test\n\
                    +- org.lwjgl:lwjgl:jar:natives-windows:3.3.3:compile\n\
                    \\- com.google.guava:guava:jar:32.0.0-jre:compile (omitted for conflict with 33.0.0-jre)\n";
        let tree = parse_tree(text);
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].children[0].coordinates(), "org.hamcrest:hamcrest-core:1.3");
        assert_eq!(tree[0].children[0].scope, "test");
        assert_eq!(tree[1].version, "3.3.3");
        assert_eq!(tree[2].state, DepState::Omitted { used: "33.0.0-jre".to_owned() });
    }

    #[test]
    fn plugin_goals_from_describe() {
        let out = "[INFO] --- help:3.5.2:describe (default-cli) @ x ---\n[INFO] org.codehaus.mojo:exec-maven-plugin:3.5.0\nName: Exec Maven Plugin\nGoal Prefix: exec\n\nThis plugin has 3 goals:\n\nexec:exec\n  Description: A Plugin for executing external programs.\n\nexec:help\n  Description: Display help.\n\nexec:java\n  Description: Executes the supplied java class.\n";
        assert_eq!(parse_describe(out), ["exec", "help", "java"]);
    }

    #[test]
    fn goal_lines_are_checked() {
        assert_eq!(clean_goal(" mvn clean install -DskipTests ").unwrap(), "clean install -DskipTests");
        assert!(clean_goal("clean; rm -rf /").is_err());
        assert!(clean_goal("clean && x").is_err());
        assert!(clean_goal("$(evil)").is_err());
        assert!(clean_goal("  ").is_err());
    }

    #[test]
    fn central_search() {
        assert!(search_url("google guava").ends_with("q=google+guava&rows=20&wt=json"));
        assert!(search_url("com.google.guava:guava").contains("q=g%3Acom.google.guava+AND+a%3Aguava&"));
        let json = r#"{"response":{"numFound":2,"docs":[{"id":"junit:junit","g":"junit","a":"junit","latestVersion":"4.13.2"},{"g":"x","a":"y","v":"1"}]}}"#;
        let found = parse_search(json).unwrap();
        assert_eq!(found[0], Found { group_id: "junit".into(), artifact_id: "junit".into(), version: "4.13.2".into() });
        assert_eq!(found[1].version, "1");
        assert!(parse_search("<html>").is_err());
    }

    #[test]
    fn dependencies_are_added_where_they_belong() {
        let found =
            Found { group_id: "com.google.guava".into(), artifact_id: "guava".into(), version: "33.0.0-jre".into() };
        let (out, line) = add_dependency(POM, &found, None).unwrap();
        let pom = parse_pom(&out);
        assert_eq!(pom.dependencies.len(), 3);
        assert_eq!(pom.dependencies[2].artifact_id, "guava");
        assert_eq!(pom.managed.len(), 1, "not under dependencyManagement");
        assert_eq!(out.lines().nth(line).unwrap(), "    <dependency>");
        assert!(
            out.contains("      <version>33.0.0-jre</version>\n    </dependency>\n  </dependencies>\n  <build>"),
            "{out}"
        );
        assert!(add_dependency(&out, &found, None).is_err(), "already there");
        // No <dependencies> yet: one is made before </project>, CRLF kept.
        let bare = "<project>\r\n\t<artifactId>x</artifactId>\r\n</project>\r\n";
        let (out, line) = add_dependency(bare, &found, Some("test")).unwrap();
        assert!(out.contains("\t<dependencies>\r\n\t\t<dependency>\r\n"));
        assert!(out.contains("\t\t\t<scope>test</scope>\r\n\t\t</dependency>\r\n\t</dependencies>\r\n</project>"));
        assert_eq!(out.lines().nth(line).unwrap(), "\t\t<dependency>");
        assert_eq!(parse_pom(&out).dependencies[0].scope.as_deref(), Some("test"));
    }

    #[test]
    fn new_project_names() {
        assert!(valid_id("com.example"));
        assert!(valid_id("my-app_2"));
        assert!(!valid_id("my app"));
        assert!(!valid_id(".x"));
        assert!(!valid_id(""));
        let args = archetype_args(("g", "a", "1"), "com.example", "demo");
        assert!(args.contains(&"-DartifactId=demo".to_owned()) && args[0] == "-B");
    }

    #[test]
    fn profiles_apply_as_maven_decides() {
        let pom = parse_pom(
            "<project><artifactId>x</artifactId><profiles>\
             <profile><id>linux</id><activation><os><name>linux</name><arch>amd64</arch></os></activation>\
             <properties><fx>linux</fx></properties></profile>\
             <profile><id>win</id><activation><os><family>windows</family><arch>amd64</arch></os>\
             <property><name>monocle</name><value>!true</value></property></activation>\
             <properties><fx>win</fx></properties>\
             <dependencies><dependency><groupId>g</groupId><artifactId>w</artifactId><version>1</version></dependency></dependencies></profile>\
             <profile><id>fallback</id><activation><activeByDefault>true</activeByDefault></activation></profile>\
             <profile><id>java17</id><activation><jdk>[17,)</jdk></activation></profile>\
             </profiles><dependencies><dependency><groupId>g</groupId><artifactId>top</artifactId></dependency></dependencies></project>",
        );
        assert_eq!(pom.profiles.len(), 4);
        assert_eq!(pom.dependencies.len(), 1, "a profile's dependencies stay in it");
        assert_eq!(pom.profiles[1].body.dependencies.len(), 1);
        assert_eq!(pom.profiles[1].property, Some(("monocle".to_owned(), Some("!true".to_owned()))));
        let windows = Environment {
            os_name: "windows 11".into(),
            os_arch: "amd64".into(),
            family: "windows",
            jdk: Some("21.0.12".into()),
            properties: HashMap::new(),
        };
        let ids = |env: &Environment| -> Vec<String> {
            active_profiles(&pom, env, None).iter().map(|p| p.id.clone()).collect()
        };
        assert_eq!(ids(&windows), ["win", "java17"]);
        let old =
            Environment { os_name: "linux".into(), family: "unix", jdk: Some("1.8.0_402".into()), ..windows.clone() };
        assert_eq!(ids(&old), ["linux"]);
        let mac =
            Environment { os_name: "mac os x".into(), os_arch: "aarch64".into(), family: "mac", jdk: None, ..windows };
        assert_eq!(ids(&mac), ["fallback"], "none applies: the default one");
        assert!(jdk_matches("1.8", "1.8.0_402") && jdk_matches("17", "17.0.2") && !jdk_matches("17", "21"));
        assert!(jdk_matches("[11,17)", "11.0.1") && !jdk_matches("[11,17)", "17") && jdk_matches("!1.8", "21"));
    }

    #[test]
    fn profile_properties_choose_a_classifier() {
        let repo = repo(
            "profiles",
            &[
                (
                    "fx",
                    "parent",
                    "1",
                    "<profiles><profile><id>w</id><activation><os><family>windows</family></os></activation><properties><fx.platform>win</fx.platform></properties></profile>\
                  <profile><id>l</id><activation><os><family>unix</family></os></activation><properties><fx.platform>linux</fx.platform></properties></profile></profiles>",
                ),
                (
                    "fx",
                    "controls",
                    "1",
                    "<parent><groupId>fx</groupId><artifactId>parent</artifactId><version>1</version></parent><dependencies><dependency><groupId>fx</groupId><artifactId>controls</artifactId><version>1</version><classifier>${fx.platform}</classifier></dependency></dependencies>",
                ),
            ],
        );
        let dir = repo.join("work");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("pom.xml"),
            format!(
                "<project><groupId>w</groupId><artifactId>w</artifactId><version>1</version>{}</project>",
                deps(&[("fx", "controls", "1")])
            ),
        )
        .unwrap();
        let project = find_projects(&dir).remove(0);
        let env = Environment {
            os_name: "windows 11".into(),
            os_arch: "amd64".into(),
            family: "windows",
            jdk: None,
            properties: HashMap::new(),
        };
        let tree = Resolver::with_env(repo.clone(), env).resolve(&project);
        assert_eq!(tree[0].children.len(), 1, "the platform jar isn't the same artifact");
        assert_eq!(tree[0].children[0].coordinates(), "fx:controls:win:1");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn errors_are_summed_up() {
        let out = "[INFO] Scanning\n[ERROR] Failed to execute goal x (default) on project p: Compilation failure -> [Help 1]\n[ERROR] \n[ERROR] To see the full stack trace\n";
        assert_eq!(error_summary(out), "Compilation failure");
        assert_eq!(error_summary("[ERROR] Unknown lifecycle phase \"x\".\n"), "Unknown lifecycle phase \"x\".");
        let dir = std::env::temp_dir().join(format!("rc-maven-blocked-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(new_project_blocked(&dir), None);
        std::fs::write(dir.join("pom.xml"), "<project><artifactId>a</artifactId></project>").unwrap();
        assert!(new_project_blocked(&dir).unwrap().contains("jar packaging"));
        std::fs::write(dir.join("pom.xml"), "<project><artifactId>a</artifactId><packaging>pom</packaging></project>")
            .unwrap();
        assert_eq!(new_project_blocked(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(error_summary("boom\n"), "boom");
    }
}
