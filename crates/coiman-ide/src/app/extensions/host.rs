//! Pumps the VS Code extension host: starts it after the first frame, keeps
//! open documents in sync, and turns its messages into status/log entries.

use std::time::{Duration, Instant};

use coiman_core::language::lsp::language_id;
use eframe::egui;

use crate::app::CodeApp;

/// A pending request sent to the extension host.
#[derive(Clone, Debug)]
pub(crate) enum Request {
    Completion { uid: u64 },
    Hover { uid: u64, pos: usize },
    Format { uid: u64, save: bool },
    Tree { view: String },
    Locations { title: &'static str, word: String },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ExtDecorationType {
    pub(crate) color: Option<egui::Color32>,
    pub(crate) background_color: Option<egui::Color32>,
    pub(crate) is_whole_line: bool,
}

pub(crate) type ExtLineHighlights =
    std::sync::Arc<std::collections::HashMap<usize, Vec<crate::ui::editor_view::Highlight>>>;

pub(crate) fn parse_ext_color(s: &str) -> Option<egui::Color32> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(found) = coiman_core::language::css::colors(s).into_iter().next() {
        let [r, g, b, a] = found.color.srgb8();
        return Some(egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    None
}

impl CodeApp {
    pub(crate) fn pump_ext_host(&mut self, ctx: &egui::Context) {
        if !self.settings.vscode_extensions_enabled() {
            if self.ext_host.running() {
                let _ = self.ext_host.shutdown();
            }
            self.ext_views.clear();
            self.active_ext_view = None;
            if self.sidebar_view == crate::app::SidebarView::Extensions {
                self.sidebar_view = crate::app::SidebarView::Explorer;
                self.sidebar_visible = false;
            }
            if matches!(&self.active_view, Some(crate::app::views::view_tabs::ActiveView::Webview(_))) {
                self.active_view = None;
            }
            self.ext_webviews.clear();
            self.ext_decorations.clear();
            self.ext_diagnostics.clear();
            self.ext_host_at = None;
            return;
        }
        let now = Instant::now();
        let start_at = *self.ext_host_at.get_or_insert(now + Duration::from_secs(1));
        if now >= start_at && !self.ext_host.running() {
            let mut workspace_folders = Vec::new();
            if let Some(root) = &self.explorer.root {
                let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");
                workspace_folders.push(serde_json::json!({
                    "uri": format!("file://{}", root.display()),
                    "name": name,
                    "index": 0,
                }));
            }
            for (i, root) in self.explorer.extra_roots.iter().enumerate() {
                let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("workspace");
                workspace_folders.push(serde_json::json!({
                    "uri": format!("file://{}", root.display()),
                    "name": name,
                    "index": i + 1,
                }));
            }
            let config = serde_json::json!({
                "editor.tabSize": self.settings.editor.tab_size,
                "editor.insertSpaces": self.settings.editor.insert_spaces,
            });
            match self.ext_host.start_with(workspace_folders, config) {
                Ok(()) => {
                    self.status =
                        Some(format!("VS Code extension host: {} extensions", self.ext_host.extensions.len()));
                }
                Err(e) => {
                    coiman_core::applog::info(&format!("extension host: {e}"));
                }
            }
        }
        if !self.ext_host.running() {
            return;
        }
        if let Some(server) = self.ext_host.webview_server().cloned() {
            while let Ok(event) = server.events.try_recv() {
                match event {
                    coiman_core::extensions::webview::WebviewEvent::Message { id, message } => {
                        self.ext_host.notify("webview/message", serde_json::json!({ "id": id, "message": message }));
                    }
                }
            }
        }
        for doc in &self.docs {
            let Some(path) = doc.path.as_deref() else { continue };
            let Some(uri) = coiman_core::language::lsp::convert::uri_from_path(path) else { continue };
            let Some(language) = language_id(&doc.extension()) else { continue };
            self.ext_host.document(uri.as_str(), language, &doc.text(), doc.buffer.version());
        }
        if let Some(doc) = self.docs.get(self.active)
            && let Some(path) = doc.path.as_deref()
            && let Some(uri) = coiman_core::language::lsp::convert::uri_from_path(path)
        {
            let sel = doc.buffer.primary();
            let rope = doc.buffer.rope();
            let start = coiman_core::language::lsp::convert::position_of(rope, sel.anchor.min(sel.head));
            let end = coiman_core::language::lsp::convert::position_of(rope, sel.anchor.max(sel.head));
            self.ext_host.set_selection(uri.as_str(), start.line, start.character, end.line, end.character);
        }
        for message in self.ext_host.poll() {
            if message.method.is_empty()
                && let Some(id) = message.id.as_ref().and_then(serde_json::Value::as_i64)
                && let Some(request) = self.ext_requests.remove(&id)
            {
                match request {
                    crate::app::extensions::host::Request::Completion { uid } => {
                        let mut items: Vec<coiman_core::language::lsp::types::CompletionItem> = Vec::new();
                        let values: Vec<serde_json::Value> = match &message.params {
                            serde_json::Value::Array(values) => values.clone(),
                            serde_json::Value::Object(object) => {
                                object.get("items").and_then(serde_json::Value::as_array).cloned().unwrap_or_default()
                            }
                            _ => Vec::new(),
                        };
                        for value in values {
                            let label =
                                value.get("label").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned();
                            if label.is_empty() {
                                continue;
                            }
                            let item = serde_json::from_value(value.clone()).unwrap_or_else(|_| {
                                coiman_core::language::lsp::types::CompletionItem::new_simple(label, String::new())
                            });
                            items.push(item);
                        }
                        self.add_companion_completions(uid, items);
                    }
                    crate::app::extensions::host::Request::Hover { uid, pos } => {
                        let mut markdown = String::new();
                        let contents = message.params.get("contents");
                        let values: Vec<&serde_json::Value> = match contents {
                            Some(serde_json::Value::Array(values)) => values.iter().collect(),
                            Some(value) => vec![value],
                            None => Vec::new(),
                        };
                        for value in values {
                            let text = value
                                .as_str()
                                .map(str::to_owned)
                                .or_else(|| value.get("value").and_then(serde_json::Value::as_str).map(str::to_owned))
                                .unwrap_or_default();
                            if !text.is_empty() {
                                if !markdown.is_empty() {
                                    markdown.push_str("\n\n");
                                }
                                markdown.push_str(&text);
                            }
                        }
                        if !markdown.is_empty() {
                            self.add_host_hover(uid, pos, markdown);
                        }
                    }
                    crate::app::extensions::host::Request::Format { uid, save } => {
                        let edits: Vec<coiman_core::language::lsp::types::TextEdit> =
                            serde_json::from_value(message.params).unwrap_or_default();
                        self.apply_host_format(uid, edits, save);
                    }
                    crate::app::extensions::host::Request::Tree { view } => {
                        let items: Vec<(String, String)> = message
                            .params
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|item| {
                                Some((
                                    item.get("label").and_then(serde_json::Value::as_str)?.to_owned(),
                                    item.get("description")
                                        .and_then(serde_json::Value::as_str)
                                        .unwrap_or("")
                                        .to_owned(),
                                ))
                            })
                            .collect();
                        self.ext_tree.insert(view, items);
                        self.ext_tree_pending.clear();
                    }
                    crate::app::extensions::host::Request::Locations { title, word } => {
                        self.on_locations(title, &word, message.params);
                    }
                }
                continue;
            }
            match message.method.as_str() {
                "ready" => {
                    let count = message.params["extensions"].as_u64().unwrap_or(0);
                    coiman_core::applog::info(&format!("extension host: ready with {count} extensions"));
                    self.ext_views = self.ext_host.all_view_containers();
                }
                "message" => {
                    let level = message.params["level"].as_str().unwrap_or("info");
                    let text = message.params["text"].as_str().unwrap_or_default().to_owned();
                    self.status = Some(text.clone());
                    coiman_core::applog::info(&format!("extension ({level}): {text}"));
                }
                "output" => {
                    let name = message.params["name"].as_str().unwrap_or("extension").to_owned();
                    let text = message.params["text"].as_str().unwrap_or_default().trim().to_owned();
                    if !text.is_empty() {
                        coiman_core::applog::info(&format!("extension {name}: {text}"));
                        self.ext_outputs.entry(name).or_default().push(text);
                    }
                }
                "log" => {
                    let text = message.params["text"].as_str().unwrap_or_default().to_owned();
                    coiman_core::applog::info(&format!("extension host: {text}"));
                }
                "open-external" => {
                    if let Some(url) = message.params["uri"].as_str() {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
                "editor/open" => {
                    if let Ok(url) =
                        coiman_core::language::lsp::types::Url::parse(message.params["uri"].as_str().unwrap_or(""))
                        && let Some(path) = coiman_core::language::lsp::convert::path_from_uri(&url)
                    {
                        self.open_file(&path);
                    }
                }
                "diagnostics" => {
                    let url =
                        coiman_core::language::lsp::types::Url::parse(message.params["uri"].as_str().unwrap_or(""))
                            .ok();
                    if let Some(path) = url.as_ref().and_then(coiman_core::language::lsp::convert::path_from_uri) {
                        let mut found: Vec<coiman_core::language::lsp::types::Diagnostic> =
                            serde_json::from_value(message.params["diagnostics"].clone()).unwrap_or_default();
                        let source = message.params["name"].as_str().unwrap_or("extension").to_owned();
                        for diagnostic in &mut found {
                            diagnostic.source.get_or_insert_with(|| source.clone());
                        }
                        self.ext_diagnostics.insert(path, found);
                    }
                }
                "workspace/applyEdit" => {
                    if let Some(edits) = message.params.get("edits").and_then(serde_json::Value::as_array) {
                        let mut per_file: std::collections::HashMap<
                            std::path::PathBuf,
                            Vec<coiman_core::language::lsp::types::TextEdit>,
                        > = std::collections::HashMap::new();
                        for entry in edits {
                            let Some(pair) = entry.as_array() else { continue };
                            if pair.len() < 2 {
                                continue;
                            };
                            let Some(uri_str) = pair[0].as_str() else { continue };
                            let Ok(url) = coiman_core::language::lsp::types::Url::parse(uri_str) else { continue };
                            let Some(path) = coiman_core::language::lsp::convert::path_from_uri(&url) else { continue };
                            let change = &pair[1];
                            let Some(range_val) = change.get("range") else { continue };
                            let Some(new_text) = change.get("newText").and_then(serde_json::Value::as_str) else {
                                continue;
                            };
                            let Ok(range) =
                                serde_json::from_value::<coiman_core::language::lsp::types::Range>(range_val.clone())
                            else {
                                continue;
                            };
                            per_file.entry(path).or_default().push(coiman_core::language::lsp::types::TextEdit {
                                range,
                                new_text: new_text.to_owned(),
                            });
                        }
                        for (path, file_edits) in per_file {
                            if let Some(idx) = self.docs.iter().position(|d| d.path.as_deref() == Some(&path)) {
                                self.apply_text_edits(idx, file_edits);
                            }
                        }
                    }
                }
                "statusBar/update" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str) {
                        let text =
                            message.params.get("text").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                        let tooltip =
                            message.params.get("tooltip").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                        let command =
                            message.params.get("command").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                        self.ext_status_bar.insert(id.to_owned(), (text, tooltip, command));
                    }
                }
                "statusBar/hide" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str) {
                        self.ext_status_bar.remove(id);
                    }
                }
                "tree/changed" => {
                    if let Some(view_id) = message.params.get("viewId").and_then(serde_json::Value::as_str) {
                        self.ext_tree.remove(view_id);
                        self.ext_tree_pending.remove(view_id);
                    }
                }
                "workspace/fileCreated"
                | "workspace/fileDeleted"
                | "workspace/fileRenamed"
                | "workspace/fileChanged" => {
                    self.explorer.refresh();
                    self.finder.mark_stale();
                }
                "terminal/create" => {
                    let id = message.params.get("id").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                    let name = message.params.get("name").and_then(serde_json::Value::as_str).unwrap_or("Terminal");
                    let cwd =
                        message.params.get("cwd").and_then(serde_json::Value::as_str).map(std::path::PathBuf::from);
                    let target_dir = cwd.unwrap_or_else(|| {
                        self.explorer.root.clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                    });
                    if self.terminal.new_tab_in(ctx, "", &[], 10_000, Some(name.to_owned()), &target_dir).is_ok()
                        && let Some(tab_id) = self.terminal.last_tab_id()
                    {
                        self.ext_terminals.insert(id, tab_id);
                    }
                }
                "terminal/sendText" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str)
                        && let Some(&tab_id) = self.ext_terminals.get(id)
                        && let Some(text) = message.params.get("text").and_then(serde_json::Value::as_str)
                    {
                        let add_new_line =
                            message.params.get("addNewLine").and_then(serde_json::Value::as_bool).unwrap_or(true);
                        let input = if add_new_line { format!("{text}\n") } else { text.to_owned() };
                        self.terminal.type_into(tab_id, &input);
                    }
                }
                "terminal/show" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str)
                        && let Some(&tab_id) = self.ext_terminals.get(id)
                    {
                        self.terminal.show_tab(tab_id);
                        self.panel_visible = true;
                        self.panel_tab = crate::app::PanelTab::Terminal;
                    }
                }
                "terminal/hide" => {
                    if self.panel_tab == crate::app::PanelTab::Terminal {
                        self.panel_visible = false;
                    }
                }
                "terminal/dispose" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str)
                        && let Some(tab_id) = self.ext_terminals.remove(id)
                    {
                        self.terminal.close_tab(tab_id);
                    }
                }
                "webview/create" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str) {
                        let title = message
                            .params
                            .get("title")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("Webview")
                            .to_owned();
                        self.ext_webviews.insert(id.to_owned(), (title, String::new()));
                        self.active_view = Some(crate::app::views::view_tabs::ActiveView::Webview(id.to_owned()));
                        if let Some(server) = self.ext_host.webview_server().cloned() {
                            let url = server.url_for(id);
                            ctx.open_url(egui::OpenUrl::new_tab(&url));
                        }
                    }
                }
                "webview/setHtml" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str)
                        && let Some(html) = message.params.get("html").and_then(serde_json::Value::as_str)
                    {
                        if let Some(entry) = self.ext_webviews.get_mut(id) {
                            entry.1 = html.to_owned();
                        }
                        if let Some(server) = self.ext_host.webview_server().cloned() {
                            server.set_html(id, html);
                        }
                    }
                }
                "webview/reveal" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str) {
                        self.active_view = Some(crate::app::views::view_tabs::ActiveView::Webview(id.to_owned()));
                        if let Some(server) = self.ext_host.webview_server().cloned() {
                            let url = server.url_for(id);
                            ctx.open_url(egui::OpenUrl::new_tab(&url));
                        }
                    }
                }
                "webview/dispose" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str) {
                        self.ext_webviews.remove(id);
                        if let Some(server) = self.ext_host.webview_server().cloned() {
                            server.remove_panel(id);
                        }
                        if matches!(&self.active_view, Some(crate::app::views::view_tabs::ActiveView::Webview(open)) if open.as_str() == id)
                        {
                            self.active_view = None;
                        }
                    }
                }
                "webview/postMessage" => {
                    if let Some(id) = message.params.get("id").and_then(serde_json::Value::as_str)
                        && let Some(msg) = message.params.get("message")
                        && let Some(server) = self.ext_host.webview_server().cloned()
                    {
                        server.post_message(id, msg);
                    }
                }
                "clipboard/copySelection" => {
                    self.copy_selection_to_clipboard(ctx);
                }
                "decoration/create" => {
                    let key = message.params.get("key").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                    let options = message.params.get("options");
                    let color = options
                        .and_then(|o| o.get("color"))
                        .and_then(serde_json::Value::as_str)
                        .and_then(parse_ext_color);
                    let background_color = options
                        .and_then(|o| o.get("backgroundColor"))
                        .and_then(serde_json::Value::as_str)
                        .and_then(parse_ext_color);
                    let is_whole_line = options
                        .and_then(|o| o.get("isWholeLine"))
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    self.ext_decoration_types.insert(key, ExtDecorationType { color, background_color, is_whole_line });
                }
                "window/setDecorations" => {
                    let key = message.params.get("key").and_then(serde_json::Value::as_str).unwrap_or("").to_owned();
                    let uri_str = message.params.get("uri").and_then(serde_json::Value::as_str).unwrap_or("");
                    if let Ok(url) = coiman_core::language::lsp::types::Url::parse(uri_str)
                        && let Some(path) = coiman_core::language::lsp::convert::path_from_uri(&url)
                        && let Some(deco_type) = self.ext_decoration_types.get(&key).cloned()
                    {
                        let mut by_line: std::collections::HashMap<usize, Vec<crate::ui::editor_view::Highlight>> =
                            std::collections::HashMap::new();
                        if let Some(ranges) = message.params.get("ranges").and_then(serde_json::Value::as_array) {
                            for item in ranges {
                                let range_val = if item.get("range").is_some() { &item["range"] } else { item };
                                let Ok(range) = serde_json::from_value::<coiman_core::language::lsp::types::Range>(
                                    range_val.clone(),
                                ) else {
                                    continue;
                                };
                                let start_line = range.start.line as usize;
                                let end_line = range.end.line as usize;
                                for line in start_line..=end_line {
                                    let start_col = if line == start_line { range.start.character as usize } else { 0 };
                                    let end_col =
                                        if line == end_line { range.end.character as usize } else { usize::MAX / 2 };
                                    by_line.entry(line).or_default().push(crate::ui::editor_view::Highlight {
                                        start: start_col,
                                        end: if deco_type.is_whole_line { usize::MAX / 2 } else { end_col },
                                        foreground: deco_type.color,
                                        background: deco_type.background_color,
                                        italic: false,
                                        strike: false,
                                        frame: None,
                                    });
                                }
                            }
                        }
                        self.ext_decorations.insert((key, path), std::sync::Arc::new(by_line));
                    }
                }
                "decoration/dispose" => {
                    if let Some(key) = message.params.get("key").and_then(serde_json::Value::as_str) {
                        self.ext_decoration_types.remove(key);
                        self.ext_decorations.retain(|(k, _), _| k != key);
                    }
                }
                _ => {}
            }
        }
        let _ = ctx;
    }

    pub(crate) fn copy_selection_to_clipboard(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let sel = doc.buffer.primary();
        let text = if sel.is_empty() {
            let line = doc.buffer.line_col(sel.head).0;
            doc.buffer.line_text(line)
        } else {
            let (start, end) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
            doc.buffer.slice_string(start, end)
        };
        if !text.is_empty() {
            ctx.copy_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::CodeApp;
    use eframe::egui;

    #[test]
    fn the_extension_host_does_not_start_during_startup() {
        let ctx = egui::Context::default();
        let app = CodeApp::new(None, &ctx);
        assert!(!app.ext_host.running(), "startup must not launch Node");
        let mut app = app;
        app.pump_ext_host(&ctx);
        assert!(!app.ext_host.running(), "the first frame still must not launch Node");
    }

    #[test]
    fn vscode_extensions_disabled_by_default_in_app() {
        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        assert!(!app.settings.vscode_extensions_enabled());
        app.pump_ext_host(&ctx);
        assert!(!app.ext_host.running());
        assert!(app.ext_views.is_empty());
        assert!(app.ext_webviews.is_empty());
        assert!(app.plugin_commands().iter().all(|(_, k)| !k.starts_with("ext\n")));
    }
}
