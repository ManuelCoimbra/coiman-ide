//! Building and debugging C and C++ the way course projects do: a makefile that names the
//! program, MinGW on Windows (`mingw32-make`, often not on PATH), and VS Code's
//! `.vscode/launch.json` and `tasks.json` (a `cppdbg` launch whose `preLaunchTask` runs
//! `make debug`), written for Linux paths such as `/usr/bin/make`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::lsp::discovery::quiet_command;
use crate::lsp::servers::which;

/// Where MinGW usually is on Windows when it isn't on PATH: lab computers and the
/// installs course guides describe.
pub fn mingw_dirs() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let mut dirs: Vec<PathBuf> =
        [r"C:\MinGW\bin", r"C:\mingw32\bin", r"C:\mingw64\bin", r"D:\mingw32\bin", r"D:\mingw64\bin"]
            .iter()
            .map(PathBuf::from)
            .collect();
    for base in [r"C:\Program Files", r"C:\Program Files (x86)"] {
        for sub in [r"mingw-w64\mingw64\bin", r"MinGW\bin", r"mingw32\bin", r"mingw64\bin"] {
            dirs.push(Path::new(base).join(sub));
        }
    }
    dirs.extend([r"C:\msys64\ucrt64\bin", r"C:\msys64\mingw64\bin"].iter().map(PathBuf::from));
    dirs
}

/// A tool on PATH, else in a MinGW folder.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    which(name).or_else(|| {
        let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        mingw_dirs().into_iter().map(|d| d.join(&file)).find(|p| p.is_file())
    })
}

/// The make to run: `make`, else MinGW's `mingw32-make`.
pub fn make_program() -> Option<PathBuf> {
    find_tool("make").or_else(|| find_tool("mingw32-make"))
}

/// gdb, from PATH or a MinGW folder.
pub fn find_gdb() -> Option<PathBuf> {
    find_tool("gdb")
}

/// The makefile in `dir`, by any of the names make reads.
pub fn makefile_in(dir: &Path) -> Option<PathBuf> {
    ["GNUmakefile", "makefile", "Makefile"].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// The value of variable `name` in a makefile: its first plain assignment.
fn make_variable(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let (key, value) = line.split_once('=')?;
        let key = key.trim_end_matches([':', '?', '+', ' ', '\t']).trim();
        (key == name).then(|| value.trim().to_owned())
    })
}

/// Expands `$(NAME)` and `${NAME}` from the makefile's variables, a few levels deep.
fn expand(text: &str, value: &str) -> String {
    let mut out = value.to_owned();
    for _ in 0..4 {
        let Some(start) = out.find("$(").or_else(|| out.find("${")) else { break };
        let close = if out[start + 1..].starts_with('(') { ')' } else { '}' };
        let Some(len) = out[start + 2..].find(close) else { break };
        let name = out[start + 2..start + 2 + len].to_owned();
        let with = make_variable(text, &name).unwrap_or_default();
        out.replace_range(start..start + 3 + len, &with);
    }
    out
}

/// The program a makefile's `target` (else its first target) builds: what follows `-o`
/// in its recipe.
pub fn makefile_program(text: &str, target: Option<&str>) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let is_target = |line: &str| {
        !line.starts_with(['\t', ' ', '#', '.'])
            && line.contains(':')
            && !line.contains(":=")
            && !line.contains('=')
            && !line.trim_start().starts_with("ifeq")
    };
    let start = lines.iter().position(|l| {
        is_target(l) && target.is_none_or(|t| l.split(':').next().is_some_and(|n| n.split_whitespace().any(|w| w == t)))
    })?;
    lines[start + 1..].iter().take_while(|l| l.starts_with('\t') || l.trim().is_empty()).find_map(|recipe| {
        let words: Vec<&str> = recipe.split_whitespace().collect();
        let at = words.iter().position(|w| *w == "-o")?;
        let name = expand(text, words.get(at + 1)?);
        (!name.is_empty()).then_some(name)
    })
}

/// The built program's path: Windows compilers add `.exe` to a name without one.
pub fn program_path(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    if cfg!(windows) && path.extension().is_none() {
        let exe = path.with_extension("exe");
        if exe.exists() || !path.exists() {
            return exe;
        }
    }
    path
}

/// JSON as VS Code writes it: `//` and `/* */` comments, and trailing commas.
pub fn parse_jsonc(text: &str) -> Option<Value> {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' {
            // A comma before a closing bracket is dropped.
            let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
            if !matches!(next, Some('}' | ']')) {
                out.push(c);
            }
        } else {
            out.push(c);
        }
        i += 1;
    }
    serde_json::from_str(&out).ok()
}

/// A command to run before debugging: a VS Code task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskCommand {
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// What `.vscode/launch.json` says to debug, for C or C++ under gdb.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VsCodeLaunch {
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub before: Option<TaskCommand>,
}

fn substitute(value: &str, root: &Path) -> String {
    let folder = root.display().to_string();
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    value
        .replace("${workspaceFolder}", &folder)
        .replace("${workspaceRoot}", &folder)
        .replace("${workspaceFolderBasename}", &name)
}

/// The first `cppdbg` (or `gdb`) launch in `root/.vscode/launch.json`, with its
/// `preLaunchTask` from `tasks.json`.
pub fn vscode_launch(root: &Path) -> Option<VsCodeLaunch> {
    let launch = parse_jsonc(&std::fs::read_to_string(root.join(".vscode").join("launch.json")).ok()?)?;
    let config = launch["configurations"].as_array()?.iter().find(|c| {
        matches!(c["type"].as_str(), Some("cppdbg" | "gdb" | "cppvsdbg"))
            && c["request"].as_str().unwrap_or("launch") == "launch"
            && c["program"].is_string()
    })?;
    let text = |v: &Value| v.as_str().map(|s| substitute(s, root));
    let program = PathBuf::from(text(&config["program"])?);
    let cwd = text(&config["cwd"]).map(PathBuf::from).unwrap_or_else(|| root.to_path_buf());
    let args = config["args"].as_array().map(|a| a.iter().filter_map(text).collect()).unwrap_or_default();
    let before = text(&config["preLaunchTask"]).and_then(|label| {
        let tasks = parse_jsonc(&std::fs::read_to_string(root.join(".vscode").join("tasks.json")).ok()?)?;
        let task = tasks["tasks"].as_array()?.iter().find(|t| t["label"].as_str() == Some(label.as_str()))?;
        Some(TaskCommand {
            label: label.clone(),
            command: text(&task["command"])?,
            args: task["args"].as_array().map(|a| a.iter().filter_map(text).collect()).unwrap_or_default(),
            cwd: text(&task["options"]["cwd"]).map(PathBuf::from).unwrap_or_else(|| root.to_path_buf()),
        })
    });
    Some(VsCodeLaunch { name: config["name"].as_str().unwrap_or("Debug").to_owned(), program, args, cwd, before })
}

/// A task's program as this computer has it: `/usr/bin/make` from a Linux setup becomes
/// the make found here, and likewise for any tool named by a path that isn't there.
pub fn local_program(command: &str) -> Option<PathBuf> {
    let path = Path::new(command);
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    let name = path.file_stem()?.to_string_lossy().into_owned();
    match name.as_str() {
        "make" | "mingw32-make" => make_program(),
        other => find_tool(other),
    }
}

/// Runs a command to its end, for its output. Fails with what it printed.
fn run_to_end(program: &Path, args: &[String], cwd: &Path) -> Result<String, String> {
    let out = quiet_command(program)
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not run {}: {e}", program.display()))?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if out.status.success() { Ok(text) } else { Err(text.trim().to_owned()) }
}

/// What to debug for a C or C++ file: built first, with debug information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugBuild {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// How it was built, for the debug console.
    pub how: String,
}

/// Builds `file`'s program for debugging: as `.vscode/launch.json` says when it has a gdb
/// launch, else with the makefile's `debug` target, else with `gcc -g` (or `g++`).
pub fn debug_build(file: &Path, root: Option<&Path>) -> Result<DebugBuild, String> {
    let dir = file.parent().unwrap_or(file).to_path_buf();
    let root = root.map(Path::to_path_buf).unwrap_or_else(|| dir.clone());
    if let Some(launch) = vscode_launch(&root) {
        let mut how = format!("{} (.vscode/launch.json)", launch.name);
        if let Some(task) = &launch.before {
            let program = local_program(&task.command)
                .ok_or_else(|| format!("The task \"{}\" runs {}, which isn't here", task.label, task.command))?;
            let output = run_to_end(&program, &task.args, &task.cwd)?;
            how = format!(
                "{how}: ran \"{}\" ({} {})\n{}",
                task.label,
                program.display(),
                task.args.join(" "),
                output.trim()
            );
        }
        let program = program_path(
            launch.program.parent().unwrap_or(&root),
            &launch.program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        );
        if !program.exists() {
            return Err(format!("{} wasn't built: launch.json names it as the program", program.display()));
        }
        return Ok(DebugBuild { program, args: launch.args, cwd: launch.cwd, how });
    }
    if let Some(makefile) = makefile_in(&dir).or_else(|| makefile_in(&root)) {
        let text = std::fs::read_to_string(&makefile).unwrap_or_default();
        let make_dir = makefile.parent().unwrap_or(&dir).to_path_buf();
        let has_debug = text.lines().any(|l| l.starts_with("debug:"));
        let name = makefile_program(&text, has_debug.then_some("debug")).or_else(|| makefile_program(&text, None));
        if let Some(name) = name {
            let make = make_program().ok_or("This makefile needs make (or MinGW's mingw32-make), not found")?;
            let target = if has_debug { vec!["debug".to_owned()] } else { Vec::new() };
            let output = run_to_end(&make, &target, &make_dir)?;
            let program = program_path(&make_dir, &name);
            return Ok(DebugBuild {
                program,
                args: Vec::new(),
                cwd: make_dir,
                how: format!("{} {}\n{}", make.display(), target.join(" "), output.trim()),
            });
        }
    }
    let cpp = file.extension().is_some_and(|e| e != "c");
    let compiler = find_tool(if cpp { "g++" } else { "gcc" }).ok_or(if cpp {
        "Debugging C++ needs g++ (MinGW), not found"
    } else {
        "Debugging C needs gcc (MinGW), not found"
    })?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "prog".to_owned());
    let out_dir = std::env::temp_dir().join("rust-code-run").join(format!("debug-{stem}"));
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let program = program_path(&out_dir, &stem);
    let args: Vec<String> = ["-g", "-O0", "-Wall", "-o"]
        .iter()
        .map(|s| (*s).to_owned())
        .chain([program.display().to_string(), file.display().to_string()])
        .collect();
    let output = run_to_end(&compiler, &args, &dir)?;
    Ok(DebugBuild {
        program,
        args: Vec::new(),
        cwd: dir,
        how: format!("{} {}\n{}", compiler.display(), args.join(" "), output.trim()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB: &str = "# Select environment\nENV ?= linux\n\nFLAGS = -Wall -Wextra\n\nifeq ($(ENV),linux)\n    CC = gcc\n    EXECUTABLE = prog\nendif\n\nifeq ($(ENV),windows)\n    EXECUTABLE = prog.exe\nendif\n\ndefault:\n\t$(CC) $(FLAGS) -o $(EXECUTABLE) main.c input.c\n\ndebug:\n\t$(CC) $(FLAGS) -g -o $(EXECUTABLE) main.c input.c\n\nclean:\n\trm -f $(EXECUTABLE)\n";

    #[test]
    fn programs_named_by_makefiles() {
        assert_eq!(makefile_program(LAB, None).as_deref(), Some("prog"));
        assert_eq!(makefile_program(LAB, Some("debug")).as_deref(), Some("prog"));
        let plain = "default:\n\tgcc -Wall -o prog main.c\n\ndebug:\n\tgcc -Wall -o prog -g main.c\n";
        assert_eq!(makefile_program(plain, None).as_deref(), Some("prog"));
        assert_eq!(makefile_program("all:\n\techo hi\n", None), None);
        let dir = Path::new("p");
        if cfg!(windows) {
            assert_eq!(program_path(dir, "prog"), dir.join("prog.exe"));
            assert_eq!(program_path(dir, "prog.exe"), dir.join("prog.exe"));
        }
    }

    #[test]
    fn vscode_files_with_comments_and_linux_paths() {
        let root = std::env::temp_dir().join(format!("rust-code-c-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".vscode")).unwrap();
        let launch = r#"{
    "version": "0.2.0",
    "configurations": [
        {
            "name": "gdb - Debug project",
            "type": "cppdbg",
            "request": "launch",
            "program": "${workspaceFolder}/prog",
            "args": [],
            "cwd": "${workspaceFolder}",
            /* This calls the debug task in "tasks.json" prior to debugging */
            "preLaunchTask": "C/C++: call make debug", // trailing comment
            "miDebuggerPath": "/usr/bin/gdb",
        }
    ]
}"#;
        let tasks = r#"{ "tasks": [ { "type": "process", "label": "C/C++: call make debug", "command": "/usr/bin/make",
            "args": [ "debug" ], "options": { "cwd": "${workspaceFolder}" } } ], "version": "2.0.0" }"#;
        std::fs::write(root.join(".vscode").join("launch.json"), launch).unwrap();
        std::fs::write(root.join(".vscode").join("tasks.json"), tasks).unwrap();
        let found = vscode_launch(&root).unwrap();
        assert_eq!(found.name, "gdb - Debug project");
        assert_eq!(found.program, PathBuf::from(format!("{}/prog", root.display())));
        assert_eq!(found.cwd, root);
        let task = found.before.unwrap();
        assert_eq!((task.command.as_str(), task.args.as_slice()), ("/usr/bin/make", &["debug".to_owned()][..]));
        // /usr/bin/make isn't here: the make found here stands in, if there is one.
        assert_eq!(local_program("/usr/bin/make"), make_program());
        assert!(
            parse_jsonc(r#"{"a": "// not a comment", "b": [1, 2,],}"#).is_some_and(|v| v["a"] == "// not a comment")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
