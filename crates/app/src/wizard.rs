//! Global solver (SDD §5.5): goal → reviewable Proposal, in four streamed
//! phases — demand graph, recipe selection, siting, routing (+ the A2.4 power
//! sourcing pass). Pure over cloned inputs so it can run off-thread and be
//! cancelled; the caller stores the result via `CreateProposal`.
//!
//! Honest scoping for the fixture-scale catalog: recipe selection scores each
//! item's candidates independently (min machines, then min power). True
//! cross-item MILP arrives when alternate recipes create real trade-offs.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

use gamedata::docs::{extraction_rate, GameData, POWER_ITEM};
use gamedata::worldnodes::WorldSnapshot;
use planner_core::commands::Command;
use planner_core::entities::*;
use planner_core::proposals::*;
use planner_core::state::PlanState;
use serde::{Deserialize, Serialize};

const WIZARD_EXTRACTOR: &str = "Build_MinerMk2_C";

/// Extractor the wizard claims a node of `item` with. Fluid nodes take their
/// dedicated pump — the wizard must never stamp a miner on crude oil (twin of
/// the renderer's maputil::extractorsFor on the manual claim path).
fn extractor_for(item: &str) -> &'static str {
    match item {
        "Desc_LiquidOil_C" => "Build_OilPump_C",
        _ => WIZARD_EXTRACTOR,
    }
}

/// Hard ceiling on demand-expansion steps AND on the resolver's backtracking
/// budget — a pure combinatorial backstop, never a normal exit. The deepest
/// legitimate real-catalog expansion (nuclear, alternates included) pops ≈994
/// queue entries, an order of magnitude under this; recipe cycles are caught
/// by the backtracking resolver before they can spin the queue. Hitting the
/// cap therefore means non-converged demand by construction, and the solve
/// returns an honest Infeasible instead of staging garbage.
const EXPANSION_CAP: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardConstraints {
    /// Consume existing unbound overproduction before proposing new machines.
    pub surplus_first: bool,
    pub max_new_sites: u32,
    /// Max new node claims (fuel claims count too — logged when they do).
    pub node_budget: u32,
    /// Minimum node purity: "impure" accepts anything.
    pub purity_floor: String,
    /// Minimum circuit headroom fraction left after the proposal (A2.4).
    pub power_margin_cap: f64,
    /// 0 = greenfield, 1 = expand existing factories where possible.
    pub expand_preference: f64,
    /// ALSO consider LOCKED alternates (as suggestions). Unlocked alternates
    /// (in the save's unlocked set) are always available and used like standard
    /// recipes; this toggle only re-scopes to pull in the genuinely-locked ones,
    /// which stay flagged "NOT UNLOCKED — suggested".
    #[serde(default)]
    pub include_alternates: bool,
}

impl Default for WizardConstraints {
    fn default() -> Self {
        Self {
            surplus_first: true,
            max_new_sites: 2,
            node_budget: 3,
            purity_floor: "impure".into(),
            power_margin_cap: 0.05,
            expand_preference: 0.5,
            include_alternates: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WizardGoal {
    /// (item class, rate/min), e.g. [("Desc_ModularFrame_C", 8.0)].
    pub items: Vec<(String, f64)>,
    #[serde(default)]
    pub constraints: WizardConstraints,
    /// Total-quantity goal mode: carried through global_solve into the
    /// Proposal untouched (no solver behaviour changes — a target annotation).
    #[serde(default)]
    pub milestone: Option<Milestone>,
    /// Product class → pinned recipe class. When a product is pinned, the solver
    /// adopts that exact recipe for it (bypassing cost scoring) instead of the
    /// cheapest one — e.g. a built-factory "adopt this alt" seeds the retired
    /// product's alternate here so its ◇ replacement is solved onto that recipe.
    /// Empty (the default) leaves recipe selection behaviour-identical.
    #[serde(default)]
    pub pinned_recipes: BTreeMap<String, String>,
}

/// Infeasible ≠ dead end (mock 5c): best achievable + named binding + one-tap
/// relaxations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Infeasible {
    pub best_rate: f64,
    pub binding: String,
    pub relaxations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum WizardOutcome {
    Proposal { proposal: Proposal },
    Infeasible(Infeasible),
    Cancelled,
}

/// One production stage of the demand graph.
struct Stage {
    item: String,
    rate: f64,
    recipe: String,
    machine: String,
    count: u32,
    clock: f64,
    power_mw: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn global_solve(
    state: &PlanState,
    gd: &GameData,
    world: &WorldSnapshot,
    goal: &WizardGoal,
    unlocked: &BTreeSet<String>,
    plan_hash: String,
    snapshot_time: String,
    mut log: impl FnMut(&str, &str),
    cancel: &AtomicBool,
) -> WizardOutcome {
    let c = &goal.constraints;
    let pinned = &goal.pinned_recipes;
    let item_name = |class: &str| -> String {
        gd.items
            .get(class)
            .map(|i| i.display_name.clone())
            .unwrap_or_else(|| class.into())
    };

    // ---------- phase 1: demand graph ----------
    let phase = "DEMAND GRAPH";
    let mut demand: BTreeMap<String, f64> = BTreeMap::new(); // produced items
    let mut raw: BTreeMap<String, f64> = BTreeMap::new(); // extracted items
    let mut surplus_taken: Vec<(Id, String, f64)> = Vec::new(); // (out port, item, rate)
    let mut queue: Vec<(String, f64)> = goal.items.clone();

    // unbound out ports are consumable surplus (one route per port)
    let mut surplus: BTreeMap<String, Vec<(Id, f64)>> = BTreeMap::new();
    if c.surplus_first {
        for p in state.ports.values() {
            if p.direction == PortDirection::Out && p.bound_route.is_none() && p.rate > 0.0 {
                surplus
                    .entry(p.item.clone())
                    .or_default()
                    .push((p.id.clone(), p.rate));
            }
        }
    }

    // One backtracking resolver serves BOTH phases: picks memoized here in the
    // demand walk are the exact picks phase 2 stages, so staging is always
    // consistent with what was expanded.
    let mut resolver = RecipeResolver::new(gd, unlocked, c.include_alternates, pinned);

    let mut expansion_steps = 0usize;
    while let Some((item, mut rate)) = queue.pop() {
        if cancel.load(Ordering::Relaxed) {
            return WizardOutcome::Cancelled;
        }
        expansion_steps += 1;
        if expansion_steps > EXPANSION_CAP {
            // Backstop only (see EXPANSION_CAP): accumulated demand is
            // non-converged garbage by construction, so refuse it honestly.
            let binding = format!(
                "expansion did not converge after {EXPANSION_CAP} steps (while expanding {})",
                item_name(&item)
            );
            log(phase, &format!("INFEASIBLE — {binding}"));
            return WizardOutcome::Infeasible(Infeasible {
                best_rate: 0.0,
                binding,
                relaxations: vec![format!("pin a recipe for {}", item_name(&item))],
            });
        }
        if item == POWER_ITEM {
            continue; // power is sourced in phase 4 (A2.4), not belted
        }
        // surplus first — goal items keep their full rate (the goal is NEW production)
        let is_goal = goal.items.iter().any(|(i, _)| i == &item);
        if !is_goal {
            if let Some(offers) = surplus.get_mut(&item) {
                while rate > 1e-9 {
                    let Some((port, avail)) = offers.iter_mut().find(|(_, a)| *a > 1e-9) else {
                        break;
                    };
                    let take = rate.min(*avail);
                    *avail -= take;
                    rate -= take;
                    // one route per port: merge repeat takes from the same
                    // port (an item demanded by two stages is popped twice)
                    // so phase 4 emits a single AddPort+AddRoute pair for it
                    if let Some(i) = surplus_taken.iter().position(|(p, _, _)| p == port) {
                        surplus_taken[i].2 += take;
                    } else {
                        surplus_taken.push((port.clone(), item.clone(), take));
                    }
                    log(
                        phase,
                        &format!(
                            "surplus: {} {:.1}/min from existing overproduction",
                            item_name(&item),
                            take
                        ),
                    );
                }
            }
        }
        if rate <= 1e-9 {
            continue;
        }
        let extractable = world.nodes.iter().any(|n| n.item == item);
        // World-sourced raws (FGResourceDescriptor) are raw even without a map
        // node — water/nitrogen come from extractors the world snapshot doesn't
        // model. Without this, the real catalog offers Unpackage Water as
        // water's "producer" and the Package↔Unpackage pair recurses forever.
        let is_resource = gd.items.get(&item).map(|i| i.is_resource).unwrap_or(false);
        let craftable = gd.recipes.values().any(|r| {
            !r.produced_in.is_empty()
                && r.products.iter().any(|(i, _)| i == &item)
                && r.products
                    .first()
                    .map(|(i, _)| i != POWER_ITEM)
                    .unwrap_or(true)
        });
        if extractable || is_resource || !craftable {
            *raw.entry(item.clone()).or_default() += rate;
            log(phase, &format!("raw: {} {:.1}/min", item_name(&item), rate));
            continue;
        }
        *demand.entry(item.clone()).or_default() += rate;
        // expand via the resolver's pick; phase 2 stages the SAME memoized pick
        let picked = resolver.resolve(&item);
        for note in resolver.take_notes() {
            log(phase, &note);
        }
        if let Some(r) = picked {
            let out_per_cycle = r
                .products
                .iter()
                .find(|(i, _)| i == &item)
                .map(|(_, n)| *n)
                .unwrap_or(1.0);
            let cycles_per_min = rate / out_per_cycle;
            for (ing, n) in &r.ingredients {
                queue.push((ing.clone(), n * cycles_per_min));
            }
            log(
                phase,
                &format!("{} {:.1}/min ← {}", item_name(&item), rate, r.display_name),
            );
        } else {
            if resolver.exhausted {
                // Same shape as the queue cap: the resolver's backtracking
                // budget ran out, so nothing it reported is trustworthy.
                let binding = format!(
                    "expansion did not converge after {EXPANSION_CAP} steps (while expanding {})",
                    item_name(&item)
                );
                log(phase, &format!("INFEASIBLE — {binding}"));
                return WizardOutcome::Infeasible(Infeasible {
                    best_rate: 0.0,
                    binding,
                    relaxations: vec![format!("pin a recipe for {}", item_name(&item))],
                });
            }
            if is_goal && resolver.has_candidates(&item) {
                // Cycle-blocked GOAL (distinct from no-producer: candidates
                // exist, but every chain re-enters itself). A fresh
                // include_alts=true probe decides whether an alternate recipe
                // is the escape hatch worth naming.
                let alt_escape = RecipeResolver::new(gd, unlocked, true, pinned)
                    .resolve(&item)
                    .is_some();
                let (binding, relaxations) = if alt_escape {
                    (
                        format!(
                            "every unlocked recipe chain for {} loops back on itself — an alternate recipe breaks the loop",
                            item_name(&item)
                        ),
                        vec![
                            "enable alternate recipes ✓".to_string(),
                            format!("pin a recipe for {}", item_name(&item)),
                        ],
                    )
                } else {
                    (
                        format!(
                            "recipe cycle: every recipe producing {} consumes it downstream",
                            item_name(&item)
                        ),
                        vec![format!("pin a recipe for {}", item_name(&item))],
                    )
                };
                log(phase, &format!("INFEASIBLE — {binding}"));
                return WizardOutcome::Infeasible(Infeasible {
                    best_rate: 0.0,
                    binding,
                    relaxations,
                });
            }
            // Zero-candidate items keep today's degradation exactly: the
            // demand entry stays, phase 2 names the fix for goals (locked
            // alternates / no recipe at all) and mid-chain items surface as
            // T1 `Disconnected` shortfalls. A cycle-blocked MID-CHAIN item
            // cannot reach here: the goal resolving means its entire memo
            // tree resolved, and only resolved recipes enqueue ingredients.
        }
    }

    // ---------- phase 2: recipe selection ----------
    let phase = "RECIPE SELECTION";
    let mut stages: Vec<Stage> = Vec::new();
    for (item, rate) in &demand {
        if cancel.load(Ordering::Relaxed) {
            return WizardOutcome::Cancelled;
        }
        // Same resolver instance as phase 1: memo hits guarantee the staged
        // pick is the one the demand walk expanded.
        let picked = resolver.resolve(item);
        for note in resolver.take_notes() {
            log(phase, &note);
        }
        let Some(r) = picked else {
            // A GOAL item with no pickable recipe can never be staged — the
            // proposal would dangle a `$g.` alias and accept would always
            // fail. Name the fix instead of emitting a broken proposal.
            // (Non-goal mid-chain items keep degrading honestly via the
            // ingredient-edge guard + T1 `Disconnected` shortfalls.)
            if goal.items.iter().any(|(i, _)| i == item) {
                // include_alts=true resolver probe (pick_recipe is exactly
                // that): does relaxing to locked alternates make it makeable?
                let (binding, relaxations) = if !c.include_alternates
                    && pick_recipe(gd, item, unlocked, true, pinned).is_some()
                {
                    (
                        format!(
                            "only locked alternate recipes produce {} (not unlocked)",
                            item_name(item)
                        ),
                        vec!["enable alternate recipes ✓".to_string()],
                    )
                } else {
                    (
                        format!("no usable recipe produces {}", item_name(item)),
                        vec![format!(
                            "pick another goal item — {} has no enabled recipe",
                            item_name(item)
                        )],
                    )
                };
                log(phase, &format!("INFEASIBLE — {binding}"));
                return WizardOutcome::Infeasible(Infeasible {
                    best_rate: 0.0,
                    binding,
                    relaxations,
                });
            }
            continue;
        };
        let machine_class = r.produced_in.first().cloned().unwrap_or_default();
        let machine = gd.machines.get(&machine_class);
        let per_machine = r
            .products
            .iter()
            .find(|(i, _)| i == item)
            .map(|(_, n)| n * 60.0 / r.duration_s)
            .unwrap_or(1.0);
        let exact = rate / per_machine;
        let count = exact.ceil().max(1.0) as u32;
        let clock = (exact / count as f64).clamp(0.01, 2.5);
        let alts = gd
            .recipes
            .values()
            .filter(|q| q.products.iter().any(|(i, _)| i == item) && !q.produced_in.is_empty())
            .count();
        log(
            phase,
            &format!(
                "{}: {} ×{} @ {:.0}% ({} candidate{})",
                item_name(item),
                r.display_name,
                count,
                clock * 100.0,
                alts,
                if alts == 1 { "" } else { "s" }
            ),
        );
        // Clock-scaled like the solver (power ∝ clock^1.321928) — nameplate ×
        // count alone overstates the impact of every underclocked stage.
        let power_mw = machine
            .map(|_| {
                gamedata::db::recipe_power(gd, r, &machine_class)
                    * count as f64
                    * clock.powf(solver::model::POWER_EXPONENT)
            })
            .unwrap_or(0.0);
        stages.push(Stage {
            item: item.clone(),
            rate: *rate,
            recipe: r.class_name.clone(),
            machine: machine_class,
            count,
            clock,
            power_mw,
        });
    }

    // A world-sourced raw (FGResourceDescriptor) with NO node in the world
    // snapshot — water today, nitrogen and other well gases until wells land —
    // cannot be claimed or metered. Its supply is ASSUMED: no claim, no in
    // port, no feed edge, no goal wiring. Generalizes the old hardcoded
    // Desc_Water_C checks (water is exactly a resource with zero nodes, so its
    // behavior is unchanged byte-for-byte).
    let supply_assumed = |item: &str| -> bool {
        gd.items.get(item).map(|i| i.is_resource).unwrap_or(false)
            && !world.nodes.iter().any(|n| n.item == item)
    };

    // ---------- phase 3: siting ----------
    let phase = "SITING";
    let claimed: std::collections::BTreeSet<&str> = state
        .node_claims
        .values()
        .map(|c| c.node.as_str())
        .collect();
    let purity_rank = |p: &str| match p {
        "pure" => 2,
        "normal" => 1,
        _ => 0,
    };
    let floor = purity_rank(&c.purity_floor);

    // pick nodes per raw item, best purity first, clustered near first pick
    let mut picked_nodes: Vec<(String, String, f64)> = Vec::new(); // (node id, item, extraction rate — per-item extractor via extractor_for)
    let mut anchor: Option<(f64, f64)> = None;
    // Running sum of picked node coordinates → the site lands on their centroid
    // (close to every resource it draws), not just the first pick + a fixed
    // offset (which could shove it off an edge node into out-of-bounds space).
    let mut node_sum = (0.0f64, 0.0f64);
    let mut budget = c.node_budget;
    let mut binding: Option<String> = None;
    for (item, need) in &raw {
        if supply_assumed(item) {
            log(
                phase,
                &format!(
                    "{}: supply assumed — no {} nodes in the world snapshot (unmetered until wells land)",
                    item_name(item),
                    item_name(item)
                ),
            );
            continue;
        }
        let mut candidates: Vec<&gamedata::worldnodes::WorldNode> = world
            .nodes
            .iter()
            .filter(|n| {
                &n.item == item
                    && !claimed.contains(n.id.as_str())
                    && purity_rank(&n.purity) >= floor
            })
            .collect();
        candidates.sort_by(|a, b| {
            let d = |n: &gamedata::worldnodes::WorldNode| {
                anchor
                    .map(|(x, y)| ((n.x - x).powi(2) + (n.y - y).powi(2)).sqrt())
                    .unwrap_or(0.0)
            };
            (purity_rank(&b.purity), d(a) as i64).cmp(&(purity_rank(&a.purity), d(b) as i64))
        });
        let mut covered = 0.0;
        for n in candidates {
            if covered >= *need - 1e-9 {
                break;
            }
            if budget == 0 {
                binding = Some(format!(
                    "node budget ({}) reached on {}",
                    c.node_budget,
                    item_name(item)
                ));
                break;
            }
            let machine = gd.machines.get(extractor_for(item));
            let rate = machine
                .map(|m| extraction_rate(m, &n.purity, 1.0))
                .unwrap_or(0.0);
            covered += rate;
            budget -= 1;
            if anchor.is_none() {
                anchor = Some((n.x, n.y));
            }
            node_sum.0 += n.x;
            node_sum.1 += n.y;
            picked_nodes.push((n.id.clone(), item.clone(), rate));
            log(
                phase,
                &format!(
                    "claim {} ({} {}) → {:.0}/min",
                    n.id,
                    item_name(item),
                    n.purity,
                    rate
                ),
            );
        }
        if covered < *need - 1e-9 {
            let short = need - covered;
            let best_fraction = if *need > 0.0 { covered / need } else { 1.0 };
            let best_rate = goal
                .items
                .first()
                .map(|(_, r)| r * best_fraction)
                .unwrap_or(0.0);
            let binding = binding.unwrap_or_else(|| {
                format!(
                    "{} extraction short {:.0}/min (no eligible nodes left)",
                    item_name(item),
                    short
                )
            });
            log(phase, &format!("INFEASIBLE — {binding}"));
            let mut relaxations = vec![format!(
                "allow {} more node claim(s) → {:.1}/min ✓",
                ((short / 120.0).ceil() as u32).max(1),
                goal.items.first().map(|(_, r)| *r).unwrap_or(0.0)
            )];
            if floor > 0 {
                relaxations.push("lower the purity floor to IMPURE".into());
            }
            relaxations.push(format!("accept best achievable {best_rate:.1}/min"));
            return WizardOutcome::Infeasible(Infeasible {
                best_rate,
                binding,
                relaxations,
            });
        }
    }

    // Place the site on the centroid of the nodes it claims — close to every
    // resource it draws — nudged slightly off the cluster so the pin doesn't
    // cover a node marker, then CLAMPED inside the map so it can never land
    // out of bounds (an edge node + offset, or a stray save-only coordinate).
    let raw_site = if picked_nodes.is_empty() {
        // No metered raws (everything supply-assumed): fall back to the existing
        // empire's centroid, else the map center — never a blind (0,0).
        empire_centroid(state).unwrap_or((
            (world.bounds.min_x + world.bounds.max_x) / 2.0,
            (world.bounds.min_y + world.bounds.max_y) / 2.0,
        ))
    } else {
        let n = picked_nodes.len() as f64;
        (node_sum.0 / n + 120.0, node_sum.1 / n + 120.0)
    };
    let site_pos = clamp_to_bounds(raw_site, &world.bounds);
    let goal_name = goal
        .items
        .first()
        .map(|(i, _)| item_name(i))
        .unwrap_or_default();
    // Multi-output goals (◆ replacements) name the site after EVERYTHING it
    // ships — a single-item goal reads exactly as before.
    let site_name = format!(
        "{} WORKS",
        goal.items
            .iter()
            .map(|(i, _)| item_name(i))
            .collect::<Vec<_>>()
            .join(" + ")
            .to_uppercase()
    );
    log(
        phase,
        &format!(
            "site: {} @ ({:.0}, {:.0}) — {} stage{}, {} claim{}",
            site_name,
            site_pos.x,
            site_pos.y,
            stages.len(),
            if stages.len() == 1 { "" } else { "s" },
            picked_nodes.len(),
            if picked_nodes.len() == 1 { "" } else { "s" }
        ),
    );

    // ---------- build the CREATE item ----------
    let mut items: Vec<ProposalItem> = Vec::new();
    let mut cmds: Vec<Command> = Vec::new();
    let mut aliases: Vec<Option<String>> = Vec::new();
    let push = |cmds: &mut Vec<Command>,
                aliases: &mut Vec<Option<String>>,
                cmd: Command,
                alias: Option<&str>| {
        cmds.push(cmd);
        aliases.push(alias.map(String::from));
    };

    push(
        &mut cmds,
        &mut aliases,
        Command::CreateFactory {
            name: site_name.clone(),
            position: site_pos,
            region: nearest_region(world, site_pos),
        },
        Some("site"),
    );

    // in ports per raw item (ceiling = claimed extraction), out port per goal
    let mut y = 80.0;
    for (item, need) in &raw {
        if supply_assumed(item) {
            continue; // assumed raws get no in port — nothing meters them
        }
        let ceiling: f64 = picked_nodes
            .iter()
            .filter(|(_, i, _)| i == item)
            .map(|(_, _, r)| r)
            .sum();
        push(
            &mut cmds,
            &mut aliases,
            Command::AddPort {
                factory: "$site".into(),
                direction: PortDirection::In,
                item: item.clone(),
                rate: 0.0,
                rate_ceiling: Some(ceiling.max(*need)),
                graph_pos: GraphPos { x: 0.0, y },
            },
            Some(&format!("in.{item}")),
        );
        y += 128.0;
    }
    // EVERY goal item ships: one OUT port each (a multi-output replacement
    // that only shipped its first item would silently starve the second
    // item's consumers after the dismantle).
    let goal_rate = goal.items.first().map(|(_, r)| *r).unwrap_or(0.0);
    for (i, (item, _)) in goal.items.iter().enumerate() {
        push(
            &mut cmds,
            &mut aliases,
            Command::AddPort {
                factory: "$site".into(),
                direction: PortDirection::Out,
                item: item.clone(),
                rate: 0.0,
                rate_ceiling: None,
                graph_pos: GraphPos {
                    x: 1400.0,
                    y: 200.0 + 128.0 * i as f64,
                },
            },
            Some(&format!("out.{item}")),
        );
    }

    // stage groups laid out by depth, then edges along the recipe graph
    let mut total_mw = 0.0;
    for (i, st) in stages.iter().enumerate() {
        push(
            &mut cmds,
            &mut aliases,
            Command::AddGroup {
                factory: "$site".into(),
                machine: st.machine.clone(),
                recipe: st.recipe.clone(),
                count: st.count,
                clock: st.clock,
                graph_pos: GraphPos {
                    x: 280.0 + 300.0 * (i as f64 % 4.0),
                    y: 80.0 + 260.0 * (i as f64 / 4.0).floor(),
                },
                floor: 0,
            },
            Some(&format!("g.{}", st.item)),
        );
        total_mw += st.power_mw;
    }
    let tier_for = |rate: f64| -> u8 {
        for t in 1..=6u8 {
            if belt_capacity(t) >= rate {
                return t;
            }
        }
        6
    };
    for st in &stages {
        let Some(r) = gd.recipes.get(&st.recipe) else {
            continue;
        };
        for (ing, n) in &r.ingredients {
            let rate = n
                * (st.rate
                    / r.products
                        .iter()
                        .find(|(i, _)| i == &st.item)
                        .map(|(_, m)| *m)
                        .unwrap_or(1.0));
            let from = if raw.contains_key(ing) {
                if supply_assumed(ing) {
                    continue; // no in port exists for an assumed raw
                }
                EdgeEnd::Port(format!("$in.{ing}"))
            } else if stages.iter().any(|s| &s.item == ing) {
                EdgeEnd::Group(format!("$g.{ing}"))
            } else {
                continue; // consumed from surplus via route — enters as a port later
            };
            push(
                &mut cmds,
                &mut aliases,
                Command::AddEdge {
                    factory: "$site".into(),
                    from,
                    to: EdgeEnd::Group(format!("$g.{}", st.item)),
                    item: ing.clone(),
                    tier: tier_for(rate),
                },
                None,
            );
        }
    }
    // Each out port's feed depends on how its item is produced: a stage group
    // when one exists, else the raw in port (extraction-and-ship — the claims
    // feed the in port, which feeds the out port). A supply-assumed raw
    // (water, well gases) has no in port: leave that out port unwired — an
    // honest T1 shortfall, not a dangling `$g.` alias that would roll back
    // the whole accept.
    for (item, rate) in &goal.items {
        let from = if stages.iter().any(|s| &s.item == item) {
            Some(EdgeEnd::Group(format!("$g.{item}")))
        } else if raw.contains_key(item) && !supply_assumed(item) {
            Some(EdgeEnd::Port(format!("$in.{item}")))
        } else {
            None
        };
        if let Some(from) = from {
            push(
                &mut cmds,
                &mut aliases,
                Command::AddEdge {
                    factory: "$site".into(),
                    from,
                    to: EdgeEnd::Port(format!("$out.{item}")),
                    item: item.clone(),
                    tier: tier_for(*rate),
                },
                None,
            );
        }
        push(
            &mut cmds,
            &mut aliases,
            Command::SetPortRate {
                id: format!("$out.{item}"),
                rate: *rate,
            },
            None,
        );
    }

    let create_id = new_id();
    let machines_total: u32 = stages.iter().map(|s| s.count).sum();
    items.push(ProposalItem {
        id: create_id.clone(),
        kind: ProposalItemKind::Create,
        included: true,
        label: format!("+ {site_name} — NEW"),
        detail: format!(
            "{} stage{} · {} machine{} · {}",
            stages.len(),
            if stages.len() == 1 { "" } else { "s" },
            machines_total,
            if machines_total == 1 { "" } else { "s" },
            goal.items
                .iter()
                .map(|(i, r)| format!("{} {:.1}/min", item_name(i), r))
                .collect::<Vec<_>>()
                .join(" + ")
        ),
        impact: format!("+{total_mw:.0} MW"),
        commands: cmds,
        aliases,
        depends_on: vec![],
        sync: None,
        conflict: None,
    });

    // CLAIM items (each excludable; factory alias binds them to the site)
    for (node, item, rate) in &picked_nodes {
        items.push(ProposalItem {
            id: new_id(),
            kind: ProposalItemKind::Claim,
            included: true,
            label: format!("◉ CLAIM {}", node.to_uppercase()),
            detail: format!(
                "{} · {} · {:.0}/min",
                item_name(item),
                gd.machines
                    .get(extractor_for(item))
                    .map(|m| m.display_name.as_str())
                    .unwrap_or(extractor_for(item)),
                rate
            ),
            impact: "FREE ✓".into(),
            commands: vec![Command::ClaimNode {
                factory: "$site".into(),
                node: node.clone(),
                extractor: extractor_for(item).into(),
                clock: 1.0,
            }],
            aliases: vec![None],
            depends_on: vec![create_id.clone()],
            sync: None,
            conflict: None,
        });
    }

    // ---------- phase 4: routing + power sourcing (A2.4) ----------
    let phase = "ROUTING";
    // deliver EACH goal item to an existing unbound IN port if one exists —
    // a port can bind only one route, so claim each candidate port once.
    let mut routed_ports: std::collections::BTreeSet<Id> = std::collections::BTreeSet::new();
    for (item, rate) in &goal.items {
        let Some(consumer) = state.ports.values().find(|p| {
            p.direction == PortDirection::In
                && &p.item == item
                && p.bound_route.is_none()
                && !routed_ports.contains(&p.id)
        }) else {
            continue;
        };
        routed_ports.insert(consumer.id.clone());
        let item_label = item_name(item);
        let dst = state.factories.get(&consumer.factory);
        let dst_name = dst.map(|f| f.name.clone()).unwrap_or_default();
        let dst_pos = dst.map(|f| f.position).unwrap_or(MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        // A3.3: pick the transport by distance/rate thresholds
        let dist = ((dst_pos.x - site_pos.x).powi(2) + (dst_pos.y - site_pos.y).powi(2)).sqrt();
        let picked = planner_core::transport::pick_transport(dist, *rate);
        let route_kind = match picked {
            "rail" => RouteKind::Rail {
                spec: RailSpec::default(),
            },
            "drone" => RouteKind::Drone {
                spec: DroneSpec::default(),
            },
            _ => RouteKind::Belt {
                tier: tier_for(*rate),
            },
        };
        log(
            phase,
            &format!(
                "route: {} ⟶ {} ({} {:.1}/min · {} — {:.1} km)",
                site_name,
                dst_name,
                item_label,
                rate,
                picked.to_uppercase(),
                dist / 1000.0
            ),
        );
        items.push(ProposalItem {
            id: new_id(),
            kind: ProposalItemKind::RouteAdd,
            included: true,
            label: format!("⟶ {} ⟶ {}", site_name, dst_name.to_uppercase()),
            detail: format!("{} {:.1}/min · MK.{}", item_label, rate, tier_for(*rate)),
            impact: format!("proj {:.0}%", 100.0 * rate / belt_capacity(tier_for(*rate))),
            commands: vec![Command::AddRoute {
                kind: route_kind,
                from: format!("$out.{item}"),
                to: consumer.id.clone(),
                path: vec![site_pos, dst_pos],
            }],
            aliases: vec![None],
            depends_on: vec![create_id.clone()],
            sync: None,
            conflict: None,
        });
    }
    // routes consuming surplus from existing factories into the new site —
    // the new IN port must also be belted to every stage that eats the item,
    // or the site graph solves to zero.
    for (port, item, take) in &surplus_taken {
        let src = state
            .ports
            .get(port)
            .and_then(|p| state.factories.get(&p.factory));
        let src_name = src.map(|f| f.name.clone()).unwrap_or_default();
        let src_pos = src.map(|f| f.position).unwrap_or(MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        let alias = format!("surplus.{item}");
        let mut route_cmds = vec![
            Command::AddPort {
                factory: "$site".into(),
                direction: PortDirection::In,
                item: item.clone(),
                rate: 0.0,
                rate_ceiling: None,
                graph_pos: GraphPos { x: 0.0, y: 600.0 },
            },
            Command::AddRoute {
                kind: RouteKind::Belt {
                    tier: tier_for(*take),
                },
                from: port.clone(),
                to: format!("${alias}"),
                path: vec![src_pos, site_pos],
            },
        ];
        let mut route_aliases = vec![Some(alias.clone()), None];
        for st in &stages {
            let consumes = gd
                .recipes
                .get(&st.recipe)
                .map(|r| r.ingredients.iter().any(|(i, _)| i == item))
                .unwrap_or(false);
            if consumes {
                route_cmds.push(Command::AddEdge {
                    factory: "$site".into(),
                    from: EdgeEnd::Port(format!("${alias}")),
                    to: EdgeEnd::Group(format!("$g.{}", st.item)),
                    item: item.clone(),
                    tier: tier_for(*take),
                });
                route_aliases.push(None);
            }
        }
        items.push(ProposalItem {
            id: new_id(),
            kind: ProposalItemKind::RouteAdd,
            included: true,
            label: format!("⟶ {} ⟶ {}", src_name.to_uppercase(), site_name),
            detail: format!("{} {:.1}/min from surplus", item_name(item), take),
            impact: "REUSES SURPLUS".into(),
            commands: route_cmds,
            aliases: route_aliases,
            depends_on: vec![create_id.clone()],
            sync: None,
            conflict: None,
        });
    }

    // power: the site must be fed (A2.4) — expand a generator factory or add one
    if total_mw > 0.0 {
        let gen_factory = state
            .ports
            .values()
            .find(|p| p.direction == PortDirection::Out && p.item == POWER_ITEM);
        match gen_factory {
            Some(gp) => {
                let f = state.factories.get(&gp.factory);
                let f_name = f.map(|x| x.name.clone()).unwrap_or_default();
                let f_pos = f.map(|x| x.position).unwrap_or(MapPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                });
                let new_target = gp.rate + total_mw * (1.0 + c.power_margin_cap);
                log(
                    phase,
                    &format!(
                        "power: expand {} to {:.0} MW + line to {}",
                        f_name, new_target, site_name
                    ),
                );
                let expand_id = new_id();
                items.push(ProposalItem {
                    id: expand_id.clone(),
                    kind: ProposalItemKind::Modify,
                    included: true,
                    label: format!("Δ EXPAND {}", f_name.to_uppercase()),
                    detail: format!(
                        "MW target {:.0} → {:.0} (covers +{:.0} MW draw)",
                        gp.rate, new_target, total_mw
                    ),
                    impact: format!("+{:.0} MW GEN", new_target - gp.rate),
                    commands: vec![Command::SetPortRate {
                        id: gp.id.clone(),
                        rate: new_target,
                    }],
                    aliases: vec![None],
                    depends_on: vec![],
                    sync: None,
                    conflict: None,
                });
                items.push(ProposalItem {
                    id: new_id(),
                    kind: ProposalItemKind::RouteAdd,
                    included: true,
                    label: format!("⚡ {} ⚡ {}", f_name.to_uppercase(), site_name),
                    detail: "power line — joins the site to the grid".into(),
                    impact: format!("-{total_mw:.0} MW MARGIN"),
                    commands: vec![Command::AddRoute {
                        kind: RouteKind::Power,
                        from: gp.factory.clone(),
                        to: "$site".into(),
                        path: vec![f_pos, site_pos],
                    }],
                    aliases: vec![None],
                    depends_on: vec![create_id.clone(), expand_id],
                    sync: None,
                    conflict: None,
                });
            }
            None => {
                log(phase, &format!("power: no grid — site draws {total_mw:.0} MW unsourced (excluded rows would too)"));
                // No generator anywhere: surface the gap honestly rather than
                // invent a fuel chain the empire has no claims for.
            }
        }
    }
    log(phase, "done");

    // A single-item goal keeps the long-standing "PRODUCE X AT N/MIN" form; a
    // multi-output goal names every shipped item so the review surface matches
    // what the proposal actually does.
    let title = if goal.items.len() > 1 {
        format!(
            "PRODUCE {}",
            goal.items
                .iter()
                .map(|(i, _)| item_name(i).to_uppercase())
                .collect::<Vec<_>>()
                .join(" + ")
        )
    } else {
        format!(
            "PRODUCE {} AT {:.1}/MIN",
            goal_name.to_uppercase(),
            goal_rate
        )
    };
    WizardOutcome::Proposal {
        proposal: Proposal {
            id: String::new(), // assigned by CreateProposal
            source: ProposalSource::GlobalSolver,
            title,
            goal: goal.items.clone(),
            status: ProposalStatus::Draft,
            number: 0, // stamped by CreateProposal
            snapshot_time,
            input_hash: plan_hash,
            provenance: "GLOBAL SOLVER".into(),
            items,
            // passthrough: the total-quantity target rides into the review
            // surface; the solve itself never read it (rate drives the plan).
            milestone: goal.milestone.clone(),
        },
    }
}

/// Memoized backtracking recipe resolver (the wizard's picker).
///
/// A greedy per-item pick cannot survive the real catalog: recipe cycles
/// (Package↔Unpackage fluids, the turbofuel family) make the locally-cheapest
/// candidate a dead end while a pricier sibling resolves fine. The resolver
/// does a depth-first search over candidates in cost order, rejecting any
/// candidate whose craft ingredient sits on the current DFS path (re-entry =
/// cycle) or fails to resolve while producers for it exist; the first
/// candidate whose whole ingredient tree resolves wins.
///
/// Eligibility (W2b): an alternate is available iff the save has unlocked it
/// (`unlocked.contains(&r.class_name)`). `include_alts` re-scopes to ALSO pull
/// in genuinely-locked alternates as suggestions. Standard recipes are always
/// eligible; with an empty unlocked set + `include_alts=false` this collapses
/// to the historical "standard only" filter (honest fixture degradation).
/// Candidate cost order: (starved, cyclic, 1/per_min, power, alternate).
type CandidateCost = (bool, bool, f64, f64, bool);

struct RecipeResolver<'a, 'b> {
    gd: &'a GameData,
    unlocked: &'b BTreeSet<String>,
    include_alts: bool,
    pinned: &'b BTreeMap<String, String>,
    /// item → resolved recipe class. SUCCESSES ONLY: failures are
    /// path-dependent (an item can be unreachable from inside a cycle yet
    /// resolve fine from outside), so only proven picks are cached. The memo
    /// graph is provably acyclic: the first member of a would-be cycle to be
    /// memoized resolved while every other member was still on the DFS path,
    /// so its recorded tree cannot re-enter the cycle.
    memo: BTreeMap<String, String>,
    /// Diagnostics for the caller's phase log (pin-ignored notices).
    notes: Vec<String>,
    /// Backtracking budget (shared value with the expansion cap): one step
    /// per candidate attempt. Exhaustion poisons the resolver — the caller
    /// must surface the same "did not converge" Infeasible as the cap.
    steps: usize,
    exhausted: bool,
}

impl<'a, 'b> RecipeResolver<'a, 'b> {
    fn new(
        gd: &'a GameData,
        unlocked: &'b BTreeSet<String>,
        include_alts: bool,
        pinned: &'b BTreeMap<String, String>,
    ) -> Self {
        Self {
            gd,
            unlocked,
            include_alts,
            pinned,
            memo: BTreeMap::new(),
            notes: Vec::new(),
            steps: 0,
            exhausted: false,
        }
    }

    fn item_name(&self, class: &str) -> String {
        self.gd
            .items
            .get(class)
            .map(|i| i.display_name.clone())
            .unwrap_or_else(|| class.into())
    }

    fn is_resource(&self, item: &str) -> bool {
        self.gd
            .items
            .get(item)
            .map(|i| i.is_resource)
            .unwrap_or(false)
    }

    fn eligible(&self, item: &str, r: &gamedata::docs::Recipe) -> bool {
        !r.produced_in.is_empty()
            && (!r.alternate || self.unlocked.contains(&r.class_name) || self.include_alts)
            && r.products.iter().any(|(i, _)| i == item)
            && r.products.iter().all(|(i, _)| i != POWER_ITEM)
    }

    fn has_candidates(&self, item: &str) -> bool {
        self.gd.recipes.values().any(|r| self.eligible(item, r))
    }

    /// Does `item` expand through recipes during the demand walk? Power is
    /// phase-4 territory, world raws never expand, and an item nothing
    /// produces is the existing T1 `Disconnected` degradation — none of them
    /// can recurse, so the resolver skips them.
    fn expands(&self, item: &str) -> bool {
        item != POWER_ITEM
            && !self.is_resource(item)
            && self.gd.recipes.values().any(|r| {
                !r.produced_in.is_empty()
                    && r.products.iter().any(|(i, _)| i == item)
                    && r.products
                        .first()
                        .map(|(i, _)| i != POWER_ITEM)
                        .unwrap_or(true)
            })
    }

    /// Cost tuple, computed ONCE per candidate before sorting:
    /// (starved, cyclic, 1/per_min, power, alternate).
    ///
    /// `starved`: some craft ingredient has ZERO eligible in-scope producers —
    /// the chain is guaranteed a shortfall, so any non-starved candidate
    /// (however slow) beats it.
    ///
    /// `cyclic`: some non-raw ingredient's PRIMARY producer consumes our
    /// product — one half of a Package↔Unpackage-style 2-cycle, never a sane
    /// default. Narrowed to primary-product partners on purpose: a recipe that
    /// returns the ingredient only as a BYPRODUCT (Rocket Fuel's Compacted
    /// Coal, Aluminum Scrap's water) is not that ingredient's producer of
    /// record and must not paint honest recipes cyclic. Raw-resource
    /// ingredients stay exempt: raws never expand, so a loop through one
    /// cannot recurse.
    fn cost(&self, item: &str, r: &gamedata::docs::Recipe) -> CandidateCost {
        let starved = r.ingredients.iter().any(|(y, _)| {
            self.expands(y) && !self.has_candidates(y) // craftable, none in scope
        });
        let cyclic = r.ingredients.iter().any(|(y, _)| {
            !self.is_resource(y)
                && self.gd.recipes.values().any(|s| {
                    s.products.first().map(|(p, _)| p == y).unwrap_or(false)
                        && s.ingredients.iter().any(|(i, _)| i == item)
                })
        });
        let per_min = r
            .products
            .iter()
            .find(|(i, _)| i == item)
            .map(|(_, n)| n * 60.0 / r.duration_s)
            .unwrap_or(1e-9);
        let power = r
            .produced_in
            .first()
            .map(|m| gamedata::db::recipe_power(self.gd, r, m))
            .unwrap_or(0.0);
        (starved, cyclic, 1.0 / per_min, power, r.alternate)
    }

    /// Candidates in the order the DFS tries them: an explicit pin first
    /// (bypassing cost scoring AND the alternate gate, matching the pre-
    /// resolver pin semantics), then cost order. The stable sort keeps
    /// catalog order among cost ties — the old `min_by` first-wins behavior.
    fn ordered_candidates(&self, item: &str) -> Vec<&'a gamedata::docs::Recipe> {
        let mut scored: Vec<(CandidateCost, &'a gamedata::docs::Recipe)> = self
            .gd
            .recipes
            .values()
            .filter(|r| self.eligible(item, r))
            .map(|r| (self.cost(item, r), r))
            .collect();
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut out: Vec<&'a gamedata::docs::Recipe> = Vec::with_capacity(scored.len() + 1);
        if let Some(class) = self.pinned.get(item) {
            if let Some(pin) = self.gd.recipes.get(class) {
                // A bad pin (unknown class / wrong product) is ignored — it
                // falls through to normal scoring, never panics.
                if !pin.produced_in.is_empty() && pin.products.iter().any(|(i, _)| i == item) {
                    out.push(pin);
                }
            }
        }
        let pin_class = out.first().map(|p| p.class_name.clone());
        for (_, r) in scored {
            if pin_class.as_deref() != Some(r.class_name.as_str()) {
                out.push(r);
            }
        }
        out
    }

    /// One-shot entry point: resolve `item` from an empty DFS path.
    fn resolve(&mut self, item: &str) -> Option<&'a gamedata::docs::Recipe> {
        let mut path = BTreeSet::new();
        self.resolve_path(item, &mut path)
            .and_then(|class| self.gd.recipes.get(&class))
    }

    fn resolve_path(&mut self, item: &str, path: &mut BTreeSet<String>) -> Option<String> {
        if let Some(hit) = self.memo.get(item) {
            return Some(hit.clone());
        }
        // The DFS path includes the item being resolved: a candidate whose
        // chain re-enters ANY item on the path (itself included) is a cycle.
        path.insert(item.to_string());
        let mut chosen: Option<String> = None;
        let mut pin_failed = false;
        for r in self.ordered_candidates(item) {
            self.steps += 1;
            if self.steps > EXPANSION_CAP {
                self.exhausted = true;
                break;
            }
            let mut ok = true;
            for (ing, _) in &r.ingredients {
                if !self.expands(ing) {
                    continue;
                }
                if path.contains(ing) {
                    ok = false;
                    break;
                }
                if self.resolve_path(ing, path).is_none() && self.has_candidates(ing) {
                    // An ingredient nothing produces stays ALLOWED (the
                    // existing T1 `Disconnected` degradation); one that has
                    // producers yet cannot resolve is cycle-blocked — this
                    // candidate fails, try the next.
                    ok = false;
                    break;
                }
            }
            if ok {
                chosen = Some(r.class_name.clone());
                break;
            }
            if self.pinned.get(item) == Some(&r.class_name) {
                pin_failed = true;
            }
        }
        path.remove(item);
        let class = chosen?;
        if pin_failed {
            // Only when a sibling rescued the item — a pin that fails with no
            // fallback surfaces through the normal Infeasible arms instead.
            self.notes.push(format!(
                "pin for {} sits on a recipe cycle — ignored",
                self.item_name(item)
            ));
        }
        self.memo.insert(item.to_string(), class.clone());
        Some(class)
    }

    fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }
}

/// Cheapest RESOLVABLE recipe for an item: a thin one-shot wrapper over
/// `RecipeResolver` (cost order: starved, cyclic, min machines for 1/min,
/// min power; standard recipes win ties over alternates).
pub fn pick_recipe<'a>(
    gd: &'a GameData,
    item: &str,
    unlocked: &BTreeSet<String>,
    include_alts: bool,
    pinned: &BTreeMap<String, String>,
) -> Option<&'a gamedata::docs::Recipe> {
    RecipeResolver::new(gd, unlocked, include_alts, pinned).resolve(item)
}

/// Centroid of the existing factory pins, if any — the anchor for a site that
/// draws only supply-assumed raws (no nodes to cluster on).
fn empire_centroid(state: &PlanState) -> Option<(f64, f64)> {
    let mut sum = (0.0f64, 0.0f64);
    let mut n = 0.0f64;
    for f in state.factories.values() {
        sum.0 += f.position.x;
        sum.1 += f.position.y;
        n += 1.0;
    }
    (n > 0.0).then(|| (sum.0 / n, sum.1 / n))
}

/// Keep a site inside the map. A margin holds the pin (and its chip) off the
/// very edge; the `.max(lo)` guards a degenerate bounds where the margins would
/// cross, so `clamp`'s `min <= max` precondition always holds.
fn clamp_to_bounds((x, y): (f64, f64), b: &gamedata::worldnodes::Bounds) -> MapPos {
    const MARGIN: f64 = 200.0;
    let lo_x = b.min_x + MARGIN;
    let hi_x = (b.max_x - MARGIN).max(lo_x);
    let lo_y = b.min_y + MARGIN;
    let hi_y = (b.max_y - MARGIN).max(lo_y);
    MapPos {
        x: x.clamp(lo_x, hi_x),
        y: y.clamp(lo_y, hi_y),
        z: 0.0,
    }
}

fn nearest_region(world: &WorldSnapshot, pos: MapPos) -> String {
    world
        .regions
        .iter()
        .min_by(|a, b| {
            let d = |r: &gamedata::worldnodes::Region| (r.label_x - pos.x).hypot(r.label_y - pos.y);
            d(a).partial_cmp(&d(b)).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|r| r.name.clone())
        .unwrap_or_default()
}

/// T2 (SDD §5.3): factory-scoped recipe optimization. Explores alternates for
/// each machine group's primary product; a swap is proposed only when every
/// ingredient of the alternate is already sourceable inside the factory (a
/// producing group or a boundary IN port) — T2 rewires feed belts, it never
/// invents supply chains (that is the global solver's job). Output is a
/// mini-proposal of MODIFY items; nothing applies until accepted.
pub fn t2_optimize(
    state: &PlanState,
    gd: &GameData,
    unlocked: &BTreeSet<String>,
    factory_id: &Id,
) -> Option<Proposal> {
    let factory = state.factories.get(factory_id)?;
    let item_name = |class: &str| -> String {
        gd.items
            .get(class)
            .map(|i| i.display_name.clone())
            .unwrap_or_else(|| class.into())
    };
    let per_machine = |r: &gamedata::docs::Recipe, item: &str| -> f64 {
        r.products
            .iter()
            .find(|(i, _)| i == item)
            .map(|(_, n)| n * 60.0 / r.duration_s)
            .unwrap_or(0.0)
    };
    // in-factory sources: producing group (primary product) or boundary IN port
    let source_of = |item: &str| -> Option<EdgeEnd> {
        for gid in &factory.groups {
            // Skip unresolvable groups (imported factories carry recipes
            // outside the loaded catalog) — one unknown recipe must not veto
            // sourcing for the whole factory, or imported factories never get
            // a T2 proposal at all.
            let Some(g) = state.groups.get(gid) else {
                continue;
            };
            let Some(r) = gd.recipes.get(&g.recipe) else {
                continue;
            };
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

    let tier_for = |rate: f64| -> u8 { (1..=6u8).find(|t| belt_capacity(*t) >= rate).unwrap_or(6) };

    let mut items: Vec<ProposalItem> = Vec::new();
    let mut goal: Vec<(String, f64)> = Vec::new();
    for gid in &factory.groups {
        let Some(group) = state.groups.get(gid) else {
            continue;
        };
        let Some(current) = gd.recipes.get(&group.recipe) else {
            continue;
        };
        let Some((product, _)) = current.products.first() else {
            continue;
        };
        if product == POWER_ITEM {
            continue;
        }
        let cur_rate = per_machine(current, product) * group.count as f64 * group.clock;
        let cur_exact = group.count as f64 * group.clock;

        let mut best: Option<(&gamedata::docs::Recipe, f64)> = None;
        for alt in gd.recipes.values() {
            if alt.class_name == current.class_name
                || alt.produced_in.is_empty()
                || !alt.products.iter().any(|(i, _)| i == product)
            {
                continue;
            }
            let alt_exact = cur_rate / per_machine(alt, product).max(1e-9);
            if alt_exact < cur_exact - 1e-6
                && alt
                    .ingredients
                    .iter()
                    .all(|(ing, _)| source_of(ing).is_some())
                && best.map(|(_, e)| alt_exact < e).unwrap_or(true)
            {
                best = Some((alt, alt_exact));
            }
        }
        let Some((alt, alt_exact)) = best else {
            continue;
        };
        let new_count = alt_exact.ceil().max(1.0) as u32;
        let new_clock = (alt_exact / new_count as f64).clamp(0.01, 2.5);
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
        // rewire feeds: old inbound edges out, alternate's ingredients in
        for e in state.edges.values() {
            if e.factory == *factory_id && e.to == EdgeEnd::Group(gid.clone()) {
                cmds.push(Command::DeleteEdge { id: e.id.clone() });
            }
        }
        let cycles_per_min = cur_rate
            / alt
                .products
                .iter()
                .find(|(i, _)| i == product)
                .map(|(_, n)| *n)
                .unwrap_or(1.0);
        for (ing, n) in &alt.ingredients {
            let Some(src) = source_of(ing) else { continue };
            cmds.push(Command::AddEdge {
                factory: factory_id.clone(),
                from: src,
                to: EdgeEnd::Group(gid.clone()),
                item: ing.clone(),
                tier: tier_for(n * cycles_per_min),
            });
        }
        let aliases = vec![None; cmds.len()];
        // Unlocked alternates are first-class — only genuinely-locked ones
        // (alternate + not in the save's unlocked set) carry the suggestion flag.
        let locked_note = if alt.alternate && !unlocked.contains(&alt.class_name) {
            " · NOT UNLOCKED — suggested"
        } else {
            ""
        };
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
                "×{} @ {:.0}% → ×{} @ {:.0}%{}",
                group.count,
                group.clock * 100.0,
                new_count,
                new_clock * 100.0,
                locked_note
            ),
            impact: format!("−{} MACHINES", group.count.saturating_sub(new_count)),
            commands: cmds,
            aliases,
            depends_on: vec![],
            sync: None,
            conflict: None,
        });
        goal.push((product.clone(), cur_rate));
    }

    if items.is_empty() {
        return None;
    }
    Some(Proposal {
        id: String::new(),
        source: ProposalSource::T2Optimize,
        title: format!("OPTIMIZE {}", factory.name.to_uppercase()),
        goal,
        status: ProposalStatus::Draft,
        number: 0,
        snapshot_time: String::new(), // stamped by the caller
        input_hash: String::new(),
        provenance: "T2 OPTIMIZE".into(),
        items,
        milestone: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamedata::docs::Recipe;

    fn recipe(class: &str, item: &str, per_cycle: f64, dur_s: f64, alternate: bool) -> Recipe {
        Recipe {
            class_name: class.into(),
            display_name: class.into(),
            duration_s: dur_s,
            ingredients: vec![("Desc_Ore_C".into(), 1.0)],
            products: vec![(item.into(), per_cycle)],
            produced_in: vec!["Build_ConstructorMk1_C".into()],
            alternate,
            variable_power_mw: None,
        }
    }

    /// W2b: an alternate whose recipe class is in the save's unlocked set is a
    /// first-class recipe — `pick_recipe` selects it with `include_alts=false`
    /// and (being unlocked) it carries no "NOT UNLOCKED" flag.
    #[test]
    fn unlocked_alt_is_selectable() {
        let mut gd = GameData::default();
        // standard: 1/min · alternate: 4/min (cheaper) — the alt wins on cost
        gd.recipes.insert(
            "Recipe_Std_C".into(),
            recipe("Recipe_Std_C", "Desc_Widget_C", 1.0, 60.0, false),
        );
        gd.recipes.insert(
            "Recipe_Alt_C".into(),
            recipe("Recipe_Alt_C", "Desc_Widget_C", 2.0, 30.0, true),
        );
        let mut unlocked = BTreeSet::new();
        unlocked.insert("Recipe_Alt_C".to_string());

        let picked = pick_recipe(&gd, "Desc_Widget_C", &unlocked, false, &BTreeMap::new())
            .expect("an unlocked cheaper alt is eligible without include_alts");
        assert_eq!(
            picked.class_name, "Recipe_Alt_C",
            "the unlocked alt is chosen like a standard recipe"
        );
        // t2's flag predicate: unlocked alts are NOT genuinely locked → no flag
        assert!(
            !picked.alternate || unlocked.contains(&picked.class_name),
            "an unlocked alt drops the NOT UNLOCKED flag"
        );
    }

    /// The real-catalog alumina trap: the honest producer (raws in, byproduct
    /// water out elsewhere in the catalog) must beat the packaging pair even
    /// when the packager is cheaper on power and ties on throughput.
    /// Aluminum Scrap's water BYPRODUCT must not paint the honest recipe
    /// cyclic — cycles through raw resources can't recurse and don't count.
    #[test]
    fn byproduct_water_loop_does_not_shadow_honest_producer() {
        let mut gd = GameData::default();
        let mk = |class: &str,
                  ings: Vec<(&str, f64)>,
                  prods: Vec<(&str, f64)>,
                  machine: &str,
                  dur: f64| Recipe {
            class_name: class.into(),
            display_name: class.into(),
            duration_s: dur,
            ingredients: ings.into_iter().map(|(i, n)| (i.into(), n)).collect(),
            products: prods.into_iter().map(|(i, n)| (i.into(), n)).collect(),
            produced_in: vec![machine.into()],
            alternate: false,
            variable_power_mw: None,
        };
        // honest: raws → alumina (+ silica), refinery
        gd.recipes.insert(
            "Recipe_AluminaSolution_C".into(),
            mk(
                "Recipe_AluminaSolution_C",
                vec![("Desc_OreBauxite_C", 12.0), ("Desc_Water_C", 18.0)],
                vec![("Desc_AluminaSolution_C", 12.0), ("Desc_Silica_C", 5.0)],
                "Build_OilRefinery_C",
                6.0,
            ),
        );
        // packaging pair (identical alumina throughput, cheaper machine)
        gd.recipes.insert(
            "Recipe_UnpackageAlumina_C".into(),
            mk(
                "Recipe_UnpackageAlumina_C",
                vec![("Desc_PackagedAlumina_C", 2.0)],
                vec![("Desc_AluminaSolution_C", 2.0)],
                "Build_Packager_C",
                1.0,
            ),
        );
        gd.recipes.insert(
            "Recipe_PackagedAlumina_C".into(),
            mk(
                "Recipe_PackagedAlumina_C",
                vec![
                    ("Desc_AluminaSolution_C", 2.0),
                    ("Desc_FluidCanister_C", 2.0),
                ],
                vec![("Desc_PackagedAlumina_C", 2.0)],
                "Build_Packager_C",
                1.0,
            ),
        );
        // the byproduct edge that painted the honest recipe cyclic: scrap
        // consumes alumina and RETURNS water
        gd.recipes.insert(
            "Recipe_AluminaScrap_C".into(),
            mk(
                "Recipe_AluminaScrap_C",
                vec![("Desc_AluminaSolution_C", 4.0), ("Desc_Coal_C", 2.0)],
                vec![("Desc_AluminumScrap_C", 6.0), ("Desc_Water_C", 2.0)],
                "Build_OilRefinery_C",
                1.0,
            ),
        );
        for (class, res) in [
            ("Desc_Water_C", true),
            ("Desc_OreBauxite_C", true),
            ("Desc_Coal_C", true),
            ("Desc_AluminaSolution_C", false),
            ("Desc_PackagedAlumina_C", false),
            ("Desc_FluidCanister_C", false),
        ] {
            gd.items.insert(
                class.to_string(),
                gamedata::docs::Item {
                    class_name: class.into(),
                    display_name: class.into(),
                    form: "RF_LIQUID".into(),
                    stack_size: String::new(),
                    energy_mj: 0.0,
                    is_resource: res,
                },
            );
        }
        // refinery costs more power than the packager — the old tiebreak's trap
        for (class, mw) in [("Build_OilRefinery_C", 30.0), ("Build_Packager_C", 10.0)] {
            gd.machines.insert(
                class.to_string(),
                gamedata::docs::Machine {
                    class_name: class.into(),
                    display_name: class.into(),
                    power_mw: mw,
                    footprint_m: None,
                    kind: gamedata::docs::MachineKind::Manufacturer,
                },
            );
        }

        let picked = pick_recipe(
            &gd,
            "Desc_AluminaSolution_C",
            &BTreeSet::new(),
            false,
            &BTreeMap::new(),
        )
        .expect("alumina has producers");
        assert_eq!(
            picked.class_name, "Recipe_AluminaSolution_C",
            "the honest raws-in recipe wins over the packaging half-cycle"
        );
    }

    /// Cost-order guard (kills tuple-order mutants post-resolver): a FAST
    /// candidate that is painted cyclic but still RESOLVES (its painted
    /// ingredient has an honest second producer, so backtracking would accept
    /// it) must lose to a slower unpainted candidate. Only the cyclic-first
    /// tuple order keeps the honest recipe on top — a throughput-first mutant
    /// returns the painted one, because resolution alone would not reject it.
    #[test]
    fn painted_but_resolvable_candidate_loses_ordering() {
        let mut gd = GameData::default();
        let mk =
            |class: &str, ings: Vec<(&str, f64)>, prods: Vec<(&str, f64)>| gamedata::docs::Recipe {
                class_name: class.into(),
                display_name: class.into(),
                duration_s: 60.0,
                ingredients: ings.into_iter().map(|(i, n)| (i.into(), n)).collect(),
                products: prods.into_iter().map(|(i, n)| (i.into(), n)).collect(),
                produced_in: vec!["Build_ConstructorMk1_C".into()],
                alternate: false,
                variable_power_mw: None,
            };
        // honest W: raw-only, slow (1/min)
        gd.recipes.insert(
            "Recipe_W_C".into(),
            mk(
                "Recipe_W_C",
                vec![("Desc_Ore_C", 1.0)],
                vec![("Desc_T_C", 1.0)],
            ),
        );
        // fast X (100/min) consumes M — painted cyclic by P1 below
        gd.recipes.insert(
            "Recipe_X_C".into(),
            mk(
                "Recipe_X_C",
                vec![("Desc_M_C", 1.0)],
                vec![("Desc_T_C", 100.0)],
            ),
        );
        // P1: PRIMARY producer of M that consumes T → paints X cyclic
        gd.recipes.insert(
            "Recipe_P1_C".into(),
            mk(
                "Recipe_P1_C",
                vec![("Desc_T_C", 1.0)],
                vec![("Desc_M_C", 1.0)],
            ),
        );
        // P2: honest producer of M → X's chain RESOLVES despite the paint
        gd.recipes.insert(
            "Recipe_P2_C".into(),
            mk(
                "Recipe_P2_C",
                vec![("Desc_Ore_C", 1.0)],
                vec![("Desc_M_C", 1.0)],
            ),
        );

        let picked = pick_recipe(&gd, "Desc_T_C", &BTreeSet::new(), false, &BTreeMap::new())
            .expect("T has producers");
        assert_eq!(
            picked.class_name, "Recipe_W_C",
            "cyclic-first ordering beats raw throughput even when the painted \
             candidate would resolve"
        );
    }

    /// Paint narrowing (primary-product partners only): a recipe that returns
    /// the ingredient purely as a BYPRODUCT while consuming our product — the
    /// turbofuel shape, Rocket Fuel's Compacted Coal — must NOT paint the fast
    /// candidate cyclic. With the narrow paint the fast recipe wins on
    /// throughput; the old any-product scan painted it and picked the slow one.
    #[test]
    fn byproduct_partner_does_not_paint() {
        let mut gd = GameData::default();
        let mk =
            |class: &str, ings: Vec<(&str, f64)>, prods: Vec<(&str, f64)>| gamedata::docs::Recipe {
                class_name: class.into(),
                display_name: class.into(),
                duration_s: 60.0,
                ingredients: ings.into_iter().map(|(i, n)| (i.into(), n)).collect(),
                products: prods.into_iter().map(|(i, n)| (i.into(), n)).collect(),
                produced_in: vec!["Build_ConstructorMk1_C".into()],
                alternate: false,
                variable_power_mw: None,
            };
        // fast X: Y → T at 100/min
        gd.recipes.insert(
            "Recipe_X_C".into(),
            mk(
                "Recipe_X_C",
                vec![("Desc_Y_C", 1.0)],
                vec![("Desc_T_C", 100.0)],
            ),
        );
        // slow honest W: ore → T at 1/min
        gd.recipes.insert(
            "Recipe_W_C".into(),
            mk(
                "Recipe_W_C",
                vec![("Desc_Ore_C", 1.0)],
                vec![("Desc_T_C", 1.0)],
            ),
        );
        // S consumes T and returns Y ONLY as a byproduct (primary product Z)
        gd.recipes.insert(
            "Recipe_S_C".into(),
            mk(
                "Recipe_S_C",
                vec![("Desc_T_C", 1.0)],
                vec![("Desc_Z_C", 1.0), ("Desc_Y_C", 1.0)],
            ),
        );
        // Y's PRIMARY producer is honest — X's chain resolves through it
        gd.recipes.insert(
            "Recipe_Y_C".into(),
            mk(
                "Recipe_Y_C",
                vec![("Desc_Ore_C", 1.0)],
                vec![("Desc_Y_C", 1.0)],
            ),
        );

        let picked = pick_recipe(&gd, "Desc_T_C", &BTreeSet::new(), false, &BTreeMap::new())
            .expect("T has producers");
        assert_eq!(
            picked.class_name, "Recipe_X_C",
            "a byproduct-only partner is not the ingredient's producer of \
             record and must not paint the fast recipe cyclic"
        );
    }

    /// A locked alternate (not in the unlocked set) is excluded when
    /// `include_alts=false`; `include_alts=true` re-scopes to pull it in as a
    /// suggestion. Being unlocked also admits it. This is the `pick_recipe` gate
    /// `!alternate || unlocked.contains(class) || include_alts`.
    #[test]
    fn locked_alt_excluded_unless_opted_in() {
        let mut gd = GameData::default();
        gd.recipes.insert(
            "Recipe_AltOnly_C".into(),
            recipe("Recipe_AltOnly_C", "Desc_Gizmo_C", 1.0, 60.0, true),
        );
        let empty = BTreeSet::new();
        assert!(
            pick_recipe(&gd, "Desc_Gizmo_C", &empty, false, &BTreeMap::new()).is_none(),
            "a locked alt is excluded by default"
        );
        assert_eq!(
            pick_recipe(&gd, "Desc_Gizmo_C", &empty, true, &BTreeMap::new())
                .expect("include_alts pulls the locked alt in")
                .class_name,
            "Recipe_AltOnly_C"
        );
        // and being unlocked admits it even with include_alts off, flag-free
        let mut unlocked = BTreeSet::new();
        unlocked.insert("Recipe_AltOnly_C".to_string());
        let picked = pick_recipe(&gd, "Desc_Gizmo_C", &unlocked, false, &BTreeMap::new())
            .expect("the unlocked alt is available");
        assert!(picked.alternate && unlocked.contains(&picked.class_name));
    }
}
