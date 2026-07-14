//! Refactor/cutover projection (W2a) — a DERIVED overlay, recomputed every
//! empire solve like the build queue. A `Cutover` exists for each ◇ factory
//! whose `replaces` names a running ◆ factory: it is an ordered plan for
//! swapping the old factory out for the new one WITHOUT ever mutating the ◆
//! built layer. Three phases, and the ORDER is the pain relief (never tear the
//! old factory down before the replacement is up):
//!
//!   0 BuildNew  — the new ◇ factory's build steps (reuses the build queue /
//!                 `nearest_built_match`; completion = a built twin appears).
//!   1 Switch    — one step per item the old factory supplies downstream; the
//!                 save can't see belt connectivity, so completion is MANUAL,
//!                 keyed on a synthetic `switch:<oldFid>:<item>` override.
//!   2 Dismantle — the old ◆ factory. Completion is DERIVED: Done when the ◆
//!                 disappears from state (game executed it, re-import synced it)
//!                 OR a `BuildOverride` pins it. NEVER a write to the ◆ entity —
//!                 "dismantle" is INTENT, a referenced id, never a mutation.
//!
//! The downtime engine (`Session::cutover_plan`) is on-demand only — it scratch-
//! solves the empire at each phase boundary and reports honest, ripple-inclusive
//! production dips. That lives in `session.rs`; this module is the pure, cheap
//! presence/steps projection plus the boundary-shaping helper it reuses.

use planner_core::commands::{remove_factory_cascading, Transaction};
use planner_core::entities::*;
use planner_core::state::{Entity, PlanState};
use serde::Serialize;

use gamedata::docs::GameData;

use crate::buildqueue::{derive_build_queue, BuildStep, BuildStepState};

/// Minutes of downtime attributed to tearing down one machine — a DOCUMENTED
/// estimate (Principle 10: the production *rate* in a dip is computed and honest;
/// only the wall-clock is an estimate, always rendered "(est)"). Mid/late-game
/// machine teardown-and-rebuild lands around a few minutes each once you factor
/// dismantling, re-siting, and re-belting; 4 min/machine is a conservative peg.
pub const SWITCH_MIN_PER_MACHINE: f64 = 4.0;

/// Cutover phase, in execution order. The numeric value IS the ordering key and
/// the boundary index the downtime engine solves at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverPhase {
    BuildNew = 0,
    Switch = 1,
    Dismantle = 2,
}

/// One step of a cutover, in the ◇◈◆ completion grammar (reused from the build
/// queue). BuildNew/Dismantle derive completion from the ◆ layer; Switch is
/// manual-only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutoverStep {
    /// Step key: the factory id (BuildNew/Dismantle) or a synthetic
    /// `switch:<oldFid>:<item>` (Switch) — the id a `BuildOverride` pins.
    pub id: Id,
    pub phase: CutoverPhase,
    /// Owning factory, for the dashboard's "go there" navigation.
    pub factory: Option<Id>,
    pub label: String,
    pub detail: String,
    /// Derived completion (ignores the override) — drives the ◇◈◆ glyph.
    pub state: BuildStepState,
    /// Resolved answer: `override ?? (state == Done)`.
    pub done: bool,
    /// A manual `BuildOverride` is pinning `done`.
    pub overridden: bool,
    /// Completion CANNOT be auto-detected (Switch: belt connectivity is
    /// invisible to the save) — the UI must label the check manual.
    pub manual_only: bool,
}

/// A derived cutover: the ◇ replacement, the ◆ it replaces, and the ordered
/// steps. Lightweight — the N+1 scratch-solves that price the downtime happen
/// only on demand in [`crate::session::Session::cutover_plan`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cutover {
    /// The ◇ planned replacement (the factory carrying `replaces`).
    pub new_factory: Id,
    pub new_name: String,
    /// The ◆ factory being retired. May already be gone (dismantle complete).
    pub old_factory: Id,
    pub old_name: String,
    pub steps: Vec<CutoverStep>,
    /// The new ◇ claims a WorldNodeId the old ◆ still holds — they cannot run
    /// simultaneously, so downtime for the build window is UNAVOIDABLE. This
    /// also naturally lights the existing `DerivedNode.conflict` marker.
    pub node_reuse: bool,
    /// Ordering key: the creating proposal's number (0 = MANUAL bucket).
    pub number: u32,
}

/// A single tracked-item production dip at a phase boundary — the honest cost of
/// the cutover. `rate` and `baseline` are COMPUTED (real scratch-solve output);
/// `est_hours` is the labeled machine-count estimate.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dip {
    /// Boundary this dip is measured at: 1 = Switch, 2 = Dismantle.
    pub phase: u8,
    pub item: String,
    /// Achieved production at this boundary (items/min) — computed.
    pub rate: f64,
    /// Baseline production before the cutover (k=0) — computed.
    pub baseline: f64,
    /// Estimated wall-clock of the downtime (torn-down machines × const).
    pub est_hours: f64,
}

/// On-demand downtime pricing for one cutover: the tracked items, their
/// baseline, and the per-boundary dips. Scratch-solved (ripple-inclusive), never
/// stored — fetched via the endpoint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CutoverPlan {
    pub new_factory: Id,
    pub old_factory: Id,
    /// Items the old factory supplies (what the cutover puts at risk).
    pub tracked: Vec<String>,
    /// Baseline production per tracked item (production at boundary k=0).
    pub baseline: std::collections::BTreeMap<String, f64>,
    /// Production per tracked item at each boundary k = 0, 1, 2.
    pub production: Vec<std::collections::BTreeMap<String, f64>>,
    pub dips: Vec<Dip>,
    /// Node-reuse: unavoidable downtime for the build window — surfaced loudly.
    pub hard: bool,
    /// Whether the downtime could actually be COMPUTED. False when the old
    /// factory declares positive output (its Out ports carry a positive rate)
    /// but the scratch-solve yields ~0 baseline for those items — an imported /
    /// unsolved / starved factory that does not produce in the current solve.
    /// A silent-empty `dips` is dishonest; this discriminates "no impact" (true,
    /// empty dips) from "can't compute" (false, with a reason). Transient — a
    /// derived result field, never persisted into plan state.
    pub downtime_available: bool,
    /// Human-readable reason set when `downtime_available` is false (else None).
    pub unavailable_reason: Option<String>,
}

/// Ordering stamp for a cutover's steps: the proposal that created the new ◇
/// factory, else 0 (MANUAL bucket).
fn factory_number(state: &PlanState, created_by: &CreatedBy) -> u32 {
    match created_by {
        CreatedBy::Proposal(pid) => state.proposals.get(pid).map(|p| p.number).unwrap_or(0),
        _ => 0,
    }
}

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

/// The synthetic override key for a Switch step (one per item the old factory
/// supplies). Manual-only, so the id is not a real entity.
pub fn switch_step_id(old_factory: &Id, item: &str) -> String {
    format!("switch:{old_factory}:{item}")
}

/// Items a factory supplies downstream — its Out ports' item classes, sorted
/// and de-duplicated. Drives the Switch phase (one step per belted item) and the
/// downtime engine's tracked-item set.
pub(crate) fn supplied_items(state: &PlanState, factory: &Factory) -> Vec<String> {
    let mut items: Vec<String> = factory
        .ports
        .iter()
        .filter_map(|pid| state.ports.get(pid))
        .filter(|p| p.direction == PortDirection::Out)
        .map(|p| p.item.clone())
        .collect();
    items.sort();
    items.dedup();
    items
}

/// Derive every cutover from canonical state, reusing an ALREADY-COMPUTED build
/// queue (the empire-solve path computes it once into `derived.build_queue` and
/// hands it here, avoiding a second `derive_build_queue` pass). Pure over
/// `(state, gamedata, build_steps)`.
pub fn derive_cutovers_with(
    state: &PlanState,
    gd: &GameData,
    build_steps_queue: &[BuildStep],
) -> Vec<Cutover> {
    // Build-queue completion for the BuildNew phase, indexed by step id.
    let build_steps: std::collections::BTreeMap<Id, (BuildStepState, bool, bool)> =
        build_steps_queue
            .iter()
            .map(|s| (s.id.clone(), (s.state, s.done, s.overridden)))
            .collect();

    let resolve_override = |id: &Id, derived_done: bool| -> (bool, bool) {
        match state.build_overrides.get(id) {
            Some(o) => (o.done, true),
            None => (derived_done, false),
        }
    };

    let mut cutovers: Vec<Cutover> = Vec::new();
    for f in state.factories.values() {
        let Some(old_id) = f.replaces.clone() else {
            continue;
        };
        let old = state.factories.get(&old_id);
        let number = factory_number(state, &f.created_by);
        let mut steps: Vec<CutoverStep> = Vec::new();

        // ---- Phase 0: BuildNew — the new ◇ factory's build step ----
        // Reuse the build queue's factory rollup (derived from the ◆ layer via
        // nearest_built_match); fall back to Done if the new factory is itself
        // already built (no planned step).
        let (bstate, bdone, boverridden) =
            build_steps.get(&f.id).copied().unwrap_or(match f.status {
                Status::Built => (BuildStepState::Done, true, false),
                _ => (BuildStepState::Pending, false, false),
            });
        steps.push(CutoverStep {
            id: f.id.clone(),
            phase: CutoverPhase::BuildNew,
            factory: Some(f.id.clone()),
            label: format!("BUILD {}", f.name),
            detail: "stand up the replacement beside the old factory".into(),
            state: bstate,
            done: bdone,
            overridden: boverridden,
            manual_only: false,
        });

        // ---- Phase 1: Switch — one manual step per item the old ◆ supplies ----
        if let Some(old) = old {
            for item in supplied_items(state, old) {
                let sid = switch_step_id(&old_id, &item);
                let (done, overridden) = resolve_override(&sid, false);
                steps.push(CutoverStep {
                    id: sid,
                    phase: CutoverPhase::Switch,
                    factory: Some(f.id.clone()),
                    label: format!("SWITCH {} feed", item_name(gd, &item)),
                    detail: "repoint the belts to the new factory — mark when done".into(),
                    state: BuildStepState::Pending,
                    done,
                    overridden,
                    manual_only: true,
                });
            }
        }

        // ---- Phase 2: Dismantle — the old ◆ factory ----
        // Completion is DERIVED: Done when the ◆ is gone (re-import synced the
        // teardown) OR a BuildOverride pins it. A missing target reads Done.
        let (dstate, ddone, doverridden) = match old {
            None => (BuildStepState::Done, true, false),
            Some(_) => {
                let (done, overridden) = resolve_override(&old_id, false);
                (BuildStepState::Pending, done, overridden)
            }
        };
        steps.push(CutoverStep {
            id: old_id.clone(),
            phase: CutoverPhase::Dismantle,
            factory: old.map(|_| old_id.clone()),
            label: format!(
                "DISMANTLE {}",
                old.map(|o| o.name.clone())
                    .unwrap_or_else(|| old_id.clone())
            ),
            detail: match old {
                Some(_) => "tear the old factory down in-game (re-import syncs it)".into(),
                None => "gone — re-import synced the teardown".into(),
            },
            state: dstate,
            done: ddone,
            overridden: doverridden,
            manual_only: false,
        });

        // Order: (phase, id) — every step in a cutover shares the same proposal
        // `number`, so the ordering key collapses to (phase, id); ULIDs sort
        // chronologically within a phase.
        steps.sort_by(|a, b| a.phase.cmp(&b.phase).then_with(|| a.id.cmp(&b.id)));

        // Node reuse: the new ◇ claims a node the old ◆ still holds.
        let old_nodes: std::collections::BTreeSet<&str> = old
            .map(|o| {
                o.node_claims
                    .iter()
                    .filter_map(|cid| state.node_claims.get(cid))
                    .map(|c| c.node.as_str())
                    .collect()
            })
            .unwrap_or_default();
        let node_reuse = f
            .node_claims
            .iter()
            .filter_map(|cid| state.node_claims.get(cid))
            .any(|c| old_nodes.contains(c.node.as_str()));

        cutovers.push(Cutover {
            new_factory: f.id.clone(),
            new_name: f.name.clone(),
            old_factory: old_id.clone(),
            old_name: old
                .map(|o| o.name.clone())
                .unwrap_or_else(|| old_id.clone()),
            steps,
            node_reuse,
            number,
        });
    }

    // Stable order: (proposal number, new-factory ULID).
    cutovers.sort_by(|a, b| {
        a.number
            .cmp(&b.number)
            .then_with(|| a.new_factory.cmp(&b.new_factory))
    });
    cutovers
}

/// Derive every cutover from canonical state. Pure over `(state, gamedata)`.
/// Thin wrapper for on-demand callers (dissolve path, `cutover_plan`) that don't
/// already hold a build queue — it computes one and delegates to
/// [`derive_cutovers_with`].
pub fn derive_cutovers(state: &PlanState, gd: &GameData) -> Vec<Cutover> {
    derive_cutovers_with(state, gd, &derive_build_queue(state, gd))
}

/// Shape canonical state to a phase boundary `k` for a downtime scratch-solve.
/// Clones `base`, then (via a throwaway transaction) removes the members that are
/// not producing at boundary `k`: the new ◇ factory while it is not yet fully
/// online (`k < Dismantle=2`, so its output can't inflate the picture), and the
/// old ◆ factory once the switch begins (`k >= Switch=1`, because it is being
/// torn down).
///
/// The intermediate boundary (`k = 1`, the Switch window) is therefore the honest
/// worst case — old already down, new not yet up — which is exactly the downtime
/// the player feels. Removal cascades through routes so the FULL downstream
/// ripple (retire the screw factory → screws AND everything fed by screws dip) is
/// captured on the re-solve. See the `downtime_drop_across_boundaries` test.
pub fn shape_for_boundary(base: &PlanState, cutover: &Cutover, k: usize) -> PlanState {
    let mut state = base.clone();
    let mut tx = Transaction::new("cutover-scratch");
    // New ◇ online only once fully switched over (k >= 2).
    if k < CutoverPhase::Dismantle as usize && state.factories.contains_key(&cutover.new_factory) {
        remove_factory_cascading(&mut state, &mut tx, &cutover.new_factory);
    }
    // Old ◆ torn down as the switch begins (k >= 1).
    if k >= CutoverPhase::Switch as usize && state.factories.contains_key(&cutover.old_factory) {
        remove_factory_cascading(&mut state, &mut tx, &cutover.old_factory);
    }
    state
}

/// Estimated downtime hours for tearing down `machine_count` machines — the
/// labeled wall-clock estimate (the rate in a dip is computed; this is not).
pub fn est_hours(machine_count: u32) -> f64 {
    machine_count as f64 * SWITCH_MIN_PER_MACHINE / 60.0
}

/// Auto-null any `replaces` pointing at a now-removed factory (mirrors the
/// planned-delta dissolve + `dissolve_stale_overrides`). Called after a re-import
/// drift accept: once the old ◆ is gone, the link is dangling intent — clear it
/// so the cutover reads dismantle-complete on its own. Recorded into `tx` so the
/// drop is folded into the same undoable move.
pub fn dissolve_stale_replaces(state: &mut PlanState, tx: &mut Transaction) {
    let stale: Vec<Id> = state
        .factories
        .values()
        .filter(|f| {
            f.replaces
                .as_ref()
                .is_some_and(|old| !state.factories.contains_key(old))
        })
        .map(|f| f.id.clone())
        .collect();
    for fid in stale {
        if let Some(mut f) = state.factories.get(&fid).cloned() {
            f.replaces = None;
            tx.record(state.upsert(Entity::Factory(f)));
        }
    }
}
