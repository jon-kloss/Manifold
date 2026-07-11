// Zustand projected store — hydrated once, then patched by command responses.

import { create } from "zustand";
import { backend } from "./backend";
import { applyPatches } from "./patch";
import type {
  AdvisorFeed,
  Command,
  Derived,
  DerivedFactory,
  GameData,
  Id,
  Plan,
  ViewState,
  World,
} from "./types";

export type Selection =
  | { kind: "factory"; id: Id }
  | { kind: "node"; id: string }
  | { kind: "group"; id: Id }
  | { kind: "edge"; id: Id }
  | { kind: "port"; id: Id }
  | { kind: "junction"; id: Id }
  | { kind: "route"; id: Id }
  | { kind: "switch"; id: Id }
  | null;

export type ViewMode = { mode: "map" } | { mode: "factory"; factoryId: Id };

const emptyPlan: Plan = {
  meta: { schemaVersion: 1, gameBuild: "", name: "" },
  factories: {},
  groups: {},
  ports: {},
  edges: {},
  nodeClaims: {},
  routes: {},
  junctions: {},
  proposals: {},
  switches: {},
  styleGuides: {},
};

const emptyDerived: Derived = {
  factories: {},
  nodes: {},
  routes: {},
  deficits: [],
  circuits: [],
  totalGenerationMw: 0,
  empireCycle: false,
  recomputeUs: 0,
  totalPowerMw: 0,
};

export interface AppStore {
  ready: boolean;
  error: string | null;
  plan: Plan;
  derived: Derived;
  gamedata: GameData;
  world: World;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  view: ViewMode;
  selection: Selection;
  overlays: { flows: boolean; nodes: boolean; power: boolean };
  /** T0 projection during slider drag — rendered italic, replaced on settle. */
  projected: { factoryId: Id; result: DerivedFactory; targetRate: number } | null;
  /** ids whose numbers changed in the last authoritative patch (settle flash). */
  settled: Set<string>;
  placingFactory: boolean;
  viewState: ViewState;
  /** plan-content hash — compare a proposal's inputHash for the STALE badge */
  planHash: string;
  /** proposal id currently open in the review surface */
  reviewing: Id | null;
  /** wizard modal: closed | open (optionally pre-filled from FIX WITH SOLVER) */
  wizard: { open: boolean; prefill?: { item: string; rate: number } };
  /** ambient advisor feed (updated by every backend response) */
  advisor: AdvisorFeed;
  advisorOpen: boolean;

  hydrate(): Promise<void>;
  dispatch(cmds: Command[], opts?: { select?: boolean }): Promise<Id[]>;
  undo(): Promise<void>;
  redo(): Promise<void>;
  setSelection(sel: Selection): void;
  setView(view: ViewMode): void;
  setOverlay(key: "flows" | "nodes" | "power", on: boolean): void;
  setProjected(p: AppStore["projected"]): void;
  setPlacingFactory(on: boolean): void;
  saveViewState(patch: Partial<ViewState>): void;
  setReviewing(id: Id | null): void;
  setWizard(w: AppStore["wizard"]): void;
  acceptProposal(id: Id): Promise<void>;
  setAdvisor(feed: AdvisorFeed): void;
  setAdvisorOpen(open: boolean): void;
}

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  error: null,
  plan: emptyPlan,
  derived: emptyDerived,
  gamedata: { items: {}, recipes: {}, machines: {}, belts: {}, buildables: {}, buildVersion: "" },
  world: { version: 0, source: "", bounds: { minX: 0, minY: 0, maxX: 1, maxY: 1 }, regions: [], nodes: [] },
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  view: { mode: "map" },
  selection: null,
  overlays: { flows: true, nodes: true, power: true },
  projected: null,
  settled: new Set(),
  placingFactory: false,
  viewState: {},
  planHash: "",
  reviewing: null,
  wizard: { open: false },
  advisor: { cards: [], muted: [], paused: false, callsThisHour: 0, callBudget: 6, aiStatus: "offline" },
  advisorOpen: false,

  async hydrate() {
    try {
      const init = await backend.hydrate();
      const openFactory = init.viewState?.openFactory;
      set({
        ready: true,
        plan: init.plan,
        derived: init.derived,
        gamedata: init.gamedata,
        world: init.world,
        canUndo: init.canUndo,
        canRedo: init.canRedo,
        undoLabel: init.undoLabel,
        planHash: init.planHash,
        advisor: init.advisor,
        viewState: init.viewState ?? {},
        view:
          openFactory && init.plan.factories[openFactory]
            ? { mode: "factory", factoryId: openFactory }
            : { mode: "map" },
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  async dispatch(cmds, opts) {
    const resp = await backend.edit(cmds);
    const settled = new Set<string>(
      resp.patches.map((p) => p.path).filter((p) => !p.startsWith("/meta")),
    );
    set((s) => ({
      plan: applyPatches(s.plan, resp.patches),
      derived: resp.derived,
      canUndo: resp.canUndo,
      canRedo: resp.canRedo,
      undoLabel: resp.undoLabel,
      planHash: resp.planHash,
      advisor: resp.advisor,
      projected: null,
      settled,
    }));
    if (opts?.select && resp.created.length > 0) {
      const id = resp.created[0];
      const { plan } = get();
      if (plan.factories[id]) set({ selection: { kind: "factory", id } });
      else if (plan.groups[id]) set({ selection: { kind: "group", id } });
      else if (plan.junctions[id]) set({ selection: { kind: "junction", id } });
      else if (plan.switches[id]) set({ selection: { kind: "switch", id } });
    }
    return resp.created;
  },

  async undo() {
    const resp = await backend.undo();
    if (!resp) return;
    set((s) => ({
      plan: applyPatches(s.plan, resp.patches),
      derived: resp.derived,
      canUndo: resp.canUndo,
      canRedo: resp.canRedo,
      undoLabel: resp.undoLabel,
      planHash: resp.planHash,
      advisor: resp.advisor,
      projected: null,
      settled: new Set(resp.patches.map((p) => p.path)),
    }));
  },

  async redo() {
    const resp = await backend.redo();
    if (!resp) return;
    set((s) => ({
      plan: applyPatches(s.plan, resp.patches),
      derived: resp.derived,
      canUndo: resp.canUndo,
      canRedo: resp.canRedo,
      undoLabel: resp.undoLabel,
      planHash: resp.planHash,
      advisor: resp.advisor,
      projected: null,
      settled: new Set(resp.patches.map((p) => p.path)),
    }));
  },

  setSelection: (selection) => set({ selection }),
  setView: (view) => {
    set({ view, selection: null });
    get().saveViewState({ openFactory: view.mode === "factory" ? view.factoryId : null });
  },
  setOverlay: (key, on) => set((s) => ({ overlays: { ...s.overlays, [key]: on } })),
  setProjected: (projected) => set({ projected }),
  setPlacingFactory: (placingFactory) => set({ placingFactory }),
  saveViewState(patch) {
    const next = { ...get().viewState, ...patch };
    set({ viewState: next });
    void backend.setViewState(next);
  },

  setReviewing(id) {
    set({ reviewing: id, selection: null });
  },

  setWizard(w) {
    set({ wizard: w });
  },

  setAdvisor(feed) {
    set({ advisor: feed });
  },

  setAdvisorOpen(open) {
    set({ advisorOpen: open });
  },

  // Accept = one backend transaction, one undo entry, ◇ entities only.
  async acceptProposal(id) {
    const resp = await backend.proposalAccept(id);
    set((s) => ({
      plan: applyPatches(s.plan, resp.patches),
      derived: resp.derived,
      canUndo: resp.canUndo,
      canRedo: resp.canRedo,
      undoLabel: resp.undoLabel,
      planHash: resp.planHash,
      advisor: resp.advisor,
      reviewing: null,
      settled: new Set(resp.patches.map((p) => p.path)),
    }));
  },
}));

/** Solve-time chip content for a factory (A4: always present, always honest). */
export function solveChip(df: DerivedFactory | undefined): { text: string; over: boolean } {
  if (!df) return { text: "SOLVE —", over: false };
  const ms = df.solveUs / 1000;
  const over = ms > 50;
  const text = `${over ? "SOLVE" : "LAST"} ${ms < 1 ? ms.toFixed(1) : ms.toFixed(0)}ms${over ? "" : " ✓"}`;
  return { text, over };
}
