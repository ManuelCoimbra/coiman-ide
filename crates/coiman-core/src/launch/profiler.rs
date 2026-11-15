//! Profiling: a program run under a sampling profiler, and what it found as a call tree for a
//! flame graph.
//!
//! The profilers are programs of their own, run like any other tool: `py-spy` for Python (on
//! every system), `perf` for anything else on Linux. Their output is read here: collapsed stacks
//! (`a;b;c 12`, what py-spy's `raw` format and flame graph tools write) or `perf script`'s text.

use std::path::{Path, PathBuf};

use crate::language::lsp::discovery::quiet_command;
use crate::language::lsp::servers::which;

/// A function in the call tree: how many samples were in it (`total`, with what it called) and
/// in it alone (`own`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub name: String,
    /// The file and line, when the profiler gave them.
    pub place: Option<(String, u32)>,
    pub total: u64,
    pub own: u64,
    pub children: Vec<Frame>,
}

impl Frame {
    fn child(&mut self, name: &str, place: &Option<(String, u32)>) -> &mut Frame {
        let at = match self.children.iter().position(|c| same_function(c.name.as_str(), &c.place, name, place)) {
            Some(at) => at,
            None => {
                self.children.push(Frame { name: name.to_owned(), place: place.clone(), ..Frame::default() });
                self.children.len() - 1
            }
        };
        &mut self.children[at]
    }

    /// Children, most samples first (the order a flame graph draws them in).
    fn sort(&mut self) {
        self.children.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.name.cmp(&b.name)));
        for c in &mut self.children {
            c.sort();
        }
    }
}

/// What a profile found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    /// Every stack under one root, named "all".
    pub root: Frame,
}

impl Profile {
    /// Adds a stack, outermost call first, seen `count` times.
    pub fn add(&mut self, stack: &[(String, Option<(String, u32)>)], count: u64) {
        if self.root.name.is_empty() {
            self.root.name = "all".to_owned();
        }
        self.root.total += count;
        let mut at = &mut self.root;
        for (name, place) in stack {
            at = at.child(name, place);
            at.total += count;
        }
        at.own += count;
    }

    pub fn samples(&self) -> u64 {
        self.root.total
    }

    /// The functions most samples were in, by themselves, most first. `total` counts a sample
    /// once however often the function is on its stack (recursion).
    pub fn hottest(&self) -> Vec<Hot> {
        let mut found: Vec<Hot> = Vec::new();
        fn walk(f: &Frame, path: &mut Vec<(String, Option<(String, u32)>)>, found: &mut Vec<Hot>) {
            let key = (f.name.clone(), f.place.as_ref().map(|(file, _)| (file.clone(), 0)));
            let outer = path.contains(&key);
            let at = match found.iter().position(|h| same_function(&h.name, &h.place, &f.name, &f.place)) {
                Some(at) => at,
                None => {
                    found.push(Hot { name: f.name.clone(), place: f.place.clone(), own: 0, total: 0 });
                    found.len() - 1
                }
            };
            found[at].own += f.own;
            if !outer {
                found[at].total += f.total;
            }
            path.push(key);
            for c in &f.children {
                walk(c, path, found);
            }
            path.pop();
        }
        let mut path = Vec::new();
        for c in &self.root.children {
            walk(c, &mut path, &mut found);
        }
        found.sort_by(|a, b| b.own.cmp(&a.own).then_with(|| b.total.cmp(&a.total)).then_with(|| a.name.cmp(&b.name)));
        found
    }
}

/// One function: the same name in the same file. Profilers that give the line being run name a
/// function once per line; the first line seen stands for it.
fn same_function(a: &str, a_place: &Option<(String, u32)>, b: &str, b_place: &Option<(String, u32)>) -> bool {
    a == b && a_place.as_ref().map(|p| &p.0) == b_place.as_ref().map(|p| &p.0)
}

/// A function's share of the time in two profiles: `now` and `before`, as fractions of each
/// profile's samples, so runs of different lengths compare.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub name: String,
    pub place: Option<(String, u32)>,
    pub now: f64,
    pub before: f64,
}

/// What changed from `before` to `now`, by time in the function itself: the biggest changes
/// first, functions only one has among them.
pub fn compare(now: &Profile, before: &Profile) -> Vec<Change> {
    let share = |p: &Profile| {
        let total = p.samples().max(1) as f64;
        p.hottest().into_iter().map(move |h| (h.name, h.place, h.own as f64 / total)).collect::<Vec<_>>()
    };
    let old = share(before);
    let mut out: Vec<Change> = share(now)
        .into_iter()
        .map(|(name, place, now)| {
            let before = old.iter().find(|(n, p, _)| same_function(n, p, &name, &place)).map_or(0.0, |(_, _, b)| *b);
            Change { name, place, now, before }
        })
        .collect();
    for (name, place, before) in old {
        if !out.iter().any(|c| same_function(&c.name, &c.place, &name, &place)) {
            out.push(Change { name, place, now: 0.0, before });
        }
    }
    out.sort_by(|a, b| (b.now - b.before).abs().total_cmp(&(a.now - a.before).abs()).then_with(|| a.name.cmp(&b.name)));
    out
}

/// A function with its samples, for the hottest list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hot {
    pub name: String,
    pub place: Option<(String, u32)>,
    pub own: u64,
    pub total: u64,
}

/// Reads a profile: collapsed stacks or `perf script` output, whichever the text is.
pub fn parse(text: &str) -> Result<Profile, String> {
    let mut profile = if looks_like_perf_script(text) { parse_perf_script(text) } else { parse_folded(text) };
    profile.root.sort();
    if profile.samples() == 0 {
        return Err("no samples in it: is it collapsed stacks (a;b;c 12) or perf script output?".to_owned());
    }
    Ok(profile)
}

/// `perf script` writes a header line per sample, then indented frames.
fn looks_like_perf_script(text: &str) -> bool {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    match (lines.next(), lines.next()) {
        (Some(first), Some(second)) => {
            !first.starts_with(char::is_whitespace) && second.starts_with(char::is_whitespace)
        }
        _ => false,
    }
}

/// Collapsed stacks: `outer;inner;innermost 12` per line.
pub fn parse_folded(text: &str) -> Profile {
    let mut profile = Profile::default();
    for line in text.lines() {
        let line = line.trim_end();
        let Some((stack, count)) = line.rsplit_once(' ') else { continue };
        let Ok(count) = count.parse::<u64>() else { continue };
        let frames: Vec<_> = stack.split(';').filter(|f| !f.is_empty()).map(split_frame).collect();
        if !frames.is_empty() {
            profile.add(&frames, count);
        }
    }
    profile
}

/// A frame as py-spy writes it, `name (file.py:12)`, else the name alone.
pub fn split_frame(raw: &str) -> (String, Option<(String, u32)>) {
    let raw = raw.trim();
    if let Some(open) = raw.rfind(" (")
        && raw.ends_with(')')
    {
        let inside = &raw[open + 2..raw.len() - 1];
        if let Some((file, line)) = inside.rsplit_once(':')
            && let Ok(line) = line.parse::<u32>()
        {
            return (raw[..open].to_owned(), Some((file.to_owned(), line)));
        }
    }
    (raw.to_owned(), None)
}

/// `perf script` output: per sample a header line, then its frames innermost first, as
/// `address symbol+offset (library)`.
pub fn parse_perf_script(text: &str) -> Profile {
    let mut profile = Profile::default();
    let mut stack: Vec<(String, Option<(String, u32)>)> = Vec::new();
    let mut in_sample = false;
    let mut flush = |stack: &mut Vec<(String, Option<(String, u32)>)>| {
        if !stack.is_empty() {
            stack.reverse();
            profile.add(stack, 1);
            stack.clear();
        }
    };
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut stack);
            in_sample = false;
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            flush(&mut stack);
            in_sample = true;
            continue;
        }
        if in_sample {
            stack.push((perf_symbol(line.trim()), None));
        }
    }
    flush(&mut stack);
    profile
}

/// `7f1c2d address symbol+0x1a (/lib/x.so)` → `symbol`; an unknown one → its library's name.
fn perf_symbol(line: &str) -> String {
    let rest = line.split_once(char::is_whitespace).map_or("", |(_, r)| r.trim());
    let (symbol, dso) = match rest.rfind(" (") {
        Some(at) if rest.ends_with(')') => (&rest[..at], rest[at + 2..rest.len() - 1].trim()),
        _ => (rest, ""),
    };
    let symbol = symbol.rsplit_once("+0x").map_or(symbol, |(s, _)| s).trim();
    if symbol.is_empty() || symbol == "[unknown]" {
        let lib = Path::new(dso).file_name().map_or(dso.to_owned(), |n| n.to_string_lossy().into_owned());
        if lib.is_empty() { "[unknown]".to_owned() } else { format!("[{lib}]") }
    } else {
        symbol.to_owned()
    }
}

/// Which profiler runs a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profiler {
    PySpy,
    Perf,
}

/// How to profile a file: the profiler's command, where it writes, and how to read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilePlan {
    pub profiler: Profiler,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// What the profiler writes.
    pub output: PathBuf,
}

/// Samples a second: often enough for short programs, light enough not to slow them.
const RATE: &str = "200";

/// The plan for profiling `file`: Python with py-spy; on Linux anything Run can run, with perf.
/// `out_dir` is where the profile is written.
pub fn plan(file: &Path, source: &str, root: Option<&Path>, out_dir: &Path) -> Result<ProfilePlan, String> {
    let cwd = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "py" || ext == "pyw" {
        let python = real_python(file, root)?;
        let py_spy = find_py_spy(&python).ok_or(
            "Profiling Python needs py-spy: install it with `pip install py-spy` (it's run as a program of its own)",
        )?;
        let output = out_dir.join("profile.folded");
        // `--function`: by function, at its first line, rather than by the line being run.
        let args = ["record", "--function", "--format", "raw", "--rate", RATE, "-o"]
            .map(str::to_owned)
            .into_iter()
            .chain([output.to_string_lossy().into_owned(), "--".to_owned(), python.to_string_lossy().into_owned()])
            .chain([file.to_string_lossy().into_owned()])
            .collect();
        return Ok(ProfilePlan { profiler: Profiler::PySpy, program: py_spy, args, cwd, output });
    }
    if !cfg!(target_os = "linux") {
        return Err(format!(
            "Profiling .{ext} files needs perf, which is Linux's; here Python files can be profiled (with py-spy)"
        ));
    }
    let perf =
        which("perf").ok_or("Profiling needs perf: install it with your distribution's linux-tools or perf package")?;
    let run = match crate::launch::run::plan(file, source, root)? {
        crate::launch::run::Outcome::Run(plan) => plan,
        crate::launch::run::Outcome::Choose(plans) => plans.into_iter().next().ok_or("nothing to run here")?,
    };
    let output = out_dir.join("perf.data");
    let args = ["record", "-g", "-F", RATE, "-o"]
        .map(str::to_owned)
        .into_iter()
        .chain([output.to_string_lossy().into_owned(), "--".to_owned(), "sh".to_owned(), "-c".to_owned(), run.command])
        .collect();
    Ok(ProfilePlan { profiler: Profiler::Perf, program: perf, args, cwd: run.cwd, output })
}

/// The interpreter itself, not a launcher in front of it (Windows' `python` can be one, and
/// py-spy would sample the launcher).
fn real_python(file: &Path, root: Option<&Path>) -> Result<PathBuf, String> {
    let start = file.parent().unwrap_or(file);
    let env = crate::launch::environment::resolve(start, root).ok_or("Python isn't on PATH")?;
    let out = quiet_command(&env.python)
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .map_err(|e| format!("couldn't start {}: {e}", env.python.display()))?;
    let real = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(if out.status.success() && !real.is_empty() { PathBuf::from(real) } else { env.python })
}

/// py-spy on PATH, else where pip puts programs for this Python (its own Scripts or bin, and
/// the user's).
fn find_py_spy(python: &Path) -> Option<PathBuf> {
    if let Some(found) = which("py-spy") {
        return Some(found);
    }
    let exe = format!("py-spy{}", std::env::consts::EXE_SUFFIX);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = python.parent() {
        dirs.extend([dir.join("Scripts"), dir.to_path_buf()]);
    }
    let user = quiet_command(python)
        .args(["-c", "import os, sysconfig; print(sysconfig.get_path('scripts', f'{os.name}_user'))"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()));
    dirs.extend(user);
    dirs.into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// Runs a plan to the end and reads what it found. Blocks: run it off the UI thread.
/// A profiled run: what the program printed, and the profile or why there's none.
#[derive(Debug)]
pub struct Ran {
    pub printed: String,
    pub profile: Result<Profile, String>,
}

/// Runs a plan to the end and reads what it found. Blocks: run it off the UI thread. `Err`
/// only when the profiler couldn't be started.
pub fn run(plan: &ProfilePlan) -> Result<Ran, String> {
    let out = quiet_command(&plan.program)
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("couldn't start {}: {e}", plan.program.display()))?;
    // The program's output, without what py-spy says of itself.
    let mut printed = String::new();
    let streams = [String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)];
    for line in streams.iter().flat_map(|s| s.lines()).filter(|l| !l.starts_with("py-spy>")) {
        printed.push_str(line);
        printed.push('\n');
    }
    Ok(Ran { printed, profile: read(plan, &out) })
}

fn read(plan: &ProfilePlan, out: &std::process::Output) -> Result<Profile, String> {
    let said = |out: &std::process::Output| {
        let err = String::from_utf8_lossy(&out.stderr);
        err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned()
    };
    if !plan.output.is_file() {
        return Err(format!("the profiler wrote nothing: {}", said(out)));
    }
    let text = match plan.profiler {
        Profiler::PySpy => std::fs::read_to_string(&plan.output).map_err(|e| e.to_string())?,
        Profiler::Perf => {
            let script = quiet_command(&plan.program)
                .args(["script", "-i"])
                .arg(&plan.output)
                .output()
                .map_err(|e| format!("couldn't read the profile: {e}"))?;
            if !script.status.success() {
                return Err(format!("perf script failed: {}", said(&script)));
            }
            String::from_utf8_lossy(&script.stdout).into_owned()
        }
    };
    parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_collapsed_stacks() {
        let text = "main (a.py:3);work (a.py:6);fib (a.py:2) 5\nmain (a.py:3);work (a.py:6) 1\nmain (a.py:3);other (a.py:10) 4\n";
        let p = parse(text).unwrap();
        assert_eq!(p.samples(), 10);
        let main = &p.root.children[0];
        assert_eq!((main.name.as_str(), main.place.clone()), ("main", Some(("a.py".to_owned(), 3))));
        assert_eq!(main.total, 10);
        assert_eq!(
            main.children.iter().map(|c| (c.name.as_str(), c.total)).collect::<Vec<_>>(),
            [("work", 6), ("other", 4)]
        );
        let hot = p.hottest();
        assert_eq!((hot[0].name.as_str(), hot[0].own), ("fib", 5));
        assert_eq!((hot[1].name.as_str(), hot[1].own), ("other", 4));
    }

    #[test]
    fn lines_of_one_function_are_one() {
        let p = parse_folded(
            "main (a.py:1);other (a.py:10) 3
main (a.py:1);other (a.py:11) 7
",
        );
        assert_eq!(p.root.children[0].children.len(), 1);
        let hot = p.hottest();
        assert_eq!(
            (hot[0].name.as_str(), hot[0].own, hot[0].place.clone()),
            ("other", 10, Some(("a.py".to_owned(), 10)))
        );
    }

    #[test]
    fn compares_two_runs_by_share() {
        let before = parse_folded("main;a 50\nmain;b 50\n");
        let now = parse_folded("main;a 10\nmain;b 30\nmain;c 60\n");
        let changes = compare(&now, &before);
        let get = |n: &str| changes.iter().find(|c| c.name == n).unwrap().clone();
        assert_eq!((get("c").before, get("c").now), (0.0, 0.6));
        assert!((get("a").now - 0.1).abs() < 1e-9 && (get("a").before - 0.5).abs() < 1e-9);
        assert_eq!(changes[0].name, "c", "the biggest change first");
    }

    #[test]
    fn recursion_counts_once_in_total() {
        let p = parse_folded("f;f;f 3\nf 1\n");
        let f = p.hottest().into_iter().find(|h| h.name == "f").unwrap();
        assert_eq!((f.own, f.total), (4, 4));
    }

    #[test]
    fn reads_perf_script() {
        let text = "\
prog  1234 100.000: 250000 cpu-clock:
\t    55d0c1a0 fib+0x10 (/tmp/prog)
\t    55d0c1b0 main+0x20 (/tmp/prog)
\t    7f00aa00 __libc_start_main+0xf3 (/usr/lib/libc.so.6)

prog  1234 100.004: 250000 cpu-clock:
\t    55d0c1b0 main+0x20 (/tmp/prog)
\t    7f00aa00 [unknown] (/usr/lib/libc.so.6)
";
        let p = parse(text).unwrap();
        assert_eq!(p.samples(), 2);
        let names: Vec<_> = p.root.children.iter().map(|c| c.name.as_str()).collect();
        // Equal samples: by name.
        assert_eq!(names, ["[libc.so.6]", "__libc_start_main"]);
        assert_eq!(p.root.children[1].children[0].children[0].name, "fib");
    }

    #[test]
    fn splits_py_spy_frames() {
        assert_eq!(split_frame("fib (busy.py:2)"), ("fib".to_owned(), Some(("busy.py".to_owned(), 2))));
        assert_eq!(
            split_frame("_load (<frozen importlib._bootstrap>:915)"),
            ("_load".to_owned(), Some(("<frozen importlib._bootstrap>".to_owned(), 915)))
        );
        assert_eq!(split_frame("process 123:\"python x.py\""), ("process 123:\"python x.py\"".to_owned(), None));
    }

    #[test]
    fn empty_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("not a profile").is_err());
    }

    /// With py-spy installed, a real Python program profiled end to end.
    #[test]
    fn profiles_python_with_py_spy() {
        let dir = std::env::temp_dir().join(format!("coiman-profile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("busy.py");
        let source = "def spin():\n    s = 0\n    for i in range(3000000):\n        s += i * i\n    return s\n\nspin()\nprint('spun')\n";
        std::fs::write(&file, source).unwrap();
        let plan = match plan(&file, source, None, &dir) {
            Ok(plan) => plan,
            Err(why) => {
                eprintln!("skipped: {why}");
                return;
            }
        };
        let ran = run(&plan).unwrap();
        let profile = ran.profile.unwrap();
        assert!(profile.hottest().iter().any(|h| h.name == "spin" && h.own > 0), "{:?}", profile.hottest());
        assert!(ran.printed.contains("spun"), "{:?}", ran.printed);
        assert!(!ran.printed.contains("py-spy>"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
