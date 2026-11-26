//! The chat view (the Agents panel): a conversation with an agent, filled by an extension.
//!
//! Coiman has no AI of its own. The Agents extension runs `opencode` in the open folder and
//! fills this view through the plugin API (`set-chat`): the transcript, sessions, models and
//! choices. The view keeps the box you type in, its history, queue and attachments, and sends
//! what you do back as `chat-action` events. Sessions are a full-panel picker.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use coiman_core::extensions::plugin::host::{ChatAction, ChatPart, ChatView};

use coiman_core::language::highlight;
use coiman_core::language::markdown::Inline;
use coiman_core::project::trust;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Align2, Color32, FontId, Rect, RichText, ScrollArea, Sense, TextEdit};
use serde_json::Value;

use crate::app::CodeApp;
use crate::ui::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    User,
    Agent,
}

#[derive(Clone)]
struct ToolPart {
    id: String,
    name: String,
    title: String,
    input: String,
    output: String,
    status: String,
}

#[derive(Clone)]
enum Part {
    Text(String),
    Thinking(String),
    Tool(ToolPart),
}

#[derive(Clone)]
struct Message {
    role: Role,
    parts: Vec<Part>,
}

impl From<coiman_core::extensions::plugin::host::ChatMessage> for Message {
    fn from(m: coiman_core::extensions::plugin::host::ChatMessage) -> Self {
        let parts = m
            .parts
            .into_iter()
            .map(|p| match p {
                ChatPart::Text(t) => Part::Text(t),
                ChatPart::Thinking(t) => Part::Thinking(t),
                ChatPart::Tool(t) => Part::Tool(ToolPart {
                    id: t.id,
                    name: t.name,
                    title: t.title,
                    input: t.input,
                    output: t.output,
                    status: t.status,
                }),
            })
            .collect();
        Self { role: if m.from_user { Role::User } else { Role::Agent }, parts }
    }
}

#[derive(Clone)]
struct SessionInfo {
    id: String,
    title: String,
    updated: u64,
}

/// At this width the panel becomes the reference Agents workspace: a persistent
/// sessions rail beside the conversation. Below it, the panel keeps the focused
/// Chat view instead of squeezing two unusable columns in.
///
/// The reference mock-up's fixed chrome heights. Keeping these as constants
/// makes the panel line up with the HTML at normal DPI instead of drifting by
/// a few pixels every time a label changes.
const CHAT_CONTEXT_HEIGHT: f32 = 31.0;
const SESSIONS_TOP_HEIGHT: f32 = CHAT_CONTEXT_HEIGHT;
const SESSIONS_HEADER_HEIGHT: f32 = 29.0;
const COMPOSER_MIN_HEIGHT: f32 = 109.0;

#[derive(Default)]
pub(crate) struct AgentsUi {
    pub(crate) open: bool,
    messages: Vec<Message>,
    input: String,
    session: String,
    /// The title of the current session, shown in the conversation bar.
    session_title: Option<String>,
    /// The extension filling the view, and what to send it.
    plugin: Option<String>,
    outbox: Vec<ChatAction>,
    /// Before the session's title ("OpenCode").
    name: String,
    /// The agent's program isn't installed: how to get it.
    missing: Option<String>,
    /// The status the extension gave last, so one it repeats doesn't hide the view's own.
    plugin_status: Option<String>,
    /// The agent is working.
    running: bool,
    deleting: bool,
    /// The session waiting for a delete confirmation.
    delete_pending: Option<SessionInfo>,
    /// The session being deleted, remembered so the transcript can be cleared.
    deleting_id: Option<String>,
    session_loading: bool,
    /// Files attached as context; sent as `@path` references before the message.
    attached: Vec<String>,
    /// The file open in the editor, offered as context by "Add Context…".
    active_file: Option<PathBuf>,
    root: Option<PathBuf>,
    status: Option<String>,
    models: Vec<String>,
    models_loading: bool,
    models_loaded: bool,
    models_asked: bool,
    /// The model list filtered for the currently open picker. Recomputed only
    /// when the query changes, so opening the popup does not scan every model
    /// on every frame.
    model_results: Vec<String>,
    /// The query `model_results` was made for; None when the list changed since.
    model_results_query: Option<String>,
    history: Vec<SessionInfo>,
    history_open: bool,
    history_loading: bool,
    history_loaded: bool,
    session_search_open: bool,
    session_search: String,
    model: Option<String>,
    /// The agent mode: "build" edits files, "plan" only discusses.
    mode: String,
    /// "auto" approves tool use; "manual" asks first.
    permissions: String,
    model_search: String,
    focus_input: bool,
    /// Messages typed while the agent runs, sent when it finishes.
    queue: Vec<String>,
    /// Sent prompts, cycled back with the up arrow in an empty ask box.
    prompt_history: Vec<String>,
    history_index: Option<usize>,
    /// Whether the ask box had keyboard focus last frame, for the composer border.
    input_focused: bool,
    /// Files edited in the last turn, for the working set.
    last_changed_files: Vec<String>,
    /// Tool calls in the last turn, for the "ran" status line.
    last_turn_tools: usize,
    /// Ask the app to open this file in the editor.
    pub(crate) open_request: Option<PathBuf>,
    /// Ask the app to show this file's diff.
    pub(crate) diff_request: Option<PathBuf>,
    /// How long the last turn took, for the mock-up's completion line.
    last_turn_seconds: Option<u64>,
    /// The last prompt sent, to swallow accidental double-submits.
    last_sent: Option<(String, std::time::Instant)>,
    /// Ask the app to open Settings.
    pub(crate) requests_settings: bool,
    /// The MCP servers button was clicked.
    pub(crate) requests_mcp: bool,
    /// True while the transcript is scrolled to the bottom, for the jump pill.
    at_bottom: bool,
    /// Set to scroll the transcript back to the bottom.
    jump_to_bottom: bool,
    /// Where the composer box sits on screen, for the floating jump pill.
    composer_rect: Option<egui::Rect>,
    /// The composer's measured height, so the transcript can take the rest.
    composer_height: f32,
    /// Focus the model search box on the frame its popup opens.
    focus_model_search: bool,
    /// Focus the sessions search box on the frame it opens.
    focus_session_search: bool,
    /// The panel fills the window, like a maximized terminal.
    pub(crate) maximized: bool,
}

impl AgentsUi {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    /// Whether the side-by-side sessions column is shown.
    /// Opens the panel and focuses the ask box.
    pub(crate) fn focus_chat(&mut self) {
        self.open = true;
        self.focus_input = true;
    }

    pub(crate) fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.focus_input = true;
        }
    }

    /// Asks the extension that fills the view to do something.
    fn act(&mut self, action: &str, argument: &str, attachments: Vec<String>) {
        self.outbox.push(ChatAction { action: action.to_owned(), argument: argument.to_owned(), attachments });
    }

    /// What the extension filling the view (the Agents extension) says now.
    pub(crate) fn apply(&mut self, plugin: &str, view: ChatView) {
        let was_running = self.running;
        self.plugin = Some(plugin.to_owned());
        self.name = view.name;
        self.missing = view.missing;
        self.session = view.session;
        self.session_title = Some(view.title).filter(|t| !t.is_empty());
        let from = (view.messages_from as usize).min(self.messages.len());
        self.messages.truncate(from);
        self.messages.extend(view.messages.into_iter().map(Message::from));
        self.running = view.running;
        self.session_loading = view.loading;
        if !view.status.is_empty() && self.plugin_status.as_deref() != Some(view.status.as_str()) {
            self.status = Some(view.status.clone());
        }
        self.plugin_status = Some(view.status);
        if let Some(sessions) = view.sessions {
            self.history =
                sessions.into_iter().map(|s| SessionInfo { id: s.id, title: s.title, updated: s.updated }).collect();
            self.history_loaded = true;
            self.deleting = false;
            self.deleting_id = None;
        }
        self.history_loading = view.sessions_loading;
        if let Some(models) = view.models {
            self.models = models;
            self.models_loaded = true;
            self.model_results.clear();
            self.model_results_query = None;
        }
        self.models_loading = view.models_loading;
        self.model = Some(view.model).filter(|m| !m.is_empty());
        self.mode = view.mode;
        self.permissions = view.permissions;
        self.last_turn_seconds = view.turn_seconds;
        self.refresh_working_set();
        if was_running && !self.running {
            self.focus_input = true;
            // A message typed while the agent ran sends now.
            if !self.queue.is_empty() {
                let next = self.queue.remove(0);
                let root = self.root.clone();
                self.input = next;
                self.send(root.as_deref());
            }
        }
    }

    /// As if the Agents extension filled the view, for tests elsewhere in the app.
    #[cfg(test)]
    pub(crate) fn filled_for_test(&mut self) {
        self.plugin = Some("coiman.agents".to_owned());
        self.name = "OpenCode".to_owned();
    }

    /// No extension fills the view any more (it was stopped or removed).
    pub(crate) fn detach(&mut self, plugin: &str) {
        if self.plugin.as_deref() == Some(plugin) {
            self.plugin = None;
            self.running = false;
            self.session_loading = false;
        }
    }

    /// What the view asked for since the last frame, for the extension.
    pub(crate) fn take_actions(&mut self) -> Option<(String, Vec<ChatAction>)> {
        let plugin = self.plugin.clone()?;
        (!self.outbox.is_empty()).then(|| (plugin, std::mem::take(&mut self.outbox)))
    }

    fn save_prefs(&mut self) {
        let model = self.model.clone().unwrap_or_default();
        let (mode, permissions) = (self.mode.clone(), self.permissions.clone());
        self.act("model", &model, Vec::new());
        self.act("mode", &mode, Vec::new());
        self.act("permissions", &permissions, Vec::new());
    }

    pub(crate) fn load_models(&mut self) {
        if !self.models_loaded && !self.models_asked && self.plugin.is_some() {
            self.models_asked = true;
            self.act("models", "", Vec::new());
        }
    }

    fn load_history(&mut self) {
        if !self.history_loading && self.plugin.is_some() {
            self.history_loading = true;
            self.act("sessions", "", Vec::new());
        }
    }

    /// Deletes a session, then the list comes again.
    fn delete_session(&mut self, session: SessionInfo) {
        self.delete_pending = None;
        if self.running || self.session_loading {
            self.status = Some("Stop the active task before deleting a session".to_owned());
            return;
        }
        self.deleting = true;
        self.deleting_id = Some(session.id.clone());
        self.act("delete", &session.id, Vec::new());
    }

    /// Opens a past session.
    fn open_session(&mut self, session: SessionInfo) {
        if self.running || self.session_loading {
            self.status = Some("Stop the active task before switching sessions".to_owned());
            return;
        }
        self.last_changed_files.clear();
        self.last_turn_tools = 0;
        self.last_sent = None;
        self.history_open = false;
        self.focus_input = true;
        self.session_loading = true;
        self.act("open", &session.id, Vec::new());
    }

    fn send(&mut self, root: Option<&Path>) {
        let text = self.input.trim().to_owned();
        if text.is_empty() || self.running || self.session_loading {
            return;
        }
        if let Some((last, at)) = &self.last_sent
            && text == *last
            && at.elapsed() < std::time::Duration::from_secs(2)
        {
            self.input.clear();
            self.status = Some("Duplicate prompt ignored".to_owned());
            return;
        }
        if let Some(command) = text.strip_prefix('/') {
            self.slash(command);
            return;
        }
        if !root.is_some_and(trust::is_trusted) {
            self.status = Some("The working folder is restricted. Trust it before starting an agent.".to_owned());
            return;
        }
        if self.plugin.is_none() {
            self.status = Some("The Agents extension isn't running".to_owned());
            return;
        }
        let rendered = self.with_attachments(&text);
        let duplicate_last =
            self.messages.last().is_some_and(|message| message.role == Role::User && message_text(message) == rendered);
        if duplicate_last {
            self.input.clear();
            self.status = Some("Duplicate prompt ignored".to_owned());
            return;
        }
        let attachments = std::mem::take(&mut self.attached);
        self.act("send", &text, attachments);
        // Running until the extension says otherwise: what's typed meanwhile waits its turn.
        self.running = true;
        self.last_sent = Some((text.clone(), std::time::Instant::now()));
        self.prompt_history.push(text);
        self.history_index = None;
        self.input.clear();
        self.load_models();
    }

    /// Stops the current turn and sends the composed message at once: VS Code's "Stop and
    /// Send".
    fn send_now(&mut self, root: Option<&Path>) {
        if self.running {
            self.stop();
        }
        self.last_sent = None;
        self.send(root);
    }

    fn stop(&mut self) {
        self.act("stop", "", Vec::new());
        self.running = false;
        // A stopped run drops its queue: those messages must not fire after some later run.
        self.queue.clear();
    }

    /// Files edited in the last turn and how many tools it used.
    fn refresh_working_set(&mut self) {
        self.last_changed_files = files_touched(&self.messages);
        self.last_turn_tools = tools_in_last_turn(&self.messages);
    }

    /// Starts a fresh chat.
    fn new_chat(&mut self) -> bool {
        if self.running || self.session_loading {
            self.status = Some("Stop the active task before starting a new session".to_owned());
            return false;
        }
        self.act("new", "", Vec::new());
        self.messages.clear();
        self.session.clear();
        self.session_title = None;
        self.queue.clear();
        self.attached.clear();
        self.last_changed_files.clear();
        self.last_turn_tools = 0;
        self.input.clear();
        self.history_index = None;
        self.history_open = false;
        self.last_turn_seconds = None;
        self.last_sent = None;
        self.focus_input = true;
        self.status = Some("New session".to_owned());
        true
    }

    fn slash(&mut self, command: &str) {
        match command.trim() {
            "clear" | "new" => {
                self.new_chat();
            }
            "model" => {
                self.input.clear();
                self.model_search.clear();
                self.status = Some("Pick a model with the chip in the composer".to_owned());
            }
            "help" => {
                self.messages.push(Message {
                    role: Role::Agent,
                    parts: vec![Part::Text(
                        "Commands: /clear (new chat), /model (choose a model), /help (this text).\nEnter sends; Shift+Enter adds a newline.".to_owned(),
                    )],
                });
                self.input.clear();
            }
            other => {
                self.status = Some(format!("Unknown command /{other}"));
                self.input.clear();
            }
        }
    }

    pub(crate) fn panel(&mut self, ui: &mut egui::Ui) {
        let pal = theme::colors();
        let running = self.running;
        let jump = std::mem::take(&mut self.jump_to_bottom);

        // Sessions are a full-screen picker. Selecting one returns to Chat;
        // this keeps the transcript from being squeezed into a narrow column.
        let total = ui.available_width().max(320.0);
        let height = ui.available_height();
        if !self.history_loaded && !self.history_loading {
            self.load_history();
        }

        ui.allocate_ui_with_layout(egui::vec2(total, height), egui::Layout::top_down(Align::Min), |ui| {
            if self.history_open {
                self.sessions_column(ui, pal, total);
            } else {
                self.chat_column(ui, pal, running, jump, false);
            }
        });
        self.jump_pill(ui, pal);
        self.delete_modal(ui.ctx(), pal);
        // Files dropped onto the window attach as context.
        let dropped: Vec<PathBuf> = ui
            .ctx()
            .input(|i| i.raw.dropped_files.iter().filter_map(|file| file.path().to_path_buf().into()).collect());
        for path in dropped {
            self.attach(&path);
        }
    }

    /// The compact 31px context bar, transcript, and composer. The reference's
    /// Chat/Antigravity/Markdown Preview tabs are intentionally omitted.
    fn chat_column(
        &mut self,
        ui: &mut egui::Ui,
        pal: theme::Palette,
        running: bool,
        jump: bool,
        sessions_visible: bool,
    ) {
        let width = ui.available_width();
        let left = ui.cursor().left();
        let canvas = ui.max_rect();
        ui.painter().rect_filled(canvas, 0.0, pal.sidebar_bg);

        let context_top = ui.cursor().top();
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(left, context_top), egui::vec2(width, CHAT_CONTEXT_HEIGHT)),
            0.0,
            pal.sidebar_bg,
        );
        ui.allocate_ui_with_layout(
            egui::vec2(width, CHAT_CONTEXT_HEIGHT),
            egui::Layout::left_to_right(Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if bar_icon_button(ui, crate::ui::icons::Icon::ArrowLeft, "Back to sessions", pal).clicked()
                    && !sessions_visible
                {
                    self.history_open = true;
                    self.load_history();
                }
                let title = self.session_title.clone().unwrap_or_else(|| "New Session".to_owned());
                let prefix = format!("{} · ", if self.name.is_empty() { "Agents" } else { self.name.as_str() });
                ui.add(
                    egui::Label::new(RichText::new(prefix).size(11.0).color(pal.text_dim))
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );
                ui.add(
                    egui::Label::new(RichText::new(title).size(11.0).strong().color(pal.text))
                        .wrap_mode(egui::TextWrapMode::Truncate),
                );

                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if bar_icon_button(ui, crate::ui::icons::Icon::Close, "Close panel", pal).clicked() {
                        self.open = false;
                    }
                    let (icon, tip) = if self.maximized {
                        (crate::ui::icons::Icon::Restore, "Restore size")
                    } else {
                        (crate::ui::icons::Icon::Maximize, "Maximize panel")
                    };
                    if bar_icon_button(ui, icon, tip, pal).clicked() {
                        self.maximized = !self.maximized;
                    }
                    let more = bar_icon_button(ui, crate::ui::icons::Icon::Menu, "More actions", pal);
                    egui::Popup::menu(&more).show(|ui| {
                        if ui.button("New session").clicked() {
                            self.new_chat();
                            ui.close();
                        }
                        if ui.button("Continue last session").clicked() {
                            self.continue_session();
                            ui.close();
                        }
                        if ui.button("Copy conversation").clicked() {
                            ui.ctx().copy_text(transcript_markdown(&self.messages));
                            self.status = Some("Conversation copied".to_owned());
                            ui.close();
                        }
                        if ui.button("MCP servers").clicked() {
                            self.requests_mcp = true;
                            ui.close();
                        }
                        if ui.button("Chat settings").clicked() {
                            self.requests_settings = true;
                            ui.close();
                        }
                    });
                    if bar_icon_button(ui, crate::ui::icons::Icon::Plus, "New session", pal).clicked() {
                        self.new_chat();
                    }
                    if bar_icon_button(ui, crate::ui::icons::Icon::Copy, "Copy conversation", pal).clicked() {
                        ui.ctx().copy_text(transcript_markdown(&self.messages));
                        self.status = Some("Conversation copied".to_owned());
                    }
                });
            },
        );
        let bottom = ui.cursor().top();
        ui.painter().line_segment(
            [egui::pos2(left, bottom - 1.0), egui::pos2(left + width, bottom - 1.0)],
            egui::Stroke::new(1.0, pal.border),
        );

        if self.plugin.is_none() || self.missing.is_some() {
            // No agent: a centered notice fills the column.
            let (headline, how) = match &self.missing {
                Some(how) => (format!("{} isn't installed", self.name), format!("Install it with: {how}")),
                None => (
                    "The Agents extension isn't running".to_owned(),
                    "Install the Agents pack from the Extensions view".to_owned(),
                ),
            };
            let height = ui.available_height();
            ui.allocate_ui(egui::vec2(width, height), |ui| {
                ui.add_space(height * 0.3);
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 22.0, pal.accent.gamma_multiply(0.15));
                    crate::ui::icons::paint(ui.painter(), rect.shrink(10.0), crate::ui::icons::Icon::Agent, pal.accent);
                    ui.add_space(8.0);
                    ui.label(RichText::new(headline).strong());
                    ui.label(RichText::new(how).small().color(pal.text_dim));
                    ui.label(
                        RichText::new("Coiman runs the program on this computer; it has no AI of its own.")
                            .small()
                            .color(pal.text_dim),
                    );
                });
            });
            return;
        }

        // The composer stays at the bottom while the transcript scrolls above it.
        // The composer's height comes from the previous frame; a one-frame lag is
        // invisible, and it keeps the two from fighting over the remaining space.
        let width = ui.available_width();
        let avail = ui.available_height();
        let spacing = ui.spacing().item_spacing.y;
        let composer_h = self.composer_height.clamp(COMPOSER_MIN_HEIGHT, (avail - 100.0).max(COMPOSER_MIN_HEIGHT));
        let rest = (avail - composer_h - spacing).max(60.0);
        ui.allocate_ui_with_layout(egui::vec2(width, rest), egui::Layout::top_down(Align::Min), |ui| {
            self.transcript(ui, pal, running, jump);
        });
        self.composer_rect = Some(self.composer(ui, pal, running));
    }

    /// The conversation above the composer: prompts, replies, tool cards, working set.
    fn transcript(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool, jump: bool) {
        let last_user = self.messages.iter().rposition(|message| message.role == Role::User);
        let output = ScrollArea::vertical()
            .id_salt("agents_transcript")
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                // The reference pads the transcript 10px / 13px / 12px.
                egui::Frame::new().inner_margin(egui::Margin { left: 13, right: 13, top: 10, bottom: 12 }).show(
                    ui,
                    |ui| {
                        ui.set_width((ui.available_width() - 1.0).max(40.0));
                        if self.session_loading {
                            ui.add_space(40.0);
                            ui.vertical_centered(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Opening session…").small().color(pal.text_dim));
                            });
                            return;
                        }
                        if self.messages.is_empty() {
                            ui.add_space(48.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("✧").size(23.0).color(pal.text_dim));
                                ui.add_space(7.0);
                                ui.label(
                                    RichText::new("What can I help you with?").size(13.0).strong().color(pal.text),
                                );
                                ui.add_space(3.0);
                                ui.label(
                                    RichText::new("Ask a question or describe a task for your project.")
                                        .size(11.0)
                                        .color(pal.text_dim),
                                );
                            });
                            return;
                        }

                        let messages = self.messages.clone();
                        let mut last_user_text: Option<String> = None;
                        for (index, message) in messages.iter().enumerate() {
                            match message.role {
                                Role::User => {
                                    let text = message_text(message);
                                    if last_user_text.as_deref() == Some(text.as_str()) {
                                        continue;
                                    }
                                    last_user_text = Some(text.clone());
                                    let root = self.root.clone();
                                    if let Some(action) = user_bubble(ui, message, pal, index) {
                                        let text = message_text(message);
                                        match action {
                                            UserAction::Retry => {
                                                self.input = text;
                                                self.send(root.as_deref());
                                            }
                                            UserAction::Edit => {
                                                self.input = text;
                                                self.focus_input = true;
                                            }
                                        }
                                    }
                                    if Some(index) == last_user {
                                        self.status_line(ui, pal, running);
                                    }
                                }
                                Role::Agent => {
                                    last_user_text = None;
                                    agent_message(ui, message, pal, running, index)
                                }
                            }
                        }
                        if jump {
                            ui.scroll_to_rect(ui.min_rect(), Some(Align::BOTTOM));
                        }
                    },
                );
            });
        self.at_bottom = output.state.clamped_offset().y + output.inner_rect.height() >= output.content_size.y - 2.0;
    }

    /// The reference's completion line: "Completed N steps in 1m 47s" plus
    /// compact +added / -removed statistics.
    fn status_line(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool) {
        ui.add_space(2.0);
        if running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Working…").size(10.5).color(pal.text_dim));
            });
            ui.add_space(8.0);
            return;
        }

        let tools = self.last_turn_tools;
        let changed = self.last_changed_files.len();
        let status = self.status.clone().filter(|text| !text.is_empty());
        if tools == 0 && changed == 0 && status.is_none() {
            ui.add_space(4.0);
            return;
        }

        egui::Frame::new()
            .fill(pal.hover_bg)
            .stroke(egui::Stroke::new(1.0, pal.border))
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(7, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if tools > 0 {
                        crate::ui::icons::paint(
                            ui.painter(),
                            egui::Rect::from_min_size(
                                egui::pos2(ui.cursor().left(), ui.cursor().top() + 5.0),
                                egui::vec2(10.0, 10.0),
                            ),
                            crate::ui::icons::Icon::Check,
                            pal.accent,
                        );
                        ui.add_space(13.0);
                        let mut label = format!("Completed {tools} steps");
                        if let Some(seconds) = self.last_turn_seconds {
                            if seconds >= 60 {
                                label.push_str(&format!(" in {}m {}s", seconds / 60, seconds % 60));
                            } else {
                                label.push_str(&format!(" in {seconds}s"));
                            }
                        }
                        ui.label(RichText::new(label).size(10.5).color(pal.text_dim));
                    }
                    if changed > 0 {
                        ui.label(RichText::new(format!("+{changed}")).size(10.0).monospace().color(pal.modified));
                    }
                    if let Some(status) = status
                        && tools == 0
                        && changed == 0
                    {
                        ui.label(RichText::new(status).size(10.5).color(pal.text_dim));
                    }
                });
            });
        ui.add_space(9.0);
    }

    /// The reference's 23px circular scroll-to-latest button.
    fn jump_pill(&mut self, ui: &mut egui::Ui, pal: theme::Palette) {
        if !self.at_bottom
            && let Some(composer) = self.composer_rect
        {
            let size = egui::vec2(23.0, 23.0);
            let pill = egui::Rect::from_min_size(
                egui::pos2(composer.right() - size.x - 10.0, composer.top() - size.y - 8.0),
                size,
            );
            let response = ui.interact(pill, egui::Id::unique("agents_jump_pill"), Sense::click());
            ui.painter().circle_filled(
                pill.center(),
                size.x / 2.0,
                if response.hovered() { pal.hover_bg } else { pal.widget_bg },
            );
            ui.painter().circle_stroke(pill.center(), size.x / 2.0, egui::Stroke::new(1.0, pal.border));
            crate::ui::icons::paint(
                ui.painter(),
                egui::Rect::from_min_size(egui::pos2(pill.left() + 6.5, pill.center().y - 5.0), egui::vec2(10.0, 10.0)),
                crate::ui::icons::Icon::ArrowDown,
                pal.text_dim,
            );
            if response.clicked() {
                self.jump_to_bottom = true;
            }
        }
    }

    /// The 260px Sessions rail from the reference. It is a persistent part of the
    /// wide Agents workspace, not a full-screen picker.
    fn sessions_column(&mut self, ui: &mut egui::Ui, pal: theme::Palette, width: f32) {
        let width = width.min(ui.available_width());
        let left = ui.cursor().left();
        ui.painter().rect_filled(ui.max_rect(), 0.0, pal.sidebar_bg);
        ui.allocate_ui_with_layout(
            egui::vec2(width, ui.available_height()),
            egui::Layout::top_down(Align::Min),
            |ui| {
                // Aligns with the context bar above the conversation.
                ui.allocate_ui_with_layout(
                    egui::vec2(width, SESSIONS_TOP_HEIGHT),
                    egui::Layout::left_to_right(Align::Center),
                    |_ui| {},
                );
                ui.painter().line_segment(
                    [egui::pos2(left, ui.cursor().top() - 1.0), egui::pos2(left + width, ui.cursor().top() - 1.0)],
                    egui::Stroke::new(1.0, pal.border),
                );

                ui.allocate_ui_with_layout(
                    egui::vec2(width, SESSIONS_HEADER_HEIGHT),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add_space(12.0);
                        if width > 180.0 {
                            ui.label(RichText::new("Sessions").size(11.0).strong().color(pal.text));
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if bar_icon_button(ui, crate::ui::icons::Icon::Lines, "Toggle Sessions pane", pal).clicked()
                            {
                                self.history_open = false;
                            }
                            if bar_icon_button(ui, crate::ui::icons::Icon::Search, "Search sessions", pal).clicked() {
                                self.session_search_open = !self.session_search_open;
                                if self.session_search_open {
                                    self.focus_session_search = true;
                                } else {
                                    self.session_search.clear();
                                }
                            }
                            if bar_icon_button(ui, crate::ui::icons::Icon::Refresh, "Refresh sessions", pal).clicked() {
                                self.load_history();
                            }
                        });
                    },
                );

                let new_session = ui.add_sized(
                    [width - 14.0, 23.0],
                    egui::Button::new(RichText::new("New Session").size(10.5).color(pal.text)),
                );
                if new_session.clicked() {
                    self.new_chat();
                }
                ui.add_space(9.0);

                ui.horizontal(|ui| {
                    ui.add_space(13.0);
                    ui.label(RichText::new("Older").size(10.5).color(pal.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(self.matching_sessions().len().to_string()).size(10.5).color(pal.text_dim),
                        );
                    });
                });
                ui.add_space(3.0);

                if self.session_search_open {
                    let search = ui.add_sized(
                        [width - 16.0, 0.0],
                        TextEdit::singleline(&mut self.session_search)
                            .id(egui::Id::unique("agents_sidebar_session_search"))
                            .hint_text("Search sessions"),
                    );
                    if std::mem::take(&mut self.focus_session_search) {
                        search.request_focus();
                    }
                    ui.add_space(3.0);
                }

                let sessions = self.matching_sessions();
                let current = self.session.clone();
                ScrollArea::vertical().id_salt("agents_sessions_rail").auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if self.history_loading {
                        ui.horizontal(|ui| {
                            ui.add_space(13.0);
                            ui.spinner();
                            ui.label(RichText::new("Loading sessions…").small().color(pal.text_dim));
                        });
                    } else if sessions.is_empty() {
                        ui.add_space(12.0);
                        ui.label(RichText::new("No sessions yet").small().color(pal.text_dim));
                    }
                    for session in &sessions {
                        self.session_row(ui, session, pal, session.id == current);
                    }
                });
            },
        );
        let top = ui.cursor().top();
        ui.painter().line_segment(
            [egui::pos2(left + width - 1.0, top), egui::pos2(left + width - 1.0, top + ui.available_height())],
            egui::Stroke::new(1.0, pal.border),
        );
    }

    /// One session row from the reference: dot, title, relative time, and a
    /// delete action that appears on hover.
    fn session_row(&mut self, ui: &mut egui::Ui, session: &SessionInfo, pal: theme::Palette, selected: bool) {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        let fill = if selected {
            pal.selection
        } else if hovered {
            pal.hover_bg
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, 3.0, fill);

        let dot = egui::pos2(rect.left() + 8.0, rect.top() + 14.0);
        ui.painter().circle_filled(dot, 2.0, if selected { pal.accent } else { pal.text_dim });
        let name_x = rect.left() + 17.0;
        ui.painter().text(
            egui::pos2(name_x, rect.top() + 8.0),
            Align2::LEFT_TOP,
            fit_text(ui, &truncate(&session.title, 64), FontId::proportional(11.0), pal.text, rect.width() - 40.0),
            FontId::proportional(11.0),
            pal.text,
        );
        ui.painter().text(
            egui::pos2(name_x, rect.bottom() - 12.0),
            Align2::LEFT_CENTER,
            session_time_label(session.updated),
            FontId::proportional(9.5),
            pal.text_dim,
        );

        let delete_rect =
            egui::Rect::from_min_size(egui::pos2(rect.right() - 25.0, rect.top() + 9.0), egui::vec2(19.0, 19.0));
        if hovered {
            crate::ui::icons::paint(ui.painter(), delete_rect.shrink(3.0), crate::ui::icons::Icon::Close, pal.text_dim);
        }
        let deleting = self.deleting && self.deleting_id.as_deref() == Some(session.id.as_str());
        if deleting {
            ui.painter().circle_stroke(delete_rect.center(), 4.5, egui::Stroke::new(1.2, pal.text_dim));
        }
        let delete = ui.interact(delete_rect, egui::Id::unique("agents_delete_row").with(&session.id), Sense::click());
        if delete.clicked() && !deleting {
            self.delete_pending = Some(session.clone());
        }
        if response.clicked() && !delete.hovered() {
            self.open_session(session.clone());
        }
        response.on_hover_text(format!("{}\n{}\n{}", session.title, session.id, time_label(session.updated)));
    }

    /// Confirmation before a session is deleted.
    fn delete_modal(&mut self, ctx: &egui::Context, pal: theme::Palette) {
        let Some(session) = self.delete_pending.clone() else { return };
        let modal = egui::Modal::new(egui::Id::unique("agents_delete_session")).show(ctx, |ui| {
            ui.label(RichText::new("Delete session?").strong());
            ui.label(RichText::new(&session.title).small().color(pal.text_dim));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    self.delete_session(session.clone());
                    ui.close();
                }
                if ui.button("Cancel").clicked() {
                    self.delete_pending = None;
                    ui.close();
                }
            });
        });
        if modal.should_close() {
            self.delete_pending = None;
        }
    }

    /// Opens the most recent session; the extension loads the list first if it must.
    fn continue_session(&mut self) {
        if self.running || self.session_loading {
            self.status = Some("Stop the active task before switching sessions".to_owned());
            return;
        }
        self.history_open = false;
        self.act("continue", "", Vec::new());
    }

    /// Steps through sent prompts with the up/down arrows in an empty box.
    fn cycle_history(&mut self, step: i32) {
        if self.prompt_history.is_empty() {
            return;
        }
        let last = self.prompt_history.len() - 1;
        self.history_index = match self.history_index {
            None if step < 0 => Some(last),
            None => None,
            Some(index) if step < 0 => Some(index.saturating_sub(1)),
            Some(index) if index < last => Some(index + 1),
            Some(_) => None,
        };
        self.input = self.history_index.map(|index| self.prompt_history[index].clone()).unwrap_or_default();
    }

    fn composer(&mut self, ui: &mut egui::Ui, pal: theme::Palette, running: bool) -> Rect {
        let root = self.root.clone();
        let outer =
            egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 0, bottom: 7 }).show(ui, |ui| {
                // Messages typed while the agent runs, removable until they send.
                if !self.queue.is_empty() {
                    let queued = self.queue.clone();
                    for (index, text) in queued.into_iter().enumerate() {
                        egui::Frame::new()
                            .fill(pal.hover_bg)
                            .corner_radius(4.0)
                            .inner_margin(egui::Margin::symmetric(7, 3))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Queued · {}", truncate(&text, 28)))
                                            .size(10.5)
                                            .color(pal.text_dim),
                                    );
                                    if bar_icon_button(ui, crate::ui::icons::Icon::Close, "Remove from queue", pal)
                                        .clicked()
                                    {
                                        self.queue.remove(index);
                                    }
                                });
                            });
                    }
                    ui.add_space(2.0);
                }

                // Attached context sits above the ask box, as in the reference.
                if !self.attached.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(5.0, 3.0);
                        for path in self.attached.clone() {
                            let name = Path::new(&path)
                                .file_name()
                                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                            egui::Frame::new()
                                .fill(pal.hover_bg)
                                .corner_radius(3.0)
                                .inner_margin(egui::Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        crate::ui::icons::paint_file(
                                            ui.painter(),
                                            egui::Rect::from_min_size(
                                                egui::pos2(ui.cursor().left(), ui.cursor().top() + 2.0),
                                                egui::vec2(10.0, 10.0),
                                            ),
                                            &path,
                                        );
                                        ui.add_space(13.0);
                                        ui.label(RichText::new(truncate(&name, 28)).size(10.0).color(pal.text_dim));
                                        if bar_icon_button(
                                            ui,
                                            crate::ui::icons::Icon::Close,
                                            "Remove attached context",
                                            pal,
                                        )
                                        .clicked()
                                        {
                                            self.attached.retain(|kept| kept != &path);
                                        }
                                    });
                                });
                        }
                    });
                    ui.add_space(2.0);
                }

                // The composer: the text area and the control row inside one
                // 7px rounded box, padded evenly on all four sides.
                let border = if self.input_focused { mix(pal.border, pal.text, 0.35) } else { pal.border };
                egui::Frame::new()
                    .fill(pal.widget_bg)
                    .stroke(egui::Stroke::new(1.0, border))
                    .corner_radius(7.0)
                    .inner_margin(egui::Margin { left: 6, right: 6, top: 6, bottom: 6 })
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.spacing_mut().item_spacing.y = 0.0;

                        let output = egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(6, 6))
                            .show(ui, |ui| {
                                ui.set_min_height(29.0);
                                let avail = ui.available_width();
                                TextEdit::multiline(&mut self.input)
                                    .id(egui::Id::unique("agents_prompt"))
                                    .desired_rows(1)
                                    .frame(egui::Frame::NONE)
                                    .hint_text("Chat with OpenCode")
                                    .desired_width(avail)
                                    .show(ui)
                            })
                            .inner;
                        let focus_requested = std::mem::take(&mut self.focus_input);
                        let focused = output.response.response.has_focus();
                        if focus_requested {
                            output.response.response.request_focus();
                        }
                        self.input_focused = focused || focus_requested;
                        if focused && let Some(cursor) = &output.cursor_range {
                            let caret =
                                output.galley.pos_from_cursor(cursor.primary).translate(output.galley_pos.to_vec2());
                            ui.scroll_to_rect(caret, None);
                        }

                        // The @ file reference popup.
                        if let Some((start, token)) = at_token(&self.input).map(|(at, token)| (at, token.to_owned())) {
                            let entries = at_entries(root.as_deref(), &token);
                            if !entries.is_empty() {
                                let anchor = egui::Rect::from_min_size(
                                    ui.cursor().left_top() + egui::vec2(0.0, 4.0),
                                    egui::vec2(1.0, 1.0),
                                );
                                let popup = egui::Popup::new(
                                    egui::Id::unique("agents_at_popup"),
                                    ui.ctx().clone(),
                                    anchor,
                                    egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("agents_at_popup")),
                                );
                                popup.show(|ui| {
                                    ui.set_width(220.0);
                                    for name in entries {
                                        if ui.selectable_label(false, &name).clicked() {
                                            let dir = token
                                                .rsplit_once('/')
                                                .map(|(dir, _)| format!("{dir}/"))
                                                .unwrap_or_default();
                                            self.input = format!("{}{}@{}{} ", &self.input[..start], "", dir, name);
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        }

                        // Paste an image from the clipboard as a file.
                        #[cfg(not(target_os = "android"))]
                        if ui.input(|i| i.key_pressed(egui::Key::V) && i.modifiers.ctrl)
                            && let Ok(mut clipboard) = arboard::Clipboard::new()
                            && let Ok(image) = clipboard.get_image()
                        {
                            let nanos = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |d| d.as_nanos());
                            let path =
                                std::env::temp_dir().join(format!("coiman_paste_{}_{nanos}.png", std::process::id()));
                            let png = image::RgbaImage::from_raw(
                                image.width as u32,
                                image.height as u32,
                                image.bytes.into_owned(),
                            );
                            if let Some(png) = png
                                && png.save(&path).is_ok()
                            {
                                self.attach(&path);
                            }
                        }

                        // Enter sends, or queues while the agent runs.
                        if self.input_focused && ui.input(|i| i.key_pressed(egui::Key::Enter) && i.modifiers.is_none())
                        {
                            let text = self.input.trim().to_owned();
                            self.input.clear();
                            if !text.is_empty() {
                                if running {
                                    self.queue.push(text);
                                    self.status = Some("Queued — it sends when this turn finishes".to_owned());
                                } else {
                                    self.input = text;
                                    self.send(root.as_deref());
                                }
                            }
                        }
                        let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp) && i.modifiers.is_none());
                        let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown) && i.modifiers.is_none());
                        if self.input.is_empty() && self.input_focused {
                            if up {
                                self.cycle_history(-1);
                            } else if down {
                                self.cycle_history(1);
                            }
                        }

                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let add_context = bar_icon_button(ui, crate::ui::icons::Icon::Plus, "Add context", pal);
                            egui::Popup::menu(&add_context).show(|ui| {
                                if let Some(file) = self.active_file.clone() {
                                    let name = file.file_name().map_or_else(
                                        || file.display().to_string(),
                                        |n| n.to_string_lossy().into_owned(),
                                    );
                                    if ui.button(format!("Add context: {name}")).clicked() {
                                        self.attach(&file);
                                        ui.close();
                                    }
                                }
                                if let Some(root) = self.root.clone() {
                                    let folder = root.file_name().map_or_else(
                                        || root.display().to_string(),
                                        |n| n.to_string_lossy().into_owned(),
                                    );
                                    if ui.button(format!("Add context: {folder}")).clicked() {
                                        self.attach(&root);
                                        ui.close();
                                    }
                                }
                            });

                            let mode = if self.mode == "plan" { "Plan" } else { "Agent" };
                            let mode_resp = chip(ui, pal, mode, true, Some(crate::ui::icons::Icon::Agent));
                            egui::Popup::menu(&mode_resp).show(|ui| {
                                if ui.selectable_label(self.mode != "plan", "Agent — edits files").clicked() {
                                    self.mode = "build".to_owned();
                                    self.save_prefs();
                                    ui.close();
                                }
                                if ui.selectable_label(self.mode == "plan", "Plan — discuss only").clicked() {
                                    self.mode = "plan".to_owned();
                                    self.save_prefs();
                                    ui.close();
                                }
                            });
                            ui.add_space(4.0);

                            let model = self
                                .model
                                .as_deref()
                                .map(|name| truncate(&model_label(name), 50))
                                .unwrap_or_else(|| "Auto".to_owned());
                            let model_resp = chip(ui, pal, &model, true, None);
                            if model_resp.clicked() {
                                self.focus_model_search = true;
                                self.model_search.clear();
                            }
                            egui::Popup::menu(&model_resp).show(|ui| {
                                let search = ui.add(
                                    TextEdit::singleline(&mut self.model_search)
                                        .id(egui::Id::unique("agents_model_search"))
                                        .hint_text("Search models"),
                                );
                                if std::mem::take(&mut self.focus_model_search) {
                                    search.request_focus();
                                }
                                if ui.selectable_label(self.model.is_none(), "Auto — let OpenCode choose").clicked() {
                                    self.model = None;
                                    self.save_prefs();
                                    ui.close();
                                }
                                if self.models_loading {
                                    ui.label(RichText::new("Loading models…").small().color(pal.text_dim));
                                }
                                let query = self.model_search.clone();
                                if self.model_results_query.as_deref() != Some(query.as_str()) {
                                    self.model_results = self
                                        .models
                                        .iter()
                                        .filter(|name| loose_model_match(name, &query))
                                        .take(80)
                                        .cloned()
                                        .collect();
                                    self.model_results_query = Some(query);
                                }
                                egui::ScrollArea::vertical()
                                    .id_salt("agents_model_list")
                                    .auto_shrink([false, false])
                                    .max_height(8.0 * 22.0)
                                    .show(ui, |ui| {
                                        for name in self.model_results.clone() {
                                            if ui
                                                .selectable_label(
                                                    self.model.as_deref() == Some(name.as_str()),
                                                    model_label(&name),
                                                )
                                                .clicked()
                                            {
                                                self.model = Some(name.clone());
                                                self.save_prefs();
                                                ui.close();
                                            }
                                        }
                                        if self.model_results.len() >= 80 {
                                            ui.label(
                                                RichText::new("Showing 80 matches — keep typing to narrow")
                                                    .size(9.5)
                                                    .color(pal.text_dim),
                                            );
                                        }
                                    });
                            });

                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                let can_send = !self.input.trim().is_empty() && !self.session_loading;
                                if running && can_send {
                                    let (send_rect, send_resp) =
                                        ui.allocate_exact_size(egui::vec2(24.0, 23.0), Sense::click());
                                    if send_resp.hovered() {
                                        ui.painter().rect_filled(send_rect, 4.0, pal.hover_bg);
                                    }
                                    crate::ui::icons::paint(
                                        ui.painter(),
                                        send_rect.shrink(5.0),
                                        crate::ui::icons::Icon::Send,
                                        pal.text,
                                    );
                                    if send_resp.clicked() {
                                        self.send_now(root.as_deref());
                                    }
                                    send_resp.on_hover_text("Stop and send now (Enter queues)");
                                }
                                let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 23.0), Sense::click());
                                if resp.hovered() {
                                    ui.painter().rect_filled(rect, 4.0, pal.hover_bg);
                                }
                                if running {
                                    crate::ui::icons::paint(
                                        ui.painter(),
                                        rect.shrink(7.0),
                                        crate::ui::icons::Icon::Stop,
                                        pal.text,
                                    );
                                    if resp.clicked() {
                                        self.stop();
                                    }
                                    resp.on_hover_text("Stop");
                                } else {
                                    crate::ui::icons::paint(
                                        ui.painter(),
                                        rect.shrink(5.0),
                                        crate::ui::icons::Icon::Send,
                                        if can_send { pal.text } else { pal.text_dim },
                                    );
                                    if resp.clicked() && can_send {
                                        self.send(root.as_deref());
                                    }
                                    resp.on_hover_text("Send (Enter)");
                                }
                            });
                        });
                    });
                // The 20px footer from the reference.
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 20.0),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(if self.mode == "plan" { "Plan" } else { "Interactive" })
                                .size(10.0)
                                .color(pal.text_dim),
                        );
                        ui.label(RichText::new("•").size(10.0).color(pal.text_dim));
                        let permissions = ui.add(
                            egui::Label::new(
                                RichText::new(if self.permissions == "manual" {
                                    "Manual permissions"
                                } else {
                                    "Auto permissions"
                                })
                                .size(10.0)
                                .color(pal.text_dim),
                            )
                            .sense(Sense::click()),
                        );
                        egui::Popup::menu(&permissions).show(|ui| {
                            if ui.selectable_label(self.permissions != "manual", "Auto — approve tool calls").clicked()
                            {
                                self.permissions = "auto".to_owned();
                                self.save_prefs();
                                ui.close();
                            }
                            if ui
                                .selectable_label(
                                    self.permissions == "manual",
                                    "Manual — approval-requiring calls are rejected by opencode run",
                                )
                                .clicked()
                            {
                                self.permissions = "manual".to_owned();
                                self.save_prefs();
                                self.status = Some(
                                    "Manual mode: tool calls needing approval are rejected by opencode run".to_owned(),
                                );
                                ui.close();
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            let path = root
                                .as_ref()
                                .map(|root| root.display().to_string())
                                .unwrap_or_else(|| "no folder".to_owned());
                            ui.add(
                                egui::Label::new(RichText::new(truncate(&path, 42)).size(10.0).color(pal.text_dim))
                                    .wrap_mode(egui::TextWrapMode::Truncate),
                            );
                        });
                    },
                );
            });
        self.composer_height = outer.response.rect.height();
        outer.response.rect
    }

    /// Adds a path to the attached context, once.
    /// The sessions that match the search text, in the list's order.
    fn matching_sessions(&self) -> Vec<SessionInfo> {
        let query = self.session_search.to_lowercase();
        self.history
            .iter()
            .filter(|session| {
                query.is_empty()
                    || session.title.to_lowercase().contains(&query)
                    || session.id.to_lowercase().contains(&query)
            })
            .cloned()
            .collect()
    }

    /// The models that match the popup's search text.
    #[cfg(test)]
    fn matching_models(&self) -> Vec<String> {
        if self.model_search.trim().is_empty() {
            return self.models.clone();
        }
        self.models.iter().filter(|name| loose_model_match(name, &self.model_search)).take(80).cloned().collect()
    }

    /// The prompt with its attached context, as @references the agent can look up.
    fn with_attachments(&self, text: &str) -> String {
        if self.attached.is_empty() {
            return text.to_owned();
        }
        let references = self.attached.iter().map(|path| format!("@{path}")).collect::<Vec<_>>().join(" ");
        format!("{references}\n{text}")
    }

    fn attach(&mut self, path: &Path) {
        let text = path.display().to_string();
        if !self.attached.contains(&text) {
            self.attached.push(text);
        }
        self.focus_input = true;
    }
}

/// What a hover action on a user bubble asked for.
pub(crate) enum UserAction {
    Retry,
    Edit,
}

/// A model name without its provider, variant suffix, or punctuation.
fn model_label(name: &str) -> String {
    let base = name.split('#').next().unwrap_or(name);
    let base = base.rsplit('/').next().unwrap_or(base);
    let cleaned = base.replace(['-', '_'], " ");
    cleaned
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn loose_model_match(name: &str, query: &str) -> bool {
    let normalize = |text: &str| {
        text.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let name = normalize(name);
    let query = normalize(query);
    if query.is_empty() {
        return true;
    }
    let compact = |text: &str| text.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>();
    if compact(&name).contains(&compact(&query)) {
        return true;
    }
    query.split_whitespace().all(|term| name.split_whitespace().any(|word| word == term))
}

fn split_refs(text: &str) -> (Vec<String>, String) {
    let mut refs = Vec::new();
    let mut body = String::new();
    for line in text.lines() {
        if body.is_empty() && line.trim_start().starts_with('@') {
            for token in line.split_whitespace().filter(|token| token.starts_with('@')) {
                refs.push(token.trim_start_matches('@').to_owned());
            }
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    (refs, body.trim().to_owned())
}

fn ref_tag(ui: &mut egui::Ui, name: &str, pal: theme::Palette) {
    let label = Path::new(name).file_name().map_or_else(|| name.to_owned(), |file| file.to_string_lossy().into_owned());
    egui::Frame::new()
        .fill(pal.widget_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(7, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                crate::ui::icons::paint_file(
                    ui.painter(),
                    egui::Rect::from_min_size(
                        egui::pos2(ui.cursor().left(), ui.cursor().top() + 2.0),
                        egui::vec2(10.0, 10.0),
                    ),
                    name,
                );
                ui.add_space(13.0);
                ui.label(RichText::new(label).size(10.0).color(pal.text_dim));
            });
        });
}

fn user_bubble(ui: &mut egui::Ui, message: &Message, pal: theme::Palette, index: usize) -> Option<UserAction> {
    ui.add_space(if index == 0 { 2.0 } else { 18.0 });
    let (refs, body) = split_refs(&message_text(message));

    let action = ui
        .vertical(|ui| {
            ui.set_width(ui.available_width());
            for name in &refs {
                ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| {
                    ref_tag(ui, name, pal);
                });
                ui.add_space(4.0);
            }
            if body.is_empty() {
                return None;
            }
            ui.with_layout(egui::Layout::right_to_left(Align::TOP), |ui| -> Option<UserAction> {
                ui.set_max_width((ui.available_width() * 0.95).max(120.0));
                let bubble = egui::Frame::new()
                    .fill(pal.selection)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(10, 9))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(body).size(11.7).color(pal.text)).wrap());
                    });

                if ui.rect_contains_pointer(bubble.response.rect) {
                    let rect = bubble.response.rect;
                    let size = 16.0;
                    let y = rect.top() + 2.0;
                    let mut x = rect.right() - size - 2.0;
                    let copy = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                    let resp = ui.interact(copy, egui::Id::unique("agents_user_copy").with(index), Sense::click());
                    ui.painter().rect_filled(copy, 3.0, pal.editor_bg);
                    crate::ui::icons::paint(ui.painter(), copy.shrink(2.5), crate::ui::icons::Icon::Copy, pal.text_dim);
                    if resp.clicked() {
                        ui.ctx().copy_text(message_text(message));
                    }
                    x -= size + 4.0;
                    let retry = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                    let resp = ui.interact(retry, egui::Id::unique("agents_user_retry").with(index), Sense::click());
                    ui.painter().rect_filled(retry, 3.0, pal.editor_bg);
                    crate::ui::icons::paint(
                        ui.painter(),
                        retry.shrink(2.5),
                        crate::ui::icons::Icon::Refresh,
                        pal.text_dim,
                    );
                    if resp.clicked() {
                        return Some(UserAction::Retry);
                    }
                    x -= size + 4.0;
                    let edit = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(size, size));
                    let resp = ui.interact(edit, egui::Id::unique("agents_user_edit").with(index), Sense::click());
                    ui.painter().rect_filled(edit, 3.0, pal.editor_bg);
                    crate::ui::icons::paint(
                        ui.painter(),
                        edit.shrink(2.5),
                        crate::ui::icons::Icon::Discard,
                        pal.text_dim,
                    );
                    if resp.clicked() {
                        return Some(UserAction::Edit);
                    }
                }
                None
            })
            .inner
        })
        .inner;
    ui.add_space(8.0);
    action
}

fn agent_message(ui: &mut egui::Ui, message: &Message, pal: theme::Palette, live: bool, index: usize) {
    ui.add_space(6.0);
    let top = ui.cursor().left_top();
    for (part_index, part) in message.parts.iter().enumerate() {
        match part {
            Part::Text(text) => {
                if text.starts_with("Error:") {
                    ui.add_space(3.0);
                    ui.label(RichText::new(truncate(text, 160)).size(10.5).color(pal.modified));
                    ui.add_space(6.0);
                } else {
                    markdown(ui, text, pal);
                }
            }
            Part::Thinking(text) => {
                ui.add_space(4.0);
                egui::Frame::new()
                    .fill(pal.hover_bg)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::CollapsingHeader::new(RichText::new("Thinking").small().strong().color(pal.text_dim))
                            .id_salt(format!("agents_thinking_{index}_{part_index}"))
                            .default_open(live)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(text).italics().color(pal.text_dim).size(10.5))
                                        .wrap(),
                                );
                            });
                    });
                ui.add_space(6.0);
            }
            Part::Tool(tool) => tool_card(ui, tool, pal),
        }
    }
    let rect = Rect::from_min_max(top, egui::pos2(ui.min_rect().right(), ui.cursor().top()));
    copy_overlay(ui, rect, egui::Id::unique("agents_agent_copy").with(index), message_text(message), pal);
    ui.add_space(10.0);
}

fn tool_icon(name: &str) -> crate::ui::icons::Icon {
    match name {
        "read" | "write" | "edit" | "patch" | "apply_patch" => crate::ui::icons::Icon::Files,
        "grep" | "glob" | "search" => crate::ui::icons::Icon::Search,
        "shell" | "bash" => crate::ui::icons::Icon::Terminal,
        "task" => crate::ui::icons::Icon::Agent,
        _ => crate::ui::icons::Icon::Files,
    }
}

/// One compact activity row from the reference: chevron, tool glyph, label,
/// and a trailing status mark. Clicking expands the tool input/output.
fn tool_card(ui: &mut egui::Ui, tool: &ToolPart, pal: theme::Palette) {
    ui.add_space(2.0);
    let id = egui::Id::unique("agents_tool_card").with(&tool.id);
    let open = ui.memory_mut(|memory| memory.data.get_temp::<bool>(id).unwrap_or_else(|| is_edit_tool(&tool.name)));
    let (head, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.0), Sense::click());
    if response.clicked() {
        ui.memory_mut(|memory| memory.data.insert_temp(id, !open));
    }
    if response.hovered() {
        ui.painter().rect_filled(head, 3.0, pal.hover_bg);
    }

    // The vertical activity line from the mock-up.
    ui.painter().line_segment(
        [egui::pos2(head.left() + 6.0, head.top()), egui::pos2(head.left() + 6.0, head.bottom())],
        egui::Stroke::new(1.0, pal.border),
    );
    crate::ui::icons::paint(
        ui.painter(),
        egui::Rect::from_min_size(egui::pos2(head.left() + 11.0, head.center().y - 5.0), egui::vec2(10.0, 10.0)),
        if open { crate::ui::icons::Icon::ChevronDown } else { crate::ui::icons::Icon::ChevronRight },
        pal.text_dim,
    );

    let icon_rect =
        egui::Rect::from_min_size(egui::pos2(head.left() + 27.0, head.center().y - 6.0), egui::vec2(12.0, 12.0));
    crate::ui::icons::paint(ui.painter(), icon_rect, tool_icon(&tool.name), pal.text_dim);

    let label = if tool.title.is_empty() || tool.title == tool.name {
        tool.name.clone()
    } else {
        format!("{} · {}", tool.name, tool.title)
    };
    ui.painter().text(
        egui::pos2(head.left() + 45.0, head.center().y),
        Align2::LEFT_CENTER,
        fit_text(ui, &label, FontId::proportional(10.5), pal.text, head.width() - 70.0),
        FontId::proportional(10.5),
        pal.text,
    );

    let (color, mark) = match tool.status.as_str() {
        "completed" => (pal.text_dim, "✓"),
        "error" => (pal.modified, "×"),
        _ => (pal.accent, "…"),
    };
    ui.painter().text(
        egui::pos2(head.right() - 8.0, head.center().y),
        Align2::RIGHT_CENTER,
        mark,
        FontId::proportional(11.0),
        color,
    );

    if open {
        tool_details(ui, tool, pal);
    }
}

/// The command a shell tool ran, plus any other fields it carried. A plain
/// string is a command too, as older OpenCode versions sent it that way.
fn shell_input(input: &str) -> (String, Vec<(String, String)>) {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(input) else {
        return (input.to_owned(), Vec::new());
    };
    let command = match map.get("command") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(words)) => words.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => input.to_owned(),
    };
    let rest = map
        .iter()
        .filter(|(key, _)| key.as_str() != "command")
        .map(|(key, value)| (key.clone(), json_text(value)))
        .collect();
    (command, rest)
}

/// A shell call reads as a command block and an output block, not as a JSON
/// bag with escape codes in it.
fn shell_details(ui: &mut egui::Ui, tool: &ToolPart, pal: theme::Palette) -> bool {
    if !matches!(tool.name.as_str(), "shell" | "bash") {
        return false;
    }
    let (command, rest) = shell_input(&tool.input);
    if !command.is_empty() {
        ui.label(RichText::new("Command").size(9.5).strong().color(pal.text_dim));
        code_block(ui, &command, pal, "bash");
    }
    for (key, value) in rest {
        ui.label(RichText::new(format!("{key}: {value}")).size(9.5).color(pal.text_dim));
    }
    if !tool.output.is_empty() {
        ui.label(RichText::new("Output").size(9.5).strong().color(pal.text_dim));
        let plain = coiman_core::terminal::strip_colors(&tool.output).replace("\r\n", "\n").replace('\r', "\n");
        code_block(ui, plain.trim_end_matches(['\n', ' ']), pal, "text");
    }
    true
}

/// A readable tool payload: JSON keys become labels, strings become code blocks,
/// and edit tools show their old/new text without JSON escaping.
fn tool_details(ui: &mut egui::Ui, tool: &ToolPart, pal: theme::Palette) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(38.0);
        ui.vertical(|ui| {
            if shell_details(ui, tool, pal) {
                return;
            }
            if is_edit_tool(&tool.name) && edit_diff(ui, tool, pal) {
                if !tool.output.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(truncate(&tool.output, 240)).size(9.5).color(pal.text_dim));
                }
                return;
            }
            if !tool.input.is_empty() {
                ui.label(RichText::new("Input").size(9.5).strong().color(pal.text_dim));
                tool_payload(ui, &tool.input, pal);
                ui.add_space(3.0);
            }
            if !tool.output.is_empty() {
                ui.label(RichText::new("Output").size(9.5).strong().color(pal.text_dim));
                tool_payload(ui, &tool.output, pal);
            }
        });
    });
    ui.add_space(5.0);
}

/// Render an edit tool's old/new strings as a compact red/green line diff.
/// Returns false when the payload is not an edit with both sides.
fn edit_diff(ui: &mut egui::Ui, tool: &ToolPart, pal: theme::Palette) -> bool {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&tool.input) else {
        return false;
    };
    let path = map
        .get("path")
        .or_else(|| map.get("file_path"))
        .or_else(|| map.get("filePath"))
        .and_then(Value::as_str)
        .unwrap_or(&tool.title);
    let old = map.get("oldString").or_else(|| map.get("old_string")).and_then(Value::as_str);
    let new = map.get("newString").or_else(|| map.get("new_string")).and_then(Value::as_str);
    let (Some(old), Some(new)) = (old, new) else {
        return false;
    };

    let old_lines = coiman_core::project::git::lines(old);
    let new_lines = coiman_core::project::git::lines(new);
    let hunks = coiman_core::project::git::diff_lines(&old_lines, &new_lines, false);

    let mut rows: Vec<(Option<usize>, Option<usize>, char, String)> = Vec::new();
    let (mut oi, mut ni, mut on, mut nn) = (0usize, 0usize, 1usize, 1usize);
    for hunk in &hunks {
        while oi < hunk.old.start && ni < hunk.new.start {
            if let Some(line) = old_lines.get(oi) {
                rows.push((Some(on), Some(nn), ' ', (*line).to_owned()));
                on += 1;
                nn += 1;
            }
            oi += 1;
            ni += 1;
        }
        for _ in 0..hunk.old.len().max(hunk.new.len()) {
            if oi < hunk.old.end {
                if let Some(line) = old_lines.get(oi) {
                    rows.push((Some(on), None, '-', (*line).to_owned()));
                    on += 1;
                }
                oi += 1;
            }
            if ni < hunk.new.end {
                if let Some(line) = new_lines.get(ni) {
                    rows.push((None, Some(nn), '+', (*line).to_owned()));
                    nn += 1;
                }
                ni += 1;
            }
        }
    }
    while oi < old_lines.len() || ni < new_lines.len() {
        if oi < old_lines.len() {
            rows.push((Some(on), Some(nn), ' ', old_lines[oi].to_owned()));
            on += 1;
            oi += 1;
        }
        if ni < new_lines.len() {
            rows.push((Some(on.saturating_sub(1)), Some(nn), ' ', new_lines[ni].to_owned()));
            nn += 1;
            ni += 1;
        }
    }

    let path_label =
        Path::new(path).file_name().map_or_else(|| path.to_owned(), |name| name.to_string_lossy().into_owned());
    egui::Frame::new()
        .fill(pal.editor_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(7, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                crate::ui::icons::paint_file(
                    ui.painter(),
                    egui::Rect::from_min_size(
                        egui::pos2(ui.cursor().left(), ui.cursor().top() + 2.0),
                        egui::vec2(11.0, 11.0),
                    ),
                    path,
                );
                ui.add_space(14.0);
                ui.label(RichText::new(path_label).size(10.0).strong().color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} lines", rows.len())).size(9.5).color(pal.text_dim));
                });
            });
            ui.add_space(3.0);
            for (index, (old_no, new_no, sign, text)) in rows.iter().take(400).enumerate() {
                let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 15.0), Sense::hover());
                let (bg, fg) = match sign {
                    '-' => (pal.removed.gamma_multiply(0.14), pal.removed),
                    '+' => (pal.added.gamma_multiply(0.14), pal.added),
                    _ => (Color32::TRANSPARENT, pal.text_dim),
                };
                if *sign != ' ' {
                    ui.painter().rect_filled(row, 0.0, bg);
                }
                let old_label = old_no.map_or(String::new(), |n| n.to_string());
                let new_label = new_no.map_or(String::new(), |n| n.to_string());
                ui.painter().text(
                    egui::pos2(row.left() + 18.0, row.center().y),
                    Align2::RIGHT_CENTER,
                    old_label,
                    FontId::monospace(9.0),
                    pal.text_dim,
                );
                ui.painter().text(
                    egui::pos2(row.left() + 38.0, row.center().y),
                    Align2::RIGHT_CENTER,
                    new_label,
                    FontId::monospace(9.0),
                    pal.text_dim,
                );
                ui.painter().text(
                    egui::pos2(row.left() + 46.0, row.center().y),
                    Align2::LEFT_CENTER,
                    sign.to_string(),
                    FontId::monospace(10.0),
                    fg,
                );
                ui.painter().text(
                    egui::pos2(row.left() + 58.0, row.center().y),
                    Align2::LEFT_CENTER,
                    fit_text(ui, &text.replace('\t', "  "), FontId::monospace(10.0), fg, row.width() - 64.0),
                    FontId::monospace(10.0),
                    fg,
                );
                if index + 1 == 400 && rows.len() > 400 {
                    ui.label(RichText::new("…").size(9.0).color(pal.text_dim));
                }
            }
        });
    true
}

fn tool_payload(ui: &mut egui::Ui, text: &str, pal: theme::Palette) {
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
        mono_text(ui, &truncate(text, 6_000), pal.text, 10.0);
        return;
    };

    let mut entries: Vec<_> = map.iter().collect();
    entries.sort_by_key(|(key, _)| match key.as_str() {
        "path" | "file_path" | "filePath" => 0,
        "oldString" | "old_string" => 1,
        "newString" | "new_string" => 2,
        _ => 3,
    });

    for (key, value) in entries {
        ui.horizontal(|ui| {
            ui.add_sized(
                [78.0, 0.0],
                egui::Label::new(RichText::new(key).size(9.5).monospace().color(pal.text_dim))
                    .wrap_mode(egui::TextWrapMode::Truncate),
            );
            ui.vertical(|ui| match value {
                Value::String(text) => mono_text(ui, &truncate(text, 6_000), pal.text, 10.0),
                Value::Null => mono_text(ui, "null", pal.text_dim, 10.0),
                other => mono_text(
                    ui,
                    &truncate(&serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()), 6_000),
                    pal.text,
                    10.0,
                ),
            });
        });
        ui.add_space(2.0);
    }
}

/// Highlighted lines of a code block, shared across frames.
type HighlightLines = Option<std::rc::Rc<Vec<Vec<highlight::Span>>>>;

/// A code block with syntect highlighting, cached by language and text.
fn code_block(ui: &mut egui::Ui, text: &str, pal: theme::Palette, lang: &str) {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    thread_local! {
        static CACHE: RefCell<HashMap<(String, String), HighlightLines>> = RefCell::new(HashMap::new());
    }
    let text = truncate(text, 12_000);
    let key = (lang.to_lowercase(), text.clone());
    let lines = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 64 {
            cache.clear();
        }
        cache.entry(key).or_insert_with(|| highlight::snippet(lang, &text).map(Rc::new)).clone()
    });
    let top = ui.cursor().left_top();
    egui::Frame::new()
        .fill(pal.editor_bg)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(5.0)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            match &lines {
                Some(lines) => {
                    let mut job = LayoutJob::default();
                    let format =
                        |color: Color32| TextFormat { font_id: FontId::monospace(12.0), color, ..Default::default() };
                    for line in &**lines {
                        let mut col = 0usize;
                        for span in line {
                            let end = col + span.len;
                            if end <= text.len() && text.is_char_boundary(col) && text.is_char_boundary(end) {
                                job.append(&text[col..end], 0.0, format(theme::c32(span.color)));
                            }
                            col = end;
                        }
                        job.append("\n", 0.0, format(pal.text));
                    }
                    ui.add(egui::Label::new(job).wrap());
                }
                None => {
                    ui.add(egui::Label::new(RichText::new(&text).monospace().size(12.0).color(pal.text)).wrap());
                }
            }
        });
    // Copy on hover, over the block's top-right corner.
    let rect =
        Rect::from_min_max(top, egui::pos2(ui.min_rect().right(), ui.cursor().top() - ui.spacing().item_spacing.y));
    let id = egui::Id::unique("agents_code_copy").with(top.x as u32).with(top.y as u32);
    if ui.rect_contains_pointer(rect) {
        let button = Rect::from_min_size(egui::pos2(rect.right() - 18.0, rect.top() + 4.0), egui::vec2(16.0, 16.0));
        let resp = ui.interact(button, id, Sense::click());
        ui.painter().rect_filled(button, 3.0, pal.widget_bg);
        crate::ui::icons::paint(ui.painter(), button.shrink(2.5), crate::ui::icons::Icon::Copy, pal.text_dim);
        if resp.clicked() {
            ui.ctx().copy_text(text);
        }
    }
}

/// A small copy button over the message's top-right corner, while it's hovered.
fn copy_overlay(ui: &mut egui::Ui, rect: Rect, id: egui::Id, text: String, pal: theme::Palette) {
    if rect.width() <= 0.0 || !ui.rect_contains_pointer(rect) {
        return;
    }
    let button = Rect::from_min_size(egui::pos2(rect.right() - 18.0, rect.top() + 2.0), egui::vec2(16.0, 16.0));
    let resp = ui.interact(button, id, Sense::click());
    ui.painter().rect_filled(button, 3.0, pal.editor_bg);
    // A quick press animation: the icon dips and springs back.
    let anim = ui.ctx().animate_bool_with_time(id.with("pressed"), resp.is_pointer_button_down_on(), 0.12);
    let scale = 1.0 - 0.18 * anim;
    let icon = egui::Rect::from_center_size(button.center(), egui::vec2(11.0, 11.0) * scale);
    crate::ui::icons::paint(ui.painter(), icon, crate::ui::icons::Icon::Copy, pal.text_dim);
    if resp.clicked() {
        ui.ctx().copy_text(text);
    }
}

/// A 22x24 icon button sized for the mock-up's 30px title/context strips.
fn bar_icon_button(
    ui: &mut egui::Ui,
    icon: crate::ui::icons::Icon,
    tooltip: &str,
    pal: theme::Palette,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, pal.hover_bg);
    }
    crate::ui::icons::paint(
        ui.painter(),
        egui::Rect::from_min_size(egui::pos2(rect.left() + 5.0, rect.center().y - 6.5), egui::vec2(13.0, 13.0)),
        icon,
        if response.hovered() { pal.text } else { pal.text_dim },
    );
    response.on_hover_text(tooltip)
}

/// A flat chip like the mockup's tool-select: label, hover fill, optional caret.
fn chip(
    ui: &mut egui::Ui,
    pal: theme::Palette,
    label: &str,
    caret: bool,
    icon: Option<crate::ui::icons::Icon>,
) -> egui::Response {
    let width = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(10.5), pal.text).size().x
        + f32::from(caret) * 9.0
        + if icon.is_some() { 12.0 } else { 0.0 }
        + 6.0;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 21.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, pal.hover_bg);
    }
    let mut x = rect.left() + 3.0;
    if let Some(icon) = icon {
        crate::ui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 5.0), egui::vec2(10.0, 10.0)),
            icon,
            pal.text_dim,
        );
        x += 12.0;
    }
    ui.painter().text(egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(10.5), pal.text);
    if caret {
        crate::ui::icons::paint(
            ui.painter(),
            egui::Rect::from_min_size(egui::pos2(rect.right() - 13.0, rect.center().y - 4.0), egui::vec2(8.0, 8.0)),
            crate::ui::icons::Icon::ChevronDown,
            pal.text_dim,
        );
    }
    resp
}

/// A copy of `text` that fits `max` pixels, with an ellipsis — for texts drawn
/// straight onto a painter, which never clip or wrap.
fn fit_text(ui: &egui::Ui, text: &str, font: FontId, color: Color32, max: f32) -> String {
    if max <= 0.0 {
        return String::new();
    }
    if ui.painter().layout_no_wrap(text.to_owned(), font.clone(), color).size().x <= max {
        return text.to_owned();
    }
    let mut end = text.len();
    while end > 0 {
        // Step back to the previous character boundary.
        let mut start = end - 1;
        while start > 0 && !text.is_char_boundary(start) {
            start -= 1;
        }
        end = start;
        let candidate = format!("{}…", &text[..end]);
        if ui.painter().layout_no_wrap(candidate.clone(), font.clone(), color).size().x <= max {
            return candidate;
        }
    }
    "…".to_owned()
}

/// The `@token` at the end of the ask box, if any: (start, token).
fn at_token(text: &str) -> Option<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        if bytes[i] == b'@' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            let rest = &text[i + 1..];
            if rest.contains(char::is_whitespace) {
                return None;
            }
            return Some((i, rest));
        }
    }
    None
}

/// The folder's entries matching the `@token`: its directory and prefix.
fn at_entries(root: Option<&Path>, token: &str) -> Vec<String> {
    let Some(root) = root else { return Vec::new() };
    let (dir, prefix) = token.rsplit_once('/').map_or(("", token), |(dir, prefix)| (dir, prefix));
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(root.join(dir)) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(prefix) {
                entries.push(name);
            }
        }
    }
    entries.sort();
    entries
}

/// Mixes two colors, for the composer's focus border.
fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b()))
}

fn mono_text(ui: &mut egui::Ui, text: &str, color: Color32, size: f32) {
    ui.add(egui::Label::new(RichText::new(text).monospace().size(size).color(color)).wrap());
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn markdown(ui: &mut egui::Ui, text: &str, pal: theme::Palette) {
    use coiman_core::language::markdown::Block;
    for block in coiman_core::language::markdown::parse(text) {
        match block {
            Block::Heading { level, text, .. } => {
                let size = 14.0 + (3 - level.min(3)) as f32;
                ui.add(egui::Label::new(inline_text(&text, pal, size)).wrap());
                ui.add_space(2.0);
            }
            Block::Paragraph(text) => {
                ui.add(egui::Label::new(inline_text(&text, pal, 11.8)).wrap());
                ui.add_space(2.0);
            }
            Block::Code { lang, text } => {
                code_block(ui, &text, pal, &lang);
                ui.add_space(2.0);
            }
            Block::List { items, .. } => {
                for item in items {
                    ui.horizontal(|ui| {
                        ui.label("•");
                        ui.vertical(|ui| {
                            for block in item.blocks {
                                match block {
                                    Block::Paragraph(text) => {
                                        ui.add(egui::Label::new(inline_text(&text, pal, 11.8)).wrap());
                                    }
                                    Block::Code { lang, text } => code_block(ui, &text, pal, &lang),
                                    _ => {}
                                }
                            }
                        });
                    });
                }
            }
            Block::Quote(blocks) => {
                for block in blocks {
                    if let Block::Paragraph(text) = block {
                        ui.add(egui::Label::new(inline_text(&text, pal, 11.8)).wrap());
                    }
                }
            }
            Block::Rule => {
                ui.separator();
            }
            Block::Table { header, rows, .. } => {
                for row in header.iter().chain(rows.iter().flatten()) {
                    ui.label(RichText::new(plain_inline(row)).monospace().small());
                }
            }
        }
    }
}

fn inline_text(inlines: &[Inline], pal: theme::Palette, size: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for inline in inlines {
        match inline {
            Inline::Text(text, style) => {
                let font = if style.code { FontId::monospace(size) } else { FontId::proportional(size) };
                // Links and file references take the accent, like the mockup's file links.
                let color =
                    if style.link.is_some() || (style.code && is_file_like(text)) { pal.accent } else { pal.text };
                job.append(
                    text,
                    0.0,
                    TextFormat {
                        font_id: font,
                        color,
                        italics: style.italic,
                        strikethrough: if style.strike { egui::Stroke::new(1.0, color) } else { egui::Stroke::NONE },
                        ..Default::default()
                    },
                );
            }
            Inline::Image { alt, .. } => {
                job.append(
                    &format!("[{alt}]"),
                    0.0,
                    TextFormat {
                        font_id: FontId::proportional(size),
                        color: pal.text_dim,
                        italics: true,
                        ..Default::default()
                    },
                );
            }
            Inline::Break => job.append(
                "\n",
                0.0,
                TextFormat { font_id: FontId::proportional(size), color: pal.text, ..Default::default() },
            ),
        }
    }
    job
}

fn plain_inline(inlines: &[Inline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text, _) => out.push_str(text),
            Inline::Image { alt, .. } => out.push_str(&format!("[{alt}]")),
            Inline::Break => out.push('\n'),
        }
    }
    out
}

fn json_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

/// The sessions rail's "2 wks ago" style timestamp.
fn session_time_label(updated: u64) -> String {
    let date = local_date(updated);
    let minutes = (chrono::Local::now() - date).num_minutes().max(0);
    if minutes < 1 {
        "now".to_owned()
    } else if minutes < 60 {
        format!("{minutes}m ago")
    } else if minutes < 60 * 24 {
        format!("{}h ago", minutes / 60)
    } else if minutes < 60 * 24 * 7 {
        format!("{}d ago", minutes / (60 * 24))
    } else if minutes < 60 * 24 * 30 {
        format!("{}w ago", minutes / (60 * 24 * 7))
    } else if minutes < 60 * 24 * 365 {
        format!("{}mo ago", minutes / (60 * 24 * 30))
    } else {
        date.format("%b %-d").to_string()
    }
}

/// A short label for when a session was last used: the time today, else the date.
fn time_label(updated: u64) -> String {
    let date = local_date(updated);
    if date.date_naive() == chrono::Local::now().date_naive() {
        date.format("%H:%M").to_string()
    } else {
        date.format("%b %-d").to_string()
    }
}

/// A message's plain text: its text parts, without thinking or tools.
fn message_text(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The whole conversation as markdown, for "Copy conversation".
fn transcript_markdown(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        match message.role {
            Role::User => out.push_str("## You\n"),
            Role::Agent => out.push_str("## opencode\n"),
        }
        let text = message_text(message);
        if !text.is_empty() {
            out.push_str(&text);
            out.push('\n');
        }
        for part in &message.parts {
            if let Part::Tool(tool) = part {
                out.push_str(&format!("- {}/{}\n", tool.name, tool.title));
            }
        }
        out.push('\n');
    }
    out
}

/// Files edited in the last turn, from the file-editing tools' inputs.
fn files_touched(messages: &[Message]) -> Vec<String> {
    let start = messages.iter().rposition(|message| message.role == Role::User).map_or(0, |index| index + 1);
    let mut files: Vec<String> = Vec::new();
    for message in &messages[start.min(messages.len())..] {
        for part in &message.parts {
            if let Part::Tool(tool) = part {
                if !is_edit_tool(&tool.name) || tool.status == "error" {
                    continue;
                }
                if let Some(path) = path_from_input(&tool.input)
                    && !files.contains(&path)
                {
                    files.push(path);
                }
            }
        }
    }
    files.truncate(20);
    files
}

/// How many tools the last turn used.
fn tools_in_last_turn(messages: &[Message]) -> usize {
    let start = messages.iter().rposition(|message| message.role == Role::User).map_or(0, |index| index + 1);
    messages[start.min(messages.len())..]
        .iter()
        .flat_map(|message| &message.parts)
        .filter(|part| matches!(part, Part::Tool(_)))
        .count()
}

fn is_edit_tool(name: &str) -> bool {
    matches!(
        name,
        "edit"
            | "write"
            | "str_replace"
            | "str_replace_editor"
            | "multiedit"
            | "multi_edit"
            | "apply_patch"
            | "applypatch"
            | "notebookedit"
            | "notebook_edit"
            | "create_file"
            | "create-file"
            | "save_file"
            | "patch_file"
    )
}

/// The first file path in a tool call's JSON input.
fn path_from_input(input: &str) -> Option<String> {
    let value: Value = serde_json::from_str(input).ok()?;
    fn walk(value: &Value, key: Option<&str>, out: &mut Option<String>) {
        if out.is_some() {
            return;
        }
        match value {
            Value::Object(map) => {
                for (name, item) in map {
                    walk(item, Some(name), out);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, key, out);
                }
            }
            Value::String(text)
                if matches!(
                    key,
                    Some("file_path" | "filePath" | "path" | "file" | "filename" | "notebook_path" | "notebookPath")
                ) && is_file_like(text) =>
            {
                *out = Some(text.clone());
            }
            _ => {}
        }
    }
    let mut found = None;
    walk(&value, None, &mut found);
    found.or_else(|| {
        let mut strings = Vec::new();
        collect_strings(&value, &mut strings);
        strings.into_iter().find(|text| is_file_like(text))
    })
}

/// Collects every string in a JSON value, depth first.
fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// Whether a token looks like a file path: no spaces, with a directory or extension.
fn is_file_like(text: &str) -> bool {
    let trimmed = text.trim().trim_start_matches('@').trim_matches('`').trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) || trimmed.len() > 240 {
        return false;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return true;
    }
    // An extension at the end, like `README.md` but not `1.5`.
    match trimmed.rsplit_once('.') {
        Some((base, extension)) => {
            !base.is_empty()
                && base.chars().any(|c| c.is_ascii_alphabetic())
                && !extension.is_empty()
                && extension.len() <= 6
                && extension.chars().all(|c| c.is_ascii_alphanumeric())
                && extension.chars().any(|c| c.is_ascii_alphabetic())
        }
        None => false,
    }
}

fn local_date(updated: u64) -> chrono::DateTime<chrono::Local> {
    let time = UNIX_EPOCH + std::time::Duration::from_millis(updated);
    chrono::DateTime::<chrono::Local>::from(time)
}

impl CodeApp {
    /// Prepares the Agents panel: the current folder and the model list.
    pub(crate) fn agents_prepare(&mut self) {
        if !self.settings.workbench.agents_enabled {
            self.agents.open = false;
            return;
        }
        let root = if self.file_manager.is_showing() {
            self.file_manager.current_dir()
        } else {
            self.explorer
                .root
                .clone()
                .or_else(|| self.docs.get(self.active).and_then(|d| d.path.as_deref()?.parent()).map(Path::to_path_buf))
        };
        self.agents.root = root;
        self.agents.load_models();
        self.agents.active_file = self.docs.get(self.active).and_then(|doc| doc.path.clone());
    }
}

#[cfg(test)]
// The tests build panels in many states, field by field.
#[allow(clippy::field_reassign_with_default, clippy::type_complexity)]
mod tests {
    use super::*;
    use coiman_core::extensions::plugin::host::{ChatMessage, ChatSession, ChatTool};

    /// A view the Agents extension fills.
    fn filled() -> AgentsUi {
        AgentsUi { plugin: Some("coiman.agents".to_owned()), name: "OpenCode".to_owned(), ..Default::default() }
    }

    fn view(from: u32, messages: Vec<ChatMessage>) -> ChatView {
        ChatView {
            name: "OpenCode".to_owned(),
            missing: None,
            session: "ses_1".to_owned(),
            title: "A chat".to_owned(),
            messages_from: from,
            messages,
            running: false,
            loading: false,
            status: String::new(),
            sessions: None,
            sessions_loading: false,
            models: None,
            models_loading: false,
            model: String::new(),
            mode: "build".to_owned(),
            permissions: "manual".to_owned(),
            turn_seconds: None,
        }
    }

    fn said(user: bool, text: &str) -> ChatMessage {
        ChatMessage { from_user: user, parts: vec![ChatPart::Text(text.to_owned())] }
    }

    #[test]
    fn the_extension_fills_the_view_a_piece_at_a_time() {
        let mut agents = AgentsUi::default();
        agents.apply("coiman.agents", view(0, vec![said(true, "hi"), said(false, "Hel")]));
        assert_eq!(agents.plugin.as_deref(), Some("coiman.agents"));
        assert_eq!(agents.session_title.as_deref(), Some("A chat"));
        // Only the last message comes again as it grows.
        agents.apply("coiman.agents", view(1, vec![said(false, "Hello")]));
        assert_eq!(agents.messages.len(), 2);
        assert_eq!(message_text(&agents.messages[1]), "Hello");
        // A tool card, and the working set it makes.
        let edit = ChatPart::Tool(ChatTool {
            id: "t".to_owned(),
            name: "edit".to_owned(),
            title: "edit".to_owned(),
            input: "{\"filePath\": \"src/a.rs\"}".to_owned(),
            output: String::new(),
            status: "completed".to_owned(),
        });
        agents.apply("coiman.agents", view(1, vec![ChatMessage { from_user: false, parts: vec![edit] }]));
        assert_eq!(agents.last_changed_files, ["src/a.rs"]);
        // Nothing from 2 on: the two stay.
        agents.apply("coiman.agents", view(2, vec![]));
        assert_eq!(agents.messages.len(), 2);
        // From 0 with none: cleared.
        agents.apply("coiman.agents", view(0, vec![]));
        assert!(agents.messages.is_empty());
        // Lists come only when they changed.
        let mut v = view(0, vec![]);
        v.sessions = Some(vec![ChatSession { id: "s".to_owned(), title: "S".to_owned(), updated: 3 }]);
        v.models = Some(vec!["a/b".to_owned()]);
        agents.apply("coiman.agents", v);
        agents.apply("coiman.agents", view(0, vec![]));
        assert_eq!(agents.history.len(), 1);
        assert_eq!(agents.models, ["a/b"]);
        agents.detach("coiman.agents");
        assert!(agents.plugin.is_none());
    }

    #[test]
    fn what_is_done_goes_to_the_extension() {
        let mut agents = filled();
        let dir = std::env::temp_dir().join(format!("coiman-agents-send-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        agents.input = "hello".to_owned();
        agents.attached = vec!["src/a.rs".to_owned()];
        agents.send(Some(&dir));
        assert!(agents.running, "running until the extension says otherwise");
        assert!(agents.input.is_empty() && agents.attached.is_empty());
        agents.stop();
        agents.new_chat();
        let (plugin, actions) = agents.take_actions().unwrap();
        assert_eq!(plugin, "coiman.agents");
        let names: Vec<&str> = actions.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(names, ["send", "models", "stop", "new"]);
        assert_eq!(
            (actions[0].argument.as_str(), actions[0].attachments.clone()),
            ("hello", vec!["src/a.rs".to_owned()])
        );
        assert!(agents.take_actions().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_untrusted_folder_sends_nothing() {
        let mut agents = filled();
        agents.input = "hello".to_owned();
        agents.send(None);
        assert!(agents.take_actions().is_none());
        assert!(agents.status.as_deref().unwrap_or_default().contains("restricted"));
    }

    #[test]
    fn stop_clears_the_queue() {
        let mut agents = AgentsUi { queue: vec!["a queued message".to_owned()], running: true, ..filled() };
        agents.stop();
        assert!(agents.queue.is_empty(), "a stopped run drops its queue");
        assert!(!agents.running);
    }

    #[test]
    fn run_end_drains_one_queued_message() {
        let mut agents = filled();
        let mut running = view(0, vec![said(true, "q")]);
        running.running = true;
        agents.apply("coiman.agents", running);
        agents.queue = vec!["one".to_owned(), "two".to_owned()];
        // No folder: the send stops at trust, and the text waits in the box.
        agents.apply("coiman.agents", view(1, vec![said(false, "done")]));
        assert_eq!(agents.input, "one", "the first queued message moves to the box");
        assert_eq!(agents.queue.len(), 1, "the rest wait for the next run");
        assert!(agents.focus_input);
    }

    #[test]
    fn without_the_extension_the_view_says_so() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi::default();
        panel_frame(&ctx, &mut agents);
        assert!(agents.composer_rect.is_none(), "no box to type in");
        let mut missing = filled();
        missing.missing = Some("npm install -g opencode-ai".to_owned());
        panel_frame(&ctx, &mut missing);
        assert!(missing.composer_rect.is_none());
    }

    #[test]
    fn shell_command_and_output_are_read_without_json_or_escape_codes() {
        let (command, rest) = shell_input(r#"{"command":"printf hi","description":"say hi"}"#);
        assert_eq!(command, "printf hi");
        assert_eq!(rest, vec![("description".to_owned(), "say hi".to_owned())]);
        assert_eq!(shell_input("echo hi").0, "echo hi");
        let plain = coiman_core::terminal::strip_colors("\u{1b}[31mhi\u{1b}[0m\n");
        assert_eq!(plain, "hi\n");
    }

    #[test]
    fn working_set_uses_the_last_turns_edit_tools() {
        let edit = |path: &str, status: &str| {
            Part::Tool(ToolPart {
                id: "call".to_owned(),
                name: "edit".to_owned(),
                title: "edit".to_owned(),
                input: serde_json::to_string(&serde_json::json!({ "file_path": path })).unwrap(),
                output: String::new(),
                status: status.to_owned(),
            })
        };
        let read = Part::Tool(ToolPart {
            id: "call".to_owned(),
            name: "read".to_owned(),
            title: "read".to_owned(),
            input: serde_json::to_string(&serde_json::json!({ "path": "src/lib.rs" })).unwrap(),
            output: String::new(),
            status: "completed".to_owned(),
        });
        let messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("first".to_owned())] },
            Message { role: Role::Agent, parts: vec![read.clone(), edit("old.rs", "completed")] },
            Message { role: Role::User, parts: vec![Part::Text("second".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![edit("a.rs", "completed"), edit("b.rs", "completed"), edit("failed.rs", "error"), read],
            },
        ];
        // Only the last turn counts, and only its successful file edits.
        assert_eq!(files_touched(&messages), vec!["a.rs".to_owned(), "b.rs".to_owned()]);
        assert_eq!(tools_in_last_turn(&messages), 4);
        assert_eq!(message_text(&messages[0]), "first");
    }

    #[test]
    fn paths_come_from_tool_inputs() {
        let json =
            serde_json::to_string(&serde_json::json!({ "command": "cat src/main.rs", "file_path": "src/app.rs" }))
                .unwrap();
        assert_eq!(path_from_input(&json).as_deref(), Some("src/app.rs"));

        let nested = serde_json::to_string(&serde_json::json!({ "edits": [{ "path": "lib/main.rs" }] })).unwrap();
        assert_eq!(path_from_input(&nested).as_deref(), Some("lib/main.rs"));

        let prose = serde_json::to_string(&serde_json::json!({ "command": "say hello" })).unwrap();
        assert_eq!(path_from_input(&prose), None);
    }

    #[test]
    fn file_like_tokens() {
        assert!(is_file_like("src/app.rs"));
        assert!(is_file_like("@README.md"));
        assert!(is_file_like("`Cargo.toml`"));
        assert!(is_file_like("main.c"));
        assert!(!is_file_like("hello world"));
        assert!(!is_file_like("1.5"));
        assert!(!is_file_like("3.14"));
    }

    #[test]
    fn session_time_labels() {
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
        assert_eq!(session_time_label(now), "now");
        assert_eq!(session_time_label(now - 90 * 1000), "1m ago");
        assert_eq!(session_time_label(now - 3 * 3600 * 1000), "3h ago");
        assert_eq!(session_time_label(now - 14 * 24 * 3600 * 1000), "2w ago");
    }

    // ---- The composer's input handling, driven headlessly ----------------

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        let modifiers = egui::Modifiers::default();
        vec![
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers },
        ]
    }

    /// Runs one composer frame and returns the box's rect.
    fn composer_frame(
        ctx: &egui::Context,
        agents: &mut AgentsUi,
        running: bool,
        modifiers: egui::Modifiers,
        mut events: Vec<egui::Event>,
    ) -> egui::Rect {
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(420.0, 320.0))),
            ..Default::default()
        };
        // egui derives the frame's modifiers from this event, like a real window.
        events.insert(0, egui::Event::ModifiersChanged(modifiers));
        raw.events = events;
        let mut box_rect = egui::Rect::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            box_rect = agents.composer(ui, theme::colors(), running);
        });
        // Headless: clear the font texture deltas instead of dropping them.
        output.textures_delta.clear();
        box_rect
    }

    #[test]
    fn composer_typing_needs_focus_and_enter_sends() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        let none = egui::Modifiers::default();

        let box_rect = composer_frame(&ctx, &mut agents, false, none, vec![]);
        assert!(!agents.input_focused, "the box starts unfocused");

        // Typing without focus goes nowhere.
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("hi".to_owned())]);
        assert_eq!(agents.input, "", "typing without focus must not land");
        // Enter without focus sends nothing.
        composer_frame(&ctx, &mut agents, false, none, vec![key_event(egui::Key::Enter, none)]);
        assert!(agents.queue.is_empty() && agents.input.is_empty());

        // Click the box: it takes focus.
        // Click the prompt line. The context row appears only when a file is
        // attached, so an empty composer starts with the text area near the top.
        let at = egui::pos2(box_rect.center().x, box_rect.top() + 15.0);
        composer_frame(&ctx, &mut agents, false, none, click_events(at));
        assert!(agents.input_focused, "clicking the box should focus it");

        // Now typing lands.
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("hello".to_owned())]);
        assert_eq!(agents.input, "hello");

        // Shift+Enter is a newline, not a send.
        composer_frame(
            &ctx,
            &mut agents,
            false,
            egui::Modifiers::SHIFT,
            vec![key_event(egui::Key::Enter, egui::Modifiers::SHIFT)],
        );
        assert_eq!(agents.input, "hello\n", "shift-enter is a newline");
        assert!(agents.queue.is_empty());

        // Enter queues while the agent runs (no process is spawned).
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::Enter, none)]);
        assert_eq!(agents.queue, vec!["hello".to_owned()]);
        assert_eq!(agents.input, "", "enter clears the box");

        // The queue row appearing above the box must not steal focus: typing
        // still lands (this is the id-salt fix).
        composer_frame(&ctx, &mut agents, true, none, vec![egui::Event::Text("more".to_owned())]);
        assert_eq!(agents.input, "more", "focus must survive the queue row appearing");

        // Enter queues the second message too.
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::Enter, none)]);
        agents.prompt_history.push("first prompt".to_owned());
        // Up arrow in an empty box recalls the last prompt.
        composer_frame(&ctx, &mut agents, true, none, vec![key_event(egui::Key::ArrowUp, none)]);
        assert_eq!(agents.input, "first prompt", "up arrow recalls history");

        // /clear clears the conversation without running anything.
        agents.messages = vec![Message { role: Role::Agent, parts: vec![Part::Text("old".to_owned())] }];
        agents.input.clear();
        composer_frame(&ctx, &mut agents, false, none, vec![egui::Event::Text("/clear".to_owned())]);
        composer_frame(&ctx, &mut agents, false, none, vec![key_event(egui::Key::Enter, none)]);
        assert!(agents.messages.is_empty(), "/clear empties the conversation");
        assert_eq!(agents.status.as_deref(), Some("New session"));
    }

    #[test]
    fn focus_request_lands_in_the_box() {
        let ctx = egui::Context::default();
        // toggle() / focus_chat() / attach() set this flag.
        let mut agents = AgentsUi { focus_input: true, ..filled() };
        composer_frame(&ctx, &mut agents, false, egui::Modifiers::default(), vec![]);
        assert!(agents.input_focused, "focus_input focuses the box");
    }

    fn panel_frame(ctx: &egui::Context, agents: &mut AgentsUi) {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(520.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| agents.panel(ui));
        output.textures_delta.clear();
    }

    /// Runs the panel like `mod.rs` does — inside a right `Panel` — and returns
    /// the panel's rect, so the width ratchet can be measured.
    fn panel_with_chrome(ctx: &egui::Context, agents: &mut AgentsUi, screen: egui::Vec2) -> egui::Rect {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
            ..Default::default()
        };
        let mut rect = egui::Rect::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                rect = egui::Panel::right(egui::Id::unique("agents_panel_test"))
                    .default_size(430.0)
                    .size_range(360.0..=980.0)
                    .resizable(true)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(theme::colors().sidebar_bg).inner_margin(egui::Margin::ZERO))
                    .show(ui, |ui| agents.panel(ui))
                    .response
                    .rect;
            });
        });
        output.textures_delta.clear();
        rect
    }

    #[test]
    fn panel_width_stays_put_when_sessions_open() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
            Message { role: Role::Agent, parts: vec![Part::Text("Sure.".to_owned())] },
        ];
        // Closed: the panel keeps its default width.
        let closed = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        // Opening the sessions column must not inflate the panel.
        agents.history_open = true;
        agents.history = vec![SessionInfo { id: "ses_1".to_owned(), title: "A session".to_owned(), updated: 1 }];
        let mut open = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        for _ in 0..8 {
            open = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        }
        assert!(closed.width() <= 460.0, "closed panel stays ~430: {:.1}", closed.width());
        assert!(open.width() <= 470.0, "sessions column must not inflate the panel: {:.1}", open.width());
    }

    fn panel_wide_with_chrome(ctx: &egui::Context, agents: &mut AgentsUi, screen: egui::Vec2) -> egui::Rect {
        agents.history = vec![SessionInfo { id: "ses_1".to_owned(), title: "A session".to_owned(), updated: 1 }];
        agents.history_loaded = true;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen)),
            ..Default::default()
        };
        let mut rect = egui::Rect::ZERO;
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::no_frame().show(ui, |ui| {
                let mut panel = egui::Panel::right(egui::Id::unique("agents_panel_wide_test"))
                    .default_size(820.0)
                    .size_range(420.0..=1200.0)
                    .resizable(true)
                    .show_separator_line(true)
                    .frame(egui::Frame::new().fill(theme::colors().sidebar_bg).inner_margin(egui::Margin::ZERO));
                if agents.maximized {
                    panel = panel.exact_size(ui.available_width().max(200.0));
                }
                rect = panel.show(ui, |ui| agents.panel(ui)).response.rect;
            });
        });
        output.textures_delta.clear();
        rect
    }

    #[test]
    fn chat_composer_keeps_a_compact_height() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi { history_open: false, ..filled() };
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("hello".to_owned())] },
            Message { role: Role::Agent, parts: vec![Part::Text("hi".to_owned())] },
        ];
        let _ = panel_wide_with_chrome(&ctx, &mut agents, egui::vec2(1100.0, 800.0));
        let composer = agents.composer_rect.expect("the chat composer was drawn");
        assert!(composer.height() >= COMPOSER_MIN_HEIGHT - 1.0, "the composer stays compact: {:.1}", composer.height());
    }

    #[test]
    fn wide_workspace_renders_sessions() {
        let ctx = egui::Context::default();
        let mut agents = AgentsUi {
            history_open: true,
            last_changed_files: vec!["crates/coiman-ide/src/app/agents.rs".to_owned()],
            ..filled()
        };
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("Fix the UI".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![
                    Part::Tool(ToolPart {
                        id: "call_1".to_owned(),
                        name: "edit".to_owned(),
                        title: "agents.rs".to_owned(),
                        input: r#"{"file_path":"crates/coiman-ide/src/app/agents.rs"}"#.to_owned(),
                        output: String::new(),
                        status: "completed".to_owned(),
                    }),
                    Part::Text("Updated the agent workspace.".to_owned()),
                ],
            },
        ];
        agents.last_turn_tools = tools_in_last_turn(&agents.messages);
        let rect = panel_wide_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        assert!(rect.width() >= 700.0, "the Agents workspace opens wide: {:.1}", rect.width());
        assert!(agents.composer_rect.is_none(), "the full-screen Sessions picker replaces the conversation");
        // The full-screen picker must fit without panicking or widening the panel.
        let mut previous = rect;
        for _ in 0..30 {
            let next = panel_wide_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
            assert!(
                (next.width() - previous.width()).abs() < 1.5,
                "wide layout must be stable: {:.1} -> {:.1}",
                previous.width(),
                next.width()
            );
            previous = next;
        }
    }

    #[test]
    fn panel_width_is_stable_in_every_state() {
        let ctx = egui::Context::default();
        // Each state is rendered repeatedly: the panel's width must not drift.
        let states: Vec<(&str, Box<dyn Fn(&mut AgentsUi)>)> = vec![
            ("plain", Box::new(|_| {})),
            (
                "sessions",
                Box::new(|a: &mut AgentsUi| {
                    a.history_open = true;
                    a.history = vec![
                        SessionInfo { id: "ses_1".to_owned(), title: "One".to_owned(), updated: 3 },
                        SessionInfo { id: "ses_2".to_owned(), title: "Two".to_owned(), updated: 2 },
                    ];
                    a.session_search_open = true;
                }),
            ),
            (
                "working set",
                Box::new(|a: &mut AgentsUi| {
                    a.last_changed_files = vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned()];
                }),
            ),
            (
                "long prompt",
                Box::new(|a: &mut AgentsUi| {
                    a.messages = vec![
                        Message {
                            role: Role::User,
                            parts: vec![Part::Text(
                                "a very long prompt that must wrap inside the bubble and never widen the panel "
                                    .repeat(8),
                            )],
                        },
                        Message { role: Role::Agent, parts: vec![Part::Text("ok".to_owned())] },
                    ];
                }),
            ),
            (
                "attached and queued",
                Box::new(|a: &mut AgentsUi| {
                    a.attached = vec!["/home/mostafa/src/main.rs".to_owned()];
                    a.queue = vec!["a queued message that is fairly long".to_owned()];
                }),
            ),
            (
                "running",
                Box::new(|a: &mut AgentsUi| {
                    a.running = true;
                    a.queue = vec!["a queued message".to_owned()];
                    a.last_changed_files = vec!["src/main.rs".to_owned()];
                }),
            ),
            (
                "searching sessions",
                Box::new(|a: &mut AgentsUi| {
                    a.history_open = true;
                    a.history = vec![
                        SessionInfo { id: "ses_1".to_owned(), title: "One".to_owned(), updated: 3 },
                        SessionInfo { id: "ses_2".to_owned(), title: "Two".to_owned(), updated: 2 },
                    ];
                    a.session_search_open = true;
                    a.session_search = "on".to_owned();
                }),
            ),
        ];
        for (name, setup) in &states {
            let mut agents = filled();
            agents.messages = vec![
                Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
                Message { role: Role::Agent, parts: vec![Part::Text("Working on it.".to_owned())] },
            ];
            setup(&mut agents);
            // The panel sizes itself (default 430 or the content's min); what
            // must not happen is drift between frames.
            let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
            for _ in 0..5 {
                rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
            }
            let first = rect.width();
            let mut drift = 0.0_f32;
            for _ in 0..10 {
                let next = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
                drift = drift.max((next.width() - rect.width()).abs());
                rect = next;
            }
            assert!(drift <= 1.5, "{name}: the panel jitters frame to frame ({drift:.1})");
            assert!(
                (rect.width() - first).abs() <= 1.5,
                "{name}: the panel creeps over time ({:.1} -> {:.1})",
                first,
                rect.width()
            );
        }
        // A narrow window: the panel gives way but must not jitter.
        for (name, setup) in &states {
            let mut agents = filled();
            agents.messages = vec![
                Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
                Message { role: Role::Agent, parts: vec![Part::Text("Working on it.".to_owned())] },
            ];
            setup(&mut agents);
            let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
            for _ in 0..5 {
                rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
            }
            let first = rect.width();
            for _ in 0..10 {
                let next = panel_with_chrome(&ctx, &mut agents, egui::vec2(420.0, 900.0));
                assert!(
                    (next.width() - rect.width()).abs() <= 1.5,
                    "{name} narrow: the panel jitters ({:.1} -> {:.1})",
                    rect.width(),
                    next.width()
                );
                rect = next;
            }
            assert!(
                (rect.width() - first).abs() <= 1.5,
                "{name} narrow: the panel creeps ({:.1} -> {:.1})",
                first,
                rect.width()
            );
            if !agents.history_open {
                let composer = agents.composer_rect.expect("the composer was drawn");
                assert!(
                    (composer.bottom() - rect.bottom()).abs() < 30.0,
                    "{name} narrow: the composer stays at the bottom ({:?} vs {:?})",
                    composer,
                    rect
                );
            }
        }
    }

    #[test]
    fn opening_a_session_returns_to_chat() {
        let mut agents = AgentsUi { history_open: true, ..filled() };
        agents.open_session(SessionInfo { id: "ses_1".to_owned(), title: "One".to_owned(), updated: 1 });
        assert!(!agents.history_open, "selecting a session returns to the chat");
    }

    #[test]
    fn fit_text_truncates_to_the_width() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 300.0))),
            ..Default::default()
        };
        let mut result = String::new();
        let mut output = ctx.run_ui(raw, |ui| {
            result = fit_text(
                ui,
                "a very long title that will not fit in a small space at all",
                FontId::proportional(12.0),
                Color32::WHITE,
                100.0,
            );
        });
        output.textures_delta.clear();
        assert!(result.ends_with('…'), "long text is ellipsized: {result:?}");
        assert!(result.len() < 40, "the fit is short: {result:?}");
    }

    #[test]
    fn thinking_headers_do_not_clash() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("hi".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![
                    Part::Thinking("first thought".to_owned()),
                    Part::Thinking("second thought".to_owned()),
                    Part::Thinking("third thought".to_owned()),
                ],
            },
        ];
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(520.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| agents.panel(ui));
        output.textures_delta.clear();
        // egui paints an error text for id clashes; none must appear.
        let clash = output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => text.galley.text().contains("Widget is above"),
            _ => false,
        });
        assert!(!clash, "thinking headers must have unique ids");
    }

    #[test]
    fn long_code_lines_do_not_widen_the_panel() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        // A 400-char code line: the block must truncate, not widen the panel.
        let long_line = "let x = \"".to_owned() + &"y".repeat(390) + "\";";
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("show me".to_owned())] },
            Message { role: Role::Agent, parts: vec![Part::Text(format!("Here:\n\n```rust\n{long_line}\n```"))] },
        ];
        let rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        assert!(rect.width() <= 470.0, "long code lines must not widen the panel: {:.1}", rect.width());
    }

    #[test]
    fn resumed_session_renders_sane() {
        let ctx = egui::Context::default();
        // A resumed session: three prompts and their replies, one with a tool.
        let text = |t: &str| Part::Text(t.to_owned());
        let user = |t: &str| Message { role: Role::User, parts: vec![text(t)] };
        let agent = |parts: Vec<Part>| Message { role: Role::Agent, parts };
        let shell = Part::Tool(ToolPart {
            id: "call_1".to_owned(),
            name: "shell".to_owned(),
            title: "shell".to_owned(),
            input: "{\"command\": \"pwd\"}".to_owned(),
            output: "/home/mostafa\n".to_owned(),
            status: "completed".to_owned(),
        });
        let messages = vec![
            user("hi hi"),
            agent(vec![text("Hi! How can I help you today?")]),
            user("What can you see?"),
            agent(vec![shell, text("I can see the project files.")]),
            user("Show me the entry point."),
            agent(vec![text("Here it is:\n\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```")]),
        ];
        let mut agents = AgentsUi { messages, ..filled() };
        agents.session_title = Some("Greeting".to_owned());
        agents.last_changed_files = files_touched(&agents.messages);
        agents.last_turn_tools = tools_in_last_turn(&agents.messages);
        let mut rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        for _ in 0..6 {
            rect = panel_with_chrome(&ctx, &mut agents, egui::vec2(1400.0, 900.0));
        }
        assert!(rect.width() <= 470.0, "a resumed session must not inflate the panel: {:.1}", rect.width());
        let composer = agents.composer_rect.expect("the composer was drawn");
        assert!(
            (composer.bottom() - rect.bottom()).abs() < 30.0,
            "the composer stays at the bottom: composer={:?} panel={:?}",
            composer,
            rect
        );
    }

    #[test]
    fn edit_tool_renders_a_line_diff() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        agents.history_loaded = true;
        agents.messages = vec![Message {
            role: Role::Agent,
            parts: vec![Part::Tool(ToolPart {
                id: "call_1".to_owned(),
                name: "edit".to_owned(),
                title: "test.html".to_owned(),
                input: serde_json::json!({
                    "path": "/tmp/test.html",
                    "oldString": "old\nline",
                    "newString": "new\nline"
                })
                .to_string(),
                output: "Edited test.html (1 replacement)".to_owned(),
                status: "completed".to_owned(),
            })],
        }];
        let rect = panel_wide_with_chrome(&ctx, &mut agents, egui::vec2(1100.0, 800.0));
        assert!(rect.width() > 0.0, "the edit diff should render");
    }

    #[test]
    fn panel_renders_every_section() {
        let ctx = egui::Context::default();
        let mut agents = filled();
        agents.messages = vec![
            Message { role: Role::User, parts: vec![Part::Text("Build a thing".to_owned())] },
            Message {
                role: Role::Agent,
                parts: vec![
                    Part::Thinking("hmm".to_owned()),
                    Part::Text("Sure, here is the plan:\n\n```rust\nfn main() {}\n```".to_owned()),
                    Part::Tool(ToolPart {
                        id: "call_1".to_owned(),
                        name: "edit".to_owned(),
                        title: "edit".to_owned(),
                        input: r#"{"file_path": "src/main.rs"}"#.to_owned(),
                        output: "ok".to_owned(),
                        status: "completed".to_owned(),
                    }),
                ],
            },
        ];
        agents.last_changed_files = files_touched(&agents.messages);
        agents.last_turn_tools = tools_in_last_turn(&agents.messages);
        agents.history = vec![
            SessionInfo { id: "ses_1".to_owned(), title: "A session".to_owned(), updated: 1 },
            SessionInfo { id: "ses_2".to_owned(), title: "Another".to_owned(), updated: 2 },
        ];
        agents.history_open = true;
        agents.input = "a draft".to_owned();
        // Every section at once: sessions column, working set, tool cards,
        // code block, delete modal, jump pill.
        agents.delete_pending = Some(agents.history[0].clone());
        panel_frame(&ctx, &mut agents);
        // And with a run in flight (spinner, stop button, queue row).
        agents.delete_pending = None;
        agents.queue.push("queued".to_owned());
        agents.running = true;
        panel_frame(&ctx, &mut agents);
        agents.running = false;

        // Narrow windows must not panic (the negative-size crash class).
        let mut narrow = AgentsUi { history_open: true, ..filled() };
        narrow.messages = agents.messages.clone();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(240.0, 400.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| narrow.panel(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn session_search_matches_titles_and_ids() {
        let mut agents = filled();
        agents.history = vec![
            SessionInfo { id: "ses_alpha".to_owned(), title: "Alpha plan".to_owned(), updated: 1 },
            SessionInfo { id: "ses_beta".to_owned(), title: "Beta work".to_owned(), updated: 2 },
        ];
        agents.session_search = "alp".to_owned();
        let matched = agents.matching_sessions();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].id, "ses_alpha");
        agents.session_search = "beta".to_owned();
        assert_eq!(agents.matching_sessions().len(), 1, "ids match too");
        agents.session_search.clear();
        assert_eq!(agents.matching_sessions().len(), 2, "an empty query keeps all");
    }

    #[test]
    fn duplicate_consecutive_prompt_is_not_pushed_twice() {
        let mut agents = AgentsUi {
            input: "hello".to_owned(),
            messages: vec![Message { role: Role::User, parts: vec![Part::Text("hello".to_owned())] }],
            ..filled()
        };
        agents.send(Some(std::env::temp_dir().as_path()));
        assert_eq!(agents.messages.len(), 1, "the duplicate prompt must not be appended");
        assert!(agents.input.is_empty(), "the duplicate prompt is consumed");
    }

    #[test]
    fn model_search_is_loose_and_labels_are_clean() {
        let mut agents = AgentsUi { models: vec!["google/antigravity-gemini-3.8-flash-high".to_owned()], ..filled() };
        agents.model_search = "Gemini 3.8 flash".to_owned();
        assert_eq!(agents.matching_models().len(), 1, "loose search ignores punctuation/case");
        assert_eq!(model_label("google/antigravity-gemini-3.8-flash-high"), "Antigravity Gemini 3.8 Flash High");
        assert!(!truncate(&model_label("google/antigravity-gemini-3.8-flash-high"), 50).ends_with('…'));
    }

    #[test]
    fn model_search_filters_the_list() {
        let mut agents =
            AgentsUi { models: vec!["google/gemini-3.5-flash".to_owned(), "kilo/claude".to_owned()], ..filled() };
        agents.model_search = "gem".to_owned();
        assert_eq!(agents.matching_models(), vec!["google/gemini-3.5-flash".to_owned()]);
        agents.model_search.clear();
        assert_eq!(agents.matching_models().len(), 2);
    }

    #[test]
    fn at_token_finds_the_reference() {
        assert_eq!(at_token("hello @"), Some((6, "")));
        assert_eq!(at_token("@src/main"), Some((0, "src/main")));
        assert_eq!(at_token("ask @src/ma"), Some((4, "src/ma")));
        assert_eq!(at_token("no reference"), None);
        assert_eq!(at_token("email@example.com"), None, "not a token mid-word");
        assert_eq!(at_token("done @file and more"), None, "space after the token ends it");
    }

    #[test]
    fn at_entries_match_the_prefix() {
        let dir = std::env::temp_dir().join("coiman_at_entries_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "x").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "x").unwrap();
        std::fs::write(dir.join("README.md"), "x").unwrap();
        let entries = at_entries(Some(&dir), "src/ma");
        assert_eq!(entries, vec!["main.rs".to_owned()]);
        let root_entries = at_entries(Some(&dir), "");
        assert!(root_entries.contains(&"README.md".to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attached_context_prefixes_the_prompt() {
        let mut agents = AgentsUi { attached: vec!["/tmp/a.rs".to_owned(), "/tmp/b.rs".to_owned()], ..filled() };
        assert_eq!(agents.with_attachments("hi"), "@/tmp/a.rs @/tmp/b.rs\nhi");
        agents.attached.clear();
        assert_eq!(agents.with_attachments("hi"), "hi");
    }
}
