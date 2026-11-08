//! Live sessions beyond nREPL. Python kernels and BEAM nodes are reached through a
//! small program in their own language, since their wire formats would need more than
//! this editor carries. Swank and RIDE are plain sockets on this computer.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::language::lsp::discovery::{kill_tree, quiet_command};
use crate::language::lsp::servers::{is_local_address, which};
use crate::launch::live::{Eval, Identity, LiveEvent, Nrepl, Session, parse_rows};

// Windows takes about two seconds to say a port is refused.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_FRAME: usize = 64 * 1024 * 1024;

const KERNEL_BRIDGE: &str = include_str!("../../data/kernel_bridge.py");
const BEAM_BRIDGE: &str = include_str!("../../data/beam_bridge.erl");

/// The kinds of running system there's a way to talk to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiveKind {
    #[default]
    Nrepl,
    PythonKernel,
    Swank,
    Beam,
    Ride,
}

impl LiveKind {
    pub const ALL: [Self; 5] = [Self::Nrepl, Self::PythonKernel, Self::Swank, Self::Beam, Self::Ride];

    pub fn name(self) -> &'static str {
        match self {
            Self::Nrepl => "nREPL (Clojure)",
            Self::PythonKernel => "Python kernel",
            Self::Swank => "Swank (Common Lisp)",
            Self::Beam => "BEAM node (Elixir, Erlang)",
            Self::Ride => "RIDE (Dyalog APL)",
        }
    }

    /// What to type to reach one.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Nrepl => "nREPL host:port on this computer",
            Self::PythonKernel => "Blank for a new kernel, or a kernel-*.json",
            Self::Swank => "Swank host:port, such as 127.0.0.1:4005",
            Self::Beam => "name@host on this computer, then a cookie if needed",
            Self::Ride => "RIDE host:port, such as 127.0.0.1:4502",
        }
    }

    /// The kind that suits a file, so the panel starts on it.
    pub fn for_extension(extension: &str) -> Option<Self> {
        match extension {
            "clj" | "cljs" | "cljc" | "edn" | "bb" => Some(Self::Nrepl),
            "py" | "pyw" => Some(Self::PythonKernel),
            "lisp" | "cl" | "lsp" | "asd" => Some(Self::Swank),
            "ex" | "exs" | "erl" | "hrl" => Some(Self::Beam),
            "apl" | "apln" | "aplf" | "dyalog" => Some(Self::Ride),
            _ => None,
        }
    }
}

/// What it takes to start the session: which Python, and where.
#[derive(Clone, Debug, Default)]
pub struct ConnectOptions {
    pub python: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

/// Connects. Takes as long as it takes: call it away from the UI.
pub fn connect(
    kind: LiveKind,
    address: &str,
    options: &ConnectOptions,
    wake: impl Fn() + Send + 'static,
) -> Result<Box<dyn Session>, String> {
    match kind {
        LiveKind::Nrepl => Ok(Box::new(Nrepl::connect(address, wake)?)),
        LiveKind::PythonKernel => Ok(Box::new(Bridge::python(address, options, wake)?)),
        LiveKind::Swank => Ok(Box::new(Swank::connect(address, wake)?)),
        LiveKind::Beam => Ok(Box::new(Bridge::beam(address, options, wake)?)),
        LiveKind::Ride => Ok(Box::new(Ride::connect(address, wake)?)),
    }
}

fn dial(address: &str) -> Result<TcpStream, String> {
    if !is_local_address(address) {
        return Err(format!("{address} isn't on this computer"));
    }
    let target = address.to_socket_addrs().map_err(|e| format!("{address}: {e}"))?.next().ok_or("no such address")?;
    TcpStream::connect_timeout(&target, CONNECT_TIMEOUT).map_err(|e| format!("{address}: {e}"))
}

fn text_of(encoded: &str) -> String {
    B64.decode(encoded).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Writes a bridge program where it can be run from. Named by what's running it, so
/// sessions don't share one that another is using.
fn write_bridge(name: &str, text: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("coiman-bridge");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{name}", std::process::id()));
    std::fs::write(&path, text.replace("\r\n", "\n")).map_err(|e| e.to_string())?;
    Ok(path)
}

/// A session through a program that speaks line by line.
struct Bridge {
    label: &'static str,
    child: Child,
    stdin: ChildStdin,
    from_bridge: Receiver<Option<String>>,
    next_id: u64,
    /// Requests sent before it said hello are fine: its pipe holds them.
    closed: bool,
    /// What it's attached to survives it.
    leaves_running: bool,
    language_of: fn(&Eval) -> &'static str,
}

impl Bridge {
    fn start(
        label: &'static str,
        program: &Path,
        args: &[String],
        options: &ConnectOptions,
        leaves_running: bool,
        language_of: fn(&Eval) -> &'static str,
        wake: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let mut command = quiet_command(program);
        command.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        command.envs(options.env.iter().map(|(k, v)| (k, v)));
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }
        let mut child = command.spawn().map_err(|e| format!("{}: {e}", program.display()))?;
        let stdin = child.stdin.take().ok_or("no input pipe")?;
        let stdout = child.stdout.take().ok_or("no output pipe")?;
        let (tx, from_bridge) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(Some(line)).is_err() {
                    return;
                }
                wake();
            }
            let _ = tx.send(None);
            wake();
        });
        Ok(Self { label, child, stdin, from_bridge, next_id: 0, closed: false, leaves_running, language_of })
    }

    fn python(address: &str, options: &ConnectOptions, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let python = options.python.clone().or_else(|| which("python")).ok_or("No Python found")?;
        let script = write_bridge("kernel_bridge.py", KERNEL_BRIDGE)?;
        let mut args = vec!["-I".to_owned(), "-u".to_owned(), script.to_string_lossy().into_owned()];
        let file = address.trim();
        if !file.is_empty() {
            args.push(file.to_owned());
        }
        Self::start("Python kernel", &python, &args, options, !file.is_empty(), |_| "python", wake)
    }

    fn beam(address: &str, options: &ConnectOptions, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let mut words = address.split_whitespace();
        let node = words.next().ok_or("Type the node's name, such as app@myhost")?;
        let host = node.split_once('@').map(|(_, h)| h).ok_or("A node's name looks like app@myhost")?;
        if !is_this_computer(host) {
            return Err(format!("{host} isn't this computer"));
        }
        let escript = which("escript").ok_or("Erlang's escript wasn't found on PATH")?;
        let script = write_bridge("beam_bridge.erl", BEAM_BRIDGE)?;
        let mut args = vec![script.to_string_lossy().into_owned(), node.to_owned()];
        args.extend(words.next().map(str::to_owned));
        fn language(eval: &Eval) -> &'static str {
            match eval.file.as_ref().and_then(|f| f.extension()).and_then(|e| e.to_str()) {
                Some("ex" | "exs") => "elixir",
                Some("erl" | "hrl") => "erlang",
                _ => "default",
            }
        }
        Self::start("BEAM", &escript, &args, options, true, language, wake)
    }

    fn write(&mut self, line: &str) {
        let _ = writeln!(self.stdin, "{line}");
        let _ = self.stdin.flush();
    }

    fn handle(&mut self, line: &str, events: &mut Vec<LiveEvent>) {
        let mut words = line.split_whitespace();
        let tag = words.next().unwrap_or_default();
        let parts: Vec<&str> = words.collect();
        let id = || parts.first().and_then(|w| w.parse::<u64>().ok()).unwrap_or(0);
        let text = || parts.get(1).map_or_else(String::new, |w| text_of(w));
        match tag {
            "HELLO" => {
                let system = parts.first().map_or_else(String::new, |w| text_of(w));
                let capabilities =
                    parts.get(1).map_or_else(Vec::new, |w| w.split(',').map(str::to_owned).collect::<Vec<_>>());
                events.push(LiveEvent::Connected(Identity { system, capabilities }));
            }
            "OUT" => events.push(LiveEvent::Output { id: id(), text: text(), error: false }),
            "ERR" => events.push(LiveEvent::Output { id: id(), text: text(), error: true }),
            "VALUE" => events.push(LiveEvent::Value { id: id(), text: text() }),
            "FAIL" => events.push(LiveEvent::Failed { id: id(), text: text() }),
            "INPUT" => events.push(LiveEvent::NeedInput { id: id() }),
            "DONE" => events.push(LiveEvent::Done { id: id(), namespace: None }),
            "ROWS" => events.push(LiveEvent::Rows { id: id(), rows: parse_rows(&text()), problem: None }),
            "UNLISTED" => events.push(LiveEvent::Rows { id: id(), rows: Vec::new(), problem: Some(text()) }),
            "PROBLEM" => {
                let why = parts.first().map_or_else(String::new, |w| text_of(w));
                self.closed = true;
                events.push(LiveEvent::Closed(why));
            }
            _ => {}
        }
    }
}

/// Distribution only joins nodes it can reach, which may be anywhere: not here.
fn is_this_computer(host: &str) -> bool {
    let here = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let short = host.split('.').next().unwrap_or(host);
    matches!(host, "localhost" | "127.0.0.1") || short.eq_ignore_ascii_case(&here)
}

impl Session for Bridge {
    fn name(&self) -> &str {
        self.label
    }

    fn eval(&mut self, request: Eval) -> u64 {
        self.next_id += 1;
        let (id, language) = (self.next_id, (self.language_of)(&request));
        let code = B64.encode(request.code.as_bytes());
        self.write(&format!("EVAL {id} {language} {code}").replace(" python ", " "));
        id
    }

    fn stdin(&mut self, text: &str) {
        let line = format!("STDIN {}", B64.encode(text.as_bytes()));
        self.write(&line);
    }

    fn interrupt(&mut self) {
        self.write("INTERRUPT");
    }

    fn inspect(&mut self, query: Option<&str>, _namespace: Option<&str>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let line = match query {
            Some(query) => format!("INSPECT {id} {}", B64.encode(query.as_bytes())),
            None => format!("INSPECT {id}"),
        };
        self.write(&line);
        id
    }

    fn poll(&mut self) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        while let Ok(item) = self.from_bridge.try_recv() {
            match item {
                Some(line) => self.handle(&line, &mut events),
                None if !self.closed => {
                    self.closed = true;
                    events.push(LiveEvent::Closed("The connection ended".to_owned()));
                }
                None => {}
            }
        }
        events
    }

    fn handles(&self, extension: &str) -> bool {
        LiveKind::for_extension(extension)
            == Some(if self.label == "BEAM" { LiveKind::Beam } else { LiveKind::PythonKernel })
    }

    fn leaves_running(&self) -> bool {
        self.leaves_running
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        kill_tree(&mut self.child);
    }
}

// ----- s-expressions, as Swank writes them -----------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sexp {
    Atom(String),
    Text(String),
    List(Vec<Sexp>),
}

impl Sexp {
    fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(a) => Some(a),
            _ => None,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    fn list(&self) -> &[Sexp] {
        match self {
            Self::List(l) => l,
            _ => &[],
        }
    }

    /// A value out of a `:key value` list.
    fn get(&self, key: &str) -> Option<&Sexp> {
        let list = self.list();
        list.iter().position(|s| s.atom().is_some_and(|a| a.eq_ignore_ascii_case(key))).and_then(|at| list.get(at + 1))
    }
}

pub fn parse_sexp(text: &str) -> Option<Sexp> {
    fn read(chars: &[char], at: &mut usize, depth: usize) -> Option<Sexp> {
        if depth > 512 {
            return None;
        }
        while chars.get(*at)?.is_whitespace() {
            *at += 1;
        }
        match chars[*at] {
            '(' => {
                *at += 1;
                let mut items = Vec::new();
                loop {
                    while chars.get(*at)?.is_whitespace() {
                        *at += 1;
                    }
                    if chars[*at] == ')' {
                        *at += 1;
                        return Some(Sexp::List(items));
                    }
                    items.push(read(chars, at, depth + 1)?);
                }
            }
            '"' => {
                *at += 1;
                let mut out = String::new();
                loop {
                    match *chars.get(*at)? {
                        '"' => {
                            *at += 1;
                            return Some(Sexp::Text(out));
                        }
                        '\\' => {
                            out.push(*chars.get(*at + 1)?);
                            *at += 2;
                        }
                        c => {
                            out.push(c);
                            *at += 1;
                        }
                    }
                }
            }
            ')' => None,
            _ => {
                let start = *at;
                while chars.get(*at).is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | ')' | '"')) {
                    *at += 1;
                }
                Some(Sexp::Atom(chars[start..*at].iter().collect()))
            }
        }
    }
    let chars: Vec<char> = text.chars().collect();
    read(&chars, &mut 0, 0)
}

fn lisp_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Runs every form in the text, with input coming from here. Swank alone would read one
/// form, and read input from the Lisp's own console. Its names stay in Swank's package.
fn swank_eval(code: &str) -> String {
    format!(
        r#"(let* ((swank::rc-read (lambda ()
         (let ((swank::rc-tag (random 1000000000)))
           (swank::send-to-emacs (list :read-string (swank::current-thread-id) swank::rc-tag))
           (third (swank::wait-for-event (list :emacs-return-string swank::rc-tag 'swank::value))))))
       (*standard-input* (swank/backend:make-input-stream swank::rc-read))
       (*query-io* (make-two-way-stream *standard-input* *standard-output*))
       (swank::rc-last nil))
  (with-input-from-string (swank::rc-in {})
    (loop for swank::rc-form = (read swank::rc-in nil swank::rc-in)
          until (eq swank::rc-form swank::rc-in)
          do (setf swank::rc-last (multiple-value-list (eval swank::rc-form)))))
  (values-list swank::rc-last))"#,
        lisp_string(code)
    )
}

/// Lists a package's values, or what's in a list, printed as inspector rows.
fn swank_inspect(query: Option<&str>) -> String {
    const SHOW: &str = r##"(show (name child value)
             (let* ((*print-length* 20) (*print-level* 3) (*print-pretty* nil)
                    (text (handler-case (prin1-to-string value) (error () "#<unprintable>")))
                    (text (if (> (length text) 300) (subseq text 0 300) text))
                    (type (let ((*print-pretty* nil)) (prin1-to-string (type-of value)))))
               (format t "@@row@@~c~a~c~a~c~a~c~a~%" #\Tab name #\Tab child #\Tab
                       (if (> (length type) 60) (subseq type 0 60) type) #\Tab
                       (substitute #\Space #\Tab (substitute #\Space #\Newline text)))))"##;
    match query {
        None => format!(
            r#"(labels ({SHOW})
  (let ((names nil))
    (do-symbols (s *package*) (when (and (eq (symbol-package s) *package*) (boundp s) (not (keywordp s))) (pushnew s names)))
    (dolist (s (sort names #'string<))
      (let ((v (symbol-value s)))
        (show s (if (and (consp v)) (format nil "~s" s) "") v)))))"#
        ),
        Some(expression) => format!(
            r#"(labels ({SHOW})
  (let ((v {expression}) (i 0))
    (when (listp v)
      (loop for x in v while (< i 200) do
        (show i (if (consp x) (format nil "(nth ~d ~a)" i {quoted}) "") x)
        (incf i)))))"#,
            quoted = lisp_string(expression),
        ),
    }
}

/// A session with a Lisp running Swank.
pub struct Swank {
    stream: TcpStream,
    from_server: Receiver<Option<Sexp>>,
    next: u64,
    /// Requests by what they're for.
    evals: HashMap<u64, bool>,
    inspects: HashMap<u64, bool>,
    current: u64,
    waiting_read: Option<(String, String)>,
    package: String,
    closed: bool,
}

const SWANK_INFO: u64 = 1;

impl Swank {
    pub fn connect(address: &str, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let stream = dial(address)?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        let (tx, from_server) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let mut head = [0u8; 6];
                if reader.read_exact(&mut head).is_err() {
                    break;
                }
                let size = std::str::from_utf8(&head).ok().and_then(|h| usize::from_str_radix(h, 16).ok());
                let Some(size) = size.filter(|s| *s <= MAX_FRAME) else { break };
                let mut body = vec![0; size];
                if reader.read_exact(&mut body).is_err() {
                    break;
                }
                if let Some(message) = parse_sexp(&String::from_utf8_lossy(&body)) {
                    if tx.send(Some(message)).is_err() {
                        return;
                    }
                    wake();
                }
            }
            let _ = tx.send(None);
            wake();
        });
        let mut session = Self {
            stream,
            from_server,
            next: SWANK_INFO + 1,
            evals: HashMap::new(),
            inspects: HashMap::new(),
            current: 0,
            waiting_read: None,
            package: "COMMON-LISP-USER".to_owned(),
            closed: false,
        };
        session.send(&format!("(:emacs-rex (swank:connection-info) nil t {SWANK_INFO})"));
        Ok(session)
    }

    fn send(&mut self, form: &str) {
        let body = form.as_bytes();
        let _ = self.stream.write_all(format!("{:06x}", body.len()).as_bytes());
        let _ = self.stream.write_all(body);
    }

    fn handle(&mut self, message: &Sexp, events: &mut Vec<LiveEvent>) {
        let list = message.list();
        match list.first().and_then(Sexp::atom).map(str::to_ascii_lowercase).as_deref() {
            Some(":return") => self.on_return(list, events),
            Some(":write-string") => {
                if let Some(text) = list.get(1).and_then(Sexp::text) {
                    events.push(LiveEvent::Output { id: self.current, text: text.to_owned(), error: false });
                }
            }
            Some(":read-string") => {
                let word = |at: usize| list.get(at).and_then(Sexp::atom).unwrap_or("nil").to_owned();
                self.waiting_read = Some((word(1), word(2)));
                events.push(LiveEvent::NeedInput { id: self.current });
            }
            Some(":debug") => {
                // There's no debugger here to work in: say why, and get back to work.
                let thread = list.get(1).and_then(Sexp::atom).unwrap_or("t").to_owned();
                let condition = list.get(3).map(Sexp::list).unwrap_or_default();
                let why = condition.first().and_then(Sexp::text).unwrap_or("Error").to_owned();
                events.push(LiveEvent::Failed { id: self.current, text: why });
                self.next += 1;
                let id = self.next;
                self.send(&format!("(:emacs-rex (swank:throw-to-toplevel) nil {thread} {id})"));
            }
            Some(":ping") => {
                let word = |at: usize| list.get(at).and_then(Sexp::atom).unwrap_or("nil").to_owned();
                let (thread, tag) = (word(1), word(2));
                self.send(&format!("(:emacs-pong {thread} {tag})"));
            }
            _ => {}
        }
    }

    fn on_return(&mut self, list: &[Sexp], events: &mut Vec<LiveEvent>) {
        let id = list.get(2).and_then(Sexp::atom).and_then(|a| a.parse::<u64>().ok()).unwrap_or(0);
        let outcome = list.get(1).map(Sexp::list).unwrap_or_default();
        let ok = outcome.first().and_then(Sexp::atom).is_some_and(|a| a.eq_ignore_ascii_case(":ok"));
        let value = outcome.get(1);
        if id == SWANK_INFO {
            let info = value.cloned().unwrap_or(Sexp::List(Vec::new()));
            let lisp = info.get(":lisp-implementation").cloned().unwrap_or(Sexp::List(Vec::new()));
            let part = |key: &str| lisp.get(key).and_then(Sexp::text).unwrap_or("?").to_owned();
            let system = format!("{} {}", part(":type"), part(":version"));
            if let Some(package) = info.get(":package").and_then(|p| p.get(":name")).and_then(Sexp::text) {
                self.package = package.to_owned();
            }
            let capabilities = ["eval", "inspect", "interrupt", "stdin"].map(str::to_owned).to_vec();
            events.push(LiveEvent::Connected(Identity { system, capabilities }));
            return;
        }
        let grabbed = value.map(Sexp::list).unwrap_or_default();
        let printed = grabbed.first().and_then(Sexp::text).unwrap_or_default();
        if self.inspects.remove(&id).is_some() {
            let problem = (!ok).then(|| "Couldn't list values".to_owned());
            events.push(LiveEvent::Rows { id, rows: parse_rows(printed), problem });
            return;
        }
        if self.evals.remove(&id).is_none() {
            return;
        }
        if ok {
            if !printed.is_empty() {
                events.push(LiveEvent::Output { id, text: printed.to_owned(), error: false });
            }
            if let Some(result) = grabbed.get(1).and_then(Sexp::text) {
                events.push(LiveEvent::Value { id, text: result.to_owned() });
            }
        }
        let namespace = Some(self.package.clone());
        events.push(LiveEvent::Done { id, namespace });
    }

    fn rex(&mut self, code: &str, package: &str) -> u64 {
        self.next += 1;
        let id = self.next;
        let form =
            format!("(:emacs-rex (swank:eval-and-grab-output {}) {} t {id})", lisp_string(code), lisp_string(package));
        self.send(&form);
        id
    }
}

impl Session for Swank {
    fn name(&self) -> &str {
        "Swank"
    }

    fn eval(&mut self, request: Eval) -> u64 {
        if let Some(package) = request.namespace {
            self.package = package;
        }
        let package = self.package.clone();
        let id = self.rex(&swank_eval(&request.code), &package);
        self.evals.insert(id, true);
        self.current = id;
        id
    }

    fn stdin(&mut self, text: &str) {
        if let Some((thread, tag)) = self.waiting_read.take() {
            let line = format!("{}\n", text.trim_end_matches(['\r', '\n']));
            self.send(&format!("(:emacs-return-string {thread} {tag} {})", lisp_string(&line)));
        }
    }

    fn interrupt(&mut self) {
        self.send("(:emacs-interrupt t)");
    }

    fn inspect(&mut self, query: Option<&str>, _namespace: Option<&str>) -> u64 {
        let package = self.package.clone();
        let id = self.rex(&swank_inspect(query), &package);
        self.inspects.insert(id, true);
        id
    }

    fn poll(&mut self) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        while let Ok(item) = self.from_server.try_recv() {
            match item {
                Some(message) => self.handle(&message, &mut events),
                None if !self.closed => {
                    self.closed = true;
                    events.push(LiveEvent::Closed("The connection ended".to_owned()));
                }
                None => {}
            }
        }
        events
    }

    fn handles(&self, extension: &str) -> bool {
        LiveKind::for_extension(extension) == Some(LiveKind::Swank)
    }

    fn sends_forms(&self) -> bool {
        true
    }

    fn namespace_for(&self, text: &str) -> Option<String> {
        package_of(text)
    }
}

impl Drop for Swank {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

/// What a Lisp file's `(in-package ...)` names.
pub fn package_of(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let at = lower.find("(in-package")?;
    let rest = text[at + "(in-package".len()..].trim_start();
    let name: String = rest
        .trim_start_matches(['#', ':', '\'', '"'])
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, ')' | '"'))
        .collect();
    (!name.is_empty()).then(|| name.to_ascii_uppercase())
}

// ----- RIDE ------------------------------------------------------------------------

/// Writes a RIDE frame: its length, the word RIDE, then the text.
pub fn ride_frame(text: &str) -> Vec<u8> {
    let body = text.as_bytes();
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(b"RIDE");
    out.extend_from_slice(body);
    out
}

fn read_ride_frame(reader: &mut impl Read) -> Option<String> {
    let mut head = [0u8; 8];
    reader.read_exact(&mut head).ok()?;
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if &head[4..8] != b"RIDE" || !(8..=MAX_FRAME).contains(&size) {
        return None;
    }
    let mut body = vec![0; size - 8];
    reader.read_exact(&mut body).ok()?;
    Some(String::from_utf8_lossy(&body).into_owned())
}

/// A session with Dyalog APL's interpreter, started with `RIDE_INIT=SERVE:*:port`.
pub struct Ride {
    stream: TcpStream,
    from_server: Receiver<Option<String>>,
    next_id: u64,
    /// Lines sent, oldest first, each answered by the next prompt.
    queue: VecDeque<u64>,
    output: String,
    /// What was typed last, which the interpreter says back.
    echo: Option<String>,
    failed: bool,
    said_hello: bool,
    closed: bool,
}

impl Ride {
    pub fn connect(address: &str, wake: impl Fn() + Send + 'static) -> Result<Self, String> {
        let stream = dial(address)?;
        let mut reader = stream.try_clone().map_err(|e| e.to_string())?;
        let (tx, from_server) = mpsc::channel();
        std::thread::spawn(move || {
            while let Some(frame) = read_ride_frame(&mut reader) {
                if tx.send(Some(frame)).is_err() {
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
            next_id: 0,
            queue: VecDeque::new(),
            output: String::new(),
            echo: None,
            failed: false,
            said_hello: false,
            closed: false,
        };
        session.send_text("SupportedProtocols=2");
        session.send_text("UsingProtocol=2");
        session.send_text(r#"["Identify",{"apiVersion":1,"identity":1}]"#);
        Ok(session)
    }

    fn send_text(&mut self, text: &str) {
        let _ = self.stream.write_all(&ride_frame(text));
    }

    fn execute(&mut self, code: &str) {
        let line = serde_json::json!(["Execute", { "text": format!("{}\n", code.trim_end()), "trace": 0 }]);
        self.send_text(&line.to_string());
    }

    fn handle(&mut self, frame: &str, events: &mut Vec<LiveEvent>) {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(frame) else { return };
        let (Some(command), body) = (message[0].as_str(), &message[1]) else { return };
        match command {
            "Identify" | "ReplyIdentify" if !self.said_hello => {
                self.said_hello = true;
                let version = body["version"].as_str().or(body["Version"].as_str()).unwrap_or("?");
                let capabilities = ["eval", "interrupt", "stdin"].map(str::to_owned).to_vec();
                events.push(LiveEvent::Connected(Identity { system: format!("Dyalog {version}"), capabilities }));
            }
            "AppendSessionOutput" => {
                if let Some(text) = body["result"].as_str() {
                    self.output.push_str(text);
                }
            }
            "HadError" => self.failed = true,
            "SetPromptType" => match body["type"].as_i64() {
                Some(1) => self.finish(events),
                Some(3 | 4) => {
                    let id = self.queue.front().copied().unwrap_or(0);
                    self.flush_output(id, events);
                    events.push(LiveEvent::NeedInput { id });
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn take_output(&mut self) -> String {
        let text = std::mem::take(&mut self.output);
        match self.echo.take() {
            Some(echo) => text.trim_start_matches(' ').strip_prefix(echo.as_str()).map_or(text.clone(), str::to_owned),
            None => text,
        }
    }

    fn flush_output(&mut self, id: u64, events: &mut Vec<LiveEvent>) {
        let text = self.take_output();
        if !text.trim().is_empty() {
            events.push(LiveEvent::Output { id, text, error: false });
        }
    }

    fn finish(&mut self, events: &mut Vec<LiveEvent>) {
        let Some(id) = self.queue.pop_front() else {
            self.output.clear();
            return;
        };
        let text = self.take_output();
        let text = text.trim_matches(['\r', '\n']).to_owned();
        if std::mem::take(&mut self.failed) {
            events.push(LiveEvent::Failed { id, text });
        } else if !text.is_empty() {
            events.push(LiveEvent::Value { id, text });
        }
        events.push(LiveEvent::Done { id, namespace: None });
    }
}

impl Session for Ride {
    fn name(&self) -> &str {
        "RIDE"
    }

    fn eval(&mut self, request: Eval) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.queue.push_back(id);
        self.echo = Some(request.code.trim_end().to_owned());
        self.execute(&request.code);
        id
    }

    fn stdin(&mut self, text: &str) {
        self.execute(text);
    }

    fn interrupt(&mut self) {
        self.send_text(r#"["WeakInterrupt",{}]"#);
    }

    fn inspect(&mut self, _query: Option<&str>, _namespace: Option<&str>) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn poll(&mut self) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        while let Ok(item) = self.from_server.try_recv() {
            match item {
                Some(frame) => self.handle(&frame, &mut events),
                None if !self.closed => {
                    self.closed = true;
                    events.push(LiveEvent::Closed("The connection ended".to_owned()));
                }
                None => {}
            }
        }
        events
    }

    fn inspects(&self) -> bool {
        false
    }

    fn handles(&self, extension: &str) -> bool {
        LiveKind::for_extension(extension) == Some(LiveKind::Ride)
    }
}

impl Drop for Ride {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn reads_sexps() {
        let parsed = parse_sexp(r#"(:return (:ok ("out \"x\"" "42")) 7)"#).unwrap();
        let list = parsed.list();
        assert_eq!(list[0].atom(), Some(":return"));
        assert_eq!(list[1].list()[1].list()[0].text(), Some("out \"x\""));
        assert_eq!(list[2].atom(), Some("7"));
        assert_eq!(parse_sexp("(unclosed"), None);
        assert_eq!(parse_sexp(")"), None);
        assert_eq!(parse_sexp(&"(".repeat(5000)), None, "too deep isn't a crash");
        let plist = parse_sexp(r#"(:a 1 :B "x")"#).unwrap();
        assert_eq!(plist.get(":b").and_then(Sexp::text), Some("x"));
    }

    #[test]
    fn reads_packages() {
        assert_eq!(package_of("(defun a ())\n(in-package :my-app)\n").as_deref(), Some("MY-APP"));
        assert_eq!(package_of("(in-package #:x)").as_deref(), Some("X"));
        assert_eq!(package_of("(in-package \"Y\")").as_deref(), Some("Y"));
        assert_eq!(package_of("(defun a ())"), None);
    }

    #[test]
    fn knows_which_kind_suits_a_file() {
        assert_eq!(LiveKind::for_extension("py"), Some(LiveKind::PythonKernel));
        assert_eq!(LiveKind::for_extension("exs"), Some(LiveKind::Beam));
        assert_eq!(LiveKind::for_extension("rs"), None);
    }

    #[test]
    fn beam_stays_on_this_computer() {
        assert!(is_this_computer("localhost"));
        assert!(!is_this_computer("some-other-host-name"));
        let error = connect(LiveKind::Beam, "a@some-other-host-name", &ConnectOptions::default(), || {}).err().unwrap();
        assert!(error.contains("isn't this computer"), "{error}");
        for kind in [LiveKind::Swank, LiveKind::Ride] {
            let error = connect(kind, "192.0.2.1:4005", &ConnectOptions::default(), || {}).err().unwrap();
            assert!(error.contains("isn't on this computer"), "{error}");
        }
    }

    fn wait(session: &mut dyn Session, until: impl Fn(&LiveEvent) -> bool) -> Vec<LiveEvent> {
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
    fn swank_talks_as_a_lisp_does() {
        fn say(stream: &mut TcpStream, text: &str) {
            stream.write_all(format!("{:06x}{text}", text.len()).as_bytes()).unwrap();
        }
        fn hear(stream: &mut TcpStream) -> String {
            let mut head = [0u8; 6];
            stream.read_exact(&mut head).unwrap();
            let mut body = vec![0; usize::from_str_radix(std::str::from_utf8(&head).unwrap(), 16).unwrap()];
            stream.read_exact(&mut body).unwrap();
            String::from_utf8(body).unwrap()
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(hear(&mut stream).contains("swank:connection-info"));
            say(
                &mut stream,
                r#"(:return (:ok (:lisp-implementation (:type "SBCL" :version "2.6") :package (:name "CL-USER"))) 1)"#,
            );
            // One that prints and returns, after being pinged.
            let first = hear(&mut stream);
            assert!(first.contains("eval-and-grab-output") && first.contains("(+ 1 2)"), "{first}");
            say(&mut stream, "(:ping 5 9)");
            assert_eq!(hear(&mut stream), "(:emacs-pong 5 9)");
            say(&mut stream, r#"(:write-string "live ")"#);
            say(&mut stream, r#"(:return (:ok ("out" "3")) 3)"#);
            // One that reads, then fails in the debugger.
            hear(&mut stream);
            say(&mut stream, "(:read-string 7 1)");
            assert_eq!(hear(&mut stream), "(:emacs-return-string 7 1 \"bob\n\")");
            say(&mut stream, r#"(:debug 7 1 ("division by zero" "ARITHMETIC-ERROR") () () nil)"#);
            assert!(hear(&mut stream).contains("swank:throw-to-toplevel"));
            say(&mut stream, "(:return (:abort nil) 4)");
            hear(&mut stream);
            say(&mut stream, "(:return (:ok (\"@@row@@\tx\t\tFIXNUM\t5\n\" \"nil\")) 6)");
            hear(&mut stream);
        });
        let mut session = Swank::connect(&address, || {}).unwrap();
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Connected(_)));
        assert!(events.iter().any(|e| matches!(e, LiveEvent::Connected(i) if i.system == "SBCL 2.6")), "{events:?}");
        let id = session.eval(Eval { code: "(+ 1 2)".to_owned(), ..Eval::default() });
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Done { .. }));
        assert!(events.contains(&LiveEvent::Output { id, text: "live ".to_owned(), error: false }), "{events:?}");
        assert!(events.contains(&LiveEvent::Output { id, text: "out".to_owned(), error: false }), "{events:?}");
        assert!(events.contains(&LiveEvent::Value { id, text: "3".to_owned() }), "{events:?}");
        let id = session.eval(Eval { code: "(read-line)".to_owned(), ..Eval::default() });
        wait(&mut session, |e| matches!(e, LiveEvent::NeedInput { .. }));
        session.stdin("bob");
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Done { .. }));
        assert!(events.contains(&LiveEvent::Failed { id, text: "division by zero".to_owned() }), "{events:?}");
        session.inspect(None, None);
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Rows { .. }));
        assert!(events.iter().any(|e| matches!(e, LiveEvent::Rows { rows, .. } if rows[0].name == "x")), "{events:?}");
    }

    #[test]
    fn ride_talks_as_dyalog_does() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = stream.try_clone().unwrap();
            let mut say = |text: &str| stream.write_all(&ride_frame(text)).unwrap();
            say("SupportedProtocols=2");
            say("UsingProtocol=2");
            say(r#"["Identify",{"version":"20.0.1"}]"#);
            while let Some(frame) = read_ride_frame(&mut reader) {
                if frame.starts_with("[\"Execute\"") {
                    say(r#"["AppendSessionOutput",{"result":"1+1\n"}]"#);
                    say(r#"["AppendSessionOutput",{"result":"2\n"}]"#);
                    say(r#"["SetPromptType",{"type":1}]"#);
                }
            }
        });
        let mut session = Ride::connect(&address, || {}).unwrap();
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Connected(_)));
        assert!(events.iter().any(|e| matches!(e, LiveEvent::Connected(i) if i.system == "Dyalog 20.0.1")));
        let id = session.eval(Eval { code: "1+1".to_owned(), ..Eval::default() });
        let events = wait(&mut session, |e| matches!(e, LiveEvent::Done { .. }));
        assert!(events.contains(&LiveEvent::Value { id, text: "2".to_owned() }), "{events:?}");
    }
}
