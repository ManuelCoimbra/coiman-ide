//! Settings, keybindings and color theme: loading, applying and saving.

use crate::document::Document;
use crate::keymap::Keymap;
use crate::settings;
use crate::theme;
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

    /// Reloads settings, keybindings and the color theme, applying what changed.
    pub(super) fn reload_settings(&mut self, ctx: &egui::Context) {
        let loaded = settings::load(self.explorer.root.as_deref());
        let (keymap, key_problems) = Keymap::load(settings::user_keybindings_path().as_deref());
        let mut problems = loaded.problems;
        problems.extend(key_problems);
        let first = !self.settings_applied;
        let old = std::mem::replace(&mut self.settings, loaded.settings);
        let new = self.settings.clone();
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
