//! "Search" side bar: find text across all files in the open folder.

use std::path::{Path, PathBuf};

use egui::RichText;

use crate::{explorer, theme};

const MAX_RESULTS: usize = 2000;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;

pub struct SearchHit {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
}

#[derive(Default)]
pub struct SearchPanel {
    pub query: String,
    pub match_case: bool,
    hits: Vec<SearchHit>,
    searched_files: usize,
    pub focus_requested: bool,
}

impl SearchPanel {
    /// Draws the panel. Returns `(file, line)` when the user clicks a result.
    pub fn ui(&mut self, ui: &mut egui::Ui, root: Option<&Path>) -> Option<(PathBuf, usize)> {
        let Some(root) = root else {
            ui.label(RichText::new("Open a folder to search in files.").color(theme::TEXT_DIM));
            return None;
        };

        let mut run = false;
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search")
                    .desired_width(ui.available_width() - 34.0),
            );
            if self.focus_requested {
                edit.request_focus();
                self.focus_requested = false;
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run = true;
            }
            if ui
                .selectable_label(self.match_case, "Aa")
                .on_hover_text("Match Case")
                .clicked()
            {
                self.match_case = !self.match_case;
                run = true;
            }
        });
        if run && !self.query.is_empty() {
            self.run(root);
        }

        if self.searched_files > 0 {
            ui.label(
                RichText::new(format!(
                    "{} results in {} files searched",
                    self.hits.len(),
                    self.searched_files
                ))
                .small()
                .color(theme::TEXT_DIM),
            );
        }
        ui.separator();

        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut last_path: Option<&Path> = None;
                for hit in &self.hits {
                    if last_path != Some(hit.path.as_path()) {
                        let rel = hit.path.strip_prefix(root).unwrap_or(&hit.path);
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("📄 {}", rel.display())).strong());
                        last_path = Some(hit.path.as_path());
                    }
                    let label = format!("  {}: {}", hit.line, hit.text);
                    if ui
                        .add(egui::Button::new(RichText::new(label).monospace().size(12.0)).frame(false))
                        .clicked()
                    {
                        clicked = Some((hit.path.clone(), hit.line));
                    }
                }
            });
        clicked
    }

    fn run(&mut self, root: &Path) {
        self.hits.clear();
        let files = explorer::walk_files(root, 50_000);
        self.searched_files = files.len();
        let needle = if self.match_case { self.query.clone() } else { self.query.to_lowercase() };
        'files: for path in files {
            if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE_SIZE).unwrap_or(true) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            // Skip binary files.
            if bytes.iter().take(8000).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                let hay = if self.match_case { line.to_owned() } else { line.to_lowercase() };
                if hay.contains(&needle) {
                    let trimmed: String = line.trim().chars().take(200).collect();
                    self.hits.push(SearchHit { path: path.clone(), line: i + 1, text: trimmed });
                    if self.hits.len() >= MAX_RESULTS {
                        break 'files;
                    }
                }
            }
        }
    }
}
