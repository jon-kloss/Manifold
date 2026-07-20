// #110 — read-only mobile companion dashboard. Under the 640px phone
// breakpoint the app swaps the whole editing shell (map, graph, titlebar,
// status bar) for a full-screen glanceable status board: power balance,
// alerts, and the resource make/use/net ledger with per-item factory
// drill-down. Desktop viewports are untouched — every other spec in this
// suite runs at the config default 1920×1080 (reference mode), far above the
// 640px phone gate, and never sees it.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

// Phone viewport for this file only (iPhone 14-ish logical size).
test.use({ viewport: { width: 390, height: 844 } });

test("phone breakpoint swaps the shell for the read-only status dashboard", async ({ page, request }) => {
  await resetView(request);
  // A tiny solved line so power + ledger have real figures: capped ingot feed
  // → rod constructor → 30/min rod export.
  const f = (await edit(request, [{ type: "create_factory", name: "PHONE WORKS", position: { x: -2000, y: 2400 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 30, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 2, clock: 1, graphPos: { x: 300, y: 100 }, floor: 0 }])).created[0];
  const out = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 600, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: ingot }, to: { kind: "group", id: rod }, item: "Desc_IronIngot_C", tier: 3 }]);
  await edit(request, [{ type: "add_edge", factory: f, from: { kind: "group", id: rod }, to: { kind: "port", id: out }, item: "Desc_IronRod_C", tier: 3 }]);
  await edit(request, [{ type: "set_port_rate", id: out, rate: 30 }]);

  try {
    await page.goto("/");
    const dash = page.getByTestId("mobile-dashboard");
    await expect(dash).toBeVisible({ timeout: 15_000 });

    // The editing shell is GONE, not shrunk: no map, no titlebar, no status bar.
    await expect(page.getByTestId("map-root")).toHaveCount(0);
    await expect(page.locator(".titlebar")).toHaveCount(0);
    await expect(page.getByTestId("sb-resume")).toHaveCount(0);

    // ...and the surface says so.
    await expect(dash).toContainText("READ-ONLY");
    await expect(dash).toContainText("BEST ON DESKTOP");

    // Power card carries the real empire figures (the rod line draws MW).
    const power = page.getByTestId("md-power");
    await expect(power).toBeVisible();
    await expect(power).toContainText("DRAW");

    // Ledger: the seeded line shows up; tapping a row opens the factory
    // drill-down naming PHONE WORKS.
    const rodRow = page.getByTestId("md-ledger-row").filter({ hasText: "Iron Rod" });
    await expect(rodRow).toBeVisible();
    await rodRow.click();
    const drill = page.getByTestId("md-drill");
    await expect(drill).toBeVisible();
    await expect(drill).toContainText("PHONE WORKS");
    // The ingot feed is raw boundary supply — tagged RAW in the ledger.
    await expect(
      page.getByTestId("md-ledger-row").filter({ hasText: "Iron Ingot" }),
    ).toContainText("RAW");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
