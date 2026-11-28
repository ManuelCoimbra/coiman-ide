//! The Python run's drivers, and reading what they print. No host calls: tested on its own.

/// Printed before the results, so the program's own output comes first and stays apart.
pub const MARK: &str = "\u{1}COIMAN-RESULTS\u{1}";

/// Runs the file (argument 1, as `__main__`, with its folder on `sys.path`) under cProfile and
/// prints, after `MARK`, one line per function: file, line, name, calls, primitive calls, own
/// seconds, total seconds; then `error\t...` if it raised.
pub const CALLS: &str = r#"
MARK = "COIMAN-RESULTS"
import cProfile, os, pstats, runpy, sys
path = sys.argv[1]
sys.argv = sys.argv[1:]
sys.path.insert(0, os.path.dirname(os.path.abspath(path)))
p = cProfile.Profile()
err = None
try:
    p.runcall(runpy.run_path, path, run_name="__main__")
except SystemExit:
    pass
except BaseException as e:
    err = "%s: %s" % (type(e).__name__, e)
sys.stdout.flush()
clean = lambda s: str(s).replace("\t", " ").replace("\n", " ")
print("\n" + MARK)
for (f, l, n), (cc, nc, tt, ct, _) in pstats.Stats(p).stats.items():
    print("\t".join([clean(f), str(l), clean(n), str(nc), str(cc), repr(tt), repr(ct)]))
if err:
    print("error\t" + clean(err))
"#;

/// Runs the file under tracemalloc and prints, after `MARK`, `peak\t<bytes>` then one line per
/// source line still holding memory at the end: file, line, bytes, blocks; then `error\t...`.
pub const MEMORY: &str = r#"
MARK = "COIMAN-RESULTS"
import os, runpy, sys, tracemalloc
path = sys.argv[1]
sys.argv = sys.argv[1:]
sys.path.insert(0, os.path.dirname(os.path.abspath(path)))
tracemalloc.start(1)
err = None
g = {}
try:
    g = runpy.run_path(path, run_name="__main__")
except SystemExit:
    pass
except BaseException as e:
    err = "%s: %s" % (type(e).__name__, e)
snap = tracemalloc.take_snapshot()
peak = tracemalloc.get_traced_memory()[1]
tracemalloc.stop()
sys.stdout.flush()
clean = lambda s: str(s).replace("\t", " ").replace("\n", " ")
print("\n" + MARK)
print("peak\t%d" % peak)
for s in snap.statistics("lineno")[:200]:
    fr = s.traceback[0]
    print("\t".join([clean(fr.filename), str(fr.lineno), str(s.size), str(s.count)]))
if err:
    print("error\t" + clean(err))
"#;

/// A function's calls.
#[derive(Clone, Debug, PartialEq)]
pub struct Calls {
    pub file: String,
    pub line: u32,
    pub name: String,
    pub calls: u64,
    /// Calls not made by itself (recursion counted apart).
    pub primitive: u64,
    pub own: f64,
    pub total: f64,
}

/// Memory held by a line at the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Held {
    pub file: String,
    pub line: u32,
    pub bytes: u64,
    pub blocks: u64,
}

/// The program's own output, and the lines after `MARK`.
pub fn split(out: &str) -> (String, Option<&str>) {
    match out.find(MARK) {
        Some(at) => (out[..at].trim_end().to_owned(), Some(&out[at + MARK.len()..])),
        None => (out.trim_end().to_owned(), None),
    }
}

fn error(line: &str) -> Option<String> {
    line.strip_prefix("error\t").map(str::to_owned)
}

pub fn calls(results: &str) -> (Vec<Calls>, Option<String>) {
    let mut list = Vec::new();
    let mut err = None;
    for line in results.lines() {
        if let Some(e) = error(line) {
            err = Some(e);
            continue;
        }
        let p: Vec<&str> = line.split('\t').collect();
        let [file, l, name, nc, cc, tt, ct] = p[..] else { continue };
        let (Ok(line), Ok(calls), Ok(primitive), Ok(own), Ok(total)) =
            (l.parse(), nc.parse(), cc.parse(), tt.parse(), ct.parse())
        else {
            continue;
        };
        list.push(Calls { file: file.to_owned(), line, name: name.to_owned(), calls, primitive, own, total });
    }
    list.sort_by(|a, b| b.calls.cmp(&a.calls).then(b.own.total_cmp(&a.own)));
    (list, err)
}

pub fn memory(results: &str) -> (u64, Vec<Held>, Option<String>) {
    let mut peak = 0;
    let mut list = Vec::new();
    let mut err = None;
    for line in results.lines() {
        if let Some(e) = error(line) {
            err = Some(e);
            continue;
        }
        if let Some(v) = line.strip_prefix("peak\t") {
            peak = v.trim().parse().unwrap_or(0);
            continue;
        }
        let p: Vec<&str> = line.split('\t').collect();
        let [file, l, size, count] = p[..] else { continue };
        if let (Ok(line), Ok(bytes), Ok(blocks)) = (l.parse(), size.parse(), count.parse()) {
            list.push(Held { file: file.to_owned(), line, bytes, blocks });
        }
    }
    (peak, list, err)
}

/// A file path as the project knows it: relative to the open folder when inside it.
pub fn relative(file: &str, root: &str) -> Option<String> {
    let norm = |s: &str| s.replace('\\', "/");
    let (f, r) = (norm(file), norm(root));
    let r = r.trim_end_matches('/');
    let rest = if cfg!(windows) || f.len() > 1 && f.as_bytes()[1] == b':' {
        f.to_lowercase().strip_prefix(&format!("{}/", r.to_lowercase())).map(|_| &f[r.len() + 1..])
    } else {
        f.strip_prefix(&format!("{r}/"))
    }?;
    Some(rest.to_owned())
}

/// "1.2 MB", "340 KB", "12 B".
pub fn bytes(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 {
        format!("{:.1} GB", f / 1e9)
    } else if f >= 1e6 {
        format!("{:.1} MB", f / 1e6)
    } else if f >= 1e3 {
        format!("{:.0} KB", f / 1e3)
    } else {
        format!("{n} B")
    }
}

/// "1.20 s", "35.0 ms", "4 µs".
pub fn secs(s: f64) -> String {
    if s >= 1.0 {
        format!("{s:.2} s")
    } else if s >= 1e-3 {
        format!("{:.1} ms", s * 1e3)
    } else if s >= 1e-6 {
        format!("{:.0} µs", s * 1e6)
    } else {
        "<1 µs".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_calls() {
        let out = format!(
            "hello\n\n{MARK}\nC:/p/main.py\t2\tfib\t21891\t1\t0.01\t0.02\n\
             ~\t0\t<built-in method builtins.print>\t1\t1\t1e-05\t1e-05\nbad\nerror\tValueError: x\n"
        );
        let (own, rest) = split(&out);
        assert_eq!(own, "hello");
        let (list, err) = calls(rest.unwrap());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "fib");
        assert_eq!((list[0].calls, list[0].primitive), (21891, 1));
        assert_eq!(err.as_deref(), Some("ValueError: x"));
    }

    #[test]
    fn reads_memory() {
        let (peak, list, err) = memory("peak\t1048576\nC:/p/a.py\t5\t800000\t10000\n");
        assert_eq!(peak, 1_048_576);
        assert_eq!(list, vec![Held { file: "C:/p/a.py".into(), line: 5, bytes: 800_000, blocks: 10_000 }]);
        assert!(err.is_none());
        assert_eq!(bytes(1_048_576), "1.0 MB");
        assert_eq!(secs(0.035), "35.0 ms");
        assert_eq!(secs(2e-7), "<1 µs");
    }

    #[test]
    fn paths() {
        assert_eq!(relative("C:\\Proj\\src\\a.py", "C:/proj").as_deref(), Some("src/a.py"));
        assert_eq!(relative("C:/other/a.py", "C:/proj"), None);
        assert_eq!(relative("~", "C:/proj"), None);
    }
}
