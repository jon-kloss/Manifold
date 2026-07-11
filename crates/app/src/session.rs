//! One open plan file + its canonical state, undo log, gamedata, and solver
//! orchestration. Every mutation: apply commands → T1 re-solve → fold solve
//! write-backs into the same transaction → commit (one undo entry) → persist.

use std::collections::BTreeMap;
use std::path::Path;

use planner_core::commands::{self, Command, DomainError, Transaction};
use planner_core::entities::*;
use planner_core::patch::PatchBatch;
use planner_core::proposals::{fnv1a, resolve_aliases, Proposal, ProposalItem, ProposalStatus};
use planner_core::state::{Entity, PlanState};
use planner_core::undo::UndoLog;

use crate::advisor::{AdvisorFeed, AdvisorState};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalCheck {
    pub item: String,
    pub requested: f64,
    pub achieved: f64,
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
    /// Model API key (env `FICSIT_AI_KEY`; OS keychain when the shell wires it).
    pub ai_key: Option<String>,
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
        let world = gamedata::worldnodes::bundled();
        let (state, entries, cursor) = file.load()?;
        let undo = UndoLog::hydrate_with_cursor(entries, cursor);
        let mut advisor = AdvisorState::default();
        for json in file.load_advisor_cards().unwrap_or_default() {
            if let Ok(card) = serde_json::from_str(&json) {
                advisor.cards.push(card);
            }
        }
        advisor.muted = file.load_mutes().unwrap_or_default().into_iter().collect();
        Ok(Self {
            state,
            undo,
            file,
            gamedata: gd,
            world,
            slow_solves: BTreeMap::new(),
            advisor,
            ai_key: std::env::var("FICSIT_AI_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
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
            "advisor": self.advisor.feed(self.ai_key.is_some()),
            "canUndo": self.undo.can_undo(),
            "canRedo": self.undo.can_redo(),
            "undoLabel": self.undo.undo_label(),
            "viewState": self.file.view_state().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()),
        })
    }

    /// `plan.edit` — apply one or more commands as a single undoable step.
    pub fn edit(&mut self, cmds: Vec<Command>) -> Result<EditResponse, SessionError> {
        if cmds.is_empty() {
            return Err(SessionError::Internal("empty command list".into()));
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
        self.advise(&derived);

        let created = tx.created.clone();
        let entry = self.undo.commit(tx);
        self.file
            .commit(&entry, &self.state.meta, self.undo.entries().len())?;

        Ok(EditResponse {
            patches: entry.forward,
            derived,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            undo_label: self.undo.undo_label().map(String::from),
            created,
            plan_hash: self.plan_hash(),
            advisor: self.advisor.feed(self.ai_key.is_some()),
        })
    }

    pub fn undo(&mut self) -> Result<Option<EditResponse>, SessionError> {
        let Some(batch) = self.undo.undo(&mut self.state) else {
            return Ok(None);
        };
        self.file
            .checkpoint(&batch, &self.state.meta, self.applied_count())?;
        Ok(Some(self.nav_response(batch)))
    }

    pub fn redo(&mut self) -> Result<Option<EditResponse>, SessionError> {
        let Some(batch) = self.undo.redo(&mut self.state) else {
            return Ok(None);
        };
        self.file
            .checkpoint(&batch, &self.state.meta, self.applied_count())?;
        Ok(Some(self.nav_response(batch)))
    }

    /// Plan-content hash for proposal staleness — proposals themselves are
    /// excluded, or drafting one would immediately mark it stale.
    pub fn plan_hash(&self) -> String {
        let mut projection = self.state.project();
        if let Some(map) = projection.as_object_mut() {
            map.remove("proposals");
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
        let label = format!("accept proposal #{}", p.number);
        let mut tx = Transaction::new(label);
        let mut symbols: BTreeMap<String, Id> = BTreeMap::new();
        let mut apply_all =
            |state: &mut PlanState, tx: &mut Transaction| -> Result<(), SessionError> {
                for item in ordered_included(&p) {
                    // SaveReimport drift items sync the ◆ Built layer directly
                    // — the one documented exception to accept-creates-◇-only
                    if let Some(sync) = &item.sync {
                        let op: crate::import::SyncOp = serde_json::from_value(sync.clone())
                            .map_err(|e| SessionError::Internal(e.to_string()))?;
                        crate::import::apply_sync(state, tx, &op, &p.id);
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
        let derived = self.empire_solve(&T0Edit::Recompute, Some(&mut tx));
        self.advise(&derived);
        let created = tx.created.clone();
        let entry = self.undo.commit(tx);
        self.file
            .commit(&entry, &self.state.meta, self.undo.entries().len())?;
        Ok(EditResponse {
            patches: entry.forward,
            derived,
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            undo_label: self.undo.undo_label().map(String::from),
            created,
            plan_hash: self.plan_hash(),
            advisor: self.advisor.feed(self.ai_key.is_some()),
        })
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
                    crate::import::apply_sync(&mut self.state, &mut scratch, &op, &p.id);
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
        let out_rate_of = |state: &PlanState, derived: &Derived, item: &str| -> f64 {
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
        };
        let goal: Vec<GoalCheck> = p
            .goal
            .iter()
            .map(|(item, requested)| GoalCheck {
                item: item.clone(),
                requested: *requested,
                achieved: out_rate_of(&self.state, &after, item)
                    - out_rate_of(&saved, &before, item),
            })
            .collect();
        // new deficits + circuits gone critical feed the amber warning strip
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
        for c in &after.circuits {
            let headroom = if c.generation_mw > 0.0 {
                (c.generation_mw - c.demand_mw) / c.generation_mw
            } else if c.demand_mw > 0.0 {
                -1.0
            } else {
                1.0
            };
            if headroom < 0.05 {
                warnings.push(format!(
                    "{} at {:.0}/{:.0} MW — margin critical",
                    c.name, c.demand_mw, c.generation_mw
                ));
            }
        }
        let consequence = ProposalConsequence {
            goal_met: goal.iter().all(|g| g.achieved >= g.requested - 1e-6),
            goal,
            delta_power_mw: after.total_power_mw - before.total_power_mw,
            delta_generation_mw: after.total_generation_mw - before.total_generation_mw,
            machines,
            warnings,
        };
        self.state = saved;
        Ok(consequence)
    }

    /// Save import (SDD §8). First import writes the ◆ Built layer directly;
    /// re-imports never write — they diff into a SaveReimport proposal.
    pub fn import_save(
        &mut self,
        snapshot: crate::import::ImportSnapshot,
    ) -> Result<ImportOutcome, SessionError> {
        let clusters = crate::import::cluster(&snapshot, &self.gamedata);
        let has_built = self
            .state
            .factories
            .values()
            .any(|f| f.status == Status::Built);
        if !has_built {
            let import_id = planner_core::entities::new_id();
            let mut tx = Transaction::new("import save");
            crate::import::write_built_layer(&mut self.state, &mut tx, &clusters, &import_id);
            let derived = self.empire_solve(&T0Edit::Recompute, Some(&mut tx));
            self.advise(&derived);
            let created = tx.created.clone();
            let entry = self.undo.commit(tx);
            self.file
                .commit(&entry, &self.state.meta, self.undo.entries().len())?;
            return Ok(ImportOutcome::Imported {
                response: EditResponse {
                    patches: entry.forward,
                    derived,
                    can_undo: self.undo.can_undo(),
                    can_redo: self.undo.can_redo(),
                    undo_label: self.undo.undo_label().map(String::from),
                    created,
                    plan_hash: self.plan_hash(),
                    advisor: self.advisor.feed(self.ai_key.is_some()),
                },
                factories: clusters.len() as u32,
                machines: snapshot.machines.len() as u32,
                quarantined: snapshot.quarantined.values().sum(),
            });
        }
        // re-import: diff only, never write
        let items = crate::import::diff_against_built(&self.state, &self.gamedata, &clusters);
        if items.is_empty() {
            return Ok(ImportOutcome::InSync);
        }
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
        };
        let response = self.edit(vec![Command::CreateProposal { proposal }])?;
        let proposal_id = response.created[0].clone();
        Ok(ImportOutcome::Drift {
            response,
            proposal: proposal_id,
        })
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
        self.advisor.feed(self.ai_key.is_some())
    }

    pub fn advisor_unmute(&mut self, rule: &str) -> AdvisorFeed {
        self.advisor.muted.remove(rule);
        let _ = self.file.remove_mute(rule);
        self.advisor.feed(self.ai_key.is_some())
    }

    pub fn advisor_set_paused(&mut self, paused: bool) -> AdvisorFeed {
        self.advisor.paused = paused;
        self.advisor.feed(self.ai_key.is_some())
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
            advisor: self.advisor.feed(self.ai_key.is_some()),
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
            let power = self
                .gamedata
                .machines
                .get(&g.machine)
                .map(|m| m.power_mw)
                .unwrap_or(0.0);
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
                count: g.count,
                clock: g.clock,
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
            if snapshot.groups.is_empty() {
                derived
                    .factories
                    .insert(fid.clone(), Self::error_factory("no machine groups yet"));
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
                    continue;
                }
            };

            // write-backs (only on the edit path)
            if let Some(tx) = tx.as_deref_mut() {
                for (gid, gr) in &result.groups {
                    if let Some(g) = self.state.groups.get(gid) {
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
                // Clamp write-back only for the port the user actually edited —
                // an upstream dip must surface as a deficit, never silently
                // rewrite a downstream target.
                if result.clamped {
                    if let T0Edit::SetTarget { port, .. } = trigger {
                        if let (Some(p), Some(rate)) =
                            (self.state.ports.get(port), result.ports.get(port))
                        {
                            if (p.rate - rate).abs() > 1e-9 {
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
            for pid in &self.state.factories[fid].ports.clone() {
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
                let out_rate = result.ports.get(pid).copied().unwrap_or(0.0);
                let supply = out_rate.min(cap);
                supplies.insert(route.endpoints.1.clone(), supply);
                route_supply.insert(rid.clone(), supply);
            }

            derived.total_power_mw += result.total_power_mw;
            derived
                .factories
                .insert(fid.clone(), to_derived(&result, solve_on_release));
        }

        // Route flows (= downstream intake), deficits, manifests.
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
            // Deficit: the downstream factory's own target is clamped by this port.
            if let (Some(fid), Some(df)) = (
                dst_factory.clone(),
                dst_factory.as_ref().and_then(|f| derived.factories.get(f)),
            ) {
                if let Some(ceiling) = &df.target_ceiling {
                    if let solver::model::Constraint::InputCeiling { port, item, .. } =
                        &ceiling.binding
                    {
                        if port == dst_port {
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
                            if requested > ceiling.max_rate + 1e-6 && ceiling.max_rate > 0.0 {
                                let needed = flow * requested / ceiling.max_rate;
                                derived.deficits.push(DeficitRow {
                                    factory: fid,
                                    port: dst_port.clone(),
                                    route: Some(r.id.clone()),
                                    item: item.clone(),
                                    needed,
                                    supplied,
                                });
                            }
                        }
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

        // Node claim conflicts (§3.1.3 — representable, rendered CRIT, never prevented).
        let mut by_node: BTreeMap<String, u32> = BTreeMap::new();
        for c in self.state.node_claims.values() {
            *by_node.entry(c.node.clone()).or_insert(0) += 1;
        }
        for (node, claims) in by_node {
            derived.nodes.insert(
                node,
                DerivedNode {
                    claims,
                    conflict: claims > 1,
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
            derived.total_generation_mw = self.state.factories.keys().map(gen_of).sum();
        }
        derived.recompute_us = started.elapsed().as_micros() as u64;
        derived
    }

    fn error_factory(message: &str) -> DerivedFactory {
        DerivedFactory {
            groups: BTreeMap::new(),
            edges: BTreeMap::new(),
            ports: BTreeMap::new(),
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

fn item_or(manifest: &[(String, f64)], src_port: &Id, state: &PlanState) -> String {
    manifest
        .first()
        .map(|(i, _)| i.clone())
        .or_else(|| state.ports.get(src_port).map(|p| p.item.clone()))
        .unwrap_or_default()
}
