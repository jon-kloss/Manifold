//! The command layer — every mutation flows through here (SDD §4 `plan.edit(op)`).
//! Commands validate invariants (§3.1), mutate canonical state, and record
//! forward/inverse ops into a `Transaction`. Solve-induced writes are recorded
//! into the *same* transaction by the app layer before commit, so ⌘Z undoes the
//! edit and its solve together.

use serde::{Deserialize, Serialize};

use crate::entities::*;
use crate::patch::{PatchBatch, PatchOp};
use crate::proposals::{Proposal, ProposalStatus};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        #[serde(default)]
        floor: u32,
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
    SetGroupFloor {
        id: Id,
        floor: u32,
    },
    MoveGroupCard {
        id: Id,
        graph_pos: GraphPos,
    },
    /// Recompute every card position in a factory via the layered auto-layout
    /// (In ports → ranked groups/junctions → Out ports). Cosmetic: applies to
    /// built cards too, one undoable step.
    TidyLayout {
        factory: Id,
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
    AddJunction {
        factory: Id,
        kind: JunctionKind,
        graph_pos: GraphPos,
        #[serde(default)]
        floor: u32,
    },
    MoveJunctionCard {
        id: Id,
        graph_pos: GraphPos,
    },
    SetJunctionFloor {
        id: Id,
        floor: u32,
    },
    DeleteJunction {
        id: Id,
    },
    /// Bind an Out port of one factory to an In port of another with a map
    /// route. Phase 2 kinds: Belt (items) and Power (endpoints are factories).
    AddRoute {
        kind: RouteKind,
        from: Id,
        to: Id,
        path: Vec<MapPos>,
    },
    SetRouteTier {
        id: Id,
        tier: u8,
    },
    /// Swap a cargo route's kind/spec (belt↔rail↔truck↔drone, or edit the
    /// consists/cars/dwell/headway of the current kind).
    SetRouteSpec {
        id: Id,
        kind: RouteKind,
    },
    DeleteRoute {
        id: Id,
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
    /// Store a solver-drafted proposal (Draft). Nothing else changes.
    CreateProposal {
        proposal: Proposal,
    },
    /// Check/uncheck a review row. Excluding cascades to dependents;
    /// including requires (and pulls in) everything it depends on.
    ToggleProposalItem {
        proposal: Id,
        item: Id,
        included: bool,
    },
    SetProposalStatus {
        id: Id,
        status: ProposalStatus,
    },
    DeleteProposal {
        id: Id,
    },
    /// Place a priority switch on a power line (A2.3). Priority 1–8;
    /// higher numbers shed first.
    AddPrioritySwitch {
        route: Id,
        priority: u8,
    },
    SetSwitchPriority {
        id: Id,
        priority: u8,
    },
    DeleteSwitch {
        id: Id,
    },
    CreateStyleGuide {
        guide: StyleGuide,
    },
    DeleteStyleGuide {
        id: Id,
    },
    SetFactoryTheme {
        factory: Id,
        style_guide: Option<Id>,
    },
    /// Manually assert (or clear) a build-queue step's completion (W1c).
    /// `Some(done)` upserts the override; `None` removes it, reverting the step
    /// to its derived state. Metadata, not a ◆ mutation — no `require_planned`.
    ///
    /// The `id` is NOT validated against a live step: planner-core has no view of
    /// the derived queue/cutover projection (which lives in the `app` crate and
    /// mints synthetic ids like `switch:<fid>:<item>`). An override for an id that
    /// no step carries is an inert sparse overlay — it changes nothing until a
    /// matching step appears, and it auto-dissolves on the next re-import via
    /// `dissolve_stale_overrides`. So validation can't (and needn't) live here.
    /// (The `app` layer — which CAN derive the valid step ids — rejects a bogus
    /// id at the `Session::edit` dispatch before it reaches this arm.)
    SetBuildDone {
        id: Id,
        done: Option<bool>,
    },
    /// Link a ◇ planned factory to the running ◆ factory it replaces (W2a
    /// refactor). `Some` sets the label, `None` clears it. A planner-side label
    /// (same species as RenameFactory) — never a ◆ mutation and never a write to
    /// the referenced entity; the cutover/downtime are DERIVED from it. Undo is
    /// free via the standard upsert patch-pair.
    SetFactoryReplaces {
        id: Id,
        replaces: Option<Id>,
    },
    /// Upsert (or clear) a plan-local resource-node correction (W2b-C).
    /// `Some(ov)` writes the override; `None` removes it, reverting the node to
    /// its ambient catalog geometry. Plan-local metadata, not a ◆ mutation — no
    /// `require_planned` (same species as [`Command::SetBuildDone`]); undo is
    /// free via the standard upsert/remove patch-pair.
    SetNodeOverride {
        id: String,
        node_override: Option<NodeOverride>,
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
            Command::SetGroupFloor { .. } => "set floor",
            Command::MoveGroupCard { .. } => "move card",
            Command::TidyLayout { .. } => "tidy layout",
            Command::DeleteGroup { .. } => "delete group",
            Command::AddPort { .. } => "add port",
            Command::SetPortRate { .. } => "set target rate",
            Command::SetPortCeiling { .. } => "set input ceiling",
            Command::MovePortCard { .. } => "move port",
            Command::DeletePort { .. } => "delete port",
            Command::AddEdge { .. } => "connect belt",
            Command::AddJunction { .. } => "add junction",
            Command::MoveJunctionCard { .. } => "move junction",
            Command::SetJunctionFloor { .. } => "set junction floor",
            Command::DeleteJunction { .. } => "delete junction",
            Command::AddRoute { .. } => "draw route",
            Command::SetRouteTier { .. } => "set route tier",
            Command::SetRouteSpec { .. } => "set route spec",
            Command::DeleteRoute { .. } => "delete route",
            Command::SetEdgeTier { .. } => "set belt tier",
            Command::DeleteEdge { .. } => "delete belt",
            Command::ClaimNode { .. } => "claim node",
            Command::ReleaseNode { .. } => "release node",
            Command::RenamePlan { .. } => "rename plan",
            Command::CreateProposal { .. } => "draft proposal",
            Command::ToggleProposalItem { .. } => "toggle proposal item",
            Command::SetProposalStatus { .. } => "set proposal status",
            Command::DeleteProposal { .. } => "discard proposal",
            Command::AddPrioritySwitch { .. } => "add priority switch",
            Command::SetSwitchPriority { .. } => "set switch priority",
            Command::DeleteSwitch { .. } => "delete switch",
            Command::CreateStyleGuide { .. } => "save style guide",
            Command::DeleteStyleGuide { .. } => "delete style guide",
            Command::SetFactoryTheme { .. } => "set factory theme",
            Command::SetBuildDone { .. } => "mark build done",
            Command::SetFactoryReplaces { .. } => "link replacement",
            Command::SetNodeOverride { .. } => "correct node position",
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

/// Endpoint midpoint of a route path — where a priority switch sits (square
/// pin at the line's midpoint, A2.3). Shared by AddPrioritySwitch and
/// MoveFactoryPin so placement and refresh can never disagree. Empty paths
/// fall back to the origin, exactly as switch placement always has.
fn line_midpoint(path: &[MapPos]) -> MapPos {
    let a = path.first().copied().unwrap_or(MapPos {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    });
    let b = path.last().copied().unwrap_or(a);
    MapPos {
        x: (a.x + b.x) / 2.0,
        y: (a.y + b.y) / 2.0,
        z: (a.z + b.z) / 2.0,
    }
}

/// Resolve an edge endpoint to its owning factory, or fail: `NotFound` for a
/// dangling reference, `Invalid` for an endpoint owned by another factory.
/// Dev-bridge clients and delete/connect races can produce both; either would
/// corrupt the intra-factory graph.
fn require_edge_end(state: &PlanState, end: &EdgeEnd, factory: &Id) -> Result<(), DomainError> {
    let (id, owner) = match end {
        EdgeEnd::Group(gid) => (gid, state.groups.get(gid).map(|g| g.factory.clone())),
        EdgeEnd::Port(pid) => (pid, state.ports.get(pid).map(|p| p.factory.clone())),
        EdgeEnd::Junction(jid) => (jid, state.junctions.get(jid).map(|j| j.factory.clone())),
    };
    let owner = owner.ok_or(DomainError::NotFound { id: id.clone() })?;
    if &owner != factory {
        return Err(DomainError::Invalid {
            message: format!("edge endpoint {id} belongs to a different factory"),
        });
    }
    Ok(())
}

/// Remove a route and everything riding it, recording each op into `tx`:
/// unbind any port still bound to the line, cascade the priority switches
/// sitting on it (switches riding this line go with it), then remove the
/// route. Every removal path (DeleteRoute, DeleteFactory, DeletePort) goes
/// through here so no path can forget the cascade. Deliberately carries no
/// status check — DeleteFactory bypasses `require_planned` for its children.
/// Drop a build-queue override for a step entity that is being removed, so an
/// override can never dangle past the thing it tracked. Recorded into `tx` so
/// undo restores it with the entity. No-op when there is no override.
fn prune_build_override(state: &mut PlanState, tx: &mut Transaction, id: &Id) {
    if let Some(ops) = state.remove(COLL_BUILD_OVERRIDES, id) {
        tx.record(ops);
    }
}

fn remove_route_cascading(state: &mut PlanState, tx: &mut Transaction, route_id: &Id) {
    if let Some(r) = state.routes.get(route_id).cloned() {
        for pid in [&r.endpoints.0, &r.endpoints.1] {
            if let Some(mut p) = state.ports.get(pid).cloned() {
                if p.bound_route.as_deref() == Some(route_id.as_str()) {
                    p.bound_route = None;
                    tx.record(state.upsert(Entity::Port(p)));
                }
            }
        }
    }
    let sw_ids: Vec<Id> = state
        .switches
        .values()
        .filter(|s| &s.route == route_id)
        .map(|s| s.id.clone())
        .collect();
    for sid in sw_ids {
        if let Some(ops) = state.remove(COLL_SWITCHES, &sid) {
            tx.record(ops);
        }
    }
    if let Some(ops) = state.remove(COLL_ROUTES, route_id) {
        tx.record(ops);
    }
    prune_build_override(state, tx, route_id);
}

/// Remove a factory and everything belonging to it, recording each op into
/// `tx`: routes touching its ports (each via `remove_route_cascading`), then
/// edges, groups, ports, node claims, junctions, and finally the factory
/// itself. Every factory-removal path (DeleteFactory, re-import drift sync)
/// goes through here so no path can forget the cascade. Deliberately carries
/// no status check — drift sync removes ◆ Built factories legally (the one
/// documented exception); DeleteFactory checks `require_planned` first.
pub fn remove_factory_cascading(state: &mut PlanState, tx: &mut Transaction, id: &Id) {
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
    let junction_ids: Vec<Id> = state
        .junctions
        .values()
        .filter(|j| &j.factory == id)
        .map(|j| j.id.clone())
        .collect();
    let port_set: std::collections::BTreeSet<Id> = port_ids.iter().cloned().collect();
    let route_ids: Vec<Id> = state
        .routes
        .values()
        .filter(|r| {
            port_set.contains(&r.endpoints.0)
                || port_set.contains(&r.endpoints.1)
                || r.endpoints.0 == *id
                || r.endpoints.1 == *id
        })
        .map(|r| r.id.clone())
        .collect();
    for rid in route_ids {
        remove_route_cascading(state, tx, &rid);
    }
    for eid in edge_ids {
        if let Some(ops) = state.remove(COLL_EDGES, &eid) {
            tx.record(ops);
        }
    }
    for gid in group_ids {
        if let Some(ops) = state.remove(COLL_GROUPS, &gid) {
            tx.record(ops);
        }
        prune_build_override(state, tx, &gid);
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
        prune_build_override(state, tx, &cid);
    }
    for jid in junction_ids {
        if let Some(ops) = state.remove(COLL_JUNCTIONS, &jid) {
            tx.record(ops);
        }
    }
    if let Some(ops) = state.remove(COLL_FACTORIES, id) {
        tx.record(ops);
    }
    prune_build_override(state, tx, id);
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
                replaces: None,
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
            // Deliberately NOT `require_planned` (§3.1.1 exemption): names are
            // planner-side labels, not game ground truth — the save format has
            // no factory-name concept, import *synthesizes* names from the
            // dominant output, and re-import matching is positional, so a
            // rename can never break drift detection. Same reasoning as
            // TidyLayout / card moves on ◆ built entities (DECISIONS
            // "Built-immutability matrix").
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
            // Route paths store endpoint positions — refresh the waypoint that
            // sits on this factory (belt endpoints are its ports; power lines
            // reference the factory directly) so lines and 3D lengths track
            // pin moves and elevation edits.
            let routes: Vec<Route> = state.routes.values().cloned().collect();
            for mut r in routes {
                let owns = |end: &Id| {
                    end == id
                        || state
                            .ports
                            .get(end)
                            .map(|p| &p.factory == id)
                            .unwrap_or(false)
                };
                let mut touched = false;
                if owns(&r.endpoints.0) && !r.path.is_empty() {
                    r.path[0] = *position;
                    touched = true;
                }
                if owns(&r.endpoints.1) && !r.path.is_empty() {
                    let last = r.path.len() - 1;
                    r.path[last] = *position;
                    touched = true;
                }
                if touched {
                    // Priority switches sit at the line's midpoint (A2.3) —
                    // snap them to the refreshed geometry so a pin move never
                    // strands them on the stale line.
                    let mid = line_midpoint(&r.path);
                    let rid = r.id.clone();
                    tx.record(state.upsert(Entity::Route(r)));
                    let switches: Vec<PrioritySwitch> = state
                        .switches
                        .values()
                        .filter(|s| s.route == rid)
                        .cloned()
                        .collect();
                    for mut sw in switches {
                        if sw.position != mid {
                            sw.position = mid;
                            tx.record(state.upsert(Entity::Switch(sw)));
                        }
                    }
                }
            }
        }
        Command::DeleteFactory { id } => {
            let f = state
                .factories
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(f.status, id, "delete")?;
            remove_factory_cascading(state, &mut tx, id);
        }
        Command::AddGroup {
            factory,
            machine,
            recipe,
            count,
            clock,
            graph_pos,
            floor,
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
                floor: *floor,
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
            let count = (*count).max(1);
            if g.status == Status::Built {
                // §3.1.1: the edit materializes as a planned delta — the built
                // baseline is game ground truth (only import sync writes it).
                // Setting the built value back clears that component.
                let mut delta = g.planned_delta.unwrap_or_default();
                delta.count = (count != g.count).then_some(count);
                g.planned_delta = (!delta.is_empty()).then_some(delta);
            } else {
                g.count = count;
            }
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::SetGroupClock { id, clock } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            let clock = clamp_clock(*clock)?;
            if g.status == Status::Built {
                // §3.1.1: see SetGroupCount — same delta materialization rule.
                let mut delta = g.planned_delta.unwrap_or_default();
                delta.clock = ((clock - g.clock).abs() > 1e-9).then_some(clock);
                g.planned_delta = (!delta.is_empty()).then_some(delta);
            } else {
                g.clock = clock;
            }
            tx.record(state.upsert(Entity::Group(g)));
        }
        Command::SetGroupFloor { id, floor } => {
            let mut g = state
                .groups
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(g.status, id, "set floor")?;
            g.floor = *floor;
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
        Command::TidyLayout { factory } => {
            use crate::layout::{layered_layout, LKind, LNode};
            let f = state
                .factories
                .get(factory)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: factory.clone(),
                })?;
            let mut nodes: Vec<LNode> = Vec::new();
            for pid in &f.ports {
                if let Some(p) = state.ports.get(pid) {
                    nodes.push(LNode {
                        id: p.id.clone(),
                        kind: if p.direction == PortDirection::In {
                            LKind::InPort
                        } else {
                            LKind::OutPort
                        },
                    });
                }
            }
            for gid in &f.groups {
                if state.groups.contains_key(gid) {
                    nodes.push(LNode {
                        id: gid.clone(),
                        kind: LKind::Group,
                    });
                }
            }
            for j in state.junctions.values().filter(|j| &j.factory == factory) {
                nodes.push(LNode {
                    id: j.id.clone(),
                    kind: LKind::Junction,
                });
            }
            let end_id = |e: &EdgeEnd| match e {
                EdgeEnd::Group(id) | EdgeEnd::Port(id) | EdgeEnd::Junction(id) => id.clone(),
            };
            let edge_pairs: Vec<(Id, Id)> = state
                .edges
                .values()
                .filter(|e| &e.factory == factory)
                .map(|e| (end_id(&e.from), end_id(&e.to)))
                .collect();
            let positions = layered_layout(&nodes, &edge_pairs);
            for n in &nodes {
                let Some(pos) = positions.get(&n.id) else {
                    continue;
                };
                match n.kind {
                    LKind::Group => {
                        if let Some(g) = state.groups.get(&n.id) {
                            if g.graph_pos != *pos {
                                let mut g = g.clone();
                                g.graph_pos = *pos;
                                tx.record(state.upsert(Entity::Group(g)));
                            }
                        }
                    }
                    LKind::Junction => {
                        if let Some(j) = state.junctions.get(&n.id) {
                            if j.graph_pos != *pos {
                                let mut j = j.clone();
                                j.graph_pos = *pos;
                                tx.record(state.upsert(Entity::Junction(j)));
                            }
                        }
                    }
                    LKind::InPort | LKind::OutPort => {
                        if let Some(p) = state.ports.get(&n.id) {
                            if p.graph_pos != *pos {
                                let mut p = p.clone();
                                p.graph_pos = *pos;
                                tx.record(state.upsert(Entity::Port(p)));
                            }
                        }
                    }
                }
            }
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
            prune_build_override(state, &mut tx, id);
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
            if let Some(rid) = p.bound_route.clone() {
                remove_route_cascading(state, &mut tx, &rid);
            }
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
            // Every endpoint must exist and belong to this edge's factory —
            // a dangling or cross-factory end would corrupt the graph.
            if from == to {
                return Err(DomainError::Invalid {
                    message: "an edge needs two different endpoints".into(),
                });
            }
            for end in [from, to] {
                require_edge_end(state, end, factory)?;
            }
            // Junction port budgets are physical game constraints (splitter
            // 1-in/3-out, merger 3-in/1-out, storage 1/1) — refuse overflow.
            for (end, incoming) in [(from, false), (to, true)] {
                if let EdgeEnd::Junction(jid) = end {
                    let j = state
                        .junctions
                        .get(jid)
                        .ok_or(DomainError::NotFound { id: jid.clone() })?;
                    let (in_cap, out_cap) = j.kind.port_caps();
                    let used = state
                        .edges
                        .values()
                        .filter(|e| {
                            if incoming {
                                e.to == EdgeEnd::Junction(jid.clone())
                            } else {
                                e.from == EdgeEnd::Junction(jid.clone())
                            }
                        })
                        .count();
                    let cap = if incoming { in_cap } else { out_cap };
                    if used >= cap {
                        return Err(DomainError::Invalid {
                            message: format!(
                                "{:?} has all {} {} ports connected",
                                j.kind,
                                cap,
                                if incoming { "input" } else { "output" }
                            ),
                        });
                    }
                    // A standard splitter/merger/storage carries one item type;
                    // smart/programmable splitters may filter per output.
                    if !matches!(
                        j.kind,
                        JunctionKind::SmartSplitter | JunctionKind::ProgrammableSplitter
                    ) {
                        if let Some(other) = state.edges.values().find(|e| {
                            e.from == EdgeEnd::Junction(jid.clone())
                                || e.to == EdgeEnd::Junction(jid.clone())
                        }) {
                            if &other.item != item {
                                return Err(DomainError::Invalid {
                                    message: format!(
                                        "{:?} already carries a different item",
                                        j.kind
                                    ),
                                });
                            }
                        }
                    }
                }
            }
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
            // Belt tier is physical game infrastructure — a Mk.N belt exists
            // in the world, so the ◆ built layer is import-owned (§3.1.1).
            // Tier-upgrade-as-planned-delta is BACKLOG.
            require_planned(e.status, id, "set tier")?;
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
        Command::AddJunction {
            factory,
            kind,
            graph_pos,
            floor,
        } => {
            state.factories.get(factory).ok_or(DomainError::NotFound {
                id: factory.clone(),
            })?;
            let j = Junction {
                id: new_id(),
                factory: factory.clone(),
                kind: *kind,
                buildable: kind.buildable_class().to_string(),
                graph_pos: *graph_pos,
                floor: *floor,
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(j.id.clone());
            tx.record(state.upsert(Entity::Junction(j)));
        }
        Command::MoveJunctionCard { id, graph_pos } => {
            let mut j = state
                .junctions
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            j.graph_pos = *graph_pos;
            tx.record(state.upsert(Entity::Junction(j)));
        }
        Command::SetJunctionFloor { id, floor } => {
            let mut j = state
                .junctions
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(j.status, id, "set floor")?;
            j.floor = *floor;
            tx.record(state.upsert(Entity::Junction(j)));
        }
        Command::DeleteJunction { id } => {
            let j = state
                .junctions
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(j.status, id, "delete")?;
            let edge_ids: Vec<Id> = state
                .edges
                .values()
                .filter(|e| {
                    e.from == EdgeEnd::Junction(id.clone()) || e.to == EdgeEnd::Junction(id.clone())
                })
                .map(|e| e.id.clone())
                .collect();
            for eid in edge_ids {
                if let Some(ops) = state.remove(COLL_EDGES, &eid) {
                    tx.record(ops);
                }
            }
            if let Some(ops) = state.remove(COLL_JUNCTIONS, id) {
                tx.record(ops);
            }
        }
        Command::AddRoute {
            kind,
            from,
            to,
            path,
        } => {
            match kind {
                RouteKind::Power => {
                    // power lines join factories; endpoints are factory ids
                    for fid in [from, to] {
                        state
                            .factories
                            .get(fid)
                            .ok_or(DomainError::NotFound { id: fid.clone() })?;
                    }
                    if from == to {
                        return Err(DomainError::Invalid {
                            message: "a power line needs two different factories".into(),
                        });
                    }
                    if state.routes.values().any(|r| {
                        matches!(r.kind, RouteKind::Power)
                            && ((r.endpoints.0 == *from && r.endpoints.1 == *to)
                                || (r.endpoints.0 == *to && r.endpoints.1 == *from))
                    }) {
                        return Err(DomainError::Invalid {
                            message: "these factories are already connected".into(),
                        });
                    }
                    let r = Route {
                        id: new_id(),
                        kind: kind.clone(),
                        path: path.clone(),
                        endpoints: (from.clone(), to.clone()),
                        manifest: vec![],
                        status: Status::Planned,
                        created_by: CreatedBy::Manual,
                    };
                    tx.created.push(r.id.clone());
                    tx.record(state.upsert(Entity::Route(r)));
                }
                RouteKind::Belt { .. }
                | RouteKind::Rail { .. }
                | RouteKind::Truck { .. }
                | RouteKind::Drone { .. } => {
                    if let RouteKind::Belt { tier } = kind {
                        valid_tier(*tier)?;
                    }
                    let src = state
                        .ports
                        .get(from)
                        .cloned()
                        .ok_or(DomainError::NotFound { id: from.clone() })?;
                    let dst = state
                        .ports
                        .get(to)
                        .cloned()
                        .ok_or(DomainError::NotFound { id: to.clone() })?;
                    if src.direction != PortDirection::Out || dst.direction != PortDirection::In {
                        return Err(DomainError::Invalid {
                            message: "belt routes run from an OUT port to an IN port".into(),
                        });
                    }
                    if src.item != dst.item {
                        return Err(DomainError::Invalid {
                            message: "the ports carry different items".into(),
                        });
                    }
                    if src.bound_route.is_some() || dst.bound_route.is_some() {
                        return Err(DomainError::Invalid {
                            message: "a port is already bound to a route".into(),
                        });
                    }
                    let r = Route {
                        id: new_id(),
                        kind: kind.clone(),
                        path: path.clone(),
                        endpoints: (from.clone(), to.clone()),
                        manifest: vec![(src.item.clone(), 0.0)],
                        status: Status::Planned,
                        created_by: CreatedBy::Manual,
                    };
                    tx.created.push(r.id.clone());
                    let mut src = src;
                    let mut dst = dst;
                    src.bound_route = Some(r.id.clone());
                    dst.bound_route = Some(r.id.clone());
                    tx.record(state.upsert(Entity::Route(r)));
                    tx.record(state.upsert(Entity::Port(src)));
                    tx.record(state.upsert(Entity::Port(dst)));
                }
                RouteKind::Pipe { .. } => {
                    return Err(DomainError::Invalid {
                        message: "pipe routes arrive with fluids".into(),
                    });
                }
            }
        }
        Command::SetRouteSpec { id, kind } => {
            let mut r = state
                .routes
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(r.status, id, "respec")?;
            // cargo kinds interchange freely (same port binding); power and
            // pipe have different endpoint semantics — never switch across
            let cargo = |k: &RouteKind| {
                matches!(
                    k,
                    RouteKind::Belt { .. }
                        | RouteKind::Rail { .. }
                        | RouteKind::Truck { .. }
                        | RouteKind::Drone { .. }
                )
            };
            if !cargo(&r.kind) || !cargo(kind) {
                return Err(DomainError::Invalid {
                    message: "only cargo routes (belt/rail/truck/drone) can be re-specced".into(),
                });
            }
            if let RouteKind::Belt { tier } = kind {
                valid_tier(*tier)?;
            }
            r.kind = kind.clone();
            tx.record(state.upsert(Entity::Route(r)));
        }
        Command::SetRouteTier { id, tier } => {
            let mut r = state
                .routes
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            // Same field SetRouteSpec guards ("respec") — a built route's
            // belt tier is physical infrastructure (§3.1.1).
            require_planned(r.status, id, "set tier")?;
            match &mut r.kind {
                RouteKind::Belt { tier: t } => *t = valid_tier(*tier)?,
                other => {
                    return Err(DomainError::Invalid {
                        message: format!("{other:?} routes have no belt tier"),
                    });
                }
            }
            tx.record(state.upsert(Entity::Route(r)));
        }
        Command::DeleteRoute { id } => {
            let r = state
                .routes
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(r.status, id, "delete")?;
            remove_route_cascading(state, &mut tx, id);
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
                save_node_id: None,
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
            prune_build_override(state, &mut tx, id);
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
        Command::CreateProposal { proposal } => {
            let mut p = proposal.clone();
            if p.id.is_empty() {
                p.id = new_id();
            }
            // Review stamps are monotonic per plan.
            let max = state
                .proposals
                .values()
                .map(|q| q.number)
                .max()
                .unwrap_or(0);
            if p.number == 0 {
                p.number = max + 1;
            }
            tx.created.push(p.id.clone());
            tx.record(state.upsert(Entity::Proposal(p)));
        }
        Command::ToggleProposalItem {
            proposal,
            item,
            included,
        } => {
            let mut p = state
                .proposals
                .get(proposal)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: proposal.clone(),
                })?;
            if p.status == ProposalStatus::Accepted || p.status == ProposalStatus::Rejected {
                return Err(DomainError::Invalid {
                    message: "proposal is closed — re-solve to review again".into(),
                });
            }
            if p.item(item).is_none() {
                return Err(DomainError::NotFound { id: item.clone() });
            }
            // Cascade: excluding an item excludes everything depending on it
            // (transitively); including pulls its dependencies back in.
            let mut queue = vec![item.clone()];
            while let Some(next) = queue.pop() {
                let deps_of_next: Vec<Id> = if *included {
                    p.item(&next)
                        .map(|i| i.depends_on.clone())
                        .unwrap_or_default()
                } else {
                    p.items
                        .iter()
                        .filter(|i| i.depends_on.contains(&next))
                        .map(|i| i.id.clone())
                        .collect()
                };
                if let Some(it) = p.items.iter_mut().find(|i| i.id == next) {
                    if it.included != *included {
                        it.included = *included;
                        queue.extend(deps_of_next);
                    } else if next == *item {
                        queue.extend(deps_of_next);
                    }
                }
            }
            if p.status == ProposalStatus::Draft {
                p.status = ProposalStatus::Reviewing;
            }
            tx.record(state.upsert(Entity::Proposal(p)));
        }
        Command::SetProposalStatus { id, status } => {
            let mut p = state
                .proposals
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            p.status = *status;
            tx.record(state.upsert(Entity::Proposal(p)));
        }
        Command::DeleteProposal { id } => {
            state
                .proposals
                .get(id)
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            if let Some(ops) = state.remove(COLL_PROPOSALS, id) {
                tx.record(ops);
            }
        }
        Command::AddPrioritySwitch { route, priority } => {
            let r = state
                .routes
                .get(route)
                .ok_or(DomainError::NotFound { id: route.clone() })?;
            if !matches!(r.kind, RouteKind::Power) {
                return Err(DomainError::Invalid {
                    message: "priority switches sit on power lines".into(),
                });
            }
            if !(1..=8).contains(priority) {
                return Err(DomainError::Invalid {
                    message: format!("priority P{priority} outside P1–P8"),
                });
            }
            let sw = PrioritySwitch {
                id: new_id(),
                route: route.clone(),
                priority: *priority,
                // midpoint of the line — square pin grammar (A2.3)
                position: line_midpoint(&r.path),
                status: Status::Planned,
                created_by: CreatedBy::Manual,
            };
            tx.created.push(sw.id.clone());
            tx.record(state.upsert(Entity::Switch(sw)));
        }
        Command::SetSwitchPriority { id, priority } => {
            let mut sw = state
                .switches
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            if !(1..=8).contains(priority) {
                return Err(DomainError::Invalid {
                    message: format!("priority P{priority} outside P1–P8"),
                });
            }
            require_planned(sw.status, id, "reprioritize")?;
            sw.priority = *priority;
            tx.record(state.upsert(Entity::Switch(sw)));
        }
        Command::CreateStyleGuide { guide } => {
            let mut g = guide.clone();
            if g.id.is_empty() {
                g.id = new_id();
            }
            tx.created.push(g.id.clone());
            tx.record(state.upsert(Entity::StyleGuide(g)));
        }
        Command::DeleteStyleGuide { id } => {
            state
                .style_guides
                .get(id)
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            // unlink any factory themed with it
            let themed: Vec<Factory> = state
                .factories
                .values()
                .filter(|f| f.style_guide.as_deref() == Some(id.as_str()))
                .cloned()
                .collect();
            for mut f in themed {
                f.style_guide = None;
                tx.record(state.upsert(Entity::Factory(f)));
            }
            if let Some(ops) = state.remove(COLL_STYLE_GUIDES, id) {
                tx.record(ops);
            }
        }
        Command::SetFactoryTheme {
            factory,
            style_guide,
        } => {
            let mut f = state
                .factories
                .get(factory)
                .cloned()
                .ok_or(DomainError::NotFound {
                    id: factory.clone(),
                })?;
            if let Some(sg) = style_guide {
                state
                    .style_guides
                    .get(sg)
                    .ok_or(DomainError::NotFound { id: sg.clone() })?;
            }
            f.style_guide = style_guide.clone();
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::SetBuildDone { id, done } => {
            // The override is a sparse assertion overlay routed through the same
            // upsert/remove machinery as every other entity, so it undoes in one
            // step. No `require_planned`: it is build-progress metadata, never a
            // mutation of the ◆ game-ground-truth layer.
            match done {
                Some(v) => tx.record(state.upsert(Entity::BuildOverride(BuildOverride {
                    id: id.clone(),
                    done: *v,
                }))),
                None => {
                    if let Some(ops) = state.remove(COLL_BUILD_OVERRIDES, id) {
                        tx.record(ops);
                    }
                }
            }
        }
        Command::SetNodeOverride { id, node_override } => {
            // Sparse plan-local overlay routed through the same upsert/remove
            // machinery as every other entity, so it undoes in one step. No
            // `require_planned`: the bundled/ambient catalog is never mutated —
            // this only records a plan-side correction of node geometry.
            match node_override {
                Some(ov) => {
                    let mut ov = ov.clone();
                    ov.id = id.clone();
                    tx.record(state.upsert(Entity::NodeOverride(ov)));
                }
                None => {
                    if let Some(ops) = state.remove(COLL_NODE_OVERRIDES, id) {
                        tx.record(ops);
                    }
                }
            }
        }
        Command::SetFactoryReplaces { id, replaces } => {
            let mut f = state
                .factories
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            // Deliberately NOT `require_planned` (§3.1.1 exemption): `replaces`
            // is a planner-side label with the same reasoning as RenameFactory —
            // the save format has no such concept, so it can never break drift.
            if let Some(target) = replaces {
                if target == id {
                    return Err(DomainError::Invalid {
                        message: "a factory can't replace itself".into(),
                    });
                }
                let old = state
                    .factories
                    .get(target)
                    .ok_or(DomainError::NotFound { id: target.clone() })?;
                // The replaced factory must be a running ◆ Built one — that is
                // the whole premise of a cutover (tear down the built factory
                // once its ◇ replacement is up).
                if old.status != Status::Built {
                    return Err(DomainError::Invalid {
                        message: format!("replacement target {target} is not a built factory"),
                    });
                }
            }
            f.replaces = replaces.clone();
            tx.record(state.upsert(Entity::Factory(f)));
        }
        Command::DeleteSwitch { id } => {
            let sw = state
                .switches
                .get(id)
                .cloned()
                .ok_or(DomainError::NotFound { id: id.clone() })?;
            require_planned(sw.status, id, "delete")?;
            if let Some(ops) = state.remove(COLL_SWITCHES, id) {
                tx.record(ops);
            }
        }
    }
    Ok(tx)
}
