//! Local subgraph, tree, layout, Graphviz output, and the canvas image.

use std::collections::{BTreeSet, HashMap, VecDeque};

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::index::{FileId, Index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Root,
    /// The parent links to this node.
    Out,
    /// This node links to the parent.
    In,
    Both,
}

impl Mark {
    pub fn arrow(self) -> &'static str {
        match self {
            Mark::Root => "",
            Mark::Out => "->",
            Mark::In => "<-",
            Mark::Both => "<->",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: FileId,
    pub hop: u32,
    pub parent: Option<FileId>,
    pub mark: Mark,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    /// Breadth-first order; the root first.
    pub nodes: Vec<Node>,
    /// Forward links between included nodes, each once, sorted by path.
    pub edges: Vec<(FileId, FileId)>,
    pub left_out: usize,
}

/// Files `id` links to (resolved, or the pick of an ambiguous link), not
/// itself.
fn outgoing(index: &Index, id: FileId) -> BTreeSet<FileId> {
    index.forward[id]
        .iter()
        .filter_map(|r| r.target())
        .filter(|&t| t != id)
        .collect()
}

fn incoming(index: &Index, id: FileId) -> BTreeSet<FileId> {
    index.back[id]
        .iter()
        .map(|&(s, _)| s)
        .filter(|&s| s != id)
        .collect()
}

/// The notes within `hops` links of `root`, following links both ways.
pub fn local(index: &Index, root: FileId, hops: u32, cap: Option<usize>) -> Local {
    let rel = |id: FileId| index.files[id].rel.as_str();
    let mut nodes = vec![Node {
        id: root,
        hop: 0,
        parent: None,
        mark: Mark::Root,
    }];
    let mut seen: HashMap<FileId, usize> = HashMap::from([(root, 0)]);
    let mut queue = VecDeque::from([root]);
    while let Some(cur) = queue.pop_front() {
        let hop = nodes[seen[&cur]].hop;
        if hop >= hops {
            continue;
        }
        let out = outgoing(index, cur);
        let inn = incoming(index, cur);
        let mut next: Vec<FileId> = out.union(&inn).copied().collect();
        next.sort_by(|a, b| rel(*a).cmp(rel(*b)));
        for n in next {
            if seen.contains_key(&n) {
                continue;
            }
            let mark = match (out.contains(&n), inn.contains(&n)) {
                (true, true) => Mark::Both,
                (true, false) => Mark::Out,
                _ => Mark::In,
            };
            seen.insert(n, nodes.len());
            nodes.push(Node {
                id: n,
                hop: hop + 1,
                parent: Some(cur),
                mark,
            });
            queue.push_back(n);
        }
    }

    let mut left_out = 0;
    if let Some(cap) = cap {
        if nodes.len() > cap {
            // The farthest hop goes first, and within it the nodes with the
            // fewest links; the root stays.
            let degree = |id: FileId| index.forward[id].len() + index.back[id].len();
            let mut order: Vec<usize> = (1..nodes.len()).collect();
            order.sort_by(|&a, &b| {
                nodes[b]
                    .hop
                    .cmp(&nodes[a].hop)
                    .then(degree(nodes[a].id).cmp(&degree(nodes[b].id)))
                    .then(rel(nodes[b].id).cmp(rel(nodes[a].id)))
            });
            let drop: BTreeSet<usize> = order.into_iter().take(nodes.len() - cap).collect();
            left_out = drop.len();
            nodes = nodes
                .into_iter()
                .enumerate()
                .filter(|(i, _)| !drop.contains(i))
                .map(|(_, n)| n)
                .collect();
        }
    }

    let included: BTreeSet<FileId> = nodes.iter().map(|n| n.id).collect();
    let mut edges: Vec<(FileId, FileId)> = nodes
        .iter()
        .flat_map(|n| {
            outgoing(index, n.id)
                .into_iter()
                .filter(|t| included.contains(t))
                .map(move |t| (n.id, t))
        })
        .collect();
    edges.sort_by(|a, b| (rel(a.0), rel(a.1)).cmp(&(rel(b.0), rel(b.1))));
    edges.dedup();
    Local {
        nodes,
        edges,
        left_out,
    }
}

/// The indented tree: the root's path, then each node under the node that
/// first reached it, two spaces per hop, with its mark.
pub fn tree(index: &Index, local: &Local) -> Vec<String> {
    let rel = |id: FileId| index.files[id].rel.clone();
    let mut lines = Vec::new();
    fn walk(
        local: &Local,
        parent: FileId,
        lines: &mut Vec<String>,
        rel: &dyn Fn(FileId) -> String,
    ) {
        let mut kids: Vec<&Node> = local
            .nodes
            .iter()
            .filter(|n| n.parent == Some(parent))
            .collect();
        kids.sort_by_key(|n| rel(n.id));
        for n in kids {
            lines.push(format!(
                "{}{} {}",
                "  ".repeat(n.hop as usize),
                n.mark.arrow(),
                rel(n.id)
            ));
            walk(local, n.id, lines, rel);
        }
    }
    let Some(root) = local.nodes.first() else {
        return lines;
    };
    lines.push(rel(root.id));
    walk(local, root.id, &mut lines, &rel);
    if local.left_out > 0 {
        lines.push(format!("({} more not shown)", local.left_out));
    }
    lines
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Graphviz: nodes sorted by path, then edges.
pub fn dot(index: &Index, local: &Local) -> String {
    let mut names: Vec<&str> = local
        .nodes
        .iter()
        .map(|n| index.files[n.id].rel.as_str())
        .collect();
    names.sort();
    let mut out = String::from("digraph knapp {\n");
    for name in names {
        out.push_str(&format!("  {};\n", quote(name)));
    }
    for &(a, b) in &local.edges {
        out.push_str(&format!(
            "  {} -> {};\n",
            quote(&index.files[a].rel),
            quote(&index.files[b].rel)
        ));
    }
    out.push_str("}\n");
    out
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// A cell for each node, in `local.nodes` order. Fruchterman-Reingold with
/// the root pinned at the centre and starting positions from a hash of each
/// path, so the same graph always lays out the same way. Rows count twice,
/// since a cell is about twice as tall as it is wide.
pub fn layout(index: &Index, local: &Local, cols: u16, rows: u16) -> Vec<(u16, u16)> {
    let n = local.nodes.len();
    if n == 0 || cols < 3 || rows < 3 {
        return vec![(0, 0); n];
    }
    let (w, h) = (f64::from(cols), f64::from(rows) * 2.0);
    let k = (w * h / n as f64).sqrt();
    let pos_of: HashMap<FileId, usize> = local
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    let mut pos: Vec<(f64, f64)> = local
        .nodes
        .iter()
        .map(|node| {
            let hash = fnv(&index.files[node.id].rel);
            let x = (hash & 0xffff) as f64 / 65535.0;
            let y = ((hash >> 16) & 0xffff) as f64 / 65535.0;
            (1.0 + x * (w - 2.0), 1.0 + y * (h - 2.0))
        })
        .collect();
    pos[0] = (w / 2.0, h / 2.0);
    let edges: Vec<(usize, usize)> = local
        .edges
        .iter()
        .filter_map(|(a, b)| Some((*pos_of.get(a)?, *pos_of.get(b)?)))
        .collect();
    let iterations = 300;
    for step in 0..iterations {
        let temp = (w / 10.0) * (1.0 - step as f64 / iterations as f64) + 0.01;
        let mut disp = vec![(0.0f64, 0.0f64); n];
        for i in 0..n {
            for j in (i + 1)..n {
                let (dx, dy) = (pos[i].0 - pos[j].0, pos[i].1 - pos[j].1);
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = k * k / d;
                disp[i].0 += dx / d * f;
                disp[i].1 += dy / d * f;
                disp[j].0 -= dx / d * f;
                disp[j].1 -= dy / d * f;
            }
        }
        for &(a, b) in &edges {
            let (dx, dy) = (pos[a].0 - pos[b].0, pos[a].1 - pos[b].1);
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            let f = d * d / k;
            disp[a].0 -= dx / d * f;
            disp[a].1 -= dy / d * f;
            disp[b].0 += dx / d * f;
            disp[b].1 += dy / d * f;
        }
        for i in 1..n {
            let (dx, dy) = disp[i];
            let d = (dx * dx + dy * dy).sqrt().max(0.01);
            pos[i].0 = (pos[i].0 + dx / d * d.min(temp)).clamp(1.0, w - 2.0);
            pos[i].1 = (pos[i].1 + dy / d * d.min(temp)).clamp(1.0, h - 2.0);
        }
    }

    // Round to cells; a taken cell sends the node to the nearest free one.
    let mut taken: BTreeSet<(u16, u16)> = BTreeSet::new();
    pos.iter()
        .map(|&(x, y)| {
            let want = (x.round() as i32, (y / 2.0).round() as i32);
            let cell = (0..(i32::from(cols.max(rows))))
                .flat_map(|r| {
                    (-r..=r).flat_map(move |dx| (-r..=r).map(move |dy| (want.0 + dx, want.1 + dy)))
                })
                .map(|(cx, cy)| {
                    (
                        cx.clamp(0, i32::from(cols) - 1) as u16,
                        cy.clamp(0, i32::from(rows) - 1) as u16,
                    )
                })
                .find(|c| !taken.contains(c))
                .unwrap_or((0, 0));
            taken.insert(cell);
            cell
        })
        .collect()
}

/// What the canvas image is drawn from, in cells: no index needed, so the
/// event loop can draw it once it knows the cell size in pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct CanvasSpec {
    pub cols: u16,
    pub rows: u16,
    /// Cell, radius as a fraction of the cell's smaller side, and whether it
    /// is the root.
    pub dots: Vec<((u16, u16), f32, bool)>,
    /// Indices into `dots`.
    pub edges: Vec<(usize, usize)>,
}

pub fn spec(
    index: &Index,
    local: &Local,
    cells: &[(u16, u16)],
    cols: u16,
    rows: u16,
) -> CanvasSpec {
    let at: HashMap<FileId, usize> = local
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    CanvasSpec {
        cols,
        rows,
        dots: local
            .nodes
            .iter()
            .zip(cells)
            .map(|(n, &c)| {
                let inbound = index.back[n.id].len() as f32;
                let radius = (0.25 + 0.06 * (inbound + 1.0).ln()).min(0.45);
                (c, radius, n.mark == Mark::Root)
            })
            .collect(),
        edges: local
            .edges
            .iter()
            .filter_map(|(a, b)| Some((*at.get(a)?, *at.get(b)?)))
            .collect(),
    }
}

/// Rows `band` of the canvas as a PNG: transparent, edges as lines, a dot
/// at each node's cell centre. `cell_px` is the terminal cell size in
/// pixels.
pub fn canvas(
    spec: &CanvasSpec,
    cell_px: (u32, u32),
    band: std::ops::Range<u16>,
) -> Result<Vec<u8>, String> {
    let (cw, ch) = (cell_px.0.max(1), cell_px.1.max(1));
    let rows = band.end.saturating_sub(band.start);
    let mut pm = Pixmap::new(u32::from(spec.cols) * cw, u32::from(rows) * ch)
        .ok_or("canvas: empty or too large")?;
    let top = f32::from(band.start) * ch as f32;
    let centre = |(x, y): (u16, u16)| {
        (
            (f32::from(x) + 0.5) * cw as f32,
            (f32::from(y) + 0.5) * ch as f32 - top,
        )
    };
    let mut line = Paint {
        anti_alias: true,
        ..Paint::default()
    };
    line.set_color(Color::from_rgba8(128, 128, 128, 200));
    for &(a, b) in &spec.edges {
        let (p, q) = (centre(spec.dots[a].0), centre(spec.dots[b].0));
        let mut pb = PathBuilder::new();
        pb.move_to(p.0, p.1);
        pb.line_to(q.0, q.1);
        if let Some(path) = pb.finish() {
            pm.stroke_path(
                &path,
                &line,
                &Stroke {
                    width: 1.5,
                    ..Stroke::default()
                },
                Transform::identity(),
                None,
            );
        }
    }
    let side = cw.min(ch) as f32;
    for &(cell, radius, root) in &spec.dots {
        let (x, y) = centre(cell);
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(if root {
            Color::from_rgba8(0, 175, 215, 255)
        } else {
            Color::from_rgba8(160, 160, 160, 255)
        });
        if let Some(dot) = PathBuilder::from_circle(x, y, (radius * side).max(2.0)) {
            pm.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    pm.encode_png().map_err(|e| format!("canvas: {e}"))
}

/// Rows `band` of an image shown `rows` cells tall, as a PNG: the matching
/// slice of its pixel rows.
pub fn crop_rows(pm: &Pixmap, rows: u16, band: std::ops::Range<u16>) -> Result<Vec<u8>, String> {
    let rows = u32::from(rows.max(1));
    let y0 = pm.height() * u32::from(band.start) / rows;
    let y1 = (pm.height() * u32::from(band.end) / rows)
        .max(y0 + 1)
        .min(pm.height());
    let rect = tiny_skia::IntRect::from_xywh(0, y0 as i32, pm.width(), y1.saturating_sub(y0))
        .ok_or("image band out of range")?;
    pm.clone_rect(rect)
        .ok_or("image band out of range")?
        .encode_png()
        .map_err(|e| e.to_string())
}
