//! What a dependency graph says about a project's architecture: import cycles, how coupled
//! each file is, what a change to a file reaches (and which tests), the same graph by folder,
//! and a layered drawing of it, as positions or as SVG and Mermaid text.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use super::deps::{Edge, Graph, Lang, Node};

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

/// Where each node goes in a drawing: imports point downward, layer by layer, so what a file
/// uses sits below it. Positions are in units of one node: `x` across, `y` the layer.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pos: Vec<(f32, f32)>,
    pub layers: usize,
    pub width: usize,
}

pub fn layout(graph: &Graph) -> Layout {
    let n = graph.nodes.len();
    if n == 0 {
        return Layout::default();
    }
    // Cycles first: edges inside one are left out of the layering.
    let in_cycle = cycle_edges(graph);
    let mut out = vec![Vec::new(); n];
    let mut indegree = vec![0; n];
    for e in &graph.edges {
        if !in_cycle.contains(&(e.from, e.to)) {
            out[e.from].push(e.to);
            indegree[e.to] += 1;
        }
    }
    // Longest path from the top: a file sits below everything that imports it.
    let mut layer = vec![0usize; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
    let mut seen = 0;
    while let Some(v) = queue.pop_front() {
        seen += 1;
        for &w in &out[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    debug_assert_eq!(seen, n, "cycle edges left out, so the rest is acyclic");
    let layers = layer.iter().max().map_or(0, |m| m + 1);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for v in 0..n {
        rows[layer[v]].push(v);
    }
    // A few sweeps ordering each row by where its neighbours are, so fewer edges cross.
    let (all_out, all_in) = graph.adjacency();
    let mut x = vec![0.0f32; n];
    for row in &rows {
        for (i, &v) in row.iter().enumerate() {
            x[v] = i as f32;
        }
    }
    for sweep in 0..6 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> =
            if down { (1..layers).collect() } else { (0..layers.saturating_sub(1)).rev().collect() };
        for r in order {
            let row = &mut rows[r];
            let mut keyed: Vec<(f32, usize)> = row
                .iter()
                .map(|&v| {
                    let near = if down { &all_in[v] } else { &all_out[v] };
                    let key = if near.is_empty() {
                        x[v]
                    } else {
                        near.iter().map(|&w| x[w]).sum::<f32>() / near.len() as f32
                    };
                    (key, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            *row = keyed.into_iter().map(|(_, v)| v).collect();
            for (i, &v) in row.iter().enumerate() {
                x[v] = i as f32;
            }
        }
    }
    // A layer wider than the picture would be (many entry points or tests, say) wraps onto
    // several rows, so the drawing comes out roughly as wide as it is tall.
    let most = ((n as f32 * 2.5).sqrt().ceil() as usize).max(6);
    let rows: Vec<&[usize]> = rows.iter().flat_map(|row| row.chunks(most)).collect();
    let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut pos = vec![(0.0, 0.0); n];
    for (r, row) in rows.iter().enumerate() {
        // Each row centred under the widest.
        let offset = (width - row.len()) as f32 / 2.0;
        for (i, &v) in row.iter().enumerate() {
            pos[v] = (offset + i as f32, r as f32);
        }
    }
    Layout { pos, layers: rows.len(), width }
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

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The graph as an SVG picture: boxes by layer, arrows for imports, imports in cycles in red.
pub fn to_svg(graph: &Graph, layout: &Layout) -> String {
    const W: f32 = 180.0;
    const H: f32 = 34.0;
    const GAP_X: f32 = 30.0;
    const GAP_Y: f32 = 70.0;
    let place = |(x, y): (f32, f32)| (20.0 + x * (W + GAP_X), 20.0 + y * (H + GAP_Y));
    let width = 40.0 + layout.width as f32 * (W + GAP_X);
    let height = 40.0 + layout.layers as f32 * (H + GAP_Y);
    let cyclic = cycle_edges(graph);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"Segoe UI, sans-serif\" font-size=\"12\">\n\
         <defs><marker id=\"a\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#6b7a99\"/></marker>\
         <marker id=\"r\" viewBox=\"0 0 10 10\" refX=\"10\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto\"><path d=\"M0,0 L10,5 L0,10 z\" fill=\"#e5484d\"/></marker></defs>\n\
         <rect width=\"100%\" height=\"100%\" fill=\"#0f1a33\"/>\n"
    );
    for e in &graph.edges {
        let (ax, ay) = place(layout.pos[e.from]);
        let (bx, by) = place(layout.pos[e.to]);
        let (x1, y1) = (ax + W / 2.0, if by >= ay { ay + H } else { ay });
        let (x2, y2) = (bx + W / 2.0, if by >= ay { by } else { by + H });
        let red = cyclic.contains(&(e.from, e.to));
        let (color, marker) = if red { ("#e5484d", "r") } else { ("#6b7a99", "a") };
        svg.push_str(&format!(
            "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\" stroke=\"{color}\" stroke-width=\"1.2\" marker-end=\"url(#{marker})\"/>\n"
        ));
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        let (x, y) = place(layout.pos[i]);
        let fill = if node.test { "#1f3a2c" } else { "#16264d" };
        let stroke = if node.test { "#57c06a" } else { "#2f8cff" };
        let name = label(&node.path);
        let label: String =
            if name.chars().count() > 26 { name.chars().take(25).chain(['…']).collect() } else { name };
        svg.push_str(&format!(
            "<g><title>{}</title><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{W}\" height=\"{H}\" rx=\"8\" fill=\"{fill}\" stroke=\"{stroke}\"/>\
             <text x=\"{:.1}\" y=\"{:.1}\" fill=\"#e8eefc\" text-anchor=\"middle\">{}</text></g>\n",
            escape(&node.path),
            x + W / 2.0,
            y + H / 2.0 + 4.0,
            escape(&label)
        ));
    }
    svg.push_str("</svg>\n");
    svg
}

/// Mermaid stops drawing a diagram past this many arrows (GitHub's limit); the folder graph is
/// the readable one there.
pub const MERMAID_MAX_EDGES: usize = 500;

/// The folder a relative path sits in, `.` at the project's root.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(".", |(dir, _)| dir)
}

/// Text Mermaid reads as it is: `#` starts its entity codes, and quotes and brackets end a
/// label or a box, so those become codes (Mermaid reads `#quot;` and friends back as the
/// character).
fn mermaid_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '#' => out.push_str("#35;"),
            '"' => out.push_str("#quot;"),
            '&' => out.push_str("#38;"),
            '<' => out.push_str("#lt;"),
            '>' => out.push_str("#gt;"),
            '[' => out.push_str("#91;"),
            ']' => out.push_str("#93;"),
            '(' => out.push_str("#40;"),
            ')' => out.push_str("#41;"),
            '{' => out.push_str("#123;"),
            '}' => out.push_str("#125;"),
            '|' => out.push_str("#124;"),
            other => out.push(other),
        }
    }
    out
}

/// The graph as a Mermaid flowchart, for Markdown files and wikis: files sit in a subgraph per
/// folder, tests are green, imports in cycles are red, and the arrows point from the file to
/// what it uses.
pub fn to_mermaid(graph: &Graph, nodes_are: &str) -> String {
    let cyclic = cycle_edges(graph);
    let mut in_cycle = vec![false; graph.nodes.len()];
    for (from, to) in &cyclic {
        in_cycle[*from] = true;
        in_cycle[*to] = true;
    }
    let mut text = String::from("flowchart TD\n");
    text.push_str(&format!("  %% {} {nodes_are}, {} imports\n", graph.nodes.len(), graph.edges.len()));
    // Folders nest as subgraphs, as they do on disk, so each name is read inside its folder.
    // A folder holding no files and one subfolder shares its box ("crates/coiman-core").
    let mut files_in: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut subfolders: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        let folder = folder_of(&node.path);
        let folder = if folder == "." { String::new() } else { folder.to_owned() };
        files_in.entry(folder.clone()).or_default().push(i);
        let mut here = folder;
        while !here.is_empty() {
            let parent = here.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_owned());
            subfolders.entry(parent.clone()).or_default().insert(here);
            here = parent;
        }
    }
    let mut groups = 0;
    fn emit(
        text: &mut String,
        graph: &Graph,
        files_in: &BTreeMap<String, Vec<usize>>,
        subfolders: &BTreeMap<String, std::collections::BTreeSet<String>>,
        groups: &mut usize,
        folder: &str,
        depth: usize,
    ) {
        let pad = "  ".repeat(depth);
        for &i in files_in.get(folder).into_iter().flatten() {
            text.push_str(&format!("{pad}n{i}[\"{}\"]\n", mermaid_text(short_name(&graph.nodes[i].path))));
        }
        for child in subfolders.get(folder).into_iter().flatten() {
            // Down through folders that only hold one folder.
            let mut inner = child.clone();
            loop {
                let only = subfolders.get(&inner).filter(|s| s.len() == 1 && !files_in.contains_key(&inner));
                match only.and_then(|s| s.iter().next()) {
                    Some(next) => inner = next.clone(),
                    None => break,
                }
            }
            let name = inner.strip_prefix(folder).unwrap_or(&inner).trim_start_matches('/');
            text.push_str(&format!("{pad}subgraph g{}[\"{}\"]\n", *groups, mermaid_text(name)));
            *groups += 1;
            emit(text, graph, files_in, subfolders, groups, &inner, depth + 1);
            text.push_str(&format!("{pad}end\n"));
        }
    }
    emit(&mut text, graph, &files_in, &subfolders, &mut groups, "", 1);
    // Colors that read in Mermaid's light and dark themes alike: the text color is set too.
    if graph.nodes.iter().any(|n| n.test) {
        text.push_str("  classDef test fill:#d3f9d8,stroke:#2f9e44,color:#0b2e13\n");
    }
    if in_cycle.iter().any(|&c| c) {
        text.push_str("  classDef cycle stroke:#e5484d,stroke-width:2px\n");
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        if node.test {
            text.push_str(&format!("  class n{i} test\n"));
        }
        if in_cycle[i] {
            text.push_str(&format!("  class n{i} cycle\n"));
        }
    }
    // The arrows, then the ones inside a cycle colored; linkStyle counts them from the first.
    let mut in_cycle_edges = Vec::new();
    for (k, e) in graph.edges.iter().enumerate() {
        text.push_str(&format!("  n{} --> n{}\n", e.from, e.to));
        if cyclic.contains(&(e.from, e.to)) {
            in_cycle_edges.push(k.to_string());
        }
    }
    if !in_cycle_edges.is_empty() {
        text.push_str(&format!("  linkStyle {} stroke:#e5484d,stroke-width:2px\n", in_cycle_edges.join(",")));
    }
    text
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
    fn layout_puts_imports_below_and_survives_cycles() {
        let graph = g(&["top", "mid", "low", "x", "y"], &[(0, 1), (1, 2), (0, 2), (3, 4), (4, 3)]);
        let l = layout(&graph);
        assert!(l.pos[0].1 < l.pos[1].1 && l.pos[1].1 < l.pos[2].1);
        assert_eq!(l.layers, 3);
        // A hundred files nothing imports wrap onto rows instead of one long line.
        let paths: Vec<String> = (0..101).map(|i| format!("f{i}")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let wide = layout(&g(&refs, &(1..101).map(|i| (i, 0)).collect::<Vec<_>>()));
        assert!(wide.width <= 16 && wide.layers >= 7, "{} x {}", wide.width, wide.layers);
        assert!(wide.pos[0].1 > wide.pos[100].1, "the shared import stays below them all");
        let svg = to_svg(&graph, &l);
        assert!(svg.starts_with("<svg") && svg.contains("#e5484d") && svg.matches("<rect x=").count() == 5);
        assert!(to_mermaid(&graph, "files").contains("n0 --> n1"));
    }

    #[test]
    fn mermaid_groups_folders_styles_cycles_and_escapes() {
        let graph = g(
            &["src/a.rs", "src/b.rs", "src/mod.rs", "lib/c.rs", "main.rs"],
            &[(0, 1), (1, 0), (0, 2), (3, 0), (4, 3)],
        );
        let text = to_mermaid(&graph, "files");
        assert!(text.starts_with("flowchart TD\n  %% 5 files, 5 imports\n"), "{text}");
        // Files of one folder share a subgraph, named by the folder; root files stand outside.
        assert!(text.contains("  subgraph g1[\"src\"]\n    n0[\"a.rs\"]"), "{text}");
        assert!(text.contains("  subgraph g0[\"lib\"]\n    n3[\"c.rs\"]"), "{text}");
        assert!(text.contains("\n  n4[\"main.rs\"]"), "{text}");
        // The first two imports are the two halves of a cycle; the third isn't.
        assert!(text.contains("linkStyle 0,1 stroke:#e5484d"), "{text}");
        assert!(text.contains("class n0 cycle") && text.contains("classDef cycle"), "{text}");
        assert!(text.contains("n3 --> n0") && !text.contains("linkStyle 2"), "{text}");
    }

    #[test]
    fn mermaid_escapes_what_it_would_read_as_syntax() {
        let graph = g(&["a #1 (x) \"q\" [y].rs"], &[]);
        let text = to_mermaid(&graph, "files");
        assert!(text.contains("n0[\"a #35;1 #40;x#41; #quot;q#quot; #91;y#93;.rs\"]"), "{text}");
        // A folder named with any of those too.
        let graph = g(&["src [old]/a.rs", "src [old]/b.rs"], &[(0, 1)]);
        let text = to_mermaid(&graph, "files");
        assert!(text.contains("subgraph g0[\"src #91;old#93;\"]"), "{text}");
    }

    #[test]
    fn mermaid_nests_folders_as_on_disk() {
        let graph = g(
            &["crates/core/src/a.rs", "crates/core/src/x/b.rs", "crates/core/tests/t.rs", "plugins/p/src/lib.rs"],
            &[(0, 1)],
        );
        let text = to_mermaid(&graph, "files");
        // crates/core holds no files and two folders: one box, with src and tests inside.
        assert!(text.contains("subgraph g0[\"crates/core\"]\n    subgraph g1[\"src\"]\n      n0[\"a.rs\"]"), "{text}");
        assert!(text.contains("      subgraph g2[\"x\"]\n        n1[\"b.rs\"]"), "{text}");
        // A chain of single folders is one box, named by the whole chain.
        assert!(text.contains("subgraph g4[\"plugins/p/src\"]\n    n3[\"lib.rs\"]"), "{text}");
        assert_eq!(text.matches("subgraph").count(), text.matches("\n  end\n").count() + 3, "{text}");
        // Tests read on light and dark backgrounds.
        assert!(text.contains("classDef test fill:#d3f9d8,stroke:#2f9e44,color:#0b2e13"), "{text}");
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
