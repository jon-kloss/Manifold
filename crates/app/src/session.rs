//! One open plan file + its canonical state, undo log, gamedata, and solver
//! orchestration. Every mutation: apply commands → T1 re-solve → fold solve
//! write-backs into the same transaction → persist → commit (one undo entry).
//! Disk commits first — see [`Session::commit_mutation`] for the invariant.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use planner_core::commands::{self, Command, DomainError, Transaction};
use planner_core::entities::*;
use planner_core::patch::PatchBatch;
use planner_core::proposals::{fnv1a, resolve_aliases, Proposal, ProposalItem, ProposalStatus};
use planner_core::state::{Entity, NextPreferences, PlanState};
use planner_core::undo::UndoLog;

use crate::advisor::{AdvisorFeed, AdvisorState};
use crate::buildqueue::{derive_build_queue, BuildStep};
use crate::cutover::{derive_cutovers, Cutover, CutoverPlan, Dip};

use gamedata::docs::GameData;
use gamedata::worldnodes::WorldSnapshot;
use persist::plan_file::{PersistError, PlanFile};
use serde::Serialize;
use solver::model::{
    EdgeSpec, FactorySnapshot, GroupSpec, InputPortSpec, NodeRef, OutputPortSpec, RecipeSpec,
    SolveResult, T0Edit, TargetCeiling,
};

/// T1 budget (Addendum A4): three consecutive misses flip a factory to
/// solve-on-release.
const T1_BUDGET_MS: f64 = 50.0;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error(transparent)]
    Persist(#[from] PersistError),
    #[error("{0}")]
    Internal(String),
}

impl Serialize for SessionError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedGroup {
    pub in_rates: BTreeMap<String, f64>,
    pub out_rates: BTreeMap<String, f64>,
    pub power_mw: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedEdge {
    pub flow: f64,
    pub saturation: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedFactory {
    pub groups: BTreeMap<String, DerivedGroup>,
    pub edges: BTreeMap<String, DerivedEdge>,
    pub ports: BTreeMap<String, f64>,
    /// Unmet output targets (SDD §5.2 degraded solve) — `ports` holds the
    /// achieved rates; the canonical targets are never rewritten for these.
    pub shortfalls: BTreeMap<String, solver::model::Shortfall>,
    pub total_power_mw: f64,
    pub target_ceiling: Option<TargetCeiling>,
    pub solve_us: u64,
    pub solve_on_release: bool,
    /// Set when the factory couldn't be solved (cycle, missing recipe).
    pub solve_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedNode {
    pub claims: u32,
    pub conflict: bool,
    /// A plan-local node override disagrees with the ambient catalog position
    /// (W2b-C) — the node renders at its corrected coord with a drift marker.
    /// `conflict` stays double-claim only; this is a separate, orthogonal flag.
    pub drift: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedRoute {
    /// items/min actually moving (downstream intake).
    pub flow: f64,
    /// what the upstream factory can push (post-cap) — flow < supplied means slack.
    pub supplied: f64,
    pub capacity: f64,
    pub saturation: f64,
    pub length_m: f64,
    /// Total meters climbed / descended along the path (0 on flat plans).
    pub climb_up_m: f64,
    pub climb_down_m: f64,
    pub item: Option<String>,
    /// Rail/truck/drone math block (A3) — None for belts and power.
    pub transport: Option<planner_core::transport::TransportMath>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeficitRow {
    pub factory: Id,
    pub port: Id,
    pub route: Option<Id>,
    pub item: String,
    /// items/min the factory's own target would need through this port.
    pub needed: f64,
    pub supplied: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedSwitch {
    pub id: Id,
    pub priority: u8,
    /// Demand on the load side of the switch (what shedding drops).
    pub downstream_mw: f64,
    /// Total circuit demand at which this switch sheds (A2.3 SHEDS AT).
    pub sheds_at_mw: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivedCircuit {
    pub name: String,
    pub members: Vec<Id>,
    pub generation_mw: f64,
    pub demand_mw: f64,
    /// Priority switches on this grid, shed order first (P8 → P1).
    pub switches: Vec<DerivedSwitch>,
    /// Brownout sim: `P4 @ +0.4 GW growth` — the next group to shed.
    pub next_shed: Option<String>,
}

/// Per-grid power delta a proposal would cause (mock 3a review banner).
/// Transient — derived for the review, never persisted, so no `serde(default)`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircuitImpact {
    pub name: String,
    pub demand_before_mw: f64,
    pub demand_after_mw: f64,
    pub generation_before_mw: f64,
    pub generation_after_mw: f64,
    /// Headroom AFTER the change, via [`circuit_level`].
    pub headroom_after: f64,
    /// `"ok" | "warn" | "crit"` — banner color follows the derived condition.
    pub level: String,
}

/// Circuit headroom + level from generation/demand — the ONE place the
/// `(gen - demand) / gen` formula and the 0.20/0.05 thresholds live (SDD §12).
/// Demand with no generation reads fully overdrawn (-1); an idle grid reads
/// full margin (1). Routed through the advisor's power_swing rule, the review
/// consequence, and the per-circuit impact so all three stay byte-identical.
pub(crate) fn circuit_level(generation_mw: f64, demand_mw: f64) -> (f64, &'static str) {
    let headroom = if generation_mw > 0.0 {
        (generation_mw - demand_mw) / generation_mw
    } else if demand_mw > 0.0 {
        -1.0
    } else {
        1.0
    };
    let level = if headroom < 0.05 {
        "crit"
    } else if headroom < 0.20 {
        "warn"
    } else {
        "ok"
    };
    (headroom, level)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCheck {
    pub item: String,
    pub requested: f64,
    pub achieved: f64,
}

/// PR 3: what `POST /api/next/preferences` returns — the persisted preferences
/// plus the freshly-derived heuristic opportunity list (the renderer folds it in
/// immediately and bumps its rank epoch for a full re-rank).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferencesView {
    pub preferences: NextPreferences,
    pub opportunities: Vec<crate::opportunities::Opportunity>,
}

/// Live partial-accept consequence (mock 3a footer + amber strip).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalConsequence {
    pub goal: Vec<GoalCheck>,
    pub goal_met: bool,
    pub delta_power_mw: f64,
    pub delta_generation_mw: f64,
    pub machines: u32,
    pub warnings: Vec<String>,
    /// Per-grid before→after power for every TOUCHED circuit (mock 3a banner).
    /// Replaces the old margin-critical `warnings` strings — power lives here.
    pub circuit_impacts: Vec<CircuitImpact>,
}

/// Result of adopting an alternate empire-wide (W2b-D CTA). The optimizer is
/// advisory: this drafts the proposal(s) that carry the change into the existing
/// review surface — a T2 `SetGroupRecipe` proposal for an all-◇ opportunity, or
/// a W2a Refactor per ◆ built factory (the ◆ layer is never mutated). Any
/// per-factory infeasibility is relayed in `note`, never silently dropped.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptOutcome {
    /// Drafted-and-stored proposal ids (open these in review).
    pub proposals: Vec<Id>,
    /// `"t2"` (all ◇ planned) or `"refactor"` (any ◆ built).
    pub route: String,
    /// Relayed infeasibility reason(s) for a built factory that could not be
    /// replaced (e.g. node budget) — surfaced in the row, not swallowed.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum ImportOutcome {
    /// First import — the Built layer was written (one undo entry).
    Imported {
        response: EditResponse,
        factories: u32,
        machines: u32,
        quarantined: u32,
    },
    /// Re-import with drift — review the SaveReimport proposal.
    Drift {
        response: EditResponse,
        proposal: Id,
    },
    /// Re-import found the built layer already in sync.
    InSync,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Derived {
    pub factories: BTreeMap<String, DerivedFactory>,
    pub nodes: BTreeMap<String, DerivedNode>,
    pub routes: BTreeMap<String, DerivedRoute>,
    pub deficits: Vec<DeficitRow>,
    /// Power grids: connected components over Power routes (A2.1).
    pub circuits: Vec<DerivedCircuit>,
    pub total_generation_mw: f64,
    /// True when factories feed each other in a loop — solved independently.
    pub empire_cycle: bool,
    /// Whole-empire recompute wall time (SDD §5.4 budget: 200ms).
    pub recompute_us: u64,
    pub total_power_mw: f64,
    /// Derived build queue (W1c): ordered ◇ planned / partially-built steps
    /// with resolved completion. Recomputed every solve like circuits/deficits.
    pub build_queue: Vec<BuildStep>,
    /// Derived cutovers (W2a): the lightweight presence/steps for each ◇→◆
    /// refactor link. The N+1 scratch-solves that price the downtime are NOT run
    /// here — they are on-demand via [`Session::cutover_plan`].
    pub cutovers: Vec<Cutover>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditResponse {
    pub patches: PatchBatch,
    pub derived: Derived,
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub created: Vec<Id>,
    /// Plan-content hash (proposals excluded) — STALE badge comparator.
    pub plan_hash: String,
    /// Current advisor feed (badge + cards re-render on every response).
    pub advisor: AdvisorFeed,
}

pub struct Session {
    pub state: PlanState,
    pub undo: UndoLog,
    pub file: PlanFile,
    pub gamedata: GameData,
    pub world: WorldSnapshot,
    /// Consecutive over-budget T1 solves per factory (A4 miss behavior).
    slow_solves: BTreeMap<Id, u32>,
    /// Ambient advisor state (cards/mutes persist outside the undo journal).
    pub advisor: AdvisorState,
    /// Recipe classes the imported save has unlocked (W2b). Resolved from
    /// `mPurchasedSchematics × FGSchematic` unlocks at import; persisted in the
    /// meta KV store, OUTSIDE the undo journal / plan_hash (a save-derived fact,
    /// not canonical plan state). Empty when no save with schematics is imported.
    pub unlocked: BTreeSet<String>,
    /// Bring-your-own-model endpoint config (PR 10). Env defaults
    /// (`FICSIT_AI_BASE_URL` / `FICSIT_AI_MODEL` / `FICSIT_AI_KEY`), grown
    /// from the old `ai_key` field. IN MEMORY ONLY — the key is never
    /// serialized, logged, or persisted (v1; keychain is the shell's later).
    pub ai: crate::ai::AiConfig,
}

impl Session {
    /// Open a session against a plan file. Gamedata comes from `docs_json`
    /// bytes if given (real install), else the bundled fixture.
    pub fn open(
        plan_path: impl AsRef<Path>,
        docs_json: Option<Vec<u8>>,
        game_build: &str,
    ) -> Result<Self, SessionError> {
        let file = PlanFile::open(plan_path)?;
        Self::with_file(file, docs_json, game_build)
    }

    pub fn in_memory(docs_json: Option<Vec<u8>>) -> Result<Self, SessionError> {
        let file = PlanFile::in_memory().map_err(SessionError::Persist)?;
        Self::with_file(file, docs_json, "fixture")
    }

    fn with_file(
        file: PlanFile,
        docs_json: Option<Vec<u8>>,
        game_build: &str,
    ) -> Result<Self, SessionError> {
        let text = match &docs_json {
            Some(bytes) => gamedata::docs::decode(bytes),
            None => include_str!("../../gamedata/assets/docs-fixture.json").to_string(),
        };
        let gd = gamedata::docs::parse_docs(&text, game_build)
            .map_err(|e| SessionError::Internal(format!("Docs.json parse failed: {e}")))?;
        let world = gamedata::worldnodes::load();
        let (state, entries, cursor) = file.load()?;
        let undo = UndoLog::hydrate_with_cursor(entries, cursor);
        let mut advisor = AdvisorState::default();
        for json in file.load_advisor_cards().unwrap_or_default() {
            if let Ok(card) = serde_json::from_str(&json) {
                advisor.cards.push(card);
            }
        }
        advisor.muted = file.load_mutes().unwrap_or_default().into_iter().collect();
        if let Some(json) = file.advisor_gate() {
            // Arming state survives restarts: still-true conditions were
            // already reported and must not fire duplicate cards on launch.
            advisor.restore_gate_snapshot(&json);
        }
        // Unlocked recipe set survives restarts (save-derived fact). Tolerant
        // default: a plan file with no "unlocked" blob hydrates as empty.
        let unlocked = file
            .unlocked()
            .and_then(|s| serde_json::from_str::<BTreeSet<String>>(&s).ok())
            .unwrap_or_default();
        Ok(Self {
            state,
            undo,
            file,
            gamedata: gd,
            world,
            slow_solves: BTreeMap::new(),
            advisor,
            unlocked,
            ai: crate::ai::AiConfig::from_env(),
        })
    }

    /// Full projection for renderer hydration.
    pub fn hydrate(&mut self) -> serde_json::Value {
        let derived = self.solve_all_readonly();
        serde_json::json!({
            "plan": self.state.project(),
            "derived": derived,
            "gamedata": {
                "items": self.gamedata.items,
                "recipes": self.gamedata.recipes,
                "machines": self.gamedata.machines,
                "belts": self.gamedata.belts,
                "buildables": self.gamedata.buildables,
                "buildVersion": self.gamedata.build_version,
            },
            "world": self.world,
            "planHash": self.plan_hash(),
            "advisor": self.advisor_feed(),
            "canUndo": self.undo.can_undo(),
            "canRedo": self.undo.can_redo(),
            "undoLabel": self.undo.undo_label(),
            "viewState": self.file.view_state().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            "lastImport": self.file.last_import().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
            "unlocked": self.unlocked,
        })
    }

    /// `plan.edit` — apply one or more commands as a single undoable step.
    pub fn edit(&mut self, cmds: Vec<Command>) -> Result<EditResponse, SessionError> {
        if cmds.is_empty() {
            return Err(SessionError::Internal("empty command list".into()));
        }
        // B3: planner-core can't validate a `SetBuildDone` id (it never sees the
        // derived queue, which mints synthetic `switch:<fid>:<item>` ids), but
        // the app layer CAN. Reject an id no build-queue or cutover step carries
        // so a bogus overlay is refused instead of silently upserting an inert
        // override. Built once, and only when a SetBuildDone is actually present.
        if cmds
            .iter()
            .any(|c| matches!(c, Command::SetBuildDone { .. }))
        {
            let valid: BTreeSet<Id> = derive_build_queue(&self.state, &self.gamedata)
                .into_iter()
                .map(|s| s.id)
                .chain(
                    derive_cutovers(&self.state, &self.gamedata)
                        .into_iter()
                        .flat_map(|c| c.steps.into_iter().map(|s| s.id)),
                )
                .collect();
            for cmd in &cmds {
                if let Command::SetBuildDone { id, .. } = cmd {
                    if !valid.contains(id) {
                        return Err(SessionError::Domain(DomainError::Invalid {
                            message: format!("no build step {id} to mark done"),
                        }));
                    }
                }
            }
        }
        let mut tx = Transaction::new(cmds[0].label());
        for cmd in &cmds {
            match commands::apply(&mut self.state, cmd) {
                Ok(t) => {
                    tx.forward.extend(t.forward);
                    tx.inverse.extend(t.inverse);
                    tx.created.extend(t.created);
                }
                Err(e) => {
                    // Roll back what already applied — a failed edit leaves no trace.
                    let mut rollback = tx.inverse.clone();
                    rollback.reverse();
                    self.state
                        .apply_batch(&rollback)
                        .map_err(|m| SessionError::Internal(format!("rollback failed: {m}")))?;
                    return Err(e.into());
                }
            }
        }

        // Empire re-solve (SDD §5.4): edits ripple downstream through routes.
        // Solver-owned write-backs (counts/clocks, clamped targets, route
        // manifests) fold into the same undo entry as the causing edit.
        let trigger = Self::solve_trigger(&cmds);
        let derived = self.empire_solve(&trigger, Some(&mut tx));
        self.commit_mutation(tx, derived)
    }

    /// The ONLY path from an applied `Transaction` to a durable undo entry.
    ///
    /// Invariant: **disk commits first; the in-memory undo log advances only
    /// after the plan file has durably committed the same entry.** A persist
    /// failure therefore can never diverge memory from disk: the SQLite
    /// transaction rolled back (disk holds the pre-edit state), so we roll
    /// the applied transaction back out of canonical state and surface the
    /// error — renderer, memory, and disk all agree on the pre-edit state,
    /// and the redo tail (memory and disk) is untouched. Advisor cards are
    /// gated and persisted only after the commit succeeds, so a rolled-back
    /// edit never leaves phantom cards.
    fn commit_mutation(
        &mut self,
        tx: Transaction,
        derived: Derived,
    ) -> Result<EditResponse, SessionError> {
        let created = tx.created.clone();
        let entry = UndoLog::stage(tx);
        // `+ 1`: the log hasn't advanced yet, so the applied count after this
        // commit is the current depth plus this entry. PlanFile::commit keeps
        // `applied - 1` prior journal rows (its redo-tail DELETE) — exactly
        // the entries applied before this one, same as the pre-staging code.
        if let Err(e) = self
            .file
            .commit(&entry, &self.state.meta, self.undo.entries().len() + 1)
        {
            // entry.inverse is already in application order (stage reversed it).
            if let Err(m) = self.state.apply_batch(&entry.inverse) {
                // Compensation failed — self-heal from disk, which is intact.
                self.rehydrate_from_disk(&m)?;
            }
            return Err(e.into());
        }
        self.undo.push(entry.clone());
        self.advise(&derived);
        Ok(EditResponse {
            patches: entry.forward,
            derived,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            undo_label: self.undo.undo_label().map(String::from),
            created,
            plan_hash: self.plan_hash(),
            advisor: self.advisor_feed(),
        })
    }

    /// Last-resort recovery when in-memory rollback itself fails: reload
    /// canonical state + undo journal from the plan file, which is always a
    /// valid restore point (every durable write is one atomic transaction).
    fn rehydrate_from_disk(&mut self, cause: &str) -> Result<(), SessionError> {
        let (state, entries, cursor) = self.file.load().map_err(|e| {
            SessionError::Internal(format!(
                "rollback after persist failure failed ({cause}) and reload failed: {e}"
            ))
        })?;
        self.state = state;
        self.undo = UndoLog::hydrate_with_cursor(entries, cursor);
        Ok(())
    }

    pub fn undo(&mut self) -> Result<Option<EditResponse>, SessionError> {
        let batch = match self.undo.undo(&mut self.state) {
            Ok(None) => return Ok(None),
            Ok(Some(batch)) => batch,
            Err(m) => {
                // Corrupt journal entry: the log is untouched but state may
                // hold a partial application. Disk is intact (no checkpoint
                // ran), so restore from it — every subsequent ⌘Z re-fails
                // cleanly instead of panicking.
                self.rehydrate_from_disk(&m)?;
                return Err(SessionError::Internal(format!("undo failed: {m}")));
            }
        };
        if let Err(e) = self
            .file
            .checkpoint(&batch, &self.state.meta, self.applied_count())
        {
            // Disk untouched (the checkpoint transaction rolled back) —
            // compensate with the opposite move: re-applying the just-undone
            // entry restores state and cursor in one call. It re-applies a
            // batch that applied cleanly moments ago; if it somehow fails,
            // restore from disk, which still holds the pre-undo state.
            if let Err(m) = self.undo.redo(&mut self.state) {
                self.rehydrate_from_disk(&m)?;
            }
            return Err(e.into());
        }
        Ok(Some(self.nav_response(batch)))
    }

    pub fn redo(&mut self) -> Result<Option<EditResponse>, SessionError> {
        let batch = match self.undo.redo(&mut self.state) {
            Ok(None) => return Ok(None),
            Ok(Some(batch)) => batch,
            Err(m) => {
                // Mirror of undo(): self-heal from disk, surface the error.
                self.rehydrate_from_disk(&m)?;
                return Err(SessionError::Internal(format!("redo failed: {m}")));
            }
        };
        if let Err(e) = self
            .file
            .checkpoint(&batch, &self.state.meta, self.applied_count())
        {
            // Mirror of undo(): un-apply the just-redone entry.
            if let Err(m) = self.undo.undo(&mut self.state) {
                self.rehydrate_from_disk(&m)?;
            }
            return Err(e.into());
        }
        Ok(Some(self.nav_response(batch)))
    }

    /// Plan-content hash for proposal staleness — proposals themselves are
    /// excluded, or drafting one would immediately mark it stale.
    pub fn plan_hash(&self) -> String {
        let mut projection = self.state.project();
        if let Some(map) = projection.as_object_mut() {
            map.remove("proposals");
            // PR 3: NEXT preferences are an advisory filter, not plan geometry —
            // excluded so a preference toggle never staleness-flags open
            // proposals or trips the per-edit merge (and old plans stay
            // hash-stable — they serialized no `preferences` key at all).
            if let Some(meta) = map.get_mut("meta").and_then(|m| m.as_object_mut()) {
                meta.remove("preferences");
            }
        }
        fnv1a(projection.to_string().as_bytes())
    }

    /// Accept a proposal: materialize every included item's commands (aliases
    /// resolved in dependency order) + flip the status, as ONE undo entry.
    /// ◇ planned entities only — the built layer is never touched (mock 3c).
    pub fn accept_proposal(&mut self, id: &str) -> Result<EditResponse, SessionError> {
        let p = self
            .state
            .proposals
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::Internal(format!("proposal {id} not found")))?;
        if p.status == ProposalStatus::Accepted || p.status == ProposalStatus::Rejected {
            return Err(SessionError::Internal("proposal is closed".into()));
        }
        // A drift diff is only valid against the save state that produced it:
        // the last_import blob names the proposal the NEWEST import drafted
        // (null for in-sync/first-import), and any other still-open diff —
        // e.g. one an in-sync re-import made moot without writing — must not
        // rewrite the ◆ layer with stale counts. Blobs predating the key
        // (legacy plan files) carry no verdict and pass.
        if p.source == planner_core::proposals::ProposalSource::SaveReimport {
            let current = self
                .file
                .last_import()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                .and_then(|blob| blob.get("proposal").cloned());
            if let Some(current) = current {
                if current.as_str() != Some(id) {
                    return Err(SessionError::Internal(
                        "this drift diff was superseded by a newer save import — re-import for a fresh diff".into(),
                    ));
                }
            }
        }
        // Fail loud instead of silently accepting a subset: a dependency cycle
        // among CHECKED items would otherwise drop them all while the response
        // still reads ACCEPTED (reachable via raw CreateProposal payloads).
        let cycles = cycle_dropped(&p);
        if !cycles.is_empty() {
            return Err(SessionError::Internal(format!(
                "cannot accept: dependency cycle among included items ({})",
                cycles.join(", ")
            )));
        }
        let label = format!("accept proposal #{}", p.number);
        let mut tx = Transaction::new(label);
        let mut symbols: BTreeMap<String, Id> = BTreeMap::new();
        let mut apply_all = |state: &mut PlanState,
                             tx: &mut Transaction|
         -> Result<(), SessionError> {
            for item in ordered_included(&p) {
                // SaveReimport drift items sync the ◆ Built layer directly
                // — the one documented exception to accept-creates-◇-only
                if let Some(sync) = &item.sync {
                    let op: crate::import::SyncOp = serde_json::from_value(sync.clone())
                        .map_err(|e| SessionError::Internal(e.to_string()))?;
                    crate::import::apply_sync(state, tx, &op, &p.id, &self.gamedata, &self.world);
                    continue;
                }
                for (idx, cmd) in item.commands.iter().enumerate() {
                    let resolved =
                        resolve_aliases(cmd, &symbols).map_err(SessionError::Internal)?;
                    let t = commands::apply(state, &resolved)?;
                    if let (Some(Some(alias)), Some(created)) =
                        (item.aliases.get(idx), t.created.first())
                    {
                        symbols.insert(alias.clone(), created.clone());
                    }
                    tx.forward.extend(t.forward);
                    tx.inverse.extend(t.inverse);
                    tx.created.extend(t.created);
                }
            }
            let t = commands::apply(
                state,
                &Command::SetProposalStatus {
                    id: id.to_string(),
                    status: ProposalStatus::Accepted,
                },
            )?;
            tx.forward.extend(t.forward);
            tx.inverse.extend(t.inverse);
            Ok(())
        };
        if let Err(e) = apply_all(&mut self.state, &mut tx) {
            let mut rollback = tx.inverse.clone();
            rollback.reverse();
            self.state
                .apply_batch(&rollback)
                .map_err(|m| SessionError::Internal(format!("rollback failed: {m}")))?;
            return Err(e);
        }
        // Stamp proposal provenance on the step-bearing entities this accept
        // created (the raw commands default to CreatedBy::Manual): the build
        // queue buckets steps by their creating proposal's number and lights
        // milestone progress from it. Folded into the same undo entry.
        for cid in tx.created.clone() {
            self.stamp_proposal_provenance(&mut tx, &cid, id);
        }
        // A re-import drift accept writes the ◆ Built layer directly; any manual
        // build-override the game has now caught up to is redundant, so dissolve
        // it (mirrors the planned-delta dissolve in import.rs). Folded into the
        // same undo entry as the accept.
        if p.items.iter().any(|i| i.sync.is_some()) {
            crate::buildqueue::dissolve_stale_overrides(&mut self.state, &mut tx, &self.gamedata);
            // Node-position overrides that the save has caught back up to (or
            // whose claim is gone) auto-dissolve, same undo entry (W2b-C).
            crate::import::dissolve_stale_node_overrides(&mut self.state, &mut tx, &self.world);
            // Any `replaces` pointing at a now-removed ◆ factory is dangling
            // intent — null it so the cutover reads dismantle-complete (mirrors
            // the override dissolve). Folded into the same undo entry.
            crate::cutover::dissolve_stale_replaces(&mut self.state, &mut tx);
        }
        let derived = self.empire_solve(&T0Edit::Recompute, Some(&mut tx));
        self.commit_mutation(tx, derived)
    }

    /// Live consequence of the CURRENT checkbox state (mock 3a partial
    /// accept): apply included items to a scratch copy, solve, diff, discard.
    pub fn eval_proposal(&mut self, id: &str) -> Result<ProposalConsequence, SessionError> {
        let p = self
            .state
            .proposals
            .get(id)
            .cloned()
            .ok_or_else(|| SessionError::Internal(format!("proposal {id} not found")))?;
        let before = self.solve_all_readonly();
        let saved = self.state.clone();
        let mut symbols: BTreeMap<String, Id> = BTreeMap::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut machines: u32 = 0;
        'items: for item in ordered_included(&p) {
            if let Some(sync) = &item.sync {
                if let Ok(op) = serde_json::from_value::<crate::import::SyncOp>(sync.clone()) {
                    let mut scratch = Transaction::new("eval");
                    crate::import::apply_sync(
                        &mut self.state,
                        &mut scratch,
                        &op,
                        &p.id,
                        &self.gamedata,
                        &self.world,
                    );
                }
                continue 'items;
            }
            for (idx, cmd) in item.commands.iter().enumerate() {
                let resolved = match resolve_aliases(cmd, &symbols) {
                    Ok(c) => c,
                    Err(e) => {
                        warnings.push(format!("{} skipped: {e}", item.label));
                        continue 'items;
                    }
                };
                if let Command::AddGroup { count, .. } = &resolved {
                    machines += count;
                }
                match commands::apply(&mut self.state, &resolved) {
                    Ok(t) => {
                        if let (Some(Some(alias)), Some(created)) =
                            (item.aliases.get(idx), t.created.first())
                        {
                            symbols.insert(alias.clone(), created.clone());
                        }
                    }
                    Err(e) => {
                        warnings.push(format!("{}: {e}", item.label));
                        continue 'items;
                    }
                }
            }
        }
        let after = self.solve_all_readonly();
        // goal check: production delta of each goal item across all out ports
        let goal: Vec<GoalCheck> = p
            .goal
            .iter()
            .map(|(item, requested)| GoalCheck {
                item: item.clone(),
                requested: *requested,
                achieved: Self::out_rate(&self.state, &after, item)
                    - Self::out_rate(&saved, &before, item),
            })
            .collect();
        // new deficits feed the amber warning strip; per-circuit power now
        // lives in the structured `circuit_impacts` below, not `warnings`.
        let before_keys: std::collections::BTreeSet<String> = before
            .deficits
            .iter()
            .map(|d| format!("{}:{}", d.factory, d.item))
            .collect();
        for d in &after.deficits {
            if !before_keys.contains(&format!("{}:{}", d.factory, d.item)) {
                let name = self
                    .state
                    .factories
                    .get(&d.factory)
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| d.factory.clone());
                warnings.push(format!(
                    "{} starved of {} — {:.1}/min short",
                    name,
                    d.item,
                    d.needed - d.supplied
                ));
            }
        }
        // Per-circuit before→after power (mock 3a banner). Grid `name` is
        // index-based and renumbers when sites are added, so membership overlap —
        // not name — is the identity link. Attribute each BEFORE grid to its
        // PRIMARY destination (the after grid it shares the most members with),
        // then aggregate per after grid: a MERGE sums its sources' demand/gen and
        // a SPLIT attributes the whole before grid to ONE child, so the sibling
        // reads as newly-formed (no double-counting). This replaces a per-after
        // single-match that mis-summed both directions. No before grid maps to an
        // after ⇒ that grid is newly-formed (before = 0, delta = full after values).
        type BeforeAgg<'a> = (f64, f64, std::collections::BTreeSet<&'a Id>);
        let mut before_for_after: std::collections::BTreeMap<usize, BeforeAgg> =
            std::collections::BTreeMap::new();
        for bc in &before.circuits {
            let bc_set: std::collections::BTreeSet<&Id> = bc.members.iter().collect();
            let primary = after
                .circuits
                .iter()
                .enumerate()
                .map(|(i, ac)| {
                    let overlap = ac.members.iter().filter(|m| bc_set.contains(m)).count();
                    (overlap, i)
                })
                .filter(|(overlap, _)| *overlap > 0)
                .max_by_key(|(overlap, _)| *overlap)
                .map(|(_, i)| i);
            if let Some(i) = primary {
                let entry = before_for_after
                    .entry(i)
                    .or_insert_with(|| (0.0, 0.0, std::collections::BTreeSet::new()));
                entry.0 += bc.demand_mw;
                entry.1 += bc.generation_mw;
                entry.2.extend(bc.members.iter());
            }
        }
        let mut circuit_impacts: Vec<CircuitImpact> = Vec::new();
        for (i, ac) in after.circuits.iter().enumerate() {
            let after_set: std::collections::BTreeSet<&Id> = ac.members.iter().collect();
            let (demand_before, gen_before, before_set) = before_for_after
                .get(&i)
                .map(|(d, g, s)| (*d, *g, s.clone()))
                .unwrap_or((0.0, 0.0, std::collections::BTreeSet::new()));
            let touched = before_set != after_set
                || (ac.demand_mw - demand_before).abs() > 1e-6
                || (ac.generation_mw - gen_before).abs() > 1e-6;
            if !touched {
                continue;
            }
            let (headroom_after, level) = circuit_level(ac.generation_mw, ac.demand_mw);
            circuit_impacts.push(CircuitImpact {
                name: ac.name.clone(),
                demand_before_mw: demand_before,
                demand_after_mw: ac.demand_mw,
                generation_before_mw: gen_before,
                generation_after_mw: ac.generation_mw,
                headroom_after,
                level: level.to_string(),
            });
        }
        let consequence = ProposalConsequence {
            goal_met: goal.iter().all(|g| g.achieved >= g.requested - 1e-6),
            goal,
            delta_power_mw: after.total_power_mw - before.total_power_mw,
            delta_generation_mw: after.total_generation_mw - before.total_generation_mw,
            machines,
            warnings,
            circuit_impacts,
        };
        self.state = saved;
        Ok(consequence)
    }

    /// Achieved production of `item` summed across every Out port empire-wide
    /// (the port's derived rate). The one place the goal-check delta and the
    /// cutover downtime engine measure "how much of this is being produced", so
    /// the two can never disagree.
    fn out_rate(state: &PlanState, derived: &Derived, item: &str) -> f64 {
        state
            .ports
            .values()
            .filter(|port| port.direction == PortDirection::Out && port.item == item)
            .filter_map(|port| {
                derived
                    .factories
                    .get(&port.factory)
                    .and_then(|df| df.ports.get(&port.id))
            })
            .sum()
    }

    /// Plan a whole-factory replacement (W2a): target the item(s) the running ◆
    /// factory produces, run the existing global solver to site a ◇ replacement
    /// beside the old pin, and bind the two with a trailing `SetFactoryReplaces`
    /// alias command. Returns a Draft proposal — accept goes through the
    /// UNTOUCHED accept path (◇-only, one undo, the old ◆ never touched).
    pub fn plan_replacement(
        &mut self,
        old_factory_id: Id,
        pin: Option<String>,
    ) -> Result<Proposal, SessionError> {
        let old = self
            .state
            .factories
            .get(&old_factory_id)
            .cloned()
            .ok_or_else(|| SessionError::Internal(format!("factory {old_factory_id} not found")))?;
        if old.status != Status::Built {
            return Err(SessionError::Internal(
                "only a running ◆ built factory can be replaced".into(),
            ));
        }
        // Goal = the achieved production of every item the old factory ships
        // (its Out ports). Power (generator output) is sourced separately, never
        // belted — skip it as a replacement goal.
        let derived = self.solve_all_readonly();
        let mut by_item: BTreeMap<String, f64> = BTreeMap::new();
        for pid in &old.ports {
            let Some(port) = self.state.ports.get(pid) else {
                continue;
            };
            if port.direction != PortDirection::Out || port.item == gamedata::docs::POWER_ITEM {
                continue;
            }
            let rate = derived
                .factories
                .get(&old_factory_id)
                .and_then(|df| df.ports.get(pid))
                .copied()
                .filter(|r| *r > 0.0)
                .unwrap_or(port.rate);
            *by_item.entry(port.item.clone()).or_insert(0.0) += rate;
        }
        if by_item.is_empty() {
            return Err(SessionError::Internal(
                "the factory ships nothing to replace (no output ports)".into(),
            ));
        }
        // O1: when the caller pins a recipe (a built-factory "adopt this alt"),
        // seed the retired product's alternate so the ◇ replacement is solved
        // ONTO that recipe (the ◆ is never touched). A pin whose product cannot
        // be resolved degrades to no pin — behaviour-identical to the None path.
        let mut pinned_recipes: BTreeMap<String, String> = BTreeMap::new();
        if let Some(recipe) = pin {
            if let Some(product) = self
                .gamedata
                .recipes
                .get(&recipe)
                .and_then(|r| r.products.first())
                .map(|(i, _)| i.clone())
            {
                pinned_recipes.insert(product, recipe);
            }
        }
        let goal = crate::wizard::WizardGoal {
            items: by_item.into_iter().collect(),
            constraints: crate::wizard::WizardConstraints::default(),
            milestone: None,
            pinned_recipes,
        };
        let outcome = crate::wizard::global_solve(
            &self.state,
            &self.gamedata,
            &self.world,
            &goal,
            &self.unlocked,
            self.plan_hash(),
            crate::jobs::now_rfc3339(),
            |_, _| {},
            &std::sync::atomic::AtomicBool::new(false),
        );
        let mut proposal = match outcome {
            crate::wizard::WizardOutcome::Proposal { proposal } => proposal,
            crate::wizard::WizardOutcome::Infeasible(inf) => {
                return Err(SessionError::Internal(format!(
                    "replacement infeasible — {}",
                    inf.binding
                )))
            }
            crate::wizard::WizardOutcome::Cancelled => {
                return Err(SessionError::Internal("replacement solve cancelled".into()))
            }
        };
        // Refactor provenance + a title that names the retirement.
        proposal.source = planner_core::proposals::ProposalSource::Refactor;
        proposal.provenance = "REFACTOR".into();
        proposal.title = format!("REPLACE {}", old.name.to_uppercase());
        // Find the CREATE item minting the new factory (alias "site") and
        // (a) re-site it beside the old pin, (b) append the SetFactoryReplaces
        // link. The alias resolves at accept via the untouched $alias machinery.
        let target_pos = MapPos {
            x: old.position.x + 400.0,
            y: old.position.y,
            z: old.position.z,
        };
        let create = proposal
            .items
            .iter_mut()
            .find(|it| it.kind == planner_core::proposals::ProposalItemKind::Create)
            .ok_or_else(|| SessionError::Internal("solver produced no CREATE item".into()))?;
        let orig_pos = create.commands.iter().find_map(|c| match c {
            Command::CreateFactory { position, .. } => Some(*position),
            _ => None,
        });
        if let Some(orig) = orig_pos {
            for item in &mut proposal.items {
                for cmd in &mut item.commands {
                    shift_site_pos(cmd, &orig, &target_pos);
                }
            }
        }
        let create = proposal
            .items
            .iter_mut()
            .find(|it| it.kind == planner_core::proposals::ProposalItemKind::Create)
            .expect("CREATE item present");
        create.commands.push(Command::SetFactoryReplaces {
            id: "$site".into(),
            replaces: Some(old_factory_id.clone()),
        });
        create.aliases.push(None);
        create.detail = format!("{} · replaces {}", create.detail, old.name);
        Ok(proposal)
    }

    /// Price the downtime of a cutover ON DEMAND (never in the per-edit solve):
    /// scratch-solve the whole empire at each phase boundary and report the
    /// honest, ripple-inclusive production dip per tracked item. Baseline is
    /// boundary k=0; the Switch boundary (k=1) is the worst case (old down, new
    /// not yet up). Restores canonical state before returning.
    pub fn cutover_plan(&mut self, factory: Id) -> Result<CutoverPlan, SessionError> {
        let cutover = derive_cutovers(&self.state, &self.gamedata)
            .into_iter()
            .find(|c| c.new_factory == factory || c.old_factory == factory)
            .ok_or_else(|| {
                SessionError::Internal(format!("no cutover involving factory {factory}"))
            })?;
        let tracked: Vec<String> = self
            .state
            .factories
            .get(&cutover.old_factory)
            .map(|old| crate::cutover::supplied_items(&self.state, old))
            .unwrap_or_default();
        // Machines torn down = the old factory's group counts (drives the est).
        let old_machines: u32 = self
            .state
            .factories
            .get(&cutover.old_factory)
            .map(|old| {
                old.groups
                    .iter()
                    .filter_map(|gid| self.state.groups.get(gid))
                    .map(|g| g.count)
                    .sum()
            })
            .unwrap_or(0);

        // Scratch-solve at each boundary against a SAVED base, then restore.
        let saved = self.state.clone();
        let mut production: Vec<BTreeMap<String, f64>> = Vec::new();
        for k in 0..=2usize {
            self.state = crate::cutover::shape_for_boundary(&saved, &cutover, k);
            let derived = self.solve_all_readonly();
            let mut row = BTreeMap::new();
            for item in &tracked {
                row.insert(item.clone(), Self::out_rate(&self.state, &derived, item));
            }
            production.push(row);
        }
        self.state = saved;

        let baseline = production[0].clone();
        const EPS: f64 = 1e-6;

        // Discriminate "no downtime" from "can't compute downtime". The old
        // factory declares positive output when any of its Out ports carries a
        // positive rate. If it declares output but the scratch-solve baseline is
        // ~0 for every tracked item, the factory does not actually produce in the
        // current solve (imported/unsolved/starved — the bundled fixture catalog
        // can't resolve its recipes) — downtime is UNAVAILABLE, not zero. A
        // silent-empty dips list here would read as "no impact"; that is the
        // dishonesty this feature exists to prevent.
        let declared_positive = self
            .state
            .factories
            .get(&cutover.old_factory)
            .map(|old| {
                old.ports
                    .iter()
                    .filter_map(|pid| self.state.ports.get(pid))
                    .filter(|p| {
                        p.direction == PortDirection::Out && p.item != gamedata::docs::POWER_ITEM
                    })
                    .any(|p| p.rate > EPS)
            })
            .unwrap_or(false);
        let baseline_positive = baseline.values().any(|r| *r > EPS);
        // Discriminate WHY nothing is produced. If every one of the old factory's
        // group recipes resolves in the catalog, the factory is real but STARVED
        // (its inputs aren't supplied in the current solve) — the fix is to route
        // its feed. If any recipe is unknown, it's an imported factory the bundled
        // fixture catalog can't solve — point the player at FICSIT_DOCS_JSON.
        let recipes_known = self
            .state
            .factories
            .get(&cutover.old_factory)
            .map(|old| {
                old.groups
                    .iter()
                    .filter_map(|gid| self.state.groups.get(gid))
                    .all(|g| self.gamedata.recipes.contains_key(&g.recipe))
            })
            .unwrap_or(false);
        let (downtime_available, unavailable_reason) = if declared_positive && !baseline_positive {
            let reason = if recipes_known {
                format!(
                    "{} produces nothing in the current solve — its inputs are starved; route its feed, then retry",
                    cutover.old_name
                )
            } else {
                format!(
                    "{} does not produce in the current solve — imported factories may need a real recipe catalog (set FICSIT_DOCS_JSON to your game's Docs.json)",
                    cutover.old_name
                )
            };
            (false, Some(reason))
        } else {
            (true, None)
        };

        let mut dips: Vec<Dip> = Vec::new();
        for (k, row) in production.iter().enumerate() {
            if k == 0 {
                continue;
            }
            for item in &tracked {
                let base = baseline.get(item).copied().unwrap_or(0.0);
                let rate = row.get(item).copied().unwrap_or(0.0);
                if rate < base - EPS {
                    dips.push(Dip {
                        // k=1 (Switch) is a TIMED teardown window — the machine-
                        // count estimate is the honest wall-clock. k=2 (Dismantle)
                        // is steady-state: a dip that persists there is a PERMANENT
                        // shortfall (the new factory doesn't cover the old output),
                        // not a timed window, so a wall-clock estimate would be a
                        // lie — zero it. (The renderer labels the k=2 dip
                        // "PERMANENT SHORTFALL" in batch D.)
                        est_hours: if k < 2 {
                            crate::cutover::est_hours(old_machines)
                        } else {
                            0.0
                        },
                        phase: k as u8,
                        item: item.clone(),
                        rate,
                        baseline: base,
                    });
                }
            }
        }
        Ok(CutoverPlan {
            new_factory: cutover.new_factory,
            old_factory: cutover.old_factory,
            tracked,
            baseline,
            production,
            dips,
            hard: cutover.node_reuse,
            downtime_available,
            unavailable_reason,
        })
    }

    /// Route an empire-wide alternate adoption (W2b-D) into the existing review
    /// surface, preserving the contract pivot: an opportunity touching only ◇
    /// planned groups drafts a T2 `SetGroupRecipe` proposal (legal on planned);
    /// an opportunity touching ANY ◆ built factory drafts a W2a Refactor per
    /// built factory via `plan_replacement` (so downtime/cutover engage and the
    /// ◆ layer is never mutated). Read the affected split off canonical state so
    /// the decision matches the ranked row exactly. Each drafted proposal is
    /// stored (one edit each); the ids come back for the renderer to open.
    pub fn optimize_adopt(&mut self, recipe: &str) -> Result<AdoptOutcome, SessionError> {
        let target = self
            .gamedata
            .recipes
            .get(recipe)
            .cloned()
            .ok_or_else(|| SessionError::Internal(format!("unknown recipe {recipe}")))?;
        let product = target
            .products
            .first()
            .map(|(i, _)| i.clone())
            .ok_or_else(|| SessionError::Internal("recipe has no product".into()))?;
        // Distinct ◆ built factories whose primary-product-on-a-different-recipe
        // group would adopt the alt — the presence of any routes to Refactor.
        let mut built_factories: Vec<Id> = self
            .state
            .groups
            .values()
            .filter(|g| g.status == Status::Built && g.recipe != target.class_name)
            .filter(|g| {
                self.gamedata
                    .recipes
                    .get(&g.recipe)
                    .and_then(|r| r.products.first())
                    .map(|(i, _)| i == &product)
                    .unwrap_or(false)
            })
            .map(|g| g.factory.clone())
            .collect();
        built_factories.sort();
        built_factories.dedup();

        if built_factories.is_empty() {
            // All ◇ planned → a single T2-style adopt proposal (SetGroupRecipe).
            // When no planned group can LOCALLY source the alt (an all-◇ dead-end),
            // this is honest degradation, not an error: return an empty draft set
            // with a note the row can surface, rather than an Err.
            let Some(mut proposal) = crate::altopt::optimize_to_recipe(
                &self.state,
                &self.gamedata,
                &self.unlocked,
                recipe,
            ) else {
                return Ok(AdoptOutcome {
                    proposals: vec![],
                    route: "t2".into(),
                    note: Some("no factory can locally source this alternate".into()),
                });
            };
            proposal.input_hash = self.plan_hash();
            proposal.snapshot_time = crate::jobs::now_rfc3339();
            let resp = self.edit(vec![Command::CreateProposal { proposal }])?;
            return Ok(AdoptOutcome {
                proposals: resp.created,
                route: "t2".into(),
                note: None,
            });
        }

        // Any ◆ built → a W2a Refactor per built factory. plan_replacement only
        // PLANS a ◇ replacement bound by `replaces` (with the alt PINNED in its
        // solve goal); it never touches the ◆.
        let mut proposals: Vec<Id> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for fid in built_factories {
            let name = self
                .state
                .factories
                .get(&fid)
                .map(|f| f.name.clone())
                .unwrap_or_else(|| fid.clone());
            match self.plan_replacement(fid, Some(recipe.to_string())) {
                Ok(proposal) => {
                    let resp = self.edit(vec![Command::CreateProposal { proposal }])?;
                    proposals.extend(resp.created);
                }
                Err(e) => notes.push(format!("{name}: {e}")),
            }
        }
        // O4: a mixed opportunity also touches ◇ PLANNED groups (disjoint from the
        // ◆ groups the Refactors retire — no double-apply). Draft the T2
        // SetGroupRecipe for those too so the whole opportunity adopts in one
        // review. Route reflects what was actually drafted.
        let mut route = "refactor";
        if let Some(mut proposal) =
            crate::altopt::optimize_to_recipe(&self.state, &self.gamedata, &self.unlocked, recipe)
        {
            proposal.input_hash = self.plan_hash();
            proposal.snapshot_time = crate::jobs::now_rfc3339();
            let resp = self.edit(vec![Command::CreateProposal { proposal }])?;
            proposals.extend(resp.created);
            route = "mixed";
        }
        Ok(AdoptOutcome {
            proposals,
            route: route.into(),
            note: (!notes.is_empty()).then(|| notes.join("; ")),
        })
    }

    /// Save import (SDD §8). First import writes the ◆ Built layer directly;
    /// re-imports never write — they diff into a SaveReimport proposal.
    pub fn import_save(
        &mut self,
        snapshot: crate::import::ImportSnapshot,
    ) -> Result<ImportOutcome, SessionError> {
        // Resolve the save's unlocked recipe set: mPurchasedSchematics ×
        // FGSchematic unlocks. A save-derived META fact — persisted outside the
        // undo journal / plan_hash, surfaced through hydrate as `unlocked`. With
        // the trimmed fixture catalog gamedata.schematics is empty, so this
        // degrades to an empty set and alternates stay locked exactly as before.
        let resolved: BTreeSet<String> = snapshot
            .unlocked_schematics
            .iter()
            .filter_map(|s| self.gamedata.schematics.get(s))
            .flatten()
            .cloned()
            .collect();
        // Only overwrite when the parse actually resolved alts: a transient
        // absent/failed schematic set (empty `resolved`) must not re-lock alts
        // the previous import unlocked. Empty → leave `self.unlocked` intact.
        if !resolved.is_empty() {
            self.unlocked = resolved;
            let _ = self.file.set_unlocked(
                &serde_json::to_string(&self.unlocked).unwrap_or_else(|_| "[]".into()),
            );
        }
        let clusters = crate::import::cluster(&snapshot, &self.gamedata);
        let has_built = self
            .state
            .factories
            .values()
            .any(|f| f.status == Status::Built);
        if !has_built {
            let import_id = planner_core::entities::new_id();
            let mut tx = Transaction::new("import save");
            crate::import::write_built_layer(
                &mut self.state,
                &mut tx,
                &clusters,
                &import_id,
                &self.gamedata,
                &self.world,
            );
            let derived = self.empire_solve(&T0Edit::Recompute, Some(&mut tx));
            let response = self.commit_mutation(tx, derived)?;
            let groups_written: u32 = clusters.iter().map(|c| c.groups.len() as u32).sum();
            self.write_last_import(
                &snapshot.save_name,
                "imported",
                clusters.len() as u32,
                groups_written,
                None,
            );
            return Ok(ImportOutcome::Imported {
                response,
                factories: clusters.len() as u32,
                machines: snapshot.machines.len() as u32,
                quarantined: snapshot.quarantined.values().sum(),
            });
        }
        // re-import: diff only, never write
        let items =
            crate::import::diff_against_built(&self.state, &self.gamedata, &clusters, &self.world);
        if items.is_empty() {
            self.write_last_import(&snapshot.save_name, "in_sync", 0, 0, None);
            return Ok(ImportOutcome::InSync);
        }
        let drift_count = items.len() as u32;
        let proposal = Proposal {
            id: String::new(),
            source: planner_core::proposals::ProposalSource::SaveReimport,
            title: format!("RE-IMPORT {}", snapshot.save_name.to_uppercase()),
            goal: vec![],
            status: ProposalStatus::Draft,
            number: 0,
            snapshot_time: crate::jobs::now_rfc3339(),
            input_hash: self.plan_hash(),
            provenance: "SAVE RE-IMPORT".into(),
            items,
            milestone: None,
        };
        // Each new drift diff supersedes every still-open one (a newer diff is a
        // cumulative superset): reject stale SaveReimport proposals in the same
        // edit, so the review surface and PLAN DRIFT tab never offer obsolete
        // SyncOps whose accept would rewrite the ◆ layer with old counts. One
        // batch → one undoable step.
        let mut cmds: Vec<Command> = self
            .state
            .proposals
            .values()
            .filter(|q| {
                q.source == planner_core::proposals::ProposalSource::SaveReimport
                    && matches!(q.status, ProposalStatus::Draft | ProposalStatus::Reviewing)
            })
            .map(|q| Command::SetProposalStatus {
                id: q.id.clone(),
                status: ProposalStatus::Rejected,
            })
            .collect();
        cmds.push(Command::CreateProposal { proposal });
        let response = self.edit(cmds)?;
        let proposal_id = response.created[0].clone();
        self.write_last_import(
            &snapshot.save_name,
            "drift",
            0,
            drift_count,
            Some(&proposal_id),
        );
        Ok(ImportOutcome::Drift {
            response,
            proposal: proposal_id,
        })
    }

    /// Re-stamp a just-created ◇ Planned step-bearing entity (factory / group /
    /// route / node claim) with `CreatedBy::Proposal(pid)`, recording the change
    /// into `tx`. Only these four kinds surface as build-queue steps; the
    /// `Planned` guard leaves ◆ Built entities minted by drift sync on their
    /// `Import` provenance. No-op when already stamped.
    fn stamp_proposal_provenance(&mut self, tx: &mut Transaction, id: &Id, pid: &str) {
        let prov = CreatedBy::Proposal(pid.to_string());
        if let Some(f) = self.state.factories.get(id) {
            if f.status == Status::Planned && f.created_by != prov {
                let mut f = f.clone();
                f.created_by = prov;
                tx.record(self.state.upsert(Entity::Factory(f)));
            }
        } else if let Some(g) = self.state.groups.get(id) {
            if g.status == Status::Planned && g.created_by != prov {
                let mut g = g.clone();
                g.created_by = prov;
                tx.record(self.state.upsert(Entity::Group(g)));
            }
        } else if let Some(r) = self.state.routes.get(id) {
            if r.status == Status::Planned && r.created_by != prov {
                let mut r = r.clone();
                r.created_by = prov;
                tx.record(self.state.upsert(Entity::Route(r)));
            }
        } else if let Some(c) = self.state.node_claims.get(id) {
            if c.status == Status::Planned && c.created_by != prov {
                let mut c = c.clone();
                c.created_by = prov;
                tx.record(self.state.upsert(Entity::NodeClaim(c)));
            }
        }
    }

    /// Persist the "what changed since last import" summary blob (best-effort,
    /// like the advisor writes — a failed session-fact write must not fail the
    /// import). Surfaced through [`Session::hydrate`] as `lastImport`.
    /// `proposal` is the drift proposal the latest import drafted (None for
    /// first-import / in-sync): [`Session::accept_proposal`] keys its stale-
    /// drift gate on it, so a diff the newest import didn't produce — including
    /// one an in-sync re-import made moot — can never rewrite the ◆ layer.
    fn write_last_import(
        &self,
        save_name: &str,
        outcome: &str,
        factories_added: u32,
        groups_changed: u32,
        proposal: Option<&str>,
    ) {
        let blob = serde_json::json!({
            "at": crate::jobs::now_rfc3339(),
            "saveName": save_name,
            "outcome": outcome,
            "factoriesAdded": factories_added,
            "groupsChanged": groups_changed,
            "proposal": proposal,
        });
        let _ = self.file.set_last_import(&blob.to_string());
    }

    /// Run the advisor gate over fresh derived state and persist new cards.
    fn advise(&mut self, derived: &Derived) {
        let events = crate::advisor::evaluate(&self.state, derived);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let created = self.advisor.gate(events, now, &crate::jobs::now_rfc3339());
        for card in &created {
            let _ = self
                .file
                .save_advisor_card(&card.id, &serde_json::to_string(card).unwrap_or_default());
        }
        // Gate state changes even when nothing fires (keys arm and prune) —
        // snapshot it best-effort, like the card writes above.
        let _ = self
            .file
            .save_advisor_gate(&self.advisor.gate_snapshot_json());
    }

    /// The advisor feed with expiry derived at the boundary: a Review CTA is
    /// only actionable while its proposal is still open, so cards pointing at
    /// a closed or deleted proposal drop out here — zero writes, and an undo
    /// that reopens the proposal revives the card for free. Non-Review cards
    /// pass through untouched; mutes/pause/budget are the gate's as-is.
    pub fn advisor_feed(&self) -> AdvisorFeed {
        // ai_ready gates on configured() (base + model), not on the key —
        // keyless endpoints (Ollama / LM Studio) are first-class, and the
        // OFFLINE chip must agree with the ranking layer's own gate.
        let mut feed = self.advisor.feed(self.ai.configured());
        feed.cards.retain(|c| match &c.cta {
            Some(crate::advisor::CardCta::Review { proposal }) => self
                .state
                .proposals
                .get(proposal)
                .map(|p| matches!(p.status, ProposalStatus::Draft | ProposalStatus::Reviewing))
                .unwrap_or(false),
            _ => true,
        });
        feed
    }

    /// Dismiss = hide the card AND mute its rule (persisted) — the spec's
    /// anti-nag contract: dismissing means "stop telling me about this".
    pub fn advisor_dismiss(&mut self, card_id: &str) -> AdvisorFeed {
        let mut rule = None;
        if let Some(card) = self.advisor.cards.iter_mut().find(|c| c.id == card_id) {
            card.dismissed = true;
            rule = Some(card.rule.clone());
            let _ = self
                .file
                .save_advisor_card(&card.id, &serde_json::to_string(card).unwrap_or_default());
        }
        if let Some(rule) = rule {
            self.advisor.muted.insert(rule.clone());
            let _ = self.file.add_mute(&rule, &crate::jobs::now_rfc3339());
        }
        self.advisor_feed()
    }

    pub fn advisor_unmute(&mut self, rule: &str) -> AdvisorFeed {
        self.advisor.muted.remove(rule);
        let _ = self.file.remove_mute(rule);
        self.advisor_feed()
    }

    pub fn advisor_set_paused(&mut self, paused: bool) -> AdvisorFeed {
        self.advisor.paused = paused;
        self.advisor_feed()
    }

    pub fn set_view_state(&mut self, json: &str) -> Result<(), SessionError> {
        self.file.set_view_state(json)?;
        Ok(())
    }

    fn applied_count(&self) -> usize {
        self.undo.entries().len()
    }

    fn nav_response(&mut self, batch: PatchBatch) -> EditResponse {
        EditResponse {
            patches: batch,
            derived: self.solve_all_readonly(),
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            undo_label: self.undo.undo_label().map(String::from),
            created: vec![],
            plan_hash: self.plan_hash(),
            advisor: self.advisor_feed(),
        }
    }

    fn solve_trigger(cmds: &[Command]) -> T0Edit {
        for cmd in cmds {
            match cmd {
                Command::SetPortRate { id, rate } => {
                    return T0Edit::SetTarget {
                        port: id.clone(),
                        rate: *rate,
                    }
                }
                Command::SetGroupClock { id, clock } => {
                    return T0Edit::SetClock {
                        group: id.clone(),
                        clock: *clock,
                    }
                }
                _ => {}
            }
        }
        T0Edit::Recompute
    }

    /// Build the pure solver snapshot for one factory from canonical state + gamedata.
    pub fn snapshot(&self, fid: &Id) -> Option<FactorySnapshot> {
        let factory = self.state.factories.get(fid)?;
        let mut groups = Vec::new();
        for gid in &factory.groups {
            let g = self.state.groups.get(gid)?;
            let recipe = self.gamedata.recipes.get(&g.recipe)?;
            let power = gamedata::db::recipe_power(&self.gamedata, recipe, &g.machine);
            groups.push(GroupSpec {
                id: g.id.clone(),
                recipe: RecipeSpec {
                    id: recipe.class_name.clone(),
                    machine: g.machine.clone(),
                    duration_s: recipe.duration_s,
                    inputs: recipe.ingredients.clone(),
                    outputs: recipe.products.clone(),
                    power_mw: power,
                },
                // The solver plans with the effective values (built baseline
                // overlaid by any planned delta) but never writes deltas back.
                count: g.effective_count(),
                clock: g.effective_clock(),
            });
        }
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        for pid in &factory.ports {
            let p = self.state.ports.get(pid)?;
            match p.direction {
                PortDirection::In => inputs.push(InputPortSpec {
                    id: p.id.clone(),
                    item: p.item.clone(),
                    ceiling: p.rate_ceiling,
                }),
                PortDirection::Out => outputs.push(OutputPortSpec {
                    id: p.id.clone(),
                    item: p.item.clone(),
                    rate: p.rate,
                }),
            }
        }
        let edges = self
            .state
            .edges
            .values()
            .filter(|e| &e.factory == fid)
            .map(|e| EdgeSpec {
                id: e.id.clone(),
                from: to_node_ref(&e.from, &self.state),
                to: to_node_ref(&e.to, &self.state),
                item: e.item.clone(),
                capacity: belt_capacity(e.tier),
            })
            .collect();
        let junctions = self
            .state
            .junctions
            .values()
            .filter(|j| &j.factory == fid)
            .map(|j| j.id.clone())
            .collect();
        Some(FactorySnapshot {
            groups,
            edges,
            inputs,
            outputs,
            junctions,
        })
    }

    /// Extraction ceiling for a node claim, from gamedata (items/min).
    pub fn claim_rate(&self, claim: &NodeClaim) -> f64 {
        let Some(node) = self.world.nodes.iter().find(|n| n.id == claim.node) else {
            return 0.0;
        };
        let Some(machine) = self.gamedata.machines.get(&claim.extractor) else {
            return 0.0;
        };
        gamedata::docs::extraction_rate(machine, &node.purity, claim.clock)
    }

    /// If the factory has a single output port, always solve as SetTarget on it
    /// so the ceiling/binding is available for the slider tick.
    fn trigger_for_factory(&self, snapshot: &FactorySnapshot, trigger: &T0Edit) -> T0Edit {
        match trigger {
            T0Edit::SetTarget { port, rate } if snapshot.outputs.iter().any(|p| &p.id == port) => {
                T0Edit::SetTarget {
                    port: port.clone(),
                    rate: *rate,
                }
            }
            T0Edit::SetClock { group, clock } if snapshot.groups.iter().any(|g| &g.id == group) => {
                T0Edit::SetClock {
                    group: group.clone(),
                    clock: *clock,
                }
            }
            _ => {
                if snapshot.outputs.len() == 1 {
                    T0Edit::SetTarget {
                        port: snapshot.outputs[0].id.clone(),
                        rate: snapshot.outputs[0].rate,
                    }
                } else {
                    T0Edit::Recompute
                }
            }
        }
    }

    /// Solve order over the inter-factory route graph (upstream first).
    /// Cycles fall back to key order with the cycle flagged — no dead ends.
    fn empire_order(&self) -> (Vec<Id>, bool) {
        let mut deps: BTreeMap<Id, Vec<Id>> = BTreeMap::new(); // factory -> upstream factories
        for r in self.state.routes.values() {
            if !matches!(r.kind, RouteKind::Power | RouteKind::Pipe { .. }) {
                let (Some(src), Some(dst)) = (
                    self.state.ports.get(&r.endpoints.0),
                    self.state.ports.get(&r.endpoints.1),
                ) else {
                    continue;
                };
                deps.entry(dst.factory.clone())
                    .or_default()
                    .push(src.factory.clone());
            }
        }
        let all: Vec<Id> = self.state.factories.keys().cloned().collect();
        let mut order = Vec::new();
        let mut placed: std::collections::BTreeSet<Id> = Default::default();
        let mut remaining: Vec<Id> = all.clone();
        while !remaining.is_empty() {
            let before = order.len();
            remaining.retain(|fid| {
                let ready = deps
                    .get(fid)
                    .map(|ups| {
                        ups.iter()
                            .all(|u| placed.contains(u) || !self.state.factories.contains_key(u))
                    })
                    .unwrap_or(true);
                if ready {
                    order.push(fid.clone());
                    placed.insert(fid.clone());
                }
                !ready
            });
            if order.len() == before {
                // cycle: take the rest in stable order
                order.extend(remaining.iter().cloned());
                return (order, true);
            }
        }
        (order, false)
    }

    /// Feed a factory's bound Out routes downstream: the achieved out rate
    /// (absent from `ports` = 0) capped by the route's cargo capacity becomes
    /// the downstream In port's supply ceiling. Error/skip paths call this
    /// with EMPTY ports so a factory that couldn't solve honestly propagates
    /// zero supply instead of leaving downstream fully supplied.
    fn feed_downstream(
        &self,
        fid: &Id,
        ports: &BTreeMap<String, f64>,
        supplies: &mut BTreeMap<Id, f64>,
        route_supply: &mut BTreeMap<Id, f64>,
    ) {
        let Some(factory) = self.state.factories.get(fid) else {
            return;
        };
        for pid in &factory.ports {
            let Some(port) = self.state.ports.get(pid) else {
                continue;
            };
            if port.direction != PortDirection::Out {
                continue;
            }
            let Some(rid) = &port.bound_route else {
                continue;
            };
            let Some(route) = self.state.routes.get(rid) else {
                continue;
            };
            let item = route.manifest.first().map(|(i, _)| i.as_str());
            let Some((cap, _)) = cargo_capacity(
                &self.gamedata,
                &route.kind,
                polyline_length(&route.path),
                item,
            ) else {
                continue;
            };
            let out_rate = ports.get(pid).copied().unwrap_or(0.0);
            let supply = out_rate.min(cap);
            supplies.insert(route.endpoints.1.clone(), supply);
            route_supply.insert(rid.clone(), supply);
        }
    }

    /// The empire pass: solve factories upstream-first, propagating supply
    /// ceilings through bound routes. With `tx`, solver-owned numbers write
    /// back into canonical state (counts/clocks, clamped edited target, route
    /// manifests) — all inside the causing command's undo entry.
    fn empire_solve(&mut self, trigger: &T0Edit, mut tx: Option<&mut Transaction>) -> Derived {
        let started = std::time::Instant::now();
        let mut derived = Derived::default();
        let (order, cyclic) = self.empire_order();
        derived.empire_cycle = cyclic;

        // supplied rate per bound In port (upstream out rate capped by the belt)
        let mut supplies: BTreeMap<Id, f64> = BTreeMap::new();
        let mut route_supply: BTreeMap<Id, f64> = BTreeMap::new();

        for fid in &order {
            let Some(mut snapshot) = self.snapshot(fid) else {
                derived.factories.insert(
                    fid.clone(),
                    Self::error_factory("missing recipe or machine data"),
                );
                self.feed_downstream(fid, &BTreeMap::new(), &mut supplies, &mut route_supply);
                continue;
            };
            // effective ceilings: a bound In port can't intake more than its route supplies
            for input in &mut snapshot.inputs {
                if let Some(port) = self.state.ports.get(&input.id) {
                    if port.bound_route.is_some() {
                        if let Some(supply) = supplies.get(&input.id) {
                            input.ceiling = Some(match input.ceiling {
                                Some(c) => c.min(*supply),
                                None => *supply,
                            });
                        }
                    }
                }
            }
            // A wired group-less factory (e.g. the wizard's extraction-and-
            // ship site: in port → out port) is a valid pass-through — T1
            // solves it with edge vars, ceilings and elastic targets alone.
            // Only a factory with neither groups nor edges keeps the error.
            if snapshot.groups.is_empty() && snapshot.edges.is_empty() {
                derived
                    .factories
                    .insert(fid.clone(), Self::error_factory("no machine groups yet"));
                self.feed_downstream(fid, &BTreeMap::new(), &mut supplies, &mut route_supply);
                continue;
            }
            let trig = self.trigger_for_factory(&snapshot, trigger);
            let solve_on_release = self.slow_solves.get(fid).copied().unwrap_or(0) >= 3;
            let result = match solver::t1::solve(&snapshot, &trig) {
                Ok(r) => r,
                Err(e) => {
                    derived
                        .factories
                        .insert(fid.clone(), Self::error_factory(&e.to_string()));
                    self.feed_downstream(fid, &BTreeMap::new(), &mut supplies, &mut route_supply);
                    continue;
                }
            };

            // write-backs (only on the edit path). A degraded solve (any
            // shortfall) is advisory: never rewrite planned counts/clocks to
            // the starved values — they spring back once the gap is wired.
            if let Some(tx) = tx.as_deref_mut() {
                if result.shortfalls.is_empty() {
                    for (gid, gr) in &result.groups {
                        if let Some(g) = self.state.groups.get(gid) {
                            // ◆ built groups are game ground truth: the solver may
                            // read them but never resize them — only import sync
                            // (the documented exception) writes the built layer.
                            if g.status == Status::Built {
                                continue;
                            }
                            if g.count != gr.count || (g.clock - gr.clock).abs() > 1e-9 {
                                let mut g = g.clone();
                                g.count = gr.count;
                                g.clock = gr.clock;
                                let ops = self.state.upsert(Entity::Group(g));
                                tx.forward.push(ops.0);
                                tx.inverse.push(ops.1);
                            }
                        }
                    }
                }
                // Clamp write-back only for the port the user actually edited —
                // an upstream dip must surface as a deficit, never silently
                // rewrite a downstream target. Out ports only: for an In-port
                // trigger, result.ports carries the solved INTAKE (not a clamped
                // target), and writing that back would replace the value the same
                // command batch just set with an unrelated flow figure.
                if result.clamped {
                    if let T0Edit::SetTarget { port, .. } = trigger {
                        if let (Some(p), Some(rate)) =
                            (self.state.ports.get(port), result.ports.get(port))
                        {
                            if p.direction == PortDirection::Out && (p.rate - rate).abs() > 1e-9 {
                                let mut p = p.clone();
                                p.rate = *rate;
                                let ops = self.state.upsert(Entity::Port(p));
                                tx.forward.push(ops.0);
                                tx.inverse.push(ops.1);
                            }
                        }
                    }
                }
                let ms = result.solve_us as f64 / 1000.0;
                let slow = self.slow_solves.entry(fid.clone()).or_insert(0);
                if ms > T1_BUDGET_MS {
                    *slow += 1;
                } else {
                    *slow = 0;
                }
            }

            // feed downstream: out port rate capped by the route's belt tier
            self.feed_downstream(fid, &result.ports, &mut supplies, &mut route_supply);

            derived.total_power_mw += result.total_power_mw;
            derived
                .factories
                .insert(fid.clone(), to_derived(&result, solve_on_release));
        }

        // Route flows (= downstream intake), deficits, manifests.
        // Probe memo for DeficitRow::needed: canonical snapshot (no supply
        // injection), elastic Recompute — the intake the factory would pull
        // at its own targets. At most one probe solve per starved factory.
        let mut probes: BTreeMap<Id, Option<BTreeMap<String, f64>>> = BTreeMap::new();
        let route_list: Vec<planner_core::entities::Route> =
            self.state.routes.values().cloned().collect();
        for r in &route_list {
            let item_class = r.manifest.first().map(|(i, _)| i.as_str());
            let Some((capacity, transport)) = cargo_capacity(
                &self.gamedata,
                &r.kind,
                polyline_length(&r.path),
                item_class,
            ) else {
                continue;
            };
            let dst_port = &r.endpoints.1;
            let dst_factory = self.state.ports.get(dst_port).map(|p| p.factory.clone());
            let flow = dst_factory
                .as_ref()
                .and_then(|f| derived.factories.get(f))
                .and_then(|df| df.ports.get(dst_port))
                .copied()
                .unwrap_or(0.0);
            let supplied = route_supply.get(&r.id).copied().unwrap_or(0.0);
            let item = r.manifest.first().map(|(i, _)| i.clone());
            derived.routes.insert(
                r.id.clone(),
                DerivedRoute {
                    flow,
                    supplied,
                    capacity,
                    saturation: if capacity > 0.0 { flow / capacity } else { 0.0 },
                    length_m: polyline_length(&r.path),
                    climb_up_m: polyline_climb(&r.path).0,
                    climb_down_m: polyline_climb(&r.path).1,
                    item,
                    transport,
                },
            );
            // Deficit contract — ONE decision, at most ONE row per route: the
            // route is in deficit iff the downstream factory's solve was
            // limited by this In port. T1 partitions unmet demand into two
            // mutually exclusive channels and both feed this row:
            //  - clamped channel: an edited/synthesized SetTarget hard-stopped
            //    at `target_ceiling` binding InputCeiling on this port
            //    (shortfalls are empty by construction on that path);
            //  - degraded channel: a Recompute/multi-Out solve reporting
            //    `shortfalls` whose binding names InputCeiling on this port.
            // `needed` = the intake the factory's canonical targets require:
            // the proportional flow·requested/max_rate when the clamped
            // channel yields a usable max_rate, else a memoized probe solve
            // (canonical snapshot, elastic Recompute). Total starvation
            // (max_rate = 0) is a deficit like any other — never dropped.
            if let (Some(fid), Some(df)) = (
                dst_factory.clone(),
                dst_factory.as_ref().and_then(|f| derived.factories.get(f)),
            ) {
                let requested = self
                    .state
                    .factories
                    .get(&fid)
                    .and_then(|f| {
                        f.ports.iter().find_map(|pid| {
                            let p = self.state.ports.get(pid)?;
                            (p.direction == PortDirection::Out).then_some(p.rate)
                        })
                    })
                    .unwrap_or(0.0);
                let ceiling_max = df.target_ceiling.as_ref().and_then(|c| match &c.binding {
                    solver::model::Constraint::InputCeiling { port, .. } if port == dst_port => {
                        Some(c.max_rate)
                    }
                    _ => None,
                });
                let starved_by_ceiling =
                    ceiling_max.is_some_and(|max_rate| requested > max_rate + 1e-6);
                let starved_by_shortfall = df.shortfalls.values().any(|s| {
                    matches!(
                        &s.binding,
                        Some(solver::model::Constraint::InputCeiling { port, .. })
                            if port == dst_port
                    )
                });
                if starved_by_ceiling || starved_by_shortfall {
                    let needed = match ceiling_max {
                        Some(max_rate) if starved_by_ceiling && max_rate > 0.0 => {
                            Some(flow * requested / max_rate)
                        }
                        _ => probes
                            .entry(fid.clone())
                            .or_insert_with(|| {
                                self.snapshot(&fid).and_then(|snap| {
                                    solver::t1::solve(&snap, &T0Edit::Recompute)
                                        .ok()
                                        .map(|res| res.ports)
                                })
                            })
                            .as_ref()
                            .and_then(|ports| ports.get(dst_port).copied()),
                    };
                    // Skip gracefully when no probe is available (the
                    // canonical snapshot itself can't be built or solved).
                    if let Some(needed) = needed {
                        derived.deficits.push(DeficitRow {
                            factory: fid,
                            port: dst_port.clone(),
                            route: Some(r.id.clone()),
                            item: self
                                .state
                                .ports
                                .get(dst_port)
                                .map(|p| p.item.clone())
                                .unwrap_or_default(),
                            needed,
                            supplied,
                        });
                    }
                }
            }
            // Manifest is canonical (§3.1.4) — the solver maintains it.
            if let Some(tx) = tx.as_deref_mut() {
                if let Some(cur) = self.state.routes.get(&r.id) {
                    let want = vec![(item_or(&cur.manifest, &r.endpoints.0, &self.state), flow)];
                    if cur.manifest != want {
                        let mut updated = cur.clone();
                        updated.manifest = want;
                        let ops = self.state.upsert(Entity::Route(updated));
                        tx.forward.push(ops.0);
                        tx.inverse.push(ops.1);
                    }
                }
            }
        }

        // Node claim conflicts (§3.1.3 — representable, rendered CRIT, never
        // prevented) + position drift (W2b-C). `conflict` stays double-claim
        // only; `drift` fires when a plan-local override disagrees with the
        // ambient catalog coordinate past the correction threshold. Save-only
        // nodes (absent from the catalog) have nothing to disagree with.
        let mut by_node: BTreeMap<String, u32> = BTreeMap::new();
        for c in self.state.node_claims.values() {
            *by_node.entry(c.node.clone()).or_insert(0) += 1;
        }
        let drifted = |node: &str| -> bool {
            let Some(ov) = self.state.node_overrides.get(node) else {
                return false;
            };
            let Some(pos) = ov.pos else {
                return false;
            };
            self.world
                .nodes
                .iter()
                .find(|n| n.id == node)
                .map(|n| (n.x - pos.x).hypot(n.y - pos.y) > crate::import::NODE_DRIFT_M)
                .unwrap_or(false)
        };
        // Only claimed nodes render: iterate `by_node` alone. An override-only
        // (zero-claim) node stays inert in canonical state and auto-dissolves on
        // re-import via `dissolve_stale_node_overrides`, so it never draws an
        // owner-less dot. A claimed node's `drifted()` still consults its
        // override (path unchanged).
        let node_ids: BTreeSet<String> = by_node.keys().cloned().collect();
        for node in node_ids {
            let claims = by_node.get(&node).copied().unwrap_or(0);
            let drift = drifted(&node);
            derived.nodes.insert(
                node,
                DerivedNode {
                    claims,
                    conflict: claims > 1,
                    drift,
                },
            );
        }
        // Power circuits: union-find over Power routes; a grid's generation is
        // the POWER_ITEM output of its member factories, demand their draw.
        {
            let mut parent: BTreeMap<Id, Id> = self
                .state
                .factories
                .keys()
                .map(|k| (k.clone(), k.clone()))
                .collect();
            fn find(parent: &mut BTreeMap<Id, Id>, x: &Id) -> Id {
                let p = parent.get(x).cloned().unwrap_or_else(|| x.clone());
                if &p == x {
                    p
                } else {
                    let root = find(parent, &p);
                    parent.insert(x.clone(), root.clone());
                    root
                }
            }
            let mut in_grid: std::collections::BTreeSet<Id> = Default::default();
            for r in self.state.routes.values() {
                if matches!(r.kind, RouteKind::Power) {
                    let (a, b) = (&r.endpoints.0, &r.endpoints.1);
                    in_grid.insert(a.clone());
                    in_grid.insert(b.clone());
                    let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                    if ra != rb {
                        parent.insert(ra, rb);
                    }
                }
            }
            let mut grids: BTreeMap<Id, Vec<Id>> = BTreeMap::new();
            for fid in in_grid {
                let root = find(&mut parent, &fid);
                grids.entry(root).or_default().push(fid);
            }
            let gen_of = |fid: &Id| -> f64 {
                derived
                    .factories
                    .get(fid)
                    .map(|df| {
                        df.groups
                            .values()
                            .map(|g| {
                                g.out_rates
                                    .get(gamedata::docs::POWER_ITEM)
                                    .copied()
                                    .unwrap_or(0.0)
                            })
                            .sum::<f64>()
                    })
                    .unwrap_or(0.0)
            };
            for (i, (_, members)) in grids.into_iter().enumerate() {
                let generation_mw: f64 = members.iter().map(&gen_of).sum();
                let demand_mw: f64 = members
                    .iter()
                    .filter_map(|f| derived.factories.get(f))
                    .map(|df| df.total_power_mw)
                    .sum();
                // A2.3 shedding: split the grid at each switch's line; the
                // load side is the half with less generation. Shed order is
                // P8 → P1; SHEDS AT accumulates earlier sheds.
                let member_set: std::collections::BTreeSet<&Id> = members.iter().collect();
                let circuit_routes: Vec<&Route> = self
                    .state
                    .routes
                    .values()
                    .filter(|r| {
                        matches!(r.kind, RouteKind::Power) && member_set.contains(&r.endpoints.0)
                    })
                    .collect();
                let demand_of = |fid: &Id| -> f64 {
                    derived
                        .factories
                        .get(fid)
                        .map(|df| df.total_power_mw)
                        .unwrap_or(0.0)
                };
                let mut switches: Vec<DerivedSwitch> = Vec::new();
                for sw in self.state.switches.values() {
                    let Some(on) = circuit_routes.iter().find(|r| r.id == sw.route) else {
                        continue;
                    };
                    // component containing endpoint B with the switch's line cut
                    let mut side: std::collections::BTreeSet<Id> = Default::default();
                    let mut stack = vec![on.endpoints.1.clone()];
                    while let Some(f) = stack.pop() {
                        if !side.insert(f.clone()) {
                            continue;
                        }
                        for r in &circuit_routes {
                            if r.id == sw.route {
                                continue;
                            }
                            if r.endpoints.0 == f && !side.contains(&r.endpoints.1) {
                                stack.push(r.endpoints.1.clone());
                            } else if r.endpoints.1 == f && !side.contains(&r.endpoints.0) {
                                stack.push(r.endpoints.0.clone());
                            }
                        }
                    }
                    let gen_b: f64 = side.iter().map(&gen_of).sum();
                    let gen_a = generation_mw - gen_b;
                    let downstream_mw = if gen_b <= gen_a {
                        side.iter().map(&demand_of).sum()
                    } else {
                        members
                            .iter()
                            .filter(|m| !side.contains(*m))
                            .map(&demand_of)
                            .sum()
                    };
                    switches.push(DerivedSwitch {
                        id: sw.id.clone(),
                        priority: sw.priority,
                        downstream_mw,
                        sheds_at_mw: 0.0, // filled after the shed-order sort
                    });
                }
                switches.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
                let mut shed_acc = 0.0;
                for sw in switches.iter_mut() {
                    sw.sheds_at_mw = generation_mw + shed_acc;
                    shed_acc += sw.downstream_mw;
                }
                let next_shed = switches.first().map(|sw| {
                    format!(
                        "P{} @ +{:.0} MW growth",
                        sw.priority,
                        (sw.sheds_at_mw - demand_mw).max(0.0)
                    )
                });
                derived.circuits.push(DerivedCircuit {
                    name: format!("GRID {}", (b'A' + (i as u8 % 26)) as char),
                    members,
                    generation_mw,
                    demand_mw,
                    switches,
                    next_shed,
                });
            }
            // Empire generation, per generator group: the solved POWER_ITEM
            // output when the group's recipe resolves and its factory solved —
            // so a fuel-starved plant reads its real (lower) output, matching
            // the per-grid sums above — else nameplate mw × count × clock.
            // The nameplate arm covers recipe-less imported generators (#58),
            // unresolvable recipes, and solve-skipped/errored factories: never
            // a silent 0 that would read as a false "NO GEN".
            derived.total_generation_mw = self
                .state
                .groups
                .values()
                .filter_map(|g| {
                    let mw = match self.gamedata.machines.get(&g.machine).map(|m| &m.kind) {
                        Some(gamedata::docs::MachineKind::Generator {
                            power_production_mw,
                        }) => *power_production_mw,
                        _ => return None,
                    };
                    let solved = derived
                        .factories
                        .get(&g.factory)
                        .filter(|df| df.solve_error.is_none())
                        .and_then(|df| df.groups.get(&g.id))
                        .map(|dg| {
                            dg.out_rates
                                .get(gamedata::docs::POWER_ITEM)
                                .copied()
                                .unwrap_or(0.0)
                        });
                    match solved {
                        Some(solved) if self.gamedata.recipes.contains_key(&g.recipe) => {
                            Some(solved)
                        }
                        _ => Some(mw * g.effective_count() as f64 * g.effective_clock()),
                    }
                })
                .sum();
        }
        // Build queue: a pure projection over canonical state + gamedata,
        // recomputed here like circuits/deficits (no stored ordering entity).
        derived.build_queue = derive_build_queue(&self.state, &self.gamedata);
        // Cutovers: cheap presence/steps projection (no scratch-solves here).
        // Reuse the queue just computed above rather than deriving it twice.
        derived.cutovers =
            crate::cutover::derive_cutovers_with(&self.state, &self.gamedata, &derived.build_queue);
        derived.recompute_us = started.elapsed().as_micros() as u64;
        derived
    }

    fn error_factory(message: &str) -> DerivedFactory {
        DerivedFactory {
            groups: BTreeMap::new(),
            edges: BTreeMap::new(),
            ports: BTreeMap::new(),
            shortfalls: BTreeMap::new(),
            total_power_mw: 0.0,
            target_ceiling: None,
            solve_us: 0,
            solve_on_release: false,
            solve_error: Some(message.into()),
        }
    }

    /// Recompute derived state for everything, without touching canonical state.
    pub fn solve_all_readonly(&mut self) -> Derived {
        self.empire_solve(&T0Edit::Recompute, None)
    }

    /// Ranked next-move opportunities (PR 9). Computed on demand over a fresh
    /// read-only solve, exactly like the advisor feed — no persistence, no new
    /// commands, nothing undoable (a read-only projection).
    pub fn next_moves(&mut self) -> Vec<crate::opportunities::Opportunity> {
        let derived = self.solve_all_readonly();
        crate::opportunities::derive_opportunities(
            &self.state,
            &self.gamedata,
            &derived,
            &self.world,
            &self.unlocked,
            &self.state.meta.preferences,
        )
    }

    /// PR 3: set the plan-scoped NEXT-MOVES preferences and persist them.
    /// NOT undoable (a filter toggle is not plan geometry) and excluded from
    /// `plan_hash`, so it never staleness-flags open proposals; it persists via
    /// the plan meta row and reaches the renderer through hydrate
    /// (`plan.meta.preferences`). Returns the fresh heuristic view so the caller
    /// can render immediately (the renderer also bumps its epoch to re-rank).
    pub fn set_next_preferences(
        &mut self,
        preferences: NextPreferences,
    ) -> Result<PreferencesView, SessionError> {
        self.state.meta.preferences = preferences;
        self.file
            .save_meta(&self.state.meta)
            .map_err(SessionError::Persist)?;
        let opportunities = self.next_moves();
        Ok(PreferencesView {
            preferences: self.state.meta.preferences.clone(),
            opportunities,
        })
    }

    /// Read-only train answer-sheet for a PROSPECTIVE route (task #49): given
    /// two factories, a transport kind, a demand rate, and the moved item,
    /// return the trains-needed answer from the canonical transport math. The
    /// route length is the straight line between the two factory pins (the same
    /// path a confirmed route would take). Creates and mutates nothing; belt/
    /// pipe kinds have no consist math and return None.
    pub fn route_calc(
        &self,
        from: &str,
        to: &str,
        kind: &RouteKind,
        demand_per_min: f64,
        item: Option<&str>,
    ) -> Option<planner_core::transport::TrainAnswer> {
        let a = self.state.factories.get(from)?;
        let b = self.state.factories.get(to)?;
        let path = [a.position, b.position];
        let (_, math) = cargo_capacity(&self.gamedata, kind, polyline_length(&path), item)?;
        let math = math?;
        let units = match kind {
            RouteKind::Rail { spec } => spec.consists as u32,
            RouteKind::Truck { spec } => spec.trucks as u32,
            _ => 1,
        };
        Some(planner_core::transport::train_answer(
            math,
            units,
            demand_per_min,
        ))
    }
}

fn to_node_ref(end: &EdgeEnd, state: &PlanState) -> NodeRef {
    match end {
        EdgeEnd::Group(id) => NodeRef::Group(id.clone()),
        EdgeEnd::Junction(id) => NodeRef::Junction(id.clone()),
        EdgeEnd::Port(id) => match state.ports.get(id).map(|p| p.direction) {
            Some(PortDirection::In) => NodeRef::Input(id.clone()),
            _ => NodeRef::Output(id.clone()),
        },
    }
}

fn to_derived(r: &SolveResult, solve_on_release: bool) -> DerivedFactory {
    DerivedFactory {
        groups: r
            .groups
            .iter()
            .map(|(id, g)| {
                (
                    id.clone(),
                    DerivedGroup {
                        in_rates: g.in_rates.clone(),
                        out_rates: g.out_rates.clone(),
                        power_mw: g.power_mw,
                    },
                )
            })
            .collect(),
        edges: r
            .edges
            .iter()
            .map(|(id, e)| {
                (
                    id.clone(),
                    DerivedEdge {
                        flow: e.flow,
                        saturation: e.saturation,
                    },
                )
            })
            .collect(),
        ports: r.ports.clone(),
        shortfalls: r.shortfalls.clone(),
        total_power_mw: r.total_power_mw,
        target_ceiling: r.target_ceiling.clone(),
        solve_us: r.solve_us,
        solve_on_release,
        solve_error: None,
    }
}

/// Included items in dependency order (deps first). Items whose dependencies
/// are excluded are skipped — the toggle cascade should prevent that state,
/// but accept must never guess.
fn ordered_included(p: &Proposal) -> Vec<&ProposalItem> {
    let mut out: Vec<&ProposalItem> = Vec::new();
    let mut placed: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    loop {
        let mut progressed = false;
        for item in p.items.iter().filter(|i| i.included) {
            if placed.contains(item.id.as_str()) {
                continue;
            }
            let deps_ok = item.depends_on.iter().all(|d| {
                placed.contains(d.as_str()) || p.item(d).map(|i| !i.included).unwrap_or(true)
            });
            let deps_included = item
                .depends_on
                .iter()
                .all(|d| p.item(d).map(|i| i.included).unwrap_or(false) || p.item(d).is_none());
            if deps_ok && deps_included {
                placed.insert(item.id.as_str());
                out.push(item);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

/// Included items `ordered_included` could NOT place for any reason other than
/// a legitimately excluded dependency — i.e. a dependency cycle among included
/// items. Skipping an item whose dependency (direct or transitive) is unchecked
/// is documented intent; dropping a cyclic pair while reporting ACCEPTED is
/// silent data loss, so accept fails loud on these (the same abort-before-commit
/// policy unresolved aliases follow).
fn cycle_dropped(p: &Proposal) -> Vec<&str> {
    let placeable: std::collections::BTreeSet<&str> =
        ordered_included(p).iter().map(|i| i.id.as_str()).collect();
    // Excluded-tainted closure: an included item is a legitimate skip when any
    // dependency is an existing unchecked item, or is itself legitimately skipped.
    let mut tainted: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    loop {
        let mut grew = false;
        for item in p.items.iter().filter(|i| i.included) {
            if tainted.contains(item.id.as_str()) {
                continue;
            }
            let hit = item.depends_on.iter().any(|d| {
                tainted.contains(d.as_str()) || p.item(d).map(|i| !i.included).unwrap_or(false)
            });
            if hit {
                tainted.insert(item.id.as_str());
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    p.items
        .iter()
        .filter(|i| {
            i.included && !placeable.contains(i.id.as_str()) && !tainted.contains(i.id.as_str())
        })
        .map(|i| i.label.as_str())
        .collect()
}

/// Stack size for transport math: SS_* → items per inventory slot.
fn stack_size_of(gd: &GameData, item: Option<&str>) -> f64 {
    let Some(item) = item.and_then(|i| gd.items.get(i)) else {
        return 100.0;
    };
    match item.stack_size.as_str() {
        "SS_ONE" => 1.0,
        "SS_SMALL" => 50.0,
        "SS_MEDIUM" => 100.0,
        "SS_BIG" | "SS_LARGE" => 200.0,
        "SS_HUGE" => 500.0,
        _ => 100.0,
    }
}

/// Capacity + optional math block for a cargo route. None for power/pipe.
fn cargo_capacity(
    gd: &GameData,
    kind: &RouteKind,
    path_len_m: f64,
    item: Option<&str>,
) -> Option<(f64, Option<planner_core::transport::TransportMath>)> {
    use planner_core::transport::*;
    let stack = stack_size_of(gd, item);
    match kind {
        RouteKind::Belt { tier } => Some((belt_capacity(*tier), None)),
        RouteKind::Rail { spec } => {
            let m = rail_math(path_len_m, spec, stack);
            Some((m.throughput_per_min, Some(m)))
        }
        RouteKind::Truck { spec } => {
            let m = truck_math(path_len_m, spec, stack);
            Some((m.throughput_per_min, Some(m)))
        }
        RouteKind::Drone { spec } => {
            let m = drone_math(path_len_m, spec, stack);
            Some((m.throughput_per_min, Some(m)))
        }
        RouteKind::Pipe { .. } | RouteKind::Power => None,
    }
}

fn polyline_length(path: &[MapPos]) -> f64 {
    path.windows(2)
        .map(|w| {
            ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2) + (w[1].z - w[0].z).powi(2))
                .sqrt()
        })
        .sum()
}

/// Total climb along a path: (meters up, meters down). Zero on flat plans —
/// elevation is planner-entered until a licensed heightmap exists.
fn polyline_climb(path: &[MapPos]) -> (f64, f64) {
    path.windows(2).fold((0.0, 0.0), |(up, down), w| {
        let dz = w[1].z - w[0].z;
        if dz > 0.0 {
            (up + dz, down)
        } else {
            (up, down - dz)
        }
    })
}

/// Re-site a solver-drafted factory beside the old pin (W2a): rewrite every
/// occurrence of the solver's original site position `orig` to `target` inside a
/// command — the CreateFactory pin and any AddRoute path endpoint anchored there
/// — so the replacement lands next to the factory it retires and its routes
/// track the move. Purely on the DRAFT proposal, before accept.
fn shift_site_pos(cmd: &mut Command, orig: &MapPos, target: &MapPos) {
    let same = |p: &MapPos| p.x == orig.x && p.y == orig.y && p.z == orig.z;
    match cmd {
        Command::CreateFactory { position, .. } if same(position) => *position = *target,
        Command::AddRoute { path, .. } => {
            for p in path.iter_mut() {
                if same(p) {
                    *p = *target;
                }
            }
        }
        _ => {}
    }
}

fn item_or(manifest: &[(String, f64)], src_port: &Id, state: &PlanState) -> String {
    manifest
        .first()
        .map(|(i, _)| i.clone())
        .or_else(|| state.ports.get(src_port).map(|p| p.item.clone()))
        .unwrap_or_default()
}

#[cfg(test)]
mod circuit_tests {
    use super::circuit_level;

    /// The shared helper reproduces the EXACT 0.20 / 0.05 boundaries the
    /// advisor's power_swing rule and the review consequence used inline, so
    /// routing all three through it is behavior-preserving.
    #[test]
    fn circuit_level_matches_the_inline_thresholds() {
        // 20% headroom is the OK floor: the advisor fired STRICTLY under 0.20
        assert_eq!(circuit_level(100.0, 79.0).1, "ok"); // 21% headroom
        assert_eq!(circuit_level(100.0, 80.0).1, "ok"); // exactly 20% → still OK
        assert_eq!(circuit_level(100.0, 81.0).1, "warn"); // 19% → thin
                                                          // 5% is the crit floor: the consequence pushed a warning STRICTLY under
                                                          // 0.05, so exactly 5% is thin (warn), not yet critical
        assert_eq!(circuit_level(100.0, 94.0).1, "warn"); // 6% → thin
        assert_eq!(circuit_level(100.0, 95.0).1, "warn"); // exactly 5% → thin
        assert_eq!(circuit_level(100.0, 96.0).1, "crit"); // 4% → critical
                                                          // headroom value itself is the inline formula, byte-for-byte
        assert!((circuit_level(150.0, 30.0).0 - 0.8).abs() < 1e-9);
        // degenerate fallbacks: draw with no generation is fully overdrawn,
        // an idle grid is full margin
        assert_eq!(circuit_level(0.0, 10.0), (-1.0, "crit"));
        assert_eq!(circuit_level(0.0, 0.0), (1.0, "ok"));
    }
}
