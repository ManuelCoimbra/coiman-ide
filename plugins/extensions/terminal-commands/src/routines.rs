//! A project's routines: named lists of commands in `.coiman/routines.toml`, run one after the
//! other in a terminal tab, stopping at the first that fails. A step `@name` runs another
//! routine's steps there.
//!
//! ```toml
//! [[routine]]
//! name = "check"
//! description = "What CI runs"
//! steps = ["cargo fmt --check", "cargo clippy -- -D warnings", "@test"]
//!
//! [[routine]]
//! name = "test"
//! steps = ["cargo test"]
//! shell = "sh"          # optional: powershell (the default where it's there) or sh
//! ```

pub const FILE: &str = ".coiman/routines.toml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routine {
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    /// "powershell" or "sh"; empty for the first that's here.
    pub shell: String,
}

pub fn parse(text: &str) -> Result<Vec<Routine>, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut out: Vec<Routine> = Vec::new();
    for (i, r) in table.get("routine").and_then(toml::Value::as_array).into_iter().flatten().enumerate() {
        let get = |k: &str| r.get(k).and_then(toml::Value::as_str).unwrap_or("").trim().to_owned();
        let name = get("name");
        if name.is_empty() {
            return Err(format!("routine {} has no `name`", i + 1));
        }
        if out.iter().any(|o| o.name == name) {
            return Err(format!("two routines are called {name}"));
        }
        let steps: Vec<String> = r
            .get("steps")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter().filter_map(|s| s.as_str()).map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
            })
            .unwrap_or_default();
        if steps.is_empty() {
            return Err(format!("routine {name} has no `steps`"));
        }
        let shell = get("shell").to_lowercase();
        if !matches!(shell.as_str(), "" | "powershell" | "sh") {
            return Err(format!("routine {name}: `shell` is powershell or sh"));
        }
        out.push(Routine { name, description: get("description"), steps, shell });
    }
    Ok(out)
}

/// A routine's steps with each `@name` replaced by that routine's own, in order.
pub fn expand(all: &[Routine], name: &str) -> Result<Vec<String>, String> {
    fn walk(all: &[Routine], name: &str, path: &mut Vec<String>, out: &mut Vec<String>) -> Result<(), String> {
        if path.iter().any(|p| p == name) {
            path.push(name.to_owned());
            return Err(format!("routines call each other round in a circle: {}", path.join(" → ")));
        }
        let r = all.iter().find(|r| r.name == name).ok_or(format!("there's no routine called {name}"))?;
        path.push(name.to_owned());
        for step in &r.steps {
            match step.strip_prefix('@') {
                Some(other) => walk(all, other.trim(), path, out)?,
                None => out.push(step.clone()),
            }
        }
        path.pop();
        Ok(())
    }
    let mut out = Vec::new();
    walk(all, name, &mut Vec::new(), &mut out)?;
    Ok(out)
}

/// A PowerShell script that says each step, runs it, and stops at the first that fails (a
/// program's exit code or a command's error).
pub fn powershell(name: &str, steps: &[String]) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    // A command's error stops it; its output goes to the screen at once (`Out-Host`), not after
    // the next step or never (a table PowerShell formats late is lost at `exit`).
    let mut s = String::from("$ErrorActionPreference = 'Stop'\n");
    for (i, step) in steps.iter().enumerate() {
        s.push_str(&format!(
            "Write-Host {} -ForegroundColor Cyan\n$global:LASTEXITCODE = 0\n$ok = $true\n\
             try {{ & {{ {step} }} | Out-Host }} catch {{ Write-Host $_ -ForegroundColor Red; $ok = $false }}\n\
             if (-not $ok -or $LASTEXITCODE -ne 0) {{ Write-Host {} -ForegroundColor Red; exit 1 }}\n",
            q(&format!("> [{}/{}] {step}", i + 1, steps.len())),
            q(&format!("Stopped: step {} of routine {name} failed", i + 1)),
        ));
    }
    s.push_str(&format!("Write-Host {} -ForegroundColor Green\n", q(&format!("Routine {name} done"))));
    s
}

/// The same for `sh`.
pub fn sh(name: &str, steps: &[String]) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let mut s = String::new();
    for (i, step) in steps.iter().enumerate() {
        s.push_str(&format!(
            "printf '%s\\n' {}\n{step} || {{ printf '%s\\n' {} >&2; exit 1; }}\n",
            q(&format!("> [{}/{}] {step}", i + 1, steps.len())),
            q(&format!("Stopped: step {} of routine {name} failed", i + 1)),
        ));
    }
    s.push_str(&format!("printf '%s\\n' {}\n", q(&format!("Routine {name} done"))));
    s
}

/// Base64 of the script in UTF-16LE, as `powershell -EncodedCommand` takes it: no quoting for
/// the command line to get wrong.
pub fn encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].iter().enumerate() {
            out.push(if i <= c.len() { char::from(A[(n >> shift) as usize & 63]) } else { '=' });
        }
    }
    out
}

/// The programs to try, in order, and the arguments each gets.
pub fn commands(r: &Routine, steps: &[String]) -> Vec<(&'static str, Vec<String>)> {
    let ps = |p: &'static str| {
        (p, vec!["-NoLogo".into(), "-NoProfile".into(), "-EncodedCommand".into(), encoded(&powershell(&r.name, steps))])
    };
    let unix = ("sh", vec!["-c".into(), sh(&r.name, steps)]);
    match r.shell.as_str() {
        "sh" => vec![unix],
        "powershell" => vec![ps("pwsh"), ps("powershell")],
        _ => vec![ps("pwsh"), ps("powershell"), unix],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE_TEXT: &str = "[[routine]]\nname = \"check\"\ndescription = \"What CI runs\"\n\
        steps = [\"cargo fmt --check\", \"@test\", \"echo it's done\"]\n\
        [[routine]]\nname = \"test\"\nsteps = [\"cargo test\", \"@unit\"]\nshell = \"sh\"\n\
        [[routine]]\nname = \"unit\"\nsteps = [\"cargo test --lib\"]\n";

    #[test]
    fn reads_and_expands() {
        let all = parse(FILE_TEXT).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[1].shell, "sh");
        assert_eq!(
            expand(&all, "check").unwrap(),
            ["cargo fmt --check", "cargo test", "cargo test --lib", "echo it's done"]
        );
        assert!(expand(&all, "nope").unwrap_err().contains("no routine called nope"));
        let looped =
            parse("[[routine]]\nname = \"a\"\nsteps = [\"@b\"]\n[[routine]]\nname = \"b\"\nsteps = [\"@a\"]\n")
                .unwrap();
        assert_eq!(expand(&looped, "a").unwrap_err(), "routines call each other round in a circle: a → b → a");
    }

    #[test]
    fn mistakes_say_what() {
        assert!(parse("[[routine]]\nsteps = [\"x\"]\n").unwrap_err().contains("no `name`"));
        assert!(parse("[[routine]]\nname = \"a\"\n").unwrap_err().contains("no `steps`"));
        assert!(
            parse("[[routine]]\nname = \"a\"\nsteps = [\"x\"]\nshell = \"fish\"\n")
                .unwrap_err()
                .contains("powershell or sh")
        );
        let twice = "[[routine]]\nname = \"a\"\nsteps = [\"x\"]\n[[routine]]\nname = \"a\"\nsteps = [\"y\"]\n";
        assert!(parse(twice).unwrap_err().contains("two routines are called a"));
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn scripts() {
        let steps = vec!["cargo build".to_owned(), "echo it's".to_owned()];
        let ps = powershell("ci", &steps);
        assert!(ps.contains("Write-Host '> [2/2] echo it''s' -ForegroundColor Cyan"), "{ps}");
        assert!(
            ps.contains("try { & { cargo build } | Out-Host }")
                && ps.contains(
                    "if (-not $ok -or $LASTEXITCODE -ne 0) { Write-Host 'Stopped: step 1 of routine ci failed'"
                ),
            "{ps}"
        );
        let s = sh("ci", &steps);
        assert!(
            s.contains("echo it's || { printf '%s\\n' 'Stopped: step 2 of routine ci failed' >&2; exit 1; }"),
            "{s}"
        );
        assert!(s.contains("printf '%s\\n' '> [2/2] echo it'\\''s'"), "{s}");
        // Base64 of UTF-16LE "hi": h=68 00, i=69 00.
        assert_eq!(encoded("hi"), "aABpAA==");
        assert_eq!(encoded("a"), "YQA=");
        let r = Routine { name: "x".into(), description: String::new(), steps: vec![], shell: String::new() };
        assert_eq!(commands(&r, &steps).iter().map(|c| c.0).collect::<Vec<_>>(), ["pwsh", "powershell", "sh"]);
    }

    /// The script really runs: the second step fails, so the third doesn't.
    #[test]
    fn a_real_shell_stops_at_the_failure() {
        let steps = vec!["echo one".to_owned(), "exit 3".to_owned(), "echo three".to_owned()];
        // PowerShell: a table's output shows before the failure, and a cmdlet's error stops it.
        let ps_steps = vec![
            "Get-ChildItem . | Select-Object -First 1 Name".to_owned(),
            "Get-Item no-such-file-here".to_owned(),
            "echo three".to_owned(),
        ];
        if let Ok(out) = std::process::Command::new("powershell")
            .args(["-NoLogo", "-NoProfile", "-EncodedCommand", &encoded(&powershell("t", &ps_steps))])
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success() && text.contains("Name") && !text.contains("three"), "{text}");
        }
        let tries: [(&str, Vec<String>); 2] = [
            (
                "powershell",
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-EncodedCommand".into(),
                    encoded(&powershell("t", &steps)),
                ],
            ),
            ("sh", vec!["-c".into(), sh("t", &steps)]),
        ];
        for (program, args) in tries {
            let Ok(out) = std::process::Command::new(program).args(&args).output() else { continue };
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{program}: {text}");
            assert!(text.contains("one") && !text.contains("three"), "{program}: {text}");
        }
    }
}
