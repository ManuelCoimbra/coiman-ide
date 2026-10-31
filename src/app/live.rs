//! Live sessions: a panel attached to a running system, code sent from the editor
//! with its answer beside it, and what's inside the system in an inspector.
//! Nothing here waits on the system: slow code leaves the editor as it was.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Color32, RichText, TextEdit};
use rust_code_core::live::{self, Eval, Identity, InspectRow, LiveEvent, Nrepl, Session};

use super::{CodeApp, PanelTab, repaint};
use crate::inspector;
use crate::theme;

/// What's kept of a session's conversation.
const TRANSCRIPT_LINES: usize = 2000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Input,
    Output,
    Error,
    Value,
    Note,
}

/// Where an answer goes.
struct InEditor {
    uid: u64,
    line: usize,
    line_text: String,
}

#[derive(Default)]
pub(super) struct Live {
    session: Option<Box<dyn Session>>,
    /// A connection on its way: a dead port can take seconds to say so.
    connecting: Option<Receiver<Result<Nrepl, String>>>,
    identity: Option<Identity>,
    address: String,
    namespace: String,
    input: String,
    transcript: Vec<(Kind, String)>,
    problem: Option<String>,
    /// Requests still running, oldest first.
    running: Vec<u64>,
    in_editor: HashMap<u64, InEditor>,
    /// The inspector: rows by what asked for them (`None` is the top).
    rows: HashMap<Option<String>, Vec<InspectRow>>,
    asked: HashMap<u64, Option<String>>,
    open: HashSet<String>,
    inspect_problem: Option<String>,
    used: bool,
}

fn add_text(transcript: &mut Vec<(Kind, String)>, kind: Kind, text: &str) {
    for line in text.trim_end_matches('\n').split('\n') {
        transcript.push((kind, line.trim_end_matches('\r').to_owned()));
    }
    if transcript.len() > TRANSCRIPT_LINES {
        transcript.drain(..transcript.len() - TRANSCRIPT_LINES);
    }
}

fn is_clojure(extension: &str) -> bool {
    matches!(extension, "clj" | "cljs" | "cljc" | "edn" | "bb")
}

impl CodeApp {
    pub(super) fn live_tab_shown(&self) -> bool {
        self.live.used
    }

    /// `Live: Show Session`.
    pub(super) fn show_live(&mut self) {
        self.live.used = true;
        if self.live.address.is_empty()
            && let Some(root) = &self.explorer.root
        {
            self.live.address = live::nrepl_port_file(root).unwrap_or_default();
        }
        self.panel_tab = PanelTab::Live;
        self.panel_visible = true;
    }

    fn live_connect(&mut self, ctx: &egui::Context) {
        let address = self.live.address.trim().to_owned();
        self.live.problem = None;
        // This session runs what's sent to it, so the folder has to be trusted first.
        if let Some(root) = self.explorer.root.clone()
            && !self.allowed_to_run(&root)
        {
            return;
        }
        add_text(&mut self.live.transcript, Kind::Note, &format!("Connecting to {address}"));
        let (tx, rx) = mpsc::channel();
        let (wake, done) = (repaint(ctx), ctx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Nrepl::connect(&address, wake));
            done.request_repaint();
        });
        self.live.connecting = Some(rx);
    }

    fn live_disconnect(&mut self) {
        self.live.session = None;
        self.live.identity = None;
        self.live.running.clear();
        add_text(&mut self.live.transcript, Kind::Note, "Disconnected. The server keeps running.");
    }

    pub(super) fn pump_live(&mut self) {
        if let Some(result) = self.live.connecting.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.live.connecting = None;
            match result {
                Ok(session) => {
                    self.live.session = Some(Box::new(session));
                    self.live.rows.clear();
                    self.live.open.clear();
                    self.live.running.clear();
                    self.live.namespace.clear();
                }
                Err(why) => {
                    add_text(&mut self.live.transcript, Kind::Error, &why);
                    self.live.problem = Some(why);
                }
            }
        }
        let Some(session) = &mut self.live.session else { return };
        let events = session.poll();
        let state = &mut self.live;
        let mut refresh = false;
        for event in events {
            match event {
                LiveEvent::Connected(identity) => {
                    add_text(&mut state.transcript, Kind::Note, &format!("Attached to {}", identity.system));
                    state.identity = Some(identity);
                    refresh = true;
                }
                LiveEvent::Output { text, error, .. } => {
                    add_text(&mut state.transcript, if error { Kind::Error } else { Kind::Output }, &text);
                }
                LiveEvent::Value { id, text } => {
                    add_text(&mut state.transcript, Kind::Value, &text);
                    if let Some(place) = state.in_editor.get(&id) {
                        let answer = text.lines().next().unwrap_or_default().to_owned();
                        self.repls.set_answer(place.uid, place.line, place.line_text.clone(), answer);
                    }
                }
                LiveEvent::Failed { id, text } => {
                    add_text(&mut state.transcript, Kind::Error, &text);
                    if let Some(place) = state.in_editor.get(&id) {
                        self.repls.set_answer(place.uid, place.line, place.line_text.clone(), text);
                    }
                }
                LiveEvent::Done { id, namespace } => {
                    state.running.retain(|r| *r != id);
                    state.in_editor.remove(&id);
                    if let Some(namespace) = namespace {
                        state.namespace = namespace;
                    }
                    refresh = true;
                }
                LiveEvent::Rows { id, rows, problem } => {
                    if problem.is_some() {
                        state.inspect_problem = problem;
                    }
                    if let Some(query) = state.asked.remove(&id) {
                        state.rows.insert(query, rows);
                    }
                }
                LiveEvent::Closed(why) => {
                    add_text(&mut state.transcript, Kind::Error, &why);
                    state.problem = Some(why);
                    state.session = None;
                    state.identity = None;
                    state.running.clear();
                    state.in_editor.clear();
                    return;
                }
            }
        }
        // What's inside may have changed: what's open is asked for again.
        if refresh {
            self.live_refresh();
        }
    }

    fn live_refresh(&mut self) {
        let mut queries: Vec<Option<String>> = vec![None];
        queries.extend(self.live.open.iter().cloned().map(Some));
        for query in queries {
            self.live_ask(query);
        }
    }

    fn live_ask(&mut self, query: Option<String>) {
        let namespace = self.live.namespace.clone();
        let Some(session) = &mut self.live.session else { return };
        let namespace = (!namespace.is_empty()).then_some(namespace);
        let id = session.inspect(query.as_deref(), namespace.as_deref());
        self.live.asked.insert(id, query);
    }

    /// Sends code from the editor if there's a session for it. Says whether it did.
    pub(super) fn send_to_live(&mut self) -> bool {
        if self.live.session.is_none() {
            return false;
        }
        let Some(doc) = self.docs.get(self.active) else { return false };
        if !is_clojure(&doc.extension()) {
            return false;
        }
        let text = doc.text();
        let primary = doc.buffer.primary();
        let rope = doc.buffer.rope();
        let head_line = rope.char_to_line(primary.head);
        // The selection; else the top-level form around the cursor; else the line.
        let (code, first, last) = if primary.is_empty() {
            match live::top_level_form(&text, head_line) {
                Some((first, last)) => {
                    let end = doc.buffer.line_end(last);
                    (doc.buffer.slice_string(doc.buffer.line_start(first), end), first, last)
                }
                None => (doc.buffer.line_text(head_line), head_line, head_line),
            }
        } else {
            let last = rope.char_to_line(primary.end().saturating_sub(1).max(primary.start()));
            (doc.buffer.slice_string(primary.start(), primary.end()), rope.char_to_line(primary.start()), last)
        };
        if code.trim().is_empty() {
            return true;
        }
        let place = InEditor { uid: doc.uid, line: last, line_text: doc.buffer.line_text(last) };
        let request = Eval {
            code: code.clone(),
            file: doc.path.clone(),
            line: Some(first as u32 + 1),
            namespace: if live::defines_namespace(&code) { None } else { live::namespace_of(&text) },
        };
        self.repls.set_answer(place.uid, place.line, place.line_text.clone(), "…".to_owned());
        self.live_send(request, Some(place));
        self.live.used = true;
        true
    }

    fn live_send(&mut self, request: Eval, place: Option<InEditor>) {
        add_text(&mut self.live.transcript, Kind::Input, &request.code);
        let Some(session) = &mut self.live.session else { return };
        let id = session.eval(request);
        self.live.running.push(id);
        if let Some(place) = place {
            self.live.in_editor.insert(id, place);
        }
    }

    /// `Live: Load File`.
    pub(super) fn live_load_file(&mut self) {
        if self.live.session.is_none() {
            self.status = Some("Connect a live session first".to_owned());
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let (code, file): (String, Option<PathBuf>) = (doc.text(), doc.path.clone());
        // What was said about these lines before no longer counts.
        self.repls.clear_answers(doc.uid);
        self.live_send(Eval { code, file, line: Some(1), namespace: None }, None);
    }

    /// The status bar's reminder that this isn't a scratch copy.
    pub(super) fn live_label(&self) -> Option<(String, String)> {
        let identity = self.live.identity.as_ref()?;
        let busy = if self.live.running.is_empty() { "" } else { " (running)" };
        let tip = format!("Attached to {}. What you send runs in that live system.", identity.system);
        let namespace = &self.live.namespace;
        let place = if namespace.is_empty() { String::new() } else { format!(": {namespace}") };
        Some((format!("Live{place}{busy}"), tip))
    }

    fn inspector_rows(
        ui: &mut egui::Ui,
        state: &Live,
        query: Option<String>,
        depth: usize,
        mono: &egui::FontId,
        toggle: &mut Option<String>,
    ) {
        let pal = theme::colors();
        let Some(rows) = state.rows.get(&query) else {
            ui.label(RichText::new("…").color(pal.text_dim));
            return;
        };
        if rows.is_empty() && depth == 0 {
            ui.label(RichText::new("No values defined here yet.").color(pal.text_dim));
        }
        for row in rows {
            let open = row.children.as_ref().is_some_and(|c| state.open.contains(c));
            let shown = inspector::Row {
                name: &row.name,
                value: &row.value,
                type_name: &row.type_name,
                depth,
                expandable: row.children.is_some(),
                open,
            };
            if inspector::row(ui, &shown, mono, |_| {}).clicked()
                && let Some(children) = &row.children
            {
                *toggle = Some(children.clone());
            }
            if open {
                Self::inspector_rows(ui, state, row.children.clone(), depth + 1, mono, toggle);
            }
        }
    }

    pub(super) fn live_panel(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
        let mut connect = false;
        let mut disconnect = false;
        let mut interrupt = false;
        let mut send = None;
        let connected = self.live.session.is_some();
        let running = !self.live.running.is_empty();
        let can_interrupt = self.live.identity.as_ref().is_some_and(|i| i.can("interrupt"));
        ui.horizontal(|ui| {
            match &self.live.identity {
                Some(identity) => {
                    ui.label(RichText::new(format!("Attached to {}", identity.system)).strong());
                }
                None if connected || self.live.connecting.is_some() => {
                    ui.label(RichText::new("Connecting…").color(pal.text_dim));
                }
                None => {
                    let edit = TextEdit::singleline(&mut self.live.address)
                        .hint_text("nREPL host:port on this computer")
                        .desired_width(240.0);
                    ui.add(edit);
                    connect = ui.button("Connect").clicked();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if connected {
                    disconnect = ui.button("Disconnect").clicked();
                }
                if running && can_interrupt {
                    interrupt = ui.button("Interrupt").clicked();
                }
            });
        });
        if self.live.identity.is_some() {
            ui.label(RichText::new("This is a running system: what you send changes it.").color(warn));
        }
        if let Some(problem) = &self.live.problem {
            ui.label(RichText::new(problem).color(warn));
        }
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            let half = ui.available_width() / 2.0;
            ui.vertical(|ui| {
                ui.set_width(half);
                ui.set_height(height);
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                let room = (height - 30.0).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt("live_log")
                    .max_height(room)
                    .stick_to_bottom(true)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (kind, line) in &self.live.transcript {
                            let color = match kind {
                                Kind::Input => pal.accent,
                                Kind::Output => pal.editor_fg,
                                Kind::Error => Color32::from_rgb(0xF1, 0x4C, 0x4C),
                                Kind::Value => Color32::from_rgb(0x89, 0xD1, 0x85),
                                Kind::Note => pal.text_dim,
                            };
                            ui.add(egui::Label::new(RichText::new(line).font(mono.clone()).color(color)).wrap());
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}=>", self.live.namespace)).font(mono.clone()).color(pal.text_dim),
                    );
                    let edit =
                        TextEdit::singleline(&mut self.live.input).font(mono.clone()).desired_width(f32::INFINITY);
                    let response = ui.add_enabled(connected, edit);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        send = Some(std::mem::take(&mut self.live.input));
                        response.request_focus();
                    }
                });
            });
            ui.separator();
            ui.vertical(|ui| {
                let mono = egui::TextStyle::Monospace.resolve(ui.style());
                ui.label(RichText::new("INSPECTOR").small().strong());
                if let Some(problem) = &self.live.inspect_problem {
                    ui.add(egui::Label::new(RichText::new(problem).small().color(warn)).truncate());
                }
                let mut toggle = None;
                egui::ScrollArea::both().id_salt("live_inspect").auto_shrink([false, false]).show(ui, |ui| {
                    if connected {
                        Self::inspector_rows(ui, &self.live, None, 0, &mono, &mut toggle);
                    }
                });
                if let Some(children) = toggle {
                    if self.live.open.remove(&children) {
                        self.live.rows.remove(&Some(children));
                    } else {
                        self.live.open.insert(children.clone());
                        self.live_ask(Some(children));
                    }
                }
            });
        });
        if connect {
            self.live_connect(&ctx);
        }
        if disconnect {
            self.live_disconnect();
        }
        if interrupt && let Some(session) = &mut self.live.session {
            session.interrupt();
        }
        if let Some(code) = send.filter(|c| !c.trim().is_empty()) {
            self.live_send(Eval { code, ..Eval::default() }, None);
        }
    }
}
