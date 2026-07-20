// MAKE smart reuse: asking for screws when the factory already makes rods
// reuses & wires into the existing rod line instead of duplicating it. And the
// "free up the node" action removes an existing consumer to unblock a build.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}`);
  return res.json();
}
const countRecipe = (h: any, factory: string, recipe: string) =>
  Object.values<any>(h.plan.groups).filter((g) => g.factory === factory && g.recipe === recipe).length;

async function openFactoryGraph(page: any, name: string) {
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
}

test("MAKE reuses an existing rod line for screws instead of duplicating it", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "REUSE WORKS", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: ingot }, to: { kind: "group", id: rod }, item: "Desc_IronIngot_C", tier: 3 }]);

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openFactoryGraph(page, "REUSE WORKS");
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    await modal.getByTestId("mfr-item-Desc_IronScrew_C").click();
    // reuse offer appears and names the rod line
    await expect(modal.getByTestId("mfr-reuse")).toBeVisible();
    await expect(modal.getByTestId("mfr-reuse")).toContainText(/Iron Rod/i);

    await modal.getByTestId("mfr-rate").fill("20");
    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    const h = await hydrate(request);
    // still exactly ONE rod group (reused, not duplicated) + one new screw group
    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(1);
    expect(countRecipe(h, f, "Recipe_Screw_C")).toBe(1);
    await expect(page.getByTestId("port-out-Desc_IronScrew_C")).toContainText("20");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

test("MAKE default-to-max under reuse: dialing down sticks, toggling reuse never clobbers the rate", async ({ page, request }) => {
  await resetView(request);
  // Existing rod line + a capped ingot feed → picking screws offers reuse AND
  // seeds the rate to a computed max. Under reuse the raw draw is affine (not
  // proportional), the trap that made a naive rate×ratio max oscillate.
  const f = (await edit(request, [{ type: "create_factory", name: "REUSE MAX WORKS", position: { x: -1800, y: 1800 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: ingot }, to: { kind: "group", id: rod }, item: "Desc_IronIngot_C", tier: 3 }]);

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openFactoryGraph(page, "REUSE MAX WORKS");
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    await modal.getByTestId("mfr-item-Desc_IronScrew_C").click();
    await expect(modal.getByTestId("mfr-reuse")).toBeVisible(); // reuse ON by default

    // Seeded to a positive max the nodes can feed.
    const rateInput = modal.getByTestId("mfr-rate");
    const seeded = Number(await rateInput.inputValue());
    expect(seeded).toBeGreaterThan(1);

    // Dial DOWN — the value must STICK. A rate-dependent max would re-fire the
    // seed and pull it back toward the max (or diverge); it must not.
    const low = Math.max(2, Math.floor(seeded / 3));
    await rateInput.fill(String(low));
    await page.waitForTimeout(600);
    expect(Number(await rateInput.inputValue())).toBe(low);

    // Toggling the reuse checkbox changes the raw totals (hence the computed
    // max) but must NOT re-seed and discard the user's manual rate.
    const reuseBox = modal.getByTestId("mfr-reuse").locator('input[type="checkbox"]');
    await reuseBox.uncheck();
    await page.waitForTimeout(300);
    expect(Number(await rateInput.inputValue())).toBe(low);
    await reuseBox.check();
    await page.waitForTimeout(300);
    expect(Number(await rateInput.inputValue())).toBe(low);

    // ...and the modal is still alive (no max-update-depth loop).
    await expect(modal).toBeVisible();
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

test("MAKE extend redirects a fully-exported intermediate to feed the new consumer", async ({ page, request }) => {
  await resetView(request);
  // A rod line whose entire output is exported to the world (rodOut = 15/min).
  const f = (await edit(request, [{ type: "create_factory", name: "REDIRECT WORKS", position: { x: -2200, y: 2200 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  const rodOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: ingot }, to: { kind: "group", id: rod }, item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "group", id: rod }, to: { kind: "port", id: rodOut }, item: "Desc_IronRod_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: rodOut, rate: 15 }]); // all rod exported

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openFactoryGraph(page, "REDIRECT WORKS");

    expect((await hydrate(request)).plan.ports[rodOut].rate).toBe(15);

    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();
    // Extend: make screws (which consume rod) reusing the rod line.
    await modal.getByTestId("mfr-item-Desc_IronScrew_C").click();
    await expect(modal.getByTestId("mfr-reuse")).toContainText(/Iron Rod/i);
    await modal.getByTestId("mfr-rate").fill("20");
    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    const h = await hydrate(request);
    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(1); // reused, not duplicated
    expect(countRecipe(h, f, "Recipe_Screw_C")).toBe(1);
    // The rod's world export was trimmed to feed the new screw line instead of
    // starving it — the whole point of the extend redirect.
    expect(h.plan.ports[rodOut].rate).toBeLessThan(15);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

test("MAKE free-up removes an existing consumer to unblock a build", async ({ page, request }) => {
  await resetView(request);
  // ingot capped at 30; an existing rod line already draws all of it (headroom 0)
  const f = (await edit(request, [{ type: "create_factory", name: "FREEUP WORKS", position: { x: -1500, y: 1500 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 30, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 2, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  const rodOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: ingot }, to: { kind: "group", id: rod }, item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "group", id: rod }, to: { kind: "port", id: rodOut }, item: "Desc_IronRod_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: rodOut, rate: 30 }]); // 30 rod → 30 ingot, all of it

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openFactoryGraph(page, "FREEUP WORKS");
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");

    // Make plates (consumes ingot) — no ingot headroom left → blocked, free-up offered
    await modal.getByTestId("mfr-item-Desc_IronPlate_C").click();
    await modal.getByTestId("mfr-rate").fill("20");
    await expect(modal.getByTestId("mfr-warn")).toBeVisible();
    const freeup = modal.getByTestId("mfr-freeup");
    await expect(freeup).toBeVisible();

    // two-click confirm removes the rod line, freeing the ingot
    await freeup.click();
    await expect(freeup).toContainText(/CONFIRM/i);
    await freeup.click();

    const h = await hydrate(request);
    expect(countRecipe(h, f, "Recipe_IronRod_C")).toBe(0); // freed
    await expect(modal.getByTestId("mfr-warn")).toHaveCount(0); // unblocked
    await expect(modal.getByTestId("mfr-build")).toBeEnabled();
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
