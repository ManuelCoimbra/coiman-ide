//! The types read from the project's files, made into a graph: each type a box, what it
//! extends or implements a line to its parent (a parent from outside the project gets a box
//! of its own, marked external), and a field whose type is the project's a line to that type.
//! Lookups go through maps: a project has thousands of types.

use std::collections::{HashMap, HashSet};

use crate::parse::{Rel, Type};

pub struct FileTypes {
    /// Relative to the open folder, with `/`.
    pub path: String,
    pub types: Vec<Type>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Inherits,
    Implements,
    Has,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub group: String,
    pub members: Vec<String>,
    pub path: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub kind: Kind,
    pub label: String,
}

#[derive(Debug, Default)]
pub struct Built {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// A type to pick and centre on.
    pub focus: Option<String>,
    /// Types left out to keep the picture drawable.
    pub dropped: usize,
}

pub enum Scope<'a> {
    /// Every type, at most this many (those with relations first).
    All(usize),
    /// The types of one file and those related to them.
    File(&'a str),
}

fn folder(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

pub fn build(files: &[FileTypes], scope: Scope) -> Built {
    // Every type of the project: (id, path, type).
    let mut all: Vec<(String, &str, &Type)> = Vec::new();
    for f in files {
        for t in &f.types {
            all.push((format!("{}#{}", f.path, t.name), f.path.as_str(), t));
        }
    }
    let group = |path: &str, t: &Type| if t.group.is_empty() { folder(path).to_owned() } else { t.group.clone() };
    let groups: Vec<String> = all.iter().map(|(_, path, t)| group(path, t)).collect();
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, (_, _, t)) in all.iter().enumerate() {
        by_name.entry(t.name.as_str()).or_default().push(i);
    }
    // A name used in `path` means the type of that name in the same file, else the same
    // group, else any.
    let resolve = |name: &str, path: &str, grp: &str| -> Option<usize> {
        let same = by_name.get(name)?;
        same.iter()
            .copied()
            .find(|&i| all[i].1 == path)
            .or_else(|| same.iter().copied().find(|&i| groups[i] == grp))
            .or_else(|| same.first().copied())
    };
    let mut nodes: Vec<Node> = all
        .iter()
        .map(|(id, path, t)| Node {
            id: id.clone(),
            label: t.label.clone(),
            kind: t.kind.to_owned(),
            group: group(path, t),
            members: t.fields.iter().map(|f| f.text.clone()).chain(t.methods.iter().cloned()).collect(),
            path: (*path).to_owned(),
            line: t.line,
        })
        .collect();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<(String, String, u8)> = HashSet::new();
    let mut externals: Vec<String> = Vec::new();
    let mut known_external: HashSet<String> = HashSet::new();
    let mut push = |edges: &mut Vec<Edge>, e: Edge| {
        if e.source != e.target && seen.insert((e.source.clone(), e.target.clone(), e.kind as u8)) {
            edges.push(e);
        }
    };
    for (k, (id, path, t)) in all.iter().enumerate() {
        let grp = &groups[k];
        for (parent, rel) in &t.parents {
            let kind = if *rel == Rel::Extends { Kind::Inherits } else { Kind::Implements };
            let target = match resolve(parent, path, grp) {
                Some(i) => all[i].0.clone(),
                None => {
                    if known_external.insert(parent.clone()) {
                        externals.push(parent.clone());
                    }
                    format!("external:{parent}")
                }
            };
            push(&mut edges, Edge { source: id.clone(), target, kind, label: String::new() });
        }
        for f in &t.fields {
            for name in &f.types {
                if name == &t.name {
                    continue;
                }
                if let Some(i) = resolve(name, path, grp) {
                    let label = if f.embedded { "embeds".to_owned() } else { String::new() };
                    push(&mut edges, Edge { source: id.clone(), target: all[i].0.clone(), kind: Kind::Has, label });
                }
            }
        }
    }
    for name in externals {
        nodes.push(Node {
            id: format!("external:{name}"),
            label: name,
            kind: "external".to_owned(),
            group: String::new(),
            members: Vec::new(),
            path: String::new(),
            line: 0,
        });
    }
    let mut degree: HashMap<&str, usize> = HashMap::new();
    for e in &edges {
        *degree.entry(e.source.as_str()).or_default() += 1;
        *degree.entry(e.target.as_str()).or_default() += 1;
    }
    let mut focus = None;
    let keep: Vec<bool> = match scope {
        Scope::All(most) => {
            if nodes.len() <= most {
                vec![true; nodes.len()]
            } else {
                let mut order: Vec<usize> = (0..nodes.len()).collect();
                order.sort_by_key(|&i| std::cmp::Reverse(degree.get(nodes[i].id.as_str()).copied().unwrap_or(0)));
                let mut keep = vec![false; nodes.len()];
                for &i in order.iter().take(most) {
                    keep[i] = true;
                }
                keep
            }
        }
        Scope::File(file) => {
            let core: Vec<&str> = nodes.iter().filter(|n| n.path == file).map(|n| n.id.as_str()).collect();
            focus = core.first().map(|c| (*c).to_owned());
            let core_set: HashSet<&str> = core.iter().copied().collect();
            let mut wanted: HashSet<&str> = core_set.clone();
            let mut parents_of: HashMap<&str, Vec<&str>> = HashMap::new();
            for e in edges.iter().filter(|e| e.kind != Kind::Has) {
                parents_of.entry(e.source.as_str()).or_default().push(e.target.as_str());
            }
            // Every ancestor.
            let mut frontier = core.clone();
            while let Some(id) = frontier.pop() {
                for &p in parents_of.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                    if wanted.insert(p) {
                        frontier.push(p);
                    }
                }
            }
            // Children and what they hold, one step.
            for e in &edges {
                if core_set.contains(e.target.as_str()) {
                    wanted.insert(e.source.as_str());
                }
                if core_set.contains(e.source.as_str()) {
                    wanted.insert(e.target.as_str());
                }
            }
            nodes.iter().map(|n| wanted.contains(n.id.as_str())).collect()
        }
    };
    let dropped = keep.iter().filter(|k| !**k).count();
    let kept: Vec<Node> = nodes.into_iter().zip(keep).filter(|(_, k)| *k).map(|(n, _)| n).collect();
    let ids: HashSet<&str> = kept.iter().map(|n| n.id.as_str()).collect();
    let edges: Vec<Edge> =
        edges.into_iter().filter(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str())).collect();
    let dropped = if matches!(scope, Scope::All(_)) { dropped } else { 0 };
    Built { nodes: kept, edges, focus, dropped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{Lang, parse};

    fn files() -> Vec<FileTypes> {
        let java = |path: &str, src: &str| FileTypes { path: path.to_owned(), types: parse(Lang::Java, src) };
        vec![
            java(
                "src/model/Animal.java",
                "package model; public abstract class Animal implements Comparable<Animal> { Owner owner; }",
            ),
            java("src/model/Dog.java", "package model; public class Dog extends Animal { private Collar collar; }"),
            java("src/model/Collar.java", "package model; public class Collar {}"),
            java("src/model/Owner.java", "package model; public class Owner { java.util.List<Dog> dogs; }"),
            java("src/other/Collar.java", "package other; public class Collar {}"),
            java("src/util/Lonely.java", "package util; class Lonely {}"),
        ]
    }

    #[test]
    fn whole_project() {
        let b = build(&files(), Scope::All(100));
        let has = |s: &str, t: &str, k: Kind| {
            b.edges.iter().any(|e| e.source.ends_with(s) && e.target.ends_with(t) && e.kind == k)
        };
        assert!(has("Dog.java#Dog", "Animal.java#Animal", Kind::Inherits));
        assert!(has("Animal.java#Animal", "external:Comparable", Kind::Implements));
        // The same package's Collar, not the other one.
        assert!(has("Dog.java#Dog", "model/Collar.java#Collar", Kind::Has));
        assert!(!b.edges.iter().any(|e| e.target.contains("other/")));
        assert!(has("Owner.java#Owner", "Dog.java#Dog", Kind::Has));
        assert!(has("Animal.java#Animal", "Owner.java#Owner", Kind::Has));
        let ext = b.nodes.iter().find(|n| n.id == "external:Comparable").unwrap();
        assert_eq!(ext.kind, "external");
        let dog = b.nodes.iter().find(|n| n.label == "Dog").unwrap();
        assert_eq!((dog.group.as_str(), dog.line, dog.path.as_str()), ("model", 1, "src/model/Dog.java"));
        assert_eq!(dog.members, ["- collar: Collar"]);
        assert_eq!(b.nodes.len(), 7);
        // Too many: the ones with relations stay.
        let small = build(&files(), Scope::All(3));
        assert_eq!((small.nodes.len(), small.dropped), (3, 4));
        assert!(!small.nodes.iter().any(|n| n.label == "Lonely"));
    }

    #[test]
    fn one_file() {
        let b = build(&files(), Scope::File("src/model/Dog.java"));
        let labels: Vec<&str> = b.nodes.iter().map(|n| n.label.as_str()).collect();
        for want in ["Dog", "Animal", "Comparable", "Collar", "Owner"] {
            assert!(labels.contains(&want), "{want} missing from {labels:?}");
        }
        assert!(!labels.contains(&"Lonely"));
        assert_eq!(b.focus.as_deref(), Some("src/model/Dog.java#Dog"));
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use crate::parse::{Lang, parse};

    /// How long reading this repository takes, natively:
    /// `cargo test --release timing -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn this_repository() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if p.is_dir() {
                    if !name.starts_with('.') && name != "target" && name != "node_modules" {
                        walk(&p, out);
                    }
                } else {
                    out.push(p);
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut paths = Vec::new();
        walk(&root, &mut paths);
        let start = std::time::Instant::now();
        let mut files = Vec::new();
        let mut slowest = (std::time::Duration::ZERO, String::new());
        for p in &paths {
            let rel = p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            let Some(lang) = Lang::of(&rel) else { continue };
            let Ok(text) = std::fs::read_to_string(p) else { continue };
            let t = std::time::Instant::now();
            let types = parse(lang, &text);
            if t.elapsed() > slowest.0 {
                slowest = (t.elapsed(), rel.clone());
            }
            files.push(FileTypes { path: rel, types });
        }
        let parsed = start.elapsed();
        let types: usize = files.iter().map(|f| f.types.len()).sum();
        let t = std::time::Instant::now();
        let built = build(&files, Scope::All(400));
        println!(
            "{} files, {types} types, parsed in {parsed:?} (slowest {:?} {}), built in {:?}: {} nodes, {} edges",
            files.len(),
            slowest.0,
            slowest.1,
            t.elapsed(),
            built.nodes.len(),
            built.edges.len()
        );
    }
}
