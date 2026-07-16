// On-device AI (WebLLM / WebGPU): runs a small instruct model entirely in the
// browser so the NEXT MOVES ranking/narration works with no API key and no
// server. Everything here is behind a DYNAMIC import + a worker instantiated
// only on opt-in, so a user who never enables the feature pays zero bytes.
//
// The model only reorders and narrates the solver's candidates; the Rust
// firewall (apply_rank_reply) validates every reply, so weak on-device output
// degrades to the heuristic order — never a wrong number.

import type { MLCEngineInterface, InitProgressReport } from "@mlc-ai/web-llm";

/** Default on-device model: ~0.9 GB q4f16 Llama-3.2-1B — small enough to load on
 *  a desktop GPU, capable enough to emit the rank JSON the firewall expects. */
export const DEFAULT_WEBLLM_MODEL = "Llama-3.2-1B-Instruct-q4f16_1-MLC";

/** WebGPU is required to run a model in-browser. Absent on most iOS Safari and
 *  on browsers without GPU access — the caller shows an honest "needs WebGPU"
 *  note instead of offering the download. */
export function webgpuSupported(): boolean {
  return typeof navigator !== "undefined" && "gpu" in navigator && Boolean((navigator as { gpu?: unknown }).gpu);
}

let engine: MLCEngineInterface | null = null;
let worker: Worker | null = null;
let currentModel: string | null = null;

export function engineReady(): boolean {
  return engine !== null;
}

/** Load (or reuse) the on-device engine for `model`. `onProgress` reports the
 *  download/compile fraction (0..1) with a human label. Weights are cached by
 *  the browser (Cache API), so a second load after a reload is fast — no
 *  re-download. Throws if WebGPU is unavailable or the load fails. */
export async function loadEngine(model: string, onProgress: (fraction: number, text: string) => void): Promise<void> {
  if (engine && currentModel === model) return;
  if (!webgpuSupported()) throw new Error("this browser has no WebGPU — on-device AI needs a desktop Chrome/Edge");
  // A model switch tears down the old engine first.
  if (engine && currentModel !== model) await unloadEngine();
  const webllm = await import("@mlc-ai/web-llm");
  if (!worker) {
    worker = new Worker(new URL("./webllmWorker.ts", import.meta.url), {
      type: "module",
    });
  }
  engine = await webllm.CreateWebWorkerMLCEngine(worker, model, {
    initProgressCallback: (r: InitProgressReport) => onProgress(r.progress, r.text),
  });
  currentModel = model;
}

/** Run one system+user turn and return the raw assistant text (validated Rust
 *  side). Low temperature — we want the structured reorder, not creativity.
 *  `maxTokens` comes from the prepared job (it scales with candidate count, so
 *  the reply JSON never truncates mid-string on a large empire). */
export async function runRank(system: string, user: string, maxTokens: number): Promise<string> {
  if (!engine) throw new Error("on-device model is not loaded");
  const res = await engine.chat.completions.create({
    messages: [
      { role: "system", content: system },
      { role: "user", content: user },
    ],
    temperature: 0.2,
    max_tokens: maxTokens,
  });
  return res.choices[0]?.message?.content ?? "";
}

/** Free the engine + GPU memory and drop the worker (on disable). Best-effort. */
export async function unloadEngine(): Promise<void> {
  try {
    await engine?.unload();
  } catch {
    /* ignore — we're tearing down anyway */
  }
  worker?.terminate();
  engine = null;
  worker = null;
  currentModel = null;
}

/** Delete `model`'s cached weights/wasm/config from browser storage, reclaiming
 *  the ~0.9 GB the download occupies. Imports the library on demand — only ever
 *  called after a model was downloaded, so the chunk is already cached. Unloads
 *  first so nothing is mid-read. Best-effort; resolves even if parts were
 *  already evicted. */
export async function deleteModelFromCache(model: string): Promise<void> {
  await unloadEngine();
  const webllm = await import("@mlc-ai/web-llm");
  await webllm.deleteModelAllInfoInCache(model);
}
