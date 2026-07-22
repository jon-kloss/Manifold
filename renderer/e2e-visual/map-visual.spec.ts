// Visual functional sweep of the WORLD MAP surface. Every test screenshots the
// state it asserts (out/shots) and records a verdict (out/results.jsonl); the
// HTML report is built from both by report.mjs.
//
// Deliberately NOT serial-mode: workers=1 keeps declaration order, and a
// failing test must not cascade-skip the rest — every area needs a verdict.
// Tests are self-seeding (API) and boot a fresh page, so each stands alone on
// the progressively-built empire.

import { test, expect } from "@playwright/test";

import {
  belt,
  bootMap,
  edit,
  G,
  hydrate,
  mkFactory,
  mkGroup,
  mkPort,
  newEmpire,
  P,
  pinCenter,
  recordVerdict,
  resetView,
  rightDrag,
  shot,
} from "./vh";

test.afterEach(async ({}, testInfo) => recordVerdict(testInfo));

// ---------------------------------------------------------------------------
test("01 boot: empty empire shows onboarding, skip lands on the map", async ({ page, request }) => {
  await newEmpire(request);
  await resetView(request);
  await page.goto("/");

  const onboarding = page.getByTestId("onboarding");
  await expect(onboarding).toBeVisible();
  await shot(page, test.info(), "onboarding", "Fresh empire: the onboarding card with its doors (wizard / factory / import / docs).");

  await page.getByTestId("onboard-skip").click();
  await expect(page.getByTestId("map-root")).toBeVisible();
  await expect(page.locator(".statusbar")).toContainText("FACTORIES");
  await shot(page, test.info(), "clean-map", "The empty world map: survey grid, resource nodes, legend, status bar. 0 factories.");
});

// ---------------------------------------------------------------------------
test("02 zoom and pan: wheel zoom updates the % readout, drag pans, F frames", async ({ page, request }) => {
  await mkFactory(request, "VIEWPORT WORKS", -1800, 1800);
  await bootMap(page, request);

  const root = (await page.getByTestId("map-root").boundingBox())!;
  const cx = root.x + root.width / 2;
  const cy = root.y + root.height / 2;

  // The wheel zoom is a custom rAF-eased handler (zoom-smooth spec): wait for
  // a steady frame cadence (headless chromium parks rAF on a fresh page), then
  // dispatch wheel events on the leaflet container and read the live
  // data-zoom stamp — the % readout can round tiny eased steps away.
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        let last = performance.now();
        let streak = 0;
        const tick = () => {
          const now = performance.now();
          streak = now - last < 50 ? streak + 1 : 0;
          last = now;
          if (streak >= 5) resolve();
          else requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
  );
  const zoomStamp = async () =>
    Number(await page.getByTestId("map-root").getAttribute("data-zoom"));
  const wheel = (deltaY: number) =>
    page.evaluate(
      ({ x, y, dy }) => {
        const el = document.querySelector(".leaflet-container") as HTMLElement;
        el.dispatchEvent(
          new WheelEvent("wheel", { deltaY: dy, clientX: x, clientY: y, bubbles: true, cancelable: true }),
        );
      },
      { x: cx, y: cy, dy: deltaY },
    );

  const zoomBefore = await zoomStamp();
  const pctBefore = await page.getByTestId("zoom-pct").innerText();
  // zoom OUT — the boot view can already sit at the zoom ceiling, where a
  // zoom-in is a legitimate no-op
  await wheel(720);
  await expect.poll(zoomStamp).not.toBe(zoomBefore);
  const zoomOut = await zoomStamp();
  await shot(
    page,
    test.info(),
    "zoomed-out",
    `Wheel zoom out: live zoom ${zoomBefore.toFixed(2)} → ${zoomOut.toFixed(2)} (readout ${pctBefore} → ${await page.getByTestId("zoom-pct").innerText()}).`,
  );

  await wheel(-720);
  await expect.poll(zoomStamp).not.toBe(zoomOut);
  await shot(
    page,
    test.info(),
    "zoomed-in",
    `Wheel zoom back in through fractional eased steps: ${zoomOut.toFixed(2)} → ${(await zoomStamp()).toFixed(2)}.`,
  );

  await page.mouse.move(cx - 200, cy + 150);
  await page.mouse.down();
  await page.mouse.move(cx + 150, cy - 100, { steps: 8 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  await shot(page, test.info(), "panned", "After a left-drag pan on empty ground — the viewport moved, nothing was selected or created.");

  await page.keyboard.press("f");
  await page.waitForTimeout(500);
  await expect(page.locator(".pin-wrap", { hasText: "VIEWPORT WORKS" })).toBeVisible();
  await shot(page, test.info(), "framed", "F frames all factories: VIEWPORT WORKS pin centered in view.");
});

// ---------------------------------------------------------------------------
test("03 terrain overlay toggles from the toolbar button", async ({ page, request }) => {
  await bootMap(page, request);
  await shot(page, test.info(), "terrain-off", "Baseline map before toggling the terrain overlay.");
  await page.getByTestId("btn-overlay-terrain").click();
  await page.waitForTimeout(400);
  await shot(page, test.info(), "terrain-on", "Terrain overlay ON (biome ghost labels / placeholder treatment).");
  await page.getByTestId("btn-overlay-terrain").click();
  await page.waitForTimeout(200);
});

// ---------------------------------------------------------------------------
test("04 node search live-filters the map; no-match query hides nodes; clear restores", async ({ page, request }) => {
  await bootMap(page, request);
  const root = page.getByTestId("map-root");
  const shown = async () => Number(await root.getAttribute("data-nodes-shown"));

  const all = await shown();
  expect(all).toBeGreaterThan(50);

  await page.locator(".searchbox input").fill("iron");
  await expect.poll(shown).toBeLessThan(all);
  expect(await shown()).toBeGreaterThan(0);
  await shot(page, test.info(), "filter-iron", `Search "iron": only iron nodes stay lit (${await shown()} of ${all}).`);

  await page.locator(".searchbox input").fill("");
  await expect.poll(shown).toBe(all);
  await shot(page, test.info(), "filter-cleared", `Query cleared: all ${all} nodes shown again.`);
});

// ---------------------------------------------------------------------------
test("05 node drawer: search-jump opens a coal node, claim it for a factory", async ({ page, request }) => {
  const f = await mkFactory(request, "CLAIM WORKS", -2700, 2500);
  await bootMap(page, request);

  await page.locator(".searchbox input").fill("coal");
  await page.keyboard.press("Enter");
  const drawer = page.getByTestId("node-drawer");
  await expect(drawer).toBeVisible();
  await shot(page, test.info(), "node-drawer", "Enter on a search hit jumps to the node and opens its drawer: resource, purity, extractor pick, CLAIM FOR.");

  const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
  await claimFor.locator("select").first().selectOption({ label: "CLAIM WORKS" });
  await page.getByTestId("btn-claim").click();
  await expect(drawer.locator("section:has(h3:has-text('CLAIMS')) .drawer-row")).toHaveCount(1);

  const h = await hydrate(request);
  const claims = Object.values<{ factory: string }>(h.plan.nodeClaims).filter((c) => c.factory === f);
  expect(claims).toHaveLength(1);
  const coalIn = Object.values<{ factory: string; item: string; direction: string; rateCeiling: number | null }>(
    h.plan.ports,
  ).filter((p) => p.factory === f && p.item === "Desc_Coal_C" && p.direction === "in");
  expect(coalIn).toHaveLength(1);
  expect(coalIn[0].rateCeiling).toBeGreaterThan(0);
  await shot(page, test.info(), "claimed", "After CLAIM: the drawer lists the claim, and a tether links the node to CLAIM WORKS (coal IN port auto-created with the extraction ceiling).");
});

// ---------------------------------------------------------------------------
test("06 manual factory creation: N enters placing mode, a map click drops the pin", async ({ page, request }) => {
  await bootMap(page, request);
  const before = Object.keys((await hydrate(request)).plan.factories).length;

  await page.keyboard.press("n");
  await shot(page, test.info(), "placing-mode", "Placing mode armed via the N hotkey (also the + FACTORY button).");

  const root = (await page.getByTestId("map-root").boundingBox())!;
  await page.mouse.click(root.x + root.width * 0.62, root.y + root.height * 0.38);

  await expect
    .poll(async () => Object.keys((await hydrate(request)).plan.factories).length)
    .toBe(before + 1);
  const h = await hydrate(request);
  const created = Object.values<{ name: string; region: string }>(h.plan.factories).find((f) =>
    /^FACTORY \d+$/.test(f.name),
  );
  expect(created, "auto-named FACTORY N with an auto-resolved region").toBeTruthy();
  await expect(page.locator(".pin-wrap", { hasText: created!.name })).toBeVisible();
  await shot(page, test.info(), "pin-dropped", `Click placed ◇ ${created!.name} (region auto-resolved to ${created!.region}).`);
});

// ---------------------------------------------------------------------------
test("07 pin drag moves the factory; a map pan does not", async ({ page, request }) => {
  const f = await mkFactory(request, "DRAGPIN VISUAL", -1200, 900);
  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  const pin = page.locator(".pin-wrap", { hasText: "DRAGPIN VISUAL" });
  await expect(pin).toBeVisible();
  const before = (await hydrate(request)).plan.factories[f].position;
  const b = (await pin.boundingBox())!;
  await shot(page, test.info(), "before-drag", "DRAGPIN VISUAL at its seeded position, about to be dragged by the pin head.");
  await page.mouse.move(b.x + b.width / 2, b.y + 8);
  await page.mouse.down();
  await page.mouse.move(b.x + b.width / 2 + 80, b.y + 8 + 50, { steps: 8 });
  await page.mouse.up();

  await expect
    .poll(async () => {
      const p = (await hydrate(request)).plan.factories[f].position;
      return Math.hypot(p.x - before.x, p.y - before.y);
    })
    .toBeGreaterThan(1);
  const afterDrag = (await hydrate(request)).plan.factories[f].position;
  await shot(page, test.info(), "after-drag", "The drag committed a new stored position (move_factory_pin).");

  const root = (await page.getByTestId("map-root").boundingBox())!;
  await page.mouse.move(root.x + 150, root.y + root.height - 150);
  await page.mouse.down();
  await page.mouse.move(root.x + 60, root.y + root.height - 220, { steps: 8 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const afterPan = (await hydrate(request)).plan.factories[f].position;
  expect(Math.hypot(afterPan.x - afterDrag.x, afterPan.y - afterDrag.y)).toBeLessThan(0.5);
});

// ---------------------------------------------------------------------------
test("08 summary drawer: pin click opens it; OPEN FACTORY enters the graph view", async ({ page, request }) => {
  await mkFactory(request, "DRAWER WORKS", 400, 900);
  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  const pc = await pinCenter(page, "DRAWER WORKS");
  await page.mouse.click(pc.x, pc.y);
  const drawer = page.getByTestId("summary-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("DRAWER WORKS");
  await expect(page.getByTestId("factory-elevation")).toBeVisible();
  await shot(page, test.info(), "summary-drawer", "Pin click → summary drawer: name, region, elevation, theme, OPEN FACTORY / BUILD SHEET actions.");

  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();
  await expect(page.getByTestId("graph-empty-hint")).toBeVisible();
  await shot(page, test.info(), "empty-graph", "OPEN FACTORY on an empty factory: the graph view with its empty-state hint and tool rail.");

  await page.getByRole("button", { name: "WORLD MAP" }).click();
  await expect(page.getByTestId("map-root")).toBeVisible();
});

// ---------------------------------------------------------------------------
test("09 hand-built factory: the graph shows the machines, ports, belts and clocks", async ({ page, request }) => {
  const f = await mkFactory(request, "INGOT VISUAL WORKS", -2600, 2600);
  const oreIn = await mkPort(request, f, "in", "Desc_OreIron_C", 120, 0);
  const ingotOut = await mkPort(request, f, "out", "Desc_IronIngot_C", null, 600);
  const smelt = await mkGroup(request, f, "Build_SmelterMk1_C", "Recipe_IngotIron_C");
  await belt(request, f, P(oreIn), G(smelt), "Desc_OreIron_C");
  await belt(request, f, G(smelt), P(ingotOut), "Desc_IronIngot_C");
  await edit(request, [{ type: "set_port_rate", id: ingotOut, rate: 48 }]);

  // 48/min = 1.6 smelters → the solver plans 2 machines @ 80%.
  const g = (await hydrate(request)).plan.groups[smelt];
  expect(g.count).toBe(2);
  expect(Math.abs(g.clock - 0.8)).toBeLessThan(1e-6);

  await bootMap(page, request);
  const pc = await pinCenter(page, "INGOT VISUAL WORKS");
  await page.mouse.click(pc.x, pc.y);
  await page.getByTestId("btn-open-factory").click();
  const graph = page.getByTestId("graph-root");
  await expect(graph).toBeVisible();
  await expect(graph).toContainText("SMELTER");
  await expect(graph).toContainText("×2");
  await expect(graph).toContainText("80%");
  await expect(graph).toContainText("Iron Ore");
  await expect(graph).toContainText("Iron Ingot");
  await shot(page, test.info(), "built-graph", "The hand-built chain as the graph renders it: ore IN port → SMELTER ×2 @ ↓80% → ingot OUT port at 48/min, belts with live flow.");

  await page.getByRole("button", { name: "WORLD MAP" }).click();
  await expect(page.getByTestId("map-root")).toBeVisible();
});

// ---------------------------------------------------------------------------
test("10 wizard factory: P → 25/min Iron Plate → review → accept → verify the real build", async ({ page, request }) => {
  await bootMap(page, request);

  await page.keyboard.press("p");
  await expect(page.getByTestId("wizard-modal")).toBeVisible();
  await shot(page, test.info(), "wizard-open", "The supply-chain wizard opened with the P hotkey (also the PLAN SUPPLY CHAIN button).");

  await page.getByTestId("wizard-item").fill("iron plate");
  await page.getByTestId("wizard-item-option").first().click();
  await page.fill('[data-testid="wizard-rate"]', "25");
  await shot(page, test.info(), "wizard-goal", "Goal configured: Iron Plate at 25/min.");

  await page.click('[data-testid="wizard-solve"]');
  const review = page.getByTestId("proposal-review");
  await expect(review).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId("goal-check")).toContainText("25");
  await expect(review).toContainText("Δ POWER");
  await shot(page, test.info(), "proposal-review", "The solved proposal: CREATE + CLAIM items with impacts, goal check 25/25 ✓, Δ POWER, machine count.");

  const factoriesBefore = Object.keys((await hydrate(request)).plan.factories).length;
  await page.getByTestId("btn-accept-proposal").click();
  await expect(review).toBeHidden();
  await expect
    .poll(async () => Object.keys((await hydrate(request)).plan.factories).length)
    .toBe(factoriesBefore + 1);

  // ---- verify the ACTUAL build the wizard materialized ----
  const h = await hydrate(request);
  const works = Object.values<{ id: string; name: string }>(h.plan.factories).find((x) =>
    x.name.includes("IRON PLATE"),
  );
  expect(works, "an IRON PLATE factory was created").toBeTruthy();
  const groups = Object.values<{
    factory: string;
    recipe: string;
    machine: string;
    count: number;
    clock: number;
  }>(h.plan.groups).filter((x) => x.factory === works!.id);
  const smelters = groups.find((x) => x.recipe === "Recipe_IngotIron_C");
  const ctors = groups.find((x) => x.recipe === "Recipe_IronPlate_C");
  expect(ctors, "plate constructor stage exists").toBeTruthy();
  // 25 plate/min = 1.25 constructors → 2 machines @ 62.5%
  expect(ctors!.count).toBe(2);
  expect(Math.abs(ctors!.clock - 0.625)).toBeLessThan(1e-6);
  const outPorts = Object.values<{ factory: string; direction: string; item: string; rate: number }>(
    h.plan.ports,
  ).filter((p) => p.factory === works!.id && p.direction === "out");
  expect(outPorts.some((p) => p.item === "Desc_IronPlate_C" && Math.abs(p.rate - 25) < 1e-6)).toBe(true);
  // Ingot sourcing is SURPLUS-FIRST: with an existing ingot surplus in the
  // empire (test 09's INGOT VISUAL WORKS) the wizard routes from it instead of
  // building smelters + claiming ore; on an empty empire it builds the smelter
  // stage. Either way the chain must be fed — assert whichever form it took.
  const ingotIn = Object.values<{ factory: string; direction: string; item: string }>(h.plan.ports).some(
    (p) => p.factory === works!.id && p.direction === "in" && p.item === "Desc_IronIngot_C",
  );
  const claims = Object.values<{ factory: string }>(h.plan.nodeClaims).filter((c) => c.factory === works!.id);
  expect(
    smelters !== undefined || ingotIn,
    "ingots are sourced: a smelter stage OR a surplus-fed ingot IN port",
  ).toBe(true);
  if (smelters) {
    expect(smelters.count).toBe(2);
    expect(Math.abs(smelters.clock - 0.625)).toBeLessThan(1e-6);
    expect(claims.length, "a smelter stage needs a claimed ore node").toBeGreaterThan(0);
  }

  await page.keyboard.press("f");
  await page.waitForTimeout(500);
  await expect(page.locator(".pin-wrap", { hasText: works!.name })).toBeVisible();
  await shot(page, test.info(), "map-after-accept", `Accept materialized ◇ ${works!.name} on the map, tethered to its claimed iron node.`);

  const pc = await pinCenter(page, works!.name);
  await page.mouse.click(pc.x, pc.y);
  await page.getByTestId("btn-open-factory").click();
  const graph = page.getByTestId("graph-root");
  await expect(graph).toBeVisible();
  await expect(graph).toContainText("CONSTRUCTOR");
  await expect(graph).toContainText("62.5%");
  await expect(graph).toContainText("Iron Plate");
  await shot(page, test.info(), "wizard-build-graph", "The wizard's actual build: ingots in (surplus-routed or smelted on site) → CONSTRUCTOR ×2 @ ↓62.5% → plate OUT at 25/min. Counts, clocks and belts as solved.");

  await page.getByRole("button", { name: "WORLD MAP" }).click();
  await expect(page.getByTestId("map-root")).toBeVisible();
});

// ---------------------------------------------------------------------------
test("11 belt route: right-drag between pins, pick the item, confirm, tier re-caps", async ({ page, request }) => {
  const src = await mkFactory(request, "ROD SOURCE", -1000, 2200);
  const rodOut = await mkPort(request, src, "out", "Desc_IronRod_C", null, 600);
  await edit(request, [{ type: "set_port_rate", id: rodOut, rate: 30 }]);
  const dst = await mkFactory(request, "ROD SINK", -450, 2200);
  await mkPort(request, dst, "in", "Desc_IronRod_C", null, 0);

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "ROD SOURCE"), await pinCenter(page, "ROD SINK"));
  const popover = page.getByTestId("route-popover");
  await expect(popover).toBeVisible();
  await expect(page.locator(".route-cand").first()).toContainText("Iron Rod");
  await shot(page, test.info(), "route-popover", "Right-drag ROD SOURCE → ROD SINK: the popover lists the matching item candidates and the transport pick (BELT suggested at this distance).");

  await page.selectOption('[data-testid="popover-transport"]', "belt");
  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("BELT ROUTE");
  await expect(drawer).toContainText("Iron Rod");
  await shot(page, test.info(), "belt-route-drawer", "The committed belt route selects itself: BELT ROUTE inspector with the item, load and tier.");

  await page.getByTestId("route-tier-select").selectOption("1");
  await expect(drawer).toContainText("60/min CAP");
  await shot(page, test.info(), "belt-tier-mk1", "Tier dropped to Mk.1 → capacity re-derives to 60/min CAP.");

  const h = await hydrate(request);
  const routes = Object.values<{ kind: { kind: string } }>(h.plan.routes).filter((r) => r.kind.kind === "belt");
  expect(routes.length).toBeGreaterThan(0);
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("12 route draft cancels: ESC mid-drag drops the ghost, no popover", async ({ page, request }) => {
  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  const a = await pinCenter(page, "ROD SOURCE");
  const b = await pinCenter(page, "ROD SINK");
  await page.mouse.move(a.x, a.y);
  await page.mouse.down({ button: "right" });
  await page.mouse.move((a.x + b.x) / 2, (a.y + b.y) / 2, { steps: 5 });
  await expect(page.locator(".map-placing-hint")).toContainText("RELEASE OVER A FACTORY");
  await shot(page, test.info(), "mid-drag-hint", "Mid right-drag: the placing hint and rubber-band ghost are up.");
  await page.keyboard.press("Escape");
  await expect(page.locator(".map-placing-hint")).toHaveCount(0);
  await page.mouse.up({ button: "right" });
  await expect(page.getByTestId("route-popover")).not.toBeVisible();
  await shot(page, test.info(), "drag-cancelled", "ESC cancelled the draft: no hint, no popover, nothing committed.");
});

// ---------------------------------------------------------------------------
test("13 pipe route: a water OUT→IN pair forces the PIPE medium", async ({ page, request }) => {
  const farm = await mkFactory(request, "WATER FARM VISUAL", -1000, 200);
  await mkPort(request, farm, "out", "Desc_Water_C", null, 600);
  const plant = await mkFactory(request, "WATER SINK VISUAL", 300, 200);
  await mkPort(request, plant, "in", "Desc_Water_C", null, 0);

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "WATER FARM VISUAL"), await pinCenter(page, "WATER SINK VISUAL"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await expect(page.locator(".route-cand").first()).toContainText("Water");
  await expect(page.getByTestId("popover-transport-fluid")).toContainText("PIPE");
  await expect(page.getByTestId("popover-transport")).toHaveCount(0);
  await shot(page, test.info(), "pipe-popover", "A fluid candidate: the popover pins the medium to PIPE (no belt/rail/truck/drone select) with pipe tiers only.");

  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("PIPE ROUTE");
  await page.getByTestId("route-tier-select").selectOption("2");
  await expect(drawer).toContainText("600/min CAP");
  await shot(page, test.info(), "pipe-route-drawer", "PIPE ROUTE inspector: Water on a pipe-blue line, Mk.2 → 600/min CAP.");
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("14 power line + priority switch: coal plant grid-links a consumer", async ({ page, request }) => {
  const plant = await mkFactory(request, "COAL POWER VISUAL", -1800, 1400);
  const coalIn = await mkPort(request, plant, "in", "Desc_Coal_C", 120, 0);
  const waterIn = await mkPort(request, plant, "in", "Desc_Water_C", 300, 0);
  const mwOut = await mkPort(request, plant, "out", "__PowerMW", null, 600);
  const gens = await mkGroup(request, plant, "Build_GeneratorCoal_C", "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C");
  await belt(request, plant, P(coalIn), G(gens), "Desc_Coal_C", 1);
  await belt(request, plant, P(waterIn), G(gens), "Desc_Water_C", 1);
  await belt(request, plant, G(gens), P(mwOut), "__PowerMW", 1);
  await edit(request, [{ type: "set_port_rate", id: mwOut, rate: 150 }]);

  const consumer = await mkFactory(request, "POWER SINK VISUAL", -900, 1400);
  await mkGroup(request, consumer, "Build_ConstructorMk1_C", "Recipe_IronRod_C");

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "COAL POWER VISUAL"), await pinCenter(page, "POWER SINK VISUAL"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await expect(page.locator(".route-cand")).toHaveCount(1);
  await expect(page.locator(".route-cand")).toContainText("Power line");
  await shot(page, test.info(), "power-popover", "The coal plant's only unbound OUT is the MW pseudo-item — the single candidate is a Power line.");

  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toBeVisible();
  await expect(drawer).toContainText("POWER LINE");
  await expect(drawer).toContainText("GRID");
  await shot(page, test.info(), "power-line-drawer", "POWER LINE inspector: grid membership and the 150 MW target on the plant.");

  await page.getByTestId("btn-add-switch").click();
  await expect(page.getByTestId("switch-drawer")).toBeVisible();
  await expect(page.getByTestId("switch-priority")).toBeVisible();
  await shot(page, test.info(), "priority-switch", "+ PRIORITY SWITCH dropped a square pin on the line; its drawer sets the shed priority.");
  await page.keyboard.press("Escape");

  await expect(page.getByTestId("sb-power")).toContainText("MW");
  await shot(page, test.info(), "statusbar-power", "The status bar shows empire draw vs generation for the grid.");
});

// ---------------------------------------------------------------------------
test("15 rail route: distance suggests RAIL and the train answer does the math", async ({ page, request }) => {
  const src = await mkFactory(request, "RAIL DEPOT WEST", -2600, -800);
  const out = await mkPort(request, src, "out", "Desc_IronScrew_C", null, 600);
  await edit(request, [{ type: "set_port_rate", id: out, rate: 300 }]);
  const dst = await mkFactory(request, "RAIL DEPOT EAST", 1400, -800);
  await mkPort(request, dst, "in", "Desc_IronScrew_C", null, 0);

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "RAIL DEPOT WEST"), await pinCenter(page, "RAIL DEPOT EAST"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await page.selectOption('[data-testid="popover-transport"]', "rail");
  const answer = page.getByTestId("train-answer");
  await expect(answer).toBeVisible();
  await expect(answer).toContainText("TRAINS NEEDED");
  await page.getByTestId("train-answer-demand").fill("500");
  await expect(page.getByTestId("train-answer-count")).toContainText(/\d+×/);
  await shot(page, test.info(), "train-answer-popover", "RAIL picked in the popover: the pre-build TRAIN ANSWER sizes consists for a 500/min demand before anything is committed.");

  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toContainText("RAIL ROUTE");
  const math = page.getByTestId("math-block");
  await expect(math).toBeVisible();
  await expect(math).toContainText("ROUND TRIP");
  await expect(math).toContainText("THROUGHPUT");
  await shot(page, test.info(), "rail-math-block", "RAIL ROUTE inspector: the math block (round trip, headway, RTT, throughput vs demand) is the product.");

  await page.getByTestId("btn-add-consist").click();
  await expect(page.getByTestId("consist-row")).toContainText("2×");
  await shot(page, test.info(), "rail-two-consists", "+1 consist → 2× trains, throughput doubles in the math block.");
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("16 truck route: the road option commits with its fuel spec", async ({ page, request }) => {
  const src = await mkFactory(request, "TRUCK STOP A", -2200, -1800);
  const out = await mkPort(request, src, "out", "Desc_IronPlate_C", null, 600);
  await edit(request, [{ type: "set_port_rate", id: out, rate: 60 }]);
  const dst = await mkFactory(request, "TRUCK STOP B", -600, -1800);
  await mkPort(request, dst, "in", "Desc_IronPlate_C", null, 0);

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "TRUCK STOP A"), await pinCenter(page, "TRUCK STOP B"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await page.selectOption('[data-testid="popover-transport"]', "truck");
  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toContainText("TRUCK ROUTE");
  await expect(drawer).toContainText("TRUCK");
  await shot(page, test.info(), "truck-route-drawer", "TRUCK ROUTE inspector: truck count and fuel item spec on the committed road link.");
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("17 drone route: the drone option commits with its battery spec", async ({ page, request }) => {
  const src = await mkFactory(request, "DRONE PORT A", 1800, -1800);
  const out = await mkPort(request, src, "out", "Desc_IronScrew_C", null, 600);
  await edit(request, [{ type: "set_port_rate", id: out, rate: 30 }]);
  const dst = await mkFactory(request, "DRONE PORT B", 2600, -900);
  await mkPort(request, dst, "in", "Desc_IronScrew_C", null, 0);

  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);

  await rightDrag(page, await pinCenter(page, "DRONE PORT A"), await pinCenter(page, "DRONE PORT B"));
  await expect(page.getByTestId("route-popover")).toBeVisible();
  await page.selectOption('[data-testid="popover-transport"]', "drone");
  await page.getByTestId("btn-route-confirm").click();
  const drawer = page.getByTestId("route-drawer");
  await expect(drawer).toContainText("DRONE ROUTE");
  await shot(page, test.info(), "drone-route-drawer", "DRONE ROUTE inspector: batteries-per-trip spec on the committed drone link.");
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("18 geyser: search-jump offers PLACE GEOTHERMAL, claiming stamps a generator factory", async ({ page, request }) => {
  await bootMap(page, request);
  const geyser = Object.values<{ id: string; nodeType: string }>((await hydrate(request)).world.nodes).find(
    (n) => n.nodeType === "geyser",
  );
  expect(geyser, "the catalog has a geyser").toBeTruthy();

  await page.locator(".searchbox input").fill(geyser!.id);
  await page.keyboard.press("Enter");
  const drawer = page.getByTestId("node-drawer");
  await expect(drawer).toBeVisible();
  await expect(page.getByTestId("btn-claim-geyser")).toContainText("GEOTHERMAL");
  await shot(page, test.info(), "geyser-drawer", "A geyser's drawer: purity → MW figure and PLACE GEOTHERMAL (no miner claim offered).");

  await Promise.all([
    page.waitForResponse((r) => r.url().includes("/api/edit") && r.request().method() === "POST"),
    page.getByTestId("btn-claim-geyser").click(),
  ]);
  const h = await hydrate(request);
  const gf = Object.values<{ id: string; name: string }>(h.plan.factories).find((f) => f.name.includes("GEYSER"));
  expect(gf, "a GEYSER factory was created").toBeTruthy();
  expect(
    Object.values<{ factory: string; machine: string }>(h.plan.groups).filter(
      (g) => g.factory === gf!.id && g.machine === "Build_GeneratorGeoThermal_C",
    ),
  ).toHaveLength(1);

  await page.locator(".searchbox input").fill(geyser!.id);
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("btn-goto-geyser")).toBeVisible();
  await shot(page, test.info(), "geyser-claimed", "Re-opening the same geyser: claimed state offers GO TO GENERATOR, never a second PLACE.");
});

// ---------------------------------------------------------------------------
test("19 fracking well: one satellite claims the whole well; the graph shows the build", async ({ page, request }) => {
  await bootMap(page, request);
  await page.locator(".searchbox input").fill("nitrogen");
  await page.keyboard.press("Enter");
  const drawer = page.getByTestId("node-drawer");
  await expect(drawer).toBeVisible();
  await expect(page.getByTestId("well-purity")).toBeVisible();
  await shot(page, test.info(), "well-drawer", "A fracking satellite's drawer: the well's per-satellite purity split and CLAIM WELL.");

  await Promise.all([
    page.waitForResponse((r) => r.url().includes("/api/edit") && r.request().method() === "POST"),
    page.getByTestId("btn-claim-well").click(),
  ]);

  const h = await hydrate(request);
  const wf = Object.values<{ id: string; name: string }>(h.plan.factories).find((f) => f.name.includes("NITROGEN"));
  expect(wf, "a NITROGEN well factory was created").toBeTruthy();
  const wellGroups = Object.values<{ factory: string; machine: string }>(h.plan.groups).filter(
    (g) => g.factory === wf!.id,
  );
  expect(wellGroups.some((g) => g.machine === "Build_FrackingSmasher_C")).toBe(true);
  expect(wellGroups.some((g) => g.machine === "Build_FrackingExtractor_C")).toBe(true);

  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible();
  await page.keyboard.press("f");
  await page.waitForTimeout(400);
  const pc = await pinCenter(page, wf!.name);
  await page.mouse.click(pc.x, pc.y);
  await page.getByTestId("btn-open-factory").click();
  const graph = page.getByTestId("graph-root");
  await expect(graph).toBeVisible();
  await expect(graph).toContainText("PRESSURIZER");
  await shot(page, test.info(), "well-build-graph", "The well factory's actual build: Resource Well Pressurizer + per-satellite fracking extractor groups wired to a fluid OUT port.");
  await page.getByRole("button", { name: "WORLD MAP" }).click();
});

// ---------------------------------------------------------------------------
test("20 overlays: FLOWS / POWER / NODES toggles and the TAB audit drawer", async ({ page, request }) => {
  await bootMap(page, request);
  await page.keyboard.press("f");
  await page.waitForTimeout(400);
  await shot(page, test.info(), "overlays-baseline", "The accumulated empire before overlay toggles: factories, routes, claims.");

  await page.keyboard.press("1");
  await page.waitForTimeout(400);
  await shot(page, test.info(), "overlay-flows", "FLOWS overlay (key 1): item flow bands ride the drawn routes.");
  await page.keyboard.press("1");

  await page.keyboard.press("2");
  await page.waitForTimeout(400);
  await shot(page, test.info(), "overlay-power", "POWER overlay (key 2): grid links and circuit accents.");
  await page.keyboard.press("2");

  await page.keyboard.press("3");
  await page.waitForTimeout(400);
  await shot(page, test.info(), "overlay-nodes", "NODES overlay (key 3): resource node visibility toggled.");
  await page.keyboard.press("3");

  await page.keyboard.press("Tab");
  await expect(page.getByTestId("audit-drawer")).toBeVisible();
  await shot(page, test.info(), "audit-drawer", "TAB opens the audit drawer: empire-wide saturation, deficits and circuit margins.");
  await page.keyboard.press("Escape");
});

// ---------------------------------------------------------------------------
test("21 resource overview: aggregate table, drill-down, collapse to rail", async ({ page, request }) => {
  await bootMap(page, request);
  const panel = page.getByTestId("resource-overview");
  await expect(panel).toBeVisible();
  await shot(page, test.info(), "overview-compact", "The resource overview panel aggregating empire-wide production/consumption.");

  await panel.getByTitle(/Full table \+ grids/i).click();
  await page.waitForTimeout(300);
  await shot(page, test.info(), "overview-full", "Expanded: the full table with per-grid power.");

  const row = panel.locator(".ro-rowbtn").first();
  if (await row.isVisible().catch(() => false)) {
    await row.click();
    await page.waitForTimeout(300);
    await shot(page, test.info(), "overview-drill", "Per-item drill-down: which factories produce/consume the item.");
  }

  // the step-down control is stateful: detailed → brief ("Back to brief
  // view"), brief → rail ("Collapse to rail")
  await panel.getByTitle(/Back to brief view/i).click();
  await panel.getByTitle(/Collapse to rail/i).click();
  await page.waitForTimeout(300);
  await shot(page, test.info(), "overview-rail", "Collapsed to the rail edge strip.");
  await panel.locator(".ro-rail").click();
});

// ---------------------------------------------------------------------------
test("22 DATA menu: start-new-empire needs a two-click confirm, then wipes the plan", async ({ page, request }) => {
  await bootMap(page, request);
  expect(Object.keys((await hydrate(request)).plan.factories).length).toBeGreaterThan(0);

  await page.getByTestId("btn-data-menu").click();
  const reset = page.getByTestId("btn-new-empire");
  await expect(reset).toBeVisible();
  await shot(page, test.info(), "data-menu", "The DATA menu with Start-new-empire (plus import/docs entries).");

  await reset.click();
  await expect(reset).toContainText(/Click again/i);
  await shot(page, test.info(), "wipe-armed", "First click arms the destructive action — 'Click again' confirm state.");

  await reset.click();
  await expect.poll(async () => Object.keys((await hydrate(request)).plan.factories).length, { timeout: 10_000 }).toBe(0);
  await page.goto("/");
  await expect(page.getByTestId("onboarding")).toBeVisible();
  await shot(page, test.info(), "wiped", "Confirmed: the whole plan is gone and the empty-plan onboarding returns.");
});
