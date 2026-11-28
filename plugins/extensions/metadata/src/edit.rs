//! Reading and changing a project file's name, version, description and licence in place,
//! keeping everything else (comments, order, spacing) as it was. No host calls: tested alone.

/// The fields shown, in order: (key, label).
pub const FIELDS: [(&str, &str); 5] = [
    ("name", "Name"),
    ("version", "Version"),
    ("description", "Description"),
    ("license", "Licence"),
    ("repository", "Repository"),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// TOML, the keys under this table (`package` for Cargo, `project` for pyproject).
    Toml(&'static str),
    Json,
}

/// The project files read, and how.
pub const FILES: [(&str, Kind); 3] =
    [("Cargo.toml", Kind::Toml("package")), ("package.json", Kind::Json), ("pyproject.toml", Kind::Toml("project"))];

/// A TOML or JSON basic string's text, unescaped (the common escapes only).
fn unquote(v: &str) -> Option<String> {
    let v = v.trim();
    let inner = v.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
    None
}

fn quote(v: &str) -> String {
    let mut out = String::from("\"");
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The line ranges of a TOML table's own lines (after its header, before the next header).
fn toml_table(text: &str, table: &str) -> Option<(usize, usize)> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| l.trim() == format!("[{table}]"))?;
    let end =
        lines[start + 1..].iter().position(|l| l.trim_start().starts_with('[')).map_or(lines.len(), |i| start + 1 + i);
    Some((start, end))
}

/// A key's line in a TOML table: `key = ...`.
fn toml_line(text: &str, table: &str, key: &str) -> Option<usize> {
    let (start, end) = toml_table(text, table)?;
    text.lines().enumerate().skip(start + 1).take(end - start - 1).find_map(|(i, l)| {
        let (k, _) = l.split_once('=')?;
        (k.trim() == key).then_some(i)
    })
}

/// A JSON object's top-level key line (`"key": ...`), counting braces outside strings.
fn json_line(text: &str, key: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, line) in text.lines().enumerate() {
        let at_top = depth == 1;
        if at_top && line.trim_start().starts_with(&format!("\"{key}\"")) {
            return Some(i);
        }
        let mut in_str = false;
        let mut esc = false;
        for c in line.chars() {
            match (in_str, esc, c) {
                (true, true, _) => esc = false,
                (true, false, '\\') => esc = true,
                (true, false, '"') => in_str = false,
                (false, _, '"') => in_str = true,
                (false, _, '{' | '[') => depth += 1,
                (false, _, '}' | ']') => depth -= 1,
                _ => {}
            }
        }
    }
    None
}

/// What `key` holds, if it's a string ("" when it isn't there; None when it isn't a string).
pub fn get(text: &str, kind: Kind, key: &str) -> Option<String> {
    let line = match kind {
        Kind::Toml(t) => toml_line(text, t, key),
        Kind::Json => json_line(text, key),
    };
    let Some(n) = line else { return Some(String::new()) };
    let l = text.lines().nth(n)?;
    let value = match kind {
        Kind::Toml(_) => l.split_once('=')?.1,
        Kind::Json => l.split_once(':')?.1.trim().trim_end_matches(','),
    };
    unquote(value)
}

/// `text` with `key` set to `value` (added after the table's or object's last key when new;
/// left out when `value` is empty and the key isn't there).
pub fn set(text: &str, kind: Kind, key: &str, value: &str) -> Result<String, String> {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let found = match kind {
        Kind::Toml(t) => toml_line(text, t, key),
        Kind::Json => json_line(text, key),
    };
    match (found, kind) {
        (Some(n), Kind::Toml(_)) => {
            let l = &lines[n];
            let (k, rest) = l.split_once('=').expect("a key line");
            if unquote(rest).is_none() {
                return Err(format!("{key} isn't a plain string here; change it in the file"));
            }
            let comment = rest.rfind("\" #").map(|i| &rest[i + 1..]).unwrap_or("");
            lines[n] = format!("{k}= {}{comment}", quote(value));
        }
        (Some(n), Kind::Json) => {
            let l = &lines[n];
            let (k, rest) = l.split_once(':').expect("a key line");
            if unquote(rest.trim().trim_end_matches(',')).is_none() {
                return Err(format!("{key} isn't a plain string here; change it in the file"));
            }
            let comma = if rest.trim_end().ends_with(',') { "," } else { "" };
            lines[n] = format!("{k}: {}{comma}", quote(value));
        }
        (None, _) if value.is_empty() => return Ok(text.to_owned()),
        (None, Kind::Toml(t)) => {
            if text.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| k.trim().starts_with(&format!("{key}.")))) {
                return Err(format!("{key} comes from elsewhere here (like the workspace); change it in the file"));
            }
            let (start, end) = toml_table(text, t).ok_or(format!("there's no [{t}] table"))?;
            // After the table's last line that isn't blank.
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, format!("{key} = {}", quote(value)));
        }
        (None, Kind::Json) => {
            // After the first top-level key, with its indent.
            let first = lines.iter().position(|l| l.trim_start().starts_with('"')).ok_or("the object is empty")?;
            let indent: String = lines[first].chars().take_while(|c| c.is_whitespace()).collect();
            lines.insert(first + 1, format!("{indent}\"{key}\": {},", quote(value)));
        }
    }
    let mut out = lines.join(nl);
    if text.ends_with('\n') {
        out.push_str(nl);
    }
    Ok(out)
}

/// Why a field's value won't do, if it won't.
pub fn check(file: &str, key: &str, value: &str) -> Option<String> {
    let v = value.trim();
    match key {
        "name" if v.is_empty() => Some("The name can't be empty".into()),
        "name" if file == "package.json" && (v != v.to_lowercase() || v.contains(' ')) => {
            Some("npm names are lowercase, with no spaces".into())
        }
        "name" if file != "package.json" && v.contains(' ') => Some("The name can't have spaces".into()),
        "version" if file == "Cargo.toml" || file == "package.json" => {
            let core = v.split(['-', '+']).next().unwrap_or("");
            let ok = core.split('.').count() == 3
                && core.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            (!ok).then(|| "The version is three numbers, like 1.2.3".into())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARGO: &str = "[package]\nname = \"demo\" # the crate\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nserde = \"1\"\n";
    const NPM: &str = "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\",\n  \"scripts\": { \"name\": \"x\" },\n  \"private\": true\n}\n";

    #[test]
    fn reads() {
        let k = Kind::Toml("package");
        assert_eq!(get(CARGO, k, "name").unwrap(), "demo");
        assert_eq!(get(CARGO, k, "description").unwrap(), "");
        assert_eq!(get(NPM, Kind::Json, "version").unwrap(), "1.0.0");
        assert_eq!(get(NPM, Kind::Json, "private"), None);
    }

    #[test]
    fn changes_in_place() {
        let k = Kind::Toml("package");
        let t = set(CARGO, k, "name", "renamed").unwrap();
        assert!(t.starts_with("[package]\nname = \"renamed\" # the crate\n"), "{t}");
        let t = set(&t, k, "description", "A \"demo\"").unwrap();
        assert!(t.contains("edition = \"2024\"\ndescription = \"A \\\"demo\\\"\"\n\n[dependencies]"), "{t}");
        assert_eq!(get(&t, k, "description").unwrap(), "A \"demo\"");
        assert_eq!(set(CARGO, k, "license", "").unwrap(), CARGO);
        assert!(set("[package]\nversion.workspace = true\nname = { x = 1 }\n", k, "name", "a").is_err());

        let j = set(NPM, Kind::Json, "name", "other").unwrap();
        assert!(j.contains("  \"name\": \"other\",\n") && j.contains("{ \"name\": \"x\" }"), "{j}");
        let j = set(&j, Kind::Json, "license", "MIT").unwrap();
        assert!(j.contains("  \"name\": \"other\",\n  \"license\": \"MIT\",\n"), "{j}");
        let crlf = CARGO.replace('\n', "\r\n");
        assert!(set(&crlf, k, "version", "0.2.0").unwrap().contains("version = \"0.2.0\"\r\n"));
    }

    #[test]
    fn checks() {
        assert!(check("Cargo.toml", "version", "1.2").is_some());
        assert!(check("Cargo.toml", "version", "1.2.3-beta.1").is_none());
        assert!(check("package.json", "name", "My App").is_some());
        assert!(check("pyproject.toml", "version", "2024.1").is_none());
    }
}
