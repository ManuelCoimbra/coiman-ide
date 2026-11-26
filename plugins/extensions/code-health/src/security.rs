//! Code that is easy to get wrong in a way an attacker can use: SQL put together from strings
//! (injection), HTML made from data without escaping (cross-site scripting), C and C++ calls
//! that write past a buffer, and Rust's `unsafe`. Each finding says where and why. They're read
//! a line at a time, so they're leads to look at, not proof; a line marked `nosec` or
//! `coiman:allow` is left out.

use std::sync::OnceLock;

use regex::Regex;

/// Files larger than this aren't read.
pub const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Sql,
    Html,
    Memory,
    Unsafe,
}

impl Category {
    pub const ALL: [Category; 4] = [Category::Sql, Category::Html, Category::Memory, Category::Unsafe];

    pub fn title(self) -> &'static str {
        match self {
            Category::Sql => "SQL built from strings (injection)",
            Category::Html => "HTML made from data (cross-site scripting)",
            Category::Memory => "C and C++ calls that can write past a buffer",
            Category::Unsafe => "Rust unsafe code",
        }
    }

    /// What to do about it, said once for the group.
    pub fn advice(self) -> &'static str {
        match self {
            Category::Sql => {
                "A value put into SQL text can change what the statement does. Pass it as a parameter \
                 (?, $1, :name, @p) and let the driver send it apart from the SQL."
            }
            Category::Html => {
                "Text put into a page as HTML runs as HTML: a <script> in it runs too. Set it as text \
                 (textContent, {value}), or escape or sanitise it first."
            }
            Category::Memory => {
                "These calls don't know how big the buffer they write to is. Use the ones that take its \
                 size (snprintf, strncat, fgets, strlcpy) or std::string."
            }
            Category::Unsafe => {
                "Inside unsafe the compiler doesn't check memory safety. Each block should be small and say \
                 in a // SAFETY: comment why what it does is sound."
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub category: Category,
    /// Relative to the project, with `/`.
    pub path: String,
    /// 1-based.
    pub line: usize,
    /// Why this line: "strcpy doesn't know how big its target is".
    pub why: String,
    /// The line, trimmed and shortened.
    pub code: String,
}

struct Rule {
    category: Category,
    /// File extensions it's for.
    languages: &'static [&'static str],
    pattern: &'static str,
    why: &'static str,
}

const SCRIPT: &[&str] = &["js", "jsx", "mjs", "cjs", "ts", "tsx", "vue", "svelte", "html", "htm"];
const SQL_LANGS: &[&str] = &["py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "java", "kt", "cs", "go", "php", "rb", "rs"];
const C_LANGS: &[&str] = &["c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "m", "mm"];

/// SQL's statements as they start a query.
const SQL: &str = r"(?:select\b[^\x22\x27`]*\bfrom\b|insert\s+into\b|update\s+[\w.\x22`\[\]]+\s+set\b|delete\s+from\b)";

fn rules() -> &'static [(Rule, Regex)] {
    static RULES: OnceLock<Vec<(Rule, Regex)>> = OnceLock::new();
    RULES.get_or_init(|| {
        // A string of each quote kind with SQL in it (the other quotes may be inside).
        let q = |body: &str| {
            format!(r#"(?:"[^"]*{body}[^"]*"|'[^']*{body}[^']*'|`[^`]*{body}[^`]*`)"#)
        };
        let sql_concat = format!(r#"(?i){}\s*(?:\+\s*[\w(]|%\s*[\w(]|\.format\s*\()"#, q(SQL));
        let sql_concat_before = format!(
            r#"(?i)[\w)\]]\s*\+\s*{}"#,
            q(r"\b(?:where|and|or|values|set|from|in)\b")
        );
        // A value put into the text: f"…{x}…", `…${x}…`, $"…{x}…".
        let sql_interp = format!(r#"(?i){}"#, q(&format!(r#"{SQL}[^"'`]*(?:\{{|\$\{{)"#)));
        let sql_format = format!(
            r#"(?i)(?:format!|fmt\.Sprintf|String\.format|string\.Format|sprintf)\s*\(\s*(?:&|@)?"[^"]*{SQL}[^"]*(?:\{{|%[sdv])"#
        );
        let list: Vec<(Rule, String)> = vec![
            (
                Rule {
                    category: Category::Sql,
                    languages: SQL_LANGS,
                    pattern: "",
                    why: "SQL text joined with a value (+, % or .format)",
                },
                sql_concat,
            ),
            (
                Rule {
                    category: Category::Sql,
                    languages: SQL_LANGS,
                    pattern: "",
                    why: "a value joined into SQL text",
                },
                sql_concat_before,
            ),
            (
                Rule {
                    category: Category::Sql,
                    languages: SQL_LANGS,
                    pattern: "",
                    why: "a value put into SQL text with {…}",
                },
                sql_interp,
            ),
            (
                Rule {
                    category: Category::Sql,
                    languages: SQL_LANGS,
                    pattern: "",
                    why: "SQL text made with a format call",
                },
                sql_format,
            ),
        ]
        .into_iter()
        .chain(FIXED.iter().map(|r| (Rule { ..*r }, r.pattern.to_owned())))
        .collect();
        list.into_iter().map(|(r, p)| (r, Regex::new(&p).expect("rule compiles"))).collect()
    })
}

/// Rules with a pattern of their own.
const FIXED: &[Rule] = &[
    Rule {
        category: Category::Html,
        languages: SCRIPT,
        // Not a plain string: something is put in it.
        pattern: r#"\.(?:inner|outer)HTML\s*\+?=\s*(?:[^"'`\s;]|["'][^"']*["']\s*\+|`[^`]*\$\{)"#,
        why: "innerHTML set from a value",
    },
    Rule {
        category: Category::Html,
        languages: SCRIPT,
        pattern: r#"\binsertAdjacentHTML\s*\(\s*["'][^"']*["']\s*,\s*(?:[^"'`\s)]|`[^`]*\$\{)"#,
        why: "insertAdjacentHTML with a value",
    },
    Rule {
        category: Category::Html,
        languages: SCRIPT,
        pattern: r#"\bdocument\.write(?:ln)?\s*\(\s*(?:[^"'`\s)]|["'][^"']*["']\s*\+|`[^`]*\$\{)"#,
        why: "document.write with a value",
    },
    Rule {
        category: Category::Html,
        languages: &["jsx", "tsx", "js", "ts"],
        pattern: r"\bdangerouslySetInnerHTML\s*=",
        why: "dangerouslySetInnerHTML puts the value in as HTML",
    },
    Rule {
        category: Category::Html,
        languages: &["vue", "html", "htm"],
        pattern: r#"\bv-html\s*="#,
        why: "v-html puts the value in as HTML",
    },
    Rule {
        category: Category::Html,
        languages: &["svelte"],
        pattern: r"\{@html\s",
        why: "{@html} puts the value in as HTML",
    },
    Rule {
        category: Category::Html,
        languages: &["ts", "js"],
        pattern: r"\bbypassSecurityTrust(?:Html|Script|Url|ResourceUrl)\s*\(",
        why: "Angular's escaping turned off for this value",
    },
    Rule {
        category: Category::Html,
        languages: &["py"],
        pattern: r"\b(?:mark_safe|Markup)\s*\(\s*(?:f[\x22\x27]|[\w.]+\s*[\),%+]|[\x22\x27][^\x22\x27]*[\x22\x27]\s*(?:%|\+|\.format))",
        why: "a value marked as safe HTML",
    },
    Rule {
        category: Category::Html,
        languages: &["py"],
        pattern: r"\brender_template_string\s*\(\s*(?:f[\x22\x27]|[\w.]+\s*[\),+%]|[\x22\x27][^\x22\x27]*[\x22\x27]\s*(?:%|\+|\.format))",
        why: "a template made from a value",
    },
    Rule {
        category: Category::Html,
        languages: &["html", "htm", "jinja", "j2", "jinja2"],
        pattern: r"\{\{[^}]*\|\s*safe\b",
        why: "|safe turns the template's escaping off",
    },
    Rule {
        category: Category::Html,
        languages: &["php"],
        pattern: r"\b(?:echo|print)\b[^;]*\$_(?:GET|POST|REQUEST|COOKIE)\b",
        why: "a request's value written to the page as it is",
    },
    Rule {
        category: Category::Memory,
        languages: C_LANGS,
        pattern: r"\bgets\s*\(",
        why: "gets can't be told the buffer's size: never safe",
    },
    Rule {
        category: Category::Memory,
        languages: C_LANGS,
        pattern: r"\b(?:strcpy|wcscpy|_mbscpy|lstrcpy[AW]?)\s*\(",
        why: "strcpy doesn't know how big its target is",
    },
    Rule {
        category: Category::Memory,
        languages: C_LANGS,
        pattern: r"\b(?:strcat|wcscat|_mbscat|lstrcat[AW]?)\s*\(",
        why: "strcat doesn't know how big its target is",
    },
    Rule {
        category: Category::Memory,
        languages: C_LANGS,
        pattern: r"\b(?:sprintf|vsprintf|swprintf|wsprintf[AW]?)\s*\(",
        why: "sprintf doesn't know how big its target is",
    },
    Rule {
        category: Category::Memory,
        languages: C_LANGS,
        pattern: r#"\b(?:scanf|fscanf|sscanf)\s*\([^;]*"[^"]*%s"#,
        why: "%s with no width reads as much as it's given",
    },
    Rule {
        category: Category::Unsafe,
        languages: &["rs"],
        pattern: r"\bunsafe\s*(?:\{|fn\b|impl\b|trait\b|extern\b)",
        why: "",
    },
];

/// Lines that are only a comment.
fn comment(line: &str) -> bool {
    let t = line.trim_start();
    ["//", "#", "/*", "*", "--", "<!--", "{#"].iter().any(|c| t.starts_with(c))
}

const ALLOW: [&str; 3] = ["nosec", "coiman:allow", "NOSONAR"];

fn short(line: &str) -> String {
    let t = line.trim();
    if t.chars().count() > 120 { format!("{}…", t.chars().take(119).collect::<String>()) } else { t.to_owned() }
}

/// What `unsafe` this is, and whether a `SAFETY:` comment says why it's sound.
fn unsafe_why(lines: &[&str], i: usize, line: &str) -> String {
    let what = ["fn", "impl", "trait", "extern"]
        .iter()
        .find(|k| Regex::new(&format!(r"\bunsafe\s*{k}\b")).is_ok_and(|r| r.is_match(line)))
        .map_or("block", |k| match *k {
            "fn" => "function",
            "impl" => "impl",
            "trait" => "trait",
            _ => "extern block",
        });
    // A SAFETY note above it (through attributes and comments), or on the line.
    let mut noted = line.contains("SAFETY:");
    let mut j = i;
    while !noted && j > 0 {
        j -= 1;
        let t = lines[j].trim_start();
        if t.contains("SAFETY:") || t.contains("# Safety") {
            noted = true;
        } else if !(t.starts_with("//") || t.starts_with("#[") || t.starts_with("///") || t.is_empty()) {
            break;
        }
    }
    if noted {
        format!("unsafe {what}, with a SAFETY note")
    } else {
        format!("unsafe {what} with no SAFETY note saying why it's sound")
    }
}

/// The findings in one file's text; `ext` is its extension, lower case.
pub fn scan_text(path: &str, ext: &str, text: &str) -> Vec<Finding> {
    let mine: Vec<&(Rule, Regex)> = rules().iter().filter(|(r, _)| r.languages.contains(&ext)).collect();
    if mine.is_empty() {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut in_block_comment = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if in_block_comment {
            if t.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if t.starts_with("/*") && !t.contains("*/") {
            in_block_comment = true;
            continue;
        }
        if line.len() > 2000 || comment(line) || ALLOW.iter().any(|a| line.contains(a)) {
            continue;
        }
        // One finding a line, the first rule that matches.
        for (rule, re) in &mine {
            if re.is_match(line) {
                let why =
                    if rule.category == Category::Unsafe { unsafe_why(&lines, i, line) } else { rule.why.to_owned() };
                out.push(Finding {
                    category: rule.category,
                    path: path.to_owned(),
                    line: i + 1,
                    why,
                    code: short(line),
                });
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(ext: &str, text: &str) -> Vec<(usize, String)> {
        scan_text(&format!("a.{ext}"), ext, text).into_iter().map(|f| (f.line, f.why)).collect()
    }

    #[test]
    fn sql_built_from_strings() {
        let py = "cur.execute(\"SELECT * FROM users WHERE name = '\" + name + \"'\")\n\
                  cur.execute(f\"DELETE FROM users WHERE id = {uid}\")\n\
                  cur.execute(\"SELECT * FROM users WHERE id = %s\" % uid)\n\
                  cur.execute(\"SELECT * FROM users WHERE id = %s\", (uid,))\n\
                  q = \"UPDATE t SET a = {}\".format(a)\n\
                  # cur.execute(f\"SELECT * FROM x WHERE {y}\")\n";
        let got = found("py", py);
        assert_eq!(got.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 2, 3, 5], "{got:?}");
        let js = "db.query(`SELECT * FROM users WHERE id = ${req.params.id}`);\n\
                  db.query('SELECT * FROM users WHERE id = $1', [id]);\n\
                  const q = \"select name from t where id = \" + id;\n";
        assert_eq!(found("js", js).iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 3]);
        let rs = "let q = format!(\"SELECT * FROM users WHERE name = '{}'\", name);\n\
                  sqlx::query(\"SELECT * FROM users WHERE name = $1\").bind(name);\n";
        assert_eq!(found("rs", rs), [(1, "SQL text made with a format call".to_owned())]);
        let go = "db.Query(fmt.Sprintf(\"SELECT * FROM t WHERE id = %d\", id))\n";
        assert_eq!(found("go", go).len(), 1);
        let cs = "var cmd = new SqlCommand($\"SELECT * FROM t WHERE id = {id}\", conn);\n";
        assert_eq!(found("cs", cs).len(), 1);
        let java = "stmt.executeQuery(\"SELECT * FROM t WHERE name = '\" + name + \"'\");\n";
        assert_eq!(found("java", java).len(), 1);
    }

    #[test]
    fn html_from_data() {
        let js = "el.innerHTML = userText;\n\
                  el.innerHTML = '<b>' + name + '</b>';\n\
                  el.innerHTML = `<p>${msg}</p>`;\n\
                  el.innerHTML = '';\n\
                  el.innerHTML = '<br>';\n\
                  el.textContent = userText;\n\
                  document.write(location.hash);\n\
                  el.insertAdjacentHTML('beforeend', row);\n";
        assert_eq!(found("js", js).iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 2, 3, 7, 8]);
        assert_eq!(found("tsx", "<div dangerouslySetInnerHTML={{ __html: body }} />\n").len(), 1);
        assert_eq!(found("vue", "<div v-html=\"post.body\"></div>\n").len(), 1);
        assert_eq!(found("svelte", "{@html post.body}\n").len(), 1);
        let py =
            "return mark_safe(f\"<b>{name}</b>\")\nreturn mark_safe(\"<br>\")\nrender_template_string(\"<p>\" + msg)\n";
        assert_eq!(found("py", py).iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(found("html", "<p>{{ comment | safe }}</p>\n<p>{{ comment }}</p>\n").len(), 1);
        assert_eq!(found("php", "<?php echo $_GET['q']; ?>\n<?php echo htmlspecialchars($x); ?>\n").len(), 1);
    }

    #[test]
    fn c_calls() {
        let c = "char buf[16];\ngets(buf);\nstrcpy(buf, argv[1]);\nstrncpy(buf, argv[1], sizeof buf);\n\
                 sprintf(buf, \"%s\", name);\nsnprintf(buf, sizeof buf, \"%s\", name);\nscanf(\"%s\", buf);\n\
                 scanf(\"%15s\", buf);\n/* strcpy(a, b); */\n// strcat(a, b);\nstrcat(buf, x); // nosec: sized above\n";
        let got = found("c", c);
        assert_eq!(got.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [2, 3, 5, 7], "{got:?}");
        assert!(got[0].1.contains("never safe"));
        assert!(found("cpp", "  std::strcpy(dst, src);\n").len() == 1);
        assert!(found("rs", "strcpy(a, b);\n").is_empty(), "C rules are for C");
    }

    #[test]
    fn rust_unsafe() {
        let rs = "fn a() {\n    unsafe { ptr.read() }\n}\n\
                  // SAFETY: the pointer came from a live Box.\nlet v = unsafe { *p };\n\
                  /// # Safety\n/// The caller keeps `p` valid.\n#[inline]\npub unsafe fn get(p: *const u8) -> u8 { *p }\n\
                  unsafe impl Send for X {}\nlet s = \"unsafe { }\"; // a string\n";
        let got = found("rs", rs);
        assert_eq!(
            got,
            [
                (2, "unsafe block with no SAFETY note saying why it's sound".to_owned()),
                (5, "unsafe block, with a SAFETY note".to_owned()),
                (9, "unsafe function, with a SAFETY note".to_owned()),
                (10, "unsafe impl with no SAFETY note saying why it's sound".to_owned()),
                (11, "unsafe block with no SAFETY note saying why it's sound".to_owned()),
            ]
        );
    }
}
