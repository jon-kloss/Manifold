// WasmBackend (Web Phase 3): the THIRD transport behind the `Backend`
// interface, alongside TauriBackend (IPC) and BridgeBackend (HTTP). It posts
// `{ id, cmd, args }` to the wasm session worker (wasmWorker.ts) and awaits the
// id-correlated reply from a small promise registry. Command names + arg shapes
// mirror the wasm `dispatch` router, which itself mirrors the dev-bridge route
// table — so this transport is a 1:1 restatement of BridgeBackend over
// worker-RPC instead of HTTP.
//
// This module is imported ONLY when the `__WASM_BACKEND__` compile-time define
// is true (the `--mode web` build), via a dynamic import in backend.ts, so a
// desktop/dev build eliminates it and never bundles the worker or the wasm.

import type {
  AdoptOutcome,
  AdvisorFeed,
  AiConfigPublic,
  AiConfigUpdate,
  AltOpportunity,
  ChatReply,
  ChatScope,
  Command,
  ContextSnapshot,
  CutoverPlan,
  EditResponse,
  Id,
  ImportOutcome,
  ImportSnapshot,
  InitPayload,
  JobProgress,
  NextPreferences,
  Opportunity,
  PreferencesView,
  Proposal,
  ProposalConsequence,
  RankPrepare,
  RankResponse,
  RouteKind,
  TrainAnswer,
  ViewState,
  WizardGoal,
} from "./types";
import type { Backend, EmpireList, PlanReplacementResult } from "./backend";

interface WorkerReply {
  id: number;
  ok: boolean;
  result?: unknown;
  error?: string;
}

export class WasmBackend implements Backend {
  private worker: Worker;
  private seq = 0;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  /** L4: set once the worker has fatally failed. A dead worker can no longer
   *  answer, so NEW calls reject immediately instead of `postMessage`-ing into
   *  the void and hanging on a promise that never settles. */
  private deadError: Error | null = null;

  constructor() {
    this.worker = new Worker(new URL("./wasmWorker.ts", import.meta.url), { type: "module" });
    this.worker.onmessage = (e: MessageEvent<WorkerReply>) => {
      const { id, ok, result, error } = e.data;
      const p = this.pending.get(id);
      if (!p) return;
      this.pending.delete(id);
      if (ok) p.resolve(result);
      else p.reject(new Error(error ?? "wasm session error"));
    };
    // A worker-level failure (bad wasm load, uncaught throw) must reject every
    // in-flight call rather than hang the UI on a promise that never settles —
    // and mark the worker dead so subsequent calls fail fast (L4).
    this.worker.onerror = (e) => {
      this.deadError = new Error(e.message || "wasm session worker crashed");
      for (const [, p] of this.pending) p.reject(this.deadError);
      this.pending.clear();
    };
  }

  private call<T>(cmd: string, args?: unknown): Promise<T> {
    // L4: never postMessage to a worker known dead — it would hang forever.
    if (this.deadError) return Promise.reject(this.deadError);
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ id, cmd, args });
    });
  }

  /** Phase 4a: upload a real Docs.json. This is a worker CONTROL message, not a
   *  `dispatch` — gamedata is set only at session construction, so the worker
   *  rebuilds the WebSession over the uploaded catalog while preserving the
   *  plan. Same id-correlated reply path as `call`; the payload carries `bytes`
   *  and a `kind` the worker branches on ahead of the dispatch router. */
  uploadDocs(bytes: Uint8Array): Promise<void> {
    if (this.deadError) return Promise.reject(this.deadError);
    const id = ++this.seq;
    return new Promise<void>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      // TRANSFER the buffer (not structured-clone it): a real Docs.json is
      // multi-MB and the caller never reuses these bytes after the upload, so
      // move ownership to the worker instead of copying it across the boundary.
      this.worker.postMessage({ id, kind: "upload_docs", bytes }, [bytes.buffer]);
    });
  }

  /** Clear the plan (keep the catalog). A normal `dispatch("new_empire")`: the
   *  wasm `Session::new_empire` resets the store, and the worker's
   *  snapshot-after-mutate writes the now-empty blob to IndexedDB. */
  async newEmpire() {
    await this.call("new_empire");
  }

  /** Multi-empire (1.0): CONTROL messages like uploadDocs — the worker owns
   *  the IndexedDB slots and rebuilds the WebSession from whichever blob the
   *  named empire stores; a dispatch can't do that (gamedata + plan are
   *  construction-only). */
  private empireOp(op: string, args?: Record<string, unknown>): Promise<EmpireList> {
    if (this.deadError) return Promise.reject(this.deadError);
    const id = ++this.seq;
    return new Promise<EmpireList>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ id, kind: "empire", op, ...args });
    });
  }
  empires() {
    return this.empireOp("list");
  }
  empireCreate(name: string) {
    return this.empireOp("create", { name });
  }
  empireSwitch(name: string) {
    return this.empireOp("switch", { name });
  }
  empireRename(from: string, to: string) {
    return this.empireOp("rename", { from, to });
  }
  empireDelete(name: string) {
    return this.empireOp("delete", { name });
  }

  hydrate() {
    return this.call<InitPayload>("hydrate");
  }
  edit(cmds: Command[]) {
    return this.call<EditResponse>("edit", { cmds });
  }
  undo() {
    return this.call<EditResponse | null>("undo");
  }
  redo() {
    return this.call<EditResponse | null>("redo");
  }
  async setViewState(v: ViewState) {
    await this.call("set_view_state", v);
  }
  async wizardSolve(goal: WizardGoal) {
    const r = await this.call<{ jobId: string }>("wizard_solve", goal);
    return r.jobId;
  }
  wizardProgress(jobId: string, after: number) {
    return this.call<JobProgress>("wizard_progress", { jobId, after });
  }
  async wizardCancel(jobId: string) {
    await this.call("wizard_cancel", { jobId });
  }
  async t2Optimize(factory: string) {
    const r = await this.call<{ proposal: Proposal | null }>("t2_optimize", { factory });
    return r.proposal;
  }
  proposalAccept(id: string) {
    return this.call<EditResponse>("proposal_accept", { id });
  }
  proposalEval(id: string) {
    return this.call<ProposalConsequence>("proposal_eval", { id });
  }
  planReplacement(factory: string) {
    return this.call<PlanReplacementResult>("plan_replacement", { factory });
  }
  cutoverPlan(factory: string) {
    return this.call<CutoverPlan>("cutover_plan", { factory });
  }
  optimizeEmpire() {
    return this.call<AltOpportunity[]>("optimize_empire");
  }
  optimizeAdopt(recipe: string) {
    return this.call<AdoptOutcome>("optimize_adopt", { recipe });
  }
  async nextMoves() {
    const r = await this.call<{ opportunities: Opportunity[] }>("next_moves");
    return r.opportunities;
  }
  nextRank() {
    return this.call<RankResponse>("next_rank");
  }
  rankPrepare(model: string) {
    return this.call<RankPrepare>("next_rank_prepare", model);
  }
  rankApply(jobId: number, content: string) {
    return this.call<RankResponse>("next_rank_apply", { jobId, content });
  }
  setPreferences(prefs: NextPreferences) {
    return this.call<PreferencesView>("set_next_preferences", prefs);
  }
  aiConfig() {
    return this.call<AiConfigPublic>("ai_config_get");
  }
  setAiConfig(update: AiConfigUpdate) {
    return this.call<AiConfigPublic>("ai_config_set", update);
  }
  importRun(snapshot: ImportSnapshot) {
    return this.call<ImportOutcome>("import_run", snapshot);
  }
  advisorDismiss(id: string) {
    return this.call<AdvisorFeed>("advisor_dismiss", { id });
  }
  advisorUnmute(rule: string) {
    return this.call<AdvisorFeed>("advisor_unmute", { rule });
  }
  advisorPause(paused: boolean) {
    return this.call<AdvisorFeed>("advisor_pause", { paused });
  }
  chatSend(scope: ChatScope, message: string) {
    return this.call<ChatReply>("chat_send", { scope, message });
  }
  chatContext(scope: ChatScope) {
    return this.call<ContextSnapshot>("chat_context", scope);
  }
  routeCalc(from: Id, to: Id, kind: RouteKind, demandPerMin: number, item: string | null) {
    return this.call<TrainAnswer | null>("route_calc", { from, to, kind, demandPerMin, item });
  }
}
