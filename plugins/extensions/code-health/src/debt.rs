//! Technical debt left as notes in the code: `TODO`, `FIXME`, `HACK` and `XXX`, where they are
//! (by area: the project's top folders) and how old (from `git blame`: when each line was
//! written). The months the ones still here were written show how debt piled up over time.

use std::collections::BTreeMap;

/// Files larger than this aren't read.
pub const MAX_BYTES: u64 = 1024 * 1024;

pub const KINDS: [&str; 4] = ["FIXME", "HACK", "XXX", "TODO"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    /// Relative to the project, with `/`.
    pub path: String,
    /// 1-based.
    pub line: usize,
    /// FIXME, HACK, XXX or TODO.
    pub kind: &'static str,
    /// What follows the mark.
    pub text: String,
    /// When the line was written (seconds since 1970), from `git blame`.
    pub written: Option<i64>,
}

/// Where a mark counts: in a comment, followed by `:` `(` or a space, in capitals.
fn mark_at(line: &str) -> Option<(&'static str, usize)> {
    let comment = ["//", "#", "/*", "*", "--", "<!--", ";", "%", "'"].iter().filter_map(|c| line.find(c)).min()?;
    let after = &line[comment..];
    for kind in KINDS {
        let mut from = 0;
        while let Some(i) = after[from..].find(kind) {
            let at = from + i;
            let before = after[..at].chars().last();
            let next = after[at + kind.len()..].chars().next();
            let word_start = before.is_none_or(|c| !c.is_alphanumeric() && c != '_');
            let word_end = next.is_none_or(|c| matches!(c, ':' | '(' | ' ' | '!' | '-' | '\t'));
            if word_start && word_end {
                return Some((kind, comment + at + kind.len()));
            }
            from = at + kind.len();
        }
    }
    None
}

/// The notes in one file's text.
pub fn scan_text(path: &str, text: &str) -> Vec<Note> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.len() > 2000 {
            continue;
        }
        let Some((kind, end)) = mark_at(line) else { continue };
        let rest = line[end..].trim_start_matches(['(', ')', ':', '!', '-', ' ', '\t']);
        // An owner in brackets stays: TODO(ann): …
        let rest = rest.trim_end_matches("*/").trim_end_matches("-->").trim();
        let text: String = rest.chars().take(140).collect();
        out.push(Note { path: path.to_owned(), line: i + 1, kind, text, written: None });
    }
    out
}

/// `git blame --line-porcelain` output: each line's author time, by its line number.
pub fn blame_times(porcelain: &str) -> BTreeMap<usize, i64> {
    let mut out = BTreeMap::new();
    let mut line = None;
    for l in porcelain.lines() {
        if l.starts_with('\t') {
            continue;
        }
        let mut parts = l.split(' ');
        let first = parts.next().unwrap_or("");
        if first.len() == 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
            // <sha> <line in the original> <line now> [<lines in group>]
            line = parts.nth(1).and_then(|n| n.parse::<usize>().ok());
        } else if first == "author-time"
            && let (Some(n), Some(t)) = (line, parts.next().and_then(|t| t.parse::<i64>().ok()))
        {
            out.insert(n, t);
        }
    }
    out
}

/// The age buckets, newest first, with their upper bound in days.
pub const AGES: [(&str, i64); 5] =
    [("under a month", 31), ("1–6 months", 183), ("6–12 months", 366), ("1–2 years", 731), ("over 2 years", i64::MAX)];

pub fn age_bucket(written: i64, now: i64) -> &'static str {
    let days = (now - written).max(0) / 86_400;
    AGES.iter().find(|(_, max)| days < *max).map_or(AGES[AGES.len() - 1].0, |(name, _)| name)
}

/// The area a file is in: its first folder, or its first two when the first is a usual
/// container (`src`, `crates`, `packages`…); files at the top are "(top)".
pub fn area(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 {
        return "(top)".into();
    }
    let containers = ["src", "crates", "packages", "apps", "libs", "lib", "modules", "plugins", "services"];
    if containers.contains(&parts[0]) && parts.len() > 2 {
        format!("{}/{}", parts[0], parts[1])
    } else {
        parts[0].to_owned()
    }
}

/// `YYYY-MM` of a time (UTC).
pub fn month(t: i64) -> String {
    // Civil from days (Howard Hinnant's algorithm).
    let z = t.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}")
}

/// How many notes each area has, most first, with how many of each kind.
pub fn by_area(notes: &[Note]) -> Vec<(String, usize, BTreeMap<&'static str, usize>)> {
    let mut map: BTreeMap<String, BTreeMap<&'static str, usize>> = BTreeMap::new();
    for n in notes {
        *map.entry(area(&n.path)).or_default().entry(n.kind).or_default() += 1;
    }
    let mut out: Vec<_> = map.into_iter().map(|(a, k)| (a, k.values().sum::<usize>(), k)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// How many of the notes still here were written each month, oldest first.
pub fn by_month(notes: &[Note]) -> Vec<(String, usize)> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for t in notes.iter().filter_map(|n| n.written) {
        *map.entry(month(t)).or_default() += 1;
    }
    map.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_in_comments() {
        let text = "fn a() {} // TODO: split this\n\
                    let todo = 1; // not a TODOlist\n\
                    # FIXME(ann) breaks on Windows\n\
                    /* HACK: until the API has it */\n\
                    s = \"TODO in a string\"\n\
                    <!-- XXX remove -->\n\
                    // todo lower case isn't one\n";
        let found: Vec<(usize, &str, String)> =
            scan_text("a.rs", text).into_iter().map(|n| (n.line, n.kind, n.text)).collect();
        assert_eq!(
            found,
            [
                (1, "TODO", "split this".to_owned()),
                (3, "FIXME", "ann) breaks on Windows".to_owned()),
                (4, "HACK", "until the API has it".to_owned()),
                (6, "XXX", "remove".to_owned()),
            ]
        );
    }

    #[test]
    fn blame_gives_each_lines_time() {
        let porcelain = "0123456789012345678901234567890123456789 1 1 2\n\
                         author Ann\nauthor-time 1700000000\nauthor-tz +0000\nfilename a.rs\n\tfn a() {}\n\
                         0123456789012345678901234567890123456789 2 2\n\
                         author Ann\nauthor-time 1700000000\nfilename a.rs\n\t// TODO\n\
                         abcdefabcdefabcdefabcdefabcdefabcdefabcd 7 3 1\n\
                         author Bob\nauthor-time 1760000000\nfilename a.rs\n\t// FIXME\n";
        let t = blame_times(porcelain);
        assert_eq!(t.get(&1), Some(&1_700_000_000));
        assert_eq!(t.get(&2), Some(&1_700_000_000));
        assert_eq!(t.get(&3), Some(&1_760_000_000));
    }

    #[test]
    fn ages_areas_and_months() {
        let now = 1_764_000_000;
        assert_eq!(age_bucket(now - 5 * 86_400, now), "under a month");
        assert_eq!(age_bucket(now - 100 * 86_400, now), "1–6 months");
        assert_eq!(age_bucket(now - 400 * 86_400, now), "1–2 years");
        assert_eq!(age_bucket(now - 1000 * 86_400, now), "over 2 years");
        assert_eq!(area("crates/core/src/a.rs"), "crates/core");
        assert_eq!(area("web/app.js"), "web");
        assert_eq!(area("main.c"), "(top)");
        assert_eq!(month(0), "1970-01");
        assert_eq!(month(1_764_000_000), "2025-11");
        assert_eq!(month(951_782_400), "2000-02-29"[..7].to_owned());
        let n = |path: &str, kind, written| Note { path: path.into(), line: 1, kind, text: String::new(), written };
        let notes = [
            n("web/a.js", "TODO", Some(1_700_000_000)),
            n("web/b.js", "FIXME", Some(1_700_100_000)),
            n("src/x/c.rs", "TODO", Some(1_760_000_000)),
            n("main.c", "HACK", None),
        ];
        let areas = by_area(&notes);
        assert_eq!(areas[0].0, "web");
        assert_eq!(areas[0].1, 2);
        assert_eq!(by_month(&notes), [("2023-11".to_owned(), 2), ("2025-10".to_owned(), 1)]);
    }
}
