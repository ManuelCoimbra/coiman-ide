//! Language server features refreshed as you work: semantic tokens, inlay
//! hints, highlights of the symbol under the cursor, signature help, and
//! workspace symbols. All are read-only views of what the server reports.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, RichText};
use rust_code_core::lsp::types as lt;
use rust_code_core::lsp::{Client, convert};
use serde_json::{Value, json};

use super::CodeApp;
use super::lsp::{Pending, lsp_target};
use crate::editor_view::{Decorations, Highlight, Inlays};
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

#[derive(Default)]
pub(super) struct LspExtra {
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

/// What a semantic token type looks like, as a highlight capture name.
fn capture_of(token_type: &str) -> Option<&'static str> {
    Some(match token_type {
        "function" => "function",
        "method" => "function.method",
        "macro" => "function.macro",
        "type" | "struct" | "class" | "enum" | "interface" | "typeParameter" | "typeAlias" | "builtinType" => "type",
        "enumMember" => "constructor",
        "parameter" => "variable.parameter",
        "property" => "property",
        "namespace" => "module",
        "lifetime" | "label" => "label",
        _ => return None,
    })
}

fn decode_tokens(rope: &ropey::Rope, data: &[u32], legend: &[String]) -> HashMap<usize, Vec<Highlight>> {
    let current = theme::current();
    let colors: Vec<Option<Color32>> = legend
        .iter()
        .map(|t| {
            let capture = capture_of(t)?;
            theme::capture_style(&current, capture).map(|(c, _)| theme::c32(c))
        })
        .collect();
    let mut lines: HashMap<usize, Vec<Highlight>> = HashMap::new();
    let (mut line, mut start) = (0u32, 0u32);
    for token in data.as_chunks::<5>().0 {
        let (delta_line, delta_start, length, kind) = (token[0], token[1], token[2], token[3]);
        if delta_line > 0 {
            line += delta_line;
            start = delta_start;
        } else {
            start += delta_start;
        }
        let Some(color) = colors.get(kind as usize).copied().flatten() else { continue };
        let (a, b) = convert::chars_of(
            rope,
            lt::Range::new(lt::Position::new(line, start), lt::Position::new(line, start + length)),
        );
        let line_start = rope.line_to_char(rope.char_to_line(a));
        lines.entry(line as usize).or_default().push(Highlight {
            start: a - line_start,
            end: b - line_start,
            foreground: Some(color),
            background: None,
            italic: false,
        });
    }
    lines
}

fn legend_of(client: &Client) -> Option<Vec<String>> {
    let legend = match client.capabilities()?.semantic_tokens_provider.as_ref()? {
        lt::SemanticTokensServerCapabilities::SemanticTokensOptions(o) => &o.legend,
        lt::SemanticTokensServerCapabilities::SemanticTokensRegistrationOptions(o) => &o.semantic_tokens_options.legend,
    };
    Some(legend.token_types.iter().map(|t| t.as_str().to_owned()).collect())
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
        let (path, _, kind) = self.docs.get(self.active).and_then(lsp_target)?;
        self.lsp.clients.iter().position(|c| c.kind == kind && path.starts_with(&c.root) && c.is_ready())
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
        if let Some((_, map)) = extra.inlays.get(&doc.uid).filter(|(n, _)| *n == lines) {
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
