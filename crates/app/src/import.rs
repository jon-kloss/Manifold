//! Save import (SDD §8). The renderer's Web Worker reduces the parsed .sav to
//! this compact `ImportSnapshot`; Rust clusters machines into logical
//! factories (DBSCAN on XY, eps ≈ 120 m) and either:
//!   - FIRST import: writes the ◆ Built layer directly (one undo entry), or
//!   - RE-import: never writes — diffs the snapshot against the current Built
//!     layer into a `Proposal { source: SaveReimport }` (drift), reviewed like
//!     any proposal. Import is enrichment, never load-bearing (Principle 6).

use std::collections::{BTreeMap, HashMap};

use gamedata::worldnodes::WorldSnapshot;
use planner_core::entities::*;
use planner_core::proposals::*;
use planner_core::state::{Entity, PlanState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Extractors only (W2b node context): stable ref to the resource node /
    /// water volume this extractor sits on (the save's level instance name),
    /// for re-match on re-import. `None` for manufacturers/generators.
    #[serde(default)]
    pub node_actor_id: Option<String>,
    /// Resource item (Desc_…). The save does not carry it — `None` until the
    /// world catalog supplies it (W2b-C).
    #[serde(default)]
    pub resource: Option<String>,
    /// Node purity. Not carried in the save — `None` (snapshot-primary purity:
    /// the bundled world catalog is the trusted source, W2b-C).
    #[serde(default)]
    pub purity: Option<String>,
    /// Extraction rate items/min. Not exposed by the parser — `None`.
    #[serde(default)]
    pub extraction_rate: Option<f64>,
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
    /// Purchased/unlocked schematic class names (W2b unlocked-alt awareness).
    /// Empty when the schematic manager actor is absent (old snapshots).
    #[serde(default)]
    pub unlocked_schematics: Vec<String>,
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
    /// Node-bound extractors only (see `extractors`); excludes water pumps, which
    /// fold into `groups` as producers.
    pub extractor_count: u32,
    /// NODE-BOUND extractors (miners, oil pumps) attributed to this cluster (each
    /// to its nearest centroid) — carry the position + stable node ref so
    /// [`write_built_layer`] can bind ◆ NodeClaims to real save nodes (W2b-C).
    /// Water extractors are NOT here: they have no node to claim, so they import
    /// as producing groups in `groups` instead. serde-default so drift proposals
    /// persisted before W2b-C (SyncOp::CreateCluster) still load.
    #[serde(default)]
    pub extractors: Vec<ClusterExtractor>,
}

/// One miner/pump attributed to a cluster, with the geometry + stable node ref
/// needed to reconcile it against the world catalog (W2b-C).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterExtractor {
    pub class: String,
    pub position: MapPos,
    /// Serde-default because a `ClusterExtractor` is persisted inside
    /// `SyncOp::CreateCluster`; older proposals lack the field.
    #[serde(default = "one")]
    pub clock: f64,
    #[serde(default)]
    pub node_actor_id: Option<String>,
    /// Purity read from the save's `mPurityOverride` for the node this extractor
    /// sits on (`pure` | `normal` | `impure`); `None` when the save didn't
    /// carry it. Authoritative over the bundled catalog — handles randomized /
    /// modded purities.
    #[serde(default)]
    pub purity: Option<String>,
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
/// A per-cluster tally of aggregated groups keyed by (machine class, recipe
/// class) → (count, summed clock); the clock is averaged when the ClusterGroup
/// is built.
type GroupTally = BTreeMap<(String, String), (u32, f64)>;
/// Clusters match an existing Built factory within this range on re-import.
pub(crate) const REMATCH_M: f64 = 250.0;
/// Clock drift below this (absolute, on the 0–2.5 scale) is rounding noise,
/// not player intent: cluster mean clocks are rounded to 3 decimals (≤ 5e-4
/// error), while deliberate in-game reclocks move in ≥ 1% steps.
const CLOCK_EPS: f64 = 0.005;
/// Generous tolerance for binding a miner to a bundled world node (W2b-C): the
/// community catalog's coordinates and the save's differ by tens of meters, and
/// a miner's footprint spans the node. Beyond this, the miner is on no known
/// node → a plan-local `"save:<id>"` claim. Reuses the [`REMATCH_M`] site idiom.
const NODE_MATCH_M: f64 = REMATCH_M;
/// A miner whose position differs from its bound snapshot node by MORE than this
/// is the ground truth — write a plan-local corrected position. Chosen above the
/// community-extraction coordinate noise so normal binding stays silent.
pub(crate) const NODE_DRIFT_M: f64 = 30.0;

/// Every synthesized zero-ingredient extraction recipe, grouped by its extractor
/// machine → list of (produced item, recipe class). A Water Extractor draws its
/// resource from the environment with no world node to claim, so `docs.rs` gives
/// such extractors a recipe that produces from nothing; on import they build as ◆
/// GROUPS, not node claims. The `MachineKind::Extractor` gate is load-bearing: it
/// pins the docs.rs invariant "only extractors get a zero-ingredient recipe", so
/// a future/modded zero-ingredient recipe on a non-extractor can't misclassify
/// it. Built ONCE per import (the clustering is near-linear by design).
fn extract_recipes(gd: &gamedata::docs::GameData) -> BTreeMap<&str, Vec<(&str, &str)>> {
    let mut out: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    for r in gd.recipes.values() {
        if !r.ingredients.is_empty() {
            continue;
        }
        let Some((item, _)) = r.products.first() else {
            continue;
        };
        for m in &r.produced_in {
            if matches!(
                gd.machines.get(m).map(|mc| &mc.kind),
                Some(gamedata::docs::MachineKind::Extractor { .. })
            ) {
                out.entry(m.as_str())
                    .or_default()
                    .push((item.as_str(), r.class_name.as_str()));
            }
        }
    }
    out
}

/// Water pumps: extractors with exactly ONE zero-ingredient recipe (always
/// Water) → class → that recipe. A single-resource extractor imports as one
/// producing group regardless of position.
fn water_extract_recipes(gd: &gamedata::docs::GameData) -> BTreeMap<&str, &str> {
    extract_recipes(gd)
        .into_iter()
        .filter(|(_, recipes)| recipes.len() == 1)
        .map(|(class, recipes)| (class, recipes[0].1))
        .collect()
}

/// Fracking extractors: extractors with MORE THAN ONE zero-ingredient recipe —
/// the multi-resource well extractor synthesizes one recipe per (resource,
/// purity), so it has many, while a water pump has exactly one. The exact recipe
/// for a given imported extractor is resolved per-instance from the satellite it
/// sits on (see [`fracking_group_for`]).
fn fracking_classes(gd: &gamedata::docs::GameData) -> std::collections::BTreeSet<&str> {
    extract_recipes(gd)
        .into_iter()
        .filter(|(_, recipes)| recipes.len() > 1)
        .map(|(class, _)| class)
        .collect()
}

/// Resolve an imported fracking extractor to its producing group. The satellite
/// it sits on gives the RESOURCE (oil / nitrogen / water) and the catalog PURITY;
/// the save's own purity (`mPurityOverride`) wins when present, mirroring the
/// miner path (`bind_extractors`). Returns the per-(resource, purity) synthesized
/// recipe class — purity lives in the recipe RATE, so the caller keeps the
/// untouched save overclock as the group clock (never folded, so it can't
/// overflow the [0.01, 2.5] clock clamp and silently halve a pure well). `None`
/// if no satellite is in range or the resolved (resource, purity) has no recipe.
fn fracking_group_for(
    class: &str,
    x: f64,
    y: f64,
    purity_override: Option<&str>,
    world: &gamedata::worldnodes::WorldSnapshot,
    gd: &gamedata::docs::GameData,
    fracking_classes: &std::collections::BTreeSet<&str>,
) -> Option<String> {
    if !fracking_classes.contains(class) {
        return None;
    }
    let sat = world
        .nodes
        .iter()
        .filter(|n| n.node_type == "fracking-satellite")
        .map(|n| (n, (n.x - x).hypot(n.y - y)))
        .filter(|(_, d)| *d <= NODE_MATCH_M)
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?
        .0;
    let purity = purity_override.unwrap_or(sat.purity.as_str());
    let recipe = format!(
        "Recipe_Extract_{}_{}_{}",
        class.trim_end_matches("_C"),
        purity,
        sat.item
    );
    gd.recipes.contains_key(&recipe).then_some(recipe)
}

/// The plan-local id a save-only node (no catalog match) claims under.
fn save_node_key(e: &ClusterExtractor) -> String {
    match &e.node_actor_id {
        Some(a) => format!("save:{a}"),
        None => format!("save:{:.0},{:.0}", e.position.x, e.position.y),
    }
}

/// One extractor's reconciliation against the world catalog: the resolved node
/// id, the stable save ref to re-match on, and any plan-local geometry override.
struct BoundNode {
    node: String,
    save_node_id: Option<String>,
    node_override: Option<NodeOverride>,
}

/// Bind a batch of extractors to real world nodes by position (W2b-C). Greedy
/// nearest wins so a shared node goes to its closest miner; nodes already
/// claimed in `state` are off-limits (no phantom conflicts on a fresh import).
/// A miner beyond [`NODE_MATCH_M`] of every free node becomes a `"save:<id>"`
/// plan-local node synthesized from its override alone. A snapshot match whose
/// position drifts past [`NODE_DRIFT_M`] carries a corrected-position override.
fn bind_extractors(
    state: &PlanState,
    world: &WorldSnapshot,
    exts: &[ClusterExtractor],
) -> Vec<BoundNode> {
    let mut taken: std::collections::BTreeSet<String> =
        state.node_claims.values().map(|c| c.node.clone()).collect();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (ei, e) in exts.iter().enumerate() {
        for (ni, n) in world.nodes.iter().enumerate() {
            // Only plain nodes are bind targets today — binding is proximity-only
            // (not item-matched), so without this gate an imported miner could
            // snap to a nearby geyser or fracking satellite. Fracking extractors
            // binding to satellite nodes lands with the fracking feature.
            if !n.is_plain_node() {
                continue;
            }
            let d = (n.x - e.position.x).hypot(n.y - e.position.y);
            if d <= NODE_MATCH_M {
                pairs.push((d, ei, ni));
            }
        }
    }
    pairs.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    let mut assigned: Vec<Option<usize>> = vec![None; exts.len()];
    for (_, ei, ni) in pairs {
        let nid = &world.nodes[ni].id;
        if assigned[ei].is_none() && !taken.contains(nid) {
            assigned[ei] = Some(ni);
            taken.insert(nid.clone());
        }
    }
    // save-only ids must be unique per extractor: node-bound extractors on the
    // same resource actor (or several sharing one ref) would collapse into one
    // node under a bare `save:<actor>` (a false conflict). Disambiguate against
    // ids already in use. (Water pumps never reach here — they import as groups.)
    let mut used_save: std::collections::BTreeSet<String> = state
        .node_claims
        .values()
        .map(|c| c.node.clone())
        .filter(|n| n.starts_with("save:"))
        .collect();
    let mut out = Vec::with_capacity(exts.len());
    for (ei, e) in exts.iter().enumerate() {
        match assigned[ei] {
            Some(ni) => {
                let n = &world.nodes[ni];
                let d = (n.x - e.position.x).hypot(n.y - e.position.y);
                let pos = (d > NODE_DRIFT_M).then_some(e.position);
                // The save's purity (mPurityOverride) is authoritative — record
                // it when it disagrees with the bundled catalog (a randomized or
                // modded node the community dump can't know), even if the
                // position matches. That corrects the node's purity + its
                // extraction rate throughout the app.
                let purity = e.purity.clone().filter(|p| *p != n.purity);
                let node_override = (pos.is_some() || purity.is_some()).then(|| NodeOverride {
                    id: n.id.clone(),
                    pos,
                    save_actor: e.node_actor_id.clone(),
                    purity,
                });
                out.push(BoundNode {
                    node: n.id.clone(),
                    save_node_id: e.node_actor_id.clone(),
                    node_override,
                });
            }
            None => {
                let base = save_node_key(e);
                let mut key = base.clone();
                let mut n = 2;
                while used_save.contains(&key) {
                    key = format!("{base}#{n}");
                    n += 1;
                }
                used_save.insert(key.clone());
                out.push(BoundNode {
                    node: key.clone(),
                    save_node_id: e.node_actor_id.clone(),
                    node_override: Some(NodeOverride {
                        id: key,
                        pos: Some(e.position),
                        save_actor: e.node_actor_id.clone(),
                        purity: e.purity.clone(),
                    }),
                });
            }
        }
    }
    out
}

/// Bake save-derived purity corrections into a world snapshot: for every node
/// carrying a purity override, overwrite the catalog purity with the save's.
/// Applied to the SESSION's in-memory world copy (never the ambient asset), so
/// all downstream reads (`claim_rate`, opportunities, wizard, the map overlay)
/// use the authoritative save purity. Idempotent.
pub fn apply_purity_overrides(
    world: &mut WorldSnapshot,
    overrides: &BTreeMap<String, NodeOverride>,
) {
    for node in world.nodes.iter_mut() {
        if let Some(purity) = overrides.get(&node.id).and_then(|o| o.purity.as_ref()) {
            node.purity = purity.clone();
        }
    }
}

/// Resolved world position of a node id under the plan-local overlay: the
/// catalog coordinate corrected by any override, or the override's own position
/// for a save-only node. `None` when nothing knows where the node is.
pub fn resolved_node_pos(
    world: &WorldSnapshot,
    overrides: &BTreeMap<String, NodeOverride>,
    node: &str,
) -> Option<MapPos> {
    if let Some(ov) = overrides.get(node) {
        if let Some(pos) = ov.pos {
            return Some(pos);
        }
    }
    world.nodes.iter().find(|n| n.id == node).map(|n| MapPos {
        x: n.x,
        y: n.y,
        z: n.z,
    })
}

/// DBSCAN (min_pts 1 ⇒ every machine belongs somewhere) over machine XY.
pub fn cluster(
    snapshot: &ImportSnapshot,
    gd: &gamedata::docs::GameData,
    world: &gamedata::worldnodes::WorldSnapshot,
) -> Vec<Cluster> {
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

    // First pass: centroid + machine groups + name per cluster. Centroids are
    // needed up front so each extractor can be attributed to its NEAREST cluster
    // (a shared miner near two banks belongs to exactly one — no double-claims).
    struct Pre {
        centroid: MapPos,
        groups: Vec<ClusterGroup>,
        name: String,
    }
    let mut pre: Vec<Pre> = Vec::new();
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
        pre.push(Pre {
            centroid: MapPos {
                x: cx,
                y: cy,
                z: cz,
            },
            groups,
            name,
        });
    }

    // Second pass: attribute each extractor to its nearest cluster centroid
    // within the same generous radius the count used, so each miner claims one
    // node under exactly one factory.
    let water_recipes = water_extract_recipes(gd);
    let fracking = fracking_classes(gd);
    let mut attributed: Vec<Vec<ClusterExtractor>> = vec![Vec::new(); pre.len()];
    // A water extractor has NO world node to claim (water is drawn from any
    // surface) — it runs a synthesized zero-ingredient extraction recipe. So it's
    // not a claim but a GROUP producing Desc_Water_C, aggregated per cluster like
    // machine groups, so its water becomes a real routable ◆ built output (and
    // auto-wires to any water consumer in the same cluster) instead of an inert
    // save-only claim that produces nothing.
    // Named `water_groups`/`orphan_pumps` for history, but these now hold EVERY
    // recipe-based extraction group: water pumps, fracking extractors (per
    // resource/purity recipe), and the recipe-less Pressurizer/Activator (empty
    // recipe, power-only). All fold into ordinary ◆ built groups the same way.
    let mut water_groups: Vec<GroupTally> = vec![BTreeMap::new(); pre.len()];
    // Extractors with no machine cluster in range are NOT dropped (that would
    // silently lose real production); they form standalone well/water factories
    // below.
    let mut orphan_pumps: Vec<(String, String, f64, MapPos)> = Vec::new();
    for e in &snapshot.extractors {
        let nearest = pre
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (p.centroid.x - e.x).hypot(p.centroid.y - e.y)))
            .filter(|(_, d)| *d <= DBSCAN_EPS_M * 3.0)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        // The Resource Well Pressurizer produces nothing but draws 150 MW — import
        // it as a recipe-less power-only GROUP (its draw is credited by
        // `inject_activator_power`) that joins its well's factory alongside the
        // satellite extractors, rather than an inert save-local claim.
        if matches!(
            gd.machines.get(e.class.as_str()).map(|m| &m.kind),
            Some(gamedata::docs::MachineKind::Activator)
        ) {
            match nearest {
                Some((i, _)) => {
                    let ent = water_groups[i]
                        .entry((e.class.clone(), String::new()))
                        .or_insert((0, 0.0));
                    ent.0 += 1;
                    ent.1 += e.clock;
                }
                None => orphan_pumps.push((
                    e.class.clone(),
                    String::new(),
                    e.clock,
                    MapPos {
                        x: e.x,
                        y: e.y,
                        z: e.z,
                    },
                )),
            }
            continue;
        }
        // A fluid extractor (has a zero-ingredient extraction recipe) → producing
        // group; a node-bound extractor (miner/oil pump) → node claim. Water pumps
        // use one fixed recipe; fracking extractors resolve theirs (resource +
        // purity) from the satellite they sit on — the (resource, purity) picks a
        // synthesized recipe that already carries the purity-scaled rate, so the
        // group keeps the untouched save clock (purity is NOT folded in).
        let fluid = water_recipes
            .get(e.class.as_str())
            .map(|&r| r.to_string())
            .or_else(|| {
                fracking_group_for(
                    &e.class,
                    e.x,
                    e.y,
                    e.purity.as_deref(),
                    world,
                    gd,
                    &fracking,
                )
            });
        if let Some(recipe) = fluid {
            let clock = e.clock;
            match nearest {
                Some((i, _)) => {
                    let ent = water_groups[i]
                        .entry((e.class.clone(), recipe))
                        .or_insert((0, 0.0));
                    ent.0 += 1;
                    ent.1 += clock;
                }
                None => orphan_pumps.push((
                    e.class.clone(),
                    recipe,
                    clock,
                    MapPos {
                        x: e.x,
                        y: e.y,
                        z: e.z,
                    },
                )),
            }
            continue;
        }
        if let Some((i, _)) = nearest {
            attributed[i].push(ClusterExtractor {
                class: e.class.clone(),
                position: MapPos {
                    x: e.x,
                    y: e.y,
                    z: e.z,
                },
                clock: e.clock,
                node_actor_id: e.node_actor_id.clone(),
                purity: e.purity.clone(),
            });
        }
    }

    // Fold the water-extractor groups into each cluster's machine groups so they
    // build as ordinary ◆ producers (their water nets to a routable OUT port, or
    // wires straight to a water consumer in the same cluster).
    for (i, wg) in water_groups.into_iter().enumerate() {
        for ((machine, recipe), (count, clock_sum)) in wg {
            pre[i].groups.push(ClusterGroup {
                machine,
                recipe,
                count,
                clock: (clock_sum / count as f64 * 1000.0).round() / 1000.0,
            });
        }
    }

    let mut clusters: Vec<Cluster> = pre
        .into_iter()
        .zip(attributed)
        .map(|(p, extractors)| Cluster {
            name: p.name,
            position: p.centroid,
            groups: p.groups,
            extractor_count: extractors.len() as u32,
            extractors,
        })
        .collect();

    // Orphan water pumps (no machine cluster in range) become standalone water
    // factories rather than vanishing. Greedily group orphans within one DBSCAN
    // radius so a pump farm reads as one factory, not one per pump.
    let mut orphan_sites: Vec<(MapPos, GroupTally)> = Vec::new();
    for (class, recipe, clock, pos) in orphan_pumps {
        let bucket = match orphan_sites
            .iter_mut()
            .find(|(c, _)| (c.x - pos.x).hypot(c.y - pos.y) <= DBSCAN_EPS_M)
        {
            Some((_, b)) => b,
            None => {
                orphan_sites.push((pos, BTreeMap::new()));
                &mut orphan_sites.last_mut().unwrap().1
            }
        };
        let ent = bucket.entry((class, recipe)).or_insert((0, 0.0));
        ent.0 += 1;
        ent.1 += clock;
    }
    for (position, tally) in orphan_sites {
        let groups: Vec<ClusterGroup> = tally
            .into_iter()
            .map(|((machine, recipe), (count, clock_sum))| ClusterGroup {
                machine,
                recipe,
                count,
                clock: (clock_sum / count as f64 * 1000.0).round() / 1000.0,
            })
            .collect();
        // Name the well after the fluid it produces — skip the recipe-less
        // Pressurizer group (it would resolve to nothing) and read the first
        // group that has a product. A Pressurizer-only site (no satellites
        // captured) has no fluid to name it by → "RESOURCE WELL", never "WATER".
        let name = groups
            .iter()
            .find_map(|g| gd.recipes.get(&g.recipe))
            .and_then(|r| r.products.first())
            .and_then(|(item, _)| gd.items.get(item))
            .map(|i| i.display_name.to_uppercase())
            .unwrap_or_else(|| "RESOURCE WELL".into());
        clusters.push(Cluster {
            name,
            position,
            groups,
            extractor_count: 0,
            extractors: Vec::new(),
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

/// Lowest transport tier whose capacity covers `rate` — the pipe table for a
/// fluid edge (Mk.1/Mk.2), the belt table otherwise. Built edges store their
/// tier immutably, and the solver reads capacity back through the SAME
/// pipe-vs-belt split, so a fluid edge must be tiered against pipe rates or its
/// derived capacity would be wrong.
fn tier_for(rate: f64, is_fluid: bool) -> u8 {
    let table: &[f64] = if is_fluid {
        &PIPE_CAPACITY
    } else {
        &BELT_CAPACITY
    };
    for (i, cap) in table.iter().enumerate() {
        if rate <= *cap {
            return (i + 1) as u8;
        }
    }
    table.len() as u8
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
    world: &WorldSnapshot,
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
            let fluid = gd.items.get(item).is_some_and(|i| i.is_fluid());
            if let (Some(ps), Some(cs)) = (producers.get(item), consumers.get(item)) {
                for (pg, pr) in ps {
                    for (cg, cr) in cs {
                        edges.push(BeltEdge {
                            id: new_id(),
                            factory: fid.clone(),
                            from: EdgeEnd::Group(pg.clone()),
                            to: EdgeEnd::Group(cg.clone()),
                            item: item.clone(),
                            tier: tier_for(pr.min(*cr), fluid),
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
                        tier: tier_for(net, fluid),
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
                        tier: tier_for(-net, fluid),
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

        // Bind ◆ node claims to real save nodes (W2b-C). Each attributed miner
        // reconciles to the nearest free bundled node (or a plan-local
        // `"save:<id>"`); a position past the drift threshold writes a silent
        // ground-truth correction into the node-overrides overlay. THIS is the
        // first time import creates ◆ claims — closing the "zero claims" gap.
        let mut claim_ids = Vec::new();
        let bound = bind_extractors(state, world, &c.extractors);
        for (e, b) in c.extractors.iter().zip(bound) {
            if let Some(ov) = b.node_override {
                tx.record(state.upsert(Entity::NodeOverride(ov)));
            }
            let claim = NodeClaim {
                id: new_id(),
                node: b.node,
                factory: fid.clone(),
                extractor: e.class.clone(),
                clock: e.clock.clamp(0.01, 2.5),
                save_node_id: b.save_node_id,
                status: Status::Built,
                created_by: CreatedBy::Import(import_id.to_string()),
            };
            claim_ids.push(claim.id.clone());
            tx.record(state.upsert(Entity::NodeClaim(claim)));
        }

        tx.record(state.upsert(Entity::Factory(Factory {
            id: fid.clone(),
            name: c.name.clone(),
            position: c.position,
            region: String::new(),
            node_claims: claim_ids,
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
    /// A claimed node moved in game past the drift threshold (W2b-C): accepting
    /// writes the corrected position into the plan-local node-overrides overlay
    /// (the bundled catalog is never mutated). One undo entry, like the other
    /// ◆-sync ops.
    CorrectNodePosition {
        node: String,
        x: f64,
        y: f64,
        z: f64,
    },
}

/// Re-import: diff clusters against the current Built layer → drift items.
pub fn diff_against_built(
    state: &PlanState,
    gd: &gamedata::docs::GameData,
    clusters: &[Cluster],
    world: &WorldSnapshot,
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
                // group-level diff. Carry each built group's `planned_delta` so a
                // save change that lands on a component the USER also edited can
                // be surfaced as a conflict (mine vs theirs) rather than a plain
                // one-directional drift.
                // (built count, built clock, group id, the user's planned delta)
                type Built = (u32, f64, Id, Option<GroupDelta>);
                let existing: BTreeMap<(String, String), Built> = f
                    .groups
                    .iter()
                    .filter_map(|gid| state.groups.get(gid))
                    .map(|g| {
                        (
                            (g.machine.clone(), g.recipe.clone()),
                            (g.count, g.clock, g.id.clone(), g.planned_delta),
                        )
                    })
                    .collect();
                for g in &c.groups {
                    let key = (g.machine.clone(), g.recipe.clone());
                    match existing.get(&key) {
                        Some((count, clock, _, delta)) if *count == g.count => {
                            // Compare against the clamped save clock — the
                            // value apply_sync will actually store — so an
                            // accepted item converges to InSync on re-import.
                            let target = g.clock.clamp(0.01, 2.5);
                            if (clock - target).abs() > CLOCK_EPS {
                                let op = SyncOp::UpdateGroup {
                                    factory: f.id.clone(),
                                    machine: g.machine.clone(),
                                    recipe: g.recipe.clone(),
                                    count: g.count,
                                    clock: g.clock,
                                };
                                let label = format!(
                                    "Δ {} — {} reclocked in game",
                                    f.name,
                                    item_name(&g.recipe)
                                );
                                // Conflict iff the user retuned the clock to a value
                                // the save doesn't match.
                                match delta
                                    .and_then(|d| d.clock)
                                    .filter(|mc| (mc - target).abs() > CLOCK_EPS)
                                {
                                    // Count is unchanged in the save here, but the
                                    // user may have edited it — "keep mine" keeps
                                    // that too, so show the effective count on the
                                    // mine side (the save side is the built count).
                                    Some(mc) => items.push(conflict_item(
                                        label,
                                        op,
                                        format!(
                                            "×{} @ {:.0}%",
                                            delta.and_then(|d| d.count).unwrap_or(*count),
                                            mc * 100.0
                                        ),
                                        format!("×{count} @ {:.0}%", target * 100.0),
                                    )),
                                    None => items.push(drift_item(
                                        label,
                                        format!(
                                            "{:.1}% built → {:.1}% in save",
                                            clock * 100.0,
                                            target * 100.0
                                        ),
                                        op,
                                    )),
                                }
                            }
                        }
                        Some((count, clock_b, _, delta)) => {
                            let target = g.clock.clamp(0.01, 2.5);
                            let op = SyncOp::UpdateGroup {
                                factory: f.id.clone(),
                                machine: g.machine.clone(),
                                recipe: g.recipe.clone(),
                                count: g.count,
                                clock: g.clock,
                            };
                            let label = format!("Δ {} — {}", f.name, item_name(&g.recipe));
                            // Conflict only where the SAVE changed a component the
                            // user also edited to a different value. Count drifted
                            // by definition here (this is the count-change arm);
                            // clock is a conflict only if the save moved it too
                            // (baseline ≠ save) — a user merely "ahead" on an
                            // unchanged clock is no collision.
                            let mine_count = delta.and_then(|d| d.count).filter(|c| *c != g.count);
                            let clock_drifted = (clock_b - target).abs() > CLOCK_EPS;
                            let mine_clock = delta
                                .and_then(|d| d.clock)
                                .filter(|mc| clock_drifted && (mc - target).abs() > CLOCK_EPS);
                            if mine_count.is_some() || mine_clock.is_some() {
                                // Show the user's true effective values vs the
                                // save's. "Keep mine" preserves the WHOLE delta,
                                // so an edited-but-not-conflicting component is the
                                // user's value too (not just the colliding one);
                                // an unedited component follows the save.
                                let eff_count = delta.and_then(|d| d.count).unwrap_or(g.count);
                                let eff_clock = delta.and_then(|d| d.clock).unwrap_or(target);
                                items.push(conflict_item(
                                    label,
                                    op,
                                    format!("×{eff_count} @ {:.0}%", eff_clock * 100.0),
                                    format!("×{} @ {:.0}%", g.count, target * 100.0),
                                ));
                            } else {
                                items.push(drift_item(
                                    label,
                                    format!("×{count} built → ×{} in save", g.count),
                                    op,
                                ));
                            }
                        }
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
                for ((machine, recipe), (count, clock, _, _)) in &existing {
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
                // Node-position drift (W2b-C): re-match a miner to its ◆ claim by
                // the STABLE save node id, then compare the save's position to the
                // node's RESOLVED position (snapshot ⊕ existing override). A move
                // past the drift threshold is a reviewable correction — never
                // auto-applied. We only reconcile an UNAMBIGUOUS 1:1 match — one
                // miner and one claim under a given save ref: real saves reuse a
                // ref for a whole water volume / SAM cluster (many extractors), and
                // guessing which pump a shared ref means invents drift every time.
                let mut claims_by_save: BTreeMap<&String, Vec<&NodeClaim>> = BTreeMap::new();
                for cl in f
                    .node_claims
                    .iter()
                    .filter_map(|cid| state.node_claims.get(cid))
                {
                    if let Some(sid) = &cl.save_node_id {
                        claims_by_save.entry(sid).or_default().push(cl);
                    }
                }
                let mut miners_by_save: BTreeMap<&String, Vec<&ClusterExtractor>> = BTreeMap::new();
                for e in &c.extractors {
                    if let Some(sid) = &e.node_actor_id {
                        miners_by_save.entry(sid).or_default().push(e);
                    }
                }
                for (sid, miners) in &miners_by_save {
                    if miners.len() != 1 {
                        continue; // shared save ref — ambiguous, skip
                    }
                    let Some(claims) = claims_by_save.get(sid) else {
                        continue;
                    };
                    if claims.len() != 1 {
                        continue;
                    }
                    let claim = claims[0];
                    let e = miners[0];
                    // Both catalog and `save:<id>` nodes reconcile position:
                    // `resolved_node_pos` returns the override pos for a save
                    // node, so a relocated save-only miner past the threshold
                    // emits a reviewable correction like any catalog node.
                    let resolved = resolved_node_pos(world, &state.node_overrides, &claim.node);
                    let moved = resolved
                        .map(|r| (r.x - e.position.x).hypot(r.y - e.position.y))
                        .unwrap_or(f64::INFINITY);
                    if moved > NODE_DRIFT_M {
                        let was = resolved
                            .map(|r| format!("({:.0}, {:.0})", r.x, r.y))
                            .unwrap_or_else(|| "unknown".into());
                        items.push(drift_item(
                            format!("Δ {} — node {} moved in game", f.name, claim.node),
                            format!(
                                "was {was} → ({:.0}, {:.0}) in save",
                                e.position.x, e.position.y
                            ),
                            SyncOp::CorrectNodePosition {
                                node: claim.node.clone(),
                                x: e.position.x,
                                y: e.position.y,
                                z: e.position.z,
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
                    conflict: None,
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

/// Auto-dissolve node overrides that no longer earn their place (W2b-C), mirror
/// of [`crate::buildqueue::dissolve_stale_overrides`]. Called after a re-import
/// drift accept: an override whose catalog node now AGREES with it (the node
/// moved back within the drift threshold) is redundant, and an override whose
/// node no longer has any claim is dangling — both are removed as one undoable
/// move. Save-only overrides (no catalog node) are the sole record of that
/// node's position, so they survive as long as a claim references them.
pub fn dissolve_stale_node_overrides(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    world: &WorldSnapshot,
) {
    use planner_core::state::COLL_NODE_OVERRIDES;
    let claimed: std::collections::BTreeSet<&String> =
        state.node_claims.values().map(|c| &c.node).collect();
    let stale: Vec<String> = state
        .node_overrides
        .values()
        .filter(|ov| {
            // dangling: nothing claims this node anymore
            if !claimed.contains(&ov.id) {
                return true;
            }
            // redundant: the catalog node exists and the POSITION correction now
            // agrees — but only drop when there is no purity correction to keep
            // (the catalog purity is permanently wrong for a randomized/modded
            // node; it never "catches up").
            match (ov.pos, world.nodes.iter().find(|n| n.id == ov.id)) {
                (Some(pos), Some(n)) => {
                    ov.purity.is_none() && (n.x - pos.x).hypot(n.y - pos.y) <= NODE_DRIFT_M
                }
                _ => false,
            }
        })
        .map(|ov| ov.id.clone())
        .collect();
    for id in stale {
        if let Some(ops) = state.remove(COLL_NODE_OVERRIDES, &id) {
            tx.record(ops);
        }
    }
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
        conflict: None,
    }
}

/// A drift item where the save AND an in-app edit both touched this group — the
/// user must pick a side (see [`SyncConflict`]). `mine`/`theirs` are the two
/// candidate values, shown side by side; `choice` starts undecided so accept is
/// blocked until the user resolves it. The `op` is the same `UpdateGroup` that
/// writes the save baseline; "take theirs" additionally clears the delta at
/// apply time (see [`apply_sync`]).
fn conflict_item(label: String, op: SyncOp, mine: String, theirs: String) -> ProposalItem {
    ProposalItem {
        id: new_id(),
        kind: ProposalItemKind::Modify,
        included: true,
        label,
        detail: format!("you: {mine}  ·  save: {theirs}"),
        impact: "CONFLICT".into(),
        commands: vec![],
        aliases: vec![],
        depends_on: vec![],
        sync: Some(serde_json::to_value(op).unwrap()),
        conflict: Some(SyncConflict {
            mine,
            theirs,
            choice: None,
        }),
    }
}

/// Re-derive a ◆ Built factory's logical wiring after a drift-sync group
/// change — the incremental twin of [`write_built_layer`]'s wiring pass.
/// Recomputes per-item production/consumption over the surviving Built
/// groups (baseline count × clock — the ◆ layer is game truth, so
/// `planned_delta` doesn't participate), then:
///
///   - refreshes each Built boundary port's rate to the new net flow,
///   - removes Built ports whose flow dried up (cascading their belts) —
///     except a port anchoring a user-drawn inter-factory route, which is
///     kept at rate 0 so the route survives and its deadness is visible,
///   - creates ports + belts for items that newly cross the boundary,
///   - ensures every Built producer→consumer pair is belted (wires groups
///     added in game), and
///   - raises — never lowers, the player may have overbuilt — belt tiers
///     the new rates outgrew, so a doubled bank isn't capped at its old
///     export tier.
///
/// Only ◆ Built ports/edges are managed; planned (user) wiring in the same
/// factory is left alone, and no port is created for an item the user
/// already models with a planned port.
fn resync_built_wiring(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    fid: &Id,
    import_id: &str,
    gd: &gamedata::docs::GameData,
) {
    let Some(f) = state.factories.get(fid).cloned() else {
        return;
    };
    let mut producers: BTreeMap<String, Vec<(Id, f64)>> = BTreeMap::new();
    let mut consumers: BTreeMap<String, Vec<(Id, f64)>> = BTreeMap::new();
    for g in f.groups.iter().filter_map(|gid| state.groups.get(gid)) {
        if g.status != Status::Built {
            continue;
        }
        let Some(r) = gd.recipes.get(&g.recipe) else {
            continue;
        };
        if r.duration_s <= 0.0 {
            continue;
        }
        let cycles_per_min = 60.0 / r.duration_s * g.count as f64 * g.clock;
        for (item, n) in &r.products {
            producers
                .entry(item.clone())
                .or_default()
                .push((g.id.clone(), n * cycles_per_min));
        }
        for (item, n) in &r.ingredients {
            consumers
                .entry(item.clone())
                .or_default()
                .push((g.id.clone(), n * cycles_per_min));
        }
    }

    // Internal producer→consumer belts: ensure every pair is wired, raising
    // Built tiers the new rates outgrew.
    let all_items: std::collections::BTreeSet<String> =
        producers.keys().chain(consumers.keys()).cloned().collect();
    for item in &all_items {
        let (Some(ps), Some(cs)) = (producers.get(item), consumers.get(item)) else {
            continue;
        };
        let fluid = gd.items.get(item).is_some_and(|i| i.is_fluid());
        for (pg, pr) in ps {
            for (cg, cr) in cs {
                let want = tier_for(pr.min(*cr), fluid);
                let existing = state
                    .edges
                    .values()
                    .find(|e| {
                        e.from == EdgeEnd::Group(pg.clone())
                            && e.to == EdgeEnd::Group(cg.clone())
                            && &e.item == item
                    })
                    .map(|e| e.id.clone());
                match existing {
                    None => tx.record(state.upsert(Entity::Edge(BeltEdge {
                        id: new_id(),
                        factory: fid.clone(),
                        from: EdgeEnd::Group(pg.clone()),
                        to: EdgeEnd::Group(cg.clone()),
                        item: item.clone(),
                        tier: want,
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    }))),
                    Some(eid) => raise_built_tier(state, tx, &eid, want),
                }
            }
        }
    }

    // Boundary ports: every item the factory now nets in or out, plus every
    // item an existing Built port still references (its flow may have dried
    // up entirely).
    let mut port_items = all_items;
    for p in f.ports.iter().filter_map(|pid| state.ports.get(pid)) {
        if p.status == Status::Built {
            port_items.insert(p.item.clone());
        }
    }
    for item in &port_items {
        let prod: f64 = producers
            .get(item)
            .map_or(0.0, |v| v.iter().map(|p| p.1).sum());
        let cons: f64 = consumers
            .get(item)
            .map_or(0.0, |v| v.iter().map(|p| p.1).sum());
        let net = prod - cons;
        let groups = if net > 1e-6 {
            producers.get(item)
        } else {
            consumers.get(item)
        };
        let groups: Vec<Id> = groups
            .into_iter()
            .flatten()
            .map(|(gid, _)| gid.clone())
            .collect();
        for dir in [PortDirection::Out, PortDirection::In] {
            let flows = match dir {
                PortDirection::Out => net > 1e-6,
                PortDirection::In => net < -1e-6,
            };
            let built: Vec<Id> = f
                .ports
                .iter()
                .filter_map(|pid| state.ports.get(pid))
                .filter(|p| p.status == Status::Built && &p.item == item && p.direction == dir)
                .map(|p| p.id.clone())
                .collect();
            if !flows {
                for pid in built {
                    // A bound route is user-drawn wiring (only AddRoute ever
                    // binds one): cascading it away with the dried-up port
                    // would silently destroy planned intent. Keep the port as
                    // the route's anchor at rate 0 — the dead route surfaces
                    // honestly in audits — and cascade only routeless ports.
                    let routed = state
                        .ports
                        .get(&pid)
                        .is_some_and(|p| p.bound_route.is_some());
                    if routed {
                        let mut p = state.ports[&pid].clone();
                        if p.rate.abs() > 1e-9 {
                            p.rate = 0.0;
                            tx.record(state.upsert(Entity::Port(p)));
                        }
                    } else {
                        planner_core::commands::remove_port_cascading(state, tx, &pid);
                    }
                }
                continue;
            }
            let rate = net.abs();
            let pid = match built.first() {
                Some(pid) => {
                    let mut p = state.ports[pid].clone();
                    if (p.rate - rate).abs() > 1e-9 {
                        p.rate = rate;
                        tx.record(state.upsert(Entity::Port(p)));
                    }
                    pid.clone()
                }
                None => {
                    // The user already modeling this boundary with a planned
                    // port means sync must not double it.
                    let planned_exists =
                        f.ports
                            .iter()
                            .filter_map(|pid| state.ports.get(pid))
                            .any(|p| {
                                p.status != Status::Built && &p.item == item && p.direction == dir
                            });
                    if planned_exists {
                        continue;
                    }
                    // Place it beside the groups it serves: outputs to the
                    // right, inputs to the left.
                    let xs = groups.iter().filter_map(|gid| state.groups.get(gid));
                    let graph_pos = match dir {
                        PortDirection::Out => GraphPos {
                            x: xs.map(|g| g.graph_pos.x).fold(0.0, f64::max) + 320.0,
                            y: 100.0,
                        },
                        PortDirection::In => GraphPos {
                            x: xs.map(|g| g.graph_pos.x).fold(320.0, f64::min) - 320.0,
                            y: 100.0,
                        },
                    };
                    let pid = new_id();
                    tx.record(state.upsert(Entity::Port(Port {
                        id: pid.clone(),
                        factory: fid.clone(),
                        direction: dir,
                        item: item.clone(),
                        rate,
                        rate_ceiling: None,
                        bound_route: None,
                        graph_pos,
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    })));
                    if let Some(mut f) = state.factories.get(fid).cloned() {
                        f.ports.push(pid.clone());
                        tx.record(state.upsert(Entity::Factory(f)));
                    }
                    pid
                }
            };
            // Every serving group is belted to the port at a tier that
            // covers the whole net flow (matching write_built_layer).
            let want = tier_for(rate, gd.items.get(item).is_some_and(|i| i.is_fluid()));
            for gid in &groups {
                let (from, to) = match dir {
                    PortDirection::Out => (EdgeEnd::Group(gid.clone()), EdgeEnd::Port(pid.clone())),
                    PortDirection::In => (EdgeEnd::Port(pid.clone()), EdgeEnd::Group(gid.clone())),
                };
                let existing = state
                    .edges
                    .values()
                    .find(|e| e.from == from && e.to == to && &e.item == item)
                    .map(|e| e.id.clone());
                match existing {
                    None => tx.record(state.upsert(Entity::Edge(BeltEdge {
                        id: new_id(),
                        factory: fid.clone(),
                        from,
                        to,
                        item: item.clone(),
                        tier: want,
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    }))),
                    Some(eid) => raise_built_tier(state, tx, &eid, want),
                }
            }
        }
    }
}

/// Raise a Built belt to `want` if its current tier can't carry the resynced
/// rate. Never lowers (the player may have overbuilt) and never touches
/// planned belts (user wiring).
fn raise_built_tier(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    eid: &Id,
    want: u8,
) {
    let Some(e) = state.edges.get(eid) else {
        return;
    };
    if e.status == Status::Built && e.tier < want {
        let mut e = e.clone();
        e.tier = want;
        tx.record(state.upsert(Entity::Edge(e)));
    }
}

/// Apply one sync op to the Built layer (accept path for SaveReimport items).
/// `take_save` is only meaningful for a resolved CONFLICT item: `true` ("take
/// save") drops the user's `planned_delta` so the effective value becomes the
/// save's; `false` (non-conflict item, or "keep mine") preserves the delta,
/// dissolving only components the game has caught up to.
pub fn apply_sync(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    op: &SyncOp,
    import_id: &str,
    gd: &gamedata::docs::GameData,
    world: &WorldSnapshot,
    take_save: bool,
) {
    match op {
        SyncOp::CreateCluster { cluster } => {
            write_built_layer(
                state,
                tx,
                std::slice::from_ref(cluster),
                import_id,
                gd,
                world,
            );
        }
        SyncOp::CorrectNodePosition { node, x, y, z } => {
            // Plan-local overlay write (the one documented ◆-sync exception) —
            // the bundled catalog stays untouched. Preserve any save actor ref
            // the binding recorded so the node keeps re-matching by stable id.
            let save_actor = state
                .node_overrides
                .get(node)
                .and_then(|o| o.save_actor.clone())
                .or_else(|| {
                    state
                        .node_claims
                        .values()
                        .find(|c| &c.node == node)
                        .and_then(|c| c.save_node_id.clone())
                });
            // Preserve any purity override this node already carries — a
            // position correction must not drop the save-derived purity.
            let purity = state
                .node_overrides
                .get(node)
                .and_then(|o| o.purity.clone());
            tx.record(state.upsert(Entity::NodeOverride(NodeOverride {
                id: node.clone(),
                pos: Some(MapPos {
                    x: *x,
                    y: *y,
                    z: *z,
                }),
                save_actor,
                purity,
            })));
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
                    if take_save {
                        // Conflict resolved "take save": the save wins wholesale,
                        // so drop the user's overlay — effective == baseline.
                        g.planned_delta = None;
                    } else if let Some(mut d) = g.planned_delta {
                        // Sync writes the baseline but keeps the user's planned
                        // delta — except components the game caught up to, which
                        // dissolve ("visible until built in-game").
                        if d.count == Some(g.count) {
                            d.count = None;
                        }
                        if d.clock.is_some_and(|c| (c - g.clock).abs() < 1e-9) {
                            d.clock = None;
                        }
                        g.planned_delta = (!d.is_empty()).then_some(d);
                    }
                    tx.record(state.upsert(Entity::Group(g)));
                    // The changed count/clock moves the factory's net flows:
                    // refresh boundary port rates + belt tiers so an expanded
                    // bank isn't capped at its stale export.
                    resync_built_wiring(state, tx, factory, import_id, gd);
                }
                Some(g) => {
                    // Demolished in game: cascade like DeleteGroup (belts,
                    // factory ref, build override), then resync the boundary
                    // so ports the group fed/drained don't survive as orphans
                    // reporting phantom flow.
                    planner_core::commands::remove_group_cascading(state, tx, &g.id);
                    resync_built_wiring(state, tx, factory, import_id, gd);
                }
                None if *count > 0 => {
                    let gid = new_id();
                    // Land below the factory's existing cards instead of on
                    // top of them.
                    let below = f
                        .groups
                        .iter()
                        .filter_map(|id| state.groups.get(id))
                        .map(|g| g.graph_pos.y)
                        .fold(f64::NEG_INFINITY, f64::max);
                    let graph_pos = GraphPos {
                        x: 280.0,
                        y: if below.is_finite() {
                            below + 260.0
                        } else {
                            80.0
                        },
                    };
                    tx.record(state.upsert(Entity::Group(MachineGroup {
                        id: gid.clone(),
                        factory: factory.clone(),
                        machine: machine.clone(),
                        recipe: recipe.clone(),
                        count: *count,
                        clock: clock.clamp(0.01, 2.5),
                        somersloops: 0,
                        planned_delta: None,
                        graph_pos,
                        floor: 0,
                        status: Status::Built,
                        created_by: CreatedBy::Import(import_id.to_string()),
                    })));
                    let mut f = f;
                    f.groups.push(gid);
                    tx.record(state.upsert(Entity::Factory(f)));
                    // Auto-wire the group added in game the way first import
                    // would have: belts to/from its recipe partners and a
                    // refreshed boundary.
                    resync_built_wiring(state, tx, factory, import_id, gd);
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
