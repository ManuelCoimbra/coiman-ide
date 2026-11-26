//! The SQL Databases extension's Compare and Migrations (`plugins/extensions/sql`) in the real
//! plugin host, with the real `sqlite3`: two SQLite files compared, the SQL it writes run on the
//! open one until nothing differs, and the project's migrations checked against the table each
//! tool keeps.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::host::{PickAction, PickChoice};
use coiman_core::extensions::plugin::{Block, Event, FormValues, Permission, PluginHost, Update, registry};

const ID: &str = "coiman.sql";

thread_local! {
    static BACKLOG: std::cell::RefCell<std::collections::VecDeque<Update>> = Default::default();
}

fn wait(host: &PluginHost, mut want: impl FnMut(&Update) -> bool) -> Update {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        BACKLOG.with_borrow_mut(|b| b.extend(host.poll()));
        let found = BACKLOG.with_borrow_mut(|b| {
            while let Some(u) = b.pop_front() {
                if let Update::Stopped { error, .. } = &u {
                    panic!("the extension stopped: {error}");
                }
                if want(&u) {
                    return Some(u);
                }
            }
            None
        });
        if let Some(u) = found {
            return u;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for an update");
}

fn panel(host: &PluginHost, until: &str) -> String {
    panel_all(host, &[until])
}

/// The panel's text once it holds all of `until`.
fn panel_all(host: &PluginHost, until: &[&str]) -> String {
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            assert!(!last.contains("stopped (exit"), "{last}");
            if until.iter().all(|u| last.contains(u)) {
                return last;
            }
        }
    }
    panic!("the panel never said {until:?}; last:\n{last}");
}

fn text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(t) | Block::Text(t) => out.push_str(&format!("{t}\n")),
            Block::Spans(spans) => {
                out.push_str(&spans.iter().map(|s| s.text.as_str()).collect::<String>());
                out.push('\n');
            }
            Block::Rows(rows) => {
                for r in rows {
                    let cells: Vec<&str> = r.cells.iter().map(|s| s.text.as_str()).collect();
                    let buttons: Vec<String> =
                        r.buttons.iter().map(|b| format!("[{} {} {}]", b.label, b.command, b.argument)).collect();
                    out.push_str(&format!("{} {}\n", cells.join(" | "), buttons.join(" ")));
                }
            }
            Block::Table(t) => {
                out.push_str(&format!("table {}\n", t.headers.join(",")));
                for r in &t.rows {
                    out.push_str(&format!("  {}\n", r.join(",")));
                }
            }
            Block::Form(f) => {
                let fields: Vec<String> = f.fields.iter().filter(|f| !f.id.is_empty()).map(|f| f.id.clone()).collect();
                out.push_str(&format!("form {} ({}) {:?}\n", f.id, fields.join(","), f.buttons));
            }
            _ => {}
        }
    }
    out
}

fn submit(host: &PluginHost, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(ID, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

fn sqlite(db: &Path, sql: &str) {
    let mut child = std::process::Command::new("sqlite3")
        .arg(db)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("sqlite3 is on PATH");
    use std::io::Write;
    child.stdin.take().unwrap().write_all(sql.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success());
}

fn pick(host: &PluginHost, text: &str) {
    host.deliver(ID, Event::Picked(PickChoice { pick: "compare".into(), text: text.into(), action: PickAction::Run }));
}

#[test]
fn sql_compare_and_migrations() {
    let tmp = std::env::temp_dir().join(format!("coiman-sql-cmp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("project");
    std::fs::create_dir_all(&ws).unwrap();
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe { std::env::set_var("APPDATA", tmp.join("appdata")) };
    let dev = ws.join("dev.db");
    let prod = ws.join("prod.db");
    sqlite(
        &dev,
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\n\
         CREATE INDEX users_name ON users(name);\n\
         CREATE TABLE old_log (id INTEGER PRIMARY KEY);\n\
         INSERT INTO users VALUES (1, 'ann'), (2, 'bob');\n\
         CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT, success BOOLEAN);\n\
         INSERT INTO _sqlx_migrations VALUES (20240101000000, 'add users', 1), (20240102000000, 'add orders', 0);\n\
         CREATE TABLE alembic_version (version_num TEXT);\n\
         INSERT INTO alembic_version VALUES ('aa');\n",
    );
    sqlite(
        &prod,
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT);\n\
         CREATE UNIQUE INDEX users_name ON users(name);\n\
         CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), total NUMERIC);\n\
         CREATE INDEX orders_user ON orders(user_id);\n\
         CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT, success BOOLEAN);\n\
         CREATE TABLE alembic_version (version_num TEXT);\n",
    );
    let w = |rel: &str, text: &str| {
        let p = ws.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    };
    w("migrations/20240101000000_add_users.sql", "CREATE TABLE users (id INTEGER);");
    w("migrations/20240102000000_add_orders.sql", "CREATE TABLE orders (id INTEGER);");
    w("migrations/20240103000000_add_email.sql", "ALTER TABLE users ADD email TEXT;");
    w("alembic/versions/aa_first.py", "\"\"\"create users\"\"\"\nrevision = 'aa'\ndown_revision = None\n");
    w("alembic/versions/bb_second.py", "\"\"\"add email\"\"\"\nrevision = 'bb'\ndown_revision = 'aa'\n");
    w("db/migration/V1__init.sql", "SELECT 1;");

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/sql");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    let programs = ["sqlite3", "psql", "mariadb", "mysql", "sqlcmd", "SqlLocalDB"].map(str::to_owned).into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, programs);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // Migrations before a database is open: found, not yet checked.
    host.run_command(ID, "migrations", "");
    let m = panel(&host, "open the database to see which have run");
    assert!(
        m.contains("20240103000000 | add email | ? [Open open-file migrations/20240103000000_add_email.sql]"),
        "{m}"
    );

    host.run_command(ID, "open-sqlite", &dev.to_string_lossy());
    panel(&host, "users | 2 columns");
    host.run_command(ID, "migrations", "");
    let m = panel_all(&host, &["1 ran, 1 not run, 1 failed", "aa | create users | ran"]);
    assert!(m.contains("20240101000000 | add users | ran"), "{m}");
    assert!(m.contains("20240102000000 | add orders | failed"), "{m}");
    assert!(m.contains("20240103000000 | add email | not run"), "{m}");
    assert!(m.contains("aa | create users | ran") && m.contains("bb | add email | not run"), "{m}");
    // No flyway_schema_history table: none of Flyway's ran here.
    assert!(m.contains("1 | init | not run"), "{m}");

    // Compared with the other file: what differs, and SQL that makes this one like it.
    host.run_command(ID, "compare", "");
    let offered = wait(&host, |u| matches!(u, Update::Pick { .. }));
    let Update::Pick { pick: offered, .. } = offered else { unreachable!() };
    let items: Vec<String> = offered.items.iter().map(|i| i.text.clone()).collect();
    assert_eq!(items, ["prod.db"], "the open file isn't offered");
    pick(&host, "prod.db");
    let c = panel(&host, "Compared with prod.db");
    assert!(c.contains("+ | table orders (3 columns)"), "{c}");
    assert!(c.contains("~ | users.name: TEXT → TEXT not null"), "{c}");
    assert!(c.contains("+ | users.email: TEXT"), "{c}");
    assert!(c.contains("− | table old_log"), "{c}");

    // The SQL, run on the open database, leaves nothing to tell apart.
    host.run_command(ID, "compare-box", "");
    panel(&host, "form sql");
    host.run_command(ID, "compare-open", "");
    let sql = match wait(&host, |u| matches!(u, Update::OpenText(_))) {
        Update::OpenText(t) => t,
        _ => unreachable!(),
    };
    assert!(
        sql.contains("CREATE TABLE \"users__coiman_new\"") || sql.contains("CREATE TABLE \"Users__coiman_new\""),
        "{sql}"
    );
    sqlite(&dev, &sql);
    host.run_command(ID, "compare-again", "");
    panel(&host, "Same tables, columns, keys and indexes.");
    // The rows came through the rebuild.
    host.run_command(ID, "compare-close", "");
    submit(&host, "sql", "Run", &[("sql", "SELECT name FROM users ORDER BY id")]);
    let rows = panel(&host, "table name");
    assert!(rows.contains("  ann\n  bob\n"), "{rows}");

    let _ = std::fs::remove_dir_all(&tmp);
}
