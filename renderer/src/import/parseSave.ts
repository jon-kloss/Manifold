// Parse a Satisfactory `.sav` into an `ImportSnapshot` in a Web Worker (the
// community-reverse-engineered format is heavy; keep it off the main thread).
// Shared by the ImportModal preview flow and the one-click "Sync from save"
// path so both spawn the exact same parser + reducer.

import type { ImportSnapshot } from "../state/types";

/** Parse raw `.sav` bytes off the main thread. The desktop save-sync path
 *  reads bytes natively (Rust `read_save`) and calls this directly; the browser
 *  import/sync paths go through `parseSaveFile`, which reads the File's bytes
 *  first. Both spawn the exact same worker + reducer. */
export function parseSaveBytes(name: string, bytes: ArrayBuffer): Promise<ImportSnapshot> {
  return new Promise<ImportSnapshot>((resolve, reject) => {
    const worker = new Worker(new URL("./parseWorker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e: MessageEvent<{ snapshot?: ImportSnapshot; error?: string }>) => {
      worker.terminate();
      if (e.data.error || !e.data.snapshot) {
        reject(new Error(e.data.error ?? "empty parse"));
      } else {
        resolve(e.data.snapshot);
      }
    };
    worker.onerror = (e) => {
      worker.terminate();
      reject(new Error(e.message || "save parse worker crashed"));
    };
    worker.postMessage({ name, bytes }, [bytes]);
  });
}

export function parseSaveFile(file: File): Promise<ImportSnapshot> {
  return file.arrayBuffer().then((bytes) => parseSaveBytes(file.name, bytes));
}
