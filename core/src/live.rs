//! Live sessions: code sent to a program that's already running, and a look at
//! what's inside it. A [`Session`] is the same whichever system is behind it. It
//! never makes the caller wait: requests go out at once and answers come back
//! through [`Session::poll`]. nREPL is the first adapter.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// A message larger than this is someone else's protocol.
const MAX_MESSAGE: usize = 256 * 1024 * 1024;

/// What a session says about itself after connecting.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    /// Such as "babashka 1.12".
    pub system: String,
    /// What the server can do, as it names them.
    pub capabilities: Vec<String>,
}

impl Identity {
    pub fn can(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|c| c == capability)
    }
}

/// Code to run, and where it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Eval {
    pub code: String,
    pub file: Option<PathBuf>,
    /// From 1.
    pub line: Option<u32>,
    pub namespace: Option<String>,
}

/// One row of an inspector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectRow {
    pub name: String,
    pub value: String,
    pub type_name: String,
    /// What to ask for its children, if it has any.
    pub children: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveEvent {
    Connected(Identity),
    /// Something the code printed.
    Output {
        id: u64,
        text: String,
        error: bool,
    },
    /// The code's value.
    Value {
        id: u64,
        text: String,
    },
    /// It failed, or the server wouldn't take it.
    Failed {
        id: u64,
        text: String,
    },
    /// That's all for this request.
    Done {
        id: u64,
        namespace: Option<String>,
    },
    /// Rows for an inspector request.
    Rows {
        id: u64,
        rows: Vec<InspectRow>,
        problem: Option<String>,
    },
    Closed(String),
}

/// A running system to talk to.
pub trait Session: Send {
    fn name(&self) -> &str;
    /// Runs code. Returns the request's id.
    fn eval(&mut self, request: Eval) -> u64;
    /// Asks the running code to stop, if the server can.
    fn interrupt(&mut self);
    /// Asks for rows: the top level, or the children a row named.
    fn inspect(&mut self, query: Option<&str>, namespace: Option<&str>) -> u64;
    /// What's come in since last time.
    fn poll(&mut self) -> Vec<LiveEvent>;
}

// ----- bencode ---------------------------------------------------------------------

/// nREPL's wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bencode {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<Bencode>),
    Dict(BTreeMap<String, Bencode>),
}

impl Bencode {
    pub fn text(s: &str) -> Self {
        Self::Bytes(s.as_bytes().to_vec())
    }

    pub fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(n) => out.extend(format!("i{n}e").bytes()),
            Self::Bytes(b) => {
                out.extend(b.len().to_string().bytes());
                out.push(b':');
                out.extend(b);
            }
            Self::List(items) => {
                out.push(b'l');
                items.iter().for_each(|i| i.encode(out));
                out.push(b'e');
            }
            Self::Dict(map) => {
                out.push(b'd');
                for (key, value) in map {
                    Self::text(key).encode(out);
                    value.encode(out);
                }
                out.push(b'e');
            }
        }
    }

    /// Reads one value. `None` at a clean end of the stream, or on what isn't bencode.
    pub fn decode(reader: &mut impl BufRead) -> Option<Self> {
        let first = next_byte(reader)?;
        Self::decode_from(first, reader, 0)
    }

    fn decode_from(first: u8, reader: &mut impl BufRead, depth: usize) -> Option<Self> {
        if depth > 64 {
            return None;
        }
        match first {
            b'i' => {
                let mut digits = Vec::new();
                loop {
                    match next_byte(reader)? {
                        b'e' => break,
                        d => digits.push(d),
                    }
                }
                std::str::from_utf8(&digits).ok()?.parse().ok().map(Self::Int)
            }
            b'l' => {
                let mut items = Vec::new();
                loop {
                    match next_byte(reader)? {
                        b'e' => return Some(Self::List(items)),
                        byte => items.push(Self::decode_from(byte, reader, depth + 1)?),
                    }
                }
            }
            b'd' => {
                let mut map = BTreeMap::new();
                loop {
                    match next_byte(reader)? {
                        b'e' => return Some(Self::Dict(map)),
                        byte => {
                            let Self::Bytes(key) = Self::decode_from(byte, reader, depth + 1)? else { return None };
                            let first = next_byte(reader)?;
                            let value = Self::decode_from(first, reader, depth + 1)?;
                            map.insert(String::from_utf8_lossy(&key).into_owned(), value);
                        }
                    }
                }
            }
            d if d.is_ascii_digit() => {
                let mut length = usize::from(d - b'0');
                loop {
                    match next_byte(reader)? {
                        b':' => break,
                        d if d.is_ascii_digit() => {
                            length = length.checked_mul(10)?.checked_add(usize::from(d - b'0'))?;
                            if length > MAX_MESSAGE {
                                return None;
                            }
                        }
                        _ => return None,
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).ok()?;
                Some(Self::Bytes(bytes))
            }
            _ => None,
        }
    }

    fn as_text(&self) -> Option<String> {
        match self {
            Self::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
}

fn next_byte(reader: &mut impl BufRead) -> Option<u8> {
    let buffer = reader.fill_buf().ok()?;
    let byte = *buffer.first()?;
    reader.consume(1);
    Some(byte)
}

/// A message to send, from its fields.
fn message(fields: &[(&str, Bencode)]) -> Vec<u8> {
    let map = fields.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect();
    let mut out = Vec::new();
    Bencode::Dict(map).encode(&mut out);
    out
}

// ----- nREPL -----------------------------------------------------------------------

/// The `host:port` a project's nREPL left behind, if any.
pub fn nrepl_port_file(root: &Path) -> Option<String> {
    let places = [root.join(".nrepl-port"), root.join(".shadow-cljs").join("nrepl.port")];
    let text = places.iter().find_map(|p| std::fs::read_to_string(p).ok())?;
    let port: u16 = text.trim().parse().ok()?;
    Some(format!("127.0.0.1:{port}"))
}

/// What a request was for, to know how to read its answers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Clone,
    ToolClone,
    Describe,
    Eval,
    Inspect,
    Other,
}

/// Escapes text as a Clojure string.
fn clojure_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Prints rows, a line each, tab separated: name, what asks for its children, type, value.
const SHOW: &str = r#"(fn [n e x]
  (let [one (fn [s] (clojure.string/replace (str s) #"[\t\r\n]+" " "))
        open? (or (map? x) (sequential? x) (set? x))
        text (binding [*print-length* 20 *print-level* 3] (pr-str x))
        text (if (> (count text) 300) (str (subs text 0 300) "...") text)]
    (println (str "@@row@@\t" (one n) "\t" (if open? (one e) "") "\t" (one (type x)) "\t" (one text)))))"#;

/// Asks for a namespace's public values.
pub fn clojure_top_level() -> String {
    format!(
        r#"(let [show {SHOW}]
  (doseq [[k v] (sort-by key (ns-publics *ns*))]
    (when-not (:macro (meta v))
      (show (str k)
            (str "(deref (var " (ns-name *ns*) "/" k "))")
            (try (deref v) (catch Exception e (str "unbound: " (.getMessage e))))))))"#
    )
}

/// Asks for what's inside a value, named by the expression that gives it.
pub fn clojure_children(expression: &str) -> String {
    let quoted = clojure_string(expression);
    format!(
        r#"(let [show {SHOW}
      x {expression}
      e {quoted}]
  (cond
    (map? x) (doseq [[k v] (take 200 x)]
               (show (pr-str k) (str "(get " e " " (pr-str k) ")") v))
    (or (sequential? x) (set? x)) (doseq [[i v] (map-indexed vector (take 200 x))]
               (show (str i) (str "(nth (seq " e ") " i ")") v))))"#
    )
}

/// Reads rows back from what the queries printed.
pub fn parse_rows(printed: &str) -> Vec<InspectRow> {
    printed
        .lines()
        .filter_map(|line| {
            let mut fields = line.strip_prefix("@@row@@\t")?.splitn(4, '\t');
            let (name, children, type_name, value) = (fields.next()?, fields.next()?, fields.next()?, fields.next()?);
            Some(InspectRow {
                name: name.to_owned(),
                value: value.to_owned(),
                type_name: type_name.to_owned(),
                children: (!children.is_empty()).then(|| children.to_owned()),
            })
        })
        .collect()
}

/// The top-level form around a line: from the nearest line above that starts a form
/// in its first column, up to where its brackets close. As (first line, last line).
pub fn top_level_form(text: &str, line: usize) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let opens = |l: &str| l.starts_with(['(', '[', '{', '#']);
    let start = (0..=line.min(lines.len().checked_sub(1)?)).rev().find(|i| opens(lines[*i]))?;
    let mut depth = 0i32;
    let mut in_string = false;
    for (at, content) in lines.iter().enumerate().skip(start) {
        let mut chars = content.chars();
        while let Some(c) = chars.next() {
            match (c, in_string) {
                ('\\', _) => {
                    chars.next();
                }
                ('"', _) => in_string = !in_string,
                (';', false) => break,
                ('(' | '[' | '{', false) => depth += 1,
                (')' | ']' | '}', false) => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 && !in_string {
            return (at >= line).then_some((start, at));
        }
    }
    None
}

/// Whether code makes its own namespace, so can't be sent into it.
pub fn defines_namespace(code: &str) -> bool {
    let code = code.trim_start();
    ["(ns", "(in-ns"]
        .iter()
        .any(|form| code.strip_prefix(form).is_some_and(|rest| rest.starts_with(char::is_whitespace)))
}

/// What a file's `(ns name)` says.
pub fn namespace_of(text: &str) -> Option<String> {
    let (at, _) = text.match_indices("(ns").find(|(at, _)| text[at + 3..].starts_with(char::is_whitespace))?;
    let name: String = text[at + 3..]
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ')' | '(' | '[' | '"'))
        .collect();
    (!name.is_empty()).then_some(name)
}

/// A request's fields, before it has an id.
type Fields = Vec<(String, Bencode)>;

/// A session with an nREPL server.
pub struct Nrepl {
    stream: TcpStream,
    from_server: Receiver<Option<BTreeMap<String, Bencode>>>,
    next_id: u64,
    session: Option<String>,
    /// Inspecting doesn't touch `*1` and friends in the session in use.
    tool_session: Option<String>,
    pending: HashMap<String, (Purpose, u64)>,
    printed: HashMap<String, String>,
    /// Requests that came before the session was ready.
    waiting: Vec<(Purpose, u64, Fields)>,
    current: Option<String>,
    interruptible: bool,
    closed: bool,
    /// The namespace the running request last reported.
    last_namespace: Option<String>,
}

impl Nrepl {
    /// Connects to a server on this computer, and starts the handshake.
    pub fn connect(address: &str, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        if !crate::lsp::servers::is_local_address(address) {
            return Err(format!("{address} isn't on this computer"));
        }
        let target = address.to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no such address")?;
        let stream = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|e| format!("{address}: {e}"))?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        let (tx, from_server) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Some(value) = Bencode::decode(&mut reader) {
                if let Bencode::Dict(map) = value
                    && tx.send(Some(map)).is_err()
                {
                    return;
                }
                wake();
            }
            let _ = tx.send(None);
            wake();
        });
        let mut session = Self {
            stream,
            from_server,
            next_id: 1,
            session: None,
            tool_session: None,
            pending: HashMap::new(),
            printed: HashMap::new(),
            waiting: Vec::new(),
            current: None,
            interruptible: false,
            closed: false,
            last_namespace: None,
        };
        session.send_now(Purpose::Clone, 0, &[("op", Bencode::text("clone"))]);
        session.send_now(Purpose::ToolClone, 0, &[("op", Bencode::text("clone"))]);
        session.send_now(Purpose::Describe, 0, &[("op", Bencode::text("describe"))]);
        Ok(session)
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn send_now(&mut self, purpose: Purpose, id: u64, fields: &[(&str, Bencode)]) {
        let wire_id = format!("rc-{}", self.new_id());
        self.pending.insert(wire_id.clone(), (purpose, id));
        let mut all: Vec<(&str, Bencode)> = fields.to_vec();
        all.push(("id", Bencode::text(&wire_id)));
        let session = if purpose == Purpose::Inspect { &self.tool_session } else { &self.session };
        if let Some(session) = session
            && !matches!(purpose, Purpose::Clone | Purpose::ToolClone | Purpose::Describe)
        {
            all.push(("session", Bencode::text(session)));
        }
        let _ = self.stream.write_all(&message(&all));
    }

    fn send(&mut self, purpose: Purpose, id: u64, fields: Fields) {
        if self.session.is_none() || self.tool_session.is_none() {
            self.waiting.push((purpose, id, fields));
            return;
        }
        let borrowed: Vec<(&str, Bencode)> = fields.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        self.send_now(purpose, id, &borrowed);
        if purpose == Purpose::Eval {
            self.current = Some(format!("rc-{}", self.next_id));
        }
    }

    fn flush_waiting(&mut self) {
        if self.session.is_some() && self.tool_session.is_some() {
            for (purpose, id, fields) in std::mem::take(&mut self.waiting) {
                self.send(purpose, id, fields);
            }
        }
    }

    fn identify(map: &BTreeMap<String, Bencode>) -> Identity {
        let mut parts = Vec::new();
        if let Some(Bencode::Dict(versions)) = map.get("versions") {
            for (name, info) in versions {
                if let Bencode::Dict(info) = info
                    && let Some(text) = info.get("version-string").and_then(Bencode::as_text)
                {
                    parts.push(format!("{name} {text}"));
                }
            }
        }
        let capabilities = match map.get("ops") {
            Some(Bencode::Dict(ops)) => ops.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let system = if parts.is_empty() { "nREPL".to_owned() } else { parts.join(", ") };
        Identity { system, capabilities }
    }

    fn handle(&mut self, map: &BTreeMap<String, Bencode>, events: &mut Vec<LiveEvent>) {
        let wire_id = map.get("id").and_then(Bencode::as_text).unwrap_or_default();
        let Some(&(purpose, id)) = self.pending.get(&wire_id) else { return };
        let statuses: Vec<String> = match map.get("status") {
            Some(Bencode::List(list)) => list.iter().filter_map(Bencode::as_text).collect(),
            _ => Vec::new(),
        };
        let done = statuses.iter().any(|s| s == "done");
        match purpose {
            Purpose::Clone => {
                self.session = map.get("new-session").and_then(Bencode::as_text);
                self.flush_waiting();
            }
            Purpose::ToolClone => {
                self.tool_session = map.get("new-session").and_then(Bencode::as_text);
                self.flush_waiting();
            }
            Purpose::Describe => {
                if map.contains_key("ops") {
                    let identity = Self::identify(map);
                    self.interruptible = identity.can("interrupt");
                    events.push(LiveEvent::Connected(identity));
                }
            }
            Purpose::Eval => {
                if let Some(text) = map.get("out").and_then(Bencode::as_text) {
                    events.push(LiveEvent::Output { id, text, error: false });
                }
                if let Some(text) = map.get("err").and_then(Bencode::as_text) {
                    events.push(LiveEvent::Output { id, text, error: true });
                }
                if let Some(namespace) = map.get("ns").and_then(Bencode::as_text) {
                    self.last_namespace = Some(namespace);
                }
                if let Some(text) = map.get("value").and_then(Bencode::as_text) {
                    events.push(LiveEvent::Value { id, text });
                }
                if let Some(text) = map.get("ex").and_then(Bencode::as_text) {
                    events.push(LiveEvent::Failed { id, text });
                }
                if statuses.iter().any(|s| s == "interrupted") {
                    events.push(LiveEvent::Failed { id, text: "Interrupted".to_owned() });
                }
                if statuses.iter().any(|s| s == "namespace-not-found") {
                    events.push(LiveEvent::Failed {
                        id,
                        text: "That namespace isn't loaded yet: run Live: Load File first".to_owned(),
                    });
                }
                if done {
                    let namespace = self.last_namespace.take();
                    events.push(LiveEvent::Done { id, namespace });
                }
            }
            Purpose::Inspect => {
                let printed = self.printed.entry(wire_id.clone()).or_default();
                if let Some(text) = map.get("out").and_then(Bencode::as_text) {
                    printed.push_str(&text);
                }
                let problem = map.get("err").and_then(Bencode::as_text).filter(|t| !t.trim().is_empty());
                if let Some(problem) = problem {
                    events.push(LiveEvent::Rows { id, rows: Vec::new(), problem: Some(problem) });
                }
                if done {
                    let printed = self.printed.remove(&wire_id).unwrap_or_default();
                    events.push(LiveEvent::Rows { id, rows: parse_rows(&printed), problem: None });
                }
            }
            Purpose::Other => {}
        }
        if done || matches!(purpose, Purpose::Clone | Purpose::ToolClone) {
            self.pending.remove(&wire_id);
            if self.current.as_deref() == Some(&wire_id) {
                self.current = None;
            }
        }
    }
}

impl Session for Nrepl {
    fn name(&self) -> &str {
        "nREPL"
    }

    fn eval(&mut self, request: Eval) -> u64 {
        let id = self.new_id();
        let mut fields =
            vec![("op".to_owned(), Bencode::text("eval")), ("code".to_owned(), Bencode::text(&request.code))];
        if let Some(file) = &request.file {
            fields.push(("file".to_owned(), Bencode::text(&file.to_string_lossy())));
        }
        if let Some(line) = request.line {
            fields.push(("line".to_owned(), Bencode::Int(i64::from(line))));
        }
        if let Some(namespace) = &request.namespace {
            fields.push(("ns".to_owned(), Bencode::text(namespace)));
        }
        self.send(Purpose::Eval, id, fields);
        id
    }

    fn interrupt(&mut self) {
        if !self.interruptible {
            return;
        }
        let Some(target) = self.current.clone() else { return };
        let fields = [("op", Bencode::text("interrupt")), ("interrupt-id", Bencode::text(&target))];
        self.send_now(Purpose::Other, 0, &fields);
    }

    fn inspect(&mut self, query: Option<&str>, namespace: Option<&str>) -> u64 {
        let id = self.new_id();
        let code = query.map_or_else(clojure_top_level, clojure_children);
        let mut fields = vec![("op".to_owned(), Bencode::text("eval")), ("code".to_owned(), Bencode::text(&code))];
        if let Some(namespace) = namespace {
            fields.push(("ns".to_owned(), Bencode::text(namespace)));
        }
        self.send(Purpose::Inspect, id, fields);
        id
    }

    fn poll(&mut self) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        while let Ok(item) = self.from_server.try_recv() {
            match item {
                Some(map) => self.handle(&map, &mut events),
                None => {
                    if !self.closed {
                        self.closed = true;
                        events.push(LiveEvent::Closed("The connection ended".to_owned()));
                    }
                }
            }
        }
        events
    }
}

impl Drop for Nrepl {
    fn drop(&mut self) {
        // Ours to close: the server and what it's running carry on.
        for session in [self.session.take(), self.tool_session.take()].into_iter().flatten() {
            let fields = [("op", Bencode::text("close")), ("session", Bencode::text(&session))];
            let _ = self.stream.write_all(&message(&fields));
        }
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn round_trip(value: &Bencode) -> Option<Bencode> {
        let mut out = Vec::new();
        value.encode(&mut out);
        Bencode::decode(&mut BufReader::new(&out[..]))
    }

    #[test]
    fn bencode_round_trips() {
        let value = Bencode::Dict(
            [
                ("a".to_owned(), Bencode::Int(-3)),
                ("b".to_owned(), Bencode::List(vec![Bencode::text("x"), Bencode::text("é")])),
            ]
            .into(),
        );
        assert_eq!(round_trip(&value), Some(value));
    }

    #[test]
    fn bencode_writes_the_wire_form() {
        let mut out = Vec::new();
        Bencode::Dict([("op".to_owned(), Bencode::text("clone"))].into()).encode(&mut out);
        assert_eq!(out, b"d2:op5:clonee");
    }

    #[test]
    fn bencode_refuses_nonsense() {
        let decode = |bytes: &[u8]| Bencode::decode(&mut BufReader::new(bytes));
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"x"), None);
        assert_eq!(decode(b"i1"), None);
        assert_eq!(decode(b"5:ab"), None, "cut short");
        assert_eq!(decode(b"99999999999999:a"), None, "too long");
        assert_eq!(decode(&b"l".repeat(500)), None, "too deep");
        assert_eq!(decode(b"di1ei2ee"), None, "a key must be a string");
    }

    #[test]
    fn finds_the_port_a_server_left() {
        let root = std::env::temp_dir().join(format!("rc-live-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(nrepl_port_file(&root), None);
        std::fs::write(root.join(".nrepl-port"), "1667\n").unwrap();
        assert_eq!(nrepl_port_file(&root).as_deref(), Some("127.0.0.1:1667"));
        std::fs::write(root.join(".nrepl-port"), "nope").unwrap();
        assert_eq!(nrepl_port_file(&root), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_top_level_forms() {
        let text = "(ns a)\n\n(defn f [x]\n  ;; a ) comment\n  (str \"a ) b\"\n       x))\n\n(def y 1)\n";
        assert_eq!(top_level_form(text, 0), Some((0, 0)));
        assert_eq!(top_level_form(text, 2), Some((2, 5)));
        assert_eq!(top_level_form(text, 4), Some((2, 5)));
        assert_eq!(top_level_form(text, 5), Some((2, 5)));
        assert_eq!(top_level_form(text, 7), Some((7, 7)));
        assert_eq!(top_level_form(text, 6), None, "between forms");
        assert_eq!(top_level_form("  indented", 0), None);
        assert_eq!(top_level_form("(never closed", 0), None);
        assert_eq!(top_level_form("", 0), None);
    }

    #[test]
    fn reads_namespaces() {
        assert_eq!(namespace_of("(ns my.app\n  (:require x))").as_deref(), Some("my.app"));
        assert_eq!(namespace_of("(ns\n  other)").as_deref(), Some("other"));
        assert!(defines_namespace("  (ns a)"));
        assert!(defines_namespace("(in-ns 'a)"));
        assert!(!defines_namespace("(nsfoo)"));
        assert_eq!(namespace_of("(def x 1)"), None);
    }

    #[test]
    fn reads_rows_and_skips_what_else_was_printed() {
        let rows = parse_rows(
            "noise\n@@row@@\tm\t(deref (var user/m))\tclojure.lang.PersistentArrayMap\t{:a 1}\n@@row@@\tn\t\tjava.lang.Long\t1\n",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].children.as_deref(), Some("(deref (var user/m))"));
        assert_eq!(rows[1].children, None);
        assert_eq!(rows[1].value, "1");
    }

    #[test]
    fn child_queries_quote_their_expression() {
        let query = clojure_children(r#"(get x "a")"#);
        assert!(query.contains(r#"e "(get x \"a\")""#), "{query}");
    }

    #[test]
    fn refuses_other_computers() {
        let error = Nrepl::connect("192.0.2.1:7888", || {}).err().unwrap();
        assert!(error.contains("isn't on this computer"));
    }

    /// A server that answers the handshake and one eval the way nREPL does.
    fn fake_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut sessions = 0;
            while let Some(Bencode::Dict(map)) = Bencode::decode(&mut reader) {
                let id = map["id"].clone();
                let op = map["op"].as_text().unwrap();
                let status = |list: &[&str]| Bencode::List(list.iter().map(|s| Bencode::text(s)).collect());
                let reply = |fields: Vec<(&str, Bencode)>| {
                    let mut all = fields;
                    all.push(("id", id.clone()));
                    message(&all)
                };
                match op.as_str() {
                    "clone" => {
                        sessions += 1;
                        let name = format!("s{sessions}");
                        stream
                            .write_all(&reply(vec![
                                ("new-session", Bencode::text(&name)),
                                ("status", status(&["done"])),
                            ]))
                            .unwrap();
                    }
                    "describe" => {
                        let ops = Bencode::Dict([("eval".to_owned(), Bencode::Dict(BTreeMap::new()))].into());
                        let version = Bencode::Dict([("version-string".to_owned(), Bencode::text("1.2"))].into());
                        let versions = Bencode::Dict([("clojure".to_owned(), version)].into());
                        stream
                            .write_all(&reply(vec![
                                ("ops", ops),
                                ("versions", versions),
                                ("status", status(&["done"])),
                            ]))
                            .unwrap();
                    }
                    "eval" => {
                        assert_eq!(map["session"].as_text().as_deref(), Some("s1"));
                        stream.write_all(&reply(vec![("out", Bencode::text("hi\n"))])).unwrap();
                        stream
                            .write_all(&reply(vec![("value", Bencode::text("42")), ("ns", Bencode::text("user"))]))
                            .unwrap();
                        stream.write_all(&reply(vec![("status", status(&["done"]))])).unwrap();
                    }
                    _ => {}
                }
            }
        });
        address
    }

    fn wait_for(session: &mut Nrepl, until: impl Fn(&LiveEvent) -> bool) -> Vec<LiveEvent> {
        let mut all = Vec::new();
        for _ in 0..500 {
            all.extend(session.poll());
            if all.iter().any(&until) {
                return all;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("nothing came: {all:?}");
    }

    #[test]
    fn talks_to_a_server() {
        let mut session = Nrepl::connect(&fake_server(), || {}).unwrap();
        // Sent before the handshake is done, and it still goes through.
        let id = session.eval(Eval { code: "(+ 40 2)".to_owned(), ..Eval::default() });
        let events = wait_for(&mut session, |e| matches!(e, LiveEvent::Done { .. }));
        assert!(events.iter().any(|e| matches!(e, LiveEvent::Connected(i) if i.system == "clojure 1.2" && i.can("eval") && !i.can("interrupt"))));
        assert!(events.contains(&LiveEvent::Output { id, text: "hi\n".to_owned(), error: false }));
        assert!(events.contains(&LiveEvent::Value { id, text: "42".to_owned() }));
        assert!(events.contains(&LiveEvent::Done { id, namespace: Some("user".to_owned()) }));
    }
}
