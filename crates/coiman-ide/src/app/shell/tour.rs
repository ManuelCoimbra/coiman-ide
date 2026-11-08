//! The introduction: a short tour of the window, pointing at each part as it explains it,
//! and a sample project to try things on. It's offered on the Welcome page, never
//! forced, and Help: Take the Tour shows it again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use egui::{Align2, Color32, RichText};

use crate::app::CodeApp;
use crate::commands::Command;
use crate::ui::palette::PickerKind;
use crate::ui::theme;

/// One stop: what it says, what it points at (a spot the window drew), and what it offers
/// to do.
struct Step {
    title: &'static str,
    body: &'static str,
    spot: Option<&'static str>,
    action: Option<(&'static str, Command)>,
}

const STEPS: &[Step] = &[
    Step {
        title: "Welcome to Coiman IDE",
        body: "A minute's tour of where things are. Next and Back move through it, Escape stops it, and Help: Take the Tour brings it back.",
        spot: None,
        action: None,
    },
    Step {
        title: "Projects and files",
        body: "The Project view shows the open folder. Open a folder, or make a small sample project to try things on. The project's name at the top switches between recent ones.",
        spot: Some("explorer"),
        action: Some(("New Sample Project...", Command::NewSampleProject)),
    },
    Step {
        title: "The editor",
        body: "Files open here in tabs. Ctrl+P goes to a file by name, Ctrl+G to a line, and Ctrl+\\ splits the editor.",
        spot: Some("editor"),
        action: Some(("Go to File...", Command::QuickOpen)),
    },
    Step {
        title: "Search and commands",
        body: "This box (or Shift twice) searches files, symbols, commands, settings and text. Ctrl+Shift+P lists every command.",
        spot: Some("search"),
        action: Some(("Show All Commands", Command::CommandPalette)),
    },
    Step {
        title: "Run and debug",
        body: "The green arrow runs what's chosen beside it. Ctrl+Shift+F10 runs the file in front, F5 debugs it.",
        spot: Some("run"),
        action: None,
    },
    Step {
        title: "The terminal",
        body: "Ctrl+` opens a terminal in the project's folder. Programs started with Run show their output here, and take input.",
        spot: Some("terminal"),
        action: Some(("Open the Terminal", Command::ToggleTerminal)),
    },
    Step {
        title: "Language tooling",
        body: "Completion, errors and go to definition come from a language server for each language. When one is missing, a bar over the file says what to install, and nothing is downloaded until you say so. Language: Doctor checks a file's tools.",
        spot: Some("status"),
        action: Some(("Language: Doctor", Command::ShowLanguageDoctor)),
    },
    Step {
        title: "Git",
        body: "Source Control shows changes, commits and branches. Set the name and email your commits are made with, clone a repository, or start one here.",
        spot: Some("git"),
        action: Some(("Set Commit Author...", Command::GitSetAuthor)),
    },
    Step {
        title: "Settings and extensions",
        body: "The gear opens Settings, searchable. Extensions holds plugins, imports themes and snippets from VS Code, and says which VS Code extensions work here.",
        spot: Some("settings"),
        action: Some(("Extensions", Command::ShowExtensions)),
    },
    Step {
        title: "Try it",
        body: "Make a sample project, open its main file and press Ctrl+Shift+F10 to run it. That's the tour: Help: Take the Tour shows it again.",
        spot: None,
        action: Some(("New Sample Project...", Command::NewSampleProject)),
    },
];

/// Sample projects: label, folder name, files (path, text), the file to open.
type Sample = (&'static str, &'static str, &'static [(&'static str, &'static str)], &'static str);

const SAMPLES: &[Sample] = &[
    (
        "Python",
        "hello-python",
        &[
            (
                "main.py",
                "def greet(name: str) -> str:\n    return f\"Hello, {name}!\"\n\n\nif __name__ == \"__main__\":\n    print(greet(\"Coiman\"))\n",
            ),
            ("README.md", "# hello-python\n\nPress **Ctrl+Shift+F10** in `main.py` to run it, or **F5** to debug.\n"),
        ],
        "main.py",
    ),
    (
        "C",
        "hello-c",
        &[
            (
                "main.c",
                "#include <stdio.h>\n\nint main(void) {\n    printf(\"Hello, Coiman!\\n\");\n    return 0;\n}\n",
            ),
            (
                "README.md",
                "# hello-c\n\nPress **Ctrl+Shift+F10** in `main.c` to compile and run it (gcc or clang), or **F5** to debug.\n",
            ),
        ],
        "main.c",
    ),
    (
        "Java",
        "hello-java",
        &[
            (
                "Main.java",
                "public class Main {\n    public static void main(String[] args) {\n        System.out.println(\"Hello, Coiman!\");\n    }\n}\n",
            ),
            ("README.md", "# hello-java\n\nPress **Ctrl+Shift+F10** in `Main.java` to compile and run it.\n"),
        ],
        "Main.java",
    ),
    (
        "JavaScript",
        "hello-js",
        &[
            ("main.js", "function greet(name) {\n  return `Hello, ${name}!`;\n}\n\nconsole.log(greet(\"Coiman\"));\n"),
            ("README.md", "# hello-js\n\nPress **Ctrl+Shift+F10** in `main.js` to run it with Node.js.\n"),
        ],
        "main.js",
    ),
    (
        "Rust",
        "hello-rust",
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"hello-rust\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\n",
            ),
            ("src/main.rs", "fn main() {\n    println!(\"Hello, Coiman!\");\n}\n"),
            ("README.md", "# hello-rust\n\nPress **Ctrl+Shift+F10** in `src/main.rs` to build and run it.\n"),
        ],
        "src/main.rs",
    ),
];

#[derive(Default)]
pub(crate) struct Tour {
    /// The step showing, while the tour runs.
    pub(crate) step: Option<usize>,
    /// Where the window drew the parts the tour points at, this frame.
    pub(crate) spots: HashMap<&'static str, egui::Rect>,
    /// A sample project being made: the language picked.
    sample: Option<usize>,
}

fn seen_file() -> Option<PathBuf> {
    coiman_core::settings::config_dir().map(|d| d.join("tour_seen"))
}

/// Whether the tour was taken or put aside before.
pub(crate) fn seen() -> bool {
    seen_file().is_some_and(|f| f.exists())
}

fn mark_seen() {
    if let Some(f) = seen_file() {
        let _ = std::fs::write(f, "");
    }
}

/// Writes a sample project into `parent`. Returns its folder and the file to open.
pub(crate) fn make_sample(index: usize, parent: &Path) -> Result<(PathBuf, PathBuf), String> {
    let (_, folder, files, main) = SAMPLES.get(index).ok_or("No such sample")?;
    let dir = parent.join(folder);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    for (path, text) in *files {
        let file = dir.join(path);
        if let Some(p) = file.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
    }
    Ok((dir.clone(), dir.join(main)))
}

impl CodeApp {
    /// Help: Take the Tour.
    pub(crate) fn start_tour(&mut self) {
        self.tour.step = Some(0);
        mark_seen();
    }

    /// Notes where a part of the window is, for the tour to point at.
    pub(crate) fn tour_spot(&mut self, name: &'static str, rect: egui::Rect) {
        if self.tour.step.is_some() {
            self.tour.spots.insert(name, rect);
        }
    }

    /// Help: New Sample Project... — a language, then a folder to make it in.
    pub(crate) fn new_sample_project(&mut self) {
        let items = SAMPLES.iter().map(|s| s.0.to_owned()).collect();
        self.palette.show_picker(PickerKind::SampleProject, items, "", "Select a language for the sample project");
    }

    pub(crate) fn picked_sample(&mut self, label: &str) {
        let Some(index) = SAMPLES.iter().position(|s| s.0 == label) else { return };
        self.tour.sample = Some(index);
        let Some(parent) =
            self.file_dialog("sample_project").set_title("Folder to make the sample project in").pick_folder()
        else {
            return;
        };
        self.dialog_chose("sample_project", &parent);
        match make_sample(index, &parent) {
            Ok((dir, main)) => {
                coiman_core::applog::info(&format!("sample project made: {}", dir.display()));
                // It was made here, by the user's choice: trusted like a folder they opened.
                let _ = coiman_core::project::trust::trust(&dir);
                self.open_folder(dir);
                self.open_file(&main);
                self.status = Some("Sample project made: Ctrl+Shift+F10 runs it".to_owned());
                if self.tour.step.is_some() {
                    self.tour.step = Some(STEPS.len() - 1);
                }
            }
            Err(e) => self.status = Some(e),
        }
    }

    /// The tour's card, and the part it points at, over everything. Returns a command
    /// its button asks for.
    pub(crate) fn tour_ui(&mut self, ctx: &egui::Context) -> Option<Command> {
        let at = self.tour.step?;
        let Some(step) = STEPS.get(at) else {
            self.tour.step = None;
            return None;
        };
        let pal = theme::colors();
        let screen = ctx.content_rect();
        let spot = step.spot.and_then(|s| self.tour.spots.get(s).copied());
        // Dim all but the part spoken of, and ring it.
        let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("tour_dim"));
        let painter = ctx.layer_painter(layer);
        let shade = Color32::from_black_alpha(110);
        match spot {
            Some(r) => {
                let r = r.expand(4.0);
                painter.rect_filled(
                    egui::Rect::from_min_max(screen.min, egui::pos2(screen.max.x, r.min.y)),
                    0.0,
                    shade,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(screen.min.x, r.max.y), screen.max),
                    0.0,
                    shade,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(screen.min.x, r.min.y), egui::pos2(r.min.x, r.max.y)),
                    0.0,
                    shade,
                );
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(r.max.x, r.min.y), egui::pos2(screen.max.x, r.max.y)),
                    0.0,
                    shade,
                );
                painter.rect_stroke(r, 8.0, egui::Stroke::new(2.0, pal.accent), egui::StrokeKind::Outside);
            }
            None => {
                painter.rect_filled(screen, 0.0, shade);
            }
        }
        // The card beside the spot, inside the window; centered when there's none.
        const W: f32 = 360.0;
        let (pos, pivot) = match spot {
            Some(r) if r.right() + 16.0 + W <= screen.right() && r.width() < screen.width() * 0.5 => (
                egui::pos2(r.right() + 16.0, r.center().y.clamp(screen.top() + 80.0, screen.bottom() - 120.0)),
                Align2::LEFT_CENTER,
            ),
            Some(r) if r.bottom() + 220.0 <= screen.bottom() => (
                egui::pos2(
                    r.center().x.clamp(screen.left() + W / 2.0 + 8.0, screen.right() - W / 2.0 - 8.0),
                    r.bottom() + 16.0,
                ),
                Align2::CENTER_TOP,
            ),
            Some(r) => (
                egui::pos2(
                    r.center().x.clamp(screen.left() + W / 2.0 + 8.0, screen.right() - W / 2.0 - 8.0),
                    r.top() - 16.0,
                ),
                Align2::CENTER_BOTTOM,
            ),
            None => (screen.center(), Align2::CENTER_CENTER),
        };
        // Keys the tour takes, so the editor behind doesn't move too.
        let mut stop = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut next = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight));
        let mut back = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft));
        let mut action = None;
        egui::Area::new(egui::Id::unique("tour_card"))
            .order(egui::Order::Tooltip)
            .fixed_pos(pos)
            .pivot(pivot)
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(pal.sidebar_bg).inner_margin(16.0).corner_radius(10.0).show(
                    ui,
                    |ui| {
                        ui.set_width(W);
                        ui.label(RichText::new(format!("{} of {}", at + 1, STEPS.len())).small().color(pal.text_dim));
                        ui.label(RichText::new(step.title).size(18.0).strong().color(pal.text));
                        ui.add_space(4.0);
                        ui.label(RichText::new(step.body).color(pal.text));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if let Some((label, cmd)) = step.action
                                && ui.button(label).clicked()
                            {
                                action = Some(cmd);
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let last = at + 1 == STEPS.len();
                                next |= ui.button(if last { "Done" } else { "Next" }).clicked();
                                if at > 0 {
                                    back |= ui.button("Back").clicked();
                                }
                                if !last && ui.link("Skip the tour").clicked() {
                                    stop = true;
                                }
                            });
                        });
                    },
                );
            });
        if stop {
            self.tour.step = None;
        } else if next {
            self.tour.step = (at + 1 < STEPS.len()).then_some(at + 1);
        } else if back {
            self.tour.step = Some(at.saturating_sub(1));
        }
        if self.tour.step.is_none() {
            self.tour.spots.clear();
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_made_once() {
        let parent = std::env::temp_dir().join(format!("rc-samples-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&parent);
        std::fs::create_dir_all(&parent).unwrap();
        for i in 0..SAMPLES.len() {
            let (dir, main) = make_sample(i, &parent).unwrap();
            assert!(main.is_file() && main.starts_with(&dir));
            assert!(dir.join("README.md").is_file());
            assert!(make_sample(i, &parent).is_err(), "not over one that's there");
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn every_step_says_something() {
        assert!(STEPS.len() >= 8);
        for s in STEPS {
            assert!(!s.title.is_empty() && s.body.len() > 40);
        }
    }
}
