//! What the editor does with a plugin's graph (`show-graph`): lays the boxes out, with what is
//! inherited above what inherits it, and writes the graph as SVG, Mermaid and PlantUML.
//!
//! Sizes are estimated from the text (the editor draws at the same sizes), so the layout,
//! the window and the SVG agree.

use std::collections::{HashMap, VecDeque};

use super::host::{EdgeKind, Graph, GraphNode};

/// Pixels of a character of the box text, a text row, and the padding inside a box.
pub const CHAR_W: f32 = 7.0;
pub const ROW_H: f32 = 17.0;
pub const PAD: f32 = 8.0;
/// Room between boxes across and between rows.
pub const GAP_X: f32 = 36.0;
pub const GAP_Y: f32 = 64.0;
/// The widest a box is drawn; longer lines are cut.
pub const MAX_W: f32 = 340.0;
/// Members listed in a box at most; the rest are summed up in one line.
pub const MAX_MEMBERS: usize = 14;

/// The stereotype shown above a node's name, when it has one.
pub fn stereotype(kind: &str) -> Option<String> {
    (!kind.is_empty() && kind != "class").then(|| format!("«{kind}»"))
}

/// The member lines a box shows: at most [`MAX_MEMBERS`], then how many more.
pub fn shown_members(node: &GraphNode) -> Vec<String> {
    let mut lines: Vec<String> = node.members.iter().take(MAX_MEMBERS).cloned().collect();
    if node.members.len() > MAX_MEMBERS {
        lines.push(format!("… {} more", node.members.len() - MAX_MEMBERS));
    }
    lines
}

/// A box's width and height, with or without its members.
pub fn box_size(node: &GraphNode, members: bool) -> (f32, f32) {
    let mut longest = node.label.chars().count() as f32 + 2.0;
    let mut rows = 1.0;
    if let Some(s) = stereotype(&node.kind) {
        longest = longest.max(s.chars().count() as f32);
        rows += 1.0;
    }
    let shown = if members { shown_members(node) } else { Vec::new() };
    for m in &shown {
        longest = longest.max(m.chars().count() as f32);
    }
    let w = (longest * CHAR_W + 2.0 * PAD).clamp(90.0, MAX_W);
    let extra = if shown.is_empty() { 0.0 } else { shown.len() as f32 * ROW_H + PAD };
    (w, rows * ROW_H + 2.0 * PAD + extra)
}

/// Where each box goes: (x, y, width, height), and the size of the whole picture.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    pub rects: Vec<[f32; 4]>,
    pub size: (f32, f32),
}

/// Lays the graph out. Inheritance and implementation make the layers (what is inherited
/// above what inherits it); nodes with neither go below, folded into rows.
pub fn layout(graph: &Graph, members: bool) -> Placed {
    let n = graph.nodes.len();
    if n == 0 {
        return Placed::default();
    }
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, node)| (node.id.as_str(), i)).collect();
    // Upper to lower: a parent above what inherits it, a user above what it uses.
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &graph.edges {
        let (Some(&s), Some(&t)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else {
            continue;
        };
        let (upper, lower) = match e.kind {
            EdgeKind::Inherits | EdgeKind::Implements => (t, s),
            EdgeKind::Uses => (s, t),
            EdgeKind::Has => continue,
        };
        if upper != lower && !down[upper].contains(&lower) {
            down[upper].push(lower);
            up[lower].push(upper);
        }
    }
    // Back edges (a type that ends up inheriting itself) are left out of the layering.
    let mut state = vec![0u8; n];
    let mut keep: Vec<Vec<usize>> = vec![Vec::new(); n];
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        state[start] = 1;
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            if let Some(&w) = down[v].get(*i) {
                *i += 1;
                match state[w] {
                    0 => {
                        keep[v].push(w);
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    2 => keep[v].push(w),
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let mut indegree = vec![0usize; n];
    for children in &keep {
        for &c in children {
            indegree[c] += 1;
        }
    }
    let connected: Vec<bool> = (0..n).map(|v| !down[v].is_empty() || !up[v].is_empty()).collect();
    let mut layer = vec![0usize; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&v| indegree[v] == 0).collect();
    while let Some(v) = queue.pop_front() {
        for &w in &keep[v] {
            layer[w] = layer[w].max(layer[v] + 1);
            indegree[w] -= 1;
            if indegree[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    let layers = (0..n).filter(|&v| connected[v]).map(|v| layer[v] + 1).max().unwrap_or(0);
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); layers];
    for v in (0..n).filter(|&v| connected[v]) {
        rows[layer[v]].push(v);
    }
    // A few sweeps ordering each row by where its parents and children are.
    let mut x = vec![0.0f32; n];
    let renumber = |rows: &Vec<Vec<usize>>, x: &mut Vec<f32>| {
        for row in rows {
            for (i, &v) in row.iter().enumerate() {
                x[v] = i as f32;
            }
        }
    };
    renumber(&rows, &mut x);
    for sweep in 0..8 {
        let downward = sweep % 2 == 0;
        let order: Vec<usize> =
            if downward { (1..layers).collect() } else { (0..layers.saturating_sub(1)).rev().collect() };
        for r in order {
            let near = if downward { &up } else { &down };
            let mut keyed: Vec<(f32, usize)> = rows[r]
                .iter()
                .map(|&v| {
                    let k = if near[v].is_empty() {
                        x[v]
                    } else {
                        near[v].iter().map(|&w| x[w]).sum::<f32>() / near[v].len() as f32
                    };
                    (k, v)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            rows[r] = keyed.into_iter().map(|(_, v)| v).collect();
            renumber(&rows, &mut x);
        }
    }
    // The rest below: each under the types it is related to (by a field, say), then the
    // unrelated ones by group and name.
    let across: Vec<f32> =
        (0..n).map(|v| if connected[v] { x[v] / rows[layer[v]].len().max(1) as f32 } else { f32::NAN }).collect();
    let mut near_x: Vec<Vec<f32>> = vec![Vec::new(); n];
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        if !across[b].is_nan() {
            near_x[a].push(across[b]);
        }
        if !across[a].is_nan() {
            near_x[b].push(across[a]);
        }
    }
    let key = |v: usize| {
        let xs = &near_x[v];
        if xs.is_empty() { 2.0 } else { xs.iter().sum::<f32>() / xs.len() as f32 }
    };
    let mut loose: Vec<usize> = (0..n).filter(|&v| !connected[v]).collect();
    loose.sort_by(|&a, &b| {
        let (na, nb) = (&graph.nodes[a], &graph.nodes[b]);
        key(a).total_cmp(&key(b)).then_with(|| na.group.cmp(&nb.group)).then_with(|| na.label.cmp(&nb.label))
    });
    // Boxes per row: enough that the picture comes out about half again as wide as it is
    // tall, from the boxes' average size.
    let sizes: Vec<(f32, f32)> = graph.nodes.iter().map(|node| box_size(node, members)).collect();
    let (avg_w, avg_h) = (
        sizes.iter().map(|s| s.0).sum::<f32>() / n as f32 + GAP_X,
        sizes.iter().map(|s| s.1).sum::<f32>() / n as f32 + GAP_Y,
    );
    let most = ((1.5 * n as f32 * avg_h / avg_w).sqrt().ceil() as usize).max(4);
    let mut all_rows: Vec<Vec<usize>> = rows.iter().flat_map(|r| r.chunks(most).map(<[usize]>::to_vec)).collect();
    all_rows.extend(loose.chunks(most).map(<[usize]>::to_vec));

    let row_w =
        |row: &[usize]| row.iter().map(|&v| sizes[v].0).sum::<f32>() + GAP_X * row.len().saturating_sub(1) as f32;
    let width = all_rows.iter().map(|r| row_w(r)).fold(0.0, f32::max);
    let mut rects = vec![[0.0; 4]; n];
    let mut y = 0.0;
    for row in &all_rows {
        let mut x = (width - row_w(row)) / 2.0;
        let h = row.iter().map(|&v| sizes[v].1).fold(0.0, f32::max);
        for &v in row {
            rects[v] = [x, y, sizes[v].0, sizes[v].1];
            x += sizes[v].0 + GAP_X;
        }
        y += h + GAP_Y;
    }
    // Room under the last row for the curves between its boxes (see [`route`]).
    let curves = 24.0 + width * 0.12;
    Placed { rects, size: (width, (y - GAP_Y).max(0.0) + curves) }
}

/// A name Mermaid and PlantUML take as an identifier.
fn ident(i: usize) -> String {
    format!("C{i}")
}

/// Text safe inside a Mermaid or PlantUML label: generics as Mermaid's `~T~`, no braces
/// or quotes.
fn mermaid_text(text: &str) -> String {
    text.replace(['<', '>'], "~").replace(['{', '}', '"'], "")
}

fn plain(text: &str) -> String {
    text.replace(['{', '}', '"'], "")
}

/// Whether a graph is a plain graph of what uses what (no members, only `uses`): exported as a
/// flowchart rather than a class diagram.
pub fn is_flow(graph: &Graph) -> bool {
    graph.nodes.iter().all(|n| n.members.is_empty()) && graph.edges.iter().all(|e| e.kind == EdgeKind::Uses)
}

/// The graph as a Mermaid flowchart: one subgraph per group, colours kept.
fn to_mermaid_flow(graph: &Graph) -> String {
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut out = String::from("flowchart TD\n");
    let mut groups: Vec<&str> = graph.nodes.iter().map(|n| n.group.as_str()).collect();
    groups.sort_unstable();
    groups.dedup();
    let label = |text: &str| text.replace('"', "'");
    for (g, group) in groups.iter().enumerate() {
        let members: Vec<usize> = (0..graph.nodes.len()).filter(|&i| graph.nodes[i].group == *group).collect();
        let indent = if group.is_empty() { "  " } else { "    " };
        if !group.is_empty() {
            out.push_str(&format!("  subgraph G{g}[\"{}\"]\n", label(group)));
        }
        for i in members {
            out.push_str(&format!("{indent}{}[\"{}\"]\n", ident(i), label(&graph.nodes[i].label)));
        }
        if !group.is_empty() {
            out.push_str("  end\n");
        }
    }
    let mut styled = Vec::new();
    // Mermaid numbers the arrows as they are written.
    let mut k = 0;
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        let text = if e.label.is_empty() { String::new() } else { format!("|{}|", label(&e.label)) };
        out.push_str(&format!("  {} -->{text} {}\n", ident(a), ident(b)));
        if let Some(c) = e.accent {
            styled.push(format!("  linkStyle {k} stroke:#{c:06x}\n"));
        }
        k += 1;
    }
    for (i, n) in graph.nodes.iter().enumerate() {
        if let Some(c) = n.accent {
            out.push_str(&format!("  style {} stroke:#{c:06x}\n", ident(i)));
        }
    }
    out.extend(styled);
    out
}

/// The graph as Mermaid: a class diagram, or a flowchart for a plain graph of what uses what.
pub fn to_mermaid(graph: &Graph, members: bool) -> String {
    if is_flow(graph) {
        return to_mermaid_flow(graph);
    }
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut out = String::from("classDiagram\n");
    for (i, node) in graph.nodes.iter().enumerate() {
        out.push_str(&format!("  class {}[\"{}\"]", ident(i), mermaid_text(&node.label)));
        let lines: Vec<String> =
            if members { node.members.iter().map(|m| mermaid_text(m)).collect() } else { Vec::new() };
        let stereo = (!node.kind.is_empty() && node.kind != "class").then(|| format!("<<{}>>", node.kind));
        if lines.is_empty() && stereo.is_none() {
            out.push('\n');
            continue;
        }
        out.push_str(" {\n");
        if let Some(s) = stereo {
            out.push_str(&format!("    {s}\n"));
        }
        for m in lines {
            out.push_str(&format!("    {m}\n"));
        }
        out.push_str("  }\n");
    }
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        let (a, b) = (ident(a), ident(b));
        let line = match e.kind {
            EdgeKind::Inherits => format!("{b} <|-- {a}"),
            EdgeKind::Implements => format!("{b} <|.. {a}"),
            EdgeKind::Has => format!("{a} *-- {b}"),
            EdgeKind::Uses => format!("{a} ..> {b}"),
        };
        let label = if e.label.is_empty() { String::new() } else { format!(" : {}", mermaid_text(&e.label)) };
        out.push_str(&format!("  {line}{label}\n"));
    }
    out
}

/// The graph as PlantUML.
pub fn to_plantuml(graph: &Graph, members: bool) -> String {
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let mut out = String::from("@startuml\nhide empty members\n");
    for (i, node) in graph.nodes.iter().enumerate() {
        let (keyword, stereo) = match node.kind.as_str() {
            "interface" => ("interface", None),
            "enum" => ("enum", None),
            "abstract" => ("abstract class", None),
            "" | "class" => ("class", None),
            "trait" => ("interface", Some("trait")),
            other => ("class", Some(other)),
        };
        let stereo = stereo.map(|s| format!(" <<{s}>>")).unwrap_or_default();
        out.push_str(&format!("{keyword} \"{}\" as {}{stereo}", plain(&node.label), ident(i)));
        if members && !node.members.is_empty() {
            out.push_str(" {\n");
            for m in &node.members {
                out.push_str(&format!("  {}\n", plain(m)));
            }
            out.push_str("}\n");
        } else {
            out.push('\n');
        }
    }
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        let (a, b) = (ident(a), ident(b));
        let line = match e.kind {
            EdgeKind::Inherits => format!("{b} <|-- {a}"),
            EdgeKind::Implements => format!("{b} <|.. {a}"),
            EdgeKind::Has => format!("{a} *-- {b}"),
            EdgeKind::Uses => format!("{a} ..> {b}"),
        };
        let label = if e.label.is_empty() { String::new() } else { format!(" : {}", plain(&e.label)) };
        out.push_str(&format!("{line}{label}\n"));
    }
    out.push_str("@enduml\n");
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Where a line between two boxes leaves the first and meets the second: top and bottom
/// when one is above the other, else the sides.
pub fn attach(a: [f32; 4], b: [f32; 4]) -> ((f32, f32), (f32, f32)) {
    let (ac, bc) = ((a[0] + a[2] / 2.0, a[1] + a[3] / 2.0), (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0));
    if b[1] >= a[1] + a[3] {
        ((ac.0, a[1] + a[3]), (bc.0, b[1]))
    } else if a[1] >= b[1] + b[3] {
        ((ac.0, a[1]), (bc.0, b[1] + b[3]))
    } else if b[0] >= a[0] + a[2] {
        ((a[0] + a[2], ac.1), (b[0], bc.1))
    } else {
        ((a[0], ac.1), (b[0] + b[2], bc.1))
    }
}

/// The line between two boxes, as points: straight when one is above the other, else (two
/// boxes in one row) a curve under the row, so it doesn't run through the boxes between.
pub fn route(a: [f32; 4], b: [f32; 4]) -> Vec<(f32, f32)> {
    let apart = b[1] >= a[1] + a[3] || a[1] >= b[1] + b[3];
    if apart {
        let (p, q) = attach(a, b);
        return vec![p, q];
    }
    let p = (a[0] + a[2] / 2.0, a[1] + a[3]);
    let q = (b[0] + b[2] / 2.0, b[1] + b[3]);
    let low = p.1.max(q.1) + 24.0 + (q.0 - p.0).abs() * 0.12;
    (0..=24)
        .map(|i| {
            let t = i as f32 / 24.0;
            let u = 1.0 - t;
            // A cubic curve with both control points straight below the ends, at `low`.
            let x = u * u * u * p.0 + 3.0 * u * u * t * p.0 + 3.0 * u * t * t * q.0 + t * t * t * q.0;
            let y = u * u * u * p.1 + 3.0 * u * u * t * low + 3.0 * u * t * t * low + t * t * t * q.1;
            (x, y)
        })
        .collect()
}

/// The graph as an SVG picture, light, as laid out.
pub fn to_svg(graph: &Graph, placed: &Placed, members: bool) -> String {
    let index: HashMap<&str, usize> = graph.nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();
    let (w, h) = (placed.size.0 + 40.0, placed.size.1 + 40.0);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"-20 -20 {w:.0} {h:.0}\" \
         font-family=\"sans-serif\" font-size=\"12\">\n<rect x=\"-20\" y=\"-20\" width=\"{w:.0}\" height=\"{h:.0}\" fill=\"white\"/>\n"
    );
    out.push_str(
        "<defs><marker id=\"tri\" viewBox=\"0 0 12 12\" refX=\"11\" refY=\"6\" markerWidth=\"12\" markerHeight=\"12\" orient=\"auto\">\
         <path d=\"M0,0 L12,6 L0,12 z\" fill=\"white\" stroke=\"#555\"/></marker>\
         <marker id=\"dia\" viewBox=\"0 0 14 8\" refX=\"1\" refY=\"4\" markerWidth=\"14\" markerHeight=\"8\" orient=\"auto\">\
         <path d=\"M0,4 L7,0 L14,4 L7,8 z\" fill=\"#555\"/></marker>\
         <marker id=\"arr\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"10\" markerHeight=\"10\" orient=\"auto\">\
         <path d=\"M0,0 L10,5 L0,10\" fill=\"none\" stroke=\"#555\"/></marker></defs>\n",
    );
    for e in &graph.edges {
        let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) else { continue };
        let points = route(placed.rects[a], placed.rects[b]);
        let (dash, marker) = match e.kind {
            EdgeKind::Inherits => ("", "marker-end=\"url(#tri)\""),
            EdgeKind::Implements => (" stroke-dasharray=\"6 4\"", "marker-end=\"url(#tri)\""),
            EdgeKind::Has => ("", "marker-start=\"url(#dia)\""),
            EdgeKind::Uses => (" stroke-dasharray=\"6 4\"", "marker-end=\"url(#arr)\""),
        };
        let points: Vec<String> = points.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect();
        out.push_str(&format!(
            "<polyline points=\"{}\" fill=\"none\" stroke=\"#555\"{dash} {marker}/>\n",
            points.join(" ")
        ));
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        let [x, y, bw, bh] = placed.rects[i];
        let external = node.kind == "external";
        let fill = if external { "#f2f2f2" } else { "#fffbe8" };
        out.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bw:.1}\" height=\"{bh:.1}\" rx=\"4\" fill=\"{fill}\" stroke=\"#555\"/>\n"
        ));
        let cx = x + bw / 2.0;
        let mut ty = y + PAD + ROW_H * 0.75;
        if let Some(s) = stereotype(&node.kind) {
            out.push_str(&format!(
                "<text x=\"{cx:.1}\" y=\"{ty:.1}\" text-anchor=\"middle\" font-style=\"italic\" fill=\"#555\">{}</text>\n",
                escape(&s)
            ));
            ty += ROW_H;
        }
        out.push_str(&format!(
            "<text x=\"{cx:.1}\" y=\"{ty:.1}\" text-anchor=\"middle\" font-weight=\"bold\">{}</text>\n",
            escape(&node.label)
        ));
        let shown = if members { shown_members(node) } else { Vec::new() };
        if !shown.is_empty() {
            let sep = ty + PAD * 0.75;
            out.push_str(&format!(
                "<line x1=\"{x:.1}\" y1=\"{sep:.1}\" x2=\"{:.1}\" y2=\"{sep:.1}\" stroke=\"#555\"/>\n",
                x + bw
            ));
            let mut my = sep + ROW_H * 0.9;
            for m in shown {
                out.push_str(&format!(
                    "<text x=\"{:.1}\" y=\"{my:.1}\" font-family=\"monospace\" font-size=\"11\">{}</text>\n",
                    x + PAD,
                    escape(&m)
                ));
                my += ROW_H;
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::plugin::host::GraphEdge;

    fn node(id: &str, kind: &str, members: &[&str]) -> GraphNode {
        GraphNode {
            id: id.into(),
            label: id.into(),
            kind: kind.into(),
            group: String::new(),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
            path: String::new(),
            line: 1,
            accent: None,
        }
    }

    fn edge(from: &str, to: &str, kind: EdgeKind) -> GraphEdge {
        GraphEdge { source: from.into(), target: to.into(), kind, label: String::new(), accent: None }
    }

    fn sample() -> Graph {
        Graph {
            id: "g".into(),
            title: "t".into(),
            nodes: vec![
                node("Dog", "class", &["+ name: String", "+ bark()"]),
                node("Animal", "abstract", &[]),
                node("Pet", "interface", &[]),
                node("Owner", "class", &["- dog: Dog"]),
                node("Loner", "class", &[]),
            ],
            edges: vec![
                edge("Dog", "Animal", EdgeKind::Inherits),
                edge("Dog", "Pet", EdgeKind::Implements),
                edge("Owner", "Dog", EdgeKind::Has),
            ],
            focus: None,
        }
    }

    #[test]
    fn parents_sit_above_children() {
        let g = sample();
        let p = layout(&g, true);
        let y = |i: usize| p.rects[i][1];
        assert!(y(1) < y(0) && y(2) < y(0), "{:?}", p.rects);
        // Not in a hierarchy: below everything.
        assert!(y(3) > y(0) && y(4) > y(0));
        // No two boxes overlap.
        for a in 0..5 {
            for b in a + 1..5 {
                let (r, s) = (p.rects[a], p.rects[b]);
                let apart = r[0] + r[2] <= s[0] || s[0] + s[2] <= r[0] || r[1] + r[3] <= s[1] || s[1] + s[3] <= r[1];
                assert!(apart, "{a} and {b} overlap");
            }
        }
        // A cycle doesn't hang the layout.
        let mut c = sample();
        c.edges.push(edge("Animal", "Dog", EdgeKind::Inherits));
        assert_eq!(layout(&c, false).rects.len(), 5);
        assert!(layout(&Graph { nodes: vec![], ..sample() }, true).rects.is_empty());
    }

    #[test]
    fn exports() {
        let g = sample();
        let m = to_mermaid(&g, true);
        assert!(m.starts_with("classDiagram\n"));
        assert!(m.contains("class C0[\"Dog\"] {\n    + name: String\n    + bark()\n  }"), "{m}");
        assert!(m.contains("<<interface>>") && m.contains("<<abstract>>"));
        assert!(m.contains("C1 <|-- C0") && m.contains("C2 <|.. C0") && m.contains("C3 *-- C0"), "{m}");
        let p = to_plantuml(&g, false);
        assert!(p.contains("interface \"Pet\" as C2\n") && p.contains("abstract class \"Animal\" as C1\n"), "{p}");
        assert!(p.contains("C1 <|-- C0") && p.trim_end().ends_with("@enduml"));
        let svg = to_svg(&g, &layout(&g, true), true);
        assert!(svg.contains(">Dog</text>") && svg.contains("«interface»") && svg.contains("+ bark()"));
        assert_eq!(svg.matches("<polyline").count(), 3, "three edges");
        assert_eq!(svg.matches("<line").count(), 2, "two member separators");
        // Boxes side by side: the line curves under them.
        let side = route([0.0, 0.0, 50.0, 30.0], [300.0, 0.0, 50.0, 30.0]);
        assert!(side.len() > 2 && side.iter().all(|p| p.1 >= 30.0) && side.iter().any(|p| p.1 > 60.0));
        assert_eq!(route([0.0, 0.0, 50.0, 30.0], [0.0, 100.0, 50.0, 30.0]), [(25.0, 30.0), (25.0, 100.0)]);
        let generic = node("List<T>", "class", &["+ get(i: int) {x}"]);
        assert!(to_mermaid(&Graph { nodes: vec![generic], edges: vec![], ..sample() }, true).contains("List~T~"));
    }

    #[test]
    fn graphs_of_what_uses_what() {
        // main uses lib, lib uses util: top to bottom.
        let mut g = Graph {
            nodes: vec![node("util", "", &[]), node("main", "", &[]), node("lib", "", &[])],
            edges: vec![edge("main", "lib", EdgeKind::Uses), edge("lib", "util", EdgeKind::Uses)],
            ..sample()
        };
        g.nodes[1].group = "src".into();
        g.nodes[2].group = "src".into();
        g.nodes[0].accent = Some(0xE5484D);
        g.edges[1].accent = Some(0x57C06A);
        let p = layout(&g, true);
        let y = |i: usize| p.rects[i][1];
        assert!(y(1) < y(2) && y(2) < y(0), "{:?}", p.rects);
        assert!(is_flow(&g) && !is_flow(&sample()));
        let m = to_mermaid(&g, true);
        assert!(m.starts_with("flowchart TD\n"), "{m}");
        assert!(m.contains("  subgraph G1[\"src\"]\n    C1[\"main\"]\n    C2[\"lib\"]\n  end\n"), "{m}");
        assert!(m.contains("  C1 --> C2\n  C2 --> C0\n"), "{m}");
        assert!(m.contains("  style C0 stroke:#e5484d\n") && m.contains("  linkStyle 1 stroke:#57c06a\n"), "{m}");
    }
}
