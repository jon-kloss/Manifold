//! Domain entities per SDD §3. The full shape ships from day one — Phase 1 only
//! creates Planned/Manual entities, but `status` and `created_by` are always present.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};

/// Ulid rendered as its canonical string — JSON- and SQLite-friendly.
pub type Id = String;

thread_local! {
    // Monotonic generator: ids minted within the same millisecond keep
    // ascending order (the random component is incremented, not re-rolled).
    // Plain `Ulid::new()` re-randomizes each call, so ids created in one burst
    // — an import or a proposal accept mints many at once — sorted in random
    // order, which the build-queue "chronological within a bucket" ordering
    // relies on. Thread-local because every logical plan operation runs on a
    // single thread; cross-thread ordering is not a chronology we promise.
    static GEN: RefCell<ulid::Generator> = const { RefCell::new(ulid::Generator::new()) };
}

pub fn new_id() -> Id {
    GEN.with(|g| {
        // `generate` only errors on random-component overflow within a
        // millisecond (>1.2e24 ids) — impossible here; fall back if it ever does.
        g.borrow_mut()
            .generate()
            .unwrap_or_else(|_| ulid::Ulid::new())
            .to_string()
    })
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
    /// Elevation in meters. Planner-entered (no heightmap is bundled); defaults
    /// to 0 so pre-elevation plan files load unchanged. Drives 3D route length,
    /// climb readouts, and later pipe head-lift / rail grade checks.
    #[serde(default)]
    pub z: f64,
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
    /// Refactor/cutover link (W2a): when this ◇ factory is a planned replacement
    /// for a running ◆ factory, `replaces` names that old factory's id. It is a
    /// planner-side LABEL (same species as `name`) — never a ◆ mutation and never
    /// a write to the referenced entity; the cutover + downtime are DERIVED from
    /// it, and it auto-nulls on re-import once the old factory is gone. serde-
    /// default so plan files predating W2a load unchanged (no migration).
    #[serde(default)]
    pub replaces: Option<Id>,
    pub status: Status,
    pub created_by: CreatedBy,
}

/// Planned overlay on a ◆ built group (SDD §3.1.1): each component holds the
/// planned *effective* value; `None` means "track the built baseline". Per-
/// component so a clock-only retune never pins the count — unedited components
/// keep following the baseline through re-import sync.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupDelta {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub clock: Option<f64>,
}

impl GroupDelta {
    pub fn is_empty(&self) -> bool {
        self.count.is_none() && self.clock.is_none()
    }
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
    /// User-planned changes riding on a Built baseline. Baseline `count`/`clock`
    /// stay game ground truth (only import sync writes them); solvers read the
    /// effective values, never write the delta.
    #[serde(default)]
    pub planned_delta: Option<GroupDelta>,
    pub graph_pos: GraphPos,
    /// Vertical factory floor (0 = ground). Display + planning aid; belts
    /// crossing floors render as lifts.
    #[serde(default)]
    pub floor: u32,
    pub status: Status,
    pub created_by: CreatedBy,
}

impl MachineGroup {
    /// Count the solver should plan with: the delta overlay if present, else baseline.
    pub fn effective_count(&self) -> u32 {
        self.planned_delta
            .and_then(|d| d.count)
            .unwrap_or(self.count)
    }

    /// Clock the solver should plan with: the delta overlay if present, else baseline.
    pub fn effective_clock(&self) -> f64 {
        self.planned_delta
            .and_then(|d| d.clock)
            .unwrap_or(self.clock)
    }
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
    Junction(Id),
}

/// Belt-side logistics buildables. Junctions transform nothing — they only
/// split/merge/buffer flows — so solvers treat them as conservation nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JunctionKind {
    Splitter,
    SmartSplitter,
    ProgrammableSplitter,
    Merger,
    Storage,
}

impl JunctionKind {
    /// Physical port budget (inputs, outputs) — game constraints, enforced on connect.
    pub fn port_caps(&self) -> (usize, usize) {
        match self {
            JunctionKind::Splitter
            | JunctionKind::SmartSplitter
            | JunctionKind::ProgrammableSplitter => (1, 3),
            JunctionKind::Merger => (3, 1),
            JunctionKind::Storage => (1, 1),
        }
    }

    /// Default buildable class for display/footprint lookup.
    pub fn buildable_class(&self) -> &'static str {
        match self {
            JunctionKind::Splitter => "Build_ConveyorAttachmentSplitter_C",
            JunctionKind::SmartSplitter => "Build_ConveyorAttachmentSplitterSmart_C",
            JunctionKind::ProgrammableSplitter => "Build_ConveyorAttachmentSplitterProgrammable_C",
            JunctionKind::Merger => "Build_ConveyorAttachmentMerger_C",
            JunctionKind::Storage => "Build_StorageContainerMk1_C",
        }
    }
}

/// A belt junction on the factory graph (splitter/merger/storage).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Junction {
    pub id: Id,
    pub factory: Id,
    pub kind: JunctionKind,
    /// Buildable class for display (icon/footprint); defaults per kind.
    pub buildable: String,
    pub graph_pos: GraphPos,
    #[serde(default)]
    pub floor: u32,
    pub status: Status,
    pub created_by: CreatedBy,
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
    /// Resolved node id: a bundled-snapshot [`crate::...`] WorldNode id, or a
    /// plan-local `"save:<nodeActorId>"` when the extractor sits on no known
    /// snapshot node (W2b-C).
    pub node: String,
    pub factory: Id,
    /// Extractor machine class.
    pub extractor: String,
    pub clock: f64,
    /// The save's STABLE node reference (`mExtractableResource` pathName) this
    /// claim was bound from — the re-match key on re-import, so binding survives
    /// position noise (W2b-C). `None` for manually-drawn claims. serde-default so
    /// plan files predating W2b-C load unchanged (no migration).
    #[serde(default)]
    pub save_node_id: Option<String>,
    pub status: Status,
    pub created_by: CreatedBy,
}

/// Plan-local correction of a resource node's geometry (W2b-C). Sparse overlay
/// keyed by node id (`"<snapshot id>"` or `"save:<nodeActorId>"`): the bundled
/// world catalog (and any `FICSIT_WORLD_NODES` swap) stays an ambient, never-
/// mutated default — a node's RESOLVED position is `snapshot ⊕ override`. Purity
/// is deliberately NOT correctable: the save carries none, so the snapshot is
/// the trusted source (snapshot-primary). Save-only nodes (`"save:<id>"`, absent
/// from every catalog) synthesize into the resolved set from `pos` alone.
/// serde-default so plan files predating W2b-C load unchanged (no migration).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeOverride {
    /// Node id this correction applies to (matches [`NodeClaim::node`]).
    pub id: String,
    /// Corrected world position; `None` leaves the catalog coordinate intact.
    #[serde(default)]
    pub pos: Option<MapPos>,
    /// The save's stable node actor id this correction was reconciled from.
    #[serde(default)]
    pub save_actor: Option<String>,
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

impl Default for RailSpec {
    fn default() -> Self {
        Self {
            consists: 1,
            locos: 1,
            cars: 4,
            stations: vec![
                StationSpec {
                    name: "LOAD".into(),
                    platforms: 1,
                    dwell_s: 25.0,
                },
                StationSpec {
                    name: "UNLOAD".into(),
                    platforms: 1,
                    dwell_s: 25.0,
                },
            ],
            headway_penalty: 0.15,
        }
    }
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

impl Default for TruckSpec {
    fn default() -> Self {
        Self {
            trucks: 1,
            fuel_item: "Desc_Coal_C".into(),
        }
    }
}

impl Default for DroneSpec {
    fn default() -> Self {
        Self {
            batteries_per_trip: 4.0,
        }
    }
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

/// Style guide (SDD §9 image→style-guide): typed aesthetic descriptor,
/// linkable to factories. The vision call fills it when a model key exists;
/// manual creation keeps the surface honest offline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleGuide {
    pub id: Id,
    pub name: String,
    /// (material, share 0..1)
    pub palette: Vec<(String, f64)>,
    pub massing: String,
    pub techniques: Vec<String>,
    pub sequence: Vec<String>,
    pub source_note: String,
}

/// Priority switch (A2.3): an 18px square map pin sitting ON a power line.
/// Shedding order is highest priority number first (P8 before P1) — the audit
/// POWER tab derives each switch's SHEDS AT threshold from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrioritySwitch {
    pub id: Id,
    /// The power route this switch sits on.
    pub route: Id,
    /// 1–8; higher sheds first.
    pub priority: u8,
    pub position: MapPos,
    pub status: Status,
    pub created_by: CreatedBy,
}

/// Manual completion assertion for a build-queue step (W1c). A sparse overlay
/// keyed by the step entity's id (factory / group / route / claim): present
/// ONLY when the user has manually checked or unchecked a step against its
/// derived state. Completion itself stays DERIVED (◇◈◆) — this override just
/// pins the resolved answer, and auto-dissolves on re-import once the derived
/// state agrees (mirroring `MachineGroup::planned_delta`). Lives in its own
/// collection so the solver/drift reads never see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildOverride {
    pub id: Id,
    pub done: bool,
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
