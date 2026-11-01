//! Log files of any size: a sparse index of where lines start, reads of just the
//! lines in view, a regex filter that runs alongside, and following as it grows.
//!
//! Files are read, not memory-mapped: a log that's truncated or rotated while
//! mapped takes the whole process down with it, and following makes that likely.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use regex::bytes::{Regex, RegexBuilder};

/// Where to remember a line's place, at the latest.
const LINES_PER_MARK: u64 = 256;
const BYTES_PER_MARK: u64 = 1024 * 1024;
const CHUNK: usize = 1024 * 1024;
/// Characters of a line that are shown and searched. The rest is skipped over.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

/// Files at least this size open in the log view whatever they're called.
const ANY_FILE_LIMIT: u64 = 128 * 1024 * 1024;
/// Logs open there from a smaller size: the editor would hold them all in memory.
const LOG_FILE_LIMIT: u64 = 16 * 1024 * 1024;

/// Whether a file is better read in the log view than loaded into the editor.
pub fn belongs_in_log_view(path: &Path) -> bool {
    belongs_in_log_view_from(path, LOG_FILE_LIMIT, ANY_FILE_LIMIT)
}

/// The same with sizes chosen in settings. A limit of 0 is none.
pub fn belongs_in_log_view_from(path: &Path, log_limit: u64, any_limit: u64) -> bool {
    let Ok(size) = std::fs::metadata(path).map(|m| m.len()) else { return false };
    let is_log = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("log"))
        || path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase().contains(".log."));
    (any_limit > 0 && size >= any_limit) || (log_limit > 0 && is_log && size >= log_limit)
}

/// Where a file's lines are, as far as it has been read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineIndex {
    /// (line, byte offset of its start), ascending, the first always (0, 0).
    marks: Vec<(u64, u64)>,
    /// Line breaks seen.
    breaks: u64,
    /// How far it's been read.
    pub scanned: u64,
    /// Whether the file doesn't end in a line break, so its last line is partial.
    trailing: bool,
    /// Bumped when the file got smaller: it's another file now, as far as anyone can tell.
    pub generation: u32,
    last_mark_offset: u64,
    /// The start of the file, to notice it being replaced by one that's as long.
    head: Vec<u8>,
}

/// Bytes kept to tell one file from another.
const HEAD_BYTES: usize = 256;

/// UTF-16 in Latin letters is readable with its zeros gone, as logs almost always are.
fn without_zeros(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if bytes.contains(&0) || bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let body = bytes.strip_prefix(&[0xFF, 0xFE]).or_else(|| bytes.strip_prefix(&[0xFE, 0xFF])).unwrap_or(bytes);
        body.iter().copied().filter(|b| *b != 0).collect::<Vec<u8>>().into()
    } else {
        bytes.into()
    }
}

impl LineIndex {
    pub fn new() -> Self {
        Self { marks: vec![(0, 0)], ..Self::default() }
    }

    pub fn lines(&self) -> u64 {
        self.breaks + u64::from(self.trailing)
    }

    /// Reads what's new in the file, up to `budget` bytes. Returns whether anything
    /// changed, and whether there's more to read.
    pub fn update(&mut self, path: &Path, budget: u64) -> std::io::Result<(bool, bool)> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        let mut changed = false;
        let mut head = vec![0; HEAD_BYTES];
        let got = file.read(&mut head)?;
        head.truncate(got);
        if size < self.scanned || !head.starts_with(&self.head) {
            let generation = self.generation + 1;
            *self = Self { generation, ..Self::new() };
            changed = true;
        }
        self.head = head;
        if size == self.scanned {
            return Ok((changed, false));
        }
        file.seek(SeekFrom::Start(self.scanned))?;
        let mut buffer = vec![0; CHUNK];
        let mut left = budget.min(size - self.scanned);
        while left > 0 {
            let want = (left as usize).min(CHUNK);
            let got = file.read(&mut buffer[..want])?;
            if got == 0 {
                break;
            }
            for (at, byte) in buffer[..got].iter().enumerate() {
                if *byte == b'\n' {
                    self.breaks += 1;
                    let next = self.scanned + at as u64 + 1;
                    let enough_lines = self.breaks.is_multiple_of(LINES_PER_MARK);
                    if enough_lines || next - self.last_mark_offset >= BYTES_PER_MARK {
                        self.marks.push((self.breaks, next));
                        self.last_mark_offset = next;
                    }
                }
            }
            self.trailing = buffer[got - 1] != b'\n';
            self.scanned += got as u64;
            left -= got as u64;
            changed = true;
        }
        Ok((changed, self.scanned < size))
    }

    /// The nearest place at or before a line.
    fn mark_before(&self, line: u64) -> (u64, u64) {
        let at = self.marks.partition_point(|(l, _)| *l <= line);
        self.marks.get(at.saturating_sub(1)).copied().unwrap_or((0, 0))
    }

    /// `count` lines from `first`. Long lines come back cut short.
    pub fn read_lines(&self, path: &Path, first: u64, count: usize) -> std::io::Result<Vec<String>> {
        let mut out = Vec::with_capacity(count);
        if count == 0 || first >= self.lines() {
            return Ok(out);
        }
        let (mut line, offset) = self.mark_before(first);
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0; 64 * 1024];
        let mut current: Vec<u8> = Vec::new();
        let finish = |current: &mut Vec<u8>, out: &mut Vec<String>| {
            let text = without_zeros(current);
            let text = text.strip_suffix(b"\r").unwrap_or(&text);
            out.push(String::from_utf8_lossy(text).into_owned());
            current.clear();
        };
        loop {
            let got = file.read(&mut buffer)?;
            if got == 0 {
                break;
            }
            for piece in buffer[..got].split_inclusive(|b| *b == b'\n') {
                let ended = piece.last() == Some(&b'\n');
                if line >= first {
                    let room = MAX_LINE_BYTES.saturating_sub(current.len());
                    let body = if ended { &piece[..piece.len() - 1] } else { piece };
                    current.extend_from_slice(&body[..body.len().min(room)]);
                }
                if ended {
                    if line >= first {
                        finish(&mut current, &mut out);
                        if out.len() == count {
                            return Ok(out);
                        }
                    }
                    line += 1;
                }
            }
        }
        if line >= first && !current.is_empty() {
            finish(&mut current, &mut out);
        }
        Ok(out)
    }

    pub fn offset_scanned(&self) -> (u64, u64) {
        (self.breaks, self.scanned)
    }
}

/// Builds a filter, or says what's wrong with it. Empty is none.
pub fn compile_filter(pattern: &str, match_case: bool) -> Result<Option<Regex>, String> {
    if pattern.is_empty() {
        return Ok(None);
    }
    RegexBuilder::new(pattern)
        .case_insensitive(!match_case)
        .build()
        .map(Some)
        .map_err(|e| e.to_string().lines().last().unwrap_or("bad pattern").trim().to_owned())
}

/// Where a filter has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilterPlace {
    pub line: u64,
    pub offset: u64,
}

/// Goes through complete lines from `place` up to the file's end at most, telling
/// `found` each line that matches (or doesn't, with `invert`). A line still being
/// written waits for its line break. Returns where it stopped.
pub fn scan_matches(
    path: &Path,
    regex: &Regex,
    invert: bool,
    place: FilterPlace,
    limit: u64,
    cancel: &AtomicBool,
    mut found: impl FnMut(u64),
) -> std::io::Result<FilterPlace> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(place.offset))?;
    let mut buffer = vec![0; CHUNK];
    let (mut line, mut offset) = (place.line, place.offset);
    // Bytes kept from the start of a line that crossed a chunk.
    let mut held: Vec<u8> = Vec::new();
    let mut held_len = 0u64;
    let mut read = 0u64;
    let test = |bytes: &[u8]| {
        let bytes = without_zeros(bytes);
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(&bytes);
        regex.is_match(&bytes[..bytes.len().min(MAX_LINE_BYTES)]) != invert
    };
    while !cancel.load(Ordering::Relaxed) && read < limit {
        let got = file.read(&mut buffer)?;
        if got == 0 {
            break;
        }
        read += got as u64;
        let mut from = 0;
        while let Some(at) = buffer[from..got].iter().position(|b| *b == b'\n') {
            let end = from + at;
            let matched = if held_len == 0 {
                test(&buffer[from..end])
            } else {
                let room = MAX_LINE_BYTES.saturating_sub(held.len());
                held.extend_from_slice(&buffer[from..end.min(from + room)]);
                test(&held)
            };
            if matched {
                found(line);
            }
            offset += held_len + (end - from) as u64 + 1;
            held.clear();
            held_len = 0;
            line += 1;
            from = end + 1;
        }
        let rest = &buffer[from..got];
        let room = MAX_LINE_BYTES.saturating_sub(held.len());
        held.extend_from_slice(&rest[..rest.len().min(room)]);
        held_len += rest.len() as u64;
    }
    Ok(FilterPlace { line, offset })
}

/// How serious a line says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Quiet,
    Plain,
}

/// Reads a level from the start of a line, where logs put it. A word further along
/// is more likely to be in the message: "no error found".
pub fn level_of(line: &str) -> Level {
    let head: String = line.chars().take(80).collect();
    for word in head.split(|c: char| !c.is_ascii_alphabetic()) {
        match word {
            "ERROR" | "ERR" | "FATAL" | "CRITICAL" | "CRIT" | "SEVERE" | "PANIC" => return Level::Error,
            "WARN" | "WARNING" => return Level::Warning,
            "DEBUG" | "TRACE" | "VERBOSE" => return Level::Quiet,
            "INFO" | "NOTICE" => return Level::Plain,
            _ => {}
        }
    }
    Level::Plain
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rc-log-{}-{name}", std::process::id()))
    }

    fn index_of(path: &Path) -> LineIndex {
        let mut index = LineIndex::new();
        while index.update(path, u64::MAX).unwrap().1 {}
        index
    }

    #[test]
    fn indexes_and_reads_any_line() {
        let path = temp("read");
        let text: String = (0..5000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, text).unwrap();
        let index = index_of(&path);
        assert_eq!(index.lines(), 5000);
        assert!(index.marks.len() > 10, "sparse, but there");
        assert_eq!(index.read_lines(&path, 0, 2).unwrap(), ["line 0", "line 1"]);
        assert_eq!(index.read_lines(&path, 256, 1).unwrap(), ["line 256"]);
        assert_eq!(index.read_lines(&path, 4999, 5).unwrap(), ["line 4999"]);
        assert!(index.read_lines(&path, 5000, 1).unwrap().is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn notices_a_file_replaced_by_one_as_long() {
        let path = temp("replaced");
        std::fs::write(&path, "first\nline\n").unwrap();
        let mut index = index_of(&path);
        assert_eq!(index.generation, 0);
        std::fs::write(&path, "other\nfile\nthat is longer\n").unwrap();
        while index.update(&path, u64::MAX).unwrap().1 {}
        assert_eq!(index.generation, 1);
        assert_eq!(index.lines(), 3);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn reads_utf16_lines() {
        let path = temp("utf16");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("ERROR one\nok\n".encode_utf16().flat_map(u16::to_le_bytes));
        std::fs::write(&path, bytes).unwrap();
        let index = index_of(&path);
        assert_eq!(index.read_lines(&path, 0, 1).unwrap(), ["ERROR one"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn handles_line_ends_and_a_last_partial_line() {
        let path = temp("ends");
        std::fs::write(&path, "a\r\n\r\nb").unwrap();
        let index = index_of(&path);
        assert_eq!(index.lines(), 3);
        assert_eq!(index.read_lines(&path, 0, 9).unwrap(), ["a", "", "b"]);
        std::fs::write(&path, "").unwrap();
        assert_eq!(index_of(&path).lines(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn cuts_long_lines_and_stays_fast_past_them() {
        let path = temp("long");
        let long = "x".repeat(5 * 1024 * 1024);
        std::fs::write(&path, format!("first\n{long}\nlast\n")).unwrap();
        let index = index_of(&path);
        assert_eq!(index.lines(), 3);
        let lines = index.read_lines(&path, 1, 2).unwrap();
        assert_eq!(lines[0].len(), MAX_LINE_BYTES);
        assert_eq!(lines[1], "last");
        // A mark right after the long line, so reaching "last" doesn't read through it.
        assert!(index.mark_before(2).1 > 5 * 1024 * 1024);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bad_bytes_show_rather_than_fail() {
        let path = temp("bytes");
        std::fs::write(&path, b"ok\n\xff\xfe bad\n").unwrap();
        let lines = index_of(&path).read_lines(&path, 1, 1).unwrap();
        assert!(lines[0].ends_with(" bad"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn follows_growth_and_starts_over_when_it_shrinks() {
        let path = temp("follow");
        std::fs::write(&path, "one\ntw").unwrap();
        let mut index = index_of(&path);
        assert_eq!(index.lines(), 2);
        assert_eq!(index.update(&path, u64::MAX).unwrap(), (false, false));
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"o\nthree\n").unwrap();
        drop(file);
        assert_eq!(index.update(&path, u64::MAX).unwrap(), (true, false));
        assert_eq!(index.lines(), 3);
        assert_eq!(index.read_lines(&path, 1, 2).unwrap(), ["two", "three"]);
        std::fs::write(&path, "new\n").unwrap();
        index.update(&path, u64::MAX).unwrap();
        assert_eq!((index.lines(), index.generation), (1, 1));
        assert_eq!(index.read_lines(&path, 0, 1).unwrap(), ["new"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_read_budget_lets_the_ui_show_progress() {
        let path = temp("budget");
        std::fs::write(&path, "x\n".repeat(1000)).unwrap();
        let mut index = LineIndex::new();
        assert_eq!(index.update(&path, 100).unwrap(), (true, true));
        assert_eq!(index.lines(), 50);
        while index.update(&path, 100).unwrap().1 {}
        assert_eq!(index.lines(), 1000);
        std::fs::remove_file(path).unwrap();
    }

    fn matches(text: &[u8], pattern: &str, invert: bool) -> Vec<u64> {
        let path = temp(&format!("filter-{pattern}-{invert}-{}", text.len()).replace(['|', '^', '$'], "_"));
        std::fs::write(&path, text).unwrap();
        let regex = compile_filter(pattern, false).unwrap().unwrap();
        let mut found = Vec::new();
        let cancel = AtomicBool::new(false);
        scan_matches(&path, &regex, invert, FilterPlace::default(), u64::MAX, &cancel, |l| found.push(l)).unwrap();
        std::fs::remove_file(path).unwrap();
        found
    }

    #[test]
    fn filters_by_regex() {
        let text = b"INFO a\nERROR b\r\nwarn c\nERROR d\n";
        assert_eq!(matches(text, "error", false), [1, 3]);
        assert_eq!(matches(text, "error", true), [0, 2]);
        assert_eq!(matches(text, "^warn|d$", false), [2, 3], "line ends don't get in the way");
        assert_eq!(matches(text, "none", false), Vec::<u64>::new());
    }

    #[test]
    fn a_line_being_written_waits_for_its_break() {
        assert_eq!(matches(b"ERROR a\nERROR par", "error", false), [0]);
    }

    #[test]
    fn filters_lines_across_chunks() {
        // The first needle starts just before a chunk ends.
        let mut text = b"x\n".repeat((CHUNK - 3) / 2);
        let first = text.len() / 2;
        if text.len() < CHUNK - 3 {
            text.push(b'\n');
        }
        text.extend_from_slice(b"needle\nother\nneedle\n");
        assert_eq!(text[CHUNK - 3..CHUNK - 1], *b"ne");
        let lines = text.iter().filter(|b| **b == b'\n').count() as u64;
        let expected = [lines - 3, lines - 1];
        assert!(first > 0);
        assert_eq!(matches(&text, "needle", false), expected);
    }

    #[test]
    fn a_filter_carries_on_from_where_it_stopped() {
        let path = temp("resume");
        std::fs::write(&path, "hit 1\nno\npar").unwrap();
        let regex = compile_filter("hit", false).unwrap().unwrap();
        let cancel = AtomicBool::new(false);
        let mut found = Vec::new();
        let place =
            scan_matches(&path, &regex, false, FilterPlace::default(), u64::MAX, &cancel, |l| found.push(l)).unwrap();
        assert_eq!(place, FilterPlace { line: 2, offset: 9 });
        let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"t hit\nhit 3\n").unwrap();
        drop(file);
        let place = scan_matches(&path, &regex, false, place, u64::MAX, &cancel, |l| found.push(l)).unwrap();
        assert_eq!(found, [0, 2, 3]);
        assert_eq!(place.line, 4);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn says_what_is_wrong_with_a_pattern() {
        assert!(compile_filter("(", false).is_err());
        assert!(compile_filter("", false).unwrap().is_none());
        assert!(compile_filter("Err", true).unwrap().unwrap().is_match(b"Err"));
        assert!(!compile_filter("Err", true).unwrap().unwrap().is_match(b"err"));
    }

    #[test]
    fn reads_levels_from_the_start_of_a_line() {
        assert_eq!(level_of("2026-10-31 12:00 ERROR boom"), Level::Error);
        assert_eq!(level_of("[WARN] careful"), Level::Warning);
        assert_eq!(level_of("level=DEBUG x"), Level::Quiet);
        assert_eq!(level_of("INFO no error found"), Level::Plain);
        assert_eq!(level_of("just text"), Level::Plain);
        let far = format!("{} ERROR", "x ".repeat(60));
        assert_eq!(level_of(&far), Level::Plain);
    }

    #[test]
    fn decides_what_belongs_in_the_view() {
        let path = temp("small.log");
        std::fs::write(&path, "x").unwrap();
        assert!(!belongs_in_log_view(&path));
        std::fs::remove_file(&path).unwrap();
        assert!(!belongs_in_log_view(&path), "a missing file doesn't");
    }
}
