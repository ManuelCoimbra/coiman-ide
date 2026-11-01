//! The app's two logs, kept on this computer in `<config>/logs`: `rust-code.log` says what
//! the app did, and `crash.log` says why it stopped. Text typed into files is never logged.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// A log this large is moved to `.1`, replacing the one before it.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
/// Lines of the regular log a crash report repeats.
const RECENT_LINES: usize = 80;

struct Logger {
    dir: PathBuf,
    /// Held locked while running: one left unlocked, and not removed, belongs to a session
    /// that didn't get to close.
    marker: Option<(PathBuf, File)>,
    recent: VecDeque<String>,
}

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);
/// What the last session's end left to report.
static LAST_CRASH: Mutex<Option<String>> = Mutex::new(None);

/// `<config>/logs`.
pub fn dir() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("logs"))
}

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("rust-code.log")
}

pub fn crash_path(dir: &Path) -> PathBuf {
    dir.join("crash.log")
}

/// `2026-11-01 14:03:22.041` in UTC.
fn stamp(since_epoch_ms: u128) -> String {
    let seconds = (since_epoch_ms / 1000) as i64;
    let (days, of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (hour, minute, second) = (of_day / 3600, of_day / 60 % 60, of_day % 60);
    let ms = since_epoch_ms % 1000;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{ms:03}")
}

fn now() -> String {
    stamp(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis()))
}

/// Starts logging in the usual place. Logging that can't start stays off: it never stops the app.
pub fn start() {
    if let Some(dir) = dir() {
        start_in(&dir);
    }
    install_crash_hook();
}

/// Starts logging in `dir`, and reports any session there that ended without closing.
pub fn start_in(dir: &Path) {
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let lost = sessions_that_did_not_close(dir);
    let pid = std::process::id();
    let marker_path = dir.join(format!("running-{pid}.marker"));
    let marker = File::create(&marker_path).ok().filter(|f| f.try_lock().is_ok()).map(|f| (marker_path, f));
    if let Ok(mut logger) = LOGGER.lock() {
        *logger = Some(Logger { dir: dir.to_path_buf(), marker, recent: VecDeque::new() });
    }
    info(&format!("started: version {}, {}; times are UTC", env!("CARGO_PKG_VERSION"), std::env::consts::OS));
    if lost > 0 {
        let text = format!(
            "{} {lost} earlier session(s) ended without closing: the process was killed, the computer stopped, or it crashed where no report could be written\n",
            now()
        );
        append(&crash_path(dir), &text);
        warn(&format!("{lost} earlier session(s) ended without closing"));
        if let Ok(mut last) = LAST_CRASH.lock() {
            *last = Some("The last session ended without closing: see Developer: Open Crash Log".to_owned());
        }
    }
}

/// Removes the markers of sessions gone without closing, and says how many there were.
fn sessions_that_did_not_close(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else { return 0 };
    let mut lost = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("running-") && name.ends_with(".marker")) {
            continue;
        }
        // Locked: that session is still running.
        let free = File::open(&path).is_ok_and(|f| f.try_lock().is_ok());
        if free && fs::remove_file(&path).is_ok() {
            lost += 1;
        }
    }
    lost
}

/// What to tell the user about how the last session ended, once.
pub fn take_last_crash() -> Option<String> {
    LAST_CRASH.lock().ok()?.take()
}

/// Records that this session is closing normally.
pub fn close() {
    info("closed");
    if let Ok(mut logger) = LOGGER.lock()
        && let Some((path, file)) = logger.as_mut().and_then(|l| l.marker.take())
    {
        drop(file);
        let _ = fs::remove_file(path);
    }
}

fn append(path: &Path, text: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

fn write(level: &str, message: &str) {
    let line = format!("{} [{}] {level} {message}\n", now(), std::process::id());
    let Ok(mut logger) = LOGGER.lock() else { return };
    let Some(logger) = logger.as_mut() else { return };
    let path = log_path(&logger.dir);
    if fs::metadata(&path).is_ok_and(|m| m.len() >= MAX_LOG_BYTES) {
        let _ = fs::rename(&path, logger.dir.join("rust-code.log.1"));
    }
    append(&path, &line);
    if logger.recent.len() == RECENT_LINES {
        logger.recent.pop_front();
    }
    logger.recent.push_back(line);
}

pub fn info(message: &str) {
    write("INFO", message);
}

pub fn warn(message: &str) {
    write("WARN", message);
}

/// What a crash report says, from what a panic knows.
fn crash_report(thread: &str, location: &str, message: &str, backtrace: &str, recent: &[String]) -> String {
    let mut text = format!(
        "==== {} crash, version {}, {} {}\nthread: {thread}\nat: {location}\nmessage: {message}\n\nbacktrace:\n{backtrace}\n",
        now(),
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    if !recent.is_empty() {
        text.push_str("\nlast lines of the log:\n");
        text.extend(recent.iter().cloned());
    }
    text.push('\n');
    text
}

/// Writes a crash report for every panic, on any thread, then does what panics did before.
fn install_crash_hook() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_owned());
        let location = info.location().map_or_else(|| "unknown".to_owned(), ToString::to_string);
        let current = std::thread::current();
        let thread = current.name().unwrap_or("unnamed");
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        // Not `lock`: the panic may have happened while holding it.
        if let Ok(mut logger) = LOGGER.try_lock()
            && let Some(logger) = logger.as_mut()
        {
            let recent: Vec<String> = logger.recent.iter().cloned().collect();
            append(&crash_path(&logger.dir), &crash_report(thread, &location, &message, &backtrace, &recent));
            let line = format!("{} [{}] ERROR panic at {location}: {message}\n", now(), std::process::id());
            append(&log_path(&logger.dir), &line);
        }
        before(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_right() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00.000");
        assert_eq!(stamp(951_782_400_123), "2000-02-29 00:00:00.123");
        assert_eq!(stamp(1_793_530_002_041), "2026-11-01 10:46:42.041");
    }

    #[test]
    fn crash_reports_say_where_and_what_came_before() {
        let recent = ["a line\n".to_owned()];
        let text = crash_report("main", "src/a.rs:1:2", "boom", "0: frame", &recent);
        for part in ["thread: main", "at: src/a.rs:1:2", "message: boom", "0: frame", "a line", "crash, version"] {
            assert!(text.contains(part), "{part}");
        }
    }

    #[test]
    fn finds_sessions_gone_without_closing_and_spares_running_ones() {
        let dir = std::env::temp_dir().join(format!("rc_log_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("running-1.marker"), "").unwrap();
        let held = dir.join("running-2.marker");
        let lock = File::create(&held).unwrap();
        lock.try_lock().unwrap();
        fs::write(dir.join("rust-code.log"), "kept").unwrap();
        assert_eq!(sessions_that_did_not_close(&dir), 1);
        assert!(!dir.join("running-1.marker").exists() && held.exists());
        assert_eq!(sessions_that_did_not_close(&dir), 0);
        drop(lock);
        fs::remove_dir_all(&dir).unwrap();
    }
}
