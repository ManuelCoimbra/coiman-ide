//! Language server features refreshed as you work: semantic tokens, inlay
//! hints, highlights of the symbol under the cursor, signature help, and
//! workspace symbols. All are read-only views of what the server reports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use coiman_core::language::lsp::servers::ServerKind;
use coiman_core::language::lsp::types as lt;
use coiman_core::language::lsp::{Client, convert};
use egui::{Color32, RichText};
use serde_json::{Value, json};

use super::CodeApp;
use super::lsp::{Pending, path_key};
use crate::editor_view::{Decorations, Highlight, Inlays};
use crate::palette::PickerKind;
use crate::theme;

/// Seconds after the last change before asking again.
const REFRESH_DELAY: f64 = 0.3;

type LineHighlights = Arc<HashMap<usize, Vec<Highlight>>>;

/// A symbol from `workspace/symbol`.
#[derive(Clone)]
pub(super) struct WorkspaceSymbol {
    pub(super) name: String,
    pub(super) detail: String,
    pub(super) path: PathBuf,
    pub(super) range: lt::Range,
}

pub(super) struct Signature {
    uid: u64,
    label: String,
    /// The active parameter, as a char range of `label`.
    active: Option<(usize, usize)>,
    documentation: String,
}

/// Which relation of the symbol under the cursor to list.
#[derive(Clone, Copy)]
pub(super) enum Hierarchy {
    IncomingCalls,
    OutgoingCalls,
    Supertypes,
    Subtypes,
}

impl Hierarchy {
    fn prepare(self) -> &'static str {
        match self {
            Self::IncomingCalls | Self::OutgoingCalls => "textDocument/prepareCallHierarchy",
            Self::Supertypes | Self::Subtypes => "textDocument/prepareTypeHierarchy",
        }
    }

    fn method(self) -> &'static str {
        match self {
            Self::IncomingCalls => "callHierarchy/incomingCalls",
            Self::OutgoingCalls => "callHierarchy/outgoingCalls",
            Self::Supertypes => "typeHierarchy/supertypes",
            Self::Subtypes => "typeHierarchy/subtypes",
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            Self::IncomingCalls => "Incoming Calls",
            Self::OutgoingCalls => "Outgoing Calls",
            Self::Supertypes => "Supertypes",
            Self::Subtypes => "Subtypes",
        }
    }

    /// Where an answer's entries keep their item.
    pub(super) fn field(self) -> Option<&'static str> {
        match self {
            Self::IncomingCalls => Some("from"),
            Self::OutgoingCalls => Some("to"),
            Self::Supertypes | Self::Subtypes => None,
        }
    }
}

/// Lenses resolved per answer: each one is a request, and a large file has many.
const MAX_LENS_RESOLVES: usize = 100;

/// A symbol in a hierarchy, and what's known of what's under it.
struct HierNode {
    item: Value,
    name: String,
    detail: String,
    place: Option<(PathBuf, lt::Range)>,
    /// `None` until asked for.
    children: Option<Vec<usize>>,
    open: bool,
}

/// A call or type hierarchy, opened a level at a time.
pub(super) struct HierTree {
    hierarchy: Hierarchy,
    id: u64,
    word: String,
    nodes: Vec<HierNode>,
    roots: Vec<usize>,
}

impl HierTree {
    fn add(&mut self, item: &Value) -> usize {
        let place = serde_json::from_value::<lt::Url>(item["uri"].clone()).ok().and_then(|uri| {
            let range = serde_json::from_value(item["selectionRange"].clone()).ok()?;
            Some((convert::path_from_uri(&uri)?, range))
        });
        self.nodes.push(HierNode {
            item: item.clone(),
            name: item["name"].as_str().unwrap_or("?").to_owned(),
            detail: item["detail"].as_str().unwrap_or_default().to_owned(),
            place,
            children: None,
            open: false,
        });
        self.nodes.len() - 1
    }
}

#[derive(Default)]
pub(super) struct LspExtra {
    pub(super) tree: Option<HierTree>,
    next_tree: u64,
    /// Code lenses per document: (line count they were made for, lenses).
    lenses: HashMap<u64, (usize, Vec<lt::CodeLens>)>,
    /// Inlay hints and lens titles together, as drawn.
    shown_inlays: HashMap<u64, (usize, Inlays)>,
    picked_lenses: Vec<lt::Command>,
    /// Semantic colors per document: (line count they were made for, by line).
    tokens: HashMap<u64, (usize, LineHighlights)>,
    inlays: HashMap<u64, (usize, Inlays)>,
    /// (doc uid, version) last asked about, and when that version was first seen.
    asked: Option<(u64, u64)>,
    seen: Option<(u64, u64, f64)>,
    /// (doc uid, version, cursor) of the symbol highlights, and when the cursor stopped.
    symbol_at: Option<(u64, u64, usize)>,
    symbol_seen: Option<(u64, u64, usize, f64)>,
    symbol_highlights: Option<(u64, u64, LineHighlights)>,
    signature: Option<Signature>,
    pub(super) workspace_symbols: Vec<WorkspaceSymbol>,
    symbols_query: Option<String>,
}

/// What a semantic token type looks like, as a highlight capture name, when the theme
/// has no semantic rule for it. Kinds of type each have their own, falling back to `type`.
fn capture_of(token_type: &str, modifiers: &[&str]) -> Option<&'static str> {
    let constant = modifiers.iter().any(|m| matches!(*m, "readonly" | "constant"));
    let library = modifiers.contains(&"defaultLibrary");
    Some(match token_type {
        "function" if library => "function.builtin",
        "function" => "function",
        "method" => "function.method",
        "macro" => "function.macro",
        "decorator" => "attribute",
        "class" if library => "type.builtin",
        "class" => "type.class",
        "struct" => "type.struct",
        "interface" => "type.interface",
        "enum" => "type.enum",
        "typeParameter" => "type.parameter",
        "type" | "typeAlias" | "builtinType" => "type",
        "enumMember" => "constant.enum",
        "variable" | "property" if constant => "constant",
        "parameter" => "variable.parameter",
        "property" => "property",
        "selfKeyword" | "selfTypeKeyword" => "variable.builtin",
        "namespace" => "module",
        "lifetime" | "label" => "label",
        _ => return None,
    })
}

/// A semantic token as drawn: color (transparent for none), italic, struck through.
type TokenLook = (Color32, bool, bool);

fn decode_tokens(
    rope: &ropey::Rope,
    data: &[u32],
    legend: &(Vec<String>, Vec<String>),
) -> HashMap<usize, Vec<Highlight>> {
    let current = theme::current();
    let (types, modifier_names) = legend;
    // By (type, modifier bits): what it looks like. Few combinations recur.
    let mut looks: HashMap<(u32, u32), Option<TokenLook>> = HashMap::new();
    let mut look = |kind: u32, bits: u32| {
        *looks.entry((kind, bits)).or_insert_with(|| {
            let token_type = types.get(kind as usize)?;
            let modifiers: Vec<&str> = modifier_names
                .iter()
                .enumerate()
                .filter(|(i, _)| *i < 32 && bits & (1 << i) != 0)
                .map(|(_, m)| m.as_str())
                .collect();
            let semantic = theme::semantic_style(&current, token_type, &modifiers);
            let color = semantic.color.or_else(|| {
                capture_of(token_type, &modifiers).and_then(|c| theme::capture_style(&current, c)).map(|(c, _)| c)
            });
            let (italic, strike) = (semantic.italic, semantic.strike);
            (color.is_some() || italic || strike)
                .then(|| (color.map_or(Color32::TRANSPARENT, theme::c32), italic, strike))
        })
    };
    let mut lines: HashMap<usize, Vec<Highlight>> = HashMap::new();
    let (mut line, mut start) = (0u32, 0u32);
    for token in data.as_chunks::<5>().0 {
        let (delta_line, delta_start, length, kind, bits) = (token[0], token[1], token[2], token[3], token[4]);
        if delta_line > 0 {
            line += delta_line;
            start = delta_start;
        } else {
            start += delta_start;
        }
        let Some((color, italic, strike)) = look(kind, bits) else { continue };
        let (a, b) = convert::chars_of(
            rope,
            lt::Range::new(lt::Position::new(line, start), lt::Position::new(line, start + length)),
        );
        let line_start = rope.line_to_char(rope.char_to_line(a));
        lines.entry(line as usize).or_default().push(Highlight {
            start: a - line_start,
            end: b - line_start,
            foreground: (color != Color32::TRANSPARENT).then_some(color),
            background: None,
            italic,
            strike,
            frame: None,
        });
    }
    lines
}

/// The server's token types and modifiers, by index.
fn legend_of(client: &Client) -> Option<(Vec<String>, Vec<String>)> {
    let legend = match client.capabilities()?.semantic_tokens_provider.as_ref()? {
        lt::SemanticTokensServerCapabilities::SemanticTokensOptions(o) => &o.legend,
        lt::SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(o) => &o.semantic_tokens_options.legend,
    };
    let types = legend.token_types.iter().map(|t| t.as_str().to_owned()).collect();
    let modifiers = legend.token_modifiers.iter().map(|m| m.as_str().to_owned()).collect();
    Some((types, modifiers))
}

/// A title as drawn. A text-style selector after a symbol would show as a circle.
fn lens_title(title: &str) -> String {
    title.replace('\u{fe0e}', "")
}

fn inlay_text(hint: &lt::InlayHint) -> String {
    let label = match &hint.label {
        lt::InlayHintLabel::String(s) => s.clone(),
        lt::InlayHintLabel::LabelParts(parts) => parts.iter().map(|p| p.value.as_str()).collect(),
    };
    let before = if hint.padding_left == Some(true) { " " } else { "" };
    let after = if hint.padding_right == Some(true) { " " } else { "" };
    format!("{before}{label}{after}")
}

impl CodeApp {
    fn active_client(&self) -> Option<usize> {
        self.doc_client(self.active)
    }

    /// Runs each frame: asks again once the text or cursor settled.
    pub(super) fn refresh_lsp_views(&mut self, ctx: &egui::Context) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let (uid, version, head, empty) = {
            let p = doc.buffer.primary();
            (doc.uid, doc.buffer.version(), p.head, p.is_empty())
        };
        let now = ctx.input(|i| i.time);
        let Some(ci) = self.active_client() else { return };

        if self.lsp.extra.asked != Some((uid, version)) {
            let waited = match self.lsp.extra.seen {
                Some((u, v, since)) if (u, v) == (uid, version) => now - since,
                _ => {
                    self.lsp.extra.seen = Some((uid, version, now));
                    0.0
                }
            };
            if waited >= REFRESH_DELAY {
                self.lsp.extra.asked = Some((uid, version));
                self.request_document_views(ci, uid);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_DELAY));
            }
        }

        // The symbol under a resting cursor.
        if !empty {
            self.lsp.extra.symbol_highlights = None;
        } else if self.lsp.extra.symbol_at != Some((uid, version, head)) {
            let waited = match self.lsp.extra.symbol_seen {
                Some((u, v, h, since)) if (u, v, h) == (uid, version, head) => now - since,
                _ => {
                    self.lsp.extra.symbol_seen = Some((uid, version, head, now));
                    0.0
                }
            };
            if waited >= REFRESH_DELAY {
                self.lsp.extra.symbol_at = Some((uid, version, head));
                let i = self.active;
                let supported =
                    self.lsp.clients[ci].capabilities().is_some_and(|c| c.document_highlight_provider.is_some());
                if supported {
                    self.request_for(
                        i,
                        "textDocument/documentHighlight",
                        Pending::SymbolHighlights { uid, version },
                        |doc, td| Self::position_params(doc, td, head),
                    );
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_DELAY));
            }
        }
    }

    fn request_document_views(&mut self, ci: usize, uid: u64) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let lines = doc.buffer.len_lines();
        let (semantic, inlays) = (self.settings.editor.semantic_highlighting, self.settings.editor.inlay_hints);
        let caps = self.lsp.clients[ci].capabilities();
        let has_tokens = semantic && caps.is_some_and(|c| c.semantic_tokens_provider.is_some());
        let has_inlays = inlays && caps.is_some_and(|c| c.inlay_hint_provider.is_some());
        if !has_tokens {
            self.lsp.extra.tokens.remove(&uid);
        }
        if !has_inlays {
            self.lsp.extra.inlays.remove(&uid);
        }
        let has_lens = self.settings.editor.code_lens && caps.is_some_and(|c| c.code_lens_provider.is_some());
        if !has_lens {
            self.lsp.extra.lenses.remove(&uid);
        }
        if caps.is_some_and(|c| c.diagnostic_provider.is_some())
            && let Some(path) = doc.path.clone()
        {
            self.request_for(
                i,
                "textDocument/diagnostic",
                Pending::PullDiagnostics { path },
                |_, td| json!({ "textDocument": td }),
            );
        }
        if has_tokens {
            self.request_for(
                i,
                "textDocument/semanticTokens/full",
                Pending::SemanticTokens { uid, ci, lines },
                |_, td| json!({ "textDocument": td }),
            );
        }
        if has_inlays {
            self.request_for(i, "textDocument/inlayHint", Pending::InlayHints { uid, lines }, |doc, td| {
                let end = convert::position_of(doc.buffer.rope(), doc.buffer.len_chars());
                json!({ "textDocument": td, "range": { "start": { "line": 0, "character": 0 }, "end": end } })
            });
        }
        if has_lens {
            self.request_for(
                i,
                "textDocument/codeLens",
                Pending::CodeLens { uid },
                |_, td| json!({ "textDocument": td }),
            );
        }
        self.rebuild_inlays(uid);
    }

    pub(super) fn on_code_lens(&mut self, uid: u64, value: Value) {
        let Ok(Some(lenses)) = serde_json::from_value::<Option<Vec<lt::CodeLens>>>(value) else { return };
        let Some(i) = self.doc_index(uid) else { return };
        let resolves = self
            .active_client()
            .and_then(|ci| self.lsp.clients[ci].capabilities()?.code_lens_provider.as_ref()?.resolve_provider);
        if resolves == Some(true) {
            for (index, lens) in lenses.iter().enumerate().filter(|(_, l)| l.command.is_none()).take(MAX_LENS_RESOLVES)
            {
                self.request_for(i, "codeLens/resolve", Pending::CodeLensResolve { uid, index }, |_, _| json!(lens));
            }
        }
        let lines = self.docs[i].buffer.len_lines();
        self.lsp.extra.lenses.insert(uid, (lines, lenses));
        self.rebuild_inlays(uid);
    }

    pub(super) fn on_code_lens_resolved(&mut self, uid: u64, index: usize, value: Value) {
        let Ok(resolved) = serde_json::from_value::<lt::CodeLens>(value) else { return };
        let Some((_, lenses)) = self.lsp.extra.lenses.get_mut(&uid) else { return };
        // The list may have been replaced since this was asked.
        match lenses.get_mut(index) {
            Some(lens) if lens.range == resolved.range => *lens = resolved,
            _ => return,
        }
        self.rebuild_inlays(uid);
    }

    /// Merges inlay hints and lens titles, the titles at the end of their line.
    fn rebuild_inlays(&mut self, uid: u64) {
        let Some(i) = self.doc_index(uid) else { return };
        let rope = self.docs[i].buffer.rope();
        let lines = self.docs[i].buffer.len_lines();
        let extra = &mut self.lsp.extra;
        let mut map = extra
            .inlays
            .get(&uid)
            .filter(|(n, _)| *n == lines)
            .map_or_else(HashMap::new, |(_, hints)| (**hints).clone());
        let mut titles: HashMap<usize, Vec<String>> = HashMap::new();
        for lens in extra.lenses.get(&uid).filter(|(n, _)| *n == lines).into_iter().flat_map(|(_, l)| l) {
            if let Some(command) = &lens.command {
                titles.entry(lens.range.start.line as usize).or_default().push(lens_title(&command.title));
            }
        }
        for (line, titles) in titles {
            if line < rope.len_lines() {
                let text = rope.line(line).to_string();
                let end = text.chars().rev().skip_while(|c| matches!(c, '\n' | '\r')).count();
                map.entry(line).or_default().push((end, format!("  {}", titles.join(" | "))));
            }
        }
        extra.shown_inlays.insert(uid, (lines, Arc::new(map)));
    }

    pub(super) fn show_code_lens(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let line = doc.buffer.rope().char_to_line(doc.buffer.primary().head);
        let commands: Vec<lt::Command> = self
            .lsp
            .extra
            .lenses
            .get(&doc.uid)
            .into_iter()
            .flat_map(|(_, lenses)| lenses)
            .filter(|l| l.range.start.line as usize == line)
            .filter_map(|l| l.command.clone())
            .collect();
        if commands.is_empty() {
            self.status = Some("No code lens on this line".to_owned());
            return;
        }
        let titles = commands.iter().map(|c| c.title.clone()).collect();
        self.lsp.extra.picked_lenses = commands;
        self.palette.show_picker(PickerKind::CodeLens, titles, "", "Select a code lens");
    }

    /// A title beside the code was clicked. They're drawn joined, so `into` says which.
    pub(super) fn click_code_lens(&mut self, ctx: &egui::Context, line: usize, into: usize) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let lenses = self.lsp.extra.lenses.get(&doc.uid).into_iter().flat_map(|(_, lenses)| lenses);
        let commands: Vec<lt::Command> =
            lenses.filter(|l| l.range.start.line as usize == line).filter_map(|l| l.command.clone()).collect();
        let mut end = 2;
        for command in commands {
            end += lens_title(&command.title).chars().count() + 3;
            if into < end {
                self.run_lens_command(ctx, command);
                return;
            }
        }
    }

    pub(super) fn pick_code_lens(&mut self, ctx: &egui::Context, title: &str) {
        let commands = std::mem::take(&mut self.lsp.extra.picked_lenses);
        let Some(command) = commands.into_iter().find(|c| c.title == title) else { return };
        self.run_lens_command(ctx, command);
    }

    /// What a lens asks for: some things the editor does itself, the rest the server.
    fn run_lens_command(&mut self, ctx: &egui::Context, command: lt::Command) {
        let title = command.title.clone();
        let arg = |at: usize| command.arguments.as_ref().and_then(|a| a.get(at)).cloned().unwrap_or(Value::Null);
        match command.command.as_str() {
            "rust-analyzer.showReferences" | "editor.action.showReferences" | "java.show.references" => {
                self.on_locations("References", &title, arg(2));
                return;
            }
            "java.show.implementations" => {
                self.on_locations("Implementations", &title, arg(2));
                return;
            }
            "rust-analyzer.gotoLocation" => {
                let target = arg(0);
                let uri = serde_json::from_value::<lt::Url>(target["targetUri"].clone()).ok();
                let range = serde_json::from_value::<lt::Range>(target["targetSelectionRange"].clone()).ok();
                if let (Some(path), Some(range)) = (uri.as_ref().and_then(convert::path_from_uri), range) {
                    self.jump_to(&path, range, true);
                }
                return;
            }
            "rust-analyzer.runSingle" => {
                let runnable = arg(0);
                let words = |key: &str| -> Vec<String> {
                    let list = runnable["args"][key].as_array().map_or(&[][..], Vec::as_slice);
                    list.iter().filter_map(Value::as_str).map(str::to_owned).collect()
                };
                let quote = |w: String| {
                    if w.contains([' ', '\'']) { format!("'{}'", w.replace('\'', "''")) } else { w }
                };
                let mut line = vec!["cargo".to_owned()];
                line.extend(words("cargoArgs").into_iter().map(quote));
                let after = words("executableArgs");
                if !after.is_empty() {
                    line.push("--".to_owned());
                    line.extend(after.into_iter().map(quote));
                }
                let folder = runnable["args"]["cwd"].as_str().or(runnable["args"]["workspaceRoot"].as_str());
                let Some(cwd) = folder.map(PathBuf::from).or_else(|| self.explorer.root.clone()) else { return };
                let name = runnable["label"].as_str().unwrap_or(&title).to_owned();
                self.start_run(ctx, &coiman_core::launch::run::RunPlan { title: name, command: line.join(" "), cwd });
                return;
            }
            _ => {}
        }
        // Others belong to the server's VS Code extension, which isn't here.
        let runs_on_server = self.active_client().is_some_and(|ci| {
            self.lsp.clients[ci]
                .capabilities()
                .and_then(|c| c.execute_command_provider.as_ref())
                .is_some_and(|p| p.commands.contains(&command.command))
        });
        if !runs_on_server {
            self.status = Some(format!("'{title}' needs the server's VS Code extension ({})", command.command));
            return;
        }
        let args = command.arguments.unwrap_or_default();
        self.request_for(
            self.active,
            "workspace/executeCommand",
            Pending::Execute,
            |_, _| json!({ "command": command.command, "arguments": args }),
        );
    }

    pub(super) fn request_hierarchy(&mut self, hierarchy: Hierarchy) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let pos = doc.buffer.primary().head;
        let word = doc.buffer.word_at(pos).map(|(a, b)| doc.buffer.slice_string(a, b)).unwrap_or_default();
        self.request_for(i, hierarchy.prepare(), Pending::HierarchyPrepare { hierarchy, word }, |doc, td| {
            Self::position_params(doc, td, pos)
        });
    }

    pub(super) fn on_hierarchy_prepared(&mut self, hierarchy: Hierarchy, word: String, value: Value) {
        let Some(item) = value.as_array().and_then(|items| items.first()) else {
            self.status = Some(format!("No {} found for '{word}'", hierarchy.title().to_lowercase()));
            return;
        };
        self.status = None;
        self.lsp.extra.next_tree += 1;
        let id = self.lsp.extra.next_tree;
        let mut tree = HierTree { hierarchy, id, word, nodes: Vec::new(), roots: Vec::new() };
        let root = tree.add(item);
        tree.roots.push(root);
        self.lsp.extra.tree = Some(tree);
        self.panel_tab = super::PanelTab::References;
        self.panel_visible = true;
        self.toggle_hierarchy_node(root);
    }

    fn toggle_hierarchy_node(&mut self, node: usize) {
        let Some(tree) = &mut self.lsp.extra.tree else { return };
        let (hierarchy, id) = (tree.hierarchy, tree.id);
        let Some(entry) = tree.nodes.get_mut(node) else { return };
        entry.open = !entry.open;
        if entry.open && entry.children.is_none() {
            let params = json!({ "item": entry.item });
            let pending = Pending::HierarchyItems { hierarchy, node, tree: id };
            self.request_for(self.active, hierarchy.method(), pending, |_, _| params);
        }
    }

    pub(super) fn on_hierarchy_items(&mut self, hierarchy: Hierarchy, node: usize, id: u64, value: Value) {
        // A newer hierarchy may have taken this one's place.
        let Some(tree) = self.lsp.extra.tree.as_mut().filter(|t| t.id == id) else { return };
        let entries = value.as_array().map_or(&[][..], Vec::as_slice);
        let field = hierarchy.field();
        let children = entries.iter().map(|e| tree.add(field.map_or(e, |f| &e[f]))).collect();
        if let Some(entry) = tree.nodes.get_mut(node) {
            entry.children = Some(children);
        }
    }

    fn hierarchy_rows(
        ui: &mut egui::Ui,
        tree: &HierTree,
        list: &[usize],
        depth: usize,
        toggle: &mut Option<usize>,
        jump: &mut Option<(PathBuf, lt::Range)>,
    ) {
        let pal = theme::colors();
        let size = ui.text_style_height(&egui::TextStyle::Body);
        for &at in list {
            let node = &tree.nodes[at];
            let leaf = node.children.as_ref().is_some_and(Vec::is_empty);
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0);
                if leaf {
                    ui.add_space(size);
                } else {
                    let chevron =
                        if node.open { crate::icons::Icon::ChevronDown } else { crate::icons::Icon::ChevronRight };
                    if crate::icons::button(ui, chevron, "Expand").clicked() {
                        *toggle = Some(at);
                    }
                }
                let name = egui::Label::new(RichText::new(&node.name).monospace()).sense(egui::Sense::click());
                if ui.add(name).clicked() {
                    jump.clone_from(&node.place.clone());
                }
                if !node.detail.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&node.detail).small().color(pal.text_dim)).truncate());
                }
                if let Some((path, range)) = &node.place {
                    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let place = format!("{file}:{}", range.start.line + 1);
                    ui.label(RichText::new(place).small().color(pal.text_dim));
                }
            });
            if node.open {
                match &node.children {
                    Some(children) => Self::hierarchy_rows(ui, tree, children, depth + 1, toggle, jump),
                    None => {
                        ui.horizontal(|ui| {
                            ui.add_space((depth + 1) as f32 * 14.0 + size);
                            ui.label(RichText::new("…").color(pal.text_dim));
                        });
                    }
                }
            }
        }
    }

    /// Shows the hierarchy in the References panel, if that's what's there.
    pub(super) fn hierarchy_panel(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(tree) = &self.lsp.extra.tree else { return false };
        ui.label(RichText::new(format!("{} of '{}'", tree.hierarchy.title(), tree.word)).strong());
        let (mut toggle, mut jump) = (None, None);
        egui::ScrollArea::both().id_salt("hierarchy").auto_shrink([false, false]).show(ui, |ui| {
            Self::hierarchy_rows(ui, tree, &tree.roots, 0, &mut toggle, &mut jump);
        });
        if let Some(node) = toggle {
            self.toggle_hierarchy_node(node);
        }
        if let Some((path, range)) = jump {
            self.jump_to(&path, range, true);
        }
        true
    }

    pub(super) fn on_pulled_diagnostics(&mut self, path: PathBuf, value: Value) {
        // "unchanged" only answers a request naming an earlier result, which we don't send.
        if value["kind"] != "full" {
            return;
        }
        let items = serde_json::from_value::<Vec<lt::Diagnostic>>(value["items"].clone()).unwrap_or_default();
        if items.is_empty() {
            self.lsp.pulled_diagnostics.remove(&path_key(&path));
        } else {
            self.lsp.pulled_diagnostics.insert(path_key(&path), (path, items));
        }
    }

    /// A server asked to show a file or page. Files open; of web addresses only
    /// local ones do, so a server can't send the browser anywhere on its own.
    pub(super) fn on_show_document(&mut self, kind: ServerKind, params: &Value) -> bool {
        let Ok(url) = serde_json::from_value::<lt::Url>(params["uri"].clone()) else { return false };
        if let Some(path) = convert::path_from_uri(&url).filter(|_| url.scheme() == "file") {
            let range = serde_json::from_value::<lt::Range>(params["selection"].clone());
            match range {
                Ok(range) => self.jump_to(&path, range, true),
                Err(_) => match self.doc_index_by_path(&path) {
                    Some(i) => self.active = i,
                    None => self.open_file(&path),
                },
            }
            return true;
        }
        if convert::is_loopback_url(&url) {
            self.lsp.open_url = Some(url.to_string());
            return true;
        }
        if !matches!(url.scheme(), "http" | "https" | "file") {
            let range = serde_json::from_value::<lt::Range>(params["selection"].clone()).unwrap_or_default();
            self.request_virtual_document(url, range);
            return true;
        }
        self.status = Some(format!("{} asked to open {url}; not opened", kind.name()));
        false
    }

    /// Asks the server for a document only it can read.
    pub(super) fn request_virtual_document(&mut self, uri: lt::Url, range: lt::Range) {
        self.remember_virtual(&uri);
        let method = if uri.scheme() == "jdt" { "java/classFileContents" } else { "workspace/textDocumentContent" };
        let params = json!({ "uri": uri });
        self.request_for(self.active, method, Pending::VirtualDocument { uri, range }, |_, _| params);
    }

    /// Keeps the text in a file of its own and shows it, read-only.
    pub(super) fn on_virtual_document(&mut self, uri: &lt::Url, range: lt::Range, value: &Value) {
        let Some(text) = value.as_str().or_else(|| value["text"].as_str()) else {
            self.status = Some(format!("The language server has no text for {uri}"));
            return;
        };
        let Some(path) = convert::virtual_path(uri) else { return };
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, text));
        match written {
            Ok(()) => {
                // The server may have more to say now than when the tab opened.
                if let Some(i) = self.doc_index_by_path(&path)
                    && self.docs[i].text() != text
                {
                    // Read-only is for typing: the buffer refuses this too otherwise.
                    self.docs[i].buffer.read_only = false;
                    let reloaded = self.docs[i].reload_from_disk();
                    self.docs[i].buffer.read_only = true;
                    if let Err(e) = reloaded {
                        self.status = Some(format!("Could not refresh {uri}: {e}"));
                    }
                }
                self.jump_to(&path, range, false);
            }
            Err(e) => self.status = Some(format!("Could not keep {uri}: {e}")),
        }
    }

    /// The server's data changed (it finished indexing, say): ask again.
    pub(super) fn lsp_views_stale(&mut self) {
        self.lsp.extra.asked = None;
    }

    pub(super) fn on_semantic_tokens(&mut self, uid: u64, ci: usize, lines: usize, value: Value) {
        let Ok(Some(result)) = serde_json::from_value::<Option<lt::SemanticTokensResult>>(value) else { return };
        let lt::SemanticTokensResult::Tokens(tokens) = result else { return };
        let Some(legend) = self.lsp.clients.get(ci).and_then(legend_of) else { return };
        let Some(i) = self.doc_index(uid) else { return };
        let mut data = Vec::with_capacity(tokens.data.len() * 5);
        for t in &tokens.data {
            data.extend([t.delta_line, t.delta_start, t.length, t.token_type, t.token_modifiers_bitset]);
        }
        let map = decode_tokens(self.docs[i].buffer.rope(), &data, &legend);
        self.lsp.extra.tokens.insert(uid, (lines, Arc::new(map)));
    }

    pub(super) fn on_inlay_hints(&mut self, uid: u64, lines: usize, value: Value) {
        let Ok(Some(hints)) = serde_json::from_value::<Option<Vec<lt::InlayHint>>>(value) else { return };
        let Some(i) = self.doc_index(uid) else { return };
        let rope = self.docs[i].buffer.rope();
        let mut map: HashMap<usize, Vec<(usize, String)>> = HashMap::new();
        for hint in &hints {
            let pos = convert::char_of(rope, hint.position);
            let line = rope.char_to_line(pos);
            map.entry(line).or_default().push((pos - rope.line_to_char(line), inlay_text(hint)));
        }
        for list in map.values_mut() {
            list.sort_by_key(|h| h.0);
        }
        self.lsp.extra.inlays.insert(uid, (lines, Arc::new(map)));
        self.rebuild_inlays(uid);
    }

    pub(super) fn on_symbol_highlights(&mut self, uid: u64, version: u64, value: Value) {
        let Ok(Some(found)) = serde_json::from_value::<Option<Vec<lt::DocumentHighlight>>>(value) else { return };
        let Some(i) = self.doc_index(uid) else { return };
        let rope = self.docs[i].buffer.rope();
        let color = theme::colors().selection.gamma_multiply(0.45);
        let mut map: HashMap<usize, Vec<Highlight>> = HashMap::new();
        for h in &found {
            let (a, b) = convert::chars_of(rope, h.range);
            let line = rope.char_to_line(a);
            let start = rope.line_to_char(line);
            map.entry(line).or_default().push(Highlight {
                start: a - start,
                end: b - start,
                foreground: None,
                background: Some(color),
                italic: false,
                strike: false,
                frame: None,
            });
        }
        self.lsp.extra.symbol_highlights = Some((uid, version, Arc::new(map)));
    }

    /// Semantic colors, symbol highlights and inlay hints for document `i`.
    pub(super) fn lsp_views_decorate(&self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i) else { return };
        let lines = doc.buffer.len_lines();
        let extra = &self.lsp.extra;
        // Lines moved since: wrong until the server answers again.
        if let Some((_, map)) = extra.tokens.get(&doc.uid).filter(|(n, _)| *n == lines) {
            deco.highlights.push(map.clone());
        }
        if let Some((_, _, map)) =
            extra.symbol_highlights.as_ref().filter(|(u, v, _)| (*u, *v) == (doc.uid, doc.buffer.version()))
        {
            deco.highlights.push(map.clone());
        }
        if let Some((_, map)) = extra.shown_inlays.get(&doc.uid).filter(|(n, _)| *n == lines) {
            deco.inlays = Some(map.clone());
        }
    }

    /// Signature help: `(` and `,` ask; `)` closes.
    pub(super) fn signature_after_typing(&mut self, typed: &str) {
        match typed.chars().last() {
            Some('(' | ',') => self.request_signature(),
            Some(')') => self.lsp.extra.signature = None,
            _ => {}
        }
    }

    pub(super) fn request_signature(&mut self) {
        let i = self.active;
        let Some(doc) = self.docs.get(i) else { return };
        let (uid, head) = (doc.uid, doc.buffer.primary().head);
        let Some(ci) = self.active_client() else { return };
        if self.lsp.clients[ci].capabilities().is_some_and(|c| c.signature_help_provider.is_some()) {
            self.request_for(i, "textDocument/signatureHelp", Pending::Signature { uid }, |doc, td| {
                Self::position_params(doc, td, head)
            });
        }
    }

    pub(super) fn on_signature(&mut self, uid: u64, value: Value) {
        let help = serde_json::from_value::<Option<lt::SignatureHelp>>(value).ok().flatten();
        self.lsp.extra.signature = help.and_then(|h| {
            let index = h.active_signature.unwrap_or(0) as usize;
            let sig = h.signatures.get(index).or(h.signatures.first())?.clone();
            let param = sig.active_parameter.or(h.active_parameter).unwrap_or(0) as usize;
            let active = sig.parameters.as_ref().and_then(|p| p.get(param)).and_then(|p| match &p.label {
                lt::ParameterLabel::LabelOffsets([a, b]) => Some((*a as usize, *b as usize)),
                lt::ParameterLabel::Simple(text) => {
                    let byte = sig.label.find(text.as_str())?;
                    let start = sig.label[..byte].chars().count();
                    Some((start, start + text.chars().count()))
                }
            });
            let documentation = match &sig.documentation {
                Some(lt::Documentation::String(s)) => s.clone(),
                Some(lt::Documentation::MarkupContent(m)) => m.value.clone(),
                None => String::new(),
            };
            Some(Signature { uid, label: sig.label, active, documentation })
        });
    }

    /// The signature popup above the cursor.
    pub(super) fn signature_popup(&mut self, ctx: &egui::Context) {
        let Some(sig) = &self.lsp.extra.signature else { return };
        let (Some(rect), Some(doc)) = (self.lsp.cursor_rect, self.docs.get(self.active)) else { return };
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if doc.uid != sig.uid || escape {
            self.lsp.extra.signature = None;
            return;
        }
        let pal = theme::colors();
        egui::Area::new(egui::Id::unique("lsp_signature"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(rect.left_top() - egui::vec2(0.0, 2.0))
            .constrain(true)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(pal.sidebar_bg).inner_margin(6.0).show(ui, |ui| {
                    ui.set_max_width(640.0);
                    let font = egui::FontId::monospace(self.font_size * 0.92);
                    let mut job = egui::text::LayoutJob::default();
                    let (a, b) = sig.active.unwrap_or((usize::MAX, usize::MAX));
                    let part = |from: usize, to: usize| {
                        sig.label.chars().skip(from).take(to.saturating_sub(from)).collect::<String>()
                    };
                    let total = sig.label.chars().count();
                    let plain = |font: &egui::FontId, color| egui::TextFormat {
                        font_id: font.clone(),
                        color,
                        ..Default::default()
                    };
                    if a >= total {
                        job.append(&sig.label, 0.0, plain(&font, pal.text));
                    } else {
                        job.append(&part(0, a), 0.0, plain(&font, pal.text));
                        job.append(&part(a, b), 0.0, plain(&font, pal.accent));
                        job.append(&part(b, total), 0.0, plain(&font, pal.text));
                    }
                    ui.label(job);
                    if let Some(first) = sig.documentation.lines().find(|l| !l.trim().is_empty()) {
                        ui.label(RichText::new(first).small().color(pal.text_dim));
                    }
                });
            });
    }

    /// Runs each frame the palette is showing `#`: asks for matching symbols.
    pub(super) fn pump_workspace_symbols(&mut self) {
        let Some(query) = self.palette.symbol_query().map(str::to_owned) else {
            self.lsp.extra.symbols_query = None;
            return;
        };
        if self.lsp.extra.symbols_query.as_deref() == Some(query.as_str()) {
            return;
        }
        self.lsp.extra.symbols_query = Some(query.clone());
        let ci = self.active_client().or_else(|| self.lsp.clients.iter().position(Client::is_ready));
        if let Some(ci) = ci {
            let client = &mut self.lsp.clients[ci];
            let id = client.request("workspace/symbol", json!({ "query": query }));
            self.lsp.pending.insert((client.kind, client.root.clone(), id), Pending::WorkspaceSymbols);
        }
    }

    pub(super) fn on_workspace_symbols(&mut self, value: Value) {
        let Ok(Some(response)) = serde_json::from_value::<Option<lt::WorkspaceSymbolResponse>>(value) else { return };
        let symbols: Vec<WorkspaceSymbol> = match response {
            lt::WorkspaceSymbolResponse::Flat(list) => list
                .into_iter()
                .filter_map(|s| {
                    Some(WorkspaceSymbol {
                        path: convert::path_from_uri(&s.location.uri)?,
                        detail: s.container_name.unwrap_or_default(),
                        name: s.name,
                        range: s.location.range,
                    })
                })
                .collect(),
            lt::WorkspaceSymbolResponse::Nested(list) => list
                .into_iter()
                .filter_map(|s| {
                    let (uri, range) = match s.location {
                        lt::OneOf::Left(l) => (l.uri, l.range),
                        lt::OneOf::Right(l) => (l.uri, lt::Range::default()),
                    };
                    Some(WorkspaceSymbol {
                        path: convert::path_from_uri(&uri)?,
                        detail: s.container_name.unwrap_or_default(),
                        name: s.name,
                        range,
                    })
                })
                .collect(),
        };
        self.palette.symbols = symbols.iter().map(|s| (s.name.clone(), s.detail.clone())).collect();
        self.lsp.extra.workspace_symbols = symbols;
    }

    pub(super) fn picked_workspace_symbol(&mut self, index: usize) {
        if let Some(s) = self.lsp.extra.workspace_symbols.get(index).cloned() {
            self.jump_to(&s.path, s.range, true);
        }
    }
}
