//! Two databases' tables compared, and the SQL that turns one into the other, in the dialect of
//! the one that changes. Only what the schema read says is compared: tables, their columns
//! (type, NOT NULL, primary key), foreign keys and indexes. Views are listed, not written:
//! their definitions aren't read.

use std::collections::{BTreeMap, BTreeSet};

use crate::sqlite::{Column, ForeignKey, Index, Table};

/// Whose SQL the script is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    MySql,
    SqlServer,
}

impl Dialect {
    pub fn quote(self, name: &str) -> String {
        match self {
            Self::Sqlite | Self::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
            Self::MySql => format!("`{}`", name.replace('`', "``")),
            Self::SqlServer => format!("[{}]", name.replace(']', "]]")),
        }
    }

    /// An index's column: a name quoted, an expression (`(lower(name))`) as it is.
    fn index_column(self, column: &str) -> String {
        if column.starts_with('(') { column.to_owned() } else { self.quote(column) }
    }
}

/// One difference: what the database being changed lacks, has too many of, or has otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    AddTable(Table),
    DropTable(Table),
    /// A view only in one of them: listed, not written.
    View {
        name: String,
        added: bool,
    },
    AddColumn {
        table: String,
        column: Column,
    },
    DropColumn {
        table: String,
        column: Column,
    },
    AlterColumn {
        table: String,
        from: Column,
        to: Column,
    },
    /// The primary key's columns differ.
    PrimaryKey {
        table: String,
        from: Vec<String>,
        to: Vec<String>,
    },
    AddForeignKey {
        table: String,
        key: ForeignKey,
    },
    DropForeignKey {
        table: String,
        key: ForeignKey,
    },
    AddIndex {
        table: String,
        index: Index,
    },
    DropIndex {
        table: String,
        index: Index,
    },
}

impl Change {
    pub fn table(&self) -> &str {
        match self {
            Change::AddTable(t) | Change::DropTable(t) => &t.name,
            Change::View { name, .. } => name,
            Change::AddColumn { table, .. }
            | Change::DropColumn { table, .. }
            | Change::AlterColumn { table, .. }
            | Change::PrimaryKey { table, .. }
            | Change::AddForeignKey { table, .. }
            | Change::DropForeignKey { table, .. }
            | Change::AddIndex { table, .. }
            | Change::DropIndex { table, .. } => table,
        }
    }

    /// "+" for what the other has and this lacks, "−" for what only this has, "~" for changed.
    pub fn sign(&self) -> &'static str {
        match self {
            Change::AddTable(_)
            | Change::AddColumn { .. }
            | Change::AddForeignKey { .. }
            | Change::AddIndex { .. }
            | Change::View { added: true, .. } => "+",
            Change::DropTable(_)
            | Change::DropColumn { .. }
            | Change::DropForeignKey { .. }
            | Change::DropIndex { .. }
            | Change::View { added: false, .. } => "−",
            Change::AlterColumn { .. } | Change::PrimaryKey { .. } => "~",
        }
    }

    /// What it is, in a few words.
    pub fn describe(&self) -> String {
        match self {
            Change::AddTable(t) => format!("table {} ({} columns)", t.name, t.columns.len()),
            Change::DropTable(t) => format!("table {}", t.name),
            Change::View { name, .. } => format!("view {name}"),
            Change::AddColumn { table, column } | Change::DropColumn { table, column } => {
                format!("{table}.{}: {}", column.name, column_text(column))
            }
            Change::AlterColumn { table, from, to } => {
                format!("{table}.{}: {} → {}", to.name, column_text(from), column_text(to))
            }
            Change::PrimaryKey { table, from, to } => {
                format!("{table}: primary key ({}) → ({})", from.join(", "), to.join(", "))
            }
            Change::AddForeignKey { table, key } | Change::DropForeignKey { table, key } => {
                format!("{table}.{} → {}.{}", key.column, key.table, key.to)
            }
            Change::AddIndex { table, index } | Change::DropIndex { table, index } => format!(
                "{}index {} on {table} ({})",
                if index.unique { "unique " } else { "" },
                index.name,
                index.columns.join(", ")
            ),
        }
    }
}

fn column_text(c: &Column) -> String {
    let mut s = if c.ty.is_empty() { "(no type)".to_owned() } else { c.ty.clone() };
    if c.not_null {
        s.push_str(" not null");
    }
    s
}

/// Types alike when they differ only in case and spaces.
fn same_type(a: &str, b: &str) -> bool {
    let n = |t: &str| t.to_ascii_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    n(a) == n(b)
}

fn primary_key(t: &Table) -> Vec<String> {
    t.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()
}

/// The indexes made with CREATE INDEX (a key's own index goes with the key).
fn own_indexes(t: &Table) -> impl Iterator<Item = &Index> {
    t.indexes.iter().filter(|i| !i.automatic)
}

/// Tables migration tools keep what ran in: they say where each database is, so they're never
/// compared (and never dropped or made by the SQL written here).
pub const TOOL_TABLES: [&str; 10] = [
    "_sqlx_migrations",
    "__diesel_schema_migrations",
    "alembic_version",
    "flyway_schema_history",
    "schema_migrations",
    "__efmigrationshistory",
    "goose_db_version",
    "knex_migrations",
    "knex_migrations_lock",
    "django_migrations",
];

pub fn is_tool_table(name: &str) -> bool {
    TOOL_TABLES.contains(&name.to_lowercase().as_str())
}

/// What turns `this` into `other`, leaving out migration tools' own tables. Names are matched
/// without regard to case.
pub fn diff(this: &[Table], other: &[Table]) -> Vec<Change> {
    let key = |n: &str| n.to_lowercase();
    let kept = |t: &&Table| !is_tool_table(&t.name);
    let mine: BTreeMap<String, &Table> = this.iter().filter(kept).map(|t| (key(&t.name), t)).collect();
    let theirs: BTreeMap<String, &Table> = other.iter().filter(kept).map(|t| (key(&t.name), t)).collect();
    let mut out = Vec::new();
    for (k, t) in &theirs {
        match mine.get(k) {
            None if t.view => out.push(Change::View { name: t.name.clone(), added: true }),
            None => out.push(Change::AddTable((*t).clone())),
            Some(m) => out.extend(table_diff(m, t)),
        }
    }
    for (k, t) in &mine {
        if !theirs.contains_key(k) {
            out.push(if t.view {
                Change::View { name: t.name.clone(), added: false }
            } else {
                Change::DropTable((*t).clone())
            });
        }
    }
    out
}

fn table_diff(mine: &Table, theirs: &Table) -> Vec<Change> {
    let mut out = Vec::new();
    if mine.view || theirs.view {
        // A view on one side and a table on the other, or two views: left as they are.
        return out;
    }
    let name = &mine.name;
    let col = |t: &Table, n: &str| t.columns.iter().find(|c| c.name.eq_ignore_ascii_case(n)).cloned();
    for c in &theirs.columns {
        match col(mine, &c.name) {
            None => out.push(Change::AddColumn { table: name.clone(), column: c.clone() }),
            Some(m) if !same_type(&m.ty, &c.ty) || m.not_null != c.not_null => {
                out.push(Change::AlterColumn { table: name.clone(), from: m, to: c.clone() })
            }
            Some(_) => {}
        }
    }
    for c in &mine.columns {
        if col(theirs, &c.name).is_none() {
            out.push(Change::DropColumn { table: name.clone(), column: c.clone() });
        }
    }
    let (pk_mine, pk_theirs) = (primary_key(mine), primary_key(theirs));
    let lower = |v: &[String]| v.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>();
    if lower(&pk_mine) != lower(&pk_theirs) {
        out.push(Change::PrimaryKey { table: name.clone(), from: pk_mine, to: pk_theirs });
    }
    let fk = |k: &ForeignKey| (k.column.to_lowercase(), k.table.to_lowercase(), k.to.to_lowercase());
    let fks_mine: BTreeSet<_> = mine.foreign_keys.iter().map(fk).collect();
    let fks_theirs: BTreeSet<_> = theirs.foreign_keys.iter().map(fk).collect();
    for k in &theirs.foreign_keys {
        if !fks_mine.contains(&fk(k)) {
            out.push(Change::AddForeignKey { table: name.clone(), key: k.clone() });
        }
    }
    for k in &mine.foreign_keys {
        if !fks_theirs.contains(&fk(k)) {
            out.push(Change::DropForeignKey { table: name.clone(), key: k.clone() });
        }
    }
    let same_index =
        |a: &Index, b: &Index| a.unique == b.unique && a.partial == b.partial && lower(&a.columns) == lower(&b.columns);
    for i in own_indexes(theirs) {
        match own_indexes(mine).find(|m| m.name.eq_ignore_ascii_case(&i.name)) {
            Some(m) if same_index(m, i) => {}
            Some(m) => {
                out.push(Change::DropIndex { table: name.clone(), index: m.clone() });
                out.push(Change::AddIndex { table: name.clone(), index: i.clone() });
            }
            None => out.push(Change::AddIndex { table: name.clone(), index: i.clone() }),
        }
    }
    for m in own_indexes(mine) {
        if !own_indexes(theirs).any(|i| i.name.eq_ignore_ascii_case(&m.name)) {
            out.push(Change::DropIndex { table: name.clone(), index: m.clone() });
        }
    }
    out
}

fn column_sql(d: Dialect, c: &Column) -> String {
    let ty = if c.ty.is_empty() && d != Dialect::Sqlite { "TEXT" } else { c.ty.as_str() };
    let mut s = format!("{} {ty}", d.quote(&c.name));
    if c.not_null {
        s.push_str(" NOT NULL");
    }
    s.trim_end().to_owned()
}

fn create_table(d: Dialect, t: &Table, name: &str) -> String {
    let mut lines: Vec<String> = t.columns.iter().map(|c| format!("  {}", column_sql(d, c))).collect();
    let pk = primary_key(t);
    if !pk.is_empty() {
        lines.push(format!("  PRIMARY KEY ({})", pk.iter().map(|c| d.quote(c)).collect::<Vec<_>>().join(", ")));
    }
    for k in &t.foreign_keys {
        lines.push(format!(
            "  FOREIGN KEY ({}) REFERENCES {} ({})",
            d.quote(&k.column),
            d.quote(&k.table),
            d.quote(&k.to)
        ));
    }
    format!("CREATE TABLE {} (\n{}\n);", d.quote(name), lines.join(",\n"))
}

fn create_index(d: Dialect, table: &str, i: &Index) -> String {
    let columns = i.columns.iter().map(|c| d.index_column(c)).collect::<Vec<_>>().join(", ");
    let mut s = format!(
        "CREATE {}INDEX {} ON {} ({columns});",
        if i.unique { "UNIQUE " } else { "" },
        d.quote(&i.name),
        d.quote(table)
    );
    if i.partial {
        s.push_str("\n-- In the other database this index is partial (CREATE INDEX … WHERE): add its WHERE.");
    }
    s
}

fn drop_index(d: Dialect, table: &str, i: &Index) -> String {
    match d {
        Dialect::Sqlite | Dialect::Postgres => format!("DROP INDEX {};", d.quote(&i.name)),
        Dialect::MySql | Dialect::SqlServer => format!("DROP INDEX {} ON {};", d.quote(&i.name), d.quote(table)),
    }
}

fn add_column(d: Dialect, table: &str, c: &Column) -> String {
    let add = if d == Dialect::SqlServer { "ADD" } else { "ADD COLUMN" };
    let mut s = format!("ALTER TABLE {} {add} {};", d.quote(table), column_sql(d, c));
    if c.not_null {
        s.push_str(&format!(
            "\n-- {} is NOT NULL: the rows already there need a value (add a DEFAULT, or fill it in first).",
            c.name
        ));
    }
    s
}

fn alter_column(d: Dialect, table: &str, from: &Column, to: &Column) -> Vec<String> {
    let (t, c) = (d.quote(table), d.quote(&to.name));
    match d {
        Dialect::Postgres => {
            let mut out = Vec::new();
            if !same_type(&from.ty, &to.ty) {
                out.push(format!("ALTER TABLE {t} ALTER COLUMN {c} TYPE {} USING {c}::{};", to.ty, to.ty));
            }
            if from.not_null != to.not_null {
                let how = if to.not_null { "SET" } else { "DROP" };
                out.push(format!("ALTER TABLE {t} ALTER COLUMN {c} {how} NOT NULL;"));
            }
            out
        }
        Dialect::MySql => vec![format!("ALTER TABLE {t} MODIFY COLUMN {};", column_sql(d, to))],
        Dialect::SqlServer => {
            let null = if to.not_null { "NOT NULL" } else { "NULL" };
            vec![format!("ALTER TABLE {t} ALTER COLUMN {c} {} {null};", to.ty)]
        }
        // Rebuilt instead (see `script`).
        Dialect::Sqlite => Vec::new(),
    }
}

/// Changes SQLite's ALTER TABLE can't make: the table is made again in the other's shape.
fn needs_rebuild(c: &Change, this: &[Table]) -> bool {
    match c {
        Change::AlterColumn { .. }
        | Change::PrimaryKey { .. }
        | Change::AddForeignKey { .. }
        | Change::DropForeignKey { .. } => true,
        // DROP COLUMN refuses a column in a key or an index.
        Change::DropColumn { table, column } => this.iter().find(|t| &t.name == table).is_some_and(|t| {
            column.primary_key
                || t.foreign_keys.iter().any(|k| k.column.eq_ignore_ascii_case(&column.name))
                || t.indexes.iter().any(|i| i.columns.iter().any(|ic| ic.eq_ignore_ascii_case(&column.name)))
        }),
        // A NOT NULL column can't be added without a default.
        Change::AddColumn { column, .. } => column.not_null,
        _ => false,
    }
}

/// Tables to create, those others point to first.
fn creation_order(tables: Vec<&Table>) -> Vec<&Table> {
    let mut left = tables;
    let mut out: Vec<&Table> = Vec::new();
    while !left.is_empty() {
        let names: BTreeSet<String> = left.iter().map(|t| t.name.to_lowercase()).collect();
        let ready = left.iter().position(|t| {
            t.foreign_keys
                .iter()
                .all(|k| !names.contains(&k.table.to_lowercase()) || k.table.eq_ignore_ascii_case(&t.name))
        });
        // A cycle: the rest as they come.
        out.push(left.remove(ready.unwrap_or(0)));
    }
    out
}

/// The SQL that turns `this` into `other`, as `diff` found, written for `d`.
pub fn script(changes: &[Change], this: &[Table], other: &[Table], d: Dialect) -> String {
    if changes.is_empty() {
        return String::new();
    }
    let mut out: Vec<String> = Vec::new();
    let find = |tables: &[Table], name: &str| tables.iter().find(|t| t.name.eq_ignore_ascii_case(name)).cloned();
    // SQLite: tables made again rather than altered.
    let rebuilt: BTreeSet<String> = if d == Dialect::Sqlite {
        changes.iter().filter(|c| needs_rebuild(c, this)).map(|c| c.table().to_owned()).collect()
    } else {
        BTreeSet::new()
    };
    let mut views = Vec::new();
    for c in changes {
        if let Change::View { name, added } = c {
            views.push(format!(
                "-- View {name} is only in the {} database: views aren't written here.",
                if *added { "other" } else { "open" }
            ));
        }
    }
    out.extend(views);
    // Keys and indexes that go, first.
    for c in changes {
        if rebuilt.contains(c.table()) {
            continue;
        }
        match c {
            Change::DropIndex { table, index } => out.push(drop_index(d, table, index)),
            Change::DropForeignKey { table, key } => out.push(drop_foreign_key(d, table, key)),
            _ => {}
        }
    }
    let created: Vec<&Table> = changes
        .iter()
        .filter_map(|c| match c {
            Change::AddTable(t) => Some(t),
            _ => None,
        })
        .collect();
    for t in creation_order(created) {
        out.push(create_table(d, t, &t.name));
        out.extend(own_indexes(t).map(|i| create_index(d, &t.name, i)));
    }
    for c in changes {
        if rebuilt.contains(c.table()) {
            continue;
        }
        match c {
            Change::AddColumn { table, column } => out.push(add_column(d, table, column)),
            Change::DropColumn { table, column } => {
                out.push(format!("ALTER TABLE {} DROP COLUMN {};", d.quote(table), d.quote(&column.name)))
            }
            Change::AlterColumn { table, from, to } => out.extend(alter_column(d, table, from, to)),
            Change::PrimaryKey { table, from, to } => out.push(primary_key_sql(d, table, from, to)),
            _ => {}
        }
    }
    for name in &rebuilt {
        let (Some(mine), Some(theirs)) = (find(this, name), find(other, name)) else { continue };
        out.push(rebuild(&mine, &theirs));
    }
    for c in changes {
        if rebuilt.contains(c.table()) {
            continue;
        }
        match c {
            Change::AddForeignKey { table, key } => out.push(format!(
                "ALTER TABLE {} ADD FOREIGN KEY ({}) REFERENCES {} ({});",
                d.quote(table),
                d.quote(&key.column),
                d.quote(&key.table),
                d.quote(&key.to)
            )),
            Change::AddIndex { table, index } => out.push(create_index(d, table, index)),
            _ => {}
        }
    }
    // Tables that go, last: keys pointing at them went before.
    for c in changes {
        if let Change::DropTable(t) = c {
            out.push(format!("DROP TABLE {};", d.quote(&t.name)));
        }
    }
    let mut text = out.join("\n\n");
    text.push('\n');
    text
}

fn drop_foreign_key(d: Dialect, table: &str, key: &ForeignKey) -> String {
    match d {
        // PostgreSQL's own name for it, unless it was given another.
        Dialect::Postgres => format!(
            "ALTER TABLE {} DROP CONSTRAINT {};  -- PostgreSQL's usual name for it",
            d.quote(table),
            d.quote(&format!("{table}_{}_fkey", key.column))
        ),
        _ => format!(
            "-- Drop the foreign key {table}.{} → {}.{}: its name isn't read here (ALTER TABLE {} DROP {} <name>).",
            key.column,
            key.table,
            key.to,
            d.quote(table),
            if d == Dialect::MySql { "FOREIGN KEY" } else { "CONSTRAINT" }
        ),
    }
}

fn primary_key_sql(d: Dialect, table: &str, from: &[String], to: &[String]) -> String {
    let t = d.quote(table);
    let cols = to.iter().map(|c| d.quote(c)).collect::<Vec<_>>().join(", ");
    match d {
        Dialect::MySql => {
            let mut parts = Vec::new();
            if !from.is_empty() {
                parts.push("DROP PRIMARY KEY".to_owned());
            }
            if !to.is_empty() {
                parts.push(format!("ADD PRIMARY KEY ({cols})"));
            }
            format!("ALTER TABLE {t} {};", parts.join(", "))
        }
        Dialect::Postgres => {
            let mut s = String::new();
            if !from.is_empty() {
                s.push_str(&format!(
                    "ALTER TABLE {t} DROP CONSTRAINT {};  -- PostgreSQL's usual name for it\n",
                    d.quote(&format!("{table}_pkey"))
                ));
            }
            if !to.is_empty() {
                s.push_str(&format!("ALTER TABLE {t} ADD PRIMARY KEY ({cols});"));
            }
            s.trim_end().to_owned()
        }
        _ => {
            let mut s = String::new();
            if !from.is_empty() {
                s.push_str(&format!("-- Drop {table}'s primary key: its name isn't read here (ALTER TABLE {t} DROP CONSTRAINT <name>).\n"));
            }
            if !to.is_empty() {
                s.push_str(&format!("ALTER TABLE {t} ADD PRIMARY KEY ({cols});"));
            }
            s.trim_end().to_owned()
        }
    }
}

/// SQLite's way to change a table: a new one in the other's shape, the rows copied over (the
/// columns both have), the old one dropped and the new one renamed, its indexes made again.
fn rebuild(mine: &Table, theirs: &Table) -> String {
    let d = Dialect::Sqlite;
    let temp = format!("{}__coiman_new", theirs.name);
    let common: Vec<String> = theirs
        .columns
        .iter()
        .filter(|c| mine.columns.iter().any(|m| m.name.eq_ignore_ascii_case(&c.name)))
        .map(|c| d.quote(&c.name))
        .collect();
    let mut s = format!("-- SQLite can't alter {} this way: it is made again, its rows copied.\n", theirs.name);
    s.push_str("PRAGMA foreign_keys = OFF;\nBEGIN;\n");
    s.push_str(&create_table(d, theirs, &temp));
    s.push('\n');
    if !common.is_empty() {
        s.push_str(&format!(
            "INSERT INTO {} ({cols}) SELECT {cols} FROM {};\n",
            d.quote(&temp),
            d.quote(&mine.name),
            cols = common.join(", ")
        ));
    }
    s.push_str(&format!("DROP TABLE {};\n", d.quote(&mine.name)));
    s.push_str(&format!("ALTER TABLE {} RENAME TO {};\n", d.quote(&temp), d.quote(&theirs.name)));
    for i in own_indexes(theirs) {
        s.push_str(&create_index(d, &theirs.name, i));
        s.push('\n');
    }
    for c in &theirs.columns {
        if c.not_null && !mine.columns.iter().any(|m| m.name.eq_ignore_ascii_case(&c.name)) {
            s.push_str(&format!(
                "-- {} is NOT NULL and new: the copied rows need a value (add a DEFAULT to it above).\n",
                c.name
            ));
        }
    }
    s.push_str("PRAGMA foreign_key_check;\nCOMMIT;\nPRAGMA foreign_keys = ON;");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, ty: &str, pk: bool, nn: bool) -> Column {
        Column { name: name.into(), ty: ty.into(), primary_key: pk, not_null: nn }
    }

    fn table(name: &str, columns: Vec<Column>) -> Table {
        Table { name: name.into(), view: false, columns, foreign_keys: vec![], indexes: vec![] }
    }

    fn index(name: &str, unique: bool, columns: &[&str]) -> Index {
        Index {
            name: name.into(),
            unique,
            automatic: false,
            partial: false,
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    /// The open database (dev) and the other one (what it should become).
    fn pair() -> (Vec<Table>, Vec<Table>) {
        let mut users = table("users", vec![col("id", "INTEGER", true, true), col("name", "TEXT", false, false)]);
        users.indexes.push(index("users_name", false, &["name"]));
        let old = table("old_log", vec![col("id", "INTEGER", true, true)]);
        let this = vec![users, old];

        let mut users = table(
            "Users",
            vec![
                col("id", "integer", true, true),
                col("name", "TEXT", false, true),
                col("email", "TEXT", false, false),
            ],
        );
        users.indexes.push(index("users_name", true, &["name"]));
        users.indexes.push(index("users_email", true, &["(lower(email))"]));
        let mut orders = table(
            "orders",
            vec![
                col("id", "INTEGER", true, true),
                col("user_id", "INTEGER", false, true),
                col("total", "NUMERIC", false, false),
            ],
        );
        orders.foreign_keys.push(ForeignKey { column: "user_id".into(), table: "users".into(), to: "id".into() });
        let mut lines = table("lines", vec![col("order_id", "INTEGER", false, true)]);
        lines.foreign_keys.push(ForeignKey { column: "order_id".into(), table: "orders".into(), to: "id".into() });
        let view =
            Table { name: "big_orders".into(), view: true, columns: vec![], foreign_keys: vec![], indexes: vec![] };
        (this, vec![users, lines, orders, view])
    }

    #[test]
    fn what_differs() {
        let (this, other) = pair();
        let got: Vec<String> = diff(&this, &other).iter().map(|c| format!("{} {}", c.sign(), c.describe())).collect();
        assert_eq!(
            got,
            [
                "+ view big_orders",
                "+ table lines (1 columns)",
                "+ table orders (3 columns)",
                "~ users.name: TEXT → TEXT not null",
                "+ users.email: TEXT",
                "− index users_name on users (name)",
                "+ unique index users_name on users (name)",
                "+ unique index users_email on users ((lower(email)))",
                "− table old_log",
            ]
        );
        assert!(diff(&other, &other).is_empty(), "nothing differs from itself");
        // A migration tool's own table is never compared: dropping it would lose its history.
        let mut with_history = other.clone();
        with_history.push(table("_sqlx_migrations", vec![col("version", "BIGINT", true, true)]));
        with_history.push(table("__EFMigrationsHistory", vec![col("MigrationId", "TEXT", true, true)]));
        assert!(diff(&with_history, &other).is_empty() && diff(&other, &with_history).is_empty());
    }

    #[test]
    fn postgres_script() {
        let (this, other) = pair();
        let changes = diff(&this, &other);
        let sql = script(&changes, &this, &other, Dialect::Postgres);
        // Tables others point to are made first.
        let orders = sql.find("CREATE TABLE \"orders\"").unwrap();
        let lines = sql.find("CREATE TABLE \"lines\"").unwrap();
        assert!(orders < lines, "{sql}");
        assert!(sql.contains("FOREIGN KEY (\"user_id\") REFERENCES \"users\" (\"id\")"), "{sql}");
        assert!(sql.contains("ALTER TABLE \"users\" ALTER COLUMN \"name\" SET NOT NULL;"), "{sql}");
        assert!(sql.contains("ALTER TABLE \"users\" ADD COLUMN \"email\" TEXT;"), "{sql}");
        assert!(sql.contains("CREATE UNIQUE INDEX \"users_email\" ON \"users\" ((lower(email)));"), "{sql}");
        // The old index goes before its new form comes.
        assert!(
            sql.find("DROP INDEX \"users_name\";").unwrap() < sql.find("CREATE UNIQUE INDEX \"users_name\"").unwrap()
        );
        assert!(sql.trim_end().ends_with("DROP TABLE \"old_log\";"), "{sql}");
        assert!(sql.contains("-- View big_orders is only in the other database"), "{sql}");
    }

    #[test]
    fn other_dialects() {
        let (this, other) = pair();
        let changes = diff(&this, &other);
        let my = script(&changes, &this, &other, Dialect::MySql);
        assert!(my.contains("ALTER TABLE `users` MODIFY COLUMN `name` TEXT NOT NULL;"), "{my}");
        assert!(my.contains("DROP INDEX `users_name` ON `users`;"), "{my}");
        let ms = script(&changes, &this, &other, Dialect::SqlServer);
        assert!(ms.contains("ALTER TABLE [users] ALTER COLUMN [name] TEXT NOT NULL;"), "{ms}");
        assert!(ms.contains("ALTER TABLE [users] ADD [email] TEXT;"), "{ms}");
    }

    #[test]
    fn sqlite_makes_a_table_again() {
        let (this, other) = pair();
        let changes = diff(&this, &other);
        let sql = script(&changes, &this, &other, Dialect::Sqlite);
        // users changes a column: made again, not altered.
        assert!(!sql.contains("ALTER COLUMN") && !sql.contains("ADD COLUMN \"email\""), "{sql}");
        assert!(sql.contains("CREATE TABLE \"Users__coiman_new\" ("), "{sql}");
        assert!(
            sql.contains(
                "INSERT INTO \"Users__coiman_new\" (\"id\", \"name\") SELECT \"id\", \"name\" FROM \"users\";"
            ),
            "{sql}"
        );
        assert!(sql.contains("ALTER TABLE \"Users__coiman_new\" RENAME TO \"Users\";"), "{sql}");
        assert!(sql.contains("CREATE UNIQUE INDEX \"users_email\" ON \"Users\""), "{sql}");
        assert!(sql.contains("PRAGMA foreign_keys = OFF;") && sql.contains("COMMIT;"), "{sql}");
        // Not dropped on its own, as it goes with the table.
        assert!(!sql.contains("DROP INDEX"), "{sql}");
    }

    #[test]
    fn keys_change() {
        let this = vec![table("t", vec![col("a", "INT", true, true), col("b", "INT", false, true)])];
        let other = vec![table("t", vec![col("a", "INT", true, true), col("b", "INT", true, true)])];
        let changes = diff(&this, &other);
        assert_eq!(
            changes,
            [Change::PrimaryKey { table: "t".into(), from: vec!["a".into()], to: vec!["a".into(), "b".into()] }]
        );
        let pg = script(&changes, &this, &other, Dialect::Postgres);
        assert!(pg.contains("DROP CONSTRAINT \"t_pkey\"") && pg.contains("ADD PRIMARY KEY (\"a\", \"b\");"), "{pg}");
        let my = script(&changes, &this, &other, Dialect::MySql);
        assert!(my.contains("ALTER TABLE `t` DROP PRIMARY KEY, ADD PRIMARY KEY (`a`, `b`);"), "{my}");
        // A foreign key that goes: its name is PostgreSQL's usual, elsewhere unknown.
        let mut with_key = this.clone();
        with_key[0].foreign_keys.push(ForeignKey { column: "b".into(), table: "u".into(), to: "id".into() });
        let changes = diff(&with_key, &this);
        let pg = script(&changes, &with_key, &this, Dialect::Postgres);
        assert!(pg.contains("ALTER TABLE \"t\" DROP CONSTRAINT \"t_b_fkey\";"), "{pg}");
        let ms = script(&changes, &with_key, &this, Dialect::SqlServer);
        assert!(ms.starts_with("-- Drop the foreign key t.b → u.id"), "{ms}");
    }

    #[test]
    fn not_null_added_says_what_it_needs() {
        let this = vec![table("t", vec![col("a", "INT", true, true)])];
        let other = vec![table("t", vec![col("a", "INT", true, true), col("b", "INT", false, true)])];
        let pg = script(&diff(&this, &other), &this, &other, Dialect::Postgres);
        assert!(pg.contains("ADD COLUMN \"b\" INT NOT NULL;\n-- b is NOT NULL"), "{pg}");
        let lite = script(&diff(&this, &other), &this, &other, Dialect::Sqlite);
        assert!(lite.contains("-- b is NOT NULL and new"), "{lite}");
    }
}
