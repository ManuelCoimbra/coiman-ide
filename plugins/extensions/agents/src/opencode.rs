//! What the extension knows of `opencode`: the lines `opencode run --format json` prints, the
//! sessions it lists and exports, its models, and the arguments of a run. No host calls here,
//! so all of it is tested natively.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub title: String,
    pub input: String,
    pub output: String,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Thinking(String),
    Tool(Tool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub user: bool,
    pub parts: Vec<Part>,
}

impl Message {
    pub fn user(text: &str) -> Self {
        Self { user: true, parts: vec![Part::Text(text.to_owned())] }
    }

    fn agent(part: Part) -> Self {
        Self { user: false, parts: vec![part] }
    }

    /// The message's text, as it was sent.
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub updated: u64,
}

/// What one line of a run says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Text(String),
    Thinking(String),
    Tool(Tool),
    /// A line opencode wrote to stderr (a permission it turned down, an error): on a line of
    /// its own.
    Note(String),
    /// The id opencode gave the session the run is in.
    SessionId(String),
}

/// How the ids the extension makes for new sessions start (opencode wants "ses").
pub const NEW_SESSION: &str = "ses_coiman_";

/// Manual for an older opencode: edits, commands and fetches need asking. `opencode run` has
/// no one to ask, so an older client turns those down rather than doing them unasked.
pub const LEGACY_MANUAL_PERMISSIONS: &str = r#"{"permission":{"edit":"ask","bash":"ask","webfetch":"ask"}}"#;

/// The choices a run is made with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefs {
    /// None: opencode chooses.
    pub model: Option<String>,
    /// "build" edits files; "plan" only discusses.
    pub mode: String,
    /// "manual" asks first (turns down what needs asking); "auto" approves tool use.
    pub permissions: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { model: None, mode: "build".to_owned(), permissions: "manual".to_owned() }
    }
}

impl Prefs {
    pub fn from_json(text: &str) -> Self {
        let mut prefs = Self::default();
        let Ok(value) = serde_json::from_str::<Value>(text) else { return prefs };
        if let Some(model) = value.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()) {
            prefs.model = Some(model.to_owned());
        }
        if let Some(mode) = value.get("mode").and_then(Value::as_str).filter(|m| *m == "plan" || *m == "build") {
            prefs.mode = mode.to_owned();
        }
        if let Some(p) = value.get("permissions").and_then(Value::as_str).filter(|p| *p == "auto" || *p == "manual") {
            prefs.permissions = p.to_owned();
        }
        prefs
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "model": self.model, "mode": self.mode, "permissions": self.permissions
        }))
        .unwrap_or_default()
    }
}

/// A run of one prompt: its arguments, what goes to its stdin, and its environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub args: Vec<String>,
    pub stdin: String,
    pub env: Vec<(String, String)>,
}

/// `args` after `--standalone` where this opencode has it (newer ones, which otherwise use a
/// background service that keeps the folder it started in).
pub fn with_standalone(standalone: bool, args: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 1);
    if standalone {
        out.push("--standalone".to_owned());
    }
    out.extend(args.iter().map(|a| (*a).to_owned()));
    out
}

/// Whether `opencode run --help` names `--standalone`.
pub fn has_standalone(help: &str) -> bool {
    help.contains("--standalone")
}

/// The `opencode run` for one prompt.
pub fn run(standalone: bool, session: &str, text: &str, attachments: &[String], prefs: &Prefs) -> Run {
    let mut args = with_standalone(standalone, &["run", "--thinking", "--format", "json"]);
    // An older opencode won't take a session id it didn't make: a new chat starts without
    // one, and the id comes with the first event.
    if standalone || !session.starts_with(NEW_SESSION) {
        args.extend(["--session".to_owned(), session.to_owned()]);
    }
    let mut env = Vec::new();
    if prefs.permissions != "manual" {
        args.push("--auto".to_owned());
    } else if !standalone {
        // An older opencode edits and runs commands without asking unless its config says
        // "ask"; with no one to ask in `run`, it then turns them down.
        env.push(("OPENCODE_CONFIG_CONTENT".to_owned(), LEGACY_MANUAL_PERMISSIONS.to_owned()));
    }
    // opencode keeps a session's first agent unless told otherwise: the choice goes every
    // time, so going back to Agent really leaves Plan.
    let agent = if prefs.mode == "plan" { "plan" } else { "build" };
    args.extend(["--agent".to_owned(), agent.to_owned()]);
    if let Some(model) = &prefs.model {
        args.extend(["--model".to_owned(), model.clone()]);
    }
    for file in attachments {
        args.extend(["--file".to_owned(), file.clone()]);
    }
    // A newer opencode reads the prompt from stdin, exactly as typed (as one argument it would
    // be quoted, and stored twice). An older one has no stdin prompt: its words go as they are.
    let stdin = if standalone {
        text.to_owned()
    } else {
        args.extend(text.split_whitespace().map(str::to_owned));
        String::new()
    };
    Run { args, stdin, env }
}

/// The prompt as shown: attachments as `@path` first.
pub fn with_attachments(text: &str, attachments: &[String]) -> String {
    if attachments.is_empty() {
        return text.to_owned();
    }
    let refs = attachments.iter().map(|p| format!("@{p}")).collect::<Vec<_>>().join(" ");
    format!("{refs}\n{text}")
}

/// `opencode models`: one per line.
pub fn models(stdout: &str) -> Vec<String> {
    stdout.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("Error")).map(str::to_owned).collect()
}

/// `opencode session list --format json`, newest first.
pub fn sessions(stdout: &str) -> Vec<Session> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(stdout) else { return Vec::new() };
    let mut out: Vec<Session> = items
        .iter()
        .filter_map(|item| {
            Some(Session {
                id: item.get("id")?.as_str()?.to_owned(),
                title: item.get("title").and_then(Value::as_str).unwrap_or("Untitled").to_owned(),
                updated: item.get("updated").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s.updated));
    out
}

/// Adds what a line said to the transcript; returns the index of the first message changed.
pub fn apply(messages: &mut Vec<Message>, event: Event) -> Option<usize> {
    let last = messages.len().saturating_sub(1);
    let agent_last = messages.last().is_some_and(|m| !m.user);
    match event {
        Event::Text(text) => {
            // Errors are a part of their own, so their red shows.
            let is_error = text.starts_with("Error:");
            if agent_last
                && !is_error
                && let Some(Part::Text(existing)) = messages[last].parts.last_mut()
                && !existing.starts_with("Error:")
            {
                existing.push_str(&text);
                return Some(last);
            }
            push_part(messages, Part::Text(text))
        }
        Event::Thinking(text) => {
            if agent_last && let Some(Part::Thinking(existing)) = messages[last].parts.last_mut() {
                existing.push_str(&text);
                return Some(last);
            }
            push_part(messages, Part::Thinking(text))
        }
        Event::Tool(mut tool) => {
            if agent_last
                && let Some(Part::Tool(existing)) =
                    messages[last].parts.iter_mut().find(|p| matches!(p, Part::Tool(t) if t.id == tool.id))
            {
                // An end event may leave out the name and input its start gave.
                if tool.name.is_empty() || tool.name == "tool" {
                    tool.name = existing.name.clone();
                }
                if tool.title.is_empty() || tool.title == "tool" {
                    tool.title = existing.title.clone();
                }
                if tool.input.is_empty() {
                    tool.input = existing.input.clone();
                }
                *existing = tool;
                return Some(last);
            }
            push_part(messages, Part::Tool(tool))
        }
        Event::Note(text) => {
            if agent_last
                && let Some(Part::Text(existing)) = messages[last].parts.last_mut()
                && !existing.starts_with("Error:")
            {
                if !existing.ends_with('\n') {
                    existing.push('\n');
                }
                existing.push_str(&text);
                return Some(last);
            }
            push_part(messages, Part::Text(text))
        }
        Event::SessionId(_) => None,
    }
}

fn push_part(messages: &mut Vec<Message>, part: Part) -> Option<usize> {
    match messages.last_mut() {
        Some(m) if !m.user => m.parts.push(part),
        _ => messages.push(Message::agent(part)),
    }
    Some(messages.len() - 1)
}

fn json_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => {
            blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
        }
        other => json_text(other),
    }
}

fn error_message(error: Option<&Value>) -> String {
    error
        .and_then(|e| e.get("message").or_else(|| e.get("error")))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| error.map(json_text))
        .unwrap_or_else(|| "unknown error".to_owned())
}

fn tool_part(part: &Value) -> Option<Event> {
    let id = part.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
    let name = part
        .get("tool")
        .and_then(Value::as_str)
        .or_else(|| part.get("name").and_then(Value::as_str))
        .unwrap_or("tool")
        .to_owned();
    let state = part.get("state")?;
    let title = state.get("title").and_then(Value::as_str).unwrap_or(&name).to_owned();
    let input = state.get("input").map(json_text).unwrap_or_default();
    let output =
        state.get("output").map(json_text).or_else(|| state.get("content").map(content_text)).unwrap_or_default();
    let status = state.get("status").and_then(Value::as_str).unwrap_or("running").to_owned();
    Some(Event::Tool(Tool { id, name, title, input, output, status }))
}

/// A line of `opencode run --format json`. Lines that aren't JSON are what it wrote to stderr:
/// those come back as text, cleaned (`line` below).
pub fn event(line: &str) -> Option<Event> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        let text = stderr_line(line);
        return (!text.trim().is_empty()).then(|| Event::Note(format!("{text}\n")));
    };
    let kind = value.get("type").and_then(Value::as_str)?;
    let part = value.get("part");
    let part_type = part.and_then(|p| p.get("type")).and_then(Value::as_str);
    let text =
        part.and_then(|p| p.get("text")).and_then(Value::as_str).or_else(|| value.get("text").and_then(Value::as_str));
    let data = value.get("data");
    let delta =
        || data.and_then(|d| d.get("delta").or_else(|| d.get("text"))).and_then(Value::as_str).map(str::to_owned);
    match kind {
        // A newer opencode's stream: deltas come while the model writes.
        "session.text.delta" => delta().map(Event::Text),
        "session.reasoning.delta" => delta().map(Event::Thinking),
        "session.tool.called" => data.map(|data| {
            let name = data.get("name").and_then(Value::as_str).unwrap_or("tool").to_owned();
            Event::Tool(Tool {
                id: data.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                title: name.clone(),
                name,
                input: data.get("input").map(json_text).unwrap_or_default(),
                output: String::new(),
                status: "running".to_owned(),
            })
        }),
        "session.tool.success" | "session.tool.failed" => data.map(|data| {
            let failed = kind == "session.tool.failed";
            let name = data.get("name").and_then(Value::as_str).unwrap_or("tool").to_owned();
            let output = if failed {
                if data.get("error").is_some() { error_message(data.get("error")) } else { "tool failed".to_owned() }
            } else {
                data.get("content").map(content_text).unwrap_or_default()
            };
            Event::Tool(Tool {
                id: data.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                title: name.clone(),
                name,
                input: String::new(),
                output,
                status: (if failed { "error" } else { "completed" }).to_owned(),
            })
        }),
        "message.part.updated" if part_type == Some("tool") => tool_part(part?),
        "text" if part_type != Some("reasoning") => Some(Event::Text(text?.to_owned())),
        "reasoning" => Some(Event::Thinking(text?.to_owned())),
        "step_start" => Some(Event::SessionId(value.get("sessionID").and_then(Value::as_str)?.to_owned())),
        "tool_use" => tool_part(part?),
        // One clean line, not the raw JSON.
        "error" => Some(Event::Text(format!(
            "Error: {}",
            error_message(value.get("error").or_else(|| part.and_then(|p| p.get("error"))))
        ))),
        _ if part_type == Some("reasoning") => Some(Event::Thinking(text?.to_owned())),
        _ => None,
    }
}

/// Text without terminal colors and cursor codes.
pub fn strip_colors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                // Up to the final byte, @ to ~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                // Up to BEL or ESC \.
                chars.next();
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    out
}

/// A line opencode wrote to stderr, without its colors; a permission it turned down (Manual,
/// with nobody to ask) says so plainly.
pub fn stderr_line(line: &str) -> String {
    let line = strip_colors(line);
    let plain = line.trim_start_matches(['!', ' ']);
    match plain.strip_prefix("permission requested: ").and_then(|rest| rest.strip_suffix("; auto-rejecting")) {
        Some(what) => format!("Not allowed in Manual: {what}. Switch to Auto to let the agent do this."),
        None => line,
    }
}

/// An item of an older opencode's `opencode export` (`{info: {role}, parts}`) in the shape
/// `session export` gives (`{type, text}` or `{type, content}`); others as they are.
fn older_export_item(item: &Value) -> Value {
    let (Some(role), Some(parts)) = (
        item.get("info").and_then(|i| i.get("role")).and_then(Value::as_str),
        item.get("parts").and_then(Value::as_array),
    ) else {
        return item.clone();
    };
    if role == "user" {
        let text: Vec<&str> = parts
            .iter()
            .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect();
        return serde_json::json!({ "type": "user", "text": text.join("\n") });
    }
    let content: Vec<Value> = parts
        .iter()
        .map(|p| {
            let mut p = p.clone();
            // The tool's output is a string here, blocks there.
            if let Some(state) = p.get_mut("state").and_then(Value::as_object_mut)
                && let Some(Value::String(output)) = state.get("output").cloned()
            {
                state.insert("content".into(), serde_json::json!([{ "type": "text", "text": output }]));
            }
            p
        })
        .collect();
    serde_json::json!({ "type": "assistant", "content": content })
}

/// The transcript of a past session, from `opencode session export` (or an older opencode's
/// `opencode export`).
pub fn export(stdout: &str) -> Vec<Message> {
    let Ok(value) = serde_json::from_str::<Value>(stdout) else { return Vec::new() };
    let Some(items) = value.get("messages").and_then(Value::as_array) else { return Vec::new() };
    let mut messages: Vec<Message> = Vec::new();
    let add = |messages: &mut Vec<Message>, mut parts: Vec<Part>| match messages.last_mut() {
        // A turn's steps come as separate assistant messages: one reply, as live.
        Some(m) if !m.user => m.parts.append(&mut parts),
        _ => messages.push(Message { user: false, parts }),
    };
    for item in items {
        let item = &older_export_item(item);
        match item.get("type").and_then(Value::as_str) {
            Some("user") => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or_default();
                if !text.is_empty() {
                    messages.push(Message::user(text));
                }
            }
            Some("assistant") => {
                let mut parts = Vec::new();
                for part in item.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
                    let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") if !text.is_empty() => parts.push(Part::Text(text.to_owned())),
                        Some("reasoning") if !text.is_empty() => parts.push(Part::Thinking(text.to_owned())),
                        Some("tool") => {
                            let state = part.get("state");
                            let name = part
                                .get("name")
                                .and_then(Value::as_str)
                                .or_else(|| part.get("tool").and_then(Value::as_str))
                                .unwrap_or("tool")
                                .to_owned();
                            let field = |key: &str| state.and_then(|s| s.get(key));
                            parts.push(Part::Tool(Tool {
                                id: part.get("id").and_then(Value::as_str).unwrap_or_default().to_owned(),
                                title: field("title").and_then(Value::as_str).unwrap_or(&name).to_owned(),
                                input: field("input").map(json_text).unwrap_or_default(),
                                output: field("content").filter(|c| c.is_array()).map(content_text).unwrap_or_default(),
                                status: field("status").and_then(Value::as_str).unwrap_or("completed").to_owned(),
                                name,
                            }));
                        }
                        _ => {}
                    }
                }
                if !parts.is_empty() {
                    add(&mut messages, parts);
                }
            }
            Some("idle") if item.get("outcome").and_then(Value::as_str) == Some("failed") => {
                // A run that failed: its error, as live.
                add(&mut messages, vec![Part::Text(format!("Error: {}", error_message(item.get("error"))))]);
            }
            _ => {}
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(id: &str, name: &str, status: &str) -> Tool {
        Tool {
            id: id.into(),
            name: name.into(),
            title: name.into(),
            input: String::new(),
            output: String::new(),
            status: status.into(),
        }
    }

    #[test]
    fn text_thinking_and_tools() {
        assert_eq!(event(r#"{"type":"text","part":{"type":"text","text":"Hi"}}"#), Some(Event::Text("Hi".into())));
        assert_eq!(
            event(r#"{"type":"reasoning","part":{"type":"reasoning","text":"hmm"}}"#),
            Some(Event::Thinking("hmm".into()))
        );
        let Some(Event::Tool(t)) = event(
            r#"{"type":"tool_use","part":{"id":"t1","tool":"bash","state":{"title":"ls","input":{"command":"ls"},"output":"a","status":"completed"}}}"#,
        ) else {
            panic!()
        };
        assert_eq!(
            (t.id.as_str(), t.name.as_str(), t.title.as_str(), t.status.as_str()),
            ("t1", "bash", "ls", "completed")
        );
        assert!(t.input.contains("\"command\": \"ls\""));
        assert_eq!(event(r#"{"type":"step_start","sessionID":"ses_1"}"#), Some(Event::SessionId("ses_1".into())));
    }

    #[test]
    fn live_deltas() {
        assert_eq!(event(r#"{"type":"session.text.delta","data":{"delta":"He"}}"#), Some(Event::Text("He".into())));
        assert_eq!(
            event(r#"{"type":"session.reasoning.delta","data":{"delta":"x"}}"#),
            Some(Event::Thinking("x".into()))
        );
        let Some(Event::Tool(t)) =
            event(r#"{"type":"session.tool.failed","data":{"id":"a","error":{"message":"no"}}}"#)
        else {
            panic!()
        };
        assert_eq!((t.status.as_str(), t.output.as_str()), ("error", "no"));
    }

    #[test]
    fn errors_are_one_line() {
        assert_eq!(
            event(r#"{"type":"error","error":{"name":"X","data":{"a":1},"message":"Model unavailable: m"}}"#),
            Some(Event::Text("Error: Model unavailable: m".into()))
        );
    }

    #[test]
    fn stderr_lines_read_plainly() {
        assert_eq!(event("\u{1b}[31mboom\u{1b}[0m"), Some(Event::Note("boom\n".into())));
        let mut m = vec![Message::user("q")];
        apply(&mut m, Event::Text("Done".into()));
        apply(&mut m, Event::Note("careful\n".into()));
        assert_eq!(m[1].parts, vec![Part::Text("Done\ncareful\n".into())]);
        assert_eq!(event("   "), None);
        assert_eq!(
            stderr_line("! permission requested: edit (src/a.rs); auto-rejecting"),
            "Not allowed in Manual: edit (src/a.rs). Switch to Auto to let the agent do this."
        );
    }

    #[test]
    fn text_joins_and_errors_stand_apart() {
        let mut m = vec![Message::user("q")];
        assert_eq!(apply(&mut m, Event::Text("Hel".into())), Some(1));
        assert_eq!(apply(&mut m, Event::Text("lo".into())), Some(1));
        apply(&mut m, Event::Text("Error: x".into()));
        apply(&mut m, Event::Text("more".into()));
        assert_eq!(
            m[1].parts,
            vec![Part::Text("Hello".into()), Part::Text("Error: x".into()), Part::Text("more".into())]
        );
    }

    #[test]
    fn a_tools_end_keeps_its_start() {
        let mut m = vec![Message::user("q")];
        let mut start = tool("t", "edit", "running");
        start.input = "{\"filePath\":\"a.rs\"}".into();
        apply(&mut m, Event::Tool(start));
        let mut end = tool("t", "tool", "completed");
        end.title = String::new();
        end.output = "ok".into();
        apply(&mut m, Event::Tool(end));
        let Part::Tool(t) = &m[1].parts[0] else { panic!() };
        assert_eq!((t.name.as_str(), t.status.as_str(), t.output.as_str()), ("edit", "completed", "ok"));
        assert!(t.input.contains("a.rs"));
        assert_eq!(m[1].parts.len(), 1);
    }

    #[test]
    fn a_run_new_and_resumed() {
        let prefs = Prefs::default();
        let r = run(true, "ses_coiman_1", "two\nlines", &["src/a.rs".into()], &prefs);
        assert_eq!(
            r.args,
            [
                "--standalone",
                "run",
                "--thinking",
                "--format",
                "json",
                "--session",
                "ses_coiman_1",
                "--agent",
                "build",
                "--file",
                "src/a.rs"
            ]
        );
        assert_eq!(r.stdin, "two\nlines");
        assert!(r.env.is_empty());

        let auto = Prefs { model: Some("m/x".into()), mode: "plan".into(), permissions: "auto".into() };
        let r = run(true, "ses_abc", "hi", &[], &auto);
        assert!(r.args.windows(2).any(|w| w == ["--agent", "plan"]));
        assert!(r.args.windows(2).any(|w| w == ["--model", "m/x"]));
        assert!(r.args.contains(&"--auto".to_owned()));
    }

    #[test]
    fn an_older_opencode() {
        let r = run(false, "ses_coiman_1", "fix  it", &[], &Prefs::default());
        assert!(!r.args.contains(&"--session".to_owned()));
        assert!(!r.args.contains(&"--standalone".to_owned()));
        assert_eq!(r.args[r.args.len() - 2..], ["fix", "it"]);
        assert_eq!(r.stdin, "");
        assert_eq!(r.env, [("OPENCODE_CONFIG_CONTENT".to_owned(), LEGACY_MANUAL_PERMISSIONS.to_owned())]);
        let r = run(false, "ses_given", "x", &[], &Prefs::default());
        assert!(r.args.windows(2).any(|w| w == ["--session", "ses_given"]));
        assert!(has_standalone("  --standalone  run without the service"));
    }

    #[test]
    fn prefs_round_trip() {
        let p = Prefs { model: Some("a/b".into()), mode: "plan".into(), permissions: "auto".into() };
        assert_eq!(Prefs::from_json(&p.to_json()), p);
        assert_eq!(Prefs::from_json("{\"mode\":\"odd\"}"), Prefs::default());
        assert_eq!(Prefs::from_json(""), Prefs::default());
    }

    #[test]
    fn lists() {
        assert_eq!(models("a/b\n\nError: x\n c/d \n"), ["a/b", "c/d"]);
        let s = sessions(r#"[{"id":"s1","title":"Old","updated":1},{"id":"s2","updated":5}]"#);
        assert_eq!(s.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s1"]);
        assert_eq!(s[0].title, "Untitled");
        assert!(sessions("not json").is_empty());
        assert_eq!(with_attachments("hi", &["a.rs".into(), "b".into()]), "@a.rs @b\nhi");
        assert_eq!(with_attachments("hi", &[]), "hi");
    }

    #[test]
    fn an_exported_session() {
        let m = export(
            r#"{"messages":[
            {"type":"user","text":"q"},
            {"type":"assistant","content":[{"type":"reasoning","text":"r"},{"type":"text","text":"a"}]},
            {"type":"assistant","content":[{"type":"tool","name":"bash","id":"t","state":{"title":"ls","input":{"command":"ls"},"content":[{"type":"text","text":"out"}],"status":"completed"}}]},
            {"type":"idle","outcome":"failed","error":{"message":"boom"}}]}"#,
        );
        assert_eq!(m.len(), 2);
        assert!(m[0].user);
        assert_eq!(m[1].parts.len(), 4);
        let Part::Tool(t) = &m[1].parts[2] else { panic!() };
        assert_eq!((t.title.as_str(), t.output.as_str()), ("ls", "out"));
        assert_eq!(m[1].parts[3], Part::Text("Error: boom".into()));
        assert!(export("{}").is_empty());
        assert!(export("").is_empty());
    }

    #[test]
    fn an_older_export() {
        let m = export(
            r#"{"messages":[
            {"info":{"role":"user"},"parts":[{"type":"text","text":"q"}]},
            {"info":{"role":"assistant"},"parts":[{"type":"text","text":"a"},{"type":"tool","tool":"read","id":"x","state":{"output":"file","status":"completed"}}]}]}"#,
        );
        assert_eq!(m[0], Message::user("q"));
        let Part::Tool(t) = &m[1].parts[1] else { panic!() };
        assert_eq!((t.name.as_str(), t.output.as_str()), ("read", "file"));
    }
}
