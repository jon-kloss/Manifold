// BUILD SHEET exit criterion: the per-factory copy/print export bridging
// plan → game. Seeds a self-contained factory (machines + in/out ports + a
// belt route to a neighbour) through the same command surface the UI uses,
// opens its graph view, then asserts the BUILD SHEET panel renders the derived
// machines / inputs / outputs / routes / power spec with real numbers and that
// COPY places a faithful plain-text rendering on the clipboard. Read-only:
// nothing here mutates the plan.

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

test("build sheet: derived spec renders + clipboard copy", async ({ page, request, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);

  // ---- seed two factories through the bridge (same commands as the UI) ----
  const mk = async (name: string, x: number, y: number) =>
    (await edit(request, [{ type: "create_factory", name, position: { x, y }, region: "GRASS FIELDS" }]))
      .created[0];
  const port = async (factory: string, direction: string, item: string, rate: number, ceiling: number | null, x: number) =>
    (
      await edit(request, [
        { type: "add_port", factory, direction, item, rate, rateCeiling: ceiling, graphPos: { x, y: 100 } },
      ])
    ).created[0];

  // Distinctive names so the search box can't collide with an imported factory
  // in the shared serial plan (this spec runs last).
  const rodWorks = await mk("BSHEET ROD DEPOT", -3200, 1800);
  const screwWorks = await mk("BSHEET SCREW DEPOT", -1800, 1800);

  // ROD WORKS: 4× Constructor @ 100% making Iron Rod, fed Iron Ingot.
  const rodGroup = (
    await edit(request, [
      {
        type: "add_group",
        factory: rodWorks,
        machine: "Build_ConstructorMk1_C",
        recipe: "Recipe_IronRod_C",
        count: 4,
        clock: 1.0,
        graphPos: { x: 300, y: 100 },
        floor: 0,
      },
    ])
  ).created[0];
  const ingotIn = await port(rodWorks, "in", "Desc_IronIngot_C", 0, 120, 0);
  const rodOut = await port(rodWorks, "out", "Desc_IronRod_C", 60, null, 600);
  const screwIn = await port(screwWorks, "in", "Desc_IronRod_C", 0, 120, 0);

  // Wire the machine internally so the solver clocks it to meet the target
  // (port → group → port), same as the graph UI does.
  await edit(request, [
    { type: "add_edge", factory: rodWorks, from: { kind: "port", id: ingotIn }, to: { kind: "group", id: rodGroup }, item: "Desc_IronIngot_C", tier: 2 },
    { type: "add_edge", factory: rodWorks, from: { kind: "group", id: rodGroup }, to: { kind: "port", id: rodOut }, item: "Desc_IronRod_C", tier: 2 },
  ]);

  // A belt route ROD WORKS → SCREW WORKS (item Iron Rod, Mk.2).
  await edit(request, [
    {
      type: "add_route",
      kind: { kind: "belt", tier: 2 },
      from: rodOut,
      to: screwIn,
      path: [
        { x: -3200, y: 1800, z: 0 },
        { x: -1800, y: 1800, z: 0 },
      ],
    },
  ]);

  // ---- open ROD WORKS' graph view ----
  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 10_000 });
  // Settle to the map: dismiss any auto-presented resume dashboard / onboarding
  // and exit any factory view a prior serial spec left open. The dashboard can
  // mount a beat after hydrate, so poll it away before interacting.
  await expect(async () => {
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("dashboard")).toBeHidden({ timeout: 1000 });
  }).toPass({ timeout: 10_000 });
  await page.locator(".searchbox input").fill("bsheet rod depot");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();

  // Group card carries the machine footprint derived from the catalog's
  // clearance data (fixture Constructor: 8 × 10 m), not a placeholder.
  await expect(page.getByTestId("group-Recipe_IronRod_C")).toContainText(
    /\d+(\.\d+)? × \d+(\.\d+)? m/,
  );
  // …and its strip tooltip names the provenance: the fixture Constructor's
  // dims come from the game's own clearance data, not the community table.
  await expect(page.getByTestId("group-Recipe_IronRod_C").locator(".fp-strip")).toHaveAttribute(
    "title",
    /game clearance data/,
  );

  // ---- open the BUILD SHEET ----
  await page.getByTestId("btn-build-sheet").click();
  const sheet = page.getByTestId("build-sheet");
  await expect(sheet).toBeVisible();

  // MACHINES: 4× Constructor @ 100% — Iron Rod
  const machines = page.getByTestId("bs-machines");
  await expect(machines).toContainText("4×");
  await expect(machines).toContainText("Constructor");
  await expect(machines).toContainText("100%");
  await expect(machines).toContainText("Iron Rod");

  // INPUTS / OUTPUTS carry real derived rates + honest source labels.
  await expect(page.getByTestId("bs-inputs")).toContainText("Iron Ingot");
  const outputs = page.getByTestId("bs-outputs");
  await expect(outputs).toContainText("Iron Rod");
  await expect(outputs).toContainText("60");

  // ROUTES: the belt to SCREW WORKS with its tier.
  const routes = page.getByTestId("bs-routes");
  await expect(routes).toContainText("BSHEET SCREW DEPOT");
  await expect(routes).toContainText("Belt Mk.2");

  // POWER: non-zero MW at planned clocks.
  await expect(page.getByTestId("bs-power")).toContainText("MW");

  // ---- COPY → faithful plain-text on the clipboard ----
  await page.getByTestId("btn-build-sheet-copy").click();
  await expect(page.getByTestId("btn-build-sheet-copy")).toContainText("COPIED");
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  expect(clip).toContain("BSHEET ROD DEPOT");
  expect(clip).toContain("4× Constructor");
  // Per-machine footprint rides the machine stage line, labeled for what it
  // is — the clearance pad (build + approach), not wall-to-wall dims (Docs
  // clearance data: Constructor 8 × 10 m).
  expect(clip).toContain("· 8 × 10 m clearance each");
  expect(clip).toContain("MACHINES");
  expect(clip).toContain("BSHEET SCREW DEPOT");
});
