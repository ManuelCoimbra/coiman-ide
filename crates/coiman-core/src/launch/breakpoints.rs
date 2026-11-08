//! Breakpoints beyond source lines, and attaching to a running process.
//!
//! - Exception breakpoints: the filters an adapter offers (`exceptionBreakpointFilters`).
//! - Function breakpoints: break when a function by name is entered.
//! - Data breakpoints: break when a variable is written (or read).
//! - Export and import of every breakpoint as a JSON file.
//! - The processes running here, for attaching a debugger to one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::dap::DebugKind;

// ----- what an adapter supports ------------------------------------------------------------

/// An exception filter an adapter offers, such as "Raised exceptions" or "C++ throw".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExceptionFilter {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    /// On unless the user turns it off.
    pub default: bool,
}

/// The breakpoint kinds an adapter takes, from its `initialize` answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Support {
    pub functions: bool,
    pub data: bool,
    pub exceptions: Vec<ExceptionFilter>,
}

impl Support {
    pub fn parse(capabilities: &Value) -> Self {
        let on = |name: &str| capabilities[name].as_bool().unwrap_or(false);
        let exceptions = capabilities["exceptionBreakpointFilters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let id = f["filter"].as_str()?.to_owned();
                Some(ExceptionFilter {
                    label: f["label"].as_str().map(str::to_owned).unwrap_or_else(|| id.clone()),
                    description: f["description"].as_str().map(str::to_owned),
                    default: f["default"].as_bool().unwrap_or(false),
                    id,
                })
            })
            .collect();
        Self { functions: on("supportsFunctionBreakpoints"), data: on("supportsDataBreakpoints"), exceptions }
    }
}

// ----- requests ----------------------------------------------------------------------------

/// `setExceptionBreakpoints`: the filters that are on.
pub fn exception_args(on: &[String]) -> Value {
    json!({ "filters": on })
}

/// A function breakpoint as the user wrote it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionBreakpoint {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

/// `setFunctionBreakpoints`: every enabled one.
pub fn function_args(list: &[FunctionBreakpoint]) -> Value {
    let list: Vec<Value> = list
        .iter()
        .filter(|f| f.enabled)
        .map(|f| {
            let mut v = json!({ "name": f.name });
            if let Some(c) = &f.condition {
                v["condition"] = json!(c);
            }
            v
        })
        .collect();
    json!({ "breakpoints": list })
}

/// Whether each breakpoint sent was placed, in the order sent: `None` while
/// it waits for code to load (gdb says `pending` before the program starts).
pub fn verified(body: &Value) -> Vec<Option<bool>> {
    body["breakpoints"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| match b["verified"].as_bool().unwrap_or(false) {
                    false if b["reason"].as_str() == Some("pending") => None,
                    placed => Some(placed),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `dataBreakpointInfo`: can variable `name` of container `reference` be watched?
pub fn data_info_args(reference: i64, name: &str) -> Value {
    json!({ "variablesReference": reference, "name": name })
}

/// What the adapter said about watching a variable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataInfo {
    /// It can: the id to set it with, how to describe it, and the access kinds offered.
    Watchable { id: String, description: String, access: Vec<String> },
    /// It can't, and why.
    No(String),
}

pub fn parse_data_info(body: &Value) -> DataInfo {
    let description = body["description"].as_str().unwrap_or("").to_owned();
    match body["dataId"].as_str() {
        Some(id) => DataInfo::Watchable {
            id: id.to_owned(),
            description,
            access: body["accessTypes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a.as_str().map(str::to_owned))
                .collect(),
        },
        None if description.is_empty() => DataInfo::No("this variable can't be watched".to_owned()),
        None => DataInfo::No(description),
    }
}

/// A data breakpoint set in this session (its id means nothing to the next one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataBreakpoint {
    pub id: String,
    pub description: String,
    /// `write`, `read` or `readWrite`.
    pub access: String,
    pub enabled: bool,
    pub verified: Option<bool>,
}

/// The access to ask for: writes when offered (what "break when it changes" means),
/// else what the adapter has.
pub fn pick_access(offered: &[String]) -> String {
    if offered.is_empty() || offered.iter().any(|a| a == "write") { "write".to_owned() } else { offered[0].clone() }
}

/// `setDataBreakpoints`: every enabled one.
pub fn data_args(list: &[DataBreakpoint]) -> Value {
    let list: Vec<Value> =
        list.iter().filter(|d| d.enabled).map(|d| json!({ "dataId": d.id, "accessType": d.access })).collect();
    json!({ "breakpoints": list })
}

// ----- export and import -------------------------------------------------------------------

/// A source line breakpoint in an exported file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLine {
    /// Relative to the project with `/`, or absolute when outside it.
    pub path: String,
    /// 1-based, as editors show it.
    pub line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// Every breakpoint, as written to and read from a file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    #[serde(default)]
    pub lines: Vec<SavedLine>,
    #[serde(default)]
    pub functions: Vec<FunctionBreakpoint>,
    /// Exception filters turned on or off, by id; the rest keep the adapter's default.
    #[serde(default)]
    pub exceptions: Vec<(String, bool)>,
}

/// The file name exports go to by default, in the project's `.coiman` folder.
pub const FILE_NAME: &str = "breakpoints.json";

/// `path` as written to an export: relative to `root` with `/` when inside it.
pub fn portable_path(path: &Path, root: Option<&Path>) -> String {
    let relative = root.and_then(|r| path.strip_prefix(r).ok());
    match relative {
        Some(r) => r.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/"),
        None => path.to_string_lossy().into_owned(),
    }
}

/// An exported path, as a path here.
pub fn local_path(path: &str, root: Option<&Path>) -> PathBuf {
    let p = Path::new(path);
    let absolute = p.is_absolute() || path.starts_with('/') || path.get(1..2) == Some(":");
    match root {
        Some(r) if !absolute => path.split('/').filter(|s| !s.is_empty()).fold(r.to_path_buf(), |acc, s| acc.join(s)),
        _ => p.to_path_buf(),
    }
}

impl Saved {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.functions.is_empty() && self.exceptions.is_empty()
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(&json!({
            "coimanBreakpoints": 1,
            "lines": self.lines,
            "functions": self.functions,
            "exceptions": self.exceptions.iter().map(|(id, on)| json!({ "filter": id, "enabled": on })).collect::<Vec<_>>(),
        }))
        .unwrap_or_default();
        text.push('\n');
        text
    }

    /// Reads an export. VS Code-style lists (`{"breakpoints": [...]}`) with
    /// `path`/`line` entries are read too.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| format!("not a breakpoints file: {e}"))?;
        if !v.is_object() {
            return Err("not a breakpoints file: expected an object".to_owned());
        }
        let lines_value = if v["lines"].is_array() { &v["lines"] } else { &v["breakpoints"] };
        let mut lines = Vec::new();
        for item in lines_value.as_array().into_iter().flatten() {
            let line: SavedLine =
                serde_json::from_value(item.clone()).map_err(|e| format!("a line breakpoint: {e}"))?;
            if line.line == 0 {
                return Err(format!("{}: lines start at 1", line.path));
            }
            lines.push(line);
        }
        let functions = match v.get("functions") {
            Some(f) => serde_json::from_value(f.clone()).map_err(|e| format!("a function breakpoint: {e}"))?,
            None => Vec::new(),
        };
        let exceptions = v["exceptions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| Some((e["filter"].as_str()?.to_owned(), e["enabled"].as_bool().unwrap_or(true))))
            .collect();
        if lines.is_empty() && functions.is_empty() && v.get("coimanBreakpoints").is_none() {
            return Err("no breakpoints in this file".to_owned());
        }
        Ok(Self { lines, functions, exceptions })
    }
}

// ----- attaching ---------------------------------------------------------------------------

/// A process running here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub name: String,
}

/// `tasklist /fo csv /nh`: `"name.exe","1234","Console","1","12,345 K"`.
pub fn parse_tasklist(text: &str) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.trim().trim_matches('"').split("\",\"").collect();
            let pid = fields.get(1)?.trim().parse().ok()?;
            Some(Process { pid, name: fields[0].to_owned() })
        })
        .collect()
}

/// `ps -eo pid=,comm=`: `  123 bash`.
pub fn parse_ps(text: &str) -> Vec<Process> {
    text.lines()
        .filter_map(|line| {
            let (pid, name) = line.trim().split_once(char::is_whitespace)?;
            Some(Process { pid: pid.parse().ok()?, name: name.trim().to_owned() })
        })
        .collect()
}

/// The processes running here, newest first, without this one and the System Idle Process.
pub fn processes() -> Result<Vec<Process>, String> {
    type Parse = fn(&str) -> Vec<Process>;
    let (program, args, parse): (&str, &[&str], Parse) = if cfg!(windows) {
        ("tasklist", &["/fo", "csv", "/nh"], parse_tasklist)
    } else {
        ("ps", &["-eo", "pid=,comm="], parse_ps)
    };
    let out = crate::language::lsp::discovery::quiet_command(Path::new(program))
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let me = std::process::id();
    let mut list: Vec<Process> =
        parse(&String::from_utf8_lossy(&out.stdout)).into_iter().filter(|p| p.pid != 0 && p.pid != me).collect();
    list.reverse();
    Ok(list)
}

/// The debugger that suits a process, from its name: debugpy for Python,
/// js-debug for Node, else a native debugger (gdb when there is one, CodeLLDB otherwise).
pub fn suggested_kinds(name: &str) -> Vec<&'static str> {
    let stem = name.to_lowercase();
    let stem = stem.strip_suffix(".exe").unwrap_or(&stem);
    if stem.starts_with("python") || stem == "py" || stem == "pythonw" {
        vec!["debugpy", "gdb", "CodeLLDB"]
    } else if stem == "node" {
        vec!["js-debug", "gdb", "CodeLLDB"]
    } else {
        vec!["gdb", "CodeLLDB", "Delve"]
    }
}

impl DebugKind {
    /// The `attach` request for process `pid`.
    pub fn attach_args(&self, pid: u32, step_into_std: bool) -> Value {
        match self {
            Self::Lldb => json!({
                "type": "lldb",
                "request": "attach",
                "name": format!("Attach to {pid}"),
                "pid": pid,
                "sourceLanguages": ["rust"],
                "_adapterSettings": { "consoleMode": "evaluate" },
            }),
            Self::Python { .. } => json!({
                "type": "python",
                "request": "attach",
                "name": format!("Attach to {pid}"),
                "processId": pid,
                "justMyCode": !step_into_std,
            }),
            Self::Go => json!({
                "type": "go",
                "request": "attach",
                "name": format!("Attach to {pid}"),
                "mode": "local",
                "processId": pid,
            }),
            Self::Js => json!({
                "type": "pwa-node",
                "request": "attach",
                "name": format!("Attach to {pid}"),
                "processId": pid.to_string(),
            }),
            Self::Gdb => json!({ "request": "attach", "name": format!("Attach to {pid}"), "pid": pid }),
            Self::Data { debug_type, .. } => json!({
                "type": debug_type,
                "request": "attach",
                "name": format!("Attach to {pid}"),
                "pid": pid,
                "processId": pid,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_what_an_adapter_supports() {
        let caps = json!({
            "supportsFunctionBreakpoints": true,
            "supportsDataBreakpoints": true,
            "exceptionBreakpointFilters": [
                {"filter": "raised", "label": "Raised Exceptions", "default": false, "description": "Every raise"},
                {"filter": "uncaught", "label": "Uncaught Exceptions", "default": true},
                {"label": "no id"}
            ]
        });
        let s = Support::parse(&caps);
        assert!(s.functions && s.data);
        assert_eq!(s.exceptions.len(), 2);
        assert_eq!(s.exceptions[0].description.as_deref(), Some("Every raise"));
        assert!(!s.exceptions[0].default && s.exceptions[1].default);
        assert_eq!(Support::parse(&json!({})), Support::default());
    }

    #[test]
    fn builds_requests() {
        assert_eq!(exception_args(&["throw".into()]), json!({"filters": ["throw"]}));
        let f = vec![
            FunctionBreakpoint { name: "main".into(), condition: None, enabled: true },
            FunctionBreakpoint { name: "add".into(), condition: Some("a > 2".into()), enabled: true },
            FunctionBreakpoint { name: "off".into(), condition: None, enabled: false },
        ];
        assert_eq!(
            function_args(&f),
            json!({"breakpoints": [{"name": "main"}, {"name": "add", "condition": "a > 2"}]})
        );
        let d = vec![
            DataBreakpoint {
                id: "x".into(),
                description: "x".into(),
                access: "write".into(),
                enabled: true,
                verified: None,
            },
            DataBreakpoint {
                id: "y".into(),
                description: "y".into(),
                access: "read".into(),
                enabled: false,
                verified: None,
            },
        ];
        assert_eq!(data_args(&d), json!({"breakpoints": [{"dataId": "x", "accessType": "write"}]}));
        assert_eq!(
            verified(&json!({"breakpoints": [{"verified": true}, {}, {"verified": false, "reason": "pending"}]})),
            vec![Some(true), Some(false), None]
        );
        assert_eq!(data_info_args(7, "n"), json!({"variablesReference": 7, "name": "n"}));
    }

    #[test]
    fn reads_data_breakpoint_info() {
        let ok = json!({"dataId": "0x1000,4", "description": "n (4 bytes)", "accessTypes": ["read", "write"]});
        assert_eq!(
            parse_data_info(&ok),
            DataInfo::Watchable {
                id: "0x1000,4".into(),
                description: "n (4 bytes)".into(),
                access: vec!["read".into(), "write".into()]
            }
        );
        assert_eq!(
            parse_data_info(&json!({"dataId": null, "description": "a register"})),
            DataInfo::No("a register".into())
        );
        assert!(matches!(parse_data_info(&json!({})), DataInfo::No(_)));
        assert_eq!(pick_access(&["read".into(), "write".into()]), "write");
        assert_eq!(pick_access(&["readWrite".into()]), "readWrite");
        assert_eq!(pick_access(&[]), "write");
    }

    #[test]
    fn exports_and_imports() {
        let saved = Saved {
            lines: vec![
                SavedLine {
                    path: "src/main.c".into(),
                    line: 9,
                    condition: Some("i == 3".into()),
                    log: None,
                    enabled: true,
                },
                SavedLine {
                    path: "lib.c".into(),
                    line: 1,
                    condition: None,
                    log: Some("x is {x}".into()),
                    enabled: false,
                },
            ],
            functions: vec![FunctionBreakpoint { name: "main".into(), condition: None, enabled: true }],
            exceptions: vec![("throw".into(), true), ("catch".into(), false)],
        };
        let text = saved.to_json();
        assert!(text.contains("\"coimanBreakpoints\": 1"));
        assert_eq!(Saved::from_json(&text).unwrap(), saved);
        // An empty export is still an export.
        assert_eq!(Saved::from_json(&Saved::default().to_json()).unwrap(), Saved::default());
    }

    #[test]
    fn keeps_in_the_session_file() {
        let saved = Saved {
            lines: vec![SavedLine { path: "a.c".into(), line: 2, condition: None, log: None, enabled: true }],
            functions: Vec::new(),
            exceptions: vec![("throw".into(), true)],
        };
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
        assert!(!saved.is_empty() && Saved::default().is_empty());
        // Older session files have none.
        assert_eq!(serde_json::from_str::<Saved>("{}").unwrap(), Saved::default());
    }

    #[test]
    fn refuses_what_isnt_breakpoints() {
        assert!(Saved::from_json("not json").is_err());
        assert!(Saved::from_json("[1, 2]").is_err());
        assert!(Saved::from_json("{}").is_err());
        assert!(Saved::from_json(r#"{"lines": [{"path": "a.c", "line": 0}]}"#).unwrap_err().contains("start at 1"));
        assert!(Saved::from_json(r#"{"lines": [{"line": 3}]}"#).is_err());
        // VS Code-like lists are read.
        let vs = Saved::from_json(r#"{"breakpoints": [{"path": "a.c", "line": 3}]}"#).unwrap();
        assert_eq!(vs.lines[0].line, 3);
        assert!(vs.lines[0].enabled);
    }

    #[test]
    fn paths_travel_relative_to_the_project() {
        let root = Path::new(if cfg!(windows) { r"C:\proj" } else { "/proj" });
        let inside = root.join("src").join("main.c");
        assert_eq!(portable_path(&inside, Some(root)), "src/main.c");
        assert_eq!(local_path("src/main.c", Some(root)), inside);
        let outside = if cfg!(windows) { r"D:\other\x.c" } else { "/other/x.c" };
        assert_eq!(portable_path(Path::new(outside), Some(root)), outside);
        assert_eq!(local_path(outside, Some(root)), PathBuf::from(outside));
        assert_eq!(local_path("a.c", None), PathBuf::from("a.c"));
    }

    #[test]
    fn reads_process_lists() {
        let tasklist = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\"lab.exe\",\"4242\",\"Console\",\"1\",\"3,104 K\"\r\nbad line\r\n";
        assert_eq!(
            parse_tasklist(tasklist),
            vec![Process { pid: 0, name: "System Idle Process".into() }, Process { pid: 4242, name: "lab.exe".into() }]
        );
        let ps = "    1 init\n  812 python3\nnope\n";
        assert_eq!(
            parse_ps(ps),
            vec![Process { pid: 1, name: "init".into() }, Process { pid: 812, name: "python3".into() }]
        );
    }

    #[test]
    fn lists_processes_here() {
        let list = processes().expect("the process list");
        assert!(!list.is_empty());
        assert!(list.iter().all(|p| p.pid != std::process::id() && p.pid != 0));
    }

    #[test]
    fn suggests_a_debugger_by_name() {
        assert_eq!(suggested_kinds("python.exe")[0], "debugpy");
        assert_eq!(suggested_kinds("node.exe")[0], "js-debug");
        assert_eq!(suggested_kinds("lab.exe")[0], "gdb");
    }

    #[test]
    fn attach_requests() {
        assert_eq!(DebugKind::Gdb.attach_args(12, false)["pid"], 12);
        assert_eq!(DebugKind::Lldb.attach_args(12, false)["request"], "attach");
        let py = DebugKind::Python { python: "python".into(), env: Vec::new() }.attach_args(12, false);
        assert_eq!((py["processId"].as_u64(), py["justMyCode"].as_bool()), (Some(12), Some(true)));
        assert_eq!(DebugKind::Go.attach_args(12, false)["mode"], "local");
        assert_eq!(DebugKind::Js.attach_args(12, false)["processId"], "12");
    }
}
