//! Build queue (W1c) — a DERIVED projection over canonical state, recomputed
//! every empire solve like circuits and deficits. There is NO ordering entity
//! and NO stored done-flag: a "step" is simply an existing ◇ planned (or
//! partially-built) entity, its completion is derived from the ◆ built layer
//! (◇ todo, ◈ half-built rollup, ◆ done — ◈ is never written to `status`), and
//! order is `(proposal number, entity ULID)` with manual-created entities in a
//! MANUAL bucket (number 0). The one piece of un-derivable, undoable state is
//! the manual `BuildOverride` overlay, resolved here as `override ?? derived`.
//!
//! Pure: `derive_build_queue(state, gamedata)` reads state + gamedata only, so
//! it is unit-testable without a solver.

use planner_core::entities::*;
use planner_core::state::PlanState;
use serde::Serialize;

use gamedata::docs::GameData;

use crate::cutover::derive_cutovers;
use crate::import::nearest_built_match;

/// Derived completion of a step, in the ◇◈◆ grammar. `Partial` (◈) is a
/// rollup only — it is never written to any entity's `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStepState {
    /// ◇ nothing of this step is built in-game yet.
    Pending,
    /// ◈ some of a factory's groups have a built twin, not all.
    Partial,
    /// ◆ the built layer covers this step.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildStepKind {
    Factory,
    Group,
    Route,
    Claim,
}

/// Milestone "built so far" against the total the game handed the player.
/// `built` is current empire production of the item from ◆ built groups
/// (a standing-capacity proxy — the save has no lifetime-crafted counter).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildProgress {
    pub item: String,
    pub built: f64,
    pub total: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildStep {
    pub id: Id,
    pub kind: BuildStepKind,
    /// Owning factory, for the dashboard's "go there" navigation.
    pub factory: Option<Id>,
    pub label: String,
    pub detail: String,
    /// Derived completion (ignores the override) — drives the ◇◈◆ glyph.
    pub state: BuildStepState,
    /// Resolved answer: `override ?? (state == Done)`.
    pub done: bool,
    /// True when a manual `BuildOverride` is pinning `done`.
    pub overridden: bool,
    /// Completion CANNOT be auto-detected (routes/claims: the save carries
    /// machines, not belt connectivity) — the UI must label the check manual.
    pub manual_only: bool,
    /// Ordering key: creating proposal's number, 0 for MANUAL/import.
    pub number: u32,
    /// Milestone progress, when this step's proposal carries a milestone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<BuildProgress>,
}

/// Order stamp for a step: the number of the proposal that created it, else 0
/// (MANUAL / import bucket).
fn step_number(state: &PlanState, created_by: &CreatedBy) -> u32 {
    match created_by {
        CreatedBy::Proposal(pid) => state.proposals.get(pid).map(|p| p.number).unwrap_or(0),
        _ => 0,
    }
}

/// Does a ◆ built twin near `pos` already have a group of this (machine,
/// recipe)? The per-group completion test behind factory rollups.
///
/// PRESENCE, not count/clock, is intentional: this shares the coarse 250m
/// nearest-match rule with re-import drift detection — a twin group of the same
/// (machine, recipe) means "this step exists in-game", and magnitude (how many /
/// what clock) lives in the milestone bar, not here. A count-sensitive test would
/// double-flag the same build as drift AND incomplete.
fn built_twin_has(state: &PlanState, pos: &MapPos, machine: &str, recipe: &str) -> bool {
    let Some(twin) = nearest_built_match(state, pos) else {
        return false;
    };
    twin.groups
        .iter()
        .filter_map(|gid| state.groups.get(gid))
        .any(|g| g.status == Status::Built && g.machine == machine && g.recipe == recipe)
}

/// Gross production rate (items/min) of `item` from the ◆ built layer — the
/// milestone "built" figure. Baseline count/clock (ground truth), not the
/// planned delta.
fn built_production(state: &PlanState, gd: &GameData, item: &str) -> f64 {
    let mut total = 0.0;
    for g in state.groups.values() {
        if g.status != Status::Built {
            continue;
        }
        let Some(recipe) = gd.recipes.get(&g.recipe) else {
            continue;
        };
        if recipe.duration_s <= 0.0 {
            continue;
        }
        let cycles_per_min = 60.0 / recipe.duration_s * g.count as f64 * g.clock;
        for (out_item, qty) in &recipe.products {
            if out_item == item {
                total += qty * cycles_per_min;
            }
        }
    }
    total
}

/// Recipe display name, falling back to the trimmed class.
fn recipe_name(gd: &GameData, recipe: &str) -> String {
    gd.recipes
        .get(recipe)
        .map(|r| r.display_name.clone())
        .unwrap_or_else(|| {
            recipe
                .trim_start_matches("Recipe_")
                .trim_end_matches("_C")
                .to_string()
        })
}

/// Item display name, falling back to the trimmed class.
fn item_name(gd: &GameData, item: &str) -> String {
    gd.items
        .get(item)
        .map(|i| i.display_name.clone())
        .unwrap_or_else(|| {
            item.trim_start_matches("Desc_")
                .trim_end_matches("_C")
                .to_string()
        })
}

/// Derive the whole build queue. Pure over `(state, gamedata)`.
pub fn derive_build_queue(state: &PlanState, gd: &GameData) -> Vec<BuildStep> {
    let mut steps: Vec<BuildStep> = Vec::new();

    // Milestone progress, attached to a step when its creating proposal carries a
    // milestone. `built_production` is a full-empire group scan, so memoize it per
    // distinct milestone ITEM (many steps share a proposal, hence an item) — this
    // turns an O(steps × groups) cost into O(distinct-items × groups).
    let production_cache: std::cell::RefCell<std::collections::BTreeMap<String, f64>> =
        std::cell::RefCell::new(std::collections::BTreeMap::new());
    let progress_for = |created_by: &CreatedBy| -> Option<BuildProgress> {
        let CreatedBy::Proposal(pid) = created_by else {
            return None;
        };
        let m = state.proposals.get(pid)?.milestone.as_ref()?;
        let built = *production_cache
            .borrow_mut()
            .entry(m.item.clone())
            .or_insert_with(|| built_production(state, gd, &m.item));
        Some(BuildProgress {
            item: m.item.clone(),
            built,
            total: m.total,
        })
    };

    let resolve = |id: &Id, derived_done: bool| -> (bool, bool) {
        match state.build_overrides.get(id) {
            Some(o) => (o.done, true),
            None => (derived_done, false),
        }
    };

    // 1) Planned factories → a rollup step over their groups.
    for f in state.factories.values() {
        if f.status != Status::Planned {
            continue;
        }
        let groups: Vec<&MachineGroup> = f
            .groups
            .iter()
            .filter_map(|gid| state.groups.get(gid))
            .collect();
        let (mut done_n, total_n) = (0usize, groups.len());
        for g in &groups {
            let built = g.status == Status::Built
                || built_twin_has(state, &f.position, &g.machine, &g.recipe);
            if built {
                done_n += 1;
            }
        }
        let derived = if total_n == 0 || done_n == 0 {
            BuildStepState::Pending
        } else if done_n == total_n {
            BuildStepState::Done
        } else {
            BuildStepState::Partial
        };
        let (done, overridden) = resolve(&f.id, derived == BuildStepState::Done);
        steps.push(BuildStep {
            id: f.id.clone(),
            kind: BuildStepKind::Factory,
            factory: Some(f.id.clone()),
            label: f.name.clone(),
            detail: if total_n == 0 {
                "empty planned site".into()
            } else {
                format!("{done_n}/{total_n} machine groups built in-game")
            },
            state: derived,
            done,
            overridden,
            manual_only: false,
            number: step_number(state, &f.created_by),
            progress: progress_for(&f.created_by),
        });
    }

    // 2) Standalone group steps: a ◇ planned group whose factory is NOT itself a
    //    planned-factory step (already rolled up above), or a ◆ built group
    //    carrying a planned delta not yet reflected in-game.
    for g in state.groups.values() {
        let owner_planned = state
            .factories
            .get(&g.factory)
            .map(|f| f.status == Status::Planned)
            .unwrap_or(false);
        let is_delta = g.status == Status::Built && g.planned_delta.is_some();
        let is_planned_standalone = g.status == Status::Planned && !owner_planned;
        if !is_delta && !is_planned_standalone {
            continue;
        }
        let derived = if is_delta {
            // The plan wants a different count/clock than the built baseline —
            // Pending until the game catches up (the delta then dissolves and
            // the step disappears), mirroring import sync.
            BuildStepState::Pending
        } else if state
            .factories
            .get(&g.factory)
            .map(|f| built_twin_has(state, &f.position, &g.machine, &g.recipe))
            .unwrap_or(false)
        {
            BuildStepState::Done
        } else {
            BuildStepState::Pending
        };
        let (done, overridden) = resolve(&g.id, derived == BuildStepState::Done);
        let fname = state
            .factories
            .get(&g.factory)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        steps.push(BuildStep {
            id: g.id.clone(),
            kind: BuildStepKind::Group,
            factory: Some(g.factory.clone()),
            label: format!("{fname} · {}", recipe_name(gd, &g.recipe)),
            detail: if is_delta {
                format!(
                    "plan ×{} @ {:.0}%",
                    g.effective_count(),
                    g.effective_clock() * 100.0
                )
            } else {
                format!("×{} @ {:.0}%", g.count, g.clock * 100.0)
            },
            state: derived,
            done,
            overridden,
            manual_only: false,
            number: step_number(state, &g.created_by),
            progress: progress_for(&g.created_by),
        });
    }

    // 3) Planned routes — manual-only (belt connectivity is invisible to the save).
    for r in state.routes.values() {
        if r.status != Status::Planned {
            continue;
        }
        let (done, overridden) = resolve(&r.id, false);
        let item = r.manifest.first().map(|(i, _)| item_name(gd, i));
        let endpoints = {
            let name = |pid: &Id| {
                state
                    .ports
                    .get(pid)
                    .and_then(|p| state.factories.get(&p.factory))
                    .or_else(|| state.factories.get(pid))
                    .map(|f| f.name.clone())
            };
            match (name(&r.endpoints.0), name(&r.endpoints.1)) {
                (Some(a), Some(b)) => format!("{a} → {b}"),
                _ => "route".into(),
            }
        };
        steps.push(BuildStep {
            id: r.id.clone(),
            kind: BuildStepKind::Route,
            factory: None,
            label: match item {
                Some(i) => format!("{endpoints} · {i}"),
                None => endpoints,
            },
            detail: "can't detect in-game — mark when built".into(),
            state: BuildStepState::Pending,
            done,
            overridden,
            manual_only: true,
            number: step_number(state, &r.created_by),
            progress: progress_for(&r.created_by),
        });
    }

    // 4) Planned node claims — manual-only for the same reason.
    for c in state.node_claims.values() {
        if c.status != Status::Planned {
            continue;
        }
        let (done, overridden) = resolve(&c.id, false);
        let fname = state
            .factories
            .get(&c.factory)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        steps.push(BuildStep {
            id: c.id.clone(),
            kind: BuildStepKind::Claim,
            factory: Some(c.factory.clone()),
            label: format!("{fname} · claim {}", c.node),
            detail: "can't detect in-game — mark when built".into(),
            state: BuildStepState::Pending,
            done,
            overridden,
            manual_only: true,
            number: step_number(state, &c.created_by),
            progress: progress_for(&c.created_by),
        });
    }

    // Order: (proposal number, entity ULID) — ULIDs are time-sortable, so this
    // reads chronologically within each proposal, MANUAL (0) first.
    steps.sort_by(|a, b| a.number.cmp(&b.number).then_with(|| a.id.cmp(&b.id)));
    steps
}

/// Auto-dissolve redundant / dangling overrides against freshly-derived state
/// (mirrors the planned-delta dissolve on import sync, import.rs). Called after
/// a re-import drift accept: an override that now AGREES with the derived answer
/// is redundant, and an override whose step no longer exists is dangling —
/// both are removed, recorded into `tx` so the drop is one undoable move.
pub fn dissolve_stale_overrides(
    state: &mut PlanState,
    tx: &mut planner_core::commands::Transaction,
    gd: &GameData,
) {
    use planner_core::state::COLL_BUILD_OVERRIDES;
    // Derived answer per step id, ignoring the override (state == Done).
    let mut derived_done: std::collections::BTreeMap<Id, bool> = derive_build_queue(state, gd)
        .into_iter()
        .map(|s| (s.id, s.state == BuildStepState::Done))
        .collect();
    // Cutover step ids (Switch `switch:…` synthetics and the Dismantle key on the
    // old factory id) are ALSO valid override targets but are NOT build-queue
    // steps, so the dangling test above would wrongly drop their overrides. Union
    // them in with their derived completion (Switch is always Pending ⇒ false; the
    // manual override then reads as a real disagreement and survives). `or_insert`
    // keeps any build-queue answer that already covers the id (e.g. the BuildNew
    // step, which is a real factory step).
    for c in derive_cutovers(state, gd) {
        for s in c.steps {
            derived_done
                .entry(s.id)
                .or_insert(s.state == BuildStepState::Done);
        }
    }
    let redundant_or_dangling: Vec<Id> = state
        .build_overrides
        .values()
        .filter(|o| match derived_done.get(&o.id) {
            Some(d) => o.done == *d, // override agrees with derived ⇒ redundant
            None => true,            // no step ⇒ dangling
        })
        .map(|o| o.id.clone())
        .collect();
    for id in redundant_or_dangling {
        if let Some(ops) = state.remove(COLL_BUILD_OVERRIDES, &id) {
            tx.record(ops);
        }
    }
}
