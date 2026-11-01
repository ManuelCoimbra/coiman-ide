//! What a project's folders are for, as JetBrains IDEs mark them: sources, tests,
//! resources and output. Those ask to be set up; here the conventions of each language say.

/// In rising order: a folder shown for several gets the highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FolderKind {
    Plain,
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
            FolderKind::Excluded => Some("Build output"),
            FolderKind::Source => Some("Source folder"),
            FolderKind::Resources => Some("Resources folder"),
            FolderKind::Test => Some("Test folder"),
            FolderKind::TestResources => Some("Test resources folder"),
        }
    }

    pub fn is_test(self) -> bool {
        matches!(self, FolderKind::Test | FolderKind::TestResources)
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

/// A folder's kind. `relative` is its path in the project, with either kind of slash;
/// `children` names what's in it, and whether each is a folder.
pub fn folder_kind(relative: &str, children: &[(&str, bool)]) -> FolderKind {
    let parts: Vec<&str> = relative.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    let Some((&name, above)) = parts.split_last() else { return FolderKind::Plain };
    let lower = name.to_lowercase();

    const OUTPUT: &[&str] = &[
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
    // `src/bin` is where Rust keeps programs.
    let is_output = |part: &str| {
        OUTPUT.contains(&part.to_lowercase().as_str()) || (matches!(part, "bin" | "obj") && !above.contains(&"src"))
    };
    if is_output(name) || above.iter().any(|p| OUTPUT.contains(&p.to_lowercase().as_str())) {
        return FolderKind::Excluded;
    }

    // Everything under a test folder is tests, so that deep packages still look it.
    if above.iter().any(|p| is_test_folder(p)) {
        return if is_test_data(name) { FolderKind::TestResources } else { FolderKind::Test };
    }
    if matches!(lower.as_str(), "testdata" | "test_data" | "test-data" | "fixtures" | "__snapshots__") {
        return FolderKind::TestResources;
    }
    if is_test_folder(name) || only_tests(relative, children) {
        return FolderKind::Test;
    }
    if lower == "resources" {
        return FolderKind::Resources;
    }
    // `src/main/java` is the root in Maven and Gradle, and then `src` itself is nothing.
    const LANGUAGES: &[&str] = &["java", "kotlin", "scala", "groovy", "go", "rust", "python", "cpp", "swift"];
    let in_main = above.len() >= 2
        && above[above.len() - 2] == "src"
        && (above[above.len() - 1] == "main" || above[above.len() - 1].ends_with("Main"));
    let splits_main = children.iter().any(|(child, dir)| *dir && (*child == "main" || is_test_folder(child)));
    if (lower == "src" && !splits_main)
        || (lower == "lib" && above.is_empty())
        || (in_main && LANGUAGES.contains(&lower.as_str()))
    {
        return FolderKind::Source;
    }
    FolderKind::Plain
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
    fn test_folders_of_each_language() {
        for path in [
            "src/test",
            "src/test/java",
            "src/test/java/demo",
            "tests",
            "tests/unit",
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
    fn a_folder_of_only_tests_is_a_test_folder() {
        assert_eq!(folder_kind("pkg", &[("a_test.go", false), ("b_test.go", false), ("data.json", false)]), Test);
        assert_eq!(folder_kind("pkg", &[("a_test.go", false), ("a.go", false)]), Plain);
        assert_eq!(folder_kind("pkg", &[("data.json", false)]), Plain);
    }

    #[test]
    fn sources_and_resources_where_the_build_tools_keep_them() {
        assert_eq!(folder_kind("src", &[("main.rs", false)]), Source);
        assert_eq!(folder_kind("src", &[("main", true), ("test", true)]), Plain);
        assert_eq!(folder_kind("src/main/java", &[]), Source);
        assert_eq!(folder_kind("app/src/main/kotlin", &[]), Source);
        assert_eq!(folder_kind("src/main/resources", &[]), Resources);
        assert_eq!(folder_kind("src/main/java/demo", &[]), Plain);
        assert_eq!(folder_kind("docs", &[]), Plain);
    }

    #[test]
    fn output_is_dimmed_except_rust_programs() {
        for path in ["build", "dist", "bin", "obj", "__pycache__", "build/classes", ".venv"] {
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
    fn the_highest_kind_speaks_for_a_compact_row() {
        assert_eq!(Plain.max(Source).max(Test), Test);
        assert!(TestResources.is_test() && !Source.is_test());
    }
}
