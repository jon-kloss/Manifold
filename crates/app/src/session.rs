//! One open plan file + its canonical state, undo log, gamedata, and solver
//! orchestration. Every mutation: apply commands → T1 re-solve → fold solve
//! write-backs into the same transaction → commit (one undo entry) → persist.

use std::collections::BTreeMap;
use std::path::Path;

use planner_core::commands::{self, Command, DomainError, Transaction};
use planner_core::entities::*;
use planner_core::patch::PatchBatch;
use planner_core::state::{Entity, PlanState};
use planner_core::undo::UndoLog;

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

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Derived {
    pub factories: BTreeMap<String, DerivedFactory>,
    pub nodes: BTreeMap<String, DerivedNode>,
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
}

pub struct Session {
    pub state: PlanState,
    pub undo: UndoLog,
    pub file: PlanFile,
    pub gamedata: GameData,
    pub world: WorldSnapshot,
    /// Consecutive over-budget T1 solves per factory (A4 miss behavior).
    slow_solves: BTreeMap<Id, u32>,
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
        Ok(Self {
            state,
            undo,
            file,
            gamedata: gd,
            world,
            slow_solves: BTreeMap::new(),
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
                "buildVersion": self.gamedata.build_version,
            },
            "world": self.world,
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

        // T1 re-solve every factory touched by the edit; fold write-backs in.
        let trigger = Self::solve_trigger(&cmds);
        let touched = self.touched_factories(&cmds);
        for fid in &touched {
            self.solve_factory_into_tx(fid, &trigger, &mut tx);
        }

        let created = tx.created.clone();
        let entry = self.undo.commit(tx);
        self.file
            .commit(&entry, &self.state.meta, self.undo.entries().len())?;

        Ok(EditResponse {
            patches: entry.forward,
            derived: self.solve_all_readonly(),
            can_undo: self.undo.can_undo(),
            can_redo: self.undo.can_redo(),
            undo_label: self.undo.undo_label().map(String::from),
            created,
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

    fn touched_factories(&self, cmds: &[Command]) -> Vec<Id> {
        let mut out: Vec<Id> = Vec::new();
        let mut push = |id: Option<Id>| {
            if let Some(id) = id {
                if !out.contains(&id) {
                    out.push(id);
                }
            }
        };
        for cmd in cmds {
            match cmd {
                Command::AddGroup { factory, .. }
                | Command::AddPort { factory, .. }
                | Command::AddEdge { factory, .. }
                | Command::ClaimNode { factory, .. } => push(Some(factory.clone())),
                Command::SetGroupRecipe { id, .. }
                | Command::SetGroupCount { id, .. }
                | Command::SetGroupClock { id, .. }
                | Command::DeleteGroup { id } => {
                    push(self.state.groups.get(id).map(|g| g.factory.clone()))
                }
                Command::SetPortRate { id, .. }
                | Command::SetPortCeiling { id, .. }
                | Command::DeletePort { id } => {
                    push(self.state.ports.get(id).map(|p| p.factory.clone()))
                }
                Command::SetEdgeTier { id, .. } | Command::DeleteEdge { id } => {
                    push(self.state.edges.get(id).map(|e| e.factory.clone()))
                }
                Command::ReleaseNode { id } => {
                    push(self.state.node_claims.get(id).map(|c| c.factory.clone()))
                }
                _ => {}
            }
        }
        out
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
        Some(FactorySnapshot {
            groups,
            edges,
            inputs,
            outputs,
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

    /// T1 solve one factory; fold count/clock write-backs into the open tx.
    fn solve_factory_into_tx(&mut self, fid: &Id, trigger: &T0Edit, tx: &mut Transaction) {
        let Some(snapshot) = self.snapshot(fid) else {
            return;
        };
        if snapshot.groups.is_empty() {
            return;
        }
        let trigger = self.trigger_for_factory(fid, &snapshot, trigger);
        let Ok(result) = solver::t1::solve(&snapshot, &trigger) else {
            return;
        };
        // Write back solver-owned numbers (counts/clocks) — same undo entry.
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
        // Honest clamp: if the committed target exceeded the ceiling, the port
        // rate settles at the ceiling (slider hard-stop made visible).
        if result.clamped {
            if let T0Edit::SetTarget { port, .. } = &trigger {
                if let (Some(p), Some(rate)) = (self.state.ports.get(port), result.ports.get(port))
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
        // A4 miss tracking.
        let ms = result.solve_us as f64 / 1000.0;
        let slow = self.slow_solves.entry(fid.clone()).or_insert(0);
        if ms > T1_BUDGET_MS {
            *slow += 1;
        } else {
            *slow = 0;
        }
    }

    /// If the factory has a single output port, always solve as SetTarget on it
    /// so the ceiling/binding is available for the slider tick.
    fn trigger_for_factory(
        &self,
        _fid: &Id,
        snapshot: &FactorySnapshot,
        trigger: &T0Edit,
    ) -> T0Edit {
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

    /// Recompute derived state for everything, without touching canonical state.
    pub fn solve_all_readonly(&mut self) -> Derived {
        let mut derived = Derived::default();
        let fids: Vec<Id> = self.state.factories.keys().cloned().collect();
        for fid in fids {
            let Some(snapshot) = self.snapshot(&fid) else {
                derived.factories.insert(
                    fid,
                    DerivedFactory {
                        groups: BTreeMap::new(),
                        edges: BTreeMap::new(),
                        ports: BTreeMap::new(),
                        total_power_mw: 0.0,
                        target_ceiling: None,
                        solve_us: 0,
                        solve_on_release: false,
                        solve_error: Some("missing recipe or machine data".into()),
                    },
                );
                continue;
            };
            let trigger = if snapshot.outputs.len() == 1 {
                T0Edit::SetTarget {
                    port: snapshot.outputs[0].id.clone(),
                    rate: snapshot.outputs[0].rate,
                }
            } else {
                T0Edit::Recompute
            };
            let solve_on_release = self.slow_solves.get(&fid).copied().unwrap_or(0) >= 3;
            match solver::t1::solve(&snapshot, &trigger) {
                Ok(r) => {
                    derived.total_power_mw += r.total_power_mw;
                    derived
                        .factories
                        .insert(fid, to_derived(&r, solve_on_release));
                }
                Err(e) => {
                    derived.factories.insert(
                        fid,
                        DerivedFactory {
                            groups: BTreeMap::new(),
                            edges: BTreeMap::new(),
                            ports: BTreeMap::new(),
                            total_power_mw: 0.0,
                            target_ceiling: None,
                            solve_us: 0,
                            solve_on_release,
                            solve_error: Some(e.to_string()),
                        },
                    );
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
        derived
    }
}

fn to_node_ref(end: &EdgeEnd, state: &PlanState) -> NodeRef {
    match end {
        EdgeEnd::Group(id) => NodeRef::Group(id.clone()),
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
