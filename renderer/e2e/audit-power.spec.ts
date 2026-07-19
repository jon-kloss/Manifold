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
  } finally {
    await edit(request, [{ type: "delete_factory", id: gf }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: ls }]).catch(() => {});
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
