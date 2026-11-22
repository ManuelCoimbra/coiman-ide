//! A project overview, as Markdown: what the README says first, the languages and their lines,
//! the build files, the top-level folders, tests, the largest files and a Git summary (from
//! `git`, when the folder is a repository).

use std::collections::BTreeMap;

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

/// Where a README may be.
pub const READMES: [&str; 6] = ["README.md", "readme.md", "README.markdown", "README.rst", "README.txt", "README"];

/// A README's title and first paragraph.
pub fn readme(text: &str) -> (String, String) {
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
    (title, para.join(" "))
}

/// What the files say, counted.
#[derive(Default)]
pub struct Counts {
    files: usize,
    /// Language: (files, lines).
    languages: BTreeMap<&'static str, (usize, usize)>,
    /// Top-level folder ("" for files at the root): files.
    folders: BTreeMap<String, usize>,
    tests: usize,
    /// (lines, path), every code file.
    sizes: Vec<(usize, String)>,
}

impl Counts {
    /// One file of the project, with its text when it is a language counted.
    pub fn add(&mut self, rel: &str, text: Option<&str>) {
        self.files += 1;
        let top = rel.split_once('/').map_or(String::new(), |(t, _)| t.to_owned());
        *self.folders.entry(top).or_default() += 1;
        let Some(text) = text else { return };
        let Some(lang) = language(&extension(rel)) else { return };
        let lines = text.bytes().filter(|&b| b == b'\n').count();
        let e = self.languages.entry(lang).or_default();
        e.0 += 1;
        e.1 += lines;
        if lang != "Docs" && lang != "Config" {
            if is_test(rel) {
                self.tests += 1;
            }
            self.sizes.push((lines, rel.to_owned()));
        }
    }
}

/// A file's extension, lower case.
pub fn extension(rel: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Whether the file's text is worth reading for the counts.
pub fn counted(rel: &str) -> bool {
    language(&extension(rel)).is_some()
}

/// What `git` says of the repository.
#[derive(Default)]
pub struct GitFacts {
    pub branch: Option<String>,
    pub commits: Option<String>,
    pub first: Option<String>,
    pub last: Option<String>,
    pub authors: Option<usize>,
    pub tag: Option<String>,
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

/// The overview, as Markdown: the project's name, its README's text, the counts, the build
/// files there (by name), and what Git says.
pub fn overview(name: &str, readme_text: Option<&str>, c: Counts, files: &[String], git: &GitFacts) -> String {
    let mut md = format!("# {name}: overview\n\n");
    if let Some((title, para)) = readme_text.map(readme) {
        if !title.is_empty() && title != name {
            md.push_str(&format!("**{title}**"));
            md.push_str(if para.is_empty() { "\n\n" } else { ": " });
        }
        if !para.is_empty() {
            md.push_str(&format!("{para}\n\n"));
        }
    }
    let code_lines: usize =
        c.languages.iter().filter(|(l, _)| **l != "Docs" && **l != "Config").map(|(_, (_, n))| n).sum();
    md.push_str(&format!(
        "{} files; {} lines of code; {} test files.\n\n",
        thousands(c.files),
        thousands(code_lines),
        thousands(c.tests)
    ));

    let builds: Vec<String> = BUILD_FILES
        .iter()
        .filter(|(f, _)| files.iter().any(|x| x == f))
        .map(|(f, what)| format!("{what} (`{f}`)"))
        .collect();
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

    if let Some(branch) = &git.branch {
        md.push_str("## Git\n\n");
        md.push_str(&format!("- Branch: `{branch}`\n"));
        if let Some(n) = &git.commits {
            md.push_str(&format!("- Commits: {n}\n"));
        }
        if let Some(first) = &git.first {
            let last = git.last.clone().unwrap_or_default();
            md.push_str(&format!("- From {first} to {last}\n"));
        }
        if let Some(n) = git.authors {
            md.push_str(&format!("- Authors: {n}\n"));
        }
        if let Some(tag) = &git.tag {
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
        let mut c = Counts::default();
        let readme_text = "# Tool\n\n[![ci](x)](y)\n\nDoes one thing\nwell.\n\nMore.\n";
        for (rel, text) in [
            ("README.md", readme_text),
            ("Cargo.toml", "[package]\nname = \"tool\"\n"),
            ("src/main.rs", "fn main() {\n}\n// end\n"),
            ("tests/it.rs", "#[test]\nfn t() {}\n"),
        ] {
            c.add(rel, counted(rel).then_some(text));
        }
        let files = ["README.md".to_owned(), "Cargo.toml".to_owned()];
        let git = GitFacts {
            branch: Some("main".into()),
            commits: Some("12".into()),
            authors: Some(2),
            ..Default::default()
        };
        let md = overview("tool", Some(readme_text), c, &files, &git);
        assert!(md.contains("**Tool**: Does one thing well."), "{md}");
        assert!(md.contains("5 lines of code; 1 test files"), "{md}");
        assert!(md.contains("Cargo (`Cargo.toml`)"), "{md}");
        assert!(md.contains("| Rust | 2 | 5 |"), "{md}");
        assert!(md.contains("- `src/main.rs`: 3 lines"), "{md}");
        assert!(md.contains("- Branch: `main`\n- Commits: 12\n- Authors: 2\n"), "{md}");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
