//! Constant preview: runs a program with nothing typed into it, stops it when it
//! overstays or is replaced, and says which of its lines are new.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::lsp::discovery::{kill_tree, quiet_command};
use crate::run::RunPlan;

/// Output past this is a program printing forever: it's stopped.
const MAX_OUTPUT: usize = 2 * 1024 * 1024;
/// How long after a program ends its last output may still arrive.
const DRAIN: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    Exit(Option<i32>),
    TimedOut,
    TooMuchOutput,
    CouldNotStart(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Line { text: String, error: bool },
    Finished(Ending),
}

/// A program running. Dropping this stops it.
pub struct Job {
    pub events: Receiver<RunEvent>,
    stop: Arc<AtomicBool>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Waits, so a program can't outlive the editor closing.
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
}

fn read_lines(
    from: impl Read + Send + 'static,
    error: bool,
    tx: Sender<RunEvent>,
    size: Arc<AtomicUsize>,
    open: Arc<AtomicUsize>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(from);
        let mut line = Vec::new();
        while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
            size.fetch_add(line.len(), Ordering::Relaxed);
            let text = String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_owned();
            line.clear();
            if tx.send(RunEvent::Line { text, error }).is_err() {
                break;
            }
            wake();
        }
        open.fetch_sub(1, Ordering::Relaxed);
    });
}

/// Starts `program`, with no input, for at most `limit`.
pub fn start(
    program: &str,
    args: &[String],
    plan: &RunPlan,
    env: &[(String, String)],
    limit: Option<Duration>,
    wake: impl Fn() + Send + Sync + 'static,
) -> Job {
    let (tx, events) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let mut job = Job { events, stop: stop.clone(), watcher: None };
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let mut command = quiet_command(Path::new(program));
    command.args(args).current_dir(&plan.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Output should arrive as it's printed, and be readable.
    command.env("PYTHONUNBUFFERED", "1").env("PYTHONIOENCODING", "utf-8");
    command.envs(env.iter().map(|(k, v)| (k, v)));
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(why) => {
            let _ = tx.send(RunEvent::Finished(Ending::CouldNotStart(why.to_string())));
            wake();
            return job;
        }
    };
    let size = Arc::new(AtomicUsize::new(0));
    let open = Arc::new(AtomicUsize::new(2));
    match child.stdout.take() {
        Some(out) => read_lines(out, false, tx.clone(), size.clone(), open.clone(), wake.clone()),
        None => {
            open.fetch_sub(1, Ordering::Relaxed);
        }
    }
    match child.stderr.take() {
        Some(err) => read_lines(err, true, tx.clone(), size.clone(), open.clone(), wake.clone()),
        None => {
            open.fetch_sub(1, Ordering::Relaxed);
        }
    }
    job.watcher = Some(std::thread::spawn(move || {
        let began = Instant::now();
        let ending = loop {
            if stop.load(Ordering::Relaxed) {
                kill_tree(&mut child);
                return;
            }
            if let Ok(Some(status)) = child.try_wait() {
                break Ending::Exit(status.code());
            }
            if limit.is_some_and(|l| began.elapsed() >= l) {
                kill_tree(&mut child);
                break Ending::TimedOut;
            }
            if size.load(Ordering::Relaxed) > MAX_OUTPUT {
                kill_tree(&mut child);
                break Ending::TooMuchOutput;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let waited = Instant::now();
        while open.load(Ordering::Relaxed) > 0 && waited.elapsed() < DRAIN {
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = tx.send(RunEvent::Finished(ending));
        wake();
    }));
    job
}

/// For each line of `new`, whether it differs from `old`.
pub fn changed_lines(old: &str, new: &str) -> Vec<bool> {
    let diff = similar::TextDiff::from_lines(old, new);
    let mut marks = Vec::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Equal => marks.push(false),
            similar::ChangeTag::Insert => marks.push(true),
            similar::ChangeTag::Delete => {}
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(command: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            ("powershell.exe".into(), vec!["-NoProfile".into(), "-Command".into(), command.into()])
        } else {
            ("sh".into(), vec!["-c".into(), command.into()])
        }
    }

    fn run(command: &str, limit: Option<Duration>) -> Vec<RunEvent> {
        let (program, args) = shell(command);
        let plan = RunPlan { title: "t".into(), command: command.into(), cwd: std::env::temp_dir() };
        let job = start(&program, &args, &plan, &[], limit, || {});
        let mut all = Vec::new();
        while let Ok(event) = job.events.recv_timeout(Duration::from_secs(30)) {
            let done = matches!(event, RunEvent::Finished(_));
            all.push(event);
            if done {
                break;
            }
        }
        all
    }

    #[test]
    fn reports_output_and_how_it_ended() {
        let events = run("echo hello; exit 3", None);
        assert!(events.contains(&RunEvent::Line { text: "hello".into(), error: false }), "{events:?}");
        assert_eq!(events.last(), Some(&RunEvent::Finished(Ending::Exit(Some(3)))));
    }

    #[test]
    fn stops_a_program_that_overstays() {
        let forever = if cfg!(windows) { "Start-Sleep 60" } else { "sleep 60" };
        let began = Instant::now();
        let events = run(forever, Some(Duration::from_millis(1500)));
        assert_eq!(events.last(), Some(&RunEvent::Finished(Ending::TimedOut)));
        assert!(began.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn stops_a_program_that_prints_forever() {
        let flood = if cfg!(windows) { "while ($true) { 'x' * 1000 }" } else { "yes xxxxxxxxxxxxxxxx" };
        let events = run(flood, None);
        assert_eq!(events.last(), Some(&RunEvent::Finished(Ending::TooMuchOutput)));
    }

    #[test]
    fn says_when_it_cannot_start() {
        let plan = RunPlan { title: "t".into(), command: String::new(), cwd: std::env::temp_dir() };
        let job = start("no-such-program-here", &[], &plan, &[], None, || {});
        let event = job.events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(event, RunEvent::Finished(Ending::CouldNotStart(_))), "{event:?}");
    }

    #[test]
    fn marks_only_what_changed() {
        assert_eq!(changed_lines("a\nb\nc\n", "a\nB\nc\nd\n"), [false, true, false, true]);
        assert_eq!(changed_lines("", "a\n"), [true]);
        assert_eq!(changed_lines("a\n", "a\n"), [false]);
    }
}
