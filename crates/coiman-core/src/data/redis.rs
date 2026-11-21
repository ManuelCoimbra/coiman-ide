//! Redis (and servers that speak its protocol: Valkey, Memurai, KeyDB) through its own
//! command-line client, `redis-cli`: the keys, what each holds, and a console. As with the
//! SQL servers, the app opens no connection itself, and a password is never saved: it goes to
//! the client in its environment (`REDISCLI_AUTH`) for that run only.
//!
//! Every command goes to the client as arguments, never through its own line parser, so a
//! value is sent exactly as typed.

use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::language::lsp::discovery::quiet_command;
use crate::language::lsp::servers::which;

/// A server to connect to. Everything but the password is kept for next time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub name: String,
    pub host: String,
    pub port: u16,
    /// The database number (`SELECT n`).
    #[serde(default)]
    pub db: u32,
    /// For an ACL user; empty: the default user.
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub tls: bool,
    #[serde(skip)]
    pub password: Option<String>,
}

impl Default for Server {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: "127.0.0.1".to_owned(),
            port: 6379,
            db: 0,
            user: String::new(),
            tls: false,
            password: None,
        }
    }
}

/// A reply, as `redis-cli --no-raw` prints it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Nil,
    Int(i64),
    /// A bulk string (it came in quotes).
    Text(String),
    /// A status (`OK`, `PONG`, a type name).
    Status(String),
    Error(String),
    Array(Vec<Reply>),
}

impl Reply {
    pub fn text(&self) -> Option<&str> {
        match self {
            Reply::Text(s) | Reply::Status(s) => Some(s),
            _ => None,
        }
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Reply::Int(n) => Some(*n),
            Reply::Text(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Reply] {
        match self {
            Reply::Array(items) => items,
            _ => &[],
        }
    }

    /// As `redis-cli` shows it.
    pub fn show(&self) -> String {
        let mut out = String::new();
        show(self, 0, &mut out);
        out.trim_end().to_owned()
    }
}

fn show(r: &Reply, indent: usize, out: &mut String) {
    match r {
        Reply::Nil => out.push_str("(nil)\n"),
        Reply::Int(n) => out.push_str(&format!("(integer) {n}\n")),
        Reply::Text(s) => out.push_str(&format!("{}\n", quote(s))),
        Reply::Status(s) => out.push_str(&format!("{s}\n")),
        Reply::Error(e) => out.push_str(&format!("(error) {e}\n")),
        Reply::Array(items) if items.is_empty() => out.push_str("(empty array)\n"),
        Reply::Array(items) => {
            let width = items.len().to_string().len();
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(&" ".repeat(indent));
                }
                let label = format!("{:>width$}) ", i + 1);
                out.push_str(&label);
                show(item, indent + label.len(), out);
            }
        }
    }
}

/// A value in quotes, the way the client escapes it.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text inside a quoted value, escapes undone (`\xhh` bytes as UTF-8 where they make it).
fn unquote(s: &str) -> String {
    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('n') => bytes.push(b'\n'),
            Some('r') => bytes.push(b'\r'),
            Some('t') => bytes.push(b'\t'),
            Some('a') => bytes.push(7),
            Some('b') => bytes.push(8),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                bytes.push(u8::from_str_radix(&hex, 16).unwrap_or(b'?'));
            }
            Some(other) => {
                let mut buf = [0u8; 4];
                bytes.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            }
            None => bytes.push(b'\\'),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The replies in `redis-cli --no-raw` output: one per command it was given.
pub fn parse(text: &str) -> Vec<Reply> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        out.push(parse_at(&lines, &mut i, 0));
    }
    out
}

/// One reply starting at `lines[*i]`, after `indent` columns of an enclosing array's labels.
fn parse_at(lines: &[&str], i: &mut usize, indent: usize) -> Reply {
    let line = lines[*i].get(indent..).unwrap_or("");
    // An array: "1) …". Its first item shares the line with the label of the array it is in;
    // the next ones start lines indented to here (the client pads labels to one width).
    if array_label(line).is_some() {
        let mut items = Vec::new();
        while let Some(full) = lines.get(*i) {
            let (head, rest) = (full.get(..indent).unwrap_or(full), full.get(indent..).unwrap_or(""));
            if !items.is_empty() && !head.trim().is_empty() {
                // An item of an outer array.
                break;
            }
            let Some((label, _)) = array_label(rest) else { break };
            items.push(parse_at(lines, i, indent + label.len()));
        }
        return Reply::Array(items);
    }
    *i += 1;
    let s = line.trim_end();
    if s == "(nil)" {
        Reply::Nil
    } else if s == "(empty array)" || s == "(empty list or set)" {
        Reply::Array(Vec::new())
    } else if let Some(n) = s.strip_prefix("(integer) ") {
        Reply::Int(n.trim().parse().unwrap_or(0))
    } else if let Some(e) = s.strip_prefix("(error) ") {
        Reply::Error(e.to_owned())
    } else if s.starts_with('"') {
        Reply::Text(unquote(s))
    } else {
        Reply::Status(s.to_owned())
    }
}

/// "  12) rest": the label with its padding ("  12) ") and the rest.
fn array_label(line: &str) -> Option<(&str, &str)> {
    let digits_start = line.len() - line.trim_start().len();
    let rest = &line[digits_start..];
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || !rest[digits..].starts_with(") ") {
        return None;
    }
    let end = digits_start + digits + 2;
    Some((&line[..end], &line[end..]))
}

/// A key and what the scan says about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub name: String,
    /// `string`, `hash`, `list`, `set`, `zset`, `stream`…
    pub kind: String,
    /// Milliseconds left, or -1 for none.
    pub ttl_ms: i64,
    /// Its length (characters, fields, items), or -1.
    pub size: i64,
}

/// One round of SCAN, with each key's type, time to live and size, in one call.
const SCAN_SCRIPT: &str = "local r = redis.call('SCAN', ARGV[1], 'MATCH', ARGV[2], 'COUNT', ARGV[3]) \
local out = {r[1]} \
for _, k in ipairs(r[2]) do \
  local t = redis.call('TYPE', k)['ok'] \
  local n = -1 \
  if t == 'string' then n = redis.call('STRLEN', k) \
  elseif t == 'hash' then n = redis.call('HLEN', k) \
  elseif t == 'list' then n = redis.call('LLEN', k) \
  elseif t == 'set' then n = redis.call('SCARD', k) \
  elseif t == 'zset' then n = redis.call('ZCARD', k) \
  elseif t == 'stream' then n = redis.call('XLEN', k) end \
  table.insert(out, {k, t, redis.call('PTTL', k), n}) \
end \
return out";

/// How many items of a big value are shown.
pub const SHOWN: usize = 500;

/// The client: on `PATH` (Redis, Valkey or Memurai), or unpacked into the config folder's
/// `tools/redis`.
pub fn client() -> Option<PathBuf> {
    for name in ["redis-cli", "valkey-cli", "memurai-cli"] {
        if let Some(found) = which(name) {
            return Some(found);
        }
    }
    let exe = if cfg!(windows) { "redis-cli.exe" } else { "redis-cli" };
    let bundled = crate::settings::config_dir()?.join("tools").join("redis").join(exe);
    bundled.is_file().then_some(bundled)
}

pub const MISSING: &str = "The redis-cli program isn't on this computer. Install Redis's tools (or Valkey's, \
                           or Memurai on Windows), or unpack a Windows build into the config folder's \
                           tools/redis, and try again.";

impl Server {
    pub fn label(&self) -> String {
        let scheme = if self.tls { "rediss" } else { "redis" };
        let user = if self.user.is_empty() { String::new() } else { format!("{}@", self.user) };
        format!("{scheme}://{user}{}:{}/{}", self.host, self.port, self.db)
    }

    pub fn title(&self) -> String {
        if self.name.trim().is_empty() { self.label() } else { self.name.trim().to_owned() }
    }

    /// Runs one command (its words, as typed) and gives back the reply.
    pub fn run(&self, args: &[String]) -> Result<Reply, String> {
        let program = client().ok_or(MISSING)?;
        let mut cmd = quiet_command(&program);
        cmd.args(["--no-raw", "-h", &self.host, "-p", &self.port.to_string(), "-n", &self.db.to_string()]);
        if !self.user.is_empty() {
            cmd.args(["--user", &self.user]);
        }
        if self.tls {
            cmd.arg("--tls");
        }
        if let Some(pw) = &self.password {
            cmd.env("REDISCLI_AUTH", pw);
        }
        let out = cmd
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("Couldn't start {}: {e}", program.display()))?;
        let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
        let err = String::from_utf8_lossy(&out.stderr);
        // Its warning about a password given on the command line isn't one here.
        let err: Vec<&str> = err.lines().filter(|l| !l.starts_with("Warning:")).collect();
        if !out.status.success() || (text.trim().is_empty() && !err.is_empty()) {
            let err = err.join("\n");
            return Err(if err.trim().is_empty() { format!("redis-cli stopped ({})", out.status) } else { err });
        }
        // An error reply, and Redis's NOAUTH, come on standard output.
        let mut replies = parse(&text);
        // One command, several replies: text the client printed as it is (INFO, CLIENT LIST).
        let reply =
            if replies.len() > 1 { Reply::Status(text.trim().to_owned()) } else { replies.pop().unwrap_or(Reply::Nil) };
        match reply {
            Reply::Error(e) if e.starts_with("NOAUTH") || e.starts_with("WRONGPASS") => {
                Err(format!("{e} (give the password in the connection)"))
            }
            r => Ok(r),
        }
    }

    fn run_words(&self, words: &[&str]) -> Result<Reply, String> {
        self.run(&words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>())
    }

    /// One round of SCAN for keys matching `pattern`: the cursor to go on from ("0" when it's
    /// done) and the keys.
    pub fn scan(&self, cursor: &str, pattern: &str, count: usize) -> Result<(String, Vec<Key>), String> {
        let pattern = if pattern.trim().is_empty() { "*" } else { pattern.trim() };
        let reply = self.run_words(&["EVAL", SCAN_SCRIPT, "0", cursor, pattern, &count.to_string()])?;
        if let Reply::Error(e) = &reply {
            return Err(e.clone());
        }
        let items = reply.items();
        let next = items.first().and_then(Reply::text).unwrap_or("0").to_owned();
        let keys = items[1.min(items.len())..]
            .iter()
            .filter_map(|k| {
                let f = k.items();
                Some(Key {
                    name: f.first()?.text()?.to_owned(),
                    kind: f.get(1)?.text()?.to_owned(),
                    ttl_ms: f.get(2)?.int()?,
                    size: f.get(3)?.int()?,
                })
            })
            .collect();
        Ok((next, keys))
    }

    /// What a key holds, as rows of up to three cells (field/value, index/item, member/score…)
    /// with their headers.
    pub fn value(&self, key: &Key) -> Result<(Vec<&'static str>, Vec<Vec<String>>), String> {
        let shown = SHOWN.to_string();
        let last = (SHOWN - 1).to_string();
        let k = key.name.as_str();
        let reply = match key.kind.as_str() {
            "string" => self.run_words(&["GET", k])?,
            "hash" => self.run_words(&["HSCAN", k, "0", "COUNT", &shown])?,
            "list" => self.run_words(&["LRANGE", k, "0", &last])?,
            "set" => self.run_words(&["SSCAN", k, "0", "COUNT", &shown])?,
            "zset" => self.run_words(&["ZRANGE", k, "0", &last, "WITHSCORES"])?,
            "stream" => self.run_words(&["XREVRANGE", k, "+", "-", "COUNT", "100"])?,
            other => return Err(format!("Coiman doesn't show {other} values; the console can ask for them.")),
        };
        if let Reply::Error(e) = &reply {
            return Err(e.clone());
        }
        let text = |r: &Reply| r.text().map_or_else(|| r.show(), str::to_owned);
        Ok(match key.kind.as_str() {
            "string" => (vec!["Value"], vec![vec![text(&reply)]]),
            "hash" => {
                let pairs = reply.items().get(1).map(Reply::items).unwrap_or(&[]);
                (vec!["Field", "Value"], pairs.chunks(2).map(|p| p.iter().map(text).collect()).collect())
            }
            "list" => (
                vec!["Index", "Item"],
                reply.items().iter().enumerate().map(|(i, r)| vec![i.to_string(), text(r)]).collect(),
            ),
            "set" => {
                let members = reply.items().get(1).map(Reply::items).unwrap_or(&[]);
                (vec!["Member"], members.iter().map(|r| vec![text(r)]).collect())
            }
            "zset" => {
                (vec!["Member", "Score"], reply.items().chunks(2).map(|p| p.iter().map(text).collect()).collect())
            }
            _ => {
                // A stream entry: its id and its fields.
                let rows = reply
                    .items()
                    .iter()
                    .map(|e| {
                        let id = e.items().first().map(text).unwrap_or_default();
                        let fields = e.items().get(1).map(Reply::items).unwrap_or(&[]);
                        let shown: Vec<String> = fields
                            .chunks(2)
                            .map(|p| format!("{}={}", text(&p[0]), p.get(1).map(text).unwrap_or_default()))
                            .collect();
                        vec![id, shown.join("  ")]
                    })
                    .collect();
                (vec!["Entry", "Fields"], rows)
            }
        })
    }

    /// How many keys each database holds (`INFO keyspace`).
    pub fn keyspace(&self) -> Result<Vec<(u32, u64)>, String> {
        let reply = self.run_words(&["INFO", "keyspace"])?;
        let text = reply.text().unwrap_or_default().to_owned();
        Ok(text
            .lines()
            .filter_map(|l| {
                let (db, rest) = l.trim().strip_prefix("db")?.split_once(':')?;
                let keys = rest.split(',').find_map(|p| p.strip_prefix("keys="))?;
                Some((db.parse().ok()?, keys.parse().ok()?))
            })
            .collect())
    }
}

/// A command line typed in the console, as words: quotes group (`SET k "a b"`), and inside
/// double quotes the client's escapes work (`\n`, `\"`, `\x41`).
pub fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else { break };
        let mut word = String::new();
        if first == '"' || first == '\'' {
            chars.next();
            let mut closed = false;
            let mut raw = String::new();
            while let Some(c) = chars.next() {
                if c == first {
                    closed = true;
                    break;
                }
                if c == '\\' && first == '"' {
                    raw.push(c);
                    if let Some(n) = chars.next() {
                        raw.push(n);
                    }
                    continue;
                }
                raw.push(c);
            }
            if !closed {
                return Err("A quote isn't closed".to_owned());
            }
            word = if first == '"' { unquote(&format!("\"{raw}\"")) } else { raw };
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
        }
        words.push(word);
    }
    Ok(words)
}

/// A time to live as people read it: `42 s`, `3 min`, `2 h`, `5 d`; none for -1.
pub fn human_ttl(ms: i64) -> String {
    match ms {
        n if n < 0 => String::new(),
        n if n < 1000 => format!("{n} ms"),
        n if n < 120_000 => format!("{} s", n / 1000),
        n if n < 7_200_000 => format!("{} min", n / 60_000),
        n if n < 172_800_000 => format!("{} h", n / 3_600_000),
        n => format!("{} d", n / 86_400_000),
    }
}

/// Connections used before, newest first, without passwords: `<config>/redis_connections.toml`.
pub fn load_saved() -> Vec<Server> {
    #[derive(Deserialize)]
    struct File {
        #[serde(default)]
        server: Vec<Server>,
    }
    let Some(path) = crate::settings::config_dir().map(|d| d.join("redis_connections.toml")) else {
        return Vec::new();
    };
    std::fs::read_to_string(path).ok().and_then(|t| toml::from_str::<File>(&t).ok()).map_or_else(Vec::new, |f| f.server)
}

pub fn remember(saved: &mut Vec<Server>, server: &Server) -> Result<(), String> {
    #[derive(Serialize)]
    struct File<'a> {
        server: &'a [Server],
    }
    let mut kept = server.clone();
    kept.password = None;
    saved.retain(|s| s.label() != kept.label());
    saved.insert(0, kept);
    saved.truncate(12);
    let dir = crate::settings::config_dir().ok_or("no config folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let text = toml::to_string(&File { server: saved }).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("redis_connections.toml"), text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let text = "1) \"0\"\n2) 1) \"events\"\n   2) \"tags\"\n(nil)\n(integer) -1\nOK\n(error) ERR unknown command\n\"a\\\"b\\nc\\x41\"\n(empty array)\n";
        let r = parse(text);
        assert_eq!(r.len(), 7, "{r:?}");
        assert_eq!(
            r[0],
            Reply::Array(vec![
                Reply::Text("0".into()),
                Reply::Array(vec![Reply::Text("events".into()), Reply::Text("tags".into())])
            ])
        );
        assert_eq!(
            (r[1].clone(), r[2].clone(), r[3].clone()),
            (Reply::Nil, Reply::Int(-1), Reply::Status("OK".into()))
        );
        assert_eq!(r[4], Reply::Error("ERR unknown command".into()));
        assert_eq!(r[5], Reply::Text("a\"b\ncA".into()));
        assert_eq!(r[6], Reply::Array(vec![]));
        // A stream: entries of [id, [field, value…]], two levels down; ten items and more widen
        // the labels.
        let stream =
            "1) 1) \"1-0\"\n   2) 1) \"kind\"\n      2) \"click\"\n2) 1) \"2-0\"\n   2) 1) \"x\"\n      2) \"3\"\n";
        let r = parse(stream);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].items().len(), 2);
        assert_eq!(r[0].items()[1].items()[1].items()[1], Reply::Text("3".into()));
        let many: String = (1..=11).map(|i| format!("{i:>2}) \"{i}\"\n")).collect();
        assert_eq!(parse(&many)[0].items().len(), 11);
        // Shown again as the client shows it.
        assert_eq!(parse(stream)[0].show(), stream.trim_end());
    }

    #[test]
    fn words_and_times() {
        assert_eq!(split_words(r#"SET k "a b\n" 'c d'"#).unwrap(), ["SET", "k", "a b\n", "c d"]);
        assert!(split_words("GET \"open").is_err());
        assert_eq!(human_ttl(-1), "");
        assert_eq!(human_ttl(3_600_000), "60 min");
        assert_eq!(human_ttl(86_400_000 * 3), "3 d");
        assert_eq!(quote("a\"b\n"), "\"a\\\"b\\n\"");
    }

    /// Against a real server when `COIMAN_TEST_REDIS` is `host:port` (nothing is left there).
    #[test]
    fn real_server() {
        let Ok(at) = std::env::var("COIMAN_TEST_REDIS") else { return };
        let (host, port) = at.split_once(':').unwrap();
        let s = Server { host: host.into(), port: port.parse().unwrap(), db: 9, ..Server::default() };
        let w = |a: &[&str]| s.run_words(a).unwrap();
        w(&["FLUSHDB"]);
        w(&["SET", "coiman:s", "a \"quoted\"\nvalue"]);
        w(&["HSET", "coiman:h", "f1", "1", "f2", "two"]);
        w(&["RPUSH", "coiman:l", "x", "y"]);
        w(&["ZADD", "coiman:z", "2", "b", "1", "a"]);
        w(&["EXPIRE", "coiman:s", "100"]);
        let (next, keys) = s.scan("0", "coiman:*", 1000).unwrap();
        assert_eq!(next, "0");
        assert_eq!(keys.len(), 4, "{keys:?}");
        let k = |n: &str| keys.iter().find(|k| k.name == n).unwrap().clone();
        assert_eq!((k("coiman:h").kind.as_str(), k("coiman:h").size), ("hash", 2));
        assert!(k("coiman:s").ttl_ms > 90_000 && k("coiman:l").ttl_ms == -1);
        assert_eq!(s.value(&k("coiman:s")).unwrap().1, vec![vec!["a \"quoted\"\nvalue".to_owned()]]);
        assert_eq!(
            s.value(&k("coiman:z")).unwrap().1,
            vec![vec!["a".to_owned(), "1".into()], vec!["b".into(), "2".into()]]
        );
        assert_eq!(s.value(&k("coiman:h")).unwrap().1.len(), 2);
        assert!(s.keyspace().unwrap().iter().any(|(db, n)| *db == 9 && *n == 4));
        assert!(matches!(w(&["NOSUCH"]), Reply::Error(_)));
        w(&["FLUSHDB"]);
    }
}
