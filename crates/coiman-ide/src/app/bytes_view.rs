//! Bytes and instructions: binary files, and a paused program's memory and
//! disassembly when its debug adapter says it can give them.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use coiman_core::data::hexdata::{self, BYTES_PER_ROW, Instruction, MemoryBlock, PAGE};
use eframe::egui::{self, Color32, RichText};

use super::CodeApp;
use crate::grid_view::{self, GridSource, GridState};

/// Rows of memory reachable from the address asked for, before asking for another.
const MEMORY_ROWS: u64 = 65_536;

#[derive(Clone, PartialEq)]
enum Showing {
    File(PathBuf),
    Memory,
    Disassembly,
}

/// A page, as far as it's known.
#[derive(Clone, Default)]
struct Page {
    bytes: Vec<u8>,
    /// Bytes after these couldn't be read.
    unreadable: bool,
}

pub(super) struct BytesView {
    showing: Showing,
    open: bool,
    over: Option<u64>,
    /// The address or offset of row 0.
    base: u64,
    address_text: String,
    address_problem: Option<String>,
    pages: HashMap<u64, Page>,
    /// The same pages at the previous stop, to show what changed.
    before: HashMap<u64, Page>,
    asked: HashSet<u64>,
    file_size: u64,
    grid: GridState,
    instructions: Vec<Instruction>,
    pointer: Option<u64>,
    /// The stop and frame what's shown belongs to.
    seen: Option<(u64, usize)>,
    grid_for: Option<Showing>,
    /// The file as last read, to notice it changing.
    file_stamp: Option<(u64, Option<std::time::SystemTime>)>,
    find_text: String,
    /// What was asked for, and the answer on its way.
    finding: Option<std::sync::mpsc::Receiver<Option<u64>>>,
    found_at: Option<u64>,
}

impl BytesView {
    fn new(showing: Showing, over: Option<u64>) -> Self {
        Self {
            showing,
            open: true,
            over,
            base: 0,
            address_text: String::new(),
            address_problem: None,
            pages: HashMap::new(),
            before: HashMap::new(),
            asked: HashSet::new(),
            file_size: 0,
            grid: GridState::default(),
            instructions: Vec::new(),
            pointer: None,
            seen: None,
            grid_for: None,
            file_stamp: None,
            find_text: String::new(),
            finding: None,
            found_at: None,
        }
    }
}

#[derive(Default)]
pub(super) struct BytesState {
    view: Option<BytesView>,
    to_open: Option<PathBuf>,
}

/// A byte, if known: `None` when it's still on its way, or unreadable.
enum Cell {
    Waiting,
    Unreadable,
    Beyond,
    Byte(u8),
}

struct HexSource<'a> {
    view: &'a BytesView,
    rows: u64,
    wide: bool,
    missing: RefCell<HashSet<u64>>,
}

impl HexSource<'_> {
    fn cell(&self, address: u64) -> Cell {
        let start = address - address % PAGE;
        let Some(page) = self.view.pages.get(&start) else {
            self.missing.borrow_mut().insert(start);
            return Cell::Waiting;
        };
        match page.bytes.get((address - start) as usize) {
            Some(byte) => Cell::Byte(*byte),
            None if page.unreadable => Cell::Unreadable,
            None => Cell::Beyond,
        }
    }

    fn address(&self, row: usize, column: usize) -> u64 {
        self.view.base.wrapping_add(row as u64 * BYTES_PER_ROW).wrapping_add(column as u64)
    }
}

impl GridSource for HexSource<'_> {
    fn rows(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        BYTES_PER_ROW as usize + 1
    }

    fn header(&self, column: usize) -> String {
        if column == BYTES_PER_ROW as usize { "Text".to_owned() } else { format!("{column:02X}") }
    }

    fn row(&self, row: usize) -> Vec<String> {
        let mut cells = Vec::with_capacity(self.columns());
        let mut text = String::new();
        for column in 0..BYTES_PER_ROW as usize {
            match self.cell(self.address(row, column)) {
                Cell::Byte(byte) => {
                    cells.push(format!("{byte:02X}"));
                    text.push(hexdata::printable(byte));
                }
                Cell::Unreadable => {
                    cells.push("??".to_owned());
                    text.push(' ');
                }
                Cell::Waiting => {
                    cells.push("··".to_owned());
                    text.push(' ');
                }
                Cell::Beyond => {
                    cells.push(String::new());
                    text.push(' ');
                }
            }
        }
        cells.push(text);
        cells
    }

    fn row_label(&self, row: usize) -> String {
        let address = self.address(row, 0);
        if self.wide { format!("{address:016X}") } else { format!("{address:08X}") }
    }

    fn cell_color(&self, row: usize, column: usize) -> Option<Color32> {
        if column >= BYTES_PER_ROW as usize {
            return None;
        }
        let address = self.address(row, column);
        let start = address - address % PAGE;
        let at = (address - start) as usize;
        let now = self.view.pages.get(&start)?.bytes.get(at)?;
        let was = self.view.before.get(&start)?.bytes.get(at)?;
        (now != was).then_some(Color32::from_rgb(0xF1, 0x4C, 0x4C))
    }
}

struct CodeSource<'a> {
    list: &'a [Instruction],
    pointer: Option<u64>,
}

impl GridSource for CodeSource<'_> {
    fn rows(&self) -> usize {
        self.list.len()
    }

    fn columns(&self) -> usize {
        4
    }

    fn header(&self, column: usize) -> String {
        ["Instruction", "Bytes", "Symbol", "Source"][column.min(3)].to_owned()
    }

    fn row(&self, row: usize) -> Vec<String> {
        let Some(i) = self.list.get(row) else { return Vec::new() };
        let file = i.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        let source = match (file, i.line) {
            (Some(file), Some(line)) => format!("{file}:{line}"),
            _ => String::new(),
        };
        vec![i.text.clone(), i.bytes.clone(), i.symbol.clone(), source]
    }

    fn row_label(&self, row: usize) -> String {
        self.list.get(row).map_or_else(String::new, |i| i.address_text.clone())
    }

    fn cell_color(&self, row: usize, _column: usize) -> Option<Color32> {
        let at = self.list.get(row)?.address;
        (Some(at) == self.pointer).then_some(Color32::from_rgb(0xFF, 0xD7, 0x00))
    }
}

impl CodeApp {
    pub(super) fn bytes_tab(&self) -> Option<(String, bool)> {
        let view = self.bytes.view.as_ref()?;
        let title = match &view.showing {
            Showing::File(path) => path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            Showing::Memory => "Memory".to_owned(),
            Showing::Disassembly => "Disassembly".to_owned(),
        };
        Some((title, view.open))
    }

    pub(super) fn hide_bytes(&mut self) {
        if let Some(view) = &mut self.bytes.view {
            view.open = false;
        }
    }

    pub(super) fn show_bytes_tab(&mut self) {
        let over = self.docs.get(self.active).map(|d| d.uid);
        if let Some(view) = &mut self.bytes.view {
            view.open = true;
            view.over = over;
        }
    }

    pub(super) fn close_bytes_tab(&mut self) {
        self.bytes.view = None;
    }

    /// A binary file asks for this view when it's opened.
    pub(super) fn open_binary_file(&mut self, path: &Path) -> bool {
        if !hexdata::looks_binary(path) {
            return false;
        }
        self.bytes.to_open = Some(path.to_path_buf());
        true
    }

    /// `Hex: Open File`: the active file, or one to choose.
    pub(super) fn hex_command(&mut self) {
        let current = self.docs.get(self.active).and_then(|d| d.path.clone());
        let picked = || self.file_dialog("hex_file").pick_file();
        if let Some(path) = current.or_else(picked) {
            self.dialog_chose("hex_file", &path);
            self.bytes.to_open = Some(path);
        }
    }

    pub(super) fn show_memory(&mut self, address: Option<String>) {
        if !self.debug_is_paused() {
            self.status = Some("Pause the program to read its memory".to_owned());
            return;
        }
        if !self.debug_capabilities().read_memory {
            self.status = Some("This debugger doesn't offer reading memory".to_owned());
            return;
        }
        let over = self.docs.get(self.active).map(|d| d.uid);
        let keep = self.bytes.view.as_ref().filter(|v| matches!(v.showing, Showing::Memory | Showing::Disassembly));
        let text = address.or_else(|| keep.map(|v| v.address_text.clone())).unwrap_or_default();
        let mut view = BytesView::new(Showing::Memory, over);
        view.address_text = text.clone();
        self.bytes.view = Some(view);
        if !text.is_empty() {
            self.go_to_address(&text);
        }
    }

    pub(super) fn show_disassembly(&mut self) {
        if !self.debug_is_paused() {
            self.status = Some("Pause the program to see its instructions".to_owned());
            return;
        }
        if !self.debug_capabilities().disassemble {
            self.status = Some("This debugger doesn't offer disassembly".to_owned());
            return;
        }
        let over = self.docs.get(self.active).map(|d| d.uid);
        self.bytes.view = Some(BytesView::new(Showing::Disassembly, over));
    }

    /// Goes to a typed address, or asks the debugger what an expression's address is.
    fn go_to_address(&mut self, text: &str) {
        let Some(view) = &mut self.bytes.view else { return };
        view.address_problem = None;
        let is_memory = view.showing == Showing::Memory;
        match hexdata::parse_address(text) {
            Some(address) => {
                let row = address / BYTES_PER_ROW;
                view.base = if is_memory { address - address % BYTES_PER_ROW } else { 0 };
                view.pages.clear();
                view.asked.clear();
                view.grid.selected = None;
                view.grid.reveal = Some(if is_memory { 0 } else { row as usize });
            }
            None if is_memory => self.debug_evaluate_address(text),
            None => view.address_problem = Some("Type an offset in hex".to_owned()),
        }
    }

    pub(super) fn on_memory_address(&mut self, reference: Option<String>, why: Option<String>) {
        match (reference, &mut self.bytes.view) {
            (Some(reference), Some(_)) => self.go_to_address(&reference),
            (None, Some(view)) => {
                view.address_problem = Some(why.unwrap_or_else(|| "That has no address".to_owned()));
            }
            _ => {}
        }
    }

    pub(super) fn on_memory_page(&mut self, start: u64, block: Option<MemoryBlock>) {
        let Some(view) = &mut self.bytes.view else { return };
        let page = match block {
            Some(block) => {
                // Bytes the adapter skipped at the start are unreadable too: none can be shown.
                if block.address != start && block.bytes.is_empty() {
                    Page { bytes: Vec::new(), unreadable: true }
                } else {
                    Page { unreadable: (block.bytes.len() as u64) < PAGE, bytes: block.bytes }
                }
            }
            None => Page { bytes: Vec::new(), unreadable: true },
        };
        view.pages.insert(start, page);
    }

    pub(super) fn on_disassembly(&mut self, list: Vec<Instruction>) {
        let pointer = self.debug_pointer();
        let Some(view) = &mut self.bytes.view else { return };
        view.pointer = pointer;
        if let Some(row) = list.iter().position(|i| Some(i.address) == pointer) {
            view.grid.selected = Some((row, 0));
            view.grid.reveal = Some(row);
        }
        view.instructions = list;
    }

    /// Shows the view in the editor's place. Says whether it did.
    pub(super) fn bytes_view(&mut self, ui: &mut egui::Ui) -> bool {
        let over_now = self.docs.get(self.active).map(|d| d.uid);
        if let Some(path) = self.bytes.to_open.take() {
            self.hide_logs();
            self.hide_image();
            self.bytes.view = Some(BytesView::new(Showing::File(path), over_now));
        }
        let paused = self.debug_is_paused();
        let stop = self.debug_stop_key();
        let capabilities = self.debug_capabilities();
        let frames_ready = self.debug_frames_ready();
        let font_size = self.font_size;
        let Some(view) = &mut self.bytes.view else { return false };
        if view.open && view.over != over_now {
            view.open = false;
        }
        if !view.open {
            return false;
        }

        // A new stop: what's in memory may have changed, and what changed shows.
        let debugging = !matches!(view.showing, Showing::File(_));
        let mut ask_code = false;
        if debugging && view.seen != Some(stop) {
            let same_stop = view.seen.is_some_and(|s| s.0 == stop.0);
            if !same_stop {
                view.before = std::mem::take(&mut view.pages);
                view.asked.clear();
            }
            // Instructions start from the frame, which arrives after the stop does.
            if view.showing != Showing::Disassembly || frames_ready {
                view.seen = Some(stop);
                ask_code = view.showing == Showing::Disassembly;
            } else {
                view.seen = Some((stop.0, usize::MAX));
            }
        }
        if ask_code {
            self.debug_request_disassembly();
        }
        let Some(view) = &mut self.bytes.view else { return false };

        if let Some(found) = view.finding.as_ref().and_then(|rx| rx.try_recv().ok()) {
            view.finding = None;
            match found {
                Some(at) => {
                    let row = (at / BYTES_PER_ROW) as usize;
                    view.found_at = Some(at);
                    view.grid.selected = Some((row, (at % BYTES_PER_ROW) as usize));
                    view.grid.reveal = Some(row);
                    view.address_problem = Some(format!("Found at 0x{at:x}"));
                }
                None => {
                    view.found_at = None;
                    view.address_problem = Some("Not found after here".to_owned());
                }
            }
        }
        let mut find = false;
        let mut close = false;
        let mut go = None;
        let mut switch = None;
        let mut step = None;
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if let Showing::File(path) = &view.showing {
                let title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(RichText::new(title).strong());
            }
            if debugging
                && capabilities.read_memory
                && ui.selectable_label(view.showing == Showing::Memory, "Memory").clicked()
            {
                switch = Some(Showing::Memory);
            }
            if debugging
                && capabilities.disassemble
                && ui.selectable_label(view.showing == Showing::Disassembly, "Disassembly").clicked()
            {
                switch = Some(Showing::Disassembly);
            }
            if view.showing != Showing::Disassembly {
                let hint = if debugging { "Address or expression" } else { "Offset" };
                let edit = egui::TextEdit::singleline(&mut view.address_text).hint_text(hint).desired_width(220.0);
                let response = ui.add(edit);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = Some(view.address_text.clone());
                }
                if !debugging {
                    let edit = egui::TextEdit::singleline(&mut view.find_text)
                        .hint_text("Find: hex bytes or text")
                        .desired_width(200.0);
                    let response = ui.add(edit);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        find = true;
                        response.request_focus();
                    }
                }
            } else if capabilities.instruction_stepping {
                if ui.button("Step Instruction").clicked() {
                    step = Some("next");
                }
                if ui.button("Step Into Instruction").clicked() {
                    step = Some("stepIn");
                }
            }
            close = ui.button("Close").clicked();
        });
        if let (true, Showing::File(path)) = (find, &view.showing) {
            let (path, needle) = (path.clone(), hexdata::parse_search(&view.find_text));
            let start = view.found_at.map_or(0, |at| at + 1);
            let (tx, rx) = std::sync::mpsc::channel();
            let wake = super::repaint(ui.ctx());
            std::thread::spawn(move || {
                let _ = tx.send(hexdata::find_in_file(&path, &needle, start));
                wake();
            });
            view.finding = Some(rx);
            view.address_problem = None;
        }
        let warn = Color32::from_rgb(0xCC, 0xA7, 0x00);
        if let Some(problem) = &view.address_problem {
            ui.label(RichText::new(format!("  {problem}")).color(warn));
        }
        if debugging && !paused {
            ui.label(RichText::new("  The program isn't paused: this is how it last stood.").color(warn));
        }

        let mut missing = HashSet::new();
        let mut open_source = None;
        match view.showing.clone() {
            Showing::Disassembly => {
                if view.grid_for.as_ref() != Some(&Showing::Disassembly) {
                    let w = font_size * 0.65;
                    view.grid.widths = vec![w * 44.0, w * 24.0, w * 28.0, w * 18.0];
                    view.grid_for = Some(Showing::Disassembly);
                }
                let source = CodeSource { list: &view.instructions, pointer: view.pointer };
                let id = ui.scope_id().with("disassembly");
                let events = grid_view::show(ui, id, &mut view.grid, &source, font_size);
                if let Some(row) = events.row_opened
                    && let Some(i) = view.instructions.get(row)
                    && let (Some(path), Some(line)) = (i.path.clone(), i.line)
                {
                    open_source = Some((path, line));
                }
            }
            shown => {
                if let Showing::File(path) = &shown {
                    let info = std::fs::metadata(path).ok();
                    view.file_size = info.as_ref().map_or(0, |m| m.len());
                    let stamp = (view.file_size, info.and_then(|m| m.modified().ok()));
                    // It changed on disk: what was read of it no longer holds.
                    if view.file_stamp != Some(stamp) {
                        view.file_stamp = Some(stamp);
                        view.pages.clear();
                        view.found_at = None;
                        view.address_problem = None;
                    }
                }
                if view.grid_for.as_ref() != Some(&shown) {
                    let w = font_size * 0.65;
                    let mut widths = vec![w * 2.0 + 10.0; BYTES_PER_ROW as usize];
                    widths.push(w * 16.0 + 12.0);
                    view.grid.widths = widths;
                    view.grid_for = Some(shown.clone());
                }
                let is_file = matches!(shown, Showing::File(_));
                let rows = if is_file { view.file_size.div_ceil(BYTES_PER_ROW) } else { MEMORY_ROWS };
                let mut grid = std::mem::take(&mut view.grid);
                let source = HexSource { view, rows, wide: !is_file, missing: RefCell::new(HashSet::new()) };
                let id = ui.scope_id().with("hex");
                grid_view::show(ui, id, &mut grid, &source, font_size);
                missing = source.missing.into_inner();
                view.grid = grid;
            }
        }
        // Files are near, and read here. A debugger answers when it answers.
        let file = match &view.showing {
            Showing::File(path) => Some(path.clone()),
            _ => None,
        };
        let mut to_ask = Vec::new();
        for start in missing {
            if let Some(path) = &file {
                let bytes = hexdata::read_page(path, start).unwrap_or_default();
                view.pages.insert(start, Page { bytes, unreadable: false });
            } else if view.asked.insert(start) {
                to_ask.push(start);
            }
        }
        if close {
            self.bytes.view = None;
            return true;
        }
        for start in to_ask {
            self.debug_request_memory(start);
        }
        if let Some(text) = go {
            self.go_to_address(&text);
        }
        if let Some(command) = step {
            self.debug_step_instruction(command);
        }
        match switch {
            Some(Showing::Memory) => self.show_memory(None),
            Some(Showing::Disassembly) => self.show_disassembly(),
            _ => {}
        }
        if let Some((path, line)) = open_source {
            // It's over the editor: step aside to show the source.
            if let Some(view) = &mut self.bytes.view {
                view.open = false;
            }
            self.open_file(&path);
            if let Some(doc) = self.active_doc() {
                doc.goto_line(line as usize);
            }
        }
        true
    }
}
