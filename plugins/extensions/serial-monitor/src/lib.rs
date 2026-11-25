//! Serial Monitor: a board's serial port (Arduino, ESP32, a Pico…) or something standing in for
//! one, in a panel: what it prints as text with times, as hex or as a plot of its numbers, and a
//! box to send it lines. The editor opens and reads the port (permission `serial`); this
//! extension shows what comes.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod monitor;

use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{
    Block, Button, FieldKind, Form, FormField, FormValues, LogView, Parity, PlotSeries, Row, SerialSettings, Span,
};
use monitor::{Ending, Plot, Received};

const RED: u32 = 0xE5484D;
const DIM: u32 = 0x8B949E;

/// Lines sent that can be sent again.
const RECENT: usize = 5;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum View {
    #[default]
    Text,
    Hex,
    Plot,
}

/// The port open now.
struct Open {
    id: u64,
    path: String,
    /// When it opened, in milliseconds since 1970.
    since: i64,
}

struct State {
    /// Ports there are: path and what it is.
    ports: Vec<(String, String)>,
    path: String,
    baud: String,
    format: String,
    dtr: bool,
    rts: bool,
    open: Option<Open>,
    received: Received,
    plot: Plot,
    view: View,
    times: bool,
    follow: bool,
    ending: Ending,
    /// Lines sent, the last one last.
    history: Vec<String>,
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
            path: String::new(),
            baud: "115200".to_owned(),
            format: "8N1".to_owned(),
            dtr: true,
            rts: true,
            open: None,
            received: Received::default(),
            plot: Plot::default(),
            view: View::Text,
            times: false,
            follow: true,
            ending: Ending::default(),
            history: Vec::new(),
            error: None,
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// A port as the list shows it: `COM3  USB serial (CH340)`.
fn port_label(path: &str, description: &str) -> String {
    if description.is_empty() { path.to_owned() } else { format!("{path}  {description}") }
}

fn refresh() {
    let ports = host::serial_ports();
    with(|s| match ports {
        Ok(ports) => {
            s.ports = ports.into_iter().map(|p| (p.path, p.description)).collect();
            if s.path.is_empty()
                && let Some((first, _)) = s.ports.first()
            {
                s.path = first.clone();
            }
        }
        Err(e) => s.error = Some(e),
    });
}

fn connect() {
    with(|s| {
        let Ok(baud) = s.baud.trim().parse::<u32>() else {
            s.error = Some("The speed is a number of baud, like 115200".to_owned());
            return;
        };
        let Some((data_bits, parity, two_stop_bits)) = monitor::parse_format(&s.format) else {
            s.error = Some(format!("{} isn't a format like 8N1", s.format));
            return;
        };
        let path = s.path.trim().to_owned();
        if path.is_empty() {
            s.error = Some("Choose a port".to_owned());
            return;
        }
        let parity = match parity {
            'E' => Parity::Even,
            'O' => Parity::Odd,
            _ => Parity::None,
        };
        let settings = SerialSettings { baud, data_bits, parity, two_stop_bits, dtr: s.dtr, rts: s.rts };
        match host::serial_open(&path, settings) {
            Ok(opened) => {
                s.error = None;
                s.received.clear();
                s.plot.clear();
                let kind = if opened.is_serial {
                    format!("{baud} {}", s.format.to_ascii_uppercase())
                } else {
                    "not a serial port: no settings".to_owned()
                };
                host::show_message(&format!("Opened {path} ({kind})"));
                s.open = Some(Open { id: opened.id, path, since: host::now().unix_millis });
            }
            Err(e) => s.error = Some(e),
        }
    });
}

fn disconnect() {
    with(|s| {
        if let Some(open) = s.open.take() {
            host::serial_close(open.id);
            host::show_message(&format!("Closed {}", open.path));
        }
    });
}

fn send(text: String) {
    with(|s| {
        let Some(open) = &s.open else { return };
        let mut data = text.as_bytes().to_vec();
        data.extend_from_slice(s.ending.bytes());
        match host::serial_write(open.id, &data) {
            Ok(()) => {
                if !text.is_empty() {
                    s.history.retain(|h| *h != text);
                    s.history.push(text);
                }
            }
            Err(e) => s.error = Some(e),
        }
    });
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: hint.into() }
}

fn line_break() -> FormField {
    field("", "", FieldKind::LineBreak, "", Vec::new(), "")
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        port_ui(s, &mut b);
        if let Some(e) = &s.error {
            b.push(Block::Spans(vec![span(e.clone(), Some(RED))]));
        }
        view_bar(s, &mut b);
        b.push(Block::Separator);
        match s.view {
            View::Text => {
                let lines = s.received.shown(s.times);
                if lines.is_empty() {
                    let text = if s.open.is_some() {
                        "Nothing received yet."
                    } else {
                        "Connect to a port to see what it prints."
                    };
                    b.push(Block::Spans(vec![span(text, Some(DIM))]));
                }
                b.push(Block::Log(LogView { lines, follow: s.follow }));
            }
            View::Hex => {
                let bytes: Vec<u8> = s.received.bytes.iter().copied().collect();
                let offset = s.received.total - bytes.len() as u64;
                b.push(Block::Log(LogView { lines: monitor::hex_rows(&bytes, offset), follow: s.follow }));
            }
            View::Plot => {
                if s.plot.series.is_empty() {
                    let text = "Lines with numbers are plotted: 21.5,40 or temp:21.5 hum:40 (as Arduino's plotter reads them).";
                    b.push(Block::Spans(vec![span(text, Some(DIM))]));
                } else {
                    let series = s
                        .plot
                        .series
                        .iter()
                        .map(|(name, values)| PlotSeries {
                            name: name.clone(),
                            values: values.iter().copied().collect(),
                        })
                        .collect();
                    b.push(Block::Plot(series));
                }
            }
        }
        b.push(Block::Separator);
        send_ui(s, &mut b);
        b
    });
    host::set_panel("serial", &blocks);
}

fn port_ui(s: &State, b: &mut Vec<Block>) {
    let connected = s.open.is_some();
    let mut fields = Vec::new();
    if !connected {
        let mut options: Vec<String> = s.ports.iter().map(|(p, d)| port_label(p, d)).collect();
        if options.is_empty() {
            options.push("No serial ports: plug the board in, then Refresh".to_owned());
        }
        let chosen = s.ports.iter().find(|(p, _)| *p == s.path).map(|(p, d)| port_label(p, d));
        let typed = if chosen.is_some() { String::new() } else { s.path.clone() };
        fields.push(field(
            "port",
            "Port",
            FieldKind::Choice,
            &chosen.unwrap_or_else(|| options[0].clone()),
            options,
            "",
        ));
        fields.push(field("path", "", FieldKind::Text, &typed, Vec::new(), "or a path: COM7, /dev/ttyUSB0"));
        fields.push(line_break());
        let bauds: Vec<String> = monitor::BAUDS.iter().map(u32::to_string).collect();
        let (listed, other) = if bauds.contains(&s.baud) {
            (s.baud.clone(), String::new())
        } else {
            ("115200".to_owned(), s.baud.clone())
        };
        fields.push(field("baud", "Speed", FieldKind::Choice, &listed, bauds, ""));
        fields.push(field("other", "", FieldKind::Text, &other, Vec::new(), "other speed"));
        let formats = monitor::FORMATS.iter().map(|f| (*f).to_owned()).collect();
        fields.push(field("format", "Format", FieldKind::Choice, &s.format, formats, ""));
    }
    fields.push(field("dtr", "DTR", FieldKind::Check, &s.dtr.to_string(), Vec::new(), ""));
    fields.push(field("rts", "RTS", FieldKind::Check, &s.rts.to_string(), Vec::new(), ""));
    let buttons = if connected {
        vec!["Disconnect".to_owned(), "Set DTR and RTS".to_owned()]
    } else {
        vec!["Connect".to_owned(), "Refresh".to_owned()]
    };
    b.push(Block::Form(Form { id: "port".into(), fields, buttons }));
    if !connected {
        let why = "DTR on: many boards (Arduino) restart when the port opens. Format: data bits, parity (None, Even, Odd), stop bits.";
        b.push(Block::Spans(vec![span(why, Some(DIM))]));
    }
}

fn view_bar(s: &State, b: &mut Vec<Block>) {
    let mark = |on: bool, label: &str| if on { format!("● {label}") } else { label.to_owned() };
    let buttons = vec![
        button(&mark(s.view == View::Text, "Text"), "view", "text"),
        button(&mark(s.view == View::Hex, "Hex"), "view", "hex"),
        button(&mark(s.view == View::Plot, "Plot"), "view", "plot"),
        button(&mark(s.times, "Times"), "times", ""),
        button(&mark(s.follow, "Follow"), "follow", ""),
        button("Clear", "clear", ""),
        button("Open as text", "save", ""),
    ];
    b.push(Block::Rows(vec![Row { cells: Vec::new(), buttons }]));
    let state = match &s.open {
        Some(open) => format!("{} · {} bytes", open.path, s.received.total),
        None => "Not connected".to_owned(),
    };
    b.push(Block::Spans(vec![span(state, Some(DIM))]));
}

fn send_ui(s: &State, b: &mut Vec<Block>) {
    if s.open.is_none() {
        return;
    }
    let endings = Ending::ALL.iter().map(|e| e.label().to_owned()).collect();
    let fields = vec![
        field("text", "", FieldKind::Text, "", Vec::new(), "Text to send (Enter)"),
        field("ending", "", FieldKind::Choice, s.ending.label(), endings, ""),
    ];
    b.push(Block::Form(Form { id: "send".into(), fields, buttons: vec!["Send".into()] }));
    if !s.history.is_empty() {
        let rows = s
            .history
            .iter()
            .rev()
            .take(RECENT)
            .enumerate()
            .map(|(i, h)| Row {
                cells: vec![Span { text: h.clone(), color: None, bold: false, monospace: true }],
                buttons: vec![button("Send again", "again", &i.to_string())],
            })
            .collect();
        b.push(Block::Rows(rows));
    }
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" => refresh(),
        "view" => with(|s| {
            s.view = match arg {
                "hex" => View::Hex,
                "plot" => View::Plot,
                _ => View::Text,
            }
        }),
        "times" => with(|s| s.times = !s.times),
        "follow" => with(|s| s.follow = !s.follow),
        "clear" => with(|s| {
            s.received.clear();
            s.plot.clear();
        }),
        "save" => {
            let text = with(|s| s.received.text(s.times));
            host::open_text(&text);
        }
        "again" => {
            let text = with(|s| s.history.iter().rev().nth(arg.parse().unwrap_or(0)).cloned());
            if let Some(text) = text {
                send(text);
            }
        }
        _ => {}
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    let value = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.clone()).unwrap_or_default();
    match f.form.as_str() {
        "port" => {
            with(|s| {
                s.dtr = value("dtr") == "true";
                s.rts = value("rts") == "true";
                if s.open.is_none() {
                    let typed = value("path").trim().to_owned();
                    let listed = value("port");
                    let from_list = s.ports.iter().find(|(p, d)| port_label(p, d) == listed).map(|(p, _)| p.clone());
                    s.path = if typed.is_empty() { from_list.unwrap_or_default() } else { typed };
                    let other = value("other").trim().to_owned();
                    s.baud = if other.is_empty() { value("baud") } else { other };
                    s.format = value("format");
                }
            });
            match f.button.as_str() {
                "Connect" => connect(),
                "Refresh" => refresh(),
                "Disconnect" => disconnect(),
                _ => with(|s| {
                    if let Some(open) = &s.open
                        && let Err(e) = host::serial_set_lines(open.id, s.dtr, s.rts)
                    {
                        s.error = Some(e);
                    }
                }),
            }
        }
        "send" => {
            with(|s| s.ending = Ending::from_label(&value("ending")));
            send(value("text"));
        }
        _ => {}
    }
    render();
}

fn received(id: u64, data: &[u8]) {
    with(|s| {
        let Some(open) = &s.open else { return };
        if open.id != id {
            return;
        }
        let now = (host::now().unix_millis - open.since) as f64 / 1000.0;
        let dropped = s.received.push(data, now);
        s.plot.update(&s.received, dropped);
    });
    render();
}

fn closed(id: u64, reason: &str) {
    with(|s| {
        if s.open.as_ref().is_some_and(|o| o.id == id) {
            let path = s.open.take().map(|o| o.path).unwrap_or_default();
            s.error = Some(format!("{path} closed: {reason}"));
        }
    });
    render();
}

struct Plugin;

impl Guest for Plugin {
    fn activate() -> Result<(), String> {
        Ok(())
    }

    fn run_command(id: String, argument: String) -> Result<(), String> {
        command(&id, &argument)
    }

    fn on_event(event: Event) {
        match event {
            Event::FormSubmitted(f) => submitted(f),
            Event::SerialData(d) => received(d.id, &d.data),
            Event::SerialClosed(c) => closed(c.id, &c.reason),
            _ => {}
        }
    }
}

export!(Plugin);
