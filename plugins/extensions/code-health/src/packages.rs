//! What a project depends on: each dependency's licence (Cargo's and npm's), and the known
//! vulnerabilities `cargo audit` and `npm audit` report. Licences are read from Cargo's
//! metadata (`cargo metadata --offline`) and from `node_modules`. The audits run their own
//! tools, which fetch the advisory lists themselves.

/// A dependency and its licence.
#[derive(Clone, Debug, PartialEq)]
pub struct Licensed {
    /// "cargo" or "npm".
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
    /// The SPDX expression as declared; empty when none is.
    pub license: String,
}

/// How a licence asks to be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// MIT, Apache, BSD…: use it, keep the notice.
    Permissive,
    /// MPL, LGPL, EPL: changes to the dependency itself stay open.
    WeakCopyleft,
    /// GPL, AGPL: the whole program shared under it may have to be.
    Copyleft,
    /// None declared, or one not known here.
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Permissive => "Permissive",
            Self::WeakCopyleft => "Weak copyleft",
            Self::Copyleft => "Copyleft",
            Self::Unknown => "Unknown",
        }
    }
}

fn kind_of_one(id: &str) -> Kind {
    let id = id.trim().trim_matches(|c| c == '(' || c == ')').to_ascii_uppercase();
    let permissive = [
        "MIT",
        "MIT-0",
        "APACHE-2.0",
        "BSD-2-CLAUSE",
        "BSD-3-CLAUSE",
        "BSD-1-CLAUSE",
        "0BSD",
        "ISC",
        "ZLIB",
        "UNLICENSE",
        "CC0-1.0",
        "BSL-1.0",
        "UNICODE-3.0",
        "UNICODE-DFS-2016",
        "PYTHON-2.0",
        "BLUEOAK-1.0.0",
        "WTFPL",
        "CC-BY-4.0",
        "X11",
        "NCSA",
        "APACHE-2.0 WITH LLVM-EXCEPTION",
    ];
    let weak = ["MPL-2.0", "LGPL-2.1", "LGPL-3.0", "EPL-2.0", "EPL-1.0", "CDDL-1.0", "LGPL-2.1-ONLY", "LGPL-3.0-ONLY"];
    if permissive.contains(&id.as_str()) {
        Kind::Permissive
    } else if weak.iter().any(|w| id.starts_with(w)) || id.starts_with("LGPL") {
        Kind::WeakCopyleft
    } else if id.starts_with("GPL") || id.starts_with("AGPL") || id.starts_with("SSPL") || id.starts_with("EUPL") {
        Kind::Copyleft
    } else {
        Kind::Unknown
    }
}

/// An SPDX expression's kind: with OR the most permissive choice counts, with AND the least
/// (`MIT OR Apache-2.0` is permissive; `MIT AND GPL-3.0` is copyleft). `/` is the old OR.
pub fn kind(expression: &str) -> Kind {
    let text = expression.trim();
    if text.is_empty() {
        return Kind::Unknown;
    }
    let alternatives = text.split(" OR ").flat_map(|a| a.split('/'));
    alternatives
        .map(|alt| alt.split(" AND ").map(kind_of_one).max().unwrap_or(Kind::Unknown))
        .min()
        .unwrap_or(Kind::Unknown)
}

/// Cargo's dependencies (the project's own crates left out), from `cargo metadata` output.
pub fn parse_cargo_metadata(json: &str) -> Result<Vec<Licensed>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let members: Vec<&str> =
        value["workspace_members"].as_array().into_iter().flatten().filter_map(|m| m.as_str()).collect();
    let mut out: Vec<Licensed> = value["packages"]
        .as_array()
        .ok_or("no packages in cargo metadata")?
        .iter()
        .filter(|p| !p["id"].as_str().is_some_and(|id| members.contains(&id)))
        .map(|p| Licensed {
            ecosystem: "cargo",
            name: p["name"].as_str().unwrap_or("").to_owned(),
            version: p["version"].as_str().unwrap_or("").to_owned(),
            license: p["license"].as_str().unwrap_or("").to_owned(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    Ok(out)
}

/// A `package.json`'s licence: `"license": "MIT"`, `{ "type": "MIT" }`, or the old `licenses` list.
fn npm_license(value: &serde_json::Value) -> String {
    match &value["license"] {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => o.get("type").and_then(|t| t.as_str()).unwrap_or("").to_owned(),
        _ => value["licenses"]
            .as_array()
            .map(|list| list.iter().filter_map(|l| l["type"].as_str().or(l.as_str())).collect::<Vec<_>>().join(" OR "))
            .unwrap_or_default(),
    }
}

/// One `node_modules/<package>/package.json`, as a licence entry.
pub fn npm_package(text: &str) -> Option<Licensed> {
    let v = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let name = v["name"].as_str().unwrap_or("").to_owned();
    (!name.is_empty()).then(|| Licensed {
        ecosystem: "npm",
        name,
        version: v["version"].as_str().unwrap_or("").to_owned(),
        license: npm_license(&v),
    })
}

/// A known vulnerability in a dependency.
#[derive(Clone, Debug, PartialEq)]
pub struct Advisory {
    /// "cargo" or "npm".
    pub ecosystem: &'static str,
    pub package: String,
    pub version: String,
    /// RUSTSEC-…, GHSA-…, or the npm advisory's number.
    pub id: String,
    pub title: String,
    /// "critical", "high", "moderate", "low", or "" when not given.
    pub severity: String,
    /// What fixes it: versions, or "no fix yet".
    pub fix: String,
    pub url: String,
}

/// `cargo audit --json` output.
pub fn parse_cargo_audit(json: &str) -> Result<Vec<Advisory>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let list = value["vulnerabilities"]["list"].as_array().ok_or("no vulnerability list in cargo audit's output")?;
    Ok(list
        .iter()
        .map(|v| {
            let patched: Vec<&str> =
                v["versions"]["patched"].as_array().into_iter().flatten().filter_map(|p| p.as_str()).collect();
            Advisory {
                ecosystem: "cargo",
                package: v["package"]["name"].as_str().unwrap_or("").to_owned(),
                version: v["package"]["version"].as_str().unwrap_or("").to_owned(),
                id: v["advisory"]["id"].as_str().unwrap_or("").to_owned(),
                title: v["advisory"]["title"].as_str().unwrap_or("").to_owned(),
                severity: v["advisory"]["cvss"]
                    .as_str()
                    .map(|_| "see advisory".to_owned())
                    .or_else(|| v["advisory"]["severity"].as_str().map(str::to_owned))
                    .unwrap_or_default(),
                fix: if patched.is_empty() {
                    "no fix yet".to_owned()
                } else {
                    format!("upgrade to {}", patched.join(" or "))
                },
                url: v["advisory"]["url"].as_str().map(str::to_owned).unwrap_or_else(|| {
                    format!("https://rustsec.org/advisories/{}", v["advisory"]["id"].as_str().unwrap_or(""))
                }),
            }
        })
        .collect())
}

/// `npm audit --json` output (npm 7 and later).
pub fn parse_npm_audit(json: &str) -> Result<Vec<Advisory>, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if let Some(error) = value["error"]["summary"].as_str() {
        return Err(error.to_owned());
    }
    let vulns = value["vulnerabilities"].as_object().ok_or("no vulnerabilities in npm audit's output")?;
    let mut out = Vec::new();
    for (name, v) in vulns {
        // `via` holds the advisories, or names of other packages that bring one in.
        let advisories: Vec<&serde_json::Value> =
            v["via"].as_array().into_iter().flatten().filter(|x| x.is_object()).collect();
        let fix = match &v["fixAvailable"] {
            serde_json::Value::Bool(true) => "npm audit fix".to_owned(),
            serde_json::Value::Object(o) => format!(
                "upgrade {} to {}",
                o.get("name").and_then(|n| n.as_str()).unwrap_or(name),
                o.get("version").and_then(|n| n.as_str()).unwrap_or("?")
            ),
            _ => "no fix yet".to_owned(),
        };
        let severity = v["severity"].as_str().unwrap_or("").to_owned();
        if advisories.is_empty() {
            let through: Vec<&str> = v["via"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
            out.push(Advisory {
                ecosystem: "npm",
                package: name.clone(),
                version: v["range"].as_str().unwrap_or("").to_owned(),
                id: String::new(),
                title: format!("through {}", through.join(", ")),
                severity,
                fix,
                url: String::new(),
            });
            continue;
        }
        for a in advisories {
            let url = a["url"].as_str().unwrap_or("").to_owned();
            out.push(Advisory {
                ecosystem: "npm",
                package: name.clone(),
                version: a["range"].as_str().or(v["range"].as_str()).unwrap_or("").to_owned(),
                id: url.rsplit('/').next().unwrap_or("").to_owned(),
                title: a["title"].as_str().unwrap_or("").to_owned(),
                severity: a["severity"].as_str().unwrap_or(&severity).to_owned(),
                fix: fix.clone(),
                url,
            });
        }
    }
    Ok(out)
}

/// Orders advisories most severe first.
pub fn severity_rank(severity: &str) -> u8 {
    match severity.to_ascii_lowercase().as_str() {
        "critical" => 0,
        "high" => 1,
        "moderate" | "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licence_expressions_read_as_their_strictest_or_freest() {
        assert_eq!(kind("MIT OR Apache-2.0"), Kind::Permissive);
        assert_eq!(kind("MIT/Apache-2.0"), Kind::Permissive);
        assert_eq!(kind("Apache-2.0 WITH LLVM-exception"), Kind::Permissive);
        assert_eq!(kind("MPL-2.0"), Kind::WeakCopyleft);
        assert_eq!(kind("GPL-3.0-or-later"), Kind::Copyleft);
        assert_eq!(kind("MIT AND GPL-2.0"), Kind::Copyleft);
        assert_eq!(kind("GPL-2.0 OR MIT"), Kind::Permissive);
        assert_eq!(kind(""), Kind::Unknown);
        assert_eq!(kind("SEE LICENSE IN LICENSE.txt"), Kind::Unknown);
    }

    #[test]
    fn cargo_metadata_leaves_the_project_out() {
        let json = r#"{"workspace_members":["path+file:///p#app@0.1.0"],"packages":[
            {"id":"path+file:///p#app@0.1.0","name":"app","version":"0.1.0","license":"MIT"},
            {"id":"registry+x#serde@1.0.0","name":"serde","version":"1.0.0","license":"MIT OR Apache-2.0"},
            {"id":"registry+x#odd@0.2.0","name":"odd","version":"0.2.0","license":null}]}"#;
        let list = parse_cargo_metadata(json).unwrap();
        let names: Vec<(&str, &str)> = list.iter().map(|l| (l.name.as_str(), l.license.as_str())).collect();
        assert_eq!(names, [("odd", ""), ("serde", "MIT OR Apache-2.0")]);
    }

    #[test]
    fn npm_packages_are_read() {
        let mut list: Vec<Licensed> = [
            r#"{"name":"left-pad","version":"1.3.0","license":"WTFPL"}"#,
            r#"{"name":"@scope/old","version":"0.1.0","licenses":[{"type":"MIT"}]}"#,
            r#"{"name":"obj","version":"2.0.0","license":{"type":"ISC"}}"#,
            r#"{"version":"1.0.0"}"#,
        ]
        .iter()
        .filter_map(|t| npm_package(t))
        .collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<(&str, &str)> = list.iter().map(|l| (l.name.as_str(), l.license.as_str())).collect();
        assert_eq!(got, [("@scope/old", "MIT"), ("left-pad", "WTFPL"), ("obj", "ISC")]);
    }

    #[test]
    fn audit_outputs_are_read() {
        let cargo = r#"{"vulnerabilities":{"found":true,"count":1,"list":[{"advisory":{"id":"RUSTSEC-2023-0001",
            "title":"Data race","url":null},"package":{"name":"tokio","version":"1.0.0"},
            "versions":{"patched":[">=1.18.4"]}}]}}"#;
        let list = parse_cargo_audit(cargo).unwrap();
        assert_eq!(list[0].package, "tokio");
        assert_eq!(list[0].fix, "upgrade to >=1.18.4");
        assert_eq!(list[0].url, "https://rustsec.org/advisories/RUSTSEC-2023-0001");
        let npm = r#"{"vulnerabilities":{
            "minimist":{"name":"minimist","severity":"critical","range":"<1.2.6","fixAvailable":true,
              "via":[{"title":"Prototype Pollution","url":"https://github.com/advisories/GHSA-xvch-5gv4-984h",
                      "severity":"critical","range":"<1.2.6"}]},
            "mkdirp":{"name":"mkdirp","severity":"critical","range":"0.4.1 - 0.5.1","via":["minimist"],
              "fixAvailable":{"name":"mkdirp","version":"0.5.6"}}}}"#;
        let list = parse_npm_audit(npm).unwrap();
        assert_eq!(list.len(), 2);
        let minimist = list.iter().find(|a| a.package == "minimist").unwrap();
        assert_eq!((minimist.id.as_str(), minimist.fix.as_str()), ("GHSA-xvch-5gv4-984h", "npm audit fix"));
        let mkdirp = list.iter().find(|a| a.package == "mkdirp").unwrap();
        assert_eq!((mkdirp.title.as_str(), mkdirp.fix.as_str()), ("through minimist", "upgrade mkdirp to 0.5.6"));
        assert!(severity_rank("critical") < severity_rank("low"));
    }
}
