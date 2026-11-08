//! Live preview: shows a page in the user's own browser, following edits as they
//! happen, and brings what the page reports back here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use coiman_core::language::lsp::types as lt;
use coiman_core::launch::preview::{PreviewEvent, PreviewServer};
use eframe::egui::{self, RichText};

use crate::app::{CodeApp, PanelTab, repaint};
use crate::ui::theme;

/// Seconds between passing unsaved edits to the page.
const SYNC_EVERY: f64 = 0.25;
/// Console lines kept, newest last.
const CONSOLE_LINES: usize = 2000;

struct ConsoleLine {
    level: String,
    text: String,
    place: Option<(PathBuf, u32, u32)>,
}

#[derive(Default)]
pub(crate) struct Preview {
    pub(crate) server: Option<PreviewServer>,
    console: Vec<ConsoleLine>,
    /// Errors from the page's scripts: (file, diagnostic).
    problems: Vec<(PathBuf, lt::Diagnostic)>,
    /// The buffer version last passed on, for files with unsaved changes.
    sent: HashMap<u64, (PathBuf, u64)>,
    next_sync: f64,
}

fn is_page(file: &Path) -> bool {
    file.extension().is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
}

impl CodeApp {
    /// Whether Run on the active file means showing it in the browser.
    pub(crate) fn active_is_page(&self) -> bool {
        self.docs.get(self.active).and_then(|d| d.path.as_deref()).is_some_and(is_page)
    }

    /// Opens the active page, or the folder's `index.html`, in the browser.
    pub(crate) fn open_live_preview(&mut self, ctx: &egui::Context) {
        let active = self.docs.get(self.active).and_then(|d| d.path.clone());
        let root = self.explorer.root.clone();
        let page = active.clone().filter(|p| is_page(p)).or_else(|| {
            let folder = root.clone().or_else(|| active.as_deref().and_then(Path::parent).map(Path::to_path_buf))?;
            Some(folder.join("index.html")).filter(|p| p.is_file())
        });
        let Some(page) = page else {
            self.refuse_run(ctx, "Live preview needs an HTML file: open one, or add index.html".to_owned());
            return;
        };
        let folder = root
            .filter(|r| page.starts_with(r))
            .or_else(|| page.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| page.clone());
        if !self.allowed_to_run(&folder) {
            return;
        }
        // A folder that doesn't hold this page needs a server of its own.
        let serves_it = self.preview.server.as_ref().is_some_and(|s| s.url_of(&page).is_some());
        if !serves_it {
            self.stop_live_preview();
            let port = self.settings.workbench.preview_port;
            match PreviewServer::start(&folder, port, repaint(ctx)) {
                Ok(server) => self.preview.server = Some(server),
                Err(e) => {
                    self.refuse_run(ctx, format!("Could not start the live preview: {e}"));
                    return;
                }
            }
        }
        self.preview.next_sync = 0.0;
        self.sync_preview(ctx);
        let Some(url) = self.preview.server.as_ref().and_then(|s| s.url_of(&page)) else { return };
        self.status = Some(format!("Live preview: {url}"));
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }

    pub(crate) fn stop_live_preview(&mut self) {
        self.preview.server = None;
        self.preview.problems.clear();
        self.preview.sent.clear();
        if self.status.as_deref().is_some_and(|s| s.starts_with("Live preview:")) {
            self.status = None;
        }
        if self.panel_tab == PanelTab::PageConsole {
            self.panel_tab = PanelTab::Terminal;
        }
    }

    pub(crate) fn preview_address(&self) -> Option<String> {
        self.preview.server.as_ref().map(|s| format!("http://127.0.0.1:{}/", s.port()))
    }

    pub(crate) fn preview_running(&self) -> bool {
        self.preview.server.is_some()
    }

    /// What the page's scripts reported about `file` (a [`crate::app::lsp::path_key`]).
    pub(crate) fn preview_problems(&self, key: &Path) -> Vec<lt::Diagnostic> {
        let found = self.preview.problems.iter().filter(|(p, _)| crate::app::lsp::path_key(p) == key);
        found.map(|(_, d)| d.clone()).collect()
    }

    /// Files the page's scripts reported about: (key, path).
    pub(crate) fn preview_problem_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        self.preview.problems.iter().map(|(p, _)| (crate::app::lsp::path_key(p), p.clone())).collect()
    }

    pub(crate) fn pump_preview(&mut self, ctx: &egui::Context) {
        if self.preview.server.is_none() {
            return;
        }
        let mut events = Vec::new();
        if let Some(server) = &self.preview.server {
            while let Ok(event) = server.events.try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.on_preview_event(ctx, event);
        }
        self.sync_preview(ctx);
    }

    fn on_preview_event(&mut self, ctx: &egui::Context, event: PreviewEvent) {
        match event {
            PreviewEvent::Reloading => self.preview.problems.clear(),
            PreviewEvent::Reveal { file, line } => {
                let at = lt::Position::new(line.saturating_sub(1), 0);
                self.jump_to(&file, lt::Range::new(at, at), false);
                if let Some(doc) = self.docs.get_mut(self.active) {
                    doc.request_focus = true;
                    doc.reveal = true;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            PreviewEvent::Console { level, text, place } => {
                // Compiled TypeScript's lines don't match the source, so only
                // scripts served as written can be placed.
                let exact = place.clone().filter(|(file, line, _)| {
                    *line > 0
                        && !file.extension().is_some_and(|e| matches!(e.to_str(), Some("ts" | "mts" | "tsx" | "jsx")))
                });
                if let Some((file, line, col)) = exact {
                    let at = lt::Position::new(line - 1, col.saturating_sub(1));
                    let until = lt::Position::new(line - 1, col);
                    let problem = lt::Diagnostic {
                        range: lt::Range::new(at, until),
                        severity: Some(lt::DiagnosticSeverity::ERROR),
                        source: Some("page".to_owned()),
                        message: text.clone(),
                        ..Default::default()
                    };
                    // Each open tab of the page reports the same error.
                    if !self.preview.problems.iter().any(|(f, d)| *f == file && *d == problem) {
                        self.preview.problems.push((file, problem));
                    }
                }
                self.preview.console.push(ConsoleLine { level, text, place });
                let extra = self.preview.console.len().saturating_sub(CONSOLE_LINES);
                self.preview.console.drain(..extra);
            }
        }
    }

    /// Passes unsaved text to the server, so the page shows what's on screen.
    fn sync_preview(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let mut waiting = false;
        if now >= self.preview.next_sync {
            self.preview.next_sync = now + SYNC_EVERY;
            let Some(server) = &self.preview.server else { return };
            let mut live = Vec::new();
            for doc in &self.docs {
                let Some(path) = doc.path.as_ref().filter(|_| doc.is_dirty()) else { continue };
                live.push(doc.uid);
                let version = doc.buffer.version();
                if self.preview.sent.get(&doc.uid).is_none_or(|(_, v)| *v != version) {
                    server.set_overlay(path, Some(doc.text()));
                    self.preview.sent.insert(doc.uid, (path.clone(), version));
                }
            }
            // Saved or closed: the file on disk speaks for itself again.
            self.preview.sent.retain(|uid, (path, _)| {
                let keep = live.contains(uid);
                if !keep {
                    server.set_overlay(path, None);
                }
                keep
            });
        } else {
            waiting = self
                .docs
                .iter()
                .any(|d| d.is_dirty() && self.preview.sent.get(&d.uid).is_none_or(|(_, v)| *v != d.buffer.version()));
        }
        if waiting {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(SYNC_EVERY));
        }
    }

    /// The panel listing what the page printed.
    pub(crate) fn page_console(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let mut open: Option<(PathBuf, u32)> = None;
        ui.horizontal(|ui| {
            if let Some(address) = self.preview_address() {
                ui.label(RichText::new(address).small().color(pal.text_dim));
            }
            ui.label(RichText::new("Alt+click an element in the page to find it here").small().color(pal.text_dim));
            if ui.small_button("Clear").clicked() {
                self.preview.console.clear();
            }
        });
        let font = egui::FontId::monospace(self.font_size * 0.9);
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if self.preview.console.is_empty() {
                ui.label("What the page prints with console.log shows up here.");
            }
            for line in &self.preview.console {
                let color = match line.level.as_str() {
                    "error" => egui::Color32::from_rgb(0xF1, 0x4C, 0x4C),
                    "warn" => egui::Color32::from_rgb(0xCC, 0xA7, 0x00),
                    "debug" => pal.text_dim,
                    _ => pal.text,
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&line.text).font(font.clone()).color(color));
                    if let Some((file, at, _)) = line.place.as_ref().filter(|(_, at, _)| *at > 0) {
                        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        if ui.link(RichText::new(format!("{name}:{at}")).small()).clicked() {
                            open = Some((file.clone(), *at));
                        }
                    }
                });
            }
        });
        if let Some((file, line)) = open {
            let at = lt::Position::new(line.saturating_sub(1), 0);
            self.jump_to(&file, lt::Range::new(at, at), false);
        }
    }
}
