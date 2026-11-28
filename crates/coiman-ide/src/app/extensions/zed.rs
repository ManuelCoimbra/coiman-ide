//! Zed extensions support in Coiman IDE:
//! - Sidebar "Zed" tab: installed extensions, enable/disable/uninstall, browse/search `api.zed.dev`,
//!   category filters, and "Install Folder…" for local dev extensions.
//! - Dedicated editor detail tab with capabilities summary & README.
//! - Wasm extension runtime wiring: Language Servers (`[language_servers]`),
//!   Context/MCP Servers (`[context_servers]`), Agent Slash Commands (`[slash_commands]`),
//!   and Debug Adapters (`[debug_adapters]`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};

use coiman_core::extensions::opencode_mcp::{Config as McpConfig, Kind as McpKind, Server as McpServer};
use coiman_core::extensions::zed::languages::{self as zed_langs, LoadedLanguage};
use coiman_core::extensions::zed::registry::{self, InstalledExtension, RegistryExtension};
use coiman_core::extensions::zed::wasm_host::{
    self, ZedCommand, ZedContextServerConfig, ZedSlashCommandOutput, ZedWasmHost,
};
use coiman_core::language::lsp::convert;
use coiman_core::language::lsp::servers::{Launch, ServerKind};
use coiman_core::language::lsp::types as lt;
use coiman_core::language::lsp::{Client, Event};
use coiman_core::project::trust;
use eframe::egui::{self, RichText};
use serde_json::Value;

use crate::app::language::lsp::path_key;
use crate::app::views::view_tabs::ActiveView;
use crate::app::{CodeApp, repaint};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

const CATEGORIES: [(&str, Option<&str>); 8] = [
    ("All", None),
    ("Languages", Some("languages")),
    ("LSP", Some("language-servers")),
    ("Themes", Some("themes")),
    ("Icons", Some("icon-themes")),
    ("MCP", Some("context-servers")),
    ("Slash", Some("slash-commands")),
    ("Snippets", Some("snippets")),
];

#[derive(Clone, Debug)]
pub(crate) enum ZedDetailTarget {
    Registry(Box<RegistryExtension>),
    Installed(Box<InstalledExtension>),
}

impl ZedDetailTarget {
    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Registry(r) => &r.id,
            Self::Installed(i) => &i.id,
        }
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Registry(r) => &r.name,
            Self::Installed(i) => &i.name,
        }
    }

    pub(crate) fn version(&self) -> &str {
        match self {
            Self::Registry(r) => &r.version,
            Self::Installed(i) => &i.version,
        }
    }

    pub(crate) fn description(&self) -> &str {
        match self {
            Self::Registry(r) => r.description.as_deref().unwrap_or(""),
            Self::Installed(i) => &i.description,
        }
    }

    pub(crate) fn authors(&self) -> &[String] {
        match self {
            Self::Registry(r) => &r.authors,
            Self::Installed(i) => &i.authors,
        }
    }

    pub(crate) fn repository(&self) -> &str {
        match self {
            Self::Registry(r) => r.repository.as_deref().unwrap_or(""),
            Self::Installed(i) => &i.repository,
        }
    }
}

struct ZedLspRunning {
    key: String,
    ext_id: String,
    server_id: String,
    root: PathBuf,
    language_ids: HashMap<String, String>,
    workspace_config: Option<Value>,
    client: Client,
    open: HashMap<u64, (lt::Url, u64)>,
}

type LspSpawnResult = Result<(ZedCommand, Option<Value>, Option<Value>), String>;
type McpResolveResult = Result<(ZedCommand, Option<ZedContextServerConfig>), String>;

pub(crate) struct ZedUi {
    pub(crate) host: Arc<ZedWasmHost>,
    query: String,
    category: Option<&'static str>,
    searching: Option<Receiver<Result<Vec<RegistryExtension>, String>>>,
    results: Option<Result<Vec<RegistryExtension>, String>>,
    installing: Option<(String, Receiver<Result<InstalledExtension, String>>)>,
    done: Option<(String, Result<InstalledExtension, String>)>,
    pub(crate) installed: Vec<InstalledExtension>,
    installed_loaded: bool,
    pub(crate) detail_tab: Option<ZedDetailTarget>,
    readme: Option<(String, Option<String>)>,
    readme_rx: Option<(String, Receiver<Result<String, String>>)>,
    lsp_running: Vec<ZedLspRunning>,
    lsp_spawning: HashMap<(String, String, PathBuf), (InstalledExtension, Receiver<LspSpawnResult>)>,
    lsp_failed: HashSet<(String, String, PathBuf)>,
    lsp_diagnostics: HashMap<(String, PathBuf), (PathBuf, Vec<lt::Diagnostic>)>,
    mcp_pending: Option<(String, String, Receiver<McpResolveResult>)>,
    mcp_info: Option<String>,
    slash_pending: Option<(String, Receiver<Result<ZedSlashCommandOutput, String>>)>,
}

impl Default for ZedUi {
    fn default() -> Self {
        Self {
            host: Arc::new(ZedWasmHost::default()),
            query: String::new(),
            category: None,
            searching: None,
            results: None,
            installing: None,
            done: None,
            installed: Vec::new(),
            installed_loaded: false,
            detail_tab: None,
            readme: None,
            readme_rx: None,
            lsp_running: Vec::new(),
            lsp_spawning: HashMap::new(),
            lsp_failed: HashSet::new(),
            lsp_diagnostics: HashMap::new(),
            mcp_pending: None,
            mcp_info: None,
            slash_pending: None,
        }
    }
}

impl CodeApp {
    /// Refreshes installed Zed extensions and propagates languages, themes, icons, and slash commands.
    pub(crate) fn reload_zed_extensions(&mut self, ctx: &egui::Context) {
        coiman_core::extensions::zed::set_enabled_global(self.settings.plugins.zed_extensions);
        self.zed.installed = registry::list_installed();
        self.zed.installed_loaded = true;
        coiman_core::extensions::zed::sync_assets();
        self.themes = coiman_core::appearance::theme::discover();
        let name = self.theme_setting(ctx);
        let (t, _) = coiman_core::appearance::theme::resolve(&name, &self.themes);
        crate::ui::theme::set(ctx, t);
        self.sync_plugin_languages();
        self.refresh_open_document_languages();
        self.sync_zed_agents_slash_commands();
    }

    /// Re-evaluates syntax highlighting and editor rules for open documents after Zed changes.
    pub(crate) fn refresh_open_document_languages(&mut self) {
        let mut docs = std::mem::take(&mut self.docs);
        for doc in &mut docs {
            doc.refresh_language();
            self.apply_file_rules(doc);
        }
        self.docs = docs;
        let mut pending = std::mem::take(&mut self.pending_docs);
        for doc in &mut pending {
            doc.refresh_language();
            self.apply_file_rules(doc);
        }
        self.pending_docs = pending;
    }

    pub(crate) fn sync_zed_agents_slash_commands(&mut self) {
        if !self.settings.plugins.zed_extensions {
            self.agents.external_slash_commands.clear();
            return;
        }
        let mut cmds = Vec::new();
        for ext in self.zed.installed.iter().filter(|e| e.enabled && e.has_wasm) {
            for (name, entry) in &ext.manifest.slash_commands {
                cmds.push((ext.id.clone(), name.clone(), entry.description.clone(), entry.requires_argument));
            }
        }
        cmds.sort_by(|a, b| a.1.cmp(&b.1));
        self.agents.external_slash_commands = cmds;
    }

    /// Merges languages declared by native Coiman plugins and enabled Zed extensions.
    pub(crate) fn active_zed_languages(&self) -> Vec<LoadedLanguage> {
        if !self.settings.plugins.zed_extensions {
            return Vec::new();
        }
        let active: Vec<InstalledExtension> =
            self.zed.installed.iter().filter(|e| e.enabled).cloned().collect();
        zed_langs::collect_languages(&active)
    }

    pub(crate) fn showing_zed_extension_tab(&self) -> bool {
        self.zed.detail_tab.as_ref().is_some_and(|t| {
            matches!(&self.active_view, Some(ActiveView::ZedExtension(open)) if open == t.id())
        })
    }

    pub(crate) fn show_zed_extension_tab(&mut self, ctx: &egui::Context, target: ZedDetailTarget) {
        let id = target.id().to_owned();
        let repo = target.repository().to_owned();
        self.zed.detail_tab = Some(target);
        self.hide_view_tabs();
        self.active_view = Some(ActiveView::ZedExtension(id.clone()));
        if self.zed.readme.as_ref().is_none_or(|(rid, _)| rid != &id) {
            self.zed.readme = Some((id.clone(), None));
            self.zed.readme_rx = None;
            if !repo.is_empty() {
                let (tx, rx) = mpsc::channel();
                let wake = repaint(ctx);
                let fetch_id = id.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(registry::fetch_readme(&repo));
                    wake();
                });
                self.zed.readme_rx = Some((fetch_id, rx));
            }
        }
    }

    fn start_zed_search(&mut self, ctx: &egui::Context) {
        let filter = self.zed.query.trim().to_owned();
        let cat = self.zed.category.map(str::to_owned);
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let filter_opt = (!filter.is_empty()).then_some(filter.as_str());
            let _ = tx.send(registry::search(filter_opt, cat.as_deref()));
            wake();
        });
        self.zed.searching = Some(rx);
    }

    fn start_zed_install(&mut self, ctx: &egui::Context, ext: RegistryExtension) {
        let id = ext.id.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let _ = tx.send(registry::download_and_install(&ext));
            wake();
        });
        self.zed.installing = Some((id, rx));
        self.zed.done = None;
    }

    pub(crate) fn install_zed_from_folder(&mut self, ctx: &egui::Context) {
        let Some(folder) =
            self.file_dialog("zed_folder").set_title("Zed extension folder (extension.toml)").pick_folder()
        else {
            return;
        };
        self.dialog_chose("zed_folder", &folder);
        match registry::install_from_folder(&folder) {
            Ok(ext) => {
                self.zed.host.unload(&ext.id);
                self.status = Some(format!("Installed Zed extension {} {}", ext.name, ext.version));
                self.reload_zed_extensions(ctx);
            }
            Err(e) => self.status = Some(format!("Not a Zed extension folder: {e}")),
        }
    }

    /// Runs every frame: completes background Zed registry, Wasm slash command, MCP, and LSP tasks.
    pub(crate) fn pump_zed(&mut self, ctx: &egui::Context) {
        if !self.zed.installed_loaded && self.settings.plugins.zed_extensions {
            self.zed.installed = registry::list_installed();
            self.zed.installed_loaded = true;
            self.sync_zed_agents_slash_commands();
        }
        if !self.settings.plugins.zed_extensions {
            if !self.zed.lsp_running.is_empty() || !self.zed.lsp_spawning.is_empty() {
                self.zed.lsp_running.clear();
                self.zed.lsp_spawning.clear();
                self.zed.lsp_diagnostics.clear();
                self.zed.host.unload_all();
            }
            return;
        }
        if let Some(rx) = self.zed.searching.as_ref()
            && let Ok(res) = rx.try_recv()
        {
            self.zed.searching = None;
            self.zed.results = Some(res);
        }
        if let Some((id, res)) = self.zed.installing.as_ref().and_then(|(id, rx)| rx.try_recv().ok().map(|r| (id.clone(), r))) {
            self.zed.installing = None;
            if let Ok(ext) = &res {
                self.zed.host.unload(&ext.id);
                self.status = Some(format!("Installed Zed extension {} {}", ext.name, ext.version));
                self.reload_zed_extensions(ctx);
            } else if let Err(e) = &res {
                self.status = Some(format!("Zed install failed ({id}): {e}"));
            }
            self.zed.done = Some((id, res));
        }
        if let Some((id, rx)) = &self.zed.readme_rx
            && let Ok(res) = rx.try_recv()
        {
            let id = id.clone();
            self.zed.readme_rx = None;
            self.zed.readme = Some((id, res.ok()));
        }
        // Agents slash commands backed by Zed extensions.
        if let Some((ext_id, cmd_name, args)) = self.agents.pending_external_slash.take()
            && let Some(ext) = self.zed.installed.iter().find(|e| e.id == ext_id && e.enabled && e.has_wasm).cloned()
        {
            let entry = ext.manifest.slash_commands.get(&cmd_name).cloned().unwrap_or_default();
            let host = self.zed.host.clone();
            let root = self.explorer.root.clone();
            let (tx, rx) = mpsc::channel();
            let wake = repaint(ctx);
            let label = format!("/{cmd_name}");
            std::thread::spawn(move || {
                let _ = tx.send(host.run_slash_command(
                    &ext,
                    &cmd_name,
                    &entry.description,
                    entry.requires_argument,
                    &args,
                    root.as_deref(),
                ));
                wake();
            });
            self.zed.slash_pending = Some((label, rx));
        }
        if let Some((label, rx)) = &self.zed.slash_pending
            && let Ok(res) = rx.try_recv()
        {
            let label = label.clone();
            self.zed.slash_pending = None;
            match res {
                Ok(out) => {
                    let text = if out.text.trim().is_empty() { format!("{label}: (no output)") } else { out.text };
                    self.agents.push_agent_text(text);
                }
                Err(e) => self.agents.push_agent_text(format!("{label} error: {e}")),
            }
        }
        if let Some((server_id, scope_path, rx)) = &self.zed.mcp_pending
            && let Ok(res) = rx.try_recv()
        {
            let (server_id, scope_path) = (server_id.clone(), scope_path.clone());
            self.zed.mcp_pending = None;
            match res {
                Ok((cmd, cfg)) => {
                    let mut full_cmd = vec![cmd.command];
                    full_cmd.extend(cmd.args);
                    let mut mcp_cfg = McpConfig::load(&PathBuf::from(&scope_path)).unwrap_or_default();
                    mcp_cfg.servers.retain(|s| s.name != server_id);
                    mcp_cfg.servers.push(McpServer {
                        name: server_id.clone(),
                        enabled: true,
                        kind: McpKind::Local { command: full_cmd, env: cmd.env },
                    });
                    if let Err(e) = mcp_cfg.save(&PathBuf::from(&scope_path)) {
                        self.zed.mcp_info = Some(format!("Could not save MCP config: {e}"));
                    } else {
                        self.zed.mcp_info = Some(format!("Added Zed MCP server `{server_id}` to opencode."));
                        if let Some(c) = cfg
                            && !c.installation_instructions.trim().is_empty()
                        {
                            self.zed.mcp_info = Some(format!("Added `{server_id}`.\n\n{}", c.installation_instructions));
                        }
                        self.mcp_reload_if_open();
                    }
                }
                Err(e) => {
                    self.zed.mcp_info = Some(format!("Zed MCP `{server_id}` failed: {e}"));
                }
            }
        }
        self.pump_zed_lsp(ctx);
    }

    pub(crate) fn zed_sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        let avail = ui.available_width();
        if avail > 0.0 {
            ui.set_max_width(avail);
        }
        if !self.zed.installed_loaded {
            self.zed.installed = registry::list_installed();
            self.zed.installed_loaded = true;
            self.sync_zed_agents_slash_commands();
        }
        Self::ext_item_row(
            ui,
            |ui| {
                if icons::button(ui, Icon::Refresh, "Refresh Zed extension registry").clicked() {
                    self.start_zed_search(&ctx);
                }
                if icons::button(ui, Icon::FolderOpen, "Install Zed Extension Folder…").clicked() {
                    self.install_zed_from_folder(&ctx);
                }
            },
            |ui| {
                ui.add_space(12.0);
                ui.add(egui::Label::new(RichText::new("ZED EXTENSIONS").small().strong()).truncate());
            },
        );

        let mut btn_search = false;
        let mut enter_search = false;
        Self::ext_item_row(
            ui,
            |ui| {
                if icons::button(ui, Icon::Search, "Search api.zed.dev").clicked() {
                    btn_search = true;
                }
            },
            |ui| {
                ui.add_space(12.0);
                let w = ui.available_width().max(24.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.zed.query)
                        .hint_text("Search Zed registry")
                        .desired_width(w),
                );
                if (edit.has_focus() || edit.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    enter_search = true;
                }
            },
        );
        let mut search_now = btn_search || enter_search;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width().max(1.0));
                ui.horizontal_wrapped(|ui| {
                    for (label, cat) in CATEGORIES {
                        let selected = self.zed.category == cat;
                        if ui.selectable_label(selected, RichText::new(label).small()).clicked() {
                            self.zed.category = cat;
                            search_now = true;
                        }
                    }
                });
            });
        });
        if search_now || (self.zed.results.is_none() && self.zed.searching.is_none()) {
            self.start_zed_search(&ctx);
        }
        if let Some((id, _)) = &self.zed.installing {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.spinner();
                ui.add(egui::Label::new(RichText::new(format!("Installing {id}…")).small().color(pal.text_dim)).wrap());
            });
        }
        ui.separator();

        let installed = self.zed.installed.clone();
        let mut toggle_ext: Option<(String, bool)> = None;
        let mut uninstall_id: Option<String> = None;
        let mut open_detail: Option<ZedDetailTarget> = None;
        let mut install_reg: Option<RegistryExtension> = None;

        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            let scroll_w = ui.available_width();
            if scroll_w > 0.0 {
                ui.set_max_width(scroll_w);
            }
            if !installed.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    ui.label(RichText::new("INSTALLED").small().strong());
                });
                for ext in &installed {
                    Self::ext_item_row(
                        ui,
                        |ui| {
                            if icons::button(ui, Icon::Trash, "Uninstall").clicked() {
                                uninstall_id = Some(ext.id.clone());
                            }
                        },
                        |ui| {
                            ui.add_space(12.0);
                            let mut enabled = ext.enabled;
                            if ui.checkbox(&mut enabled, "").on_hover_text("Enable extension").changed() {
                                toggle_ext = Some((ext.id.clone(), enabled));
                            }
                            if ui
                                .selectable_label(
                                    self.zed.detail_tab.as_ref().is_some_and(|t| t.id() == ext.id),
                                    RichText::new(&ext.name).strong(),
                                )
                                .clicked()
                            {
                                open_detail = Some(ZedDetailTarget::Installed(Box::new(ext.clone())));
                            }
                            ui.label(RichText::new(&ext.version).small().color(pal.text_dim));
                        },
                    );
                    if !ext.description.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(36.0);
                            ui.vertical(|ui| {
                                ui.set_max_width(ui.available_width().max(1.0));
                                ui.add(
                                    egui::Label::new(RichText::new(&ext.description).small().color(pal.text_dim))
                                        .wrap(),
                                );
                            });
                        });
                    }
                    let provides = ext.provides().join(", ");
                    if !provides.is_empty() {
                        ui.horizontal(|ui| {
                            ui.add_space(36.0);
                            ui.vertical(|ui| {
                                ui.set_max_width(ui.available_width().max(1.0));
                                ui.add(egui::Label::new(RichText::new(provides).small().color(pal.accent)).wrap());
                            });
                        });
                    }
                }
                ui.add_space(6.0);
                ui.separator();
            }

            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.label(RichText::new("ZED REGISTRY").small().strong());
                if self.zed.searching.is_some() {
                    ui.spinner();
                }
            });
            match &self.zed.results {
                None => {}
                Some(Err(e)) => {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(ui.available_width().max(1.0));
                            ui.add(egui::Label::new(RichText::new(e).small().color(pal.modified)).wrap());
                        });
                    });
                }
                Some(Ok(list)) => {
                    for entry in list.iter().take(60) {
                        let local = installed.iter().find(|i| i.id == entry.id);
                        ui.add_space(4.0);
                        Self::ext_item_row(
                            ui,
                            |ui| match local {
                                Some(i) if i.version == entry.version => {
                                    ui.label(RichText::new("installed").small().color(pal.text_dim));
                                }
                                Some(_) => {
                                    if ui.small_button("Update").clicked() {
                                        install_reg = Some(entry.clone());
                                    }
                                }
                                None => {
                                    if ui.small_button("Install").clicked() {
                                        install_reg = Some(entry.clone());
                                    }
                                }
                            },
                            |ui| {
                                ui.add_space(12.0);
                                let selected = self.zed.detail_tab.as_ref().is_some_and(|t| t.id() == entry.id);
                                if ui.selectable_label(selected, RichText::new(&entry.name).strong()).clicked() {
                                    open_detail = Some(ZedDetailTarget::Registry(Box::new(entry.clone())));
                                }
                                ui.label(RichText::new(&entry.version).small().color(pal.text_dim));
                            },
                        );
                        if let Some(desc) = &entry.description
                            && !desc.is_empty()
                        {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.set_max_width(ui.available_width().max(1.0));
                                    ui.add(egui::Label::new(RichText::new(desc).small().color(pal.text_dim)).wrap());
                                });
                            });
                        }
                        if !entry.provides.is_empty() || entry.download_count > 0 {
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.set_max_width(ui.available_width().max(1.0));
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "{} · {} downloads",
                                                entry.provides.join(", "),
                                                entry.download_count
                                            ))
                                            .small()
                                            .color(pal.text_dim),
                                        )
                                        .wrap(),
                                    );
                                });
                            });
                        }
                    }
                }
            }
        });

        if let Some((id, enabled)) = toggle_ext {
            if let Err(e) = registry::set_enabled(&id, enabled) {
                self.status = Some(e);
            } else {
                if !enabled {
                    self.zed.host.unload(&id);
                    self.zed.lsp_running.retain(|r| r.ext_id != id);
                    self.zed.lsp_diagnostics.retain(|(k, _), _| !k.starts_with(&format!("{id}:")));
                }
                self.reload_zed_extensions(&ctx);
            }
        }
        if let Some(id) = uninstall_id {
            self.zed.host.unload(&id);
            self.zed.lsp_running.retain(|r| r.ext_id != id);
            self.zed.lsp_diagnostics.retain(|(k, _), _| !k.starts_with(&format!("{id}:")));
            if let Err(e) = registry::uninstall(&id) {
                self.status = Some(e);
            } else {
                self.status = Some(format!("Uninstalled Zed extension {id}"));
                self.reload_zed_extensions(&ctx);
            }
        }
        if let Some(target) = open_detail {
            self.show_zed_extension_tab(&ctx, target);
        }
        if let Some(entry) = install_reg {
            self.start_zed_install(&ctx, entry);
        }
    }

    pub(crate) fn zed_detail_view(&mut self, ui: &mut egui::Ui) {
        let Some(target) = self.zed.detail_tab.clone() else { return };
        let ctx = ui.ctx().clone();
        let pal = theme::colors();
        let id = target.id().to_owned();
        let installed = self.zed.installed.iter().find(|e| e.id == id).cloned();

        let mut install_reg = None;
        let mut toggle = None;
        let mut do_uninstall = false;
        let mut add_mcp: Option<(String, InstalledExtension)> = None;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(24, 18)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(target.name()).size(22.0).strong());
                    ui.label(RichText::new(format!("v{}", target.version())).color(pal.text_dim));
                });
                ui.label(RichText::new(format!("Zed Extension ID: {id}")).small().color(pal.text_dim));
                if !target.authors().is_empty() {
                    ui.label(RichText::new(format!("By {}", target.authors().join(", "))).small().color(pal.text_dim));
                }
                if !target.description().is_empty() {
                    ui.add_space(6.0);
                    ui.label(target.description());
                }
                if !target.repository().is_empty() {
                    ui.hyperlink_to(target.repository(), target.repository());
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    match (&installed, &target) {
                        (Some(inst), ZedDetailTarget::Registry(reg))
                            if inst.version != reg.version && ui.button(format!("Update to {}", reg.version)).clicked() =>
                        {
                            install_reg = Some((**reg).clone());
                        }
                        (None, ZedDetailTarget::Registry(reg)) if ui.button("Install").clicked() => {
                            install_reg = Some((**reg).clone());
                        }
                        _ => {}
                    }
                    if let Some(inst) = &installed {
                        let label = if inst.enabled { "Disable" } else { "Enable" };
                        if ui.button(label).clicked() {
                            toggle = Some(!inst.enabled);
                        }
                        if ui.button("Uninstall").clicked() {
                            do_uninstall = true;
                        }
                    }
                });

                if let Some(inst) = &installed {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.label(RichText::new("Provides & Integrations").strong());
                    if let Some((a, b, c)) = inst.wasm_api_version {
                        ui.label(RichText::new(format!("Wasm Extension API: v{a}.{b}.{c}")).small().color(pal.text_dim));
                    }
                    if !inst.manifest.themes.is_empty() {
                        ui.label(format!("Color Themes: {} file(s) (see Preferences: Color Theme)", inst.manifest.themes.len()));
                    }
                    if !inst.manifest.icon_themes.is_empty() {
                        ui.label(format!("Icon Themes: {} pack(s) (see Preferences: File Icon Theme)", inst.manifest.icon_themes.len()));
                    }
                    if !inst.manifest.languages.is_empty() {
                        ui.label(format!("Languages: {}", inst.manifest.languages.iter().filter_map(|p| p.file_name()?.to_str()).collect::<Vec<_>>().join(", ")));
                    }
                    for (ls_id, ls) in &inst.manifest.language_servers {
                        let status = wasm_host::language_server_status(ls_id)
                            .map(|s| format!(" — {s}"))
                            .unwrap_or_default();
                        ui.label(format!(
                            "Language Server: {ls_id} (languages: {}){status}",
                            ls.all_languages().join(", ")
                        ));
                    }
                    for cs_id in inst.manifest.context_servers.keys() {
                        ui.horizontal(|ui| {
                            ui.label(format!("Context / MCP Server: {cs_id}"));
                            if ui.small_button("Add to OpenCode MCP").clicked() {
                                add_mcp = Some((cs_id.clone(), inst.clone()));
                            }
                        });
                    }
                    for (sc_name, sc) in &inst.manifest.slash_commands {
                        ui.label(format!("Agent Slash Command: /{sc_name} — {}", sc.description));
                    }
                    for da_name in inst.manifest.debug_adapters.keys() {
                        ui.label(format!("Debug Adapter: {da_name}"));
                    }
                }

                ui.add_space(12.0);
                ui.separator();
                if self.zed.readme_rx.as_ref().is_some_and(|(rid, _)| rid == &id) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Loading README…").small().color(pal.text_dim));
                    });
                } else if let Some((rid, Some(md))) = &self.zed.readme
                    && rid == &id
                {
                    ui.label(RichText::new("README").strong());
                    ui.add_space(4.0);
                    ui.add(egui::Label::new(md).wrap());
                }
            });
        });

        if let Some(reg) = install_reg {
            self.start_zed_install(&ctx, reg);
        }
        if let Some(on) = toggle {
            let _ = registry::set_enabled(&id, on);
            if !on {
                self.zed.host.unload(&id);
            }
            self.reload_zed_extensions(&ctx);
        }
        if do_uninstall {
            self.zed.host.unload(&id);
            let _ = registry::uninstall(&id);
            self.zed.detail_tab = None;
            if matches!(&self.active_view, Some(ActiveView::ZedExtension(open)) if open == &id) {
                self.active_view = None;
            }
            self.reload_zed_extensions(&ctx);
        }
        if let Some((cs_id, ext)) = add_mcp {
            self.trigger_zed_mcp_install(&ctx, ext, cs_id);
        }
    }

    pub(crate) fn trigger_zed_mcp_install(
        &mut self,
        ctx: &egui::Context,
        ext: InstalledExtension,
        server_id: String,
    ) {
        let Some(target) = self
            .explorer
            .root
            .as_deref()
            .map(coiman_core::extensions::opencode_mcp::project_file)
            .or_else(coiman_core::extensions::opencode_mcp::user_file)
        else {
            self.status = Some("No opencode.json config path available".to_owned());
            return;
        };
        let host = self.zed.host.clone();
        let has_wt = self.explorer.root.is_some();
        let sid = server_id.clone();
        let (tx, rx) = mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let cmd = host.context_server_command(&ext, &sid, has_wt);
            let cfg = host.context_server_configuration(&ext, &sid, has_wt).ok().flatten();
            let _ = tx.send(cmd.map(|c| (c, cfg)));
            wake();
        });
        self.zed.mcp_pending = Some((server_id, target.to_string_lossy().into_owned(), rx));
    }

    /// Shows active Zed `[context_servers.*]` extensions inside the MCP Servers window.
    pub(crate) fn zed_mcp_section(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, configured: &[McpServer]) {
        if !self.settings.plugins.zed_extensions {
            return;
        }
        let servers: Vec<(String, InstalledExtension)> = self
            .zed
            .installed
            .iter()
            .filter(|e| e.enabled && e.has_wasm && !e.manifest.context_servers.is_empty())
            .flat_map(|e| e.manifest.context_servers.keys().map(|k| (k.clone(), e.clone())))
            .collect();
        if servers.is_empty() && self.zed.mcp_info.is_none() && self.zed.mcp_pending.is_none() {
            return;
        }
        let pal = theme::colors();
        ui.add_space(6.0);
        ui.separator();
        ui.label(RichText::new("Zed MCP Extensions").strong());
        if let Some((sid, ..)) = &self.zed.mcp_pending {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("Configuring {sid}…")).small().color(pal.text_dim));
            });
        }
        if let Some(info) = &self.zed.mcp_info {
            ui.add(egui::Label::new(RichText::new(info).small().color(pal.text_dim)).wrap());
        }
        let mut add = None;
        for (sid, ext) in servers {
            let already = configured.iter().any(|s| s.name == sid);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&sid).strong());
                ui.label(RichText::new(format!("from {}", ext.name)).small().color(pal.text_dim));
                if already {
                    ui.label(RichText::new("configured").small().color(pal.text_dim));
                } else if ui.small_button("Add to OpenCode").clicked() {
                    add = Some((sid, ext));
                }
            });
        }
        if let Some((sid, ext)) = add {
            self.trigger_zed_mcp_install(ctx, ext, sid);
        }
    }

    // ----- Zed LSP bridge -------------------------------------------------------------------------

    fn pump_zed_lsp(&mut self, ctx: &egui::Context) {
        if !self.settings.lsp.enabled {
            if !self.zed.lsp_running.is_empty() {
                self.zed.lsp_running.clear();
                self.zed.lsp_diagnostics.clear();
            }
            return;
        }

        // Complete background `language_server_command` spawns.
        let mut finished = Vec::new();
        for (key, (_, rx)) in &self.zed.lsp_spawning {
            if let Ok(res) = rx.try_recv() {
                finished.push((key.clone(), res));
            }
        }
        for ((ext_id, server_id, root), res) in finished {
            self.zed.lsp_spawning.remove(&(ext_id.clone(), server_id.clone(), root.clone()));
            let Some(ext) = self.zed.installed.iter().find(|e| e.id == ext_id && e.enabled) else {
                continue;
            };
            let language_ids = ext
                .manifest
                .language_servers
                .get(&server_id)
                .map(|m| m.language_ids.clone())
                .unwrap_or_default();
            match res {
                Ok((cmd, init_opts, ws_cfg)) => {
                    let launch = Launch {
                        program: PathBuf::from(&cmd.command),
                        args: cmd.args,
                        tcp: None,
                        env: cmd.env,
                    };
                    match Client::start(
                        ServerKind::Custom(u16::MAX),
                        &launch,
                        &root,
                        init_opts.unwrap_or(Value::Null),
                        repaint(ctx),
                    ) {
                        Ok(client) => {
                            let full_key = format!("{ext_id}:{server_id}");
                            self.zed.lsp_running.push(ZedLspRunning {
                                key: full_key,
                                ext_id,
                                server_id,
                                root,
                                language_ids: language_ids.into_iter().collect(),
                                workspace_config: ws_cfg,
                                client,
                                open: HashMap::new(),
                            });
                        }
                        Err(e) => {
                            self.zed.lsp_failed.insert((ext_id, server_id, root));
                            coiman_core::applog::info(&format!("zed lsp start failed: {e}"));
                        }
                    }
                }
                Err(e) => {
                    coiman_core::applog::info(&format!("zed lsp command `{server_id}`: {e}"));
                    self.zed.lsp_failed.insert((ext_id, server_id, root));
                }
            }
        }

        // Spawn needed servers & sync open documents matching Zed `[language_servers.*]`.
        let active_exts: Vec<InstalledExtension> =
            self.zed.installed.iter().filter(|e| e.enabled && e.has_wasm).cloned().collect();
        let top = self.explorer.root.clone();

        for i in 0..self.docs.len() {
            let doc = &self.docs[i];
            let Some(path) = doc.path.clone().filter(|_| !doc.large) else { continue };
            let lang_name = doc.language_name();

            for ext in &active_exts {
                for (server_id, entry) in &ext.manifest.language_servers {
                    if !entry.all_languages().iter().any(|l| l.eq_ignore_ascii_case(lang_name))
                        || entry.is_opt_in_for(lang_name)
                    {
                        continue;
                    }
                    let root = top
                        .as_deref()
                        .filter(|r| path.starts_with(r))
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| path.parent().unwrap_or(&path).to_path_buf());
                    if !trust::is_trusted(&root) {
                        continue;
                    }
                    let spawn_key = (ext.id.clone(), server_id.clone(), root.clone());
                    if let Some(running) = self
                        .zed
                        .lsp_running
                        .iter_mut()
                        .find(|r| r.ext_id == ext.id && r.server_id == *server_id && r.root == root)
                    {
                        if !running.client.is_ready() {
                            continue;
                        }
                        let doc_lang = running
                            .language_ids
                            .get(lang_name)
                            .cloned()
                            .unwrap_or_else(|| lang_name.to_lowercase());
                        let version = doc.buffer.version();
                        match running.open.get(&doc.uid) {
                            None => {
                                if let Some(uri) = convert::uri_from_path(&path) {
                                    running.client.did_open(uri.clone(), leak_str(&doc_lang), doc.text());
                                    running.open.insert(doc.uid, (uri, version));
                                }
                            }
                            Some((uri, sent)) if *sent != version => {
                                let uri = uri.clone();
                                running.client.did_change(&uri, doc.text());
                                running.open.insert(doc.uid, (uri, version));
                            }
                            _ => {}
                        }
                    } else if !self.zed.lsp_spawning.contains_key(&spawn_key)
                        && !self.zed.lsp_failed.contains(&spawn_key)
                    {
                        let ext_c = ext.clone();
                        let sid = server_id.clone();
                        let root_c = root.clone();
                        let host = self.zed.host.clone();
                        let (tx, rx) = mpsc::channel();
                        let wake = repaint(ctx);
                        std::thread::spawn(move || {
                            let cmd = host.language_server_command(&ext_c, &sid, &root_c);
                            let res = cmd.map(|c| {
                                let init = host
                                    .language_server_initialization_options(&ext_c, &sid, &root_c)
                                    .ok()
                                    .flatten();
                                let ws = host
                                    .language_server_workspace_configuration(&ext_c, &sid, &root_c)
                                    .ok()
                                    .flatten();
                                (c, init, ws)
                            });
                            let _ = tx.send(res);
                            wake();
                        });
                        self.zed.lsp_spawning.insert(spawn_key, (ext.clone(), rx));
                    }
                }
            }
        }

        // Poll events from running Zed LSP servers.
        let uids: HashSet<u64> = self.docs.iter().map(|d| d.uid).collect();
        let mut batch = Vec::new();
        for r in &mut self.zed.lsp_running {
            let closed: Vec<u64> = r.open.keys().filter(|u| !uids.contains(u)).copied().collect();
            for uid in closed {
                if let Some((uri, _)) = r.open.remove(&uid) {
                    r.client.did_close(&uri);
                }
            }
            for ev in r.client.poll() {
                batch.push((r.key.clone(), r.server_id.clone(), r.workspace_config.clone(), ev));
            }
        }
        self.zed.lsp_running.retain(|r| !r.client.has_exited());

        for (key, server_id, ws_cfg, ev) in batch {
            match ev {
                Event::Notification { method, params } if method == "textDocument/publishDiagnostics" => {
                    if let Ok(p) = serde_json::from_value::<lt::PublishDiagnosticsParams>(params)
                        && let Some(path) = convert::path_from_uri(&p.uri)
                    {
                        let map_key = (key, path_key(&path));
                        if p.diagnostics.is_empty() {
                            self.zed.lsp_diagnostics.remove(&map_key);
                        } else {
                            let mut diags = p.diagnostics;
                            for d in &mut diags {
                                d.source.get_or_insert_with(|| server_id.clone());
                            }
                            self.zed.lsp_diagnostics.insert(map_key, (path, diags));
                        }
                    }
                }
                Event::Request { id, method, params } => {
                    let answer = match method.as_str() {
                        "workspace/configuration" => {
                            let items = params["items"].as_array().map(|a| a.len()).unwrap_or(1);
                            let val = ws_cfg.unwrap_or(Value::Null);
                            Ok(Value::Array(vec![val; items]))
                        }
                        "window/workDoneProgress/create"
                        | "client/registerCapability"
                        | "client/unregisterCapability" => Ok(Value::Null),
                        _ => Ok(Value::Null),
                    };
                    if let Some(r) = self.zed.lsp_running.iter_mut().find(|r| r.key == key) {
                        r.client.respond(id, answer);
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn zed_diagnostics_for(&self, k: &Path) -> Vec<lt::Diagnostic> {
        if !self.settings.plugins.zed_extensions {
            return Vec::new();
        }
        self.zed
            .lsp_diagnostics
            .iter()
            .filter(|((_, p), _)| p == k)
            .flat_map(|(_, (_, d))| d.iter().cloned())
            .collect()
    }

    pub(crate) fn zed_diagnostic_paths(&self) -> Vec<(PathBuf, PathBuf)> {
        if !self.settings.plugins.zed_extensions {
            return Vec::new();
        }
        self.zed
            .lsp_diagnostics
            .iter()
            .map(|((_, k), (p, _))| (k.clone(), p.clone()))
            .collect()
    }
}

fn leak_str(s: &str) -> &'static str {
    static INTERN: OnceLock<std::sync::Mutex<HashSet<&'static str>>> = OnceLock::new();
    let set = INTERN.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    let mut guard = set.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&existing) = guard.get(s) {
        return existing;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    guard.insert(leaked);
    leaked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zed_enabled_by_default_and_agents_starts_when_enabled() {
        let defaults = coiman_core::settings::Settings::default();
        assert!(defaults.plugins.zed_extensions, "Zed extensions must be enabled by default");
        assert!(defaults.workbench.agents_enabled, "Agents setting should default to on");

        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        assert!(!app.agents.is_open(), "Agents window should not force-open on launch");
        assert!(!app.agents_pinned, "Agents window should not be pinned open by default");
        app.settings.workbench.agents_enabled = true;
        app.sync_agents_extension(&ctx);

        // Rendering the Extensions view defaults cleanly with Zed enabled.
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(340.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.extensions_view(ui));
        output.textures_delta.clear();
        app.pump_plugins(&ctx);
        assert!(
            app.agents_plugin_enabled(),
            "coiman.agents should auto-install and load when agents_enabled is true"
        );

        app.settings.workbench.agents_enabled = false;
        app.sync_agents_extension(&ctx);
        assert!(
            !app.agents_plugin_enabled(),
            "coiman.agents should unload when agents_enabled turns off"
        );
    }

    #[test]
    fn zed_detail_tab_opens_as_view_tab() {
        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        let reg = RegistryExtension {
            id: "demo-zed".to_owned(),
            name: "Demo Zed".to_owned(),
            version: "0.1.0".to_owned(),
            description: Some("Demo language and theme".to_owned()),
            authors: vec!["Coiman".to_owned()],
            repository: None,
            schema_version: 1,
            wasm_api_version: Some("0.7.0".to_owned()),
            provides: vec!["languages".to_owned(), "themes".to_owned()],
            published_at: None,
            download_count: 42,
        };
        app.show_zed_extension_tab(&ctx, ZedDetailTarget::Registry(Box::new(reg)));
        assert!(app.showing_zed_extension_tab());
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| app.zed_detail_view(ui));
        output.textures_delta.clear();
        assert!(
            CodeApp::view_tabs(&app)
                .iter()
                .any(|(tab, ..)| matches!(tab, crate::app::views::view_tabs::ViewTab::ZedExtension(_)))
        );
    }

    #[test]
    fn agents_slash_dispatches_zed_extension_commands() {
        let ctx = egui::Context::default();
        let mut app = CodeApp::new(None, &ctx);
        app.agents.external_slash_commands.push((
            "demo".to_owned(),
            "docs".to_owned(),
            "Search docs".to_owned(),
            true,
        ));
        app.agents.slash("/docs serde".trim_start_matches('/'));
        assert_eq!(
            app.agents.pending_external_slash,
            Some(("demo".to_owned(), "docs".to_owned(), vec!["serde".to_owned()]))
        );
    }
}

