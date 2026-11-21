//! Serial: Open Monitor: a board's serial port (or something standing in for one), what it
//! prints as text, hex or a plot, and a box to send it lines.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use coiman_core::project::serial::{self, Ending, Port, PortInfo, Received, Settings};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::app::{CodeApp, repaint};
use crate::ui::theme;

const RED: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
/// The plot's series colors, in order.
const SERIES: [Color32; 6] = [
    Color32::from_rgb(0x4F, 0x9D, 0xFF),
    Color32::from_rgb(0xF2, 0x9E, 0x4C),
    Color32::from_rgb(0x57, 0xC0, 0x6A),
    Color32::from_rgb(0xE5, 0x48, 0x4D),
    Color32::from_rgb(0xB0, 0x7C, 0xF7),
    Color32::from_rgb(0x3F, 0xC1, 0xC9),
];
/// How many readings of each series the plot keeps.
const PLOT_POINTS: usize = 500;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Text,
    Hex,
    Plot,
}

enum Event {
    Data(Vec<u8>),
    Closed(String),
}

struct Session {
    port: Arc<Port>,
    stop: Arc<AtomicBool>,
    rx: Receiver<Event>,
    path: String,
    started: f64,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub(crate) struct SerialView {
    open: bool,
    ports: Vec<PortInfo>,
    path: String,
    baud: String,
    settings: Settings,
    session: Option<Session>,
    received: Received,
    view: View,
    times: bool,
    follow: bool,
    send: String,
    ending: Ending,
    history: Vec<String>,
    history_at: Option<usize>,
    /// Plot series: name, values; lines read so far for the plot.
    plot: Vec<(String, std::collections::VecDeque<f64>)>,
    plotted: usize,
    error: Option<String>,
}

impl Default for SerialView {
    fn default() -> Self {
        Self {
            open: false,
            ports: Vec::new(),
            path: String::new(),
            baud: "115200".to_owned(),
            settings: Settings::default(),
            session: None,
            received: Received::default(),
            view: View::Text,
            times: false,
            follow: true,
            send: String::new(),
            ending: Ending::default(),
            history: Vec::new(),
            history_at: None,
            plot: Vec::new(),
            plotted: 0,
            error: None,
        }
    }
}

enum Ask {
    Refresh,
    Connect,
    Disconnect,
    Send,
    Lines,
    Clear,
    Save,
}

impl CodeApp {
    pub(crate) fn show_serial(&mut self) {
        let v = &mut self.serial;
        v.open = true;
        v.ports = serial::ports();
        if v.path.is_empty()
            && let Some(first) = v.ports.first()
        {
            v.path = first.path.clone();
        }
    }

    fn serial_connect(&mut self, ctx: &egui::Context) {
        let v = &mut self.serial;
        let Ok(baud) = v.baud.trim().parse::<u32>() else {
            v.error = Some("The speed is a number of baud, like 115200".to_owned());
            return;
        };
        v.settings.baud = baud;
        let path = v.path.trim().to_owned();
        if path.is_empty() {
            v.error = Some("Choose a port".to_owned());
            return;
        }
        let port = match Port::open(&path, &v.settings) {
            Ok(p) => Arc::new(p),
            Err(e) => {
                v.error = Some(e);
                return;
            }
        };
        v.error = None;
        v.received.clear();
        v.plot.clear();
        v.plotted = 0;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (reader, stopped, wake) = (port.clone(), stop.clone(), repaint(ctx));
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            while !stopped.load(Ordering::Relaxed) {
                match reader.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        if tx.send(Event::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                        wake();
                    }
                    Err(e) => {
                        // Unplugged, or the other end hung up (a pipe): the OS says it its way.
                        let gone = e.kind() == std::io::ErrorKind::BrokenPipe
                            || matches!(e.raw_os_error(), Some(109 | 22 | 995 | 1167));
                        let why = if gone { "the device went away (unplugged?)".to_owned() } else { e.to_string() };
                        let _ = tx.send(Event::Closed(why));
                        wake();
                        break;
                    }
                }
            }
        });
        let started = ctx.input(|i| i.time);
        v.session = Some(Session { port, stop, rx, path: path.clone(), started });
        let kind = if v.session.as_ref().is_some_and(|s| s.port.is_serial()) {
            format!("{baud} {}", v.settings.format())
        } else {
            "not a serial port: no settings".to_owned()
        };
        self.status = Some(format!("Opened {path} ({kind})"));
    }

    pub(crate) fn serial_window(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        // What came, while the window is open or not.
        let mut closed = None;
        if let Some(s) = &self.serial.session {
            while let Ok(e) = s.rx.try_recv() {
                match e {
                    Event::Data(d) => self.serial.received.push(&d, now - s.started),
                    Event::Closed(e) => closed = Some(e),
                }
            }
        }
        if let Some(e) = closed {
            let path = self.serial.session.take().map(|s| s.path.clone()).unwrap_or_default();
            self.serial.error = Some(format!("{path} closed: {e}"));
        }
        self.serial_plot_update();
        if !self.serial.open {
            return;
        }
        let pal = theme::colors();
        let mut open = true;
        let mut ask = None;
        let room = ctx.content_rect().size();
        let size = egui::vec2((room.x * 0.7).min(980.0), (room.y * 0.7).max(380.0));
        egui::Window::new(RichText::new("Serial Monitor").size(16.0).strong())
            .id(egui::Id::unique("serial_window"))
            .open(&mut open)
            .collapsible(false)
            .default_size(size)
            .max_height(room.y - 40.0)
            .default_pos(ctx.content_rect().center() - size / 2.0)
            .frame(
                egui::Frame::window(&ctx.global_style())
                    .fill(pal.editor_bg)
                    .corner_radius(16.0)
                    .stroke(Stroke::new(1.0, pal.border))
                    .inner_margin(egui::Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| ask = serial_ui(ui, &mut self.serial, room.y * 0.7 - 170.0));
        self.serial.open = open;
        if !open {
            // Closing the window closes the port.
            self.serial.session = None;
        }
        let Some(ask) = ask else { return };
        match ask {
            Ask::Refresh => self.serial.ports = serial::ports(),
            Ask::Connect => self.serial_connect(ctx),
            Ask::Disconnect => {
                if let Some(s) = self.serial.session.take() {
                    self.status = Some(format!("Closed {}", s.path));
                }
            }
            Ask::Send => {
                let v = &mut self.serial;
                let Some(s) = &v.session else { return };
                let mut data = v.send.as_bytes().to_vec();
                data.extend_from_slice(v.ending.bytes());
                match s.port.write_all(&data) {
                    Ok(()) => {
                        let text = std::mem::take(&mut v.send);
                        if !text.is_empty() && v.history.last() != Some(&text) {
                            v.history.push(text);
                        }
                        v.history_at = None;
                    }
                    Err(e) => v.error = Some(format!("Couldn't send: {e}")),
                }
            }
            Ask::Lines => {
                let v = &mut self.serial;
                if let Some(s) = &v.session
                    && let Err(e) = s.port.set_lines(v.settings.dtr, v.settings.rts)
                {
                    v.error = Some(format!("Couldn't set DTR/RTS: {e}"));
                }
            }
            Ask::Clear => {
                self.serial.received.clear();
                self.serial.plot.clear();
                self.serial.plotted = 0;
            }
            Ask::Save => {
                let Some(path) = self.file_dialog("serial_log").set_file_name("serial.log").save_file() else { return };
                self.dialog_chose("serial_log", &path);
                let text = self.serial.received.text(self.serial.times);
                self.status = Some(match std::fs::write(&path, text) {
                    Ok(()) => format!("Saved the log to {}", path.display()),
                    Err(e) => format!("Couldn't save the log: {e}"),
                });
            }
        }
    }

    /// The plot takes the numbers of each new whole line.
    fn serial_plot_update(&mut self) {
        let v = &mut self.serial;
        // Lines dropped from the front (only the last ones are kept) shift the count.
        v.plotted = v.plotted.min(v.received.lines.len());
        for (_, line) in &v.received.lines[v.plotted..] {
            for (i, (name, value)) in serial::plot_values(line).into_iter().enumerate() {
                let name = name.unwrap_or_else(|| format!("value {}", i + 1));
                let at = match v.plot.iter().position(|(n, _)| *n == name) {
                    Some(at) => at,
                    None => {
                        v.plot.push((name, Default::default()));
                        v.plot.len() - 1
                    }
                };
                let values = &mut v.plot[at].1;
                values.push_back(value);
                if values.len() > PLOT_POINTS {
                    values.pop_front();
                }
            }
        }
        v.plotted = v.received.lines.len();
    }
}

fn serial_ui(ui: &mut egui::Ui, v: &mut SerialView, body: f32) -> Option<Ask> {
    let pal = theme::colors();
    let dim = |s: &str| RichText::new(s.to_owned()).color(pal.text_dim);
    let mut ask = None;
    let connected = v.session.is_some();
    // The connection.
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!connected, |ui| {
            let shown = v.ports.iter().find(|p| p.path == v.path).map_or_else(
                || if v.path.is_empty() { "Choose a port".to_owned() } else { v.path.clone() },
                |p| format!("{}  {}", p.path, p.description),
            );
            egui::ComboBox::from_id_salt("serial_port").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                for p in &v.ports {
                    ui.selectable_value(&mut v.path, p.path.clone(), format!("{}  {}", p.path, p.description));
                }
                if v.ports.is_empty() {
                    ui.label(dim("No serial ports: plug the board in, then Refresh."));
                }
            });
            if ui.small_button("Refresh").clicked() {
                ask = Some(Ask::Refresh);
            }
            ui.add(
                egui::TextEdit::singleline(&mut v.path).hint_text("or a path: COM7, /dev/ttyUSB0").desired_width(220.0),
            );
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if connected {
                if ui.button("Disconnect").clicked() {
                    ask = Some(Ask::Disconnect);
                }
            } else if ui.button("Connect").clicked() {
                ask = Some(Ask::Connect);
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!connected, |ui| {
            ui.label("Speed");
            // Typed, or one of the usual ones from the list beside.
            ui.add(egui::TextEdit::singleline(&mut v.baud).desired_width(70.0)).on_hover_text("Any speed, in baud");
            egui::ComboBox::from_id_salt("serial_baud").width(16.0).selected_text("").show_ui(ui, |ui| {
                for b in serial::BAUDS {
                    ui.selectable_value(&mut v.baud, b.to_string(), b.to_string());
                }
            });
            ui.label("baud");
            ui.add_space(8.0);
            let format = v.settings.format();
            egui::ComboBox::from_id_salt("serial_format")
                .width(56.0)
                .selected_text(&format)
                .show_ui(ui, |ui| {
                    for f in serial::FORMATS {
                        if ui.selectable_label(format == f, f).clicked()
                            && let Some(s) = v.settings.with_format(f)
                        {
                            v.settings = s;
                        }
                    }
                })
                .response
                .on_hover_text("Data bits, parity (None, Even, Odd) and stop bits");
        });
        let dtr = ui
            .checkbox(&mut v.settings.dtr, "DTR")
            .on_hover_text("On: many boards (Arduino) restart when the port opens");
        let rts = ui.checkbox(&mut v.settings.rts, "RTS");
        if connected && (dtr.changed() || rts.changed()) {
            ask = Some(Ask::Lines);
        }
    });
    if let Some(e) = &v.error {
        ui.colored_label(RED, e);
    }
    // The view.
    ui.horizontal(|ui| {
        for (view, label) in [(View::Text, "Text"), (View::Hex, "Hex"), (View::Plot, "Plot")] {
            if ui.selectable_label(v.view == view, label).clicked() {
                v.view = view;
            }
        }
        ui.separator();
        ui.checkbox(&mut v.times, "Times");
        ui.checkbox(&mut v.follow, "Follow");
        if ui.small_button("Clear").clicked() {
            ask = Some(Ask::Clear);
        }
        if ui.small_button("Save log…").clicked() {
            ask = Some(Ask::Save);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let state = match &v.session {
                Some(s) => format!("{} · {} bytes", s.path, v.received.total),
                None => "Not connected".to_owned(),
            };
            ui.label(dim(&state));
        });
    });
    ui.separator();
    let height = body.max(160.0);
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.set_min_height(height);
        match v.view {
            View::Text => {
                let partial = v.received.partial();
                if v.received.lines.is_empty() && partial.is_none() {
                    ui.label(dim(if connected {
                        "Nothing received yet."
                    } else {
                        "Connect to a port to see what it prints."
                    }));
                }
                let scroll = egui::ScrollArea::both()
                    .id_salt("serial_text")
                    .auto_shrink([false, false])
                    .stick_to_bottom(v.follow);
                let rows = v.received.lines.len() + usize::from(partial.is_some());
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                scroll.show_rows(ui, row_height, rows, |ui, range| {
                    for i in range {
                        let (t, line) = match v.received.lines.get(i) {
                            Some((t, l)) => (*t, l.as_str()),
                            None => match &partial {
                                Some((t, l)) => (*t, l.as_str()),
                                None => continue,
                            },
                        };
                        ui.horizontal(|ui| {
                            if v.times {
                                ui.label(RichText::new(serial::clock(t)).monospace().color(pal.text_dim));
                            }
                            ui.label(RichText::new(line).monospace());
                        });
                    }
                });
            }
            View::Hex => {
                let bytes: Vec<u8> = v.received.bytes.iter().copied().collect();
                let offset = v.received.total - bytes.len() as u64;
                let rows = serial::hex_rows(&bytes, offset);
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .id_salt("serial_hex")
                    .auto_shrink([false, false])
                    .stick_to_bottom(v.follow)
                    .show_rows(ui, row_height, rows.len(), |ui, range| {
                        for row in &rows[range] {
                            ui.label(RichText::new(row).monospace());
                        }
                    });
            }
            View::Plot => plot_ui(ui, v),
        }
    });
    ui.separator();
    // Sending.
    ui.add_enabled_ui(connected, |ui| {
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut v.send)
                    .hint_text("Text to send (Enter; Up for the last ones)")
                    .font(egui::TextStyle::Monospace)
                    .desired_width(ui.available_width() - 260.0),
            );
            if r.has_focus() && !v.history.is_empty() {
                let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
                if up || down {
                    let last = v.history.len() - 1;
                    let at = match (v.history_at, up) {
                        (None, true) => Some(last),
                        (Some(i), true) => Some(i.saturating_sub(1)),
                        (Some(i), false) if i < last => Some(i + 1),
                        _ => None,
                    };
                    v.history_at = at;
                    v.send = at.map(|i| v.history[i].clone()).unwrap_or_default();
                }
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Send").clicked() || enter {
                ask = Some(Ask::Send);
                r.request_focus();
            }
            egui::ComboBox::from_id_salt("serial_ending").width(150.0).selected_text(v.ending.label()).show_ui(
                ui,
                |ui| {
                    for e in Ending::ALL {
                        ui.selectable_value(&mut v.ending, e, e.label());
                    }
                },
            );
        });
    });
    ask
}

/// The numbers of each line over time, one colored line per series, scaled to fit.
fn plot_ui(ui: &mut egui::Ui, v: &SerialView) {
    let pal = theme::colors();
    if v.plot.is_empty() {
        ui.label(
            RichText::new(
                "Lines with numbers are plotted: 21.5,40 or temp:21.5 hum:40 (as Arduino's plotter reads them).",
            )
            .color(pal.text_dim),
        );
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (i, (name, values)) in v.plot.iter().enumerate() {
            let color = SERIES[i % SERIES.len()];
            let last = values.back().copied().unwrap_or_default();
            ui.label(RichText::new(format!("■ {name} {last}")).color(color));
        }
    });
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, values) in &v.plot {
        for &x in values {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if hi - lo < 1e-9 {
        lo -= 1.0;
        hi += 1.0;
    }
    let pad = (hi - lo) * 0.05;
    let (lo, hi) = (lo - pad, hi + pad);
    let inner = rect.shrink(6.0);
    let y = |x: f64| inner.bottom() - ((x - lo) / (hi - lo)) as f32 * inner.height();
    // Zero, when it's in view.
    if lo < 0.0 && hi > 0.0 {
        painter.hline(inner.x_range(), y(0.0), Stroke::new(1.0, pal.border));
    }
    for (i, (_, values)) in v.plot.iter().enumerate() {
        let n = values.len().max(2);
        let points: Vec<egui::Pos2> = values
            .iter()
            .enumerate()
            .map(|(k, &x)| egui::pos2(inner.left() + inner.width() * k as f32 / (n - 1).max(1) as f32, y(x)))
            .collect();
        painter.add(egui::Shape::line(points, Stroke::new(1.5, SERIES[i % SERIES.len()])));
    }
    let label = |t: f64| RichText::new(format!("{t:.3}").trim_end_matches('0').trim_end_matches('.').to_owned());
    painter.text(
        inner.left_top(),
        egui::Align2::LEFT_TOP,
        label(hi).text(),
        egui::FontId::monospace(11.0),
        pal.text_dim,
    );
    painter.text(
        inner.left_bottom(),
        egui::Align2::LEFT_BOTTOM,
        label(lo).text(),
        egui::FontId::monospace(11.0),
        pal.text_dim,
    );
}
