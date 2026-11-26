//! Binary Size: what takes room in a program, by group (crate or namespace), symbol and
//! section, and what changed since it was last looked at, in a panel. The file is read through
//! the editor a piece at a time, so a program of hundreds of megabytes only has its headers,
//! sections and symbol names read.

// The bindings wit-bindgen writes.
#![allow(clippy::too_many_arguments)]

wit_bindgen::generate!({
    path: "../../../crates/coiman-core/wit/plugin.wit",
    world: "plugin",
});

pub mod memory;
pub mod pages;
pub mod size;

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Mutex;

use coiman::plugin::host;
use coiman::plugin::types::{Block, Button, FieldKind, Form, FormField, FormValues, Row, Span, Table as Grid};
use pages::Pages;
use size::{Report, Scan, Summary, human, human_change};

const RED: u32 = 0xE5484D;
/// Smaller than before (the color the rest of Coiman uses for good news).
const GREEN: u32 = 0x57C06A;
const DIM: u32 = 0x8B949E;
const ACCENT: u32 = 0x4C9AFF;
const ORANGE: u32 = 0xE2A93B;

/// How many symbols are listed; the filter finds the rest.
const SHOWN: usize = 400;
/// Programs offered from the build folders, newest first.
const CANDIDATES: usize = 40;
/// How long one step of reading may take, in milliseconds.
const STEP_MS: i64 = 300;
/// Symbols looked at between looks at the clock.
const BATCH: usize = 2000;

/// A file of the open folder read through the editor, as `object` reads it.
struct HostFile {
    path: String,
    at: u64,
    len: u64,
}

impl Read for HostFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let want = buf.len().min(8 << 20) as u32;
        let got = host::read_bytes(&self.path, self.at, want).map_err(std::io::Error::other)?;
        buf[..got.len()].copy_from_slice(&got);
        self.at += got.len() as u64;
        Ok(got.len())
    }
}

impl Seek for HostFile {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let at = match to {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.at.checked_add_signed(d),
        };
        self.at = at.ok_or_else(|| std::io::Error::other("seek before the start"))?;
        Ok(self.at)
    }
}

/// A file being read.
struct Running {
    path: String,
    cache: Pages<HostFile>,
    scan: Scan,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Tab {
    #[default]
    Groups,
    Symbols,
    Sections,
    /// A firmware's memory map: regions and what sits in them.
    Memory,
}

#[derive(Default)]
struct State {
    /// Programs in the build folders (relative paths), newest first.
    candidates: Vec<String>,
    /// The file in the editor, when it's one.
    active: Option<String>,
    chosen: Option<String>,
    running: Option<Running>,
    report: Option<Result<Report, String>>,
    /// The report before the latest one, for each file looked at.
    earlier: HashMap<String, Summary>,
    tab: Tab,
    filter: String,
    /// The memory layout: the file it came from (relative path) and its regions.
    layout: Option<(String, Vec<memory::Region>)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Whether a file of the open folder is a program, a library or an object file.
fn is_binary(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    size::binary_name(name).unwrap_or_else(|| host::read_bytes(path, 0, 4).is_ok_and(|head| size::binary_start(&head)))
}

/// Programs, libraries and object files in the build folders (`target`, `build`, `out`,
/// `cmake-build-*`), newest first.
fn candidates() -> Vec<String> {
    let mut found: Vec<(u64, String)> = Vec::new();
    let Ok(top) = host::read_dir("") else { return Vec::new() };
    for entry in top {
        if entry.is_dir && size::build_folder(&entry.name) {
            walk(&entry.name, 0, &mut found);
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found.into_iter().map(|(_, p)| p).take(CANDIDATES).collect()
}

fn walk(dir: &str, depth: usize, found: &mut Vec<(u64, String)>) {
    let Ok(entries) = host::read_dir(dir) else { return };
    for entry in entries {
        let path = format!("{dir}/{}", entry.name);
        if entry.is_dir {
            if depth < 3 && !size::skipped_folder(&entry.name) {
                walk(&path, depth + 1, found);
            }
        } else if entry.size > 0 && is_binary(&path) {
            found.push((entry.modified, path));
        }
    }
}

fn show() -> Result<(), String> {
    let found = candidates();
    let pick = with(|s| {
        s.candidates = found;
        s.active.clone().or_else(|| s.chosen.clone()).or_else(|| s.candidates.first().cloned())
    });
    match pick {
        Some(path) => analyze(path),
        None => {
            render();
            Ok(())
        }
    }
}

/// Starts reading a file; the steps go on as `resumed` events.
fn analyze(path: String) -> Result<(), String> {
    let path = path.trim().trim_start_matches("./").replace('\\', "/");
    with(|s| {
        if !s.candidates.contains(&path) {
            s.candidates.insert(0, path.clone());
        }
        // The report being replaced becomes the one to compare with.
        if let Some(Ok(old)) = &s.report
            && old.path == path
        {
            s.earlier.insert(path.clone(), old.summary());
        }
        s.chosen = Some(path.clone());
        s.report = None;
        s.running = None;
    });
    let started = host::file_size(&path).and_then(|len| {
        let cache = Pages::new(HostFile { path: path.clone(), at: 0, len }, len);
        let scan = Scan::start(&path, &cache, len)?;
        Ok(Running { path: path.clone(), cache, scan })
    });
    match started {
        Ok(running) => {
            with(|s| s.running = Some(running));
            host::continue_later("scan");
        }
        Err(e) => with(|s| s.report = Some(Err(e))),
    }
    render();
    Ok(())
}

/// A step of reading: symbols for up to `STEP_MS`, then the report when all are read.
fn step() {
    let start = host::now().unix_millis;
    let done = with(|s| {
        // Nothing being read (it was replaced): nothing more to do.
        let Some(r) = s.running.as_mut() else { return Some(Ok(())) };
        loop {
            match r.scan.step(&r.cache, BATCH) {
                Ok(true) => break Some(Ok(())),
                Ok(false) if host::now().unix_millis - start >= STEP_MS => break None,
                Ok(false) => {}
                Err(e) => break Some(Err(e)),
            }
        }
    });
    match done {
        None => host::continue_later("scan"),
        Some(result) => {
            let elf = with(|s| {
                let r = s.running.take()?;
                let report = result.map(|()| r.scan.finish());
                let elf = report.as_ref().ok().filter(|r| !r.placed.is_empty()).map(|r| r.path.clone());
                s.report = Some(report);
                elf
            });
            // Its memory layout, unless one was chosen already.
            if let Some(path) = elf
                && with(|s| s.layout.is_none())
            {
                let found = find_layout(&path);
                with(|s| s.layout = found);
            }
        }
    }
    render();
}

fn span(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: false }
}

fn mono(text: impl Into<String>, color: Option<u32>) -> Span {
    Span { text: text.into(), color, bold: false, monospace: true }
}

fn bold(text: impl Into<String>) -> Span {
    Span { text: text.into(), color: None, bold: true, monospace: false }
}

fn button(label: &str, command: &str, argument: &str) -> Button {
    Button { label: label.to_owned(), command: command.to_owned(), argument: argument.to_owned() }
}

fn field(id: &str, label: &str, kind: FieldKind, value: &str, options: Vec<String>, hint: &str) -> FormField {
    FormField { id: id.into(), label: label.into(), kind, value: value.into(), options, hint: hint.into() }
}

/// A bar of `part` out of `whole`, in block characters.
fn bar(part: u64, whole: u64) -> Span {
    const WIDTH: usize = 12;
    const EIGHTHS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    let fill = (part as f64 / whole.max(1) as f64).clamp(0.0, 1.0);
    let eighths = ((fill * (WIDTH * 8) as f64).round() as usize).max(usize::from(part > 0));
    let mut text = "█".repeat(eighths / 8);
    if eighths % 8 > 0 {
        text.push(EIGHTHS[eighths % 8]);
    }
    let used = text.chars().count();
    text.push_str(&" ".repeat(WIDTH - used));
    mono(text, Some(ACCENT))
}

fn percent(n: u64, whole: u64) -> String {
    format!("{:.1}%", n as f64 * 100.0 / whole.max(1) as f64)
}

/// Where a firmware's memory layout may be: a GNU map beside the program (`app.map`,
/// `app.elf.map`), then `memory.x` or a linker script (`*.ld`) in the project, near the top.
fn find_layout(program: &str) -> Option<(String, Vec<memory::Region>)> {
    let read = |path: &str| -> Option<(String, Vec<memory::Region>)> {
        let regions = memory::regions_from(&host::read_file(path).ok()?);
        (!regions.is_empty()).then(|| (path.to_owned(), regions))
    };
    let stem = program.rsplit_once('.').map_or(program, |(s, _)| s);
    for map in [format!("{program}.map"), format!("{stem}.map")] {
        if let Some(found) = read(&map) {
            return Some(found);
        }
    }
    fn walk(dir: &str, depth: usize, out: &mut Vec<String>) {
        let Ok(entries) = host::read_dir(dir) else { return };
        let path = |name: &str| if dir.is_empty() { name.to_owned() } else { format!("{dir}/{name}") };
        for e in &entries {
            let lower = e.name.to_ascii_lowercase();
            if !e.is_dir && (lower == "memory.x" || lower.ends_with(".ld")) {
                out.push(path(&e.name));
            }
        }
        for e in entries.iter().filter(|e| e.is_dir) {
            let skip = e.name.starts_with('.') || matches!(e.name.as_str(), "target" | "node_modules" | "build");
            if depth < 2 && !skip {
                walk(&path(&e.name), depth + 1, out);
            }
        }
    }
    let mut scripts = Vec::new();
    walk("", 0, &mut scripts);
    // memory.x first (the usual one in Rust), the shallowest first.
    scripts.sort_by_key(|p| (!p.to_ascii_lowercase().ends_with("memory.x"), p.matches('/').count()));
    scripts.iter().find_map(|p| read(p))
}

fn hex(n: u64) -> String {
    format!("0x{n:08X}")
}

/// The Memory tab: each region with how full it is and what's in it, then the sections with
/// where they run and are loaded from.
fn memory_ui(s: &State, report: &Report, b: &mut Vec<Block>) {
    let (from, regions) = s.layout.clone().unwrap_or_default();
    let fields = vec![field(
        "layout",
        "Memory layout",
        FieldKind::Text,
        &from,
        Vec::new(),
        "memory.x, a linker script or a GNU map",
    )];
    b.push(Block::Form(Form { id: "layout".into(), fields, buttons: vec!["Use".into()] }));
    if regions.is_empty() {
        let why = "No memory regions found: give a linker script with MEMORY (memory.x), or a map GNU ld wrote (-Map).";
        b.push(Block::Spans(vec![span(why, Some(DIM))]));
    } else {
        let (uses, outside) = memory::place(&regions, &report.placed);
        for u in &uses {
            let r = &u.region;
            let full = if u.used > r.length { Some(RED) } else { None };
            b.push(Block::Rows(vec![Row {
                cells: vec![
                    bold(&r.name),
                    span(format!("{} of {}", human(u.used), human(r.length)), full),
                    bar(u.used, r.length),
                    span(percent(u.used, r.length), full.or(Some(DIM))),
                    mono(format!("{} – {}", hex(r.origin), hex(r.origin + r.length)), Some(DIM)),
                ],
                buttons: Vec::new(),
            }]));
            if u.sections.is_empty() {
                continue;
            }
            let rows = u
                .sections
                .iter()
                .map(|(name, size, how)| {
                    let what = match how {
                        memory::How::Runs => "runs here".to_owned(),
                        memory::How::Loaded => "start values, copied out at start".to_owned(),
                    };
                    Row {
                        cells: vec![
                            mono(format!("  {name}"), None),
                            span(human(*size), None),
                            bar(*size, r.length),
                            span(what, Some(DIM)),
                        ],
                        buttons: Vec::new(),
                    }
                })
                .collect();
            b.push(Block::Rows(rows));
            // The biggest symbols that run in this region.
            let here: Vec<&str> =
                u.sections.iter().filter(|(_, _, how)| *how == memory::How::Runs).map(|(n, _, _)| n.as_str()).collect();
            let biggest: Vec<String> = report
                .symbols
                .iter()
                .filter(|x| here.contains(&x.section.as_str()))
                .take(5)
                .map(|x| format!("{} {}", x.name, human(x.size)))
                .collect();
            if !biggest.is_empty() {
                b.push(Block::Spans(vec![span(format!("  Biggest: {}", biggest.join(", ")), Some(DIM))]));
            }
        }
        if !outside.is_empty() {
            let names: Vec<String> = outside.iter().map(|p| format!("{} ({})", p.name, hex(p.vma))).collect();
            b.push(Block::Spans(vec![span(format!("Outside every region: {}", names.join(", ")), Some(ORANGE))]));
        }
    }
    b.push(Block::Separator);
    let rows = report
        .placed
        .iter()
        .filter(|p| p.size > 0)
        .map(|p| {
            let at = |a: u64| regions.iter().find(|r| a >= r.origin && a - r.origin < r.length).map(|r| r.name.clone());
            let runs = at(p.vma).map_or_else(|| hex(p.vma), |r| format!("{} {r}", hex(p.vma)));
            let loads = if p.lma != p.vma && p.contents {
                at(p.lma).map_or_else(|| hex(p.lma), |r| format!("{} {r}", hex(p.lma)))
            } else {
                "".to_owned()
            };
            vec![
                p.name.clone(),
                runs,
                loads,
                human(p.size),
                (if p.contents { "contents" } else { "zero-filled" }).to_owned(),
            ]
        })
        .collect();
    b.push(Block::Table(Grid {
        headers: ["Section", "Runs at", "Loaded from", "Size", "Kind"].map(str::to_owned).to_vec(),
        rows,
    }));
}

fn render() {
    let blocks = with(|s| {
        let mut b = Vec::new();
        file_ui(s, &mut b);
        if let Some(r) = &s.running {
            b.push(Block::Spans(vec![span(
                format!("Reading {}… {} symbols looked at", r.path, r.scan.progress()),
                Some(DIM),
            )]));
            return b;
        }
        match &s.report {
            None => {
                let text = "Build the project, or give a program, a library or an object file in this folder.";
                b.push(Block::Spans(vec![span(text, Some(DIM))]));
            }
            Some(Err(e)) => b.push(Block::Spans(vec![span(e.clone(), Some(RED))])),
            Some(Ok(report)) => report_ui(s, report, &mut b),
        }
        b
    });
    host::set_panel("binary-size", &blocks);
}

fn file_ui(s: &State, b: &mut Vec<Block>) {
    let chosen = s.chosen.clone().unwrap_or_default();
    let mut fields = Vec::new();
    if !s.candidates.is_empty() {
        let value = if s.candidates.contains(&chosen) { chosen.clone() } else { s.candidates[0].clone() };
        fields.push(field("built", "Built here", FieldKind::Choice, &value, s.candidates.clone(), ""));
    }
    let typed = if s.candidates.contains(&chosen) { String::new() } else { chosen };
    fields.push(field("path", "", FieldKind::Text, &typed, Vec::new(), "or a path in this folder: firmware/app.elf"));
    let mut buttons = vec!["Analyze".to_owned()];
    if s.chosen.is_some() {
        buttons.push("Analyze again".to_owned());
    }
    b.push(Block::Form(Form { id: "file".into(), fields, buttons }));
}

fn report_ui(s: &State, report: &Report, b: &mut Vec<Block>) {
    let earlier = s.earlier.get(&report.path);
    // The summary: what it is, and its size.
    let mut line = vec![
        bold(&report.format),
        span(format!(" · file {}", human(report.file_size)), Some(DIM)),
        span(format!(" · loaded {}", human(report.loaded())), Some(DIM)),
    ];
    if let Some(m) = report.memory {
        line.push(bold(format!(" · flash {} · RAM {}", human(m.flash), human(m.ram))));
    }
    if let Some(old) = earlier {
        let (a, now) = (old.loaded, report.loaded());
        let color = if now > a { RED } else { GREEN };
        line.push(span(format!(" · {} since last time", human_change(a, now)), Some(color)));
    }
    b.push(Block::Spans(line));
    let parts = report.parts();
    let total: u64 = parts.iter().filter(|(p, _)| p.loaded()).map(|(_, n)| *n).sum::<u64>().max(1);
    let listed: Vec<String> = parts.iter().map(|(p, n)| format!("{} {}", p.label(), human(*n))).collect();
    b.push(Block::Spans(vec![span(listed.join("   "), Some(DIM))]));
    if report.memory.is_some() {
        let why = "Flash: code, read-only data and data's start values. RAM: data and zero-filled data.";
        b.push(Block::Spans(vec![span(why, Some(DIM))]));
    }
    if let Some(why) = report.no_symbols {
        b.push(Block::Spans(vec![span(why, Some(ORANGE))]));
    }
    let groups = report.groups();
    let tabs = [
        (Tab::Groups, "groups", format!("Groups {}", groups.len())),
        (Tab::Symbols, "symbols", format!("Symbols {}", report.symbols.len())),
        (Tab::Sections, "sections", format!("Sections {}", report.sections.len())),
        (Tab::Memory, "memory", "Memory".to_owned()),
    ];
    // The memory map is for a program whose sections sit at addresses (ELF).
    let buttons = tabs
        .iter()
        .filter(|(tab, ..)| *tab != Tab::Memory || !report.placed.is_empty())
        .map(|(tab, id, label)| {
            let label = if *tab == s.tab { format!("● {label}") } else { label.clone() };
            button(&label, "tab", id)
        })
        .collect();
    b.push(Block::Rows(vec![Row { cells: Vec::new(), buttons }]));
    b.push(Block::Separator);
    match s.tab {
        Tab::Groups => {
            let symbols: u64 = groups.iter().map(|g| g.1).sum();
            let changes: HashMap<String, (u64, u64)> = earlier
                .map(|old| size::group_changes(old, report).into_iter().map(|(g, a, n)| (g, (a, n))).collect())
                .unwrap_or_default();
            let mut rows: Vec<Row> = groups
                .iter()
                .map(|(group, n, count)| {
                    let what = if *count == 1 { "symbol" } else { "symbols" };
                    let change = match changes.get(group) {
                        Some((a, now)) => span(human_change(*a, *now), Some(if now > a { RED } else { GREEN })),
                        None => span("", None),
                    };
                    Row {
                        cells: vec![
                            mono(group, None),
                            span(human(*n), None),
                            bar(*n, symbols),
                            span(format!("{} · {count} {what}", percent(*n, symbols)), Some(DIM)),
                            change,
                        ],
                        buttons: vec![button("Symbols", "group", group)],
                    }
                })
                .collect();
            // Groups that went away entirely.
            for (group, (a, now)) in &changes {
                if *now == 0 {
                    rows.push(Row {
                        cells: vec![
                            mono(group, Some(DIM)),
                            span("gone", Some(GREEN)),
                            span("", None),
                            span("", None),
                            span(human_change(*a, 0), Some(GREEN)),
                        ],
                        buttons: Vec::new(),
                    });
                }
            }
            b.push(Block::Rows(rows));
        }
        Tab::Symbols => {
            let fields = vec![field("filter", "", FieldKind::Text, &s.filter, Vec::new(), "Filter by name or group")];
            b.push(Block::Form(Form { id: "filter".into(), fields, buttons: vec!["Filter".into(), "Clear".into()] }));
            let needle = s.filter.to_lowercase();
            let matching: Vec<_> = report
                .symbols
                .iter()
                .filter(|x| {
                    needle.is_empty() || x.name.to_lowercase().contains(&needle) || x.group.to_lowercase() == needle
                })
                .collect();
            let biggest = matching.first().map_or(1, |x| x.size);
            let rows = matching
                .iter()
                .take(SHOWN)
                .map(|x| {
                    let mut name = x.name.clone();
                    if name.chars().count() > 110 {
                        name = name.chars().take(109).collect::<String>() + "…";
                    }
                    let kind = if x.code { "code" } else { "data" };
                    Row {
                        cells: vec![
                            span(human(x.size), None),
                            bar(x.size, biggest),
                            mono(name, None),
                            span(format!("{} · {kind}", x.section), Some(DIM)),
                        ],
                        buttons: Vec::new(),
                    }
                })
                .collect();
            b.push(Block::Rows(rows));
            if matching.is_empty() {
                b.push(Block::Spans(vec![span("No symbol matches.", Some(DIM))]));
            }
            if matching.len() > SHOWN {
                let more = format!("…and {} smaller ones; the filter finds them.", matching.len() - SHOWN);
                b.push(Block::Spans(vec![span(more, Some(DIM))]));
            }
        }
        Tab::Memory => memory_ui(s, report, b),
        Tab::Sections => {
            let rows = report
                .sections
                .iter()
                .map(|x| {
                    let (whole, share) = if x.part.loaded() {
                        (total, percent(x.size, total))
                    } else {
                        (report.file_size, "not loaded".to_owned())
                    };
                    Row {
                        cells: vec![
                            mono(&x.name, None),
                            span(human(x.size), None),
                            bar(x.size, whole),
                            span(x.part.label(), Some(DIM)),
                            span(share, Some(DIM)),
                        ],
                        buttons: Vec::new(),
                    }
                })
                .collect();
            b.push(Block::Rows(rows));
        }
    }
}

fn command(id: &str, arg: &str) -> Result<(), String> {
    match id {
        "show" => return show(),
        "tab" => with(|s| {
            s.tab = match arg {
                "symbols" => Tab::Symbols,
                "sections" => Tab::Sections,
                "memory" => Tab::Memory,
                _ => Tab::Groups,
            }
        }),
        "group" => with(|s| {
            s.filter = arg.to_owned();
            s.tab = Tab::Symbols;
        }),
        _ => {}
    }
    render();
    Ok(())
}

fn submitted(f: FormValues) {
    let value = |id: &str| f.values.iter().find(|(k, _)| k == id).map(|(_, v)| v.trim().to_owned()).unwrap_or_default();
    match f.form.as_str() {
        "file" => {
            let typed = value("path");
            let path = if f.button == "Analyze again" {
                with(|s| s.chosen.clone()).unwrap_or_default()
            } else if typed.is_empty() {
                value("built")
            } else {
                typed
            };
            if path.is_empty() {
                host::show_message("Give a program, a library or an object file in this folder");
                return;
            }
            let _ = analyze(path);
        }
        "filter" => {
            with(|s| s.filter = if f.button == "Clear" { String::new() } else { value("filter") });
            render();
        }
        "layout" => {
            let path = value("layout").replace('\\', "/");
            match host::read_file(&path) {
                Ok(text) => {
                    let regions = memory::regions_from(&text);
                    if regions.is_empty() {
                        host::show_message(&format!("{path} declares no memory regions (MEMORY {{ … }})"));
                    }
                    with(|s| s.layout = Some((path, regions)));
                }
                Err(e) => host::show_message(&e),
            }
            render();
        }
        _ => {}
    }
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
            Event::Resumed(token) if token == "scan" => step(),
            Event::FormSubmitted(f) => submitted(f),
            Event::ActiveChanged(path) => {
                let binary = !path.is_empty() && is_binary(&path);
                with(|s| s.active = binary.then_some(path));
            }
            _ => {}
        }
    }
}

export!(Plugin);
