//! Reading benchmark results from what the tools print, and the recorded history. No host
//! calls: tested on its own.

/// A run: when, at which commit, and each benchmark's time in nanoseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The project it belongs to (a hash of its folder).
    pub project: String,
    /// Short commit hash, with "+" when there were changes not committed; "" outside Git.
    pub commit: String,
    /// Seconds since 1970 (UTC).
    pub time: i64,
    pub results: Vec<(String, f64)>,
}

/// Nanoseconds from a number and its unit ("ns", "µs", "us", "ms", "s", "ps").
fn nanos(value: &str, unit: &str) -> Option<f64> {
    let v: f64 = value.replace(',', "").parse().ok()?;
    let scale = match unit {
        "ps" => 0.001,
        "ns" => 1.0,
        "µs" | "us" | "μs" => 1e3,
        "ms" => 1e6,
        "s" => 1e9,
        _ => return None,
    };
    Some(v * scale)
}

/// Results from `cargo bench` (Criterion's "name  time: [low mid high]", its name on the line
/// before when long, and libtest's "test name ... bench: 1,234 ns/iter") and from
/// `go test -bench` ("BenchmarkName-8  1000  1234 ns/op"), and a table of a harness of its own
/// whose header names a `p50` column (each row's first time is its p50).
pub fn parse(out: &str) -> Vec<(String, f64)> {
    let mut found: Vec<(String, f64)> = Vec::new();
    let mut previous = "";
    let mut p50_table = false;
    for line in out.lines() {
        let t = line.trim();
        if t.split_whitespace().any(|w| w == "p50") {
            p50_table = true;
            continue;
        }
        if t.is_empty() {
            p50_table = false;
        }
        if p50_table && !t.starts_with('-') {
            // The first "number unit" pair; the name is what comes before it.
            let words: Vec<&str> = t.split_whitespace().collect();
            let at = words.windows(2).position(|w| nanos(w[0], w[1]).is_some());
            if let Some(i) = at
                && i > 0
                && let Some(ns) = nanos(words[i], words[i + 1])
            {
                found.push((words[..i].join(" "), ns));
            }
            continue;
        }
        if let Some(at) = t.find("time:")
            && let Some(open) = t[at..].find('[')
        {
            // Criterion: the middle of the three estimates.
            let inside = t[at + open + 1..].trim_end_matches(']');
            let parts: Vec<&str> = inside.split_whitespace().collect();
            let name = t[..at].trim();
            let name = if name.is_empty() { previous } else { name };
            if parts.len() >= 4
                && !name.is_empty()
                && let Some(ns) = nanos(parts[2], parts[3])
            {
                found.push((name.to_owned(), ns));
            }
        } else if let Some(rest) = t.strip_prefix("test ")
            && let Some((name, bench)) = rest.split_once(" ... bench:")
        {
            let mut words = bench.split_whitespace();
            if let (Some(v), Some(unit)) = (words.next(), words.next())
                && let Some(ns) = nanos(v, unit.trim_end_matches("/iter"))
            {
                found.push((name.trim().to_owned(), ns));
            }
        } else if t.starts_with("Benchmark") && t.contains("ns/op") {
            let words: Vec<&str> = t.split_whitespace().collect();
            if let Some(i) = words.iter().position(|w| *w == "ns/op")
                && i >= 1
                && let Ok(ns) = words[i - 1].parse::<f64>()
            {
                let raw = words[0].trim_start_matches("Benchmark");
                // "Fib20-8": the -8 is GOMAXPROCS.
                let name = match raw.rsplit_once('-') {
                    Some((n, cpus)) if cpus.bytes().all(|b| b.is_ascii_digit()) => n,
                    _ => raw,
                };
                found.push((name.to_owned(), ns));
            }
        }
        if !t.is_empty() && !t.starts_with("Benchmarking") && !t.contains("time:") {
            previous = t;
        }
    }
    // A name seen twice (a re-run): the last one.
    let mut out: Vec<(String, f64)> = Vec::new();
    for (n, v) in found {
        match out.iter_mut().find(|(m, _)| *m == n) {
            Some(slot) => slot.1 = v,
            None => out.push((n, v)),
        }
    }
    out
}

/// "1.23 ms", "456 ns".
pub fn human(ns: f64) -> String {
    let (v, unit) = if ns >= 1e9 {
        (ns / 1e9, "s")
    } else if ns >= 1e6 {
        (ns / 1e6, "ms")
    } else if ns >= 1e3 {
        (ns / 1e3, "µs")
    } else {
        (ns, "ns")
    };
    if v >= 100.0 {
        format!("{v:.0} {unit}")
    } else if v >= 10.0 {
        format!("{v:.1} {unit}")
    } else {
        format!("{v:.2} {unit}")
    }
}

/// How much slower `new` is than `old`, in percent (negative: faster).
pub fn change(old: f64, new: f64) -> f64 {
    if old <= 0.0 { 0.0 } else { (new - old) / old * 100.0 }
}

/// The history as stored: one line per result, "project\tcommit\ttime\tname\tns".
pub fn read_history(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for line in text.lines() {
        let p: Vec<&str> = line.split('\t').collect();
        let [project, commit, time, name, ns] = p[..] else { continue };
        let (Ok(time), Ok(ns)) = (time.parse::<i64>(), ns.parse::<f64>()) else { continue };
        match runs.last_mut() {
            Some(r) if r.project == project && r.time == time && r.commit == commit => {
                r.results.push((name.to_owned(), ns))
            }
            _ => runs.push(Run {
                project: project.to_owned(),
                commit: commit.to_owned(),
                time,
                results: vec![(name.to_owned(), ns)],
            }),
        }
    }
    runs
}

/// At most `keep` runs per project, oldest dropped first.
pub fn write_history(runs: &[Run], keep: usize) -> String {
    let mut out = String::new();
    for (i, r) in runs.iter().enumerate() {
        let later = runs[i + 1..].iter().filter(|x| x.project == r.project).count();
        if later >= keep {
            continue;
        }
        for (name, ns) in &r.results {
            let name = name.replace(['\t', '\n'], " ");
            out.push_str(&format!("{}\t{}\t{}\t{name}\t{ns}\n", r.project, r.commit, r.time));
        }
    }
    out
}

/// The benchmarks of `new` slower than in `old` by more than `threshold` percent.
pub fn regressions(old: &Run, new: &Run, threshold: f64) -> Vec<(String, f64)> {
    new.results
        .iter()
        .filter_map(|(n, v)| {
            let (_, o) = old.results.iter().find(|(m, _)| m == n)?;
            let c = change(*o, *v);
            (c > threshold).then(|| (n.clone(), c))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn criterion() {
        let out = "Benchmarking fib 20: Warming up for 3.0000 s\n\
                   fib 20                  time:   [26.029 µs 26.251 µs 26.505 µs]\n\
                   Found 2 outliers\n\
                   a very long benchmark name that wraps\n\
                   \x20                       time:   [1.5000 ms 1.6000 ms 1.7000 ms]\n\
                   \x20                       change: [-1.2% +0.3% +1.8%] (p = 0.71 > 0.05)\n";
        let r = parse(out);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0].0, "fib 20");
        assert!((r[0].1 - 26_251.0).abs() < 0.01);
        assert_eq!(r[1].0, "a very long benchmark name that wraps");
        assert!((r[1].1 - 1_600_000.0).abs() < 0.01);
    }

    #[test]
    fn p50_table() {
        let out = "metric                         p50         p99  target        result
                   ----------------------------------------------------------------
                   keystroke: edit + highlight   0.98 ms     1.19 ms  p99 <= 4 ms   PASS  (10k lines)
                   open large log (2 GB)      5044.85 ms  5044.85 ms  first <= 1 s  GAP
                   
error: something else 3 ms
";
        let r = parse(out);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0].0, "keystroke: edit + highlight");
        assert!((r[0].1 - 980_000.0).abs() < 0.5);
        assert_eq!(r[1].0, "open large log (2 GB)");
    }

    #[test]
    fn libtest_and_go() {
        let r = parse("test tests::bench_add ... bench:       1,234 ns/iter (+/- 56)\n");
        assert_eq!(r, vec![("tests::bench_add".to_owned(), 1234.0)]);
        let r = parse(
            "goos: windows\nBenchmarkFib20-8   \t   30000\t     41653 ns/op\nBenchmarkSort\t100\t12.5 ns/op\nPASS\n",
        );
        assert_eq!(r, vec![("Fib20".to_owned(), 41653.0), ("Sort".to_owned(), 12.5)]);
    }

    #[test]
    fn history_round_trip_and_regressions() {
        let a = Run {
            project: "p".into(),
            commit: "abc".into(),
            time: 1,
            results: vec![("x".into(), 100.0), ("y".into(), 50.0)],
        };
        let b = Run {
            project: "p".into(),
            commit: "def+".into(),
            time: 2,
            results: vec![("x".into(), 125.0), ("y".into(), 51.0)],
        };
        let other = Run { project: "q".into(), commit: String::new(), time: 3, results: vec![("z".into(), 1.0)] };
        let text = write_history(&[a.clone(), b.clone(), other.clone()], 10);
        assert_eq!(read_history(&text), vec![a.clone(), b.clone(), other.clone()]);
        // Only the newest run of p kept.
        assert_eq!(read_history(&write_history(&[a.clone(), b.clone(), other], 1)).len(), 2);
        let slow = regressions(&a, &b, 10.0);
        assert_eq!(slow.len(), 1);
        assert_eq!(slow[0].0, "x");
        assert!((slow[0].1 - 25.0).abs() < 1e-9);
        assert_eq!(human(1_600_000.0), "1.60 ms");
        assert_eq!(human(41653.0), "41.7 µs");
    }
}
