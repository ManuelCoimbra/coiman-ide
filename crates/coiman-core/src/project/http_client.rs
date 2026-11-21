//! HTTP requests in `.http` (or `.rest`) files, as JetBrains' HTTP Client and VS Code's REST
//! Client write them, sent through `curl`: the app opens no connection itself.
//!
//! ```text
//! @host = https://api.example.com
//!
//! ### Log in
//! # @name login
//! POST {{host}}/login
//! Content-Type: application/json
//!
//! {"user": "ann", "password": "{{password}}"}
//!
//! ###
//! GET {{host}}/orders?limit=10
//! Authorization: Bearer {{login.response.body.$.token}}
//! ```
//!
//! Values come from the file's `@name = value` lines, the chosen environment
//! (`http-client.env.json` with `http-client.private.env.json` over it, or VS Code's
//! `rest-client.environmentVariables`), earlier responses of named requests, and `{{$uuid}}`,
//! `{{$timestamp}}`, `{{$randomInt 1 10}}`, `{{$datetime iso8601}}`, `{{$processEnv NAME}}`,
//! `{{$dotenv NAME}}`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const METHODS: [&str; 9] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];

/// Whether a file is one of these.
pub fn is_http_file(extension: &str) -> bool {
    matches!(extension.to_ascii_lowercase().as_str(), "http" | "rest")
}

/// A request as written, before its `{{variables}}` are filled in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Request {
    /// `# @name login`: later requests can use its response.
    pub name: Option<String>,
    /// The `### Title` above it.
    pub title: Option<String>,
    /// The line (0-based) of `GET …`.
    pub line: usize,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Body>,
    /// `# @no-redirect`: don't follow redirects.
    pub no_redirect: bool,
    /// `# @insecure`: don't check the server's certificate.
    pub insecure: bool,
    /// `# @timeout 10`: seconds.
    pub timeout: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text(String),
    /// `< ./payload.json`: a file's contents, as they are.
    File(String),
}

impl Request {
    /// How it's shown in lists: its title, its name, or the method and address.
    pub fn label(&self) -> String {
        self.title.clone().or_else(|| self.name.clone()).unwrap_or_else(|| format!("{} {}", self.method, self.url))
    }
}

/// A file's requests, and the variables its `@name = value` lines define.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub vars: Vec<(String, String)>,
    pub requests: Vec<Request>,
}

fn comment(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix('#').or_else(|| t.strip_prefix("//")).map(str::trim)
}

fn variable(line: &str) -> Option<(String, String)> {
    let rest = line.trim_start().strip_prefix('@')?;
    let (name, value) = rest.split_once('=')?;
    let name = name.trim();
    (!name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.'))
        .then(|| (name.to_owned(), value.trim().to_owned()))
}

pub fn parse(text: &str) -> File {
    let mut file = File::default();
    let lines: Vec<&str> = text.lines().collect();
    // Blocks between `###` lines: (title, first line, end).
    let mut blocks: Vec<(Option<String>, usize, usize)> = Vec::new();
    let mut start = 0;
    let mut title = None;
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = line.trim_start().strip_prefix("###") {
            blocks.push((title.take(), start, i));
            let t = rest.trim();
            title = (!t.is_empty()).then(|| t.to_owned());
            start = i + 1;
        }
    }
    blocks.push((title, start, lines.len()));
    for (title, start, end) in blocks {
        let mut req = Request { title, ..Request::default() };
        let mut i = start;
        // Comments, directives and variables before the request line.
        let mut found = false;
        while i < end {
            let line = lines[i];
            if line.trim().is_empty() {
                i += 1;
                continue;
            }
            if let Some(c) = comment(line) {
                if let Some(d) = c.strip_prefix('@') {
                    let mut words = d.split_whitespace();
                    match words.next() {
                        Some("name") => req.name = words.next().map(str::to_owned),
                        Some("no-redirect") => req.no_redirect = true,
                        Some("insecure") | Some("no-reject-unauthorized") => req.insecure = true,
                        Some("timeout") => req.timeout = words.next().and_then(|n| n.parse().ok()),
                        _ => {}
                    }
                }
                i += 1;
                continue;
            }
            if let Some(var) = variable(line) {
                file.vars.push(var);
                i += 1;
                continue;
            }
            found = true;
            break;
        }
        if !found {
            continue;
        }
        // The request line: `METHOD URL [HTTP/1.1]`, or just the URL (GET).
        req.line = i;
        let words: Vec<&str> = lines[i].split_whitespace().collect();
        let (method, rest) = match words.first() {
            Some(m) if METHODS.contains(m) => (m.to_string(), &words[1..]),
            _ => ("GET".to_owned(), &words[..]),
        };
        let rest: Vec<&str> = rest.iter().copied().filter(|w| !w.starts_with("HTTP/")).collect();
        req.method = method;
        req.url = rest.join(" ");
        i += 1;
        // The query may go on over lines starting with `?` or `&`.
        while i < end && matches!(lines[i].trim_start().chars().next(), Some('?' | '&')) {
            req.url.push_str(lines[i].trim());
            i += 1;
        }
        // Headers, up to a blank line.
        while i < end && !lines[i].trim().is_empty() {
            if comment(lines[i]).is_none()
                && let Some((name, value)) = lines[i].split_once(':')
            {
                req.headers.push((name.trim().to_owned(), value.trim().to_owned()));
            }
            i += 1;
        }
        // The body: what's left, without JetBrains' response handlers (`> {% … %}`, `>> file`).
        let mut body: Vec<&str> = lines[i.min(end)..end].to_vec();
        while body.first().is_some_and(|l| l.trim().is_empty()) {
            body.remove(0);
        }
        if let Some(at) = body.iter().position(|l| l.starts_with("> ") || l.starts_with(">> ") || l.starts_with(">>! "))
        {
            body.truncate(at);
        }
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        if !body.is_empty() {
            req.body = Some(match body[0].trim().strip_prefix("< ") {
                Some(path) if body.len() == 1 => Body::File(path.trim().to_owned()),
                _ => Body::Text(body.join("\n")),
            });
        }
        file.requests.push(req);
    }
    file
}

/// The request whose lines hold `line` (the one starting at or before it).
pub fn request_at(file: &File, line: usize) -> Option<&Request> {
    file.requests.iter().rev().find(|r| r.line <= line).or_else(|| file.requests.first())
}

// ----- Environments ------------------------------------------------------------------------

/// The environments for a `.http` file: `http-client.env.json` beside it or in a folder above
/// it (up to `root`), with `http-client.private.env.json` over it, then VS Code's
/// `rest-client.environmentVariables` (with its `$shared` in each). By name.
pub fn environments(http_file: &Path, root: Option<&Path>) -> Vec<(String, HashMap<String, String>)> {
    let mut envs: Vec<(String, HashMap<String, String>)> = Vec::new();
    let mut add = |name: &str, vars: &serde_json::Map<String, Value>| {
        let entry = match envs.iter_mut().find(|(n, _)| n == name) {
            Some((_, e)) => e,
            None => {
                envs.push((name.to_owned(), HashMap::new()));
                &mut envs.last_mut().expect("just pushed").1
            }
        };
        for (k, v) in vars {
            let v = match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            entry.insert(k.clone(), v);
        }
    };
    let read = |p: &Path| -> Option<serde_json::Map<String, Value>> {
        let text = std::fs::read_to_string(p).ok()?;
        serde_json::from_str::<Value>(&crate::appearance::theme::strip_jsonc(&text)).ok()?.as_object().cloned()
    };
    let mut dir = http_file.parent();
    while let Some(d) = dir {
        let public = read(&d.join("http-client.env.json"));
        let private = read(&d.join("http-client.private.env.json"));
        if public.is_some() || private.is_some() {
            for file in [public, private].into_iter().flatten() {
                for (name, vars) in &file {
                    if let Some(vars) = vars.as_object() {
                        add(name, vars);
                    }
                }
            }
            break;
        }
        if root.is_some_and(|r| d == r) {
            break;
        }
        dir = d.parent();
    }
    if let Some(settings) = root.and_then(|r| read(&r.join(".vscode").join("settings.json")))
        && let Some(envs_vs) = settings.get("rest-client.environmentVariables").and_then(Value::as_object)
    {
        let shared = envs_vs.get("$shared").and_then(Value::as_object).cloned().unwrap_or_default();
        for (name, vars) in envs_vs {
            if name == "$shared" {
                continue;
            }
            add(name, &shared);
            if let Some(vars) = vars.as_object() {
                add(name, vars);
            }
        }
    }
    envs
}

/// `NAME=value` lines of the `.env` file beside the `.http` file or in the project.
fn dotenv(http_file: &Path, root: Option<&Path>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for dir in http_file.parent().into_iter().chain(root) {
        if let Ok(text) = std::fs::read_to_string(dir.join(".env")) {
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.trim_start_matches("export ").split_once('=') {
                    out.entry(k.trim().to_owned()).or_insert_with(|| v.trim().trim_matches(['"', '\'']).to_owned());
                }
            }
        }
    }
    out
}

// ----- Filling in variables ----------------------------------------------------------------

/// What `{{…}}` can stand for.
pub struct Values<'a> {
    pub file_vars: &'a [(String, String)],
    pub env: Option<&'a HashMap<String, String>>,
    /// Responses of named requests, latest first.
    pub responses: &'a HashMap<String, Response>,
    pub dotenv: HashMap<String, String>,
}

impl<'a> Values<'a> {
    pub fn new(
        file: &'a File,
        env: Option<&'a HashMap<String, String>>,
        responses: &'a HashMap<String, Response>,
        http_file: &Path,
        root: Option<&Path>,
    ) -> Self {
        Values { file_vars: &file.vars, env, responses, dotenv: dotenv(http_file, root) }
    }

    /// `text` with each `{{…}}` filled in. An unknown name is an error, said plainly.
    pub fn fill(&self, text: &str) -> Result<String, String> {
        self.fill_depth(text, 0)
    }

    fn fill_depth(&self, text: &str, depth: usize) -> Result<String, String> {
        if depth > 8 {
            return Err("variables refer to each other in a loop".to_owned());
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("{{") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                out.push_str(&rest[start..]);
                return Ok(out);
            };
            let name = after[..end].trim();
            out.push_str(&self.value(name, depth)?);
            rest = &after[end + 2..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn value(&self, name: &str, depth: usize) -> Result<String, String> {
        if let Some(system) = name.strip_prefix('$') {
            return self.system(system);
        }
        // An earlier response: `login.response.body.$.token`, `login.response.headers.Location`.
        if let Some((req, rest)) = name.split_once(".response.") {
            let Some(resp) = self.responses.get(req) else {
                return Err(format!("{{{{{name}}}}}: send the request named {req} first"));
            };
            return if let Some(path) = rest.strip_prefix("body") {
                let path = path.trim_start_matches('.');
                if path.is_empty() || path == "*" {
                    Ok(String::from_utf8_lossy(&resp.body).into_owned())
                } else {
                    resp.json_path(path)
                        .ok_or_else(|| format!("{{{{{name}}}}}: {req}'s response has nothing at {path}"))
                }
            } else if let Some(header) = rest.strip_prefix("headers.") {
                resp.header(header).map(str::to_owned).ok_or_else(|| format!("{{{{{name}}}}}: no {header} header"))
            } else {
                Err(format!("{{{{{name}}}}}: say response.body… or response.headers…"))
            };
        }
        let found = self
            .env
            .and_then(|e| e.get(name).cloned())
            .or_else(|| self.file_vars.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()));
        match found {
            Some(v) => self.fill_depth(&v, depth + 1),
            None => Err(format!("{{{{{name}}}}} isn't defined: add @{name} = … or choose an environment that has it")),
        }
    }

    fn system(&self, spec: &str) -> Result<String, String> {
        let mut words = spec.split_whitespace();
        let head = words.next().unwrap_or_default();
        let now = chrono::Utc::now();
        Ok(match head {
            "uuid" | "guid" | "random.uuid" => uuid4(),
            "timestamp" => now.timestamp().to_string(),
            "isoTimestamp" => now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            "randomInt" => {
                let lo: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
                let hi: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(1000);
                (lo + (random_u64() % (hi - lo).max(1) as u64) as i64).to_string()
            }
            "datetime" | "localDatetime" => match words.next().unwrap_or("iso8601") {
                "rfc1123" => now.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                _ => now.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            },
            "processEnv" => {
                let var = words.next().ok_or("{{$processEnv NAME}}: which variable?")?;
                std::env::var(var.trim_start_matches('%')).map_err(|_| format!("{{{{$processEnv {var}}}}}: not set"))?
            }
            "dotenv" => {
                let var = words.next().ok_or("{{$dotenv NAME}}: which variable?")?;
                self.dotenv.get(var).cloned().ok_or_else(|| format!("{{{{$dotenv {var}}}}}: not in .env"))?
            }
            other => return Err(format!("{{{{${other}}}}} isn't a value Coiman knows")),
        })
    }
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.finish()
}

fn uuid4() -> String {
    let (a, b) = (random_u64(), random_u64());
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_le_bytes());
    bytes[8..].copy_from_slice(&b.to_le_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// A request with its values filled in, ready to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    pub no_redirect: bool,
    pub insecure: bool,
    pub timeout: Option<u32>,
}

pub fn prepare(req: &Request, values: &Values, http_file: &Path) -> Result<Ready, String> {
    let url = values.fill(&req.url)?;
    let url = if url.contains("://") { url } else { format!("http://{url}") };
    let mut headers = Vec::new();
    for (k, v) in &req.headers {
        headers.push((values.fill(k)?, values.fill(v)?));
    }
    let body = match &req.body {
        None => None,
        Some(Body::Text(t)) => Some(values.fill(t)?.into_bytes()),
        Some(Body::File(p)) => {
            let path = http_file.parent().unwrap_or(Path::new(".")).join(values.fill(p)?);
            Some(std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?)
        }
    };
    Ok(Ready {
        method: req.method.clone(),
        url,
        headers,
        body,
        no_redirect: req.no_redirect,
        insecure: req.insecure,
        timeout: req.timeout,
    })
}

// ----- Sending -----------------------------------------------------------------------------

/// What came back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub reason: String,
    pub version: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub millis: f64,
    /// Where it ended up, after redirects.
    pub url: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn is_json(&self) -> bool {
        self.header("content-type").is_some_and(|c| c.contains("json"))
            || serde_json::from_slice::<Value>(&self.body).is_ok_and(|v| v.is_object() || v.is_array())
    }

    /// The body as text to read: JSON indented, the rest as it came.
    pub fn pretty(&self) -> String {
        if self.is_json()
            && let Ok(v) = serde_json::from_slice::<Value>(&self.body)
        {
            return serde_json::to_string_pretty(&v).unwrap_or_default();
        }
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// A value in a JSON body, by a path like `$.data.items[0].id` (or without the `$.`).
    pub fn json_path(&self, path: &str) -> Option<String> {
        let v: Value = serde_json::from_slice(&self.body).ok()?;
        let path = path.trim().trim_start_matches('$').trim_start_matches('.');
        let mut at = &v;
        for part in path.split('.').filter(|p| !p.is_empty()) {
            let (key, indexes) = match part.find('[') {
                Some(i) => (&part[..i], &part[i..]),
                None => (part, ""),
            };
            if !key.is_empty() {
                at = at.get(key)?;
            }
            for idx in indexes.split('[').filter(|s| !s.is_empty()) {
                at = at.get(idx.trim_end_matches(']').parse::<usize>().ok()?)?;
            }
        }
        Some(match at {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }
}

/// The curl command line for a request, as one could paste it in a shell (for "Copy as curl").
pub fn curl_line(ready: &Ready) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut out = format!("curl -X {} {}", ready.method, q(&ready.url));
    for (k, v) in &ready.headers {
        out.push_str(&format!(" \\\n  -H {}", q(&format!("{k}: {v}"))));
    }
    if let Some(body) = &ready.body {
        out.push_str(&format!(" \\\n  --data-binary {}", q(&String::from_utf8_lossy(body))));
    }
    if !ready.no_redirect {
        out.push_str(" -L");
    }
    if ready.insecure {
        out.push_str(" -k");
    }
    out
}

/// A number not used before in this run, for temporary file names.
fn unique() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Sends a request through `curl` and reads what came back.
pub fn send(ready: &Ready) -> Result<Response, String> {
    let curl = crate::language::lsp::servers::system_tool("curl");
    let dir = std::env::temp_dir();
    let stem = format!("coiman-http-{}-{}", std::process::id(), unique());
    let (head_file, body_file, data_file) =
        (dir.join(format!("{stem}.head")), dir.join(format!("{stem}.body")), dir.join(format!("{stem}.data")));
    let mut cmd = crate::language::lsp::discovery::quiet_command(&curl);
    cmd.args(["-sS", "--compressed", "-D"]).arg(&head_file).arg("-o").arg(&body_file).args(["-w", "%{json}"]);
    // `-X HEAD` waits for a body that never comes: `-I` asks for the head only.
    if ready.method == "HEAD" {
        cmd.arg("-I");
    } else {
        cmd.args(["-X", &ready.method]);
    }
    for (k, v) in &ready.headers {
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    if let Some(body) = &ready.body {
        std::fs::write(&data_file, body).map_err(|e| e.to_string())?;
        cmd.arg("--data-binary").arg(format!("@{}", data_file.display()));
    }
    if !ready.no_redirect {
        cmd.args(["-L", "--max-redirs", "10"]);
    }
    if ready.insecure {
        cmd.arg("-k");
    }
    cmd.args(["--max-time", &ready.timeout.unwrap_or(60).to_string()]);
    cmd.arg("--url").arg(&ready.url).stdin(Stdio::null());
    let out = cmd.output().map_err(|e| format!("Couldn't start curl: {e}"));
    let head = std::fs::read_to_string(&head_file).unwrap_or_default();
    let body = std::fs::read(&body_file).unwrap_or_default();
    for f in [&head_file, &body_file, &data_file] {
        let _ = std::fs::remove_file(f);
    }
    let out = out?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.lines().find(|l| l.starts_with("curl:")).unwrap_or(err.trim());
        return Err(err.trim_start_matches("curl: ").to_owned());
    }
    let info: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let (status, reason, version, headers) = parse_head(&head);
    Ok(Response {
        status: info["http_code"].as_u64().map_or(status, |c| c as u16),
        reason,
        version,
        headers,
        body,
        millis: info["time_total"].as_f64().unwrap_or(0.0) * 1000.0,
        url: info["url_effective"].as_str().unwrap_or(&ready.url).to_owned(),
    })
}

/// The last block of a headers dump (after redirects and `100 Continue`): status and headers.
fn parse_head(head: &str) -> (u16, String, String, Vec<(String, String)>) {
    let text = head.replace("\r\n", "\n");
    let block = text.split("\n\n").filter(|b| b.trim_start().starts_with("HTTP/")).last().unwrap_or("");
    let mut lines = block.trim().lines();
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("").to_owned();
    let status = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let reason = parts.next().unwrap_or("").trim().to_owned();
    let headers =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
    (status, reason, version, headers)
}

// ----- History -----------------------------------------------------------------------------

/// A response kept: what was asked, and what came.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since 1970.
    pub when: i64,
    pub request: String,
    pub method: String,
    pub url: String,
    pub response: Response,
}

/// How many responses a file keeps, and how much of each body.
pub const KEEP: usize = 50;
pub const KEEP_BODY: usize = 512 * 1024;

fn history_file(http_file: &Path) -> Option<PathBuf> {
    use sha2::Digest;
    let key = sha2::Sha256::digest(http_file.to_string_lossy().as_bytes());
    let name: String = key.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Some(crate::settings::config_dir()?.join("http-history").join(format!("{name}.json")))
}

/// Responses kept for this file, newest first.
pub fn history(http_file: &Path) -> Vec<Entry> {
    history_file(http_file)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn remember(http_file: &Path, mut entry: Entry) -> Result<Vec<Entry>, String> {
    entry.response.body.truncate(KEEP_BODY);
    let mut all = history(http_file);
    all.insert(0, entry);
    all.truncate(KEEP);
    let path = history_file(http_file).ok_or("no config folder")?;
    std::fs::create_dir_all(path.parent().expect("in a folder")).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string(&all).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(all)
}

pub fn clear_history(http_file: &Path) {
    if let Some(f) = history_file(http_file) {
        let _ = std::fs::remove_file(f);
    }
}

/// Two responses side by side: the status, each header that differs, and the body's lines,
/// each line marked `-` (only in the first), `+` (only in the second) or ` ` (both).
pub fn compare(a: &Response, b: &Response) -> Vec<(char, String)> {
    let mut out = Vec::new();
    if (a.status, &a.reason) != (b.status, &b.reason) {
        out.push(('-', format!("{} {}", a.status, a.reason)));
        out.push(('+', format!("{} {}", b.status, b.reason)));
    }
    let names: Vec<String> = {
        let mut n: Vec<String> = a.headers.iter().chain(&b.headers).map(|(k, _)| k.to_ascii_lowercase()).collect();
        n.sort();
        n.dedup();
        n
    };
    // Headers that change on every response say nothing.
    for name in names.iter().filter(|n| !matches!(n.as_str(), "date" | "age" | "x-request-id" | "set-cookie")) {
        let (x, y) = (a.header(name), b.header(name));
        if x != y {
            if let Some(x) = x {
                out.push(('-', format!("{name}: {x}")));
            }
            if let Some(y) = y {
                out.push(('+', format!("{name}: {y}")));
            }
        }
    }
    if !out.is_empty() {
        out.push((' ', String::new()));
    }
    let (ta, tb) = (a.pretty(), b.pretty());
    for change in similar::TextDiff::from_lines(&ta, &tb).iter_all_changes() {
        let mark = match change.tag() {
            similar::ChangeTag::Delete => '-',
            similar::ChangeTag::Insert => '+',
            similar::ChangeTag::Equal => ' ',
        };
        out.push((mark, change.value().trim_end_matches('\n').to_owned()));
    }
    out
}

/// A syntax definition for `.http` files.
pub const SYNTAX: &str = r#"%YAML 1.2
---
name: HTTP Request
file_extensions: [http, rest]
scope: source.http
contexts:
  main:
    - match: '^\s*###.*$'
      scope: comment.line.separator.http markup.heading.http
    - match: '^\s*(#|//)\s*@\w+.*$'
      scope: keyword.other.directive.http
    - match: '^\s*(#|//).*$'
      scope: comment.line.http
    - match: '^\s*(@)([\w.-]+)\s*(=)'
      captures:
        1: punctuation.definition.variable.http
        2: variable.other.http
        3: keyword.operator.assignment.http
    - match: '^\s*(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|TRACE|CONNECT)\b'
      scope: keyword.control.method.http
    - match: '\bHTTP/[\d.]+\b'
      scope: constant.language.version.http
    - match: '^\s*([\w-]+)(:)\s'
      captures:
        1: entity.name.tag.header.http
        2: punctuation.separator.http
    - match: '\{\{'
      scope: punctuation.definition.template.begin.http
      push: template
    - match: '"(?:[^"\\]|\\.)*"(?=\s*:)'
      scope: support.type.property-name.json.http
    - match: '"(?:[^"\\]|\\.)*"'
      scope: string.quoted.double.http
    - match: '\b(true|false|null)\b'
      scope: constant.language.http
    - match: '-?\b\d+(\.\d+)?([eE][-+]?\d+)?\b'
      scope: constant.numeric.http
  template:
    - meta_scope: variable.parameter.template.http
    - match: '\}\}'
      scope: punctuation.definition.template.end.http
      pop: true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "@host = localhost:{{port}}\n@port = 8080\n\n### Log in\n# @name login\nPOST http://{{host}}/login HTTP/1.1\nContent-Type: application/json\n\n{\"user\": \"ann\"}\n\n> {% client.global.set(\"x\", 1) %}\n\n###\n// a comment\nGET {{host}}/orders\n  ?limit=10\n  &sort=new\nAuthorization: Bearer {{login.response.body.$.token}}\n\n### Upload\n# @no-redirect\n# @timeout 5\nPUT http://example.com/up\n\n< ./payload.json\n";

    #[test]
    fn parses_files() {
        let f = parse(SAMPLE);
        assert_eq!(
            f.vars,
            [("host".to_owned(), "localhost:{{port}}".to_owned()), ("port".to_owned(), "8080".to_owned())]
        );
        assert_eq!(f.requests.len(), 3);
        let login = &f.requests[0];
        assert_eq!(
            (login.title.as_deref(), login.name.as_deref(), login.method.as_str()),
            (Some("Log in"), Some("login"), "POST")
        );
        assert_eq!(login.url, "http://{{host}}/login");
        assert_eq!(login.line, 5);
        assert_eq!(login.headers, [("Content-Type".to_owned(), "application/json".to_owned())]);
        assert_eq!(login.body, Some(Body::Text("{\"user\": \"ann\"}".to_owned())));
        let orders = &f.requests[1];
        assert_eq!(orders.method, "GET");
        assert_eq!(orders.url, "{{host}}/orders?limit=10&sort=new");
        assert_eq!(orders.label(), "GET {{host}}/orders?limit=10&sort=new");
        let up = &f.requests[2];
        assert!(up.no_redirect && up.timeout == Some(5));
        assert_eq!(up.body, Some(Body::File("./payload.json".to_owned())));
        assert_eq!(request_at(&f, 17).unwrap().method, "GET");
        assert_eq!(request_at(&f, 0).unwrap().name.as_deref(), Some("login"));
        // Just an address is a GET.
        assert_eq!(parse("https://example.com/x").requests[0].method, "GET");
    }

    #[test]
    fn fills_values() {
        let f = parse(SAMPLE);
        let mut responses = HashMap::new();
        let body = br#"{"token": "abc", "items": [{"id": 7}]}"#.to_vec();
        responses.insert(
            "login".to_owned(),
            Response {
                status: 200,
                reason: "OK".into(),
                version: "HTTP/1.1".into(),
                headers: vec![("Location".into(), "/x".into())],
                body,
                millis: 1.0,
                url: String::new(),
            },
        );
        let env: HashMap<String, String> = [("port".to_owned(), "9000".to_owned())].into();
        let values = Values { file_vars: &f.vars, env: Some(&env), responses: &responses, dotenv: HashMap::new() };
        // The environment wins over the file; variables refer to others.
        assert_eq!(values.fill("{{host}}/a").unwrap(), "localhost:9000/a");
        assert_eq!(values.fill("{{login.response.body.$.token}}").unwrap(), "abc");
        assert_eq!(values.fill("{{login.response.body.items[0].id}}").unwrap(), "7");
        assert_eq!(values.fill("{{login.response.headers.location}}").unwrap(), "/x");
        assert!(values.fill("{{nope}}").unwrap_err().contains("isn't defined"));
        assert!(values.fill("{{other.response.body.x}}").unwrap_err().contains("send the request named other first"));
        assert_eq!(values.fill("{{$uuid}}").unwrap().len(), 36);
        let n: i64 = values.fill("{{$randomInt 5 6}}").unwrap().parse().unwrap();
        assert_eq!(n, 5);
        let ready = prepare(&f.requests[1], &values, Path::new("x.http")).unwrap();
        assert_eq!(ready.url, "http://localhost:9000/orders?limit=10&sort=new");
        assert_eq!(ready.headers[0].1, "Bearer abc");
        assert!(curl_line(&ready).starts_with("curl -X GET 'http://localhost:9000/orders?limit=10&sort=new'"));
        let looped = File { vars: vec![("a".into(), "{{b}}".into()), ("b".into(), "{{a}}".into())], requests: vec![] };
        let values = Values { file_vars: &looped.vars, env: None, responses: &responses, dotenv: HashMap::new() };
        assert!(values.fill("{{a}}").unwrap_err().contains("loop"));
    }

    #[test]
    fn heads_environments_and_comparisons() {
        let head =
            "HTTP/1.1 301 Moved\r\nLocation: /b\r\n\r\nHTTP/2 200 \r\ncontent-type: application/json\r\nx-a: 1\r\n\r\n";
        let (status, _, version, headers) = parse_head(head);
        assert_eq!((status, version.as_str()), (200, "HTTP/2"));
        assert_eq!(headers.len(), 2);

        let dir = std::env::temp_dir().join(format!("coiman-http-env-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("api").join(".vscode")).unwrap();
        std::fs::write(
            dir.join("http-client.env.json"),
            r#"{"dev": {"host": "localhost", "token": "public"}, "prod": {"host": "example.com"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("http-client.private.env.json"), r#"{"dev": {"token": "secret"}}"#).unwrap();
        let envs = environments(&dir.join("api").join("a.http"), Some(&dir));
        let dev = &envs.iter().find(|(n, _)| n == "dev").unwrap().1;
        assert_eq!((dev["host"].as_str(), dev["token"].as_str()), ("localhost", "secret"));
        assert!(envs.iter().any(|(n, _)| n == "prod"));
        std::fs::remove_dir_all(&dir).unwrap();

        let resp = |status: u16, body: &str, h: &str| Response {
            status,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            headers: vec![("Content-Type".into(), "application/json".into()), ("Date".into(), h.into())],
            body: body.as_bytes().to_vec(),
            millis: 0.0,
            url: String::new(),
        };
        let diff = compare(&resp(200, r#"{"a":1,"b":2}"#, "x"), &resp(200, r#"{"a":1,"b":3}"#, "y"));
        // The date differs but isn't shown; one body line changed.
        assert!(diff.iter().all(|(_, l)| !l.contains("date")));
        assert!(diff.contains(&('-', "  \"b\": 2".to_owned())) && diff.contains(&('+', "  \"b\": 3".to_owned())));
        assert!(syntect::parsing::SyntaxDefinition::load_from_str(SYNTAX, true, None).is_ok());
    }

    /// Against a server made here: a request goes out through curl and comes back.
    #[test]
    fn sends_through_curl() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut got = Vec::new();
            // Read the head and the 13-byte body.
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                if let Some(at) = got.windows(4).position(|w| w == b"\r\n\r\n")
                    && got.len() >= at + 4 + 13
                {
                    break;
                }
            }
            let body = b"{\"ok\": true, \"token\": \"t1\"}";
            let head = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nX-Seen: yes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            s.write_all(head.as_bytes()).unwrap();
            s.write_all(body).unwrap();
            String::from_utf8_lossy(&got).into_owned()
        });
        let ready = Ready {
            method: "POST".into(),
            url: format!("http://127.0.0.1:{port}/things?x=1"),
            headers: vec![("Content-Type".into(), "application/json".into()), ("X-Name".into(), "coiman".into())],
            body: Some(br#"{"name": "a"}"#.to_vec()),
            no_redirect: false,
            insecure: false,
            timeout: Some(10),
        };
        let resp = send(&ready).unwrap();
        let seen = server.join().unwrap();
        assert!(seen.starts_with("POST /things?x=1 HTTP/1.1"), "{seen}");
        assert!(seen.contains("X-Name: coiman") && seen.ends_with(r#"{"name": "a"}"#), "{seen}");
        assert_eq!((resp.status, resp.reason.as_str()), (201, "Created"));
        assert_eq!(resp.header("x-seen"), Some("yes"));
        assert_eq!(resp.json_path("$.token").as_deref(), Some("t1"));
        assert!(resp.pretty().contains("\n  \"ok\": true"));
        // Nothing listening: curl's own words.
        let down = Ready { url: "http://127.0.0.1:1/".into(), body: None, ..ready };
        let e = send(&down).unwrap_err();
        assert!(e.to_lowercase().contains("connect"), "{e}");
    }
}
