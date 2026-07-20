//! T0 — ratio propagation (SDD §5.1). Fixed structure, fixed recipes: a
//! topological walk scaling counts/clocks/rates linearly from the changed
//! target. Pure function; compiled to WASM for the renderer's drag frames.
//!
//! With distribution fractions frozen at snapshot weights, every flow is a
//! convex, nondecreasing, PIECEWISE-linear function of the edited target —
//! globally affine only when no recipe has multiple outputs (a group's
//! cycles are the max over its per-output demands, which is where the kinks
//! come from). The hard-stop ceiling is therefore found by bracketing the
//! feasibility boundary and bisecting — feasibility is monotone in the
//! target because every flow is nondecreasing, and every belt cap and input
//! ceiling is checked directly rather than extrapolated — then polished
//! with one secant step on the binding constraint inside the final linear
//! segment, which is exact to floating point.

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
                // Input weight: the ceiling BOUNDED BY the edge's belt — and a
                // ceiling-less port is open supply (bounded only by its belt),
                // NEVER 0. Weight 0 on an open port sent 100% of a parallel
                // same-item pull through the capped sibling, clamping the drag
                // preview at one node's rate while the open port sat unused
                // (MAKE's pooled wiring makes this topology routine). Weights
                // stay static per snapshot — draw-aware weights would break
                // the piecewise-linear/monotone contract the hard-stop
                // bracketing above depends on; T1 settles the exact split.
                NodeRef::Input(pid) => s
                    .inputs
                    .iter()
                    .find(|p| &p.id == pid)
                    .map(|p| match p.ceiling {
                        Some(c) => c.min(s.edges[ei].capacity),
                        None => s.edges[ei].capacity,
                    })
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
                // Driven-generator floor (mirror of T1's `m + s == n`): an
                // un-wired generator's power output is demanded by nothing, so
                // demand alone idles it to 0 cycles — and every drag frame read
                // "GENERATES 0 MW". Hold it at its nameplate cycles; fuel is
                // pulled below, and any over-cap fuel draw surfaces as
                // saturation like every other T0 demand (T1 settles exactly).
                if let Some(d) = group.driven_cycles {
                    cycles = cycles.max(d);
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

/// The portion of each edge's flow that serves SOFT group inputs (a generator's
/// cooling water). Soft demand is elastic — it never has to be delivered — so
/// the flow that genuinely competes for a pipe's or port's capacity, and thus
/// the only flow that may bind a target ceiling, is `total - soft`. This traces
/// each group's soft demand backward to the edges (and THROUGH junctions) that
/// carry it, splitting at each node in proportion to the already-computed total
/// flow so an edge's soft share never exceeds its total. A snapshot with no
/// soft inputs yields all zeros — identical to ignoring softness — so ordinary
/// factories are wholly unaffected. This is what makes the soft-input invariant
/// robust to merged/shared water (a merger junction, or one water port feeding
/// both a generator and a refinery), which a per-edge structural test cannot be.
fn soft_edge_flows_with(
    graph: &Graph,
    order: &[NodeRef],
    cycles: &BTreeMap<String, f64>,
    total_flows: &[f64],
) -> Vec<f64> {
    let s = graph.snapshot;
    let mut soft = vec![0.0f64; s.edges.len()];
    // Distribute `amount` of soft demand for `item` entering `node` back across
    // its incoming edges carrying that item, proportional to their total flow
    // (equal split only when every candidate carries none).
    let push = |node: &NodeRef, item: &str, amount: f64, soft: &mut Vec<f64>| {
        if amount <= 0.0 {
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
            return;
        }
        let tot: f64 = incoming.iter().map(|&ei| total_flows[ei]).sum();
        for &ei in &incoming {
            let share = if tot > 1e-12 {
                total_flows[ei] / tot
            } else {
                1.0 / incoming.len() as f64
            };
            soft[ei] += amount * share;
        }
    };
    for node in order {
        match node {
            NodeRef::Group(gid) => {
                let Some(group) = s.groups.iter().find(|g| &g.id == gid) else {
                    continue;
                };
                let m = cycles.get(gid).copied().unwrap_or(0.0);
                if m <= 0.0 {
                    continue;
                }
                for (item, _) in &group.recipe.inputs {
                    if group.soft_inputs.contains(item) {
                        push(node, item, m * group.recipe.in_rate(item), &mut soft);
                    }
                }
            }
            NodeRef::Junction(_) => {
                // Soft demand leaving on out-edges must be pulled in across the
                // junction's in-edges, so softness propagates through mergers.
                let mut soft_by_item: BTreeMap<&str, f64> = BTreeMap::new();
                for &ei in graph
                    .out_edges
                    .get(node)
                    .map(|v| v.as_slice())
                    .unwrap_or(&[])
                {
                    *soft_by_item.entry(s.edges[ei].item.as_str()).or_insert(0.0) += soft[ei];
                }
                for (item, amount) in soft_by_item {
                    push(node, item, amount, &mut soft);
                }
            }
            _ => {} // outputs are hard demand; inputs terminate the trace
        }
    }
    soft
}

/// Standalone soft-flow trace for callers without a prebuilt `Graph` (T1).
/// Builds the graph and a reverse-topo order; on a cyclic graph it returns all
/// zeros (no soft subtraction — safe, never over-reports a binding).
#[cfg(feature = "lp")]
pub(crate) fn soft_edge_flows(
    snapshot: &FactorySnapshot,
    cycles: &BTreeMap<String, f64>,
    total_flows: &[f64],
) -> Vec<f64> {
    let graph = Graph::build(snapshot);
    match graph.reverse_topo() {
        Ok(order) => soft_edge_flows_with(&graph, &order, cycles, total_flows),
        Err(_) => vec![0.0; snapshot.edges.len()],
    }
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

    // ---- Cap bookkeeping, shared by the ceiling analysis and the post-solve
    // defense check. Caps are scanned in T1's `find_binding` order (edges by
    // index, then input ceilings) so tie-breaks name the same constraint in
    // both tiers.
    #[derive(Clone, Copy)]
    enum CapKey {
        Edge(usize),
        Input(usize),
    }
    /// Relative tolerance of the ceiling search (bracket + bisection).
    const REL_TOL: f64 = 1e-9;
    // Every belt/pipe edge and every ceilinged input is a cap. Soft (elastic)
    // water demand is excluded not by dropping caps but by measuring each cap
    // against HARD flow (`total - soft`, via `soft_edge_flows`) below — so a
    // pure-soft water pipe carries zero hard flow and never binds, while a pipe
    // shared with a hard consumer binds only on that consumer's genuine demand.
    let cap_keys: Vec<CapKey> = (0..snapshot.edges.len())
        .map(CapKey::Edge)
        .chain(
            snapshot
                .inputs
                .iter()
                .enumerate()
                .filter(|(_, p)| p.ceiling.is_some())
                .map(|(i, _)| CapKey::Input(i)),
        )
        .collect();
    let cap_bound = |k: CapKey| -> f64 {
        match k {
            CapKey::Edge(i) => snapshot.edges[i].capacity,
            CapKey::Input(i) => snapshot.inputs[i].ceiling.unwrap_or(f64::INFINITY),
        }
    };
    let cap_flow = |k: CapKey, flows: &[f64]| -> f64 {
        match k {
            CapKey::Edge(i) => flows[i],
            CapKey::Input(i) => {
                let node = NodeRef::Input(snapshot.inputs[i].id.clone());
                graph
                    .out_edges
                    .get(&node)
                    .map(|v| v.iter().map(|&ei| flows[ei]).sum())
                    .unwrap_or(0.0)
            }
        }
    };
    let cap_constraint = |k: CapKey| -> Constraint {
        match k {
            CapKey::Edge(i) => {
                let e = &snapshot.edges[i];
                Constraint::BeltCapacity {
                    edge: e.id.clone(),
                    item: e.item.clone(),
                    capacity: e.capacity,
                }
            }
            CapKey::Input(i) => {
                let p = &snapshot.inputs[i];
                Constraint::InputCeiling {
                    port: p.id.clone(),
                    item: p.item.clone(),
                    ceiling: p.ceiling.unwrap_or(f64::INFINITY),
                }
            }
        }
    };
    // Minimum normalized slack across all caps (negative = violated), with
    // the constraint it occurs at; first-in-order wins ties.
    let min_slack = |flows: &[f64]| -> Option<(CapKey, f64)> {
        let mut best: Option<(CapKey, f64)> = None;
        for &k in &cap_keys {
            let cap = cap_bound(k);
            let slack = (cap - cap_flow(k, flows)) / (1.0 + cap);
            if best.is_none_or(|(_, s)| slack < s) {
                best = Some((k, slack));
            }
        }
        best
    };

    // Ceiling analysis for the edited target. Flows are convex nondecreasing
    // piecewise-linear in the target (see module header), so a two-point
    // extrapolation can both miss real ceilings (a cap whose [0,1] slope is
    // zero) and invent low ones (a kink below T=1). Instead: bracket the
    // feasibility boundary, bisect on monotone feasibility, and polish with
    // one secant step on the binding constraint inside the final linear
    // segment (exact there; keeps single-output ceilings exact too).
    let mut target_ceiling: Option<TargetCeiling> = None;
    let mut clamped = false;
    // Whether every cap held with the edited target at 0. When true, the
    // clamp below guarantees the final solve satisfies every cap (asserted
    // after the solve); when false, the violation belongs to sibling fixed
    // targets and T1 owns the shortfall story.
    let mut base_feasible = true;
    if let Some(port) = &edited_port {
        let requested = targets[port];
        let base_targets = targets.clone();
        // Probes return HARD flow (total minus the soft/elastic water portion),
        // so the entire bracket/bisect/binding machinery below sees only the
        // flow that may legitimately bind the ceiling.
        let probe = |t: f64| -> Result<Vec<f64>, SolveError> {
            let mut probe_targets = base_targets.clone();
            probe_targets.insert(port.clone(), t);
            let (flows, cyc) = demand_pass(&graph, &order, &probe_targets)?;
            let soft = soft_edge_flows_with(&graph, &order, &cyc, &flows);
            Ok(flows
                .iter()
                .zip(&soft)
                .map(|(f, s)| (f - s).max(0.0))
                .collect())
        };
        let f0 = probe(0.0)?;
        // Per-cap slack with the edited target at ZERO. A violation here is
        // owned by SIBLING fixed targets — it exists with this edit
        // contributing nothing, so it must not zero the edited port (the old
        // behavior collapsed an independent chain to 0/min and blamed the
        // sibling's constraint). The search below uses RELATIVE feasibility:
        // no cap may fall below its own baseline (or -REL_TOL, whichever is
        // lower) — the edit may coexist with a sibling's violation but never
        // deepen one. Flows are nondecreasing in t, so each cap's slack is
        // nonincreasing and relative feasibility stays monotone — the
        // bracket/bisect machinery is unchanged.
        let base_slack: Vec<f64> = cap_keys
            .iter()
            .map(|&k| {
                let cap = cap_bound(k);
                (cap - cap_flow(k, &f0)) / (1.0 + cap)
            })
            .collect();
        let floor_of = |i: usize| (-REL_TOL).min(base_slack[i] - REL_TOL);
        let feasible_rel = |flows: &[f64]| -> bool {
            cap_keys.iter().enumerate().all(|(i, &k)| {
                let cap = cap_bound(k);
                (cap - cap_flow(k, flows)) / (1.0 + cap) >= floor_of(i)
            })
        };
        // Binding selection by slack RELATIVE to baseline: a sibling-violated
        // cap sits at large negative absolute slack forever — the cap that
        // binds the EDIT is the one closest to its own floor.
        // Binding selection considers only caps the edit actually PUSHES
        // (flow increasing across the final bracket): a flow-invariant
        // violated sibling sits at relative slack exactly 0 and would
        // otherwise tie-win against the true binding at the crossing point,
        // re-blaming the sibling the relative-feasibility rework exonerated.
        let min_rel_slack = |f_lo: &[f64], f_hi: &[f64]| -> Option<(CapKey, f64)> {
            let mut best: Option<(CapKey, f64)> = None;
            for (i, &k) in cap_keys.iter().enumerate() {
                if cap_flow(k, f_hi) <= cap_flow(k, f_lo) + 1e-12 {
                    continue; // the edit does not push this cap — cannot be its binding
                }
                let cap = cap_bound(k);
                let rel = (cap - cap_flow(k, f_lo)) / (1.0 + cap) - base_slack[i].min(0.0);
                if best.is_none_or(|(_, s)| rel < s) {
                    best = Some((k, rel));
                }
            }
            best
        };
        let ceiling: Option<(f64, CapKey)> = 'ceiling: {
            if min_slack(&f0).is_none() {
                break 'ceiling None; // no caps anywhere — unbounded
            }
            base_feasible = base_slack.iter().all(|s| *s >= -REL_TOL);
            // Seed the upper bracket from the [0,1] affine crossings — exact
            // for single-output graphs, so the common case brackets with no
            // extra doubling.
            let f1 = probe(1.0)?;
            let mut seed = f64::INFINITY;
            for &k in &cap_keys {
                let (a, b) = (cap_flow(k, &f0), cap_flow(k, &f1));
                if b - a > 1e-9 {
                    seed = seed.min((cap_bound(k) - a) / (b - a));
                }
            }
            // Bracket: [lo feasible, hi infeasible], growing hi by doubling.
            let mut lo = 0.0;
            let mut f_lo = f0.clone();
            let mut hi = requested.max(1.0);
            if seed.is_finite() {
                hi = hi.max(seed);
            }
            let mut f_hi = probe(hi)?;
            let mut doublings = 0;
            while feasible_rel(&f_hi) {
                if doublings >= 32 {
                    break 'ceiling None; // no cap ever binds — no finite ceiling
                }
                lo = hi;
                f_lo = f_hi;
                hi *= 2.0;
                f_hi = probe(hi)?;
                doublings += 1;
            }
            // Bisect down to relative REL_TOL (~40 halvings always suffice).
            for _ in 0..40 {
                if hi - lo <= REL_TOL * (1.0 + hi) {
                    break;
                }
                let mid = 0.5 * (lo + hi);
                let f_mid = probe(mid)?;
                if feasible_rel(&f_mid) {
                    lo = mid;
                    f_lo = f_mid;
                } else {
                    hi = mid;
                    f_hi = f_mid;
                }
            }
            // The binding is the minimum RELATIVE-slack cap at the feasible end.
            let Some((key, _)) = min_rel_slack(&f_lo, &f_hi) else {
                break 'ceiling None;
            };
            // Secant polish: the final bracket lies inside a single linear
            // segment of the binding flow, so its cap crossing is exact.
            let (g_lo, g_hi) = (cap_flow(key, &f_lo), cap_flow(key, &f_hi));
            let mut t_max = lo;
            if g_hi - g_lo > 1e-12 {
                let t = lo + (cap_bound(key) - g_lo) * (hi - lo) / (g_hi - g_lo);
                if t.is_finite() {
                    t_max = t.clamp(lo, hi);
                }
            }
            Some((t_max, key))
        };
        if let Some((max_t, key)) = ceiling {
            let max_rate = max_t.max(0.0);
            if targets[port] > max_rate + 1e-9 {
                targets.insert(port.clone(), max_rate);
                clamped = true;
            }
            target_ceiling = Some(TargetCeiling {
                max_rate,
                binding: cap_constraint(key),
            });
        }
    }

    let (edge_flow, group_cycles) = demand_pass(&graph, &order, &targets)?;

    // Defense in depth: when the graph satisfied every cap before the edit
    // contributed (base_feasible), the ceiling clamp above guarantees the
    // final solve does too — `clamped == false` alongside a violated cap is
    // impossible by construction, and a failure here means the ceiling
    // search returned an over-ceiling.
    if cfg!(debug_assertions) && edited_port.is_some() && base_feasible {
        // Assert on HARD flow, matching the cap set the ceiling search used.
        let soft = soft_edge_flows_with(&graph, &order, &group_cycles, &edge_flow);
        let hard: Vec<f64> = edge_flow
            .iter()
            .zip(&soft)
            .map(|(f, s)| (f - s).max(0.0))
            .collect();
        if let Some((key, slack)) = min_slack(&hard) {
            debug_assert!(
                slack >= -1e-6,
                "T0 ceiling under-clamped: {:?} violated (normalized slack {slack})",
                cap_constraint(key)
            );
        }
    }

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
        // T0's demand-pull never reports shortfalls; T1 owns that contract.
        shortfalls: BTreeMap::new(),
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
