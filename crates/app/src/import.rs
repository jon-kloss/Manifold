//! Save import (SDD §8). The renderer's Web Worker reduces the parsed .sav to
//! this compact `ImportSnapshot`; Rust clusters machines into logical
//! factories (DBSCAN on XY, eps ≈ 120 m) and either:
//!   - FIRST import: writes the ◆ Built layer directly (one undo entry), or
//!   - RE-import: never writes — diffs the snapshot against the current Built
//!     layer into a `Proposal { source: SaveReimport }` (drift), reviewed like
//!     any proposal. Import is enrichment, never load-bearing (Principle 6).

use std::collections::{BTreeMap, HashMap};

use planner_core::entities::*;
use planner_core::proposals::*;
use planner_core::state::{Entity, PlanState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportMachine {
    pub class: String,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default = "one")]
    pub clock: f64,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub z: f64,
}

fn one() -> f64 {
    1.0
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSnapshot {
    pub save_name: String,
    #[serde(default)]
    pub build_version: String,
    /// Manufacturers + generators (things that become machine groups).
    pub machines: Vec<ImportMachine>,
    /// Miners/pumps — counted per cluster, become node-claim context later.
    #[serde(default)]
    pub extractors: Vec<ImportMachine>,
    /// Infrastructure counts (belts by class, rails, power lines, trains).
    #[serde(default)]
    pub belts: BTreeMap<String, u32>,
    #[serde(default)]
    pub rails: u32,
    #[serde(default)]
    pub power_lines: u32,
    #[serde(default)]
    pub locomotives: u32,
    #[serde(default)]
    pub wagons: u32,
    #[serde(default)]
    pub train_stations: u32,
    /// Unknown / modded classes → count (quarantine list, surfaced in preview).
    #[serde(default)]
    pub quarantined: BTreeMap<String, u32>,
}

/// One clustered logical factory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub name: String,
    pub position: MapPos,
    /// (machine class, recipe class) → (count, mean clock)
    pub groups: Vec<ClusterGroup>,
    pub extractor_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterGroup {
    pub machine: String,
    pub recipe: String,
    pub count: u32,
    pub clock: f64,
}

const DBSCAN_EPS_M: f64 = 120.0;
/// Clusters match an existing Built factory within this range on re-import.
pub(crate) const REMATCH_M: f64 = 250.0;
/// Clock drift below this (absolute, on the 0–2.5 scale) is rounding noise,
/// not player intent: cluster mean clocks are rounded to 3 decimals (≤ 5e-4
/// error), while deliberate in-game reclocks move in ≥ 1% steps.
const CLOCK_EPS: f64 = 0.005;

/// DBSCAN (min_pts 1 ⇒ every machine belongs somewhere) over machine XY.
pub fn cluster(snapshot: &ImportSnapshot, gd: &gamedata::docs::GameData) -> Vec<Cluster> {
    let pts: Vec<&ImportMachine> = snapshot.machines.iter().collect();
    let mut cluster_of: Vec<Option<usize>> = vec![None; pts.len()];
    let mut n_clusters = 0usize;
    // Uniform grid with eps-sized cells: every point within eps of `p` lies in
    // the 3×3 cell block around `p`'s cell, so the neighbor scan touches only
    // nearby buckets instead of the whole array. Points are marked at PUSH
    // time (each enters the stack at most once) and leave the grid exactly
    // once via swap_remove when assigned, keeping expansion ~O(n) in time and
    // O(n) in stack memory even on dense megabase saves (CODE-REVIEW M17).
    // NaN/±inf coordinates saturate to a finite cell key and fail the `<= eps`
    // distance test against everything, so they stay singletons as before.
    let cell = |p: &ImportMachine| {
        (
            (p.x / DBSCAN_EPS_M).floor() as i64,
            (p.y / DBSCAN_EPS_M).floor() as i64,
        )
    };
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (k, p) in pts.iter().enumerate() {
        grid.entry(cell(p)).or_default().push(k);
    }
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..pts.len() {
        if cluster_of[i].is_some() {
            continue;
        }
        let id = n_clusters;
        n_clusters += 1;
        cluster_of[i] = Some(id);
        stack.push(i);
        while let Some(j) = stack.pop() {
            let (cx, cy) = cell(pts[j]);
            for dx in -1..=1i64 {
                for dy in -1..=1i64 {
                    let key = (cx.saturating_add(dx), cy.saturating_add(dy));
                    let Some(bucket) = grid.get_mut(&key) else {
                        continue;
                    };
                    let mut t = 0;
                    while t < bucket.len() {
                        let k = bucket[t];
                        if cluster_of[k].is_some() {
                            bucket.swap_remove(t);
                        } else if (pts[k].x - pts[j].x).hypot(pts[k].y - pts[j].y) <= DBSCAN_EPS_M {
                            cluster_of[k] = Some(id);
                            stack.push(k);
                            bucket.swap_remove(t);
                        } else {
                            t += 1;
                        }
                    }
                }
            }
        }
    }

    let mut clusters: Vec<Cluster> = Vec::new();
    for id in 0..n_clusters {
        let members: Vec<&ImportMachine> = pts
            .iter()
            .enumerate()
            .filter(|(k, _)| cluster_of[*k] == Some(id))
            .map(|(_, p)| *p)
            .collect();
        if members.is_empty() {
            continue;
        }
        let cx = members.iter().map(|m| m.x).sum::<f64>() / members.len() as f64;
        let cy = members.iter().map(|m| m.y).sum::<f64>() / members.len() as f64;
        let cz = members.iter().map(|m| m.z).sum::<f64>() / members.len() as f64;
        // group by (machine class, recipe)
        let mut groups: BTreeMap<(String, String), (u32, f64)> = BTreeMap::new();
        for m in &members {
            let key = (m.class.clone(), m.recipe.clone().unwrap_or_default());
            let e = groups.entry(key).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += m.clock;
        }
        let groups: Vec<ClusterGroup> = groups
            .into_iter()
            .map(|((machine, recipe), (count, clock_sum))| ClusterGroup {
                machine,
                recipe,
                count,
                clock: (clock_sum / count as f64 * 1000.0).round() / 1000.0,
            })
            .collect();
        // extractors near the centroid count toward the cluster
        let extractor_count = snapshot
            .extractors
            .iter()
            .filter(|e| (e.x - cx).hypot(e.y - cy) <= DBSCAN_EPS_M * 3.0)
            .count() as u32;
        // name by dominant output: biggest group's recipe product
        let dominant = groups.iter().max_by_key(|g| g.count);
        let name = dominant
            .and_then(|g| gd.recipes.get(&g.recipe))
            .and_then(|r| r.products.first())
            .and_then(|(item, _)| gd.items.get(item))
            .map(|i| i.display_name.to_uppercase())
            .or_else(|| {
                dominant
                    .and_then(|g| gd.machines.get(&g.machine))
                    .map(|m| m.display_name.to_uppercase())
            })
            .unwrap_or_else(|| "IMPORTED".into());
        clusters.push(Cluster {
            name,
            position: MapPos {
                x: cx,
                y: cy,
                z: cz,
            },
            groups,
            extractor_count,
        });
    }
    // stable numbering per name: IRON ROD WORKS 1, 2, …
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    for c in clusters.iter_mut() {
        let n = seen.entry(c.name.clone()).or_insert(0);
        *n += 1;
        c.name = format!("{} WORKS {}", c.name, n);
    }
    clusters
}

/// Nearest ◆ Built factory to `pos` within [`REMATCH_M`], or `None`. The
/// shared "is there a built twin here" rule: re-import drift diffing matches
/// clusters to built factories within this same radius, so the build queue
/// (planned entity → its built twin) and drift detection agree on what counts
/// as "the same site", never disagreeing over whether a plan is built yet.
pub(crate) fn nearest_built_match<'a>(state: &'a PlanState, pos: &MapPos) -> Option<&'a Factory> {
    state
        .factories
        .values()
        .filter(|f| f.status == Status::Built)
        .map(|f| (f, (f.position.x - pos.x).hypot(f.position.y - pos.y)))
        .filter(|(_, d)| *d <= REMATCH_M)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(f, _)| f)
}

/// Lowest belt tier whose capacity covers `rate`.
fn tier_for(rate: f64) -> u8 {
    for (i, cap) in BELT_CAPACITY.iter().enumerate() {
        if rate <= *cap {
            return (i + 1) as u8;
        }
    }
    6
}

/// First import: materialize clusters as ◆ Built entities into `state`,
/// recording into `tx`. Returns created factory ids.
///
/// The save parser extracts machines, not belt connectivity, so groups are
/// auto-wired LOGICALLY by recipe I/O: producer groups edge into consumer
/// groups per item, items consumed but not produced become In ports (they
/// surface as deficits until routed — honest), and net surplus becomes Out
/// ports. Groups whose recipe isn't in the loaded catalog stay unwired.
/// Cards land in layered-layout positions (In ports → ranked groups → Out
/// ports) so generated factories read left→right at a glance.
pub fn write_built_layer(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    clusters: &[Cluster],
    import_id: &str,
    gd: &gamedata::docs::GameData,
) -> Vec<Id> {
    use planner_core::layout::{layered_layout, LKind, LNode};
    let mut created = Vec::new();
    for c in clusters {
        let fid = new_id();
        // pass 1: assign ids + collect per-item recipe I/O
        let mut group_specs: Vec<(Id, &ClusterGroup, f64)> = Vec::new();
        let mut producers: BTreeMap<String, Vec<(Id, f64)>> = BTreeMap::new();
        let mut consumers: BTreeMap<String, Vec<(Id, f64)>> = BTreeMap::new();
        for g in &c.groups {
            let gid = new_id();
            let clock = g.clock.clamp(0.01, 2.5);
            if let Some(r) = gd.recipes.get(&g.recipe) {
                if r.duration_s > 0.0 {
                    let cycles_per_min = 60.0 / r.duration_s * g.count as f64 * clock;
                    for (item, n) in &r.products {
                        producers
                            .entry(item.clone())
                            .or_default()
                            .push((gid.clone(), n * cycles_per_min));
                    }
                    for (item, n) in &r.ingredients {
                        consumers
                            .entry(item.clone())
                            .or_default()
                            .push((gid.clone(), n * cycles_per_min));
                    }
                }
            }
            group_specs.push((gid, g, clock));
        }

        // pass 2: wiring — internal edges + boundary ports (positions later)
        let mut ports: Vec<Port> = Vec::new();
        let mut edges: Vec<BeltEdge> = Vec::new();
        let items: std::collections::BTreeSet<&String> =
            producers.keys().chain(consumers.keys()).collect();
        for item in items {
            let prod: f64 = producers
                .get(item)
                .map_or(0.0, |v| v.iter().map(|p| p.1).sum());
            let cons: f64 = consumers
                .get(item)
                .map_or(0.0, |v| v.iter().map(|p| p.1).sum());
            if let (Some(ps), Some(cs)) = (producers.get(item), consumers.get(item)) {
                for (pg, pr) in ps {
                    for (cg, cr) in cs {
                        edges.push(BeltEdge {
                            id: new_id(),
                            factory: fid.clone(),
                            from: EdgeEnd::Group(pg.clone()),
                            to: EdgeEnd::Group(cg.clone()),
                            item: item.clone(),
                            tier: tier_for(pr.min(*cr)),
                            status: Status::Built,
                            created_by: CreatedBy::Import(import_id.to_string()),
                        });
                    }
                }
            }
            let net = prod - cons;
            if net > 1e-6 {
                let pid = new_id();
                for (pg, _) in producers.get(item).into_iter().flatten() {
                    edges.push(BeltEdge {
                        id: new_id(),
                        factory: fid.clone(),
                        from: EdgeEnd::Group(pg.clone()),
                        to: EdgeEnd::Port(pid.clone()),
                        item: item.clone(),
                        tier: tier_for(net),
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    });
                }
                ports.push(Port {
                    id: pid,
                    factory: fid.clone(),
                    direction: PortDirection::Out,
                    item: item.clone(),
                    rate: net,
                    rate_ceiling: None,
                    bound_route: None,
                    graph_pos: GraphPos { x: 0.0, y: 0.0 },
                    status: Status::Built,
                    created_by: CreatedBy::Import(import_id.to_string()),
                });
            } else if net < -1e-6 {
                let pid = new_id();
                for (cg, _) in consumers.get(item).into_iter().flatten() {
                    edges.push(BeltEdge {
                        id: new_id(),
                        factory: fid.clone(),
                        from: EdgeEnd::Port(pid.clone()),
                        to: EdgeEnd::Group(cg.clone()),
                        item: item.clone(),
                        tier: tier_for(-net),
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    });
                }
                ports.push(Port {
                    id: pid,
                    factory: fid.clone(),
                    direction: PortDirection::In,
                    item: item.clone(),
                    rate: -net,
                    rate_ceiling: None,
                    bound_route: None,
                    graph_pos: GraphPos { x: 0.0, y: 0.0 },
                    status: Status::Built,
                    created_by: CreatedBy::Import(import_id.to_string()),
                });
            }
        }

        // pass 3: layered layout over the wired graph
        let mut lnodes: Vec<LNode> = group_specs
            .iter()
            .map(|(gid, _, _)| LNode {
                id: gid.clone(),
                kind: LKind::Group,
            })
            .collect();
        for p in &ports {
            lnodes.push(LNode {
                id: p.id.clone(),
                kind: if p.direction == PortDirection::In {
                    LKind::InPort
                } else {
                    LKind::OutPort
                },
            });
        }
        let end_id = |e: &EdgeEnd| match e {
            EdgeEnd::Group(id) | EdgeEnd::Port(id) | EdgeEnd::Junction(id) => id.clone(),
        };
        let pairs: Vec<(Id, Id)> = edges
            .iter()
            .map(|e| (end_id(&e.from), end_id(&e.to)))
            .collect();
        let positions = layered_layout(&lnodes, &pairs);

        // pass 4: materialize everything in final positions
        let mut group_ids = Vec::new();
        for (i, (gid, g, clock)) in group_specs.iter().enumerate() {
            let fallback = GraphPos {
                x: 280.0 + 300.0 * (i as f64 % 4.0),
                y: 80.0 + 260.0 * (i as f64 / 4.0).floor(),
            };
            tx.record(state.upsert(Entity::Group(MachineGroup {
                id: gid.clone(),
                factory: fid.clone(),
                machine: g.machine.clone(),
                recipe: g.recipe.clone(),
                count: g.count,
                clock: *clock,
                somersloops: 0,
                planned_delta: None,
                graph_pos: positions.get(gid).copied().unwrap_or(fallback),
                floor: 0,
                status: Status::Built,
                created_by: CreatedBy::Import(import_id.to_string()),
            })));
            group_ids.push(gid.clone());
        }
        let mut port_ids = Vec::new();
        for mut p in ports {
            if let Some(pos) = positions.get(&p.id) {
                p.graph_pos = *pos;
            }
            port_ids.push(p.id.clone());
            tx.record(state.upsert(Entity::Port(p)));
        }
        for e in edges {
            tx.record(state.upsert(Entity::Edge(e)));
        }

        tx.record(state.upsert(Entity::Factory(Factory {
            id: fid.clone(),
            name: c.name.clone(),
            position: c.position,
            region: String::new(),
            node_claims: vec![],
            groups: group_ids,
            ports: port_ids,
            style_guide: None,
            replaces: None,
            status: Status::Built,
            created_by: CreatedBy::Import(import_id.to_string()),
        })));
        tx.created.push(fid.clone());
        created.push(fid);
    }
    created
}

/// Drift payload carried on SaveReimport proposal items (accept applies these
/// to the Built layer directly — the one documented exception to ◇-only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
pub enum SyncOp {
    CreateCluster {
        cluster: Box<Cluster>,
    },
    /// count 0 ⇒ the group vanished in game.
    UpdateGroup {
        factory: Id,
        machine: String,
        recipe: String,
        count: u32,
        clock: f64,
    },
    /// The whole factory vanished in game — remove it and everything it owns.
    RemoveFactory {
        factory: Id,
    },
}

/// Re-import: diff clusters against the current Built layer → drift items.
pub fn diff_against_built(
    state: &PlanState,
    gd: &gamedata::docs::GameData,
    clusters: &[Cluster],
) -> Vec<ProposalItem> {
    let item_name = |recipe: &str| -> String {
        gd.recipes
            .get(recipe)
            .map(|r| r.display_name.clone())
            .unwrap_or_else(|| {
                recipe
                    .trim_start_matches("Recipe_")
                    .trim_end_matches("_C")
                    .to_string()
            })
    };
    let built: Vec<&Factory> = state
        .factories
        .values()
        .filter(|f| f.status == Status::Built)
        .collect();
    let mut items = Vec::new();

    // Global assignment: every (cluster, factory) pair within REMATCH_M,
    // taken greedily by ascending distance so the globally closest surviving
    // pair always wins. A new nearby cluster can therefore never steal an
    // existing factory's identity from its genuinely nearest cluster, no
    // matter what order DBSCAN emits clusters in. (Hungarian would minimize
    // the distance *sum* and could hand a factory a farther cluster —
    // semantically worse for identity matching, and overkill here.)
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new(); // (distance, built idx, cluster idx)
    for (ci, c) in clusters.iter().enumerate() {
        for (fi, f) in built.iter().enumerate() {
            let d = (f.position.x - c.position.x).hypot(f.position.y - c.position.y);
            if d <= REMATCH_M {
                pairs.push((d, fi, ci));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| built[a.1].id.cmp(&built[b.1].id))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut assigned: Vec<Option<usize>> = vec![None; clusters.len()]; // cluster idx → built idx
    let mut matched: Vec<bool> = vec![false; built.len()];
    for (_, fi, ci) in pairs {
        if assigned[ci].is_none() && !matched[fi] {
            assigned[ci] = Some(fi);
            matched[fi] = true;
        }
    }

    for (ci, c) in clusters.iter().enumerate() {
        match assigned[ci].map(|fi| built[fi]) {
            Some(f) => {
                // group-level diff
                let existing: BTreeMap<(String, String), (u32, f64, Id)> = f
                    .groups
                    .iter()
                    .filter_map(|gid| state.groups.get(gid))
                    .map(|g| {
                        (
                            (g.machine.clone(), g.recipe.clone()),
                            (g.count, g.clock, g.id.clone()),
                        )
                    })
                    .collect();
                for g in &c.groups {
                    let key = (g.machine.clone(), g.recipe.clone());
                    match existing.get(&key) {
                        Some((count, clock, _)) if *count == g.count => {
                            // Compare against the clamped save clock — the
                            // value apply_sync will actually store — so an
                            // accepted item converges to InSync on re-import.
                            let target = g.clock.clamp(0.01, 2.5);
                            if (clock - target).abs() > CLOCK_EPS {
                                items.push(drift_item(
                                    format!(
                                        "Δ {} — {} reclocked in game",
                                        f.name,
                                        item_name(&g.recipe)
                                    ),
                                    format!(
                                        "{:.1}% built → {:.1}% in save",
                                        clock * 100.0,
                                        target * 100.0
                                    ),
                                    SyncOp::UpdateGroup {
                                        factory: f.id.clone(),
                                        machine: g.machine.clone(),
                                        recipe: g.recipe.clone(),
                                        count: g.count,
                                        clock: g.clock,
                                    },
                                ));
                            }
                        }
                        Some((count, _, _)) => items.push(drift_item(
                            format!("Δ {} — {}", f.name, item_name(&g.recipe)),
                            format!("×{count} built → ×{} in save", g.count),
                            SyncOp::UpdateGroup {
                                factory: f.id.clone(),
                                machine: g.machine.clone(),
                                recipe: g.recipe.clone(),
                                count: g.count,
                                clock: g.clock,
                            },
                        )),
                        None => items.push(drift_item(
                            format!("Δ {} — {} added in game", f.name, item_name(&g.recipe)),
                            format!("×{} @ {:.0}%", g.count, g.clock * 100.0),
                            SyncOp::UpdateGroup {
                                factory: f.id.clone(),
                                machine: g.machine.clone(),
                                recipe: g.recipe.clone(),
                                count: g.count,
                                clock: g.clock,
                            },
                        )),
                    }
                }
                for ((machine, recipe), (count, clock, _)) in &existing {
                    if !c
                        .groups
                        .iter()
                        .any(|g| &g.machine == machine && &g.recipe == recipe)
                    {
                        items.push(drift_item(
                            format!("Δ {} — {} demolished in game", f.name, item_name(recipe)),
                            format!("×{count} built → gone"),
                            SyncOp::UpdateGroup {
                                factory: f.id.clone(),
                                machine: machine.clone(),
                                recipe: recipe.clone(),
                                count: 0,
                                clock: *clock,
                            },
                        ));
                    }
                }
            }
            None => {
                let machines: u32 = c.groups.iter().map(|g| g.count).sum();
                items.push(ProposalItem {
                    id: new_id(),
                    kind: ProposalItemKind::Create,
                    included: true,
                    label: format!("+ {} — NEW IN GAME", c.name),
                    detail: format!("{} machines · {} groups", machines, c.groups.len()),
                    impact: "BUILT".into(),
                    commands: vec![],
                    aliases: vec![],
                    depends_on: vec![],
                    sync: Some(
                        serde_json::to_value(SyncOp::CreateCluster {
                            cluster: Box::new(c.clone()),
                        })
                        .unwrap(),
                    ),
                });
            }
        }
    }

    // Built factories with no surviving cluster were demolished in game —
    // silence here would report IN SYNC over a missing factory.
    for (fi, f) in built.iter().enumerate() {
        if matched[fi] {
            continue;
        }
        let groups: Vec<&MachineGroup> = f
            .groups
            .iter()
            .filter_map(|gid| state.groups.get(gid))
            .collect();
        let machines: u32 = groups.iter().map(|g| g.count).sum();
        items.push(drift_item(
            format!("Δ {} — factory demolished in game", f.name),
            format!("×{machines} machines · {} groups → gone", groups.len()),
            SyncOp::RemoveFactory {
                factory: f.id.clone(),
            },
        ));
    }
    items
}

fn drift_item(label: String, detail: String, op: SyncOp) -> ProposalItem {
    ProposalItem {
        id: new_id(),
        kind: ProposalItemKind::Modify,
        included: true,
        label,
        detail,
        impact: "DRIFT".into(),
        commands: vec![],
        aliases: vec![],
        depends_on: vec![],
        sync: Some(serde_json::to_value(op).unwrap()),
    }
}

/// Apply one sync op to the Built layer (accept path for SaveReimport items).
pub fn apply_sync(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    op: &SyncOp,
    import_id: &str,
    gd: &gamedata::docs::GameData,
) {
    match op {
        SyncOp::CreateCluster { cluster } => {
            write_built_layer(state, tx, std::slice::from_ref(cluster), import_id, gd);
        }
        SyncOp::UpdateGroup {
            factory,
            machine,
            recipe,
            count,
            clock,
        } => {
            let Some(f) = state.factories.get(factory).cloned() else {
                return;
            };
            let existing = f
                .groups
                .iter()
                .filter_map(|gid| state.groups.get(gid))
                .find(|g| &g.machine == machine && &g.recipe == recipe)
                .cloned();
            match existing {
                Some(mut g) if *count > 0 => {
                    g.count = *count;
                    g.clock = clock.clamp(0.01, 2.5);
                    // Sync writes the baseline but keeps the user's planned
                    // delta — except components the game caught up to, which
                    // dissolve ("visible until built in-game").
                    if let Some(mut d) = g.planned_delta {
                        if d.count == Some(g.count) {
                            d.count = None;
                        }
                        if d.clock.is_some_and(|c| (c - g.clock).abs() < 1e-9) {
                            d.clock = None;
                        }
                        g.planned_delta = (!d.is_empty()).then_some(d);
                    }
                    tx.record(state.upsert(Entity::Group(g)));
                }
                Some(g) => {
                    // demolished in game
                    let mut f = f;
                    f.groups.retain(|gid| gid != &g.id);
                    if let Some(ops) = state.remove("groups", &g.id) {
                        tx.record(ops);
                    }
                    tx.record(state.upsert(Entity::Factory(f)));
                }
                None if *count > 0 => {
                    let gid = new_id();
                    tx.record(state.upsert(Entity::Group(MachineGroup {
                        id: gid.clone(),
                        factory: factory.clone(),
                        machine: machine.clone(),
                        recipe: recipe.clone(),
                        count: *count,
                        clock: clock.clamp(0.01, 2.5),
                        somersloops: 0,
                        planned_delta: None,
                        graph_pos: GraphPos { x: 280.0, y: 600.0 },
                        floor: 0,
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    })));
                    let mut f = f;
                    f.groups.push(gid);
                    tx.record(state.upsert(Entity::Factory(f)));
                }
                None => {}
            }
        }
        SyncOp::RemoveFactory { factory } => {
            // Tolerate stale ops (factory already gone), like the arms above.
            if state.factories.contains_key(factory) {
                planner_core::commands::remove_factory_cascading(state, tx, factory);
            }
        }
    }
}
