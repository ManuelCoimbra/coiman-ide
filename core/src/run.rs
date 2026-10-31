//! "Run Program": how a source file is actually executed, per language. Each
//! language has its own rules: Java needs the class with `main` and its
//! package, Go a `package main`, C# its project, Rust the Cargo target, and
//! compiled languages a build step first. Nothing here runs anything; the
//! editor runs the resulting command in a terminal tab (trusted folders only).

use std::path::{Path, PathBuf};

use tree_sitter::{Node, Parser};

use crate::cargo::CargoPackage;
use crate::lsp::servers::which;
use crate::syntax::Lang;

/// A command line for the platform's shell, and where to run it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunPlan {
    pub title: String,
    pub command: String,
    pub cwd: PathBuf,
}

/// What Run Program should do for a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Run(RunPlan),
    /// Several programs fit (Java classes with `main`, Cargo binaries).
    Choose(Vec<RunPlan>),
}

/// `text` as one argument for the shell ([`crate`]'s terminals use PowerShell on Windows).
fn quote(text: &str) -> String {
    if cfg!(windows) { format!("'{}'", text.replace('\'', "''")) } else { format!("'{}'", text.replace('\'', r"'\''")) }
}

fn q(path: &Path) -> String {
    quote(&path.to_string_lossy())
}

/// Calls a program named by a quoted path.
fn call(path: &Path) -> String {
    if cfg!(windows) { format!("& {}", q(path)) } else { q(path) }
}

/// `first`, then `second` if it worked (PowerShell 5 has no `&&`).
fn then(first: &str, second: &str) -> String {
    if cfg!(windows) { format!("{first}; if ($?) {{ {second} }}") } else { format!("{first} && {second}") }
}

fn exe_name(stem: &str) -> String {
    format!("{stem}{}", std::env::consts::EXE_SUFFIX)
}

/// A folder for build output, outside the project.
fn build_dir(stem: &str) -> PathBuf {
    std::env::temp_dir().join("rust-code-run").join(stem)
}

fn need(tool: &str, what: &str) -> Result<(), String> {
    if which(tool).is_some() { Ok(()) } else { Err(format!("{what} needs `{tool}`, which isn't on PATH")) }
}

fn stem_of(file: &Path) -> String {
    file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn name_of(file: &Path) -> String {
    file.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Folders from the file's up to the open folder (or the filesystem root).
fn upward<'a>(file: &'a Path, root: Option<&'a Path>) -> impl Iterator<Item = &'a Path> {
    let inside = root.filter(|r| file.starts_with(r));
    let mut done = false;
    file.ancestors().skip(1).take_while(move |d| {
        let keep = !done;
        done = done || inside.is_some_and(|r| *d == r);
        keep
    })
}

fn find_up(file: &Path, root: Option<&Path>, name: &str) -> Option<PathBuf> {
    upward(file, root).find(|d| d.join(name).exists()).map(Path::to_path_buf)
}

/// A file with this extension in the nearest folder above `file` that has one.
fn find_ext_up(file: &Path, root: Option<&Path>, ext: &str) -> Option<PathBuf> {
    upward(file, root).find_map(|d| {
        let mut found: Vec<PathBuf> = std::fs::read_dir(d)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
            .collect();
        found.sort();
        found.into_iter().next()
    })
}

fn parse(lang: Lang, source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(&lang.language()).ok()?;
    parser.parse(source, None)
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn children<'t>(node: Node<'t>) -> Vec<Node<'t>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

// ----- Java ---------------------------------------------------------------------------

/// Where a Java file's program starts.
#[derive(Debug, PartialEq, Eq)]
enum JavaMain {
    /// The class holding `main`, with its package.
    Class { package: String, class: String },
    /// Java 25 compact source file: `void main()` with no class.
    Compact,
}

fn has_main_method(body: Node, source: &str) -> bool {
    children(body).into_iter().any(|m| {
        m.kind() == "method_declaration" && m.child_by_field_name("name").is_some_and(|n| text(n, source) == "main")
    })
}

fn java_main(source: &str) -> Option<JavaMain> {
    let tree = parse(Lang::Java, source)?;
    let root = tree.root_node();
    let mut package = String::new();
    for node in children(root) {
        match node.kind() {
            "package_declaration" => {
                package = children(node)
                    .into_iter()
                    .find(|c| matches!(c.kind(), "scoped_identifier" | "identifier"))
                    .map(|c| text(c, source).to_owned())
                    .unwrap_or_default();
            }
            "class_declaration" | "enum_declaration" | "record_declaration" | "interface_declaration" => {
                let body = node.child_by_field_name("body")?;
                if has_main_method(body, source)
                    && let Some(name) = node.child_by_field_name("name")
                {
                    return Some(JavaMain::Class { package, class: text(name, source).to_owned() });
                }
            }
            "method_declaration" if node.child_by_field_name("name").is_some_and(|n| text(n, source) == "main") => {
                return Some(JavaMain::Compact);
            }
            _ => {}
        }
    }
    None
}

/// The folder a package's sources start in: `src/main/java` for `com/app/Main.java`.
fn java_source_root(file: &Path, package: &str) -> PathBuf {
    let mut dir = file.parent().unwrap_or(file).to_path_buf();
    for part in package.split('.').filter(|p| !p.is_empty()).rev() {
        if dir.file_name().is_some_and(|n| n == part) {
            dir.pop();
        } else {
            return file.parent().unwrap_or(file).to_path_buf();
        }
    }
    dir
}

fn java_plan(file: &Path, main: &JavaMain, root: Option<&Path>) -> Result<RunPlan, String> {
    let (package, class) = match main {
        JavaMain::Compact => {
            need("java", "Java")?;
            return Ok(RunPlan {
                title: stem_of(file),
                command: format!("java {}", q(file)),
                cwd: file.parent().unwrap_or(file).to_path_buf(),
            });
        }
        JavaMain::Class { package, class } => (package, class),
    };
    let full = if package.is_empty() { class.clone() } else { format!("{package}.{class}") };
    // A build tool knows the dependencies.
    if let Some(dir) = find_up(file, root, "pom.xml") {
        let wrapper = dir.join(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" });
        let mvn = if wrapper.exists() {
            call(&wrapper)
        } else {
            need("mvn", "This Maven project")?;
            "mvn".to_owned()
        };
        let main_arg = quote(&format!("-Dexec.mainClass={full}"));
        return Ok(RunPlan {
            title: class.clone(),
            command: format!("{mvn} -q compile exec:java {main_arg}"),
            cwd: dir,
        });
    }
    let gradle = ["build.gradle", "build.gradle.kts"].iter().find_map(|f| find_up(file, root, f));
    if let Some(dir) = gradle {
        let wrapper = dir.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
        let tool = if wrapper.exists() {
            call(&wrapper)
        } else {
            need("gradle", "This Gradle project")?;
            "gradle".to_owned()
        };
        return Ok(RunPlan { title: class.clone(), command: format!("{tool} run"), cwd: dir });
    }
    need("javac", "Compiling Java")?;
    let sources = java_source_root(file, package);
    let out = build_dir(&format!("java-{class}"));
    let compile = format!("javac -encoding UTF-8 -d {} -sourcepath {} {}", q(&out), q(&sources), q(file));
    let run = format!("java -cp {} {}", q(&out), quote(&full));
    Ok(RunPlan { title: class.clone(), command: then(&compile, &run), cwd: sources })
}

fn java_files(dir: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(dir)
        .build()
        .filter_map(Result::ok)
        .map(|e| e.into_path())
        .filter(|p| p.extension().is_some_and(|x| x == "java"))
        .take(5000)
        .collect()
}

fn java(file: &Path, source: &str, root: Option<&Path>) -> Result<Outcome, String> {
    if let Some(main) = java_main(source) {
        return java_plan(file, &main, root).map(Outcome::Run);
    }
    // This file isn't a program: look for the classes that are.
    let package = java_package(source);
    let search =
        root.filter(|r| file.starts_with(r)).map_or_else(|| java_source_root(file, &package), Path::to_path_buf);
    let mut plans = Vec::new();
    for other in java_files(&search) {
        let Ok(text) = std::fs::read_to_string(&other) else { continue };
        if let Some(main) = java_main(&text) {
            let mut plan = java_plan(&other, &main, root)?;
            if let JavaMain::Class { package, class } = &main
                && !package.is_empty()
            {
                plan.title = format!("{package}.{class}");
            }
            plans.push(plan);
        }
    }
    plans.sort_by(|a, b| a.title.cmp(&b.title));
    match plans.len() {
        0 => Err("No class with a main method was found".to_owned()),
        1 => Ok(Outcome::Run(plans.remove(0))),
        _ => Ok(Outcome::Choose(plans)),
    }
}

fn java_package(source: &str) -> String {
    let Some(tree) = parse(Lang::Java, source) else { return String::new() };
    children(tree.root_node())
        .into_iter()
        .find(|n| n.kind() == "package_declaration")
        .and_then(|n| children(n).into_iter().next().map(|c| text(c, source).to_owned()))
        .unwrap_or_default()
}

// ----- C and C++ ----------------------------------------------------------------------

/// Whether a C or C++ file defines `main`.
fn c_has_main(lang: Lang, source: &str) -> bool {
    let Some(tree) = parse(lang, source) else { return false };
    children(tree.root_node()).into_iter().filter(|n| n.kind() == "function_definition").any(|f| {
        let mut declarator = f.child_by_field_name("declarator");
        while let Some(d) = declarator {
            if d.kind() == "identifier" {
                return text(d, source) == "main";
            }
            declarator = d.child_by_field_name("declarator");
        }
        false
    })
}

fn c_family(file: &Path, source: &str, root: Option<&Path>, cpp: bool) -> Result<Outcome, String> {
    let lang = if cpp { Lang::Cpp } else { Lang::C };
    if !c_has_main(lang, source) {
        return Err(format!("{} has no main function", name_of(file)));
    }
    for (marker, tool) in [("CMakeLists.txt", "CMake"), ("Makefile", "Make"), ("meson.build", "Meson")] {
        if find_up(file, root, marker).is_some() {
            return Err(format!(
                "This project builds with {tool}, which decides the program's name: add it in Run: Open Configurations, then press Ctrl+F5"
            ));
        }
    }
    let compilers: &[&str] = if cpp { &["g++", "clang++"] } else { &["gcc", "clang"] };
    let Some(compiler) = compilers.iter().find(|c| which(c).is_some()) else {
        return Err(format!("Compiling needs {}, not found on PATH", compilers.join(" or ")));
    };
    let stem = stem_of(file);
    let out = build_dir(&format!("native-{stem}"));
    let exe = out.join(exe_name(&stem));
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let compile = if cfg!(windows) {
        format!("New-Item -ItemType Directory -Force {} | Out-Null; {compiler} {} -o {}", q(&out), q(file), q(&exe))
    } else {
        format!("mkdir -p {} && {compiler} {} -o {}", q(&out), q(file), q(&exe))
    };
    Ok(Outcome::Run(RunPlan { title: stem, command: then(&compile, &call(&exe)), cwd: dir }))
}

// ----- Rust and Go ---------------------------------------------------------------------

/// A path in the form paths are compared in: resolved, no `\\\\?\\`, any case on Windows.
fn comparable(path: &Path) -> PathBuf {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    PathBuf::from(if cfg!(windows) { text.to_lowercase() } else { text.to_owned() })
}

fn cargo_run(package: &CargoPackage, flag: &str, name: &str, title: String) -> RunPlan {
    let command = format!("cargo run -p {} --{flag} {}", quote(&package.name), quote(name));
    RunPlan { title, command, cwd: package.dir.clone() }
}

/// Runs one of `found`, asking when there are several.
fn one_or_choose(mut found: Vec<RunPlan>) -> Option<Outcome> {
    match found.len() {
        0 => None,
        1 => Some(Outcome::Run(found.remove(0))),
        _ => Some(Outcome::Choose(found)),
    }
}

/// What running `file` means given what Cargo says the workspace holds: the
/// file's own target when it starts one, else the program of its package.
fn rust_target(file: &Path, packages: &[CargoPackage]) -> Result<Outcome, String> {
    let name = name_of(file);
    let at = comparable(file);
    let owner = packages
        .iter()
        .map(|p| (p, comparable(&p.dir)))
        .filter(|(_, dir)| at.starts_with(dir))
        .max_by_key(|(_, dir)| dir.components().count())
        .map(|(p, _)| p);
    let Some(package) = owner else {
        // A workspace's own manifest belongs to no member.
        let found = packages
            .iter()
            .flat_map(|p| p.binaries().map(move |t| (p, t)))
            .map(|(p, t)| cargo_run(p, "bin", &t.name, format!("{} ({})", t.name, p.name)))
            .collect();
        return one_or_choose(found).ok_or_else(|| "This Cargo workspace has no program to run".to_owned());
    };
    let own = package.targets.iter().find(|t| comparable(&t.src_path) == at);
    if let Some(target) = own {
        let quoted = (quote(&package.name), quote(&target.name));
        let tests = |tool: &str, flag: &str| {
            Outcome::Run(RunPlan {
                title: target.name.clone(),
                command: format!("cargo {tool} -p {} --{flag} {}", quoted.0, quoted.1),
                cwd: package.dir.clone(),
            })
        };
        if target.is("bin") {
            return Ok(Outcome::Run(cargo_run(package, "bin", &target.name, target.name.clone())));
        }
        if target.is("example") {
            if target.crate_types.iter().any(|c| c == "bin") {
                return Ok(Outcome::Run(cargo_run(package, "example", &target.name, target.name.clone())));
            }
            return Err(format!("The example {} is a library, so it can't run", target.name));
        }
        if target.is("test") {
            return Ok(tests("test", "test"));
        }
        if target.is("bench") {
            return Ok(tests("bench", "bench"));
        }
        if target.is("custom-build") {
            return Err(format!("{name} runs by itself whenever Cargo builds {}", package.name));
        }
    }
    let default = package.default_run.as_deref();
    if let Some(bin) = package.binaries().find(|t| Some(t.name.as_str()) == default) {
        return Ok(Outcome::Run(cargo_run(package, "bin", &bin.name, bin.name.clone())));
    }
    let found = package.binaries().map(|t| cargo_run(package, "bin", &t.name, t.name.clone())).collect();
    one_or_choose(found).ok_or_else(|| {
        format!(
            "{} is a library, so it has no program to run. Add src/main.rs to give it one, or run its tests",
            package.name
        )
    })
}

fn rust(file: &Path, root: Option<&Path>) -> Result<Outcome, String> {
    let stem = stem_of(file);
    let Some(package) = find_up(file, root, "Cargo.toml") else {
        need("rustc", "Rust")?;
        let out = build_dir(&format!("rust-{stem}"));
        let exe = out.join(exe_name(&stem));
        let dir = file.parent().unwrap_or(file).to_path_buf();
        let compile = if cfg!(windows) {
            format!("New-Item -ItemType Directory -Force {} | Out-Null; rustc {} -o {}", q(&out), q(file), q(&exe))
        } else {
            format!("mkdir -p {} && rustc {} -o {}", q(&out), q(file), q(&exe))
        };
        return Ok(Outcome::Run(RunPlan { title: stem, command: then(&compile, &call(&exe)), cwd: dir }));
    };
    if let Some(packages) = crate::cargo::packages(&package) {
        return rust_target(file, &packages);
    }
    // Cargo can't read the manifest: go by the file's place, and let it report why.
    let parent = file.parent().map(name_of).unwrap_or_default();
    let command = match parent.as_str() {
        "bin" => format!("cargo run --bin {}", quote(&stem)),
        "examples" => format!("cargo run --example {}", quote(&stem)),
        _ => "cargo run".to_owned(),
    };
    let title = if command == "cargo run" {
        package.file_name().map_or(stem, |n| n.to_string_lossy().into_owned())
    } else {
        stem
    };
    Ok(Outcome::Run(RunPlan { title, command, cwd: package }))
}

fn go(file: &Path, source: &str, root: Option<&Path>) -> Result<Outcome, String> {
    need("go", "Go")?;
    if name_of(file).ends_with("_test.go") {
        let dir = file.parent().unwrap_or(file).to_path_buf();
        return Ok(Outcome::Run(RunPlan {
            title: format!("{} tests", stem_of(file)),
            command: "go test .".into(),
            cwd: dir,
        }));
    }
    let package = source
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("package "))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or_default();
    if package != "main" {
        return Err(format!("Only package main runs; {} is in package {package}", name_of(file)));
    }
    let dir = file.parent().unwrap_or(file).to_path_buf();
    // In a module, the whole package; otherwise this file alone.
    let command =
        if find_up(file, root, "go.mod").is_some() { "go run .".to_owned() } else { format!("go run {}", q(file)) };
    Ok(Outcome::Run(RunPlan { title: stem_of(file), command, cwd: dir }))
}

// ----- interpreted languages -----------------------------------------------------------

/// The project's virtual environment, else the system's Python.
fn python_program(file: &Path, root: Option<&Path>) -> Result<String, String> {
    let relative = if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" };
    for dir in upward(file, root) {
        for venv in [".venv", "venv", "env"] {
            let python = dir.join(venv).join(relative);
            if python.is_file() {
                return Ok(call(&python));
            }
        }
    }
    ["python", "python3", "py"]
        .iter()
        .find(|p| which(p).is_some())
        .map(|p| (*p).to_owned())
        .ok_or_else(|| "Python isn't on PATH".to_owned())
}

/// A project's own TypeScript runner, else Node (which runs TypeScript since 22.18).
fn typescript_program(file: &Path, root: Option<&Path>) -> Result<String, String> {
    let tsx = if cfg!(windows) { "node_modules/.bin/tsx.cmd" } else { "node_modules/.bin/tsx" };
    if let Some(dir) = find_up(file, root, tsx) {
        return Ok(call(&dir.join(tsx)));
    }
    need("node", "TypeScript")?;
    Ok("node".to_owned())
}

/// File types `plan` has a way to run.
const RUNNABLE_TYPES: &[&str] = &[
    "py", "pyw", "js", "mjs", "cjs", "ts", "mts", "cts", "java", "c", "cpp", "cc", "cxx", "c++", "rs", "go", "cs",
    "sh", "bash", "zsh", "ps1", "bat", "cmd", "rb", "php", "lua", "pl", "r", "dart", "swift", "kts", "kt", "html",
    "htm",
];

/// Whether a file could run, judged from its name alone: cheap enough for every
/// frame. Whether it really does takes `plan`.
pub fn may_run(file: &Path) -> bool {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    name_of(file) == "Cargo.toml" || (RUNNABLE_TYPES.contains(&ext.as_str()) && never_runs(file, &ext).is_none())
}

/// Why a kind of file never runs, when it's certain.
fn never_runs(file: &Path, ext: &str) -> Option<String> {
    let name = name_of(file);
    let lower = name.to_lowercase();
    if lower.ends_with(".d.ts") || lower.ends_with(".d.mts") || lower.ends_with(".d.cts") || ext == "pyi" {
        return Some(format!("{name} only declares types: it has no code to run"));
    }
    Some(match ext {
        "h" | "hpp" | "hh" | "hxx" | "inl" => {
            format!("{name} is a header: it runs as part of the files that include it")
        }
        "css" | "scss" | "sass" | "less" => {
            format!("{name} is a stylesheet: it applies through the page that loads it")
        }
        "json" | "jsonc" | "toml" | "yaml" | "yml" | "xml" | "lock" | "ini" | "cfg" | "conf" | "csv" | "env" => {
            format!("{name} holds data or settings: it isn't a program")
        }
        "md" | "markdown" | "txt" | "rst" | "log" => format!("{name} is a document: it isn't a program"),
        _ => return None,
    })
}

/// Works out how to run `file`, whose current text is `source`. `root` is the open folder.
pub fn plan(file: &Path, source: &str, root: Option<&Path>) -> Result<Outcome, String> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    // Cargo's manifest stands for its package's program.
    if name_of(file) == "Cargo.toml" {
        return rust(file, root);
    }
    if let Some(why) = never_runs(file, &ext) {
        return Err(why);
    }
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let simple = |program: String| {
        Ok(Outcome::Run(RunPlan {
            title: name_of(file),
            command: format!("{program} {}", q(file)),
            cwd: root.filter(|r| file.starts_with(r)).map_or_else(|| dir.clone(), Path::to_path_buf),
        }))
    };
    let tool = |name: &str, what: &str| need(name, what).map(|()| name.to_owned());
    match ext.as_str() {
        "py" | "pyw" => simple(python_program(file, root)?),
        "js" | "mjs" | "cjs" => simple(tool("node", "JavaScript")?),
        "ts" | "mts" | "cts" => simple(typescript_program(file, root)?),
        "java" => java(file, source, root),
        "c" => c_family(file, source, root, false),
        "cpp" | "cc" | "cxx" | "c++" => c_family(file, source, root, true),
        "rs" => rust(file, root),
        "go" => go(file, source, root),
        "cs" => {
            need("dotnet", "C#")?;
            // A project if there is one; otherwise .NET 10 runs a single file.
            match find_ext_up(file, root, "csproj") {
                Some(project) => Ok(Outcome::Run(RunPlan {
                    title: stem_of(&project),
                    command: format!("dotnet run --project {}", q(&project)),
                    cwd: project.parent().unwrap_or(&dir).to_path_buf(),
                })),
                None => simple("dotnet run".to_owned()),
            }
        }
        "sh" | "bash" => simple(tool("bash", "This script")?),
        "zsh" => simple(tool("zsh", "This script")?),
        "ps1" => simple("powershell -NoProfile -ExecutionPolicy Bypass -File".to_owned()),
        "bat" | "cmd" => simple("cmd /c".to_owned()),
        "rb" => simple(tool("ruby", "Ruby")?),
        "php" => simple(tool("php", "PHP")?),
        "lua" => simple(tool("lua", "Lua")?),
        "pl" => simple(tool("perl", "Perl")?),
        "r" => simple(tool("Rscript", "R")?),
        "dart" => simple(format!("{} run", tool("dart", "Dart")?)),
        "swift" => simple(tool("swift", "Swift")?),
        "kts" => simple(format!("{} -script", tool("kotlinc", "Kotlin scripts")?)),
        "kt" => {
            need("kotlinc", "Kotlin")?;
            let jar = build_dir("kotlin").join(format!("{}.jar", stem_of(file)));
            let compile = format!("kotlinc {} -include-runtime -d {}", q(file), q(&jar));
            let run = format!("java -jar {}", q(&jar));
            Ok(Outcome::Run(RunPlan { title: stem_of(file), command: then(&compile, &run), cwd: dir }))
        }
        // A page opens in the default browser.
        "html" | "htm" => {
            let opener = match std::env::consts::OS {
                "windows" => "Start-Process",
                "macos" => "open",
                _ => "xdg-open",
            };
            simple(opener.to_owned())
        }
        _ => Err(format!(
            "Rust Code doesn't know how to run {}. A run configuration can: add it in Run: Open Configurations, then press Ctrl+F5",
            name_of(file)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_java_main_class_and_package() {
        let src =
            "package com.app;\nclass Helper {}\npublic class Main {\n  public static void main(String[] a) {}\n}\n";
        assert_eq!(java_main(src), Some(JavaMain::Class { package: "com.app".into(), class: "Main".into() }));
        assert_eq!(java_main("class Helper { void run() {} }"), None);
        assert_eq!(java_main("void main() { }"), Some(JavaMain::Compact));
        let root = java_source_root(Path::new("/p/src/main/java/com/app/Main.java"), "com.app");
        assert_eq!(root, Path::new("/p/src/main/java"));
        // Folders that don't match the package: the file's own folder.
        assert_eq!(java_source_root(Path::new("/p/Main.java"), "demo"), Path::new("/p"));
    }

    #[test]
    fn finds_main_in_c_and_cpp() {
        assert!(c_has_main(Lang::C, "#include <stdio.h>\nint main(void) { return 0; }\n"));
        assert!(c_has_main(Lang::Cpp, "int main(int argc, char** argv) { return 0; }\n"));
        assert!(!c_has_main(Lang::C, "int add(int a, int b) { return a + b; }\n"));
    }

    #[test]
    fn plans_rules_per_language() {
        let tmp = std::env::temp_dir().join(format!("rust-code-run-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("venvproj/.venv").join(if cfg!(windows) { "Scripts" } else { "bin" }))
            .unwrap();
        std::fs::create_dir_all(tmp.join("crate/src/bin")).unwrap();
        std::fs::create_dir_all(tmp.join("cs/App")).unwrap();
        std::fs::create_dir_all(tmp.join("make")).unwrap();
        std::fs::write(tmp.join("crate/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        std::fs::write(tmp.join("cs/App/App.csproj"), "").unwrap();
        std::fs::write(tmp.join("make/Makefile"), "").unwrap();
        let venv = tmp.join("venvproj/.venv").join(if cfg!(windows) { "Scripts/python.exe" } else { "bin/python" });
        std::fs::write(&venv, "").unwrap();

        let command = |file: &Path, source: &str| match plan(file, source, Some(&tmp)) {
            Ok(Outcome::Run(p)) => p.command,
            other => panic!("{other:?}"),
        };
        // A project's virtual environment wins.
        assert!(command(&tmp.join("venvproj/a.py"), "print(1)").contains(".venv"));
        // The Cargo target comes from where the file is.
        assert_eq!(command(&tmp.join("crate/src/bin/tool.rs"), ""), format!("cargo run --bin {}", quote("tool")));
        // Unknown types and files with no entry point say why.
        assert!(plan(&tmp.join("a.xyz"), "", Some(&tmp)).unwrap_err().contains("doesn't know"));
        assert!(plan(&tmp.join("lib.c"), "int f(void){return 0;}", Some(&tmp)).unwrap_err().contains("no main"));
        assert!(plan(&tmp.join("make/m.c"), "int main(void){return 0;}", Some(&tmp)).unwrap_err().contains("Make"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn may_run_agrees_with_plan() {
        for ext in RUNNABLE_TYPES {
            let file = PathBuf::from(format!("/no/such/place/a.{ext}"));
            assert!(may_run(&file), "{ext}");
            let known = plan(&file, "", None).map_or_else(|e| !e.contains("doesn't know"), |_| true);
            assert!(known, "plan has no way to run .{ext}");
        }
        assert!(may_run(Path::new("/p/Cargo.toml")));
        for name in ["a.json", "a.css", "README.md", "a.h", "a.d.ts", "a.pyi", "a.xyz", "Makefile", "other.toml"] {
            assert!(!may_run(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn files_that_never_run_say_why() {
        let why = |name: &str| plan(Path::new(name), "", None).unwrap_err();
        assert!(why("types.d.ts").contains("declares types"));
        assert!(why("stub.pyi").contains("declares types"));
        assert!(why("a.h").contains("header") && why("a.hpp").contains("header"));
        assert!(why("a.css").contains("stylesheet"));
        assert!(why("a.json").contains("data") && why("a.yaml").contains("data"));
        assert!(why("README.md").contains("document"));
        assert!(!why("a.xyz").contains("isn't a program"));
    }

    fn fixture() -> Vec<CargoPackage> {
        let target = |name: &str, kind: &str, crate_type: &str, src: &str| crate::cargo::CargoTarget {
            name: name.into(),
            kinds: vec![kind.into()],
            crate_types: vec![crate_type.into()],
            src_path: PathBuf::from(src),
        };
        let package = |name: &str, dir: &str, default_run: Option<&str>, targets| CargoPackage {
            name: name.into(),
            dir: PathBuf::from(dir),
            default_run: default_run.map(str::to_owned),
            targets,
        };
        vec![
            package("libonly", "/w/libonly", None, vec![target("libonly", "lib", "lib", "/w/libonly/src/lib.rs")]),
            package(
                "app",
                "/w/app",
                None,
                vec![
                    target("app", "lib", "lib", "/w/app/src/lib.rs"),
                    target("app", "bin", "bin", "/w/app/src/main.rs"),
                    target("demo", "example", "bin", "/w/app/examples/demo.rs"),
                    target("plug", "example", "cdylib", "/w/app/examples/plug.rs"),
                    target("smoke", "test", "bin", "/w/app/tests/smoke.rs"),
                    target("speed", "bench", "bin", "/w/app/benches/speed.rs"),
                    target("build-script-build", "custom-build", "bin", "/w/app/build.rs"),
                ],
            ),
            package(
                "tools",
                "/w/tools",
                None,
                vec![
                    target("one", "bin", "bin", "/w/tools/src/bin/one/main.rs"),
                    target("two", "bin", "bin", "/w/tools/src/bin/two.rs"),
                ],
            ),
            package(
                "picked",
                "/w/picked",
                Some("b"),
                vec![target("a", "bin", "bin", "/w/picked/a.rs"), target("b", "bin", "bin", "/w/picked/b.rs")],
            ),
        ]
    }

    /// What `rust_target` answers for a file: its command, or the titles to pick from.
    fn answer(file: &str) -> Result<String, String> {
        rust_target(Path::new(file), &fixture()).map(|outcome| match outcome {
            Outcome::Run(plan) => plan.command,
            Outcome::Choose(plans) => plans.into_iter().map(|p| p.title).collect::<Vec<_>>().join(","),
        })
    }

    #[test]
    fn rust_runs_what_cargo_says_the_file_is() {
        let cargo = |rest: &str, package: &str, flag: &str, name: &str| {
            format!("cargo {rest} -p {} --{flag} {}", quote(package), quote(name))
        };
        // Each file that starts a target runs that target.
        assert_eq!(answer("/w/app/src/main.rs"), Ok(cargo("run", "app", "bin", "app")));
        assert_eq!(answer("/w/app/examples/demo.rs"), Ok(cargo("run", "app", "example", "demo")));
        assert_eq!(answer("/w/app/tests/smoke.rs"), Ok(cargo("test", "app", "test", "smoke")));
        assert_eq!(answer("/w/app/benches/speed.rs"), Ok(cargo("bench", "app", "bench", "speed")));
        assert_eq!(answer("/w/tools/src/bin/one/main.rs"), Ok(cargo("run", "tools", "bin", "one")));
        // Modules, the library and the manifest stand for the package's program.
        assert_eq!(answer("/w/app/src/util.rs"), Ok(cargo("run", "app", "bin", "app")));
        assert_eq!(answer("/w/app/src/lib.rs"), Ok(cargo("run", "app", "bin", "app")));
        assert_eq!(answer("/w/app/Cargo.toml"), Ok(cargo("run", "app", "bin", "app")));
        // Several programs: its own package's only, unless one is the default.
        assert_eq!(answer("/w/tools/src/util.rs"), Ok("one,two".to_owned()));
        assert_eq!(answer("/w/picked/other.rs"), Ok(cargo("run", "picked", "bin", "b")));
        // A workspace's own manifest offers every member's.
        assert_eq!(answer("/w/Cargo.toml"), Ok("app (app),one (tools),two (tools),a (picked),b (picked)".to_owned()));
    }

    #[test]
    fn rust_refuses_what_cannot_run() {
        assert!(answer("/w/libonly/src/lib.rs").unwrap_err().contains("is a library"));
        assert!(answer("/w/app/build.rs").unwrap_err().contains("whenever Cargo builds"));
        assert!(answer("/w/app/examples/plug.rs").unwrap_err().contains("is a library"));
        assert!(rust_target(Path::new("/w/Cargo.toml"), &[]).unwrap_err().contains("no program"));
    }
}
