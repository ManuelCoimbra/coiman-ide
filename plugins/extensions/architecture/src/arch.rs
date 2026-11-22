//! What a dependency graph says about a project's architecture: import cycles, how coupled
//! each file is, what a change to a file reaches (and which tests), the same graph by folder,
//! and a layered drawing of it, as positions or as SVG and Mermaid text.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::deps::{Edge, Graph, Lang, Node};

/// Groups of files that import each other round in a circle, largest first. A group is
/// strongly connected: from any of its files, imports lead to every other.
pub fn cycles(graph: &Graph) -> Vec<Vec<usize>> {
    let (out, _) = graph.adjacency();
    let n = graph.nodes.len();
    // Tarjan's algorithm, without recursion so deep graphs don't overflow the stack.
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut groups = Vec::new();
    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        let mut work: Vec<(usize, usize)> = vec![(start, 0)];
        index[start] = next;
        low[start] = next;
        next += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&mut (v, ref mut child)) = work.last_mut() {
            if let Some(&w) = out[v].get(*child) {
                *child += 1;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut group = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    group.push(w);
                    if w == v {
                        break;
                    }
                }
                if group.len() > 1 {
                    group.sort_unstable();
                    groups.push(group);
                }
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    groups
}

/// The edges inside cycles: the imports that would have to go to break them.
pub fn cycle_edges(graph: &Graph) -> HashSet<(usize, usize)> {
    let mut group_of = HashMap::new();
    for (g, members) in cycles(graph).iter().enumerate() {
        for &m in members {
            group_of.insert(m, g);
        }
    }
    graph
        .edges
        .iter()
        .filter(|e| group_of.get(&e.from).is_some_and(|g| group_of.get(&e.to) == Some(g)))
        .map(|e| (e.from, e.to))
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coupling {
    /// Files that import this one.
    pub fan_in: usize,
    /// Files this one imports.
    pub fan_out: usize,
    /// `fan_out / (fan_in + fan_out)`: 0 for a file everything leans on, 1 for one nothing does.
    pub instability: f32,
}

pub fn coupling(graph: &Graph) -> Vec<Coupling> {
    let mut fan_in = vec![0; graph.nodes.len()];
    let mut fan_out = vec![0; graph.nodes.len()];
    for e in &graph.edges {
        fan_out[e.from] += 1;
        fan_in[e.to] += 1;
    }
    fan_in
        .iter()
        .zip(&fan_out)
        .map(|(&i, &o)| Coupling {
            fan_in: i,
            fan_out: o,
            instability: if i + o == 0 { 0.0 } else { o as f32 / (i + o) as f32 },
        })
        .collect()
}

/// Files nothing imports and that import nothing.
pub fn isolated(graph: &Graph) -> Vec<usize> {
    coupling(graph).iter().enumerate().filter(|(_, c)| c.fan_in == 0 && c.fan_out == 0).map(|(i, _)| i).collect()
}

/// Everything reachable from `node`, with how many steps away: through imports with
/// `forward` (what it uses), or against them (what uses it, so what a change to it reaches).
pub fn reach(graph: &Graph, node: usize, forward: bool) -> Vec<(usize, usize)> {
    let (out, inc) = graph.adjacency();
    let next = if forward { &out } else { &inc };
    let mut depth = vec![usize::MAX; graph.nodes.len()];
    depth[node] = 0;
    let mut queue = VecDeque::from([node]);
    let mut found = Vec::new();
    while let Some(v) = queue.pop_front() {
        for &w in &next[v] {
            if depth[w] == usize::MAX {
                depth[w] = depth[v] + 1;
                found.push((w, depth[w]));
                queue.push_back(w);
            }
        }
    }
    found
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Impact {
    /// Files that import this one directly.
    pub direct: Vec<usize>,
    /// Files that reach it through others.
    pub indirect: Vec<usize>,
    /// Test files among all of those, and the file itself if it holds tests.
    pub tests: Vec<usize>,
}

/// What a change to `node` can affect.
pub fn impact(graph: &Graph, node: usize) -> Impact {
    let mut impact = Impact::default();
    for (w, depth) in reach(graph, node, false) {
        if depth == 1 {
            impact.direct.push(w)
        } else {
            impact.indirect.push(w)
        }
        if graph.nodes[w].test {
            impact.tests.push(w);
        }
    }
    if graph.nodes[node].test {
        impact.tests.insert(0, node);
    }
    impact
}

/// The graph by folder: a node per folder holding source files, an edge where any file of
/// one imports any file of the other, `line` holding how many file imports it stands for.
pub fn by_folder(graph: &Graph) -> Graph {
    let folder = |path: &str| path.rsplit_once('/').map_or(".", |(dir, _)| dir).to_owned();
    let mut ids: BTreeMap<String, (Lang, bool)> = BTreeMap::new();
    for node in &graph.nodes {
        let entry = ids.entry(folder(&node.path)).or_insert((node.lang, true));
        entry.1 &= node.test;
    }
    let names: Vec<String> = ids.keys().cloned().collect();
    let place: HashMap<&str, usize> = names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
    let nodes = names.iter().map(|n| Node { path: n.clone(), lang: ids[n].0, test: ids[n].1 }).collect();
    let mut counts: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for e in &graph.edges {
        let a = place[folder(&graph.nodes[e.from].path).as_str()];
        let b = place[folder(&graph.nodes[e.to].path).as_str()];
        if a != b {
            *counts.entry((a, b)).or_default() += 1;
        }
    }
    let edges = counts.into_iter().map(|((from, to), n)| Edge { from, to, line: n }).collect();
    Graph { root: graph.root.clone(), nodes, edges, truncated: graph.truncated }
}

/// The last part of a path, which is what a drawing labels a node with.
pub fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A node's label: its name, with its folder when the name says little on its own
/// (`mod.rs`, `index.ts`, `__init__.py`...).
pub fn label(path: &str) -> String {
    let name = short_name(path);
    let stem = name.split('.').next().unwrap_or(name);
    let generic = matches!(stem, "mod" | "lib" | "main" | "index" | "__init__" | "__main__" | "init" | "Program");
    match path.rsplit('/').nth(1).filter(|_| generic) {
        Some(folder) => format!("{folder}/{name}"),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn g(paths: &[&str], edges: &[(usize, usize)]) -> Graph {
        Graph {
            root: PathBuf::from("."),
            nodes: paths
                .iter()
                .map(|p| Node { path: (*p).to_owned(), lang: Lang::Rust, test: p.contains("test") })
                .collect(),
            edges: edges.iter().map(|&(from, to)| Edge { from, to, line: 1 }).collect(),
            truncated: false,
        }
    }

    #[test]
    fn cycles_are_found_and_only_cycles() {
        // 0 -> 1 -> 2 -> 0 is a cycle; 3 -> 4 -> 3 another; 5 hangs off.
        let graph = g(&["a", "b", "c", "d", "e", "f"], &[(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 3), (5, 0)]);
        assert_eq!(cycles(&graph), vec![vec![0, 1, 2], vec![3, 4]]);
        let inside = cycle_edges(&graph);
        assert!(inside.contains(&(2, 0)) && inside.contains(&(4, 3)));
        assert!(!inside.contains(&(2, 3)) && !inside.contains(&(5, 0)));
        assert!(cycles(&g(&["a", "b"], &[(0, 1)])).is_empty());
    }

    #[test]
    fn a_long_chain_does_not_overflow() {
        let paths: Vec<String> = (0..50_000).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let edges: Vec<(usize, usize)> = (0..49_999).map(|i| (i, i + 1)).chain([(49_999, 0)]).collect();
        let graph = g(&refs, &edges);
        assert_eq!(cycles(&graph)[0].len(), 50_000);
    }

    #[test]
    fn coupling_reach_and_impact() {
        // main -> cart -> item; cart_test -> cart; util alone.
        let graph = g(&["main", "cart", "item", "cart_test", "util"], &[(0, 1), (1, 2), (3, 1)]);
        let c = coupling(&graph);
        assert_eq!((c[1].fan_in, c[1].fan_out), (2, 1));
        assert_eq!(c[2].instability, 0.0);
        assert_eq!(c[0].instability, 1.0);
        assert_eq!(isolated(&graph), vec![4]);
        let item = impact(&graph, 2);
        assert_eq!(item.direct, vec![1]);
        let mut indirect = item.indirect.clone();
        indirect.sort_unstable();
        assert_eq!(indirect, vec![0, 3]);
        assert_eq!(item.tests, vec![3]);
        assert_eq!(reach(&graph, 0, true), vec![(1, 1), (2, 2)]);
    }

    #[test]
    fn folders_merge_files_and_count_imports() {
        let graph = g(&["src/a.rs", "src/b.rs", "lib/c.rs", "main.rs"], &[(0, 2), (1, 2), (0, 1), (3, 0)]);
        let folders = by_folder(&graph);
        let names: Vec<&str> = folders.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(names, vec![".", "lib", "src"]);
        let src_to_lib = folders.edges.iter().find(|e| e.from == 2 && e.to == 1).unwrap();
        assert_eq!(src_to_lib.line, 2, "two file imports");
        assert_eq!(folders.edges.len(), 2, "src -> src is inside one folder");
    }

    #[test]
    fn labels_say_which_mod_rs() {
        assert_eq!(label("src/app/mod.rs"), "app/mod.rs");
        assert_eq!(label("pkg/__init__.py"), "pkg/__init__.py");
        assert_eq!(label("src/lib/index.ts"), "lib/index.ts");
        assert_eq!(label("src/buffer.rs"), "buffer.rs");
        assert_eq!(label("main.rs"), "main.rs");
    }
}
