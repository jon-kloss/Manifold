// Audit #128 acceptance (promoted from the audit probe suite): MAKE FROM
// RESOURCES planner math vs the fixture catalog.
// Focus: reuse/redirect capacity accounting, diamond intermediate summing, and
// reuse scale-up. Every probe declares its EXPECTED (correct) result in the
// header BEFORE any assertion; a failing probe is data for the mismatch
// protocol, NOT a reason to weaken the assert. Seeded through the same command
// surface the UI uses, against the dev bridge's default fixture catalog:
//   Recipe_IngotIron_C  = 1 ore   -> 1 ingot  @ 2s  => 30/min per machine
//   Recipe_IronRod_C    = 1 ingot -> 1 rod    @ 4s  => 15/min per machine
//   Recipe_Screw_C      = 1 rod   -> 4 screw  @ 6s  => 40/min per machine
//   Recipe_IronPlate_C  = 3 ingot -> 2 plate  @ 6s  => 20/min per machine
//   Recipe_IronPlateReinforced_C = 6 plate + 12 screw -> 1 RIP @ 12s => 5/min

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

// NOTE: no serial mode — the runner uses --workers=1, and per-test isolation
// (each test seeds + deletes its own factories) means a failure must NOT
// cascade-skip sibling probes: every probe needs a verdict.

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}: ${await res.text()}`);
  return res.json();
}
const groupsOf = (h: any, factory: string, recipe: string) =>
  Object.values<any>(h.plan.groups).filter((g) => g.factory === factory && g.recipe === recipe);
const countRecipe = (h: any, factory: string, recipe: string) => groupsOf(h, factory, recipe).length;

// API seeds do not stream to an already-open client, so open the graph AFTER
// the plan is fully seeded (goto reloads and re-syncs the store).
async function openGraph(page: any, name: string): Promise<void> {
  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
}

const P = (id: string) => ({ kind: "port", id });
const G = (id: string) => ({ kind: "group", id });

// ---------------------------------------------------------------------------
// PROBE 1 — Reuse+redirect build is NOT blocked when the node is fully claimed
// but the existing rod line is entirely exported.
//
// Setup: iron-ingot IN capped at 30/min; a 2-machine rod line (30/min rod
// capacity, draws all 30 ingot) whose whole output is exported (rodOut = 30).
// Ask for 120 screws (= 30/min rod = the rod line's full output; 0 NEW ingot).
//
// EXPECTED (correct behavior): the modal shows NO mfr-warn — the build is
// feasible via redirect because it adds ZERO new ingot draw (the rod already
// exists, its export is trimmed to feed the screws) — and mfr-build is ENABLED.
// After clicking build the modal hides; hydrate shows exactly ONE
// Recipe_IronRod_C group (reused, not duplicated) and exactly ONE Recipe_Screw_C
// group; plan.ports[rodOut].rate is trimmed BELOW 30 (toward 0) to feed the new
// line; port-out Desc_IronScrew_C shows 120/min; the iron-ingot input port's
// derived draw stays <= 30/min.
//
// Fixed by audit #128: the capacity guard is reuse-aware — it counts the new
// groups' real raw draw plus only the un-redirectable remainder of each reused
// feed, so a build fully covered by redirecting an existing export adds zero
// extraction and is not blocked.
// ---------------------------------------------------------------------------
test("reuse+redirect is not blocked when a fully-claimed node's line is exported", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "REUSE TIGHT", position: { x: -2600, y: 2600 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 30, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 2, clock: 1.0, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  const rodOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 640, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(ingot), to: G(rod), item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: G(rod), to: P(rodOut), item: "Desc_IronRod_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: rodOut, rate: 30 }]); // whole rod output exported

  try {
    await openGraph(page, "REUSE TIGHT");
    // sanity: the export target really is the full 30/min before the build
    expect((await hydrate(request)).plan.ports[rodOut].rate).toBe(30);

    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    await modal.getByTestId("mfr-item-Desc_IronScrew_C").click();
    // the reuse offer names the rod line; ensure the checkbox is ON (default on).
    const reuse = modal.getByTestId("mfr-reuse");
    await expect(reuse).toContainText(/Iron Rod/i);
    const reuseBox = reuse.locator("input[type='checkbox']");
    if (!(await reuseBox.isChecked())) await reuseBox.click();
    await expect(reuseBox).toBeChecked();

    await modal.getByTestId("mfr-rate").fill("120");

    // EXPECTED: feasible via redirect — no shortfall warning, build enabled.
    await expect(modal.getByTestId("mfr-warn")).toHaveCount(0);
    await expect(modal.getByTestId("mfr-build")).toBeEnabled();

    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    const h = await hydrate(request);
    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(1); // reused, not duplicated
    expect(countRecipe(h, f, "Recipe_Screw_C")).toBe(1);
    // the rod's world export was trimmed toward 0 to feed the new screw line
    expect(h.plan.ports[rodOut].rate).toBeLessThan(30);
    // the new screw output carries the full requested rate
    await expect(page.getByTestId("port-out-Desc_IronScrew_C")).toContainText("120");
    // no extra ingot was drawn — the capped node's derived draw stays within 30
    expect(h.derived.factories[f].ports[ingot]).toBeLessThanOrEqual(30 + 1e-6);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 2 — Diamond target sums a shared raw-tier intermediate into ONE group.
//
// Setup: an uncapped iron-ore IN port. Ask for 5/min Reinforced Iron Plate.
// The chain is a diamond: ingot is demanded by BOTH the plate path (45/min) and
// the rod path (15/min) = 60/min total.
//
// EXPECTED (correct behavior): hydrate for the factory contains exactly ONE
// group with recipe Recipe_IngotIron_C — the diamond summed, not duplicated —
// with count 2 and clock ~1.0 (60/min over 2×30/min machines); plus exactly one
// Recipe_IronPlate_C (count 2, clock ~0.75 = 30/min over 2×20), one
// Recipe_Screw_C (count 2, clock ~0.75 = 60/min over 2×40), one Recipe_IronRod_C
// (count 1, 15/min), one Recipe_IronPlateReinforced_C (count 1, 5/min). port-out
// Desc_IronPlateReinforced_C shows 5/min. KEY ASSERTION: number of
// Recipe_IngotIron_C groups == 1.
// ---------------------------------------------------------------------------
test("diamond target sums the shared ingot intermediate into one group", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "DIAMOND", position: { x: -2800, y: 2800 }, region: "GRASS FIELDS" }])).created[0];
  await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_OreIron_C", rate: 0, rateCeiling: null, graphPos: { x: 0, y: 100 } }]);

  try {
    await openGraph(page, "DIAMOND");
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    await modal.getByTestId("mfr-item-Desc_IronPlateReinforced_C").click();
    await modal.getByTestId("mfr-rate").fill("5");
    await expect(modal.getByTestId("mfr-warn")).toHaveCount(0);
    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    const h = await hydrate(request);
    // KEY: the diamond intermediate is ONE summed group, never duplicated.
    expect(countRecipe(h, f, "Recipe_IngotIron_C")).toBe(1);
    const ingotG = groupsOf(h, f, "Recipe_IngotIron_C")[0];
    expect(ingotG.count).toBe(2);
    expect(ingotG.clock).toBeCloseTo(1.0, 3);

    expect(countRecipe(h, f, "Recipe_IronPlate_C")).toBe(1);
    expect(groupsOf(h, f, "Recipe_IronPlate_C")[0].count).toBe(2);
    expect(groupsOf(h, f, "Recipe_IronPlate_C")[0].clock).toBeCloseTo(0.75, 3);

    expect(countRecipe(h, f, "Recipe_Screw_C")).toBe(1);
    expect(groupsOf(h, f, "Recipe_Screw_C")[0].count).toBe(2);
    expect(groupsOf(h, f, "Recipe_Screw_C")[0].clock).toBeCloseTo(0.75, 3);

    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(1);
    expect(groupsOf(h, f, "Recipe_IronRod_C")[0].count).toBe(1);

    expect(countRecipe(h, f, "Recipe_IronPlateReinforced_C")).toBe(1);
    expect(groupsOf(h, f, "Recipe_IronPlateReinforced_C")[0].count).toBe(1);

    await expect(page.getByTestId("port-out-Desc_IronPlateReinforced_C")).toContainText("5");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 3 — Reuse SCALES UP the existing line's machine count when its capacity
// can't cover the new draw.
//
// Setup: iron-ingot IN capped at 200/min; a 1-machine rod line (15/min rod
// capacity) with no downstream consumer. Ask for 80 screws reusing the rod line.
// 80 screws need 20/min rod, exceeding the 1-machine 15/min capacity.
//
// EXPECTED (correct behavior): hydrate shows still exactly ONE Recipe_IronRod_C
// group, but its count is now 2 (scaled from 1 via set_group_count so
// per*count*clock = 30 >= 20); the settle then demand-sizes the live line, so
// the persisted clock reads ~0.667 (20/min shared across 2×15/min machines —
// capacity exactly meets the draw); exactly one Recipe_Screw_C group
// (count 2); port-out Desc_IronScrew_C shows 80/min.
// KEY ASSERTION: rod group count == 2 AND number of rod groups == 1.
// ---------------------------------------------------------------------------
test("reuse scales up the rod machine count when capacity is short", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "SCALEUP", position: { x: -3000, y: 3000 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1.0, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(ingot), to: G(rod), item: "Desc_IronIngot_C", tier: 3 }]);

  try {
    await openGraph(page, "SCALEUP");
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    await modal.getByTestId("mfr-item-Desc_IronScrew_C").click();
    const reuse = modal.getByTestId("mfr-reuse");
    await expect(reuse).toContainText(/Iron Rod/i);
    const reuseBox = reuse.locator("input[type='checkbox']");
    if (!(await reuseBox.isChecked())) await reuseBox.click();
    await expect(reuseBox).toBeChecked();

    await modal.getByTestId("mfr-rate").fill("80");
    await expect(modal.getByTestId("mfr-warn")).toHaveCount(0);
    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    const h = await hydrate(request);
    // still ONE rod group, scaled up (not duplicated)
    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(1);
    const rodG = groupsOf(h, f, "Recipe_IronRod_C")[0];
    expect(rodG.count).toBe(2); // scaled 1 -> 2 to cover 20/min rod
    // The settle demand-sizes the now-live line: 2 machines × 15/min × clock
    // == the 20/min draw. Capacity meets demand exactly — nothing starves.
    expect(rodG.count * 15 * rodG.clock).toBeCloseTo(20, 3);
    expect(countRecipe(h, f, "Recipe_Screw_C")).toBe(1);
    expect(groupsOf(h, f, "Recipe_Screw_C")[0].count).toBe(2);
    await expect(page.getByTestId("port-out-Desc_IronScrew_C")).toContainText("80");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
