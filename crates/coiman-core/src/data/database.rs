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
pub struct Index {
    pub name: String,
    pub unique: bool,
    /// Made by a PRIMARY KEY or UNIQUE constraint rather than CREATE INDEX.
    pub automatic: bool,
    /// Only for some rows (CREATE INDEX ... WHERE).
    pub partial: bool,
    /// Its columns in order; an expression shows as `(expression)`.
    pub columns: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub view: bool,
    pub columns: Vec<Column>,
    pub foreign_keys: Vec<ForeignKey>,
    pub indexes: Vec<Index>,
}

/// One line of a query plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    /// How deep in the plan's tree: 0 for the top.
    pub depth: usize,
    /// What SQLite will do: `SCAN customer`, `SEARCH order USING INDEX ...`.
    pub detail: String,
    /// Reads every row of a table: worth an index when the table is big.
    pub full_scan: bool,
    /// Sorts or groups in a temporary table: an index in that order would skip it.
    pub temp_sort: bool,
}

/// One statement's plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub statement: String,
    /// Empty for statements SQLite has no plan for (CREATE TABLE and the like).
    pub steps: Vec<PlanStep>,
}

/// The name of the marker result put before each statement's plan.
const PLAN_MARK: &str = "__coiman_plan";

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
                   SELECT name AS t, type AS k FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%';\n\
                   SELECT m.name AS t, il.name AS i, il.\"unique\" AS u, il.origin AS o, il.partial AS p, ii.name AS ic \
                   FROM sqlite_schema m JOIN pragma_index_list(m.name) il JOIN pragma_index_info(il.name) ii \
                   WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' ORDER BY m.name, il.seq, ii.seqno;\n";
        let out = self.exec(db, sql, true)?;
        let values = parse_values(&out)?;
        let mut tables: Vec<Table> = Vec::new();
        let field = |row: &serde_json::Map<String, Value>, key: &str| text(row.get(key)).unwrap_or_default();
        for set in &values {
            for row in set {
                // Tell the four results apart by their columns.
                if row.contains_key("i") {
                    let name = field(row, "t");
                    let Some(t) = tables.iter_mut().find(|t| t.name == name) else { continue };
                    let index = field(row, "i");
                    let column = text(row.get("ic")).unwrap_or_else(|| "(expression)".to_owned());
                    match t.indexes.iter_mut().find(|x| x.name == index) {
                        Some(x) => x.columns.push(column),
                        None => t.indexes.push(Index {
                            name: index,
                            unique: field(row, "u") != "0",
                            automatic: field(row, "o") != "c",
                            partial: field(row, "p") == "1",
                            columns: vec![column],
                        }),
                    }
                    continue;
                }
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
                        tables.push(Table {
                            name: name.clone(),
                            view,
                            columns: Vec::new(),
                            foreign_keys: Vec::new(),
                            indexes: Vec::new(),
                        });
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

    /// How SQLite would carry out each statement (`EXPLAIN QUERY PLAN`), without running any:
    /// the database is opened read-only.
    pub fn explain(&self, db: &Path, sql: &str) -> Result<Vec<Plan>, String> {
        let statements = split_statements(sql);
        // `.explain off` makes the plan come back as rows, not as the shell's drawn tree.
        let mut text = String::from(".explain off\n");
        // The line each statement starts on, to say which one an error is in.
        let mut starts = Vec::new();
        for (i, statement) in statements.iter().enumerate() {
            text.push_str(&format!("SELECT {i} AS \"{PLAN_MARK}\";\n"));
            starts.push(text.lines().count() + 1);
            text.push_str(&format!("EXPLAIN QUERY PLAN {};\n", strip_explain(statement)));
        }
        let out = self.exec(db, &text, true).map_err(|e| name_statement(&e, &starts))?;
        let mut plans: Vec<Plan> =
            statements.into_iter().map(|statement| Plan { statement, steps: Vec::new() }).collect();
        let mut at = None;
        for set in parse_values(&out)? {
            if let Some(i) = set.first().and_then(|r| r.get(PLAN_MARK)).and_then(Value::as_u64) {
                at = Some(i as usize);
                continue;
            }
            if let Some(plan) = at.and_then(|i| plans.get_mut(i)) {
                plan.steps = plan_steps(&set);
            }
        }
        Ok(plans)
    }
}

/// sqlite3's "Parse error near line N" counts lines of the text sent to it, not the user's:
/// says which statement instead.
fn name_statement(error: &str, starts: &[usize]) -> String {
    const NEAR: &str = "error near line ";
    let Some(at) = error.find(NEAR) else { return error.to_owned() };
    let digits: String = error[at + NEAR.len()..].chars().take_while(char::is_ascii_digit).collect();
    let Ok(line) = digits.parse::<usize>() else { return error.to_owned() };
    let Some(k) = starts.iter().rposition(|&s| s <= line) else { return error.to_owned() };
    let end = at + NEAR.len() + digits.len();
    format!("{}error in statement {}{}", &error[..at], k + 1, &error[end..])
}

/// A plan's rows (`id`, `parent`, `detail`) as lines in tree order with their depth.
fn plan_steps(rows: &[serde_json::Map<String, Value>]) -> Vec<PlanStep> {
    let mut depth_of: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    rows.iter()
        .map(|row| {
            let id = row.get("id").and_then(Value::as_i64).unwrap_or(0);
            let parent = row.get("parent").and_then(Value::as_i64).unwrap_or(0);
            let depth = depth_of.get(&parent).map_or(0, |d| d + 1);
            depth_of.insert(id, depth);
            let detail = text(row.get("detail")).unwrap_or_default();
            PlanStep {
                depth,
                full_scan: detail.starts_with("SCAN ")
                    && !detail.contains(" USING ")
                    && !detail.contains("CONSTANT ROW"),
                temp_sort: detail.contains("USE TEMP B-TREE"),
                detail,
            }
        })
        .collect()
}

/// A statement without the `EXPLAIN` or `EXPLAIN QUERY PLAN` it may start with.
fn strip_explain(statement: &str) -> &str {
    let mut rest = statement.trim_start();
    for word in ["EXPLAIN", "QUERY", "PLAN"] {
        match rest.get(..word.len()) {
            Some(head) if head.eq_ignore_ascii_case(word) && rest[word.len()..].starts_with(char::is_whitespace) => {
                rest = rest[word.len()..].trim_start();
            }
            _ => break,
        }
    }
    rest
}

/// SQL split into its statements, without their final `;`: a `;` inside quotes, comments or
/// a trigger's BEGIN...END doesn't end one, and the shell's dot-commands are left out.
pub fn split_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut current = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // A dot-command: a line starting with `.` before any statement text.
        if c == '.' && current.trim().is_empty() {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            current.clear();
            continue;
        }
        match c {
            '\'' | '"' | '`' | '[' => {
                let close = if c == '[' { ']' } else { c };
                current.push(c);
                i += 1;
                while i < chars.len() {
                    current.push(chars[i]);
                    i += 1;
                    if chars[i - 1] == close {
                        // A doubled quote is one quote inside the string.
                        if close != ']' && chars.get(i) == Some(&close) {
                            current.push(close);
                            i += 1;
                            continue;
                        }
                        break;
                    }
                }
            }
            '-' if chars.get(i + 1) == Some(&'-') => {
                while i < chars.len() && chars[i] != '\n' {
                    current.push(chars[i]);
                    i += 1;
                }
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                current.push_str("/*");
                i += 2;
                while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                    current.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    current.push_str("*/");
                    i += 2;
                }
            }
            // Inside a trigger, only the `;` after its END ends the statement.
            ';' if is_trigger(&current) && !ends_with_end(&current) => {
                current.push(';');
                i += 1;
            }
            ';' => {
                let s = current.trim();
                if !s.is_empty() {
                    out.push(s.to_owned());
                }
                current.clear();
                i += 1;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }
    let s = current.trim();
    if !s.is_empty() {
        out.push(s.to_owned());
    }
    out
}

/// Whether a statement is `CREATE [TEMP] TRIGGER`, whose body holds statements of its own.
fn is_trigger(statement: &str) -> bool {
    let mut words = statement.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|w| !w.is_empty());
    words.next().is_some_and(|w| w.eq_ignore_ascii_case("create"))
        && words.take(2).any(|w| w.eq_ignore_ascii_case("trigger"))
}

/// Whether the text so far ends with the word END.
fn ends_with_end(text: &str) -> bool {
    let text = text.trim_end();
    let word_start = text.rfind(|c: char| !c.is_alphanumeric() && c != '_').map_or(0, |i| i + 1);
    text[word_start..].eq_ignore_ascii_case("end")
}

/// Where each table goes in an entity diagram: `(table, column, row)`. Tables a table points
/// to sit in columns left of it; views are left out.
pub fn er_layout(tables: &[Table]) -> Vec<(usize, usize, usize)> {
    let real: Vec<usize> = (0..tables.len()).filter(|&i| !tables[i].view).collect();
    let find = |name: &str| real.iter().copied().find(|&i| tables[i].name.eq_ignore_ascii_case(name));
    // One more than the furthest-right table it points to; a cycle stops after as many passes
    // as there are tables.
    let mut column = vec![0usize; tables.len()];
    for _ in 0..real.len() {
        let mut moved = false;
        for &i in &real {
            for fk in &tables[i].foreign_keys {
                if let Some(to) = find(&fk.table)
                    && to != i
                    && column[i] <= column[to]
                {
                    column[i] = column[to] + 1;
                    moved = true;
                }
            }
        }
        if !moved {
            break;
        }
    }
    let mut rows: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    real.iter()
        .map(|&i| {
            let row = rows.entry(column[i]).or_default();
            *row += 1;
            (i, column[i], *row - 1)
        })
        .collect()
}

/// A word Mermaid takes as a name: letters, digits and `_`.
fn mermaid_word(text: &str, empty: &str) -> String {
    let word: String = text.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if word.is_empty() { empty.to_owned() } else { word }
}

/// The tables as a Mermaid entity-relationship diagram (`erDiagram`); views are left out.
pub fn er_mermaid(tables: &[Table]) -> String {
    let mut out = String::from("erDiagram\n");
    for t in tables.iter().filter(|t| !t.view) {
        out.push_str(&format!("    {} {{\n", mermaid_word(&t.name, "table")));
        for c in &t.columns {
            let fk = t.foreign_keys.iter().any(|f| f.column == c.name);
            let keys = match (c.primary_key, fk) {
                (true, true) => " PK, FK",
                (true, false) => " PK",
                (false, true) => " FK",
                _ => "",
            };
            out.push_str(&format!(
                "        {} {}{keys}\n",
                mermaid_word(&c.ty.to_lowercase(), "any"),
                mermaid_word(&c.name, "column")
            ));
        }
        out.push_str("    }\n");
    }
    for t in tables.iter().filter(|t| !t.view) {
        for fk in &t.foreign_keys {
            // The row pointed to: exactly one when the column can't be NULL, else zero or one.
            let required = t.columns.iter().any(|c| c.name == fk.column && (c.not_null || c.primary_key));
            let one = if required { "||" } else { "|o" };
            out.push_str(&format!(
                "    {} {one}--o{{ {} : \"{}\"\n",
                mermaid_word(&fk.table, "table"),
                mermaid_word(&t.name, "table"),
                fk.column.replace('"', "'")
            ));
        }
    }
    out
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

    #[test]
    fn statements_split_on_semicolons_outside_quotes_comments_and_triggers() {
        let sql = "SELECT ';' AS a; -- one; two\nSELECT \"x;y\" FROM t /* ; */;\n.tables\n\
                   CREATE TRIGGER tr AFTER INSERT ON t BEGIN UPDATE t SET n = 1; DELETE FROM u; END;\n\
                   SELECT 'it''s; fine'";
        let got = split_statements(sql);
        assert_eq!(got.len(), 4, "{got:#?}");
        assert_eq!(got[0], "SELECT ';' AS a");
        assert!(got[1].starts_with("-- one; two\nSELECT \"x;y\""), "{}", got[1]);
        assert!(got[2].starts_with("CREATE TRIGGER") && got[2].ends_with("END"), "{}", got[2]);
        assert_eq!(got[3], "SELECT 'it''s; fine'");
        assert_eq!(strip_explain("explain query plan SELECT 1"), "SELECT 1");
        assert_eq!(strip_explain("EXPLAIN SELECT 1"), "SELECT 1");
        assert_eq!(strip_explain("EXPLAINED"), "EXPLAINED");
    }

    #[test]
    fn plan_rows_become_a_tree_with_warnings() {
        let out = r#"[{"id":4,"parent":0,"notused":91,"detail":"SEARCH a USING INTEGER PRIMARY KEY (rowid=?)"},
{"id":8,"parent":0,"notused":0,"detail":"LIST SUBQUERY 1"},
{"id":10,"parent":8,"notused":216,"detail":"SCAN b"},
{"id":24,"parent":0,"notused":56,"detail":"SCAN b USING COVERING INDEX b_a"},
{"id":37,"parent":0,"notused":0,"detail":"USE TEMP B-TREE FOR ORDER BY"}]"#;
        let steps = plan_steps(&parse_values(out).unwrap()[0]);
        let depths: Vec<usize> = steps.iter().map(|s| s.depth).collect();
        assert_eq!(depths, [0, 0, 1, 0, 0]);
        let scans: Vec<bool> = steps.iter().map(|s| s.full_scan).collect();
        assert_eq!(scans, [false, false, true, false, false]);
        assert!(steps[4].temp_sort);
    }

    fn table(name: &str, fks: &[(&str, &str)]) -> Table {
        let mut columns = vec![
            Column { name: "id".into(), ty: "INTEGER".into(), primary_key: true, not_null: false },
            Column { name: "note text".into(), ty: "VARCHAR(20)".into(), primary_key: false, not_null: false },
        ];
        columns.extend(fks.iter().map(|(c, _)| Column {
            name: (*c).into(),
            ty: "INT".into(),
            primary_key: false,
            not_null: *c == "customer_id",
        }));
        Table {
            name: name.to_owned(),
            view: false,
            columns,
            foreign_keys: fks
                .iter()
                .map(|(c, t)| ForeignKey { column: (*c).into(), table: (*t).into(), to: "id".into() })
                .collect(),
            indexes: Vec::new(),
        }
    }

    #[test]
    fn tables_a_table_points_to_sit_left_of_it() {
        let mut view = table("big", &[]);
        view.view = true;
        let tables = vec![
            table("line", &[("order_id", "order"), ("product_id", "product")]),
            table("customer", &[]),
            table("order", &[("customer_id", "customer")]),
            table("product", &[]),
            view,
            // Points to itself, and into a cycle with `b`.
            table("a", &[("parent", "a"), ("b_id", "b")]),
            table("b", &[("a_id", "a")]),
        ];
        let at = er_layout(&tables);
        let col = |name: &str| at.iter().find(|(i, ..)| tables[*i].name == name).map(|p| p.1);
        assert_eq!((col("customer"), col("product"), col("order"), col("line")), (Some(0), Some(0), Some(1), Some(2)));
        assert_eq!(col("big"), None, "views are left out");
        assert!(col("a").is_some() && col("b").is_some(), "a cycle still gets places");
        let mut places: Vec<(usize, usize)> = at.iter().map(|p| (p.1, p.2)).collect();
        places.sort();
        places.dedup();
        assert_eq!(places.len(), at.len(), "no two tables share a place");

        let mermaid = er_mermaid(&tables[..4]);
        assert!(
            mermaid.starts_with("erDiagram\n    line {\n        integer id PK\n        varchar_20_ note_text\n"),
            "{mermaid}"
        );
        assert!(mermaid.contains("        int order_id FK\n"), "{mermaid}");
        assert!(mermaid.contains("    customer ||--o{ order : \"customer_id\"\n"), "{mermaid}");
        assert!(mermaid.contains("    product |o--o{ line : \"product_id\"\n"), "{mermaid}");
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
        assert!(tables[0].indexes.is_empty(), "an INTEGER PRIMARY KEY is the rowid, not an index");

        sqlite
            .run(
                &db,
                "CREATE UNIQUE INDEX customer_name ON customer(name);\n\
                 CREATE INDEX order_by ON \"order\"(customer_id, total) WHERE total > 0;",
            )
            .unwrap();
        let tables = sqlite.schema(&db).unwrap();
        assert_eq!(
            tables[0].indexes,
            [Index {
                name: "customer_name".into(),
                unique: true,
                automatic: false,
                partial: false,
                columns: vec!["name".into()]
            }]
        );
        assert_eq!(tables[1].indexes[0].columns, ["customer_id", "total"]);
        assert!(tables[1].indexes[0].partial && !tables[1].indexes[0].unique);

        let plans = sqlite
            .explain(
                &db,
                "SELECT * FROM customer WHERE name = 'Ada';\nSELECT * FROM \"order\" ORDER BY total;\n\
                 CREATE TABLE never (x);\nDELETE FROM customer;",
            )
            .unwrap();
        assert_eq!(plans.len(), 4);
        assert!(plans[0].steps[0].detail.contains("INDEX customer_name"), "{:?}", plans[0]);
        assert!(plans[1].steps.iter().any(|s| s.full_scan), "{:?}", plans[1]);
        assert!(plans[1].steps.iter().any(|s| s.temp_sort), "{:?}", plans[1]);
        assert!(plans[2].steps.is_empty());
        let left = sqlite
            .run(
                &db,
                "SELECT count(*) AS n FROM customer; SELECT count(*) AS n FROM sqlite_schema WHERE name = 'never'",
            )
            .unwrap();
        assert_eq!(left.sets[0].rows[0][0].as_deref(), Some("2"), "explaining runs nothing");
        assert_eq!(left.sets[1].rows[0][0].as_deref(), Some("0"));
        assert!(sqlite.explain(&db, "SELECT * FROM nowhere").unwrap_err().contains("no such table"));
        let err = sqlite.explain(&db, "SELECT 1;\nSELECT\n  2;\nSELECT nope FROM customer").unwrap_err();
        assert!(err.contains("error in statement 3") && err.contains("no such column"), "{err}");

        let err = sqlite.run(&db, "SELECT * FROM nowhere").unwrap_err();
        assert!(err.contains("no such table"), "{err}");
        // Safe mode: dot-commands that reach the shell are refused.
        let shell = sqlite.run(&db, ".shell echo hi");
        assert!(shell.is_err(), "{shell:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
