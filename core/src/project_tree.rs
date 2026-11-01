//! What a project's folders and files are, as JetBrains IDEs mark them. There those ask to
//! be set up; here the conventions of each language say.

use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderKind {
    Plain,
    /// A folder inside a source root: a Java package, or a Python one by its `__init__.py`.
    Package,
    Excluded,
    Source,
    Resources,
    Test,
    TestResources,
}

impl FolderKind {
    /// How to say it to someone hovering over the folder.
    pub fn label(self) -> Option<&'static str> {
        match self {
            FolderKind::Plain => None,
            FolderKind::Package => Some("Package"),
            FolderKind::Excluded => Some("Build output"),
            FolderKind::Source => Some("Source root"),
            FolderKind::Resources => Some("Resources root"),
            FolderKind::Test => Some("Test root"),
            FolderKind::TestResources => Some("Test resources root"),
        }
    }
}

/// Java and Kotlin keep a test source set in `src/test`, and Kotlin Multiplatform and
/// Android in `src/commonTest`, `src/androidTest`; C# keeps a project in `App.Tests`.
pub fn is_test_folder(name: &str) -> bool {
    const NAMES: &[&str] =
        &["test", "tests", "__tests__", "__mocks__", "spec", "specs", "testing", "e2e", "cypress", "playwright"];
    let lower = name.to_lowercase();
    NAMES.contains(&lower.as_str())
        || name.ends_with("Test")
        || name.ends_with("Tests")
        || [".test", ".tests", "_test", "_tests", "-test", "-tests", "_spec", "-spec"]
            .iter()
            .any(|s| lower.ends_with(s))
}

/// Data for tests, rather than tests: Go's `testdata`, pytest's `fixtures`, Jest's snapshots.
fn is_test_data(name: &str) -> bool {
    const NAMES: &[&str] = &["testdata", "test_data", "test-data", "fixtures", "__snapshots__", "resources"];
    NAMES.contains(&name.to_lowercase().as_str())
}

/// Files that make the folder holding them a project or module of its own.
pub fn is_module_file(name: &str) -> bool {
    const NAMES: &[&str] = &[
        "cargo.toml",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "package.json",
        "go.mod",
        "pyproject.toml",
        "setup.py",
        "cmakelists.txt",
        "gemfile",
        "composer.json",
    ];
    let lower = name.to_lowercase();
    NAMES.contains(&lower.as_str()) || lower.ends_with(".csproj") || lower.ends_with(".fsproj")
}

const OUTPUT: &[&str] = &[
    "target",
    "node_modules",
    "build",
    "dist",
    "out",
    "__pycache__",
    ".venv",
    "venv",
    ".gradle",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".output",
    ".turbo",
    ".parcel-cache",
    ".settings",
    "coverage",
    "cmake-build-debug",
    "cmake-build-release",
];

/// Where the source root is in a path: `java` in `src/main/java`, `src/test/kotlin`,
/// `src/commonMain/kotlin`. Maven and Gradle keep them there.
fn language_root(parts: &[&str]) -> Option<usize> {
    const LANGUAGES: &[&str] = &["java", "kotlin", "scala", "groovy", "go", "rust", "python", "cpp", "swift"];
    (2..parts.len()).find(|&at| {
        let set = parts[at - 1];
        let source_set = set == "main" || set == "test" || set.ends_with("Main") || set.ends_with("Test");
        parts[at - 2] == "src" && source_set && LANGUAGES.contains(&parts[at])
    })
}

/// Whether this folder is a package of such a root, whose chains read as `org.example`.
pub fn is_root_package(relative: &str) -> bool {
    let parts: Vec<&str> = relative.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    language_root(&parts).is_some_and(|at| at + 1 < parts.len())
}

/// A folder's kind, knowing whether a `package.json` sits beside it: then these names say
/// more than they would elsewhere.
pub fn folder_kind_beside(relative: &str, children: &[(&str, bool)], beside_manifest: bool) -> FolderKind {
    let kind = folder_kind(relative, children);
    if kind != FolderKind::Plain || !beside_manifest {
        return kind;
    }
    let name = relative.rsplit(['/', '\\']).next().unwrap_or(relative).to_lowercase();
    match name.as_str() {
        "app" | "pages" | "components" | "lib" | "routes" => FolderKind::Source,
        "public" | "static" | "assets" => FolderKind::Resources,
        _ => kind,
    }
}

/// A folder's kind. `relative` is its path in the project, with either kind of slash;
/// `children` names what's in it, and whether each is a folder.
pub fn folder_kind(relative: &str, children: &[(&str, bool)]) -> FolderKind {
    let parts: Vec<&str> = relative.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some((&name, above)) = parts.split_last() else { return FolderKind::Plain };
    let lower = name.to_lowercase();

    // `src/bin` is where Rust keeps programs.
    let is_program_output = matches!(name, "bin" | "obj") && !above.contains(&"src");
    if is_program_output || parts.iter().any(|p| OUTPUT.contains(&p.to_lowercase().as_str())) {
        return FolderKind::Excluded;
    }
    if let Some(at) = language_root(&parts) {
        return match (at + 1 == parts.len(), is_test_folder(parts[at - 1])) {
            (false, _) => FolderKind::Package,
            (true, true) => FolderKind::Test,
            (true, false) => FolderKind::Source,
        };
    }
    let python_package = children.iter().any(|(child, dir)| !dir && *child == "__init__.py");
    // Only the root is marked; what's under it reads as usual, as in those IDEs.
    if above.iter().any(|p| is_test_folder(p)) {
        return if is_test_data(name) {
            FolderKind::TestResources
        } else if python_package {
            FolderKind::Package
        } else {
            FolderKind::Plain
        };
    }
    if lower != "resources" && is_test_data(name) {
        return FolderKind::TestResources;
    }
    if is_test_folder(name) || only_tests(relative, children) {
        return FolderKind::Test;
    }
    if lower == "resources" {
        return FolderKind::Resources;
    }
    if python_package {
        return FolderKind::Package;
    }
    let splits_main = children.iter().any(|(child, dir)| *dir && (*child == "main" || is_test_folder(child)));
    if (lower == "src" && !splits_main) || (lower == "lib" && above.is_empty()) {
        return FolderKind::Source;
    }
    FolderKind::Plain
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClassKind {
    Class,
    Abstract,
    Interface,
    Enum,
    Record,
    Annotation,
    Object,
    Trait,
    /// A web component: `Button.tsx`, `.vue`, `.svelte`, `.astro`.
    Component,
    /// A React or Vue `useThing`.
    Hook,
    /// What a framework serves at an address.
    Page,
    Route,
    CssModule,
}

/// What a source file is, for its icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileRole {
    /// The file is this type, of the same name.
    pub class: Option<ClassKind>,
    /// A program starts here.
    pub runs: bool,
    pub test: bool,
}

impl FileRole {
    /// What the path alone says.
    pub fn of_name(relative: &str) -> Self {
        let page = web_page(relative);
        let test = crate::file_icons::is_test_file(relative);
        Self { class: web_kind(relative), runs: !test && page.is_some_and(|p| p.1.is_some()), test }
    }

    /// What the path and the text say.
    pub fn of(relative: &str, source: &str) -> Self {
        let path = Path::new(relative);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let test = crate::file_icons::is_test_file(relative) || has_test_marks(&extension, source);
        let class = web_kind(relative).or_else(|| class_kind(&extension, &stem, source));
        let starts = !crate::run::program_lines(path, source).is_empty();
        let served = web_page(relative).is_some_and(|p| p.1.is_some());
        Self { class, runs: !test && (starts || served), test }
    }
}

/// What a web framework makes of a file by where it is, and the address it's served at,
/// unless that takes values to fill in. Next and Nuxt `app/` and `pages/`, SvelteKit `routes/`.
pub fn web_page(relative: &str) -> Option<(ClassKind, Option<String>)> {
    let parts: Vec<&str> = relative.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let (&file, above) = parts.split_last()?;
    let (stem, extension) = file.rsplit_once('.')?;
    if !matches!(extension, "tsx" | "jsx" | "ts" | "js" | "vue" | "svelte" | "astro" | "mdx") {
        return None;
    }
    let at = above.iter().rposition(|p| matches!(*p, "app" | "pages" | "routes"))?;
    let mut route: Vec<&str> = above[at + 1..].iter().copied().filter(|p| !p.starts_with('(')).collect();
    let kind = match above[at] {
        "app" if stem == "page" => ClassKind::Page,
        "app" if stem == "route" => ClassKind::Route,
        "routes" if stem == "+page" => ClassKind::Page,
        "routes" if stem == "+server" => ClassKind::Route,
        "pages" if !stem.starts_with('_') && !stem.contains('.') => {
            if stem != "index" {
                route.push(stem);
            }
            if route.first() == Some(&"api") { ClassKind::Route } else { ClassKind::Page }
        }
        _ => return None,
    };
    let filled = !route.iter().any(|p| p.contains('[') || p.starts_with(':'));
    Some((kind, filled.then(|| format!("/{}", route.join("/")))))
}

/// What a file is on the web, from its path alone.
fn web_kind(relative: &str) -> Option<ClassKind> {
    if let Some((kind, _)) = web_page(relative) {
        return Some(kind);
    }
    let name = relative.rsplit(['/', '\\']).next().unwrap_or(relative);
    let (stem, extension) = name.rsplit_once('.')?;
    let capital = stem.chars().next().is_some_and(char::is_uppercase) && !stem.contains('.');
    let hook = stem.strip_prefix("use").and_then(|r| r.chars().next()).is_some_and(char::is_uppercase);
    match extension {
        "css" | "scss" | "sass" | "less" if stem.ends_with(".module") => Some(ClassKind::CssModule),
        "vue" | "svelte" | "astro" => Some(ClassKind::Component),
        "tsx" | "jsx" if capital => Some(ClassKind::Component),
        "ts" | "js" | "tsx" | "jsx" if hook && !stem.contains('.') => Some(ClassKind::Hook),
        _ => None,
    }
}

/// Files that belong under another of the same folder, as (file, the one above it): what's
/// built from it, its tests, its styles.
pub fn nested_files(names: &[&str]) -> std::collections::HashMap<String, String> {
    const VIEWS: &[&str] = &["tsx", "jsx", "vue", "svelte", "ts", "js", "html"];
    let above = |name: &str| -> Option<String> {
        let (stem, extension) = name.rsplit_once('.')?;
        let mut tries: Vec<String> = Vec::new();
        if extension == "map" {
            tries.push(stem.to_owned());
        }
        for mark in [".test", ".spec", ".min"] {
            if let Some(base) = stem.strip_suffix(mark) {
                tries.push(format!("{base}.{extension}"));
                match extension {
                    "ts" => tries.push(format!("{base}.tsx")),
                    "js" => tries.push(format!("{base}.jsx")),
                    _ => {}
                }
            }
        }
        match extension {
            "ts" => {
                if let Some(base) = stem.strip_suffix(".d") {
                    tries.push(format!("{base}.ts"));
                    tries.push(format!("{base}.js"));
                }
            }
            "js" => tries.extend(["ts", "tsx"].map(|e| format!("{stem}.{e}"))),
            "jsx" => tries.push(format!("{stem}.tsx")),
            "css" => tries.extend(["scss", "sass", "less"].map(|e| format!("{stem}.{e}"))),
            _ => {}
        }
        if matches!(extension, "css" | "scss" | "sass" | "less") {
            let base = stem.strip_suffix(".module").unwrap_or(stem);
            tries.extend(VIEWS.iter().map(|e| format!("{base}.{e}")));
        }
        tries.into_iter().find(|t| t != name && names.contains(&t.as_str()))
    };
    let mut nested = std::collections::HashMap::new();
    for name in names {
        let Some(mut top) = above(name) else { continue };
        // `a.js.map` under `a.js`, itself under `a.ts`.
        for _ in 0..4 {
            match above(&top) {
                Some(next) if next != *name => top = next,
                _ => break,
            }
        }
        nested.insert((*name).to_owned(), top);
    }
    nested
}

fn has_test_marks(extension: &str, source: &str) -> bool {
    // Marks that only ever begin a line, and what a file of tests says somewhere.
    let (begins, holds): (&[&str], &[&str]) = match extension {
        "java" | "kt" | "scala" | "groovy" => (&["@Test", "@ParameterizedTest"], &[]),
        "cs" | "fs" | "vb" => (&["[Fact]", "[Theory]", "[Test]", "[TestMethod]"], &[]),
        "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" => {
            (&["TEST(", "TEST_F(", "TEST_P(", "TEST_CASE(", "BOOST_AUTO_TEST_CASE("], &[])
        }
        "swift" => (&["@Test"], &["XCTestCase"]),
        "dart" => (&[], &["package:test/", "package:flutter_test/"]),
        "rb" => (&[], &["RSpec.describe", "Minitest::Test"]),
        "php" => (&[], &["extends TestCase"]),
        "ex" | "exs" => (&[], &["use ExUnit.Case"]),
        "py" | "pyw" => (&[], &["unittest.TestCase"]),
        _ => return false,
    };
    source.lines().map(str::trim).filter(|l| !l.starts_with("//") && !l.starts_with('#')).any(|line| {
        let begun = begins.iter().any(|m| {
            line.strip_prefix(m).is_some_and(|rest| m.ends_with('(') || rest.is_empty() || rest.starts_with(' '))
        });
        begun || holds.iter().any(|m| line.contains(m))
    })
}

/// A name as files and types write it differently: `user_store`, `UserStore`.
fn plain_name(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// The type a file declares under its own name, in any language.
fn class_kind(extension: &str, stem: &str, source: &str) -> Option<ClassKind> {
    if !crate::file_icons::is_code(extension) {
        return None;
    }
    let wanted = plain_name(stem);
    let mut comment = false;
    for line in source.lines() {
        let line = line.trim();
        if comment {
            comment = !line.contains("*/");
            continue;
        }
        if line.starts_with("/*") {
            comment = !line.contains("*/");
            continue;
        }
        if ["//", "*", "#", "--", ";", "%"].iter().any(|c| line.starts_with(c)) {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        for (at, word) in words.iter().enumerate() {
            let named = words.get(at + 1).is_some_and(|next| {
                next.split(|c: char| !c.is_alphanumeric() && c != '_').next().is_some_and(|n| plain_name(n) == wanted)
            });
            if !named {
                continue;
            }
            let before = &words[..at];
            let kind = match *word {
                "class" if before.contains(&"abstract") => ClassKind::Abstract,
                "class" if before.contains(&"enum") => ClassKind::Enum,
                "class" if before.contains(&"annotation") => ClassKind::Annotation,
                "class" | "struct" | "union" => ClassKind::Class,
                "interface" | "protocol" => ClassKind::Interface,
                "@interface" => ClassKind::Annotation,
                "enum" => ClassKind::Enum,
                "record" => ClassKind::Record,
                "object" | "defmodule" | "module" => ClassKind::Object,
                "trait" | "mixin" => ClassKind::Trait,
                // Go: `type Name struct`.
                "type" => match words.get(at + 2).copied() {
                    Some("interface" | "interface{") => ClassKind::Interface,
                    _ => ClassKind::Class,
                },
                _ => continue,
            };
            return Some(kind);
        }
    }
    None
}

/// Whether a file goes by its type's name alone, as it does in these languages.
pub fn hides_extension(name: &str) -> bool {
    let extension = name.rsplit_once('.').map_or("", |n| n.1);
    matches!(extension, "java" | "kt" | "scala" | "groovy" | "cs")
}

/// A project's initials and color, always the same for the same name, so that windows can
/// be told apart.
pub fn project_badge(name: &str) -> (String, [u8; 3]) {
    const COLORS: [[u8; 3]; 8] = [
        [0x4A, 0x7F, 0xD9],
        [0x2F, 0x9E, 0x8F],
        [0x4F, 0x9E, 0x4F],
        [0xB0, 0x7A, 0x2E],
        [0xC4, 0x5A, 0x4A],
        [0xB0, 0x56, 0x96],
        [0x7E, 0x62, 0xC9],
        [0x3C, 0x8D, 0xB8],
    ];
    let mut words: Vec<String> = Vec::new();
    let mut after_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            words.push(String::new());
            after_lower = false;
            continue;
        }
        if words.is_empty() || (c.is_uppercase() && after_lower) {
            words.push(String::new());
        }
        if let Some(word) = words.last_mut() {
            word.push(c);
        }
        after_lower = c.is_lowercase();
    }
    words.retain(|w| !w.is_empty());
    let letters: String = match words.as_slice() {
        [] => "?".to_owned(),
        [only] => only.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    let hash = name.bytes().fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    (letters.to_uppercase(), COLORS[hash as usize % COLORS.len()])
}

/// What New offers: a file of a language, already saying what such a file must.
pub struct Template {
    pub label: &'static str,
    pub extension: &'static str,
    /// A dependency the project must list for this to be offered; empty: always.
    pub needs: &'static str,
    /// With `{name}`, `{folder}` and `{package}`, a whole `package x;` line or nothing.
    body: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template { label: "Java Class", needs: "", extension: "java", body: "{package}public class {name} {\n}\n" },
    Template { label: "Kotlin Class", needs: "", extension: "kt", body: "{package}class {name} {\n}\n" },
    Template { label: "Scala Class", needs: "", extension: "scala", body: "{package}class {name} {\n}\n" },
    Template { label: "Groovy Class", needs: "", extension: "groovy", body: "{package}class {name} {\n}\n" },
    Template { label: "C# Class", needs: "", extension: "cs", body: "class {name}\n{\n}\n" },
    Template { label: "Python File", needs: "", extension: "py", body: "" },
    Template { label: "Rust File", needs: "", extension: "rs", body: "" },
    Template { label: "Go File", needs: "", extension: "go", body: "package {folder}\n" },
    Template { label: "C File", needs: "", extension: "c", body: "" },
    Template { label: "C Header", needs: "", extension: "h", body: "#pragma once\n" },
    Template { label: "C++ File", needs: "", extension: "cpp", body: "" },
    Template { label: "C++ Header", needs: "", extension: "hpp", body: "#pragma once\n" },
    Template { label: "JavaScript File", needs: "", extension: "js", body: "" },
    Template { label: "TypeScript File", needs: "", extension: "ts", body: "" },
    Template {
        label: "HTML File",
        needs: "",
        extension: "html",
        body: "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title>{name}</title>\n</head>\n<body>\n</body>\n</html>\n",
    },
    Template { label: "Stylesheet", needs: "", extension: "css", body: "" },
    Template { label: "PHP File", needs: "", extension: "php", body: "<?php\n" },
    Template { label: "Ruby File", needs: "", extension: "rb", body: "" },
    Template { label: "Swift File", needs: "", extension: "swift", body: "" },
    Template { label: "Dart File", needs: "", extension: "dart", body: "" },
    Template { label: "Lua File", needs: "", extension: "lua", body: "" },
    Template { label: "Zig File", needs: "", extension: "zig", body: "" },
    Template { label: "Elixir File", needs: "", extension: "ex", body: "" },
    Template { label: "Haskell File", needs: "", extension: "hs", body: "module {name} where\n" },
    Template { label: "Shell Script", needs: "", extension: "sh", body: "#!/bin/sh\n" },
    Template { label: "Markdown File", needs: "", extension: "md", body: "" },
    Template { label: "JSON File", needs: "", extension: "json", body: "{}\n" },
    Template { label: "YAML File", needs: "", extension: "yaml", body: "" },
    Template { label: "TOML File", needs: "", extension: "toml", body: "" },
    Template {
        label: "React Component",
        needs: "react",
        extension: "tsx",
        body: "export default function {name}() {\n  return <div></div>;\n}\n",
    },
    Template {
        label: "Vue Component",
        needs: "vue",
        extension: "vue",
        body: "<script setup lang=\"ts\">\n</script>\n\n<template>\n  <div></div>\n</template>\n",
    },
    Template {
        label: "Svelte Component",
        needs: "svelte",
        extension: "svelte",
        body: "<script lang=\"ts\">\n</script>\n\n<div></div>\n",
    },
    Template { label: "Astro Component", needs: "astro", extension: "astro", body: "---\n---\n\n<div></div>\n" },
    Template { label: "CSS Module", needs: "", extension: "module.css", body: "" },
    Template {
        label: "Vitest Test",
        needs: "vitest",
        extension: "test.ts",
        body: "import { describe, expect, it } from 'vitest';\n\ndescribe('{name}', () => {\n  it('works', () => {\n    expect(true).toBe(true);\n  });\n});\n",
    },
    Template {
        label: "Jest Test",
        needs: "jest",
        extension: "test.ts",
        body: "describe('{name}', () => {\n  it('works', () => {\n    expect(true).toBe(true);\n  });\n});\n",
    },
    Template {
        label: "package.json",
        needs: "",
        extension: "json",
        body: "{\n  \"name\": \"{folder}\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"scripts\": {}\n}\n",
    },
];

impl Template {
    /// The name this kind of file always has, so there's nothing to ask.
    pub fn fixed_name(&self) -> Option<&'static str> {
        (self.label == "package.json").then_some("package.json")
    }

    /// A file's name without what this kind of file adds.
    pub fn stem_of<'a>(&self, file_name: &'a str) -> &'a str {
        file_name.strip_suffix(&format!(".{}", self.extension)).unwrap_or(file_name)
    }

    /// The file a typed name means.
    pub fn file_name(&self, typed: &str) -> String {
        let suffix = format!(".{}", self.extension);
        if typed.ends_with(&suffix) { typed.to_owned() } else { format!("{typed}{suffix}") }
    }

    /// The new file's text. `relative_dir` is its folder in the project.
    pub fn text(&self, stem: &str, relative_dir: &str) -> String {
        let parts: Vec<&str> = relative_dir.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
        let package = language_root(&parts).map_or(String::new(), |at| parts[at + 1..].join("."));
        let line = if package.is_empty() { String::new() } else { format!("package {package};\n\n") };
        let line = if self.extension == "java" { line } else { line.replace(';', "") };
        let folder = parts.last().copied().unwrap_or("main");
        self.body.replace("{package}", &line).replace("{name}", stem).replace("{folder}", folder)
    }
}

/// Code here, and all of it tests: a package of them, whatever the folder is called.
fn only_tests(relative: &str, children: &[(&str, bool)]) -> bool {
    let mut files = children
        .iter()
        .filter(|(_, dir)| !dir)
        .map(|(name, _)| *name)
        .filter(|name| name.rsplit_once('.').is_some_and(|(_, extension)| crate::file_icons::is_code(extension)));
    let Some(first) = files.next() else { return false };
    let is_test = |name: &str| crate::file_icons::is_test_file(&format!("{relative}/{name}"));
    is_test(first) && files.all(is_test)
}

#[cfg(test)]
mod tests {
    use super::*;
    use FolderKind::*;

    #[test]
    fn test_roots_of_each_language() {
        for path in [
            "src/test",
            "src/test/java",
            "src/test/kotlin",
            "tests",
            "test",
            "__tests__",
            "spec",
            "e2e",
            "src/commonTest",
            "src/androidTest",
            "App.Tests",
            "AppTests",
            "foo_test",
            "core\\tests",
        ] {
            assert_eq!(folder_kind(path, &[]), Test, "{path}");
        }
    }

    #[test]
    fn only_the_root_is_marked() {
        assert_eq!(folder_kind("tests/unit", &[]), Plain);
        assert_eq!(folder_kind("src/test/java/demo", &[]), Package);
        assert_eq!(folder_kind("src/main/java/demo/test", &[]), Package);
        assert_eq!(folder_kind("tests/pkg", &[("__init__.py", false)]), Package);
    }

    #[test]
    fn these_only_sound_like_tests() {
        for path in ["latest", "contest", "attest", "Contests", "testament"] {
            assert_eq!(folder_kind(path, &[]), Plain, "{path}");
        }
    }

    #[test]
    fn test_data_is_its_own_kind() {
        for path in ["testdata", "pkg/testdata", "src/test/resources", "tests/fixtures", "__snapshots__"] {
            assert_eq!(folder_kind(path, &[]), TestResources, "{path}");
        }
    }

    #[test]
    fn a_folder_of_only_tests_is_a_test_root() {
        assert_eq!(folder_kind("pkg", &[("a_test.go", false), ("b_test.go", false), ("data.json", false)]), Test);
        assert_eq!(folder_kind("pkg", &[("a_test.go", false), ("a.go", false)]), Plain);
        assert_eq!(folder_kind("pkg", &[("data.json", false)]), Plain);
    }

    #[test]
    fn sources_and_resources_where_the_build_tools_keep_them() {
        assert_eq!(folder_kind("src", &[("main.rs", false)]), Source);
        assert_eq!(folder_kind("src", &[("main", true), ("test", true)]), Plain);
        assert_eq!(folder_kind("src/main", &[("java", true)]), Plain);
        assert_eq!(folder_kind("src/main/java", &[]), Source);
        assert_eq!(folder_kind("app/src/main/kotlin", &[]), Source);
        assert_eq!(folder_kind("src/domain/java", &[]), Plain);
        assert_eq!(folder_kind("src/main/resources", &[]), Resources);
        assert_eq!(folder_kind("src/main/java/demo", &[]), Package);
        assert_eq!(folder_kind("docs", &[]), Plain);
        assert!(is_root_package("src/main/java/demo") && !is_root_package("src/main/java") && !is_root_package("a/b"));
    }

    #[test]
    fn output_is_excluded_except_rust_programs() {
        for path in [
            "target",
            "target/debug",
            "node_modules",
            "build",
            "dist",
            "bin",
            "obj",
            "__pycache__",
            "build/classes",
            ".venv",
        ] {
            assert_eq!(folder_kind(path, &[]), Excluded, "{path}");
        }
        assert_eq!(folder_kind("src/bin", &[]), Plain);
    }

    #[test]
    fn knows_module_files() {
        for yes in ["Cargo.toml", "pom.xml", "package.json", "App.csproj", "go.mod"] {
            assert!(is_module_file(yes), "{yes}");
        }
        assert!(!is_module_file("notes.md"));
    }

    #[test]
    fn reads_the_type_a_file_declares() {
        use ClassKind as K;
        for (file, source, kind) in [
            ("A.java", "package x;\n/** class A is */\npublic class A<T> extends B {", K::Class),
            ("A.java", "public abstract class A {", K::Abstract),
            ("A.java", "public interface A {", K::Interface),
            ("A.java", "public enum A { X }", K::Enum),
            ("A.java", "public record A(int x) {}", K::Record),
            ("A.java", "public @interface A {}", K::Annotation),
            ("A.kt", "data class A(val x: Int)", K::Class),
            ("A.kt", "enum class A { X }", K::Enum),
            ("A.kt", "object A {", K::Object),
            ("A.scala", "trait A {", K::Trait),
            ("A.cs", "public struct A {", K::Class),
        ] {
            assert_eq!(FileRole::of(file, source).class, Some(kind), "{source}");
        }
        for (file, source, kind) in [
            ("user.py", "class User(Base):", K::Class),
            ("user_store.rs", "pub struct UserStore {", K::Class),
            ("shape.rs", "pub trait Shape {", K::Trait),
            ("color.ts", "export enum Color {", K::Enum),
            ("shape.ts", "export default interface Shape {", K::Interface),
            ("user.go", "type User struct {", K::Class),
            ("shape.go", "type Shape interface {", K::Interface),
            ("shape.swift", "protocol Shape {", K::Interface),
            ("user.hpp", "class User : public Base {", K::Class),
            ("user.rb", "class User < Base", K::Class),
            ("user.php", "abstract class User {", K::Abstract),
            ("user.ex", "defmodule User do", K::Object),
        ] {
            assert_eq!(FileRole::of(file, source).class, Some(kind), "{source}");
        }
        for (file, source) in [
            ("A.java", "// class A\nclass B {}"),
            ("a.py", "# class A\nclass B:"),
            ("main.rs", "fn main() {}"),
            ("A.java", ""),
            ("a.txt", "class A"),
        ] {
            assert_eq!(FileRole::of(file, source).class, None, "{source}");
        }
    }

    #[test]
    fn knows_where_programs_start() {
        for (file, source) in [
            ("Main.java", "public class Main { public static void main(String[] args) {} }"),
            ("a.c", "int main(void) { return 0; }"),
            ("a.cpp", "int main() { return 0; }"),
            ("a.rs", "fn main() {}"),
            ("a.go", "package main\nfunc main() {}"),
            ("a.py", "if __name__ == '__main__':\n    run()"),
            ("a.kt", "fun main() {}"),
            ("A.cs", "class A { static void Main(string[] args) {} }"),
        ] {
            assert!(FileRole::of(file, source).runs, "{source}");
        }
        for (file, source) in [
            ("Lib.java", "public class Lib { void go() {} }"),
            ("lib.rs", "pub fn main_thing() {}"),
            ("a.go", "package app\nfunc main() {}"),
            ("a.py", "def main(): pass"),
            ("AppTest.java", "class AppTest { public static void main(String[] a) {} }"),
        ] {
            assert!(!FileRole::of(file, source).runs, "{source}");
        }
    }

    #[test]
    fn tests_by_name_or_by_their_marks() {
        assert!(FileRole::of("Checks.java", "class Checks {\n    @Test\n    void a() {}\n}").test);
        assert!(FileRole::of("Checks.java", "class Checks {\n    @Test void a() {}\n}").test);
        assert!(FileRole::of("Checks.cs", "class Checks {\n    [Fact]\n    public void A() {}\n}").test);
        assert!(FileRole::of("AppTest.java", "").test);
        assert!(!FileRole::of("Tester.java", "// uses @Test\nclass Tester {}").test);
        assert!(FileRole::of("checks.cpp", "TEST(A, B) {\n}").test);
        assert!(FileRole::of("checks.py", "class C(unittest.TestCase):").test);
        assert!(FileRole::of("checks.rb", "RSpec.describe C do").test);
        assert!(FileRole::of("checks.ex", "  use ExUnit.Case").test);
        assert!(FileRole::of("Checks.swift", "class C: XCTestCase {").test);
        assert!(!FileRole::of("checks.py", "# not unittest.TestCase").test);
    }

    #[test]
    fn projects_get_lasting_badges() {
        assert_eq!(project_badge("rust-code").0, "RC");
        assert_eq!(project_badge("spring_petclinic").0, "SP");
        assert_eq!(project_badge("PetClinic").0, "PC");
        assert_eq!(project_badge("proj").0, "PR");
        assert_eq!(project_badge("a b c").0, "AC");
        assert_eq!(project_badge("---").0, "?");
        assert_eq!(project_badge("rust-code").1, project_badge("rust-code").1);
        let many: std::collections::HashSet<_> = ["a", "b", "c", "d", "e", "f"].map(|n| project_badge(n).1).into();
        assert!(many.len() > 1);
    }

    #[test]
    fn web_folders_beside_a_manifest() {
        assert_eq!(folder_kind_beside("components", &[], true), Source);
        assert_eq!(folder_kind_beside("web/pages", &[], true), Source);
        assert_eq!(folder_kind_beside("public", &[], true), Resources);
        assert_eq!(folder_kind_beside("public", &[], false), Plain);
        assert_eq!(folder_kind_beside("dist", &[], true), Excluded);
        assert_eq!(folder_kind_beside(".next", &[], false), Excluded);
    }

    #[test]
    fn web_files_by_what_they_are() {
        let kind = |p: &str| FileRole::of_name(p).class;
        assert_eq!(kind("src/Button.tsx"), Some(ClassKind::Component));
        assert_eq!(kind("Card.vue"), Some(ClassKind::Component));
        assert_eq!(kind("card.svelte"), Some(ClassKind::Component));
        assert_eq!(kind("useAuth.ts"), Some(ClassKind::Hook));
        assert_eq!(kind("user.ts"), None);
        assert_eq!(kind("button.tsx"), None);
        assert_eq!(kind("Button.test.tsx"), None);
        assert_eq!(kind("a.module.css"), Some(ClassKind::CssModule));
        assert_eq!(kind("a.css"), None);
    }

    #[test]
    fn pages_and_their_addresses() {
        use ClassKind::{Page, Route};
        let at = |p: &str| web_page(p);
        assert_eq!(at("app/page.tsx"), Some((Page, Some("/".into()))));
        assert_eq!(at("src/app/blog/(x)/page.tsx"), Some((Page, Some("/blog".into()))));
        assert_eq!(at("app/api/a/route.ts"), Some((Route, Some("/api/a".into()))));
        assert_eq!(at("pages/index.vue"), Some((Page, Some("/".into()))));
        assert_eq!(at("pages/about.tsx"), Some((Page, Some("/about".into()))));
        assert_eq!(at("pages/api/a.ts"), Some((Route, Some("/api/a".into()))));
        assert_eq!(at("src/routes/x/+page.svelte"), Some((Page, Some("/x".into()))));
        assert_eq!(at("src/routes/x/+server.ts"), Some((Route, Some("/x".into()))));
        assert_eq!(at("app/blog/[id]/page.tsx"), Some((Page, None)));
        assert_eq!(at("pages/_app.tsx"), None);
        assert_eq!(at("app/layout.tsx"), None);
        assert_eq!(at("pages/About.java"), None);
        assert!(FileRole::of_name("pages/about.vue").runs);
        assert!(!FileRole::of_name("app/blog/[id]/page.tsx").runs);
        assert!(FileRole::of("server.js", "app.listen(3000);").runs);
        assert!(FileRole::of("page.html", "<html>").runs);
    }

    #[test]
    fn nests_what_belongs_under_a_file() {
        let names = [
            "app.ts",
            "app.js",
            "app.js.map",
            "app.test.ts",
            "app.d.ts",
            "Button.tsx",
            "Button.module.css",
            "Button.test.tsx",
            "a.scss",
            "a.css",
            "a.css.map",
            "lib.min.js",
            "lib.js",
            "alone.css",
            "other.ts",
        ];
        let nested = nested_files(&names);
        let top = |n: &str| nested.get(n).map(String::as_str);
        assert_eq!(top("app.js"), Some("app.ts"));
        assert_eq!(top("app.js.map"), Some("app.ts"));
        assert_eq!(top("app.test.ts"), Some("app.ts"));
        assert_eq!(top("app.d.ts"), Some("app.ts"));
        assert_eq!(top("Button.module.css"), Some("Button.tsx"));
        assert_eq!(top("Button.test.tsx"), Some("Button.tsx"));
        assert_eq!(top("a.css"), Some("a.scss"));
        assert_eq!(top("a.css.map"), Some("a.scss"));
        assert_eq!(top("lib.min.js"), Some("lib.js"));
        for alone in ["app.ts", "Button.tsx", "alone.css", "other.ts", "lib.js", "a.scss"] {
            assert_eq!(top(alone), None, "{alone}");
        }
    }

    #[test]
    fn web_templates() {
        let of = |label: &str| TEMPLATES.iter().find(|t| t.label == label).unwrap();
        assert_eq!(
            of("React Component").text("Card", ""),
            "export default function Card() {\n  return <div></div>;\n}\n"
        );
        assert_eq!(of("Vitest Test").file_name("a"), "a.test.ts");
        assert_eq!(of("Vitest Test").stem_of("a.test.ts"), "a");
        assert_eq!(of("CSS Module").file_name("a"), "a.module.css");
        assert_eq!(of("package.json").fixed_name(), Some("package.json"));
        assert!(of("package.json").text("x", "web").contains("\"name\": \"web\""));
    }

    #[test]
    fn new_files_say_what_they_must() {
        let of = |label: &str| TEMPLATES.iter().find(|t| t.label == label).unwrap();
        assert_eq!(of("Java Class").file_name("App"), "App.java");
        assert_eq!(of("Java Class").file_name("App.java"), "App.java");
        assert_eq!(of("Java Class").text("App", "src/main/java/org/x"), "package org.x;\n\npublic class App {\n}\n");
        assert_eq!(of("Java Class").text("App", "other"), "public class App {\n}\n");
        assert_eq!(of("Kotlin Class").text("App", "src/main/kotlin/org"), "package org\n\nclass App {\n}\n");
        assert_eq!(of("Go File").text("a", "pkg\\store"), "package store\n");
        assert_eq!(of("Go File").text("a", ""), "package main\n");
    }
}
