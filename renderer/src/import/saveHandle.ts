// "Sync from save" plumbing (web, Chrome/Edge): retain the user's chosen `.sav`
// as a FileSystemFileHandle so re-syncing re-reads the same file with no OS
// picker. Handles are structured-cloneable, so they persist straight into a
// small IndexedDB (kept separate from the wasm worker's `ficsit-planner` DB so
// their version numbers never collide). Where the File System Access API is
// unavailable (Firefox/Safari), callers fall back to a classic file input.

// The permission methods live on FileSystemHandle in the spec but are missing
// from lib.dom's typings — declare the shape we use.
type PermState = "granted" | "denied" | "prompt";
interface HandleWithPermission extends FileSystemFileHandle {
  queryPermission?(descriptor: { mode: "read" | "readwrite" }): Promise<PermState>;
  requestPermission?(descriptor: { mode: "read" | "readwrite" }): Promise<PermState>;
}
interface PickerWindow {
  showOpenFilePicker?(options?: {
    multiple?: boolean;
    excludeAcceptAllOption?: boolean;
    types?: { description?: string; accept: Record<string, string[]> }[];
  }): Promise<FileSystemFileHandle[]>;
}

/** True on browsers that expose the File System Access re-grab (Chrome/Edge). */
export function fsAccessSupported(): boolean {
  return typeof window !== "undefined" && typeof (window as PickerWindow).showOpenFilePicker === "function";
}

export interface SyncMeta {
  /** the save's file name, for the "last synced" affordance */
  name: string;
  /** epoch ms of the last successful re-read */
  lastSyncedAt: number;
}

const DB_NAME = "ficsit-planner-fs";
const STORE = "handles";
const HANDLE_KEY = "saveHandle";
const META_KEY = "syncMeta";

let dbPromise: Promise<IDBDatabase> | null = null;
function openDb(): Promise<IDBDatabase> {
  dbPromise ??= new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(STORE);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => {
      dbPromise = null; // don't cache a rejection — a later call should retry
      reject(req.error ?? new Error("indexedDB open failed"));
    };
  });
  return dbPromise;
}

async function idbGet<T>(key: string): Promise<T | undefined> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const req = db.transaction(STORE, "readonly").objectStore(STORE).get(key);
    req.onsuccess = () => resolve(req.result as T | undefined);
    req.onerror = () => reject(req.error);
  });
}

async function idbPut(key: string, value: unknown): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

async function idbDelete(key: string): Promise<void> {
  const db = await openDb();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE, "readwrite");
    tx.objectStore(STORE).delete(key);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
}

export function getSyncMeta(): Promise<SyncMeta | undefined> {
  return idbGet<SyncMeta>(META_KEY);
}
export function setSyncMeta(meta: SyncMeta): Promise<void> {
  return idbPut(META_KEY, meta);
}

/** Forget the retained handle and its metadata (e.g. after a stale re-pick). */
export async function forgetSaveHandle(): Promise<void> {
  await idbDelete(HANDLE_KEY);
  await idbDelete(META_KEY);
}

// Ensure read permission on a retained handle, re-prompting if needed. Must be
// reached from a user gesture — the re-request opens a permission dialog.
async function ensureReadPermission(handle: HandleWithPermission): Promise<boolean> {
  if (!handle.queryPermission || !handle.requestPermission) return true; // older impls skip prompts
  if ((await handle.queryPermission({ mode: "read" })) === "granted") return true;
  return (await handle.requestPermission({ mode: "read" })) === "granted";
}

/**
 * Resolve the `.sav` File to sync from. Reuses the retained handle when we still
 * have read permission and the file still exists; otherwise opens the OS picker
 * once and retains the new handle. Returns `null` if the user cancels the picker
 * (or permission is refused) — the caller should then do nothing.
 *
 * Only call after a user gesture, and only when {@link fsAccessSupported}.
 */
export async function pickSaveForSync(): Promise<File | null> {
  const stored = await idbGet<HandleWithPermission>(HANDLE_KEY);
  if (stored) {
    try {
      if (await ensureReadPermission(stored)) {
        return await stored.getFile(); // throws NotFoundError if the file moved/was deleted
      }
    } catch {
      // stale or unreadable handle — fall through to a fresh pick
    }
  }
  const picker = window as PickerWindow;
  if (!picker.showOpenFilePicker) return null;
  let handle: FileSystemFileHandle;
  try {
    [handle] = await picker.showOpenFilePicker({
      multiple: false,
      excludeAcceptAllOption: false,
      types: [{ description: "Satisfactory save", accept: { "application/octet-stream": [".sav"] } }],
    });
  } catch {
    return null; // user dismissed the picker
  }
  await idbPut(HANDLE_KEY, handle);
  return handle.getFile();
}

/** Compact "time since" for the last-synced affordance. */
export function relTime(then: number, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - then) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return d === 1 ? "yesterday" : `${d}d ago`;
}
