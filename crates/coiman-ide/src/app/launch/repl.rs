//! Sends the current line, selection or cell to a REPL in a terminal tab, and
//! shows what came back beside the code.

use std::collections::HashMap;

use coiman_core::launch::repl::{self, Kind};
use eframe::egui;

use crate::app::{CodeApp, PanelTab};
use crate::buffer::Selection;
use crate::ui::editor_view::Decorations;
use crate::ui::terminal::Task;

/// Seconds to wait for a REPL's prompt before showing what there is so far.
const WAIT_FOR_PROMPT: f64 = 30.0;
/// Seconds output must rest before it's read, when no prompt is recognized.
const OUTPUT_RESTS: f64 = 1.0;

/// Code on its way: where to show the answer, and how to find it in the output.
struct Waiting {
    uid: u64,
    line: usize,
    line_text: String,
    sent: String,
    tab: u64,
    mark: i64,
    since: f64,
    last_change: f64,
    seen: usize,
}

#[derive(Default)]
pub(crate) struct Repls {
    waiting: Option<Waiting>,
    /// Answers by document: (line, what the line said, the answer).
    results: HashMap<u64, Vec<(usize, String, String)>>,
    /// Cells of the active document: (uid, version, lines of their markers).
    cells: Option<(u64, u64, Vec<usize>)>,
}

impl Repls {
    pub(crate) fn clear_answers(&mut self, uid: u64) {
        self.results.remove(&uid);
    }

    /// An answer to show beside a line, replacing what was there.
    pub(crate) fn set_answer(&mut self, uid: u64, line: usize, line_text: String, answer: String) {
        let results = self.results.entry(uid).or_default();
        results.retain(|(l, _, _)| *l != line);
        results.push((line, line_text, answer));
    }

    pub(crate) fn clear_answer(&mut self, uid: u64, line: usize) {
        if let Some(results) = self.results.get_mut(&uid) {
            results.retain(|(l, _, _)| *l != line);
        }
    }
}

impl CodeApp {
    /// Ctrl+Enter: sends the selection, else the cell or line, then moves on.
    pub(crate) fn send_to_repl(&mut self, ctx: &egui::Context) {
        // A live session for this kind of file takes it, whichever kind that is.
        if self.send_to_live() {
            return;
        }
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(file) = doc.path.clone() else {
            self.refuse_run(ctx, "Save this file first, so its language is known".to_owned());
            return;
        };
        let source = doc.text();
        let primary = doc.buffer.primary();
        let head_line = doc.buffer.rope().char_to_line(primary.head);
        let total = doc.buffer.len_lines();
        let line_end = |line: usize| doc.buffer.line_end(line.min(total - 1));
        let (code, last_line, next) = if primary.is_empty() {
            match repl::cell_at(&source, head_line) {
                Some(cell) => {
                    let start = doc.buffer.line_start(cell.start.min(total - 1));
                    let end = line_end(cell.end.saturating_sub(1).max(cell.start));
                    (
                        doc.buffer.slice_string(start, end),
                        repl::last_code_line(&source, cell.start, cell.end),
                        cell.next,
                    )
                }
                None => (doc.buffer.line_text(head_line), head_line, repl::next_code_line(&source, head_line)),
            }
        } else {
            // A selection ending at the start of a line doesn't include it.
            let rope = doc.buffer.rope();
            let end_line = rope.char_to_line(primary.end().saturating_sub(1).max(primary.start()));
            (doc.buffer.slice_string(primary.start(), primary.end()), end_line, None)
        };
        let uid = doc.uid;
        self.send_code(ctx, &file, &code, uid, last_line);
        if let Some(line) = next
            && let Some(doc) = self.docs.get_mut(self.active)
            && line < doc.buffer.len_lines()
        {
            let at = doc.buffer.line_start(line);
            doc.buffer.set_single(Selection::cursor(at));
            doc.reveal = true;
        }
    }

    /// A cell's gutter button was clicked. Returns whether `line` starts a cell.
    pub(crate) fn run_cell_at_line(&mut self, ctx: &egui::Context, line: usize) -> bool {
        let Some(doc) = self.docs.get(self.active) else { return false };
        let source = doc.text();
        if !repl::cell_lines(&source).contains(&line) {
            return false;
        }
        let Some(cell) = repl::cell_at(&source, line) else { return false };
        let Some(file) = doc.path.clone() else { return false };
        let total = doc.buffer.len_lines();
        let start = doc.buffer.line_start(cell.start.min(total - 1));
        let last = cell.end.saturating_sub(1).max(cell.start);
        let code = doc.buffer.slice_string(start, doc.buffer.line_end(last.min(total - 1)));
        let uid = doc.uid;
        let last = repl::last_code_line(&source, cell.start, cell.end);
        self.send_code(ctx, &file, &code, uid, last);
        true
    }

    fn send_code(&mut self, ctx: &egui::Context, file: &std::path::Path, code: &str, uid: u64, line: usize) {
        if code.trim().is_empty() {
            return;
        }
        let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let Some(mut repl) = repl::for_extension(&ext) else {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.refuse_run(ctx, format!("No REPL for {name}: add `repl` to its entry in languages.toml"));
            return;
        };
        let cwd = repl::start_folder(file, self.explorer.root.as_deref());
        if !self.allowed_to_run(&cwd) {
            return;
        }
        let mut env = self.environment_variables();
        if repl.kind == Kind::Python {
            // The project's Python; the plain prompt, which takes typed text reliably.
            if let Some(python) = self.python_env() {
                let call = if cfg!(windows) { "& " } else { "" };
                repl.command = format!("{call}'{}' -q -i", python.python.display());
            }
            env.push(("PYTHON_BASIC_REPL".to_owned(), "1".to_owned()));
        }
        let title = format!("REPL: {}", repl.title);
        let existing = self.terminal.running_tab(&title);
        let tab = match existing {
            Some(tab) => tab,
            None => {
                self.terminal.close_finished_named(&title);
                let (program, args) = crate::app::cargo::shell(&repl.command);
                let scrollback = self.settings.terminal.scrollback;
                let task = Task { title: &title, program: &program, args: &args, env: &env, cwd: &cwd, scan: None };
                if let Err(e) = self.terminal.run_task(ctx, task, scrollback) {
                    self.refuse_run(ctx, format!("Could not start the REPL: {e}"));
                    return;
                }
                let Some(tab) = self.terminal.running_tab(&title) else { return };
                if repl.kind == Kind::Python {
                    match repl::python_setup() {
                        Ok(keys) => self.terminal.type_into(tab, &keys),
                        Err(e) => self.status = Some(format!("Could not set up the REPL: {e}")),
                    }
                }
                tab
            }
        };
        let keys = match repl::keystrokes(repl.kind, code) {
            Ok(keys) => keys,
            Err(e) => {
                self.refuse_run(ctx, format!("Could not pass the code on: {e}"));
                return;
            }
        };
        let mark = self.terminal.output_mark(tab);
        self.terminal.type_into(tab, &keys);
        let now = ctx.input(|i| i.time);
        let line_text = self.docs.get(self.active).map(|d| d.buffer.line_text(line)).unwrap_or_default();
        self.repls.waiting = Some(Waiting {
            uid,
            line,
            line_text,
            sent: format!("{code}\n{}", keys.trim_end()),
            tab,
            mark,
            since: now,
            last_change: now,
            seen: 0,
        });
        // Show it, and stay in the code.
        self.panel_visible = true;
        self.panel_tab = PanelTab::Terminal;
        self.terminal.detached = false;
        self.terminal.show_tab(tab);
        self.terminal.focus_requested = false;
        if let Some(doc) = self.docs.get_mut(self.active) {
            doc.request_focus = true;
        }
    }

    /// `REPL: Restart`: ends the active file's REPL, and what it said with it. The next
    /// code sent starts a new one.
    pub(crate) fn restart_repl(&mut self) {
        let Some(doc) = self.docs.get(self.active) else { return };
        let Some(repl) = repl::for_extension(&doc.extension()) else {
            self.status = Some("No REPL for this file".to_owned());
            return;
        };
        self.repls.waiting = None;
        self.repls.results.clear();
        let stopped = self.terminal.stop_named(&format!("REPL: {}", repl.title));
        let name = repl.title;
        self.status = Some(if stopped {
            format!("{name} REPL ended: the next code you send starts a new one")
        } else {
            format!("No {name} REPL was running")
        });
    }

    /// Picks up the answer once the REPL is done, and keeps cells current.
    pub(crate) fn pump_repl(&mut self, ctx: &egui::Context) {
        static CLEANED: std::sync::Once = std::sync::Once::new();
        CLEANED.call_once(|| {
            std::thread::spawn(repl::clean_old_files);
        });
        if let Some(doc) = self.docs.get(self.active) {
            let (uid, version) = (doc.uid, doc.buffer.version());
            if self.repls.cells.as_ref().is_none_or(|(u, v, _)| (*u, *v) != (uid, version)) {
                let has_repl = repl::for_extension(&doc.extension()).is_some();
                let cells = if has_repl { repl::cell_lines(&doc.text()) } else { Vec::new() };
                self.repls.cells = Some((uid, version, cells));
            }
        }
        let Some(waiting) = &mut self.repls.waiting else { return };
        let Some(lines) = self.terminal.output_since(waiting.tab, waiting.mark) else {
            self.repls.waiting = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let size: usize = lines.iter().map(String::len).sum();
        if size != waiting.seen {
            waiting.seen = size;
            waiting.last_change = now;
        }
        let quiet = now - waiting.last_change;
        let done = (repl::is_ready(&lines) && quiet > 0.1)
            || now - waiting.since > WAIT_FOR_PROMPT
            || (quiet > OUTPUT_RESTS
                && now - waiting.since > OUTPUT_RESTS
                && !lines.is_empty()
                && !repl_busy_hint(&lines));
        if !done {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        let Some(waiting) = self.repls.waiting.take() else { return };
        let results = self.repls.results.entry(waiting.uid).or_default();
        results.retain(|(line, _, _)| *line != waiting.line);
        if let Some(answer) = repl::result_text(&lines, &waiting.sent) {
            results.push((waiting.line, waiting.line_text, answer));
        }
    }

    /// Answers beside the code, and a run button on each cell.
    pub(crate) fn repl_decorate(&self, i: usize, deco: &mut Decorations) {
        let Some(doc) = self.docs.get(i) else { return };
        let count = doc.buffer.len_lines();
        for (line, text, answer) in self.repls.results.get(&doc.uid).into_iter().flatten() {
            // Lines above may have moved it: it stays with its text, if that's still nearby.
            let near = (line.saturating_sub(ANSWER_REACH)..(line + ANSWER_REACH).min(count))
                .filter(|l| doc.buffer.line_text(*l) == *text)
                .min_by_key(|l| l.abs_diff(*line));
            match near {
                Some(at) => deco.values.push((at, format!("=> {answer}"))),
                // Its line was edited: this was true of the old text, and says so.
                None if *line < count => deco.values.push((*line, format!("=> {answer} (stale)"))),
                None => {}
            }
        }
        if let Some((uid, _, cells)) = &self.repls.cells
            && *uid == doc.uid
        {
            let color = egui::Color32::from_rgb(0x89, 0xD1, 0x85);
            deco.runnables.extend(cells.iter().map(|line| (*line, None, color)));
        }
    }
}

/// How far a line's answer looks for the line, once lines above have changed.
const ANSWER_REACH: usize = 200;

/// Output that plainly hasn't finished: a Python traceback has no end line yet.
fn repl_busy_hint(lines: &[String]) -> bool {
    lines.last().is_some_and(|l| l.trim_start().starts_with("Traceback"))
}
