//! SQLite databases through the `sqlite3` program: their tables, views and columns, and SQL
//! run against them with each result as a table. The program runs in its safe mode, so a
//! `.sql` file can't reach the shell or other files through dot-commands.
//!
//! The extension has no database code of its own and opens no connections: it runs the same
//! tool anyone would use in a terminal. Each operation is the command to run ([`Call`]) and how
//! to read what it printed.

use serde_json::Value;

use crate::{Call, Out};

/// Rows kept from one result; the rest are counted, not held.
pub const MAX_ROWS: usize = 10_000;
/// The name of the extra result that says how many rows the statements changed.
const CHANGES: &str = "__coiman_changes";

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

/// The tables and views, with their columns, foreign keys and indexes, in four results.
const SCHEMA: &str = "SELECT m.name AS t, m.type AS k, p.name AS c, p.type AS ty, p.pk AS pk, p.\"notnull\" AS nn \
           FROM sqlite_schema m JOIN pragma_table_info(m.name) p \
           WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' ORDER BY m.name, p.cid;\n\
           SELECT m.name AS t, f.\"from\" AS c, f.\"table\" AS rt, f.\"to\" AS rc \
           FROM sqlite_schema m JOIN pragma_foreign_key_list(m.name) f WHERE m.type = 'table';\n\
           SELECT name AS t, type AS k FROM sqlite_schema WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%';\n\
           SELECT m.name AS t, il.name AS i, il.\"unique\" AS u, il.origin AS o, il.partial AS p, ii.name AS ic \
           FROM sqlite_schema m JOIN pragma_index_list(m.name) il JOIN pragma_index_info(il.name) ii \
           WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' ORDER BY m.name, il.seq, ii.seqno;\n";

/// `sqlite3` run on a database (its absolute path) with `sql` on its standard input; read-only
/// when it shouldn't change anything.
fn call(db: &str, sql: String, read_only: bool) -> Call {
    let mut args: Vec<String> =
        ["-safe", "-batch", "-bail", "-json", "-cmd", ".timeout 5000"].iter().map(|a| (*a).to_owned()).collect();
    if read_only {
        args.push("-readonly".into());
    }
    args.push(db.to_owned());
    Call::new("sqlite3", args, sql, Vec::new())
}

/// What sqlite3 printed, or what its failure means.
fn read(out: &Out) -> Result<String, String> {
    if !out.ok() {
        let err = out.stderr.trim();
        return Err(if err.is_empty() { out.stopped("sqlite3") } else { err.to_owned() });
    }
    Ok(out.stdout.clone())
}

/// Runs SQL (any number of statements); the total changed comes back as one more result.
pub fn run_call(db: &str, sql: &str) -> Call {
    call(db, format!("{sql}\n;\nSELECT total_changes() AS \"{CHANGES}\";\n"), false)
}

pub fn run_read(out: &Out) -> Result<Outcome, String> {
    let mut sets = parse_sets(&read(out)?)?;
    let mut changes = 0;
    if sets.last().is_some_and(|s| s.columns == [CHANGES]) {
        let last = sets.pop().unwrap_or_default();
        changes = last.rows.first().and_then(|r| r[0].as_deref()).and_then(|v| v.parse().ok()).unwrap_or(0);
    }
    Ok(Outcome { sets, changes, millis: 0 })
}

/// The database's tables and views, with their columns and foreign keys.
pub fn schema_call(db: &str) -> Call {
    call(db, SCHEMA.to_owned(), true)
}

pub fn schema_read(out: &Out) -> Result<Vec<Table>, String> {
    let out = read(out)?;
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

/// How SQLite would carry out each statement (`EXPLAIN QUERY PLAN`), without running any: the
/// database is opened read-only. Also the statements and the line each starts on.
pub fn explain_call(db: &str, sql: &str) -> (Call, Vec<String>, Vec<usize>) {
    let statements = split_statements(sql);
    // `.explain off` makes the plan come back as rows, not as the shell's drawn tree.
    let mut text = String::from(".explain off\n");
    let mut starts = Vec::new();
    for (i, statement) in statements.iter().enumerate() {
        text.push_str(&format!("SELECT {i} AS \"{PLAN_MARK}\";\n"));
        starts.push(text.lines().count() + 1);
        text.push_str(&format!("EXPLAIN QUERY PLAN {};\n", strip_explain(statement)));
    }
    (call(db, text, true), statements, starts)
}

pub fn explain_read(out: &Out, statements: Vec<String>, starts: &[usize]) -> Result<Vec<Plan>, String> {
    let out = read(out).map_err(|e| name_statement(&e, starts))?;
    let mut plans: Vec<Plan> = statements.into_iter().map(|statement| Plan { statement, steps: Vec::new() }).collect();
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
/// The dollar-quote tag starting at `i` (`$$` or `$name$`), unless `$` is part of a name.
fn dollar_tag(chars: &[char], i: usize) -> Option<Vec<char>> {
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return None;
    }
    let mut j = i + 1;
    while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
        j += 1;
    }
    let first = chars.get(i + 1).copied();
    (chars.get(j) == Some(&'$') && !first.is_some_and(|c| c.is_ascii_digit())).then(|| chars[i..=j].to_vec())
}

/// Where `needle` next starts at or after `from`.
fn find_from(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    (from..(chars.len() + 1).saturating_sub(needle.len())).find(|&k| chars[k..k + needle.len()] == *needle)
}

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
            // PostgreSQL's dollar quotes: `$$ ... $$` or `$tag$ ... $tag$`, a function's body.
            '$' if dollar_tag(&chars, i).is_some_and(|tag| find_from(&chars, i + tag.len(), &tag).is_some()) => {
                let tag = dollar_tag(&chars, i).unwrap_or_default();
                let end = find_from(&chars, i + tag.len(), &tag).unwrap_or(chars.len()) + tag.len();
                current.extend(&chars[i..end.min(chars.len())]);
                i = end;
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
    fn dollar_quoted_bodies_stay_whole() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql;\nSELECT $tag$ a;b $tag$; SELECT $1;";
        let got = split_statements(sql);
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(got[0].ends_with("LANGUAGE plpgsql"));
        assert_eq!(got[1], "SELECT $tag$ a;b $tag$");
        assert_eq!(got[2], "SELECT $1");
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
}
