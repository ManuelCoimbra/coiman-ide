//! A project's migrations (sqlx, Diesel, Alembic, Flyway), found from their files, and which
//! have run, from the table each tool keeps in the database.

use std::collections::{BTreeMap, BTreeSet};

use crate::sqlite::ResultSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tool {
    Sqlx,
    Diesel,
    Alembic,
    Flyway,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Sqlx, Tool::Diesel, Tool::Alembic, Tool::Flyway];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Sqlx => "sqlx",
            Tool::Diesel => "Diesel",
            Tool::Alembic => "Alembic",
            Tool::Flyway => "Flyway",
        }
    }

    /// The table the tool keeps what ran in.
    pub fn table(self) -> &'static str {
        match self {
            Tool::Sqlx => "_sqlx_migrations",
            Tool::Diesel => "__diesel_schema_migrations",
            Tool::Alembic => "alembic_version",
            Tool::Flyway => "flyway_schema_history",
        }
    }

    /// The query for what ran: the version first, then whether it worked, where the tool says.
    pub fn query(self) -> String {
        match self {
            Tool::Sqlx => "SELECT version, success FROM _sqlx_migrations".into(),
            Tool::Diesel => "SELECT version FROM __diesel_schema_migrations".into(),
            Tool::Alembic => "SELECT version_num FROM alembic_version".into(),
            Tool::Flyway => "SELECT version, success FROM flyway_schema_history WHERE version IS NOT NULL".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// No database open, or its table not read yet.
    Unknown,
    Ran,
    NotRun,
    /// It ran, and the tool wrote that it failed.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Migration {
    pub tool: Tool,
    /// As the tool keeps it: sqlx's number, Diesel's digits, Alembic's revision, Flyway's version.
    pub version: String,
    pub name: String,
    /// Relative to the open folder.
    pub path: String,
    /// Alembic: the revision before it.
    pub down: Option<String>,
    pub status: Status,
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn parent(path: &str) -> &str {
    let p = path.trim_end_matches(['/', '\\']);
    match p.rfind(['/', '\\']) {
        Some(i) => &p[..i],
        None => "",
    }
}

fn is_migrations_dir(dir: &str) -> bool {
    matches!(file_name(dir).to_ascii_lowercase().as_str(), "migrations" | "migration")
}

/// Leading digits, and what follows `_`: `20240101_add_users.sql` gives `20240101`, `add users`.
fn numbered(stem: &str) -> Option<(String, String)> {
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    let rest = stem[digits.len()..].strip_prefix('_')?;
    (!digits.is_empty()).then(|| (digits, rest.replace('_', " ")))
}

/// The migrations among the open folder's `.sql` and `.py` files (relative paths). `read` gives
/// a file's text, asked only of Alembic's candidates.
pub fn find(paths: &[String], mut read: impl FnMut(&str) -> Option<String>) -> Vec<Migration> {
    let mut out = Vec::new();
    for path in paths {
        let name = file_name(path);
        let lower = name.to_ascii_lowercase();
        let dir = parent(path);
        let m = |tool, version: String, name: String| Migration {
            tool,
            version,
            name,
            path: path.clone(),
            down: None,
            status: Status::Unknown,
        };
        // Flyway: V1_2__add_users.sql (versioned; repeatable R__ and undo U ones aren't listed).
        if lower.ends_with(".sql")
            && let Some(rest) = name.strip_prefix('V')
            && let Some((version, desc)) = rest[..rest.len() - 4].split_once("__")
            && !version.is_empty()
            && version.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_')
        {
            out.push(m(Tool::Flyway, version.replace('_', "."), desc.replace('_', " ")));
            continue;
        }
        // Diesel: migrations/2024-01-01-000000_add_users/up.sql.
        if lower == "up.sql" && is_migrations_dir(parent(dir)) {
            let folder = file_name(dir);
            let digits: String = folder.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            if let Some(rest) = folder[digits.len()..].strip_prefix('_') {
                let version: String = digits.chars().filter(char::is_ascii_digit).collect();
                if !version.is_empty() {
                    out.push(m(Tool::Diesel, version, rest.replace('_', " ")));
                    continue;
                }
            }
        }
        // sqlx: migrations/20240101000000_add_users.sql, or .up.sql (with a .down.sql beside it).
        if lower.ends_with(".sql") && !lower.ends_with(".down.sql") && is_migrations_dir(dir) {
            let stem = name.trim_end_matches(".sql").trim_end_matches(".SQL").trim_end_matches(".up");
            if let Some((version, desc)) = numbered(stem) {
                out.push(m(Tool::Sqlx, version.trim_start_matches('0').to_owned().max("0".into()), desc));
                continue;
            }
        }
        // Alembic: <script_location>/versions/<rev>_<slug>.py.
        if lower.ends_with(".py")
            && file_name(dir).eq_ignore_ascii_case("versions")
            && !lower.starts_with("__")
            && let Some(text) = read(path)
            && let Some(revision) = assignment(&text, "revision")
        {
            let down = assignment(&text, "down_revision");
            let title = text
                .lines()
                .map(|l| l.trim().trim_start_matches("\"\"\"").trim_end_matches("\"\"\"").trim())
                .find(|l| !l.is_empty() && !l.starts_with('#'))
                .unwrap_or("")
                .to_owned();
            let mut mig = m(Tool::Alembic, revision, title);
            mig.down = down;
            out.push(mig);
        }
    }
    out.sort_by(|a, b| (a.tool, version_key(&a.version), &a.path).cmp(&(b.tool, version_key(&b.version), &b.path)));
    // Alembic's revisions aren't in order by name: by their chain.
    let alembic: Vec<Migration> = out.iter().filter(|m| m.tool == Tool::Alembic).cloned().collect();
    if !alembic.is_empty() {
        out.retain(|m| m.tool != Tool::Alembic);
        out.extend(chain(alembic));
        out.sort_by_key(|m| m.tool);
    }
    out
}

/// Versions compared as numbers where they are (`1.10` after `1.9`).
fn version_key(v: &str) -> Vec<(u64, String)> {
    v.split('.').map(|p| (p.parse::<u64>().unwrap_or(u64::MAX), p.to_owned())).collect()
}

/// `name = "value"` (or `name: str = '…'`); None for `None`.
fn assignment(text: &str, name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(name) else { continue };
        let rest = rest.trim_start();
        // `revision: str = …`, `down_revision: Union[str, None] = …`
        let rest = match rest.strip_prefix(':') {
            Some(typed) => typed.split_once('=').map(|(_, v)| v)?,
            None => rest.strip_prefix('=')?,
        };
        let value = rest.trim();
        if value.starts_with("None") {
            return None;
        }
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        return value[1..].split(quote).next().map(str::to_owned);
    }
    None
}

/// Alembic revisions oldest first, by following `down_revision`.
fn chain(mut left: Vec<Migration>) -> Vec<Migration> {
    let mut out = Vec::new();
    let mut prev: Option<String> = None;
    while !left.is_empty() {
        let at = left.iter().position(|m| m.down == prev).unwrap_or(0);
        let m = left.remove(at);
        prev = Some(m.version.clone());
        out.push(m);
    }
    out
}

/// Marks what ran for `tool`, from its table's rows (`set`), or none ran when the table isn't
/// there (`set` None).
pub fn mark(migrations: &mut [Migration], tool: Tool, set: Option<&ResultSet>) {
    let mut ran: BTreeMap<String, bool> = BTreeMap::new();
    if let Some(set) = set {
        for row in &set.rows {
            let Some(Some(version)) = row.first() else { continue };
            let ok =
                row.get(1).and_then(|v| v.as_deref()).is_none_or(|v| matches!(v, "1" | "t" | "true" | "TRUE" | "True"));
            ran.insert(version.trim().to_owned(), ok);
        }
    }
    let mine = migrations.iter_mut().filter(|m| m.tool == tool);
    if tool == Tool::Alembic {
        // The table holds the head: it and every revision before it ran.
        let all: Vec<(String, Option<String>)> = migrations
            .iter()
            .filter(|m| m.tool == Tool::Alembic)
            .map(|m| (m.version.clone(), m.down.clone()))
            .collect();
        let mut done = BTreeSet::new();
        for head in ran.keys() {
            let mut at = Some(head.clone());
            while let Some(v) = at {
                if !done.insert(v.clone()) {
                    break;
                }
                at = all.iter().find(|(r, _)| *r == v).and_then(|(_, d)| d.clone());
            }
        }
        for m in migrations.iter_mut().filter(|m| m.tool == tool) {
            m.status = if done.contains(&m.version) { Status::Ran } else { Status::NotRun };
        }
        return;
    }
    for m in mine {
        // sqlx keeps numbers; Diesel and Flyway text (Flyway `1.2`).
        let key = |v: &str| if tool == Tool::Sqlx { v.trim_start_matches('0').to_owned() } else { v.to_owned() };
        let found = ran.iter().find(|(v, _)| key(v) == key(&m.version)).map(|(_, ok)| *ok);
        m.status = match found {
            Some(true) => Status::Ran,
            Some(false) => Status::Failed,
            None => Status::NotRun,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(rows: &[&[&str]]) -> ResultSet {
        ResultSet {
            columns: vec!["version".into(), "success".into()],
            rows: rows.iter().map(|r| r.iter().map(|c| Some((*c).to_owned())).collect()).collect(),
            more: 0,
        }
    }

    #[test]
    fn each_tool_found_by_its_files() {
        let paths: Vec<String> = [
            "migrations/20240102000000_add_orders.sql",
            "migrations/20240101000000_add_users.up.sql",
            "migrations/20240101000000_add_users.down.sql",
            "api/migrations/2024-03-01-120000_create_posts/up.sql",
            "api/migrations/2024-03-01-120000_create_posts/down.sql",
            "api/migrations/00000000000000_diesel_initial_setup/up.sql",
            "src/main/resources/db/migration/V1_1__add_index.sql",
            "src/main/resources/db/migration/V1__init.sql",
            "src/main/resources/db/migration/R__views.sql",
            "alembic/versions/bb_second.py",
            "alembic/versions/aa_first.py",
            "alembic/versions/__init__.py",
            "src/queries.sql",
            "docs/README.md",
        ]
        .map(str::to_owned)
        .to_vec();
        let read = |p: &str| match p {
            "alembic/versions/aa_first.py" => Some(
                "\"\"\"create users\n\nRevision ID: aa\n\"\"\"\nrevision = 'aa'\ndown_revision = None\n".to_owned(),
            ),
            "alembic/versions/bb_second.py" => Some(
                "\"\"\"add email\"\"\"\nrevision: str = \"bb\"\ndown_revision: Union[str, None] = \"aa\"\n".to_owned(),
            ),
            _ => None,
        };
        let found: Vec<String> =
            find(&paths, read).iter().map(|m| format!("{} {} {}", m.tool.label(), m.version, m.name)).collect();
        assert_eq!(
            found,
            [
                "sqlx 20240101000000 add users",
                "sqlx 20240102000000 add orders",
                "Diesel 00000000000000 diesel initial setup",
                "Diesel 20240301120000 create posts",
                "Alembic aa create users",
                "Alembic bb add email",
                "Flyway 1 init",
                "Flyway 1.1 add index",
            ]
        );
    }

    #[test]
    fn what_ran() {
        let paths: Vec<String> = [
            "migrations/1_a.sql",
            "migrations/2_b.sql",
            "migrations/3_c.sql",
            "db/migration/V1__x.sql",
            "db/migration/V2__y.sql",
        ]
        .map(str::to_owned)
        .to_vec();
        let mut m = find(&paths, |_| None);
        mark(&mut m, Tool::Sqlx, Some(&set(&[&["1", "1"], &["2", "0"]])));
        mark(&mut m, Tool::Flyway, None);
        let got: Vec<(String, Status)> = m.iter().map(|m| (m.version.clone(), m.status.clone())).collect();
        assert_eq!(
            got,
            [
                ("1".into(), Status::Ran),
                ("2".into(), Status::Failed),
                ("3".into(), Status::NotRun),
                ("1".into(), Status::NotRun),
                ("2".into(), Status::NotRun),
            ]
        );
    }

    #[test]
    fn alembic_head_and_before_it_ran() {
        let mk = |v: &str, d: Option<&str>| Migration {
            tool: Tool::Alembic,
            version: v.into(),
            name: String::new(),
            path: format!("versions/{v}.py"),
            down: d.map(str::to_owned),
            status: Status::Unknown,
        };
        let mut m = vec![mk("a", None), mk("b", Some("a")), mk("c", Some("b"))];
        mark(&mut m, Tool::Alembic, Some(&set(&[&["b"]])));
        let got: Vec<Status> = m.iter().map(|m| m.status.clone()).collect();
        assert_eq!(got, [Status::Ran, Status::Ran, Status::NotRun]);
    }

    #[test]
    fn flyway_and_postgres_booleans() {
        let paths: Vec<String> = ["sql/V1__a.sql".to_owned(), "sql/V1.1__b.sql".to_owned()].to_vec();
        let mut m = find(&paths, |_| None);
        mark(&mut m, Tool::Flyway, Some(&set(&[&["1", "t"], &["1.1", "f"]])));
        assert_eq!(m[0].status, Status::Ran);
        assert_eq!(m[1].status, Status::Failed);
    }
}
