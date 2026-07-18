// The map resource-overview panel aggregates empire-wide production/consumption.
// Seed one ore→ingot factory, assert the ledger shows iron ore (make≈use) and
// iron ingot (surplus), plus the power row.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

test("resource overview aggregates empire production/consumption", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "OVERVIEW WORKS", position: { x: -2600, y: 2600 }, region: "GRASS FIELDS" }])).created[0];
  const oreIn = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_OreIron_C", rate: 0, rateCeiling: 120, graphPos: { x: 0, y: 100 } }])).created[0];
  const ingotOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronIngot_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } }])).created[0];
  const smelters = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  const G = (id: string) => ({ kind: "group", id });
  const P = (id: string) => ({ kind: "port", id });
  await edit(request, [{ type: "add_edge", factory: f, from: P(oreIn), to: G(smelters), item: "Desc_OreIron_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: G(smelters), to: P(ingotOut), item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: ingotOut, rate: 30 }]);

  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();

  const panel = page.getByTestId("resource-overview");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("RESOURCES");
  await expect(panel).toContainText("POWER");
  // Iron ingot is produced and exported → shows as a surplus row.
  await expect(panel).toContainText(/Iron Ingot/i);
  // Iron ore is both supplied (raw in-port) and consumed (smelter) → present.
  await expect(panel).toContainText(/Iron Ore/i);

  await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
});

test("detailed view: per-grid power, filter, and per-item factory drill-down (+ fly)", async ({ page, request }) => {
  await resetView(request);
  const G = (id: string) => ({ kind: "group", id });
  const P = (id: string) => ({ kind: "port", id });
  const f = (await edit(request, [{ type: "create_factory", name: "DRILL WORKS", position: { x: -3200, y: 3200 }, region: "GRASS FIELDS" }])).created[0];
  const oreIn = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_OreIron_C", rate: 0, rateCeiling: 120, graphPos: { x: 0, y: 100 } }])).created[0];
  const ingotOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronIngot_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } }])).created[0];
  const smelters = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(oreIn), to: G(smelters), item: "Desc_OreIron_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: G(smelters), to: P(ingotOut), item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: ingotOut, rate: 30 }]);

  // A coal plant on the same power grid so a DerivedCircuit forms → the per-grid
  // POWER section renders (not "NO GRIDS YET").
  const plant = (await edit(request, [{ type: "create_factory", name: "DRILL PLANT", position: { x: -2800, y: 3200 }, region: "GRASS FIELDS" }])).created[0];
  const coalIn = (await edit(request, [{ type: "add_port", factory: plant, direction: "in", item: "Desc_Coal_C", rate: 0, rateCeiling: 240, graphPos: { x: 0, y: 100 } }])).created[0];
  const gen = (await edit(request, [{ type: "add_group", factory: plant, machine: "Build_GeneratorCoal_C", recipe: "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C", count: 4, clock: 1, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: plant, from: P(coalIn), to: G(gen), item: "Desc_Coal_C", tier: 4 }]);
  await edit(request, [{ type: "add_route", kind: { kind: "power" }, from: plant, to: f, path: [{ x: -2800, y: 3200 }, { x: -3200, y: 3200 }] }]);

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    const panel = page.getByTestId("resource-overview");
    await expect(panel).toBeVisible();

    // Brief → detailed via the ⤢ stepper.
    await panel.getByTitle(/Full table \+ grids/i).click();
    await expect(panel.locator(".ro-search")).toBeVisible();
    await expect(panel.locator(".ro-chip", { hasText: "RAW" })).toBeVisible();

    // Per-grid POWER section renders a grid with its headroom.
    await expect(panel.locator(".ro-grid").first()).toBeVisible();
    await expect(panel.locator(".ro-grid-head").first()).toContainText(/HEADROOM/i);

    // Filter narrows the table live as you type.
    await panel.locator(".ro-search").fill("ingot");
    await expect(panel).toContainText(/Iron Ingot/i);
    await expect(panel).not.toContainText(/Iron Ore/i);

    // Drill down a row → the producing factory is named, and clicking it flies
    // the map camera (the leaflet pane transform changes).
    await panel.locator(".ro-rowbtn").first().click();
    await expect(panel.locator(".ro-drill")).toContainText(/DRILL WORKS/i);
    const paneT = () => page.evaluate(() => getComputedStyle(document.querySelector(".leaflet-map-pane") as HTMLElement).transform);
    const before = await paneT();
    await panel.locator(".ro-drill .ro-fac", { hasText: "DRILL WORKS" }).first().click();
    await page.waitForTimeout(800);
    expect(await paneT()).not.toEqual(before);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }, { type: "delete_factory", id: plant }]).catch(() => {});
  }
});

test("collapsed rail: step down to the rail and back up", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "RAIL WORKS", position: { x: -3400, y: 3400 }, region: "GRASS FIELDS" }])).created[0];
  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    const panel = page.getByTestId("resource-overview");
    await expect(panel).toBeVisible();

    // Brief → collapsed via the ◂ stepper: the vertical rail appears.
    await panel.getByTitle(/Collapse to rail/i).click();
    await expect(panel.locator(".ro-rail")).toBeVisible();
    await expect(panel).toContainText("RESOURCES");
    await expect(panel).toContainText("POWER");
    // Table/power headline are hidden while collapsed.
    await expect(panel.locator(".ro-powerblock")).toHaveCount(0);

    // Rail click steps back up to brief.
    await panel.locator(".ro-rail").click();
    await expect(panel.locator(".ro-powerblock")).toBeVisible();
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
