//! The MCP servers opencode gives its agent, as listed under `mcp` in an `opencode.json`:
//! the project's own (in its folder) or the user's (`~/.config/opencode`, for every project).
//! Coiman edits only that section; the rest of the file is kept.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// One MCP server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub enabled: bool,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A program opencode starts: the command and its arguments, and environment variables.
    Local { command: Vec<String>, env: Vec<(String, String)> },
    /// A server at an address, with headers (an `Authorization` one, say).
    Remote { url: String, headers: Vec<(String, String)> },
}

/// The config file in `dir`: `opencode.jsonc` if that is the one there, else `opencode.json`.
fn file_in(dir: &Path) -> PathBuf {
    let jsonc = dir.join("opencode.jsonc");
    if jsonc.is_file() && !dir.join("opencode.json").is_file() { jsonc } else { dir.join("opencode.json") }
}

/// Where a config is.
pub fn project_file(root: &Path) -> PathBuf {
    file_in(root)
}

pub fn user_file() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(file_in(&PathBuf::from(home).join(".config").join("opencode")))
}

/// JSON with `//` and `/* */` comments (opencode reads JSONC) as plain JSON. Strings are kept.
fn strip_comments(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() {
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
            }
            _ => out.push(c),
        }
    }
    out
}

fn pairs(v: &Value) -> Vec<(String, String)> {
    v.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect())
        .unwrap_or_default()
}

fn object(pairs: &[(String, String)]) -> Value {
    Value::Object(pairs.iter().map(|(k, v)| (k.clone(), json!(v))).collect())
}

/// A config file read: its servers, and whether it had comments (which saving drops).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub servers: Vec<Server>,
    pub had_comments: bool,
    doc: Map<String, Value>,
}

impl Config {
    /// Reads `path`; a file that isn't there is an empty config.
    pub fn load(path: &Path) -> Result<Self, String> {
        let Ok(text) = std::fs::read_to_string(path) else { return Ok(Self::default()) };
        if text.trim().is_empty() {
            return Ok(Self::default());
        }
        let plain = strip_comments(&text);
        let had_comments = plain.len() != text.len();
        let doc: Value = serde_json::from_str(&plain).map_err(|e| format!("{}: {e}", path.display()))?;
        let doc = doc.as_object().cloned().ok_or_else(|| format!("{}: not a JSON object", path.display()))?;
        let mut servers = Vec::new();
        if let Some(mcp) = doc.get("mcp").and_then(Value::as_object) {
            for (name, s) in mcp {
                let enabled = s["enabled"].as_bool().unwrap_or(true);
                let kind = if s["type"] == "remote" {
                    Kind::Remote {
                        url: s["url"].as_str().unwrap_or_default().to_owned(),
                        headers: pairs(&s["headers"]),
                    }
                } else {
                    let command = match &s["command"] {
                        Value::Array(parts) => parts.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect(),
                        Value::String(one) => vec![one.clone()],
                        _ => Vec::new(),
                    };
                    Kind::Local { command, env: pairs(&s["environment"]) }
                };
                servers.push(Server { name: name.clone(), enabled, kind });
            }
        }
        Ok(Self { servers, had_comments, doc })
    }

    /// The file's text with these servers under `mcp` (fields of each kept where Coiman has none).
    pub fn to_text(&self) -> String {
        let mut doc = self.doc.clone();
        let old = doc.get("mcp").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut mcp = Map::new();
        for s in &self.servers {
            let mut entry = old.get(&s.name).and_then(Value::as_object).cloned().unwrap_or_default();
            entry.insert("enabled".into(), json!(s.enabled));
            match &s.kind {
                Kind::Local { command, env } => {
                    entry.insert("type".into(), json!("local"));
                    entry.insert("command".into(), json!(command));
                    entry.remove("url");
                    entry.remove("headers");
                    if env.is_empty() {
                        entry.remove("environment");
                    } else {
                        entry.insert("environment".into(), object(env));
                    }
                }
                Kind::Remote { url, headers } => {
                    entry.insert("type".into(), json!("remote"));
                    entry.insert("url".into(), json!(url));
                    entry.remove("command");
                    entry.remove("environment");
                    if headers.is_empty() {
                        entry.remove("headers");
                    } else {
                        entry.insert("headers".into(), object(headers));
                    }
                }
            }
            mcp.insert(s.name.clone(), Value::Object(entry));
        }
        if mcp.is_empty() {
            doc.remove("mcp");
        } else {
            doc.insert("mcp".into(), Value::Object(mcp));
        }
        doc.entry("$schema").or_insert_with(|| json!("https://opencode.ai/config.json"));
        let mut text = serde_json::to_string_pretty(&Value::Object(doc)).unwrap_or_default();
        text.push('\n');
        text
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, self.to_text()).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// What's wrong with a server before saving it, if anything.
pub fn problem(s: &Server, others: &[Server]) -> Option<&'static str> {
    if s.name.trim().is_empty() {
        return Some("Give it a name");
    }
    if others.iter().any(|o| o.name == s.name) {
        return Some("Another server has this name");
    }
    match &s.kind {
        Kind::Local { command, .. } if command.first().is_none_or(|c| c.trim().is_empty()) => {
            Some("Type the command that starts it")
        }
        Kind::Remote { url, .. } if !(url.starts_with("https://") || url.starts_with("http://")) => {
            Some("The address starts with https:// (or http://)")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  // my settings
  "$schema": "https://opencode.ai/config.json",
  "model": "opencode/big-pickle", /* kept */
  "mcp": {
    "files": {"type": "local", "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem", "."], "enabled": true, "timeout": 5000},
    "docs": {"type": "remote", "url": "https://mcp.example.com/mcp", "enabled": false, "headers": {"Authorization": "Bearer x"}}
  }
}"#;

    #[test]
    fn reads_edits_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("coiman-mcp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(project_file(&dir), dir.join("opencode.json"));
        std::fs::write(dir.join("opencode.jsonc"), "{}").unwrap();
        assert_eq!(project_file(&dir), dir.join("opencode.jsonc"));
        std::fs::remove_file(dir.join("opencode.jsonc")).unwrap();
        let path = dir.join("opencode.json");
        assert_eq!(Config::load(&path).unwrap().servers, []);
        std::fs::write(&path, SAMPLE).unwrap();
        let mut c = Config::load(&path).unwrap();
        assert!(c.had_comments);
        assert_eq!(c.servers.len(), 2);
        let docs = c.servers.iter().find(|s| s.name == "docs").unwrap();
        assert!(!docs.enabled);
        assert_eq!(
            docs.kind,
            Kind::Remote {
                url: "https://mcp.example.com/mcp".into(),
                headers: vec![("Authorization".into(), "Bearer x".into())]
            }
        );
        // Turn one on, add one, and save: the model and a server's own timeout stay.
        c.servers.iter_mut().find(|s| s.name == "docs").unwrap().enabled = true;
        c.servers.push(Server {
            name: "git".into(),
            enabled: true,
            kind: Kind::Local { command: vec!["uvx".into(), "mcp-server-git".into()], env: vec![] },
        });
        c.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"model\": \"opencode/big-pickle\"") && text.contains("\"timeout\": 5000"), "{text}");
        let again = Config::load(&path).unwrap();
        assert!(!again.had_comments);
        assert_eq!(again.servers.len(), 3);
        assert!(again.servers.iter().all(|s| s.enabled));
        // Removing them all removes the section.
        let mut none = again.clone();
        none.servers.clear();
        assert!(!none.to_text().contains("\"mcp\""));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn comments_strings_and_problems() {
        assert_eq!(
            strip_comments(r#"{"a": "http://x//y", /* c */ "b": 1} // end"#),
            r#"{"a": "http://x//y",  "b": 1} "#
        );
        let local = |cmd: &[&str]| Server {
            name: "x".into(),
            enabled: true,
            kind: Kind::Local { command: cmd.iter().map(|s| s.to_string()).collect(), env: vec![] },
        };
        assert_eq!(problem(&local(&[]), &[]), Some("Type the command that starts it"));
        assert_eq!(problem(&local(&["npx"]), &[local(&["a"])]), Some("Another server has this name"));
        let remote = Server {
            name: "r".into(),
            enabled: true,
            kind: Kind::Remote { url: "mcp.example.com".into(), headers: vec![] },
        };
        assert!(problem(&remote, &[]).is_some());
        assert_eq!(problem(&local(&["npx"]), &[]), None);
    }
}
