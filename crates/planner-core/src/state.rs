//! Canonical plan state. Lives in Rust only; the renderer holds a JSON projection
//! of exactly this shape, hydrated at load and patched by `state://patch` events.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::entities::*;
use crate::patch::{PatchBatch, PatchOp};
use crate::proposals::Proposal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanMeta {
    pub schema_version: u32,
    pub game_build: String,
    pub name: String,
    /// Plan-scoped NEXT-MOVES preferences (PR 3). Advisory filters that steer
    /// the opportunity engine and the model prompt — they hide *suggestions*,
    /// never *facts* (a `power_deficit` is demoted-and-noted, never removed).
    /// serde-default so plan files predating PR 3 load unchanged, and excluded
    /// from `plan_hash` (a filter toggle is not plan geometry, so it must not
    /// stale open proposals or trip the per-edit merge).
    #[serde(default)]
    pub preferences: NextPreferences,
}

impl Default for PlanMeta {
    fn default() -> Self {
        Self {
            schema_version: 1,
            game_build: String::new(),
            name: "NEW WORLD".into(),
            preferences: NextPreferences::default(),
        }
    }
}

/// NEXT-MOVES preferences (PR 3) — plan-scoped, persisted with the plan meta.
/// Deliberately small and extensible: named after the user's own examples,
/// serde-default so every field (and the whole struct) tolerates absence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NextPreferences {
    /// The player does not want rail/consist suggestions.
    pub no_trains: bool,
    /// Deprioritize power topics: advisory power cards hide, the overdraw FACT
    /// only demotes-and-notes (never disappears).
    pub ignore_power: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanState {
    pub meta: PlanMeta,
    pub factories: BTreeMap<Id, Factory>,
    pub groups: BTreeMap<Id, MachineGroup>,
    pub ports: BTreeMap<Id, Port>,
    pub edges: BTreeMap<Id, BeltEdge>,
    pub node_claims: BTreeMap<Id, NodeClaim>,
    pub routes: BTreeMap<Id, Route>,
    #[serde(default)]
    pub junctions: BTreeMap<Id, Junction>,
    #[serde(default)]
    pub proposals: BTreeMap<Id, Proposal>,
    #[serde(default)]
    pub switches: BTreeMap<Id, PrioritySwitch>,
    #[serde(default)]
    pub style_guides: BTreeMap<Id, StyleGuide>,
    /// Manual build-queue completion overrides (W1c) — sparse assertion overlay
    /// keyed by step id, auto-dissolving on re-import. serde-default so plan
    /// files predating W1c load unchanged (no migration).
    #[serde(default)]
    pub build_overrides: BTreeMap<Id, BuildOverride>,
    /// Plan-local resource-node corrections (W2b-C) — sparse overlay keyed by
    /// node id (`"<snapshot id>"` / `"save:<id>"`). The bundled catalog stays an
    /// ambient default; a node's resolved geometry is `snapshot ⊕ override`.
    /// Auto-dissolves on re-import once the save agrees with the snapshot again.
    /// serde-default so plan files predating W2b-C load unchanged (no migration).
    #[serde(default)]
    pub node_overrides: BTreeMap<String, NodeOverride>,
}

/// Collection names as they appear in patch paths and the projected store.
pub const COLL_FACTORIES: &str = "factories";
pub const COLL_GROUPS: &str = "groups";
pub const COLL_PORTS: &str = "ports";
pub const COLL_EDGES: &str = "edges";
pub const COLL_NODE_CLAIMS: &str = "nodeClaims";
pub const COLL_ROUTES: &str = "routes";
pub const COLL_JUNCTIONS: &str = "junctions";
pub const COLL_PROPOSALS: &str = "proposals";
pub const COLL_SWITCHES: &str = "switches";
pub const COLL_STYLE_GUIDES: &str = "styleGuides";
pub const COLL_BUILD_OVERRIDES: &str = "buildOverrides";
pub const COLL_NODE_OVERRIDES: &str = "nodeOverrides";

#[derive(Debug, Clone, PartialEq)]
pub enum Entity {
    Factory(Factory),
    Group(MachineGroup),
    Port(Port),
    Edge(BeltEdge),
    NodeClaim(NodeClaim),
    Route(Route),
    Junction(Junction),
    Proposal(Proposal),
    Switch(PrioritySwitch),
    StyleGuide(StyleGuide),
    BuildOverride(BuildOverride),
    NodeOverride(NodeOverride),
}

impl Entity {
    pub fn id(&self) -> &str {
        match self {
            Entity::Factory(e) => &e.id,
            Entity::Group(e) => &e.id,
            Entity::Port(e) => &e.id,
            Entity::Edge(e) => &e.id,
            Entity::NodeClaim(e) => &e.id,
            Entity::Route(e) => &e.id,
            Entity::Junction(e) => &e.id,
            Entity::Proposal(e) => &e.id,
            Entity::Switch(e) => &e.id,
            Entity::StyleGuide(e) => &e.id,
            Entity::BuildOverride(e) => &e.id,
            Entity::NodeOverride(e) => &e.id,
        }
    }

    pub fn collection(&self) -> &'static str {
        match self {
            Entity::Factory(_) => COLL_FACTORIES,
            Entity::Group(_) => COLL_GROUPS,
            Entity::Port(_) => COLL_PORTS,
            Entity::Edge(_) => COLL_EDGES,
            Entity::NodeClaim(_) => COLL_NODE_CLAIMS,
            Entity::Route(_) => COLL_ROUTES,
            Entity::Junction(_) => COLL_JUNCTIONS,
            Entity::Proposal(_) => COLL_PROPOSALS,
            Entity::Switch(_) => COLL_SWITCHES,
            Entity::StyleGuide(_) => COLL_STYLE_GUIDES,
            Entity::BuildOverride(_) => COLL_BUILD_OVERRIDES,
            Entity::NodeOverride(_) => COLL_NODE_OVERRIDES,
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Entity::Factory(e) => serde_json::to_value(e).unwrap(),
            Entity::Group(e) => serde_json::to_value(e).unwrap(),
            Entity::Port(e) => serde_json::to_value(e).unwrap(),
            Entity::Edge(e) => serde_json::to_value(e).unwrap(),
            Entity::NodeClaim(e) => serde_json::to_value(e).unwrap(),
            Entity::Route(e) => serde_json::to_value(e).unwrap(),
            Entity::Junction(e) => serde_json::to_value(e).unwrap(),
            Entity::Proposal(e) => serde_json::to_value(e).unwrap(),
            Entity::Switch(e) => serde_json::to_value(e).unwrap(),
            Entity::StyleGuide(e) => serde_json::to_value(e).unwrap(),
            Entity::BuildOverride(e) => serde_json::to_value(e).unwrap(),
            Entity::NodeOverride(e) => serde_json::to_value(e).unwrap(),
        }
    }

    pub fn from_value(collection: &str, value: &Value) -> Result<Entity, String> {
        let err = |e: serde_json::Error| e.to_string();
        Ok(match collection {
            COLL_FACTORIES => Entity::Factory(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_GROUPS => Entity::Group(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_PORTS => Entity::Port(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_EDGES => Entity::Edge(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_NODE_CLAIMS => {
                Entity::NodeClaim(serde_json::from_value(value.clone()).map_err(err)?)
            }
            COLL_ROUTES => Entity::Route(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_JUNCTIONS => Entity::Junction(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_PROPOSALS => Entity::Proposal(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_SWITCHES => Entity::Switch(serde_json::from_value(value.clone()).map_err(err)?),
            COLL_STYLE_GUIDES => {
                Entity::StyleGuide(serde_json::from_value(value.clone()).map_err(err)?)
            }
            COLL_BUILD_OVERRIDES => {
                Entity::BuildOverride(serde_json::from_value(value.clone()).map_err(err)?)
            }
            COLL_NODE_OVERRIDES => {
                Entity::NodeOverride(serde_json::from_value(value.clone()).map_err(err)?)
            }
            other => return Err(format!("unknown collection {other}")),
        })
    }
}

impl PlanState {
    /// Full JSON projection — what the renderer hydrates at load.
    pub fn project(&self) -> Value {
        json!({
            "meta": self.meta,
            COLL_FACTORIES: self.factories,
            COLL_GROUPS: self.groups,
            COLL_PORTS: self.ports,
            COLL_EDGES: self.edges,
            COLL_NODE_CLAIMS: self.node_claims,
            COLL_ROUTES: self.routes,
            COLL_JUNCTIONS: self.junctions,
            COLL_PROPOSALS: self.proposals,
            COLL_SWITCHES: self.switches,
            COLL_STYLE_GUIDES: self.style_guides,
            COLL_BUILD_OVERRIDES: self.build_overrides,
            COLL_NODE_OVERRIDES: self.node_overrides,
        })
    }

    pub fn get(&self, collection: &str, id: &str) -> Option<Entity> {
        match collection {
            COLL_FACTORIES => self.factories.get(id).cloned().map(Entity::Factory),
            COLL_GROUPS => self.groups.get(id).cloned().map(Entity::Group),
            COLL_PORTS => self.ports.get(id).cloned().map(Entity::Port),
            COLL_EDGES => self.edges.get(id).cloned().map(Entity::Edge),
            COLL_NODE_CLAIMS => self.node_claims.get(id).cloned().map(Entity::NodeClaim),
            COLL_ROUTES => self.routes.get(id).cloned().map(Entity::Route),
            COLL_JUNCTIONS => self.junctions.get(id).cloned().map(Entity::Junction),
            COLL_PROPOSALS => self.proposals.get(id).cloned().map(Entity::Proposal),
            COLL_SWITCHES => self.switches.get(id).cloned().map(Entity::Switch),
            COLL_STYLE_GUIDES => self.style_guides.get(id).cloned().map(Entity::StyleGuide),
            COLL_BUILD_OVERRIDES => self
                .build_overrides
                .get(id)
                .cloned()
                .map(Entity::BuildOverride),
            COLL_NODE_OVERRIDES => self
                .node_overrides
                .get(id)
                .cloned()
                .map(Entity::NodeOverride),
            _ => None,
        }
    }

    fn insert(&mut self, e: Entity) {
        match e {
            Entity::Factory(v) => {
                self.factories.insert(v.id.clone(), v);
            }
            Entity::Group(v) => {
                self.groups.insert(v.id.clone(), v);
            }
            Entity::Port(v) => {
                self.ports.insert(v.id.clone(), v);
            }
            Entity::Edge(v) => {
                self.edges.insert(v.id.clone(), v);
            }
            Entity::NodeClaim(v) => {
                self.node_claims.insert(v.id.clone(), v);
            }
            Entity::Route(v) => {
                self.routes.insert(v.id.clone(), v);
            }
            Entity::Junction(v) => {
                self.junctions.insert(v.id.clone(), v);
            }
            Entity::Proposal(v) => {
                self.proposals.insert(v.id.clone(), v);
            }
            Entity::Switch(v) => {
                self.switches.insert(v.id.clone(), v);
            }
            Entity::StyleGuide(v) => {
                self.style_guides.insert(v.id.clone(), v);
            }
            Entity::BuildOverride(v) => {
                self.build_overrides.insert(v.id.clone(), v);
            }
            Entity::NodeOverride(v) => {
                self.node_overrides.insert(v.id.clone(), v);
            }
        }
    }

    fn delete(&mut self, collection: &str, id: &str) {
        match collection {
            COLL_FACTORIES => {
                self.factories.remove(id);
            }
            COLL_GROUPS => {
                self.groups.remove(id);
            }
            COLL_PORTS => {
                self.ports.remove(id);
            }
            COLL_EDGES => {
                self.edges.remove(id);
            }
            COLL_NODE_CLAIMS => {
                self.node_claims.remove(id);
            }
            COLL_ROUTES => {
                self.routes.remove(id);
            }
            COLL_JUNCTIONS => {
                self.junctions.remove(id);
            }
            COLL_PROPOSALS => {
                self.proposals.remove(id);
            }
            COLL_SWITCHES => {
                self.switches.remove(id);
            }
            COLL_STYLE_GUIDES => {
                self.style_guides.remove(id);
            }
            COLL_BUILD_OVERRIDES => {
                self.build_overrides.remove(id);
            }
            COLL_NODE_OVERRIDES => {
                self.node_overrides.remove(id);
            }
            _ => {}
        }
    }

    /// Upsert an entity, returning (forward, inverse) ops.
    pub fn upsert(&mut self, e: Entity) -> (PatchOp, PatchOp) {
        let path = format!("/{}/{}", e.collection(), e.id());
        let prev = self.get(e.collection(), e.id());
        let forward = match prev {
            Some(_) => PatchOp::Replace {
                path: path.clone(),
                value: e.to_value(),
            },
            None => PatchOp::Add {
                path: path.clone(),
                value: e.to_value(),
            },
        };
        let inverse = match &prev {
            Some(old) => PatchOp::Replace {
                path: path.clone(),
                value: old.to_value(),
            },
            None => PatchOp::Remove { path },
        };
        self.insert(e);
        (forward, inverse)
    }

    /// Remove an entity, returning (forward, inverse) ops. No-op if absent.
    pub fn remove(&mut self, collection: &str, id: &str) -> Option<(PatchOp, PatchOp)> {
        let old = self.get(collection, id)?;
        let path = format!("/{collection}/{id}");
        self.delete(collection, id);
        Some((
            PatchOp::Remove { path: path.clone() },
            PatchOp::Add {
                path,
                value: old.to_value(),
            },
        ))
    }

    /// Apply a batch of entity-level ops to typed state (undo/redo path).
    pub fn apply_batch(&mut self, batch: &PatchBatch) -> Result<(), String> {
        for op in batch {
            let path = op.path().trim_start_matches('/');
            let (coll, id) = path
                .split_once('/')
                .ok_or_else(|| format!("bad path {path}"))?;
            match op {
                PatchOp::Add { value, .. } | PatchOp::Replace { value, .. } => {
                    if coll == "meta" {
                        self.apply_meta(id, value)?;
                    } else {
                        self.insert(Entity::from_value(coll, value)?);
                    }
                }
                PatchOp::Remove { .. } => self.delete(coll, id),
            }
        }
        Ok(())
    }

    fn apply_meta(&mut self, field: &str, value: &Value) -> Result<(), String> {
        match field {
            "name" => self.meta.name = value.as_str().unwrap_or_default().to_string(),
            "gameBuild" => self.meta.game_build = value.as_str().unwrap_or_default().to_string(),
            other => return Err(format!("unknown meta field {other}")),
        }
        Ok(())
    }
}
