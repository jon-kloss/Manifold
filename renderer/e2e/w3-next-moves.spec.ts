// PR 9 opportunity engine: /api/next ranks derived candidates in class order
// (broken → growth) and the dashboard's NEXT MOVES section surfaces the top
// cards with working actions. Self-contained: seeds its own starved COPPER
// chain (an item no earlier spec starves, so empire-wide grouping stays this
// spec's own) and a thin-headroom coal grid through the bridge; every UI
// assertion is checked against this run's own /api/next payload, so leftover
// state from earlier serial specs can add rows without breaking anything.
// Named w3- so it runs AFTER the phase specs — its seeded factories must not
// perturb their pin-count/pin-declutter assumptions.

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

interface Move {
  id: string;
  kind: string;
  title: string;
  evidence: string;
  action: { kind: string; item?: string; rate?: number; tab?: string };
}

test("next moves: ranked families over API + dashboard cards with live actions", async ({
  page,
  request,
}) => {
  // ---- seed through the bridge (same commands as the UI) ----
  const mk = async (name: string, x: number, y: number) =>
    (await edit(request, [{ type: "create_factory", name, position: { x, y }, region: "GRASS FIELDS" }]))
      .created[0];
  const port = async (factory: string, direction: string, item: string, ceiling: number | null, x: number) =>
    (
      await edit(request, [
        { type: "add_port", factory, direction, item, rate: 0, rateCeiling: ceiling, graphPos: { x, y: 100 } },
      ])
    ).created[0];
  const group = async (factory: string, machine: string, recipe: string, count: number) =>
    (
      await edit(request, [
        { type: "add_group", factory, machine, recipe, count, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 },
      ])
    ).created[0];
  const belt = (factory: string, from: unknown, to: unknown, item: string) =>
    edit(request, [{ type: "add_edge", factory, from, to, item, tier: 5 }]);
  const G = (id: string) => ({ kind: "group", id });
  const P = (id: string) => ({ kind: "port", id });

  // Starved copper chain: OPPORTUNITY BAY ships copper ingots to WIRE GULCH.
  // The 480-wire target (240 ingots in) is set while satisfiable (an
  // unachievable target would be clamp-written-back), then the upstream dips
  // to 10/min → an honest 230/min copper-ingot deficit, big enough to lead
  // class 1 whatever earlier specs left behind.
  const bay = await mk("OPPORTUNITY BAY", -3000, -1200);
  const bayOreIn = await port(bay, "in", "Desc_OreCopper_C", 480, 0);
  const bayOut = await port(bay, "out", "Desc_CopperIngot_C", null, 600);
  const smelters = await group(bay, "Build_SmelterMk1_C", "Recipe_IngotCopper_C", 8);
  await belt(bay, P(bayOreIn), G(smelters), "Desc_OreCopper_C");
  await belt(bay, G(smelters), P(bayOut), "Desc_CopperIngot_C");
  await edit(request, [{ type: "set_port_rate", id: bayOut, rate: 240 }]);

  const gulch = await mk("WIRE GULCH", -2400, -1200);
  const gulchIn = await port(gulch, "in", "Desc_CopperIngot_C", null, 0);
  const gulchOut = await port(gulch, "out", "Desc_Wire_C", null, 600);
  const ctors = await group(gulch, "Build_ConstructorMk1_C", "Recipe_Wire_C", 16);
  await belt(gulch, P(gulchIn), G(ctors), "Desc_CopperIngot_C");
  await belt(gulch, G(ctors), P(gulchOut), "Desc_Wire_C");
  await edit(request, [
    {
      type: "add_route",
      kind: { kind: "belt", tier: 4 },
      from: bayOut,
      to: gulchIn,
      path: [{ x: -3000, y: -1200 }, { x: -2400, y: -1200 }],
    },
  ]);
  await edit(request, [{ type: "set_port_rate", id: gulchOut, rate: 480 }]);
  await edit(request, [{ type: "set_port_rate", id: bayOut, rate: 10 }]); // the dip

  // Thin-headroom grid: BROWNOUT RIDGE generates 75 MW; LOAD LEDGE draws
  // 64 MW (16 smelters @ 4 MW, clock 1.0) → ~15% headroom (warn band).
  const ridge = await mk("BROWNOUT RIDGE", -3000, -600);
  const coalIn = await port(ridge, "in", "Desc_Coal_C", 480, 0);
  const mwOut = await port(ridge, "out", "__PowerMW", null, 600);
  const gens = await group(ridge, "Build_GeneratorCoal_C", "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C", 4);
  await belt(ridge, P(coalIn), G(gens), "Desc_Coal_C");
  await belt(ridge, G(gens), P(mwOut), "__PowerMW");
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 75 }]);

  const ledge = await mk("LOAD LEDGE", -2400, -600);
  const ledgeOreIn = await port(ledge, "in", "Desc_OreIron_C", 780, 0);
  const ledgeOut = await port(ledge, "out", "Desc_IronIngot_C", null, 600);
  const bank = await group(ledge, "Build_SmelterMk1_C", "Recipe_IngotIron_C", 16);
  await belt(ledge, P(ledgeOreIn), G(bank), "Desc_OreIron_C");
  await belt(ledge, G(bank), P(ledgeOut), "Desc_IronIngot_C");
  await edit(request, [{ type: "set_port_rate", id: ledgeOut, rate: 480 }]);
  await edit(request, [
    {
      type: "add_route",
      kind: { kind: "power" },
      from: ridge,
      to: ledge,
      path: [{ x: -3000, y: -600 }, { x: -2400, y: -600 }],
    },
  ]);

  // ---- GET /api/next: family presence + class order ----
  const res = await request.get(`${API}/next`);
  expect(res.ok()).toBeTruthy();
  const { opportunities } = (await res.json()) as { opportunities: Move[] };
  expect(opportunities.length).toBeGreaterThan(0);
  expect(opportunities.length).toBeLessThanOrEqual(12);

  const deficit = opportunities.find(
    (o) => o.kind === "deficit_repair" && o.title.includes("Copper Ingot"),
  );
  expect(deficit, "starved copper chain must fire deficit_repair").toBeTruthy();
  expect(deficit!.action.kind).toBe("wizardGoal");
  expect(deficit!.action.item).toBe("Desc_CopperIngot_C");
  expect(deficit!.action.rate).toBe(230); // ceil(240 needed − 10 supplied)
  expect(deficit!.evidence).toContain("/min");

  const margin = opportunities.find((o) => o.kind === "power_margin");
  expect(margin, "thin-headroom grid must fire power_margin").toBeTruthy();
  expect(margin!.evidence).toContain("MW");
  expect(margin!.action.kind).toBe("openAudit");
  expect(margin!.action.tab).toBe("power");

  // class order: every deficit_repair (class 1) ranks above power_margin (3)
  const lastDeficit = opportunities.map((o) => o.kind).lastIndexOf("deficit_repair");
  const firstMargin = opportunities.findIndex((o) => o.kind === "power_margin");
  expect(lastDeficit).toBeLessThan(firstMargin);

  // ---- dashboard: NEXT MOVES renders the cards, actions work ----
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });
  await page.keyboard.press("h");
  await expect(page.getByTestId("dashboard")).toBeVisible();
  const section = page.getByTestId("next-moves");
  await expect(section).toBeVisible();
  await expect(section).toContainText(`NEXT MOVES (${opportunities.length})`);

  // the top card is the ranked-first opportunity, evidence text verbatim
  const cards = page.getByTestId("next-move");
  await expect(cards.first()).toContainText(opportunities[0].title);
  await expect(page.getByTestId("next-move-evidence").first()).toHaveText(opportunities[0].evidence);
  // top 3 + "+N more" when the list is longer
  if (opportunities.length > 3) {
    await expect(cards).toHaveCount(3);
    await expect(page.getByTestId("next-moves-more")).toContainText(`+${opportunities.length - 3} more`);
  }

  // PLAN IT on the copper card → dashboard dismisses, wizard opens prefilled
  const deficitCard = cards.filter({ hasText: "Copper Ingot is short" }).first();
  await expect(deficitCard).toBeVisible();
  await deficitCard.getByTestId("next-move-action").click();
  await expect(page.getByTestId("dashboard")).not.toBeVisible();
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await expect(page.getByTestId("wizard-item")).toHaveValue("Copper Ingot");
  await expect(page.locator('[data-testid="wizard-rate"]')).toHaveValue(String(deficit!.action.rate));
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("wizard-modal")).not.toBeVisible();

  // OPEN on the power-margin card (when it made the top 3) → audit drawer
  // opens on the POWER tab through the store-level request. The branch is
  // decided by the PAYLOAD — only opportunities[0..3] render as cards — never
  // by a racy isVisible probe that turns a slow render into a silent skip.
  await page.keyboard.press("h");
  await expect(page.getByTestId("dashboard")).toBeVisible();
  await expect(page.getByTestId("next-moves")).toBeVisible();
  if (opportunities.slice(0, 3).some((o) => o.kind === "power_margin")) {
    const marginCard = page.getByTestId("next-move").filter({ hasText: "headroom" }).first();
    await marginCard.getByTestId("next-move-action").click();
    await expect(page.getByTestId("dashboard")).not.toBeVisible();
    await expect(page.getByTestId("audit-drawer")).toBeVisible();
    await expect(page.locator(".audit-tab.active")).toContainText("POWER");
    await page.keyboard.press("Tab"); // close the drawer again
    await expect(page.getByTestId("audit-drawer")).not.toBeVisible();
    // Repeat the IDENTICAL OPEN cycle. A dropped clearAuditRequest would
    // leave auditRequest latched on "power" — the second, identical set()
    // becomes a no-op, App's open-effect never re-fires, and the drawer
    // stays shut. Reopening proves the request is consume-and-clear.
    await page.keyboard.press("h");
    await expect(page.getByTestId("dashboard")).toBeVisible();
    await marginCard.getByTestId("next-move-action").click();
    await expect(page.getByTestId("dashboard")).not.toBeVisible();
    await expect(page.getByTestId("audit-drawer")).toBeVisible();
    await expect(page.locator(".audit-tab.active")).toContainText("POWER");
    await page.keyboard.press("Tab");
  } else {
    // margin ranked below the top 3 — the API half already proved the
    // family + action; just dismiss the dashboard.
    await page.keyboard.press("Escape");
  }
  await expect(page.getByTestId("audit-drawer")).not.toBeVisible();

  // ---- SHOW flies the camera to the subject (M5) ----
  // Repair this spec's own broken/trending causes so a growth-class map
  // -subject card can enter the rendered top 3: copper satisfied again (240
  // is the smelter bank's exact ceiling, so it sticks) and the coal grid
  // fattened past the 20% warn band (4 generators cover 100 MW). Leftover
  // deficits from earlier serial specs may still rank first — the pick stays
  // payload-derived; only its top-3 placement is asserted.
  await edit(request, [{ type: "set_port_rate", id: bayOut, rate: 240 }]);
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 100 }]);
  const res2 = await request.get(`${API}/next`);
  expect(res2.ok()).toBeTruthy();
  const fresh = ((await res2.json()) as { opportunities: Move[] }).opportunities;
  const flyIdx = fresh.findIndex(
    (o) => o.action.kind === "selectNode" || o.action.kind === "selectRoute",
  );
  expect(flyIdx, "seed must surface a map-subject SHOW card").toBeGreaterThanOrEqual(0);
  expect(flyIdx, "the SHOW card must be rendered (top 3)").toBeLessThan(3);
  const fly = fresh[flyIdx];

  // moveend stamps the settled world-coord center on map-root (M5 testability)
  const before = await page.getByTestId("map-root").getAttribute("data-center");
  expect(before, "map stamps its settled center").toBeTruthy();
  await page.keyboard.press("h");
  await expect(page.getByTestId("dashboard")).toBeVisible();
  const flyCard = page.getByTestId("next-move").nth(flyIdx);
  await expect(flyCard).toContainText(fly.title);
  await flyCard.getByTestId("next-move-action").click();
  // dashboard dismissed, the camera actually moved (stamp changed on settle),
  // and the subject's drawer is open over the revealed map.
  await expect(page.getByTestId("dashboard")).not.toBeVisible();
  await expect(page.getByTestId("map-root")).not.toHaveAttribute("data-center", before!);
  await expect(
    page.getByTestId(fly.action.kind === "selectNode" ? "node-drawer" : "route-drawer"),
  ).toBeVisible();
});
