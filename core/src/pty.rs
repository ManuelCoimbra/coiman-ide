//! Terminal sessions: a shell in a pseudo-terminal (ConPTY on Windows) whose
//! output is interpreted by alacritty's terminal emulator. No UI here: the
//! app draws the grid and sends keys.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub use alacritty_terminal;

/// Terminal size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub columns: usize,
    pub lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl Size {
    fn pty(self) -> PtySize {
        PtySize { rows: self.lines as u16, cols: self.columns as u16, pixel_width: 0, pixel_height: 0 }
    }
}

/// Something the session tells the UI.
#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    Title(String),
    ResetTitle,
    Bell,
    /// The program asked to copy text to the clipboard (OSC 52).
    Clipboard(String),
    /// The shell exited, with its exit code if known.
    Exited(Option<u32>),
}

type Writer = Arc<Mutex<Box<dyn Write + Send>>>;

/// Receives events from the emulator (on the reader thread).
#[derive(Clone)]
pub struct Listener {
    tx: Sender<Notice>,
    writer: Writer,
    size: Arc<Mutex<Size>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        let notice = match event {
            // Replies to queries (cursor position, device attributes) go back to the program.
            TermEvent::PtyWrite(text) => {
                write_all(&self.writer, text.as_bytes());
                return;
            }
            TermEvent::TextAreaSizeRequest(format) => {
                let size = *self.size.lock().unwrap_or_else(|e| e.into_inner());
                let window = WindowSize {
                    num_lines: size.lines as u16,
                    num_cols: size.columns as u16,
                    cell_width: 8,
                    cell_height: 16,
                };
                write_all(&self.writer, format(window).as_bytes());
                return;
            }
            TermEvent::Title(title) => Notice::Title(title),
            TermEvent::ResetTitle => Notice::ResetTitle,
            TermEvent::Bell => Notice::Bell,
            TermEvent::ClipboardStore(_, text) => Notice::Clipboard(text),
            _ => return,
        };
        let _ = self.tx.send(notice);
    }
}

fn write_all(writer: &Writer, bytes: &[u8]) {
    let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
    let _ = w.write_all(bytes).and_then(|()| w.flush());
}

/// The shell to use when the setting is empty: PowerShell on Windows,
/// `$SHELL` (or `/bin/sh`) elsewhere.
pub fn default_shell() -> String {
    if cfg!(windows) {
        "powershell.exe".to_owned()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
    }
}

/// Splits a command line into words, honoring double quotes.
fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in command.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

pub struct Session {
    term: Arc<FairMutex<Term<Listener>>>,
    writer: Writer,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    notices: Receiver<Notice>,
    size: Size,
    shared_size: Arc<Mutex<Size>>,
    /// Program name, e.g. "powershell".
    pub name: String,
    exited: bool,
}

impl Session {
    /// Starts `command` (a shell, possibly with arguments) in `cwd`.
    /// `wake` is called from background threads when there is new output.
    pub fn spawn(
        command: &str,
        cwd: &Path,
        size: Size,
        scrollback: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let words = split_command(command);
        let (program, args) = words.split_first().ok_or("no shell configured")?;
        Self::spawn_args(program, args, cwd, &[], size, scrollback, wake)
    }

    /// Like [`Session::spawn`] with the program and arguments given
    /// separately, plus extra environment variables.
    pub fn spawn_args(
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        size: Size,
        scrollback: usize,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let program = program.to_owned();
        let pair = native_pty_system().openpty(size.pty()).map_err(|e| format!("could not open a terminal: {e}"))?;
        let mut cmd = CommandBuilder::new(&program);
        cmd.args(args);
        cmd.cwd(cwd);
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "rust-code");
        let mut child = pair.slave.spawn_command(cmd).map_err(|e| format!("could not start {program}: {e}"))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer: Writer = Arc::new(Mutex::new(pair.master.take_writer().map_err(|e| e.to_string())?));

        let (tx, notices) = mpsc::channel();
        let shared_size = Arc::new(Mutex::new(size));
        let listener = Listener { tx: tx.clone(), writer: Arc::clone(&writer), size: Arc::clone(&shared_size) };
        let config = Config { scrolling_history: scrollback, ..Config::default() };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener)));
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);

        let (term_r, wake_r) = (Arc::clone(&term), Arc::clone(&wake));
        std::thread::Builder::new()
            .name("pty-reader".into())
            .spawn(move || {
                let mut processor: Processor = Processor::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            processor.advance(&mut *term_r.lock(), &buf[..n]);
                            wake_r();
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        // On Windows the output pipe stays open after the shell exits, so
        // exits are detected by waiting on the process itself.
        std::thread::Builder::new()
            .name("pty-wait".into())
            .spawn(move || {
                let code = child.wait().ok().map(|s| s.exit_code());
                let _ = tx.send(Notice::Exited(code));
                wake();
            })
            .map_err(|e| e.to_string())?;

        let name =
            Path::new(&program).file_stem().map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
        Ok(Self { term, writer, master: pair.master, killer, notices, size, shared_size, name, exited: false })
    }

    /// The emulator state (grid, cursor, modes); hold the lock briefly.
    pub fn term(&self) -> impl std::ops::DerefMut<Target = Term<Listener>> + '_ {
        self.term.lock()
    }

    /// Sends input (typed text or escape sequences) to the program.
    pub fn write(&self, bytes: &[u8]) {
        if !self.exited {
            write_all(&self.writer, bytes);
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn resize(&mut self, size: Size) {
        if size == self.size || size.columns == 0 || size.lines == 0 {
            return;
        }
        self.size = size;
        *self.shared_size.lock().unwrap_or_else(|e| e.into_inner()) = size;
        self.term.lock().resize(size);
        let _ = self.master.resize(size.pty());
    }

    /// Everything reported since the last call.
    pub fn poll(&mut self) -> Vec<Notice> {
        let notices: Vec<Notice> = self.notices.try_iter().collect();
        if notices.iter().any(|n| matches!(n, Notice::Exited(_))) {
            self.exited = true;
        }
        notices
    }

    pub fn has_exited(&self) -> bool {
        self.exited
    }

    /// The visible screen as text, one line per row (trailing spaces trimmed).
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let mut out = String::new();
        for line in 0..grid.screen_lines() {
            let row = &grid[Line(line as i32)];
            let text: String = (0..grid.columns()).map(|c| row[Column(c)].c).collect();
            out.push_str(text.trim_end());
            out.push('\n');
        }
        out
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.killer.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn splits_commands_with_quotes() {
        assert_eq!(split_command(r#"pwsh -NoLogo"#), vec!["pwsh", "-NoLogo"]);
        assert_eq!(
            split_command(r#""C:\Program Files\Git\bin\bash.exe" -l"#),
            vec![r"C:\Program Files\Git\bin\bash.exe", "-l"]
        );
    }

    #[test]
    fn runs_a_program_and_reports_its_exit() {
        let command =
            if cfg!(windows) { "cmd.exe /c echo rust-code-pty-test" } else { "sh -c \"echo rust-code-pty-test\"" };
        let size = Size { columns: 80, lines: 10 };
        let mut session = Session::spawn(command, &std::env::temp_dir(), size, 100, || {}).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut exited = false;
        while Instant::now() < deadline && !(exited && session.screen_text().contains("rust-code-pty-test")) {
            exited |= session.poll().iter().any(|n| matches!(n, Notice::Exited(_)));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.screen_text().contains("rust-code-pty-test"), "screen: {:?}", session.screen_text());
        assert!(exited && session.has_exited());
    }
}
