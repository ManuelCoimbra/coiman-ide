//! A project's own Code Health settings, in `.coiman/health.toml`: paths no check looks at,
//! findings allowed on purpose (each with why), and quality gates: the most of each kind of
//! finding the project accepts.
//!
//! ```toml
//! # Paths no check looks at.
//! ignore = ["vendor/**", "**/*.generated.rs"]
//!
//! # A finding allowed on purpose: by its check, where, and words its text has.
//! [[allow]]
//! check = "security"
//! path = "src/ffi/**"
//! rule = "unsafe"
//! why = "the FFI layer needs unsafe"
//!
//! # The most of each the project accepts; Check Project says whether it passes.
//! [gates]
//! secrets = 0
//! security = 0
//! complexity = 15
//! debt = 50
//! ```

pub const FILE: &str = ".coiman/health.toml";

/// The checks a gate or an allowance can name, with what its number means.
pub const CHECKS: [(&str, &str); 7] = [
    ("secrets", "secrets"),
    ("security", "security findings"),
    ("complexity", "the most complex function's complexity"),
    ("debt", "TODO, FIXME, HACK and XXX notes"),
    ("duplicates", "code written more than once"),
    ("undocumented", "public items without docs"),
    ("unused", "dependencies the code never uses"),
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub ignore: Vec<String>,
    pub allow: Vec<Allow>,
    /// Gate name and limit, in the file's order.
    pub gates: Vec<(String, usize)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allow {
    pub check: String,
    pub path: String,
    /// Words the finding's text must contain ("" for any).
    pub rule: String,
    pub why: String,
}

/// A finding allowed by the settings, for the list of what was left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allowed {
    pub check: String,
    pub path: String,
    pub line: usize,
    pub text: String,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gate {
    pub check: String,
    pub limit: usize,
    pub actual: usize,
}

impl Gate {
    pub fn passes(&self) -> bool {
        self.actual <= self.limit
    }
}

pub fn parse(text: &str) -> Result<Settings, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let known = |c: &str| CHECKS.iter().any(|(k, _)| *k == c);
    let ignore = table
        .get("ignore")
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let mut allow = Vec::new();
    for (i, a) in table.get("allow").and_then(toml::Value::as_array).into_iter().flatten().enumerate() {
        let get = |k: &str| a.get(k).and_then(toml::Value::as_str).unwrap_or("").to_owned();
        let check = get("check");
        if !known(&check) {
            return Err(format!("allow {}: `check` is one of {}", i + 1, names()));
        }
        let why = get("why");
        if why.trim().is_empty() {
            return Err(format!("allow {} ({check}): say `why` it's allowed", i + 1));
        }
        let path = get("path");
        allow.push(Allow { check, path: if path.is_empty() { "**".into() } else { path }, rule: get("rule"), why });
    }
    let mut gates = Vec::new();
    if let Some(g) = table.get("gates").and_then(toml::Value::as_table) {
        for (k, v) in g {
            if !known(k) {
                return Err(format!("gates: {k} isn't one of {}", names()));
            }
            let n = v.as_integer().filter(|n| *n >= 0).ok_or(format!("gates: {k} is a number, 0 or more"))?;
            gates.push((k.clone(), n as usize));
        }
    }
    Ok(Settings { ignore, allow, gates })
}

fn names() -> String {
    CHECKS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")
}

/// A path matches a glob: `**` any folders, `*` part of a name, `?` one character; one without
/// `*` is that file or everything under that folder.
pub fn matches(glob: &str, path: &str) -> bool {
    let glob = glob.trim_start_matches("./").trim_end_matches('/');
    if !glob.contains(['*', '?']) {
        return path == glob || path.starts_with(&format!("{glob}/"));
    }
    fn walk(g: &[u8], p: &[u8]) -> bool {
        match g.first() {
            None => p.is_empty(),
            Some(b'*') if g.get(1) == Some(&b'*') => {
                let rest = if g.get(2) == Some(&b'/') { &g[3..] } else { &g[2..] };
                if rest.is_empty() {
                    return true;
                }
                (0..=p.len()).any(|i| (i == 0 || p[i - 1] == b'/') && walk(rest, &p[i..])) || walk(rest, p)
            }
            Some(b'*') => (0..=p.len()).take_while(|&i| i == 0 || p[i - 1] != b'/').any(|i| walk(&g[1..], &p[i..])),
            Some(b'?') => !p.is_empty() && p[0] != b'/' && walk(&g[1..], &p[1..]),
            Some(c) => p.first() == Some(c) && walk(&g[1..], &p[1..]),
        }
    }
    walk(glob.as_bytes(), path.as_bytes())
}

impl Settings {
    pub fn ignored(&self, path: &str) -> bool {
        self.ignore.iter().any(|g| matches(g, path))
    }

    /// Why a finding of `check` at `path` with `text` is allowed, if it is.
    pub fn allowed(&self, check: &str, path: &str, text: &str) -> Option<&str> {
        self.allow
            .iter()
            .find(|a| a.check == check && matches(&a.path, path) && (a.rule.is_empty() || text.contains(&a.rule)))
            .map(|a| a.why.as_str())
    }

    /// The gates against what was found: `count` gives each check's number.
    pub fn gates(&self, count: impl Fn(&str) -> usize) -> Vec<Gate> {
        self.gates.iter().map(|(c, limit)| Gate { check: c.clone(), limit: *limit, actual: count(c) }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = "ignore = [\"vendor/**\", \"**/*.generated.rs\"]\n\
        [[allow]]\ncheck = \"security\"\npath = \"src/ffi/**\"\nrule = \"unsafe\"\nwhy = \"the FFI layer needs unsafe\"\n\
        [[allow]]\ncheck = \"debt\"\nwhy = \"tracked elsewhere\"\n\
        [gates]\nsecrets = 0\ncomplexity = 15\n";

    #[test]
    fn reads_and_applies() {
        let s = parse(EXAMPLE).unwrap();
        assert!(s.ignored("vendor/x/a.c") && s.ignored("src/api.generated.rs") && !s.ignored("src/api.rs"));
        assert_eq!(
            s.allowed("security", "src/ffi/raw.rs", "unsafe block with no SAFETY note"),
            Some("the FFI layer needs unsafe")
        );
        assert_eq!(s.allowed("security", "src/ffi/raw.rs", "strcpy doesn't know"), None, "another rule");
        assert_eq!(s.allowed("security", "src/main.rs", "unsafe block"), None, "another path");
        assert_eq!(s.allowed("debt", "anything.rs", "TODO x"), Some("tracked elsewhere"), "no path: everywhere");
        let gates = s.gates(|c| if c == "complexity" { 22 } else { 0 });
        let gate = |c: &str| gates.iter().find(|g| g.check == c).unwrap().clone();
        assert_eq!(gates.len(), 2);
        assert!(gate("secrets").passes() && !gate("complexity").passes());
        assert_eq!((gate("complexity").limit, gate("complexity").actual), (15, 22));
    }

    #[test]
    fn mistakes_say_what() {
        assert!(parse("[[allow]]\ncheck = \"speed\"\nwhy = \"x\"\n").unwrap_err().contains("`check` is one of"));
        assert!(parse("[[allow]]\ncheck = \"debt\"\n").unwrap_err().contains("say `why`"));
        assert!(parse("[gates]\nspeed = 1\n").unwrap_err().contains("speed isn't one of"));
        assert!(parse("[gates]\ndebt = -1\n").unwrap_err().contains("0 or more"));
        assert_eq!(parse("").unwrap(), Settings::default());
    }
}
