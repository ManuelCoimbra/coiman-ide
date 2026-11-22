//! HTTP requests in `.http` (or `.rest`) files, as JetBrains' HTTP Client and VS Code's REST
//! Client write them, sent through `curl`: the editor opens no connection itself.
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

use serde::{Deserialize, Serialize};
use serde_json::Value;

const METHODS: [&str; 9] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE", "CONNECT"];

/// Whether a file is one of these, by its path.
pub fn is_http_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".http") || lower.ends_with(".rest")
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

// ----- Paths in the open folder ------------------------------------------------------------

/// The folder of a file (relative to the open folder; "" at the top).
pub fn folder_of(path: &str) -> String {
    let path = path.replace('\\', "/");
    path.rsplit_once('/').map(|(d, _)| d.to_owned()).unwrap_or_default()
}

/// `dir` joined with `name`, `./` and `../` worked out; None if it leaves the open folder.
pub fn join(dir: &str, name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// The folders from a file's up to the top of the open folder.
fn folders_up(path: &str) -> Vec<String> {
    let mut out = vec![folder_of(path)];
    while let Some(last) = out.last().cloned() {
        if last.is_empty() {
            break;
        }
        out.push(folder_of(&last));
    }
    out
}

// ----- Environments ------------------------------------------------------------------------

/// JSON with `//` and `/* */` comments, and commas before a closing bracket, made plain.
pub fn strip_jsonc(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match (c, chars.get(i + 1)) {
            ('"', _) => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            (',', _) => {
                // A comma before `}` or `]` (with only space between) is dropped.
                let next = chars[i + 1..].iter().find(|c| !c.is_whitespace());
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

pub type Environments = Vec<(String, HashMap<String, String>)>;

/// The environments for a `.http` file: `http-client.env.json` beside it or in a folder above
/// it, with `http-client.private.env.json` over it, then VS Code's
/// `rest-client.environmentVariables` (with its `$shared` in each). By name. `read` reads a
/// file of the open folder.
pub fn environments(http_file: &str, read: &dyn Fn(&str) -> Option<String>) -> Environments {
    let mut envs: Environments = Vec::new();
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
    let json = |p: &str| -> Option<serde_json::Map<String, Value>> {
        serde_json::from_str::<Value>(&strip_jsonc(&read(p)?)).ok()?.as_object().cloned()
    };
    for dir in folders_up(http_file) {
        let at = |name: &str| if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
        let public = json(&at("http-client.env.json"));
        let private = json(&at("http-client.private.env.json"));
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
    }
    if let Some(settings) = json(".vscode/settings.json")
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

/// `NAME=value` lines of the `.env` file beside the `.http` file or at the top of the folder.
pub fn dotenv(http_file: &str, read: &dyn Fn(&str) -> Option<String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let dir = folder_of(http_file);
    let mut files = vec![if dir.is_empty() { ".env".to_owned() } else { format!("{dir}/.env") }];
    if !dir.is_empty() {
        files.push(".env".to_owned());
    }
    for f in files {
        if let Some(text) = read(&f) {
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

/// What the editor tells: the time, random numbers, and environment variables.
pub trait Outside {
    /// Milliseconds since 1970 (UTC).
    fn now_millis(&self) -> i64;
    fn random(&self) -> u64;
    fn env_var(&self, name: &str) -> Result<Option<String>, String>;
}

/// What `{{…}}` can stand for.
pub struct Values<'a> {
    pub file_vars: &'a [(String, String)],
    pub env: Option<&'a HashMap<String, String>>,
    /// Responses of named requests, latest first.
    pub responses: &'a HashMap<String, Response>,
    pub dotenv: HashMap<String, String>,
    pub outside: &'a dyn Outside,
}

impl Values<'_> {
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
                    Ok(resp.body.clone())
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
        let millis = self.outside.now_millis();
        Ok(match head {
            "uuid" | "guid" | "random.uuid" => uuid4(self.outside.random(), self.outside.random()),
            "timestamp" => millis.div_euclid(1000).to_string(),
            "isoTimestamp" => {
                let (y, mo, d, h, mi, s, _) = civil(millis.div_euclid(1000));
                format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}Z", millis.rem_euclid(1000))
            }
            "randomInt" => {
                let lo: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
                let hi: i64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(1000);
                (lo + (self.outside.random() % (hi - lo).max(1) as u64) as i64).to_string()
            }
            "datetime" | "localDatetime" => {
                let (y, mo, d, h, mi, s, wd) = civil(millis.div_euclid(1000));
                match words.next().unwrap_or("iso8601") {
                    "rfc1123" => {
                        const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
                        const MONTHS: [&str; 12] =
                            ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
                        format!("{}, {d:02} {} {y} {h:02}:{mi:02}:{s:02} GMT", DAYS[wd], MONTHS[mo as usize - 1])
                    }
                    _ => format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
                }
            }
            "processEnv" => {
                let var = words.next().ok_or("{{$processEnv NAME}}: which variable?")?;
                let name = var.trim_start_matches('%');
                self.outside.env_var(name)?.ok_or_else(|| format!("{{{{$processEnv {var}}}}}: not set"))?
            }
            "dotenv" => {
                let var = words.next().ok_or("{{$dotenv NAME}}: which variable?")?;
                self.dotenv.get(var).cloned().ok_or_else(|| format!("{{{{$dotenv {var}}}}}: not in .env"))?
            }
            other => return Err(format!("{{{{${other}}}}} isn't a value Coiman knows")),
        })
    }
}

/// UTC date and time of a Unix time: (year, month, day, hour, minute, second, days since a
/// Thursday, so 0 is Thursday).
pub fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32, usize) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = ((rem / 3600) as u32, (rem % 3600 / 60) as u32, (rem % 60) as u32);
    (y, m, d, h, mi, s, days.rem_euclid(7) as usize)
}

fn uuid4(a: u64, b: u64) -> String {
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
    pub body: Option<String>,
    pub no_redirect: bool,
    pub insecure: bool,
    pub timeout: Option<u32>,
}

/// The request with its values filled in; `< file` bodies are read beside the `.http` file.
pub fn prepare(
    req: &Request,
    values: &Values,
    http_file: &str,
    read: &dyn Fn(&str) -> Result<String, String>,
) -> Result<Ready, String> {
    let url = values.fill(&req.url)?;
    let url = if url.contains("://") { url } else { format!("http://{url}") };
    let mut headers = Vec::new();
    for (k, v) in &req.headers {
        headers.push((values.fill(k)?, values.fill(v)?));
    }
    let body = match &req.body {
        None => None,
        Some(Body::Text(t)) => Some(values.fill(t)?),
        Some(Body::File(p)) => {
            let name = values.fill(p)?;
            let path = join(&folder_of(http_file), &name).ok_or(format!("{name} is outside the open folder"))?;
            Some(read(&path).map_err(|e| format!("{path}: {e}"))?)
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
    pub body: String,
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
            || serde_json::from_str::<Value>(&self.body).is_ok_and(|v| v.is_object() || v.is_array())
    }

    /// The body as text to read: JSON indented, the rest as it came.
    pub fn pretty(&self) -> String {
        if self.is_json()
            && let Ok(v) = serde_json::from_str::<Value>(&self.body)
        {
            return serde_json::to_string_pretty(&v).unwrap_or_default();
        }
        self.body.clone()
    }

    /// A value in a JSON body, by a path like `$.data.items[0].id` (or without the `$.`).
    pub fn json_path(&self, path: &str) -> Option<String> {
        let v: Value = serde_json::from_str(&self.body).ok()?;
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
        out.push_str(&format!(" \\\n  --data-binary {}", q(body)));
    }
    if !ready.no_redirect {
        out.push_str(" -L");
    }
    if ready.insecure {
        out.push_str(" -k");
    }
    out
}

/// What separates curl's own report (on stderr) from what it said before.
const MARK: &str = "@@coiman-curl@@";

/// curl's arguments and what goes to its standard input (the body), to send `ready`. The
/// response heads come before the body on standard output (`-i`); timing and the final address
/// come as JSON on standard error.
pub fn curl_args(ready: &Ready) -> (Vec<String>, String) {
    let mut a: Vec<String> = ["-sS", "--compressed", "-i", "-w"].map(str::to_owned).into();
    a.push(format!("%{{stderr}}{MARK}%{{json}}"));
    // `-X HEAD` waits for a body that never comes: `-I` asks for the head only.
    if ready.method == "HEAD" {
        a.push("-I".into());
    } else {
        a.extend(["-X".into(), ready.method.clone()]);
    }
    for (k, v) in &ready.headers {
        a.extend(["-H".into(), format!("{k}: {v}")]);
    }
    if ready.body.is_some() {
        a.extend(["--data-binary".into(), "@-".into()]);
    }
    if !ready.no_redirect {
        a.extend(["-L".into(), "--max-redirs".into(), "10".into()]);
    }
    if ready.insecure {
        a.push("-k".into());
    }
    a.extend(["--max-time".into(), ready.timeout.unwrap_or(60).to_string()]);
    a.extend(["--url".into(), ready.url.clone()]);
    (a, ready.body.clone().unwrap_or_default())
}

/// What curl printed, read: the last head (after redirects and `100 Continue`) and the body.
pub fn read_curl(exit: Option<i32>, stdout: &str, stderr: &str, url: &str) -> Result<Response, String> {
    let (before, report) = stderr.split_once(MARK).unwrap_or((stderr, ""));
    if exit != Some(0) {
        let err = before.lines().find(|l| l.starts_with("curl:")).unwrap_or(before.trim());
        let err = err.trim_start_matches("curl: ").trim();
        return Err(if err.is_empty() { "curl failed".to_owned() } else { err.to_owned() });
    }
    // Each head starts with `HTTP/` and ends with an empty line; the body follows the last.
    let mut rest = stdout;
    let mut head = "";
    while rest.starts_with("HTTP/") {
        let Some(end) = rest.find("\r\n\r\n").map(|i| (i, 4)).or_else(|| rest.find("\n\n").map(|i| (i, 2))) else {
            head = rest;
            rest = "";
            break;
        };
        head = &rest[..end.0];
        rest = &rest[end.0 + end.1..];
    }
    let (status, reason, version, headers) = parse_head(head);
    let info: Value = serde_json::from_str(report.trim()).unwrap_or(Value::Null);
    let status = info["http_code"].as_u64().map_or(status, |c| c as u16);
    Ok(Response {
        status,
        reason: if reason.is_empty() { reason_of(status).to_owned() } else { reason },
        version,
        headers,
        body: rest.to_owned(),
        millis: info["time_total"].as_f64().unwrap_or(0.0) * 1000.0,
        url: info["url_effective"].as_str().unwrap_or(url).to_owned(),
    })
}

/// A head's status line and headers.
fn parse_head(head: &str) -> (u16, String, String, Vec<(String, String)>) {
    let mut lines = head.lines().map(|l| l.trim_end_matches('\r'));
    let status_line = lines.next().unwrap_or("");
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("").to_owned();
    let status = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let reason = parts.next().unwrap_or("").trim().to_owned();
    let headers =
        lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
    (status, reason, version, headers)
}

/// The usual words for a status (HTTP/2 sends none).
fn reason_of(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

// ----- History -----------------------------------------------------------------------------

/// A response kept: what was asked, and what came.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Milliseconds since 1970.
    pub when: i64,
    pub request: String,
    pub method: String,
    pub url: String,
    pub response: Response,
}

/// How many responses a file keeps, how much of each body, and how much all files keep.
pub const KEEP: usize = 30;
pub const KEEP_BODY: usize = 64 * 1024;
pub const KEEP_ALL: usize = 3 * 1024 * 1024;

/// Every file's responses, newest first, as kept in the plugin's storage.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    pub files: std::collections::BTreeMap<String, Vec<Entry>>,
}

impl History {
    pub fn read(text: &str) -> History {
        serde_json::from_str(text).unwrap_or_default()
    }

    pub fn of(&self, file: &str) -> Vec<Entry> {
        self.files.get(file).cloned().unwrap_or_default()
    }

    /// Keeps a response; the oldest go when there are too many, or too much in all.
    pub fn remember(&mut self, file: &str, mut entry: Entry) -> String {
        if entry.response.body.len() > KEEP_BODY {
            let mut cut = KEEP_BODY;
            while !entry.response.body.is_char_boundary(cut) {
                cut -= 1;
            }
            entry.response.body.truncate(cut);
        }
        let list = self.files.entry(file.to_owned()).or_default();
        list.insert(0, entry);
        list.truncate(KEEP);
        loop {
            let text = serde_json::to_string(self).unwrap_or_default();
            if text.len() <= KEEP_ALL {
                return text;
            }
            // The oldest entry of any file goes.
            let oldest =
                self.files.iter().filter_map(|(f, l)| l.last().map(|e| (e.when, f.clone()))).min().map(|(_, f)| f);
            let Some(f) = oldest else { return text };
            if let Some(l) = self.files.get_mut(&f) {
                l.pop();
                if l.is_empty() {
                    self.files.remove(&f);
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "@host = localhost:{{port}}\n@port = 8080\n\n### Log in\n# @name login\nPOST http://{{host}}/login HTTP/1.1\nContent-Type: application/json\n\n{\"user\": \"ann\"}\n\n> {% client.global.set(\"x\", 1) %}\n\n###\n// a comment\nGET {{host}}/orders\n  ?limit=10\n  &sort=new\nAuthorization: Bearer {{login.response.body.$.token}}\n\n### Upload\n# @no-redirect\n# @timeout 5\nPUT http://example.com/up\n\n< ./payload.json\n";

    struct Fixed;
    impl Outside for Fixed {
        fn now_millis(&self) -> i64 {
            // 2026-11-22 (a Sunday) 10:20:30.456 UTC.
            1_795_342_830_456
        }
        fn random(&self) -> u64 {
            42
        }
        fn env_var(&self, name: &str) -> Result<Option<String>, String> {
            Ok((name == "HOME_X").then(|| "/home/x".to_owned()))
        }
    }

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

    fn response(body: &str) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            headers: vec![("Location".into(), "/x".into())],
            body: body.into(),
            millis: 1.0,
            url: String::new(),
        }
    }

    #[test]
    fn fills_values() {
        let f = parse(SAMPLE);
        let mut responses = HashMap::new();
        responses.insert("login".to_owned(), response(r#"{"token": "abc", "items": [{"id": 7}]}"#));
        let env: HashMap<String, String> = [("port".to_owned(), "9000".to_owned())].into();
        let dotenv: HashMap<String, String> = [("KEY".to_owned(), "k1".to_owned())].into();
        let values = Values { file_vars: &f.vars, env: Some(&env), responses: &responses, dotenv, outside: &Fixed };
        // The environment wins over the file; variables refer to others.
        assert_eq!(values.fill("{{host}}/a").unwrap(), "localhost:9000/a");
        assert_eq!(values.fill("{{login.response.body.$.token}}").unwrap(), "abc");
        assert_eq!(values.fill("{{login.response.body.items[0].id}}").unwrap(), "7");
        assert_eq!(values.fill("{{login.response.headers.location}}").unwrap(), "/x");
        assert!(values.fill("{{nope}}").unwrap_err().contains("isn't defined"));
        assert!(values.fill("{{other.response.body.x}}").unwrap_err().contains("send the request named other first"));
        assert_eq!(values.fill("{{$uuid}}").unwrap().len(), 36);
        assert_eq!(values.fill("{{$randomInt 5 6}}").unwrap(), "5");
        assert_eq!(values.fill("{{$timestamp}}").unwrap(), "1795342830");
        assert_eq!(values.fill("{{$isoTimestamp}}").unwrap(), "2026-11-22T10:20:30.456Z");
        assert_eq!(values.fill("{{$datetime rfc1123}}").unwrap(), "Sun, 22 Nov 2026 10:20:30 GMT");
        assert_eq!(values.fill("{{$processEnv HOME_X}}").unwrap(), "/home/x");
        assert!(values.fill("{{$processEnv NOPE}}").unwrap_err().contains("not set"));
        assert_eq!(values.fill("{{$dotenv KEY}}").unwrap(), "k1");
        let read = |p: &str| if p == "api/payload.json" { Ok("{}".to_owned()) } else { Err("no".to_owned()) };
        let ready = prepare(&f.requests[1], &values, "api/x.http", &read).unwrap();
        assert_eq!(ready.url, "http://localhost:9000/orders?limit=10&sort=new");
        assert_eq!(ready.headers[0].1, "Bearer abc");
        assert!(curl_line(&ready).starts_with("curl -X GET 'http://localhost:9000/orders?limit=10&sort=new'"));
        let up = prepare(&f.requests[2], &values, "api/x.http", &read).unwrap();
        assert_eq!(up.body.as_deref(), Some("{}"));
        assert!(prepare(&f.requests[2], &values, "x.http", &read).is_err());
        let looped = File { vars: vec![("a".into(), "{{b}}".into()), ("b".into(), "{{a}}".into())], requests: vec![] };
        let values = Values {
            file_vars: &looped.vars,
            env: None,
            responses: &responses,
            dotenv: HashMap::new(),
            outside: &Fixed,
        };
        assert!(values.fill("{{a}}").unwrap_err().contains("loop"));
    }

    #[test]
    fn curl_output_environments_history_and_comparisons() {
        let out = "HTTP/1.1 301 Moved\r\nLocation: /b\r\n\r\nHTTP/2 200 \r\ncontent-type: application/json\r\nx-a: 1\r\n\r\n{\"a\":1}";
        let err = "@@coiman-curl@@{\"http_code\":200,\"time_total\":0.25,\"url_effective\":\"http://h/b\"}";
        let r = read_curl(Some(0), out, err, "http://h/a").unwrap();
        assert_eq!((r.status, r.reason.as_str(), r.version.as_str()), (200, "OK", "HTTP/2"));
        assert_eq!((r.headers.len(), r.body.as_str(), r.url.as_str()), (2, "{\"a\":1}", "http://h/b"));
        assert_eq!(r.millis, 250.0);
        let e = read_curl(Some(7), "", "curl: (7) Failed to connect to h port 1\n@@coiman-curl@@{}", "x").unwrap_err();
        assert_eq!(e, "(7) Failed to connect to h port 1");
        let ready = Ready {
            method: "POST".into(),
            url: "http://h/x".into(),
            headers: vec![("A".into(), "b".into())],
            body: Some("{}".into()),
            no_redirect: true,
            insecure: false,
            timeout: None,
        };
        let (args, stdin) = curl_args(&ready);
        assert_eq!(stdin, "{}");
        assert!(args.windows(2).any(|w| w == ["--data-binary", "@-"]) && !args.contains(&"-L".to_owned()));
        assert_eq!(args.last().unwrap(), "http://h/x");

        let files: HashMap<&str, &str> = [
            (
                "http-client.env.json",
                r#"{"dev": {"host": "localhost", "token": "public"}, "prod": {"host": "example.com"},}"#,
            ),
            ("http-client.private.env.json", r#"{"dev": {"token": "secret"}} // mine"#),
            ("api/.env", "KEY=\"inner\"\n# no\n"),
            (".env", "KEY=outer\nOTHER=o\n"),
        ]
        .into();
        let read = |p: &str| files.get(p).map(|s| (*s).to_owned());
        let envs = environments("api/a.http", &read);
        let dev = &envs.iter().find(|(n, _)| n == "dev").unwrap().1;
        assert_eq!((dev["host"].as_str(), dev["token"].as_str()), ("localhost", "secret"));
        assert!(envs.iter().any(|(n, _)| n == "prod"));
        let d = dotenv("api/a.http", &read);
        assert_eq!((d["KEY"].as_str(), d["OTHER"].as_str()), ("inner", "o"));

        let mut h = History::default();
        for i in 0..(KEEP + 5) {
            let mut e = Entry {
                when: i as i64,
                request: "r".into(),
                method: "GET".into(),
                url: "u".into(),
                response: response("x"),
            };
            e.response.body = "y".repeat(KEEP_BODY + 10);
            h.remember("a.http", e);
        }
        assert_eq!(h.of("a.http").len(), KEEP);
        assert_eq!(h.of("a.http")[0].response.body.len(), KEEP_BODY);
        assert!(serde_json::to_string(&h).unwrap().len() <= KEEP_ALL);

        let resp = |body: &str, date: &str| Response {
            headers: vec![("Content-Type".into(), "application/json".into()), ("Date".into(), date.into())],
            ..response(body)
        };
        let diff = compare(&resp(r#"{"a":1,"b":2}"#, "x"), &resp(r#"{"a":1,"b":3}"#, "y"));
        // The date differs but isn't shown; one body line changed.
        assert!(diff.iter().all(|(_, l)| !l.contains("date")));
        assert!(diff.contains(&('-', "  \"b\": 2".to_owned())) && diff.contains(&('+', "  \"b\": 3".to_owned())));
        assert_eq!(join("api", "../x/./y.json").as_deref(), Some("x/y.json"));
        assert_eq!(join("", "../y"), None);
    }
}
