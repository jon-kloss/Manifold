// Backend abstraction: identical command surface over Tauri IPC (production)
// or the dev-bridge HTTP API (headless development). Rust owns canonical
// state in both — the renderer only ever sees patches (SDD §4).

import type {
  AdvisorFeed,
  ChatReply,
  ChatScope,
  Command,
  ContextSnapshot,
  EditResponse,
  ImportOutcome,
  ImportSnapshot,
  InitPayload,
  JobProgress,
  Proposal,
  ProposalConsequence,
  ViewState,
  WizardGoal,
} from "./types";

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
  importRun(snapshot: ImportSnapshot): Promise<ImportOutcome>;
  advisorDismiss(id: string): Promise<AdvisorFeed>;
  advisorUnmute(rule: string): Promise<AdvisorFeed>;
  advisorPause(paused: boolean): Promise<AdvisorFeed>;
  chatSend(scope: ChatScope, message: string): Promise<ChatReply>;
  chatContext(scope: ChatScope): Promise<ContextSnapshot>;
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
}

export const backend: Backend = isTauri() ? new TauriBackend() : new BridgeBackend();
