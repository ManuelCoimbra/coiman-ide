//! Settings, keybindings and color theme: loading, applying and saving.

use crate::document::Document;
use crate::keymap::Keymap;
use crate::settings;
use crate::theme;
use rust_code_core::trust;
use std::path::{Path, PathBuf};

use super::CodeApp;

impl CodeApp {
    /// Opens a settings/keybindings file, creating it with `initial` contents first.
    pub(super) fn open_config_file(&mut self, path: Option<PathBuf>, initial: String) {
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

    pub(super) fn is_config_file(&self, path: &Path) -> bool {
        let mut files: Vec<PathBuf> =
            [settings::user_settings_path(), settings::user_keybindings_path()].into_iter().flatten().collect();
        if let Some(root) = &self.explorer.root {
            files.push(settings::workspace_settings_path(root));
        }
        files.iter().any(|f| f == path)
    }

    /// The folder in Restricted Mode for what's on screen: the open folder,
    /// else the active file's folder.
    pub(super) fn restricted_folder(&self) -> Option<PathBuf> {
        let active = self.docs.get(self.active).and_then(|d| d.path.as_deref());
        let folder = self.explorer.root.clone().or_else(|| active.and_then(Path::parent).map(Path::to_path_buf))?;
        (!trust::is_trusted(&folder)).then_some(folder)
    }

    /// Whether something may run in `dir`; says why not in the status bar.
    pub(super) fn allowed_to_run(&mut self, dir: &Path) -> bool {
        let allowed = trust::is_trusted(dir);
        if !allowed {
            self.trust_prompt_hidden = false;
            self.status = Some("Restricted Mode: trust this folder to run things in it".to_owned());
        }
        allowed
    }

    /// Bar shown while the folder is in Restricted Mode.
    pub(super) fn trust_bar(&mut self, ui: &mut egui::Ui) {
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
                    "Restricted Mode: do you trust the authors of \"{name}\"? Trusting lets its settings, language servers, Git, Cargo builds, run configurations and debugging run, and they can run code on your computer. Until then you can read and edit its files."
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
    pub(super) fn toggle_workspace_trust(&mut self, ctx: &egui::Context) {
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
        self.git_refresh_soon(ctx, 0.0);
    }

    /// Reloads settings, keybindings and the color theme, applying what changed.
    pub(super) fn reload_settings(&mut self, ctx: &egui::Context) {
        let loaded = settings::load(self.explorer.root.as_deref());
        let (keymap, key_problems) = Keymap::load(settings::user_keybindings_path().as_deref());
        let mut problems = loaded.problems;
        problems.extend(key_problems);
        let first = !self.settings_applied;
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        let new = self.settings.clone();
        // Indentation and save rules may have changed.
        let mut docs = std::mem::take(&mut self.docs);
        for doc in &mut docs {
            self.apply_file_rules(doc);
        }
        self.docs = docs;
        if first || new.editor.font_size != old.editor.font_size {
            self.font_size = new.editor.font_size.clamp(6.0, 72.0);
        }
        if first || new.editor.word_wrap != old.editor.word_wrap {
            self.word_wrap = new.editor.word_wrap;
        }
        if first || new.workbench.color_theme != old.workbench.color_theme {
            // The built-in theme needs no scan of installed VS Code themes.
            if self.themes.is_empty() && !new.workbench.color_theme.eq_ignore_ascii_case("Dark Modern") {
                self.themes = theme::discover();
            }
            let (t, err) = theme::resolve(&new.workbench.color_theme, &self.themes);
            theme::set(ctx, t);
            problems.extend(err);
        }
        theme::apply_style(ctx, new.workbench.ui_font_size);
        if first || new.editor.keyboard_mode != old.editor.keyboard_mode {
            use rust_code_core::vim::{Flavor, Vim};
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
    pub(super) fn write_user_setting(
        &mut self,
        ctx: &egui::Context,
        table: &str,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        let path = settings::user_settings_path().ok_or("No config directory available")?;
        let text = std::fs::read_to_string(&path).unwrap_or_else(|_| settings::default_file_contents());
        let updated = settings::set_value_in_text(&text, table, key, value);
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

    /// Saves the picked theme into the user settings file and applies it.
    pub(super) fn set_color_theme(&mut self, ctx: &egui::Context, label: &str) {
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
