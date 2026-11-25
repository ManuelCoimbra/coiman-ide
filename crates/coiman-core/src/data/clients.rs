//! Where client programs are when they aren't on `PATH`: database clients under Program Files
//! (PostgreSQL, MariaDB, MySQL, SQL Server's tools), `adb` in the Android SDK, or any of them
//! unpacked into the config folder's `tools`. Extensions name a client by its program name;
//! the plugin host finds it here after `PATH`.

use std::path::PathBuf;

/// A client program by name, off `PATH`: `psql`, `mariadb`, `mysql`, `sqlcmd`, `SqlLocalDB`,
/// `redis-cli`, `mongosh`, `adb`.
pub fn find(name: &str) -> Option<PathBuf> {
    if name == "adb" {
        let exe = if cfg!(windows) { "adb.exe" } else { "adb" };
        return android_sdk_places().into_iter().map(|d| d.join(exe)).find(|p| p.is_file());
    }
    let exe = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_owned() };
    let tools = || crate::settings::config_dir().map(|d| d.join("tools"));
    let file = |p: PathBuf| p.is_file().then_some(p);
    match name {
        "sqlcmd" => {
            tools().and_then(|t| file(t.join("sqlcmd").join(exe("sqlcmd")))).or_else(|| sql_server_tool("SQLCMD.EXE"))
        }
        "SqlLocalDB" => sql_server_tool("SqlLocalDB.exe"),
        "redis-cli" => tools().and_then(|t| file(t.join("redis").join(exe("redis-cli")))),
        "mongosh" => tools().and_then(|t| file(t.join("mongosh").join("bin").join(exe("mongosh")))),
        "psql" => program_files(&["PostgreSQL"], name),
        "mariadb" | "mysql" => program_files(&["MariaDB", "MySQL"], name),
        _ => None,
    }
}

/// The newest `<prefix>…\bin\<name>.exe` under Program Files: `PostgreSQL\17\bin`,
/// `MariaDB 11.8\bin`, `MySQL\MySQL Server 8.4\bin`.
fn program_files(prefixes: &[&str], name: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let exe = format!("{name}.exe");
    let roots = ["ProgramFiles", "ProgramW6432"].iter().filter_map(std::env::var_os).map(PathBuf::from);
    let mut found = Vec::new();
    for root in roots {
        for prefix in prefixes {
            let Ok(entries) = std::fs::read_dir(root.join(prefix)).or_else(|_| std::fs::read_dir(&root)) else {
                continue;
            };
            for e in entries.flatten() {
                let dir = e.path();
                let entry = e.file_name().to_string_lossy().to_string();
                if !(dir.starts_with(root.join(prefix)) || entry.starts_with(prefix)) {
                    continue;
                }
                let bin = dir.join("bin").join(&exe);
                if bin.is_file() {
                    found.push(bin);
                }
            }
        }
    }
    // The newest version: names sort by it.
    found.sort();
    found.pop()
}

/// A program among SQL Server's own tools (`SQLCMD.EXE`, `SqlLocalDB.exe`): the newest found
/// under Program Files' `Microsoft SQL Server`.
fn sql_server_tool(name: &str) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let mut found = Vec::new();
    for root in ["ProgramFiles", "ProgramW6432"].iter().filter_map(std::env::var_os).map(PathBuf::from) {
        let base = root.join("Microsoft SQL Server");
        let Ok(versions) = std::fs::read_dir(&base) else { continue };
        for v in versions.flatten() {
            for bin in [v.path().join("Tools").join("Binn"), v.path().join("Tools").join("Binn").join("x64")] {
                let exe = bin.join(name);
                if exe.is_file() {
                    found.push(exe);
                }
            }
        }
        // The ODBC tools: Client SDK\ODBC\170\Tools\Binn.
        if let Ok(odbc) = std::fs::read_dir(base.join("Client SDK").join("ODBC")) {
            for v in odbc.flatten() {
                let exe = v.path().join("Tools").join("Binn").join(name);
                if exe.is_file() {
                    found.push(exe);
                }
            }
        }
    }
    found.sort();
    found.pop()
}

/// Where `adb` may be: the Android SDK's `platform-tools` (from `ANDROID_HOME`,
/// `ANDROID_SDK_ROOT`, or where Android Studio puts it), or the config folder's
/// `tools/platform-tools`.
fn android_sdk_places() -> Vec<PathBuf> {
    let mut places = Vec::new();
    for var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(sdk) = std::env::var_os(var) {
            places.push(PathBuf::from(sdk).join("platform-tools"));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        places.push(PathBuf::from(local).join("Android").join("Sdk").join("platform-tools"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        places.push(PathBuf::from(&home).join("Android").join("Sdk").join("platform-tools"));
        places.push(PathBuf::from(home).join("Library").join("Android").join("sdk").join("platform-tools"));
    }
    if let Some(config) = crate::settings::config_dir() {
        places.push(config.join("tools").join("platform-tools"));
    }
    places
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_names_are_not_looked_for() {
        assert!(super::find("rm").is_none());
        assert!(super::find("").is_none());
    }
}
