// Audit #131 acceptance (promoted from the audit probe suite): web platform (renderer/src/state/{store,wasmBackend,wasmWorker}.ts
// + the built dist-web app). These probes drive the BUILT WEB APP — the wasm
// Session in a Web Worker over IndexedDB, NO backend server — exactly like
// e2e-web/web-smoke.spec.ts, and they FILL the gaps web-smoke leaves: the
// forced-failure Docs.json upload paths (truncated / wrong-shape), a GOOD
// upload preserving a GOOD plan, undo-snapshot persistence, and view-state
// (openFactory) hydrate fidelity.
//
// HARNESS (read before running): these probes REQUIRE the web-build harness
// (`pnpm build:web` → `dist-web`, served by `vite preview`, __WASM_BACKEND__
// on) — the SAME plumbing playwright.web.config.ts drives. They CANNOT run
// under playwright.audit.config.ts / playwright.config.ts, whose backend is the
// dev bridge (port 8791) + `pnpm dev` (no wasm backend, no __ficsitStore, no
// docs-file-input). The orchestrator must point a web-build config's testDir at
// this file (or copy it beside web-smoke). Consequently these probes do NOT use
// the bridge helper `resetView` (there is no bridge): Playwright hands each test
// a fresh browser context, so IndexedDB starts empty and a reload stays in the
// same context — the same isolation web-smoke relies on. Every seed is done
// through the in-page store (page.evaluate → __ficsitStore.dispatch), which
// snapshots to IndexedDB inline, so no "API edits don't stream" reload dance is
// needed.
//
// Each test declares its EXPECTED (correct) result in the header BEFORE any
// assertion. A failing probe is data for the mismatch protocol — the assertions
// are NOT weakened to pass. Probe 2 is a DECLARED-EXPECTED probe: it pins the
// CORRECT behavior the current implementation is believed to violate.

import { test, expect, type Page } from "@playwright/test";
import { Buffer } from "node:buffer";
import { fileURLToPath } from "node:url";

// The bundled fixture catalog as an on-disk file — PROBE 3's "good upload".
const DOCS_FIXTURE = fileURLToPath(
  new URL("../../crates/gamedata/assets/docs-fixture.json", import.meta.url),
);

// NOTE: no serial mode — the runner uses --workers=1, and per-test isolation
// comes from Playwright's fresh BrowserContext per test (each context gets its
// own empty IndexedDB partition; no cleanup dance needed), so a failure must
// NOT cascade-skip sibling probes: every probe needs a verdict.

/** The in-page store handle store.ts exposes in the web build (__WASM_BACKEND__
 *  guard). Only the members these probes touch are typed. */
interface StoreWin {
  __ficsitStore: {
    getState(): {
      ready: boolean;
      error: string | null;
      cmdError: { message: string; at: number } | null;
      toasts: { id: number; message: string; kind: string }[];
      view: { mode: "map" } | { mode: "factory"; factoryId: string };
      plan: { factories: Record<string, { name: string }> };
      gamedata: { buildVersion: string; recipes: Record<string, unknown> };
      dispatch(cmds: unknown[], opts?: { select?: boolean }): Promise<string[] | null>;
      undo(): Promise<void>;
      setView(view: { mode: "map" } | { mode: "factory"; factoryId: string }): void;
    };
  };
}

/** Wait for the wasm session to boot and hydrate (or surface a fatal error).
 *  Graph-aware (#133 rootcause `web-viewstate`): a boot with a persisted
 *  openFactory correctly renders GraphView (`graph-root`), not MapView — the
 *  old map-only gate timed out on exactly the behavior it should assert. */
async function waitReady(page: Page, root: "map-root" | "graph-root" = "map-root"): Promise<void> {
  await expect(page.getByTestId(root)).toBeVisible({ timeout: 30_000 });
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

const factoryCount = (page: Page): Promise<number> =>
  page.evaluate(
    () => Object.keys((window as unknown as StoreWin).__ficsitStore.getState().plan.factories).length,
  );

const factoryNames = (page: Page): Promise<string[]> =>
  page.evaluate(() =>
    Object.values((window as unknown as StoreWin).__ficsitStore.getState().plan.factories).map((f) => f.name),
  );

const buildVersion = (page: Page): Promise<string> =>
  page.evaluate(
    () => (window as unknown as StoreWin).__ficsitStore.getState().gamedata.buildVersion,
  );

// catalogLoaded is derived in DataMenu.tsx as `bv !== "" && bv !== "fixture"`
// (the gate that unlocks "② Import save" and flips step ① to its loaded ✓
// state — the ordered menu itself never reshuffles). Replicated here from
// buildVersion — the single source it reads.
const catalogLoaded = async (page: Page): Promise<boolean> => {
  const bv = await buildVersion(page);
  return bv !== "" && bv !== "fixture";
};

const cmdError = (page: Page): Promise<{ message: string; at: number } | null> =>
  page.evaluate(() => (window as unknown as StoreWin).__ficsitStore.getState().cmdError);

const dispatchFactory = (page: Page, name: string, pos: { x: number; y: number; z: number }): Promise<string[] | null> =>
  page.evaluate(
    ({ n, p }) =>
      (window as unknown as StoreWin).__ficsitStore.getState().dispatch([
        { type: "create_factory", name: n, position: p, region: "GRASS FIELDS" },
      ]),
    { n: name, p: pos },
  );

// ---------------------------------------------------------------------------
// PROBE 1 — Garbage/truncated Docs.json upload → friendly error, no wedged
// state.
//
// EXPECTED (correct behavior): uploading unparseable bytes (`{"NativeClass":`)
// is REJECTED. buildVersion STAYS "fixture" (never flips to "uploaded");
// cmdError is non-null and a red error toast is shown whose text starts
// "Couldn't load Docs.json —"; factoryCount stays 1 with factory "KEEP" still
// present; a subsequent dispatch(create_factory "AFTER") SUCCEEDS (the app is
// not wedged) — so after it there are 2 factories; and after page.reload() +
// waitReady the plan persisted intact: BOTH "KEEP" and "AFTER" present
// (factoryCount === 2) with buildVersion still "fixture".
//
// (Descriptor note: the source descriptor wrote "factoryCount===1" for the
// post-reload check, but it also creates "AFTER" before that reload; a
// non-wedged app persists AFTER, so the genuinely-correct post-reload count is
// 2 with both factories present. Asserting 2 is the true EXPECTED, not a
// weakening — a count of 1 here would itself be a lost-write bug.)
// ---------------------------------------------------------------------------
test("truncated Docs.json upload is rejected with a friendly error, app not wedged", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);

  // Web-only probe: the docs upload input is __WASM_BACKEND__-gated and does
  // not exist on the bridge build this suite runs against. Skip (not fail) —
  // run it under playwright.web.config.ts to get a real verdict.
  test.skip((await page.getByTestId("docs-file-input").count()) === 0, "web-only: bridge build has no docs upload input");

  // Seed a plan we expect to survive the failed upload.
  const kept = await dispatchFactory(page, "KEEP", { x: 1, y: 2, z: 0 });
  expect(kept, "the seed edit minted one factory id").toHaveLength(1);
  expect(await factoryCount(page)).toBe(1);
  expect(await buildVersion(page), "boots on the bundled fixture catalog").toBe("fixture");

  // Upload truncated/invalid JSON through the real hidden input the UPLOAD
  // button proxies. In-memory bytes (equivalent to a temp file) so no on-disk
  // scratch is needed: `{"NativeClass":` is not valid JSON.
  await page.getByTestId("docs-file-input").setInputFiles({
    name: "docs.json",
    mimeType: "application/json",
    buffer: Buffer.from('{"NativeClass":'),
  });

  // The failure surfaces on cmdError (persists — not the transient toast) AND
  // as a red error toast. Poll cmdError; the upload fails fast.
  await expect.poll(() => cmdError(page), { timeout: 30_000 }).not.toBeNull();
  const err = await cmdError(page);
  expect(err!.message.startsWith("Couldn't load Docs.json —"), "cmdError is Docs-specific").toBe(true);
  const errToast = page.locator(".toast.toast-error");
  await expect(errToast).toBeVisible();
  await expect(errToast).toContainText("Couldn't load Docs.json —");

  // The catalog choice never flipped and the seeded plan is untouched.
  expect(await buildVersion(page), "buildVersion stays fixture — the bad upload is rejected").toBe("fixture");
  expect(await catalogLoaded(page), "the first-run upload gate is NOT satisfied by a rejected upload").toBe(false);
  expect(await factoryCount(page)).toBe(1);
  expect(await factoryNames(page)).toContain("KEEP");

  // The app is not wedged — a further real edit still lands.
  const after = await dispatchFactory(page, "AFTER", { x: 5, y: 6, z: 0 });
  expect(after, "a post-failure edit still mints a factory (not wedged)").toHaveLength(1);
  expect(await factoryCount(page)).toBe(2);

  // The whole plan persists across a reload; the catalog is still the fixture.
  await page.reload();
  await waitReady(page);
  expect(await factoryCount(page), "both factories persisted across reload").toBe(2);
  expect(await factoryNames(page)).toEqual(expect.arrayContaining(["KEEP", "AFTER"]));
  expect(await buildVersion(page), "still on the bundled fixture after reload").toBe("fixture");
});

// ---------------------------------------------------------------------------
// PROBE 2 — Wrong-but-valid JSON array (`[]`) Docs.json upload is rejected.
//
// EXPECTED: an EMPTY, structurally-wrong catalog (`[]`, zero recipes) is
// REJECTED — buildVersion remains "fixture", an error toast appears, and
// catalogLoaded stays false (the "① Upload Docs.json" onboarding / DATA step
// is still shown). Fixed by audit #131: WebSession::new validates a non-empty
// recipes/items catalog before replacing anything; the old session is kept.
// ---------------------------------------------------------------------------
test("empty-array Docs.json upload is rejected with the catalog kept", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);
  expect(await buildVersion(page), "boots on the bundled fixture catalog").toBe("fixture");

  await page.getByTestId("docs-file-input").setInputFiles({
    name: "docs.json",
    mimeType: "application/json",
    buffer: Buffer.from("[]"),
  });

  // Wait for the upload to SETTLE either way (up to 30s): a toast is pushed on
  // BOTH the reject and the (current, buggy) success path, so toasts.length > 0
  // is a settle signal that never hangs regardless of which branch runs.
  await expect
    .poll(
      () => page.evaluate(() => (window as unknown as StoreWin).__ficsitStore.getState().toasts.length),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0);

  // EXPECTED-CORRECT: rejected. (Current build flips to "uploaded" — this fails.)
  expect(await buildVersion(page), "an empty catalog must be rejected — buildVersion stays fixture").toBe("fixture");
  expect(await catalogLoaded(page), "an empty catalog must NOT satisfy the first-run gate").toBe(false);
  const err = await cmdError(page);
  expect(err, "the rejected empty upload records a cmdError").not.toBeNull();
  const errToast = page.locator(".toast.toast-error");
  await expect(errToast, "a red error toast is shown for the rejected empty catalog").toBeVisible();
});

// ---------------------------------------------------------------------------
// PROBE 3 (promoted, #133) — A GOOD Docs.json upload preserves the existing
// plan through the catalog swap and a reload.
//
// EXPECTED: seed factory "SURVIVOR" on the fixture catalog; upload the bundled
// fixture catalog itself as a file → buildVersion flips to "uploaded" and the
// plan survives the swap in-memory (the worker rebuilt the WebSession over the
// new catalog from the preserved plan snapshot). After page.reload() +
// waitReady: buildVersion === "uploaded" AND "SURVIVOR" is still present
// (saveDocsAndPlan wrote docs + plan atomically). web-smoke only corrupts the
// plan away on upload; no other test asserts a GOOD plan survives a GOOD one.
// ---------------------------------------------------------------------------
test("a good Docs.json upload preserves the existing plan through the swap and reload", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);

  const seeded = await dispatchFactory(page, "SURVIVOR", { x: 3, y: 4, z: 0 });
  expect(seeded, "the seed edit minted one factory id").toHaveLength(1);
  expect(await factoryCount(page)).toBe(1);
  expect(await buildVersion(page), "boots on the bundled fixture catalog").toBe("fixture");

  // GOOD upload: the bundled fixture catalog itself. The wasm tags it
  // "uploaded" and the worker rebuilds the session over it from the plan
  // snapshot it just preserved.
  await page.getByTestId("docs-file-input").setInputFiles(DOCS_FIXTURE);
  await expect.poll(() => buildVersion(page), { timeout: 30_000 }).toBe("uploaded");

  // The plan survived the catalog swap in-memory.
  expect(await factoryCount(page), "the plan survives the catalog swap").toBe(1);
  expect(await factoryNames(page)).toContain("SURVIVOR");

  // ...and survives a reload, still on the uploaded catalog (docs + plan were
  // written atomically).
  await page.reload();
  await waitReady(page);
  expect(await buildVersion(page), "the uploaded catalog persisted across reload").toBe("uploaded");
  expect(await factoryCount(page), "the plan persisted across reload").toBe(1);
  expect(await factoryNames(page)).toContain("SURVIVOR");
});

// ---------------------------------------------------------------------------
// PROBE 4 (promoted, #133) — A wasm undo snapshot persists the UNDONE state
// across a reload.
//
// EXPECTED: create ALPHA then BETA (2 factories); undo() removes BETA's
// creation leaving 1 factory (ALPHA); after page.reload() + waitReady
// factoryCount === 1 and the only factory is ALPHA — BETA does NOT resurrect.
// Pins that the dispatch "undo" arm's mutated=true envelope actually wrote the
// POST-undo store to IndexedDB (the path web-smoke does not exercise).
// ---------------------------------------------------------------------------
test("a wasm undo snapshot persists the undone state across reload", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);

  const a = await dispatchFactory(page, "ALPHA", { x: 1, y: 1, z: 0 });
  expect(a, "ALPHA minted").toHaveLength(1);
  const b = await dispatchFactory(page, "BETA", { x: 2, y: 2, z: 0 });
  expect(b, "BETA minted").toHaveLength(1);
  expect(await factoryCount(page)).toBe(2);

  // Undo BETA's creation through the real store path.
  await page.evaluate(() => (window as unknown as StoreWin).__ficsitStore.getState().undo());
  expect(await factoryCount(page), "undo removed BETA").toBe(1);
  expect(await factoryNames(page), "ALPHA is the sole survivor after undo").toEqual(["ALPHA"]);

  // The undone state must persist — BETA must not come back on reload.
  await page.reload();
  await waitReady(page);
  expect(await factoryCount(page), "the undone state persisted across reload").toBe(1);
  expect(await factoryNames(page), "BETA did not resurrect").toEqual(["ALPHA"]);
});

// ---------------------------------------------------------------------------
// PROBE 5 (promoted, #133) — View-state (openFactory) round-trips through
// IndexedDB on reload.
//
// EXPECTED: after opening factory F via setView({mode:"factory", factoryId:F})
// (which persists openFactory via the debounced set_view_state) and then a
// REAL mutation (create "FLUSH") that snapshots inline — flushing the pending
// debounced view write — a page.reload() boots INTO THE FACTORY GRAPH
// (graph-root, the graph-aware waitReady) with view deep-equal to
// {mode:"factory", factoryId:F}, NOT {mode:"map"}. Pins view-state hydrate
// fidelity and that a real mutation flushes the coalesced view-state snapshot.
// ---------------------------------------------------------------------------
test("view-state (openFactory) round-trips through IndexedDB on reload", async ({ page }) => {
  await page.goto("/");
  await waitReady(page);

  const home = await page.evaluate(() =>
    (window as unknown as StoreWin).__ficsitStore.getState().dispatch(
      [{ type: "create_factory", name: "HOME", position: { x: 7, y: 8, z: 0 }, region: "GRASS FIELDS" }],
      { select: true },
    ),
  );
  expect(home, "HOME minted").toHaveLength(1);
  const F = home![0];

  // Open the factory — persists openFactory via the debounced set_view_state.
  await page.evaluate(
    (id) => (window as unknown as StoreWin).__ficsitStore.getState().setView({ mode: "factory", factoryId: id }),
    F,
  );
  expect(await page.evaluate(() => (window as unknown as StoreWin).__ficsitStore.getState().view)).toEqual({
    mode: "factory",
    factoryId: F,
  });

  // A real mutation snapshots inline and flushes the pending view-state write
  // with it (wasmWorker: every non-set_view_state mutation calls snapshotNow(),
  // which cancels the debounce and writes the current blob).
  const flush = await dispatchFactory(page, "FLUSH", { x: 9, y: 10, z: 0 });
  expect(flush, "FLUSH minted (forces the inline snapshot)").toHaveLength(1);

  // After reload the app reopens factory F — GraphView renders, not the map.
  await page.reload();
  await waitReady(page, "graph-root");
  const view = await page.evaluate(() => (window as unknown as StoreWin).__ficsitStore.getState().view);
  expect(view, "the app reopened the factory it was in").toEqual({ mode: "factory", factoryId: F });
});

