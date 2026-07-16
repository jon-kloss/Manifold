//! Layered graph auto-layout (Sugiyama-lite) for factory graphs. Flow reads
//! left→right: In ports in column 0, machine groups/junctions ranked by their
//! depth in the flow, Out ports in the last column. Within each column, rows
//! are ordered by neighbor barycenter (a few alternating sweeps) so edges
//! cross as little as possible. Deterministic: same graph → same positions.
//!
//! Independent production chains (a factory usually has several — an iron line,
//! a copper line, a concrete line…) are laid out as SEPARATE HORIZONTAL BANDS:
//! the graph is split into weakly-connected components, each is laid out on its
//! own, and the bands stack top-to-bottom. Two chains that share no material can
//! never interleave or cross, so the result reads as clean lanes instead of one
//! tangled column stack.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::entities::{GraphPos, Id};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LKind {
    InPort,
    OutPort,
    Group,
    Junction,
}

#[derive(Debug, Clone)]
pub struct LNode {
    pub id: Id,
    pub kind: LKind,
}

const COL_X0: f64 = 40.0;
const COL_W: f64 = 340.0;
const ROW_Y0: f64 = 60.0;
const ROW_GAP: f64 = 36.0;
/// Vertical gap between independent chains' bands — deliberately larger than
/// `ROW_GAP` so separate production lines read as distinct lanes.
const BAND_GAP: f64 = 120.0;

fn height(kind: LKind) -> f64 {
    match kind {
        LKind::Group => 220.0,
        LKind::Junction => 120.0,
        // The round resource token is a 96px disc plus a caption below it
        // (item name + source); reserve the extra so stacked ports in a column
        // don't have their captions collide with the next disc.
        LKind::InPort | LKind::OutPort => 120.0,
    }
}

/// Weakly-connected components (edges treated as undirected), each a `Vec` of
/// node indices. Components are ordered by their smallest member index so the
/// banding is deterministic and stable across re-tidies. Isolated nodes (an
/// unwired group) each form their own component.
fn weak_components(nodes: &[LNode], edges: &[(Id, Id)]) -> Vec<Vec<usize>> {
    let index: BTreeMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let n = nodes.len();
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in edges {
        if let (Some(&a), Some(&b)) = (index.get(from.as_str()), index.get(to.as_str())) {
            adj[a].push(b);
            adj[b].push(a);
        }
    }
    let mut seen = vec![false; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if seen[start] {
            continue;
        }
        let mut comp = Vec::new();
        let mut q = VecDeque::from([start]);
        seen[start] = true;
        while let Some(i) = q.pop_front() {
            comp.push(i);
            for &j in &adj[i] {
                if !seen[j] {
                    seen[j] = true;
                    q.push_back(j);
                }
            }
        }
        comp.sort_unstable();
        comps.push(comp);
    }
    comps
}

/// Compute positions for every node. `edges` are (from, to) node-id pairs;
/// edges referencing unknown ids are ignored. Independent chains are stacked as
/// separate horizontal bands (see the module docs); each band is laid out by
/// [`layout_component`].
pub fn layered_layout(nodes: &[LNode], edges: &[(Id, Id)]) -> BTreeMap<Id, GraphPos> {
    let mut positions = BTreeMap::new();
    let mut band_top = ROW_Y0;
    for comp in weak_components(nodes, edges) {
        let sub_nodes: Vec<LNode> = comp.iter().map(|&i| nodes[i].clone()).collect();
        let ids: BTreeSet<&str> = sub_nodes.iter().map(|n| n.id.as_str()).collect();
        let sub_edges: Vec<(Id, Id)> = edges
            .iter()
            .filter(|(a, b)| ids.contains(a.as_str()) && ids.contains(b.as_str()))
            .cloned()
            .collect();
        let local = layout_component(&sub_nodes, &sub_edges);
        // Shift this component so its highest node sits at `band_top`, then
        // advance the cursor past its lowest node plus the band gap.
        let top = local.values().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let dy = band_top - top;
        let mut band_bottom = band_top;
        for nd in &sub_nodes {
            let p = &local[&nd.id];
            let y = p.y + dy;
            band_bottom = band_bottom.max(y + height(nd.kind));
            positions.insert(nd.id.clone(), GraphPos { x: p.x, y });
        }
        band_top = band_bottom + BAND_GAP;
    }
    positions
}

/// Lay out ONE connected component (the original Sugiyama-lite pass). y starts
/// at `ROW_Y0`; the caller re-bands it vertically.
fn layout_component(nodes: &[LNode], edges: &[(Id, Id)]) -> BTreeMap<Id, GraphPos> {
    let index: BTreeMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let n = nodes.len();
    let mut out_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut in_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (from, to) in edges {
        if let (Some(&a), Some(&b)) = (index.get(from.as_str()), index.get(to.as_str())) {
            out_adj[a].push(b);
            in_adj[b].push(a);
        }
    }

    // ---- ranking: longest path from the sources (Kahn), cycle-tolerant ----
    let mut rank: Vec<i64> = vec![-1; n];
    let mut indeg: Vec<usize> = in_adj.iter().map(|v| v.len()).collect();
    let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    for &i in &queue {
        rank[i] = if nodes[i].kind == LKind::InPort { 0 } else { 1 };
    }
    let mut head = 0;
    while head < queue.len() {
        let i = queue[head];
        head += 1;
        for &j in &out_adj[i] {
            rank[j] = rank[j].max(rank[i] + 1);
            indeg[j] -= 1;
            if indeg[j] == 0 {
                queue.push(j);
            }
        }
    }
    // nodes trapped in cycles: one relaxation pass off any ranked predecessor
    for i in 0..n {
        if rank[i] < 0 {
            let base = in_adj[i].iter().map(|&p| rank[p]).max().unwrap_or(0);
            rank[i] = base.max(0) + 1;
        }
    }
    // groups/junctions never sit in the port columns
    for i in 0..n {
        if nodes[i].kind != LKind::InPort && rank[i] == 0 {
            rank[i] = 1;
        }
    }
    let interior_max = (0..n)
        .filter(|&i| !matches!(nodes[i].kind, LKind::OutPort))
        .map(|i| rank[i])
        .max()
        .unwrap_or(0);
    for i in 0..n {
        if nodes[i].kind == LKind::OutPort {
            rank[i] = interior_max + 1;
        }
    }

    // ---- columns, then barycenter ordering sweeps ----
    let max_rank = rank.iter().copied().max().unwrap_or(0);
    let mut cols: Vec<Vec<usize>> = vec![Vec::new(); (max_rank + 1) as usize];
    for i in 0..n {
        cols[rank[i] as usize].push(i);
    }
    let mut row: Vec<f64> = vec![0.0; n];
    let renumber = |cols: &[Vec<usize>], row: &mut [f64]| {
        for col in cols {
            for (r, &i) in col.iter().enumerate() {
                row[i] = r as f64;
            }
        }
    };
    renumber(&cols, &mut row);
    for sweep in 0..4 {
        let downward = sweep % 2 == 0;
        let order: Vec<usize> = if downward {
            (0..cols.len()).collect()
        } else {
            (0..cols.len()).rev().collect()
        };
        for c in order {
            let mut keyed: Vec<(f64, usize)> = cols[c]
                .iter()
                .map(|&i| {
                    let nbrs = if downward { &in_adj[i] } else { &out_adj[i] };
                    let bc = if nbrs.is_empty() {
                        row[i]
                    } else {
                        nbrs.iter().map(|&p| row[p]).sum::<f64>() / nbrs.len() as f64
                    };
                    (bc, i)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            cols[c] = keyed.into_iter().map(|(_, i)| i).collect();
            renumber(&cols, &mut row);
        }
    }

    // ---- coordinates: columns left→right, rows stacked by real heights ----
    let mut positions = BTreeMap::new();
    for (c, col) in cols.iter().enumerate() {
        let mut y = ROW_Y0;
        for &i in col {
            positions.insert(
                nodes[i].id.clone(),
                GraphPos {
                    x: COL_X0 + c as f64 * COL_W,
                    y,
                },
            );
            y += height(nodes[i].kind) + ROW_GAP;
        }
    }
    positions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, kind: LKind) -> LNode {
        LNode {
            id: id.into(),
            kind,
        }
    }

    #[test]
    fn ranks_flow_left_to_right_and_ports_bookend() {
        // ore(In) → smelt → construct → rods(Out); a surplus Out off smelt
        let nodes = vec![
            node("ore", LKind::InPort),
            node("smelt", LKind::Group),
            node("construct", LKind::Group),
            node("rods", LKind::OutPort),
            node("surplus", LKind::OutPort),
        ];
        let edges = vec![
            ("ore".into(), "smelt".into()),
            ("smelt".into(), "construct".into()),
            ("construct".into(), "rods".into()),
            ("smelt".into(), "surplus".into()),
        ];
        let pos = layered_layout(&nodes, &edges);
        assert!(pos["ore"].x < pos["smelt"].x);
        assert!(pos["smelt"].x < pos["construct"].x);
        assert!(pos["construct"].x < pos["rods"].x);
        // both Out ports share the last column even though surplus hangs off
        // an earlier rank
        assert_eq!(pos["rods"].x, pos["surplus"].x);
    }

    #[test]
    fn cycles_and_islands_do_not_panic_or_overlap() {
        let nodes = vec![
            node("a", LKind::Group),
            node("b", LKind::Group),
            node("island", LKind::Group),
        ];
        let edges = vec![("a".into(), "b".into()), ("b".into(), "a".into())];
        let pos = layered_layout(&nodes, &edges);
        assert_eq!(pos.len(), 3);
        let mut seen = std::collections::BTreeSet::new();
        for p in pos.values() {
            assert!(seen.insert(format!("{:.0}:{:.0}", p.x, p.y)), "overlap");
        }
    }

    #[test]
    fn barycenter_keeps_paired_chains_uncrossed() {
        // two parallel chains: a1→b1, a2→b2 — order must be preserved per column
        let nodes = vec![
            node("a1", LKind::Group),
            node("a2", LKind::Group),
            node("b1", LKind::Group),
            node("b2", LKind::Group),
        ];
        let edges = vec![("a1".into(), "b1".into()), ("a2".into(), "b2".into())];
        let pos = layered_layout(&nodes, &edges);
        assert_eq!(
            pos["a1"].y < pos["a2"].y,
            pos["b1"].y < pos["b2"].y,
            "chains must not cross"
        );
    }

    #[test]
    fn independent_chains_get_separate_non_overlapping_bands() {
        // Two chains that share no material — an iron line and a copper line —
        // must occupy disjoint vertical bands (no interleaving), each reading
        // left→right, so the graph is clean lanes rather than one tangled stack.
        let nodes = vec![
            node("ore_fe", LKind::InPort),
            node("smelt_fe", LKind::Group),
            node("plate", LKind::OutPort),
            node("ore_cu", LKind::InPort),
            node("smelt_cu", LKind::Group),
            node("wire", LKind::OutPort),
        ];
        let edges = vec![
            ("ore_fe".into(), "smelt_fe".into()),
            ("smelt_fe".into(), "plate".into()),
            ("ore_cu".into(), "smelt_cu".into()),
            ("smelt_cu".into(), "wire".into()),
        ];
        let pos = layered_layout(&nodes, &edges);
        // Each chain reads left→right.
        for chain in [
            ["ore_fe", "smelt_fe", "plate"],
            ["ore_cu", "smelt_cu", "wire"],
        ] {
            assert!(pos[chain[0]].x < pos[chain[1]].x && pos[chain[1]].x < pos[chain[2]].x);
        }
        // The two bands do not overlap vertically: the lower chain's TOP sits
        // below the upper chain's BOTTOM (band 1 is the iron line, index-first).
        let fe_bottom = [pos["ore_fe"].y, pos["smelt_fe"].y, pos["plate"].y]
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max)
            + height(LKind::Group);
        let cu_top = [pos["ore_cu"].y, pos["smelt_cu"].y, pos["wire"].y]
            .iter()
            .cloned()
            .fold(f64::INFINITY, f64::min);
        assert!(
            cu_top > fe_bottom,
            "the copper band ({cu_top}) must start below the iron band bottom ({fe_bottom})"
        );
    }
}
