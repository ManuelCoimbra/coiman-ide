//! The ssh hosts the user has named in `~/.ssh/config`, to connect to by name. Coiman only
//! reads the file; `ssh` itself connects.

use std::path::{Path, PathBuf};

/// The user's ssh config file.
pub fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".ssh").join("config"))
}

/// The hosts a config names (`Host a b`), patterns (`*`, `?`, `!`) left out, each once, in order.
pub fn parse(text: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, rest)) = line.split_once(|c: char| c.is_whitespace() || c == '=') else { continue };
        if !key.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in rest.trim_start_matches(|c: char| c.is_whitespace() || c == '=').split_whitespace() {
            let name = name.trim_matches('"');
            if !name.is_empty() && !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_owned());
            }
        }
    }
    hosts
}

/// The hosts in `path`, or none when it can't be read.
pub fn hosts_in(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_hosts_only() {
        let text = "# boards\nHost pi board\n  HostName 10.0.0.5\nHost *.lan\nHost=build\n\nhost \"nas\" pi\nMatch host x\nHost !bad ok?\n";
        assert_eq!(parse(text), ["pi", "board", "build", "nas"]);
        assert!(parse("").is_empty());
        assert!(hosts_in(Path::new("/no/such/config")).is_empty());
    }
}
