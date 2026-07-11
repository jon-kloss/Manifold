//! T0 — ratio propagation (SDD §5.1). Fixed structure, fixed recipes: a
//! topological walk scaling counts/clocks/rates linearly from the changed
//! target. Pure function; compiled to WASM for the renderer's drag frames.
//!
//! Linearity is the load-bearing property: with distribution fractions frozen
//! at snapshot weights, every flow is affine in the target (f = f₀ + T·f₁),
//! which makes the hard-stop ceiling exact: T_max = min (cap − f₀)/f₁.

use std::collections::BTreeMap;

use crate::model::*;

/// items/min a group produces of `item` at snapshot count/clock.
fn group_weight(g: &GroupSpec, item: &str) -> f64 {
    g.count as f64 * g.clock * g.recipe.out_rate(item)
}

struct Graph<'a> {
    snapshot: &'a FactorySnapshot,
    /// edge indices leaving / entering each node
    out_edges: BTreeMap<NodeRef, Vec<usize>>,
    in_edges: BTreeMap<NodeRef, Vec<usize>>,
    nodes: Vec<NodeRef>,
}

impl<'a> Graph<'a> {
    fn build(s: &'a FactorySnapshot) -> Self {
        let mut out_edges: BTreeMap<NodeRef, Vec<usize>> = BTreeMap::new();
        let mut in_edges: BTreeMap<NodeRef, Vec<usize>> = BTreeMap::new();
        let mut nodes: Vec<NodeRef> = Vec::new();
        for g in &s.groups {
            nodes.push(NodeRef::Group(g.id.clone()));
        }
        for p in &s.inputs {
            nodes.push(NodeRef::Input(p.id.clone()));
        }
        for p in &s.outputs {
            nodes.push(NodeRef::Output(p.id.clone()));
        }
        for j in &s.junctions {
            nodes.push(NodeRef::Junction(j.clone()));
        }
        for n in &nodes {
            out_edges.entry(n.clone()).or_default();
            in_edges.entry(n.clone()).or_default();
        }
        for (i, e) in s.edges.iter().enumerate() {
            out_edges.entry(e.from.clone()).or_default().push(i);
            in_edges.entry(e.to.clone()).or_default().push(i);
        }
        Self {
            snapshot: s,
            out_edges,
            in_edges,
            nodes,
        }
    }

    /// Reverse-topological order (consumers before producers). Err on cycle.
    fn reverse_topo(&self) -> Result<Vec<NodeRef>, SolveError> {
        let mut order = Vec::new();
        let mut mark: BTreeMap<NodeRef, u8> = BTreeMap::new(); // 0 unseen, 1 visiting, 2 done
        fn visit(
            n: &NodeRef,
            g: &Graph,
            mark: &mut BTreeMap<NodeRef, u8>,
            order: &mut Vec<NodeRef>,
        ) -> Result<(), SolveError> {
            match mark.get(n).copied().unwrap_or(0) {
                2 => return Ok(()),
                1 => return Err(SolveError::Cyclic),
                _ => {}
            }
            mark.insert(n.clone(), 1);
            for &ei in g.out_edges.get(n).map(|v| v.as_slice()).unwrap_or(&[]) {
                let to = &g.snapshot.edges[ei].to;
                visit(to, g, mark, order)?;
            }
            mark.insert(n.clone(), 2);
            order.push(n.clone()); // post-order = consumers first when iterated as-is
            Ok(())
        }
        for n in &self.nodes {
            visit(n, self, &mut mark, &mut order)?;
        }
        Ok(order)
    }
}

/// One linear demand pass: given target rates per output port, produce edge
/// flows and per-group cycles/min. Distribution fractions use snapshot weights.
fn demand_pass(
    graph: &Graph,
    order: &[NodeRef],
    targets: &BTreeMap<String, f64>,
) -> Result<(Vec<f64>, BTreeMap<String, f64>), SolveError> {
    let s = graph.snapshot;
    let mut edge_flow = vec![0.0f64; s.edges.len()];
    let mut group_cycles: BTreeMap<String, f64> = BTreeMap::new();

    // Distribute `demand` of `item` entering `node` across its incoming edges
    // carrying that item, proportional to source weight.
    let pull = |node: &NodeRef, item: &str, demand: f64, edge_flow: &mut Vec<f64>| {
        if demand <= 0.0 {
            return;
        }
        let incoming: Vec<usize> = graph
            .in_edges
            .get(node)
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|&ei| s.edges[ei].item == item)
                    .collect()
            })
            .unwrap_or_default();
        if incoming.is_empty() {
            return; // starved — shows up as deficit on the group's in_rates
        }
        let weights: Vec<f64> = incoming
            .iter()
            .map(|&ei| match &s.edges[ei].from {
                NodeRef::Group(gid) => s
                    .groups
                    .iter()
                    .find(|g| &g.id == gid)
                    .map(|g| group_weight(g, item))
                    .unwrap_or(0.0),
                NodeRef::Input(pid) => s
                    .inputs
                    .iter()
                    .find(|p| &p.id == pid)
                    .and_then(|p| p.ceiling)
                    .unwrap_or(0.0),
                NodeRef::Output(_) => 0.0,
                // junctions relay whatever feeds them; weight them equally
                NodeRef::Junction(_) => 1.0,
            })
            .collect();
        let total: f64 = weights.iter().sum();
        for (k, &ei) in incoming.iter().enumerate() {
            let share = if total > 0.0 {
                weights[k] / total
            } else {
                1.0 / incoming.len() as f64
            };
            edge_flow[ei] += demand * share;
        }
    };

    for node in order {
        match node {
            NodeRef::Output(pid) => {
                let port = s
                    .outputs
                    .iter()
                    .find(|p| &p.id == pid)
                    .ok_or_else(|| SolveError::UnknownRef { id: pid.clone() })?;
                let rate = targets.get(pid).copied().unwrap_or(port.rate);
                pull(node, &port.item, rate, &mut edge_flow);
            }
            NodeRef::Group(gid) => {
                let group = s
                    .groups
                    .iter()
                    .find(|g| &g.id == gid)
                    .ok_or_else(|| SolveError::UnknownRef { id: gid.clone() })?;
                // Demand on this group = flows already assigned to its outgoing edges.
                let mut cycles: f64 = 0.0;
                for (item, amount) in &group.recipe.outputs {
                    let demanded: f64 = graph.out_edges[node]
                        .iter()
                        .filter(|&&ei| &s.edges[ei].item == item)
                        .map(|&ei| edge_flow[ei])
                        .sum();
                    if *amount > 0.0 {
                        cycles = cycles.max(demanded / (amount * 60.0 / group.recipe.duration_s));
                    }
                }
                group_cycles.insert(gid.clone(), cycles);
                for (item, _) in group.recipe.inputs.clone() {
                    let need = cycles * group.recipe.in_rate(&item);
                    pull(node, &item, need, &mut edge_flow);
                }
            }
            NodeRef::Junction(_) => {
                // pure pass-through: demand assigned to outgoing edges pulls the
                // same per-item demand across incoming edges
                let mut demand_by_item: BTreeMap<&str, f64> = BTreeMap::new();
                for &ei in graph.out_edges[node].iter() {
                    *demand_by_item
                        .entry(s.edges[ei].item.as_str())
                        .or_insert(0.0) += edge_flow[ei];
                }
                for (item, demand) in demand_by_item {
                    pull(node, item, demand, &mut edge_flow);
                }
            }
            NodeRef::Input(_) => {} // sources terminate the pull
        }
    }
    Ok((edge_flow, group_cycles))
}

pub fn solve(snapshot: &FactorySnapshot, edit: &T0Edit) -> Result<SolveResult, SolveError> {
    let start = now_us();
    let graph = Graph::build(snapshot);
    let order = graph.reverse_topo()?;

    // Assemble targets, applying the edit.
    let mut targets: BTreeMap<String, f64> = snapshot
        .outputs
        .iter()
        .map(|p| (p.id.clone(), p.rate))
        .collect();
    let mut clock_override: Option<(String, f64)> = None;
    let mut edited_port: Option<String> = None;
    match edit {
        T0Edit::SetTarget { port, rate } => {
            if !targets.contains_key(port) {
                return Err(SolveError::UnknownRef { id: port.clone() });
            }
            targets.insert(port.clone(), *rate);
            edited_port = Some(port.clone());
        }
        T0Edit::SetClock { group, clock } => clock_override = Some((group.clone(), *clock)),
        T0Edit::Recompute => {}
    }

    // Ceiling analysis for the edited target: flows are affine in T.
    let mut target_ceiling: Option<TargetCeiling> = None;
    let mut clamped = false;
    if let Some(port) = &edited_port {
        let mut t0 = targets.clone();
        t0.insert(port.clone(), 0.0);
        let mut t1 = targets.clone();
        t1.insert(port.clone(), 1.0);
        let (f0, _) = demand_pass(&graph, &order, &t0)?;
        let (f1, _) = demand_pass(&graph, &order, &t1)?;
        let mut max_t = f64::INFINITY;
        let mut binding: Option<Constraint> = None;
        for (i, e) in snapshot.edges.iter().enumerate() {
            let per_unit = f1[i] - f0[i];
            if per_unit > 1e-9 {
                let t = (e.capacity - f0[i]) / per_unit;
                if t < max_t {
                    max_t = t;
                    binding = Some(Constraint::BeltCapacity {
                        edge: e.id.clone(),
                        item: e.item.clone(),
                        capacity: e.capacity,
                    });
                }
            }
        }
        for p in &snapshot.inputs {
            if let Some(ceiling) = p.ceiling {
                let node = NodeRef::Input(p.id.clone());
                let flow_at = |flows: &Vec<f64>| -> f64 {
                    graph
                        .out_edges
                        .get(&node)
                        .map(|v| v.iter().map(|&ei| flows[ei]).sum())
                        .unwrap_or(0.0)
                };
                let (p0, p1) = (flow_at(&f0), flow_at(&f1));
                let per_unit = p1 - p0;
                if per_unit > 1e-9 {
                    let t = (ceiling - p0) / per_unit;
                    if t < max_t {
                        max_t = t;
                        binding = Some(Constraint::InputCeiling {
                            port: p.id.clone(),
                            item: p.item.clone(),
                            ceiling,
                        });
                    }
                }
            }
        }
        if let Some(b) = binding {
            let max_rate = max_t.max(0.0);
            if targets[port] > max_rate + 1e-9 {
                targets.insert(port.clone(), max_rate);
                clamped = true;
            }
            target_ceiling = Some(TargetCeiling {
                max_rate,
                binding: b,
            });
        }
    }

    let (edge_flow, group_cycles) = demand_pass(&graph, &order, &targets)?;

    // Materialize results.
    let mut groups = BTreeMap::new();
    let mut total_power = 0.0;
    for g in &snapshot.groups {
        // `group_cycles` is machine-equivalents at 100% clock (demand ÷ per-machine rate).
        let cycles = group_cycles.get(&g.id).copied().unwrap_or(0.0);
        let machines_exact = cycles;
        let (count, clock) = match &clock_override {
            Some((gid, c)) if gid == &g.id => {
                let count = (machines_exact / c).ceil().max(1.0) as u32;
                (count, *c)
            }
            _ => {
                if machines_exact <= 1e-9 {
                    (g.count.max(1), 0.0)
                } else {
                    let count = machines_exact.ceil().max(1.0) as u32;
                    (count, machines_exact / count as f64)
                }
            }
        };
        let power = g.recipe.power_mw
            * count as f64
            * if clock > 0.0 {
                clock.powf(POWER_EXPONENT)
            } else {
                0.0
            };
        total_power += power;
        let mut in_rates = BTreeMap::new();
        for (item, _) in &g.recipe.inputs {
            in_rates.insert(item.clone(), cycles * g.recipe.in_rate(item));
        }
        let mut out_rates = BTreeMap::new();
        for (item, _) in &g.recipe.outputs {
            out_rates.insert(item.clone(), cycles * g.recipe.out_rate(item));
        }
        groups.insert(
            g.id.clone(),
            GroupResult {
                count,
                clock,
                power_mw: power,
                in_rates,
                out_rates,
            },
        );
    }

    let mut edges = BTreeMap::new();
    for (i, e) in snapshot.edges.iter().enumerate() {
        let flow = edge_flow[i];
        edges.insert(
            e.id.clone(),
            EdgeResult {
                flow,
                saturation: if e.capacity > 0.0 {
                    flow / e.capacity
                } else {
                    0.0
                },
            },
        );
    }

    let mut ports = BTreeMap::new();
    for p in &snapshot.inputs {
        let node = NodeRef::Input(p.id.clone());
        let flow: f64 = graph
            .out_edges
            .get(&node)
            .map(|v| v.iter().map(|&ei| edge_flow[ei]).sum())
            .unwrap_or(0.0);
        ports.insert(p.id.clone(), flow);
    }
    for p in &snapshot.outputs {
        ports.insert(p.id.clone(), targets[&p.id]);
    }

    Ok(SolveResult {
        groups,
        edges,
        ports,
        total_power_mw: total_power,
        target_ceiling,
        clamped,
        solve_us: now_us().saturating_sub(start),
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn now_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// On wasm the JS wrapper measures with performance.now(); avoid std::time::Instant
/// which aborts on wasm32-unknown-unknown.
#[cfg(target_arch = "wasm32")]
fn now_us() -> u64 {
    0
}
