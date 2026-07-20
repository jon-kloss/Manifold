// Save-sync source: one interface over the two platform layers so DataMenu
// doesn't branch on the build. On WEB (`__WASM_BACKEND__`) it delegates to
// saveHandle.ts (browser File System Access + IndexedDB). On DESKTOP / dev
// bridge it goes through the backend's native commands (remembered save PATH +
// std::fs read + the meta KV store) — native FS always supports the silent
// timer re-read, so auto-sync is available without the FS-Access API.

import { backend } from "../state/backend";
import * as web from "./saveHandle";
import type { SyncMeta } from "./saveHandle";

const isDesktop = !__WASM_BACKEND__;

/** Auto-sync (silent timer re-read) capability: always on desktop; web needs
 *  the File System Access API to re-read a retained handle without a gesture. */
export function syncAutoCapable(): boolean {
  return isDesktop || web.fsAccessSupported();
}

/** True when a manual sync retains the source (no re-pick next time) — desktop
 *  (native path) and FS-Access web. A non-FS-Access browser re-picks each time. */
export function retainsSource(): boolean {
  return isDesktop || web.fsAccessSupported();
}

/** Web-only: a non-FS-Access browser has no retained handle, so the manual sync
 *  falls back to the classic file input (re-pick each time). Never on desktop. */
export function needsClassicPicker(): boolean {
  return !isDesktop && !web.fsAccessSupported();
}

const bytesToFile = (name: string, bytes: Uint8Array | null): File | null =>
  bytes ? new File([bytes as BlobPart], name) : null;

/** Pick a save (user gesture) and return a File to reconcile. Desktop remembers
 *  the native path immediately so later silent re-reads work. */
export async function pickSaveForSync(): Promise<File | null> {
  if (isDesktop) {
    const picked = await backend.syncPick?.();
    if (!picked) return null;
    await backend.syncMetaSet?.({ path: picked.path, name: picked.name, lastSyncedAt: Date.now() });
    return bytesToFile(picked.name, (await backend.syncRead?.(picked.path)) ?? null);
  }
  return web.pickSaveForSync();
}

/** Silent re-read of the remembered save (no gesture). Null if none/gone. */
export async function readStoredSilently(): Promise<File | null> {
  if (isDesktop) {
    const meta = await backend.syncMetaGet?.();
    if (!meta?.path) return null;
    return bytesToFile(meta.name, (await backend.syncRead?.(meta.path)) ?? null);
  }
  return web.readStoredHandleSilently();
}

export async function getSyncMeta(): Promise<SyncMeta | undefined> {
  if (isDesktop) {
    const m = await backend.syncMetaGet?.();
    return m ? { name: m.name, lastSyncedAt: m.lastSyncedAt ?? 0 } : undefined;
  }
  return web.getSyncMeta();
}

/** Record a completed sync (updates lastSyncedAt), preserving the remembered
 *  path on desktop. Returns the meta for the "last synced" affordance. */
export async function recordSyncMeta(name: string): Promise<SyncMeta> {
  const meta: SyncMeta = { name, lastSyncedAt: Date.now() };
  if (isDesktop) {
    const prev = await backend.syncMetaGet?.();
    await backend.syncMetaSet?.({ name, path: prev?.path, lastSyncedAt: meta.lastSyncedAt });
  } else {
    await web.setSyncMeta(meta);
  }
  return meta;
}

export { relTime } from "./saveHandle";
export type { SyncMeta } from "./saveHandle";
