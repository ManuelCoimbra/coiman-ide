//! Which icon a file gets, from Material Icon Theme (MIT, see `data/file_icons_LICENSE.txt`),
//! and whether it holds tests, which any language's icon can show.

use std::collections::HashMap;
use std::sync::OnceLock;

const PACK: &str = include_str!("../data/file_icons.txt");
const DEFAULT_ICON: &str = "file";

#[derive(Default)]
struct Icons {
    names: HashMap<&'static str, &'static str>,
    extensions: HashMap<&'static str, &'static str>,
    light_names: HashMap<&'static str, &'static str>,
    light_extensions: HashMap<&'static str, &'static str>,
    svgs: HashMap<&'static str, &'static str>,
}

fn icons() -> &'static Icons {
    static ICONS: OnceLock<Icons> = OnceLock::new();
    ICONS.get_or_init(|| {
        let mut all = Icons::default();
        for line in PACK.lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(tag), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let table = match tag {
                "N" => &mut all.names,
                "E" => &mut all.extensions,
                "n" => &mut all.light_names,
                "e" => &mut all.light_extensions,
                _ => &mut all.svgs,
            };
            table.insert(key, value);
        }
        all
    })
}

/// A path's last part, for either kind of slash.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The icon for a file, by its whole name, then its longest known extension:
/// `a.test.ts` tries `test.ts` before `ts`.
pub fn icon_for(path: &str, light: bool) -> &'static str {
    let lower = file_name(path).to_lowercase();
    let all = icons();
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
    DEFAULT_ICON
}

/// The SVG of an icon from [`icon_for`].
pub fn svg(icon: &str) -> Option<&'static str> {
    icons().svgs.get(icon).copied()
}

/// Every icon packed, by name.
pub fn all_svgs() -> impl Iterator<Item = (&'static str, &'static str)> {
    icons().svgs.iter().map(|(name, svg)| (*name, *svg))
}

/// File types that hold code, so a folder of tests says something about them.
const CODE: &[&str] = &[
    "rs", "py", "pyw", "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "java", "kt", "kts", "scala", "groovy",
    "go", "c", "h", "cc", "cpp", "cxx", "hpp", "cs", "fs", "vb", "rb", "php", "lua", "pl", "pm", "r", "jl", "ex",
    "exs", "erl", "hs", "ml", "clj", "cljs", "cljc", "lisp", "swift", "dart", "zig", "nim", "sh", "ps1", "lean",
    "agda",
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
    by_name || by_class || folders.into_iter().any(crate::project_tree::is_test_folder)
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
