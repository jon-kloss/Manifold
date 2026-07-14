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

    while let Some((item, mut rate)) = queue.pop() {
        if cancel.load(Ordering::Relaxed) {
            return WizardOutcome::Cancelled;
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
        let craftable = gd.recipes.values().any(|r| {
            !r.produced_in.is_empty()
                && r.products.iter().any(|(i, _)| i == &item)
                && r.products
                    .first()
                    .map(|(i, _)| i != POWER_ITEM)
                    .unwrap_or(true)
        });
        if extractable || !craftable {
            *raw.entry(item.clone()).or_default() += rate;
            log(phase, &format!("raw: {} {:.1}/min", item_name(&item), rate));
            continue;
        }
        *demand.entry(item.clone()).or_default() += rate;
        // expand via the standard recipe for BFS; final pick happens in phase 2
        if let Some(r) = pick_recipe(gd, &item, unlocked, c.include_alternates, pinned) {
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
        }
    }

    // ---------- phase 2: recipe selection ----------
    let phase = "RECIPE SELECTION";
    let mut stages: Vec<Stage> = Vec::new();
    for (item, rate) in &demand {
        if cancel.load(Ordering::Relaxed) {
            return WizardOutcome::Cancelled;
        }
        let Some(r) = pick_recipe(gd, item, unlocked, c.include_alternates, pinned) else {
            // A GOAL item with no pickable recipe can never be staged — the
            // proposal would dangle a `$g.` alias and accept would always
            // fail. Name the fix instead of emitting a broken proposal.
            // (Non-goal mid-chain items keep degrading honestly via the
            // ingredient-edge guard + T1 `Disconnected` shortfalls.)
            if goal.items.iter().any(|(i, _)| i == item) {
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
        let power_mw = machine
            .map(|_| gamedata::db::recipe_power(gd, r, &machine_class) * count as f64)
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
    let mut picked_nodes: Vec<(String, String, f64)> = Vec::new(); // (node id, item, rate at Mk.2)
    let mut anchor: Option<(f64, f64)> = None;
    let mut budget = c.node_budget;
    let mut binding: Option<String> = None;
    for (item, need) in &raw {
        if item == "Desc_Water_C" {
            continue; // water is unmetered until pipes land
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
            let machine = gd.machines.get(WIZARD_EXTRACTOR);
            let rate = machine
                .map(|m| extraction_rate(m, &n.purity, 1.0))
                .unwrap_or(0.0);
            covered += rate;
            budget -= 1;
            if anchor.is_none() {
                anchor = Some((n.x, n.y));
            }
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

    let site_pos = anchor
        .map(|(x, y)| MapPos {
            x: x + 220.0,
            y: y + 220.0,
            z: 0.0,
        })
        .unwrap_or(MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let goal_name = goal
        .items
        .first()
        .map(|(i, _)| item_name(i))
        .unwrap_or_default();
    let site_name = format!("{} WORKS", goal_name.to_uppercase());
    log(
        phase,
        &format!(
            "site: {} @ ({:.0}, {:.0}) — {} stages, {} claims",
            site_name,
            site_pos.x,
            site_pos.y,
            stages.len(),
            picked_nodes.len()
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
        if item == "Desc_Water_C" {
            continue;
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
    let goal_item = goal
        .items
        .first()
        .map(|(i, _)| i.clone())
        .unwrap_or_default();
    let goal_rate = goal.items.first().map(|(_, r)| *r).unwrap_or(0.0);
    push(
        &mut cmds,
        &mut aliases,
        Command::AddPort {
            factory: "$site".into(),
            direction: PortDirection::Out,
            item: goal_item.clone(),
            rate: 0.0,
            rate_ceiling: None,
            graph_pos: GraphPos {
                x: 1400.0,
                y: 200.0,
            },
        },
        Some("site.out"),
    );

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
                if ing == "Desc_Water_C" {
                    continue;
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
    // The out port's feed depends on how the goal is produced: a stage group
    // when one exists, else the raw in port (extraction-and-ship — the claims
    // feed the in port, which feeds the out port). Unmetered water has no in
    // port: leave the out port unwired — an honest T1 shortfall, not a
    // dangling `$g.` alias that would roll back the whole accept.
    let goal_from = if stages.iter().any(|s| s.item == goal_item) {
        Some(EdgeEnd::Group(format!("$g.{goal_item}")))
    } else if raw.contains_key(&goal_item) && goal_item != "Desc_Water_C" {
        Some(EdgeEnd::Port(format!("$in.{goal_item}")))
    } else {
        None
    };
    if let Some(from) = goal_from {
        push(
            &mut cmds,
            &mut aliases,
            Command::AddEdge {
                factory: "$site".into(),
                from,
                to: EdgeEnd::Port("$site.out".into()),
                item: goal_item.clone(),
                tier: tier_for(goal_rate),
            },
            None,
        );
    }
    push(
        &mut cmds,
        &mut aliases,
        Command::SetPortRate {
            id: "$site.out".into(),
            rate: goal_rate,
        },
        None,
    );

    let create_id = new_id();
    let machines_total: u32 = stages.iter().map(|s| s.count).sum();
    items.push(ProposalItem {
        id: create_id.clone(),
        kind: ProposalItemKind::Create,
        included: true,
        label: format!("+ {site_name} — NEW"),
        detail: format!(
            "{} stages · {} machines · {} {:.1}/min",
            stages.len(),
            machines_total,
            goal_name,
            goal_rate
        ),
        impact: format!("+{total_mw:.0} MW"),
        commands: cmds,
        aliases,
        depends_on: vec![],
        sync: None,
    });

    // CLAIM items (each excludable; factory alias binds them to the site)
    for (node, item, rate) in &picked_nodes {
        items.push(ProposalItem {
            id: new_id(),
            kind: ProposalItemKind::Claim,
            included: true,
            label: format!("◉ CLAIM {}", node.to_uppercase()),
            detail: format!("{} · Mk.2 miner · {:.0}/min", item_name(item), rate),
            impact: "FREE ✓".into(),
            commands: vec![Command::ClaimNode {
                factory: "$site".into(),
                node: node.clone(),
                extractor: WIZARD_EXTRACTOR.into(),
                clock: 1.0,
            }],
            aliases: vec![None],
            depends_on: vec![create_id.clone()],
            sync: None,
        });
    }

    // ---------- phase 4: routing + power sourcing (A2.4) ----------
    let phase = "ROUTING";
    // deliver the goal item to an existing unbound IN port if one exists
    if let Some(consumer) = state.ports.values().find(|p| {
        p.direction == PortDirection::In && p.item == goal_item && p.bound_route.is_none()
    }) {
        let dst = state.factories.get(&consumer.factory);
        let dst_name = dst.map(|f| f.name.clone()).unwrap_or_default();
        let dst_pos = dst.map(|f| f.position).unwrap_or(MapPos {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
        // A3.3: pick the transport by distance/rate thresholds
        let dist = ((dst_pos.x - site_pos.x).powi(2) + (dst_pos.y - site_pos.y).powi(2)).sqrt();
        let picked = planner_core::transport::pick_transport(dist, goal_rate);
        let route_kind = match picked {
            "rail" => RouteKind::Rail {
                spec: RailSpec::default(),
            },
            "drone" => RouteKind::Drone {
                spec: DroneSpec::default(),
            },
            _ => RouteKind::Belt {
                tier: tier_for(goal_rate),
            },
        };
        log(
            phase,
            &format!(
                "route: {} ⟶ {} ({} {:.1}/min · {} — {:.1} km)",
                site_name,
                dst_name,
                goal_name,
                goal_rate,
                picked.to_uppercase(),
                dist / 1000.0
            ),
        );
        items.push(ProposalItem {
            id: new_id(),
            kind: ProposalItemKind::RouteAdd,
            included: true,
            label: format!("⟶ {} ⟶ {}", site_name, dst_name.to_uppercase()),
            detail: format!(
                "{} {:.1}/min · MK.{}",
                goal_name,
                goal_rate,
                tier_for(goal_rate)
            ),
            impact: format!(
                "proj {:.0}%",
                100.0 * goal_rate / belt_capacity(tier_for(goal_rate))
            ),
            commands: vec![Command::AddRoute {
                kind: route_kind,
                from: "$site.out".into(),
                to: consumer.id.clone(),
                path: vec![site_pos, dst_pos],
            }],
            aliases: vec![None],
            depends_on: vec![create_id.clone()],
            sync: None,
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

    let title = format!(
        "PRODUCE {} AT {:.1}/MIN",
        goal_name.to_uppercase(),
        goal_rate
    );
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

/// Cheapest recipe for an item: min machines for 1/min, then min power.
/// Standard recipes win ties over alternates.
///
/// Eligibility (W2b): an alternate is available iff the save has unlocked it
/// (`unlocked.contains(&r.class_name)`). `include_alts` re-scopes to ALSO pull
/// in genuinely-locked alternates as suggestions. Standard recipes are always
/// eligible; with an empty unlocked set + `include_alts=false` this collapses to
/// the historical "standard only" filter (honest fixture degradation).
fn pick_recipe<'a>(
    gd: &'a GameData,
    item: &str,
    unlocked: &BTreeSet<String>,
    include_alts: bool,
    pinned: &BTreeMap<String, String>,
) -> Option<&'a gamedata::docs::Recipe> {
    // An explicit pin wins over cost scoring: if the caller pinned a recipe for
    // this product and it exists in gamedata AND actually produces the item,
    // adopt it directly. A bad pin (unknown class / wrong product) is ignored —
    // it falls through to the normal scoring below, never panics.
    if let Some(class) = pinned.get(item) {
        if let Some(r) = gd.recipes.get(class) {
            if !r.produced_in.is_empty() && r.products.iter().any(|(i, _)| i == item) {
                return Some(r);
            }
        }
    }
    gd.recipes
        .values()
        .filter(|r| {
            !r.produced_in.is_empty()
                && (!r.alternate || unlocked.contains(&r.class_name) || include_alts)
                && r.products.iter().any(|(i, _)| i == item)
                && r.products.iter().all(|(i, _)| i != POWER_ITEM)
        })
        .min_by(|a, b| {
            let cost = |r: &gamedata::docs::Recipe| {
                let per_min = r
                    .products
                    .iter()
                    .find(|(i, _)| i == item)
                    .map(|(_, n)| n * 60.0 / r.duration_s)
                    .unwrap_or(1e-9);
                let power = r
                    .produced_in
                    .first()
                    .map(|m| gamedata::db::recipe_power(gd, r, m))
                    .unwrap_or(0.0);
                (1.0 / per_min, power, r.alternate)
            };
            cost(a)
                .partial_cmp(&cost(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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
