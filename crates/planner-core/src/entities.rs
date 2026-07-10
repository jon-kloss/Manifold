//! Domain entities per SDD §3. The full shape ships from day one — Phase 1 only
//! creates Planned/Manual entities, but `status` and `created_by` are always present.

use serde::{Deserialize, Serialize};

/// Ulid rendered as its canonical string — JSON- and SQLite-friendly.
pub type Id = String;

pub fn new_id() -> Id {
    ulid::Ulid::new().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Planned,
    UnderConstruction,
    Built,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum CreatedBy {
    Manual,
    Proposal(Id),
    Import(Id),
}

/// World-map position in game-world meters (Satisfactory save coordinate frame).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MapPos {
    pub x: f64,
    pub y: f64,
}

/// Graph-canvas position for factory-view cards (CSS px in React Flow space).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphPos {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Factory {
    pub id: Id,
    pub name: String,
    pub position: MapPos,
    pub region: String,
    pub node_claims: Vec<Id>,
    pub groups: Vec<Id>,
    pub ports: Vec<Id>,
    pub style_guide: Option<Id>,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineGroup {
    pub id: Id,
    pub factory: Id,
    /// Machine class name, e.g. `Build_ConstructorMk1_C`.
    pub machine: String,
    /// Recipe class name, e.g. `Recipe_ModularFrame_C`.
    pub recipe: String,
    pub count: u32,
    /// 0.01–2.50 (1.0 = 100%).
    pub clock: f64,
    pub somersloops: u8,
    pub planned_delta: Option<Id>,
    pub graph_pos: GraphPos,
    /// Vertical factory floor (0 = ground). Display + planning aid; belts
    /// crossing floors render as lifts.
    #[serde(default)]
    pub floor: u32,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    In,
    Out,
}

/// A factory boundary port: where an item crosses the factory boundary.
/// In the graph view these render as slim edge cards; on the map they anchor routes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Port {
    pub id: Id,
    pub factory: Id,
    pub direction: PortDirection,
    pub item: String,
    /// Rate in items/min. For an Out port this is the factory's target rate.
    pub rate: f64,
    /// Ceiling for In ports (e.g. node extraction rate). None = unconstrained.
    pub rate_ceiling: Option<f64>,
    pub bound_route: Option<Id>,
    pub graph_pos: GraphPos,
    pub status: Status,
    pub created_by: CreatedBy,
}

/// Belt tiers: capacity in items/min.
pub const BELT_CAPACITY: [f64; 6] = [60.0, 120.0, 270.0, 480.0, 780.0, 1200.0];

pub fn belt_capacity(tier: u8) -> f64 {
    BELT_CAPACITY[(tier.clamp(1, 6) - 1) as usize]
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum EdgeEnd {
    Group(Id),
    Port(Id),
}

/// Intra-factory belt connection (graph edge). Flow rate is derived solver
/// output; tier is user-set. See DECISIONS.md — not in SDD §3, required by the graph view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeltEdge {
    pub id: Id,
    pub factory: Id,
    pub from: EdgeEnd,
    pub to: EdgeEnd,
    pub item: String,
    /// Belt tier 1–6.
    pub tier: u8,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeClaim {
    pub id: Id,
    /// WorldNodeId from the bundled static snapshot.
    pub node: String,
    pub factory: Id,
    /// Extractor machine class.
    pub extractor: String,
    pub clock: f64,
    pub status: Status,
    pub created_by: CreatedBy,
}

// ---- Later-phase entities: full data-model shape from day one (HANDOFF mandate).
// No Phase 1 command creates these; no Phase 1 UI renders them.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RailSpec {
    pub consists: u8,
    pub locos: u8,
    pub cars: u8,
    pub stations: Vec<StationSpec>,
    /// Fixed headway penalty (0.15 in v1 — Addendum A3.1).
    pub headway_penalty: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StationSpec {
    pub name: String,
    pub platforms: u8,
    /// Dwell time in seconds.
    pub dwell_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TruckSpec {
    pub trucks: u8,
    pub fuel_item: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DroneSpec {
    pub batteries_per_trip: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteKind {
    Belt { tier: u8 },
    Pipe { tier: u8 },
    Rail { spec: RailSpec },
    Truck { spec: TruckSpec },
    Drone { spec: DroneSpec },
    Power,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub id: Id,
    pub kind: RouteKind,
    /// Map polyline, in world meters.
    pub path: Vec<MapPos>,
    pub endpoints: (Id, Id), // Port ids
    pub manifest: Vec<(String, f64)>,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Circuit {
    pub id: Id,
    pub name: String,
    pub members: Vec<Id>,
    pub switches: Vec<Id>,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Switch {
    pub id: Id,
    pub position: MapPos,
    pub circuit_a: Id,
    pub circuit_b: Id,
    pub priority: u8,
    pub status: Status,
    pub created_by: CreatedBy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalSource {
    GlobalSolver,
    Advisor,
    Chat,
    SaveReimport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Draft,
    Reviewing,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalItemKind {
    Create,
    Modify,
    Claim,
    RouteAdd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposalItem {
    pub kind: ProposalItemKind,
    pub included: bool,
    pub payload: serde_json::Value,
    pub consequences: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: Id,
    pub source: ProposalSource,
    pub goal: String,
    pub snapshot_time: String,
    pub input_hash: String,
    pub items: Vec<ProposalItem>,
    pub state: ProposalState,
    pub stale: bool,
}
