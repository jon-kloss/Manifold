// Regression (user screenshot): after EXPANDING a bank into individual
// machines behind real splitter/merger entities, the T1 solve concentrated
// all flow in one branch — siblings solved 0/min and their belts read
// "0/… · 0%" idle. The parallel-split fairness term makes identical branches
// share the demand like the real splitter does, so a fully-driven expanded
// chain has NO idle belt anywhere.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";

async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

test("expanded bank shares flow across every parallel branch", async ({ page, request }) => {
  await resetView(request);
  const factory = (
    await edit(request, [
      { type: "create_factory", name: "FANOUT WORKS", position: { x: -3000, y: 2600 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  const stoneIn = (
    await edit(request, [
      {
        type: "add_port",
        factory,
        direction: "in",
        item: "Desc_Stone_C",
        rate: 0,
        rateCeiling: 240,
        graphPos: { x: 0, y: 100 },
      },
    ])
  ).created[0];
  const bank = (
    await edit(request, [
      {
        type: "add_group",
        factory,
        machine: "Build_ConstructorMk1_C",
        recipe: "Recipe_Concrete_C",
        count: 2,
        clock: 1.0,
        graphPos: { x: 300, y: 100 },
        floor: 0,
      },
    ])
  ).created[0];
  const out = (
    await edit(request, [
      {
        type: "add_port",
        factory,
        direction: "out",
        item: "Desc_Cement_C",
        rate: 0,
        rateCeiling: null,
        graphPos: { x: 600, y: 100 },
      },
    ])
  ).created[0];
  await edit(request, [
    { type: "add_edge", factory, from: { kind: "port", id: stoneIn }, to: { kind: "group", id: bank }, item: "Desc_Stone_C", tier: 3 },
    { type: "add_edge", factory, from: { kind: "group", id: bank }, to: { kind: "port", id: out }, item: "Desc_Cement_C", tier: 3 },
    { type: "set_port_rate", id: out, rate: 30 },
    // The expand: ×2 bank → two ×1 machines behind a real splitter + merger.
    { type: "expand_group", id: bank },
  ]);

  await page.goto("/");
  await expect(page.getByTestId("map-root")).toBeVisible({ timeout: 15_000 });
  await page.keyboard.press("Escape");
  await page.locator(".searchbox input").fill("fanout works");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("summary-drawer")).toBeVisible();
  await page.getByTestId("btn-open-factory").click();
  await expect(page.getByTestId("graph-root")).toBeVisible();

  // The expanded chain: in → splitter → 2 machines → merger → out. Driven at
  // 30/min total, EVERY belt carries flow — an idle "0/…" label anywhere is
  // exactly the concentration bug.
  const labels = page.locator('[data-testid^="belt-label-"]');
  await expect(labels.nth(5)).toBeVisible(); // 6 belts once expanded
  const texts = await labels.allTextContents();
  expect(texts.length).toBeGreaterThanOrEqual(6);
  for (const t of texts) {
    expect(t, `belt reads idle: "${t}"`).not.toMatch(/^0\//);
  }
});
