//! Embedded loopback webview server: serves extension webviews, provides the
//! `vscode` webview client API (`acquireVsCodeApi`), serves extension assets,
//! and handles bidirectional messaging via SSE and HTTP POST.
//!
//! It runs only while a webview is open, listens on 127.0.0.1 alone (a random port), and
//! answers only requests under its secret path (`/<token>/…`) that name it as their host, so a
//! web page elsewhere can neither find it nor read through it. It serves files from the
//! extensions' own folders only, and sends no CORS headers.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024 * 1024;

/// An event originating from a webview in the user's browser.
#[derive(Clone, Debug, PartialEq)]
pub enum WebviewEvent {
    Message { id: String, message: serde_json::Value },
}

struct PanelData {
    html: String,
    clients: Vec<Sender<String>>,
}

struct Shared {
    port: u16,
    /// The secret first part of every path.
    token: String,
    /// Folders files may be served from (the extensions' own), canonical.
    roots: Mutex<Vec<std::path::PathBuf>>,
    panels: Mutex<HashMap<String, PanelData>>,
    events: Sender<WebviewEvent>,
    wake: Box<dyn Fn() + Send + Sync>,
    stop: AtomicBool,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// A running loopback webview server. Dropping it shuts down the server.
pub struct WebviewServer {
    shared: Arc<Shared>,
    pub events: Receiver<WebviewEvent>,
}

impl WebviewServer {
    /// Starts the server on `port` (0 to pick a free OS port).
    pub fn start(port: u16, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let port = listener.local_addr()?.port();
        let (tx, events) = mpsc::channel();
        let shared = Arc::new(Shared {
            port,
            token: secret(),
            roots: Mutex::default(),
            panels: Mutex::default(),
            events: tx,
            wake: Box::new(wake),
            stop: AtomicBool::new(false),
        });

        let accepting = Arc::clone(&shared);
        std::thread::Builder::new().name("webview-accept".into()).spawn(move || {
            for stream in listener.incoming() {
                if accepting.stop.load(Ordering::Relaxed) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&accepting);
                let _ = std::thread::Builder::new().name("webview-request".into()).spawn(move || {
                    let _ = handle_request(stream, &shared);
                });
            }
        })?;

        Ok(Self { shared, events })
    }

    pub fn port(&self) -> u16 {
        self.shared.port
    }

    /// Where the server is: `http://127.0.0.1:<port>/<token>`.
    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.shared.port, self.shared.token)
    }

    pub fn url_for(&self, id: &str) -> String {
        format!("{}/webview/{}", self.base(), percent_encode(id))
    }

    /// Lets files under `dir` (an extension's folder) be served.
    pub fn allow_folder(&self, dir: &Path) {
        if let Ok(dir) = dir.canonicalize() {
            let mut roots = locked(&self.shared.roots);
            if !roots.contains(&dir) {
                roots.push(dir);
            }
        }
    }

    /// Whether no webview is open any more.
    pub fn idle(&self) -> bool {
        locked(&self.shared.panels).is_empty()
    }

    pub fn set_html(&self, id: &str, html: &str) {
        let mut panels = locked(&self.shared.panels);
        if let Some(panel) = panels.get_mut(id) {
            panel.html = html.to_owned();
        } else {
            panels.insert(id.to_owned(), PanelData { html: html.to_owned(), clients: Vec::new() });
        }
    }

    pub fn post_message(&self, id: &str, message: &serde_json::Value) {
        let text = message.to_string();
        let mut panels = locked(&self.shared.panels);
        if let Some(panel) = panels.get_mut(id) {
            panel.clients.retain(|client| client.send(text.clone()).is_ok());
        }
    }

    pub fn remove_panel(&self, id: &str) {
        let mut panels = locked(&self.shared.panels);
        panels.remove(id);
    }
}

impl Drop for WebviewServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        locked(&self.shared.panels).clear();
        let _ = TcpStream::connect(("127.0.0.1", self.shared.port));
    }
}

struct Request {
    method: String,
    /// The `Host` header.
    host: String,
    path: String,
    query: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream.take((MAX_HEAD + MAX_BODY) as u64));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut first = line.split_whitespace();
    let (method, target) = (first.next()?.to_owned(), first.next()?.to_owned());
    let mut length = 0;
    let mut host = String::new();
    let mut head = line.len();
    loop {
        line.clear();
        head += reader.read_line(&mut line).ok()?;
        if line.trim_end().is_empty() || head > MAX_HEAD {
            break;
        }
        let Some((name, value)) = line.split_once(':') else { continue };
        if name.trim().eq_ignore_ascii_case("content-length") {
            length = value.trim().parse().unwrap_or(0);
        }
        if name.trim().eq_ignore_ascii_case("host") {
            host = value.trim().to_owned();
        }
    }
    if length > MAX_BODY {
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;

    let (raw_path, raw_query) = target.split_once('?').unwrap_or((&target, ""));
    let mut query = HashMap::new();
    for pair in raw_query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let key = percent_decode(k).unwrap_or_else(|| k.to_owned());
            let val = percent_decode(v).unwrap_or_else(|| v.to_owned());
            query.insert(key, val);
        }
    }

    Some(Request { method, host, path: raw_path.to_owned(), query, body })
}

/// A secret for the server's paths: 32 hex digits from the OS's randomly keyed hasher.
fn secret() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

fn handle_request(stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some(req) = read_request(&stream) else {
        return respond(&stream, "400 Bad Request", "text/plain", b"Bad Request");
    };
    // Only requests that name this server as their host (no DNS rebinding) and know its secret.
    let host_ok = req.host == format!("127.0.0.1:{}", shared.port);
    let prefix = format!("/{}", shared.token);
    let Some(path) = req.path.strip_prefix(&prefix).filter(|_| host_ok) else {
        return respond(&stream, "404 Not Found", "text/plain", b"Not Found");
    };

    if path == "/webview/events" {
        let panel_id = req.query.get("id").cloned().unwrap_or_default();
        return serve_sse(stream, &panel_id, shared);
    }

    if path == "/webview/msg" && req.method.eq_ignore_ascii_case("POST") {
        let panel_id = req.query.get("id").cloned().unwrap_or_default();
        return handle_message(&stream, &panel_id, &req.body, shared);
    }

    if let Some(panel_id) = path.strip_prefix("/webview/") {
        let panel_id = percent_decode(panel_id).unwrap_or_else(|| panel_id.to_owned());
        return serve_webview(&stream, &panel_id, shared);
    }

    if path == "/file"
        && let Some(file_path) = req.query.get("path")
    {
        return serve_file(&stream, Path::new(file_path), shared);
    }

    respond(&stream, "404 Not Found", "text/plain", b"Not Found")
}

/// What the extension host writes for the server's address (`asWebviewUri`, `cspSource`): it
/// doesn't know it, since the server starts only with the first webview.
pub const BASE_MARK: &str = "coiman-webview://base";

fn inject_shim(html: &str, panel_id: &str, base: &str) -> String {
    let html = html.replace(BASE_MARK, base);
    let html = html.as_str();
    let script = format!(
        r#"<script>
(function() {{
  const panelId = {id_json};
  const base = {base_json};
  window.acquireVsCodeApi = function() {{
    return {{
      postMessage: function(msg) {{
        fetch(base + '/webview/msg?id=' + encodeURIComponent(panelId), {{
          method: 'POST',
          headers: {{ 'Content-Type': 'application/json' }},
          body: JSON.stringify(msg)
        }}).catch(function(e) {{ console.error('vscode.postMessage error:', e); }});
      }},
      getState: function() {{
        try {{ return JSON.parse(sessionStorage.getItem('vscode_state_' + panelId) || 'null'); }} catch(e) {{ return null; }}
      }},
      setState: function(state) {{
        try {{ sessionStorage.setItem('vscode_state_' + panelId, JSON.stringify(state)); }} catch(e) {{}}
      }}
    }};
  }};
  try {{
    const sse = new EventSource(base + '/webview/events?id=' + encodeURIComponent(panelId));
    sse.onmessage = function(e) {{
      try {{
        const msg = JSON.parse(e.data);
        window.dispatchEvent(new MessageEvent('message', {{ data: msg }}));
      }} catch(err) {{
        console.error('Failed to parse SSE message:', err);
      }}
    }};
  }} catch(e) {{
    console.error('Failed to start EventSource:', e);
  }}
}})();
</script>"#,
        id_json = serde_json::to_string(panel_id).unwrap_or_else(|_| format!("\"{panel_id}\"")),
        base_json = serde_json::to_string(base).unwrap_or_default(),
    );

    if let Some(idx) = html.to_lowercase().find("</head>") {
        let mut out = String::with_capacity(html.len() + script.len());
        out.push_str(&html[..idx]);
        out.push_str(&script);
        out.push_str(&html[idx..]);
        out
    } else {
        format!("{script}\n{html}")
    }
}

fn serve_webview(stream: &TcpStream, panel_id: &str, shared: &Shared) -> std::io::Result<()> {
    let panels = locked(&shared.panels);
    let html = panels.get(panel_id).map(|p| p.html.as_str()).unwrap_or(
        "<!doctype html><html><head><title>Webview</title></head><body><p>Webview is initializing...</p></body></html>",
    );
    let base = format!("http://127.0.0.1:{}/{}", shared.port, shared.token);
    let injected = inject_shim(html, panel_id, &base);
    respond(stream, "200 OK", "text/html; charset=utf-8", injected.as_bytes())
}

fn serve_sse(mut stream: TcpStream, panel_id: &str, shared: &Shared) -> std::io::Result<()> {
    let (tx, rx) = mpsc::channel();
    {
        let mut panels = locked(&shared.panels);
        if let Some(panel) = panels.get_mut(panel_id) {
            panel.clients.push(tx);
        } else {
            panels.insert(panel_id.to_owned(), PanelData { html: String::new(), clients: vec![tx] });
        }
    }

    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(head.as_bytes())?;
    stream.flush()?;

    while !shared.stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(msg) => {
                let payload = format!("data: {msg}\n\n");
                if stream.write_all(payload.as_bytes()).is_err() || stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if stream.write_all(b": ping\n\n").is_err() || stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn handle_message(stream: &TcpStream, panel_id: &str, body: &[u8], shared: &Shared) -> std::io::Result<()> {
    if let Ok(message) = serde_json::from_slice::<serde_json::Value>(body) {
        let _ = shared.events.send(WebviewEvent::Message { id: panel_id.to_owned(), message });
        (shared.wake)();
    }
    respond_cors(stream, "200 OK", "application/json", b"{\"ok\":true}")
}

fn serve_file(stream: &TcpStream, path: &Path, shared: &Shared) -> std::io::Result<()> {
    // Only what the extensions themselves hold.
    let inside = path.canonicalize().is_ok_and(|p| locked(&shared.roots).iter().any(|r| p.starts_with(r)));
    if !inside || !path.is_file() {
        return respond_cors(stream, "404 Not Found", "text/plain", b"File not found");
    }
    let Ok(bytes) = std::fs::read(path) else {
        return respond_cors(stream, "500 Internal Error", "text/plain", b"Could not read file");
    };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let kind = content_type(&ext);
    respond_cors(stream, "200 OK", kind, &bytes)
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn respond(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_cors(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
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
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
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
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raw HTTP request to the server, its whole answer back.
    fn get(port: u16, host: &str, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(stream, "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
        let mut out = String::new();
        let _ = stream.read_to_string(&mut out);
        out
    }

    #[test]
    fn only_its_own_secret_host_and_folders() {
        let server = WebviewServer::start(0, || {}).expect("starts webview server");
        let port = server.port();
        let host = format!("127.0.0.1:{port}");
        let base = server.base();
        let secret = base.rsplit('/').next().unwrap().to_owned();
        assert_eq!(secret.len(), 32);
        server.set_html("p1", &format!("<html><head></head><img src=\"{BASE_MARK}/file?path=x\"></html>"));
        let url = server.url_for("p1");
        assert_eq!(url, format!("{base}/webview/p1"));

        // The page: the extension host's mark becomes the real address, and no CORS header.
        let page = get(port, &host, &format!("/{secret}/webview/p1"));
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains(&format!("{base}/file?path=x")) && !page.contains(BASE_MARK), "{page}");
        assert!(!page.to_lowercase().contains("access-control-allow-origin"), "{page}");
        // Without the secret, or naming another host (DNS rebinding): nothing.
        assert!(get(port, &host, "/webview/p1").starts_with("HTTP/1.1 404"));
        assert!(get(port, "evil.example:80", &format!("/{secret}/webview/p1")).starts_with("HTTP/1.1 404"));

        // Files: from an extension's folder only.
        let dir = std::env::temp_dir().join(format!("coiman-webview-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::write(dir.join("ext").join("a.css"), "body{}").unwrap();
        std::fs::write(dir.join("secret.txt"), "private").unwrap();
        server.allow_folder(&dir.join("ext"));
        let file = |p: &std::path::Path| {
            get(port, &host, &format!("/{secret}/file?path={}", percent_encode(&p.to_string_lossy())))
        };
        let css = file(&dir.join("ext").join("a.css"));
        assert!(css.starts_with("HTTP/1.1 200") && css.ends_with("body{}"), "{css}");
        assert!(file(&dir.join("secret.txt")).starts_with("HTTP/1.1 404"));
        assert!(file(&dir.join("ext").join("..").join("secret.txt")).starts_with("HTTP/1.1 404"));

        // It knows when the last webview has gone.
        assert!(!server.idle());
        server.remove_panel("p1");
        assert!(server.idle());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
