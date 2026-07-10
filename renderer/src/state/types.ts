// TS mirror of the Rust projection types (planner-core + session).
// The renderer is a projection patched by events — never a source of truth.

export type Id = string;
export type Status = "planned" | "under_construction" | "built";
export type CreatedBy = { kind: "manual" } | { kind: "proposal"; id: Id } | { kind: "import"; id: Id };

export interface MapPos { x: number; y: number }
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

export type EdgeEnd = { kind: "group"; id: Id } | { kind: "port"; id: Id };

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

export interface Plan {
  meta: PlanMeta;
  factories: Record<Id, Factory>;
  groups: Record<Id, MachineGroup>;
  ports: Record<Id, Port>;
  edges: Record<Id, BeltEdge>;
  nodeClaims: Record<Id, NodeClaim>;
  routes: Record<Id, unknown>;
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

export interface GameData {
  items: Record<string, GameItem>;
  recipes: Record<string, GameRecipe>;
  machines: Record<string, GameMachine>;
  belts: Record<string, GameBelt>;
  buildVersion: string;
}

// ---- world snapshot ----

export interface WorldNode { id: string; item: string; purity: "pure" | "normal" | "impure"; x: number; y: number; region: string }
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

export interface Derived {
  factories: Record<Id, DerivedFactory>;
  nodes: Record<string, { claims: number; conflict: boolean }>;
  totalPowerMw: number;
}

// ---- IPC ----

export type PatchOp =
  | { op: "add"; path: string; value: unknown }
  | { op: "replace"; path: string; value: unknown }
  | { op: "remove"; path: string };

export interface EditResponse {
  patches: PatchOp[];
  derived: Derived;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  created: Id[];
}

export interface InitPayload {
  plan: Plan;
  derived: Derived;
  gamedata: GameData;
  world: World;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  viewState: ViewState | null;
}

export interface ViewState {
  map?: { center: [number, number]; zoom: number };
  openFactory?: Id | null;
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
  | { type: "set_edge_tier"; id: Id; tier: number }
  | { type: "delete_edge"; id: Id }
  | { type: "claim_node"; factory: Id; node: string; extractor: string; clock: number }
  | { type: "release_node"; id: Id }
  | { type: "rename_plan"; name: string };

export const BELT_CAPACITY = [60, 120, 270, 480, 780, 1200];
export const beltCapacity = (tier: number) => BELT_CAPACITY[Math.min(6, Math.max(1, tier)) - 1];
