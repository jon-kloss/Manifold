// Backend abstraction: identical command surface over Tauri IPC (production)
// or the dev-bridge HTTP API (headless development). Rust owns canonical
// state in both — the renderer only ever sees patches (SDD §4).

import type { Command, EditResponse, InitPayload, ViewState } from "./types";

export interface Backend {
  hydrate(): Promise<InitPayload>;
  edit(cmds: Command[]): Promise<EditResponse>;
  undo(): Promise<EditResponse | null>;
  redo(): Promise<EditResponse | null>;
  setViewState(v: ViewState): Promise<void>;
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
}

export const backend: Backend = isTauri() ? new TauriBackend() : new BridgeBackend();
