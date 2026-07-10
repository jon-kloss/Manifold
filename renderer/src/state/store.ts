// Zustand projected store — hydrated once, then patched by command responses.

import { create } from "zustand";
import { backend } from "./backend";
import { applyPatches } from "./patch";
import type {
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
};

const emptyDerived: Derived = { factories: {}, nodes: {}, totalPowerMw: 0 };

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
  overlays: { flows: boolean; nodes: boolean };
  /** T0 projection during slider drag — rendered italic, replaced on settle. */
  projected: { factoryId: Id; result: DerivedFactory; targetRate: number } | null;
  /** ids whose numbers changed in the last authoritative patch (settle flash). */
  settled: Set<string>;
  placingFactory: boolean;
  viewState: ViewState;

  hydrate(): Promise<void>;
  dispatch(cmds: Command[], opts?: { select?: boolean }): Promise<Id[]>;
  undo(): Promise<void>;
  redo(): Promise<void>;
  setSelection(sel: Selection): void;
  setView(view: ViewMode): void;
  setOverlay(key: "flows" | "nodes", on: boolean): void;
  setProjected(p: AppStore["projected"]): void;
  setPlacingFactory(on: boolean): void;
  saveViewState(patch: Partial<ViewState>): void;
}

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  error: null,
  plan: emptyPlan,
  derived: emptyDerived,
  gamedata: { items: {}, recipes: {}, machines: {}, belts: {}, buildVersion: "" },
  world: { version: 0, source: "", bounds: { minX: 0, minY: 0, maxX: 1, maxY: 1 }, regions: [], nodes: [] },
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  view: { mode: "map" },
  selection: null,
  overlays: { flows: true, nodes: true },
  projected: null,
  settled: new Set(),
  placingFactory: false,
  viewState: {},

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
      projected: null,
      settled,
    }));
    if (opts?.select && resp.created.length > 0) {
      const id = resp.created[0];
      const { plan } = get();
      if (plan.factories[id]) set({ selection: { kind: "factory", id } });
      else if (plan.groups[id]) set({ selection: { kind: "group", id } });
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
}));

/** Solve-time chip content for a factory (A4: always present, always honest). */
export function solveChip(df: DerivedFactory | undefined): { text: string; over: boolean } {
  if (!df) return { text: "SOLVE —", over: false };
  const ms = df.solveUs / 1000;
  const over = ms > 50;
  const text = `${over ? "SOLVE" : "LAST"} ${ms < 1 ? ms.toFixed(1) : ms.toFixed(0)}ms${over ? "" : " ✓"}`;
  return { text, over };
}
