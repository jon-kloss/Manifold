// Audit #122 acceptance (promoted from the audit probe suite): the two
// planner-core lifecycle guards — release_node on a ◆ Built claim is
// REJECTED (§3.1.1), and a no-op command must not truncate the redo tail.

// AUDIT area: planner-core — command validation, undo/redo symmetry,
// Built-immutability guards, and cascade-delete atomicity, all driven through
// the same /api/edit command surface the renderer uses (dev bridge, port 8791).
//
// Every probe declares its EXPECTED (correct) result in a header BEFORE any
// assertion. Where the current code is suspected wrong the probe still asserts
// the CORRECT behavior: a failing probe is data for the mismatch protocol, NOT
// a reason to weaken the assert. API edits do not stream to an open client, so
// seeding happens via the API and (for the DOM-driven import probe) the page is
// reloaded + resetView'd to re-sync.
//
// Fixture catalog (dev bridge default) recipe used below:
//   Recipe_IronRod_C = 1 ingot -> 1 rod @ 4s => 15/min per Constructor machine.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "../e2e/helpers";
import { fileURLToPath } from "node:url";

// NOTE: no serial mode — the runner uses --workers=1, and per-test isolation
// (each test seeds + deletes its own factories) means a failure must NOT
// cascade-skip sibling probes: every probe needs a verdict.

const API = "http://localhost:8791/api";
const SAVES = fileURLToPath(new URL("../../fixtures/saves", import.meta.url));

// One command per call, returning the created ids (creation order) so a probe
// can capture the entity it just minted.
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
// Like edit(), but for commands EXPECTED to be rejected — returns the raw
// status + parsed body instead of throwing, so a probe can assert 422 + code.
async function editExpectError(request: APIRequestContext, cmds: unknown[]): Promise<{ status: number; body: any }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  let body: any = null;
  try {
    body = await res.json();
  } catch {
    body = await res.text();
  }
  return { status: res.status(), body };
}
async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function undo(request: APIRequestContext): Promise<void> {
  const res = await request.post(`${API}/undo`);
  if (!res.ok()) throw new Error(`undo ${res.status()}: ${await res.text()}`);
}
async function redo(request: APIRequestContext): Promise<void> {
  const res = await request.post(`${API}/redo`);
  if (!res.ok()) throw new Error(`redo ${res.status()}: ${await res.text()}`);
}
const size = (o: unknown) => Object.keys((o ?? {}) as object).length;
const factoryCount = (h: any) => size(h.plan.factories);


test("release_node on an imported Built claim is rejected (built-immutable)", async ({ page, request }) => {
  test.setTimeout(300_000); // cold-worker .sav parse
  await resetView(request);

  const baseline = factoryCount(await hydrate(request));
  try {
    await page.goto("/");
    await expect(page.getByTestId("map-root")).toBeVisible();

    // ---- import Dunarr-076 as the ◆ Built layer (phase4-import flow) ----
    await page.getByTestId("btn-data-menu").click();
    const [chooser] = await Promise.all([
      page.waitForEvent("filechooser"),
      page.getByTestId("btn-import").click(),
    ]);
    await chooser.setFiles(`${SAVES}/Dunarr-076.sav`);
    await expect(page.getByTestId("import-preview")).toBeVisible({ timeout: 120_000 });
    await page.getByTestId("btn-import-run").click();
    await expect(page.getByTestId("import-done")).toBeVisible({ timeout: 60_000 });
    await page.locator(".wizard-foot .btn-primary").click();

    // ---- pick a Built claim straight from canonical state ----
    const h = await hydrate(request);
    const claims = Object.values<any>(h.plan.nodeClaims);
    const builtClaim = claims.find((c) => c.status === "built");
    expect(builtClaim, "import must produce at least one ◆ Built node claim").toBeTruthy();
    const C = builtClaim.id as string;
    const owner = builtClaim.factory as string;
    expect(h.plan.factories[owner].nodeClaims).toContain(C);

    // ---- attempt to release it: must be refused ----
    const { status, body } = await editExpectError(request, [{ type: "release_node", id: C }]);
    expect(status).toBe(422);
    // the bridge surfaces DomainError via its Display string; built_immutable
    // reads "built entities are immutable: <id> (<action>)".
    expect(String(body?.error ?? body)).toMatch(/immutable/i);

    // ---- the claim survives untouched ----
    const h2 = await hydrate(request);
    expect(h2.plan.nodeClaims[C]).toBeTruthy();
    expect(h2.plan.nodeClaims[C].status).toBe("built");
    expect(h2.plan.factories[owner].nodeClaims).toContain(C);
  } finally {
    // Undo back down to the pre-import factory count (removes the release step
    // if the guard was missing and it committed, then the whole import).
    for (let i = 0; i < 8; i++) {
      const h = await hydrate(request).catch(() => null);
      if (!h || !h.canUndo || factoryCount(h) <= baseline) break;
      await undo(request).catch(() => {});
    }
  }
});

// ---------------------------------------------------------------------------
// PROBE 3 — A no-op command must not destroy the redo tail.
//
// Creates factory F (name NOOP), renames it to NOOP2, undoes the rename (so a
// redo tail exists: canRedo==true, name back to NOOP), then issues a
// tidy_layout on F while F has NO groups/ports/junctions — an empty forward
// batch, i.e. a no-op.
//
// EXPECTED: After the tidy_layout no-op, hydrate.canRedo REMAINS true; the
// subsequent /api/redo restores factories[F].name=="NOOP2". A no-op must
// neither truncate the redo tail nor commit an undoable step.
//
// (Fixed by audit #122: Session::edit skips the journal for empty
// transactions. This test is the permanent regression guard — if it fails,
// the no-op/redo bug is BACK; it is not a known mismatch.)
// ---------------------------------------------------------------------------

test("a no-op command must not destroy the redo tail", async ({ request }) => {
  await resetView(request);
  let f = "";
  try {
    f = (await edit(request, [{ type: "create_factory", name: "NOOP", position: { x: -3400, y: 3400, z: 0 }, region: "GRASS FIELDS" }])).created[0];
    await edit(request, [{ type: "rename_factory", id: f, name: "NOOP2" }]);

    await undo(request); // undo the rename
    let h = await hydrate(request);
    expect(h.canRedo).toBe(true);
    expect(h.plan.factories[f].name).toBe("NOOP");

    // no-op: F has no groups/ports/junctions, so tidy produces an empty batch.
    await edit(request, [{ type: "tidy_layout", factory: f }]);

    h = await hydrate(request);
    // the redo tail must survive the no-op
    expect(h.canRedo).toBe(true);

    await redo(request);
    h = await hydrate(request);
    expect(h.plan.factories[f].name).toBe("NOOP2");
  } finally {
    if (f) await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 4 — add_edge enforces the splitter output-port cap (1-in / 3-out).
//
// Builds factory F with one splitter junction J and four groups g1..g4, then
// connects J's OUTPUT to g1, g2, g3 (all carrying Desc_IronIngot_C), then
// attempts a 4th output edge J->g4.
//
// EXPECTED: the first three add_edge calls return 200; the fourth returns 422
// with DomainError code "invalid" and a message stating all 3 output ports are
// connected ("Splitter has all 3 output ports connected"). hydrate then shows
// EXACTLY 3 edges whose from == {kind:"junction", id:J}.
// ---------------------------------------------------------------------------
