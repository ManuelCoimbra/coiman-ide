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
pub(crate) fn quote(text: &str) -> String {
    if cfg!(windows) { format!("'{}'", text.replace('\'', "''")) } else { format!("'{}'", text.replace('\'', r"'\''")) }
}

pub(crate) fn q(path: &Path) -> String {
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

/// Why a Maven project can't run: the app offers to fetch Maven when it sees this.
pub const MAVEN_MISSING: &str = "This Maven project needs Maven: no mvnw in the project, and no mvn on PATH";

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

/// `void main()` or `void main(String[] args)`, the two forms Java starts.
/// `static` and `public` aren't required since Java 25.
fn is_main_method(m: Node, source: &str) -> bool {
    let named_main = m.child_by_field_name("name").is_some_and(|n| text(n, source) == "main");
    let void = m.child_by_field_name("type").is_some_and(|t| t.kind() == "void_type");
    let takes = m.child_by_field_name("parameters").map(children).unwrap_or_default();
    let params_fit = match takes.as_slice() {
        [] => true,
        [only] => text(*only, source).trim_start_matches("final ").starts_with("String"),
        _ => false,
    };
    m.kind() == "method_declaration" && named_main && void && params_fit
}

fn has_main_method(body: Node, source: &str) -> bool {
    children(body).into_iter().any(|m| is_main_method(m, source))
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
            "method_declaration" if is_main_method(node, source) => return Some(JavaMain::Compact),
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
        let mvn = crate::maven::find_mvn(&dir, root).ok_or_else(|| MAVEN_MISSING.to_owned())?;
        let main_arg = quote(&format!("-Dexec.mainClass={full}"));
        return Ok(RunPlan {
            title: class.clone(),
            command: mvn.command(&format!("-q compile exec:java {main_arg}")),
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
    // A build tool that picks the class itself leaves only one thing to run.
    if plans.windows(2).all(|p| p[0].command == p[1].command) {
        plans.truncate(1);
    }
    // This file isn't a program: another stands in, and the title says so.
    if let [only] = plans.as_mut_slice() {
        only.title = format!("{} (project)", only.title);
    }
    one_or_choose(plans).ok_or_else(|| "No class with a main method was found".to_owned())
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

/// A test id meaning: not a test, run the program.
pub const RUN_PROGRAM: &str = "\0program";
/// A test id meaning: run this `package.json` script.
pub const RUN_SCRIPT: &str = "\0script:";
/// A test id meaning: open this address on the project's dev server.
pub const RUN_PAGE: &str = "\0page:";

/// The lines under `"scripts"` of a `package.json`, as (0-based line, script).
fn package_script_lines(source: &str) -> Vec<(usize, String)> {
    let mut lines = source.lines().enumerate();
    if !lines.any(|(_, l)| l.trim_start().starts_with("\"scripts\"")) {
        return Vec::new();
    }
    lines
        .map_while(|(at, l)| {
            let l = l.trim_start();
            (!l.starts_with('}')).then_some((at, l))
        })
        .filter_map(|(at, l)| Some((at, l.strip_prefix('"')?.split_once('"')?.0.to_owned())))
        .collect()
}

/// Play buttons of web projects, with ids for [`RUN_SCRIPT`] and [`RUN_PAGE`]. `relative`
/// is the file's path in the project.
pub fn web_lines(file: &Path, relative: &str, source: &str) -> Vec<(usize, String)> {
    if name_of(file) == "package.json" {
        let found = package_script_lines(source).into_iter();
        return found.map(|(at, script)| (at, format!("{RUN_SCRIPT}{script}"))).collect();
    }
    let Some((_, Some(route))) = crate::project_tree::web_page(relative) else { return Vec::new() };
    let line = source.lines().position(|l| l.starts_with("export ")).unwrap_or(0);
    vec![(line, format!("{RUN_PAGE}{route}"))]
}

/// The address a dev server says it's listening at, the latest if it moved.
pub fn server_url(lines: &[String]) -> Option<String> {
    const HOSTS: &[&str] = &["localhost", "127.0.0.1", "0.0.0.0", "[::1]"];
    let at_host = |word: &str| {
        let rest = word.strip_prefix("http://").or_else(|| word.strip_prefix("https://"))?;
        HOSTS.iter().any(|h| rest.strip_prefix(h).is_some_and(|r| r.starts_with(':'))).then_some(())
    };
    let found = lines.iter().rev().flat_map(|l| l.split_whitespace()).find_map(|word| {
        let start = word.find("http")?;
        let url = word[start..].trim_end_matches(|c: char| !c.is_alphanumeric() && c != '/');
        at_host(url).map(|()| url.replace("0.0.0.0", "localhost"))
    });
    found.map(|url| url.trim_end_matches('/').to_owned())
}

/// The lines a program starts at, going by the text: a `main`, and the type holding it
/// where a language has one; a script's `#!` line or its "run as a program" check. A script
/// run from the top with neither has no such mark.
pub fn program_lines(file: &Path, source: &str) -> Vec<usize> {
    let extension = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let stem = stem_of(file);
    let starts = |line: &str, marks: &[&str]| marks.iter().any(|m| line.starts_with(m));
    let holds = |line: &str, marks: &[&str]| marks.iter().any(|m| line.contains(m));
    let is_start = |line: &str| match extension.as_str() {
        "java" | "groovy" => holds(line, &["void main("]),
        "c" | "h" | "cpp" | "cc" | "cxx" => holds(line, &[" main(", " wmain(", " WinMain("]) && !line.ends_with(';'),
        "rs" => starts(line, &["fn main(", "pub fn main(", "async fn main(", "pub async fn main("]),
        "go" => starts(line, &["func main("]),
        "py" | "pyw" => starts(line, &["if __name__ == '__main__'", "if __name__ == \"__main__\""]),
        "js" | "mjs" | "cjs" | "ts" | "mts" | "cts" => {
            holds(line, &["require.main === module", "import.meta.main", ".listen(", "Bun.serve(", "Deno.serve("])
        }
        "html" | "htm" => starts(line, &["<html", "<script"]),
        "kt" => starts(line, &["fun main(", "suspend fun main("]),
        "scala" => holds(line, &["def main(", "@main"]),
        "cs" => holds(line, &["static void Main(", "static int Main(", "static async Task Main(", "static Task Main("]),
        "fs" => starts(line, &["[<EntryPoint>]"]),
        "vb" => holds(line, &["Sub Main("]),
        "swift" => starts(line, &["@main"]),
        "dart" => starts(line, &["void main(", "main(", "Future<void> main("]),
        "rb" => starts(line, &["if __FILE__ == $0", "if __FILE__ == $PROGRAM_NAME"]),
        "zig" => starts(line, &["pub fn main("]),
        "nim" | "nims" => starts(line, &["when isMainModule"]),
        "hs" => starts(line, &["main ::", "main ="]),
        "jl" => starts(line, &["if abspath(PROGRAM_FILE)"]),
        "d" => holds(line, &["void main(", "int main("]),
        "clj" | "cljc" => starts(line, &["(defn -main"]),
        "erl" => starts(line, &["main("]),
        "ml" => starts(line, &["let () ="]),
        "pas" | "f90" | "f95" | "f03" | "f" | "for" => line.to_lowercase().starts_with("program "),
        "cob" | "cbl" => line.to_uppercase().starts_with("PROGRAM-ID"),
        _ => false,
    };
    let mut lines: Vec<usize> = Vec::new();
    if source.starts_with("#!") {
        lines.push(0);
    }
    let found: Vec<usize> = source.lines().enumerate().filter(|(_, l)| is_start(l.trim())).map(|(at, _)| at).collect();
    // Where the syntax tells better than the words do.
    let confirmed = match extension.as_str() {
        "java" => java_main(source).is_some(),
        "c" => c_has_main(Lang::C, source),
        "cpp" | "cc" | "cxx" => c_has_main(Lang::Cpp, source),
        "go" => source.lines().any(|l| l.trim_start().starts_with("package main")),
        _ => true,
    };
    if confirmed && !found.is_empty() {
        if matches!(extension.as_str(), "java" | "kt" | "scala" | "groovy" | "cs") {
            let word = |w: &str| w.split(|c: char| !c.is_alphanumeric() && c != '_').next() == Some(stem.as_str());
            let class = source.lines().position(|l| {
                let words: Vec<&str> = l.split_whitespace().collect();
                words
                    .windows(2)
                    .any(|w| matches!(w[0], "class" | "object" | "record" | "enum" | "interface") && word(w[1]))
            });
            lines.extend(class.filter(|c| *c < found[0]));
        }
        lines.extend(found);
    }
    lines
}

/// Whether a C or C++ file defines `main`.
fn c_has_main(lang: Lang, source: &str) -> bool {
    let Some(tree) = parse(lang, source) else { return false };
    c_defines_main(tree.root_node(), source)
}

/// Looks through conditional compilation and `extern "C"`, but not namespaces:
/// a `main` in one isn't where a program starts.
fn c_defines_main(node: Node, source: &str) -> bool {
    children(node).into_iter().any(|n| match n.kind() {
        "function_definition" => {
            let mut declarator = n.child_by_field_name("declarator");
            while let Some(d) = declarator {
                if d.kind() == "identifier" {
                    return text(d, source) == "main";
                }
                declarator = d.child_by_field_name("declarator");
            }
            false
        }
        "preproc_ifdef"
        | "preproc_if"
        | "preproc_else"
        | "preproc_elif"
        | "linkage_specification"
        | "declaration_list" => c_defines_main(n, source),
        _ => false,
    })
}

/// The programs a `CMakeLists.txt` declares, by a plain reading of its text.
fn cmake_executables(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect();
    let code = lines.join("\n");
    let args_of = |call: &str| -> Vec<String> {
        let lower = code.to_lowercase();
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(at) = lower[from..].find(call) {
            let start = from + at + call.len();
            let end = lower[start..].find(')').map_or(lower.len(), |e| start + e);
            let before = lower[..from + at].chars().next_back();
            if lower[start..].starts_with('(') && !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                found.push(code[start + 1..end].to_owned());
            }
            from = start;
        }
        found
    };
    let project = args_of("project").first().and_then(|a| a.split_whitespace().next().map(str::to_owned));
    args_of("add_executable")
        .iter()
        .filter_map(|args| {
            let mut words = args.split_whitespace();
            let name = words.next()?;
            if matches!(words.next(), Some("ALIAS" | "IMPORTED")) {
                return None;
            }
            match (name, &project) {
                ("${PROJECT_NAME}", Some(project)) => Some(project.clone()),
                (name, _) if name.contains('$') => None,
                (name, _) => Some(name.to_owned()),
            }
        })
        .collect()
}

/// The targets a Makefile can be asked for, by a plain reading of its text.
fn make_targets(text: &str) -> Vec<String> {
    let mut found: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with([' ', '\t', '#', '.']))
        .filter_map(|l| l.split_once(':').filter(|(_, rest)| !rest.starts_with('=')))
        .map(|(name, _)| name.trim())
        .filter(|name| !name.is_empty() && !name.contains(['$', '%', '=', ' ']))
        .map(str::to_owned)
        .collect();
    found.dedup();
    found
}

fn cmake_plans(dir: &Path, title_suffix: &str) -> Result<Outcome, String> {
    let text = std::fs::read_to_string(dir.join("CMakeLists.txt")).unwrap_or_default();
    // Windows defaults to Visual Studio's tools, often absent. A folder already
    // configured keeps its generator: it can't change.
    let generator = if !cfg!(windows) || dir.join("build").join("CMakeCache.txt").is_file() {
        ""
    } else if which("ninja").is_some() {
        " -G Ninja"
    } else if which("mingw32-make").is_some() {
        " -G 'MinGW Makefiles'"
    } else {
        ""
    };
    let plans = cmake_executables(&text)
        .into_iter()
        .map(|name| {
            let configure = format!("cmake -S . -B build{generator}");
            let build = format!("cmake --build build --target {}", quote(&name));
            let exe = dir.join("build").join(exe_name(&name));
            let command = then(&then(&configure, &build), &call(&exe));
            RunPlan { title: format!("{name}{title_suffix}"), command, cwd: dir.to_path_buf() }
        })
        .collect();
    let outcome = one_or_choose(plans).ok_or_else(|| "This CMake project declares no program to run".to_owned())?;
    need("cmake", "This CMake project")?;
    Ok(outcome)
}

/// How to call make in a shell command: `make`, else MinGW's `mingw32-make`, by name
/// when it's on PATH, else by its path (a MinGW folder).
fn make_command() -> Result<String, String> {
    let make = crate::c_build::make_program()
        .ok_or("This makefile needs make (or MinGW's mingw32-make), found neither on PATH nor in a MinGW folder")?;
    let name = make.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if which(&name).is_some() {
        return Ok(name);
    }
    Ok(if cfg!(windows) { format!("& '{}'", make.display()) } else { quote(&make.display().to_string()) })
}

/// Builds with make, then runs the program the makefile names, in its folder.
fn make_then_run(dir: &Path, program: &str, make: &str) -> RunPlan {
    let path = crate::c_build::program_path(dir, program);
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let command = if cfg!(windows) {
        format!("{make}; if ($?) {{ & '.\\{file}' }}")
    } else {
        format!("{make} && ./{}", quote(&file))
    };
    RunPlan { title: format!("make, then {file}"), command, cwd: dir.to_path_buf() }
}

fn make_plans(file: &Path) -> Result<Outcome, String> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let make = make_command()?;
    let plans = make_targets(&text)
        .into_iter()
        .map(|t| RunPlan { title: format!("make {t}"), command: format!("{make} {}", quote(&t)), cwd: dir.clone() })
        .collect();
    one_or_choose(plans).ok_or_else(|| format!("{} has no targets", name_of(file)))
}

fn c_family(file: &Path, source: &str, root: Option<&Path>, cpp: bool) -> Result<Outcome, String> {
    let lang = if cpp { Lang::Cpp } else { Lang::C };
    let has_main = c_has_main(lang, source);
    // The build system's program; this file is part of it, or stands in for it.
    if let Some(dir) = find_up(file, root, "CMakeLists.txt") {
        return cmake_plans(&dir, if has_main { "" } else { " (project)" });
    }
    // A makefile by any of the names make reads (course projects use `makefile`).
    let makefile = upward(file, root).find_map(crate::c_build::makefile_in);
    if let Some(makefile) = &makefile {
        let dir = makefile.parent().unwrap_or(file).to_path_buf();
        let text = std::fs::read_to_string(makefile).unwrap_or_default();
        if make_targets(&text).iter().any(|t| t == "run") {
            let make = make_command()?;
            let title = if has_main { "make run" } else { "make run (project)" }.to_owned();
            return Ok(Outcome::Run(RunPlan { title, command: format!("{make} run"), cwd: dir }));
        }
        // The program the makefile builds: make, then run it, as `make` and `./prog` would.
        if let Some(program) = crate::c_build::makefile_program(&text, None) {
            return Ok(Outcome::Run(make_then_run(&dir, &program, &make_command()?)));
        }
        // No `run`, and this file can't stand alone: the makefile says what there is.
        if !has_main {
            return make_plans(makefile);
        }
    }
    if !has_main {
        return Err(format!("{} has no main function", name_of(file)));
    }
    if makefile.is_some() {
        return Err(
            "This project builds with Make, but its makefile doesn't say what program it makes (no `-o`): add it in Run: Open Configurations, then press Ctrl+F5".to_owned(),
        );
    }
    if find_up(file, root, "meson.build").is_some() {
        return Err(
            "This project builds with Meson, which decides the program's name: add it in Run: Open Configurations, then press Ctrl+F5".to_owned(),
        );
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

/// Runs one of `found`, asking when there are several.
fn one_or_choose(mut found: Vec<RunPlan>) -> Option<Outcome> {
    match found.len() {
        0 => None,
        1 => Some(Outcome::Run(found.remove(0))),
        _ => Some(Outcome::Choose(found)),
    }
}

/// One thing Cargo can start for a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoAction {
    pub package: String,
    pub dir: PathBuf,
    /// `run`, `test` or `bench`.
    pub tool: &'static str,
    /// `bin`, `example`, `test` or `bench`.
    pub flag: &'static str,
    pub name: String,
    pub title: String,
}

impl CargoAction {
    fn new(package: &CargoPackage, tool: &'static str, flag: &'static str, name: &str, title: String) -> Self {
        Self { package: package.name.clone(), dir: package.dir.clone(), tool, flag, name: name.to_owned(), title }
    }

    fn plan(self) -> RunPlan {
        let command = format!("cargo {} -p {} --{} {}", self.tool, quote(&self.package), self.flag, quote(&self.name));
        RunPlan { title: self.title, command, cwd: self.dir }
    }
}

/// What starting `file` means given what Cargo says the workspace holds: the
/// file's own target when it starts one, else the program of its package.
/// Run and Debug both ask here.
fn rust_actions(file: &Path, packages: &[CargoPackage]) -> Result<Vec<CargoAction>, String> {
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
        let found: Vec<CargoAction> = packages
            .iter()
            .flat_map(|p| p.binaries().map(move |t| (p, t)))
            .map(|(p, t)| CargoAction::new(p, "run", "bin", &t.name, format!("{} ({})", t.name, p.name)))
            .collect();
        return if found.is_empty() { Err("This Cargo workspace has no program to run".to_owned()) } else { Ok(found) };
    };
    let own = package.targets.iter().find(|t| comparable(&t.src_path) == at);
    if let Some(target) = own {
        let action = |tool, flag| Ok(vec![CargoAction::new(package, tool, flag, &target.name, target.name.clone())]);
        if target.is("bin") {
            return action("run", "bin");
        }
        if target.is("example") {
            if target.crate_types.iter().any(|c| c == "bin") {
                return action("run", "example");
            }
            return Err(format!("The example {} is a library, so it can't run", target.name));
        }
        if target.is("test") {
            return action("test", "test");
        }
        if target.is("bench") {
            return action("bench", "bench");
        }
        if target.is("custom-build") {
            return Err(format!("{name} runs by itself whenever Cargo builds {}", package.name));
        }
    }
    // Not a program itself: its package's stands in, and the title says so.
    let stand_in = |bin: &str| CargoAction::new(package, "run", "bin", bin, format!("{bin} (project)"));
    let default = package.default_run.as_deref();
    if let Some(bin) = package.binaries().find(|t| Some(t.name.as_str()) == default) {
        return Ok(vec![stand_in(&bin.name)]);
    }
    let bins: Vec<&str> = package.binaries().map(|t| t.name.as_str()).collect();
    match bins.as_slice() {
        [] => Err(format!(
            "{} is a library, so it has no program to run. Add src/main.rs to give it one, or run its tests",
            package.name
        )),
        [only] => Ok(vec![stand_in(only)]),
        _ => Ok(bins.iter().map(|b| CargoAction::new(package, "run", "bin", b, (*b).to_owned())).collect()),
    }
}

fn rust_target(file: &Path, packages: &[CargoPackage]) -> Result<Outcome, String> {
    let plans = rust_actions(file, packages)?.into_iter().map(CargoAction::plan).collect();
    Ok(one_or_choose(plans).expect("rust_actions refuses when it finds nothing"))
}

/// What Debug should start for `file`, by the same rules as Run.
pub fn debug_actions(file: &Path, root: Option<&Path>) -> Result<Vec<CargoAction>, String> {
    let Some(project) = find_up(file, root, "Cargo.toml") else {
        return Err("Debugging works for Cargo projects so far: Shift+F10 runs this file".to_owned());
    };
    if !crate::trust::is_trusted(&project) {
        return Err("Restricted Mode: trust this folder to debug in it".to_owned());
    }
    let Some(packages) = crate::cargo::packages(&project) else {
        return Err("Cargo couldn't read this project: see Cargo.toml".to_owned());
    };
    rust_actions(file, &packages)
}

// ----- tests in other languages ---------------------------------------------------------

/// The identifier a line starts with.
fn ident_at(text: &str) -> &str {
    let end = text.find(|c: char| !c.is_alphanumeric() && c != '_').unwrap_or(text.len());
    &text[..end]
}

fn go_tests(source: &str) -> Vec<(usize, String)> {
    let is_test = |name: &str| {
        ["Test", "Benchmark", "Fuzz"]
            .iter()
            .any(|p| name.strip_prefix(p).is_some_and(|rest| !rest.chars().next().is_some_and(char::is_lowercase)))
    };
    source
        .lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let name = ident_at(l.strip_prefix("func ")?);
            is_test(name).then(|| (i, name.to_owned()))
        })
        .collect()
}

/// `test_` functions, and those in a class as `Class.test_`.
fn python_tests(source: &str) -> Vec<(usize, String)> {
    let mut class: Option<(&str, Option<usize>)> = None;
    let mut found = Vec::new();
    for (i, line) in source.lines().enumerate() {
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let indent = line.len() - body.len();
        if indent == 0 {
            class = body.strip_prefix("class ").map(|rest| (ident_at(rest), None));
        }
        let def = body.strip_prefix("async def ").or_else(|| body.strip_prefix("def "));
        if let Some((_, members)) = &mut class
            && indent > 0
        {
            members.get_or_insert(indent);
        }
        let Some(name) = def.map(ident_at).filter(|n| n.starts_with("test")) else { continue };
        match class {
            None if indent == 0 => found.push((i, name.to_owned())),
            Some((owner, Some(members))) if indent == members => found.push((i, format!("{owner}.{name}"))),
            _ => {}
        }
    }
    found
}

/// `test("name"` and `it("name"`, nested or not.
fn js_tests(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let body = line.trim_start();
            let args = ["test(", "it(", "describe("].iter().find_map(|mark| body.strip_prefix(mark))?;
            let quote = args.chars().next().filter(|c| matches!(c, '"' | '\'' | '`'))?;
            let (name, _) = args[1..].split_once(quote)?;
            Some((i, name.to_owned()))
        })
        .collect()
}

/// The tests a file defines, as (0-based line, id). Reads only the text, so
/// it's cheap to redo as the file is edited.
pub fn test_lines(file: &Path, source: &str) -> Vec<(usize, String)> {
    let name = name_of(file);
    if name.ends_with("_test.go") {
        go_tests(source)
    } else if name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py")) {
        python_tests(source)
    } else if [".test.", ".spec."].iter().any(|m| name.contains(m))
        && matches!(
            file.extension().and_then(|e| e.to_str()),
            Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "jsx" | "tsx")
        )
    {
        js_tests(source)
    } else {
        Vec::new()
    }
}

/// `text` matching only itself as a regular expression.
pub(crate) fn regex_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if r"\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// How to run the test `id` from [`test_lines`].
pub fn test_plan(file: &Path, source: &str, id: &str, root: Option<&Path>) -> Result<RunPlan, String> {
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let title = format!("Test {id}");
    let command = match file.extension().and_then(|e| e.to_str()).unwrap_or_default() {
        "go" => {
            need("go", "Go")?;
            format!("go test -run {} .", quote(&format!("^{id}$")))
        }
        "py" => {
            let python = python_program(file, root)?;
            match id.contains('.') && source.contains("unittest") {
                true => format!("{python} -m unittest {}", quote(&format!("{}.{id}", stem_of(file)))),
                false => {
                    let node = format!("{}::{}", file.to_string_lossy(), id.replace('.', "::"));
                    format!("{python} -m pytest {}", quote(&node))
                }
            }
        }
        _ => {
            let manifest = find_up(file, root, "package.json")
                .and_then(|d| std::fs::read_to_string(d.join("package.json")).ok())
                .unwrap_or_default();
            if manifest.contains("\"vitest\"") {
                format!("npx vitest run {} -t {}", q(file), quote(id))
            } else if manifest.contains("\"jest\"") {
                format!("npx jest {} -t {}", q(file), quote(id))
            } else {
                need("node", "JavaScript")?;
                let pattern = format!("^{}$", regex_escape(id));
                format!("node --test {} {}", quote(&format!("--test-name-pattern={pattern}")), q(file))
            }
        }
    };
    Ok(RunPlan { title, command, cwd: dir })
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
    if name_of(file).ends_with("_test.go") {
        need("go", "Go")?;
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
    let in_module = find_up(file, root, "go.mod").is_some();
    if !in_module && !source.lines().any(|l| l.trim_start().starts_with("func main(")) {
        return Err(format!("{} has no func main", name_of(file)));
    }
    need("go", "Go")?;
    let command = if in_module { "go run .".to_owned() } else { format!("go run {}", q(file)) };
    Ok(Outcome::Run(RunPlan { title: stem_of(file), command, cwd: dir }))
}

// ----- C# and F# -----------------------------------------------------------------------

/// Whether a project builds a program, from its text and that of its
/// `Directory.Build.props`. Without `OutputType`, the SDK decides: a library,
/// except for web and worker projects.
fn dotnet_is_program(project: &str) -> bool {
    let lower = project.to_lowercase();
    let value = lower.split_once("<outputtype>").and_then(|(_, rest)| rest.split_once('<')).map(|(v, _)| v.trim());
    match value {
        Some(value) => matches!(value, "exe" | "winexe") || value.contains("$("),
        None => lower.contains("microsoft.net.sdk.web") || lower.contains("microsoft.net.sdk.worker"),
    }
}

fn project_is_program(project: &Path) -> bool {
    let Ok(mut text) = std::fs::read_to_string(project) else { return true };
    let props = project.ancestors().skip(1).find_map(|d| std::fs::read_to_string(d.join("Directory.Build.props")).ok());
    text.push_str(&props.unwrap_or_default());
    dotnet_is_program(&text)
}

/// The project files a `.sln` or `.slnx` lists, relative to it.
fn solution_projects(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("Project(") || l.trim_start().starts_with("<Project"))
        .flat_map(|l| l.split('"'))
        .filter(|part| part.ends_with(".csproj") || part.ends_with(".fsproj"))
        .map(|part| PathBuf::from(part.replace('\\', "/")))
        .collect()
}

fn dotnet_plan(project: &Path, title: String) -> RunPlan {
    RunPlan {
        title,
        command: format!("dotnet run --project {}", q(project)),
        cwd: project.parent().unwrap_or(project).to_path_buf(),
    }
}

fn solution_plans(solution: &Path) -> Result<Outcome, String> {
    let text = std::fs::read_to_string(solution).unwrap_or_default();
    let dir = solution.parent().unwrap_or(solution);
    let plans = solution_projects(&text)
        .into_iter()
        .map(|p| dir.join(p))
        .filter(|p| project_is_program(p))
        .map(|p| dotnet_plan(&p, stem_of(&p)))
        .collect();
    let outcome = one_or_choose(plans)
        .ok_or_else(|| format!("{} has no program to run: all class libraries", name_of(solution)))?;
    need("dotnet", "C#")?;
    Ok(outcome)
}

/// Whether a C# file holds the program's start: `Main`, or top-level statements by
/// the name .NET's templates give them.
fn cs_is_entry(file: &Path) -> bool {
    let named = file.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Program.cs"));
    named || std::fs::read_to_string(file).is_ok_and(|text| text.contains("static") && text.contains(" Main("))
}

fn dotnet(file: &Path, root: Option<&Path>) -> Result<Outcome, String> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "sln" | "slnx") {
        return solution_plans(file);
    }
    let library = |project: &Path| format!("{} is a class library: it has no program to run", stem_of(project));
    if matches!(ext.as_str(), "csproj" | "fsproj") {
        if !project_is_program(file) {
            return Err(library(file));
        }
        need("dotnet", "C#")?;
        return Ok(Outcome::Run(dotnet_plan(file, stem_of(file))));
    }
    let project = find_ext_up(file, root, "csproj");
    if let Some(project) = project.as_ref().filter(|p| project_is_program(p)) {
        need("dotnet", "C#")?;
        // The file with the entry point is the program, not a part of it.
        let title = if cs_is_entry(file) { stem_of(project) } else { format!("{} (project)", stem_of(project)) };
        return Ok(Outcome::Run(dotnet_plan(project, title)));
    }
    // A library, or no project: its solution may hold a program.
    let solution = find_ext_up(file, root, "sln").or_else(|| find_ext_up(file, root, "slnx"));
    if let Some(solution) = solution {
        return solution_plans(&solution).map(|outcome| match outcome {
            Outcome::Run(plan) => Outcome::Run(RunPlan { title: format!("{} (project)", plan.title), ..plan }),
            several => several,
        });
    }
    if let Some(project) = &project {
        return Err(library(project));
    }
    // .NET 10 runs a single file.
    need("dotnet", "C#")?;
    let dir = file.parent().unwrap_or(file).to_path_buf();
    Ok(Outcome::Run(RunPlan { title: name_of(file), command: format!("dotnet run {}", q(file)), cwd: dir }))
}

// ----- files that describe a project -----------------------------------------------------

/// The scripts of a `package.json`, for the package manager its lockfile names.
fn package_tool(dir: &Path) -> &'static str {
    [("pnpm-lock.yaml", "pnpm"), ("yarn.lock", "yarn"), ("bun.lockb", "bun"), ("bun.lock", "bun")]
        .iter()
        .find(|(lock, _)| dir.join(lock).exists())
        .map_or("npm", |(_, tool)| *tool)
}

/// How to run one script of a `package.json`.
pub fn package_script_plan(file: &Path, script: &str) -> Result<RunPlan, String> {
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let tool = package_tool(&dir);
    need(tool, "This package")?;
    Ok(RunPlan { title: format!("{tool} run {script}"), command: format!("{tool} run {}", quote(script)), cwd: dir })
}

/// How to start the dev server of the project a file is in.
pub fn dev_script_plan(file: &Path, root: Option<&Path>) -> Result<RunPlan, String> {
    let dir = find_up(file, root, "package.json").ok_or("No package.json here to start a dev server from")?;
    let manifest = dir.join("package.json");
    let text = std::fs::read_to_string(&manifest).unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let script = ["dev", "start", "serve"].into_iter().find(|s| json["scripts"][*s].is_string());
    let script = script.ok_or("package.json has no dev, start or serve script")?;
    package_script_plan(&manifest, script)
}

fn package_scripts(file: &Path) -> Result<Outcome, String> {
    let text = std::fs::read_to_string(file).unwrap_or_default();
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let tool = package_tool(&dir);
    let plans = json["scripts"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| RunPlan {
            title: format!("{tool} run {name}"),
            command: format!("{tool} run {}", quote(name)),
            cwd: dir.clone(),
        })
        .collect();
    let outcome = one_or_choose(plans).ok_or_else(|| "package.json has no scripts to run".to_owned())?;
    need(tool, "This package")?;
    Ok(outcome)
}

fn docker(file: &Path, args: &str) -> Result<Outcome, String> {
    need("docker", "This file")?;
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let command = format!("docker {args}");
    Ok(Outcome::Run(RunPlan { title: command.clone(), command, cwd: dir }))
}

/// Compose's file names.
const COMPOSE_FILES: &[&str] = &["compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml"];

/// A file that means "this project", by name; each says what running it does.
fn project_file(file: &Path, root: Option<&Path>) -> Option<Result<Outcome, String>> {
    let name = name_of(file);
    Some(match name.as_str() {
        "Cargo.toml" => rust(file, root),
        "package.json" => package_scripts(file),
        "CMakeLists.txt" => cmake_plans(file.parent().unwrap_or(file), ""),
        "Makefile" | "makefile" | "GNUmakefile" => make_plans(file),
        "Dockerfile" => docker(file, "build ."),
        name if COMPOSE_FILES.contains(&name) => docker(file, "compose up"),
        _ => return None,
    })
}

// ----- interpreted languages -----------------------------------------------------------

/// The project's Python, by the answer every tool shares.
pub(crate) fn python_program(file: &Path, root: Option<&Path>) -> Result<String, String> {
    let start = file.parent().unwrap_or(file);
    let env = crate::environment::resolve(start, root).ok_or_else(|| "Python isn't on PATH".to_owned())?;
    // A name found on PATH reads better, and behaves the same.
    if env.prefix.is_none() && env.reason.contains("PATH") {
        return Ok(stem_of(&env.python));
    }
    Ok(call(&env.python))
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

/// Whether a refusal from [`plan`] is about the file itself. A missing tool or
/// a build system to set up can be fixed, so Run stays on offer to explain how.
pub fn refusal_is_final(why: &str) -> bool {
    !why.contains("on PATH") && !why.contains("Ctrl+F5")
}

/// File types `plan` has a way to run.
const RUNNABLE_TYPES: &[&str] = &[
    "py", "pyw", "js", "mjs", "cjs", "ts", "mts", "cts", "java", "c", "cpp", "cc", "cxx", "c++", "rs", "go", "cs",
    "sh", "bash", "zsh", "ps1", "bat", "cmd", "rb", "php", "lua", "pl", "r", "dart", "swift", "kts", "kt", "html",
    "htm", "csproj", "fsproj", "sln", "slnx",
];

/// File names that stand for a project, whatever their type.
const PROJECT_FILES: &[&str] =
    &["Cargo.toml", "package.json", "CMakeLists.txt", "Makefile", "makefile", "GNUmakefile", "Dockerfile"];

/// Whether a file could run, judged from its name alone: cheap enough for every
/// frame. Whether it really does takes `plan`.
pub fn may_run(file: &Path) -> bool {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let name = name_of(file);
    let data_run = crate::lsp::registry::by_extension(&ext, false).is_some_and(|l| l.run.is_some());
    data_run
        || PROJECT_FILES.contains(&name.as_str())
        || COMPOSE_FILES.contains(&name.as_str())
        || (RUNNABLE_TYPES.contains(&ext.as_str()) && never_runs(file, &ext).is_none())
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
        "tsx" | "jsx" => format!("{name} needs a build step: run it through a package.json script"),
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

/// A run command from a language's entry, with `{file}` for the file's path.
fn data_run(template: &str, file: &Path, root: Option<&Path>) -> Result<Outcome, String> {
    let program = template.split_whitespace().next().unwrap_or_default();
    need(program, "This file")?;
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let cwd = root.filter(|r| file.starts_with(r)).map_or(dir, Path::to_path_buf);
    let command = template.replace("{file}", &q(file));
    Ok(Outcome::Run(RunPlan { title: name_of(file), command, cwd }))
}

/// Works out how to run `file`, whose current text is `source`. `root` is the open folder.
pub fn plan(file: &Path, source: &str, root: Option<&Path>) -> Result<Outcome, String> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if let Some(outcome) = project_file(file, root) {
        return outcome;
    }
    // The user's entry wins over everything below; a shipped one fills gaps.
    if let Some(language) = crate::lsp::registry::by_extension(&ext, true)
        && let Some(template) = language.run
    {
        return data_run(template, file, root);
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
        "cs" | "csproj" | "fsproj" | "sln" | "slnx" => dotnet(file, root),
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
        _ if crate::lsp::registry::by_extension(&ext, false).is_some_and(|l| l.run.is_some()) => {
            let template = crate::lsp::registry::by_extension(&ext, false).and_then(|l| l.run).unwrap_or_default();
            data_run(template, file, root)
        }
        _ => Err(format!(
            "Rust Code doesn't know how to run {}. A run configuration can: add it in Run: Open Configurations, then press Ctrl+F5",
            name_of(file)
        )),
    }
}

/// How to run all the tests that cover a file, for a file that's no program itself.
/// How to run all the tests of a file.
pub fn tests_of(file: &Path, source: &str, root: Option<&Path>) -> Option<RunPlan> {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let title = format!("{} tests", stem_of(file));
    let command = if ext == "rs" {
        let package = find_up(file, root, "Cargo.toml")?;
        need("cargo", "Rust").ok()?;
        return Some(RunPlan { title, command: "cargo test".to_owned(), cwd: package });
    } else if ext == "go" {
        need("go", "Go").ok()?;
        "go test .".to_owned()
    } else if test_lines(file, source).is_empty() {
        return None;
    } else if ext == "py" {
        let python = python_program(file, root).ok()?;
        match source.contains("unittest") {
            true => format!("{python} -m unittest {}", quote(&stem_of(file))),
            false => format!("{python} -m pytest {}", q(file)),
        }
    } else {
        let manifest = find_up(file, root, "package.json")
            .and_then(|d| std::fs::read_to_string(d.join("package.json")).ok())
            .unwrap_or_default();
        if manifest.contains("\"vitest\"") {
            format!("npx vitest run {}", q(file))
        } else if manifest.contains("\"jest\"") {
            format!("npx jest {}", q(file))
        } else {
            need("node", "JavaScript").ok()?;
            format!("node --test {}", q(file))
        }
    };
    Some(RunPlan { title, command, cwd: dir })
}

/// Whether a C or C++ file has a `compile_commands.json` above it, or in a `build` folder.
pub fn has_compile_database(file: &Path, root: Option<&Path>) -> bool {
    upward(file, root).any(|dir| {
        dir.join("compile_commands.json").is_file() || dir.join("build").join("compile_commands.json").is_file()
    })
}

/// What a constant preview of `file` runs: its program, or else its tests.
pub fn preview_plans(file: &Path, source: &str, root: Option<&Path>) -> Result<Vec<RunPlan>, String> {
    match plan(file, source, root) {
        Ok(Outcome::Run(found)) => Ok(vec![found]),
        Ok(Outcome::Choose(found)) => Ok(found),
        Err(why) => tests_of(file, source, root).map(|found| vec![found]).ok_or(why),
    }
}

#[cfg(test)]
mod program_line_tests {
    use super::*;

    #[test]
    fn web_play_buttons() {
        let manifest = "{\n  \"name\": \"x\",\n  \"scripts\": {\n    \"dev\": \"vite\",\n    \"test\": \"vitest\"\n  },\n  \"main\": \"a\"\n}\n";
        let found = web_lines(Path::new("package.json"), "package.json", manifest);
        assert_eq!(found, [(3, format!("{RUN_SCRIPT}dev")), (4, format!("{RUN_SCRIPT}test"))]);
        let page = "import a from 'a';\nexport default function P() {}\n";
        assert_eq!(web_lines(Path::new("page.tsx"), "app/x/page.tsx", page), [(1, format!("{RUN_PAGE}/x"))]);
        assert!(web_lines(Path::new("a.ts"), "src/a.ts", page).is_empty());
        assert_eq!(program_lines(Path::new("a.html"), "<!doctype html>\n<html>\n<script src=a></script>"), [1, 2]);
        assert_eq!(program_lines(Path::new("s.js"), "app.listen(3000);"), [0]);
        assert!(program_lines(Path::new("a.css"), "<html>").is_empty());
        assert_eq!(test_lines(Path::new("a.test.ts"), "describe('d', () => {\n  it('i', f);\n});").len(), 2);
    }

    #[test]
    fn finds_what_dev_servers_print() {
        let of = |text: &str| server_url(&[text.to_owned()]);
        assert_eq!(of("  ➜  Local:   http://localhost:5173/"), Some("http://localhost:5173".into()));
        assert_eq!(of("ready on http://0.0.0.0:3000."), Some("http://localhost:3000".into()));
        assert_eq!(of("listening at https://127.0.0.1:8443/app"), Some("https://127.0.0.1:8443/app".into()));
        assert_eq!(of("see http://example.com:80 and more"), None);
        assert_eq!(of("nothing"), None);
        let moved = ["http://localhost:3000".to_owned(), "http://localhost:3001".to_owned()];
        assert_eq!(server_url(&moved), Some("http://localhost:3001".into()));
    }

    #[test]
    fn marks_the_type_and_main_as_those_ides_do() {
        let java = "package a;\n\npublic class Main {\n    public static void main(String[] args) {}\n}\n";
        assert_eq!(program_lines(Path::new("Main.java"), java), [2, 3]);
        assert_eq!(program_lines(Path::new("a.rs"), "use x;\nfn main() {}\n"), [1]);
        assert_eq!(program_lines(Path::new("a.py"), "x = 1\nif __name__ == '__main__':\n    go()\n"), [1]);
        assert_eq!(program_lines(Path::new("a.sh"), "#!/bin/sh\necho\n"), [0]);
        assert!(program_lines(Path::new("a.go"), "package app\nfunc main() {}\n").is_empty());
        assert!(program_lines(Path::new("a.c"), "// int main() {\nint f(void) { return 0; }\n").is_empty());
        assert!(program_lines(Path::new("a.py"), "def main(): pass\n").is_empty());
    }

    #[test]
    fn every_language_has_its_form() {
        for (file, source) in [
            ("a.kt", "fun main() {}"),
            ("a.scala", "@main def go() = ()"),
            ("A.cs", "class A { static void Main(string[] args) {} }"),
            ("a.fs", "[<EntryPoint>]\nlet main argv = 0"),
            ("a.swift", "@main\nstruct App {}"),
            ("a.dart", "void main() {}"),
            ("a.rb", "if __FILE__ == $0\nend"),
            ("a.js", "if (require.main === module) go();"),
            ("a.ts", "if (import.meta.main) go();"),
            ("a.zig", "pub fn main() void {}"),
            ("a.nim", "when isMainModule:\n  go()"),
            ("a.hs", "main :: IO ()"),
            ("a.jl", "if abspath(PROGRAM_FILE) == @__FILE__\nend"),
            ("a.d", "void main() {}"),
            ("a.clj", "(defn -main [] 1)"),
            ("a.f90", "PROGRAM hello\nEND"),
            ("a.pas", "program hello;"),
            ("a.cbl", "PROGRAM-ID. HELLO."),
        ] {
            assert!(!program_lines(Path::new(file), source).is_empty(), "{file}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_makefile_builds_then_its_program_runs() {
        let dir = std::env::temp_dir().join(format!("rust-code-run-makefile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = "#include <stdio.h>
int main(void) { return 0; }
";
        std::fs::write(dir.join("main.c"), main).unwrap();
        std::fs::write(
            dir.join("makefile"),
            "EXECUTABLE = prog
default:
	gcc -o $(EXECUTABLE) main.c
",
        )
        .unwrap();
        match plan(&dir.join("main.c"), main, Some(&dir)) {
            Ok(Outcome::Run(p)) => {
                assert_eq!(p.cwd, dir);
                assert!(p.command.contains("make"), "{}", p.command);
                if cfg!(windows) {
                    assert!(p.command.ends_with(r"if ($?) { & '.\prog.exe' }"), "{}", p.command);
                } else {
                    assert!(p.command.ends_with("&& ./prog"), "{}", p.command);
                }
            }
            // Without make here, it says so instead.
            Err(why) => assert!(crate::c_build::make_program().is_none() && why.contains("make"), "{why}"),
            Ok(other) => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_preview_runs_the_program_or_else_its_tests() {
        let dir = std::env::temp_dir().join(format!("rc-preview-plans-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "not a manifest [").unwrap();
        // Data has neither, and says why.
        let why = preview_plans(&dir.join("a.json"), "{}", Some(&dir)).unwrap_err();
        assert!(why.contains("isn't a program"), "{why}");
        let plans = tests_of(&dir.join("src").join("lib.rs"), "", Some(&dir)).unwrap();
        assert_eq!(plans.command, "cargo test");
        assert_eq!(comparable(&plans.cwd), comparable(&dir));
        assert_eq!(tests_of(&dir.join("plain.py"), "x = 1", Some(&dir)), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

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
    fn tests_are_found_in_other_languages() {
        let lines = |name: &str, source: &str| test_lines(Path::new(name), source);
        let ids = |found: Vec<(usize, String)>| found.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
        let go = "package a\nfunc TestOne(t *testing.T) {}\nfunc Testing() {}\nfunc BenchmarkB(b *testing.B) {}\nfunc (s S) TestM() {}\nfunc helper() {}\n";
        assert_eq!(lines("a_test.go", go)[0], (1, "TestOne".to_owned()));
        assert_eq!(ids(lines("a_test.go", go)), ["TestOne", "BenchmarkB"]);
        assert!(lines("a.go", go).is_empty());
        let py = "import unittest\ndef test_a():\n    def test_inner(): pass\nclass T(unittest.TestCase):\n    def test_b(self):\n        pass\n    def other(self): pass\n    async def test_c(self): pass\ndef helper(): pass\n";
        assert_eq!(ids(lines("test_x.py", py)), ["test_a", "T.test_b", "T.test_c"]);
        assert_eq!(ids(lines("x_test.py", "def test_a(): pass\n")), ["test_a"]);
        assert!(lines("x.py", py).is_empty());
        let js = "describe('d', () => {\n  it('adds up', () => {});\n});\ntest(\"two\", () => {});\nconst test2 = 1;\ntest(name, () => {});\n";
        assert_eq!(ids(lines("a.test.js", js)), ["d", "adds up", "two"]);
        assert_eq!(ids(lines("a.spec.ts", js)), ["d", "adds up", "two"]);
        assert!(lines("a.js", js).is_empty());
    }

    #[test]
    fn test_commands_name_one_test() {
        let plan = |name: &str, source: &str, id: &str| test_plan(Path::new(name), source, id, None).map(|p| p.command);
        if which("python").is_some() {
            let unit = plan("/p/test_x.py", "import unittest", "T.test_b").unwrap();
            assert!(unit.contains("-m unittest") && unit.contains("test_x.T.test_b"), "{unit}");
            assert!(plan("/p/test_x.py", "", "test_a").unwrap().contains("-m pytest"));
        }
        if which("node").is_some() {
            let js = plan("/p/a.test.js", "", "adds (up)").unwrap();
            assert!(js.contains(&quote(r"--test-name-pattern=^adds \(up\)$")), "{js}");
        }
    }

    #[test]
    fn debug_needs_a_cargo_project() {
        assert!(debug_actions(Path::new("/no/such/place/a.rs"), None).unwrap_err().contains("Cargo projects"));
        let found = rust_actions(Path::new("/w/app/tests/smoke.rs"), &fixture()).unwrap();
        assert_eq!((found[0].flag, found[0].name.as_str()), ("test", "smoke"));
    }

    #[test]
    fn java_main_needs_the_right_shape() {
        assert!(java_main("class A { void main() {} }").is_some());
        assert!(java_main("class A { public static void main(String[] a) {} }").is_some());
        assert!(java_main("class A { static void main(String... a) {} }").is_some());
        assert!(java_main("class A { int main() { return 0; } }").is_none());
        assert!(java_main("class A { void main(int a) {} }").is_none());
        assert!(java_main("class A { void main(String a, String b) {} }").is_none());
    }

    #[test]
    fn c_main_is_found_where_it_really_is() {
        assert!(c_has_main(Lang::C, "#ifdef X\nint main(void) { return 0; }\n#endif\n"));
        assert!(c_has_main(Lang::Cpp, "#if 1\n#else\nint main() { return 0; }\n#endif\n"));
        assert!(c_has_main(Lang::Cpp, "extern \"C\" { int main() { return 0; } }\n"));
        assert!(!c_has_main(Lang::Cpp, "namespace n { int main() { return 0; } }\n"));
    }

    #[test]
    fn dotnet_rules() {
        assert!(dotnet_is_program("<OutputType>Exe</OutputType>"));
        assert!(dotnet_is_program("<OutputType> WinExe </OutputType>"));
        assert!(dotnet_is_program("<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>"));
        assert!(!dotnet_is_program("<OutputType>Library</OutputType>"));
        assert!(!dotnet_is_program("<Project Sdk=\"Microsoft.NET.Sdk\"></Project>"));
        let sln = "Project(\"{FAE}\") = \"App\", \"src\\App\\App.csproj\", \"{1}\"\nGlobal\nEndGlobal\n";
        assert_eq!(solution_projects(sln), [PathBuf::from("src/App/App.csproj")]);
        let slnx = "<Solution>\n  <Project Path=\"lib/Lib.csproj\" />\n</Solution>\n";
        assert_eq!(solution_projects(slnx), [PathBuf::from("lib/Lib.csproj")]);
    }

    #[test]
    fn build_files_are_read() {
        let cmake = "# add_executable(commented x.c)\nproject(demo C)\nadd_library(core a.c)\nadd_executable(tool t.c)\n\
                     ADD_EXECUTABLE(${PROJECT_NAME} m.c)\nadd_executable(old ALIAS tool)\nadd_executable(${OTHER} o.c)\n";
        assert_eq!(cmake_executables(cmake), ["tool", "demo"]);
        assert!(cmake_executables("add_library(core a.c)\n").is_empty());
        let make = "CC := gcc\nall: app\n\tgcc\n.PHONY: all\nrun: all\n\t./app\n%.o: %.c\n\t$(CC)\n# note: x\n";
        assert_eq!(make_targets(make), ["all", "run"]);
    }

    #[test]
    fn project_files_answer_for_their_project() {
        let tmp = std::env::temp_dir().join(format!("rust-code-run-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("lib")).unwrap();
        let write = |rel: &str, text: &str| std::fs::write(tmp.join(rel), text).unwrap();
        write("lib/Lib.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>");
        write("lib/Thing.cs", "class Thing {}");
        assert!(plan(&tmp.join("lib/Lib.csproj"), "", None).unwrap_err().contains("class library"));
        assert!(plan(&tmp.join("lib/Thing.cs"), "", None).unwrap_err().contains("class library"));
        write("package.json", "{ \"name\": \"x\" }");
        assert!(plan(&tmp.join("package.json"), "", None).unwrap_err().contains("no scripts"));
        write("CMakeLists.txt", "add_library(core a.c)\n");
        assert!(plan(&tmp.join("CMakeLists.txt"), "", None).unwrap_err().contains("no program"));
        assert!(plan(&tmp.join("a.c"), "int f(void){return 0;}", None).unwrap_err().contains("no program"));
        assert!(plan(&tmp.join("App.tsx"), "", None).unwrap_err().contains("build step"));
        // A package without a go.mod needs its own main.
        assert!(plan(Path::new("/no/such/a.go"), "package main\n", None).unwrap_err().contains("func main"));
        assert!(plan(Path::new("/no/such/a.go"), "package util\n", None).unwrap_err().contains("package main"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn only_refusals_about_the_file_are_final() {
        assert!(refusal_is_final("x is a library, so it has no program to run"));
        assert!(!refusal_is_final(&need("no-such-tool-here", "Go").unwrap_err()));
        assert!(!refusal_is_final(&plan(Path::new("a.xyz"), "", None).unwrap_err()));
    }

    #[test]
    fn a_stand_in_program_is_labelled() {
        let title = |file: &str| match rust_target(Path::new(file), &fixture()) {
            Ok(Outcome::Run(plan)) => plan.title,
            other => panic!("{other:?}"),
        };
        assert_eq!(title("/w/app/src/main.rs"), "app");
        assert_eq!(title("/w/app/src/util.rs"), "app (project)");
        assert_eq!(title("/w/picked/other.rs"), "b (project)");
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
        for name in ["package.json", "Makefile", "CMakeLists.txt", "Dockerfile", "compose.yaml", "a.sln", "a.csproj"] {
            assert!(may_run(Path::new(name)), "{name}");
        }
        for name in
            ["a.json", "a.css", "README.md", "a.h", "a.d.ts", "a.pyi", "a.xyz", "other.toml", "a.tsx", "notes.txt"]
        {
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
