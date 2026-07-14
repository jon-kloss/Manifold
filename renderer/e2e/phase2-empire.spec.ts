// Phase 2 exit criterion: an empire — factories joined by belt routes and a
// power grid, with live saturation, deficits, and circuit margins in the audit
// drawer. Factory internals are seeded through the same command surface the UI
// uses; the route/power drawing itself runs through the real map UI.

import { test, expect, type APIRequestContext, type Page } from "@playwright/test";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

/** Center of a factory's map pin (the diamond svg, anchored on its position). */
async function pinCenter(page: Page, name: string): Promise<{ x: number; y: number }> {
  const svg = page.locator(`.pin-wrap:has(.pin-chip:has-text("${name}")) svg`);
  const box = await svg.boundingBox();
  if (!box) throw new Error(`pin not found: ${name}`);
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 };
}

/** Right-drag from one pin to another — the route-drawing gesture. */
async function rightDrag(page: Page, from: { x: number; y: number }, to: { x: number; y: number }) {
  await page.mouse.move(from.x, from.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 5 });
  await page.mouse.move(to.x, to.y, { steps: 5 });
  await page.mouse.up({ button: "right" });
}

test("empire: belt route, power grid, audit drawer", async ({ page, request }) => {
  // ---- seed three factories through the bridge (same commands as the UI) ----
  const mk = async (name: string, x: number, y: number) =>
    (await edit(request, [{ type: "create_factory", name, position: { x, y }, region: "GRASS FIELDS" }]))
      .created[0];
  const port = async (factory: string, direction: string, item: string, ceiling: number | null, x: number) =>
    (
      await edit(request, [
        {
          type: "add_port",
          factory,
          direction,
          item,
          rate: 0,
          rateCeiling: ceiling,
          graphPos: { x, y: 100 },
        },
      ])
    ).created[0];
  const group = async (factory: string, machine: string, recipe: string) =>
    (
      await edit(request, [
        { type: "add_group", factory, machine, recipe, count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 },
      ])
    ).created[0];
  const belt = (factory: string, from: unknown, to: unknown, item: string) =>
    edit(request, [{ type: "add_edge", factory, from, to, item, tier: 3 }]);
  const G = (id: string) => ({ kind: "group", id });
  const P = (id: string) => ({ kind: "port", id });

  // upstream: iron ore → ingots, 30/min target
  const ingotWorks = await mk("INGOT POINT", -2600, 2600);
  await edit(request, [
    { type: "claim_node", factory: ingotWorks, node: "bp_resourcenode496", extractor: "Build_MinerMk2_C", clock: 1.0 },
  ]);
  const oreIn = await port(ingotWorks, "in", "Desc_OreIron_C", 120, 0);
  const ingotOut = await port(ingotWorks, "out", "Desc_IronIngot_C", null, 600);
  const smelters = await group(ingotWorks, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
  await belt(ingotWorks, P(oreIn), G(smelters), "Desc_OreIron_C");
  await belt(ingotWorks, G(smelters), P(ingotOut), "Desc_IronIngot_C");
  await edit(request, [{ type: "set_port_rate", id: ingotOut, rate: 30 }]);

  // downstream: ingots → rods, 30/min target (starves when upstream dips)
  const rodCity = await mk("ROD CITY", -1000, 2600);
  const ingotIn = await port(rodCity, "in", "Desc_IronIngot_C", null, 0);
  const rodOut = await port(rodCity, "out", "Desc_IronRod_C", null, 600);
  const ctors = await group(rodCity, "Build_ConstructorMk1_C", "Recipe_IronRod_C");
  await belt(rodCity, P(ingotIn), G(ctors), "Desc_IronIngot_C");
  await belt(rodCity, G(ctors), P(rodOut), "Desc_IronRod_C");
  await edit(request, [{ type: "set_port_rate", id: rodOut, rate: 30 }]);

  // coal plant: coal → 150 MW
  const coalPlant = await mk("COAL PLANT", -1800, 1400);
  await edit(request, [
    { type: "claim_node", factory: coalPlant, node: "bp_resourcenode600", extractor: "Build_MinerMk2_C", clock: 1.0 },
  ]);
  const coalIn = await port(coalPlant, "in", "Desc_Coal_C", 120, 0);
  const mwOut = await port(coalPlant, "out", "__PowerMW", null, 600);
  const gens = await group(coalPlant, "Build_GeneratorCoal_C", "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C");
  await belt(coalPlant, P(coalIn), G(gens), "Desc_Coal_C");
  await belt(coalPlant, G(gens), P(mwOut), "__PowerMW");
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 150 }]);

  // exit criterion is a FIVE-factory empire: extend the chain two more hops
  // (rods → screws → depot) through the same command surface the UI uses
  const screwWorks = await mk("SCREW WORKS", -200, 2600);
  const rodIn = await port(screwWorks, "in", "Desc_IronRod_C", null, 0);
  const screwOut = await port(screwWorks, "out", "Desc_IronScrew_C", null, 600);
  const screwCtors = await group(screwWorks, "Build_ConstructorMk1_C", "Recipe_Screw_C");
  await belt(screwWorks, P(rodIn), G(screwCtors), "Desc_IronRod_C");
  await belt(screwWorks, G(screwCtors), P(screwOut), "Desc_IronScrew_C");
  await edit(request, [{ type: "set_port_rate", id: screwOut, rate: 60 }]);

  const depot = await mk("DEPOT SOUTH", 600, 2600);
  const screwIn = await port(depot, "in", "Desc_IronScrew_C", null, 0);

  await edit(request, [
    {
      type: "add_route",
      kind: { kind: "belt", tier: 3 },
      from: rodOut,
      to: rodIn,
      path: [{ x: -1000, y: 2600 }, { x: -200, y: 2600 }],
    },
    {
      type: "add_route",
      kind: { kind: "belt", tier: 2 },
      from: screwOut,
      to: screwIn,
      path: [{ x: -200, y: 2600 }, { x: 600, y: 2600 }],
    },
    {
      type: "add_route",
      kind: { kind: "power" },
      from: coalPlant,
      to: screwWorks,
      path: [{ x: -1800, y: 1400 }, { x: -200, y: 2600 }],
    },
  ]);

  // ---- draw the belt route INGOT POINT → ROD CITY through the map UI ----
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.locator(".statusbar")).toContainText(/[5-9] FACTORIES/); // the 5-factory empire
  await page.keyboard.press("f"); // frame all factories
  await page.waitForTimeout(400);

  // ---- ESC cancels a right-drag draft mid-flight (no stuck ghost) ----
  const src = await pinCenter(page, "INGOT POINT");
  const dst = await pinCenter(page, "ROD CITY");
  await page.mouse.move(src.x, src.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move((src.x + dst.x) / 2, (src.y + dst.y) / 2, { steps: 5 });
  await expect(page.locator(".map-placing-hint")).toContainText("RELEASE OVER A FACTORY");
  await page.keyboard.press("Escape");
  await expect(page.locator(".map-placing-hint")).toHaveCount(0);
  await page.mouse.up({ button: "right" });
  await expect(page.getByTestId("route-popover")).not.toBeVisible();

  // ---- releasing over a drawer (outside the map container) cancels, not sticks ----
  await page.locator(`.pin-wrap:has(.pin-chip:has-text("INGOT POINT")) svg`).click(); // select → drawer opens
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  const drawer = (await page.getByTestId("summary-drawer").boundingBox())!;
  await page.mouse.move(src.x, src.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(drawer.x + drawer.width / 2, drawer.y + drawer.height / 2, { steps: 5 });
  await page.mouse.up({ button: "right" });
  await expect(page.locator(".map-placing-hint")).toHaveCount(0);
  await expect(page.getByTestId("route-popover")).not.toBeVisible();

  await rightDrag(page, src, dst);
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await expect(page.locator(".route-cand").first()).toContainText("Iron Ingot");
  // 1.6 km apart: the picker suggests RAIL (A3.3) — this test wants a belt
  await page.selectOption('[data-testid="popover-transport"]', "belt");
  // M26: Enter confirms while the transport <select> is focused — and with a
  // factory selected it must NOT dive into the factory view (map stays up)
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("map-root")).toBeVisible();

  // the created route selects itself → belt route inspector with live load
  await expect(page.getByTestId("route-drawer")).toBeVisible();
  await expect(page.getByTestId("route-drawer")).toContainText("BELT ROUTE");
  await expect(page.getByTestId("route-drawer")).toContainText("Iron Ingot");

  // tier changes re-derive capacity (MK.1 = 60/min)
  await page.getByTestId("route-tier-select").selectOption("1");
  await expect(page.getByTestId("route-drawer")).toContainText("60/min CAP");
  await page.keyboard.press("Escape");

  // ---- draw the power line COAL PLANT → ROD CITY ----
  await rightDrag(page, await pinCenter(page, "COAL PLANT"), await pinCenter(page, "ROD CITY"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  // the coal plant's only unbound OUT is the MW pseudo-item — belts never offer
  // it, so the single candidate is the power line
  await expect(page.locator(".route-cand")).toHaveCount(1);
  await expect(page.locator(".route-cand")).toContainText("Power line");
  await page.getByTestId("btn-route-confirm").click();

  await expect(page.getByTestId("route-drawer")).toBeVisible();
  await expect(page.getByTestId("route-drawer")).toContainText("POWER LINE");
  await expect(page.getByTestId("route-drawer")).toContainText("GRID A");
  await expect(page.getByTestId("route-drawer")).toContainText("150 MW");
  await page.keyboard.press("Escape");

  // status bar shows draw vs generation
  await expect(page.getByTestId("sb-power")).toContainText("/ 150 MW");

  // ---- audit drawer (TAB): live saturation across the whole empire ----
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
  await expect(page.getByTestId("audit-drawer")).toContainText("INGOT POINT ⟶ ROD CITY");
  await expect(page.getByTestId("audit-drawer")).toContainText("ROD CITY ⟶ SCREW WORKS");
  await expect(page.getByTestId("audit-drawer")).toContainText("SCREW WORKS ⟶ DEPOT SOUTH");

  await page.locator(".audit-tab", { hasText: "POWER" }).click();
  await expect(page.getByTestId("audit-drawer")).toContainText("GRID A");
  await expect(page.getByTestId("audit-drawer")).toContainText("COAL PLANT");
  await expect(page.getByTestId("audit-drawer")).toContainText("SCREW WORKS");
  await expect(page.getByTestId("audit-drawer")).toContainText("of 150 MW generated");

  // ---- an upstream dip surfaces as a deficit, never a silent re-target ----
  // (edited through the bridge, so reload to hydrate the new empire state)
  await edit(request, [{ type: "set_port_rate", id: ingotOut, rate: 10 }]);
  await page.reload();
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
  await page.locator(".audit-tab", { hasText: "DEFICITS" }).click();
  await expect(page.getByTestId("audit-drawer")).toContainText("ROD CITY starved of Iron Ingot");
  await page.keyboard.press("Tab");

  // ---- POWER overlay chip toggles with key 2 ----
  const powerChip = page.locator(".overlay-chip", { hasText: "POWER" });
  await expect(powerChip).toHaveClass(/active/);
  await page.keyboard.press("2");
  await expect(powerChip).not.toHaveClass(/active/);
  await page.keyboard.press("2");
  await expect(powerChip).toHaveClass(/active/);

  // ---- a thin circuit tints the PWR chip (orange is a verb: color follows
  // the derived condition). Throttle the coal plant below the grid's draw so
  // GRID A browns out, then restore full generation for the later specs. ----
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 5 }]);
  await page.reload();
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.getByTestId("sb-power")).toHaveClass(/sb-(warn|crit)/);
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 150 }]);
  await page.reload();
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.getByTestId("sb-power")).not.toHaveClass(/sb-(warn|crit)/);
});
