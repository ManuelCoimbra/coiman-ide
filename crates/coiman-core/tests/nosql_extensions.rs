//! The Redis and MongoDB extensions (`plugins/extensions/redis`, `plugins/extensions/mongodb`,
//! rebuilt with `scripts/build-plugins.ps1`) in the real plugin host, against stand-in
//! `redis-cli` and `mongosh` programs that answer as the real ones do. A Redis password goes in
//! the client's environment, never its arguments; no password is ever saved.

#![cfg(windows)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use coiman_core::extensions::plugin::{Block, Event, FormValues, PluginHost, Update, registry};

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
            Block::Heading(t) | Block::Text(t) | Block::Code(t) => out.push_str(&format!("{t}\n")),
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

fn submit(host: &PluginHost, id: &str, form: &str, button: &str, values: &[(&str, &str)]) {
    let values = values.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    host.deliver(id, Event::FormSubmitted(FormValues { form: form.into(), button: button.into(), values }));
}

/// A stand-in program: it logs its arguments (and `env`'s value) to `<name>.log`, then answers
/// with the first of `answers` whose word is in its arguments.
fn stand_in(dir: &Path, name: &str, env: &str, answers: &[(&str, &[&str])]) {
    let mut cmd = format!("@echo off\r\n>>\"%~dp0{name}.log\" echo %* {env}=%{env}%\r\n");
    for (i, (word, _)) in answers.iter().enumerate() {
        cmd.push_str(&format!("echo %* | findstr /c:\"{word}\" >nul && goto a{i}\r\n"));
    }
    cmd.push_str("echo (error) ERR unknown command\r\nexit /b 0\r\n");
    for (i, (_, lines)) in answers.iter().enumerate() {
        cmd.push_str(&format!(":a{i}\r\n"));
        for l in *lines {
            cmd.push_str(&format!("echo {l}\r\n"));
        }
        cmd.push_str("exit /b 0\r\n");
    }
    std::fs::write(dir.join(format!("{name}.cmd")), cmd).unwrap();
}

fn load(host: &PluginHost, folder: &str, programs: &[&str]) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins/extensions").join(folder);
    let pkg = registry::read_folder(&dir).expect("the extension is built");
    let programs = programs.iter().map(|p| (*p).to_owned()).collect();
    host.load_with_programs(pkg.manifest, pkg.wasm, BTreeSet::new(), programs);
    wait(host, |u| matches!(u, Update::Activated { .. }));
}

#[test]
fn redis_and_mongodb_extensions() {
    let tmp = std::env::temp_dir().join(format!("coiman-nosql-ext-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let bin = tmp.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    stand_in(
        &bin,
        "redis-cli",
        "REDISCLI_AUTH",
        &[
            (
                "EVAL",
                &[
                    "1) \"0\"",
                    "2) 1) \"user:1\"",
                    "   2) \"hash\"",
                    "   3) (integer) -1",
                    "   4) (integer) 2",
                    "3) 1) \"greeting\"",
                    "   2) \"string\"",
                    "   3) (integer) 60000",
                    "   4) (integer) 5",
                ],
            ),
            ("INFO", &["# Keyspace", "db0:keys=2,expires=1,avg_ttl=0"]),
            ("HSCAN", &["1) \"0\"", "2) 1) \"name\"", "   2) \"ann\"", "   3) \"age\"", "   4) \"30\""]),
            ("GET", &["\"hello\""]),
            ("SET", &["OK"]),
            ("EXPIRE", &["(integer) 1"]),
            ("DEL", &["(integer) 1"]),
            ("PING", &["PONG"]),
        ],
    );
    stand_in(
        &bin,
        "mongosh",
        "NOTHING",
        &[
            ("listDatabases", &["[[\"admin\", 40960], [\"shop\", 8192]]"]),
            ("getCollectionInfos", &["[[\"items\", 3, false]]"]),
            (".find(", &["[[{\"_id\": {\"$oid\": \"abc\"}, \"name\": \"pen\", \"price\": 1.5}], 1]"]),
            ("replaceOne", &["1"]),
            ("deleteOne", &["1"]),
        ],
    );
    // SAFETY: this test binary has this one test, so nothing reads the environment meanwhile.
    unsafe {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut dirs = vec![bin.clone()];
        dirs.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
        std::env::set_var("APPDATA", tmp.join("appdata"));
    }
    let host = PluginHost::start(|| {}).unwrap();

    // ----- Redis -----
    const REDIS: &str = "coiman.redis";
    load(&host, "redis", &["redis-cli", "valkey-cli", "memurai-cli"]);
    host.run_command(REDIS, "show", "");
    panel(&host, "form connect (host,port,db,user,name,password,tls)");
    submit(
        &host,
        REDIS,
        "connect",
        "Connect",
        &[
            ("host", "cache.local"),
            ("port", "6380"),
            ("db", "0"),
            ("user", ""),
            ("name", "Cache"),
            ("password", "s3cret"),
            ("tls", "false"),
        ],
    );
    let keys = panel(&host, "2 keys");
    assert!(keys.contains("hash | user:1 [Open select 1]"), "{keys}");
    assert!(keys.contains("string | greeting | ⏱ 60 s [Open select 0]"), "{keys}");
    assert!(keys.contains("form db (db,use)"), "{keys}");
    // A hash's fields as a table.
    host.run_command(REDIS, "select", "1");
    let hash = panel(&host, "table Field,Value");
    assert!(hash.contains("  name,ann\n  age,30\n"), "{hash}");
    // A string, saved with its time to live kept.
    host.run_command(REDIS, "select", "0");
    panel(&host, "form value (value)");
    submit(&host, REDIS, "value", "Save", &[("value", "hi there")]);
    panel(&host, "Saved greeting");
    // The time to live, then deleting after asking.
    submit(&host, REDIS, "ttl", "Set", &[("ttl", "90")]);
    panel(&host, "greeting expires in 90 s");
    host.run_command(REDIS, "delete-ask", "");
    panel(&host, "Delete this key?");
    host.run_command(REDIS, "delete", "");
    panel(&host, "Deleted greeting");
    // The console.
    submit(&host, REDIS, "console", "Run", &[("line", "PING")]);
    panel(&host, "> PING\nPONG");
    let calls = std::fs::read_to_string(bin.join("redis-cli.log")).unwrap();
    assert!(calls.contains("--no-raw -h cache.local -p 6380 -n 0 SET greeting \"hi there\" PX 60000"), "{calls}");
    assert!(calls.contains("EXPIRE greeting 90") && calls.contains("DEL greeting"), "{calls}");
    // The password only in the environment, on every call.
    assert_eq!(calls.matches("s3cret").count(), calls.lines().count(), "{calls}");
    assert!(calls.lines().all(|l| l.ends_with("REDISCLI_AUTH=s3cret")), "{calls}");
    let kept = std::fs::read_to_string(tmp.join("appdata/coiman-ide/plugins/data/coiman.redis.txt")).unwrap();
    assert!(kept.contains("host = \"cache.local\"") && !kept.contains("s3cret"), "{kept}");

    // ----- MongoDB -----
    const MONGO: &str = "coiman.mongodb";
    load(&host, "mongodb", &["mongosh"]);
    host.run_command(MONGO, "show", "");
    panel(&host, "form connect (uri,user,auth_db,name,password)");
    submit(
        &host,
        MONGO,
        "connect",
        "Connect",
        &[
            ("uri", "mongodb://db.local:27017"),
            ("user", "ann"),
            ("auth_db", "admin"),
            ("name", "Shop"),
            ("password", "pw"),
        ],
    );
    let dbs = panel(&host, "▸ shop");
    assert!(dbs.contains("8.0 KB [Show expand shop]"), "{dbs}");
    host.run_command(MONGO, "expand", "shop");
    let colls = panel(&host, "    items | 3");
    host.run_command(MONGO, "choose", "shop\u{1f}items");
    let docs = panel(&host, "table _id,name,price");
    assert!(docs.contains("  ObjectId('abc'),pen,1.5\n"), "{docs}");
    assert!(docs.contains("1–1 of 1"), "{docs} {colls}");
    // A document opened, saved, then deleted.
    host.run_command(MONGO, "open", "0");
    panel(&host, "form edit (doc) [\"Save\", \"Close\"]");
    submit(&host, MONGO, "edit", "Save", &[("doc", "{\"_id\": {\"$oid\": \"abc\"}, \"name\": \"pen\", \"price\": 2}")]);
    panel(&host, "Saved (1 changed)");
    host.run_command(MONGO, "open", "0");
    submit(&host, MONGO, "edit", "Save", &[("doc", "not json")]);
    panel(&host, "Not JSON");
    host.run_command(MONGO, "delete-ask", "");
    host.run_command(MONGO, "delete", "");
    panel(&host, "Deleted 1 document");
    let calls = std::fs::read_to_string(bin.join("mongosh.log")).unwrap();
    assert!(
        calls.contains("mongodb://db.local:27017/?serverSelectionTimeoutMS=5000\" --quiet --norc \"--json=relaxed\" -u ann -p pw --authenticationDatabase admin --eval"),
        "{calls}"
    );
    let kept = std::fs::read_to_string(tmp.join("appdata/coiman-ide/plugins/data/coiman.mongodb.txt")).unwrap();
    assert!(kept.contains("user = \"ann\"") && !kept.contains("pw\""), "{kept}");

    drop(host);
    let _ = std::fs::remove_dir_all(&tmp);
}
