// TS mirror of the Rust projection types (planner-core + session).
// The renderer is a projection patched by events — never a source of truth.

export type Id = string;
export type Status = "planned" | "under_construction" | "built";
export type CreatedBy = { kind: "manual" } | { kind: "proposal"; id: Id } | { kind: "import"; id: Id };

/** z = elevation in meters, planner-entered (defaults 0; no bundled heightmap). */
export interface MapPos { x: number; y: number; z?: number }
export interface GraphPos { x: number; y: number }

export interface Factory {
  id: Id;
  name: string;
  position: MapPos;
  region: string;
  nodeClaims: Id[];
  groups: Id[];
  ports: Id[];
  styleGuide: Id | null;
  status: Status;
  createdBy: CreatedBy;
}

export interface MachineGroup {
  id: Id;
  factory: Id;
  machine: string;
  recipe: string;
  count: number;
  clock: number;
  somersloops: number;
  plannedDelta: Id | null;
  graphPos: GraphPos;
  /** Vertical factory floor (0 = ground). */
  floor: number;
  status: Status;
  createdBy: CreatedBy;
}

export type PortDirection = "in" | "out";

export interface Port {
  id: Id;
  factory: Id;
  direction: PortDirection;
  item: string;
  rate: number;
  rateCeiling: number | null;
  boundRoute: Id | null;
  graphPos: GraphPos;
  status: Status;
  createdBy: CreatedBy;
}

export type EdgeEnd = { kind: "group"; id: Id } | { kind: "port"; id: Id } | { kind: "junction"; id: Id };

export type JunctionKind = "splitter" | "smart_splitter" | "programmable_splitter" | "merger" | "storage";

export interface Junction {
  id: Id;
  factory: Id;
  kind: JunctionKind;
  buildable: string;
  graphPos: GraphPos;
  floor: number;
  status: Status;
  createdBy: CreatedBy;
}

/** Physical port budget (inputs, outputs) per junction kind — game constraints. */
export const JUNCTION_CAPS: Record<JunctionKind, [number, number]> = {
  splitter: [1, 3],
  smart_splitter: [1, 3],
  programmable_splitter: [1, 3],
  merger: [3, 1],
  storage: [1, 1],
};

export interface BeltEdge {
  id: Id;
  factory: Id;
  from: EdgeEnd;
  to: EdgeEnd;
  item: string;
  tier: number;
  status: Status;
  createdBy: CreatedBy;
}

export interface StationSpec { name: string; platforms: number; dwellS: number }
export interface RailSpec {
  consists: number;
  locos: number;
  cars: number;
  stations: StationSpec[];
  headwayPenalty: number;
}
export interface TruckSpec { trucks: number; fuelItem: string }
export interface DroneSpec { batteriesPerTrip: number }

export const DEFAULT_RAIL_SPEC: RailSpec = {
  consists: 1,
  locos: 1,
  cars: 4,
  stations: [
    { name: "LOAD", platforms: 1, dwellS: 25 },
    { name: "UNLOAD", platforms: 1, dwellS: 25 },
  ],
  headwayPenalty: 0.15,
};
export const DEFAULT_TRUCK_SPEC: TruckSpec = { trucks: 1, fuelItem: "Desc_Coal_C" };
export const DEFAULT_DRONE_SPEC: DroneSpec = { batteriesPerTrip: 4 };

export type RouteKind =
  | { kind: "belt"; tier: number }
  | { kind: "pipe"; tier: number }
  | { kind: "power" }
  | { kind: "rail"; spec: RailSpec }
  | { kind: "truck"; spec: TruckSpec }
  | { kind: "drone"; spec: DroneSpec };

export interface Route {
  id: Id;
  kind: RouteKind;
  path: MapPos[];
  endpoints: [Id, Id];
  manifest: [string, number][];
  status: Status;
  createdBy: CreatedBy;
}

/** Priority switch (A2.3): square pin ON a power line; higher P sheds first. */
export interface PrioritySwitch {
  id: Id;
  route: Id;
  priority: number;
  position: MapPos;
  status: Status;
  createdBy: CreatedBy;
}

export interface NodeClaim {
  id: Id;
  node: string;
  factory: Id;
  extractor: string;
  clock: number;
  status: Status;
  createdBy: CreatedBy;
}

export interface PlanMeta {
  schemaVersion: number;
  gameBuild: string;
  name: string;
}

export interface StyleGuide {
  id: Id;
  name: string;
  /** (material, share 0..1) */
  palette: [string, number][];
  massing: string;
  techniques: string[];
  sequence: string[];
  /** provenance: where this guide came from (vision call | manual) */
  sourceNote: string;
}

export interface Plan {
  meta: PlanMeta;
  factories: Record<Id, Factory>;
  groups: Record<Id, MachineGroup>;
  ports: Record<Id, Port>;
  edges: Record<Id, BeltEdge>;
  nodeClaims: Record<Id, NodeClaim>;
  routes: Record<Id, Route>;
  junctions: Record<Id, Junction>;
  proposals: Record<Id, Proposal>;
  switches: Record<Id, PrioritySwitch>;
  styleGuides: Record<Id, StyleGuide>;
}

// ---- gamedata ----

export interface GameItem { className: string; displayName: string; form: string; stackSize: string }
export interface GameRecipe {
  className: string;
  displayName: string;
  durationS: number;
  ingredients: [string, number][];
  products: [string, number][];
  producedIn: string[];
  alternate: boolean;
}
export interface GameMachine { className: string; displayName: string; powerMw: number; kind: string }
export interface GameBelt { className: string; displayName: string; capacityPerMin: number; tier: number }

export interface GameBuildable { className: string; displayName: string; nativeClass: string }

export interface GameData {
  items: Record<string, GameItem>;
  recipes: Record<string, GameRecipe>;
  machines: Record<string, GameMachine>;
  belts: Record<string, GameBelt>;
  buildables: Record<string, GameBuildable>;
  buildVersion: string;
}

// ---- world snapshot ----

export interface WorldNode {
  id: string;
  item: string;
  purity: "pure" | "normal" | "impure";
  x: number;
  y: number;
  /** elevation in meters */
  z: number;
  /** cave nodes are reached via their entrance, not their overhead x/y */
  zone: "surface" | "cave";
  entrance?: { x: number; y: number; z: number };
  region: string;
}
export interface WorldRegion { id: string; name: string; labelX: number; labelY: number }
export interface World {
  version: number;
  source: string;
  bounds: { minX: number; minY: number; maxX: number; maxY: number };
  regions: WorldRegion[];
  nodes: WorldNode[];
}

// ---- derived (solver output; recomputed, never persisted) ----

export type Constraint =
  | { kind: "belt_capacity"; edge: Id; item: string; capacity: number }
  | { kind: "input_ceiling"; port: Id; item: string; ceiling: number };

export interface TargetCeiling { maxRate: number; binding: Constraint }

export interface DerivedGroup { inRates: Record<string, number>; outRates: Record<string, number>; powerMw: number }
export interface DerivedEdge { flow: number; saturation: number }

export interface DerivedFactory {
  groups: Record<Id, DerivedGroup>;
  edges: Record<Id, DerivedEdge>;
  ports: Record<Id, number>;
  totalPowerMw: number;
  targetCeiling: TargetCeiling | null;
  solveUs: number;
  solveOnRelease: boolean;
  solveError: string | null;
}

/** A3 math block — every line the rail/truck/drone inspector renders. */
export interface TransportMath {
  effectiveLengthM: number;
  roundTripS: number;
  loadUnloadS: number;
  headwayS: number | null;
  rttS: number;
  perTripItems: number;
  throughputPerMin: number;
  batteriesPerMin: number | null;
  fuelItem: string | null;
}

export interface DerivedRoute {
  flow: number;
  supplied: number;
  capacity: number;
  saturation: number;
  lengthM: number;
  /** meters climbed / descended along the path (0 on flat plans) */
  climbUpM: number;
  climbDownM: number;
  item: string | null;
  /** rail/truck/drone only */
  transport: TransportMath | null;
}

export interface DeficitRow {
  factory: Id;
  port: Id;
  route: Id | null;
  item: string;
  needed: number;
  supplied: number;
}

export interface DerivedSwitch {
  id: Id;
  priority: number;
  downstreamMw: number;
  shedsAtMw: number;
}

export interface DerivedCircuit {
  name: string;
  members: Id[];
  generationMw: number;
  demandMw: number;
  /** shed order first (P8 → P1) */
  switches: DerivedSwitch[];
  /** brownout sim: next group to shed, e.g. "P4 @ +12 MW growth" */
  nextShed: string | null;
}

export interface Derived {
  factories: Record<Id, DerivedFactory>;
  nodes: Record<string, { claims: number; conflict: boolean }>;
  routes: Record<Id, DerivedRoute>;
  deficits: DeficitRow[];
  circuits: DerivedCircuit[];
  totalGenerationMw: number;
  empireCycle: boolean;
  recomputeUs: number;
  totalPowerMw: number;
}

// ---- IPC ----

export type PatchOp =
  | { op: "add"; path: string; value: unknown }
  | { op: "replace"; path: string; value: unknown }
  | { op: "remove"; path: string };

// ---- proposals (Phase 3): reviewable, partially-acceptable change sets ----

export type ProposalStatus = "draft" | "reviewing" | "accepted" | "rejected";
export type ProposalSource = "global_solver" | "t2_optimize" | "advisor" | "chat" | "save_reimport";
export type ProposalItemKind = "create" | "modify" | "claim" | "route_add";

export interface ProposalItem {
  id: Id;
  kind: ProposalItemKind;
  included: boolean;
  label: string;
  detail: string;
  impact: string;
  /** commands this item materializes to; ids may be $alias placeholders */
  commands: Command[];
  aliases: (string | null)[];
  dependsOn: Id[];
  /** SaveReimport drift payload — accept syncs the ◆ Built layer */
  sync?: unknown;
}

export interface Proposal {
  id: Id;
  source: ProposalSource;
  title: string;
  goal: [string, number][];
  status: ProposalStatus;
  number: number;
  snapshotTime: string;
  /** compare with planHash — mismatch renders the STALE badge */
  inputHash: string;
  provenance: string;
  items: ProposalItem[];
}

export interface GoalCheck { item: string; requested: number; achieved: number }

export interface ProposalConsequence {
  goal: GoalCheck[];
  goalMet: boolean;
  deltaPowerMw: number;
  deltaGenerationMw: number;
  machines: number;
  warnings: string[];
}

export interface WizardConstraints {
  surplusFirst: boolean;
  maxNewSites: number;
  nodeBudget: number;
  purityFloor: "impure" | "normal" | "pure";
  powerMarginCap: number;
  expandPreference: number;
  includeAlternates: boolean;
}

export interface WizardGoal {
  items: [string, number][];
  constraints: WizardConstraints;
}

export interface WizardLogLine { phase: string; line: string }

export interface WizardInfeasible { bestRate: number; binding: string; relaxations: string[] }

export type WizardOutcome =
  | { outcome: "proposal"; proposal: Proposal }
  | ({ outcome: "infeasible" } & WizardInfeasible)
  | { outcome: "cancelled" };

export interface JobProgress {
  log: WizardLogLine[];
  done: boolean;
  outcome: WizardOutcome | null;
}

export interface EditResponse {
  patches: PatchOp[];
  derived: Derived;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  created: Id[];
  planHash: string;
  advisor: AdvisorFeed;
}

export interface InitPayload {
  plan: Plan;
  derived: Derived;
  gamedata: GameData;
  world: World;
  planHash: string;
  advisor: AdvisorFeed;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  viewState: ViewState | null;
}

export interface ViewState {
  map?: { center: [number, number]; zoom: number };
  openFactory?: Id | null;
  /** first-run card dismissed */
  onboarded?: boolean;
}

// ---- commands (serde: tag "type" snake_case, fields camelCase) ----

export type Command =
  | { type: "create_factory"; name: string; position: MapPos; region: string }
  | { type: "rename_factory"; id: Id; name: string }
  | { type: "move_factory_pin"; id: Id; position: MapPos }
  | { type: "delete_factory"; id: Id }
  | { type: "add_group"; factory: Id; machine: string; recipe: string; count: number; clock: number; graphPos: GraphPos; floor: number }
  | { type: "set_group_recipe"; id: Id; machine: string; recipe: string }
  | { type: "set_group_count"; id: Id; count: number }
  | { type: "set_group_clock"; id: Id; clock: number }
  | { type: "set_group_floor"; id: Id; floor: number }
  | { type: "move_group_card"; id: Id; graphPos: GraphPos }
  | { type: "delete_group"; id: Id }
  | { type: "add_port"; factory: Id; direction: PortDirection; item: string; rate: number; rateCeiling: number | null; graphPos: GraphPos }
  | { type: "set_port_rate"; id: Id; rate: number }
  | { type: "set_port_ceiling"; id: Id; rateCeiling: number | null }
  | { type: "move_port_card"; id: Id; graphPos: GraphPos }
  | { type: "delete_port"; id: Id }
  | { type: "add_edge"; factory: Id; from: EdgeEnd; to: EdgeEnd; item: string; tier: number }
  | { type: "add_junction"; factory: Id; kind: JunctionKind; graphPos: GraphPos; floor: number }
  | { type: "move_junction_card"; id: Id; graphPos: GraphPos }
  | { type: "set_junction_floor"; id: Id; floor: number }
  | { type: "delete_junction"; id: Id }
  | { type: "add_route"; kind: RouteKind; from: Id; to: Id; path: MapPos[] }
  | { type: "set_route_tier"; id: Id; tier: number }
  | { type: "set_route_spec"; id: Id; kind: RouteKind }
  | { type: "delete_route"; id: Id }
  | { type: "set_edge_tier"; id: Id; tier: number }
  | { type: "delete_edge"; id: Id }
  | { type: "claim_node"; factory: Id; node: string; extractor: string; clock: number }
  | { type: "release_node"; id: Id }
  | { type: "rename_plan"; name: string }
  | { type: "create_proposal"; proposal: Proposal }
  | { type: "toggle_proposal_item"; proposal: Id; item: Id; included: boolean }
  | { type: "set_proposal_status"; id: Id; status: ProposalStatus }
  | { type: "delete_proposal"; id: Id }
  | { type: "add_priority_switch"; route: Id; priority: number }
  | { type: "set_switch_priority"; id: Id; priority: number }
  | { type: "delete_switch"; id: Id }
  | { type: "create_style_guide"; guide: StyleGuide }
  | { type: "delete_style_guide"; id: Id }
  | { type: "set_factory_theme"; factory: Id; styleGuide: Id | null };

export const BELT_CAPACITY = [60, 120, 270, 480, 780, 1200];
export const beltCapacity = (tier: number) => BELT_CAPACITY[Math.min(6, Math.max(1, tier)) - 1];

/** Pseudo-item for generator output: 1 "item/min" = 1 MW (Addendum A2). */
export const POWER_ITEM = "__PowerMW";

// ---- save import (SDD §8) ----

export interface ImportMachine {
  class: string;
  recipe?: string | null;
  clock?: number;
  x: number;
  y: number;
  z?: number;
}

export interface ImportSnapshot {
  saveName: string;
  buildVersion?: string;
  machines: ImportMachine[];
  extractors?: ImportMachine[];
  belts?: Record<string, number>;
  rails?: number;
  powerLines?: number;
  locomotives?: number;
  wagons?: number;
  trainStations?: number;
  quarantined?: Record<string, number>;
}

export type ImportOutcome =
  | { outcome: "imported"; response: EditResponse; factories: number; machines: number; quarantined: number }
  | { outcome: "drift"; response: EditResponse; proposal: Id }
  | { outcome: "in_sync" };

// ---- advisor + chat (Phase 5, SDD §9) ----

export type AdvisorSeverity = "conflict" | "trend" | "tip";

export type AdvisorCta =
  | { kind: "planProduction"; item: string; rate: number }
  | { kind: "trace"; selection: string; id: Id }
  | { kind: "review"; proposal: Id };

export interface AdvisorCard {
  id: Id;
  severity: AdvisorSeverity;
  title: string;
  body: string;
  rule: string;
  saw: string;
  at: string;
  dismissed: boolean;
  cta?: AdvisorCta;
}

export interface AdvisorFeed {
  cards: AdvisorCard[];
  muted: string[];
  paused: boolean;
  callsThisHour: number;
  callBudget: number;
  aiStatus: "offline" | "ready";
}

export type ChatScope = { scope: "empire" } | { scope: "factory"; id: Id } | { scope: "selection"; id: Id };

export interface ChatReply {
  reply: string;
  causal: [string, string][];
  entities: [string, string, Id][];
  proposal: Id | null;
  saw: string;
  engine: string;
}

export interface ContextSnapshot {
  payload: unknown;
  bytes: number;
  snapshotTime: string;
}
