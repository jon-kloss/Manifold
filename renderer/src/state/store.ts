// Zustand projected store — hydrated once, then patched by command responses.

import { create } from "zustand";
import { backend } from "./backend";
import { applyPatches } from "./patch";
import type {
  AdoptOutcome,
  AdvisorFeed,
  AltOpportunity,
  Command,
  CutoverPlan,
  Derived,
  DerivedFactory,
  EditResponse,
  GameData,
  Id,
  LastImport,
  Plan,
  ViewState,
  World,
} from "./types";

/** Human text for a rejected backend call (DomainError string or Error). */
export const errText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** Backend errors cite entity ids; users know names. Swap any ULID we can
 *  resolve for its display name (quoted), leave the rest untouched. */
const nameIds = (msg: string): string =>
  msg.replace(/[0-9A-HJKMNP-TV-Z]{26}/g, (id) => {
    const p = useStore.getState().plan;
    const group = p.groups[id];
    const name =
      p.factories[id]?.name ??
      (group ? `${p.factories[group.factory]?.name ?? "?"} machine bank` : undefined) ??
      (p.ports[id] ? `${p.ports[id].item.replace(/^Desc_|_C$/g, "")} port` : undefined) ??
      (p.routes[id] ? "route" : undefined) ??
      p.proposals[id]?.title;
    return name ? `"${name}"` : id;
  });

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
  buildOverrides: {},
  nodeOverrides: {},
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
  buildQueue: [],
  cutovers: [],
};

export interface AppStore {
  ready: boolean;
  error: string | null;
  /** last refused backend command — status-bar chip, NOT the full-screen
      BACKEND UNREACHABLE card (that is `error`, set only by hydrate). */
  cmdError: { message: string; at: number } | null;
  plan: Plan;
  derived: Derived;
  gamedata: GameData;
  world: World;
  canUndo: boolean;
  canRedo: boolean;
  undoLabel: string | null;
  view: ViewMode;
  selection: Selection;
  overlays: { flows: boolean; nodes: boolean; power: boolean; terrain: boolean };
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
  /** last save-import summary (W1c resume dashboard "what changed") */
  lastImport: LastImport | null;
  /** W2b: recipe classes the imported save has unlocked — gates alternate-recipe
      eligibility in the wizard/recipe pickers. Empty until a save is imported. */
  unlocked: Set<string>;
  /** resume dashboard overlay — auto-presents once per plan (viewState.resumeSeen) */
  dashboardOpen: boolean;

  hydrate(): Promise<void>;
  /** Resolves with the created ids, or null when the backend refused the
      commands (the refusal is recorded in `cmdError` — never a rejection). */
  dispatch(cmds: Command[], opts?: { select?: boolean }): Promise<Id[] | null>;
  reportCmdError(message: string): void;
  /** `at` must match the current error — a stale auto-clear timer armed for
      an older error must not dismiss a newer one. */
  clearCmdError(at: number): void;
  undo(): Promise<void>;
  redo(): Promise<void>;
  setSelection(sel: Selection): void;
  setView(view: ViewMode): void;
  setOverlay(key: "flows" | "nodes" | "power" | "terrain", on: boolean): void;
  setProjected(p: AppStore["projected"]): void;
  setPlacingFactory(on: boolean): void;
  saveViewState(patch: Partial<ViewState>): void;
  setReviewing(id: Id | null): void;
  setWizard(w: AppStore["wizard"]): void;
  acceptProposal(id: Id): Promise<void>;
  setAdvisor(feed: AdvisorFeed): void;
  setAdvisorOpen(open: boolean): void;
  setDashboardOpen(open: boolean): void;
  /** mark a build-queue step done/undone (manual override), or clear it back
      to derived with `null` — one undoable step (SetBuildDone). */
  markBuildDone(id: Id, done: boolean | null): Promise<void>;
  /** W2a: plan a whole-factory replacement → stores a Draft Refactor proposal
      and opens it in the review surface. Returns null on success, or the
      infeasibility/error reason string on failure so the caller can surface a
      clear inline notice at the point of action (not just the status chip). */
  planReplacement(factoryId: Id): Promise<string | null>;
  /** W2a: fetch a cutover's scratch-solved downtime on demand (or null on
      refusal — recorded in cmdError). */
  cutoverPlan(factoryId: Id): Promise<CutoverPlan | null>;
  /** W2b-D: fetch the empire alternate-recipe ranking (read-only; [] on refusal
      or when nothing is unlocked). */
  optimizeEmpire(): Promise<AltOpportunity[]>;
  /** W2b-D: adopt an alternate empire-wide → drafts the review proposal(s) and
      opens the first in review. Returns the outcome, or null on refusal. */
  optimizeAdopt(recipe: string): Promise<AdoptOutcome | null>;
}

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  error: null,
  cmdError: null,
  plan: emptyPlan,
  derived: emptyDerived,
  gamedata: { items: {}, recipes: {}, machines: {}, belts: {}, buildables: {}, buildVersion: "" },
  world: { version: 0, source: "", bounds: { minX: 0, minY: 0, maxX: 1, maxY: 1 }, regions: [], nodes: [] },
  canUndo: false,
  canRedo: false,
  undoLabel: null,
  view: { mode: "map" },
  selection: null,
  overlays: { flows: true, nodes: true, power: true, terrain: true },
  projected: null,
  settled: new Set(),
  placingFactory: false,
  viewState: {},
  planHash: "",
  reviewing: null,
  wizard: { open: false },
  advisor: { cards: [], muted: [], paused: false, callsThisHour: 0, callBudget: 6, aiStatus: "offline" },
  advisorOpen: false,
  lastImport: null,
  unlocked: new Set(),
  dashboardOpen: false,

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
        lastImport: init.lastImport ?? null,
        unlocked: new Set(init.unlocked ?? []),
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
    let resp: EditResponse;
    try {
      resp = await backend.edit(cmds);
    } catch (e) {
      get().reportCmdError(errText(e));
      return null;
    }
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
      cmdError: null,
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
    let resp: EditResponse | null;
    try {
      resp = await backend.undo();
    } catch (e) {
      get().reportCmdError(errText(e));
      return;
    }
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
      cmdError: null,
    }));
  },

  async redo() {
    let resp: EditResponse | null;
    try {
      resp = await backend.redo();
    } catch (e) {
      get().reportCmdError(errText(e));
      return;
    }
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
      cmdError: null,
    }));
  },

  reportCmdError(message) {
    set({ cmdError: { message: nameIds(message), at: Date.now() } });
  },

  clearCmdError(at) {
    set((s) => (s.cmdError?.at === at ? { cmdError: null } : {}));
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

  setDashboardOpen(open) {
    set({ dashboardOpen: open });
  },

  async markBuildDone(id, done) {
    // One undoable step: SetBuildDone upserts (Some) or clears (null) the
    // override; the response patches /buildOverrides and the derived queue.
    await get().dispatch([{ type: "set_build_done", id, done }]);
  },

  // Plan a replacement = one backend call that stores a Draft Refactor
  // proposal (◇-only) and lands it in review, exactly like an import drift.
  async planReplacement(factoryId) {
    let res: { response: EditResponse; proposal: Id };
    try {
      res = await backend.planReplacement(factoryId);
    } catch (e) {
      // Replacement infeasibility is an EXPECTED planning outcome, not a fault:
      // hand the reason back so the drawer can show a clear, actionable inline
      // notice at the button. The terse status-bar chip alone is insufficient.
      return errText(e);
    }
    const resp = res.response;
    set((s) => ({
      plan: applyPatches(s.plan, resp.patches),
      derived: resp.derived,
      canUndo: resp.canUndo,
      canRedo: resp.canRedo,
      undoLabel: resp.undoLabel,
      planHash: resp.planHash,
      advisor: resp.advisor,
      reviewing: res.proposal,
      selection: null,
      settled: new Set(resp.patches.map((p) => p.path)),
      cmdError: null,
    }));
    return null;
  },

  async cutoverPlan(factoryId) {
    try {
      return await backend.cutoverPlan(factoryId);
    } catch (e) {
      get().reportCmdError(errText(e));
      return null;
    }
  },

  // W2b-D: the optimizer is derived/advisory — a read-only fetch, never a
  // mutation. Empty in the fixture (no unlocked alternates) — honest.
  async optimizeEmpire() {
    try {
      return await backend.optimizeEmpire();
    } catch (e) {
      get().reportCmdError(errText(e));
      return [];
    }
  },

  // Adopt = draft the review proposal(s) (◇→T2, ◆→Refactor; ◆ never mutated),
  // re-hydrate so the new proposals land, and open the first in review.
  async optimizeAdopt(recipe) {
    let outcome: AdoptOutcome;
    try {
      outcome = await backend.optimizeAdopt(recipe);
    } catch (e) {
      get().reportCmdError(errText(e));
      return null;
    }
    await get().hydrate();
    const first = outcome.proposals[0] ?? null;
    set({ reviewing: first, selection: null });
    if (outcome.note) get().reportCmdError(outcome.note);
    return outcome;
  },

  // Accept = one backend transaction, one undo entry, ◇ entities only.
  async acceptProposal(id) {
    let resp: EditResponse;
    try {
      resp = await backend.proposalAccept(id);
    } catch (e) {
      // review surface stays open — the user decides what to do with the draft
      get().reportCmdError(errText(e));
      return;
    }
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
      cmdError: null,
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
