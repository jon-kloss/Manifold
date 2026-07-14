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
  /** W2a refactor link: when this ◇ factory is a planned replacement for a
   *  running ◆ factory, `replaces` names that old factory's id (a planner-side
   *  label; the cutover/downtime are DERIVED, the ◆ is never mutated). */
  replaces?: Id | null;
  status: Status;
  createdBy: CreatedBy;
}

/** Planned overlay on a ◆ built group (SDD §3.1.1): each component is the
 *  planned effective value; null/absent means "track the built baseline". */
export interface GroupDelta {
  count?: number | null;
  clock?: number | null;
}

export interface MachineGroup {
  id: Id;
  factory: Id;
  machine: string;
  recipe: string;
  count: number;
  clock: number;
  somersloops: number;
  /** Baseline count/clock stay game ground truth; edits on ◆ land here. */
  plannedDelta: GroupDelta | null;
  graphPos: GraphPos;
  /** Vertical factory floor (0 = ground). */
  floor: number;
  status: Status;
  createdBy: CreatedBy;
}

/** Count the solver plans with: delta overlay if present, else baseline. */
export const effCount = (g: MachineGroup): number => g.plannedDelta?.count ?? g.count;
/** Clock the solver plans with: delta overlay if present, else baseline. */
export const effClock = (g: MachineGroup): number => g.plannedDelta?.clock ?? g.clock;

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
  /** resolved node id: a bundled-snapshot WorldNode id, or `save:<id>` for a
   *  miner on no known catalog node (W2b-C). */
  node: string;
  factory: Id;
  extractor: string;
  clock: number;
  /** the save's stable node ref this claim was bound from (re-match key). */
  saveNodeId?: string | null;
  status: Status;
  createdBy: CreatedBy;
}

/** Plan-local correction of a resource node's geometry (W2b-C). Sparse overlay
 *  keyed by node id; the bundled catalog stays an ambient default (resolved =
 *  snapshot ⊕ override). Purity is NOT correctable — snapshot-primary. */
export interface NodeOverride {
  id: string;
  pos?: MapPos | null;
  saveActor?: string | null;
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
  /** W1c manual build-queue completion overrides (sparse assertion overlay) */
  buildOverrides: Record<Id, BuildOverride>;
  /** W2b-C plan-local resource-node corrections (snapshot ⊕ override) */
  nodeOverrides: Record<string, NodeOverride>;
}

/** Manual completion assertion for a build-queue step (W1c) — present only
 *  when the user hand-checked/unchecked a step; auto-dissolves on re-import. */
export interface BuildOverride {
  id: Id;
  done: boolean;
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
  /** Average sustained draw override for variable-power machines (absent for fixed-power recipes). */
  variablePowerMw?: number | null;
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
  /** Schematic class → recipe classes it unlocks (W2b; empty on trimmed catalogs). */
  schematics?: Record<string, string[]>;
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
  | { kind: "input_ceiling"; port: Id; item: string; ceiling: number }
  | { kind: "disconnected"; node: Id; item: string };

export interface TargetCeiling { maxRate: number; binding: Constraint }

/** Unmet output target on a degraded solve (SDD §5.2 — never a dead end). */
export interface Shortfall { requested: number; missing: number; binding: Constraint | null }

export interface DerivedGroup { inRates: Record<string, number>; outRates: Record<string, number>; powerMw: number }
export interface DerivedEdge { flow: number; saturation: number }

export interface DerivedFactory {
  groups: Record<Id, DerivedGroup>;
  edges: Record<Id, DerivedEdge>;
  ports: Record<Id, number>;
  /** Unmet output targets — ports carry the achieved rates when present. */
  shortfalls?: Record<Id, Shortfall>;
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

/** Task #49 train answer-sheet — trains-needed for a route, from the same
 *  transport math. Returned by the read-only `routeCalc` backend call for a
 *  PROSPECTIVE route, and composed client-side for an existing one. */
export interface TrainAnswer {
  math: TransportMath;
  /** Throughput of ONE consist/truck/drone at these specs (items/min). */
  perTrainPerMin: number;
  /** ceil(demand ÷ per-train) — the headline answer. */
  trainsNeeded: number;
  demandPerMin: number;
  /** Throughput at the configured unit count − demand; negative ⇒ short. */
  surplusPerMin: number;
  /** The configured fleet can't meet demand. */
  short: boolean;
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

/** W1c build queue — a DERIVED projection: each step is an existing ◇ planned
 *  (or partially-built) entity, completion derived from the ◆ built layer. */
export type BuildStepState = "pending" | "partial" | "done";
export type BuildStepKind = "factory" | "group" | "route" | "claim";

/** Milestone "built so far" (◆ production of the item) against the game total. */
export interface BuildProgress {
  item: string;
  built: number;
  total: number;
}

export interface BuildStep {
  id: Id;
  kind: BuildStepKind;
  /** owning factory, for "go there" navigation (null on map-level routes) */
  factory: Id | null;
  label: string;
  detail: string;
  /** derived completion, ignoring the override — drives the ◇◈◆ glyph */
  state: BuildStepState;
  /** resolved answer: override ?? (state === "done") */
  done: boolean;
  /** a manual BuildOverride is pinning `done` */
  overridden: boolean;
  /** completion can't be auto-detected (routes/claims) — check is manual */
  manualOnly: boolean;
  /** ordering key: creating proposal's number, 0 for MANUAL/import */
  number: number;
  progress?: BuildProgress;
}

/** W2a cutover — a DERIVED refactor overlay: a ◇ replacement linked to the ◆ it
 *  retires, with ordered BuildNew → Switch → Dismantle steps. The ◆ layer is
 *  never mutated; dismantle completion is derived from it. */
export type CutoverPhase = "build_new" | "switch" | "dismantle";

export interface CutoverStep {
  id: Id;
  phase: CutoverPhase;
  /** owning factory, for "go there" navigation */
  factory: Id | null;
  label: string;
  detail: string;
  /** derived completion, ignoring the override — drives the ◇◈◆ glyph */
  state: BuildStepState;
  done: boolean;
  overridden: boolean;
  /** completion can't be auto-detected (Switch steps) — check is manual */
  manualOnly: boolean;
}

export interface Cutover {
  newFactory: Id;
  newName: string;
  oldFactory: Id;
  oldName: string;
  steps: CutoverStep[];
  /** the new ◇ reuses a node the old ◆ still holds → unavoidable downtime */
  nodeReuse: boolean;
  number: number;
}

/** A single tracked-item production dip at a phase boundary. `rate`/`baseline`
 *  are COMPUTED (scratch-solve output); `estHours` is the labeled estimate. */
export interface Dip {
  /** boundary: 1 = Switch, 2 = Dismantle */
  phase: number;
  item: string;
  rate: number;
  baseline: number;
  estHours: number;
}

/** On-demand, scratch-solved downtime pricing for one cutover (ripple-inclusive).
 *  Fetched via cutoverPlan(factoryId) — never part of the per-edit derived. */
export interface CutoverPlan {
  newFactory: Id;
  oldFactory: Id;
  tracked: string[];
  baseline: Record<string, number>;
  production: Record<string, number>[];
  dips: Dip[];
  /** node reuse: unavoidable downtime for the build window */
  hard: boolean;
  /** whether the downtime could be COMPUTED. false when the old factory declares
   *  positive output but the scratch-solve baseline is ~0 (imported/unsolved) —
   *  distinguishes "no impact" from "can't compute". Transient (derived). */
  downtimeAvailable: boolean;
  /** human reason set when downtimeAvailable is false (else null) */
  unavailableReason: string | null;
}

/** W2b-D empire alternate-recipe optimizer: one ranked adopt-everywhere
 *  opportunity. Derived/advisory — fetched via optimizeEmpire(), never part of
 *  the per-edit derived, and empty in the fixture (no unlocked alternates). */
export interface AltOpportunity {
  recipe: string;
  recipeName: string;
  product: string;
  productName: string;
  /** Σ machines current − Σ machines alt (positive = the alt is cheaper). */
  machinesSaved: number;
  powerSavedMw: number;
  /** net per-input change (positive = the alt consumes more of that item). */
  inputDeltas: [string, number][];
  /** ◇ planned group ids retooled in place; ◆ built group ids route to Refactor. */
  affectedPlanned: Id[];
  affectedBuilt: Id[];
  retoolEstHours: number;
  nodeReuse: boolean;
}

/** Result of adopting an alternate empire-wide: the drafted review proposal(s)
 *  (T2 for ◇, W2a Refactor for ◆), plus any relayed infeasibility. */
export interface AdoptOutcome {
  proposals: Id[];
  route: "t2" | "refactor";
  note: string | null;
}

export interface Derived {
  factories: Record<Id, DerivedFactory>;
  nodes: Record<string, { claims: number; conflict: boolean; drift: boolean }>;
  routes: Record<Id, DerivedRoute>;
  deficits: DeficitRow[];
  circuits: DerivedCircuit[];
  totalGenerationMw: number;
  empireCycle: boolean;
  recomputeUs: number;
  totalPowerMw: number;
  /** ordered ◇ planned / partially-built steps with resolved completion */
  buildQueue: BuildStep[];
  /** W2a cutovers: lightweight presence/steps (downtime is fetched on demand) */
  cutovers: Cutover[];
}

// ---- IPC ----

export type PatchOp =
  | { op: "add"; path: string; value: unknown }
  | { op: "replace"; path: string; value: unknown }
  | { op: "remove"; path: string };

// ---- proposals (Phase 3): reviewable, partially-acceptable change sets ----

export type ProposalStatus = "draft" | "reviewing" | "accepted" | "rejected";
export type ProposalSource =
  | "global_solver"
  | "t2_optimize"
  | "advisor"
  | "chat"
  | "save_reimport"
  | "refactor";
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

/** Total-quantity goal target carried alongside a proposal (goal-mode). The
 *  solver never reads it — the rate drives the plan; the review surface
 *  annotates the target and its time-at-rate. */
export interface Milestone {
  item: string;
  total: number;
  rate: number;
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
  /** optional total-quantity target (goal-mode); absent on legacy plans */
  milestone?: Milestone;
}

export interface GoalCheck { item: string; requested: number; achieved: number }

/** Per-grid before→after power for a touched circuit (mock 3a review banner). */
export interface CircuitImpact {
  name: string;
  demandBeforeMw: number;
  demandAfterMw: number;
  generationBeforeMw: number;
  generationAfterMw: number;
  headroomAfter: number;
  level: "ok" | "warn" | "crit";
}

export interface ProposalConsequence {
  goal: GoalCheck[];
  goalMet: boolean;
  deltaPowerMw: number;
  deltaGenerationMw: number;
  machines: number;
  warnings: string[];
  circuitImpacts: CircuitImpact[];
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
  /** total-quantity goal mode; passed through the solver into the proposal */
  milestone?: Milestone;
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
  /** last save-import summary (W1c "what changed since last import") */
  lastImport: LastImport | null;
  /** W2b: recipe classes the imported save has unlocked (mPurchasedSchematics ×
      FGSchematic unlocks). Save-derived, outside the undo journal; [] until a
      save with schematics is imported. Gates alternate-recipe eligibility. */
  unlocked: string[];
}

/** Session fact: what the most recent save import did (W1c resume dashboard). */
export interface LastImport {
  at: string;
  saveName: string;
  outcome: "imported" | "drift" | "in_sync";
  factoriesAdded: number;
  groupsChanged: number;
}

export interface ViewState {
  map?: { center: [number, number]; zoom: number };
  openFactory?: Id | null;
  /** first-run card dismissed */
  onboarded?: boolean;
  /** resume dashboard has been auto-presented for this plan (persisted, like
      `onboarded`) — so it greets once and never ambushes the restored map. */
  resumeSeen?: boolean;
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
  | { type: "tidy_layout"; factory: Id }
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
  | { type: "set_factory_theme"; factory: Id; styleGuide: Id | null }
  | { type: "set_build_done"; id: Id; done: boolean | null }
  | { type: "set_factory_replaces"; id: Id; replaces: Id | null }
  | { type: "set_node_override"; id: string; nodeOverride: NodeOverride | null };

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
  /** Extractors only (W2b node context): stable ref to the resource node /
   *  water volume this sits on, for re-match on re-import. */
  nodeActorId?: string;
  /** Resource item (Desc_…) — not carried in the save; null until the world
   *  catalog supplies it downstream. */
  resource?: string | null;
  /** Node purity — not carried in the save; null (snapshot-primary purity). */
  purity?: string | null;
  /** Extraction rate items/min — not exposed by the parser; absent. */
  extractionRate?: number;
}

export interface ImportSnapshot {
  saveName: string;
  buildVersion?: string;
  machines: ImportMachine[];
  extractors?: ImportMachine[];
  /** Purchased/unlocked schematic class names (W2b alt-awareness); [] if the
   *  schematic manager actor is absent. */
  unlockedSchematics: string[];
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
