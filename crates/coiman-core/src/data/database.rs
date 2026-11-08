//! SQLite databases through the `sqlite3` program: their tables, views and columns, and SQL
//! run against them with each result as a table. The program runs in its safe mode, so a
//! `.sql` file can't reach the shell or other files through dot-commands.
//!
//! The app has no database code of its own and opens no connections: it is the same tool
//! anyone would use in a terminal.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;

use crate::language::lsp::discovery::quiet_command;

/// Rows kept from one result; the rest are counted, not held.
pub const MAX_ROWS: usize = 10_000;
/// The name of the extra result that says how many rows the statements changed.
const CHANGES: &str = "__coiman_changes";

/// File names databases usually have.
pub fn is_database_name(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    matches!(ext.as_str(), "db" | "sqlite" | "sqlite3" | "db3" | "s3db" | "sl3")
}

/// Whether the file starts as every SQLite database does.
pub fn is_sqlite_file(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 16];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && &head == b"SQLite format 3\0"
}

/// The `sqlite3` program: on `PATH`, or where MSYS2 and similar tools put it.
pub fn find_sqlite() -> Option<PathBuf> {
    crate::language::lsp::servers::which("sqlite3")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub ty: String,
    pub primary_key: bool,
    pub not_null: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignKey {
    pub column: String,
    pub table: String,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub view: bool,
    pub columns: Vec<Column>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultSet {
    pub columns: Vec<String>,
    /// Each cell as text; `None` is SQL's NULL.
    pub rows: Vec<Vec<Option<String>>>,
    /// Rows past [`MAX_ROWS`] that were left out.
    pub more: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// One per statement that returned rows.
    pub sets: Vec<ResultSet>,
    /// Rows inserted, updated or deleted.
    pub changes: u64,
    pub millis: u128,
}

pub struct Sqlite {
    pub program: PathBuf,
}

impl Sqlite {
    pub fn find() -> Option<Self> {
        find_sqlite().map(|program| Self { program })
    }

    /// Runs `sql` on the database and gives back its standard output.
    fn exec(&self, db: &Path, sql: &str, read_only: bool) -> Result<String, String> {
        let mut cmd = quiet_command(&self.program);
        cmd.args(["-safe", "-batch", "-bail", "-json", "-cmd", ".timeout 5000"]);
        if read_only {
            cmd.arg("-readonly");
        }
        cmd.arg(db).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("Couldn't start {}: {e}", self.program.display()))?;
        if let Some(mut stdin) = child.stdin.take() {
            // A closed pipe means sqlite3 stopped early; its error says why.
            let _ = stdin.write_all(sql.as_bytes());
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            return Err(if err.is_empty() { format!("sqlite3 stopped ({})", out.status) } else { err });
        }
        Ok(stdout)
    }

    /// Runs SQL (any number of statements) and gives back what each returned.
    pub fn run(&self, db: &Path, sql: &str) -> Result<Outcome, String> {
        let started = std::time::Instant::now();
        // The total changed comes back as one more result, taken off below.
        let text = format!("{sql}\n;\nSELECT total_changes() AS \"{CHANGES}\";\n");
        let out = self.exec(db, &text, false)?;
        let mut sets = parse_sets(&out)?;
        let mut changes = 0;
        if sets.last().is_some_and(|s| s.columns == [CHANGES]) {
            let last = sets.pop().unwrap_or_default();
            changes = last.rows.first().and_then(|r| r[0].as_deref()).and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        Ok(Outcome { sets, changes, millis: started.elapsed().as_millis() })
    }

    /// The database's tables and views, with their columns and foreign keys.
    pub fn schema(&self, db: &Path) -> Result<Vec<Table>, String> {
        let sql = "SELECT m.name AS t, m.type AS k, p.name AS c, p.type AS ty, p.pk AS pk, p.\"notnull\" AS nn \
                   FROM sqlite_schema m JOIN pragma_table_info(m.name) p \
                   WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' ORDER BY m.name, p.cid;\n\
                   SELECT m.name AS t, f.\"from\" AS c, f.\"table\" AS rt, f.\"to\" AS rc \
                   FROM sqlite_schema m JOIN pragma_foreign_key_list(m.name) f WHERE m.type = 'table';\n\
                   SELECT name AS t, type AS k FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%';\n";
        let out = self.exec(db, sql, true)?;
        let values = parse_values(&out)?;
        let mut tables: Vec<Table> = Vec::new();
        let field = |row: &serde_json::Map<String, Value>, key: &str| text(row.get(key)).unwrap_or_default();
        for set in &values {
            for row in set {
                // Tell the three results apart by their columns.
                if row.contains_key("rt") {
                    let name = field(row, "t");
                    if let Some(t) = tables.iter_mut().find(|t| t.name == name) {
                        t.foreign_keys.push(ForeignKey {
                            column: field(row, "c"),
                            table: field(row, "rt"),
                            to: field(row, "rc"),
                        });
                    }
                    continue;
                }
                let name = field(row, "t");
                let view = field(row, "k") == "view";
                let at = match tables.iter().position(|t| t.name == name) {
                    Some(at) => at,
                    None => {
                        tables.push(Table { name: name.clone(), view, columns: Vec::new(), foreign_keys: Vec::new() });
                        tables.len() - 1
                    }
                };
                if row.contains_key("c") {
                    tables[at].columns.push(Column {
                        name: field(row, "c"),
                        ty: field(row, "ty"),
                        primary_key: field(row, "pk") != "0",
                        not_null: field(row, "nn") != "0",
                    });
                }
            }
        }
        tables.sort_by(|a, b| a.view.cmp(&b.view).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(tables)
    }
}

/// A cell's text: strings as they are, numbers as written, NULL as `None`.
fn text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// `sqlite3 -json` prints one JSON array of row objects per statement that returns rows.
fn parse_values(out: &str) -> Result<Vec<Vec<serde_json::Map<String, Value>>>, String> {
    let mut sets = Vec::new();
    for value in serde_json::Deserializer::from_str(out).into_iter::<Vec<serde_json::Map<String, Value>>>() {
        sets.push(value.map_err(|e| format!("Couldn't read sqlite3's output: {e}"))?);
    }
    Ok(sets)
}

pub fn parse_sets(out: &str) -> Result<Vec<ResultSet>, String> {
    Ok(parse_values(out)?
        .into_iter()
        .map(|rows| {
            let columns: Vec<String> = rows.first().map(|r| r.keys().cloned().collect()).unwrap_or_default();
            let more = rows.len().saturating_sub(MAX_ROWS);
            let rows = rows.iter().take(MAX_ROWS).map(|r| columns.iter().map(|c| text(r.get(c))).collect()).collect();
            ResultSet { columns, rows, more }
        })
        .collect())
}

/// A quoted SQL name: `"order"`, with any `"` inside doubled.
pub fn quote_name(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_results_keep_their_column_order_and_nulls() {
        let out = "[{\"z\":1,\"a\":\"x\",\"m\":null},\n{\"z\":2,\"a\":\"y\",\"m\":3.5}]\n[{\"n\":\"only\"}]\n";
        let sets = parse_sets(out).unwrap();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].columns, ["z", "a", "m"]);
        assert_eq!(sets[0].rows[0], [Some("1".into()), Some("x".into()), None]);
        assert_eq!(sets[0].rows[1][2].as_deref(), Some("3.5"));
        assert_eq!(sets[1].rows, [[Some("only".to_owned())]]);
        assert!(parse_sets("").unwrap().is_empty());
        assert_eq!(quote_name("we\"ird"), "\"we\"\"ird\"");
    }

    /// Against the real `sqlite3`, when it is on this computer.
    #[test]
    fn a_real_database_through_sqlite3() {
        let Some(sqlite) = Sqlite::find() else {
            eprintln!("sqlite3 not found: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("coiman-db-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("shop.db");
        let _ = std::fs::remove_file(&db);
        let made = sqlite
            .run(
                &db,
                "CREATE TABLE customer (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n\
                 CREATE TABLE \"order\" (id INTEGER PRIMARY KEY, customer_id INTEGER REFERENCES customer(id), total REAL);\n\
                 CREATE VIEW big AS SELECT * FROM \"order\" WHERE total > 10;\n\
                 INSERT INTO customer (name) VALUES ('Ada'), ('Linus');\n\
                 INSERT INTO \"order\" (customer_id, total) VALUES (1, 12.5), (2, NULL), (1, 3);",
            )
            .unwrap();
        assert_eq!(made.changes, 5);
        assert!(made.sets.is_empty());
        assert!(is_sqlite_file(&db) && is_database_name(&db));

        let got = sqlite.run(&db, "SELECT name, total FROM customer JOIN \"order\" o ON o.customer_id = customer.id ORDER BY o.id; SELECT count(*) AS n FROM big").unwrap();
        assert_eq!(got.sets.len(), 2);
        assert_eq!(got.sets[0].columns, ["name", "total"]);
        assert_eq!(got.sets[0].rows[1], [Some("Linus".into()), None]);
        assert_eq!(got.sets[1].rows[0][0].as_deref(), Some("1"));
        assert_eq!(got.changes, 0);

        let tables = sqlite.schema(&db).unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["customer", "order", "big"]);
        assert!(tables[0].columns[0].primary_key && tables[0].columns[1].not_null);
        assert_eq!(
            tables[1].foreign_keys,
            [ForeignKey { column: "customer_id".into(), table: "customer".into(), to: "id".into() }]
        );
        assert!(tables[2].view);

        let err = sqlite.run(&db, "SELECT * FROM nowhere").unwrap_err();
        assert!(err.contains("no such table"), "{err}");
        // Safe mode: dot-commands that reach the shell are refused.
        let shell = sqlite.run(&db, ".shell echo hi");
        assert!(shell.is_err(), "{shell:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
