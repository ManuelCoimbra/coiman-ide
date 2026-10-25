//! Integrated terminal: tabs of shells running in pseudo-terminals (ConPTY on
//! Windows), drawn as a character grid with colors, cursor, selection and
//! scrollback. The panel can also move into its own window.

use std::path::{Path, PathBuf};

use egui::{Color32, Event, EventFilter, FontId, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2};
use rust_code_core::pty::alacritty_terminal::grid::{Dimensions, Scroll};
use rust_code_core::pty::alacritty_terminal::index::{Column, Line, Point, Side};
use rust_code_core::pty::alacritty_terminal::selection::{Selection, SelectionType};
use rust_code_core::pty::alacritty_terminal::term::TermMode;
use rust_code_core::pty::alacritty_terminal::term::cell::Flags;
use rust_code_core::pty::alacritty_terminal::term::color::Colors;
use rust_code_core::pty::alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use rust_code_core::pty::{Notice, Session, Size, default_shell};

use crate::theme;

const SCROLLBAR: f32 = 10.0;

/// VS Code's default ANSI colors (Dark+): black, red, green, yellow, blue,
/// magenta, cyan, white, then the bright variants.
const ANSI: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xCD, 0x31, 0x31),
    (0x0D, 0xBC, 0x79),
    (0xE5, 0xE5, 0x10),
    (0x24, 0x72, 0xC8),
    (0xBC, 0x3F, 0xBC),
    (0x11, 0xA8, 0xCD),
    (0xE5, 0xE5, 0xE5),
    (0x66, 0x66, 0x66),
    (0xF1, 0x4C, 0x4C),
    (0x23, 0xD1, 0x8B),
    (0xF5, 0xF5, 0x43),
    (0x3B, 0x8E, 0xEA),
    (0xD6, 0x70, 0xD6),
    (0x29, 0xB8, 0xDB),
    (0xE5, 0xE5, 0xE5),
];

struct Tab {
    id: u64,
    session: Session,
    /// Title set by the program (OSC 0/2), if any.
    title: Option<String>,
    /// Scroll wheel movement not yet turned into whole lines.
    scroll_rest: f32,
    /// A run configuration: the tab stays open after the program ends.
    task: bool,
    /// Set when the program ended (with its exit code, if known).
    exited: Option<Option<u32>>,
}

impl Tab {
    /// "1: powershell": a title that is a program path shows just its name.
    fn label(&self, index: usize) -> String {
        let title = self.title.as_deref().unwrap_or(&self.session.name);
        let program = !title.contains(' ') || title.to_lowercase().ends_with(".exe");
        let title = if title.contains(['\\', '/']) && program {
            Path::new(title).file_stem().map_or(title.to_owned(), |s| s.to_string_lossy().into_owned())
        } else {
            title.to_owned()
        };
        let short: String = title.chars().take(24).collect();
        let ellipsis = if short.len() < title.len() { "…" } else { "" };
        let status = match self.exited {
            Some(Some(code)) => format!(" (exit {code})"),
            Some(None) => " (ended)".to_owned(),
            None => String::new(),
        };
        format!("{}: {short}{ellipsis}{status}", index + 1)
    }

    fn focus_id(&self) -> egui::Id {
        egui::Id::unique(("terminal", self.id))
    }
}

/// A program to run in its own terminal tab.
pub struct Task<'a> {
    pub title: &'a str,
    pub program: &'a str,
    pub args: &'a [String],
    pub env: &'a [(String, String)],
    pub cwd: &'a Path,
}

/// What the terminal header asks the app to do.
pub enum Action {
    New,
    Kill,
    Detach,
    Attach,
}

pub struct Terminals {
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    cwd: PathBuf,
    pub focus_requested: bool,
    /// Shown in its own window instead of the bottom panel.
    pub detached: bool,
}

impl Terminals {
    pub fn new(cwd: PathBuf) -> Self {
        Self { tabs: Vec::new(), active: 0, next_id: 1, cwd, focus_requested: false, detached: false }
    }

    /// Folder new terminals start in.
    pub fn set_cwd(&mut self, cwd: &Path) {
        self.cwd = cwd.to_path_buf();
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    /// Starts a new shell (`shell` empty: the platform default) and shows it.
    pub fn new_tab(&mut self, ctx: &egui::Context, shell: &str, scrollback: usize) -> Result<(), String> {
        let shell = if shell.trim().is_empty() { default_shell() } else { shell.to_owned() };
        let ctx = ctx.clone();
        let size = Size { columns: 80, lines: 24 };
        let session = Session::spawn(&shell, &self.cwd, size, scrollback, move || ctx.request_repaint())?;
        self.push(session, None, false);
        Ok(())
    }

    /// Runs a program (a run configuration) in a new tab named after it.
    /// The tab stays open after it ends so its output can be read.
    pub fn run_task(&mut self, ctx: &egui::Context, task: Task, scrollback: usize) -> Result<(), String> {
        let ctx = ctx.clone();
        let size = Size { columns: 80, lines: 24 };
        let session = Session::spawn_args(task.program, task.args, task.cwd, task.env, size, scrollback, move || {
            ctx.request_repaint()
        })?;
        self.push(session, Some(task.title.to_owned()), true);
        Ok(())
    }

    fn push(&mut self, session: Session, title: Option<String>, task: bool) {
        self.tabs.push(Tab { id: self.next_id, session, title, scroll_rest: 0.0, task, exited: None });
        self.next_id += 1;
        self.active = self.tabs.len() - 1;
        self.focus_requested = true;
    }

    pub fn kill_active(&mut self) {
        if self.active < self.tabs.len() {
            self.tabs.remove(self.active);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
        }
    }

    /// Clears the screen and scrollback of the active terminal.
    pub fn clear_active(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            tab.session.term().grid_mut().clear_history();
            // Ctrl+L: the shell redraws its prompt on a clean screen.
            tab.session.write(b"\x0c");
        }
    }

    /// Whether the active terminal has keyboard focus.
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.tabs.get(self.active).is_some_and(|t| ctx.memory(|m| m.has_focus(t.focus_id())))
    }

    /// Handles what the sessions reported. Returns true if the last terminal
    /// just closed (its shell exited).
    pub fn pump(&mut self, ctx: &egui::Context) -> bool {
        let before = self.tabs.len();
        for tab in &mut self.tabs {
            for notice in tab.session.poll() {
                match notice {
                    // A task keeps its configuration's name.
                    Notice::Title(t) if !tab.task => tab.title = Some(t),
                    Notice::ResetTitle if !tab.task => tab.title = None,
                    Notice::Clipboard(text) => ctx.copy_text(text),
                    Notice::Exited(code) => tab.exited = Some(code),
                    _ => {}
                }
            }
        }
        // Like VS Code, a terminal closes when its shell exits.
        let active_id = self.tabs.get(self.active).map(|t| t.id);
        self.tabs.retain(|t| t.task || !t.session.has_exited());
        self.active = active_id.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(0);
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        before > 0 && self.tabs.is_empty()
    }

    /// Terminal tabs and buttons, for the right side of the panel header.
    pub fn header(&mut self, ui: &mut egui::Ui) -> Option<Action> {
        let mut action = None;
        let small = |s: &str| egui::RichText::new(s).small();
        if self.detached {
            if ui.small_button(small("Move to Panel")).on_hover_text("Move the terminal back into the panel").clicked()
            {
                action = Some(Action::Attach);
            }
        } else if ui.small_button(small("Pop Out")).on_hover_text("Move the terminal into its own window").clicked() {
            action = Some(Action::Detach);
        }
        if ui.small_button(small("Kill")).on_hover_text("Kill the active terminal").clicked() {
            action = Some(Action::Kill);
        }
        if crate::icons::button(ui, crate::icons::Icon::Plus, "New Terminal (Ctrl+Shift+`)").clicked() {
            action = Some(Action::New);
        }
        // Right-to-left layout: list tabs last-first so they read left to right.
        for i in (0..self.tabs.len()).rev() {
            let selected = i == self.active;
            if ui.selectable_label(selected, small(&self.tabs[i].label(i))).clicked() {
                self.active = i;
                self.focus_requested = true;
            }
        }
        action
    }

    /// Draws the active terminal in all remaining space and handles its input.
    pub fn body(&mut self, ui: &mut egui::Ui, font_size: f32) {
        let focus_requested = std::mem::take(&mut self.focus_requested);
        let Some(tab) = self.tabs.get_mut(self.active) else {
            ui.label(
                egui::RichText::new("No terminal is running. Press + to start one.").color(theme::colors().text_dim),
            );
            return;
        };
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, Sense::hover());
        let id = tab.focus_id();
        let response = ui.interact(rect, id, Sense::click_and_drag());
        if focus_requested || response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true },
                );
            });
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
        }

        let font = FontId::monospace(font_size);
        let row_h = ui.fonts_mut(|f| f.row_height(&font));
        let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
        let text_rect = Rect::from_min_max(rect.min, Pos2::new(rect.right() - SCROLLBAR, rect.bottom()));
        let columns = ((text_rect.width() / char_w).floor() as usize).max(2);
        let lines = ((text_rect.height() / row_h).floor() as usize).max(1);
        tab.session.resize(Size { columns, lines });

        let grid = Grid { rect: text_rect, char_w, row_h };
        if focused {
            handle_keys(ui, &tab.session);
        }
        handle_mouse(ui, tab, &response, &grid);
        paint(ui, &tab.session, &grid, &font, focused, rect);
    }
}

/// Screen geometry of the character grid.
struct Grid {
    rect: Rect,
    char_w: f32,
    row_h: f32,
}

impl Grid {
    /// Grid cell (and which half of it) under a screen position.
    fn point_at(&self, p: Pos2, display_offset: usize, size: Size) -> (Point, Side) {
        let x = ((p.x - self.rect.left()) / self.char_w).max(0.0);
        let col = (x.floor() as usize).min(size.columns - 1);
        let row = (((p.y - self.rect.top()) / self.row_h).floor().max(0.0) as usize).min(size.lines - 1);
        let side = if x.fract() < 0.5 { Side::Left } else { Side::Right };
        (Point::new(Line(row as i32 - display_offset as i32), Column(col)), side)
    }
}

/// The bytes a key press sends to the program, or `None` if it sends nothing.
fn key_bytes(key: Key, mods: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let ctrl = mods.ctrl || mods.command;
    let modifier = 1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(ctrl);
    let cursor_key = |letter: char| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{letter}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            format!("\x1b[{letter}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 { format!("\x1b[{n};{modifier}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() }
    };
    let bytes = match key {
        Key::Enter => b"\r".to_vec(),
        Key::Backspace if ctrl => b"\x08".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::ArrowUp => cursor_key('A'),
        Key::ArrowDown => cursor_key('B'),
        Key::ArrowRight => cursor_key('C'),
        Key::ArrowLeft => cursor_key('D'),
        Key::Home => cursor_key('H'),
        Key::End => cursor_key('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Space if ctrl => vec![0],
        Key::OpenBracket if ctrl => vec![0x1b],
        Key::Backslash if ctrl => vec![0x1c],
        Key::CloseBracket if ctrl => vec![0x1d],
        _ => {
            // Ctrl+letter: control characters (Ctrl+A = 1 …); Alt+letter: ESC prefix.
            let name = key.name();
            let letter = name.chars().next().filter(|c| name.len() == 1 && c.is_ascii_alphabetic())?;
            if ctrl {
                vec![letter.to_ascii_uppercase() as u8 - b'A' + 1]
            } else if mods.alt {
                let c = if mods.shift { letter.to_ascii_uppercase() } else { letter.to_ascii_lowercase() };
                vec![0x1b, c as u8]
            } else {
                return None;
            }
        }
    };
    // Alt+Enter, Alt+Backspace send ESC first, like xterm.
    if mods.alt && !ctrl && matches!(key, Key::Enter | Key::Backspace) {
        return Some([b"\x1b".as_slice(), &bytes].concat());
    }
    Some(bytes)
}

fn handle_keys(ui: &mut egui::Ui, session: &Session) {
    let events = ui.input(|i| i.events.clone());
    let mode = *session.term().mode();
    let mut sent = false;
    let mut alt_text = false;
    for event in events {
        match event {
            Event::Text(text) if !alt_text => {
                session.write(text.as_bytes());
                sent = true;
            }
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\r").replace('\n', "\r");
                if mode.contains(TermMode::BRACKETED_PASTE) {
                    session.write(format!("\x1b[200~{text}\x1b[201~").as_bytes());
                } else {
                    session.write(text.as_bytes());
                }
                sent = true;
            }
            // Ctrl+C copies when text is selected, otherwise interrupts.
            Event::Copy => {
                let selected = session.term().selection_to_string().filter(|s| !s.is_empty());
                match selected {
                    Some(text) => {
                        ui.ctx().copy_text(text);
                        session.term().selection = None;
                    }
                    None => {
                        session.write(b"\x03");
                        sent = true;
                    }
                }
            }
            Event::Cut => {
                session.write(b"\x18");
                sent = true;
            }
            Event::Key { key, pressed: true, modifiers, .. } => {
                let ctrl = modifiers.ctrl || modifiers.command;
                // Copy/cut/paste arrive as their own events.
                if ctrl && matches!(key, Key::C | Key::X | Key::V) || modifiers.shift && key == Key::Insert {
                    continue;
                }
                if modifiers.shift && matches!(key, Key::PageUp | Key::PageDown) {
                    let mut term = session.term();
                    term.scroll_display(if key == Key::PageUp { Scroll::PageUp } else { Scroll::PageDown });
                    continue;
                }
                if let Some(bytes) = key_bytes(key, modifiers, mode.contains(TermMode::APP_CURSOR)) {
                    session.write(&bytes);
                    sent = true;
                    // Alt+letter may also produce a text event; it was already sent.
                    alt_text |= modifiers.alt && !ctrl;
                }
            }
            _ => {}
        }
    }
    if sent {
        let mut term = session.term();
        term.scroll_display(Scroll::Bottom);
        term.selection = None;
    }
}

fn handle_mouse(ui: &mut egui::Ui, tab: &mut Tab, response: &egui::Response, grid: &Grid) {
    let size = tab.session.size();
    // Wheel: scroll back through history, or send arrow keys to full-screen
    // programs (less, vim) that use the alternate screen.
    if response.hovered() {
        let dy = ui.input(|i| i.smooth_scroll_delta.y);
        if dy != 0.0 {
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
            tab.scroll_rest += dy / grid.row_h;
            let lines = tab.scroll_rest.trunc() as i32;
            tab.scroll_rest -= lines as f32;
            if lines != 0 {
                let mode = *tab.session.term().mode();
                if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    let key = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
                    for _ in 0..lines.unsigned_abs() {
                        tab.session.write(key);
                    }
                } else {
                    tab.session.term().scroll_display(Scroll::Delta(lines));
                }
            }
        }
    }

    let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return };
    let offset = tab.session.term().grid().display_offset();
    let (point, side) = grid.point_at(p, offset, size);
    if response.secondary_clicked() {
        // Right-click copies the selection, like Windows Terminal.
        let text = tab.session.term().selection_to_string().filter(|s| !s.is_empty());
        if let Some(text) = text {
            ui.ctx().copy_text(text);
            tab.session.term().selection = None;
        }
        return;
    }
    let mut term = tab.session.term();
    if response.triple_clicked() {
        term.selection = Some(Selection::new(SelectionType::Lines, point, side));
    } else if response.double_clicked() {
        term.selection = Some(Selection::new(SelectionType::Semantic, point, side));
    } else if response.drag_started() {
        term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    } else if response.dragged() {
        if let Some(sel) = &mut term.selection {
            sel.update(point, side);
        }
        if p.y < grid.rect.top() || p.y > grid.rect.bottom() {
            term.scroll_display(Scroll::Delta(if p.y < grid.rect.top() { 1 } else { -1 }));
            ui.ctx().request_repaint();
        }
    } else if response.clicked() {
        term.selection = None;
    }
}

/// Resolves a cell color: the program's palette overrides, else the theme.
fn resolve(color: Color, colors: &Colors) -> Color32 {
    let pal = theme::colors();
    let rgb = |c: Rgb| Color32::from_rgb(c.r, c.g, c.b);
    match color {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => colors[i as usize].map_or_else(|| indexed(i), rgb),
        Color::Named(n) => match colors[n as usize] {
            Some(c) => rgb(c),
            None => match n {
                NamedColor::Foreground | NamedColor::BrightForeground => pal.text,
                NamedColor::DimForeground => pal.text.gamma_multiply(0.7),
                NamedColor::Background => pal.editor_bg,
                NamedColor::Cursor => pal.cursor,
                n if (n as usize) < 16 => indexed(n as u8),
                // Dim colors.
                n => indexed((n as usize - NamedColor::DimBlack as usize) as u8).gamma_multiply(0.7),
            },
        },
    }
}

/// The xterm 256-color palette.
fn indexed(i: u8) -> Color32 {
    match i {
        0..16 => {
            let (r, g, b) = ANSI[i as usize];
            Color32::from_rgb(r, g, b)
        }
        16..232 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

fn paint(ui: &mut egui::Ui, session: &Session, grid: &Grid, font: &FontId, focused: bool, rect: Rect) {
    let pal = theme::colors();
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, pal.editor_bg);
    let term = session.term();
    let content = term.renderable_content();
    let offset = content.display_offset;
    let colors = content.colors;
    let selection = content.selection;
    let origin = grid.rect.min;
    let cell_pos = |line: i32, col: usize| {
        Pos2::new(origin.x + col as f32 * grid.char_w, origin.y + (line + offset as i32) as f32 * grid.row_h)
    };
    let draw_text = |text: String, pos: Pos2, color: Color32, italic: bool| {
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat { font_id: font.clone(), color, italics: italic, ..Default::default() },
        );
        job.wrap.max_width = f32::INFINITY;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        painter.galley(pos, galley, color);
    };

    // Runs of same-styled ASCII text are drawn together; other characters
    // (wide, symbols) are placed one by one so the grid never drifts.
    let mut run = String::new();
    let mut run_start: Option<(i32, usize, Color32, bool)> = None;
    let flush = |run: &mut String, start: &mut Option<(i32, usize, Color32, bool)>| {
        if let Some((line, col, color, italic)) = start.take()
            && !run.is_empty()
        {
            draw_text(std::mem::take(run), cell_pos(line, col), color, italic);
        }
        run.clear();
    };

    for indexed_cell in content.display_iter {
        let point = indexed_cell.point;
        let cell = &indexed_cell.cell;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let (mut fg, mut bg) = (resolve(cell.fg, colors), resolve(cell.bg, colors));
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if cell.flags.contains(Flags::DIM) {
            fg = fg.gamma_multiply(0.7);
        }
        if cell.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let selected = selection.is_some_and(|s| s.contains(point));
        let wide = cell.flags.contains(Flags::WIDE_CHAR);
        let pos = cell_pos(point.line.0, point.column.0);
        let cell_rect = Rect::from_min_size(pos, Vec2::new(grid.char_w * if wide { 2.0 } else { 1.0 }, grid.row_h));
        if selected {
            painter.rect_filled(cell_rect, 0.0, pal.selection);
        } else if bg != pal.editor_bg {
            painter.rect_filled(cell_rect, 0.0, bg);
        }
        if cell.flags.contains(Flags::UNDERLINE) {
            painter.hline(cell_rect.x_range(), cell_rect.bottom() - 1.5, Stroke::new(1.0, fg));
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            painter.hline(cell_rect.x_range(), cell_rect.center().y, Stroke::new(1.0, fg));
        }

        let c = cell.c;
        let italic = cell.flags.contains(Flags::ITALIC);
        let plain = c.is_ascii() && !wide;
        let continues = run_start.is_some_and(|(line, col, color, it)| {
            line == point.line.0 && col + run.len() == point.column.0 && color == fg && it == italic
        });
        if !continues || !plain {
            flush(&mut run, &mut run_start);
        }
        if c == ' ' || c == '\0' {
            continue;
        }
        if plain {
            if run_start.is_none() {
                run_start = Some((point.line.0, point.column.0, fg, italic));
            }
            run.push(c);
        } else {
            draw_text(c.to_string(), pos, fg, italic);
        }
    }
    flush(&mut run, &mut run_start);

    // Cursor.
    let cursor = content.cursor;
    if content.mode.contains(TermMode::SHOW_CURSOR) && cursor.shape != CursorShape::Hidden {
        let pos = cell_pos(cursor.point.line.0, cursor.point.column.0);
        let cell = Rect::from_min_size(pos, Vec2::new(grid.char_w, grid.row_h));
        if grid.rect.intersects(cell) {
            let color = colors[NamedColor::Cursor as usize].map_or(pal.cursor, |c| Color32::from_rgb(c.r, c.g, c.b));
            match (cursor.shape, focused) {
                (CursorShape::Beam, _) => {
                    painter.rect_filled(Rect::from_min_size(pos, Vec2::new(2.0, grid.row_h)), 0.0, color);
                }
                (CursorShape::Underline, _) => {
                    let bar = Rect::from_min_size(Pos2::new(pos.x, cell.bottom() - 2.0), Vec2::new(grid.char_w, 2.0));
                    painter.rect_filled(bar, 0.0, color);
                }
                (CursorShape::Block, true) => {
                    painter.rect_filled(cell, 0.0, color);
                    let c = term.grid()[cursor.point].c;
                    if c != ' ' && c != '\0' {
                        draw_text(c.to_string(), pos, pal.editor_bg, false);
                    }
                }
                _ => {
                    painter.rect_stroke(cell, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
                }
            }
        }
    }

    // Scrollbar thumb when there is history.
    let history = term.grid().history_size();
    if history > 0 {
        let screen = term.grid().screen_lines() as f32;
        let total = history as f32 + screen;
        let track = Rect::from_min_max(Pos2::new(rect.right() - SCROLLBAR, rect.top()), rect.max);
        let h = (screen / total * track.height()).max(20.0);
        let t = 1.0 - offset as f32 / history as f32;
        let top = track.top() + t * (track.height() - h);
        let thumb = Color32::from_rgba_unmultiplied(0x79, 0x79, 0x79, 0x66);
        let bar = Rect::from_min_size(Pos2::new(track.left() + 2.0, top), Vec2::new(SCROLLBAR - 4.0, h));
        painter.rect_filled(bar, 3.0, thumb);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_terminal_sequences() {
        let none = Modifiers::NONE;
        assert_eq!(key_bytes(Key::Enter, none, false).unwrap(), b"\r");
        assert_eq!(key_bytes(Key::ArrowUp, none, false).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(Key::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(key_bytes(Key::ArrowLeft, Modifiers::CTRL, false).unwrap(), b"\x1b[1;5D");
        assert_eq!(key_bytes(Key::C, Modifiers::CTRL, false).unwrap(), vec![3]);
        assert_eq!(key_bytes(Key::B, Modifiers::ALT, false).unwrap(), vec![0x1b, b'b']);
        assert_eq!(key_bytes(Key::Delete, none, false).unwrap(), b"\x1b[3~");
        assert_eq!(key_bytes(Key::F5, none, false).unwrap(), b"\x1b[15~");
        assert!(key_bytes(Key::A, none, false).is_none());
    }

    #[test]
    fn palette_covers_cube_and_grays() {
        assert_eq!(indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232), Color32::from_rgb(8, 8, 8));
    }
}
