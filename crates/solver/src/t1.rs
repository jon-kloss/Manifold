//! T1 — local per-factory LP (SDD §5.2), 50ms budget. Fixed recipe set.
//! Variables: group cycle rates + edge flows. Constraints: belt capacities,
//! input ceilings, conservation. Objective: meet targets, minimize machines
//! then power. Infeasible → clamp to best achievable and name the binding
//! constraint (no dead ends).

use std::collections::BTreeMap;

use good_lp::{
    constraint, microlp, variable, variables, Expression, Solution, SolverModel, Variable,
};

use crate::model::*;

const EPS: f64 = 1e-6;

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

/// Solve the LP with the edited output's target either fixed (feasibility pass)
/// or free-and-maximized (ceiling pass). Returns (group cycles, edge flows,
/// achieved target) on success.
fn run_lp(
    snapshot: &FactorySnapshot,
    targets: &BTreeMap<String, f64>,
    maximize_port: Option<&str>,
) -> Result<(Vec<f64>, Vec<f64>, f64), SolveError> {
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

    let lp = Lp {
        group_vars,
        edge_vars,
    };

    // Objective: machines (∝ Σ m_g·duration) with a tiny power tiebreak; or
    // maximize the free target (machines as negative tiebreak).
    // Group variables are machine-equivalents at 100% clock, so Σv is machines.
    let machines: Expression = lp.group_vars.iter().map(|&v| v * 1.0).sum();
    let power_tiebreak: Expression = snapshot
        .groups
        .iter()
        .zip(&lp.group_vars)
        .map(|(g, &v)| v * (g.recipe.power_mw * 1e-4))
        .sum();
    let objective: Expression = match target_var {
        Some(t) => -1000.0 * t + machines.clone() + power_tiebreak,
        None => machines.clone() + power_tiebreak,
    };

    let mut model = vars.minimise(objective).using(microlp);

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
    for p in &snapshot.outputs {
        let node = NodeRef::Output(p.id.clone());
        let inflow: Expression = edges_into(snapshot, &node, None)
            .map(|ei| lp.edge_vars[ei])
            .sum();
        match (maximize_port, target_var) {
            (Some(mp), Some(t)) if mp == p.id => {
                model = model.with(constraint!(inflow == t));
            }
            _ => {
                let rate = targets.get(&p.id).copied().unwrap_or(p.rate);
                model = model.with(constraint!(inflow == rate));
            }
        }
    }

    let solution = model.solve().map_err(|e| match e {
        good_lp::ResolutionError::Infeasible => SolveError::Internal {
            message: "infeasible".into(),
        },
        other => SolveError::Internal {
            message: other.to_string(),
        },
    })?;

    let cycles: Vec<f64> = lp.group_vars.iter().map(|&v| solution.value(v)).collect();
    let flows: Vec<f64> = lp.edge_vars.iter().map(|&v| solution.value(v)).collect();
    let achieved = target_var.map(|t| solution.value(t)).unwrap_or(0.0);
    Ok((cycles, flows, achieved))
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

pub fn solve(snapshot: &FactorySnapshot, edit: &T0Edit) -> Result<SolveResult, SolveError> {
    let start = std::time::Instant::now();

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
        if let Ok((_, flows, max_rate)) = run_lp(snapshot, &targets, Some(port)) {
            if let Some(binding) = find_binding(snapshot, &flows) {
                if targets[port] > max_rate + EPS * (1.0 + max_rate) {
                    targets.insert(port.clone(), max_rate);
                    clamped = true;
                }
                target_ceiling = Some(TargetCeiling { max_rate, binding });
            }
        }
    }

    let (cycles, flows, _) = run_lp(snapshot, &targets, None)?;

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
        solve_us: start.elapsed().as_micros() as u64,
    })
}
