// Zustand projected store — hydrated once, then patched by command responses.

import { create } from "zustand";
import { backend } from "./backend";
import { parseSaveFile } from "../import/parseSave";
import { driftConflictCount } from "../import/saveHandle";
import { applyPatches } from "./patch";
import { friendlyError } from "./errors";
import {
  DEFAULT_WEBLLM_MODEL,
  deleteModelFromCache,
  engineReady,
  loadEngine,
  runRank,
  unloadEngine,
  webgpuSupported,
} from "../ai/webllm";
import type {
  AdoptOutcome,
  AdvisorFeed,
  AiConfigPublic,
  AiConfigUpdate,
  AiSettingsContext,
  AltOpportunity,
  AuditTab,
  Command,
  CutoverPlan,
  Derived,
  DerivedFactory,
  EditResponse,
  GameData,
  Id,
  ImportOutcome,
  LastImport,
  NextPreferences,
  Opportunity,
  Plan,
  RankResponse,
  ViewState,
  World,
} from "./types";

/** Human text for a rejected backend call (DomainError string or Error). */
export const errText = (e: unknown): string => (e instanceof Error ? e.message : String(e));

// On-device AI opt-in persists in localStorage — a device/browser capability
// choice (WebGPU + origin-cached weights), NOT plan data, so it must survive a
// plan switch and live outside the wasm ViewState. Reads are guarded: a private
// window with storage blocked simply reads the default (off), never throws.
const WEBLLM_ENABLED_KEY = "ficsit.webllm.enabled";
const WEBLLM_MODEL_KEY = "ficsit.webllm.model";
// Tracks whether the ~0.9 GB weights are present in browser cache, so the UI can
// offer "remove download" (to reclaim space) independently of the on/off toggle.
// Set once a load completes; cleared on removal. A flag, not a cache probe, so
// boot never imports the (heavy) library just to know — worst case, a browser
// eviction leaves it stale and the remove button does a harmless no-op delete.
const WEBLLM_DOWNLOADED_KEY = "ficsit.webllm.downloaded";
function readWebllmEnabled(): boolean {
  try {
    return localStorage.getItem(WEBLLM_ENABLED_KEY) === "1";
  } catch {
    return false;
  }
}
function readWebllmModel(): string {
  try {
    return localStorage.getItem(WEBLLM_MODEL_KEY) || DEFAULT_WEBLLM_MODEL;
  } catch {
    return DEFAULT_WEBLLM_MODEL;
  }
}
function readWebllmDownloaded(): boolean {
  try {
    return localStorage.getItem(WEBLLM_DOWNLOADED_KEY) === "1";
  } catch {
    return false;
  }
}
function persistWebllm(enabled: boolean, model: string): void {
  try {
    localStorage.setItem(WEBLLM_ENABLED_KEY, enabled ? "1" : "0");
    localStorage.setItem(WEBLLM_MODEL_KEY, model);
  } catch {
    /* storage blocked (private window) — the in-memory choice still works this session */
  }
}
// Auto-sync opt-in + interval persist in localStorage — a device/browser
// capability choice (it re-reads a retained File System Access handle, which
// lives in this origin's IndexedDB), NOT plan data. Guarded reads default off.
const AUTOSYNC_ENABLED_KEY = "ficsit.autosync.enabled";
const AUTOSYNC_INTERVAL_KEY = "ficsit.autosync.intervalMin";
const DEFAULT_AUTOSYNC_MIN = 5;
function readAutoSyncEnabled(): boolean {
  try {
    return localStorage.getItem(AUTOSYNC_ENABLED_KEY) === "1";
  } catch {
    return false;
  }
}
function readAutoSyncInterval(): number {
  try {
    const n = Number(localStorage.getItem(AUTOSYNC_INTERVAL_KEY));
    // ≥ 1 min: a tampered sub-minute value would hammer the parser every tick.
    return Number.isFinite(n) && n >= 1 ? n : DEFAULT_AUTOSYNC_MIN;
  } catch {
    return DEFAULT_AUTOSYNC_MIN;
  }
}
function persistAutoSync(enabled: boolean, intervalMin: number): void {
  try {
    localStorage.setItem(AUTOSYNC_ENABLED_KEY, enabled ? "1" : "0");
    localStorage.setItem(AUTOSYNC_INTERVAL_KEY, String(intervalMin));
  } catch {
    /* storage blocked (private window) — the in-memory choice still works this session */
  }
}

function persistWebllmDownloaded(downloaded: boolean): void {
  try {
    localStorage.setItem(WEBLLM_DOWNLOADED_KEY, downloaded ? "1" : "0");
  } catch {
    /* storage blocked — in-memory flag still works this session */
  }
}

/** On-device AI (WebLLM) opt-in state. `enabled`/`model` persist in
 *  localStorage (a device capability choice, not plan data); the rest is
 *  live engine status the settings UI + status chip read. */
export type WebllmPhase = "idle" | "loading" | "ready" | "error";
export interface WebllmState {
  /** user opted in (persisted). The weights only download once true. */
  enabled: boolean;
  /** WebGPU present — false hides the offer with an honest note. */
  supported: boolean;
  phase: WebllmPhase;
  /** download/compile fraction 0..1 while `phase === "loading"`. */
  progress: number;
  progressText: string;
  /** selected on-device model id (persisted). */
  model: string;
  /** weights are present in browser cache (persisted flag) — gates the
      "remove download" affordance, which reclaims the ~0.9 GB. */
  downloaded: boolean;
  error: string | null;
}

/** Transient action-feedback notice. */
export type ToastKind = "success" | "error" | "info";
export interface Toast {
  id: number;
  message: string;
  kind: ToastKind;
}

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
      // Belts (edges) and junctions were missing here, so belt errors leaked a
      // raw ULID (e.g. "set tier" on a built belt).
      (p.edges[id] ? `${p.edges[id].item.replace(/^Desc_|_C$/g, "")} belt` : undefined) ??
      (p.junctions[id] ? "junction" : undefined) ??
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

export type AdvisorTab = "feed" | "chat" | "next";

const emptyPlan: Plan = {
  meta: { schemaVersion: 1, gameBuild: "", name: "", preferences: { noTrains: false, ignorePower: false } },
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
  /** MANIFOLD boot progress (handoff §4a/§7): REAL loader stages only —
   *  the ticker narrates these verbatim and the expanding bus follows
   *  `fraction`; no synthetic timers anywhere in this model. */
  boot: { stage: string; fraction: number };
  /** Verb of the last plan mutation (MANIFOLD motion 7h/7k/7l/7m): the graph
   *  diffs entity ids itself; this only says WHICH grammar the change plays —
   *  blueprint-build for edits, ghost/pop for undo/redo. `at` keys freshness
   *  so stale verbs never animate a later render. Visual-only — never gates
   *  data. */
  motion: { kind: "edit" | "undo" | "redo"; at: number; hash: string } | null;
  /** last refused backend command — status-bar chip, NOT the full-screen
      BACKEND UNREACHABLE card (that is `error`, set only by a failed FIRST
      boot; live re-hydrate failures route here instead). */
  cmdError: { message: string; at: number } | null;
  /** transient action-feedback toasts (auto-dismiss); newest last */
  toasts: Toast[];
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
  /** Live map node filter (search box). Empty = no filter (all nodes shown). */
  mapFilter: string;
  /** Live factory-graph filter (header search): matching machines/items stay
   *  lit, everything else dims. Empty = no filter. */
  graphFilter: string;
  /** Pending .sav awaiting the import review modal (set by the DATA menu's
   *  picker or a drag-drop onto the map). */
  importFile: File | null;
  /** A Docs.json upload is in flight — the DATA button shows progress. */
  uploadingDocs: boolean;
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
  /** PR 10 (review M7): App's Escape-deference flag for the AI-settings
      popover — the same pattern as `wizard.open`/`reviewing`. The popover
      layers OVER the dashboard, but App's capture-phase window handler fires
      before any later-registered listener, so the popover registers its
      open-ness here and App closes IT first (one Escape, one layer). Cleared
      by AiSettings on unmount so the flag never leaks across dashboard
      closes.
      M2: SCOPED by context — the dashboard NEXT header and the panel NEXT
      header each mount an <AiSettings/>, so a single boolean would cross-wire
      them (one click opens both, unmounting one slams the other shut). The
      flag names the owning header ("dashboard" | "panel"); an instance is open
      only when it equals its own context, and its unmount clears only when it
      still owns the flag. */
  aiSettingsOpen: AiSettingsContext | null;
  /** pending "open the audit drawer on this tab" request (PR 9 openAudit
      action). The drawer's open flag lives in App.tsx local state, so this is
      the one store-visible signal: App opens the drawer when it appears, the
      drawer selects the tab and clears it. */
  auditRequest: AuditTab | null;
  /** pending "pan the map camera here" request (PR 9 NEXT MOVES SHOW
      actions). Same consume-and-clear idiom as auditRequest: ONLY the
      dashboard's actMove sets it (map clicks/search/drawers never pan through
      this path), MapView pans and clears it. Living in the store — not a prop
      — lets the request survive the MapView remount when a SHOW lands while
      the app is in graph view. */
  flyTo: { x: number; y: number } | null;
  /** PR 10: public model-config view (hasKey, never the key) — fetched lazily
      when the dashboard opens; null until then / on refusal. */
  aiConfig: AiConfigPublic | null;
  /** PR 3: the SHARED ranked next-move state — a single owner both the
      dashboard and the docked advisor NEXT tab render, so they never
      double-bill the provider or disagree. null until a feed first opens (the
      status-bar chip stays hidden until then — no flash). */
  rank: RankResponse | null;
  /** bumped on a config save or a preference toggle → open feeds re-rank. */
  rankEpoch: number;
  /** which advisor-panel tab is active; the status-bar NEXT chip deep-links
      here by setting it to "next". */
  advisorTab: AdvisorTab;
  /** On-device AI (WebLLM) opt-in + engine status. See {@link WebllmState}. */
  webllm: WebllmState;

  hydrate(): Promise<void>;
  /** Web Phase 4a: upload a real Docs.json (raw bytes) for the browser session,
      then re-hydrate so the richer catalog is live. Resolves true on success;
      false when the backend refused (recorded in `cmdError`, never a rejection).
      Web-only — non-wasm backends reject and this surfaces the refusal. */
  uploadDocs(bytes: Uint8Array): Promise<boolean>;
  /** Wipe the current plan (KEEPING the gamedata catalog) and re-hydrate to an
      empty empire — the "start over" before importing a fresh, unrelated save.
      A cross-platform Session::new_empire over every transport. Resolves true on
      success; false on a backend error (recorded in `cmdError`, never a
      rejection). */
  newEmpire(): Promise<boolean>;
  /** Sync Phase 2: parse a re-read `.sav` and run it through import — headless
      (no preview modal), the one-click counterpart to the ImportModal flow.
      Drift opens the review surface; every branch fires a toast. Resolves with
      the outcome, or null when the parse/import failed (surfaced as a toast). */
  syncImport(file: File): Promise<ImportOutcome | null>;
  /** Sync Phase 3: auto-pull opt-in (persisted, device-scoped) + its interval. */
  autoSync: { enabled: boolean; intervalMin: number };
  /** Toggle auto-sync and/or change its interval (persists both). */
  setAutoSync(enabled: boolean, intervalMin?: number): void;
  /** Sync Phase 3 (Option B): headless auto-pull — conflict-free drift applies
      silently; a drift with real conflicts opens review instead; an in-sync
      save is a quiet no-op (no toast). Resolves with the outcome, or null on a
      read/parse failure. */
  autoPull(file: File): Promise<ImportOutcome | null>;
  /** Resolves with the created ids, or null when the backend refused the
      commands (the refusal is recorded in `cmdError` — never a rejection). */
  dispatch(cmds: Command[], opts?: { select?: boolean }): Promise<Id[] | null>;
  reportCmdError(message: string): void;
  /** `at` must match the current error — a stale auto-clear timer armed for
      an older error must not dismiss a newer one. */
  clearCmdError(at: number): void;
  /** Push a transient toast (auto-dismisses). For action feedback the UI
      otherwise swallows — uploads, clipboard copies, background successes. */
  pushToast(message: string, kind?: ToastKind): void;
  dismissToast(id: number): void;
  /** Clear all toasts (e.g. a file drag starts, so they can't block the drop). */
  clearToasts(): void;
  undo(): Promise<void>;
  redo(): Promise<void>;
  setSelection(sel: Selection): void;
  setView(view: ViewMode): void;
  setOverlay(key: "flows" | "nodes" | "power" | "terrain", on: boolean): void;
  setMapFilter(query: string): void;
  setGraphFilter(query: string): void;
  setImportFile(f: File | null): void;
  setProjected(p: AppStore["projected"]): void;
  setPlacingFactory(on: boolean): void;
  saveViewState(patch: Partial<ViewState>): void;
  setReviewing(id: Id | null): void;
  setWizard(w: AppStore["wizard"]): void;
  /** Resolves true when the accept applied; false when the backend refused it
      (the refusal is surfaced via `cmdError`, never a rejection). */
  acceptProposal(id: Id): Promise<boolean>;
  setAdvisor(feed: AdvisorFeed): void;
  setAdvisorOpen(open: boolean): void;
  setAdvisorTab(tab: AdvisorTab): void;
  /** PR 3: a NEXT-MOVES feed surface mounted — ref-counts so the model rank is
      issued once per (planHash, epoch) across BOTH surfaces, and refetched on
      a genuinely fresh open (all surfaces were closed). */
  registerFeed(): void;
  unregisterFeed(): void;
  /** PR 3: full model rank on surface-open / epoch change, guarded so a second
      surface opening at the same (planHash, epoch) does NOT re-bill. */
  openRankFeed(): Promise<void>;
  /** PR 3: per-edit refetch of the FREE heuristic list, folded under the
      model's standing order via mergeRank (no provider call). Wired centrally
      to planHash so it runs once per edit regardless of open surfaces. */
  mergeOnEdit(): Promise<void>;
  /** PR 3: bump the rank epoch (config save / preference toggle) → re-rank. */
  bumpRankEpoch(): void;
  /** PR 3: persist plan-scoped NEXT preferences and re-rank. */
  setPreferences(prefs: NextPreferences): Promise<void>;
  setDashboardOpen(open: boolean): void;
  setAiSettingsOpen(context: AiSettingsContext | null): void;
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
  /** PR 9: fetch the ranked next-move list (read-only; [] on refusal). */
  nextMoves(): Promise<Opportunity[]>;
  /** PR 10: rank-and-narrate NEXT MOVES over the same candidates. Always
      answers a list; a failed model call surfaces via the status-bar chip and
      falls back to the heuristic order (never blocks the dashboard). */
  rankMoves(): Promise<RankResponse>;
  /** On-device AI: read the persisted opt-in + WebGPU support on boot, and
      (if the user previously enabled it) kick off the cached weight load so the
      engine is ready without a second click. Safe to call on every backend. */
  initWebllm(): void;
  /** On-device AI: opt in for `model` (defaults to the persisted/last model) —
      persists the choice and downloads+compiles the weights, streaming progress
      into `webllm`. Rejects to an `error` phase (surfaced in the settings UI),
      never throws to the caller. */
  enableWebllm(model?: string): Promise<void>;
  /** On-device AI: opt out — persist disabled, free the engine + GPU memory.
      The downloaded weights are KEPT in browser cache so a later re-enable is
      instant (no re-download); use `removeWebllmDownload` to reclaim the space. */
  disableWebllm(): Promise<void>;
  /** On-device AI: turn off AND delete the cached ~0.9 GB weights from browser
      storage — a full removal. Re-enabling afterward re-downloads. */
  removeWebllmDownload(): Promise<void>;
  /** PR 10: refresh the public model-config view. */
  fetchAiConfig(): Promise<void>;
  /** PR 10: save the model config (in-memory backend-side; key write-only).
      Returns true on success; refusals land in cmdError. */
  saveAiConfig(update: AiConfigUpdate): Promise<boolean>;
  /** PR 9: ask for the audit drawer on a specific tab (openAudit action). */
  openAuditTab(tab: AuditTab): void;
  clearAuditRequest(): void;
  /** PR 9: ask the map to pan to a world position (NEXT MOVES SHOW action). */
  requestFly(pos: { x: number; y: number }): void;
  clearFly(): void;
}

/** PR 10 (review M5), hoisted verbatim into the store slice (PR 3): fold a
 *  FRESH heuristic card list into the ranked view after a plan edit, without a
 *  model round-trip. The model's ORDER is cheap opinion and may go stale
 *  between opens; its PROSE quotes numbers — and prose never outlives the
 *  evidence it quoted. So: keep the model's order for surviving cards, but every
 *  card body comes from the fresh fetch, a note survives ONLY while its card's
 *  evidence is byte-identical to what the note was written against, vanished
 *  cards drop, new cards append in fresh (heuristic) relative order un-noted,
 *  and the headline survives only while the top card's id AND evidence are both
 *  unchanged. Exported pure — unit-pinnable without mounting a component. */
export function mergeRank(prev: RankResponse | null, fresh: Opportunity[]): RankResponse {
  if (!prev || prev.engine !== "model") {
    // Nothing model-authored to preserve: plain heuristic swap. A prior
    // fetch error is carried on the object (state stays truthful) but is
    // never re-surfaced per edit — rankMoves() already chipped it once.
    return { engine: "heuristic", opportunities: fresh, ...(prev?.error ? { error: prev.error } : {}) };
  }
  const freshById = new Map(fresh.map((o) => [o.id, o]));
  const merged: Opportunity[] = [];
  for (const p of prev.opportunities) {
    const f = freshById.get(p.id);
    if (!f) continue; // subject vanished — the card goes with it
    freshById.delete(p.id);
    // Evidence gate: byte-identical evidence keeps the note; any drift drops
    // it (a "15% headroom" note under refreshed 40% evidence is a visible
    // self-contradiction).
    merged.push(p.note !== undefined && f.evidence === p.evidence ? { ...f, note: p.note } : f);
  }
  for (const f of fresh) {
    if (freshById.has(f.id)) merged.push(f); // new since the rank — un-noted
  }
  const headlineSurvives =
    prev.headline !== undefined &&
    merged.length > 0 &&
    prev.opportunities.length > 0 &&
    merged[0].id === prev.opportunities[0].id &&
    merged[0].evidence === prev.opportunities[0].evidence;
  // DC-F2: DROP prev.wildcards on an edit. Wildcards are the model's
  // unverified brainstorm, tied to the exact plan state it saw — carrying them
  // across an unbounded run of edits would show stale ideas against drifted
  // numbers (worse than a brief flicker). They return on the next full rank /
  // epoch bump. The evidence-gated notes + headline above still ride along,
  // but only while the evidence they quoted is byte-identical.
  return {
    engine: prev.engine,
    ...(prev.model !== undefined ? { model: prev.model } : {}),
    ...(headlineSurvives ? { headline: prev.headline } : {}),
    opportunities: merged,
  };
}

// PR 3 shared-rank guard state — module-level (a store singleton), NOT reactive
// store fields, so touching them never triggers a render. `rankSeq` is the
// last-writer-wins token PR 10 kept per-component: every fetch (model rank OR
// per-edit merge) takes the next seq and only writes if still newest, so a slow
// model rank can never clobber a later merge and vice versa. `rankedKey` records
// the (planHash, epoch) a current rank already covers; it clears when the LAST
// feed surface closes, so a genuinely fresh open refetches while a second
// simultaneous surface does not re-bill. `mountedFeeds` ref-counts open feeds.
let rankSeq = 0;
let rankedKey: string | null = null;
let mountedFeeds = 0;
let lastMergedHash = "";
// Monotonic toast id — avoids duplicate React keys when two toasts fire in the
// same millisecond (Date.now would collide).
let nextToastId = 1;
const rankKey = (): string => `${useStore.getState().planHash}:${useStore.getState().rankEpoch}`;

// ---- On-device AI (WebLLM) engine orchestration ----------------------------

/** Download+compile the browser model, streaming progress into `webllm`. On
 *  success `phase` lands "ready"; on failure "error" with a human reason (the
 *  preference stays enabled so the settings UI offers a retry). Idempotent — a
 *  second call while ready/loading is a no-op via the module engine singleton. */
async function loadWebllmEngine(model: string): Promise<void> {
  const cur = useStore.getState().webllm;
  if (cur.phase === "loading") return;
  useStore.setState({
    webllm: { ...useStore.getState().webllm, phase: "loading", progress: 0, progressText: "starting…", error: null },
  });
  try {
    await loadEngine(model, (fraction, text) => {
      useStore.setState({
        webllm: { ...useStore.getState().webllm, progress: fraction, progressText: text },
      });
    });
    // A completed load means the weights are now in browser cache — record it
    // (persisted) so the settings UI can offer to remove them later.
    persistWebllmDownloaded(true);
    useStore.setState({
      webllm: {
        ...useStore.getState().webllm,
        phase: "ready",
        progress: 1,
        progressText: "ready",
        downloaded: true,
        error: null,
      },
    });
  } catch (e) {
    useStore.setState({
      webllm: { ...useStore.getState().webllm, phase: "error", error: errText(e) },
    });
  }
}

/** Run the prepare → browser-model → apply round-trip. PHASE 1 snapshots the
 *  candidates+context under the wasm lock; if there is nothing to rank it hands
 *  back a finished heuristic list (no model call). Otherwise the model runs
 *  in-browser and its raw reply goes back through the Rust firewall (PHASE 2).
 *  A model/generation failure still resolves through `rankApply` — a blank reply
 *  degrades to the heuristic order, never a thrown error. */
async function rankOnDevice(model: string): Promise<RankResponse> {
  const prep = await backend.rankPrepare!(model);
  if (prep.mode === "done") return prep.response;
  let content = "";
  try {
    content = await runRank(prep.system, prep.user, prep.maxTokens);
  } catch {
    // generation failed — fall through with empty content. The firewall treats
    // empty as a QUIET degrade (heuristic, no error), so a torn-down engine (the
    // user hit DISABLE mid-rank) never chips a spurious "invalid JSON" notice.
    content = "";
  }
  // `jobId` guards against a second, overlapping rank having replaced the parked
  // job while the model ran: a non-matching apply degrades to a clean heuristic.
  return backend.rankApply!(prep.jobId, content);
}

export const useStore = create<AppStore>((set, get) => ({
  ready: false,
  boot: { stage: "CONNECTING BACKEND", fraction: 0 },
  error: null,
  cmdError: null,
  toasts: [],
  autoSync: { enabled: readAutoSyncEnabled(), intervalMin: readAutoSyncInterval() },
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
  mapFilter: "",
  graphFilter: "",
  importFile: null,
  uploadingDocs: false,
  projected: null,
  settled: new Set(),
  motion: null,
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
  aiSettingsOpen: null,
  auditRequest: null,
  flyTo: null,
  aiConfig: null,
  rank: null,
  rankEpoch: 0,
  advisorTab: "feed",
  webllm: {
    enabled: readWebllmEnabled(),
    supported: webgpuSupported(),
    phase: "idle",
    progress: 0,
    progressText: "",
    model: readWebllmModel(),
    downloaded: readWebllmDownloaded(),
    error: null,
  },

  async hydrate() {
    // DC-F3: a hydrate is a fresh plan surface (boot, or optimizeAdopt's
    // same-plan re-hydrate). Reset the shared rank slice, its epoch, and the
    // module singletons so one plan's model content — ESPECIALLY unverified
    // wildcards — can never carry onto another across a plan-switch/reload. A
    // still-mounted feed simply re-ranks on its next open, which is correct.
    rankedKey = null;
    mountedFeeds = 0;
    lastMergedHash = "";
    set({ rank: null, rankEpoch: 0 });
    try {
      // Boot stages are the REAL awaits: the hydrate round-trip (on web this
      // spans worker spin-up + IndexedDB docs restore + the wasm session
      // rebuild), then applying the payload. The fractions are stage weights
      // of known work; the BootScreen smoother animates through the bursts.
      set({ boot: { stage: "READING PLAN FILE", fraction: 0.06 } });
      const init = await backend.hydrate();
      const nFactories = Object.keys(init.plan.factories).length;
      const nRoutes = Object.keys(init.plan.routes).length;
      set({
        boot: {
          stage: `HYDRATING FACTORIES — ${nFactories}` + (nRoutes ? ` · ROUTES — ${nRoutes}` : ""),
          fraction: 0.82,
        },
      });
      // Yield one macrotask so React commits the HYDRATING stage before the
      // heavy synchronous payload set below — without it both sets land in
      // the same microtask and the middle stage never renders. This is a
      // scheduling yield, not synthetic progress: 0.82 fronts the real
      // payload-application work that follows. (Not rAF — headless/fresh
      // pages can park rAF for ~1.5s and would stall the boot.)
      await new Promise((r) => setTimeout(r, 0));
      const openFactory = init.viewState?.openFactory;
      set({
        boot: {
          stage: `EMPIRE ONLINE — ${nFactories} ${nFactories === 1 ? "FACTORY" : "FACTORIES"}`,
          fraction: 1,
        },
        ready: true,
        error: null,
        // A hydrate (boot or live re-projection) is never an edit/undo/redo —
        // clear any verb so its diff can only ever play neutral grammar.
        motion: null,
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
      // Fatal `error` (the full-screen BACKEND UNREACHABLE card) is for a
      // failed FIRST boot only. hydrate() is also the re-projection step for
      // live-app actions (uploadDocs, newEmpire, syncImport, autoPull,
      // optimizeAdopt) — a transient blip there must not nuke a healthy
      // running app; surface it as a command error and leave the app standing.
      if (get().ready) get().reportCmdError(`Plan re-sync failed — ${errText(e)}`);
      else set({ error: String(e) });
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
      motion: { kind: "edit", at: Date.now(), hash: resp.planHash },
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
      motion: { kind: "undo", at: Date.now(), hash: resp.planHash },
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
      motion: { kind: "redo", at: Date.now(), hash: resp.planHash },
      cmdError: null,
    }));
  },

  reportCmdError(message) {
    const msg = nameIds(friendlyError(message));
    // The status-bar chip is easy to miss; a toast makes every refused action
    // visibly acknowledged (the app was too quiet about what did/didn't happen).
    set({ cmdError: { message: msg, at: Date.now() } });
    get().pushToast(msg, "error");
  },

  clearCmdError(at) {
    set((s) => (s.cmdError?.at === at ? { cmdError: null } : {}));
  },

  pushToast(message, kind = "info") {
    const id = nextToastId++;
    set((s) => {
      // Coalesce an identical trailing toast (a burst of the same refusal
      // shouldn't stack), and cap the visible column so it can't run away.
      const last = s.toasts[s.toasts.length - 1];
      if (last && last.message === message && last.kind === kind) return {};
      return { toasts: [...s.toasts, { id, message, kind }].slice(-4) };
    });
    // Errors linger a little longer than successes; both auto-dismiss.
    setTimeout(() => get().dismissToast(id), kind === "error" ? 6000 : 4200);
  },
  dismissToast(id) {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
  clearToasts() {
    set({ toasts: [] });
  },

  setSelection: (selection) => set({ selection }),
  setView: (view) => {
    set({ view, selection: null });
    get().saveViewState({ openFactory: view.mode === "factory" ? view.factoryId : null });
  },
  setOverlay: (key, on) => set((s) => ({ overlays: { ...s.overlays, [key]: on } })),
  setMapFilter: (mapFilter) => set({ mapFilter }),
  setGraphFilter: (graphFilter) => set({ graphFilter }),
  setImportFile: (importFile) => set({ importFile }),
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

  setAiSettingsOpen(context) {
    set({ aiSettingsOpen: context });
  },

  async markBuildDone(id, done) {
    // One undoable step: SetBuildDone upserts (Some) or clears (null) the
    // override; the response patches /buildOverrides and the derived queue.
    await get().dispatch([{ type: "set_build_done", id, done }]);
  },

  async uploadDocs(bytes) {
    // Web Phase 4a: hand the raw uploaded Docs.json to the wasm worker, which
    // rebuilds the session over the real catalog while preserving the plan,
    // then re-hydrate so the richer recipe set is live (gamedata.buildVersion
    // flips off "fixture"). A refusal (e.g. an unparseable file, or a non-wasm
    // backend) surfaces on the status-bar chip — never a rejection to the UI.
    set({ uploadingDocs: true });
    try {
      await backend.uploadDocs(bytes);
    } catch (e) {
      // reportCmdError already raises an error toast; make it Docs-specific.
      get().reportCmdError(`Couldn't load Docs.json — ${errText(e)}`);
      return false;
    } finally {
      set({ uploadingDocs: false });
    }
    await get().hydrate();
    const recipes = Object.keys(get().gamedata.recipes).length;
    get().pushToast(`Catalog loaded — ${recipes.toLocaleString()} recipes`, "success");
    return true;
  },

  async newEmpire() {
    // Wipe the plan via Session::new_empire (keeps the catalog), then re-hydrate
    // onto the empty session. Selection/view could point at now-deleted entities,
    // so reset them before the re-projection. A backend error surfaces on the
    // chip — never a rejection to the UI.
    try {
      await backend.newEmpire();
    } catch (e) {
      get().reportCmdError(`Couldn't start a new empire — ${errText(e)}`);
      return false;
    }
    get().setSelection(null);
    get().setView({ mode: "map" });
    await get().hydrate();
    get().pushToast("New empire — the old plan was cleared. Import your save to begin.", "success");
    return true;
  },

  async syncImport(file) {
    let outcome: ImportOutcome;
    try {
      const snapshot = await parseSaveFile(file);
      outcome = await backend.importRun(snapshot);
    } catch (e) {
      // No dead ends: a bad read/parse is a toast, never a crash — the manual
      // Import save flow (with its preview) remains available as a fallback.
      get().pushToast(`Couldn't sync that save — ${errText(e)}`, "error");
      return null;
    }
    await get().hydrate(); // the backend layer/proposal landed; re-project
    if (outcome.outcome === "drift") {
      const n = get().plan.proposals[outcome.proposal]?.items.length ?? 0;
      get().setReviewing(outcome.proposal);
      get().pushToast(`Synced — ${n} change${n === 1 ? "" : "s"} to review`, "success");
    } else if (outcome.outcome === "imported") {
      get().pushToast(`Synced — ${outcome.factories} factories imported as ◆ built`, "success");
    } else {
      get().pushToast("Synced — plan already matches this save", "info");
    }
    return outcome;
  },

  setAutoSync(enabled, intervalMin) {
    const next = { enabled, intervalMin: intervalMin ?? get().autoSync.intervalMin };
    persistAutoSync(next.enabled, next.intervalMin);
    set({ autoSync: next });
  },

  async autoPull(file) {
    let outcome: ImportOutcome;
    try {
      const snapshot = await parseSaveFile(file);
      outcome = await backend.importRun(snapshot);
    } catch (e) {
      get().pushToast(`Auto-sync couldn't read the save — ${errText(e)}`, "error");
      return null;
    }
    await get().hydrate();
    if (outcome.outcome === "drift") {
      const items = get().plan.proposals[outcome.proposal]?.items ?? [];
      const conflicts = driftConflictCount(items);
      if (conflicts === 0) {
        // Option B: a conflict-free drift applies silently — but only toast
        // success if the accept actually landed. If the backend refuses it
        // (acceptProposal already raised the error), open the draft in review
        // so the next tick's `reviewing` guard stops us re-running it in a loop.
        if (await get().acceptProposal(outcome.proposal)) {
          get().pushToast(
            `Auto-synced — ${items.length} change${items.length === 1 ? "" : "s"} applied`,
            "success",
          );
        } else {
          get().setReviewing(outcome.proposal);
        }
      } else {
        // Real mine/theirs conflicts need a human — open review, never auto-apply.
        get().setReviewing(outcome.proposal);
        get().pushToast(
          `Auto-sync paused — ${conflicts} conflict${conflicts === 1 ? "" : "s"} to resolve`,
          "info",
        );
      }
    } else if (outcome.outcome === "imported") {
      get().pushToast(`Auto-synced — ${outcome.factories} factories imported as ◆ built`, "success");
    }
    // in_sync → quiet no-op (no toast on a tick where nothing changed)
    return outcome;
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

  // PR 9: the opportunity list is derived/advisory — a read-only fetch, never
  // a mutation. Empty on a healthy finished base (honest quiet).
  async nextMoves() {
    try {
      return await backend.nextMoves();
    } catch (e) {
      get().reportCmdError(errText(e));
      return [];
    }
  },

  // PR 10: same read-only species as nextMoves. A backend refusal degrades to
  // an empty heuristic list; a model-call failure arrives as `error` on an
  // otherwise-usable heuristic response — surface it on the status-bar chip
  // and keep rendering (fail-quiet + surfaced, never wedged).
  async rankMoves() {
    let resp: RankResponse;
    try {
      // On-device path: only when the browser engine is actually LOADED (not
      // merely opted-in — a still-downloading engine falls through to the free
      // heuristic, no error). The prepare/apply split runs the model in-browser
      // between two under-lock backend halves; the Rust firewall validates the
      // reply, so a weak on-device answer degrades to the heuristic order.
      const w = get().webllm;
      const canModel =
        w.enabled && w.phase === "ready" && engineReady() && backend.rankPrepare && backend.rankApply;
      resp = canModel ? await rankOnDevice(w.model) : await backend.nextRank();
    } catch (e) {
      get().reportCmdError(errText(e));
      return { engine: "heuristic", opportunities: [] };
    }
    if (resp.error) get().reportCmdError(resp.error);
    return resp;
  },

  initWebllm() {
    // Reconcile the boot-time defaults with live WebGPU support, then — if the
    // user previously opted in and the browser can run it — warm the (cached)
    // weights so NEXT MOVES ranks on-device without a second click.
    const supported = webgpuSupported();
    set({ webllm: { ...get().webllm, supported } });
    const w = get().webllm;
    if (w.enabled && supported && w.phase === "idle") {
      void loadWebllmEngine(w.model);
    } else if (w.enabled && !supported) {
      // Opted in on a device that can no longer run it — keep the preference
      // but present an honest, non-fatal note instead of a broken "ready".
      set({ webllm: { ...get().webllm, phase: "error", error: "this browser has no WebGPU" } });
    }
  },

  async enableWebllm(model) {
    const chosen = model ?? get().webllm.model ?? DEFAULT_WEBLLM_MODEL;
    persistWebllm(true, chosen);
    set({ webllm: { ...get().webllm, enabled: true, model: chosen, error: null } });
    if (!webgpuSupported()) {
      set({ webllm: { ...get().webllm, phase: "error", error: "this browser has no WebGPU" } });
      return;
    }
    await loadWebllmEngine(chosen);
  },

  async disableWebllm() {
    persistWebllm(false, get().webllm.model);
    set({
      webllm: { ...get().webllm, enabled: false, phase: "idle", progress: 0, progressText: "", error: null },
    });
    await unloadEngine();
  },

  async removeWebllmDownload() {
    const model = get().webllm.model;
    // Off + reclaim: clear the opt-in AND the downloaded flag up front so the UI
    // reflects the removal immediately, then delete the cached weights.
    persistWebllm(false, model);
    persistWebllmDownloaded(false);
    set({
      webllm: {
        ...get().webllm,
        enabled: false,
        phase: "idle",
        progress: 0,
        progressText: "",
        downloaded: false,
        error: null,
      },
    });
    try {
      await deleteModelFromCache(model);
      get().pushToast("On-device AI removed and its download cleared.", "info");
    } catch (e) {
      // Best-effort: the feature is already off; the cache delete just couldn't
      // finish (e.g. partial eviction). Say so rather than pretend it worked.
      get().pushToast(`On-device AI turned off, but clearing the cache failed: ${errText(e)}`, "error");
    }
  },

  async fetchAiConfig() {
    try {
      set({ aiConfig: await backend.aiConfig() });
    } catch {
      // fail-quiet: the chip just reads AI: OFF until the backend answers
    }
  },

  async saveAiConfig(update) {
    try {
      set({ aiConfig: await backend.setAiConfig(update) });
      return true;
    } catch (e) {
      get().reportCmdError(errText(e));
      return false;
    }
  },

  setAdvisorTab(tab) {
    set({ advisorTab: tab });
  },

  registerFeed() {
    mountedFeeds += 1;
    void get().openRankFeed();
  },

  unregisterFeed() {
    mountedFeeds -= 1;
    // Last surface closed: forget which key we ranked so a genuinely fresh
    // open refetches (matches PR 10's per-open remount behaviour). The rank
    // itself is KEPT so the status-bar chip survives a feed close.
    if (mountedFeeds <= 0) {
      mountedFeeds = 0;
      rankedKey = null;
    }
  },

  // Full model rank on surface-open / epoch change. Guard: skip when the
  // current (planHash, epoch) already has a rank issued during this open
  // session — so a second surface opening (dashboard then panel), and the
  // openRankFeed the epoch-effect fires alongside the mount, never re-bill.
  async openRankFeed() {
    const key = rankKey();
    if (rankedKey === key) return;
    rankedKey = key;
    // Capture the hash we're actually ranking. On resolve we may claim ONLY
    // this hash — never "now" (H1): an edit landing mid-flight leaves the
    // resolved cards describing the PRE-edit plan, and stamping the current
    // hash would swallow that edit, freezing a stale, dead-clickable list.
    const startHash = get().planHash;
    const seq = ++rankSeq;
    const r = await get().rankMoves();
    if (rankSeq !== seq) return; // a later merge/rank already won
    set({ rank: r });
    lastMergedHash = startHash;
    // Plan moved while this first rank was in flight? App's per-edit effect
    // already fired mergeOnEdit for the new hash, but rank was still null then
    // so it bailed without stamping — reconcile the delta now that a rank
    // exists (folds the fresh heuristic list over the current hash).
    if (get().planHash !== startHash) void get().mergeOnEdit();
  },

  // Per-edit fold of the FREE heuristic list under the model's standing order.
  // Dedups on planHash so App's central effect can call it on every change; a
  // null rank (no feed ever opened) needs no upkeep — the chip is hidden.
  async mergeOnEdit() {
    const h = get().planHash;
    if (h === lastMergedHash) return;
    // A null rank (no feed ever opened) needs no upkeep — the chip is hidden.
    // Guard BEFORE stamping: a mergeOnEdit that fires during the first
    // in-flight rank (rank still null) must NOT claim this hash, or the merge
    // that reconciles it once the rank lands would be skipped (H1).
    if (get().rank === null) return;
    const seq = ++rankSeq;
    const fresh = await get().nextMoves();
    if (rankSeq !== seq) return; // a later merge/rank already won
    set((s) => ({ rank: mergeRank(s.rank, fresh) }));
    // Claim the hash only AFTER a real merge writes — never a hash we didn't
    // fold. DC-F2: do NOT claim rankedKey here; a genuinely fresh reopen must
    // re-rank the model rather than reuse a merge's heuristic-only result.
    lastMergedHash = h;
  },

  bumpRankEpoch() {
    set((s) => ({ rankEpoch: s.rankEpoch + 1 }));
  },

  // Persist the preference (not undoable, outside plan_hash), fold in the fresh
  // heuristic list optimistically, and bump the epoch so open feeds re-rank
  // (planHash is unchanged, so mergeOnEdit will not fire — the epoch drives it).
  async setPreferences(prefs) {
    let view;
    try {
      view = await backend.setPreferences(prefs);
    } catch (e) {
      get().reportCmdError(errText(e));
      return;
    }
    set((s) => ({
      plan: { ...s.plan, meta: { ...s.plan.meta, preferences: view.preferences } },
      rank: mergeRank(s.rank, view.opportunities),
    }));
    get().bumpRankEpoch();
  },

  openAuditTab(tab) {
    set({ auditRequest: tab });
  },

  clearAuditRequest() {
    set({ auditRequest: null });
  },

  requestFly(pos) {
    set({ flyTo: pos });
  },

  clearFly() {
    set({ flyTo: null });
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
    // The adopt succeeded (proposals opened); its note is informational, not a
    // refusal — surface it as a neutral toast, not the red error path.
    if (outcome.note) get().pushToast(outcome.note, "info");
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
      return false;
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
    return true;
  },
}));

// e2e observability: the w4 spec pins the M5 merge path — a planHash change
// while the dashboard is open must NOT re-call the model — which requires a
// REAL in-page dispatch (external /api/edit calls from the test fixture never
// reach this store; the dev bridge has no SSE push). The hook exposes nothing
// the dev tools couldn't already reach. Exposed on the dev server AND in the
// web (`--mode web`) build — the web build is a production bundle
// (`import.meta.env.DEV` is false there), so the browser smoke that drives the
// BUILT web app needs `__WASM_BACKEND__` to reach the same seam.
if (import.meta.env.DEV || __WASM_BACKEND__) {
  (window as Window & { __ficsitStore?: typeof useStore }).__ficsitStore = useStore;
  // Also expose the raw backend so the web browser-smoke can drive transports
  // the store has no direct action for (e.g. chatSend, which drafts a proposal
  // and whose snapshot-on-reload persistence is the M1 acceptance).
  (window as Window & { __ficsitBackend?: typeof backend }).__ficsitBackend = backend;
}

/** Solve-time chip content for a factory (A4: always present, always honest). */
export function solveChip(df: DerivedFactory | undefined): { text: string; over: boolean } {
  if (!df) return { text: "SOLVE —", over: false };
  const ms = df.solveUs / 1000;
  const over = ms > 50;
  const text = `${over ? "SOLVE" : "LAST"} ${ms < 1 ? ms.toFixed(1) : ms.toFixed(0)}ms${over ? "" : " ✓"}`;
  return { text, over };
}
