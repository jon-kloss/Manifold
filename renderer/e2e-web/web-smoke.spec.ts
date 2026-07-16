// Web Phase 3 ACCEPTANCE PROOF — the built web app (dist-web) runs entirely in
// the browser: the wasm Session in a Web Worker over IndexedDB, no backend
// server. This spec drives that artifact end to end and proves the core loop
// the whole phase exists for: BOOT (hydrate over the bundled fixture) → EDIT
// (create a factory through the app's real store → WasmBackend → worker → wasm)
// → RENDER (the projection the map canvas draws from reflects it, and the
// selection drawer shows it) → RELOAD → PERSIST (the factory survives the
// IndexedDB round-trip: worker reconstructs the MemoryPlanStore from the saved
// snapshot blob).
//
// Isolation: Playwright gives each test a fresh browser context, so IndexedDB
// starts empty — the first boot is a clean fixture plan, and the reload stays
// in the same context so the snapshot persists across it. This suite has its
// own config (playwright.web.config.ts) and never touches the 31 dev-bridge
// specs.

import { test, expect, type Page } from "@playwright/test";
import { fileURLToPath } from "node:url";

// The bundled fixture catalog on disk — the SAME JSON compiled into the wasm as
// the default. Uploading it exercises the whole upload→persist→reload path; the
// observable signal is `buildVersion` flipping "fixture" → "uploaded" (the wasm
// tags an uploaded Docs.json), which is what turns the first-run upload prompt
// off. Resolved relative to this spec so cwd does not matter.
const DOCS_FIXTURE = fileURLToPath(
  new URL("../../crates/gamedata/assets/docs-fixture.json", import.meta.url),
);

/** The in-page store + backend handles store.ts exposes in the web build
 *  (__WASM_BACKEND__ guard). Only the members the smoke needs are typed. */
interface StoreWin {
  __ficsitStore: {
    getState(): {
      ready: boolean;
      error: string | null;
      plan: {
        factories: Record<string, { name: string }>;
        proposals: Record<string, unknown>;
      };
      gamedata: { buildVersion: string; recipes: Record<string, unknown> };
      dispatch(cmds: unknown[], opts?: { select?: boolean }): Promise<string[] | null>;
    };
  };
  __ficsitBackend: {
    chatSend(
      scope: { scope: "empire" },
      message: string,
    ): Promise<{ proposal: string | null }>;
  };
}

/** Wait for the wasm session to boot and hydrate (or surface a fatal error). */
async function waitReady(page: Page): Promise<void> {
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 30_000 });
  await page.waitForFunction(
    () => {
      const w = window as unknown as Partial<StoreWin>;
      const st = w.__ficsitStore?.getState();
      return !!st && (st.ready || st.error !== null);
    },
    { timeout: 30_000 },
  );
  const error = await page.evaluate(
    () => (window as unknown as StoreWin).__ficsitStore.getState().error,
  );
  expect(error, "the wasm session hydrated without a fatal error").toBeNull();
}

function factoryCount(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      Object.keys((window as unknown as StoreWin).__ficsitStore.getState().plan.factories).length,
  );
}

function proposalCount(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      Object.keys((window as unknown as StoreWin).__ficsitStore.getState().plan.proposals).length,
  );
}

function buildVersion(page: Page): Promise<string> {
  return page.evaluate(
    () => (window as unknown as StoreWin).__ficsitStore.getState().gamedata.buildVersion,
  );
}

// The IndexedDB the worker owns — same names as wasmWorker.ts. The smoke reaches
// into it directly to seed a corrupt blob (M2) and to read the -corrupt backup.
const DB_NAME = "ficsit-planner";
const STORE = "plans";
const KEY = "current";
const CORRUPT_KEY = "current-corrupt";
const DOCS_KEY = "docs";
const DOCS_CORRUPT_KEY = "docs-corrupt";

/** Seed a value under an IndexedDB key in the worker's DB (creates the store if
 *  the first boot hasn't yet). Used to plant corrupt bytes on the boot path. */
function seedKey(page: Page, key: string, bytes: number[]): Promise<void> {
  return page.evaluate(
    ({ dbName, store, k, b }) =>
      new Promise<void>((resolve, reject) => {
        const req = indexedDB.open(dbName, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(store);
        req.onsuccess = () => {
          const db = req.result;
          const tx = db.transaction(store, "readwrite");
          tx.objectStore(store).put(new Uint8Array(b), k);
          tx.oncomplete = () => {
            db.close();
            resolve();
          };
          tx.onerror = () => reject(tx.error);
        };
        req.onerror = () => reject(req.error);
      }),
    { dbName: DB_NAME, store: STORE, k: key, b: bytes },
  );
}

/** Read a key from the worker's DB; resolves whether a value is present. */
function hasKey(page: Page, key: string): Promise<boolean> {
  return page.evaluate(
    ({ dbName, store, k }) =>
      new Promise<boolean>((resolve, reject) => {
        const req = indexedDB.open(dbName, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(store);
        req.onsuccess = () => {
          const db = req.result;
          const g = db.transaction(store, "readonly").objectStore(store).get(k);
          g.onsuccess = () => {
            db.close();
            resolve(!!g.result);
          };
          g.onerror = () => reject(g.error);
        };
        req.onerror = () => reject(req.error);
      }),
    { dbName: DB_NAME, store: STORE, k: key },
  );
}

test("boots the wasm session, edits, and persists across reload", async ({ page }) => {
  // BOOT — fresh context ⇒ empty IndexedDB ⇒ the bundled fixture plan (no
  // factories). This proves the wasm module instantiates and hydrate() runs.
  await page.goto("/");
  await waitReady(page);
  expect(await factoryCount(page), "fixture plan boots empty").toBe(0);

  // EDIT — create a factory through the real store path (dispatch → WasmBackend
  // → worker → wasm Session.edit → snapshot to IndexedDB). `select` opens the
  // summary drawer, the visible signal the map projection reflects the edit.
  const created = await page.evaluate(async () => {
    return (window as unknown as StoreWin).__ficsitStore.getState().dispatch(
      [
        {
          type: "create_factory",
          name: "SMOKE FACTORY",
          position: { x: 100, y: 200, z: 0 },
          region: "GRASS FIELDS",
        },
      ],
      { select: true },
    );
  });
  expect(created, "the edit minted one factory id").toHaveLength(1);

  // RENDER — the store projection (the source the map canvas draws) now holds
  // the factory, and the selection drawer shows its name.
  expect(await factoryCount(page)).toBe(1);
  const drawer = page.getByTestId("summary-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer.locator(".drawer-name")).toHaveText("SMOKE FACTORY");

  // RELOAD + PERSIST — a new page load reconstructs the session from the
  // IndexedDB snapshot blob. The factory must survive (this is the round-trip
  // that would fail if persistence were a no-op or lost on reload).
  await page.reload();
  await waitReady(page);
  expect(await factoryCount(page), "the factory persisted across reload").toBe(1);
  const names = await page.evaluate(() =>
    Object.values(
      (window as unknown as StoreWin).__ficsitStore.getState().plan.factories,
    ).map((f) => f.name),
  );
  expect(names).toContain("SMOKE FACTORY");
});

// M1 — the Rust-driven mutation signal. chat_send drafts a proposal via
// `s.edit(CreateProposal)` — a real store write. The old hand-kept MUTATING
// allowlist omitted chat_send, so the draft vanished on reload; the envelope
// `{ mutated: true }` now makes the worker snapshot it. This proves a
// chat-drafted proposal SURVIVES a reload.
test("M1: a chat-drafted proposal survives a reload", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);
  expect(await proposalCount(page), "fixture plan boots with no proposals").toBe(0);

  // Drive chatSend through the real transport (store → WasmBackend → worker →
  // wasm chat::chat → s.edit(CreateProposal) → snapshot to IndexedDB). A
  // standard-recipe target is always feasible on the fixture, so it drafts.
  const drafted = await page.evaluate(async () => {
    const reply = await (window as unknown as StoreWin).__ficsitBackend.chatSend(
      { scope: "empire" },
      "produce Iron Rod at 30/min",
    );
    return reply.proposal;
  });
  expect(drafted, "chat drafted a proposal (returns its id)").toBeTruthy();

  // RELOAD — a new page load reconstructs the session from the IndexedDB
  // snapshot blob the chat_send mutation wrote.
  await page.reload();
  await waitReady(page);

  // RELOAD + PERSIST — the worker reconstructs from the IndexedDB snapshot. If
  // chat_send had NOT been flagged mutating (the M1 bug), the proposal would be
  // gone here. It must survive.
  expect(await proposalCount(page), "the chat-drafted proposal persisted across reload").toBe(1);
});

// M2 — a corrupt / version-mismatched IndexedDB blob must NEVER brick the app.
// Seed garbage under the plan key, reload, and assert the app still BOOTS fresh
// (ensureReady catches the WebSession construction throw, backs the bad blob up
// under a -corrupt key, and constructs a fresh session) instead of caching the
// rejection into a permanent BACKEND UNREACHABLE.
test("M2: a corrupt saved blob boots fresh, not bricked, and is backed up", async ({ page }) => {
  // Boot once so the worker creates the IndexedDB + object store.
  await page.goto("/");
  await waitReady(page);

  // Seed unparseable bytes under the current-plan key.
  await page.evaluate(
    ({ dbName, store, key }) =>
      new Promise<void>((resolve, reject) => {
        const req = indexedDB.open(dbName, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(store);
        req.onsuccess = () => {
          const db = req.result;
          const tx = db.transaction(store, "readwrite");
          tx.objectStore(store).put(new Uint8Array([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]), key);
          tx.oncomplete = () => {
            db.close();
            resolve();
          };
          tx.onerror = () => reject(tx.error);
        };
        req.onerror = () => reject(req.error);
      }),
    { dbName: DB_NAME, store: STORE, key: KEY },
  );

  // Reload — the worker meets the corrupt blob on boot. The app must still come
  // up (waitReady asserts error === null) with a FRESH, empty plan.
  await page.reload();
  await waitReady(page);
  expect(await factoryCount(page), "corrupt blob → app boots a fresh empty plan").toBe(0);

  // And the bad blob was preserved under the -corrupt backup key, not dropped.
  const backedUp = await page.evaluate(
    ({ dbName, store, corruptKey }) =>
      new Promise<boolean>((resolve, reject) => {
        const req = indexedDB.open(dbName, 1);
        req.onupgradeneeded = () => req.result.createObjectStore(store);
        req.onsuccess = () => {
          const db = req.result;
          const g = db.transaction(store, "readonly").objectStore(store).get(corruptKey);
          g.onsuccess = () => {
            db.close();
            resolve(!!g.result);
          };
          g.onerror = () => reject(g.error);
        };
        req.onerror = () => reject(req.error);
      }),
    { dbName: DB_NAME, store: STORE, corruptKey: CORRUPT_KEY },
  );
  expect(backedUp, "the unreadable blob was backed up under the -corrupt key").toBe(true);
});

// Phase 4a — uploading a Docs.json runs the browser session on a real catalog
// instead of the bundled fixture, AND that choice survives a reload. Drives the
// REAL UI: the hidden file input the "UPLOAD DOCS.JSON" button proxies. The
// worker rebuilds the WebSession over the uploaded bytes (preserving the plan)
// and persists them under the docs key; on reload the worker reads them back.
test("Phase 4a: uploading a Docs.json swaps the catalog and persists across reload", async ({
  page,
}) => {
  await page.goto("/");
  await waitReady(page);
  expect(await buildVersion(page), "boots on the bundled fixture catalog").toBe("fixture");

  // Sync Phase 2: "Sync from save" is gated on a real catalog — syncing against
  // the fixture would quarantine most recipes into junk diffs. On the fixture it
  // is aria-disabled and its title says how to enable it. (The menu stays open
  // across the upload below — setInputFiles is programmatic — so we watch the
  // same button's gate flip in place, no click through the onboarding overlay.)
  await page.getByTestId("btn-data-menu").click();
  const syncBtn = page.getByTestId("btn-sync-save");
  await expect(syncBtn).toHaveAttribute("aria-disabled", "true");
  await expect(syncBtn).toHaveAttribute("title", /Docs\.json/);
  // Phase 3: the auto-sync toggle shares the same Docs.json gate.
  const autoBtn = page.getByTestId("btn-auto-sync");
  await expect(autoBtn).toHaveAttribute("aria-disabled", "true");
  await expect(autoBtn).toHaveAttribute("title", /Docs\.json/);

  // Upload through the real input the button drives (setInputFiles fires its
  // onChange → store.uploadDocs → worker rebuild → hydrate). The button is
  // web-only; the input is present in the built web app (__WASM_BACKEND__).
  await page.getByTestId("docs-file-input").setInputFiles(DOCS_FIXTURE);

  // The catalog is now a real (uploaded) one: the wasm tags it "uploaded", and
  // the recipe set is non-empty. This is the signal the first-run prompt reads.
  await expect
    .poll(() => buildVersion(page), { timeout: 30_000 })
    .toBe("uploaded");
  const recipeCount = await page.evaluate(
    () =>
      Object.keys(
        (window as unknown as StoreWin).__ficsitStore.getState().gamedata.recipes,
      ).length,
  );
  expect(recipeCount, "the uploaded catalog has recipes").toBeGreaterThan(0);

  // The gate lifts now that a real catalog is loaded: Sync from save is live.
  await expect(syncBtn).toHaveAttribute("aria-disabled", "false");

  // Auto-sync owns the manual sync: turning it ON disables the manual button
  // (the timer handles re-reads) until it's turned back off. Drive the toggle
  // through the store (the real toggle needs an OS file picker we can't script).
  const setAuto = (on: boolean) =>
    page.evaluate(
      (v) =>
        (
          window as unknown as { __ficsitStore: { getState(): { setAutoSync(e: boolean): void } } }
        ).__ficsitStore.getState().setAutoSync(v),
      on,
    );
  await setAuto(true);
  await expect(syncBtn).toHaveAttribute("aria-disabled", "true");
  await expect(syncBtn).toContainText("Auto-syncing");
  await setAuto(false);
  await expect(syncBtn).toHaveAttribute("aria-disabled", "false");

  // RELOAD + PERSIST — the worker reads the docs bytes back out of IndexedDB and
  // reconstructs the session on the real catalog. If docs were not persisted,
  // this would fall back to "fixture" (the M-class bug this test guards).
  await page.reload();
  await waitReady(page);
  expect(await buildVersion(page), "the uploaded catalog persisted across reload").toBe("uploaded");
});

// Phase 4a durability — the docs analogue of M2. A stored Docs.json that a later
// wasm's parse_docs no longer accepts (a version bump) must NOT brick the boot:
// ensureReady degrades to the bundled fixture, parks the bad docs under
// docs-corrupt, and clears the docs key so the next boot doesn't re-throw. This
// guards the exact "permanent brick on a version bump" hazard the plan-blob M2
// fix closed, now for the catalog.
test("Phase 4a: corrupt stored docs boot fresh on the fixture, not bricked", async ({ page }) => {
  // Boot once so the worker creates the DB + object store.
  await page.goto("/");
  await waitReady(page);

  // Plant unparseable bytes under the docs key (as a stale/incompatible catalog).
  await seedKey(page, DOCS_KEY, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);

  // Reload — the worker meets the bad docs on boot. The app must still come up
  // (waitReady asserts error === null) on the BUNDLED FIXTURE, not bricked.
  await page.reload();
  await waitReady(page);
  expect(await buildVersion(page), "bad docs → boots on the bundled fixture").toBe("fixture");

  // The bad docs were parked under docs-corrupt AND the docs key was cleared, so
  // the next boot won't re-throw on them.
  expect(await hasKey(page, DOCS_CORRUPT_KEY), "the bad docs were backed up").toBe(true);
  expect(await hasKey(page, DOCS_KEY), "the bad docs key was cleared off the boot path").toBe(false);
});

// Phase 4a durability — the OTHER cascade branch (PR #17 review): a corrupt PLAN
// blob when a real catalog IS uploaded must drop ONLY the plan and KEEP the
// catalog. A regression that discarded the docs here would silently throw away
// the player's uploaded game data on any plan corruption — the branch worth a
// guard.
test("Phase 4a: a corrupt plan with uploaded docs keeps the catalog, drops only the plan", async ({
  page,
}) => {
  await page.goto("/");
  await waitReady(page);

  // Upload a real catalog, then make a plan edit so there IS a plan to corrupt.
  await page.getByTestId("docs-file-input").setInputFiles(DOCS_FIXTURE);
  await expect.poll(() => buildVersion(page), { timeout: 30_000 }).toBe("uploaded");
  await page.evaluate(async () => {
    await (window as unknown as StoreWin).__ficsitStore.getState().dispatch([
      { type: "create_factory", name: "DOOMED", position: { x: 1, y: 2, z: 0 }, region: "GRASS FIELDS" },
    ]);
  });
  expect(await factoryCount(page)).toBe(1);

  // Corrupt ONLY the plan blob; leave the docs key intact.
  await seedKey(page, KEY, [9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);

  // Reload — the cascade should keep the uploaded catalog and boot a fresh plan.
  await page.reload();
  await waitReady(page);
  expect(await buildVersion(page), "the uploaded catalog is KEPT when only the plan is corrupt").toBe(
    "uploaded",
  );
  expect(await factoryCount(page), "the corrupt plan was dropped to a fresh one").toBe(0);
  expect(await hasKey(page, CORRUPT_KEY), "the corrupt plan blob was backed up").toBe(true);
  expect(await hasKey(page, DOCS_KEY), "the uploaded docs were NOT discarded").toBe(true);
});
