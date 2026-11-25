//! Embedded loopback webview server: serves extension webviews, provides the
//! `vscode` webview client API (`acquireVsCodeApi`), serves extension assets,
//! and handles bidirectional messaging via SSE and HTTP POST.

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

    pub fn url_for(&self, id: &str) -> String {
        format!("http://127.0.0.1:{}/webview/{}", self.shared.port, percent_encode(id))
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

    Some(Request { method, path: raw_path.to_owned(), query, body })
}

fn handle_request(stream: TcpStream, shared: &Shared) -> std::io::Result<()> {
    let Some(req) = read_request(&stream) else {
        return respond(&stream, "400 Bad Request", "text/plain", b"Bad Request");
    };

    if req.method.eq_ignore_ascii_case("OPTIONS") {
        return respond_cors(&stream, "204 No Content", "text/plain", b"");
    }

    if req.path.starts_with("/webview/") {
        let panel_id = req.path.trim_start_matches("/webview/");
        let panel_id = percent_decode(panel_id).unwrap_or_else(|| panel_id.to_owned());
        return serve_webview(&stream, &panel_id, shared);
    }

    if req.path == "/webview/events" {
        let panel_id = req.query.get("id").cloned().unwrap_or_default();
        return serve_sse(stream, &panel_id, shared);
    }

    if req.path == "/webview/msg" {
        let panel_id = req.query.get("id").cloned().unwrap_or_default();
        return handle_message(&stream, &panel_id, &req.body, shared);
    }

    if req.path == "/file"
        && let Some(file_path) = req.query.get("path")
    {
        return serve_file(&stream, Path::new(file_path));
    }

    respond(&stream, "404 Not Found", "text/plain", b"Not Found")
}

fn inject_shim(html: &str, panel_id: &str) -> String {
    let script = format!(
        r#"<script>
(function() {{
  const panelId = {id_json};
  window.acquireVsCodeApi = function() {{
    return {{
      postMessage: function(msg) {{
        fetch('/webview/msg?id=' + encodeURIComponent(panelId), {{
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
    const sse = new EventSource('/webview/events?id=' + encodeURIComponent(panelId));
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
        id_json = serde_json::to_string(panel_id).unwrap_or_else(|_| format!("\"{panel_id}\""))
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
    let injected = inject_shim(html, panel_id);
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

    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\nAccess-Control-Allow-Origin: *\r\n\r\n";
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

fn serve_file(stream: &TcpStream, path: &Path) -> std::io::Result<()> {
    if !path.is_file() {
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
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn respond_cors(mut stream: &TcpStream, status: &str, kind: &str, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: *\r\nConnection: close\r\n\r\n",
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

    #[test]
    fn server_starts_and_serves_html() {
        let server = WebviewServer::start(0, || {}).expect("starts webview server");
        let port = server.port();
        assert!(port > 0);
        server.set_html("test_1", "<h1>Hello Webview</h1>");
        let url = server.url_for("test_1");
        assert!(url.contains(&format!(":{port}/webview/test_1")));
    }
}
