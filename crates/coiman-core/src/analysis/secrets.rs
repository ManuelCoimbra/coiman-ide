//! Secrets left in a project's files: keys and tokens of known services by their shape,
//! private keys, and passwords or keys assigned in code that look random enough to be real.
//! What's found is shown masked; the files are only read.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// Files larger than this aren't read (bundles, data).
const MAX_BYTES: u64 = 1024 * 1024;
/// A value assigned to `password`, `token`… counts when it's at least this random.
const MIN_ENTROPY: f64 = 3.0;

/// One secret found.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    /// Relative to the project, with `/`.
    pub path: String,
    /// 1-based.
    pub line: usize,
    /// What kind: "GitHub token", "Private key"…
    pub kind: &'static str,
    /// The secret with all but its first characters hidden.
    pub masked: String,
}

struct Rule {
    kind: &'static str,
    pattern: &'static str,
}

/// Shapes that are secrets wherever they appear.
const RULES: &[Rule] = &[
    Rule {
        kind: "Private key",
        pattern: r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY(?: BLOCK)?-----",
    },
    Rule { kind: "AWS access key", pattern: r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b" },
    Rule { kind: "GitHub token", pattern: r"\b(?:gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b" },
    Rule { kind: "GitLab token", pattern: r"\bglpat-[0-9A-Za-z_-]{20,}\b" },
    Rule { kind: "Slack token", pattern: r"\bxox[abposr]-[0-9A-Za-z-]{10,}\b" },
    Rule { kind: "Stripe secret key", pattern: r"\b(?:sk|rk)_live_[0-9A-Za-z]{20,}\b" },
    Rule { kind: "Google API key", pattern: r"\bAIza[0-9A-Za-z_-]{35}\b" },
    Rule { kind: "OpenAI-style API key", pattern: r"\bsk-(?:proj-)?[A-Za-z0-9_-]{32,}\b" },
    Rule { kind: "npm token", pattern: r"\bnpm_[A-Za-z0-9]{36}\b" },
    Rule { kind: "JSON Web Token", pattern: r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}" },
    Rule { kind: "Password in a URL", pattern: r"\b[a-z][a-z0-9+.-]*://[^\s/:@]+:([^\s/:@]{6,})@[^\s/]+" },
];

fn rules() -> &'static [(&'static str, Regex)] {
    static COMPILED: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    COMPILED.get_or_init(|| RULES.iter().map(|r| (r.kind, Regex::new(r.pattern).expect("rule compiles"))).collect())
}

/// `password = "…"`, `api_key: '…'`, `TOKEN=…` (in .env files, unquoted too).
fn assignment() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"(?i)\b([a-z0-9_.-]*(?:password|passwd|pwd|secret|api_?key|access_?key|auth_?token|token|private_?key|client_?secret)[a-z0-9_.-]*)["']?\s*[:=]\s*(?:"([^"\s]{8,})"|'([^'\s]{8,})'|([^\s"'#;,]{8,}))"#,
        )
        .expect("assignment pattern compiles")
    })
}

/// Bits per character: how random a value looks ("aaaa" 0, a hex key about 4).
pub fn entropy(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for c in &chars {
        *counts.entry(*c).or_insert(0usize) += 1;
    }
    let n = chars.len() as f64;
    counts.values().map(|&k| k as f64 / n).map(|p| -p * p.log2()).sum()
}

/// Values that stand for a secret rather than being one.
fn placeholder(value: &str) -> bool {
    let lower = value.to_lowercase();
    let words = [
        "example",
        "changeme",
        "change_me",
        "password",
        "secret",
        "your",
        "xxxx",
        "****",
        "dummy",
        "sample",
        "placeholder",
        "redacted",
        "token",
        "test",
        "fake",
        "todo",
        "insert",
        "replace",
    ];
    lower.contains("${")
        || lower.starts_with('<')
        || lower.starts_with('{')
        || lower.starts_with('%')
        || lower.starts_with("process.env")
        || lower.starts_with("os.environ")
        || lower.starts_with("env(")
        || lower.starts_with("std::env")
        || lower.starts_with("getenv")
        || words.iter().any(|w| lower.contains(w))
        // Code, not a value: a call or a field.
        || value.contains('(')
        || value.contains("::")
        || value.chars().all(|c| c.is_ascii_alphabetic() || c == '_' || c == '.')
}

/// The secret with its first 4 characters shown, the rest as dots.
pub fn mask(secret: &str) -> String {
    let shown: String = secret.chars().take(4).collect();
    let hidden = secret.chars().count().saturating_sub(4).min(20);
    format!("{shown}{}", "•".repeat(hidden.max(4)))
}

/// Comments that say a line's secret-looking text is meant (test data, examples), as
/// gitleaks and detect-secrets read them.
const ALLOW_MARKS: [&str; 3] = ["gitleaks:allow", "pragma: allowlist secret", "not-a-secret"];

/// Files of settings, where `key=value` has no quotes: there an unquoted value counts. In
/// code it would be a type or an expression (`tokens: Vec<…>`).
fn settings_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    name.starts_with(".env")
        || name.ends_with(".env")
        || ["ini", "properties", "cfg", "conf", "yaml", "yml", "env"].contains(&ext)
}

/// The secrets in one file's text.
pub fn scan_text(path: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let unquoted = settings_file(path);
    for (i, line) in text.lines().enumerate() {
        if line.len() > 2000 || ALLOW_MARKS.iter().any(|m| line.contains(m)) {
            continue; // minified or data, or marked as meant
        }
        let mut found = false;
        for (kind, re) in rules() {
            if let Some(m) = re.find(line) {
                out.push(Finding { path: path.to_owned(), line: i + 1, kind, masked: mask(m.as_str()) });
                found = true;
                break;
            }
        }
        if found {
            continue;
        }
        if let Some(c) = assignment().captures(line) {
            let quoted = c.get(2).or_else(|| c.get(3));
            let value = quoted.or_else(|| c.get(4).filter(|_| unquoted)).map_or("", |m| m.as_str());
            if !value.is_empty() && !placeholder(value) && entropy(value) >= MIN_ENTROPY {
                out.push(Finding {
                    path: path.to_owned(),
                    line: i + 1,
                    kind: "Password or key in code",
                    masked: mask(value),
                });
            }
        }
    }
    out
}

/// Whether a file is worth reading: text-like, not a lock file or a bundle.
fn worth_reading(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let skip_names = ["cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "poetry.lock", "go.sum"];
    if skip_names.contains(&name.as_str()) || name.ends_with(".min.js") || name.ends_with(".map") {
        return false;
    }
    let binary = [
        "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "pdf", "zip", "gz", "7z", "exe", "dll", "so", "dylib",
        "class", "jar", "wasm", "o", "a", "lib", "rlib", "woff", "woff2", "ttf", "otf", "mp3", "mp4", "db", "sqlite",
    ];
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    !binary.contains(&ext.as_str())
}

/// Every secret in the project's files, Git-ignored ones aside (a `.env` that isn't ignored
/// is read: that's the one that gets committed).
pub fn scan(root: &Path) -> Vec<Finding> {
    let mut out = Vec::new();
    for entry in super::project_files(root, true) {
        let path: PathBuf = entry.path().to_path_buf();
        if !entry.file_type().is_some_and(|t| t.is_file()) || !worth_reading(&path) {
            continue;
        }
        if entry.metadata().map_or(true, |m| m.len() > MAX_BYTES) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else { continue };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        out.extend(scan_text(&rel, &text));
    }
    out.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<&'static str> {
        scan_text("f", text).into_iter().map(|f| f.kind).collect()
    }

    #[test]
    fn known_shapes_are_found() {
        let gh = format!("token = \"ghp_{}\"", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
        assert_eq!(kinds(&gh), ["GitHub token"]);
        assert_eq!(kinds("aws = AKIAIOSFODNN7EXAMPLE"), ["AWS access key"]); // gitleaks:allow
        assert_eq!(kinds("-----BEGIN OPENSSH PRIVATE KEY-----"), ["Private key"]); // gitleaks:allow
        assert_eq!(kinds("url = postgres://admin:s3cr3tP4ss@db.example.com/app"), ["Password in a URL"]); // gitleaks:allow
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"; // gitleaks:allow
        assert_eq!(kinds(jwt), ["JSON Web Token"]);
    }

    #[test]
    fn real_looking_assignments_count_and_placeholders_dont() {
        assert_eq!(kinds(r#"DB_PASSWORD = "q8Zr!2vLp#9Tx""#), ["Password or key in code"]); // gitleaks:allow
        let in_env = scan_text(".env", "API_KEY=f3a9c1e07b2d4c58a6e1").len();
        assert_eq!(in_env, 1, "unquoted counts in a settings file");
        assert!(kinds("API_KEY=f3a9c1e07b2d4c58a6e1").is_empty(), "in code it's an expression");
        assert!(kinds("let token: Token::Literal = next();").is_empty());
        assert!(kinds(r#"KEY = "f3a9c1e07b2d4c58a6e1" // gitleaks:allow"#).is_empty());
        assert!(kinds(r#"password = "changeme123""#).is_empty());
        assert!(kinds(r#"api_key = "${API_KEY}""#).is_empty());
        assert!(kinds("token = process.env.TOKEN").is_empty());
        assert!(kinds("let password = read_password(prompt);").is_empty());
        assert!(kinds(r#"secret: "aaaaaaaaaaaa""#).is_empty(), "too plain to be a secret");
        assert!(kinds("password_field = form.password_input").is_empty());
    }

    #[test]
    fn masks_keep_only_the_start() {
        assert_eq!(mask("ghp_abcdefgh"), "ghp_••••••••");
        assert_eq!(mask("abc"), "abc••••");
        assert!(entropy("aaaa") < 0.1);
        assert!(entropy("f3a9c1e07b2d4c58") > 3.0);
    }

    #[test]
    fn scans_a_folder_and_skips_ignored_and_binary_files() {
        let dir = std::env::temp_dir().join(format!("coiman_secrets_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        // ignore crate reads .gitignore only inside a Git repository.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".gitignore"), "secret.env\n").unwrap();
        std::fs::write(dir.join("secret.env"), "API_KEY=f3a9c1e07b2d4c58a6e1\n").unwrap();
        // Put together at run time: a key-shaped literal here would trip push protection.
        let stripe = ["sk", "live", "abcdefghijklmnopqrstuvwx"].join("_");
        std::fs::write(dir.join(".env"), format!("STRIPE={stripe}\n")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\nconst K: &str = \"AKIAIOSFODNN7EXAMPLE\";\n").unwrap(); // gitleaks:allow
        std::fs::write(dir.join("logo.png"), "AKIAIOSFODNN7EXAMPLE").unwrap(); // gitleaks:allow
        let found = scan(&dir);
        let at: Vec<(String, usize, &str)> = found.iter().map(|f| (f.path.clone(), f.line, f.kind)).collect();
        assert_eq!(at, [(".env".to_owned(), 1, "Stripe secret key"), ("src/main.rs".to_owned(), 2, "AWS access key")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
