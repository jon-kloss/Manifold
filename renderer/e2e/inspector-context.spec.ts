// Regression (#5): the factory-level OUTPUT TARGET + its binding warning are
// the overview control. When a specific belt / machine is selected, that
// entity's own sections own the inspector — the factory OUTPUT TARGET must NOT
// also render (it used to, showing e.g. a CABLE target + a BIOMASS-belt binding
// over a selected IRON ROD belt).

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function openGraph(page: any, name: string) {
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
  await expect(page.locator(".react-flow__pane")).toBeVisible();
  await page.waitForTimeout(300);
}

test("selecting a belt/machine hides the factory-level OUTPUT TARGET", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "CTX WORKS", position: { x: -1700, y: 1700 }, region: "GRASS FIELDS" }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1, graphPos: { x: 260, y: 140 }, floor: 0 }])).created[0];
  const rodOut = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 15, rateCeiling: null, graphPos: { x: 620, y: 140 } }])).created[0];
  const beltId = (await edit(request, [{ type: "add_edge", factory: f, from: { kind: "group", id: rod }, to: { kind: "port", id: rodOut }, item: "Desc_IronRod_C", tier: 1 }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openGraph(page, "CTX WORKS");

    // The inspector opens on a selection. Selecting the OUT port shows the
    // factory OUTPUT TARGET (that's the port it targets).
    await page.locator(`.react-flow__node[data-id="${rodOut}"]`).click();
    await expect(page.getByText(/OUTPUT TARGET/)).toBeVisible();

    // Select the machine → its own CLOCK section owns the panel; the factory
    // OUTPUT TARGET no longer renders alongside it.
    await page.locator(`.react-flow__node[data-id="${rod}"]`).click();
    await expect(page.getByText(/^CLOCK —/)).toBeVisible();
    await expect(page.getByText(/OUTPUT TARGET/)).toHaveCount(0);

    // Select the belt (the reported case) → the BELT section owns the panel; the
    // factory OUTPUT TARGET (a different item) must NOT show over it.
    await page.locator(`.react-flow__edge[data-id="${beltId}"]`).click();
    await expect(page.getByText(/^BELT —/)).toBeVisible();
    await expect(page.getByText(/OUTPUT TARGET/)).toHaveCount(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
