//! Tests in any language: an adapter says how to run a project's tests and in what
//! form the results come back, and everything else is the same for all of them.
//! Adapters are data, so a project can add its own in `.rust-code/tests.toml`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::SystemTime;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use regex::Regex;
use serde::Deserialize;

use crate::lsp::discovery::quiet_command;
use crate::run::{python_program, q, quote};
use crate::stacktrace;

/// How a runner reports its results.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// `go test -json`, on stdout.
    GoJson,
    /// JUnit XML, in a report file or folder.
    JUnit,
    /// Visual Studio's `.trx`, in a report file.
    Trx,
    /// Test Anything Protocol, on stdout.
    Tap,
    /// `python -m unittest -v`, as text.
    Unittest,
    /// `rspec --format json`, in a report file.
    Rspec,
    /// `jest --json`, in a report file.
    Jest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
    Skipped,
}

/// One test's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    /// A class, package or file.
    pub suite: String,
    pub name: String,
    pub outcome: Outcome,
    pub millis: Option<u64>,
    pub message: String,
    /// Where the runner says the test is.
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl TestResult {
    fn new(suite: &str, name: &str, outcome: Outcome) -> Self {
        Self {
            suite: suite.to_owned(),
            name: name.to_owned(),
            outcome,
            millis: None,
            message: String::new(),
            file: None,
            line: None,
        }
    }
}

/// Which built-in rules build a command for one test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Go,
    Pytest,
    Unittest,
    Node,
    Vitest,
    Jest,
    Dotnet,
    Maven,
    Gradle,
    Rspec,
    Custom,
}

/// How to run a project's tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    pub kind: Kind,
    /// `{report}` becomes where the results should go.
    pub command: String,
    /// The same for one test: `{id}`, `{file}` and `{line}` are filled in too.
    pub single: Option<String>,
    pub format: Format,
    /// A folder the runner writes reports in by itself, from the project's folder.
    pub report_dir: Option<String>,
    pub cwd: PathBuf,
    /// File extensions whose tests this runs.
    pub extensions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterEntry {
    name: String,
    command: String,
    format: Format,
    #[serde(default)]
    single: Option<String>,
    #[serde(default)]
    report_dir: Option<String>,
    #[serde(default)]
    extensions: Vec<String>,
}

#[derive(Deserialize)]
struct AdaptersFile {
    #[serde(default)]
    adapter: Vec<AdapterEntry>,
}

/// The project's own adapters.
pub fn custom_adapters(root: &Path) -> Result<Vec<Adapter>, String> {
    let path = root.join(".rust-code").join("tests.toml");
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(Vec::new()) };
    let file: AdaptersFile = toml::from_str(&text).map_err(|e| format!("tests.toml: {}", e.message()))?;
    Ok(file
        .adapter
        .into_iter()
        .map(|e| Adapter {
            name: e.name,
            kind: Kind::Custom,
            command: e.command,
            single: e.single,
            format: e.format,
            report_dir: e.report_dir,
            cwd: root.to_path_buf(),
            extensions: e.extensions,
        })
        .collect())
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_python_test(name: &str) -> bool {
    name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
}

fn is_js_test(name: &str) -> bool {
    [".test.", ".spec."].iter().any(|m| name.contains(m))
        && ["js", "mjs", "cjs", "ts", "mts", "cts", "jsx", "tsx"].iter().any(|e| name.ends_with(&format!(".{e}")))
}

/// Whether `python -m pytest` works, asked from outside the project so its code doesn't load.
fn has_pytest(python_path: &Path) -> bool {
    quiet_command(python_path)
        .args(["-m", "pytest", "--version"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// The adapters a project calls for, from its files. Cargo has its own view.
pub fn detect(root: &Path, files: &[PathBuf]) -> Result<Vec<Adapter>, String> {
    let mut found = custom_adapters(root)?;
    let names: Vec<String> = files.iter().map(|f| name_of(f)).collect();
    let any = |test: &dyn Fn(&str) -> bool| names.iter().any(|n| test(n));
    let at_root = |name: &str| root.join(name).is_file();
    let add = |found: &mut Vec<Adapter>,
               name: &str,
               kind,
               command: String,
               single: Option<String>,
               format,
               report_dir: Option<&str>,
               ext: &[&str]| {
        found.push(Adapter {
            name: name.to_owned(),
            kind,
            command,
            single,
            format,
            report_dir: report_dir.map(str::to_owned),
            cwd: root.to_path_buf(),
            extensions: strings(ext),
        });
    };
    if at_root("go.mod") || any(&|n| n.ends_with("_test.go")) {
        let single = "go test -json -run {id} {dir}".to_owned();
        add(
            &mut found,
            "go test",
            Kind::Go,
            "go test -json ./...".to_owned(),
            Some(single),
            Format::GoJson,
            None,
            &["go"],
        );
    }
    if any(&is_python_test) {
        let anchor = files.iter().find(|f| is_python_test(&name_of(f))).map_or(root, PathBuf::as_path);
        if let Ok(python) = python_program(anchor, Some(root)) {
            let path = crate::environment::resolve(anchor.parent().unwrap_or(root), Some(root)).map(|e| e.python);
            if path.as_deref().is_some_and(has_pytest) {
                let command = format!("{python} -m pytest -q --junitxml={{report}}");
                let single = format!("{python} -m pytest -q --junitxml={{report}} {{node}}");
                add(&mut found, "pytest", Kind::Pytest, command, Some(single), Format::JUnit, None, &["py"]);
            } else {
                let command = format!("{python} -m unittest discover -v");
                let single = format!("{python} -m unittest -v {{module}}.{{id}}");
                add(&mut found, "unittest", Kind::Unittest, command, Some(single), Format::Unittest, None, &["py"]);
            }
        }
    }
    let manifest = std::fs::read_to_string(root.join("package.json")).unwrap_or_default();
    const JS: &[&str] = &["js", "mjs", "cjs", "ts", "mts", "cts", "jsx", "tsx"];
    if manifest.contains("\"vitest\"") {
        let command = "npx vitest run --reporter=junit --outputFile={report}".to_owned();
        let single = format!("{command} {{file}} -t {{id}}");
        add(&mut found, "Vitest", Kind::Vitest, command, Some(single), Format::JUnit, None, JS);
    } else if manifest.contains("\"jest\"") {
        let command = "npx jest --json --outputFile={report}".to_owned();
        let single = format!("{command} {{file}} -t {{id}}");
        add(&mut found, "Jest", Kind::Jest, command, Some(single), Format::Jest, None, JS);
    } else if any(&is_js_test) {
        let command = "node --test --test-reporter=tap".to_owned();
        let single = "node --test --test-reporter=tap --test-name-pattern={pattern} {file}".to_owned();
        add(&mut found, "node --test", Kind::Node, command, Some(single), Format::Tap, None, JS);
    }
    let dotnet_tests: Vec<&PathBuf> = files
        .iter()
        .filter(|f| name_of(f).ends_with("proj"))
        .filter(|f| {
            std::fs::read_to_string(f).is_ok_and(|t| t.contains("Microsoft.NET.Test.Sdk") || t.contains("MSTest.Sdk"))
        })
        .collect();
    if !dotnet_tests.is_empty() {
        let command = "dotnet test --nologo --logger 'trx;LogFileName={report}'".to_owned();
        let single = format!("{command} --filter FullyQualifiedName~{{id}}");
        const CLR: &[&str] = &["cs", "fs", "vb"];
        // `dotnet test` needs a solution or project in the folder it starts in.
        let whole = names.iter().zip(files).any(|(n, f)| {
            f.parent() == Some(root) && (n.ends_with(".sln") || n.ends_with(".slnx") || n.ends_with("proj"))
        });
        if whole {
            add(&mut found, "dotnet test", Kind::Dotnet, command, Some(single), Format::Trx, None, CLR);
        } else {
            for project in dotnet_tests {
                let stem = project.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let at = found.len();
                let name = format!("dotnet test ({stem})");
                add(&mut found, &name, Kind::Dotnet, command.clone(), Some(single.clone()), Format::Trx, None, CLR);
                if let Some(dir) = project.parent() {
                    found[at].cwd = dir.to_path_buf();
                }
            }
        }
    }
    if at_root("pom.xml") {
        let program = if at_root(if cfg!(windows) { "mvnw.cmd" } else { "mvnw" }) { "./mvnw" } else { "mvn" };
        let command = format!("{program} -q test");
        let single = format!("{command} -Dtest={{class}}#{{id}}");
        add(
            &mut found,
            "Maven",
            Kind::Maven,
            command,
            Some(single),
            Format::JUnit,
            Some("target/surefire-reports"),
            &["java", "kt"],
        );
    }
    if at_root("build.gradle") || at_root("build.gradle.kts") {
        let wrapper = at_root(if cfg!(windows) { "gradlew.bat" } else { "gradlew" });
        let command = format!("{} test", if wrapper { "./gradlew" } else { "gradle" });
        let single = format!("{command} --tests *{{class}}.{{id}}");
        add(
            &mut found,
            "Gradle",
            Kind::Gradle,
            command,
            Some(single),
            Format::JUnit,
            Some("build/test-results/test"),
            &["java", "kt"],
        );
    }
    if any(&|n| n.ends_with("_spec.rb")) {
        let command = "bundle exec rspec --format json --out {report}".to_owned();
        let single = format!("{command} {{file}}:{{line}}");
        add(&mut found, "RSpec", Kind::Rspec, command, Some(single), Format::Rspec, None, &["rb"]);
    }
    Ok(found)
}

/// A report file for this run.
pub fn report_file(format: Format) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join("rust-code-tests");
    let _ = std::fs::create_dir_all(&dir);
    let ext = match format {
        Format::Trx => "trx",
        Format::JUnit => "xml",
        _ => "json",
    };
    dir.join(format!("{}-{}.{ext}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

impl Adapter {
    /// A report's place, as the command line needs it: `.trx` names sit inside a
    /// quoted logger argument already.
    fn report_text(&self, report: &Path) -> String {
        if self.format == Format::Trx { report.to_string_lossy().into_owned() } else { q(report) }
    }

    /// The command for everything, sending reports to `report`.
    pub fn command_for_all(&self, report: &Path) -> String {
        self.command.replace("{report}", &self.report_text(report))
    }

    /// The command for everything with coverage written into `folder`, for runners
    /// that can do it with no project changes.
    pub fn command_with_coverage(&self, report: &Path, folder: &Path) -> Option<String> {
        let all = self.command_for_all(report);
        let inside = |name: &str| q(&folder.join(name));
        Some(match self.kind {
            Kind::Go => format!("go test -json -coverprofile={} ./...", inside("coverage.out")),
            Kind::Node => format!(
                "node --test --experimental-test-coverage --test-reporter=tap --test-reporter-destination=stdout \
                 --test-reporter=lcov --test-reporter-destination={}",
                inside("lcov.info")
            ),
            Kind::Pytest => format!("{all} --cov --cov-report=lcov:{}", inside("lcov.info")),
            Kind::Dotnet => format!("{all} --collect:'XPlat Code Coverage' --results-directory {}", q(folder)),
            Kind::Jest => format!("{all} --coverage --coverageReporters=lcov --coverageDirectory={}", q(folder)),
            Kind::Vitest => {
                format!("{all} --coverage --coverage.reporter=lcovonly --coverage.reportsDirectory={}", q(folder))
            }
            _ => return None,
        })
    }

    /// The command for one test of `file`, if this runner can be told.
    pub fn command_for_one(&self, report: &Path, root: &Path, file: &Path, id: &str, line: usize) -> Option<String> {
        let single = self.single.as_ref()?;
        let rel = file.strip_prefix(root).unwrap_or(file);
        let module = rel.with_extension("").to_string_lossy().replace(['/', '\\'], ".");
        let dir = rel.parent().map_or(String::new(), |d| d.to_string_lossy().replace('\\', "/"));
        let dir = if dir.is_empty() { ".".to_owned() } else { format!("./{dir}") };
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let last = id.rsplit('.').next().unwrap_or(id);
        let filled = single
            .replace("{report}", &self.report_text(report))
            .replace(
                "{node}",
                &quote(&format!("{}::{}", rel.to_string_lossy().replace('\\', "/"), id.replace('.', "::"))),
            )
            .replace("{module}", &module)
            .replace("{pattern}", &quote(&format!("^{}$", crate::run::regex_escape(id))))
            .replace("{dir}", &quote(&dir))
            .replace("{file}", &q(file))
            .replace("{class}", &stem)
            .replace("{line}", &(line + 1).to_string());
        let id_text = match self.kind {
            Kind::Go => quote(&format!("^{id}$")),
            Kind::Unittest => id.to_owned(),
            Kind::Maven | Kind::Gradle | Kind::Dotnet => last.to_owned(),
            _ => quote(id),
        };
        Some(filled.replace("{id}", &id_text))
    }
}

/// Where a test run's results are, once it has ended.
pub struct Collect<'a> {
    pub format: Format,
    pub stdout: &'a str,
    pub report: &'a Path,
    pub report_dir: Option<PathBuf>,
    pub started: SystemTime,
}

/// Reads a finished run's results. An unreadable report counts as none: the caller
/// then says the run reported nothing, with the exit code.
pub fn collect(run: &Collect) -> Vec<TestResult> {
    let file = || std::fs::read_to_string(run.report).unwrap_or_default();
    match run.format {
        Format::GoJson => parse_go_json(run.stdout),
        Format::Tap => parse_tap(run.stdout),
        Format::Unittest => parse_unittest(run.stdout),
        Format::Rspec => parse_rspec(&file()),
        Format::Jest => parse_jest(&file()),
        Format::Trx => parse_trx(&file()),
        Format::JUnit => match &run.report_dir {
            None => parse_junit(&file()),
            Some(dir) => {
                let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
                // Only this run's: old reports stay behind in these folders.
                read.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "xml"))
                    .filter(|p| std::fs::metadata(p).and_then(|m| m.modified()).is_ok_and(|m| m >= run.started))
                    .flat_map(|p| parse_junit(&std::fs::read_to_string(p).unwrap_or_default()))
                    .collect()
            }
        },
    }
}

// ----- formats -----------------------------------------------------------------------

/// `go test -json`, one event per line. Anything else on the way is left out.
pub fn parse_go_json(text: &str) -> Vec<TestResult> {
    let mut out: Vec<TestResult> = Vec::new();
    let mut logs: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();
    for line in text.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let (package, test) = (event["Package"].as_str().unwrap_or(""), event["Test"].as_str());
        let Some(test) = test else {
            // A package that didn't build has no tests to fail.
            if event["Action"] == "fail" && !out.iter().any(|r| r.suite == package) {
                let mut failed = TestResult::new(package, "(package)", Outcome::Failed);
                failed.message = logs.remove(&(package.to_owned(), String::new())).unwrap_or_default();
                out.push(failed);
            } else if event["Action"] == "output" {
                logs.entry((package.to_owned(), String::new()))
                    .or_default()
                    .push_str(event["Output"].as_str().unwrap_or(""));
            }
            continue;
        };
        let key = (package.to_owned(), test.to_owned());
        let outcome = match event["Action"].as_str() {
            Some("output") => {
                logs.entry(key).or_default().push_str(event["Output"].as_str().unwrap_or(""));
                continue;
            }
            Some("pass") => Outcome::Passed,
            Some("fail") => Outcome::Failed,
            Some("skip") => Outcome::Skipped,
            _ => continue,
        };
        let mut result = TestResult::new(package, test, outcome);
        result.millis = event["Elapsed"].as_f64().map(|s| (s * 1000.0) as u64);
        if outcome == Outcome::Failed {
            result.message = logs.remove(&key).unwrap_or_default();
        }
        out.push(result);
    }
    out
}

/// TAP as `node --test` writes it: subtests indented above their parent, and a
/// YAML block after each result. A parent that's only a suite isn't a test.
pub fn parse_tap(text: &str) -> Vec<TestResult> {
    let result_line = Regex::new(r"^(\s*)(not )?ok \d+(?: - (.*))?$").expect("the pattern is valid");
    let location = Regex::new(r"^(.*):(\d+):\d+$").expect("the pattern is valid");
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<(usize, TestResult)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(c) = result_line.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let indent = c[1].len();
        let failed = c.get(2).is_some();
        let raw = c.get(3).map_or("", |m| m.as_str());
        let (name, directive) = raw.split_once(" # ").unwrap_or((raw, ""));
        let skipped = {
            let d = directive.to_uppercase();
            d.starts_with("SKIP") || d.starts_with("TODO")
        };
        let outcome = match (failed, skipped) {
            (_, true) => Outcome::Skipped,
            (true, _) => Outcome::Failed,
            _ => Outcome::Passed,
        };
        let mut result = TestResult::new("", name.trim(), outcome);
        let mut is_suite = false;
        i += 1;
        if lines.get(i).is_some_and(|l| l.trim() == "---") {
            i += 1;
            let mut in_block = false;
            let mut message = String::new();
            while let Some(line) = lines.get(i) {
                let t = line.trim();
                if t == "..." {
                    i += 1;
                    break;
                }
                let value = |key: &str| t.strip_prefix(key).map(|v| v.trim().trim_matches('\'').to_owned());
                if let Some(ms) = value("duration_ms:") {
                    result.millis = ms.parse::<f64>().ok().map(|m| m as u64);
                    in_block = false;
                } else if t == "type: 'suite'" {
                    is_suite = true;
                    in_block = false;
                } else if let Some(place) = value("location:") {
                    if let Some(p) = location.captures(&place) {
                        result.file = Some(p[1].to_owned());
                        result.line = p[2].parse().ok();
                    }
                    in_block = false;
                } else if let Some(error) = value("error:") {
                    if error != "|-" {
                        message.push_str(&error);
                        message.push('\n');
                    }
                    in_block = error == "|-";
                } else if t.starts_with("stack:") {
                    in_block = true;
                } else if in_block {
                    message.push_str(t);
                    message.push('\n');
                }
                i += 1;
            }
            if outcome == Outcome::Failed {
                result.message = message;
            }
        }
        // Subtests come first, and more indented: this one is their suite.
        let name = result.name.clone();
        for (depth, child) in out.iter_mut().rev() {
            if *depth <= indent {
                break;
            }
            if child.suite.is_empty() {
                child.suite = name.clone();
            } else {
                child.suite = format!("{name} > {}", child.suite);
            }
            // Claimed: the next one at this level isn't its parent too.
            *depth = indent;
        }
        if !is_suite {
            out.push((indent, result));
        }
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// `python -m unittest -v`: a line per test, then the failures in full.
pub fn parse_unittest(text: &str) -> Vec<TestResult> {
    let row = Regex::new(r"^(\w+) \(([\w.]+)\)(?:\n.*)? \.\.\. (.*)$").expect("the pattern is valid");
    let failure = Regex::new(r"^(?:FAIL|ERROR): (\w+) \(([\w.]+)\)").expect("the pattern is valid");
    let mut out: Vec<TestResult> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let suite_of = |path: &str, name: &str| path.strip_suffix(&format!(".{name}")).unwrap_or(path).to_owned();
    for line in &lines {
        if let Some(c) = row.captures(line) {
            let status = c[3].trim();
            let outcome = if status == "ok" {
                Outcome::Passed
            } else if status.starts_with("skipped") {
                Outcome::Skipped
            } else {
                Outcome::Failed
            };
            out.push(TestResult::new(&suite_of(&c[2], &c[1]), &c[1], outcome));
        }
    }
    let mut i = 0;
    while i < lines.len() {
        let Some(c) = failure.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let (name, suite) = (c[1].to_owned(), suite_of(&c[2], &c[1]));
        i += 1;
        let mut message = Vec::new();
        while i < lines.len() && !lines[i].starts_with("=====") && !lines[i].starts_with("Ran ") {
            if !lines[i].starts_with("-----") {
                message.push(lines[i]);
            }
            i += 1;
        }
        let text = message.join("\n").trim().to_owned();
        match out.iter_mut().find(|r| r.name == name && r.suite == suite) {
            Some(result) => {
                result.outcome = Outcome::Failed;
                result.message = text;
            }
            None => {
                let mut result = TestResult::new(&suite, &name, Outcome::Failed);
                result.message = text;
                out.push(result);
            }
        }
    }
    out
}

fn attr(tag: &BytesStart, name: &str) -> Option<String> {
    tag.attributes().flatten().find(|a| a.key.as_ref() == name.as_bytes()).and_then(|a| {
        let raw = String::from_utf8_lossy(&a.value).into_owned();
        Some(quick_xml::escape::unescape(&raw).ok()?.into_owned())
    })
}

/// What `&name;` stands for. Windows line ends arrive as `&#xD;`, and are dropped.
fn entity_text(name: &str) -> String {
    let number = match name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => name.strip_prefix('#').and_then(|d| d.parse().ok()),
    };
    match number.and_then(char::from_u32) {
        Some('\r') => String::new(),
        Some(character) => character.to_string(),
        None => quick_xml::escape::resolve_predefined_entity(name).map_or_else(|| format!("&{name};"), str::to_owned),
    }
}

/// JUnit XML: pytest, Surefire, Gradle, Vitest and many others.
pub fn parse_junit(xml: &str) -> Vec<TestResult> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut current: Option<TestResult> = None;
    let mut in_failure = false;
    let start_case = |tag: &BytesStart| {
        let name = attr(tag, "name").unwrap_or_default();
        let mut result = TestResult::new(&attr(tag, "classname").unwrap_or_default(), &name, Outcome::Passed);
        result.millis = attr(tag, "time").and_then(|t| t.parse::<f64>().ok()).map(|s| (s * 1000.0) as u64);
        result.file = attr(tag, "file");
        result.line = attr(tag, "line").and_then(|l| l.parse().ok());
        result
    };
    let mark = |result: &mut TestResult, tag: &BytesStart| match tag.name().as_ref() {
        b"failure" | b"error" => {
            result.outcome = Outcome::Failed;
            if let Some(message) = attr(tag, "message") {
                result.message = message;
                result.message.push('\n');
            }
            true
        }
        b"skipped" => {
            result.outcome = Outcome::Skipped;
            false
        }
        _ => false,
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) if tag.name().as_ref() == b"testcase" => current = Some(start_case(&tag)),
            Ok(Event::Empty(tag)) if tag.name().as_ref() == b"testcase" => out.push(start_case(&tag)),
            Ok(Event::Start(tag)) => {
                if let Some(result) = &mut current {
                    in_failure = mark(result, &tag);
                }
            }
            Ok(Event::Empty(tag)) => {
                if let Some(result) = &mut current {
                    mark(result, &tag);
                }
            }
            Ok(Event::Text(text)) if in_failure => {
                if let (Some(result), Ok(text)) = (&mut current, text.decode()) {
                    let text = quick_xml::escape::unescape(&text).map(|t| t.into_owned()).unwrap_or_default();
                    result.message.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(entity)) if in_failure => {
                if let (Some(result), Ok(name)) = (&mut current, entity.decode()) {
                    result.message.push_str(&entity_text(&name));
                }
            }
            Ok(Event::CData(text)) if in_failure => {
                if let Some(result) = &mut current {
                    result.message.push_str(&String::from_utf8_lossy(&text));
                }
            }
            Ok(Event::End(tag)) => match tag.name().as_ref() {
                b"testcase" => out.extend(current.take()),
                b"failure" | b"error" => in_failure = false,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn trx_millis(duration: &str) -> Option<u64> {
    let mut parts = duration.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    let seconds = h.parse::<f64>().ok()? * 3600.0 + m.parse::<f64>().ok()? * 60.0 + s.parse::<f64>().ok()?;
    Some((seconds * 1000.0) as u64)
}

/// Visual Studio's results, from `dotnet test --logger trx`.
pub fn parse_trx(xml: &str) -> Vec<TestResult> {
    let mut reader = Reader::from_str(xml);
    let mut out: Vec<TestResult> = Vec::new();
    let mut current: Option<TestResult> = None;
    let mut reading = false;
    loop {
        match reader.read_event() {
            Ok(event @ (Event::Start(_) | Event::Empty(_))) => {
                let (Event::Start(tag) | Event::Empty(tag)) = &event else { continue };
                match tag.name().as_ref() {
                    b"UnitTestResult" => {
                        let outcome = match attr(tag, "outcome").as_deref() {
                            Some("Passed") => Outcome::Passed,
                            Some("Failed" | "Error" | "Timeout" | "Aborted") => Outcome::Failed,
                            _ => Outcome::Skipped,
                        };
                        let full = attr(tag, "testName").unwrap_or_default();
                        let (suite, name) = match full.split('(').next().unwrap_or(&full).rsplit_once('.') {
                            Some((suite, _)) => {
                                let at = suite.len() + 1;
                                (suite.to_owned(), full[at..].to_owned())
                            }
                            None => (String::new(), full.clone()),
                        };
                        let mut result = TestResult::new(&suite, &name, outcome);
                        result.millis = attr(tag, "duration").and_then(|d| trx_millis(&d));
                        if matches!(event, Event::Empty(_)) {
                            out.push(result);
                        } else {
                            current = Some(result);
                        }
                    }
                    b"Message" | b"StackTrace" => reading = current.is_some(),
                    _ => {}
                }
            }
            Ok(Event::Text(text)) if reading => {
                if let (Some(result), Ok(text)) = (&mut current, text.decode()) {
                    let text = quick_xml::escape::unescape(&text).map(|t| t.into_owned()).unwrap_or_default();
                    result.message.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(entity)) if reading => {
                if let (Some(result), Ok(name)) = (&mut current, entity.decode()) {
                    result.message.push_str(&entity_text(&name));
                }
            }
            Ok(Event::End(tag)) => match tag.name().as_ref() {
                b"UnitTestResult" => out.extend(current.take()),
                b"Message" | b"StackTrace" => {
                    reading = false;
                    if let Some(result) = &mut current {
                        result.message.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// `rspec --format json`.
pub fn parse_rspec(text: &str) -> Vec<TestResult> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let examples = json["examples"].as_array().cloned().unwrap_or_default();
    examples
        .iter()
        .map(|e| {
            let outcome = match e["status"].as_str() {
                Some("passed") => Outcome::Passed,
                Some("failed") => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            let mut result = TestResult::new(
                e["file_path"].as_str().unwrap_or(""),
                e["full_description"].as_str().or(e["description"].as_str()).unwrap_or(""),
                outcome,
            );
            result.millis = e["run_time"].as_f64().map(|s| (s * 1000.0) as u64);
            result.file = e["file_path"].as_str().map(str::to_owned);
            result.line = e["line_number"].as_u64().map(|l| l as u32);
            if let Some(message) = e["exception"]["message"].as_str() {
                result.message = message.to_owned();
                for line in e["exception"]["backtrace"].as_array().into_iter().flatten().filter_map(|l| l.as_str()) {
                    result.message.push('\n');
                    result.message.push_str(line);
                }
            }
            result
        })
        .collect()
}

/// `jest --json`.
pub fn parse_jest(text: &str) -> Vec<TestResult> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let mut out = Vec::new();
    for file in json["testResults"].as_array().into_iter().flatten() {
        let path = file["name"].as_str().unwrap_or("");
        for test in file["assertionResults"].as_array().into_iter().flatten() {
            let outcome = match test["status"].as_str() {
                Some("passed") => Outcome::Passed,
                Some("failed") => Outcome::Failed,
                _ => Outcome::Skipped,
            };
            let name = test["title"].as_str().or(test["fullName"].as_str()).unwrap_or("");
            let mut result = TestResult::new(path, name, outcome);
            result.file = Some(path.to_owned());
            result.millis = test["duration"].as_u64();
            let messages: Vec<&str> =
                test["failureMessages"].as_array().into_iter().flatten().filter_map(|m| m.as_str()).collect();
            result.message = messages.join("\n");
            out.push(result);
        }
    }
    out
}

// ----- finding tests in the text --------------------------------------------------------

/// The name of the method a test attribute sits above.
fn method_after(lines: &[&str], from: usize) -> Option<(usize, String)> {
    let call = Regex::new(r"(\w+)\s*\(").expect("the pattern is valid");
    for (at, line) in lines.iter().enumerate().skip(from).take(4) {
        let t = line.trim();
        if t.is_empty() || t.starts_with('[') || t.starts_with('@') {
            continue;
        }
        return call.captures(t).map(|c| (at, c[1].to_owned()));
    }
    None
}

/// Tests the shared test lines don't cover: annotated methods, and RSpec's `it`.
fn annotated_tests(file: &Path, source: &str) -> Vec<(usize, String)> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    match ext.as_str() {
        "java" | "kt" => {
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim();
                if ["@Test", "@ParameterizedTest", "@RepeatedTest"].iter().any(|a| t.starts_with(a)) {
                    found.extend(method_after(&lines, i + 1));
                }
            }
        }
        "cs" => {
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim();
                if ["[Fact", "[Theory", "[Test]", "[TestCase", "[TestMethod"].iter().any(|a| t.starts_with(a)) {
                    found.extend(method_after(&lines, i + 1));
                }
            }
        }
        "rb" if name_of(file).ends_with("_spec.rb") => {
            let it = Regex::new(r#"^\s*(?:it|specify)\s+["'](.+?)["']"#).expect("the pattern is valid");
            for (i, line) in lines.iter().enumerate() {
                if let Some(c) = it.captures(line) {
                    found.push((i, c[1].to_owned()));
                }
            }
        }
        _ => {}
    }
    found
}

/// The tests a file defines, as (0-based line, id).
pub fn discover_in_file(file: &Path, source: &str) -> Vec<(usize, String)> {
    let found = crate::run::test_lines(file, source);
    if found.is_empty() { annotated_tests(file, source) } else { found }
}

/// Whether a result is for this test of this file.
pub fn matches(result: &TestResult, file: &Path, id: &str) -> bool {
    let last = id.rsplit('.').next().unwrap_or(id);
    // Parameters come after the name: `Many(x: 1)`, `test_many[1]`.
    let name = result.name.split(['(', '[']).next().unwrap_or(&result.name);
    let same_name = name == id || name == last || result.name == id || name.ends_with(&format!(" {last}"));
    if !same_name {
        return false;
    }
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match (&result.file, result.suite.as_str()) {
        (Some(place), _) => {
            let norm = |p: &str| p.replace('\\', "/").to_lowercase();
            let (place, file) = (norm(place), norm(&file.to_string_lossy()));
            file.ends_with(&place)
                || place.ends_with(&file)
                || place.ends_with(&format!("/{}", stem.to_lowercase()))
                || place.rsplit('/').next() == file.rsplit('/').next()
        }
        (None, "") => true,
        // Suites name a file, class, module or package. A class needn't be named
        // like its file, but its namespace usually follows the folders.
        (None, suite) => {
            let segments: Vec<String> = suite.split(['.', '/', '\\']).map(str::to_lowercase).collect();
            let folders = file.iter().map(|p| p.to_string_lossy().to_lowercase());
            let lower = suite.to_lowercase();
            // A folder can hold dots itself: `Lib.Tests` for the namespace `Lib.Tests.Calc`.
            let in_path = folders.into_iter().any(|p| segments.contains(&p) || lower.starts_with(&format!("{p}.")));
            suite.contains(&stem) || in_path || segments.len() == 1
        }
    }
}

/// Where a failure happened in this project, read from its message: the deepest
/// frame the trace has here, which is the first in most languages but Python's.
pub fn failure_place(result: &TestResult, root: &Path, files: &[PathBuf]) -> Option<(PathBuf, u32)> {
    let frames: Vec<(PathBuf, u32)> = result
        .message
        .lines()
        .filter_map(stacktrace::parse_frame)
        .filter_map(|f| match stacktrace::locate(&f, root, files) {
            stacktrace::Place::Project(path) => Some((path, f.line)),
            _ => None,
        })
        .collect();
    let python = result.message.contains("Traceback (most recent call last)");
    let frame = if python { frames.last() } else { frames.first() };
    frame.cloned().or_else(|| {
        let line = result.line?;
        let named = result.file.as_deref()?;
        let path = Path::new(named);
        let found = if path.is_absolute() { path.to_path_buf() } else { root.join(path) };
        found.is_file().then_some((found, line))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_go() {
        let text = r#"{"Action":"run","Package":"a/b","Test":"TestOk"}
{"Action":"output","Package":"a/b","Test":"TestOk","Output":"ok\n"}
{"Action":"pass","Package":"a/b","Test":"TestOk","Elapsed":0.25}
{"Action":"output","Package":"a/b","Test":"TestBad","Output":"  x_test.go:9: boom\n"}
{"Action":"fail","Package":"a/b","Test":"TestBad","Elapsed":0}
{"Action":"skip","Package":"a/b","Test":"TestSkip"}
{"Action":"fail","Package":"a/b"}
not json
{"Action":"output","Package":"c","Output":"c.go:1: syntax error\n"}
{"Action":"fail","Package":"c"}"#;
        let found = parse_go_json(text);
        assert_eq!(found.len(), 4);
        assert_eq!((found[0].outcome, found[0].millis), (Outcome::Passed, Some(250)));
        assert_eq!(found[1].outcome, Outcome::Failed);
        assert!(found[1].message.contains("boom"));
        assert_eq!(found[2].outcome, Outcome::Skipped);
        assert_eq!((found[3].name.as_str(), found[3].suite.as_str()), ("(package)", "c"));
        assert!(found[3].message.contains("syntax error"));
    }

    const TAP: &str = "TAP version 13
# Subtest: group
    # Subtest: inner good
    ok 1 - inner good
      ---
      duration_ms: 1.5
      type: 'test'
      ...
    # Subtest: inner bad
    not ok 2 - inner bad
      ---
      duration_ms: 2.5
      type: 'test'
      location: 'C:\\w\\a.test.js:7:3'
      failureType: 'testCodeFailure'
      error: 'expected 1'
      code: 'ERR_ASSERTION'
      stack: |-
        TestContext.<anonymous> (C:\\w\\a.test.js:8:5)
      ...
    1..2
not ok 1 - group
  ---
  duration_ms: 5
  type: 'suite'
  ...
# Subtest: top
ok 2 - top # SKIP not today
  ---
  duration_ms: 0.1
  type: 'test'
  ...
# tests 3
";

    #[test]
    fn reads_tap() {
        let found = parse_tap(TAP);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!((found[0].name.as_str(), found[0].suite.as_str()), ("inner good", "group"));
        assert_eq!(found[0].millis, Some(1));
        let bad = &found[1];
        assert_eq!(bad.outcome, Outcome::Failed);
        assert_eq!((bad.file.as_deref(), bad.line), (Some("C:\\w\\a.test.js"), Some(7)));
        assert!(bad.message.contains("expected 1") && bad.message.contains("a.test.js:8:5"));
        assert_eq!((found[2].name.as_str(), found[2].outcome), ("top", Outcome::Skipped));
    }

    #[test]
    fn reads_unittest() {
        let text = "test_a (test_m.Case.test_a) ... ok
test_b (test_m.Case.test_b) ... FAIL
test_c (test_m.Case.test_c) ... ERROR
test_d (test_m.Case.test_d) ... skipped 'later'

======================================================================
FAIL: test_b (test_m.Case.test_b)
----------------------------------------------------------------------
Traceback (most recent call last):
  File \"C:\\w\\test_m.py\", line 7, in test_b
    self.assertEqual(1, 2)
AssertionError: 1 != 2

======================================================================
ERROR: test_c (test_m.Case.test_c)
----------------------------------------------------------------------
ValueError: x

----------------------------------------------------------------------
Ran 4 tests in 0.001s
";
        let found = parse_unittest(text);
        let outcomes: Vec<Outcome> = found.iter().map(|r| r.outcome).collect();
        assert_eq!(outcomes, [Outcome::Passed, Outcome::Failed, Outcome::Failed, Outcome::Skipped]);
        assert_eq!(found[1].suite, "test_m.Case");
        assert!(found[1].message.contains("AssertionError: 1 != 2"));
        assert_eq!(found[2].message, "ValueError: x");
    }

    #[test]
    fn reads_junit() {
        let xml = r#"<?xml version="1.0"?><testsuites><testsuite name="s">
<testcase classname="a.B" name="good" time="0.5"/>
<testcase classname="a.B" name="bad" time="1" file="a/b.py" line="4"><failure message="1 &gt; 2">trace &amp; more</failure></testcase>
<testcase classname="a.B" name="err"><error message="oops"><![CDATA[in cdata]]></error></testcase>
<testcase classname="a.B" name="later"><skipped message="no"/></testcase>
</testsuite></testsuites>"#;
        let found = parse_junit(xml);
        assert_eq!(found.len(), 4);
        assert_eq!((found[0].outcome, found[0].millis), (Outcome::Passed, Some(500)));
        assert_eq!(found[1].outcome, Outcome::Failed);
        assert!(
            found[1].message.contains("1 > 2") && found[1].message.contains("trace & more"),
            "{}",
            found[1].message
        );
        assert_eq!((found[1].file.as_deref(), found[1].line), (Some("a/b.py"), Some(4)));
        assert!(found[2].message.contains("in cdata"));
        assert_eq!(found[3].outcome, Outcome::Skipped);
        assert!(parse_junit("").is_empty());
        assert!(parse_junit("<not xml").is_empty());
    }

    #[test]
    fn reads_trx() {
        let xml = r#"<TestRun><Results>
<UnitTestResult testName="Ns.Tests.Good" outcome="Passed" duration="00:00:00.5000000"/>
<UnitTestResult testName="Ns.Tests.Bad(x: 1.5)" outcome="Failed" duration="00:00:01">
<Output><ErrorInfo><Message>Assert failed&#xD;&#10;&lt;here&gt;</Message><StackTrace>at Ns.Tests.Bad() in C:\w\T.cs:line 9</StackTrace></ErrorInfo></Output>
</UnitTestResult>
<UnitTestResult testName="Plain" outcome="NotExecuted"/>
</Results></TestRun>"#;
        let found = parse_trx(xml);
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].suite.as_str(), found[0].name.as_str()), ("Ns.Tests", "Good"));
        assert_eq!(found[0].millis, Some(500));
        assert_eq!((found[1].suite.as_str(), found[1].name.as_str()), ("Ns.Tests", "Bad(x: 1.5)"));
        assert_eq!(found[1].outcome, Outcome::Failed);
        assert!(found[1].message.contains("T.cs:line 9"));
        assert!(found[1].message.contains("Assert failed\n<here>"), "{}", found[1].message);
        assert_eq!(found[2].outcome, Outcome::Skipped);
    }

    #[test]
    fn reads_rspec_and_jest() {
        let rspec = r#"{"examples":[{"full_description":"A works","status":"passed","file_path":"./spec/a_spec.rb","line_number":3,"run_time":0.1},
{"full_description":"A fails","status":"failed","file_path":"./spec/a_spec.rb","line_number":5,"exception":{"message":"boom","backtrace":["./spec/a_spec.rb:6:in `x'"]}},
{"full_description":"A later","status":"pending"}]}"#;
        let found = parse_rspec(rspec);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].line, Some(3));
        assert!(found[1].message.contains("a_spec.rb:6"));
        assert_eq!(found[2].outcome, Outcome::Skipped);
        let jest = r#"{"testResults":[{"name":"/w/a.test.js","assertionResults":[
{"title":"good","status":"passed","duration":3},{"title":"bad","status":"failed","failureMessages":["Error: x\n at /w/a.test.js:4:1"]},{"title":"later","status":"pending"}]}]}"#;
        let found = parse_jest(jest);
        assert_eq!(found.len(), 3);
        assert_eq!((found[0].millis, found[0].file.as_deref()), (Some(3), Some("/w/a.test.js")));
        assert!(found[1].message.contains("Error: x"));
        assert!(parse_rspec("nope").is_empty() && parse_jest("nope").is_empty());
    }

    fn project(files: &[&str]) -> (PathBuf, Vec<PathBuf>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "rc-testing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut all = Vec::new();
        for file in files {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "").unwrap();
            all.push(path);
        }
        (root, all)
    }

    #[test]
    fn detects_by_the_project_files() {
        let (root, files) = project(&["go.mod", "a_spec.rb", "pom.xml", "build.gradle", "x.test.js"]);
        let kinds: Vec<Kind> = detect(&root, &files).unwrap().iter().map(|a| a.kind).collect();
        assert_eq!(kinds, [Kind::Go, Kind::Node, Kind::Maven, Kind::Gradle, Kind::Rspec]);
        std::fs::write(root.join("package.json"), "{\"devDependencies\":{\"vitest\":\"1\"}}").unwrap();
        let kinds: Vec<Kind> = detect(&root, &files).unwrap().iter().map(|a| a.kind).collect();
        assert!(kinds.contains(&Kind::Vitest) && !kinds.contains(&Kind::Node));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_dotnet_project_needs_the_test_sdk() {
        let (root, files) = project(&["App/App.csproj"]);
        assert!(detect(&root, &files).unwrap().is_empty());
        std::fs::write(&files[0], "<PackageReference Include=\"Microsoft.NET.Test.Sdk\"/>").unwrap();
        assert_eq!(detect(&root, &files).unwrap()[0].kind, Kind::Dotnet);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dotnet_starts_where_it_can_find_a_project() {
        let (root, files) = project(&["src/App/App.csproj", "tests/A/A.csproj", "tests/B/B.csproj"]);
        for test in &files[1..] {
            std::fs::write(test, "Microsoft.NET.Test.Sdk").unwrap();
        }
        let found = detect(&root, &files).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].name.as_str(), found[0].cwd.clone()), ("dotnet test (A)", root.join("tests/A")));
        let solution = root.join("All.sln");
        std::fs::write(&solution, "").unwrap();
        let mut with = files.clone();
        with.push(solution);
        let found = detect(&root, &with).unwrap();
        assert_eq!((found.len(), found[0].cwd.clone()), (1, root.clone()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_project_can_add_its_own() {
        let (root, files) = project(&[".rust-code/tests.toml"]);
        std::fs::write(
            &files[0],
            "[[adapter]]\nname = \"Mine\"\ncommand = \"mine --report {report}\"\nformat = \"junit\"\nextensions = [\"zig\"]\n",
        )
        .unwrap();
        let found = detect(&root, &files).unwrap();
        assert_eq!((found[0].name.as_str(), found[0].kind, found[0].format), ("Mine", Kind::Custom, Format::JUnit));
        assert!(found[0].command_for_all(Path::new("r.xml")).contains("r.xml"));
        std::fs::write(&files[0], "[[adapter]]\nname = \"x\"\n").unwrap();
        assert!(detect(&root, &files).unwrap_err().contains("tests.toml"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn builds_commands_for_one_test() {
        let root = Path::new("/w");
        let (r, file) = (Path::new("/t/r"), Path::new("/w/pkg/x_test.go"));
        let go = Adapter {
            name: String::new(),
            kind: Kind::Go,
            command: String::new(),
            single: Some("go test -json -run {id} {dir}".to_owned()),
            format: Format::GoJson,
            report_dir: None,
            cwd: root.into(),
            extensions: Vec::new(),
        };
        let one = go.command_for_one(r, root, file, "TestA", 3).unwrap();
        assert!(one.contains("^TestA$") && one.contains("./pkg"), "{one}");
        let maven =
            Adapter { kind: Kind::Maven, single: Some("mvn test -Dtest={class}#{id}".to_owned()), ..go.clone() };
        assert_eq!(
            maven.command_for_one(r, root, Path::new("/w/s/CartTest.java"), "adds", 0).unwrap(),
            "mvn test -Dtest=CartTest#adds"
        );
        let unit =
            Adapter { kind: Kind::Unittest, single: Some("py -m unittest -v {module}.{id}".to_owned()), ..go.clone() };
        assert_eq!(
            unit.command_for_one(r, root, Path::new("/w/t/test_m.py"), "Case.test_a", 0).unwrap(),
            "py -m unittest -v t.test_m.Case.test_a"
        );
        let none = Adapter { single: None, ..go };
        assert_eq!(none.command_for_one(r, root, file, "TestA", 3), None);
    }

    #[test]
    fn finds_annotated_tests() {
        let java = "class T {\n  @Test\n  void adds() {}\n  @Test @Disabled\n  public void other() {}\n}";
        assert_eq!(discover_in_file(Path::new("T.java"), java), [(2, "adds".to_owned()), (4, "other".to_owned())]);
        let cs = "class T {\n  [Fact]\n  public void Adds() {}\n  [Theory]\n  [InlineData(1)]\n  public async Task Other(int x) {}\n}";
        assert_eq!(discover_in_file(Path::new("T.cs"), cs), [(2, "Adds".to_owned()), (5, "Other".to_owned())]);
        let rb = "describe 'x' do\n  it 'works' do\n  end\nend";
        assert_eq!(discover_in_file(Path::new("x_spec.rb"), rb), [(1, "works".to_owned())]);
        assert!(discover_in_file(Path::new("plain.rb"), rb).is_empty());
    }

    #[test]
    fn matches_results_to_tests() {
        let mut r = TestResult::new("test_m.Case", "test_a", Outcome::Passed);
        assert!(matches(&r, Path::new("/w/test_m.py"), "Case.test_a"));
        assert!(!matches(&r, Path::new("/w/test_m.py"), "Case.test_b"));
        assert!(!matches(&r, Path::new("/w/test_other.py"), "Case.test_a"));
        r.suite = "Ns.CartTests".to_owned();
        r.name = "Adds(x: 1)".to_owned();
        assert!(matches(&r, Path::new("/w/CartTests.cs"), "Adds"));
        r.suite = "Cart.CartTests".to_owned();
        assert!(matches(&r, Path::new("/w/Cart/UnitTest1.cs"), "Adds"));
        assert!(!matches(&r, Path::new("/w/Other/UnitTest1.cs"), "Adds"));
        r.suite = "Lib.Tests.CalcTests".to_owned();
        assert!(matches(&r, Path::new("/w/Lib.Tests/UnitTest1.cs"), "Adds"));
        let param = TestResult::new("test_cart", "test_many[2]", Outcome::Failed);
        assert!(matches(&param, Path::new("/w/test_cart.py"), "test_many"));
        let mut t = TestResult::new("", "first", Outcome::Passed);
        t.file = Some("C:\\w\\a.test.js".to_owned());
        assert!(matches(&t, Path::new("C:\\w\\a.test.js"), "first"));
        assert!(!matches(&t, Path::new("C:\\w\\b.test.js"), "first"));
    }

    #[test]
    fn finds_where_a_failure_happened() {
        let (root, files) = project(&["src/cart.py", "tests/test_cart.py", "src/Cart.java"]);
        let mut python = TestResult::new("s", "t", Outcome::Failed);
        python.message =
            "Traceback (most recent call last):\n  File \"tests/test_cart.py\", line 3, in t\n  File \"src/cart.py\", line 9, in add\nValueError".to_owned();
        assert_eq!(failure_place(&python, &root, &files), Some((root.join("src/cart.py"), 9)));
        let mut node = TestResult::new("s", "t", Outcome::Failed);
        node.message = "at f (src/cart.py:4:1)\nat g (tests/test_cart.py:5:1)".to_owned();
        assert_eq!(failure_place(&node, &root, &files), Some((root.join("src/cart.py"), 4)));
        let none = TestResult::new("s", "t", Outcome::Failed);
        assert_eq!(failure_place(&none, &root, &files), None);
        std::fs::remove_dir_all(root).unwrap();
    }
}
