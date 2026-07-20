// #117: the save/load DATA menu, docked in the titlebar's top-right corner —
// present on BOTH the map and the factory graph (it used to live in the map
// toolbar and vanished inside factories; auto-sync's timer now keeps ticking
// there too). Owns the whole load-data surface: import save (.sav → review
// modal), Docs.json upload/update (web), sync-from-save + auto-sync, and the
// two-click "start new empire" wipe. Escape (and invoking ⌘K search) closes
// the dropdown — its fixed backdrop would otherwise swallow clicks while the
// menu quietly stayed open.

import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useStore } from "../state/store";
import Glyph from "../lib/glyphs";
import ImportModal from "../import/ImportModal";
import {
  syncAutoCapable,
  retainsSource,
  needsClassicPicker,
  pickSaveForSync,
  readStoredSilently,
  getSyncMeta,
  recordSyncMeta,
  relTime,
  type SyncMeta,
} from "../import/syncSource";
import "./shell.css";

export default function DataMenu() {
  const importFile = useStore((s) => s.importFile);
  const setImportFile = useStore((s) => s.setImportFile);
  const uploadingDocs = useStore((s) => s.uploadingDocs);
  const uploadDocs = useStore((s) => s.uploadDocs);
  const newEmpire = useStore((s) => s.newEmpire);
  const factoryCount = useStore((s) => Object.keys(s.plan.factories).length);
  const syncImport = useStore((s) => s.syncImport);
  const pushToast = useStore((s) => s.pushToast);
  const catalogLoaded = useStore((s) => {
    const bv = s.gamedata.buildVersion;
    return !!bv && bv !== "fixture";
  });
  // "Sync from save" re-reads a previously imported save to reconcile — with
  // no imported save in the plan there is nothing to sync against, so the
  // control stays disabled until an import has landed (import-provenance
  // factories; syncMeta below also counts once a first sync recorded one).
  const hasImportedSave = useStore((s) =>
    Object.values(s.plan.factories).some((f) => f.createdBy?.kind === "import"),
  );
  const autoSync = useStore((s) => s.autoSync);
  const setAutoSync = useStore((s) => s.setAutoSync);
  const autoPull = useStore((s) => s.autoPull);

  const fileRef = useRef<HTMLInputElement>(null);
  const docsRef = useRef<HTMLInputElement>(null);
  const [dataMenu, setDataMenu] = useState(false);
  // "Start new empire": a two-click destructive latch — the first click arms
  // the confirm, the second wipes the plan (keeping the Docs.json).
  const [confirmReset, setConfirmReset] = useState(false);
  const closeDataMenu = useCallback(() => {
    setDataMenu(false);
    setConfirmReset(false);
  }, []);
  // Disarm the destructive confirm whenever the menu closes by ANY path — an
  // armed confirm surviving the close would wipe the plan on a single click.
  useEffect(() => {
    if (!dataMenu) setConfirmReset(false);
  }, [dataMenu]);

  // Escape closes the dropdown (top layer first — capture, so it works even
  // while focus sits in the header search input), and invoking the ⌘K search
  // closes it too: the menu's fixed backdrop sits above the search results'
  // stacking context, so the two must never be open at once.
  useEffect(() => {
    if (!dataMenu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // consumed: closing the menu must not ALSO clear the map selection
        // (this capture listener runs before the views' bubble handlers)
        e.stopPropagation();
        closeDataMenu();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        closeDataMenu(); // NOT consumed — ⌘K continues on to focus the search
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [dataMenu, closeDataMenu]);

  const loadDocsFile = useCallback(
    async (f: File) => {
      const bytes = new Uint8Array(await f.arrayBuffer());
      await uploadDocs(bytes); // uploadingDocs flag is store-managed
    },
    [uploadDocs],
  );

  // Sync Phase 2: "Sync from save" re-reads the retained save handle and
  // reconciles in one click. Gated on a real Docs.json (a fixture catalog would
  // quarantine most recipes → junk diffs); grayed with a how-to-enable tooltip
  // otherwise. Chrome/Edge get the no-re-pick handle path; elsewhere it falls
  // back to the classic file input (re-pick each time, no retention).
  const [syncMeta, setSyncMetaState] = useState<SyncMeta | undefined>();
  const [syncing, setSyncing] = useState(false);
  useEffect(() => {
    // A missing/blocked handle store just means no "last synced" affordance —
    // never a dead end. Runs on both builds now (desktop reads the meta KV).
    void getSyncMeta().then(setSyncMetaState).catch(() => {});
  }, []);
  // The catalog gate is WEB-only: web enforces "upload Docs.json first" (a
  // fixture catalog quarantines recipes → junk diffs), and there IS an upload
  // remedy. Desktop's catalog is host-provided (FICSIT_DOCS_JSON) with no
  // in-app upload, and import itself isn't catalog-gated there — so sync isn't
  // either.
  const catalogReady = !__WASM_BACKEND__ || catalogLoaded;
  const syncReady = catalogReady && (hasImportedSave || !!syncMeta);
  const onSync = useCallback(async () => {
    if (!syncReady || syncing) return; // defensive; the button is disabled too
    if (needsClassicPicker()) {
      // No File System Access (non-Chrome/Edge web) — reuse the classic picker
      // + ImportModal. Desktop always retains a native path, so it never lands here.
      fileRef.current?.click();
      return;
    }
    setSyncing(true);
    try {
      const file = await pickSaveForSync();
      if (!file) return; // user cancelled the picker / denied permission
      const outcome = await syncImport(file);
      if (outcome) setSyncMetaState(await recordSyncMeta(file.name));
    } catch (e) {
      // IDB/permission-layer failure (syncImport itself never rejects) — toast
      // instead of leaking an unhandled rejection.
      pushToast(`Couldn't sync from save — ${e instanceof Error ? e.message : String(e)}`, "error");
    } finally {
      setSyncing(false);
    }
  }, [syncReady, syncing, syncImport, pushToast]);

  // Sync Phase 3: auto-pull. Needs both the Docs.json gate AND File System
  // Access (the timer re-reads the retained handle with no user gesture, so it
  // is Chrome/Edge-only). Option B (in store.autoPull): conflict-free drift
  // applies silently; real conflicts open review. Mounted at titlebar level,
  // the timer now keeps running inside factory views too.
  const autoSyncReady = syncReady && syncAutoCapable();
  const autoPullBusy = useRef(false);
  const recordSync = useCallback(async (name: string) => {
    setSyncMetaState(await recordSyncMeta(name));
  }, []);
  const onToggleAutoSync = useCallback(async () => {
    if (!autoSyncReady) return; // defensive; the row is aria-disabled too
    if (autoSync.enabled) {
      setAutoSync(false);
      return;
    }
    if (syncing) return; // a pick/sync is already in flight — no double picker
    setSyncing(true);
    try {
      // Establish the source up front (this click is the user gesture the
      // silent timer can't provide later); bail if the user cancels the pick.
      let file = await readStoredSilently();
      if (!file) file = await pickSaveForSync();
      if (!file) return;
      setAutoSync(true);
      pushToast(
        __WASM_BACKEND__
          ? `Auto-sync on — every ${autoSync.intervalMin} min while this tab is open (Chrome/Edge)`
          : `Auto-sync on — re-reads your save every ${autoSync.intervalMin} min while the app is open`,
        "info",
      );
      const outcome = await autoPull(file); // one immediate pull so it visibly works
      if (outcome) await recordSync(file.name);
    } catch (e) {
      pushToast(`Couldn't start auto-sync — ${e instanceof Error ? e.message : String(e)}`, "error");
    } finally {
      setSyncing(false);
    }
  }, [autoSyncReady, autoSync, setAutoSync, pushToast, autoPull, recordSync, syncing]);
  useEffect(() => {
    if (!autoSync.enabled || !autoSyncReady) return;
    let cancelled = false;
    const tick = async () => {
      // Skip a tick that would collide: another pull running, or an open review
      // (never clobber a proposal the user is mid-decision on).
      if (cancelled || autoPullBusy.current || useStore.getState().reviewing) return;
      autoPullBusy.current = true;
      try {
        const file = await readStoredSilently();
        if (!file) return; // permission lapsed / no handle / path gone — skip quietly
        const outcome = await autoPull(file);
        if (outcome && !cancelled) await recordSync(file.name);
      } catch {
        /* transient read failure — the next tick retries */
      } finally {
        autoPullBusy.current = false;
      }
    };
    const id = window.setInterval(() => void tick(), autoSync.intervalMin * 60_000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [autoSync.enabled, autoSync.intervalMin, autoSyncReady, autoPull, recordSync]);

  return (
    <div className="data-menu-wrap">
      <button
        className={`btn btn-ghost ${dataMenu ? "active" : ""}`}
        onClick={() => (dataMenu ? closeDataMenu() : setDataMenu(true))}
        data-testid="btn-data-menu"
        title="Import a save or load your game's Docs.json"
      >
        {uploadingDocs ? "LOADING CATALOG…" : "DATA ▾"}
      </button>
      {dataMenu && (
        <>
          <div className="data-menu-backdrop" onClick={closeDataMenu} />
          <div className="data-menu" data-testid="data-menu">
            {/* Web: the catalog must load BEFORE a save so classes resolve.
                ONE menu layout, always — the ordered one. It never reshuffles
                after step ① lands (a menu that rearranges itself mid-flow
                reads as broken); the step-① row just flips to its loaded ✓
                state and keeps working as the swap-game-version action. */}
            {__WASM_BACKEND__ && (
              <div className="data-menu-order" data-testid="data-menu-order">
                Load in order: <b>① Upload Docs.json</b> → <b>② Import save</b>
              </div>
            )}
            {__WASM_BACKEND__ && (
              <button
                className="data-menu-item data-menu-step"
                onClick={() => {
                  setDataMenu(false);
                  docsRef.current?.click();
                }}
                disabled={uploadingDocs}
                data-testid="btn-upload-docs-first"
              >
                <span className="data-menu-item-label">
                  ① Upload Docs.json{catalogLoaded ? " ✓" : ""}
                </span>
                <span className="data-menu-item-sub">
                  {catalogLoaded
                    ? "loaded — upload again to swap game versions"
                    : "start here — the recipe catalog for your game version"}
                </span>
              </button>
            )}
            {/* The order is ENFORCED on web, not suggested: without a real
                catalog most save classes can't resolve, so step ② stays
                disabled (aria-disabled keeps the how-to tooltip on hover)
                until step ① lands. Desktop is unaffected. */}
            <button
              className="data-menu-item"
              onClick={() => {
                if (__WASM_BACKEND__ && !catalogLoaded) return;
                setDataMenu(false);
                fileRef.current?.click();
              }}
              aria-disabled={__WASM_BACKEND__ && !catalogLoaded}
              title={
                __WASM_BACKEND__ && !catalogLoaded
                  ? "Upload your Docs.json first (step ① above) — then import your save"
                  : undefined
              }
              data-testid="btn-import"
            >
              <span className="data-menu-item-label">
                <Glyph name="import" size={14} />{" "}
                {__WASM_BACKEND__ ? `② Import save${hasImportedSave ? " ✓" : ""}` : "Import save"}
              </span>
              <span className="data-menu-item-sub">
                {__WASM_BACKEND__ && !catalogLoaded
                  ? "unlocks after step ① — your Docs.json resolves the save's recipes"
                  : ".sav — your factories as a Built layer"}
              </span>
            </button>
            {(
              // One "Sync from save" control with an Auto toggle, on BOTH builds
              // (desktop re-reads a native path; web a retained handle).
              // aria-disabled (not the native attribute) keeps the how-to-enable
              // tooltip on hover — browsers suppress title on a natively-disabled
              // button. Turning Auto on disables the manual click (the timer owns it).
              <div className="data-menu-block sync-block">
                <div className="sync-row">
                  <button
                    className="data-menu-item sync-main"
                    onClick={() => {
                      if (!syncReady || syncing || autoSync.enabled) return;
                      setDataMenu(false);
                      void onSync();
                    }}
                    aria-disabled={!syncReady || syncing || autoSync.enabled}
                    title={
                      !catalogReady
                        ? "Upload your Docs.json first (step ① above) to enable save sync"
                        : !syncReady
                          ? "Import your save first — sync re-reads it to reconcile changes"
                          : autoSync.enabled
                            ? "Auto-sync is on — turn it off to sync manually"
                            : undefined
                    }
                    data-testid="btn-sync-save"
                  >
                    <span className="data-menu-item-label">
                      {autoSync.enabled ? "Auto-syncing" : syncing ? "Syncing…" : "Sync from save"}
                    </span>
                    <span className="data-menu-item-sub">
                      {!catalogReady
                        ? "needs your Docs.json — upload it above to enable"
                        : !syncReady
                          ? "needs an imported save — import yours above to enable"
                          : autoSync.enabled
                            ? `every ${autoSync.intervalMin} min · applies safe changes, asks on conflicts`
                            : syncMeta
                              ? `re-read ${syncMeta.name} · synced ${relTime(syncMeta.lastSyncedAt)}`
                              : retainsSource()
                                ? "re-read your save & reconcile — no re-pick next time"
                                : "re-read your save & reconcile"}
                    </span>
                  </button>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={autoSync.enabled}
                    aria-disabled={!autoSyncReady}
                    className={`sync-auto ${autoSync.enabled ? "on" : ""}`}
                    onClick={() => void onToggleAutoSync()}
                    title={
                      autoSyncReady
                        ? autoSync.enabled
                          ? "Auto-sync on — click to turn off"
                          : __WASM_BACKEND__
                            ? "Auto-sync: re-read on a timer (Chrome/Edge, this tab open)"
                            : "Auto-sync: re-read your save on a timer while the app is open"
                        : !catalogReady
                          ? "Upload your Docs.json first (step ① above) to enable save sync"
                          : !syncReady
                            ? "Import your save first — sync re-reads it to reconcile changes"
                            : "Auto-sync needs the File System Access API — use Chrome or Edge"
                    }
                    data-testid="btn-auto-sync"
                  >
                    <span className="sync-auto-text mono">AUTO</span>
                    <span className="sync-auto-track" aria-hidden>
                      <span className="sync-auto-knob" />
                    </span>
                  </button>
                </div>
                {autoSync.enabled && autoSyncReady && (
                  <div className="autosync-intervals" data-testid="autosync-intervals">
                    <span className="autosync-label mono">every</span>
                    {[5, 10, 15].map((n) => (
                      <button
                        key={n}
                        type="button"
                        className={`autosync-chip ${n === autoSync.intervalMin ? "active" : ""}`}
                        onClick={() => setAutoSync(true, n)}
                        data-testid={`autosync-${n}`}
                      >
                        {n}m
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}
            {/* Start over: a cross-platform Session::new_empire (SQLite
                wipe on desktop, store reset → IndexedDB on web), shown only
                when there's something to clear. Two-click confirm guards the
                destructive wipe; the gamedata catalog is kept. */}
            {factoryCount > 0 && (
              <button
                className={`data-menu-item data-menu-danger ${confirmReset ? "armed" : ""}`}
                onClick={() => {
                  if (!confirmReset) {
                    setConfirmReset(true);
                    return;
                  }
                  closeDataMenu();
                  void newEmpire();
                }}
                data-testid="btn-new-empire"
              >
                <span className="data-menu-item-label">
                  {confirmReset ? "Click again to delete everything" : "Start new empire"}
                </span>
                <span className="data-menu-item-sub">
                  {confirmReset
                    ? `deletes all ${factoryCount} ${factoryCount === 1 ? "factory" : "factories"} & routes — keeps your Docs.json`
                    : "wipe the current plan to import a fresh save"}
                </span>
              </button>
            )}
            {/* Path hints mirror the enforced load order on web: Docs.json
                rows lead, the save row follows (desktop shows save only) —
                numbered the SAME whether or not a catalog is loaded, so the
                menu never reshuffles mid-flow. */}
            <div className="data-menu-hint">
              <div className="data-menu-hint-head">Or drag &amp; drop a file anywhere</div>
              {__WASM_BACKEND__ && (
                <>
                  <div className="data-menu-hint-row">
                    <span className="data-menu-hint-key">① Docs (Steam)</span>
                    <code>…\steamapps\common\Satisfactory\CommunityResources\Docs\en-US.json</code>
                  </div>
                  <div className="data-menu-hint-row">
                    <span className="data-menu-hint-key">① Docs (Epic)</span>
                    <code>…\Epic Games\SatisfactoryEarlyAccess\CommunityResources\Docs\en-US.json</code>
                  </div>
                </>
              )}
              <div className="data-menu-hint-row">
                <span className="data-menu-hint-key">{__WASM_BACKEND__ ? "② Save" : "Save"}</span>
                <code>%LOCALAPPDATA%\FactoryGame\Saved\SaveGames\</code>
              </div>
            </div>
          </div>
        </>
      )}
      <input
        ref={fileRef}
        type="file"
        accept=".sav"
        style={{ display: "none" }}
        data-testid="import-file-input"
        onChange={(e) => {
          const f = e.target.files?.[0];
          if (f) setImportFile(f);
          e.currentTarget.value = "";
        }}
      />
      {__WASM_BACKEND__ && (
        <input
          ref={docsRef}
          type="file"
          accept=".json,application/json"
          style={{ display: "none" }}
          data-testid="docs-file-input"
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.currentTarget.value = "";
            if (f) void loadDocsFile(f);
          }}
        />
      )}
      {/* Portal to <body>: the modal's absolute inset-0 scrim must cover the
          viewport, not this titlebar-corner wrapper (position: relative). */}
      {importFile &&
        createPortal(<ImportModal file={importFile} onClose={() => setImportFile(null)} />, document.body)}
    </div>
  );
}
