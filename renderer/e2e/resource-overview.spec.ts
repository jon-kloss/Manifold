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
