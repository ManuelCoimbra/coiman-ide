//! How the import graph changed since another revision: the files git says changed are read as
//! they were there, the graph is made again from them, and the two are compared: imports
//! added and removed, cycles that appeared or went, files added and removed.

use std::collections::BTreeSet;

use crate::arch;
use crate::deps::Graph;

/// What `git diff --name-status --no-renames <rev> --` says, by kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub added: Vec<String>,
    pub deleted: Vec<String>,
    pub modified: Vec<String>,
}

pub fn parse_name_status(out: &str) -> Diff {
    let mut d = Diff::default();
    for line in out.lines() {
        let mut parts = line.split('\t');
        let (Some(kind), Some(path)) = (parts.next(), parts.next()) else { continue };
        let path = path.trim().to_owned();
        match kind.chars().next() {
            Some('A') => d.added.push(path),
            Some('D') => d.deleted.push(path),
            // Modified, type changed, merged.
            Some('M' | 'T' | 'U') => d.modified.push(path),
            _ => {}
        }
    }
    d
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub imports_added: Vec<(String, String)>,
    pub imports_removed: Vec<(String, String)>,
    pub cycles_new: Vec<Vec<String>>,
    pub cycles_gone: Vec<Vec<String>>,
    pub files_added: Vec<String>,
    pub files_removed: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.imports_added.is_empty()
            && self.imports_removed.is_empty()
            && self.cycles_new.is_empty()
            && self.cycles_gone.is_empty()
            && self.files_added.is_empty()
            && self.files_removed.is_empty()
    }
}

fn imports(g: &Graph) -> BTreeSet<(String, String)> {
    g.edges.iter().map(|e| (g.nodes[e.from].path.clone(), g.nodes[e.to].path.clone())).collect()
}

/// Each cycle as its files, sorted, so the same cycle matches whatever file it starts from.
fn cycles(g: &Graph) -> BTreeSet<Vec<String>> {
    arch::cycles(g)
        .into_iter()
        .map(|c| {
            let mut v: Vec<String> = c.into_iter().map(|i| g.nodes[i].path.clone()).collect();
            v.sort();
            v
        })
        .collect()
}

/// From `old` to `now`.
pub fn compare(old: &Graph, now: &Graph) -> Changes {
    let (a, b) = (imports(old), imports(now));
    let (ca, cb) = (cycles(old), cycles(now));
    let files = |g: &Graph| g.nodes.iter().map(|n| n.path.clone()).collect::<BTreeSet<_>>();
    let (fa, fb) = (files(old), files(now));
    Changes {
        imports_added: b.difference(&a).cloned().collect(),
        imports_removed: a.difference(&b).cloned().collect(),
        cycles_new: cb.difference(&ca).cloned().collect(),
        cycles_gone: ca.difference(&cb).cloned().collect(),
        files_added: fb.difference(&fa).cloned().collect(),
        files_removed: fa.difference(&fb).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deps::{Edge, Lang, Node};

    fn graph(files: &[&str], edges: &[(usize, usize)]) -> Graph {
        Graph {
            nodes: files.iter().map(|p| Node { path: (*p).into(), lang: Lang::Rust, test: false }).collect(),
            edges: edges.iter().map(|&(from, to)| Edge { from, to, line: 1 }).collect(),
            ..Graph::default()
        }
    }

    #[test]
    fn name_status() {
        let d = parse_name_status("M\tsrc/a.rs\nA\tsrc/new.rs\nD\tsrc/old.rs\nT\tlink\n\n");
        assert_eq!(d.added, ["src/new.rs"]);
        assert_eq!(d.deleted, ["src/old.rs"]);
        assert_eq!(d.modified, ["src/a.rs", "link"]);
    }

    #[test]
    fn what_changed() {
        // Before: a → b, b → c. Now: a → b, b → a (a cycle), c gone, d new and d → a.
        let old = graph(&["a.rs", "b.rs", "c.rs"], &[(0, 1), (1, 2)]);
        let now = graph(&["a.rs", "b.rs", "d.rs"], &[(0, 1), (1, 0), (2, 0)]);
        let c = compare(&old, &now);
        let pair = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        assert_eq!(c.imports_added, [pair("b.rs", "a.rs"), pair("d.rs", "a.rs")]);
        assert_eq!(c.imports_removed, [pair("b.rs", "c.rs")]);
        assert_eq!(c.cycles_new, [vec!["a.rs".to_owned(), "b.rs".to_owned()]]);
        assert!(c.cycles_gone.is_empty());
        assert_eq!(c.files_added, ["d.rs"]);
        assert_eq!(c.files_removed, ["c.rs"]);
        assert!(compare(&now, &now).is_empty());
    }
}
