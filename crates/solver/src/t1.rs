//! T1 — local per-factory LP (SDD §5.2), 50ms budget. Fixed recipe set.
//! Variables: group cycle rates + edge flows + one shortfall slack per output
//! target. Constraints: belt capacities, input ceilings, conservation; output
//! targets are ELASTIC (`inflow + shortfall == rate`), so the LP is feasible
//! for every structurally valid snapshot. Objective (weighted lexicographic):
//! minimize shortfall first (heavy penalty), then machines, then power.
//!
//! No dead ends: an unmeetable target DEGRADES — `ports` reports the achieved
//! rate and `SolveResult::shortfalls` carries the per-port gap with a named
//! binding when one is attributable (`Disconnected` for unwired ports/groups,
//! belt/ceiling via `find_binding`, else `None`). The ceiling pass still
//! hard-stops and clamps an explicitly edited target at a named capacity
//! ceiling; structural shortfalls never rewrite the user's target.

use std::collections::BTreeMap;

use good_lp::{
    constraint, microlp, variable, variables, Expression, Solution, SolverModel, Variable,
};

use crate::model::*;

const EPS: f64 = 1e-6;

/// Objective weight per item/min of unmet target. Three orders above the
/// ceiling pass's `1000·t` maximize term (so maximizing the edited port never
/// cannibalizes another port) and far above any realistic machines term (so
/// shortfall is never taken to save machines).
const SHORTFALL_PENALTY: f64 = 1e6;

/// Penalty per machine-equivalent a generator runs below its nameplate. THREE
/// orders UNDER [`SHORTFALL_PENALTY`] so a real output target wins the fuel even
/// when its line burns far more fuel per unit of output than the generator burns
/// per machine-equivalent (the penalties are per-output vs per-machine, so the
/// margin must cover the fuel-intensity ratio; 1000× covers every real recipe),
/// yet still THREE orders above the machines term (coefficient 1.0 per
/// machine-equivalent) so a generator with free fuel runs fully. This slack is
/// its own variable, never an output port — it never enters `shortfalls`,
/// `ports`, and (see the objective) is excluded from the ceiling pass so it
/// cannot cannibalize the edited target's achievable ceiling.
const GEN_PENALTY: f64 = 1e3;

/// Piecewise-linear CONCAVE fairness reward on each parallel-class member's
/// per-count utilization: `(segment length, marginal reward)`, low
/// utilization first, marginals strictly decreasing (that concavity is what
/// makes the LP water-fill a class into balance — a loaded sibling's next
/// unit is always worth less than a starved sibling's). Segments span
/// utilization 0..2.5 (the overclock ceiling). Every marginal is far below
/// the machines cost (1.0 per machine-equivalent), so fairness can NEVER buy
/// an extra machine or trade a target — it only breaks exact ties.
const FAIRNESS_SEGMENTS: [(f64, f64); 4] = [(0.25, 4e-3), (0.25, 3e-3), (0.5, 2e-3), (1.5, 1e-3)];

struct Lp {
    group_vars: Vec<Variable>,
    edge_vars: Vec<Variable>,
}

fn edges_into<'a>(
    s: &'a FactorySnapshot,
    node: &'a NodeRef,
    item: Option<&'a str>,
) -> impl Iterator<Item = usize> + 'a {
    s.edges
        .iter()
        .enumerate()
        .filter(move |(_, e)| &e.to == node && item.map(|i| e.item == i).unwrap_or(true))
        .map(|(i, _)| i)
}

fn edges_out_of<'a>(
    s: &'a FactorySnapshot,
    node: &'a NodeRef,
    item: Option<&'a str>,
) -> impl Iterator<Item = usize> + 'a {
    s.edges
        .iter()
        .enumerate()
        .filter(move |(_, e)| &e.from == node && item.map(|i| e.item == i).unwrap_or(true))
        .map(|(i, _)| i)
}

/// One LP pass's solution.
struct LpSolution {
    /// Group cycle rates (machine-equivalents at 100% clock).
    cycles: Vec<f64>,
    /// Edge flows, indexed like `snapshot.edges`.
    flows: Vec<f64>,
    /// Achieved rate of the maximized port (0.0 on fixed-target passes).
    max_rate: f64,
    /// Unmet target per fixed-target output port (items/min).
    shortfalls: BTreeMap<String, f64>,
}

/// Solve the LP with the edited output's target either fixed (feasibility pass)
/// or free-and-maximized (ceiling pass). Fixed targets are elastic — a per-port
/// shortfall slack keeps the LP feasible (the all-zero flow point always
/// satisfies it).
fn run_lp(
    snapshot: &FactorySnapshot,
    targets: &BTreeMap<String, f64>,
    maximize_port: Option<&str>,
) -> Result<LpSolution, SolveError> {
    let mut vars = variables!();
    let group_vars: Vec<Variable> = snapshot
        .groups
        .iter()
        .map(|_| vars.add(variable().min(0.0)))
        .collect();
    let edge_vars: Vec<Variable> = snapshot
        .edges
        .iter()
        .map(|e| vars.add(variable().min(0.0).max(e.capacity)))
        .collect();
    let target_var = maximize_port.map(|_| vars.add(variable().min(0.0)));
    // One shortfall slack per output whose target is held fixed.
    let shortfall_vars: Vec<Option<Variable>> = snapshot
        .outputs
        .iter()
        .map(|p| match maximize_port {
            Some(mp) if mp == p.id => None,
            _ => Some(vars.add(variable().min(0.0))),
        })
        .collect();
    // One slack per driven generator: how far below nameplate it runs.
    let gen_slack_vars: Vec<Option<Variable>> = snapshot
        .groups
        .iter()
        .map(|g| g.driven_cycles.map(|_| vars.add(variable().min(0.0))))
        .collect();

    // Parallel-split fairness: a demand several IDENTICAL groups (same recipe
    // id + machine) can serve costs the same machines however it is split, so
    // the objective below is degenerate across those splits and the simplex
    // vertex was free to load ONE branch and idle its siblings — while the T0
    // drag preview splits proportionally ("T1 settles the exact split").
    //
    // Every member of a class of ≥2 identical groups earns a piecewise-linear
    // CONCAVE reward on its per-count utilization (cycles_i / count_i):
    // utilization fills reward segments of DECREASING marginal value, so
    // moving a unit of cycles from a loaded sibling to a starved one always
    // gains reward — the LP water-fills the class into the most balanced
    // FEASIBLE split, and belt caps merely bend it. (A single per-class
    // max-min variable was not enough: one belt-throttled sibling pinned the
    // class floor and the healthy members above it were degenerate again.)
    //
    // Weight by COUNT, never count·clock: a planned group's clock is itself
    // an OUTPUT of the previous solve (the idle write-back sets it to 0), so
    // weighting by it is circular — one concentrated solve would eject the
    // starved sibling from its class and every later settle would re-conc-
    // entrate. Count is the built/planned capacity share and stays honest.
    // Driven generators keep their own law; count-0 groups have no share.
    let mut classes: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (gi, g) in snapshot.groups.iter().enumerate() {
        if g.driven_cycles.is_some() || g.count == 0 {
            continue;
        }
        classes
            .entry((g.recipe.id.clone(), g.recipe.machine.clone()))
            .or_default()
            .push(gi);
    }
    // Per class member: one variable per reward segment, in CYCLES units with
    // the segment length scaled by the member's count (so the marginal reward
    // per cycle is identical across members at equal per-count utilization —
    // scaling the reward instead would make small-count members cheaper per
    // reward unit and pull flow INTO them). Their sum is capped by the
    // member's cycles via the constraint below; greedy low-segment fill is
    // optimal because marginals decrease.
    let fairness_vars: Vec<(usize, Vec<Variable>)> = classes
        .into_values()
        .filter(|members| members.len() >= 2)
        .flatten()
        .map(|gi| {
            let count = snapshot.groups[gi].count as f64;
            let segs = FAIRNESS_SEGMENTS
                .iter()
                .map(|(len, _)| vars.add(variable().min(0.0).max(*len * count)))
                .collect();
            (gi, segs)
        })
        .collect();

    let lp = Lp {
        group_vars,
        edge_vars,
    };

    // Objective (weighted lexicographic): unmet targets first (heavy penalty),
    // then machines (∝ Σ m_g·duration) with a tiny power tiebreak; the ceiling
    // pass also maximizes the free target (dominated by the shortfall term).
    // Group variables are machine-equivalents at 100% clock, so Σv is machines.
    let machines: Expression = lp.group_vars.iter().map(|&v| v * 1.0).sum();
    let power_tiebreak: Expression = snapshot
        .groups
        .iter()
        .zip(&lp.group_vars)
        .map(|(g, &v)| v * (g.recipe.power_mw * 1e-4))
        .sum();
    let shortfall_penalty: Expression = shortfall_vars
        .iter()
        .flatten()
        .map(|&v| v * SHORTFALL_PENALTY)
        .sum();
    let gen_penalty: Expression = gen_slack_vars
        .iter()
        .flatten()
        .map(|&v| v * GEN_PENALTY)
        .sum();
    let fairness: Expression = fairness_vars
        .iter()
        .flat_map(|(_, segs)| {
            segs.iter()
                .zip(FAIRNESS_SEGMENTS.iter())
                .map(|(&z, (_, reward))| z * *reward)
        })
        .sum();
    let objective: Expression = match target_var {
        // Ceiling pass: maximize the edited port WITHOUT gen_penalty. Including it
        // let a driven generator sharing capped fuel outbid the -1000·t maximize
        // term (GEN_PENALTY ≫ 1000), starving the edited port so its ceiling read
        // ~0. The driven constraint (m + s == n) still holds, but with s free the
        // generator simply yields all contested fuel to the port being measured.
        Some(t) => {
            shortfall_penalty - 1000.0 * t + machines.clone() + power_tiebreak - fairness.clone()
        }
        None => shortfall_penalty + gen_penalty + machines.clone() + power_tiebreak - fairness,
    };

    let mut model = vars.minimise(objective).using(microlp);

    // Fairness segment fill (see fairness_vars above): a member's segments —
    // already count-scaled, i.e. in cycles units — can only fill up to its
    // actual cycles: cycles_i ≥ Σz.
    for (gi, segs) in &fairness_vars {
        let filled: Expression = segs.iter().map(|&z| z * 1.0).sum();
        model = model.with(constraint!(lp.group_vars[*gi] - filled >= 0.0));
    }

    for (gi, g) in snapshot.groups.iter().enumerate() {
        let node = NodeRef::Group(g.id.clone());
        let m = lp.group_vars[gi];
        for (item, _) in &g.recipe.inputs {
            let inflow: Expression = edges_into(snapshot, &node, Some(item))
                .map(|ei| lp.edge_vars[ei])
                .sum();
            model = model.with(constraint!(inflow == m * g.recipe.in_rate(item)));
        }
        for (item, _) in &g.recipe.outputs {
            let outflow: Expression = edges_out_of(snapshot, &node, Some(item))
                .map(|ei| lp.edge_vars[ei])
                .sum();
            model = model.with(constraint!(outflow <= m * g.recipe.out_rate(item)));
        }
        // Driven generator: elastic pull toward nameplate cycles. The input
        // constraints (fuel inflow == m·in_rate, capped by belts/ceilings) pull
        // it DOWN — never up — so it's fuel-limited (0 when unfueled). The slack
        // is standalone (not an output port), so it stays out of `shortfalls`,
        // `ports`, and the ceiling precompute; and GEN_PENALTY < SHORTFALL means
        // a real output target wins any fight for shared fuel.
        if let (Some(n), Some(s)) = (g.driven_cycles, gen_slack_vars[gi]) {
            model = model.with(constraint!(m + s == n));
        }
    }
    for p in &snapshot.inputs {
        if let Some(ceiling) = p.ceiling {
            let node = NodeRef::Input(p.id.clone());
            let outflow: Expression = edges_out_of(snapshot, &node, None)
                .map(|ei| lp.edge_vars[ei])
                .sum();
            model = model.with(constraint!(outflow <= ceiling));
        }
    }
    // Junctions conserve flow per item: Σin == Σout (no transform, no sink).
    for jid in &snapshot.junctions {
        let node = NodeRef::Junction(jid.clone());
        let items: std::collections::BTreeSet<&str> = snapshot
            .edges
            .iter()
            .filter(|e| e.from == node || e.to == node)
            .map(|e| e.item.as_str())
            .collect();
        for item in items {
            let inflow: Expression = edges_into(snapshot, &node, Some(item))
                .map(|ei| lp.edge_vars[ei])
                .sum();
            let outflow: Expression = edges_out_of(snapshot, &node, Some(item))
                .map(|ei| lp.edge_vars[ei])
                .sum();
            model = model.with(constraint!(inflow == outflow));
        }
    }
    for (pi, p) in snapshot.outputs.iter().enumerate() {
        let node = NodeRef::Output(p.id.clone());
        let inflow: Expression = edges_into(snapshot, &node, None)
            .map(|ei| lp.edge_vars[ei])
            .sum();
        match (maximize_port, target_var) {
            (Some(mp), Some(t)) if mp == p.id => {
                model = model.with(constraint!(inflow == t));
            }
            _ => {
                let rate = targets.get(&p.id).copied().unwrap_or(p.rate).max(0.0);
                let s = shortfall_vars[pi].expect("fixed-target output has a shortfall slack");
                model = model.with(constraint!(inflow + s == rate));
            }
        }
    }

    // Elastic targets make the model feasible by construction — an Infeasible
    // here is a genuine solver bug, not a planning state.
    let solution = model.solve().map_err(|e| match e {
        good_lp::ResolutionError::Infeasible => SolveError::Internal {
            message: "infeasible (bug: elastic targets make the T1 model always feasible)".into(),
        },
        other => SolveError::Internal {
            message: other.to_string(),
        },
    })?;

    Ok(LpSolution {
        cycles: lp.group_vars.iter().map(|&v| solution.value(v)).collect(),
        flows: lp.edge_vars.iter().map(|&v| solution.value(v)).collect(),
        max_rate: target_var.map(|t| solution.value(t)).unwrap_or(0.0),
        shortfalls: snapshot
            .outputs
            .iter()
            .zip(&shortfall_vars)
            .filter_map(|(p, v)| v.map(|v| (p.id.clone(), solution.value(v))))
            .collect(),
    })
}

/// Identify the constraint that binds at the ceiling solution.
fn find_binding(snapshot: &FactorySnapshot, flows: &[f64]) -> Option<Constraint> {
    for (i, e) in snapshot.edges.iter().enumerate() {
        if flows[i] >= e.capacity - EPS * (1.0 + e.capacity) {
            return Some(Constraint::BeltCapacity {
                edge: e.id.clone(),
                item: e.item.clone(),
                capacity: e.capacity,
            });
        }
    }
    for p in &snapshot.inputs {
        if let Some(ceiling) = p.ceiling {
            let node = NodeRef::Input(p.id.clone());
            let used: f64 = edges_out_of(snapshot, &node, None)
                .map(|ei| flows[ei])
                .sum();
            if used >= ceiling - EPS * (1.0 + ceiling) {
                return Some(Constraint::InputCeiling {
                    port: p.id.clone(),
                    item: p.item.clone(),
                    ceiling,
                });
            }
        }
    }
    None
}

/// Name what limits an output port that carries a shortfall: structural
/// unwiring first (an unwired port or group input forces the whole chain to
/// zero), then a saturated belt/ceiling at the achieved optimum, else `None`
/// (unmet with no single named constraint — e.g. competing pinned targets).
fn attribute_shortfall(
    snapshot: &FactorySnapshot,
    port: &OutputPortSpec,
    flows: &[f64],
) -> Option<Constraint> {
    let node = NodeRef::Output(port.id.clone());
    if edges_into(snapshot, &node, None).next().is_none() {
        return Some(Constraint::Disconnected {
            node: port.id.clone(),
            item: port.item.clone(),
        });
    }
    for g in &snapshot.groups {
        let gnode = NodeRef::Group(g.id.clone());
        for (item, _) in &g.recipe.inputs {
            if edges_into(snapshot, &gnode, Some(item)).next().is_none() {
                return Some(Constraint::Disconnected {
                    node: g.id.clone(),
                    item: item.clone(),
                });
            }
        }
    }
    find_binding(snapshot, flows)
}

pub fn solve(snapshot: &FactorySnapshot, edit: &T0Edit) -> Result<SolveResult, SolveError> {
    let start = now_us();

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

    // Ceiling pass for the edited port (also serves as the infeasibility fallback).
    let mut target_ceiling = None;
    let mut clamped = false;
    if let Some(port) = &edited_port {
        if let Ok(ceiling_pass) = run_lp(snapshot, &targets, Some(port)) {
            let max_rate = ceiling_pass.max_rate;
            if let Some(binding) = find_binding(snapshot, &ceiling_pass.flows) {
                if targets[port] > max_rate + EPS * (1.0 + max_rate) {
                    targets.insert(port.clone(), max_rate);
                    clamped = true;
                }
                target_ceiling = Some(TargetCeiling { max_rate, binding });
            }
        }
    }

    let LpSolution {
        cycles,
        flows,
        shortfalls: port_shortfalls,
        ..
    } = run_lp(snapshot, &targets, None)?;

    let mut groups = BTreeMap::new();
    let mut total_power = 0.0;
    for (gi, g) in snapshot.groups.iter().enumerate() {
        // Group variables are machine-equivalents at 100% clock.
        let m = cycles[gi];
        let machines_exact = m;
        let (count, clock) = match &clock_override {
            Some((gid, c)) if gid == &g.id => ((machines_exact / c).ceil().max(1.0) as u32, *c),
            _ => {
                if machines_exact <= EPS {
                    (g.count.max(1), 0.0)
                } else {
                    // Integer counts: relax, round up, redistribute clock (SDD §5.2).
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
            in_rates.insert(item.clone(), m * g.recipe.in_rate(item));
        }
        let mut out_rates = BTreeMap::new();
        for (item, _) in &g.recipe.outputs {
            out_rates.insert(item.clone(), m * g.recipe.out_rate(item));
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
        edges.insert(
            e.id.clone(),
            EdgeResult {
                flow: flows[i],
                saturation: if e.capacity > 0.0 {
                    flows[i] / e.capacity
                } else {
                    0.0
                },
            },
        );
    }

    let mut ports = BTreeMap::new();
    for p in &snapshot.inputs {
        let node = NodeRef::Input(p.id.clone());
        let used: f64 = edges_out_of(snapshot, &node, None)
            .map(|ei| flows[ei])
            .sum();
        ports.insert(p.id.clone(), used);
    }
    let mut shortfalls = BTreeMap::new();
    for p in &snapshot.outputs {
        let requested = targets[&p.id].max(0.0);
        let missing = port_shortfalls.get(&p.id).copied().unwrap_or(0.0);
        if missing > EPS * (1.0 + requested) {
            // Degraded: report the achieved rate; the canonical target stays.
            ports.insert(p.id.clone(), (requested - missing).max(0.0));
            shortfalls.insert(
                p.id.clone(),
                Shortfall {
                    requested,
                    missing,
                    binding: attribute_shortfall(snapshot, p, &flows),
                },
            );
        } else {
            ports.insert(p.id.clone(), targets[&p.id]);
        }
    }

    Ok(SolveResult {
        groups,
        edges,
        ports,
        shortfalls,
        total_power_mw: total_power,
        target_ceiling,
        clamped,
        solve_us: now_us().saturating_sub(start),
    })
}

/// Monotonic-ish microsecond clock for the solve-time telemetry. `Instant`
/// aborts on `wasm32-unknown-unknown`, so mirror t0: real time natively, a
/// zero stub on wasm (the browser measures wall time with `performance.now()`).
#[cfg(not(target_arch = "wasm32"))]
fn now_us() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
fn now_us() -> u64 {
    0
}
