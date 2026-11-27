//! Backups of the open database, and restoring one, through the database's own tools: SQLite's
//! `.backup` and `.restore`, `pg_dump` and `psql`, `mysqldump` and `mysql`, and SQL Server's
//! `BACKUP DATABASE` and `RESTORE DATABASE` (written by the server itself, so the path must be
//! one the server can reach). Backups go in the extension's own folder.

use crate::Call;
use crate::server::{Kind, Server};

/// `20261127-153000`, local time, from milliseconds since 1970 and the offset from UTC.
pub fn stamp(unix_millis: i64, offset_minutes: i32) -> String {
    let t = unix_millis / 1000 + i64::from(offset_minutes) * 60;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

/// A file name of letters, digits, `-` and `_` only.
pub fn safe_name(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_owned();
    if s.is_empty() { "database".into() } else { s }
}

/// The backup file's extension.
pub fn extension(kind: Option<Kind>) -> &'static str {
    match kind {
        None => "db",
        Some(Kind::SqlServer) => "bak",
        Some(_) => "sql",
    }
}

fn sqlite_path(path: &str) -> String {
    // A dot-command's argument in single quotes, as the shell reads it.
    format!("'{}'", path.replace('\'', "''"))
}

pub fn sqlite_backup(db: &str, to: &str) -> Call {
    Call::new("sqlite3", vec!["-bail".into(), db.to_owned()], format!(".backup {}\n", sqlite_path(to)), Vec::new())
}

pub fn sqlite_restore(db: &str, from: &str) -> Call {
    Call::new("sqlite3", vec!["-bail".into(), db.to_owned()], format!(".restore {}\n", sqlite_path(from)), Vec::new())
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

/// The programs that make a server's backup, in the order tried.
pub fn dump_programs(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Postgres => &["pg_dump"],
        Kind::MySql => &["mariadb-dump", "mysqldump"],
        Kind::SqlServer => &["sqlcmd"],
    }
}

/// The calls that back up a server's database to `to`, one per program to try.
pub fn server_backup(server: &Server, to: &str, script_file: &str) -> Vec<Call> {
    let port = server.port.to_string();
    match server.kind {
        Kind::Postgres => {
            let mut args = s(&["-h", &server.host, "-p", &port, "-U", &server.user, "-w"]);
            // Drops what it makes first, so the backup restores over the database as it is then.
            args.extend(s(&["--clean", "--if-exists", "--no-owner", "-F", "p", "-f", to]));
            args.push(server.database.clone());
            let env = server.password.iter().map(|p| ("PGPASSWORD".to_owned(), p.clone())).collect();
            vec![Call::new("pg_dump", args, String::new(), env)]
        }
        Kind::MySql => {
            let mut args = s(&["-h", &server.host, "-P", &port, "-u", &server.user]);
            args.extend(s(&["--single-transaction", "--routines", "--triggers", &format!("--result-file={to}")]));
            args.push(server.database.clone());
            let env: Vec<(String, String)> =
                server.password.iter().map(|p| ("MYSQL_PWD".to_owned(), p.clone())).collect();
            dump_programs(Kind::MySql).iter().map(|p| Call::new(p, args.clone(), String::new(), env.clone())).collect()
        }
        Kind::SqlServer => {
            let db = Kind::SqlServer.quote_name(&server.database);
            let script =
                format!("BACKUP DATABASE {db} TO DISK = N'{}' WITH INIT, COPY_ONLY;\nGO\n", to.replace('\'', "''"));
            vec![server.call("sqlcmd", &script, script_file)]
        }
    }
}

/// The calls that put a backup back over the server's database.
pub fn server_restore(server: &Server, from: &str, script_file: &str) -> Vec<Call> {
    let port = server.port.to_string();
    match server.kind {
        Kind::Postgres => {
            let mut args =
                s(&["-X", "-q", "-v", "ON_ERROR_STOP=1", "-h", &server.host, "-p", &port, "-U", &server.user]);
            args.extend(s(&["-d", &server.database, "-w", "-f", from]));
            let env = server.password.iter().map(|p| ("PGPASSWORD".to_owned(), p.clone())).collect();
            vec![Call::new("psql", args, String::new(), env)]
        }
        Kind::MySql => {
            let mut args = s(&["-h", &server.host, "-P", &port, "-u", &server.user]);
            args.push(server.database.clone());
            args.push(format!("--execute=source {from}"));
            let env: Vec<(String, String)> =
                server.password.iter().map(|p| ("MYSQL_PWD".to_owned(), p.clone())).collect();
            ["mariadb", "mysql"].iter().map(|p| Call::new(p, args.clone(), String::new(), env.clone())).collect()
        }
        Kind::SqlServer => {
            // From master: the database being replaced can have no one in it.
            let mut master = server.clone();
            master.database = "master".into();
            let db = Kind::SqlServer.quote_name(&server.database);
            let script = format!(
                "ALTER DATABASE {db} SET SINGLE_USER WITH ROLLBACK IMMEDIATE;\nGO\n\
                 RESTORE DATABASE {db} FROM DISK = N'{}' WITH REPLACE;\nGO\n\
                 ALTER DATABASE {db} SET MULTI_USER;\nGO\n",
                from.replace('\'', "''")
            );
            vec![master.call("sqlcmd", &script, script_file)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(kind: Kind) -> Server {
        Server {
            kind,
            host: "localhost".into(),
            port: kind.default_port(),
            user: "me".into(),
            database: "shop".into(),
            password: Some("pw".into()),
            ..Server::default()
        }
    }

    #[test]
    fn names_and_times() {
        assert_eq!(stamp(1_764_255_000_000, 0), "20251127-145000");
        assert_eq!(stamp(1_764_255_000_000, 60), "20251127-155000");
        assert_eq!(safe_name("my shop.db"), "my_shop_db");
        assert_eq!(safe_name("..."), "database");
        assert_eq!(
            (extension(None), extension(Some(Kind::Postgres)), extension(Some(Kind::SqlServer))),
            ("db", "sql", "bak")
        );
    }

    #[test]
    fn sqlite_calls() {
        let b = sqlite_backup("C:/p/shop.db", "C:/d/it's.db");
        assert_eq!(b.args, ["-bail", "C:/p/shop.db"]);
        assert_eq!(b.stdin, ".backup 'C:/d/it''s.db'\n");
        assert_eq!(sqlite_restore("x.db", "y.db").stdin, ".restore 'y.db'\n");
    }

    #[test]
    fn server_calls() {
        let pg = &server_backup(&server(Kind::Postgres), "/b/shop.sql", "s/1.sql")[0];
        assert_eq!(pg.program, "pg_dump");
        assert!(pg.args.ends_with(&["-f".into(), "/b/shop.sql".into(), "shop".into()]));
        assert!(pg.args.contains(&"--clean".into()) && !pg.args.iter().any(|a| a.contains("pw")));
        assert_eq!(pg.env, [("PGPASSWORD".to_owned(), "pw".to_owned())]);
        let my = server_backup(&server(Kind::MySql), "/b/shop.sql", "s/1.sql");
        assert_eq!(my.iter().map(|c| c.program.as_str()).collect::<Vec<_>>(), ["mariadb-dump", "mysqldump"]);
        assert!(my[0].args.contains(&"--result-file=/b/shop.sql".into()));
        let ms = &server_backup(&server(Kind::SqlServer), "C:/b/it's.bak", "s/1.sql")[0];
        assert!(ms.file.as_ref().unwrap().1.contains("BACKUP DATABASE [shop] TO DISK = N'C:/b/it''s.bak'"));
        let back = &server_restore(&server(Kind::SqlServer), "C:/b/x.bak", "s/2.sql")[0];
        let script = &back.file.as_ref().unwrap().1;
        assert!(script.contains("RESTORE DATABASE [shop]") && script.contains("WITH REPLACE"));
        assert!(back.args.windows(2).any(|w| w == ["-d", "master"]), "{:?}", back.args);
        let pgr = &server_restore(&server(Kind::Postgres), "/b/shop.sql", "s")[0];
        assert!(pgr.args.ends_with(&["-f".into(), "/b/shop.sql".into()]));
        assert!(
            server_restore(&server(Kind::MySql), "/b/x.sql", "s")[0].args.contains(&"--execute=source /b/x.sql".into())
        );
    }
}
