//! The project's own architecture rules, in `.coiman/architecture.toml`, checked against the
//! import graph: layers (each may import only those below it) and rules naming what a part
//! may not import.
//!
//! ```toml
//! # From the top down: a file may import its own layer and those below, never one above.
//! layers = ["src/ui/**", "src/app/**", "src/domain/**", "src/infra/**"]
//!
//! [[rule]]
//! from = "src/ui/**"
//! not = ["src/db/**", "src/infra/**"]
//! why = "the UI goes through the services"
//! ```

use crate::deps::Graph;

pub const FILE: &str = ".coiman/architecture.toml";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rules {
    pub layers: Vec<String>,
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub from: String,
    pub not: Vec<String>,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub from: String,
    pub to: String,
    /// The import's line in `from`.
    pub line: usize,
    pub why: String,
}

/// The rules file's text, read.
pub fn parse(text: &str) -> Result<Rules, String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let strings = |v: Option<&toml::Value>| -> Vec<String> {
        v.and_then(toml::Value::as_array)
            .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    let layers = strings(table.get("layers"));
    let mut rules = Vec::new();
    for (i, r) in table.get("rule").and_then(toml::Value::as_array).into_iter().flatten().enumerate() {
        let from = r.get("from").and_then(toml::Value::as_str).ok_or(format!("rule {} has no `from`", i + 1))?;
        let not = match r.get("not") {
            Some(toml::Value::String(s)) => vec![s.clone()],
            v => strings(v),
        };
        if not.is_empty() {
            return Err(format!("rule {} ({from}) has no `not`", i + 1));
        }
        let why = r.get("why").and_then(toml::Value::as_str).unwrap_or("").to_owned();
        rules.push(Rule { from: from.to_owned(), not, why });
    }
    Ok(Rules { layers, rules })
}

/// A path matches a glob: `**` any folders, `*` any name part, `?` one character; a glob
/// without `*` matches the path itself or anything under it.
pub fn matches(glob: &str, path: &str) -> bool {
    let glob = glob.trim_start_matches("./").trim_end_matches('/');
    if !glob.contains(['*', '?']) {
        return path == glob || path.starts_with(&format!("{glob}/"));
    }
    fn walk(g: &[u8], p: &[u8]) -> bool {
        match g.first() {
            None => p.is_empty(),
            Some(b'*') if g.get(1) == Some(&b'*') => {
                // `**/` may match no folder at all.
                let rest = if g.get(2) == Some(&b'/') { &g[3..] } else { &g[2..] };
                // At the end, it takes whatever is left.
                if rest.is_empty() {
                    return true;
                }
                (0..=p.len()).any(|i| (i == 0 || p[i - 1] == b'/') && walk(rest, &p[i..])) || walk(rest, p)
            }
            Some(b'*') => (0..=p.len()).take_while(|&i| i == 0 || p[i - 1] != b'/').any(|i| walk(&g[1..], &p[i..])),
            Some(b'?') => !p.is_empty() && p[0] != b'/' && walk(&g[1..], &p[1..]),
            Some(c) => p.first() == Some(c) && walk(&g[1..], &p[1..]),
        }
    }
    walk(glob.as_bytes(), path.as_bytes())
}

/// The first layer a path is in, from the top.
fn layer_of(layers: &[String], path: &str) -> Option<usize> {
    layers.iter().position(|l| matches(l, path))
}

/// The imports that break the rules.
pub fn check(graph: &Graph, rules: &Rules) -> Vec<Violation> {
    let mut out = Vec::new();
    for e in &graph.edges {
        let (from, to) = (&graph.nodes[e.from].path, &graph.nodes[e.to].path);
        if let (Some(a), Some(b)) = (layer_of(&rules.layers, from), layer_of(&rules.layers, to))
            && b < a
        {
            out.push(Violation {
                from: from.clone(),
                to: to.clone(),
                line: e.line,
                why: format!("{} may not import {}, a layer above it", rules.layers[a], rules.layers[b]),
            });
            continue;
        }
        for r in &rules.rules {
            if matches(&r.from, from)
                && let Some(n) = r.not.iter().find(|n| matches(n, to))
            {
                let why = if r.why.is_empty() { format!("{} may not import {n}", r.from) } else { r.why.clone() };
                out.push(Violation { from: from.clone(), to: to.clone(), line: e.line, why });
                break;
            }
        }
    }
    out.sort_by(|a, b| (&a.from, a.line).cmp(&(&b.from, b.line)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::{Edge, Lang, Node};

    #[test]
    fn globs() {
        assert!(matches("src/ui/**", "src/ui/a.rs"));
        assert!(matches("src/ui/**", "src/ui/x/y/a.rs"));
        assert!(!matches("src/ui/**", "src/uix/a.rs"));
        assert!(matches("src/*/model.rs", "src/user/model.rs"));
        assert!(!matches("src/*/model.rs", "src/a/b/model.rs"));
        assert!(matches("**/db/**", "crates/core/db/pool.rs"));
        assert!(matches("**/*.test.ts", "web/a.test.ts"));
        assert!(matches("src/db", "src/db/pool.rs"), "a folder without * is everything under it");
        assert!(matches("src/a?.rs", "src/ab.rs"));
    }

    fn graph(edges: &[(&str, &str, usize)]) -> Graph {
        let mut g = Graph::default();
        let idx = |g: &mut Graph, p: &str| match g.nodes.iter().position(|n| n.path == p) {
            Some(i) => i,
            None => {
                g.nodes.push(Node { path: p.into(), lang: Lang::Rust, test: false });
                g.nodes.len() - 1
            }
        };
        for (from, to, line) in edges {
            let (a, b) = (idx(&mut g, from), idx(&mut g, to));
            g.edges.push(Edge { from: a, to: b, line: *line });
        }
        g
    }

    #[test]
    fn layers_and_rules() {
        let rules = parse(
            "layers = [\"src/ui/**\", \"src/app/**\", \"src/infra/**\"]\n\
             [[rule]]\nfrom = \"src/ui/**\"\nnot = [\"src/infra/**\"]\nwhy = \"the UI goes through the app\"\n\
             [[rule]]\nfrom = \"src/app/**\"\nnot = \"src/app/legacy/**\"\n",
        )
        .unwrap();
        let g = graph(&[
            ("src/ui/view.rs", "src/app/service.rs", 3),
            ("src/app/service.rs", "src/infra/db.rs", 4),
            ("src/infra/db.rs", "src/app/service.rs", 9),
            ("src/ui/view.rs", "src/infra/db.rs", 5),
            ("src/app/service.rs", "src/app/legacy/old.rs", 6),
            ("src/main.rs", "src/infra/db.rs", 1),
        ]);
        let got: Vec<(String, usize, String)> =
            check(&g, &rules).into_iter().map(|v| (v.from, v.line, v.why)).collect();
        assert_eq!(
            got,
            [
                ("src/app/service.rs".to_owned(), 6, "src/app/** may not import src/app/legacy/**".to_owned()),
                (
                    "src/infra/db.rs".to_owned(),
                    9,
                    "src/infra/** may not import src/app/**, a layer above it".to_owned()
                ),
                ("src/ui/view.rs".to_owned(), 5, "the UI goes through the app".to_owned()),
            ]
        );
    }

    #[test]
    fn bad_files_say_why() {
        assert!(parse("[[rule]]\nnot = [\"x\"]\n").unwrap_err().contains("no `from`"));
        assert!(parse("[[rule]]\nfrom = \"a\"\n").unwrap_err().contains("no `not`"));
        assert!(parse("layers = [").is_err());
        assert_eq!(parse("").unwrap(), Rules::default());
    }
}
