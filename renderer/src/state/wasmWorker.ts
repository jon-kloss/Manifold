// WASM session worker (Web Phase 3). Runs the Rust `Session` (via the wasm
// `WebSession`) OFF the UI thread — the same off-thread pattern parseWorker.ts
// uses for save parsing — and OWNS the IndexedDB snapshot around it.
//
// Persistence is a SNAPSHOT layer, not a PlanStore impl: `PlanStore` is
// synchronous and IndexedDB is async, so the wasm Session keeps its in-memory
// `MemoryPlanStore` and durability is a blob. After every MUTATING dispatch the
// worker exports the whole store (`export_blob`) and `put`s it under one
// "current plan" key; on boot it reads that blob back and reconstructs the
// session from it (else the bundled fixture). IndexedDB `put` is atomic per
// key, so browser durability is a clean last-edit snapshot (no partial write).
//
// WHICH dispatches snapshot is NOT a hand-kept allowlist here (that drifted:
// chat_send drafts a proposal but was omitted). `dispatch` returns an envelope
// `{ mutated, result }` and Rust is the single source of truth — each arm
// declares whether it wrote the store. The worker snapshots iff `mutated`.
//
// Requests are serialized through a promise chain so a mutation's snapshot
// write always completes before the next request mutates — matching the
// mutex-serialized desktop shell. `dispatch` itself is synchronous inside the
// worker (a Rust call), so within one request there is no interleaving.

import init, { WebSession } from "../wasm/web-pkg/web.js";
import wasmUrl from "../wasm/web-pkg/web_bg.wasm?url";

const DB_NAME = "ficsit-planner";
const STORE = "plans";
const KEY = "current";
/** Where a blob that fails to reconstruct a session is parked (M2) so a corrupt
 *  or version-mismatched save never bricks the app AND is not silently lost. */
const CORRUPT_KEY = "current-corrupt";
/** The uploaded Docs.json (Phase 4a), kept in the SAME object store under its
 *  own key so a real game catalog survives reloads. `undefined` → the bundled
 *  fixture compiled into web_bg.wasm. Stored as the raw uploaded bytes (the
 *  Rust `decode` handles gzip), and passed to `WebSession(docs, plan)` on boot. */
const DOCS_KEY = "docs";
/** Where uploaded docs that fail to reconstruct a session are parked, mirroring
 *  CORRUPT_KEY for the plan (Phase 4a). A wasm deploy whose `parse_docs` no
 *  longer accepts previously-stored bytes must degrade to the bundled fixture,
 *  never brick the boot — durability of a catalog cannot cost opening the app. */
const DOCS_CORRUPT_KEY = "docs-corrupt";

/** The dispatch envelope Rust returns (M1): `mutated` is the authoritative
 *  "did this write the store?" signal; `result` is the marshaled reply. */
interface Envelope {
  mutated: boolean;
  result: unknown;
}

let dbPromise: Promise<IDBDatabase> | null = null;
function openDb(): Promise<IDBDatabase> {
  dbPromise ??= new Promise<IDBDatabase>((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("indexedDB open failed"));
  });
  return dbPromise;
}

async function loadBlob(): Promise<Uint8Array | undefined> {
  const db = await openDb();
  return new Promise<Uint8Array | undefined>((resolve, reject) => {
    const req = db.transaction(STORE, "readonly").objectStore(STORE).get(KEY);
    req.onsuccess = () => {
      const v = req.result as Uint8Array | ArrayBuffer | undefined;
      if (!v) resolve(undefined);
      else resolve(v instanceof Uint8Array ? v : new Uint8Array(v));
    };
    req.onerror = () => reject(req.error ?? new Error("indexedDB get failed"));
  });
}

async function saveBlob(bytes: Uint8Array): Promise<void> {
  const db = await openDb();
  return new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    // Copy off the wasm heap: the Uint8Array `export_blob` returns is a view
    // that a later mutation would invalidate; IndexedDB stores a structured
    // clone, but the clone must snapshot stable bytes, so hand it a fresh copy.
    tx.objectStore(STORE).put(new Uint8Array(bytes), KEY);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error("indexedDB put failed"));
  });
}

async function loadDocs(): Promise<Uint8Array | undefined> {
  const db = await openDb();
  return new Promise<Uint8Array | undefined>((resolve, reject) => {
    const req = db.transaction(STORE, "readonly").objectStore(STORE).get(DOCS_KEY);
    req.onsuccess = () => {
      const v = req.result as Uint8Array | ArrayBuffer | undefined;
      if (!v) resolve(undefined);
      else resolve(v instanceof Uint8Array ? v : new Uint8Array(v));
    };
    req.onerror = () => reject(req.error ?? new Error("indexedDB docs get failed"));
  });
}

/** Persist the uploaded docs AND the (preserved) plan in ONE IndexedDB
 *  transaction. An upload swaps the catalog and rewrites the plan snapshot from
 *  the new session, and those two keys must never end up inconsistent — a single
 *  atomic tx means either both land or neither does (IndexedDB aborts the tx on
 *  any failure), so uploadDocs can persist-before-swap without a partial-write
 *  window. Both byte arrays are copied off the wasm heap by `new Uint8Array`. */
async function saveDocsAndPlan(docs: Uint8Array, plan: Uint8Array): Promise<void> {
  const db = await openDb();
  return new Promise<void>((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    const os = tx.objectStore(STORE);
    os.put(new Uint8Array(docs), DOCS_KEY);
    os.put(new Uint8Array(plan), KEY);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error ?? new Error("indexedDB docs+plan put failed"));
  });
}

/** M2: park an unreadable blob under the `-corrupt` key so it is preserved for
 *  debugging/recovery but no longer sits on the boot path. Best-effort — a
 *  failure to back it up must not stop the app from booting fresh. */
async function backupCorruptBlob(bytes: Uint8Array): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).put(new Uint8Array(bytes), CORRUPT_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("indexedDB backup put failed"));
    });
  } catch (e) {
    console.warn("[wasm-worker] could not back up the corrupt blob", e);
  }
}

/** Park unreadable uploaded docs under the `-corrupt` key AND clear DOCS_KEY so
 *  the bad catalog leaves the boot path (a fresh boot degrades to the bundled
 *  fixture instead of re-throwing on it forever). Best-effort like the blob
 *  backup — a failure here must not stop the app booting. */
async function backupCorruptDocs(bytes: Uint8Array): Promise<void> {
  try {
    const db = await openDb();
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE, "readwrite");
      const os = tx.objectStore(STORE);
      os.put(new Uint8Array(bytes), DOCS_CORRUPT_KEY);
      os.delete(DOCS_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("indexedDB docs backup failed"));
    });
  } catch (e) {
    console.warn("[wasm-worker] could not back up the corrupt docs", e);
  }
}

/** Construct a WebSession, returning null (not throwing) on failure so the boot
 *  cascade can degrade one argument at a time. */
function tryConstruct(docs: Uint8Array | undefined, blob: Uint8Array | undefined): WebSession | null {
  try {
    return new WebSession(docs, blob);
  } catch (e) {
    console.warn("[wasm-worker] WebSession construction failed", e);
    return null;
  }
}

let session: WebSession | null = null;
let ready: Promise<void> | null = null;
function ensureReady(): Promise<void> {
  ready ??= (async () => {
    await init({ module_or_path: wasmUrl });
    const [blob, docs] = await Promise.all([loadBlob(), loadDocs()]);
    // docs → a previously-uploaded real Docs.json (Phase 4a); undefined → the
    // bundled fixture catalog compiled into the wasm. blob → reconstruct the
    // saved plan, else a fresh empty one. EITHER argument can make construction
    // throw — a corrupt/version-mismatched plan blob (M2) OR uploaded docs whose
    // bytes a newer wasm's parse_docs no longer accepts — so the boot degrades
    // one argument at a time. Durability of neither the plan NOR the catalog may
    // cost the ability to open the app; a fresh fixture session always boots.

    // 1. Full fidelity: uploaded catalog + saved plan.
    session = tryConstruct(docs, blob);
    if (session) return;

    // 2. Something failed. If a plan blob is present, suspect it first: keep the
    //    catalog, drop the plan (back it up under -corrupt).
    if (blob) {
      const s = tryConstruct(docs, undefined);
      if (s) {
        console.warn(
          "[wasm-worker] saved plan is unreadable — starting fresh; a backup was kept under the -corrupt key",
        );
        await backupCorruptBlob(blob);
        session = s;
        return;
      }
    }

    // 3. Still failing → the DOCS are unreadable (a wasm/parse_docs change no
    //    longer accepts the stored bytes). Degrade to the bundled fixture, but
    //    keep the plan if it loads on the fixture. Park + clear the bad docs so
    //    the next boot does not re-throw on them.
    if (docs) {
      console.warn(
        "[wasm-worker] uploaded Docs.json is unreadable — falling back to the bundled fixture; a backup was kept under the docs-corrupt key",
      );
      await backupCorruptDocs(docs);
      const s = tryConstruct(undefined, blob);
      if (s) {
        session = s;
        return;
      }
      // Plan ALSO unreadable on the fixture → back it up too.
      if (blob) await backupCorruptBlob(blob);
    }

    // 4. Nothing salvageable: a fresh fixture session. This construction cannot
    //    fail (no external bytes) — the app always opens.
    session = new WebSession(undefined, undefined);
  })();
  return ready;
}

/** Phase 4a: swap in an uploaded Docs.json without losing the current plan.
 *  gamedata is set only at construction, so this REBUILDS the WebSession from
 *  the uploaded catalog bytes plus the current plan's exported snapshot.
 *
 *  Ordering is deliberate: build the new session, PERSIST docs + plan atomically,
 *  and only THEN swap the live `session`. If persistence throws (e.g. a
 *  QuotaExceededError on a multi-MB real Docs.json), the OLD session stays live
 *  and IndexedDB is unchanged — the failure surfaces on the status chip and a
 *  reload sees the un-swapped state, with no silent divergence between the worker,
 *  the (un-hydrated) UI, and storage. `next` is built from the preserved plan, so
 *  reaching the write proves newDocs+plan reconstructs — KEY stays compatible with
 *  DOCS_KEY. `ensureReady` always leaves `session` non-null (the boot cascade's
 *  fixture fallback can't fail), so no pre-session branch is needed. */
async function uploadDocs(bytes: Uint8Array): Promise<void> {
  await ensureReady();
  // Copy the preserved plan off the wasm heap before constructing the new
  // session (whose allocation could otherwise detach the view).
  const planBlob = session!.export_blob();
  const planCopy = planBlob.length > 0 ? new Uint8Array(planBlob) : undefined;
  const next = new WebSession(bytes, planCopy);
  await saveDocsAndPlan(bytes, next.export_blob());
  session = next;
}

interface Req {
  id: number;
  /** Control message kind. Absent → the normal `dispatch(cmd, args)` path.
   *  "upload_docs" → rebuild the session over an uploaded Docs.json (Phase 4a).
   *  ("new_empire" is a plain dispatch — Session::new_empire — not a control
   *   message, so the worker's snapshot-after-mutate persists the empty plan.) */
  kind?: "upload_docs";
  cmd?: string;
  args?: unknown;
  /** upload_docs payload: the raw uploaded Docs.json bytes. */
  bytes?: Uint8Array;
}

// Serialize every request behind a single promise chain (see header): a
// mutation's snapshot write completes before the next request runs.
let chain: Promise<void> = Promise.resolve();

// L1: view-state writes (map pan/zoom fire one per gesture) are coalesced. A
// `set_view_state` mutation arms a trailing timer instead of snapshotting
// inline; a subsequent REAL mutation flushes it immediately so no view-state
// write is ever lost or reordered ahead of a plan edit.
const VIEW_DEBOUNCE_MS = 500;
let viewFlushTimer: ReturnType<typeof setTimeout> | null = null;
let viewSnapshotPending = false;

function cancelViewTimer(): void {
  if (viewFlushTimer !== null) {
    clearTimeout(viewFlushTimer);
    viewFlushTimer = null;
  }
}

/** Snapshot the store to IndexedDB now, clearing any pending debounced
 *  view-state write (this snapshot subsumes it). */
async function snapshotNow(): Promise<void> {
  cancelViewTimer();
  viewSnapshotPending = false;
  await saveBlob(session!.export_blob());
}

/** Arm (or re-arm) the trailing debounce that flushes a view-state snapshot.
 *  The timer body runs on the serialization chain so it never races a request. */
function scheduleViewSnapshot(): void {
  viewSnapshotPending = true;
  cancelViewTimer();
  viewFlushTimer = setTimeout(() => {
    viewFlushTimer = null;
    chain = chain.then(async () => {
      if (!viewSnapshotPending || !session) return;
      viewSnapshotPending = false;
      try {
        await saveBlob(session.export_blob());
      } catch (e) {
        console.warn("[wasm-worker] debounced view-state snapshot failed", e);
      }
    });
  }, VIEW_DEBOUNCE_MS);
}

self.onmessage = (e: MessageEvent<Req>) => {
  const { id, kind, cmd, args, bytes } = e.data;
  chain = chain.then(async () => {
    try {
      // Control path: rebuild the session over an uploaded Docs.json (Phase 4a).
      // Not a `dispatch` — gamedata is construction-only — so it is handled here,
      // on the same serialization chain so no request interleaves the swap.
      if (kind === "upload_docs") {
        await uploadDocs(bytes ?? new Uint8Array());
        self.postMessage({ id, ok: true, result: undefined });
        return;
      }
      await ensureReady();
      const env = session!.dispatch(cmd!, args) as Envelope;
      if (env.mutated) {
        // L1: coalesce the frequent view-state write; every other mutation
        // snapshots inline (and flushes any pending view-state write with it).
        if (cmd === "set_view_state") scheduleViewSnapshot();
        else await snapshotNow();
      }
      self.postMessage({ id, ok: true, result: env.result });
    } catch (err) {
      // dispatch rejects with the Session error MESSAGE (a JsValue string) or a
      // panic Error; normalize to a string the renderer surfaces on its chip.
      const message = err instanceof Error ? err.message : String(err);
      self.postMessage({ id, ok: false, error: message });
    }
  });
};
