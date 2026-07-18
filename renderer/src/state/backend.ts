// Backend abstraction: identical command surface over Tauri IPC (production)
// or the dev-bridge HTTP API (headless development). Rust owns canonical
// state in both — the renderer only ever sees patches (SDD §4).

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

/** Result of planning a replacement: the drafted Refactor proposal is stored,
 *  and the renderer opens it in review. */
export interface PlanReplacementResult {
  response: EditResponse;
  proposal: Id;
}

export interface Backend {
  hydrate(): Promise<InitPayload>;
  edit(cmds: Command[]): Promise<EditResponse>;
  undo(): Promise<EditResponse | null>;
  redo(): Promise<EditResponse | null>;
  setViewState(v: ViewState): Promise<void>;
  wizardSolve(goal: WizardGoal): Promise<string>;
  wizardProgress(jobId: string, after: number): Promise<JobProgress>;
  wizardCancel(jobId: string): Promise<void>;
  t2Optimize(factory: string): Promise<Proposal | null>;
  proposalAccept(id: string): Promise<EditResponse>;
  proposalEval(id: string): Promise<ProposalConsequence>;
  /** W2a: plan a whole-factory replacement (stores a Draft Refactor proposal). */
  planReplacement(factory: string): Promise<PlanReplacementResult>;
  /** W2a: price a cutover's downtime on demand (scratch-solved, ripple-inclusive). */
  cutoverPlan(factory: string): Promise<CutoverPlan>;
  /** W2b-D: empire alternate-recipe optimizer — a derived, read-only ranking. */
  optimizeEmpire(): Promise<AltOpportunity[]>;
  /** W2b-D: adopt an alternate empire-wide → draft review proposal(s) (◆ never mutated). */
  optimizeAdopt(recipe: string): Promise<AdoptOutcome>;
  /** PR 9: ranked next-move opportunities — a derived, read-only projection. */
  nextMoves(): Promise<Opportunity[]>;
  /** PR 10: rank-and-narrate over the SAME candidates as nextMoves. Always
   *  answers — unconfigured or failed model calls return the heuristic list. */
  nextRank(): Promise<RankResponse>;
  /** On-device split, PHASE 1 (web only). Snapshots candidates+context and
   *  either finishes ({mode:"done"}) or hands back the messages to run the
   *  browser model on ({mode:"call"}). Undefined on transports with no
   *  in-browser model (desktop/dev-bridge). */
  rankPrepare?(model: string): Promise<RankPrepare>;
  /** On-device split, PHASE 2 (web only). Validates the browser model's raw
   *  reply through the same firewall the native provider uses; a blank/invalid
   *  reply degrades to the heuristic list, never an error. `jobId` is the token
   *  from the matching `rankPrepare` — a mismatch (a newer rank replaced the
   *  parked job) degrades to a clean heuristic instead of cross-applying. */
  rankApply?(jobId: number, content: string): Promise<RankResponse>;
  /** PR 3: persist plan-scoped NEXT preferences (not undoable, outside
   *  plan_hash). Returns the updated view. */
  setPreferences(prefs: NextPreferences): Promise<PreferencesView>;
  /** PR 10: public model-config view (hasKey boolean, never the key). */
  aiConfig(): Promise<AiConfigPublic>;
  /** PR 10: set the in-memory model config (nothing persisted in v1). */
  setAiConfig(update: AiConfigUpdate): Promise<AiConfigPublic>;
  importRun(snapshot: ImportSnapshot): Promise<ImportOutcome>;
  advisorDismiss(id: string): Promise<AdvisorFeed>;
  advisorUnmute(rule: string): Promise<AdvisorFeed>;
  advisorPause(paused: boolean): Promise<AdvisorFeed>;
  chatSend(scope: ChatScope, message: string): Promise<ChatReply>;
  chatContext(scope: ChatScope): Promise<ContextSnapshot>;
  /** Task #49: read-only trains-needed answer for a PROSPECTIVE route between
   *  two factories (creates nothing). Null for belt/pipe or unknown factories. */
  routeCalc(
    from: Id,
    to: Id,
    kind: RouteKind,
    demandPerMin: number,
    item: string | null,
  ): Promise<TrainAnswer | null>;
  /** Web Phase 4a: upload a real Docs.json (raw bytes) so the browser session
   *  runs on the player's game catalog instead of the bundled fixture. Web-only
   *  — the desktop shell and dev bridge read the catalog from the host
   *  (FICSIT_DOCS_JSON), so their implementations reject. */
  uploadDocs(bytes: Uint8Array): Promise<void>;
  /** Clear the whole plan + undo journal (KEEPING the gamedata catalog) and
   *  project the empty plan — "start a new empire" for importing an unrelated
   *  save. A `Session::new_empire` op on every transport: SQLite wipe (desktop /
   *  dev bridge) or the wasm store reset snapshotted to IndexedDB (web). */
  newEmpire(): Promise<void>;
}

/** The desktop shell and dev bridge get their catalog from the host process
 *  (FICSIT_DOCS_JSON), not an in-app upload; the UI only calls `uploadDocs` on
 *  the web build (`__WASM_BACKEND__`), so this is an explicit guard, not a path
 *  either backend takes. */
function docsUploadUnsupported(): Promise<never> {
  return Promise.reject(
    new Error("Docs.json upload is only supported on the web build"),
  );
}

const isTauri = () => "__TAURI_INTERNALS__" in window;

class TauriBackend implements Backend {
  private invoke = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  };
  hydrate() {
    return this.invoke<InitPayload>("hydrate");
  }
  edit(cmds: Command[]) {
    return this.invoke<EditResponse>("plan_edit", { cmds });
  }
  undo() {
    return this.invoke<EditResponse | null>("plan_undo");
  }
  redo() {
    return this.invoke<EditResponse | null>("plan_redo");
  }
  async setViewState(v: ViewState) {
    await this.invoke("set_view_state", { json: JSON.stringify(v) });
  }
  async wizardSolve(goal: WizardGoal) {
    return this.invoke<string>("wizard_solve", { goal });
  }
  async wizardProgress(jobId: string, after: number) {
    const p = await this.invoke<JobProgress | null>("wizard_progress", { jobId, after });
    if (!p) throw new Error("unknown job");
    return p;
  }
  async wizardCancel(jobId: string) {
    await this.invoke("wizard_cancel", { jobId });
  }
  t2Optimize(factory: string) {
    return this.invoke<Proposal | null>("t2_optimize", { factory });
  }
  proposalAccept(id: string) {
    return this.invoke<EditResponse>("proposal_accept", { id });
  }
  proposalEval(id: string) {
    return this.invoke<ProposalConsequence>("proposal_eval", { id });
  }
  planReplacement(factory: string) {
    return this.invoke<PlanReplacementResult>("cutover_plan", { factory });
  }
  cutoverPlan(factory: string) {
    return this.invoke<CutoverPlan>("cutover_downtime", { factory });
  }
  optimizeEmpire() {
    return this.invoke<AltOpportunity[]>("optimize_empire");
  }
  optimizeAdopt(recipe: string) {
    return this.invoke<AdoptOutcome>("optimize_adopt", { recipe });
  }
  nextMoves() {
    return this.invoke<Opportunity[]>("next_moves");
  }
  nextRank() {
    return this.invoke<RankResponse>("next_rank");
  }
  setPreferences(prefs: NextPreferences) {
    return this.invoke<PreferencesView>("set_next_preferences", { prefs });
  }
  aiConfig() {
    return this.invoke<AiConfigPublic>("ai_config_get");
  }
  setAiConfig(update: AiConfigUpdate) {
    return this.invoke<AiConfigPublic>("ai_config_set", { update });
  }
  importRun(snapshot: ImportSnapshot) {
    return this.invoke<ImportOutcome>("import_run", { snapshot });
  }
  advisorDismiss(id: string) {
    return this.invoke<AdvisorFeed>("advisor_dismiss", { id });
  }
  advisorUnmute(rule: string) {
    return this.invoke<AdvisorFeed>("advisor_unmute", { rule });
  }
  advisorPause(paused: boolean) {
    return this.invoke<AdvisorFeed>("advisor_pause", { paused });
  }
  chatSend(scope: ChatScope, message: string) {
    return this.invoke<ChatReply>("chat_send", { scope, message });
  }
  chatContext(scope: ChatScope) {
    return this.invoke<ContextSnapshot>("chat_context", { scope });
  }
  routeCalc(from: Id, to: Id, kind: RouteKind, demandPerMin: number, item: string | null) {
    return this.invoke<TrainAnswer | null>("route_calc", { from, to, kind, demandPerMin, item });
  }
  uploadDocs() {
    return docsUploadUnsupported();
  }
  async newEmpire() {
    await this.invoke("new_empire");
  }
}

class BridgeBackend implements Backend {
  private async call<T>(path: string, init?: RequestInit): Promise<T> {
    const res = await fetch(`/api/${path}`, init);
    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error((body as { error?: string }).error ?? res.statusText);
    }
    return (await res.json()) as T;
  }
  hydrate() {
    return this.call<InitPayload>("hydrate");
  }
  edit(cmds: Command[]) {
    return this.call<EditResponse>("edit", { method: "POST", body: JSON.stringify(cmds) });
  }
  undo() {
    return this.call<EditResponse | null>("undo", { method: "POST" });
  }
  redo() {
    return this.call<EditResponse | null>("redo", { method: "POST" });
  }
  async setViewState(v: ViewState) {
    await this.call("view", { method: "POST", body: JSON.stringify(v) });
  }
  async wizardSolve(goal: WizardGoal) {
    const r = await this.call<{ jobId: string }>("wizard/solve", {
      method: "POST",
      body: JSON.stringify(goal),
    });
    return r.jobId;
  }
  wizardProgress(jobId: string, after: number) {
    return this.call<JobProgress>("wizard/progress", {
      method: "POST",
      body: JSON.stringify({ jobId, after }),
    });
  }
  async wizardCancel(jobId: string) {
    await this.call("wizard/cancel", { method: "POST", body: JSON.stringify({ jobId }) });
  }
  async t2Optimize(factory: string) {
    const r = await this.call<{ proposal: Proposal | null }>("t2/optimize", {
      method: "POST",
      body: JSON.stringify({ factory }),
    });
    return r.proposal;
  }
  proposalAccept(id: string) {
    return this.call<EditResponse>("proposal/accept", { method: "POST", body: JSON.stringify({ id }) });
  }
  proposalEval(id: string) {
    return this.call<ProposalConsequence>("proposal/eval", { method: "POST", body: JSON.stringify({ id }) });
  }
  planReplacement(factory: string) {
    return this.call<PlanReplacementResult>("cutover/plan", { method: "POST", body: JSON.stringify({ factory }) });
  }
  cutoverPlan(factory: string) {
    return this.call<CutoverPlan>("cutover/downtime", { method: "POST", body: JSON.stringify({ factory }) });
  }
  optimizeEmpire() {
    return this.call<AltOpportunity[]>("optimize/empire");
  }
  optimizeAdopt(recipe: string) {
    return this.call<AdoptOutcome>("optimize/adopt", { method: "POST", body: JSON.stringify({ recipe }) });
  }
  async nextMoves() {
    const r = await this.call<{ opportunities: Opportunity[] }>("next");
    return r.opportunities;
  }
  nextRank() {
    return this.call<RankResponse>("next/rank", { method: "POST" });
  }
  setPreferences(prefs: NextPreferences) {
    return this.call<PreferencesView>("next/preferences", { method: "POST", body: JSON.stringify(prefs) });
  }
  aiConfig() {
    return this.call<AiConfigPublic>("ai/config");
  }
  setAiConfig(update: AiConfigUpdate) {
    return this.call<AiConfigPublic>("ai/config", { method: "POST", body: JSON.stringify(update) });
  }
  importRun(snapshot: ImportSnapshot) {
    return this.call<ImportOutcome>("import/run", { method: "POST", body: JSON.stringify(snapshot) });
  }
  advisorDismiss(id: string) {
    return this.call<AdvisorFeed>("advisor/dismiss", { method: "POST", body: JSON.stringify({ id }) });
  }
  advisorUnmute(rule: string) {
    return this.call<AdvisorFeed>("advisor/unmute", { method: "POST", body: JSON.stringify({ rule }) });
  }
  advisorPause(paused: boolean) {
    return this.call<AdvisorFeed>("advisor/pause", { method: "POST", body: JSON.stringify({ paused }) });
  }
  chatSend(scope: ChatScope, message: string) {
    return this.call<ChatReply>("chat", { method: "POST", body: JSON.stringify({ scope, message }) });
  }
  chatContext(scope: ChatScope) {
    return this.call<ContextSnapshot>("context", { method: "POST", body: JSON.stringify(scope) });
  }
  routeCalc(from: Id, to: Id, kind: RouteKind, demandPerMin: number, item: string | null) {
    return this.call<TrainAnswer | null>("route/calc", {
      method: "POST",
      body: JSON.stringify({ from, to, kind, demandPerMin, item }),
    });
  }
  uploadDocs() {
    return docsUploadUnsupported();
  }
  async newEmpire() {
    await this.call("new_empire", { method: "POST" });
  }
}

// Transport selection. The WEB build (`vite build --mode web`) sets the
// `__WASM_BACKEND__` compile-time define true, which selects the WasmBackend —
// the wasm Session in a Web Worker over IndexedDB — checked BEFORE the
// Tauri/Bridge split. The import is dynamic + branch-guarded so a desktop/dev
// build eliminates the whole wasm module (worker + .wasm) and stays
// byte-for-byte the old build.
async function selectBackend(): Promise<Backend> {
  if (__WASM_BACKEND__) {
    const { WasmBackend } = await import("./wasmBackend");
    return new WasmBackend();
  }
  return isTauri() ? new TauriBackend() : new BridgeBackend();
}

export const backend: Backend = await selectBackend();
