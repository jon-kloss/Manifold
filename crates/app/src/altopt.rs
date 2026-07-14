//! Empire-wide alternate-recipe optimizer (W2b-D) — a DERIVED, ADVISORY,
//! READ-ONLY ranking. It never mutates canonical state and mints no new
//! canonical entity: it reads `(state, gamedata, unlocked)` and returns a
//! computed `Vec<AltOpportunity>`, each answering the "slot machine" pain —
//! "adopting this unlocked alternate everywhere would save N machines / M MW
//! across your empire, and here is the honest input trade + the retool cost."
//!
//! The algorithm is GREEDY per product item, NOT MILP (cross-item trade-offs are
//! BACKLOG). For each product with an UNLOCKED alternate that is not already the
//! chosen recipe, it computes the adopt-everywhere delta over every group making
//! that product (Σ machines/power/inputs current − alt), reusing the exact T2
//! machine/clock arithmetic. Retool cost is a CHEAP machine-count estimate at
//! ranking time (O(n), no scratch-solve): ◇ planned groups retool for ~free (a
//! legal in-place `SetGroupRecipe`), ◆ built groups cost `cutover::est_hours` of
//! the built machine count, plus a hard `node_reuse` flag when a built factory in
//! the opportunity still holds a node its refactored replacement would re-claim.
//! The expensive per-boundary scratch-solve (`Session::cutover_plan`) stays
//! DEFERRED to the per-row drill-down.
//!
//! CTA routing keeps the contract pivot intact: an all-◇ opportunity drafts a
//! T2-style `SetGroupRecipe` proposal ([`optimize_to_recipe`], legal on planned
//! groups); any ◆ built factory routes through `Session::plan_replacement` (a
//! W2a Refactor — the ◆ layer is NEVER touched). Both land in the existing
//! ProposalReview. With the trimmed fixture catalog `unlocked` is empty, so this
//! honestly returns no opportunities there — the algorithm is proven by unit
//! tests over synthetic gamedata.

use std::collections::{BTreeMap, BTreeSet};

use gamedata::docs::{GameData, POWER_ITEM};
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::*;
use planner_core::state::PlanState;
use serde::Serialize;

const EPS: f64 = 1e-6;

/// One ranked adopt-everywhere opportunity for a single unlocked alternate.
/// Purely derived — the numbers are computed from real recipe math, the input
/// trade is surfaced (never hidden), and the ◆/◇ split names exactly which
/// groups the CTA would touch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AltOpportunity {
    /// The alternate recipe class the empire would adopt.
    pub recipe: String,
    pub recipe_name: String,
    /// The product item this alternate makes (the axis of the comparison).
    pub product: String,
    pub product_name: String,
    /// Σ machines currently making the product − Σ machines the alt would need.
    /// Positive = the alt is cheaper empire-wide (the reason to surface it).
    pub machines_saved: i64,
    /// Σ current draw − Σ alt draw across the affected groups (MW).
    pub power_saved_mw: f64,
    /// Net per-input change if the alt is adopted everywhere: positive = the alt
    /// consumes MORE of that item (an honest trade — e.g. it swaps iron for
    /// steel). Sorted, near-zero entries dropped.
    pub input_deltas: Vec<(String, f64)>,
    /// ◇ planned group ids the alt would retool in place (retool ≈ free).
    pub affected_planned: Vec<Id>,
    /// ◆ built group ids — these route through a W2a Refactor, never mutated.
    pub affected_built: Vec<Id>,
    /// Machine-count retool estimate (hours): 0 for an all-◇ opportunity,
    /// `est_hours(Σ built machine count)` when built groups are involved.
    pub retool_est_hours: f64,
    /// A built factory in this opportunity still holds a node its refactored
    /// replacement would re-claim → the build window's downtime is unavoidable.
    pub node_reuse: bool,
}

/// Per-minute output of `item` from one machine of `r` (T2 arithmetic).
fn per_machine(r: &gamedata::docs::Recipe, item: &str) -> f64 {
    r.products
        .iter()
        .find(|(i, _)| i == item)
        .map(|(_, n)| n * 60.0 / r.duration_s)
        .unwrap_or(0.0)
}

/// Per-cycle output of `item` from `r` (for ingredient-rate scaling).
fn per_cycle(r: &gamedata::docs::Recipe, item: &str) -> f64 {
    r.products
        .iter()
        .find(|(i, _)| i == item)
        .map(|(_, n)| *n)
        .unwrap_or(1.0)
}

/// Can `factory` locally source `ing`? Mirrors `optimize_to_recipe`'s `source_of`
/// (per-factory, not empire-wide): a group in THIS factory whose recipe's primary
/// product is `ing`, or a boundary IN port on this factory carrying `ing`. This is
/// the exact predicate the ◇ T2 adopt route needs, so advertised savings for a
/// planned group only count when that same group could actually adopt the alt.
fn source_in_factory(state: &PlanState, gd: &GameData, factory: &Factory, ing: &str) -> bool {
    for gid in &factory.groups {
        if let Some(g) = state.groups.get(gid) {
            if let Some(r) = gd.recipes.get(&g.recipe) {
                if r.products.first().map(|(i, _)| i == ing).unwrap_or(false) {
                    return true;
                }
            }
        }
    }
    factory
        .ports
        .iter()
        .filter_map(|pid| state.ports.get(pid))
        .any(|p| p.direction == PortDirection::In && p.item == ing)
}

/// Empire-wide greedy per-item alternate-recipe ranking. Pure and read-only.
///
/// One opportunity per unlocked alternate recipe whose product is made somewhere
/// on a different recipe and whose adopt-everywhere delta saves machines. A
/// PLANNED group is counted only when the alt is locally sourceable in its
/// factory (so advertised savings == adoptable savings); built groups always
/// count (they re-source through `plan_replacement`). Ranked LEXICOGRAPHICALLY:
/// machines saved (desc), then retool hours (asc), then power saved (desc), then
/// input savings (desc), then recipe class — no cross-unit arithmetic.
pub fn empire_optimize(
    state: &PlanState,
    gd: &GameData,
    unlocked: &BTreeSet<String>,
) -> Vec<AltOpportunity> {
    let item_name = |class: &str| -> String {
        gd.items
            .get(class)
            .map(|i| i.display_name.clone())
            .unwrap_or_else(|| class.into())
    };

    let mut opps: Vec<AltOpportunity> = Vec::new();
    for alt in gd.recipes.values() {
        // Only UNLOCKED alternates are actionable now (W2b-B semantics).
        if !alt.alternate || !unlocked.contains(&alt.class_name) || alt.produced_in.is_empty() {
            continue;
        }
        let Some((product, _)) = alt.products.first() else {
            continue;
        };
        if product == POWER_ITEM || per_machine(alt, product) <= EPS {
            continue;
        }
        let alt_machine = alt.produced_in.first().cloned().unwrap_or_default();
        let alt_power = gamedata::db::recipe_power(gd, alt, &alt_machine);

        let mut machines_saved: i64 = 0;
        let mut power_saved = 0.0;
        let mut input_deltas: BTreeMap<String, f64> = BTreeMap::new();
        let mut affected_planned: Vec<Id> = Vec::new();
        let mut affected_built: Vec<Id> = Vec::new();
        let mut built_machine_count: u32 = 0;
        let mut built_factories: BTreeSet<&Id> = BTreeSet::new();

        for g in state.groups.values() {
            // Already on this alt, or not the group's primary product → skip.
            if g.recipe == alt.class_name {
                continue;
            }
            let Some(current) = gd.recipes.get(&g.recipe) else {
                continue;
            };
            if current.products.first().map(|(i, _)| i) != Some(product) {
                continue;
            }
            let count = g.effective_count();
            let clock = g.effective_clock();
            let cur_rate = per_machine(current, product) * count as f64 * clock;
            if cur_rate <= EPS {
                continue;
            }
            // Per-factory sourceability: a PLANNED group only counts toward the
            // advertised savings when it could actually adopt the alt in place —
            // i.e. every alt ingredient is locally sourceable in THIS factory (the
            // exact gate the ◇ T2 adopt route applies). Built groups keep counting
            // regardless: they route through `plan_replacement`, which re-solves a
            // fresh ◇ replacement and sources the feed freely. This keeps the
            // advertised savings == the adoptable savings (no phantom opportunity).
            if g.status != Status::Built {
                let sourceable_here = state
                    .factories
                    .get(&g.factory)
                    .map(|f| {
                        alt.ingredients
                            .iter()
                            .all(|(ing, _)| source_in_factory(state, gd, f, ing))
                    })
                    .unwrap_or(false);
                if !sourceable_here {
                    continue;
                }
            }
            let new_exact = cur_rate / per_machine(alt, product);
            let new_count = new_exact.ceil().max(1.0) as u32;

            machines_saved += count as i64 - new_count as i64;
            let cur_machine = current.produced_in.first().cloned().unwrap_or_default();
            power_saved += gamedata::db::recipe_power(gd, current, &cur_machine) * count as f64
                - alt_power * new_count as f64;

            // Honest input trade: alt intake − current intake, per ingredient.
            for (ing, n) in &current.ingredients {
                *input_deltas.entry(ing.clone()).or_default() -=
                    n * cur_rate / per_cycle(current, product);
            }
            for (ing, n) in &alt.ingredients {
                *input_deltas.entry(ing.clone()).or_default() +=
                    n * cur_rate / per_cycle(alt, product);
            }

            if g.status == Status::Built {
                affected_built.push(g.id.clone());
                built_machine_count += count;
                built_factories.insert(&g.factory);
            } else {
                affected_planned.push(g.id.clone());
            }
        }

        // Only surface a net win (the whole point — do not nag with a wash/loss).
        if machines_saved <= 0 {
            continue;
        }
        affected_planned.sort();
        affected_built.sort();

        // Retool estimate: ◇ planned retool ≈ free; ◆ built costs the labeled
        // machine-count downtime estimate. The expensive scratch-solve is
        // deferred to the per-row drill-down (Session::cutover_plan).
        let retool_est_hours = if built_machine_count > 0 {
            crate::cutover::est_hours(built_machine_count)
        } else {
            0.0
        };
        // node_reuse: any affected ◆ factory still holds a node claim its
        // refactored replacement would re-claim during the build window.
        let node_reuse = built_factories
            .iter()
            .filter_map(|fid| state.factories.get(*fid))
            .any(|f| !f.node_claims.is_empty());

        let input_deltas: Vec<(String, f64)> = input_deltas
            .into_iter()
            .filter(|(_, v)| v.abs() > EPS)
            .collect();

        opps.push(AltOpportunity {
            recipe: alt.class_name.clone(),
            recipe_name: alt.display_name.clone(),
            product: product.clone(),
            product_name: item_name(product),
            machines_saved,
            power_saved_mw: power_saved,
            input_deltas,
            affected_planned,
            affected_built,
            retool_est_hours,
            node_reuse,
        });
    }

    // Rank LEXICOGRAPHICALLY (no unit-mismatched arithmetic — machines and hours
    // are different units): most machines saved first, then the cheapest retool,
    // then most power saved, then least added intake, then recipe class for a
    // deterministic tie-break.
    opps.sort_by(|a, b| {
        let input_savings =
            |o: &AltOpportunity| -o.input_deltas.iter().map(|(_, v)| *v).sum::<f64>();
        b.machines_saved
            .cmp(&a.machines_saved)
            .then_with(|| {
                a.retool_est_hours
                    .partial_cmp(&b.retool_est_hours)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                b.power_saved_mw
                    .partial_cmp(&a.power_saved_mw)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                input_savings(b)
                    .partial_cmp(&input_savings(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.recipe.cmp(&b.recipe))
    });
    opps
}

/// Draft a T2-style adopt proposal for one target alternate across every ◇
/// PLANNED group making its product on a different recipe. `SetGroupRecipe` is
/// legal on planned groups, so this is the all-◇ CTA route; a ◆ built group is
/// NEVER touched here (it routes through `Session::plan_replacement` instead).
///
/// Mirrors `wizard::t2_optimize`'s per-group rewire (drop the old inbound feeds,
/// belt the alt's ingredients from an in-factory source) but pins the target
/// recipe instead of scoring for the best one. Returns a Draft proposal of
/// MODIFY items, or None when no planned group can adopt the alt.
pub fn optimize_to_recipe(
    state: &PlanState,
    gd: &GameData,
    unlocked: &BTreeSet<String>,
    recipe: &str,
) -> Option<Proposal> {
    let alt = gd.recipes.get(recipe)?;
    // Only adopt an already-available recipe (a standard recipe or an unlocked
    // alternate); a genuinely-locked alternate is a suggestion, never an action.
    if alt.alternate && !unlocked.contains(&alt.class_name) {
        return None;
    }
    let (product, _) = alt.products.first()?;
    let item_name = |class: &str| -> String {
        gd.items
            .get(class)
            .map(|i| i.display_name.clone())
            .unwrap_or_else(|| class.into())
    };
    let tier_for = |rate: f64| -> u8 { (1..=6u8).find(|t| belt_capacity(*t) >= rate).unwrap_or(6) };

    let mut items: Vec<ProposalItem> = Vec::new();
    let mut goal: Vec<(String, f64)> = Vec::new();
    for factory in state.factories.values() {
        // In-factory source for an ingredient (a producing group's primary
        // product or a boundary IN port) — T2 rewires feeds, never invents them.
        let source_of = |item: &str| -> Option<EdgeEnd> {
            for gid in &factory.groups {
                let g = state.groups.get(gid)?;
                let r = gd.recipes.get(&g.recipe)?;
                if r.products.first().map(|(i, _)| i == item).unwrap_or(false) {
                    return Some(EdgeEnd::Group(gid.clone()));
                }
            }
            factory
                .ports
                .iter()
                .filter_map(|pid| state.ports.get(pid))
                .find(|p| p.direction == PortDirection::In && p.item == item)
                .map(|p| EdgeEnd::Port(p.id.clone()))
        };

        for gid in &factory.groups {
            let Some(group) = state.groups.get(gid) else {
                continue;
            };
            // ◇ planned only — a ◆ built swap routes through plan_replacement.
            if group.status == Status::Built || group.recipe == alt.class_name {
                continue;
            }
            let Some(current) = gd.recipes.get(&group.recipe) else {
                continue;
            };
            if current.products.first().map(|(i, _)| i) != Some(product) {
                continue;
            }
            // Every alt ingredient must already be sourceable in this factory.
            if !alt
                .ingredients
                .iter()
                .all(|(ing, _)| source_of(ing).is_some())
            {
                continue;
            }
            let cur_rate = per_machine(current, product) * group.count as f64 * group.clock;
            if per_machine(alt, product) <= EPS || cur_rate <= EPS {
                continue;
            }
            let new_exact = cur_rate / per_machine(alt, product);
            let new_count = new_exact.ceil().max(1.0) as u32;
            let new_clock = (new_exact / new_count as f64).clamp(0.01, 2.5);
            let machine_class = alt.produced_in.first().cloned().unwrap_or_default();

            let mut cmds = vec![
                Command::SetGroupRecipe {
                    id: gid.clone(),
                    machine: machine_class,
                    recipe: alt.class_name.clone(),
                },
                Command::SetGroupCount {
                    id: gid.clone(),
                    count: new_count,
                },
                Command::SetGroupClock {
                    id: gid.clone(),
                    clock: new_clock,
                },
            ];
            for e in state.edges.values() {
                if e.factory == factory.id && e.to == EdgeEnd::Group(gid.clone()) {
                    cmds.push(Command::DeleteEdge { id: e.id.clone() });
                }
            }
            let cycles_per_min = cur_rate / per_cycle(alt, product);
            for (ing, n) in &alt.ingredients {
                let Some(src) = source_of(ing) else { continue };
                cmds.push(Command::AddEdge {
                    factory: factory.id.clone(),
                    from: src,
                    to: EdgeEnd::Group(gid.clone()),
                    item: ing.clone(),
                    tier: tier_for(n * cycles_per_min),
                });
            }
            let aliases = vec![None; cmds.len()];
            items.push(ProposalItem {
                id: new_id(),
                kind: ProposalItemKind::Modify,
                included: true,
                label: format!(
                    "Δ {} → {}",
                    item_name(product).to_uppercase(),
                    alt.display_name
                ),
                detail: format!(
                    "{} · ×{} @ {:.0}% → ×{} @ {:.0}%",
                    factory.name,
                    group.count,
                    group.clock * 100.0,
                    new_count,
                    new_clock * 100.0,
                ),
                impact: format!("−{} MACHINES", group.count.saturating_sub(new_count)),
                commands: cmds,
                aliases,
                depends_on: vec![],
                sync: None,
            });
            goal.push((product.clone(), cur_rate));
        }
    }

    if items.is_empty() {
        return None;
    }
    Some(Proposal {
        id: String::new(),
        source: ProposalSource::T2Optimize,
        title: format!("ADOPT {} EMPIRE-WIDE", alt.display_name.to_uppercase()),
        goal,
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: String::new(),
        input_hash: String::new(),
        provenance: "ALT OPTIMIZER".into(),
        items,
        milestone: None,
    })
}
