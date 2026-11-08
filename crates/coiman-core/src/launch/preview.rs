//! Live preview: a small web server on this computer that shows a folder in the
//! user's own browser, reloads the page as files change, swaps stylesheets in
//! place, compiles TypeScript, and reports console output and element clicks back
//! to the editor. It listens only on the loopback address, and answers only
//! requests addressed to it by that name.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Largest request head and body accepted; pages only send small reports.
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 256 * 1024;
/// How often files the page uses are checked for changes on disk.
const WATCH_EVERY: Duration = Duration::from_millis(300);
/// Keeps an idle event stream from looking dead, and notices closed tabs.
const PING_EVERY: Duration = Duration::from_secs(15);

/// Runs in the page: reloads, swaps stylesheets, reports console output, and
/// reports Alt+click.
const CLIENT: &str = r#"(() => {
  const post = (where, body) => fetch(where, { method: 'POST', keepalive: true, body: JSON.stringify(body) }).catch(() => {});
  const events = new EventSource('/__rc/events');
  events.onmessage = (e) => {
    if (e.data.startsWith('css ')) {
      const path = e.data.slice(4);
      const links = [...document.querySelectorAll('link[rel="stylesheet"]')].filter((l) => new URL(l.href).pathname === path);
      if (!links.length) { location.reload(); return; }
      for (const link of links) {
        const url = new URL(link.href);
        url.searchParams.set('rc', Date.now());
        const fresh = link.cloneNode();
        fresh.href = url.href;
        fresh.onload = () => link.remove();
        link.after(fresh);
      }
    } else if (e.data === 'reload') {
      location.reload();
    }
  };
  const show = (value) => {
    if (typeof value === 'string') return value;
    if (value instanceof Error) return value.stack || String(value);
    try { return JSON.stringify(value) ?? String(value); } catch { return String(value); }
  };
  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const original = console[level];
    console[level] = (...args) => {
      original.apply(console, args);
      post('/__rc/console', { level, text: args.map(show).join(' ') });
    };
  }
  const where = (url) => { try { return new URL(url).pathname; } catch { return ''; } };
  addEventListener('error', (e) => {
    if (e.target && e.target !== window) {
      const url = e.target.src || e.target.href || '';
      post('/__rc/console', { level: 'error', text: 'Failed to load ' + url });
      return;
    }
    post('/__rc/console', { level: 'error', text: e.message, path: where(e.filename), line: e.lineno, col: e.colno });
  }, true);
  addEventListener('unhandledrejection', (e) => post('/__rc/console', { level: 'error', text: 'Unhandled rejection: ' + show(e.reason) }));
  addEventListener('click', (e) => {
    if (!e.altKey) return;
    const element = e.target.closest && e.target.closest('[data-rc-l]');
    if (!element) return;
    e.preventDefault();
    e.stopPropagation();
    post('/__rc/reveal', { path: location.pathname, line: Number(element.dataset.rcL) });
  }, true);
})();
"#;

/// Something the page told the editor.
#[derive(Clone, Debug, PartialEq)]
pub enum PreviewEvent {
    Console {
        level: String,
        text: String,
        place: Option<(PathBuf, u32, u32)>,
    },
    /// Alt+click: show this line of this file.
    Reveal {
        file: PathBuf,
        line: u32,
    },
    /// The page is about to load again; what it reported is out of date.
    Reloading,
}

struct Shared {
    root: PathBuf,
    port: u16,
    /// The text of open files with unsaved changes, by full path.
    overlays: Mutex<HashMap<PathBuf, String>>,
    clients: Mutex<Vec<Sender<String>>>,
    /// Files the page has asked for, with when each last changed.
    served: Mutex<HashMap<PathBuf, Option<SystemTime>>>,
    events: Sender<PreviewEvent>,
    wake: Box<dyn Fn() + Send + Sync>,
    stop: AtomicBool,
}

/// A running preview server. Dropping it stops the server.
pub struct PreviewServer {
    shared: Arc<Shared>,
    pub events: Receiver<PreviewEvent>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl PreviewServer {
    /// Serves `root` on `port` (0 picks a free one). `wake` runs when the page
    /// sends something.
    pub fn start(root: &Path, port: u16, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let root = root.canonicalize()?;
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let (tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            root,
            port,
            overlays: Mutex::default(),
            clients: Mutex::default(),
            served: Mutex::default(),
            events: tx,
            wake: Box::new(wake),
            stop: AtomicBool::new(false),
        });
        let accepting = Arc::clone(&shared);
        std::thread::Builder::new().name("preview-accept".into()).spawn(move || {
            for stream in listener.incoming() {
                if accepting.stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&accepting);
                let _ = std::thread::Builder::new().name("preview-request".into()).spawn(move || {
                    let _ = answer(stream, &shared);
                });
            }
        })?;
        let watching = Arc::clone(&shared);
        std::thread::Builder::new().name("preview-watch".into()).spawn(move || {
            while !watching.stop.load(Ordering::Relaxed) {
                std::thread::sleep(WATCH_EVERY);
                watch_files(&watching);
            }
        })?;
        Ok(Self { shared, events })
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    pub fn root(&self) -> &Path {
        &self.shared.root
    }

    /// The address that shows `file`, if it's in the folder.
    pub fn url_of(&self, file: &Path) -> Option<String> {
        let file = file.canonicalize().ok()?;
        let relative = file.strip_prefix(&self.shared.root).ok()?;
        let path: Vec<String> =
            relative.components().map(|c| percent_encode(&c.as_os_str().to_string_lossy())).collect();
        Some(format!("http://127.0.0.1:{}/{}", self.shared.port, path.join("/")))
    }

    /// Sets the unsaved text of `file`, or clears it. The page follows along.
    pub fn set_overlay(&self, file: &Path, text: Option<String>) {
        let Ok(file) = file.canonicalize() else { return };
        let changed = {
            let mut overlays = locked(&self.shared.overlays);
            match text {
                Some(text) => overlays.insert(file.clone(), text.clone()).as_ref() != Some(&text),
                None => overlays.remove(&file).is_some(),
            }
        };
        if changed && locked(&self.shared.served).contains_key(&file) {
            announce(&self.shared, &file);
        }
    }
}

impl Drop for PreviewServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Closing the streams ends their threads; connecting ends the accept loop.
        locked(&self.shared.clients).clear();
        let _ = TcpStream::connect(("127.0.0.1", self.shared.port));
    }
}

fn changed_at(file: &Path) -> Option<SystemTime> {
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

fn watch_files(shared: &Shared) {
    let mut changed = Vec::new();
    for (file, stamp) in locked(&shared.served).iter_mut() {
        let now = changed_at(file);
        if now != *stamp {
            *stamp = now;
            changed.push(file.clone());
        }
    }
    for file in changed {
        // The page already has an open file's unsaved text.
        if !locked(&shared.overlays).contains_key(&file) {
            announce(shared, &file);
        }
    }
}

/// Tells the open pages a file changed: a stylesheet in place, anything else by loading again.
fn announce(shared: &Shared, file: &Path) {
    let is_css = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("css"));
    let message = match file.strip_prefix(&shared.root) {
        Ok(relative) if is_css => {
            let parts: Vec<String> =
                relative.components().map(|c| percent_encode(&c.as_os_str().to_string_lossy())).collect();
            format!("css /{}", parts.join("/"))
        }
        _ => "reload".to_owned(),
    };
    if message == "reload" {
        let _ = shared.events.send(PreviewEvent::Reloading);
        (shared.wake)();
    }
    locked(&shared.clients).retain(|client| client.send(message.clone()).is_ok());
}

struct Request {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.take((MAX_HEAD + MAX_BODY) as u64));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut first = line.split_whitespace();
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let (mut host, mut origin, mut length) = (String::new(), None, 0);
    let mut head = line.len();
    loop {
        line.clear();
        head += reader.read_line(&mut line).ok()?;
        if line.trim_end().is_empty() || head > MAX_HEAD {
            break;
        }
        let (name, value) = line.split_once(':')?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "host" => host = value.to_owned(),
            "origin" => origin = Some(value.to_owned()),
            "content-length" => length = value.parse().ok()?,
            _ => {}
        }
    }
    let mut body = vec![0; length];
    if length > MAX_BODY {
        return None;
    }
    reader.read_exact(&mut body).ok()?;
    let path = target.split(['?', '#']).next().unwrap_or("/").to_owned();
    Some(Request { method, path, host, origin, body })
}

fn respond(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)
}

/// Whether a request is addressed to this server by a local name. Other names
/// mean a web site pointed its own domain at this computer to read these files.
fn is_own_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

fn answer(mut stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some(request) = read_request(&stream) else {
        return respond(&stream, "400 Bad Request", "text/plain", b"Bad request");
    };
    let own_origin = |origin: &str| origin.strip_prefix("http://").is_some_and(|host| is_own_host(host, shared.port));
    if !is_own_host(&request.host, shared.port) || request.origin.as_deref().is_some_and(|o| !own_origin(o)) {
        return respond(&stream, "403 Forbidden", "text/plain", b"Forbidden");
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/__rc/client.js") => respond(&stream, "200 OK", "text/javascript", CLIENT.as_bytes()),
        ("GET", "/__rc/events") => {
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\n\r\n: ready\n\n",
            )?;
            let (tx, rx) = mpsc::channel();
            locked(&shared.clients).push(tx);
            loop {
                match rx.recv_timeout(PING_EVERY) {
                    Ok(message) => stream.write_all(format!("data: {message}\n\n").as_bytes())?,
                    Err(RecvTimeoutError::Timeout) => stream.write_all(b": ping\n\n")?,
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                }
            }
        }
        ("POST", "/__rc/console") => {
            report_console(&request.body, shared);
            respond(&stream, "204 No Content", "text/plain", b"")
        }
        ("POST", "/__rc/reveal") => {
            report_reveal(&request.body, shared);
            respond(&stream, "204 No Content", "text/plain", b"")
        }
        ("GET", path) => serve_file(&stream, path, shared),
        _ => respond(&stream, "405 Method Not Allowed", "text/plain", b"Not allowed"),
    }
}

fn report_console(body: &[u8], shared: &Shared) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return };
    let text = |key: &str| value[key].as_str().unwrap_or("").to_owned();
    let number = |key: &str| value[key].as_u64().unwrap_or(0) as u32;
    let place = find_file(&text("path"), &shared.root).map(|file| (plain_path(file), number("line"), number("col")));
    let _ = shared.events.send(PreviewEvent::Console { level: text("level"), text: text("text"), place });
    (shared.wake)();
}

fn report_reveal(body: &[u8], shared: &Shared) {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else { return };
    let Some(file) = value["path"].as_str().and_then(|path| find_file(path, &shared.root)) else { return };
    let line = value["line"].as_u64().unwrap_or(1) as u32;
    let _ = shared.events.send(PreviewEvent::Reveal { file: plain_path(file), line });
    (shared.wake)();
}

/// The file a request means, if it's one to show. Hidden names are refused, so
/// `.git` and `.env` stay private, and so is anything outside the folder.
fn find_file(url_path: &str, root: &Path) -> Option<PathBuf> {
    let decoded = percent_decode(url_path)?;
    let mut target = root.to_path_buf();
    for part in decoded.split('/').filter(|p| !p.is_empty()) {
        if part.starts_with('.') || part.contains(['\\', ':']) {
            return None;
        }
        target.push(part);
    }
    if target.is_dir() {
        target.push("index.html");
    }
    // Browsers ask for `./thing` when source says so: TypeScript imports do.
    let found = if target.is_file() {
        target
    } else {
        ["ts", "tsx", "js", "mjs", "jsx"].iter().map(|ext| add_extension(&target, ext)).find(|p| p.is_file())?
    };
    found.canonicalize().ok().filter(|p| p.starts_with(root))
}

/// A resolved path as the editor writes paths, without Windows' `\\\\?\\`, so
/// it names the same tab.
fn plain_path(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        Some(text) => PathBuf::from(text),
        None => path,
    }
}

fn add_extension(path: &Path, ext: &str) -> PathBuf {
    let mut text = path.as_os_str().to_owned();
    text.push(".");
    text.push(ext);
    PathBuf::from(text)
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" | "ts" | "mts" | "tsx" | "jsx" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn serve_file(stream: &TcpStream, url_path: &str, shared: &Shared) -> std::io::Result<()> {
    let Some(file) = find_file(url_path, &shared.root) else {
        let page = format!("<!doctype html><title>Not found</title><p>Not found: {}</p>", escape_html(url_path));
        return respond(stream, "404 Not Found", "text/html; charset=utf-8", inject_client(&page).as_bytes());
    };
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let overlay = locked(&shared.overlays).get(&file).cloned();
    let bytes = match overlay {
        Some(text) => text.into_bytes(),
        None => std::fs::read(&file)?,
    };
    locked(&shared.served).entry(file.clone()).or_insert_with(|| changed_at(&file));
    let kind = content_type(&ext);
    match ext.as_str() {
        "html" | "htm" => {
            let page = inject_client(&annotate_lines(&String::from_utf8_lossy(&bytes)));
            respond(stream, "200 OK", kind, page.as_bytes())
        }
        "ts" | "mts" | "tsx" | "jsx" => {
            let source = String::from_utf8_lossy(&bytes);
            let script = compile_script(&file, &source).unwrap_or_else(|why| {
                // Shows in the page's console, and so in the editor's.
                format!("console.error({});", serde_json::Value::String(why))
            });
            respond(stream, "200 OK", kind, script.as_bytes())
        }
        _ => respond(stream, "200 OK", kind, &bytes),
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Elements whose content isn't markup.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title"];

/// Marks each start tag with its line in the source, for Alt+click.
fn annotate_lines(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 8);
    let mut rest = html;
    let mut line = 1;
    while let Some(open) = rest.find('<') {
        let (before, tag) = rest.split_at(open);
        out.push_str(before);
        line += before.matches('\n').count();
        if tag.starts_with("<!--") {
            let end = tag.find("-->").map_or(tag.len(), |e| e + 3);
            out.push_str(&tag[..end]);
            line += tag[..end].matches('\n').count();
            rest = &tag[end..];
            continue;
        }
        let name_len = tag[1..].find(|c: char| !c.is_ascii_alphanumeric() && c != '-').unwrap_or(tag.len() - 1);
        let name = &tag[1..1 + name_len];
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            out.push('<');
            rest = &tag[1..];
            continue;
        }
        // The end of the tag, past any `>` inside attribute values.
        let mut quote = None;
        let mut end = tag.len();
        for (i, c) in tag.char_indices().skip(1 + name_len) {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), c) if q == c => quote = None,
                (None, '>') => {
                    end = i + 1;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&tag[..1 + name_len]);
        out.push_str(&format!(" data-rc-l=\"{line}\""));
        out.push_str(&tag[1 + name_len..end]);
        line += tag[..end].matches('\n').count();
        rest = &tag[end..];
        let lower = name.to_ascii_lowercase();
        if RAW_TEXT.contains(&lower.as_str()) {
            let close = format!("</{lower}");
            let until = rest.to_ascii_lowercase().find(&close).unwrap_or(rest.len());
            out.push_str(&rest[..until]);
            line += rest[..until].matches('\n').count();
            rest = &rest[until..];
        }
    }
    out.push_str(rest);
    out
}

/// Puts the client script first in the page, so it sees everything the page logs.
fn inject_client(html: &str) -> String {
    const TAG: &str = "<script src=\"/__rc/client.js\"></script>";
    let lower = html.to_ascii_lowercase();
    if let Some(at) = lower.find("<head")
        && let Some(end) = lower[at..].find('>')
    {
        let split = at + end + 1;
        return format!("{}{TAG}{}", &html[..split], &html[split..]);
    }
    // No head: after the doctype, or a page in quirks mode would change how it looks.
    if lower.trim_start().starts_with("<!doctype")
        && let Some(end) = lower.find('>')
    {
        return format!("{}{TAG}{}", &html[..end + 1], &html[end + 1..]);
    }
    format!("{TAG}{html}")
}

/// Turns TypeScript or JSX into JavaScript a browser runs, or says what's wrong.
pub fn compile_script(file: &Path, source: &str) -> Result<String, String> {
    use oxc_allocator::Allocator;
    use oxc_codegen::Codegen;
    use oxc_parser::Parser;
    use oxc_semantic::SemanticBuilder;
    use oxc_span::SourceType;
    use oxc_transformer::{TransformOptions, Transformer};

    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file).map_err(|e| format!("{name}: {e}"))?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(error) = parsed.diagnostics.first() {
        return Err(format!("{name}: {}", error.message));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new().with_enum_eval(true).build(&program).semantic.into_scoping();
    let transformed =
        Transformer::new(&allocator, file, &TransformOptions::default()).build_with_scoping(scoping, &mut program);
    if let Some(error) = transformed.diagnostics.first() {
        return Err(format!("{name}: {}", error.message));
    }
    Ok(Codegen::new().build(&program).code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("coiman-preview-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            "<!doctype html><html><head><title>x</title></head><body>hi</body></html>",
        )
        .unwrap();
        std::fs::write(dir.join("a.css"), "a{}").unwrap();
        std::fs::write(dir.join("m.ts"), "const n: number = 1; export { n };").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.join(".git").join("config"), "secret").unwrap();
        dir.canonicalize().unwrap()
    }

    fn get(port: u16, path: &str, host: &str, extra: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n{extra}\r\n").unwrap();
        let mut text = String::new();
        stream.read_to_string(&mut text).unwrap();
        text
    }

    #[test]
    fn serves_only_what_it_should() {
        let dir = folder("serve");
        let server = PreviewServer::start(&dir, 0, || {}).unwrap();
        let port = server.port();
        let host = format!("127.0.0.1:{port}");
        let page = get(port, "/", &host, "");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("/__rc/client.js") && page.contains("hi"));
        assert!(get(port, "/a.css", &host, "").contains("text/css"));
        // TypeScript arrives as JavaScript, and an import without its extension finds it.
        let script = get(port, "/m", &host, "");
        assert!(script.contains("text/javascript") && script.contains("const n = 1"), "{script}");
        // Outside the folder, and hidden files, stay out.
        for path in ["/../secret", "/%2e%2e/secret", "/.env", "/.git/config", "/sub/../.env", "/a%5c..%5c.env"] {
            assert!(get(port, path, &host, "").starts_with("HTTP/1.1 404"), "{path}");
        }
        // A site pointing its own name at this computer gets nothing.
        assert!(get(port, "/", &format!("evil.example:{port}"), "").starts_with("HTTP/1.1 403"));
        assert!(get(port, "/", &host, "Origin: http://evil.example\r\n").starts_with("HTTP/1.1 403"));
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsaved_text_is_served() {
        let dir = folder("overlay");
        let server = PreviewServer::start(&dir, 0, || {}).unwrap();
        let host = format!("127.0.0.1:{}", server.port());
        server.set_overlay(&dir.join("a.css"), Some("b{}".to_owned()));
        assert!(get(server.port(), "/a.css", &host, "").ends_with("b{}"));
        server.set_overlay(&dir.join("a.css"), None);
        assert!(get(server.port(), "/a.css", &host, "").ends_with("a{}"));
        assert!(server.url_of(&dir.join("sub")).unwrap().ends_with("/sub"));
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn marks_tags_with_their_lines() {
        let html = "<!-- <p> -->\n<div class=\"a>b\"\n  id=x>\n<script>if (a<b) {}\n</script>\n<p>t</p>\n1 < 2\n";
        let marked = annotate_lines(html);
        assert!(marked.contains("<div data-rc-l=\"2\" class=\"a>b\""), "{marked}");
        assert!(marked.contains("<script data-rc-l=\"4\">if (a<b) {}"), "{marked}");
        assert!(marked.contains("<p data-rc-l=\"6\">t</p>"), "{marked}");
        assert!(marked.contains("<!-- <p> -->") && marked.contains("1 < 2"), "{marked}");
    }

    #[test]
    fn client_goes_first() {
        assert!(inject_client("<html><head lang=x><title>").contains("<head lang=x><script"));
        assert!(inject_client("<!doctype html><p>").starts_with("<!doctype html><script"));
        assert!(inject_client("<p>x").starts_with("<script"));
    }

    #[test]
    fn typescript_compiles_or_says_why() {
        let js =
            compile_script(Path::new("a.ts"), "enum E { A }\nconst x: E = E.A;\nconsole.log(x as number);\n").unwrap();
        assert!(!js.contains(": E") && !js.contains(" as number"), "{js}");
        assert!(js.contains("E.A"), "{js}");
        assert!(compile_script(Path::new("a.ts"), "const = ;").unwrap_err().starts_with("a.ts: "));
    }

    #[test]
    fn reported_paths_are_plain() {
        assert_eq!(plain_path(PathBuf::from(r"\\?\C:\w\a.js")), PathBuf::from(r"C:\w\a.js"));
        assert_eq!(plain_path(PathBuf::from("/w/a.js")), PathBuf::from("/w/a.js"));
    }

    #[test]
    fn encoding_round_trips() {
        let text = "a b/é%.js";
        assert_eq!(percent_decode(&percent_encode(text)).as_deref(), Some(text));
        assert_eq!(percent_decode("%zz"), None);
    }
}
