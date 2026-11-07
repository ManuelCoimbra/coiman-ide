//! Test coverage reports, in the formats tools write: which lines ran, by file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde_json::Value;

use crate::stacktrace::{self, Frame, Place};

/// One line of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineCoverage {
    /// From 1.
    pub line: u32,
    pub hits: u64,
    /// It ran, but not every branch on it did.
    pub partial: bool,
}

/// A report: lines by file, named as the report names them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub files: BTreeMap<String, Vec<LineCoverage>>,
    /// Folders Cobertura says its file names start from.
    pub sources: Vec<String>,
    /// (ran, all), where the report counts them.
    pub branches: (u64, u64),
    pub functions: (u64, u64),
}

impl Report {
    fn add(&mut self, file: &str, line: u32, hits: u64, partial: bool) {
        let lines = self.files.entry(file.to_owned()).or_default();
        // Several test runs in one report: a line ran if any of them ran it.
        match lines.iter_mut().find(|l| l.line == line) {
            Some(old) => {
                old.partial = if old.hits == 0 || hits == 0 { old.partial || partial } else { old.partial && partial };
                old.hits += hits;
            }
            None => lines.push(LineCoverage { line, hits, partial }),
        }
    }

    fn finish(mut self) -> Self {
        for lines in self.files.values_mut() {
            lines.sort_by_key(|l| l.line);
        }
        self
    }
}

/// (covered lines, lines): partial lines count as covered.
pub fn counts(lines: &[LineCoverage]) -> (usize, usize) {
    (lines.iter().filter(|l| l.hits > 0).count(), lines.len())
}

pub fn percent(covered: usize, total: usize) -> f64 {
    if total == 0 { 0.0 } else { covered as f64 / total as f64 * 100.0 }
}

/// Marks for a file's report, moved to where their lines are now. Lines that were
/// changed or removed lose theirs: nothing is known about them.
pub fn follow_lines(lines: &[LineCoverage], old: &str, new: &str) -> Vec<LineCoverage> {
    let (old, new) = (old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
    let diff = similar::TextDiff::from_lines(&old, &new);
    let mut moved = Vec::with_capacity(lines.len());
    for op in diff.ops() {
        if let similar::DiffOp::Equal { old_index, new_index, len } = *op {
            for line in lines {
                let at = line.line as usize;
                if at > old_index && at <= old_index + len {
                    moved.push(LineCoverage { line: (at - old_index + new_index) as u32, ..*line });
                }
            }
        }
    }
    moved
}

/// LCOV: `SF:` starts a file, `DA:line,hits` a line, `BRDA:line,block,branch,taken` a branch.
pub fn parse_lcov(text: &str) -> Report {
    let mut report = Report::default();
    let mut file = String::new();
    let mut untaken: Vec<u32> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix("SF:") {
            file = name.to_owned();
            untaken.clear();
        } else if let Some(rest) = line.strip_prefix("DA:") {
            let mut parts = rest.split(',');
            let (Some(number), Some(hits)) = (parts.next(), parts.next()) else { continue };
            let (Ok(number), Ok(hits)) = (number.parse(), hits.parse()) else { continue };
            report.add(&file, number, hits, false);
        } else if let Some(rest) = line.strip_prefix("BRDA:") {
            let parts: Vec<&str> = rest.split(',').collect();
            if parts.len() == 4
                && matches!(parts[3], "-" | "0")
                && let Ok(number) = parts[0].parse()
            {
                untaken.push(number);
            }
        } else if let Some(n) = line.strip_prefix("BRH:").and_then(|n| n.parse::<u64>().ok()) {
            report.branches.0 += n;
        } else if let Some(n) = line.strip_prefix("BRF:").and_then(|n| n.parse::<u64>().ok()) {
            report.branches.1 += n;
        } else if let Some(n) = line.strip_prefix("FNH:").and_then(|n| n.parse::<u64>().ok()) {
            report.functions.0 += n;
        } else if let Some(n) = line.strip_prefix("FNF:").and_then(|n| n.parse::<u64>().ok()) {
            report.functions.1 += n;
        } else if line == "end_of_record"
            && let Some(lines) = report.files.get_mut(&file)
        {
            for number in untaken.drain(..) {
                if let Some(l) = lines.iter_mut().find(|l| l.line == number && l.hits > 0) {
                    l.partial = true;
                }
            }
        }
    }
    report.finish()
}

/// A Go cover profile: `path/file.go:from.col,to.col statements count`.
pub fn parse_go(text: &str) -> Report {
    let mut report = Report::default();
    for line in text.lines().filter(|l| !l.starts_with("mode:")) {
        let Some((place, numbers)) = line.rsplit_once(':') else { continue };
        let mut words = numbers.split_whitespace();
        let (Some(span), Some(_), Some(count)) = (words.next(), words.next(), words.next()) else { continue };
        let Some((from, to)) = span.split_once(',') else { continue };
        let line_of = |at: &str| at.split('.').next().and_then(|n| n.parse::<u32>().ok());
        let (Some(from), Some(to), Ok(count)) = (line_of(from), line_of(to), count.parse::<u64>()) else { continue };
        for number in from..=to {
            report.add(place, number, count, false);
        }
    }
    report.finish()
}

/// coverage.py's JSON.
pub fn parse_coverage_py(json: &Value) -> Report {
    let mut report = Report::default();
    let total = |name: &str| json["totals"][name].as_u64().unwrap_or(0);
    report.branches = (total("covered_branches"), total("num_branches"));
    let Some(files) = json["files"].as_object() else { return report };
    let numbers = |v: &Value| -> Vec<u32> {
        v.as_array().into_iter().flatten().filter_map(|n| n.as_u64()).map(|n| n as u32).collect()
    };
    for (name, file) in files {
        let partial: Vec<u32> = file["missing_branches"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|pair| pair[0].as_i64().filter(|n| *n > 0).map(|n| n as u32))
            .collect();
        for number in numbers(&file["executed_lines"]) {
            report.add(name, number, 1, partial.contains(&number));
        }
        for number in numbers(&file["missing_lines"]) {
            report.add(name, number, 0, false);
        }
    }
    report.finish()
}

fn attr(tag: &BytesStart, name: &str) -> Option<String> {
    let found = tag.attributes().flatten().find(|a| a.key.as_ref() == name.as_bytes())?;
    Some(String::from_utf8_lossy(&found.value).into_owned())
}

/// Cobertura XML.
pub fn parse_cobertura(xml: &str) -> Report {
    let mut report = Report::default();
    let mut reader = Reader::from_str(xml);
    let mut file = String::new();
    let mut in_source = false;
    let mut in_method = 0;
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => match tag.name().as_ref() {
                b"class" => file = attr(&tag, "filename").unwrap_or_default(),
                b"coverage" => {
                    let number = |key: &str| attr(&tag, key).and_then(|v| v.parse().ok()).unwrap_or(0);
                    report.branches = (number("branches-covered"), number("branches-valid"));
                }
                b"source" => in_source = true,
                b"method" => in_method += 1,
                _ => {}
            },
            Ok(Event::Empty(tag)) if tag.name().as_ref() == b"line" && in_method == 0 => {
                let number = attr(&tag, "number").and_then(|n| n.parse().ok());
                let hits = attr(&tag, "hits").and_then(|n| n.parse().ok()).unwrap_or(0);
                // `condition-coverage="50% (1/2)"` says some branches didn't run.
                let partial = attr(&tag, "condition-coverage").is_some_and(|c| !c.starts_with("100%"));
                if let Some(number) = number {
                    report.add(&file, number, hits, partial && hits > 0);
                }
            }
            Ok(Event::Text(text)) if in_source => {
                if let Ok(text) = text.decode() {
                    report.sources.push(text.trim().to_owned());
                }
            }
            Ok(Event::End(tag)) => match tag.name().as_ref() {
                b"source" => in_source = false,
                b"method" => in_method -= 1,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    report.finish()
}

/// JaCoCo XML: files by package, with missed and covered instructions per line.
pub fn parse_jacoco(xml: &str) -> Report {
    let mut report = Report::default();
    let mut reader = Reader::from_str(xml);
    let (mut package, mut file) = (String::new(), String::new());
    loop {
        match reader.read_event() {
            Ok(Event::Start(tag)) => match tag.name().as_ref() {
                b"package" => package = attr(&tag, "name").unwrap_or_default(),
                b"sourcefile" => {
                    let name = attr(&tag, "name").unwrap_or_default();
                    file = if package.is_empty() { name } else { format!("{package}/{name}") };
                }
                _ => {}
            },
            Ok(Event::Empty(tag)) if tag.name().as_ref() == b"line" && !file.is_empty() => {
                let number = |key: &str| attr(&tag, key).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let hits = number("ci");
                report.add(&file, number("nr") as u32, hits, hits > 0 && number("mb") > 0);
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    report.finish()
}

/// Reads a report in whichever format it's in, by its content, not its name.
pub fn parse(text: &str) -> Result<Report, String> {
    let head = text.trim_start();
    let report = if head.starts_with('{') {
        let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        parse_coverage_py(&json)
    } else if head.starts_with('<') {
        if head.contains("<coverage") { parse_cobertura(text) } else { parse_jacoco(text) }
    } else if head.starts_with("mode:") {
        parse_go(text)
    } else {
        parse_lcov(text)
    };
    if report.files.is_empty() {
        return Err("no coverage found in it".to_owned());
    }
    Ok(report)
}

/// Where tools usually leave reports, from the project's folder.
pub const REPORT_PLACES: &[&str] = &[
    "lcov.info",
    "coverage/lcov.info",
    "coverage.lcov",
    "coverage.xml",
    "cobertura.xml",
    "coverage/cobertura-coverage.xml",
    "coverage.json",
    "coverage.out",
    "cover.out",
    "target/site/jacoco/jacoco.xml",
    "build/reports/jacoco/test/jacocoTestReport.xml",
    "target/llvm-cov/lcov.info",
];

/// Names a report takes inside a folder a test run was told to write in.
fn is_report_name(name: &str) -> bool {
    matches!(
        name,
        "lcov.info"
            | "coverage.out"
            | "coverage.cobertura.xml"
            | "coverage.xml"
            | "cobertura.xml"
            | "coverage.json"
            | "cobertura-coverage.xml"
    )
}

/// The report a run left somewhere in this folder.
pub fn find_report_in(dir: &Path) -> Option<PathBuf> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut folders = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.is_dir() {
            folders.push(path);
        } else if path.file_name().is_some_and(|n| is_report_name(&n.to_string_lossy())) {
            return Some(path);
        }
    }
    folders.into_iter().find_map(|f| find_report_in(&f))
}

/// The newest report around a project.
pub fn newest_report(root: &Path) -> Option<(PathBuf, std::time::SystemTime)> {
    REPORT_PLACES
        .iter()
        .map(|p| root.join(p))
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, modified))
        })
        .max_by_key(|(_, m)| *m)
}

/// A report's files, matched to the project's. Returns those it couldn't place too.
pub fn resolve(report: Report, root: &Path, files: &[PathBuf]) -> (Vec<(PathBuf, Vec<LineCoverage>)>, Vec<String>) {
    let mut found: Vec<(PathBuf, Vec<LineCoverage>)> = Vec::new();
    let mut lost = Vec::new();
    for (name, lines) in report.files {
        let inside = report.sources.iter().map(|s| Path::new(s).join(&name)).find(|p| p.is_file());
        let frame = |file: String| Frame { language: "", symbol: None, file, folder_hint: None, line: 1, column: None };
        let bare = name.strip_prefix("file://").map_or(name.as_str(), |rest| rest.trim_start_matches('/'));
        let place = match inside {
            Some(path) => Place::Project(path),
            None => stacktrace::locate(&frame(bare.to_owned()), root, files),
        };
        match place {
            Place::Project(path) | Place::Maybe(path, 0) => found.push((path, lines)),
            _ => lost.push(name),
        }
    }
    (found, lost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_follow_edits() {
        let mark = |line, hits| LineCoverage { line, hits, partial: false };
        let lines = [mark(1, 1), mark(2, 0), mark(3, 1)];
        let moved = follow_lines(&lines, "a\r\nb\nc\n", "new\na\nc\n");
        assert_eq!(moved, [mark(2, 1), mark(3, 1)], "line 2 went, and the rest moved");
    }
    use serde_json::json;

    fn hits(report: &Report, file: &str) -> Vec<(u32, u64, bool)> {
        report.files[file].iter().map(|l| (l.line, l.hits, l.partial)).collect()
    }

    #[test]
    fn reads_lcov() {
        let text = "TN:\nSF:src/a.js\nDA:1,3\nDA:2,0\nDA:3,1\nBRDA:3,0,0,1\nBRDA:3,0,1,0\nend_of_record\nSF:b.js\nDA:9,1\nend_of_record\n";
        let report = parse(text).unwrap();
        assert_eq!(hits(&report, "src/a.js"), [(1, 3, false), (2, 0, false), (3, 1, true)]);
        assert_eq!(counts(&report.files["src/a.js"]), (2, 3));
        assert_eq!(hits(&report, "b.js"), [(9, 1, false)]);
    }

    #[test]
    fn an_untaken_branch_on_an_unrun_line_is_just_unrun() {
        let report = parse("SF:a\nDA:1,0\nBRDA:1,0,0,-\nend_of_record\n").unwrap();
        assert_eq!(hits(&report, "a"), [(1, 0, false)]);
    }

    #[test]
    fn reads_go_profiles() {
        let text = "mode: set\nexample.com/m/a.go:3.10,5.2 2 1\nexample.com/m/a.go:7.1,7.9 1 0\n";
        let report = parse(text).unwrap();
        assert_eq!(hits(&report, "example.com/m/a.go"), [(3, 1, false), (4, 1, false), (5, 1, false), (7, 0, false)]);
    }

    #[test]
    fn reads_coverage_py() {
        let text =
            json!({"files": {"a.py": {"executed_lines": [1, 2], "missing_lines": [3], "missing_branches": [[2, 3]]}}})
                .to_string();
        let report = parse(&text).unwrap();
        assert_eq!(hits(&report, "a.py"), [(1, 1, false), (2, 1, true), (3, 0, false)]);
    }

    #[test]
    fn reads_cobertura() {
        let xml = r#"<?xml version="1.0"?><coverage><sources><source>/w/src</source></sources><packages><package><classes>
<class filename="a.cs"><methods><method name="m"><lines><line number="1" hits="9"/></lines></method></methods>
<lines><line number="1" hits="2"/><line number="2" hits="0"/><line number="3" hits="1" branch="true" condition-coverage="50% (1/2)"/></lines></class>
<class filename="a.cs"><lines><line number="1" hits="1"/></lines></class>
</classes></package></packages></coverage>"#;
        let report = parse(xml).unwrap();
        assert_eq!(report.sources, ["/w/src"]);
        assert_eq!(hits(&report, "a.cs"), [(1, 3, false), (2, 0, false), (3, 1, true)]);
    }

    #[test]
    fn reads_jacoco() {
        let xml = r#"<report><package name="com/shop"><class name="x"><method name="m"/></class><sourcefile name="Cart.java"><line nr="3" mi="0" ci="4" mb="1" cb="1"/><line nr="4" mi="2" ci="0" mb="0" cb="0"/></sourcefile></package></report>"#;
        let report = parse(xml).unwrap();
        assert_eq!(hits(&report, "com/shop/Cart.java"), [(3, 4, true), (4, 0, false)]);
    }

    #[test]
    fn says_when_there_is_none() {
        assert!(parse("").is_err());
        assert!(parse("plain words").is_err());
        assert!(parse("{ nope").is_err());
        assert!(parse("<coverage></coverage>").is_err());
    }

    #[test]
    fn merges_runs_of_one_line() {
        let report = parse("SF:a\nDA:1,0\nend_of_record\nSF:a\nDA:1,2\nend_of_record\n").unwrap();
        assert_eq!(hits(&report, "a"), [(1, 2, false)]);
    }

    #[test]
    fn places_files_from_other_machines() {
        let root = std::env::temp_dir().join(format!("rc-cov-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.py"), "").unwrap();
        let files = vec![root.join("src/a.py")];
        let report = parse("SF:/ci/build/src/a.py\nDA:1,1\nend_of_record\nSF:/usr/lib/python3/os.py\nDA:1,1\nend_of_record\nSF:gone.py\nDA:1,1\nend_of_record\n").unwrap();
        let (found, lost) = resolve(report, &root, &files);
        assert_eq!(found[0].0, root.join("src/a.py"));
        assert_eq!(found.len(), 1);
        assert_eq!(lost.len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_a_run_report_at_any_depth() {
        let root = std::env::temp_dir().join(format!("rc-cov-find-{}", std::process::id()));
        std::fs::create_dir_all(root.join("guid")).unwrap();
        std::fs::write(root.join("guid/coverage.cobertura.xml"), "x").unwrap();
        assert_eq!(find_report_in(&root), Some(root.join("guid/coverage.cobertura.xml")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn computes_percent() {
        assert_eq!(percent(1, 4), 25.0);
        assert_eq!(percent(0, 0), 0.0);
    }
}
