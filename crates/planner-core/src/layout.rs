//! Layered graph auto-layout (Sugiyama-lite) for factory graphs. Flow reads
//! left→right: In ports in column 0, machine groups/junctions ranked by their
//! depth in the flow, Out ports in the last column. Within each column, rows
//! are ordered by neighbor barycenter (a few alternating sweeps) so edges
//! cross as little as possible. Deterministic: same graph → same positions.

use std::collections::BTreeMap;

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

fn height(kind: LKind) -> f64 {
    match kind {
        LKind::Group => 220.0,
        LKind::Junction => 120.0,
        LKind::InPort | LKind::OutPort => 96.0,
    }
}

/// Compute positions for every node. `edges` are (from, to) node-id pairs;
/// edges referencing unknown ids are ignored.
pub fn layered_layout(nodes: &[LNode], edges: &[(Id, Id)]) -> BTreeMap<Id, GraphPos> {
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
}
