//! Importing themes, snippets and languages from VS Code extensions.

use std::path::Path;

use rust_code_core::commands::Command;
use rust_code_core::vscode_import::{self, Contents};

use super::CodeApp;
use crate::palette::PickerKind;

const CHOOSE_VSIX: &str = "Choose a .vsix file...";
const CHOOSE_FOLDER: &str = "Choose an extension folder...";

impl CodeApp {
    /// Offers VS Code's installed extensions that hold data, and a file or folder.
    pub(super) fn import_from_vscode(&mut self) {
        let found = vscode_import::installed();
        let mut labels = vec![CHOOSE_VSIX.to_owned(), CHOOSE_FOLDER.to_owned()];
        labels.extend(found.iter().map(|(_, c)| format!("{}: {}", c.name, c.describe())));
        self.palette.show_picker(
            PickerKind::VsCodeImport,
            labels,
            "",
            "Import from VS Code: themes, snippets, languages",
        );
    }

    pub(super) fn picked_vscode_import(&mut self, ctx: &egui::Context, label: &str) {
        match label {
            CHOOSE_VSIX => {
                let dialog = rfd::FileDialog::new().add_filter("VS Code extension", &["vsix"]);
                let Some(file) = dialog.pick_file() else { return };
                let temp = std::env::temp_dir().join(format!("rust-code-vsix-{}", std::process::id()));
                let _ = std::fs::remove_dir_all(&temp);
                let result = vscode_import::unpack_vsix(&file, &temp);
                match result {
                    Ok(folder) => self.import_extension(ctx, &folder),
                    Err(why) => self.status = Some(why),
                }
                let _ = std::fs::remove_dir_all(&temp);
            }
            CHOOSE_FOLDER => {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    self.import_extension(ctx, &folder);
                }
            }
            _ => {
                let labels = vscode_import::installed();
                let found = labels.into_iter().find(|(_, c)| format!("{}: {}", c.name, c.describe()) == label);
                if let Some((dir, _)) = found {
                    self.import_extension(ctx, &dir);
                }
            }
        }
    }

    fn import_extension(&mut self, ctx: &egui::Context, dir: &Path) {
        let contents: Contents = match vscode_import::read(dir) {
            Ok(contents) => contents,
            Err(why) => {
                self.status = Some(why);
                return;
            }
        };
        let name = contents.name.clone();
        if !contents.is_useful() {
            let why = if contents.has_code { ": what it does is code, which only runs in VS Code" } else { "" };
            self.status = Some(format!("{name} has nothing to import{why}"));
            return;
        }
        let (has_code, grammars) = (contents.has_code, contents.grammars);
        match vscode_import::install(contents) {
            Ok(done) => {
                let mut left = Vec::new();
                if grammars > 0 {
                    left.push("its text colors");
                }
                if has_code {
                    left.push("its code");
                }
                let left = if left.is_empty() { String::new() } else { format!("; not {}", left.join(" or ")) };
                self.status = Some(format!(
                    "Imported from {name}: {} themes, {} snippets, {} languages{left}",
                    done.themes, done.snippets, done.languages
                ));
                self.themes.clear();
                self.settings_reload = true;
                if done.themes > 0 {
                    self.run_command(ctx, Command::SelectTheme);
                }
            }
            Err(why) => self.status = Some(format!("Couldn't import {name}: {why}")),
        }
    }
}
