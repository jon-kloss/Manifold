//! Save import (SDD §8). The renderer's Web Worker reduces the parsed .sav to
//! this compact `ImportSnapshot`; Rust clusters machines into logical
//! factories (DBSCAN on XY, eps ≈ 120 m) and either:
//!   - FIRST import: writes the ◆ Built layer directly (one undo entry), or
//!   - RE-import: never writes — diffs the snapshot against the current Built
//!     layer into a `Proposal { source: SaveReimport }` (drift), reviewed like
//!     any proposal. Import is enrichment, never load-bearing (Principle 6).

use std::collections::BTreeMap;

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
const REMATCH_M: f64 = 250.0;

/// DBSCAN (min_pts 1 ⇒ every machine belongs somewhere) over machine XY.
pub fn cluster(snapshot: &ImportSnapshot, gd: &gamedata::docs::GameData) -> Vec<Cluster> {
    let pts: Vec<&ImportMachine> = snapshot.machines.iter().collect();
    let mut cluster_of: Vec<Option<usize>> = vec![None; pts.len()];
    let mut n_clusters = 0usize;
    for i in 0..pts.len() {
        if cluster_of[i].is_some() {
            continue;
        }
        let id = n_clusters;
        n_clusters += 1;
        let mut stack = vec![i];
        while let Some(j) = stack.pop() {
            if cluster_of[j].is_some() {
                continue;
            }
            cluster_of[j] = Some(id);
            for (k, p) in pts.iter().enumerate() {
                if cluster_of[k].is_none() && (p.x - pts[j].x).hypot(p.y - pts[j].y) <= DBSCAN_EPS_M
                {
                    stack.push(k);
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

/// First import: materialize clusters as ◆ Built entities into `state`,
/// recording into `tx`. Returns created factory ids.
pub fn write_built_layer(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    clusters: &[Cluster],
    import_id: &str,
) -> Vec<Id> {
    let mut created = Vec::new();
    for c in clusters {
        let fid = new_id();
        let mut group_ids = Vec::new();
        for (i, g) in c.groups.iter().enumerate() {
            let gid = new_id();
            tx.record(state.upsert(Entity::Group(MachineGroup {
                id: gid.clone(),
                factory: fid.clone(),
                machine: g.machine.clone(),
                recipe: g.recipe.clone(),
                count: g.count,
                clock: g.clock.clamp(0.01, 2.5),
                somersloops: 0,
                planned_delta: None,
                graph_pos: GraphPos {
                    x: 280.0 + 300.0 * (i as f64 % 4.0),
                    y: 80.0 + 260.0 * (i as f64 / 4.0).floor(),
                },
                floor: 0,
                status: Status::Built,
                created_by: CreatedBy::Import(import_id.to_string()),
            })));
            group_ids.push(gid);
        }
        tx.record(state.upsert(Entity::Factory(Factory {
            id: fid.clone(),
            name: c.name.clone(),
            position: c.position,
            region: String::new(),
            node_claims: vec![],
            groups: group_ids,
            ports: vec![],
            style_guide: None,
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
    let mut matched: std::collections::BTreeSet<&str> = Default::default();

    for c in clusters {
        let nearest = built
            .iter()
            .filter(|f| !matched.contains(f.id.as_str()))
            .map(|f| {
                let d = (f.position.x - c.position.x).hypot(f.position.y - c.position.y);
                (*f, d)
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        match nearest {
            Some((f, d)) if d <= REMATCH_M => {
                matched.insert(f.id.as_str());
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
                        Some((count, _, _)) if *count == g.count => {}
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
            _ => {
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
) {
    match op {
        SyncOp::CreateCluster { cluster } => {
            write_built_layer(state, tx, std::slice::from_ref(cluster), import_id);
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
    }
}
