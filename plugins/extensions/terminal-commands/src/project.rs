//! Commands a project offers, read from its files: `package.json` scripts, `Makefile`
//! targets, `justfile` recipes, `Taskfile.yml` tasks, `deno.json` tasks and Cargo's own.
//! Files are read through the editor (`read` takes a path relative to the open folder).

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectCommand {
    /// What to type: `npm run build`, `make test`.
    pub command: String,
    /// The file it comes from.
    pub source: &'static str,
    /// What it does, when the file says.
    pub detail: String,
}

/// Reads a file of the open folder, and lists a folder of it (name, is a folder).
pub struct Files<'a> {
    pub read: &'a dyn Fn(&str) -> Option<String>,
    pub list: &'a dyn Fn(&str) -> crate::history::Listing,
}

impl Files<'_> {
    fn first(&self, names: &[&str]) -> Option<String> {
        names.iter().find_map(|n| (self.read)(n))
    }

    fn exists(&self, name: &str) -> bool {
        let (dir, file) = name.rsplit_once('/').unwrap_or(("", name));
        (self.list)(dir).is_some_and(|l| l.iter().any(|(n, _)| n == file))
    }
}

fn add(out: &mut Vec<ProjectCommand>, command: String, source: &'static str, detail: impl Into<String>) {
    if !out.iter().any(|c| c.command == command) {
        out.push(ProjectCommand { command, source, detail: detail.into() });
    }
}

/// Every command found, file by file in the order above.
pub fn project_commands(files: &Files) -> Vec<ProjectCommand> {
    let mut out = Vec::new();
    package_json(files, &mut out);
    makefile(files, &mut out);
    justfile(files, &mut out);
    taskfile(files, &mut out);
    deno(files, &mut out);
    cargo(files, &mut out);
    out
}

fn package_json(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = (files.read)("package.json") else { return };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let runner = if files.exists("pnpm-lock.yaml") {
        "pnpm run"
    } else if files.exists("yarn.lock") {
        "yarn run"
    } else if files.exists("bun.lockb") || files.exists("bun.lock") {
        "bun run"
    } else {
        "npm run"
    };
    if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
        for (name, body) in scripts {
            add(out, format!("{runner} {name}"), "package.json", body.as_str().unwrap_or_default());
        }
    }
}

/// `name: deps` lines, without pattern rules, variables and special targets; the detail
/// is a `## comment` on the line, else the recipe's first line.
fn makefile(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = files.first(&["GNUmakefile", "makefile", "Makefile"]) else { return };
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(['\t', ' ', '#', '.']) {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let (targets, rest) = line.split_at(colon);
        if rest.starts_with(":=") || rest.starts_with("::=") || targets.contains(['=', '%', '$', '(']) {
            continue;
        }
        let comment = rest.split_once("##").map(|(_, c)| c.trim().to_owned());
        let recipe = lines.get(i + 1).filter(|l| l.starts_with('\t')).map(|l| l.trim().to_owned());
        for t in targets.split_whitespace() {
            add(out, format!("make {t}"), "Makefile", comment.clone().or(recipe.clone()).unwrap_or_default());
        }
    }
}

/// `name args: deps` recipes; the comment line above is the detail.
fn justfile(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = files.first(&["justfile", "Justfile", ".justfile"]) else { return };
    let mut comment = String::new();
    for line in text.lines() {
        if let Some(c) = line.strip_prefix('#') {
            comment = c.trim().to_owned();
            continue;
        }
        let recipe = (|| {
            if line.starts_with([' ', '\t', '[']) {
                return None;
            }
            let line = line.trim_start_matches('@');
            let colon = line.find(':')?;
            if line[colon..].starts_with(":=") {
                return None;
            }
            let mut words = line[..colon].split_whitespace();
            let name = words.next()?;
            let keyword = ["set", "alias", "import", "mod", "export"].contains(&name);
            if keyword || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                return None;
            }
            Some((name, words.collect::<Vec<_>>()))
        })();
        if let Some((name, params)) = recipe {
            let detail = if params.is_empty() { comment.clone() } else { format!("{comment} ({})", params.join(" ")) };
            add(out, format!("just {name}"), "justfile", detail.trim().to_owned());
        }
        comment.clear();
    }
}

/// The keys under `tasks:`, and each one's `desc:`.
fn taskfile(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = files.first(&["Taskfile.yml", "Taskfile.yaml", "taskfile.yml"]) else { return };
    let mut in_tasks = false;
    let mut indent = None;
    let mut current: Option<(String, String)> = None;
    let mut found = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let depth = line.len() - trimmed.len();
        if depth == 0 {
            in_tasks = trimmed.starts_with("tasks:");
            continue;
        }
        if !in_tasks {
            continue;
        }
        let task_depth = *indent.get_or_insert(depth);
        if depth == task_depth {
            if let Some(t) = current.take() {
                found.push(t);
            }
            if let Some(name) = trimmed.strip_suffix(':').or_else(|| trimmed.split_once(':').map(|(n, _)| n)) {
                current = Some((name.trim().trim_matches(['"', '\'']).to_owned(), String::new()));
            }
        } else if let (Some((_, desc)), Some(d)) = (&mut current, trimmed.strip_prefix("desc:"))
            && desc.is_empty()
        {
            *desc = d.trim().trim_matches(['"', '\'']).to_owned();
        }
    }
    found.extend(current);
    for (name, desc) in found {
        add(out, format!("task {name}"), "Taskfile.yml", desc);
    }
}

fn deno(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = files.first(&["deno.json", "deno.jsonc"]) else { return };
    // Comments are allowed in deno.jsonc: whole-line ones are dropped.
    let text: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    if let Some(tasks) = json.get("tasks").and_then(|s| s.as_object()) {
        for (name, body) in tasks {
            let detail = body.as_str().or_else(|| body.get("command").and_then(|c| c.as_str())).unwrap_or_default();
            add(out, format!("deno task {name}"), "deno.json", detail);
        }
    }
}

/// The value of `key = "value"` on a TOML line.
fn toml_string<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let (k, v) = line.split_once('=')?;
    (k.trim() == key).then(|| v.trim().trim_matches('"'))
}

fn cargo(files: &Files, out: &mut Vec<ProjectCommand>) {
    let Some(text) = (files.read)("Cargo.toml") else { return };
    let mut section = "";
    let mut bins = Vec::new();
    let (mut package, mut workspace) = (false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            section = line;
            package |= line == "[package]";
            workspace |= line == "[workspace]";
        } else if section == "[[bin]]"
            && let Some(name) = toml_string(line, "name")
        {
            bins.push(name.to_owned());
        }
    }
    for (name, is_dir) in (files.list)("src/bin").unwrap_or_default() {
        if is_dir {
            if files.exists(&format!("src/bin/{name}/main.rs")) {
                bins.push(name);
            }
        } else if let Some(stem) = name.strip_suffix(".rs") {
            bins.push(stem.to_owned());
        }
    }
    let main = files.exists("src/main.rs");
    add(out, "cargo build".into(), "Cargo.toml", "Build");
    if package && (main || !bins.is_empty()) || workspace {
        add(out, "cargo run".into(), "Cargo.toml", "Build and run");
    }
    add(out, "cargo test".into(), "Cargo.toml", "Run the tests");
    add(out, "cargo check".into(), "Cargo.toml", "Check without building");
    add(out, "cargo clippy".into(), "Cargo.toml", "Lints");
    add(out, "cargo fmt".into(), "Cargo.toml", "Format the code");
    bins.sort();
    bins.dedup();
    if bins.len() + usize::from(main) > 1 {
        for b in &bins {
            add(out, format!("cargo run --bin {b}"), "Cargo.toml", "Run this program");
        }
    }
    let mut examples: Vec<String> = (files.list)("examples")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(n, is_dir)| (!is_dir).then(|| n.strip_suffix(".rs").map(str::to_owned)).flatten())
        .collect();
    examples.sort();
    for x in examples {
        add(out, format!("cargo run --example {x}"), "Cargo.toml", "Run this example");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn commands_from_project_files() {
        let mut fs: HashMap<&str, &str> = HashMap::new();
        fs.insert("package.json", r#"{"scripts": {"dev": "vite", "test": "vitest run"}}"#);
        fs.insert("pnpm-lock.yaml", "");
        fs.insert(
            "Makefile",
            ".PHONY: all test\nCC := gcc\nall: build ## Everything\nbuild test:\n\tgo build ./...\n%.o: %.c\n\tcc $<\n",
        );
        fs.insert(
            "justfile",
            "set shell := [\"bash\"]\nversion := \"1\"\n# Serve the docs\nserve port=\"8000\":\n  python -m http.server {{port}}\n\nlint:\n  ruff .\n",
        );
        fs.insert("Taskfile.yml", "version: '3'\ntasks:\n  deploy:\n    desc: Ship it\n    cmds: [echo hi]\n  clean:\n    cmds: [rm -rf out]\nvars:\n  X: 1\n");
        fs.insert("deno.jsonc", "{\n  // tasks\n  \"tasks\": {\"start\": \"deno run main.ts\"}\n}\n");
        fs.insert("Cargo.toml", "[package]\nname = \"x\"\n\n[[bin]]\nname = \"extra\"\npath = \"x.rs\"\n");
        fs.insert("src/main.rs", "");
        fs.insert("src/bin/tool.rs", "");
        fs.insert("examples/demo.rs", "");
        let read = |p: &str| fs.get(p).map(|t| t.to_string());
        let list = |dir: &str| -> crate::history::Listing {
            let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
            let mut out: Vec<(String, bool)> = Vec::new();
            for path in fs.keys() {
                if let Some(rest) = path.strip_prefix(&prefix) {
                    let (name, is_dir) = match rest.split_once('/') {
                        Some((d, _)) => (d.to_owned(), true),
                        None => (rest.to_owned(), false),
                    };
                    if !out.iter().any(|(n, _)| *n == name) {
                        out.push((name, is_dir));
                    }
                }
            }
            (!out.is_empty()).then_some(out)
        };
        let all = project_commands(&Files { read: &read, list: &list });
        let names: Vec<&str> = all.iter().map(|c| c.command.as_str()).collect();
        for want in [
            "pnpm run dev",
            "pnpm run test",
            "make all",
            "make build",
            "make test",
            "just serve",
            "just lint",
            "task deploy",
            "task clean",
            "deno task start",
            "cargo run",
            "cargo run --bin tool",
            "cargo run --bin extra",
            "cargo run --example demo",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        let bad = |n: &&str| {
            n.contains(".PHONY") || n.contains('%') || n.contains("CC") || n.contains("set") || *n == "task X"
        };
        assert!(!names.iter().any(bad), "{names:?}");
        assert!(!names.contains(&"just version"), "{names:?}");
        let get = |c: &str| all.iter().find(|x| x.command == c).unwrap().detail.clone();
        assert_eq!(get("make all"), "Everything");
        assert_eq!(get("make build"), "go build ./...");
        assert_eq!(get("just serve"), "Serve the docs (port=\"8000\")");
        assert_eq!(get("task deploy"), "Ship it");
        assert_eq!(get("pnpm run test"), "vitest run");
    }
}
