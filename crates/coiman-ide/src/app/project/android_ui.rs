//! Android: Devices and Logcat: the devices `adb` sees, each with a logcat view (filtered by
//! level, text and app), a shell and APK installs. Stack frames in log lines that are this
//! project's files are links.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

use coiman_core::language::lsp::discovery::{kill_tree, quiet_command};
use coiman_core::launch::stacktrace::{self, Place};
use coiman_core::project::adb::{self, Device, Filter, LogLine};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

/// Log lines kept; older ones go.
const MAX_LINES: usize = 50_000;

enum Msg {
    Devices(Result<Vec<Device>, String>),
    Line(u64, LogLine),
    Pids(Result<Vec<u32>, String>),
    Files(Vec<PathBuf>),
}

struct Logcat {
    serial: String,
    child: Child,
}

/// Coiman closing, or logcat stopped: the adb process goes too.
impl Drop for Logcat {
    fn drop(&mut self) {
        kill_tree(&mut self.child);
    }
}

pub(crate) struct AndroidView {
    open: bool,
    devices: Option<Result<Vec<Device>, String>>,
    logcat: Option<Logcat>,
    lines: VecDeque<LogLine>,
    filter: Filter,
    package: String,
    paused: bool,
    /// Which logcat run the lines are from: a new one ignores the last one's.
    run: u64,
    /// The project's files, for linking stack frames.
    files: Vec<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for AndroidView {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            open: false,
            devices: None,
            logcat: None,
            lines: VecDeque::new(),
            filter: Filter { min_level: adb::severity('I'), ..Filter::default() },
            package: String::new(),
            paused: false,
            run: 0,
            files: Vec::new(),
            tx,
            rx,
        }
    }
}

enum Action {
    Refresh,
    Logcat(String),
    StopLogcat,
    Shell(String),
    Install(String),
    Package,
    Clear,
    Open(PathBuf, u32),
}

fn missing() -> String {
    "adb isn't installed: install Android's SDK Platform-Tools (on PATH, in the Android SDK, or unpacked into the config folder's tools/platform-tools)".to_owned()
}

/// Runs `adb args`; its output, or why it failed.
fn run(args: &[&str]) -> Result<String, String> {
    let adb = adb::find_adb().ok_or_else(missing)?;
    let out = quiet_command(&adb).args(args).stdin(Stdio::null()).output().map_err(|e| format!("adb: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

fn level_color(level: char) -> Color32 {
    let pal = theme::colors();
    match level {
        'E' | 'F' | 'A' => Color32::from_rgb(0xE5, 0x53, 0x4B),
        'W' => Color32::from_rgb(0xCC, 0xA7, 0x00),
        'I' => pal.text,
        _ => pal.text_dim,
    }
}

impl CodeApp {
    pub(crate) fn show_android(&mut self, ctx: &egui::Context) {
        self.android.open = true;
        self.android_refresh(ctx);
    }

    fn android_refresh(&mut self, ctx: &egui::Context) {
        let (tx, wake) = (self.android.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let found = run(&["devices", "-l"]).map(|o| adb::parse_devices(&o));
            let _ = tx.send(Msg::Devices(found));
            wake();
        });
    }

    fn start_logcat(&mut self, ctx: &egui::Context, serial: String) {
        self.stop_logcat();
        let Some(adb) = adb::find_adb() else {
            self.status = Some(missing());
            return;
        };
        // The last few thousand lines, then what comes.
        let spawned = quiet_command(&adb)
            .args(["-s", &serial, "logcat", "-v", "threadtime", "-T", "2000"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                self.status = Some(format!("Couldn't start logcat: {e}"));
                return;
            }
        };
        self.android.run += 1;
        let run = self.android.run;
        self.android.lines.clear();
        if let Some(out) = child.stdout.take() {
            let (tx, wake) = (self.android.tx.clone(), repaint(ctx));
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(l) = adb::parse_line(&line)
                        && tx.send(Msg::Line(run, l)).is_err()
                    {
                        break;
                    }
                    wake();
                }
            });
        }
        // The project's files as they are now, for linking stack frames.
        if let Some(root) = self.explorer.root.clone() {
            let (tx, wake) = (self.android.tx.clone(), repaint(ctx));
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Files(stacktrace::project_files(&root)));
                wake();
            });
        }
        self.android.logcat = Some(Logcat { serial, child });
    }

    pub(crate) fn stop_logcat(&mut self) {
        self.android.logcat = None;
    }

    /// The package's processes, so only its lines show.
    fn android_package(&mut self, ctx: &egui::Context) {
        let package = self.android.package.trim().to_owned();
        if package.is_empty() {
            self.android.filter.pids.clear();
            return;
        }
        let Some(serial) = self.android.logcat.as_ref().map(|l| l.serial.clone()) else { return };
        let (tx, wake) = (self.android.tx.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let pids = run(&["-s", &serial, "shell", "pidof", &package]).map(|o| adb::parse_pids(&o));
            let _ = tx.send(Msg::Pids(
                pids.and_then(|p| if p.is_empty() { Err(format!("{package} isn't running")) } else { Ok(p) }),
            ));
            wake();
        });
    }

    fn android_terminal(&mut self, ctx: &egui::Context, title: String, args: Vec<String>) {
        let Some(adb) = adb::find_adb() else {
            self.status = Some(missing());
            return;
        };
        let program = adb.to_string_lossy().into_owned();
        let cwd = self.explorer.root.clone().unwrap_or_else(std::env::temp_dir);
        let env = self.environment_variables();
        let task = crate::ui::terminal::Task {
            title: &title,
            program: &program,
            args: &args,
            env: &env,
            cwd: &cwd,
            scan: None,
        };
        let scrollback = self.settings.terminal.scrollback;
        match self.terminal.run_task(ctx, task, scrollback) {
            Ok(()) => {
                self.panel_visible = true;
                self.panel_tab = crate::app::PanelTab::Terminal;
            }
            Err(e) => self.status = Some(format!("Couldn't start adb: {e}")),
        }
    }

    pub(crate) fn android_window(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.android.rx.try_recv() {
            match msg {
                Msg::Devices(d) => self.android.devices = Some(d),
                Msg::Line(run, line) => {
                    if run != self.android.run || self.android.paused {
                        continue;
                    }
                    let lines = &mut self.android.lines;
                    lines.push_back(line);
                    if lines.len() > MAX_LINES {
                        lines.pop_front();
                    }
                }
                Msg::Pids(Ok(p)) => self.android.filter.pids = p,
                Msg::Pids(Err(e)) => {
                    self.android.filter.pids.clear();
                    self.status = Some(e);
                }
                Msg::Files(f) => self.android.files = f,
            }
        }
        if !self.android.open {
            if self.android.logcat.is_some() {
                self.stop_logcat();
            }
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut action = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.85).min(1200.0), (room.y * 0.75).max(380.0));
        egui::Window::new(RichText::new("Android").size(16.0).strong())
            .id(egui::Id::unique("android_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| action = self.android_ui(ui));
        self.android.open = open;
        match action {
            Some(Action::Refresh) => self.android_refresh(ctx),
            Some(Action::Logcat(s)) => self.start_logcat(ctx, s),
            Some(Action::StopLogcat) => self.stop_logcat(),
            Some(Action::Shell(s)) => {
                self.android_terminal(ctx, format!("adb shell ({s})"), vec!["-s".into(), s, "shell".into()])
            }
            Some(Action::Install(s)) => {
                let Some(apk) = self.file_dialog("apk").add_filter("Android app", &["apk"]).pick_file() else { return };
                self.dialog_chose("apk", &apk);
                let args = vec!["-s".into(), s, "install".into(), "-r".into(), apk.to_string_lossy().into_owned()];
                let name = apk.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.android_terminal(ctx, format!("install {name}"), args);
            }
            Some(Action::Package) => self.android_package(ctx),
            Some(Action::Clear) => self.android.lines.clear(),
            Some(Action::Open(path, line)) => {
                self.open_file(&path);
                self.jump_to_symbol_line(line.saturating_sub(1) as usize);
            }
            None => {}
        }
    }

    fn android_ui(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let pal = theme::colors();
        let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
        let mut action = None;
        let a = &mut self.android;
        let watching = a.logcat.as_ref().map(|l| l.serial.clone());
        ui.horizontal(|ui| {
            ui.label(RichText::new("DEVICES").small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Refresh").clicked() {
                    action = Some(Action::Refresh);
                }
            });
        });
        match &a.devices {
            None => {
                ui.label(dim("Asking adb…"));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(pal.bracket_unexpected));
            }
            Some(Ok(list)) if list.is_empty() => {
                ui.label(dim(
                    "No devices: plug one in with USB debugging on (Developer options), or start an emulator.",
                ));
            }
            Some(Ok(list)) => {
                for d in list {
                    ui.horizontal(|ui| {
                        let color = if d.ready() { Color32::from_rgb(0x57, 0xC0, 0x6A) } else { pal.modified };
                        ui.label(RichText::new(d.title()).strong().color(color));
                        ui.label(dim(&format!("{}  {}", d.serial, d.state)));
                        if let Some(advice) = d.advice() {
                            ui.label(RichText::new(advice).color(pal.modified));
                            return;
                        }
                        if watching.as_deref() == Some(d.serial.as_str()) {
                            if ui.button("Stop logcat").clicked() {
                                action = Some(Action::StopLogcat);
                            }
                        } else if ui.button("Logcat").clicked() {
                            action = Some(Action::Logcat(d.serial.clone()));
                        }
                        if ui.button("Shell").on_hover_text("adb shell, in a terminal").clicked() {
                            action = Some(Action::Shell(d.serial.clone()));
                        }
                        if ui.button("Install APK…").on_hover_text("adb install -r, in a terminal").clicked() {
                            action = Some(Action::Install(d.serial.clone()));
                        }
                    });
                }
            }
        }
        if watching.is_none() {
            return action;
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("LOGCAT").small().strong());
            let levels = [('V', "Verbose"), ('D', "Debug"), ('I', "Info"), ('W', "Warning"), ('E', "Error")];
            let current = levels.iter().find(|(c, _)| adb::severity(*c) == a.filter.min_level).map_or("Info", |l| l.1);
            egui::ComboBox::from_id_salt("logcat_level").selected_text(current).show_ui(ui, |ui| {
                for (c, name) in levels {
                    ui.selectable_value(&mut a.filter.min_level, adb::severity(c), name);
                }
            });
            ui.add(egui::TextEdit::singleline(&mut a.filter.text).hint_text("Filter tag or text").desired_width(200.0));
            let pkg = ui.add(
                egui::TextEdit::singleline(&mut a.package)
                    .hint_text("App (com.example.app), Enter")
                    .desired_width(220.0),
            );
            if pkg.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = Some(Action::Package);
            }
            ui.toggle_value(&mut a.paused, "Pause");
            if ui.button("Clear").clicked() {
                action = Some(Action::Clear);
            }
        });
        let shown: Vec<&LogLine> = a.lines.iter().filter(|l| a.filter.keeps(l)).collect();
        ui.label(dim(&format!("{} of {} lines", shown.len(), a.lines.len())));
        let row_h = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
        let root = self.explorer.root.clone().unwrap_or_default();
        let files = &a.files;
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(!a.paused).show_rows(
            ui,
            row_h,
            shown.len(),
            |ui, range| {
                for l in &shown[range] {
                    let text = format!("{} {:>5} {} {}: {}", &l.time[6..], l.pid, l.level, l.tag, l.message);
                    let label = RichText::new(text).monospace().color(level_color(l.level));
                    // A stack frame in this project: a link to its line.
                    let place =
                        stacktrace::parse_frame(&l.message).and_then(|f| match stacktrace::locate(&f, &root, files) {
                            Place::Project(p) | Place::Maybe(p, _) => Some((p, f.line)),
                            Place::Elsewhere => None,
                        });
                    match place {
                        Some((path, line)) => {
                            if ui
                                .add(egui::Label::new(label.underline()).sense(egui::Sense::click()).extend())
                                .on_hover_text(path.display().to_string())
                                .clicked()
                            {
                                action = Some(Action::Open(path, line));
                            }
                        }
                        None => {
                            ui.add(egui::Label::new(label).extend());
                        }
                    }
                }
            },
        );
        action
    }
}
