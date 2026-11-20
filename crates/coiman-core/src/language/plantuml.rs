//! PlantUML diagrams, drawn by PlantUML itself: a `plantuml` program on PATH (Linux packages
//! ship one), or a `plantuml*.jar` in the config folder's `tools/plantuml` run with Java.
//! Coiman downloads nothing; the user puts PlantUML there.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::language::lsp::discovery::quiet_command;

/// How PlantUML is started: a program, and the arguments before its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runner {
    pub program: PathBuf,
    pub args: Vec<String>,
}

/// Where a PlantUML jar is looked for.
pub fn tools_dir() -> Option<PathBuf> {
    Some(crate::settings::config_dir()?.join("tools").join("plantuml"))
}

/// The newest `plantuml*.jar` in `dir` (sources and docs jars left out).
fn jar_in(dir: &Path) -> Option<PathBuf> {
    let mut jars: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            name.starts_with("plantuml")
                && name.ends_with(".jar")
                && !name.contains("sources")
                && !name.contains("javadoc")
        })
        .collect();
    // By the numbers in the name: 1.2026.10 after 1.2026.9.
    let numbers = |p: &PathBuf| -> Vec<u64> {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        name.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect()
    };
    jars.sort_by_key(numbers);
    jars.pop()
}

/// How to run PlantUML here, if it can be.
pub fn find() -> Option<Runner> {
    let which = crate::language::lsp::servers::which;
    if let Some(jar) = tools_dir().as_deref().and_then(jar_in) {
        let java = which("java")?;
        return Some(Runner { program: java, args: vec!["-jar".to_owned(), jar.to_string_lossy().into_owned()] });
    }
    which("plantuml").map(|program| Runner { program, args: Vec::new() })
}

/// What to say when PlantUML isn't there.
pub fn missing() -> String {
    let dir = tools_dir().map_or_else(|| "the config folder's tools/plantuml".to_owned(), |d| d.display().to_string());
    format!(
        "not installed: put a plantuml .jar (from plantuml.com or its GitHub releases) in {dir}, with Java on PATH, or install a plantuml program"
    )
}

/// Draws `source` as SVG. `@startuml`/`@enduml` are added when missing.
pub fn svg(runner: &Runner, source: &str, dark: bool) -> Result<String, String> {
    let source = if source.trim_start().starts_with("@start") {
        source.to_owned()
    } else {
        format!("@startuml\n{}\n@enduml\n", source.trim_end())
    };
    let mut cmd = quiet_command(&runner.program);
    cmd.args(&runner.args).args(["-tsvg", "-pipe", "-charset", "UTF-8"]);
    if dark {
        cmd.arg("--dark-mode");
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", runner.program.display()))?;
    if let Some(mut input) = child.stdin.take() {
        input.write_all(source.as_bytes()).map_err(|e| e.to_string())?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    // A diagram with an error is still an SVG (PlantUML draws the error), but exits non-zero.
    if text.contains("<svg") {
        return Ok(text[text.find("<svg").unwrap_or(0)..].to_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(if err.is_empty() { "PlantUML drew nothing".to_owned() } else { err })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jars_found_and_sources_left_out() {
        let dir = std::env::temp_dir().join(format!("coiman-puml-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(jar_in(&dir).is_none());
        for name in [
            "plantuml-mit-1.2026.9.jar",
            "plantuml-mit-1.2026.10-sources.jar",
            "notes.txt",
            "plantuml-mit-1.2026.10.jar",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        assert!(jar_in(&dir).unwrap().ends_with("plantuml-mit-1.2026.10.jar"));
        assert!(!jar_in(&dir).unwrap().to_string_lossy().contains("sources"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Draws with the PlantUML installed here, if one is (skipped otherwise).
    #[test]
    fn draws_when_installed() {
        let Some(runner) = find() else { return };
        let svg = svg(&runner, "Alice -> Bob: hello", false).unwrap();
        assert!(svg.starts_with("<svg") && svg.contains("Alice"), "{}", &svg[..svg.len().min(200)]);
    }
}
