//! A project overview, as Markdown: what the README says first, the languages and their lines,
//! the build files, the top-level folders, tests, the largest files and a Git summary (from
//! `git`, when the folder is a repository).

use std::collections::BTreeMap;
use std::path::Path;

/// A language by file extension, for counting; None for files that aren't code or docs.
fn language(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "Rust",
        "py" | "pyi" => "Python",
        "js" | "mjs" | "cjs" | "jsx" => "JavaScript",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "cs" => "C#",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => "C++",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "dart" => "Dart",
        "lua" => "Lua",
        "zig" => "Zig",
        "sh" | "bash" => "Shell",
        "ps1" => "PowerShell",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "sql" => "SQL",
        "md" | "markdown" | "rst" | "adoc" => "Docs",
        "toml" | "yaml" | "yml" | "json" | "xml" | "ini" => "Config",
        _ => return None,
    })
}

const BUILD_FILES: &[(&str, &str)] = &[
    ("Cargo.toml", "Cargo"),
    ("package.json", "npm"),
    ("pyproject.toml", "Python (pyproject)"),
    ("requirements.txt", "pip"),
    ("go.mod", "Go modules"),
    ("pom.xml", "Maven"),
    ("build.gradle", "Gradle"),
    ("build.gradle.kts", "Gradle"),
    ("CMakeLists.txt", "CMake"),
    ("Makefile", "Make"),
    ("meson.build", "Meson"),
    ("Gemfile", "Bundler"),
    ("composer.json", "Composer"),
    ("Dockerfile", "Docker"),
];

fn is_test(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel.split('/').any(|c| c == "tests" || c == "test" || c == "__tests__" || c == "spec")
        || name.starts_with("test_")
        || name.contains("_test.")
        || name.contains(".test.")
        || name.contains(".spec.")
}

/// The README's title and first paragraph, if there's a README.
fn readme(root: &Path) -> Option<(String, String)> {
    let path = ["README.md", "readme.md", "README.markdown", "README.rst", "README.txt", "README"]
        .iter()
        .map(|n| root.join(n))
        .find(|p| p.is_file())?;
    let text = std::fs::read_to_string(&path).ok()?;
    let mut title = String::new();
    let mut para = Vec::new();
    for line in text.lines().map(str::trim) {
        if title.is_empty() && line.starts_with('#') {
            title = line.trim_start_matches('#').trim().to_owned();
            continue;
        }
        // Badges and pictures say nothing here.
        if line.starts_with('[') || line.starts_with("![") || line.starts_with('<') || line.starts_with('#') {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if line.is_empty() {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push(line);
    }
    Some((title, para.join(" ")))
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = crate::language::lsp::discovery::quiet_command(Path::new("git"))
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// What the files say, counted.
#[derive(Default)]
struct Counts {
    files: usize,
    /// Language: (files, lines).
    languages: BTreeMap<&'static str, (usize, usize)>,
    /// Top-level folder ("" for files at the root): files.
    folders: BTreeMap<String, usize>,
    tests: usize,
    /// (lines, path), every code file.
    sizes: Vec<(usize, String)>,
}

fn count(root: &Path) -> Counts {
    let mut c = Counts::default();
    for entry in super::project_files(root, false) {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        c.files += 1;
        let top = rel.split_once('/').map_or(String::new(), |(t, _)| t.to_owned());
        *c.folders.entry(top).or_default() += 1;
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(lang) = language(&ext) else { continue };
        if entry.metadata().map_or(true, |m| m.len() > 4 * 1024 * 1024) {
            continue;
        }
        let lines = std::fs::read(path).map_or(0, |b| b.iter().filter(|&&x| x == b'\n').count());
        let e = c.languages.entry(lang).or_default();
        e.0 += 1;
        e.1 += lines;
        if lang != "Docs" && lang != "Config" {
            if is_test(&rel) {
                c.tests += 1;
            }
            c.sizes.push((lines, rel));
        }
    }
    c
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The overview of the project at `root`, as Markdown.
pub fn overview(root: &Path) -> String {
    let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    let mut md = format!("# {name}: overview\n\n");
    if let Some((title, para)) = readme(root) {
        if !title.is_empty() && title != name {
            md.push_str(&format!("**{title}**"));
            md.push_str(if para.is_empty() { "\n\n" } else { ": " });
        }
        if !para.is_empty() {
            md.push_str(&format!("{para}\n\n"));
        }
    }
    let c = count(root);
    let code_lines: usize =
        c.languages.iter().filter(|(l, _)| **l != "Docs" && **l != "Config").map(|(_, (_, n))| n).sum();
    md.push_str(&format!(
        "{} files; {} lines of code; {} test files.\n\n",
        thousands(c.files),
        thousands(code_lines),
        thousands(c.tests)
    ));

    let builds: Vec<String> =
        BUILD_FILES.iter().filter(|(f, _)| root.join(f).is_file()).map(|(f, what)| format!("{what} (`{f}`)")).collect();
    if !builds.is_empty() {
        md.push_str(&format!("**Built with:** {}.\n\n", builds.join(", ")));
    }

    if !c.languages.is_empty() {
        md.push_str("## Languages\n\n| Language | Files | Lines |\n|---|---:|---:|\n");
        let mut langs: Vec<_> = c.languages.iter().collect();
        langs.sort_by_key(|l| std::cmp::Reverse(l.1.1));
        for (lang, (files, lines)) in langs {
            md.push_str(&format!("| {lang} | {} | {} |\n", thousands(*files), thousands(*lines)));
        }
        md.push('\n');
    }

    if c.folders.len() > 1 {
        md.push_str("## Folders\n\n| Folder | Files |\n|---|---:|\n");
        let mut folders: Vec<_> = c.folders.iter().collect();
        folders.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (folder, files) in folders.into_iter().take(12) {
            let shown = if folder.is_empty() { "(top level)".to_owned() } else { format!("`{folder}/`") };
            md.push_str(&format!("| {shown} | {} |\n", thousands(*files)));
        }
        md.push('\n');
    }

    let mut sizes = c.sizes;
    if !sizes.is_empty() {
        sizes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        md.push_str("## Largest files\n\n");
        for (lines, path) in sizes.iter().take(8) {
            md.push_str(&format!("- `{path}`: {} lines\n", thousands(*lines)));
        }
        md.push('\n');
    }

    if let Some(branch) = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        md.push_str("## Git\n\n");
        md.push_str(&format!("- Branch: `{branch}`\n"));
        if let Some(n) = git(root, &["rev-list", "--count", "HEAD"]) {
            md.push_str(&format!("- Commits: {n}\n"));
        }
        if let Some(first) = git(root, &["log", "--reverse", "--format=%as", "--max-parents=0", "HEAD"]) {
            let first = first.lines().next().unwrap_or_default();
            let last = git(root, &["log", "-1", "--format=%as"]).unwrap_or_default();
            md.push_str(&format!("- From {first} to {last}\n"));
        }
        if let Some(authors) = git(root, &["shortlog", "-sn", "HEAD"]) {
            md.push_str(&format!("- Authors: {}\n", authors.lines().count()));
        }
        if let Some(tag) = git(root, &["describe", "--tags", "--abbrev=0"]) {
            md.push_str(&format!("- Last tag: `{tag}`\n"));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_of_a_small_project() {
        let dir = std::env::temp_dir().join(format!("coiman-overview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(dir.join("README.md"), "# Tool\n\n[![ci](x)](y)\n\nDoes one thing\nwell.\n\nMore.\n").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"tool\"\n").unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {\n}\n// end\n").unwrap();
        std::fs::write(dir.join("tests/it.rs"), "#[test]\nfn t() {}\n").unwrap();
        let md = overview(&dir);
        assert!(md.contains("**Tool**: Does one thing well."), "{md}");
        assert!(md.contains("5 lines of code; 1 test files"), "{md}");
        assert!(md.contains("Cargo (`Cargo.toml`)"), "{md}");
        assert!(md.contains("| Rust | 2 | 5 |"), "{md}");
        assert!(md.contains("- `src/main.rs`: 3 lines"), "{md}");
        assert_eq!(thousands(1234567), "1,234,567");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
