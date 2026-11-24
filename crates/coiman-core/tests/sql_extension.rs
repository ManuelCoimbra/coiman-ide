//! The SQL Databases extension (`plugins/extensions/sql`, rebuilt with
//! `scripts/build-plugins.ps1`) in the real plugin host: a SQLite file through the real
//! `sqlite3` (its tables, columns, keys and indexes, SQL run, a plan, the diagram), and a
//! PostgreSQL server through a stand-in `psql` that answers as psql does (the password in its
//! environment, never in its arguments, and never saved).

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    let mut last = String::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(60) {
        if let Update::Panel { blocks, .. } = wait(host, |u| matches!(u, Update::Panel { .. })) {
            last = text(&blocks);
            assert!(!last.contains("stopped (exit") || until.contains("stopped"), "{last}");
            if last.contains(until) {
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

/// A `psql` that keeps its arguments and its PGPASSWORD in `calls.log`, and answers a query as
/// psql does with the extension's markers.
fn fake_psql(dir: &Path) {
    std::fs::write(
        dir.join("psql.cmd"),
        "@echo off\r\n>>\"%~dp0calls.log\" echo psql %* PGPASSWORD=%PGPASSWORD%\r\n\
         echo __coiman_statement 0\r\necho id,name\r\necho 1,ann\r\necho 2,__coiman_null__\r\necho __coiman_rows 2\r\n",
    )
    .unwrap();
}

#[test]
fn sql_extension() {
    let tmp = std::env::temp_dir().join(format!("coiman-sql-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("project");
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&bin).unwrap();
    fake_psql(&bin);
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("appdata"));
    }
    let db = ws.join("shop.db");
    sqlite(
        &db,
        "CREATE TABLE customer (id INTEGER PRIMARY KEY, name TEXT NOT NULL);\n\
         CREATE TABLE orders (id INTEGER PRIMARY KEY, customer INTEGER REFERENCES customer(id), total REAL);\n\
         CREATE INDEX orders_by_customer ON orders(customer);\n\
         INSERT INTO customer VALUES (1, 'ann'), (2, 'bob');\n\
         INSERT INTO orders VALUES (1, 1, 9.5), (2, 1, 3.25), (3, 2, NULL);\n",
    );

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions/sql");
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    assert_eq!(pkg.manifest.openers[0].starts_with, "SQLite format 3\0");
    let host = PluginHost::start(|| {}).unwrap();
    host.set_workspace(Some(ws.clone()));
    let granted: BTreeSet<Permission> = [Permission::WorkspaceRead, Permission::EditorRead].into();
    let programs = ["sqlite3", "psql", "mariadb", "mysql", "sqlcmd", "SqlLocalDB"].map(str::to_owned).into();
    host.load_with_programs(pkg.manifest, pkg.wasm, granted, programs);
    wait(&host, |u| matches!(u, Update::Activated { .. }));

    // A SQLite file, as the opener hands it over: its tables, with columns, keys and indexes.
    host.run_command(ID, "open-sqlite", &db.to_string_lossy());
    let tables = panel(&host, "orders | 3 columns");
    assert!(tables.contains("shop.db") && tables.contains("customer | 2 columns [Rows rows customer]"), "{tables}");
    host.run_command(ID, "table", "orders");
    let cols = panel(&host, "→ customer.id");
    assert!(cols.contains("id | INTEGER | primary key"), "{cols}");
    assert!(cols.contains("orders_by_customer | customer | index"), "{cols}");

    // SQL run: each result a table, NULL said, changes counted.
    submit(
        &host,
        "sql",
        "Run",
        &[(
            "sql",
            "SELECT c.name, SUM(o.total) AS spent FROM customer c JOIN orders o ON o.customer = c.id GROUP BY c.name ORDER BY c.name; UPDATE orders SET total = 1 WHERE total IS NULL;",
        )],
    );
    let run = panel(&host, "table name,spent");
    assert!(run.contains("  ann,12.75\n  bob,NULL\n"), "{run}");
    assert!(run.contains("1 result · 1 rows changed"), "{run}");
    // A table's rows.
    host.run_command(ID, "rows", "customer");
    let rows = panel(&host, "table id,name");
    assert!(rows.contains("  1,ann\n  2,bob\n"), "{rows}");

    // A plan, with a full scan said.
    submit(&host, "sql", "Explain", &[("sql", "SELECT * FROM orders WHERE total > 2")]);
    let plan = panel(&host, "SCAN orders");
    assert!(plan.contains("reads every row: an index may help"), "{plan}");
    submit(&host, "sql", "Explain", &[("sql", "SELECT * FROM orders WHERE customer = 1")]);
    panel(&host, "SEARCH orders USING INDEX orders_by_customer");

    // An error says which statement.
    submit(&host, "sql", "Run", &[("sql", "SELECT 1; SELECT nope FROM customer;")]);
    let err = panel(&host, "no such column");
    assert!(err.contains("nope"), "{err}");

    // The diagram: tables with their columns, a line for each foreign key.
    host.run_command(ID, "diagram", "");
    match wait(&host, |u| matches!(u, Update::Graph { .. })) {
        Update::Graph { graph, .. } => {
            assert_eq!(graph.nodes.len(), 2);
            let orders = graph.nodes.iter().find(|n| n.id == "orders").unwrap();
            assert!(orders.members.contains(&"# id: INTEGER".to_owned()), "{:?}", orders.members);
            assert_eq!((graph.edges[0].source.as_str(), graph.edges[0].target.as_str()), ("orders", "customer"));
        }
        _ => unreachable!(),
    }

    // A server: the password goes in the environment only, and isn't kept.
    host.run_command(ID, "connect", "");
    panel(&host, "form connect (kind,name,host,port,user,password,database)");
    submit(
        &host,
        "connect",
        "Connect",
        &[
            ("kind", "PostgreSQL"),
            ("name", "shop"),
            ("host", "db.local"),
            ("port", ""),
            ("user", "ann"),
            ("password", "s3cret"),
            ("database", "shop"),
        ],
    );
    // The stand-in answers every script with the same rows: not a catalog.
    let connected = panel(&host, "the server's catalog didn't answer as expected");
    assert!(
        connected.starts_with(
            "shop
"
        ),
        "{connected}"
    );
    submit(&host, "sql", "Run", &[("sql", "SELECT id, name FROM customer")]);
    let pg = panel(&host, "table id,name");
    assert!(pg.contains("  1,ann\n  2,NULL\n"), "{pg}");
    let calls = std::fs::read_to_string(bin.join("calls.log")).unwrap();
    assert!(calls.contains("-h db.local -p 5432 -U ann -d shop -w PGPASSWORD=s3cret"), "{calls}");
    assert!(!calls.contains("s3cret -") && calls.matches("s3cret").count() == calls.lines().count(), "{calls}");
    let kept = std::fs::read_to_string(tmp.join("appdata/coiman-ide/plugins/data/coiman.sql.txt")).unwrap();
    assert!(kept.contains("host = \"db.local\"") && !kept.contains("s3cret"), "{kept}");

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
