// Audit #124 acceptance (promoted from the audit probe suite): per-grid
// generation carries the SAME nameplate fallback as the empire total — a
// recipe-less (imported-style / geothermal) generator must not read 0 MW on
// its GRID card while the status bar reports its nameplate — and the PWR
// chip's generation segment renders exactly when generation > 0.
//
// Every probe declares its EXPECTED (correct) result in the header BEFORE any
// assertion. Seeded through the same command surface the UI uses, against the
// dev bridge's default fixture catalog (contains Build_GeneratorGeoThermal_C
// @ 200 MW nameplate, and a coal generator + burn recipe).

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

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
async function dismissOnboarding(page: import("@playwright/test").Page) {
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
}

// ---------------------------------------------------------------------------
// PROBE 1 — Geothermal (recipe-less) generator on a grid: per-grid generation
// must equal nameplate, not 0.
//
// EXPECTED: The GRID card containing GEO FARM reads generation = 400 MW
// (200 MW nameplate x 2 generators) and the empire status bar sb-power reports
// the SAME 400 MW of generation — one physical truth, one number.
//
// (Fixed by audit #124: the per-grid gen_of and the empire total now share a
// single per-group closure with the nameplate fallback, so the two can never
// disagree. This test is the permanent regression guard — before the fix the
// grid card read "0 MW of 0 MW generated" / NO GEN while the status bar showed
// 400 MW, and switch-shed thresholds were computed off the 0 baseline.)
// ---------------------------------------------------------------------------
test("geothermal grid card reads nameplate generation, not 0", async ({ page, request }) => {
  await resetView(request);

  const gf = (
    await edit(request, [
      { type: "create_factory", name: "GEO FARM", position: { x: -2600, y: 2400 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  // Two geothermal generators: fuel-less, recipe-less (imported-style). Each is
  // a 200 MW nameplate generator ⇒ 400 MW of generation.
  await edit(request, [
    {
      type: "add_group",
      factory: gf,
      machine: "Build_GeneratorGeoThermal_C",
      recipe: "",
      count: 2,
      clock: 1.0,
      graphPos: { x: 300, y: 100 },
      floor: 0,
    },
  ]);
  const ls = (
    await edit(request, [
      { type: "create_factory", name: "LOAD SINK", position: { x: -1600, y: 2400 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  // Power circuit endpoints are FACTORY ids; the two pin positions are the
  // endpoints, exactly as phase2-empire draws a power line.
  await edit(request, [
    {
      type: "add_route",
      kind: { kind: "power" },
      from: gf,
      to: ls,
      path: [
        { x: -2600, y: 2400 },
        { x: -1600, y: 2400 },
      ],
    },
  ]);

  try {
    await page.goto("/");
    await expect(page.getByTestId("map-root")).toBeVisible();
    await dismissOnboarding(page);

    // Empire generation (nameplate fallback).
    await expect(page.getByTestId("sb-power")).toContainText("400 MW");

    // TAB opens the audit drawer; POWER tab shows per-grid cards.
    await page.keyboard.press("Tab");
    await expect(page.getByTestId("audit-drawer")).toBeVisible();
    await page.locator(".audit-tab", { hasText: "POWER" }).click();

    // The GRID card that lists GEO FARM must attribute the full 400 MW of
    // generation to the grid — the same figure the empire status bar shows.
    const gridCard = page.getByTestId("audit-drawer").locator(".audit-row", { hasText: "GEO FARM" });
    await expect(gridCard).toContainText("400 MW generated");

    // ...and drilling into the factory graph, the recipe-less generator's own
    // card must read that same nameplate, not a false 0 MW: the derive credits
    // it (session.rs inject_generator_nameplates) and the card detects a
    // generator by MACHINE KIND, so its "⚡ GENERATES" line shows 400 MW — the
    // per-generator display agrees with the grid + empire it feeds.
    await page.keyboard.press("Tab"); // close the audit drawer
    await page.locator(".searchbox input").fill("GEO FARM");
    await page.keyboard.press("Enter");
    await page.getByTestId("btn-open-factory").click();
    const genCard = page.locator(".group-card").filter({ hasText: "GENERATES" });
    await expect(genCard).toBeVisible();
    await expect(genCard.locator(".gen-mw")).toHaveText("400 MW");
    await page.keyboard.press("Escape"); // back to the map for the finally cleanup
  } finally {
    await edit(request, [{ type: "delete_factory", id: gf }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: ls }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 1b — A live output-target PROJECTION (t0 preview) on a MIXED factory
// must not flip a recipe-less generator's card to a false 0 MW mid-drag.
//
// EXPECTED: A factory holding a producing smelter (recipe-ful, demand-driven
// from an out-port) AND a recipe-less geothermal generator (200 MW nameplate)
// shows the generator card at "200 MW". Dragging the smelter out-port's target
// (which runs the t0 buildSnapshot projection) drives the smelter card into its
// ◇ projected state, yet the generator card STAYS at 200 MW — the projection
// snapshot drops recipe-less generators (as the Rust snapshot does), so the card
// falls back to the stable derived nameplate instead of reading undefined → 0.
//
// (Regression guard for the PR #66 review MAJOR: before the fallback, the
// generator's "⚡ GENERATES" line read 0 MW for every frame a projection was
// active on its factory — a false "dead generator" flash on any target drag.)
// ---------------------------------------------------------------------------
test("output-target projection keeps a recipe-less generator's card at nameplate", async ({ page, request }) => {
  await resetView(request);

  const mf = (
    await edit(request, [
      { type: "create_factory", name: "MIXED POWER", position: { x: -2600, y: 2000 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  // A demand-driven smelter (ore → ingot) gives us a projectable out-port…
  const oreIn = (
    await edit(request, [
      { type: "add_port", factory: mf, direction: "in", item: "Desc_OreIron_C", rate: 0, rateCeiling: null, graphPos: { x: 0, y: 100 } },
    ])
  ).created[0];
  const ingotOut = (
    await edit(request, [
      { type: "add_port", factory: mf, direction: "out", item: "Desc_IronIngot_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } },
    ])
  ).created[0];
  const smelter = (
    await edit(request, [
      { type: "add_group", factory: mf, machine: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 },
    ])
  ).created[0];
  await edit(request, [
    { type: "add_edge", factory: mf, from: { kind: "port", id: oreIn }, to: { kind: "group", id: smelter }, item: "Desc_OreIron_C", tier: 3 },
    { type: "add_edge", factory: mf, from: { kind: "group", id: smelter }, to: { kind: "port", id: ingotOut }, item: "Desc_IronIngot_C", tier: 3 },
  ]);
  // …and alongside it, a recipe-less geothermal generator (200 MW nameplate).
  await edit(request, [
    { type: "add_group", factory: mf, machine: "Build_GeneratorGeoThermal_C", recipe: "", count: 1, clock: 1.0, graphPos: { x: 300, y: 340 }, floor: 0 },
  ]);

  try {
    await page.goto("/");
    await expect(page.getByTestId("map-root")).toBeVisible();
    await dismissOnboarding(page);

    await page.locator(".searchbox input").fill("MIXED POWER");
    await page.keyboard.press("Enter");
    await page.getByTestId("btn-open-factory").click();

    // Baseline: the generator card reads its nameplate.
    const genCard = page.locator(".group-card").filter({ hasText: "GENERATES" });
    await expect(genCard).toBeVisible();
    await expect(genCard.locator(".gen-mw")).toHaveText("200 MW");

    // Drive the smelter out-port target — this runs the t0 projection.
    await page.keyboard.press("f");
    await page.waitForTimeout(300);
    await page.getByTestId("port-out-Desc_IronIngot_C").click();
    const slider = page.getByTestId("target-slider");
    await expect(slider).toBeVisible();
    const box = (await slider.boundingBox())!;
    await page.mouse.move(box.x + 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width * 0.5, box.y + box.height / 2, { steps: 10 });

    // Mid-drag the projection is live — the target value is italic-projected…
    await expect(page.getByTestId("target-value")).toHaveClass(/projected/);
    // …yet the generator card holds its nameplate, NOT a false 0 MW.
    await expect(genCard.locator(".gen-mw")).toHaveText("200 MW");
    await page.mouse.up();
    await page.waitForTimeout(300);

    // And after the projection settles, still 200 MW.
    await expect(genCard.locator(".gen-mw")).toHaveText("200 MW");
    await page.keyboard.press("Escape");
  } finally {
    await edit(request, [{ type: "delete_factory", id: mf }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 2 — The PWR chip shows the generation segment only when generation > 0
// (draw-vs-generation truth).
//
// EXPECTED: With zero generation, sb-power shows draw only — no '.sb-gen' span
// and no '/' in its text (StatusBar renders the '/ <gen> MW' span only when
// derived.totalGenerationMw > 0). After a generator group exists
// (totalGenerationMw > 0, via the nameplate fallback for an un-wired
// generator), the '.sb-gen' span appears and sb-power text contains ' / '
// followed by the generation figure in MW.
// ---------------------------------------------------------------------------
test("PWR chip shows the generation segment only when generation > 0", async ({ page, request }) => {
  await resetView(request);
  // Ensure the two-click wipe button is present, then run new_empire from the UI
  // so the baseline plan is empty (zero draw, zero generation).
  await edit(request, [
    { type: "create_factory", name: "WIPE SEED A", position: { x: -2400, y: 2400 }, region: "GRASS FIELDS" },
    { type: "create_factory", name: "WIPE SEED B", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" },
  ]);

  let genFactory: string | undefined;
  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await expect(page.getByTestId("map-root")).toBeVisible();

    await page.getByTestId("btn-data-menu").click();
    const reset = page.getByTestId("btn-new-empire");
    await expect(reset).toBeVisible();
    await reset.click(); // arm
    await expect(reset).toContainText(/Click again/i);
    await reset.click(); // confirm → wipe
    await expect
      .poll(async () => Object.keys((await hydrate(request)).plan.factories).length, { timeout: 10_000 })
      .toBe(0);

    // Reload into the empty plan (onboarding gates on the empty plan — dismiss it).
    await page.goto("/");
    await dismissOnboarding(page);
    await expect(page.getByTestId("map-root")).toBeVisible();

    // EXPECTED (zero generation): no .sb-gen span, no '/' in the chip text.
    const sbPower = page.getByTestId("sb-power");
    await expect(sbPower).toContainText(/PWR/);
    await expect(page.locator('[data-testid="sb-power"] .sb-gen')).toHaveCount(0);
    const zeroText = (await sbPower.textContent()) ?? "";
    expect(zeroText).toMatch(/PWR .*MW/);
    expect(zeroText).not.toContain("/");

    // Create a factory holding a single RECIPE-LESS generator group — the
    // nameplate fallback path, which is deterministic: a generator with a
    // resolvable burn recipe but no fuel wiring solves to 0 MW (the solved
    // figure wins over nameplate by design — fuel-starved truth), which
    // would keep generation at 0 and prove nothing about the segment.
    genFactory = (
      await edit(request, [
        { type: "create_factory", name: "GEN PROBE", position: { x: -2600, y: 2600 }, region: "GRASS FIELDS" },
      ])
    ).created[0];
    await edit(request, [
      {
        type: "add_group",
        factory: genFactory,
        machine: "Build_GeneratorGeoThermal_C",
        recipe: "",
        count: 1,
        clock: 1,
        graphPos: { x: 300, y: 80 },
        floor: 0,
      },
    ]);

    // Reload so the store re-derives with the generator present.
    await page.goto("/");
    await dismissOnboarding(page);
    await expect(page.getByTestId("map-root")).toBeVisible();

    // EXPECTED (generation > 0): the .sb-gen span appears and the chip text
    // carries ' / <gen> MW'.
    await expect(page.locator('[data-testid="sb-power"] .sb-gen')).toHaveCount(1);
    const genText = (await page.getByTestId("sb-power").textContent()) ?? "";
    expect(genText).toContain(" / ");
    expect(genText).toMatch(/ \/ .*MW/);
  } finally {
    if (genFactory) await edit(request, [{ type: "delete_factory", id: genFactory }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE (promoted from the audit gamedata probes, #133) — Variable-power
// recipe draw is the per-recipe average, not the machine's fixed/zero draw.
//
// EXPECTED: with Recipe_Diamond_C the factory totalPowerMw = 500 MW (variable
// constant 250 + factor 500/2), and with Recipe_DarkMatter_C on the SAME
// machine it = 1000 MW (500 + 1000/2) — the hungrier recipe beats the machine
// estimate. It must NOT read the machine's raw mPowerConsumption (~0) nor a
// fixed per-machine number.
//
// Harness note (rootcause-pass `hadron`, PROBE_INFRA): 30 diamond/min consumes
// 600 coal/min — the original probe wired the coal belt at Mk.3 (270/min) and
// starved the collider. The coal edge must be Mk.5 (780/min).
// ---------------------------------------------------------------------------
test("hadron variable-power draw follows the recipe, not the machine", async ({ request }) => {
  await resetView(request);
  const ac = (
    await edit(request, [
      { type: "create_factory", name: "ACCEL", position: { x: -2600, y: 1600 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  try {
    // Recipe_Diamond_C: 20 Coal -> 1 Diamond @ 2s ⇒ 30 Diamond/min at one
    // machine, clock 1 — drive the out-port at exactly that rate.
    const coalIn = (
      await edit(request, [
        { type: "add_port", factory: ac, direction: "in", item: "Desc_Coal_C", rate: 0, rateCeiling: null, graphPos: { x: 0, y: 100 } },
      ])
    ).created[0];
    const diamondOut = (
      await edit(request, [
        { type: "add_port", factory: ac, direction: "out", item: "Desc_Diamond_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } },
      ])
    ).created[0];
    const grp = (
      await edit(request, [
        { type: "add_group", factory: ac, machine: "Build_HadronCollider_C", recipe: "Recipe_Diamond_C", count: 1, clock: 1.0, graphPos: { x: 300, y: 100 }, floor: 0 },
      ])
    ).created[0];
    const eCoal = (
      await edit(request, [
        { type: "add_edge", factory: ac, from: { kind: "port", id: coalIn }, to: { kind: "group", id: grp }, item: "Desc_Coal_C", tier: 5 },
      ])
    ).created[0];
    const eDiamond = (
      await edit(request, [
        { type: "add_edge", factory: ac, from: { kind: "group", id: grp }, to: { kind: "port", id: diamondOut }, item: "Desc_Diamond_C", tier: 3 },
      ])
    ).created[0];
    await edit(request, [{ type: "set_port_rate", id: diamondOut, rate: 30 }]);

    let df = (await hydrate(request)).derived.factories[ac];
    expect(df.solveError).toBeNull();
    // 250 constant + 500/2 factor = 500 MW at one machine, clock 1.
    expect(df.totalPowerMw).toBeCloseTo(500, 3);

    // Swap the SAME machine to Recipe_DarkMatter_C (1 Diamond -> 1 DarkMatter
    // @ 2s ⇒ 30/min); rewire to the new recipe's items so the group stays
    // demand-driven at count 1 — the recipe swap is the point.
    await edit(request, [
      { type: "delete_edge", id: eCoal },
      { type: "delete_edge", id: eDiamond },
    ]);
    await edit(request, [
      { type: "set_group_recipe", id: grp, machine: "Build_HadronCollider_C", recipe: "Recipe_DarkMatter_C" },
    ]);
    const diamondIn = (
      await edit(request, [
        { type: "add_port", factory: ac, direction: "in", item: "Desc_Diamond_C", rate: 0, rateCeiling: null, graphPos: { x: 0, y: 260 } },
      ])
    ).created[0];
    const dmOut = (
      await edit(request, [
        { type: "add_port", factory: ac, direction: "out", item: "Desc_DarkMatter_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 260 } },
      ])
    ).created[0];
    await edit(request, [
      { type: "add_edge", factory: ac, from: { kind: "port", id: diamondIn }, to: { kind: "group", id: grp }, item: "Desc_Diamond_C", tier: 3 },
      { type: "add_edge", factory: ac, from: { kind: "group", id: grp }, to: { kind: "port", id: dmOut }, item: "Desc_DarkMatter_C", tier: 3 },
    ]);
    await edit(request, [{ type: "set_port_rate", id: dmOut, rate: 30 }]);

    df = (await hydrate(request)).derived.factories[ac];
    expect(df.solveError).toBeNull();
    // 500 constant + 1000/2 factor = 1000 MW — the hungrier recipe wins.
    expect(df.totalPowerMw).toBeCloseTo(1000, 3);
  } finally {
    await edit(request, [{ type: "delete_factory", id: ac }]).catch(() => {});
  }
});
