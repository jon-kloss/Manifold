// Inter-factory PIPE route drawing: a fluid (water) OUT port routed to a
// matching IN port draws a PIPE, not a belt — the medium follows the item's
// form. Fluids ride pipes (Mk.1–2), so the popover forces "PIPE — fluid" and
// the inspector reads "PIPE ROUTE". (The water-gates-power solve is covered by
// the Rust integration tests; this pins the map UI.)

import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });
test.beforeEach(async ({ request }) => resetView(request));

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

async function pinCenter(page: Page, name: string): Promise<{ x: number; y: number }> {
  const svg = page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`);
  const box = await svg.boundingBox();
  if (!box) throw new Error(`pin not found: ${name}`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

async function rightDrag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 5 });
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up({ button: "right" });
}

test("empire: a water OUT → IN pair draws a PIPE route", async ({ page, request }) => {
  // Clean plan first: a prior import spec can leave its own "WATER WORKS"
  // factory (imported Water Extractors now build one), which would collide with
  // ours by name in the serial suite.
  await request.post(`${API}/new_empire`, { data: "{}" });
  // WATER WORKS ships water; POWER PLANT wants it. Minimal ports are enough to
  // exercise the drawing UI (the popover enumerates unbound OUT ports on the
  // source and item-matches IN ports on the target).
  const waterWorks = (
    await edit(request, [
      { type: "create_factory", name: "WATER WORKS", position: { x: -1000, y: 0 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  await edit(request, [
    {
      type: "add_port",
      factory: waterWorks,
      direction: "out",
      item: "Desc_Water_C",
      rate: 0,
      rateCeiling: null,
      graphPos: { x: 600, y: 100 },
    },
  ]);
  const powerPlant = (
    await edit(request, [
      { type: "create_factory", name: "POWER PLANT", position: { x: 1000, y: 0 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  await edit(request, [
    {
      type: "add_port",
      factory: powerPlant,
      direction: "in",
      item: "Desc_Water_C",
      rate: 0,
      rateCeiling: null,
      graphPos: { x: 0, y: 100 },
    },
  ]);

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await expect(page.getByTestId("map-root")).toBeVisible();

    await rightDrag(page, await pinCenter(page, "WATER WORKS"), await pinCenter(page, "POWER PLANT"));
    await expect(page.getByTestId("route-popover")).toBeVisible();

    // The only candidate is Water — and because it's a fluid, the popover shows
    // the fixed PIPE medium + pipe tiers, NOT the belt/rail/truck/drone select.
    await expect(page.locator(".route-cand").first()).toContainText("Water");
    await expect(page.getByTestId("popover-transport-fluid")).toContainText("PIPE");
    await expect(page.getByTestId("popover-transport")).toHaveCount(0);
    await expect(page.getByTestId("popover-pipe-tier")).toBeVisible();

    await page.getByTestId("btn-route-confirm").click();

    // The created pipe route selects itself → the inspector reads PIPE ROUTE.
    await expect(page.getByTestId("route-drawer")).toBeVisible();
    await expect(page.getByTestId("route-drawer")).toContainText("PIPE ROUTE");
    await expect(page.getByTestId("route-drawer")).toContainText("Water");
    // Pipe tiers stop at Mk.2 (300/600 m³/min).
    await page.getByTestId("route-tier-select").selectOption("2");
    await expect(page.getByTestId("route-drawer")).toContainText("600/min CAP");
  } finally {
    await edit(request, [{ type: "delete_factory", id: waterWorks }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: powerPlant }]).catch(() => {});
  }
});
