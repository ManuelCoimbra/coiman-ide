//! Cargo integration without a UI: run configurations (`.coiman/launch.toml`),
//! test discovery from source with tree-sitter, `cargo test` / `cargo nextest`
//! command lines and output parsing, and `cargo check`/`clippy` JSON messages.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::language::lsp::discovery::quiet_command;

// ----- run configurations -----------------------------------------------------------

/// One entry of `.coiman/launch.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunConfig {
    pub name: String,
    /// A shell command line, e.g. `cargo run --release -- --verbose`.
    pub command: String,
    /// Working folder, relative to the workspace (default: the workspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// A pattern for this command's errors, with groups named `file`, `line`,
    /// `message`, and optionally `col` and `severity`. Replaces the built-in ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct LaunchFile {
    #[serde(default)]
    configurations: Vec<RunConfig>,
}

/// `<workspace>/.coiman/launch.toml`: committed with the project so the
/// whole team shares the same run configurations.
pub fn launch_file(root: &Path) -> PathBuf {
    crate::settings::project_dir(root).join("launch.toml")
}

/// Configurations from the launch file; `Err` explains a broken file.
pub fn load_configs(root: &Path) -> Result<Option<Vec<RunConfig>>, String> {
    let path = launch_file(root);
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(None) };
    let file: LaunchFile = toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))?;
    Ok(Some(file.configurations))
}

/// Configurations Cargo implies: run each binary, test, check, clippy.
pub fn default_configs(root: &Path) -> Vec<RunConfig> {
    if !root.join("Cargo.toml").is_file() {
        return Vec::new();
    }
    let packages = packages(root).unwrap_or_default();
    let mut configs: Vec<RunConfig> = packages
        .iter()
        .flat_map(|p| p.binaries().map(move |t| (p, t)))
        .map(|(package, bin)| RunConfig {
            name: format!("Run {}", bin.name),
            command: format!("cargo run -p {} --bin {}", package.name, bin.name),
            cwd: None,
            env: BTreeMap::new(),
            matcher: None,
        })
        .collect();
    let plain = |name: &str, command: &str| RunConfig {
        name: name.into(),
        command: command.into(),
        cwd: None,
        env: BTreeMap::new(),
        matcher: None,
    };
    configs.push(plain("Test workspace", "cargo test --workspace"));
    configs.push(plain("Check workspace", "cargo check --workspace --all-targets"));
    configs.push(plain("Clippy workspace", "cargo clippy --workspace --all-targets"));
    configs
}

/// Text for a new launch file listing `configs`.
pub fn launch_file_contents(configs: &[RunConfig]) -> String {
    // An empty list would be written as `configurations = []`, which clashes
    // with the `[[configurations]]` the user adds below it.
    let body = if configs.is_empty() {
        String::new()
    } else {
        toml::to_string_pretty(&LaunchFile { configurations: configs.to_vec() }).unwrap_or_default()
    };
    format!(
        "# Coiman IDE run configurations. Commit this file to share them.\n\
         # Each entry: name, command (run in a terminal tab), optional cwd and env.\n\
         #\n\
         # [[configurations]]\n\
         # name = \"Run with logs\"\n\
         # command = \"cargo run --release\"\n\
         # env = {{ RUST_LOG = \"debug\" }}\n\n{body}"
    )
}

/// One thing Cargo can build from a package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoTarget {
    pub name: String,
    /// "bin", "lib", "example", "test", "bench", "custom-build"...
    pub kinds: Vec<String>,
    pub crate_types: Vec<String>,
    /// The file the target starts from.
    pub src_path: PathBuf,
}

impl CargoTarget {
    pub fn is(&self, kind: &str) -> bool {
        self.kinds.iter().any(|k| k == kind)
    }
}

/// A workspace member, as `cargo metadata` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoPackage {
    pub name: String,
    pub dir: PathBuf,
    pub default_run: Option<String>,
    pub targets: Vec<CargoTarget>,
}

impl CargoPackage {
    pub fn binaries(&self) -> impl Iterator<Item = &CargoTarget> {
        self.targets.iter().filter(|t| t.is("bin"))
    }
}

/// Reads the output of `cargo metadata`.
pub fn parse_packages(meta: &serde_json::Value) -> Vec<CargoPackage> {
    let strings = |value: &serde_json::Value| -> Vec<String> {
        value.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect()
    };
    let path = |value: &serde_json::Value| value.as_str().map(PathBuf::from);
    meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|package| {
            let targets = package["targets"].as_array().into_iter().flatten();
            Some(CargoPackage {
                name: package["name"].as_str()?.to_owned(),
                dir: path(&package["manifest_path"])?.parent()?.to_path_buf(),
                default_run: package["default_run"].as_str().map(str::to_owned),
                targets: targets
                    .filter_map(|target| {
                        Some(CargoTarget {
                            name: target["name"].as_str()?.to_owned(),
                            kinds: strings(&target["kind"]),
                            crate_types: strings(&target["crate_types"]),
                            src_path: path(&target["src_path"])?,
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

/// The workspace members (`cargo metadata --no-deps`). `None` when Cargo can't say.
pub fn packages(root: &Path) -> Option<Vec<CargoPackage>> {
    if !crate::project::trust::is_trusted(root) {
        return None;
    }
    let out = quiet_command(Path::new("cargo"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    serde_json::from_slice(&out.stdout).ok().map(|meta| parse_packages(&meta))
}

/// Binary targets of the workspace members.
pub fn binaries(root: &Path) -> Vec<String> {
    let packages = packages(root).unwrap_or_default();
    packages.iter().flat_map(CargoPackage::binaries).map(|t| t.name.clone()).collect()
}

// ----- building for the debugger -----------------------------------------------------

/// An executable cargo built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    /// Target name (binary, test file, or package for unit tests).
    pub name: String,
    /// First target kind: "bin", "lib", "test"…
    pub kind: String,
    /// Built as a test harness (`cargo test --no-run`).
    pub test: bool,
    pub executable: PathBuf,
}

/// Runs `cargo <args>` with JSON messages in `dir` and returns the
/// executables built; on failure, the end of cargo's error output.
pub fn build_executables(dir: &Path, args: &[String]) -> Result<Vec<Artifact>, String> {
    crate::project::trust::check(dir)?;
    let out = quiet_command(Path::new("cargo"))
        .args(args)
        .arg("--message-format=json-render-diagnostics")
        .current_dir(dir)
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        return Err(tail[tail.len().saturating_sub(15)..].join("\n"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_artifact).collect())
}

fn parse_artifact(line: &str) -> Option<Artifact> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["reason"] != "compiler-artifact" {
        return None;
    }
    let executable = PathBuf::from(v["executable"].as_str()?);
    Some(Artifact {
        name: v["target"]["name"].as_str()?.to_owned(),
        kind: v["target"]["kind"][0].as_str().unwrap_or("").to_owned(),
        test: v["profile"]["test"].as_bool().unwrap_or(false),
        executable,
    })
}

/// Cargo arguments that build the test binary for `target`, and how to
/// pick it out of the artifacts.
pub fn test_build_args(package: &str, target: &Target) -> (Vec<String>, String, &'static str) {
    let mut args = vec!["test".to_owned(), "--no-run".to_owned(), "-p".to_owned(), package.to_owned()];
    let (name, kind) = match target {
        Target::Lib => {
            args.push("--lib".into());
            (package.replace('-', "_"), "lib")
        }
        Target::Bin(n) => {
            args.extend(["--bin".into(), n.clone()]);
            (n.clone(), "bin")
        }
        Target::Test(n) => {
            args.extend(["--test".into(), n.clone()]);
            (n.clone(), "test")
        }
    };
    (args, name, kind)
}

// ----- tests ------------------------------------------------------------------------

/// The compiled test binary a test lives in.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    Lib,
    Bin(String),
    /// An integration test file under `tests/`.
    Test(String),
}

/// Where a source file's tests run: package, target and module path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTarget {
    pub package: String,
    /// Folder of the package's Cargo.toml.
    pub dir: PathBuf,
    pub target: Target,
    /// Module path of the file within the target.
    pub modules: Vec<String>,
}

impl FileTarget {
    /// Name of the test binary as cargo/libtest report it (`-` becomes `_`).
    pub fn binary(&self) -> String {
        match &self.target {
            Target::Lib => self.package.replace('-', "_"),
            Target::Bin(name) | Target::Test(name) => name.replace('-', "_"),
        }
    }
}

/// Works out the package and target of a `.rs` file from its location.
pub fn file_target(path: &Path) -> Option<FileTarget> {
    let (dir, package) = path.ancestors().skip(1).find_map(|d| {
        let text = std::fs::read_to_string(d.join("Cargo.toml")).ok()?;
        let table: toml::Table = text.parse().ok()?;
        let name = table.get("package")?.get("name")?.as_str()?.to_owned();
        Some((d.to_path_buf(), name))
    })?;
    let rel: Vec<String> =
        path.strip_prefix(&dir).ok()?.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let stem = |s: &str| s.strip_suffix(".rs").unwrap_or(s).to_owned();
    let (target, mut modules) = match rel.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["src", "lib.rs"] => (Target::Lib, vec![]),
        ["src", "main.rs"] => (Target::Bin(package.clone()), vec![]),
        ["src", "bin", file] => (Target::Bin(stem(file)), vec![]),
        ["src", "bin", name, rest @ ..] => (Target::Bin((*name).to_owned()), rest.iter().map(|s| stem(s)).collect()),
        ["src", rest @ ..] => {
            let target = if dir.join("src/lib.rs").exists() { Target::Lib } else { Target::Bin(package.clone()) };
            (target, rest.iter().map(|s| stem(s)).collect())
        }
        ["tests", file] => (Target::Test(stem(file)), vec![]),
        ["tests", name, rest @ ..] => (Target::Test((*name).to_owned()), rest.iter().map(|s| stem(s)).collect()),
        _ => return None,
    };
    if matches!(modules.last().map(String::as_str), Some("mod" | "main")) {
        modules.pop();
    }
    Some(FileTarget { package, dir, target, modules })
}

/// [`file_target`], going by what Cargo says when the file starts a target:
/// a `[[bin]]` or `[[test]]` can live anywhere.
pub fn file_target_in(path: &Path, packages: &[CargoPackage]) -> Option<FileTarget> {
    let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    let own = packages.iter().find_map(|p| Some((p, p.targets.iter().find(|t| same(&t.src_path, path))?)));
    if let Some((package, target)) = own {
        let kind = if target.is("bin") {
            Target::Bin(target.name.clone())
        } else if target.is("test") {
            Target::Test(target.name.clone())
        } else if target.is("lib") {
            Target::Lib
        } else {
            return None;
        };
        return Some(FileTarget {
            package: package.name.clone(),
            dir: package.dir.clone(),
            target: kind,
            modules: Vec::new(),
        });
    }
    file_target(path)
}

/// A test function or a module containing tests, found in source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestItem {
    /// Path within the target, e.g. `buffer::tests::undo_works`.
    pub path: String,
    pub name: String,
    /// 0-based line of the function or module name.
    pub line: usize,
    /// 0-based last line of the item.
    pub end_line: usize,
    pub is_module: bool,
}

/// Finds `#[test]` functions (and the modules holding them) in Rust source.
/// `modules` is the file's own module path.
pub fn discover_tests(source: &str, modules: &[String]) -> Vec<TestItem> {
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&tree_sitter_rust::LANGUAGE.into()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else { return Vec::new() };
    let mut out = Vec::new();
    walk(tree.root_node(), source.as_bytes(), &mut modules.to_vec(), &mut out);
    out
}

fn walk(node: tree_sitter::Node, src: &[u8], path: &mut Vec<String>, out: &mut Vec<TestItem>) {
    let mut cursor = node.walk();
    let mut attrs: Vec<String> = Vec::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "attribute_item" => {
                attrs.push(child.utf8_text(src).unwrap_or("").to_owned());
                continue;
            }
            "function_item" if attrs.iter().any(|a| is_test_attribute(a)) => {
                if let Some(name) = child.child_by_field_name("name").and_then(|n| n.utf8_text(src).ok()) {
                    let line = child.child_by_field_name("name").map_or(0, |n| n.start_position().row);
                    let full = path.iter().map(String::as_str).chain([name]).collect::<Vec<_>>().join("::");
                    let end_line = child.end_position().row;
                    out.push(TestItem { path: full, name: name.to_owned(), line, end_line, is_module: false });
                }
            }
            "mod_item" => {
                if let (Some(name), Some(body)) = (
                    child.child_by_field_name("name").and_then(|n| n.utf8_text(src).ok()),
                    child.child_by_field_name("body"),
                ) {
                    let before = out.len();
                    path.push(name.to_owned());
                    walk(body, src, path, out);
                    if out.len() > before {
                        let line = child.child_by_field_name("name").map_or(0, |n| n.start_position().row);
                        let end_line = child.end_position().row;
                        let item =
                            TestItem { path: path.join("::"), name: name.to_owned(), line, end_line, is_module: true };
                        out.insert(before, item);
                    }
                    path.pop();
                }
            }
            _ => {}
        }
        attrs.clear();
    }
}

/// `#[test]`, `#[tokio::test]`, `#[rstest]`, `#[test_case(..)]`…
fn is_test_attribute(attr: &str) -> bool {
    let inner = attr.trim().trim_start_matches("#[").trim_end_matches(']');
    let path = inner.split(['(', ' ', '=']).next().unwrap_or("");
    let last = path.rsplit("::").next().unwrap_or("");
    matches!(last, "test" | "rstest" | "test_case")
}

/// Every test in the workspace under `root`: (file, its target, tests).
/// Respects `.gitignore` and skips `target`.
pub fn scan_workspace(root: &Path) -> Vec<(PathBuf, FileTarget, Vec<TestItem>)> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root).filter_entry(|e| e.file_name() != "target").build();
    for entry in walker.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        // Cheap check before parsing: most files have no tests.
        if !text.contains("test") {
            continue;
        }
        let Some(target) = file_target(path) else { continue };
        let items = discover_tests(&text, &target.modules);
        if !items.is_empty() {
            out.push((path.to_path_buf(), target, items));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The workspace a package belongs to: the outermost Cargo.toml with a
/// `[workspace]` table above `dir`, else `dir` itself.
pub fn workspace_dir(dir: &Path) -> PathBuf {
    dir.ancestors()
        .filter(|d| {
            std::fs::read_to_string(d.join("Cargo.toml")).is_ok_and(|t| t.lines().any(|l| l.trim() == "[workspace]"))
        })
        .last()
        .unwrap_or(dir)
        .to_path_buf()
}

/// How tests are run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    Cargo,
    Nextest,
}

/// Whether `cargo nextest` is installed.
pub fn has_nextest() -> bool {
    quiet_command(Path::new("cargo"))
        .args(["nextest", "--version"])
        // Not the app's folder: its rust-toolchain.toml would choose what runs.
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// What to run: everything, or tests under a path in one target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestScope {
    Workspace,
    Package(String),
    /// `exact`: a single test; otherwise every test under `path` (all of the
    /// target when `path` is empty).
    Filter {
        package: String,
        target: Target,
        path: String,
        exact: bool,
    },
}

/// Arguments after `cargo` for running `scope`.
pub fn test_args(runner: Runner, scope: &TestScope) -> Vec<String> {
    let mut args: Vec<String> = match runner {
        Runner::Cargo => vec!["test".into(), "--no-fail-fast".into()],
        Runner::Nextest => vec!["nextest".into(), "run".into(), "--no-fail-fast".into()],
    };
    let target_args = |target: &Target| -> Vec<String> {
        match target {
            Target::Lib => vec!["--lib".into()],
            Target::Bin(n) => vec!["--bin".into(), n.clone()],
            Target::Test(n) => vec!["--test".into(), n.clone()],
        }
    };
    match scope {
        TestScope::Workspace => args.push("--workspace".into()),
        TestScope::Package(p) => args.extend(["-p".into(), p.clone()]),
        TestScope::Filter { package, target, path, exact } => {
            args.extend(["-p".into(), package.clone()]);
            args.extend(target_args(target));
            if path.is_empty() {
                return args;
            }
            match (runner, exact) {
                (Runner::Cargo, true) => args.extend(["--".into(), path.clone(), "--exact".into()]),
                (Runner::Cargo, false) => args.extend(["--".into(), format!("{path}::")]),
                (Runner::Nextest, true) => args.extend(["-E".into(), format!("test(={path})")]),
                (Runner::Nextest, false) => args.push(format!("{path}::")),
            }
        }
    }
    args
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    Passed,
    Failed,
    Ignored,
}

/// Tracks which test binary output belongs to while parsing a run.
#[derive(Default)]
pub struct OutputParser {
    binary: String,
}

impl OutputParser {
    /// Reads one output line; returns (test binary, test path, outcome) for a result.
    pub fn line(&mut self, runner: Runner, line: &str) -> Option<(String, String, Outcome)> {
        let t = line.trim();
        match runner {
            Runner::Cargo => {
                // "Running unittests src/lib.rs (target/debug/deps/coiman_core-1a2b.exe)"
                if let Some(rest) = t.strip_prefix("Running ") {
                    let exe = rest.rsplit(['/', '\\']).next().unwrap_or("").trim_end_matches(')');
                    let exe = exe.strip_suffix(".exe").unwrap_or(exe);
                    self.binary = exe.rsplit_once('-').map_or(exe, |(name, _)| name).to_owned();
                    return None;
                }
                // "test buffer::tests::undo ... ok"
                let rest = t.strip_prefix("test ")?;
                let (path, result) = rest.rsplit_once(" ... ")?;
                let outcome = match result.trim() {
                    "ok" => Outcome::Passed,
                    "FAILED" => Outcome::Failed,
                    r if r.starts_with("ignored") => Outcome::Ignored,
                    _ => return None,
                };
                Some((self.binary.clone(), path.trim().to_owned(), outcome))
            }
            Runner::Nextest => {
                // "PASS [   0.004s] coiman-core buffer::tests::undo"
                let (word, rest) = t.split_once(' ')?;
                let outcome = match word {
                    "PASS" => Outcome::Passed,
                    "FAIL" | "SIGSEGV" | "SIGABRT" | "TIMEOUT" | "LEAK-FAIL" => Outcome::Failed,
                    "SKIP" => Outcome::Ignored,
                    _ => return None,
                };
                let rest = rest.trim_start();
                let rest = if rest.starts_with('[') { rest.split_once(']')?.1.trim_start() } else { rest };
                let (binary_id, path) = rest.split_once(' ')?;
                let binary = match binary_id.split_once("::") {
                    Some((_, kind)) => kind.strip_prefix("bin/").unwrap_or(kind),
                    None => binary_id,
                };
                Some((binary.replace('-', "_"), path.trim().to_owned(), outcome))
            }
        }
    }
}

// ----- cargo check / clippy ----------------------------------------------------------

/// A compiler diagnostic from `cargo check/clippy --message-format=json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CargoDiagnostic {
    pub path: PathBuf,
    /// 0-based line and column ranges (columns in chars).
    pub start: (u32, u32),
    pub end: (u32, u32),
    /// 1 error, 2 warning, 3 note/help.
    pub severity: u8,
    pub message: String,
    pub code: Option<String>,
}

/// Parses one JSON line of cargo output (`workspace` resolves relative paths).
pub fn parse_message(line: &str, workspace: &Path) -> Option<CargoDiagnostic> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v["reason"] != "compiler-message" {
        return None;
    }
    let msg = &v["message"];
    let severity = match msg["level"].as_str()? {
        "error" | "error: internal compiler error" => 1,
        "warning" => 2,
        "note" | "help" => 3,
        _ => return None,
    };
    let span = msg["spans"].as_array()?.iter().find(|s| s["is_primary"] == true)?;
    let n = |key: &str| span[key].as_u64().map(|x| x.saturating_sub(1) as u32);
    let file = span["file_name"].as_str()?;
    let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { workspace.join(file) };
    Some(CargoDiagnostic {
        path,
        start: (n("line_start")?, n("column_start")?),
        end: (n("line_end")?, n("column_end")?),
        severity,
        message: msg["message"].as_str()?.to_owned(),
        code: msg["code"]["code"].as_str().map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cargo_folders_get_cargo_configurations() {
        let dir = std::env::temp_dir().join(format!("coiman-configs-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".coiman")).unwrap();
        assert!(default_configs(&dir).is_empty());
        // The file offered to a folder with none must accept what's added to it.
        let added = "\n[[configurations]]\nname = \"Serve\"\ncommand = \"npm start\"\n";
        std::fs::write(launch_file(&dir), launch_file_contents(&[]) + added).unwrap();
        let configs = load_configs(&dir).unwrap().unwrap();
        assert_eq!(configs.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Serve"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_tests_in_nested_modules() {
        let src = r#"
fn helper() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds() {}

    #[tokio::test]
    async fn waits() {}

    fn not_a_test() {}

    mod inner {
        #[test]
        #[ignore]
        fn deep() {}
    }
}
"#;
        let items = discover_tests(src, &["buffer".to_owned()]);
        let paths: Vec<(&str, bool)> = items.iter().map(|t| (t.path.as_str(), t.is_module)).collect();
        assert_eq!(
            paths,
            vec![
                ("buffer::tests", true),
                ("buffer::tests::adds", false),
                ("buffer::tests::waits", false),
                ("buffer::tests::inner", true),
                ("buffer::tests::inner::deep", false),
            ]
        );
        assert_eq!(items[1].line, 8);
    }

    #[test]
    fn builds_test_commands() {
        let scope = TestScope::Filter { package: "core".into(), target: Target::Lib, path: "a::b".into(), exact: true };
        assert_eq!(test_args(Runner::Cargo, &scope).join(" "), "test --no-fail-fast -p core --lib -- a::b --exact");
        assert_eq!(
            test_args(Runner::Nextest, &scope).join(" "),
            "nextest run --no-fail-fast -p core --lib -E test(=a::b)"
        );
        let module = TestScope::Filter {
            package: "app".into(),
            target: Target::Test("api".into()),
            path: "m".into(),
            exact: false,
        };
        assert_eq!(test_args(Runner::Cargo, &module).join(" "), "test --no-fail-fast -p app --test api -- m::");
    }

    #[test]
    fn parses_test_output() {
        let mut p = OutputParser::default();
        assert_eq!(
            p.line(Runner::Cargo, "     Running unittests src/lib.rs (target\\debug\\deps\\coiman_core-1a2b3c.exe)"),
            None
        );
        assert_eq!(
            p.line(Runner::Cargo, "test buffer::tests::undo ... ok"),
            Some(("coiman_core".into(), "buffer::tests::undo".into(), Outcome::Passed))
        );
        assert_eq!(p.line(Runner::Cargo, "test a::b ... FAILED").map(|r| r.2), Some(Outcome::Failed));
        assert_eq!(p.line(Runner::Cargo, "test a::c ... ignored, slow").map(|r| r.2), Some(Outcome::Ignored));
        let mut n = OutputParser::default();
        assert_eq!(
            n.line(Runner::Nextest, "        PASS [   0.004s] coiman-core buffer::tests::undo"),
            Some(("coiman_core".into(), "buffer::tests::undo".into(), Outcome::Passed))
        );
        assert_eq!(
            n.line(Runner::Nextest, "        FAIL [   0.100s] app::bin/my-tool cli::parses"),
            Some(("my_tool".into(), "cli::parses".into(), Outcome::Failed))
        );
    }

    #[test]
    fn parses_build_artifacts() {
        let line = r#"{"reason":"compiler-artifact","target":{"name":"my_crate","kind":["lib"]},"profile":{"test":true},"executable":"C:/t/debug/deps/my_crate-123.exe"}"#;
        let a = parse_artifact(line).unwrap();
        assert_eq!((a.name.as_str(), a.kind.as_str(), a.test), ("my_crate", "lib", true));
        assert!(parse_artifact(r#"{"reason":"compiler-artifact","target":{"name":"x","kind":["lib"]},"profile":{"test":false},"executable":null}"#).is_none());
    }

    #[test]
    fn parses_compiler_messages() {
        let line = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused variable: `x`","code":{"code":"unused_variables"},"spans":[{"file_name":"src/main.rs","is_primary":true,"line_start":3,"line_end":3,"column_start":9,"column_end":10}]}}"#;
        let d = parse_message(line, Path::new("/ws")).unwrap();
        assert_eq!((d.path, d.start, d.end, d.severity), (Path::new("/ws").join("src/main.rs"), (2, 8), (2, 9), 2));
        assert_eq!(d.code.as_deref(), Some("unused_variables"));
        assert!(parse_message(r#"{"reason":"build-finished","success":true}"#, Path::new("/")).is_none());
    }

    #[test]
    fn maps_files_to_targets() {
        let dir = std::env::temp_dir().join(format!("coiman-cargo-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/bin")).unwrap();
        std::fs::create_dir_all(dir.join("tests")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"my-crate\"\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        let t = |rel: &str| file_target(&dir.join(rel)).map(|f| (f.target.clone(), f.modules.join("::"), f.binary()));
        assert_eq!(t("src/lib.rs"), Some((Target::Lib, String::new(), "my_crate".into())));
        assert_eq!(t("src/a/b.rs"), Some((Target::Lib, "a::b".into(), "my_crate".into())));
        assert_eq!(t("src/a/mod.rs"), Some((Target::Lib, "a".into(), "my_crate".into())));
        assert_eq!(t("src/bin/tool.rs"), Some((Target::Bin("tool".into()), String::new(), "tool".into())));
        assert_eq!(t("tests/api.rs"), Some((Target::Test("api".into()), String::new(), "api".into())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
