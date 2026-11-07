//! Importing themes, snippets and languages from VS Code extensions.

use std::path::Path;

use coiman_core::commands::Command;
use coiman_core::vscode_compat;
use coiman_core::vscode_import::{self, Contents};

use super::{CodeApp, repaint};
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
                let dialog = self.file_dialog("vsix").add_filter("VS Code extension", &["vsix"]);
                let Some(file) = dialog.pick_file() else { return };
                self.dialog_chose("vsix", &file);
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
                if let Some(folder) = self.file_dialog("vscode_extension_folder").pick_folder() {
                    self.dialog_chose("vscode_extension_folder", &folder);
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

    /// Extensions: VS Code Compatibility Report — every installed extension, what works
    /// here and why the rest can't, as a Markdown page.
    pub(super) fn vscode_compatibility(&mut self, ctx: &egui::Context) {
        let Some(file) = coiman_core::settings::config_dir().map(|d| d.join("vscode-compatibility.md")) else {
            self.status = Some("No config folder to write the report to".to_owned());
            return;
        };
        self.status = Some("Reading VS Code's extensions...".to_owned());
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = repaint(ctx);
        std::thread::spawn(move || {
            let reports = vscode_compat::installed();
            let text = vscode_compat::markdown(&reports);
            let result = std::fs::write(&file, text).map(|()| (file, reports.len())).map_err(|e| e.to_string());
            let _ = tx.send(result);
            wake();
        });
        self.vscode_report = Some(rx);
    }

    pub(super) fn pump_vscode_report(&mut self) {
        let Some(rx) = &self.vscode_report else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.vscode_report = None;
        match result {
            Ok((_, 0)) => self.status = Some("No VS Code extensions found (in ~/.vscode/extensions)".to_owned()),
            Ok((file, n)) => {
                // Read again if it was open from last time.
                if let Some(i) = self.doc_index_by_path(&file) {
                    let _ = self.docs[i].reload_from_disk();
                    self.active = i;
                } else {
                    self.open_file(&file);
                }
                if !self.md_preview.open {
                    self.toggle_markdown_preview();
                }
                self.status = Some(format!("VS Code compatibility: {n} extensions"));
            }
            Err(e) => self.status = Some(format!("Couldn't write the report: {e}")),
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
            // Why, from what its code needs, and what does it here instead.
            self.status = Some(match vscode_compat::examine(dir) {
                Ok(report) => report.summary(),
                Err(_) => format!("{name} has nothing to import"),
            });
            return;
        }
        let (has_code, grammars) = (contents.has_code, contents.grammars);
        let report = has_code.then(|| vscode_compat::examine(dir).ok()).flatten();
        match vscode_import::install(contents) {
            Ok(done) => {
                let mut left = Vec::new();
                if grammars > 0 {
                    left.push("its text colors");
                }
                if has_code {
                    left.push("its code");
                }
                let mut left = if left.is_empty() { String::new() } else { format!("; not {}", left.join(" or ")) };
                if let Some(
                    vscode_compat::Verdict::NeedsVsCode(Some(instead)) | vscode_compat::Verdict::BuiltIn(instead),
                ) = report.as_ref().map(|r| &r.verdict)
                {
                    left.push_str(&format!(" (here: {instead})"));
                }
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
