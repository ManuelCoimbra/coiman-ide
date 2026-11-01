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
        Self { test: crate::file_icons::is_test_file(relative), ..Self::default() }
    }

    /// What the path and the text say.
    pub fn of(relative: &str, source: &str) -> Self {
        let path = Path::new(relative);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let test = crate::file_icons::is_test_file(relative) || has_test_marks(&extension, source);
        Self {
            class: class_kind(&extension, &stem, source),
            runs: !test && !crate::run::program_lines(path, source).is_empty(),
            test,
        }
    }
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

/// What New offers: a file of a language, already saying what such a file must.
pub struct Template {
    pub label: &'static str,
    pub extension: &'static str,
    /// With `{name}`, `{folder}` and `{package}`, a whole `package x;` line or nothing.
    body: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template { label: "Java Class", extension: "java", body: "{package}public class {name} {\n}\n" },
    Template { label: "Kotlin Class", extension: "kt", body: "{package}class {name} {\n}\n" },
    Template { label: "Scala Class", extension: "scala", body: "{package}class {name} {\n}\n" },
    Template { label: "Groovy Class", extension: "groovy", body: "{package}class {name} {\n}\n" },
    Template { label: "C# Class", extension: "cs", body: "class {name}\n{\n}\n" },
    Template { label: "Python File", extension: "py", body: "" },
    Template { label: "Rust File", extension: "rs", body: "" },
    Template { label: "Go File", extension: "go", body: "package {folder}\n" },
    Template { label: "C File", extension: "c", body: "" },
    Template { label: "C Header", extension: "h", body: "#pragma once\n" },
    Template { label: "C++ File", extension: "cpp", body: "" },
    Template { label: "C++ Header", extension: "hpp", body: "#pragma once\n" },
    Template { label: "JavaScript File", extension: "js", body: "" },
    Template { label: "TypeScript File", extension: "ts", body: "" },
    Template {
        label: "HTML File",
        extension: "html",
        body: "<!doctype html>\n<html>\n<head>\n  <meta charset=\"utf-8\">\n  <title>{name}</title>\n</head>\n<body>\n</body>\n</html>\n",
    },
    Template { label: "Stylesheet", extension: "css", body: "" },
    Template { label: "PHP File", extension: "php", body: "<?php\n" },
    Template { label: "Ruby File", extension: "rb", body: "" },
    Template { label: "Swift File", extension: "swift", body: "" },
    Template { label: "Dart File", extension: "dart", body: "" },
    Template { label: "Lua File", extension: "lua", body: "" },
    Template { label: "Zig File", extension: "zig", body: "" },
    Template { label: "Elixir File", extension: "ex", body: "" },
    Template { label: "Haskell File", extension: "hs", body: "module {name} where\n" },
    Template { label: "Shell Script", extension: "sh", body: "#!/bin/sh\n" },
    Template { label: "Markdown File", extension: "md", body: "" },
    Template { label: "JSON File", extension: "json", body: "{}\n" },
    Template { label: "YAML File", extension: "yaml", body: "" },
    Template { label: "TOML File", extension: "toml", body: "" },
];

impl Template {
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
