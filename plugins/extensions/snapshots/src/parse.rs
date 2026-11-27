//! Reading what git prints, with no host calls (tested on its own).

/// A snapshot: a tag in the private repository, on a commit with no parent.
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub tag: String,
    /// Seconds since 1970 (UTC).
    pub time: i64,
    pub name: String,
}

/// `git for-each-ref refs/tags --format=%(refname:short)%09%(creatordate:unix)%09%(subject)`.
pub fn snapshots(out: &str) -> Vec<Snap> {
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let tag = parts.next()?.trim();
            let time = parts.next()?.trim().parse().ok()?;
            let name = parts.next().unwrap_or("").trim();
            (!tag.is_empty()).then(|| Snap { tag: tag.to_owned(), time, name: name.to_owned() })
        })
        .collect()
}

/// A file that differs between the two sides.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub path: String,
    /// 'A' (only on the right), 'D' (only on the left), 'M', or 'T' (its kind changed).
    pub status: char,
    /// Lines added and removed; None for a binary file.
    pub lines: Option<(u32, u32)>,
}

/// `git diff --raw --numstat --no-renames`: the raw lines (":…\tpath") then the numstat lines.
pub fn changes(out: &str) -> Vec<Change> {
    let mut list: Vec<Change> = Vec::new();
    for l in out.lines() {
        if let Some(raw) = l.strip_prefix(':') {
            let Some((meta, path)) = raw.split_once('\t') else { continue };
            let status = meta.split_whitespace().nth(4).and_then(|s| s.chars().next()).unwrap_or('M');
            list.push(Change { path: path.to_owned(), status, lines: None });
        } else {
            let mut parts = l.splitn(3, '\t');
            let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
            if let (Ok(a), Ok(r)) = (a.parse(), r.parse())
                && let Some(c) = list.iter_mut().find(|c| c.path == path)
            {
                c.lines = Some((a, r));
            }
        }
    }
    list.sort_by(|a, b| a.path.cmp(&b.path));
    list
}

/// FNV-1a: the open folder's own subfolder of the plugin's data folder.
pub fn hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// "2026-11-27 14:05:33", local time.
pub fn when(unix: i64, offset_minutes: i32) -> String {
    let t = unix + i64::from(offset_minutes) * 60;
    // Civil from days (Howard Hinnant's algorithm).
    let z = t.div_euclid(86_400) + 719_468;
    let secs = t.rem_euclid(86_400);
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// A snapshot's name: one line, no tabs, not too long; "Snapshot" when empty.
pub fn clean_name(name: &str) -> String {
    let one: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let one = one.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = one.chars().take(80).collect();
    if cut.is_empty() { "Snapshot".to_owned() } else { cut }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_snapshots() {
        let s = snapshots("s1795812413-ab12\t1795812413\tBefore the refactor\nbad line\ns2\t5\t\n");
        assert_eq!(s.len(), 2);
        assert_eq!(
            s[0],
            Snap { tag: "s1795812413-ab12".into(), time: 1_795_812_413, name: "Before the refactor".into() }
        );
        assert_eq!(s[1].name, "");
    }

    #[test]
    fn reads_changes() {
        let out = ":100644 100644 7898192 422c2b7 M\tsrc/a.txt\n\
                   :000000 100644 0000000 8ba3a16 A\tnew.txt\n\
                   :100644 000000 1111111 0000000 D\tlogo.png\n\
                   1\t0\tsrc/a.txt\n3\t0\tnew.txt\n-\t-\tlogo.png\n";
        let c = changes(out);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0], Change { path: "logo.png".into(), status: 'D', lines: None });
        assert_eq!(c[1], Change { path: "new.txt".into(), status: 'A', lines: Some((3, 0)) });
        assert_eq!(c[2], Change { path: "src/a.txt".into(), status: 'M', lines: Some((1, 0)) });
    }

    #[test]
    fn times_and_names() {
        assert_eq!(when(0, 0), "1970-01-01 00:00:00");
        assert_eq!(when(1_795_812_413, 60), "2026-11-27 21:46:53");
        assert_eq!(clean_name("  a\tb\nc  "), "a b c");
        assert_eq!(clean_name(""), "Snapshot");
        assert_ne!(hash("C:/a"), hash("C:/b"));
    }
}
