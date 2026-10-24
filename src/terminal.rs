//! Integrated terminal panel: runs one shell command at a time and streams its output.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};

use egui::{Color32, RichText};

use crate::theme;

const MAX_LINES: usize = 5000;

enum Line {
    Out(String),
    Err(String),
    Info(String),
}

pub struct Terminal {
    cwd: PathBuf,
    input: String,
    lines: Vec<Line>,
    history: Vec<String>,
    history_pos: Option<usize>,
    tx: Sender<Line>,
    rx: Receiver<Line>,
    child: Option<Child>,
    pub focus_requested: bool,
}

impl Terminal {
    pub fn new(cwd: PathBuf) -> Self {
        let (tx, rx) = channel();
        Self {
            cwd,
            input: String::new(),
            lines: Vec::new(),
            history: Vec::new(),
            history_pos: None,
            tx,
            rx,
            child: None,
            focus_requested: false,
        }
    }

    pub fn set_cwd(&mut self, cwd: &Path) {
        self.cwd = cwd.to_path_buf();
    }

    fn push(&mut self, line: Line) {
        self.lines.push(line);
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - MAX_LINES);
        }
    }

    fn is_running(&mut self) -> bool {
        match &mut self.child {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    let code = status.code().map_or("?".to_owned(), |c| c.to_string());
                    self.child = None;
                    if code != "0" {
                        self.push(Line::Info(format!("[process exited with code {code}]")));
                    }
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.child = None;
                    false
                }
            },
            None => false,
        }
    }

    fn run(&mut self, command: String) {
        self.push(Line::Info(format!("{}> {}", self.cwd.display(), command)));
        if !command.trim().is_empty() && self.history.last() != Some(&command) {
            self.history.push(command.clone());
        }
        self.history_pos = None;

        let trimmed = command.trim();
        // Built-ins that must affect the terminal itself.
        if trimmed == "clear" || trimmed == "cls" {
            self.lines.clear();
            return;
        }
        if trimmed == "cd" || trimmed.starts_with("cd ") {
            let target = trimmed[2..].trim().trim_matches('"');
            if target.is_empty() {
                self.push(Line::Out(self.cwd.display().to_string()));
                return;
            }
            let new = self.cwd.join(target);
            match new.canonicalize() {
                Ok(p) if p.is_dir() => self.cwd = strip_verbatim(p),
                _ => self.push(Line::Err(format!("cd: no such directory: {target}"))),
            }
            return;
        }
        if trimmed.is_empty() {
            return;
        }

        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(trimmed);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg(trimmed);
            c
        };
        cmd.current_dir(&self.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        match cmd.spawn() {
            Ok(mut child) => {
                if let Some(out) = child.stdout.take() {
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            let _ = tx.send(Line::Out(line));
                        }
                    });
                }
                if let Some(err) = child.stderr.take() {
                    let tx = self.tx.clone();
                    std::thread::spawn(move || {
                        for line in BufReader::new(err).lines().map_while(Result::ok) {
                            let _ = tx.send(Line::Err(line));
                        }
                    });
                }
                self.child = Some(child);
            }
            Err(e) => self.push(Line::Err(format!("failed to run command: {e}"))),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        while let Ok(line) = self.rx.try_recv() {
            self.push(line);
        }
        let running = self.is_running();
        if running {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("TERMINAL").small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("🗑").on_hover_text("Clear").clicked() {
                    self.lines.clear();
                }
                if running
                    && ui.small_button("⏹").on_hover_text("Kill process").clicked()
                    && let Some(child) = &mut self.child
                {
                    let _ = child.kill();
                }
            });
        });

        let input_height = 26.0;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(ui.available_height() - input_height)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for line in &self.lines {
                    let (text, color) = match line {
                        Line::Out(s) => (s, theme::colors().text),
                        Line::Err(s) => (s, Color32::from_rgb(0xf1, 0x4c, 0x4c)),
                        Line::Info(s) => (s, Color32::from_rgb(0x3b, 0x8e, 0xea)),
                    };
                    ui.label(RichText::new(text).monospace().color(color));
                }
            });

        ui.horizontal(|ui| {
            let prompt = if running { "…".to_owned() } else { format!("{}>", self.cwd.display()) };
            ui.label(RichText::new(prompt).monospace().color(theme::colors().text_dim));
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.input)
                    .font(egui::TextStyle::Monospace)
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
            if self.focus_requested {
                edit.request_focus();
                self.focus_requested = false;
            }
            if edit.has_focus() {
                let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
                if up && !self.history.is_empty() {
                    let pos = self.history_pos.map_or(self.history.len() - 1, |p| p.saturating_sub(1));
                    self.history_pos = Some(pos);
                    self.input = self.history[pos].clone();
                }
                if down && let Some(p) = self.history_pos {
                    if p + 1 < self.history.len() {
                        self.history_pos = Some(p + 1);
                        self.input = self.history[p + 1].clone();
                    } else {
                        self.history_pos = None;
                        self.input.clear();
                    }
                }
            }
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if !running {
                    let command = std::mem::take(&mut self.input);
                    self.run(command);
                }
                edit.request_focus();
            }
        });
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
        }
    }
}

/// Removes the `\\?\` prefix that `canonicalize` adds on Windows.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}
