//! Settings, keybindings and color theme: loading, applying and saving.

use crate::document::Document;
use crate::settings;
use crate::ui::keymap::Keymap;
use crate::ui::theme;
use coiman_core::language::lsp::registry;
use coiman_core::project::trust;
use std::path::{Path, PathBuf};

use crate::app::CodeApp;

impl CodeApp {
    /// Opens a settings/keybindings file, creating it with `initial` contents first.
    pub(crate) fn open_config_file(&mut self, path: Option<PathBuf>, initial: String) {
        let Some(path) = path else {
            self.status = Some("No config directory available".to_owned());
            return;
        };
        if !path.exists() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&path, initial) {
                self.status = Some(format!("Could not create {}: {e}", path.display()));
                return;
            }
        }
        self.open_file(&path);
    }

    pub(crate) fn is_config_file(&self, path: &Path) -> bool {
        let mut files: Vec<PathBuf> =
            [settings::user_settings_path(), settings::user_keybindings_path(), registry::user_file()]
                .into_iter()
                .flatten()
                .collect();
        if let Some(root) = &self.explorer.root {
            files.push(settings::workspace_settings_path(root));
        }
        files.iter().any(|f| f == path)
    }

    /// The folder in Restricted Mode for what's on screen: the open folder,
    /// else the active file's folder.
    pub(crate) fn restricted_folder(&self) -> Option<PathBuf> {
        // A server's read-only document is no one's folder to trust.
        let active = self.docs.get(self.active).filter(|d| !d.buffer.read_only).and_then(|d| d.path.as_deref());
        let folder = self.explorer.root.clone().or_else(|| active.and_then(Path::parent).map(Path::to_path_buf))?;
        (!trust::is_trusted(&folder)).then_some(folder)
    }

    /// Whether something may run in `dir`; says why not in the status bar.
    pub(crate) fn allowed_to_run(&mut self, dir: &Path) -> bool {
        let allowed = trust::is_trusted(dir);
        if !allowed {
            self.trust_prompt_hidden = false;
            self.status = Some("Restricted Mode: trust this folder to run things in it".to_owned());
        }
        allowed
    }

    /// Bar shown while the folder is in Restricted Mode.
    pub(crate) fn trust_bar(&mut self, ui: &mut egui::Ui) {
        if self.trust_prompt_hidden {
            return;
        }
        let Some(folder) = self.restricted_folder() else { return };
        let mut choice = None;
        egui::Frame::new().fill(theme::colors().hover_bg).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
                ui.label(format!(
                    "Restricted Mode: do you trust the authors of \"{name}\"? Trusting lets its settings, language servers, Git, builds, run configurations and debugging run, and they can run code on your computer. Until then you can read and edit its files."
                ))
                .on_hover_text(folder.display().to_string());
                if ui.button("Trust Folder").clicked() {
                    choice = Some(true);
                }
                if ui.button("Stay Restricted").clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => self.set_workspace_trust(ui.ctx(), &folder, true),
            Some(false) => self.trust_prompt_hidden = true,
            None => {}
        }
    }

    /// Workspaces: Trust or Restrict This Folder.
    pub(crate) fn toggle_workspace_trust(&mut self, ctx: &egui::Context) {
        let active = self.docs.get(self.active).and_then(|d| d.path.as_deref());
        let folder = self.explorer.root.clone().or_else(|| active.and_then(Path::parent).map(Path::to_path_buf));
        let Some(folder) = folder else {
            self.status = Some("Open a folder first".to_owned());
            return;
        };
        let trust_it = !trust::is_trusted(&folder);
        self.set_workspace_trust(ctx, &folder, trust_it);
    }

    fn set_workspace_trust(&mut self, ctx: &egui::Context, folder: &Path, trusted: bool) {
        let result = if trusted { trust::trust(folder) } else { trust::revoke(folder) };
        if let Err(e) = result {
            self.status = Some(format!("Could not save trust: {e}"));
            return;
        }
        self.trust_prompt_hidden = false;
        let name =
            folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned());
        self.status = Some(format!("{name} is {}", if trusted { "trusted" } else { "restricted" }));
        // Stops what was running; what is allowed starts again on the next frame.
        self.restart_lsp();
        self.settings_reload = true;
        // The lookup failed, or may no longer be allowed: do it again.
        self.git.discovered_for = None;
        self.git_refresh_soon(ctx, 0.0);
    }

    /// Uses a font for code. Loading it takes a moment, so it happens off to the side.
    fn apply_font_family(&mut self, ctx: &egui::Context, first: bool, family: &str) {
        self.font_load = None;
        if family.trim().is_empty() {
            self.editor_font = None;
            if !first {
                self.install_fonts(ctx);
            }
            return;
        }
        let (family, wake) = (family.trim().to_owned(), crate::app::repaint(ctx));
        let (tx, rx) = std::sync::mpsc::channel();
        self.font_load = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(coiman_core::appearance::fonts::load(&family));
            wake();
        });
    }

    pub(crate) fn pump_font_load(&mut self, ctx: &egui::Context) {
        let Some(result) = self.font_load.as_ref().and_then(|rx| rx.try_recv().ok()) else { return };
        self.font_load = None;
        match result {
            Ok((bytes, index)) => {
                let mut font = egui::FontData::from_owned(bytes);
                font.index = index;
                self.editor_font = Some(std::sync::Arc::new(font));
                self.install_fonts(ctx);
            }
            Err(e) => self.status = Some(e),
        }
    }

    pub(crate) fn install_fonts(&self, ctx: &egui::Context) {
        const EDITOR: &str = "editor font";
        let mut fonts = egui::FontDefinitions::default();
        if let Some(font) = &self.editor_font {
            fonts.font_data.insert(EDITOR.to_owned(), font.clone());
            // The built-in fonts stay behind it for characters it lacks.
            fonts.families.entry(egui::FontFamily::Monospace).or_default().insert(0, EDITOR.to_owned());
        }
        // Last, for what neither the code font nor the built-in ones have.
        for (name, font) in &self.fallback_fonts {
            fonts.font_data.insert(name.clone(), font.clone());
            for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
        }
        ctx.set_fonts(fonts);
    }

    /// Reloads settings, keybindings and the color theme, applying what changed.
    pub(crate) fn reload_settings(&mut self, ctx: &egui::Context) {
        let loaded = settings::load(self.explorer.root.as_deref());
        let (keymap, key_problems) = Keymap::load(settings::user_keybindings_path().as_deref());
        let mut problems = loaded.problems;
        problems.extend(key_problems);
        let (language_problem, languages_changed) = registry::reload();
        problems.extend(language_problem);
        let first = !self.settings_applied;
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        let new = self.settings.clone();
        crate::buffer::set_tab_width(new.editor.tab_size);
        // Servers read these when they start.
        if !first
            && (old.lsp.rust_analyzer_memory != new.lsp.rust_analyzer_memory
                || old.lsp.rust_analyzer_separate_target != new.lsp.rust_analyzer_separate_target)
        {
            self.restart_rust_analyzer();
        }
        if !first && old.cpp != new.cpp {
            self.restart_clangd();
        }
        coiman_core::launch::environment::set_chosen(&new.python.interpreter);
        // Servers come from these.
        if !first && languages_changed {
            self.restart_lsp();
        }
        // Indentation and save rules may have changed.
        let mut docs = std::mem::take(&mut self.docs);
        for doc in &mut docs {
            self.apply_file_rules(doc);
        }
        self.docs = docs;
        if first || new.editor.font_family != old.editor.font_family {
            self.apply_font_family(ctx, first, &new.editor.font_family);
        }
        if first || new.editor.font_size != old.editor.font_size {
            self.font_size = new.editor.font_size.clamp(6.0, 72.0);
        }
        if first || new.terminal.font_size != old.terminal.font_size {
            self.terminal_font_size = new.terminal.font_size.clamp(0.0, 72.0);
        }
        if first || new.editor.word_wrap != old.editor.word_wrap {
            self.word_wrap = new.editor.word_wrap;
        }
        if first || new.workbench.ui != old.workbench.ui {
            theme::set_islands(ctx, new.workbench.ui != "classic");
        }
        if first || new.workbench.color_theme != old.workbench.color_theme {
            // The built-in theme needs no scan of installed VS Code themes.
            let built_in = theme::built_in_names().iter().any(|n| new.workbench.color_theme.eq_ignore_ascii_case(n));
            if self.themes.is_empty() && !built_in {
                self.themes = theme::discover();
            }
            let (t, err) = theme::resolve(&new.workbench.color_theme, &self.themes);
            theme::set(ctx, t);
            problems.extend(err);
        }
        theme::apply_style(ctx, new.workbench.ui_font_size);
        theme::set_motion(ctx, new.workbench.reduced_motion);
        if first || new.workbench.zoom != old.workbench.zoom {
            self.set_zoom(ctx, new.workbench.zoom);
        }
        if first || new.editor.keyboard_mode != old.editor.keyboard_mode {
            use coiman_core::editor::vim::{Flavor, Vim};
            self.vim = match new.editor.keyboard_mode.as_str() {
                "vim" => Some(Vim::new(Flavor::Vim)),
                "helix" => Some(Vim::new(Flavor::Helix)),
                "default" => None,
                other => {
                    problems.push(format!("editor.keyboard_mode: unknown mode `{other}` (default, vim, helix)"));
                    None
                }
            };
            self.vim_doc = None;
        }
        self.keymap = keymap;
        self.settings_applied = true;
        if !problems.is_empty() {
            self.status = Some(format!("Settings: {}", problems.join("; ")));
        }
    }

    /// Sets `[table] key = value` (a TOML literal) in the user settings file,
    /// keeping its comments, and applies it.
    pub(crate) fn write_user_setting(
        &mut self,
        ctx: &egui::Context,
        table: &str,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        self.change_setting(ctx, false, table, key, Some(value))
    }

    /// Sets a value (a TOML literal), or removes it to fall back on the next
    /// layer, in the user's settings or the open folder's, and applies it.
    pub(crate) fn change_setting(
        &mut self,
        ctx: &egui::Context,
        workspace: bool,
        table: &str,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        let (path, empty) = match (&self.explorer.root, workspace) {
            (Some(root), true) => (settings::workspace_settings_path(root), String::new()),
            _ => {
                let path = settings::user_settings_path().ok_or("No config directory available")?;
                (path, settings::default_file_contents())
            }
        };
        let text = std::fs::read_to_string(&path).unwrap_or(empty);
        let updated = match value {
            Some(value) => settings::set_value_in_text(&text, table, key, value),
            None => settings::remove_value_in_text(&text, table, key),
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&path, updated).map_err(|e| format!("Could not save settings: {e}"))?;
        // An open, unmodified tab of the file would otherwise show stale text
        // (and saving it would undo the change).
        if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(path.as_path()) && !d.is_dirty())
            && let Ok(fresh) = Document::open(&path)
        {
            let uid = self.docs[i].uid;
            self.docs[i] = fresh;
            self.docs[i].uid = uid;
        }
        self.reload_settings(ctx);
        Ok(())
    }

    /// Saves the background image into the user settings file and applies it.
    pub(crate) fn set_background(&mut self, ctx: &egui::Context, source: &str) {
        let value = toml::Value::String(source.to_owned()).to_string();
        if let Err(e) = self.write_user_setting(ctx, "workbench", "background_image", &value) {
            self.status = Some(e);
        }
    }

    /// Saves the picked theme into the user settings file and applies it.
    pub(crate) fn set_color_theme(&mut self, ctx: &egui::Context, label: &str) {
        let value = toml::Value::String(label.to_owned()).to_string();
        match self.write_user_setting(ctx, "workbench", "color_theme", &value) {
            Ok(()) if self.settings.workbench.color_theme != label => {
                self.status = Some("Theme saved, but workspace settings override it".to_owned());
            }
            Ok(()) => {}
            Err(e) => self.status = Some(e),
        }
    }
}
