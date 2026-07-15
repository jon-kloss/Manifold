// H3 regression: selecting a card must NOT unmount the belt edges. The old
// blanket node rebuild handed xyflow all-new node objects without `measured`,
// so every EdgeWrapper nulled for a re-measure window — every .edge-flowing
// overlay was destroyed and recreated, restarting all dash animations at
// phase 0 in unison on any click. DOM identity is the honest probe: we tag a
// live .edge-flowing path with a marker attribute, select a card, and assert
// THE SAME DOM NODE is still there. A remount creates fresh elements → the
// marker vanishes → this fails on pre-fix code.

import { test, expect, type APIRequestContext } from "@playwright/test";

import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

// Deterministic map boot — never inherit a dead predecessor's viewState.
test.beforeEach(async ({ request }) => resetView(request));

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

test("graph motion: belt edges survive selection (no unmount, no dash restart)", async ({ page, request }) => {
  // ---- seed one flowing factory through the bridge (same commands as the UI) ----
  const factory = (
    await edit(request, [
      { type: "create_factory", name: "MOTION RIG", position: { x: -3600, y: 900 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  const port = async (direction: string, item: string, rate: number, ceiling: number | null, x: number) =>
    (
      await edit(request, [
        { type: "add_port", factory, direction, item, rate, rateCeiling: ceiling, graphPos: { x, y: 100 } },
      ])
    ).created[0];
  const group = async (machine: string, recipe: string, x: number) =>
    (
      await edit(request, [
        { type: "add_group", factory, machine, recipe, count: 1, clock: 1.0, graphPos: { x, y: 100 }, floor: 0 },
      ])
    ).created[0];

  // ore in → smelter → constructor → rods out, with a target so flow > 0.
  const oreIn = await port("in", "Desc_OreIron_C", 0, 120, 0);
  const smelter = await group("Build_SmelterMk1_C", "Recipe_IngotIron_C", 300);
  const ctor = await group("Build_ConstructorMk1_C", "Recipe_IronRod_C", 600);
  const rodOut = await port("out", "Desc_IronRod_C", 0, null, 900);
  await edit(request, [
    { type: "add_edge", factory, from: { kind: "port", id: oreIn }, to: { kind: "group", id: smelter }, item: "Desc_OreIron_C", tier: 2 },
    { type: "add_edge", factory, from: { kind: "group", id: smelter }, to: { kind: "group", id: ctor }, item: "Desc_IronIngot_C", tier: 2 },
    { type: "add_edge", factory, from: { kind: "group", id: ctor }, to: { kind: "port", id: rodOut }, item: "Desc_IronRod_C", tier: 2 },
    { type: "set_port_rate", id: rodOut, rate: 15 },
  ]);

  // ---- open the factory graph ----
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });
  // Settle to the map: dismiss any auto-presented dashboard / leftover view.
  await expect(async () => {
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("dashboard")).toBeHidden({ timeout: 1000 });
  }).toPass({ timeout: 10_000 });
  await page.locator(".searchbox input").fill("motion rig");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();

  // The chain flows: all three belts carry derived flow > 0.
  await expect(page.locator("path.edge-flowing")).toHaveCount(3);
  const beltLabels = await page.locator(".belt-label").count();
  expect(beltLabels).toBeGreaterThan(0);

  // ---- DOM-identity marker: tag a live flowing overlay, then select a card ----
  await page.locator("path.edge-flowing").first().evaluate((el) => el.setAttribute("data-marker", "keep"));
  await page.locator(".group-card").first().click();
  // Give a hypothetical remount window (~190-430ms pre-fix) time to happen.
  await page.waitForTimeout(400);

  // Same DOM node still present → the edge never unmounted, the dash phase
  // never reset. A remount would mint fresh elements without the marker.
  await expect(page.locator('path.edge-flowing[data-marker="keep"]')).toHaveCount(1);
  // And selection didn't drop any edge chrome either.
  await expect(page.locator(".belt-label")).toHaveCount(beltLabels);
});
