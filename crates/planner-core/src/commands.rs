//! The command layer — every mutation flows through here (SDD §4 `plan.edit(op)`).
//! Commands validate invariants (§3.1), mutate canonical state, and record
//! forward/inverse ops into a `Transaction`. Solve-induced writes are recorded
//! into the *same* transaction by the app layer before commit, so ⌘Z undoes the
//! edit and its solve together.

use serde::{Deserialize, Serialize};

use crate::entities::*;
use crate::patch::{PatchBatch, PatchOp};
use crate::state::*;

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DomainError {
    #[error("entity not found: {id}")]
    NotFound { id: Id },
    #[error("built entities are immutable: {id} ({action})")]
    BuiltImmutable { id: Id, action: String },
    #[error("invalid value: {message}")]
    Invalid { message: String },
}

/// An open transaction: ops applied to canonical state but not yet committed
/// to the undo log. The app layer may append solve results before committing.
#[derive(Debug, Clone, Default)]
pub struct Transaction {
    pub label: String,
    pub forward: PatchBatch,
    pub inverse: PatchBatch,
    /// Ids created by this transaction, in creation order (renderer selects them).
    pub created: Vec<Id>,
}

impl Transaction {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Default::default()
        }
    }

    pub fn record(&mut self, (forward, inverse): (PatchOp, PatchOp)) {
        self.forward.push(forward);
        // Inverse ops must apply in reverse order; store reversed at commit time.
        self.inverse.push(inverse);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Command {
    CreateFactory {
        name: String,
        position: MapPos,
        region: String,
    },
    RenameFactory {
        id: Id,
        name: String,
    },
    MoveFactoryPin {
        id: Id,
        position: MapPos,
    },
    DeleteFactory {
        id: Id,
    },
    AddGroup {
        factory: Id,
        machine: String,
        recipe: String,
        count: u32,
        clock: f64,
        graph_pos: GraphPos,
    },
    SetGroupRecipe {
        id: Id,
        machine: String,
        recipe: String,
    },
    SetGroupCount {
        id: Id,
        count: u32,
    },
    SetGroupClock {
        id: Id,
        clock: f64,
    },
    MoveGroupCard {
        id: Id,
        graph_pos: GraphPos,
    },
    DeleteGroup {
        id: Id,
    },
    AddPort {
        factory: Id,
        direction: PortDirection,
        item: String,
        rate: f64,
        rate_ceiling: Option<f64>,
        graph_pos: GraphPos,
    },
    SetPortRate {
        id: Id,
        rate: f64,
    },
    SetPortCeiling {
        id: Id,
        rate_ceiling: Option<f64>,
    },
    MovePortCard {
        id: Id,
        graph_pos: GraphPos,
    },
    DeletePort {
        id: Id,
    },
    AddEdge {
        factory: Id,
        from: EdgeEnd,
        to: EdgeEnd,
        item: String,
        tier: u8,
    },
    SetEdgeTier {
        id: Id,
        tier: u8,
    },
    DeleteEdge {
        id: Id,
    },
    ClaimNode {
        factory: Id,
        node: String,
        extractor: String,
        clock: f64,
    },
    ReleaseNode {
        id: Id,
    },
    RenamePlan {
        name: String,
    },
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Command::CreateFactory { .. } => "create factory",
            Command::RenameFactory { .. } => "rename factory",
            Command::MoveFactoryPin { .. } => "move factory",
            Command::DeleteFactory { .. } => "delete factory",
            Command::AddGroup { .. } => "add machine group",
            Command::SetGroupRecipe { .. } => "set recipe",
            Command::SetGroupCount { .. } => "set count",
            Command::SetGroupClock { .. } => "set clock",
            Command::MoveGroupCard { .. } => "move card",
            Command::DeleteGroup { .. } => "delete group",
            Command::AddPort { .. } => "add port",
            Command::SetPortRate { .. } => "set target rate",
            Command::SetPortCeiling { .. } => "set input ceiling",
            Command::MovePortCard { .. } => "move port",
            Command::DeletePort { .. } => "delete port",
            Command::AddEdge { .. } => "connect belt",
            Command::SetEdgeTier { .. } => "set belt tier",
            Command::DeleteEdge { .. } => "delete belt",
            Command::ClaimNode { .. } => "claim node",
            Command::ReleaseNode { .. } => "release node",
            Command::RenamePlan { .. } => "rename plan",
        }
    }
}

fn require_planned(status: Status, id: &Id, action: &str) -> Result<(), DomainError> {
    // Phase 1 creates Planned entities only; Built immutability (§3.1.1) is
    // enforced now so it can never regress when import lands.
    if status == Status::Built {
        return Err(DomainError::BuiltImmutable {
            id: id.clone(),
            action: action.into(),
        });
    }
    Ok(())
}

fn clamp_clock(clock: f64) -> Result<f64, DomainError> {
    if !(0.01..=2.5).contains(&clock) {
        return Err(DomainError::Invalid {
            message: format!("clock {clock} outside 1%–250%"),
        });
    }
    Ok(clock)
}

fn valid_tier(tier: u8) -> Result<u8, DomainError> {
    if !(1..=6).contains(&tier) {
        return Err(DomainError::Invalid {
            message: format!("belt tier {tier} outside Mk.1–Mk.6"),
        });
    }
    Ok(tier)
}

/// Apply a command to canonical state. Returns an open `Transaction`.
pub fn apply(state: &mut PlanState, cmd: &Command) -> Result<Transaction, DomainError> {
    let mut tx = Transaction::new(cmd.label());
    match cmd {
        Command::CreateFactory {
            name,
            position,
            region,
        } => {
            let f = Factory {
                id: new_id(),
                name: name.clone(),
                position: *position,
                region: region.clone(),
                node_claims: vec![],
                groups: vec![],
                ports: vec![],
                style_guide: None,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(f.id.clone());
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::RenameFactory { id, name } => {
            let mut f = state
                .factories
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            f.name = name.clone();
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::MoveFactoryPin { id, position } => {
            let mut f = state
                .factories
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            // Built pins are locked (UI offers "plan a move" — later phase).
            require_planned(f.status, id, "move")?;
            f.position = *position;
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::DeleteFactory { id } => {
            let f = state
                .factories
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(f.status, id, "delete")?;
            // Cascade: groups, ports, edges, claims belonging to this factory.
            let group_ids: Vec<Id> = state
                .groups
                .values()
                .filter(|g| &g.factory == id)
                .map(|g| g.id.clone())
                .collect();
            let port_ids: Vec<Id> = state
                .ports
                .values()
                .filter(|p| &p.factory == id)
                .map(|p| p.id.clone())
                .collect();
            let edge_ids: Vec<Id> = state
                .edges
                .values()
                .filter(|e| &e.factory == id)
                .map(|e| e.id.clone())
                .collect();
            let claim_ids: Vec<Id> = state
                .node_claims
                .values()
                .filter(|c| &c.factory == id)
                .map(|c| c.id.clone())
                .collect();
            for eid in edge_ids {
                if let Some(ops) = state.remove(COLL_EDGES, &eid) {
                    tx.record(ops);
                }
            }
            for gid in group_ids {
                if let Some(ops) = state.remove(COLL_GROUPS, &gid) {
                    tx.record(ops);
                }
            }
            for pid in port_ids {
                if let Some(ops) = state.remove(COLL_PORTS, &pid) {
                    tx.record(ops);
                }
            }
            for cid in claim_ids {
                if let Some(ops) = state.remove(COLL_NODE_CLAIMS, &cid) {
                    tx.record(ops);
                }
            }
            if let Some(ops) = state.remove(COLL_FACTORIES, id) {
                tx.record(ops);
            }
        }
        Command::AddGroup {
            factory,
            machine,
            recipe,
            count,
            clock,
            graph_pos,
        } => {
            let mut f = state
                .factories
                .get(factory)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: factory.clone(),
                })?;
            let g = MachineGroup {
                id: new_id(),
                factory: factory.clone(),
                machine: machine.clone(),
                recipe: recipe.clone(),
                count: (*count).max(1),
                clock: clamp_clock(*clock)?,
                somersloops: 0,
                planned_delta: None,
                graph_pos: *graph_pos,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(g.id.clone());
            f.groups.push(g.id.clone());
            tx.record(state.upsert(Entity::Group(g)));
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::SetGroupRecipe {
            id,
            machine,
            recipe,
        } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(g.status, id, "set recipe")?;
            g.machine = machine.clone();
            g.recipe = recipe.clone();
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::SetGroupCount { id, count } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(g.status, id, "set count")?;
            g.count = (*count).max(1);
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::SetGroupClock { id, clock } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(g.status, id, "set clock")?;
            g.clock = clamp_clock(*clock)?;
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::MoveGroupCard { id, graph_pos } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            g.graph_pos = *graph_pos;
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::DeleteGroup { id } => {
            let g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(g.status, id, "delete")?;
            let edge_ids: Vec<Id> = state
                .edges
                .values()
                .filter(|e| {
                    e.from == EdgeEnd::Group(id.clone()) || e.to == EdgeEnd::Group(id.clone())
                })
                .map(|e| e.id.clone())
                .collect();
            for eid in edge_ids {
                if let Some(ops) = state.remove(COLL_EDGES, &eid) {
                    tx.record(ops);
                }
            }
            if let Some(mut f) = state.factories.get(&g.factory).cloned() {
                f.groups.retain(|gid| gid != id);
                tx.record(state.upsert(Entity::Factory(f)));
            }
            if let Some(ops) = state.remove(COLL_GROUPS, id) {
                tx.record(ops);
            }
        }
        Command::AddPort {
            factory,
            direction,
            item,
            rate,
            rate_ceiling,
            graph_pos,
        } => {
            let mut f = state
                .factories
                .get(factory)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: factory.clone(),
                })?;
            let p = Port {
                id: new_id(),
                factory: factory.clone(),
                direction: *direction,
                item: item.clone(),
                rate: rate.max(0.0),
                rate_ceiling: *rate_ceiling,
                bound_route: None,
                graph_pos: *graph_pos,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(p.id.clone());
            f.ports.push(p.id.clone());
            tx.record(state.upsert(Entity::Port(p)));
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::SetPortRate { id, rate } => {
            let mut p = state
                .ports
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            if *rate < 0.0 {
                return Err(DomainError::Invalid {
                    message: "rate must be ≥ 0".into(),
                });
            }
            p.rate = *rate;
            tx.record(state.upsert(Entity::Port(p)));
        }
        Command::SetPortCeiling { id, rate_ceiling } => {
            let mut p = state
                .ports
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            p.rate_ceiling = *rate_ceiling;
            tx.record(state.upsert(Entity::Port(p)));
        }
        Command::MovePortCard { id, graph_pos } => {
            let mut p = state
                .ports
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            p.graph_pos = *graph_pos;
            tx.record(state.upsert(Entity::Port(p)));
        }
        Command::DeletePort { id } => {
            let p = state
                .ports
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(p.status, id, "delete")?;
            let edge_ids: Vec<Id> = state
                .edges
                .values()
                .filter(|e| {
                    e.from == EdgeEnd::Port(id.clone()) || e.to == EdgeEnd::Port(id.clone())
                })
                .map(|e| e.id.clone())
                .collect();
            for eid in edge_ids {
                if let Some(ops) = state.remove(COLL_EDGES, &eid) {
                    tx.record(ops);
                }
            }
            if let Some(mut f) = state.factories.get(&p.factory).cloned() {
                f.ports.retain(|pid| pid != id);
                tx.record(state.upsert(Entity::Factory(f)));
            }
            if let Some(ops) = state.remove(COLL_PORTS, id) {
                tx.record(ops);
            }
        }
        Command::AddEdge {
            factory,
            from,
            to,
            item,
            tier,
        } => {
            state.factories.get(factory).ok_or(DomainError::NotFound {
                id: factory.clone(),
            })?;
            let e = BeltEdge {
                id: new_id(),
                factory: factory.clone(),
                from: from.clone(),
                to: to.clone(),
                item: item.clone(),
                tier: valid_tier(*tier)?,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(e.id.clone());
            tx.record(state.upsert(Entity::Edge(e)));
        }
        Command::SetEdgeTier { id, tier } => {
            let mut e = state
                .edges
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            e.tier = valid_tier(*tier)?;
            tx.record(state.upsert(Entity::Edge(e)));
        }
        Command::DeleteEdge { id } => {
            let e = state
                .edges
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(e.status, id, "delete")?;
            if let Some(ops) = state.remove(COLL_EDGES, id) {
                tx.record(ops);
            }
        }
        Command::ClaimNode {
            factory,
            node,
            extractor,
            clock,
        } => {
            let mut f = state
                .factories
                .get(factory)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: factory.clone(),
                })?;
            // Note §3.1.3: conflicting claims are representable, never prevented.
            let c = NodeClaim {
                id: new_id(),
                node: node.clone(),
                factory: factory.clone(),
                extractor: extractor.clone(),
                clock: clamp_clock(*clock)?,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(c.id.clone());
            f.node_claims.push(c.id.clone());
            tx.record(state.upsert(Entity::NodeClaim(c)));
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::ReleaseNode { id } => {
            let c = state
                .node_claims
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            if let Some(mut f) = state.factories.get(&c.factory).cloned() {
                f.node_claims.retain(|cid| cid != id);
                tx.record(state.upsert(Entity::Factory(f)));
            }
            if let Some(ops) = state.remove(COLL_NODE_CLAIMS, id) {
                tx.record(ops);
            }
        }
        Command::RenamePlan { name } => {
            let old = state.meta.name.clone();
            state.meta.name = name.clone();
            tx.forward.push(PatchOp::Replace {
                path: "/meta/name".into(),
                value: serde_json::json!(name),
            });
            tx.inverse.push(PatchOp::Replace {
                path: "/meta/name".into(),
                value: serde_json::json!(old),
            });
        }
    }
    Ok(tx)
}
