//! Find in files with the ripgrep crates: a parallel walk that respects
//! .gitignore, searching each file on worker threads and streaming results.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkState;
use ignore::overrides::OverrideBuilder;

/// Stop after this many matches in total.
pub const MAX_MATCHES: usize = 20_000;
/// Longest line text kept for display.
const MAX_LINE_DISPLAY: usize = 240;

pub struct LineMatch {
    /// 1-based line number.
    pub line: usize,
    /// Line text, leading whitespace trimmed and truncated for display.
    pub text: String,
    /// Byte ranges of matches within `text`.
    pub ranges: Vec<(usize, usize)>,
    /// Char column range of the first match within the original line.
    pub cols: (usize, usize),
}

pub struct FileResult {
    pub path: PathBuf,
    pub matches: Vec<LineMatch>,
}

/// Progress of a running search.
pub enum Msg {
    File(FileResult),
    Done { files_searched: usize, truncated: bool },
    Error(String),
}

/// Where a clicked result points: file, 1-based line and char columns.
#[derive(Clone, PartialEq, Default)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include: String,
    pub exclude: String,
}

/// A running search. Dropping it (or starting another) cancels it.
pub struct SearchHandle {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

impl SearchHandle {
    /// Starts searching `root` for `query` on background threads; `notify`
    /// is called whenever new results are available.
    pub fn spawn(query: Query, root: PathBuf, notify: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            let (inner_tx, inner_rx) = channel();
            let forward = std::thread::spawn(move || {
                for msg in inner_rx {
                    if tx.send(msg).is_err() {
                        break;
                    }
                    notify();
                }
            });
            run_search(&query, &root, &inner_tx, &flag);
            drop(inner_tx);
            let _ = forward.join();
        });
        Self { rx, cancel }
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }
}

impl Drop for SearchHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

fn build_matcher(q: &Query) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!q.match_case)
        .word(q.whole_word)
        .fixed_strings(!q.regex)
        .build(&q.text)
        .map_err(|e| format!("Invalid search: {e}"))
}

fn run_search(q: &Query, root: &Path, tx: &Sender<Msg>, cancel: &Arc<AtomicBool>) {
    let matcher = match build_matcher(q) {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(Msg::Error(e));
            return;
        }
    };
    let mut walk = crate::project::finder::walker(root);
    let mut overrides = OverrideBuilder::new(root);
    let globs = |s: &str| s.split(',').map(str::trim).filter(|g| !g.is_empty()).map(str::to_owned).collect::<Vec<_>>();
    for g in globs(&q.include) {
        if let Err(e) = overrides.add(&g) {
            let _ = tx.send(Msg::Error(format!("Bad include glob `{g}`: {e}")));
            return;
        }
    }
    for g in globs(&q.exclude) {
        if let Err(e) = overrides.add(&format!("!{g}")) {
            let _ = tx.send(Msg::Error(format!("Bad exclude glob `{g}`: {e}")));
            return;
        }
    }
    match overrides.build() {
        Ok(o) => {
            walk.overrides(o);
        }
        Err(e) => {
            let _ = tx.send(Msg::Error(format!("Bad glob: {e}")));
            return;
        }
    }

    let total = Arc::new(AtomicUsize::new(0));
    let files = Arc::new(AtomicUsize::new(0));
    walk.build_parallel().run(|| {
        let (matcher, tx, cancel, total, files) =
            (matcher.clone(), tx.clone(), cancel.clone(), total.clone(), files.clone());
        let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) || total.load(Ordering::Relaxed) >= MAX_MATCHES {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            files.fetch_add(1, Ordering::Relaxed);
            let mut matches = Vec::new();
            let sink = UTF8(|line_number, line| {
                if let Some(m) = line_match(&matcher, line_number as usize, line) {
                    matches.push(m);
                }
                Ok(total.load(Ordering::Relaxed) + matches.len() < MAX_MATCHES)
            });
            let _ = searcher.search_path(&matcher, entry.path(), sink);
            if !matches.is_empty() {
                total.fetch_add(matches.len(), Ordering::Relaxed);
                if tx.send(Msg::File(FileResult { path: entry.into_path(), matches })).is_err() {
                    return WalkState::Quit;
                }
            }
            WalkState::Continue
        })
    });
    if !cancel.load(Ordering::Relaxed) {
        let _ = tx.send(Msg::Done {
            files_searched: files.load(Ordering::Relaxed),
            truncated: total.load(Ordering::Relaxed) >= MAX_MATCHES,
        });
    }
}

/// Matches of one line, trimmed for display.
fn line_match(matcher: &RegexMatcher, line: usize, raw: &str) -> Option<LineMatch> {
    let content = raw.trim_end_matches(['\n', '\r']);
    let mut found: Vec<(usize, usize)> = Vec::new();
    let _ = matcher.find_iter(content.as_bytes(), |m| {
        if m.end() > m.start() {
            found.push((m.start(), m.end()));
        }
        true
    });
    let &(first_start, first_end) = found.first()?;
    let cols = (content[..first_start].chars().count(), content[..first_end].chars().count());

    // Trim leading whitespace and cut very long lines around the first match.
    let lead = content.len() - content.trim_start().len();
    let mut start = lead;
    if first_start > start + MAX_LINE_DISPLAY / 2 {
        start = first_start - MAX_LINE_DISPLAY / 4;
        while !content.is_char_boundary(start) {
            start -= 1;
        }
    }
    let mut end = content.len().min(start + MAX_LINE_DISPLAY);
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let text = content[start..end].trim_end().to_owned();
    let ranges = found
        .into_iter()
        .filter(|&(s, e)| s >= start && e <= start + text.len())
        .map(|(s, e)| (s - start, e - start))
        .collect();
    Some(LineMatch { line, text, ranges, cols })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn search(root: &Path, q: Query) -> (Vec<FileResult>, Option<String>) {
        let (tx, rx) = channel();
        run_search(&q, root, &tx, &Arc::new(AtomicBool::new(false)));
        drop(tx);
        let mut files = Vec::new();
        let mut err = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while let Ok(msg) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            match msg {
                Msg::File(f) => files.push(f),
                Msg::Error(e) => err = Some(e),
                Msg::Done { .. } => break,
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files, err)
    }

    fn fixture() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rust-code-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {\n    let Foo = foo_bar();\n}\n").unwrap();
        std::fs::write(dir.join("notes.md"), "foo in markdown\n").unwrap();
        std::fs::write(dir.join("target/out.rs"), "foo").unwrap();
        std::fs::write(dir.join("bin.dat"), b"foo\0\x01\x02").unwrap();
        dir
    }

    #[test]
    fn literal_case_word_regex_and_globs() {
        let dir = fixture();
        let q = |text: &str| Query { text: text.to_owned(), ..Default::default() };

        let (files, _) = search(&dir, q("foo"));
        let names: Vec<String> =
            files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        // Sorted by path: <dir>/notes.md before <dir>/src/a.rs. target/ and the binary file are skipped.
        assert_eq!(names, ["notes.md", "a.rs"]);
        let a = files.iter().find(|f| f.path.ends_with("a.rs")).unwrap();
        assert_eq!(a.matches[0].line, 2);
        assert_eq!(a.matches[0].text, "let Foo = foo_bar();", "leading whitespace trimmed");
        assert_eq!(a.matches[0].ranges.len(), 2, "case-insensitive by default");
        assert_eq!(a.matches[0].cols, (8, 11));

        let (files, _) = search(&dir, Query { match_case: true, ..q("Foo") });
        assert_eq!(files.iter().map(|f| f.matches.len()).sum::<usize>(), 1);

        let (files, _) = search(&dir, Query { whole_word: true, ..q("foo") });
        let a = files.iter().find(|f| f.path.ends_with("a.rs")).unwrap();
        assert_eq!(a.matches[0].ranges, vec![(4, 7)], "foo_bar is not the word foo");

        let (files, _) = search(&dir, Query { regex: true, ..q(r"foo_\w+") });
        assert_eq!(files.len(), 1);

        let (files, _) = search(&dir, Query { include: "*.md".to_owned(), ..q("foo") });
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("notes.md"));

        let (files, _) = search(&dir, Query { exclude: "*.md".to_owned(), ..q("foo") });
        assert!(files.iter().all(|f| !f.path.ends_with("notes.md")));

        let (_, err) = search(&dir, Query { regex: true, ..q("(") });
        assert!(err.is_some(), "invalid regex is reported");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let matcher = build_matcher(&Query { text: "needle".to_owned(), ..Default::default() }).unwrap();
        let line = format!("{}needle{}", "x".repeat(1000), "y".repeat(1000));
        let m = line_match(&matcher, 1, &line).unwrap();
        assert!(m.text.len() <= MAX_LINE_DISPLAY);
        assert_eq!(&m.text[m.ranges[0].0..m.ranges[0].1], "needle");
        assert_eq!(m.cols, (1000, 1006));
    }
}
