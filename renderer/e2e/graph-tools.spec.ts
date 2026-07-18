// Factory-graph Pan/Select tool toggle: PAN (default) left-drags the view;
// SELECT left-drags a marquee. Toggle via the toolbar buttons and the V/H
// hotkeys.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function openGraph(page: any, name: string) {
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
  await expect(page.locator(".react-flow__pane")).toBeVisible();
  await page.waitForTimeout(300);
}
const viewportTransform = (page: any) =>
  page.locator(".react-flow__viewport").evaluate((el: HTMLElement) => getComputedStyle(el).transform);

test("select tool marquees, pan tool drags the view, hotkeys toggle", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "TOOLS WORKS", position: { x: -1800, y: 1800 }, region: "GRASS FIELDS" }])).created[0];
  const a = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1, graphPos: { x: 200, y: 120 }, floor: 0 }])).created[0];
  const b = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_Screw_C", count: 1, clock: 1, graphPos: { x: 560, y: 120 }, floor: 0 }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await openGraph(page, "TOOLS WORKS");

    // Pan is the default tool.
    await expect(page.getByTestId("graph-tool-pan")).toHaveAttribute("aria-pressed", "true");

    // SELECT (hotkey V): a left-drag marquees the machines it covers. Nodes are
    // fit-viewed near center, clear of the top-left toolbar / bottom overlays.
    await page.keyboard.press("v");
    await expect(page.getByTestId("graph-tool-select")).toHaveAttribute("aria-pressed", "true");
    const canvas = (await page.locator(".graph-canvas").boundingBox())!;
    const ba = (await page.locator(`.react-flow__node[data-id="${a}"]`).boundingBox())!;
    const bb = (await page.locator(`.react-flow__node[data-id="${b}"]`).boundingBox())!;
    const sx = Math.max(canvas.x + 60, Math.min(ba.x, bb.x) - 30);
    const sy = Math.max(canvas.y + 80, Math.min(ba.y, bb.y) - 30);
    const ex = Math.min(canvas.x + canvas.width - 60, Math.max(ba.x + ba.width, bb.x + bb.width) + 30);
    const ey = Math.min(canvas.y + canvas.height - 60, Math.max(ba.y + ba.height, bb.y + bb.height) + 30);
    await page.mouse.move(sx, sy);
    await page.mouse.down();
    await page.mouse.move((sx + ex) / 2, (sy + ey) / 2, { steps: 6 });
    await page.mouse.move(ex, ey, { steps: 6 });
    await page.mouse.up();
    await expect(page.locator(".react-flow__node.selected")).toHaveCount(2);

    // Back to PAN (hotkey H) and clear the marquee.
    await page.keyboard.press("Escape");
    await page.keyboard.press("h");
    await expect(page.getByTestId("graph-tool-pan")).toHaveAttribute("aria-pressed", "true");

    // A left-drag on empty pane — to the RIGHT of the rightmost machine card, at
    // the row's vertical center — now pans the view and selects nothing. (Right
    // of the cards is clear of the top-left toolbar, bottom-left minimap and
    // bottom-center recipe strip.)
    const before = await viewportTransform(page);
    const rightEdge = Math.max(ba.x + ba.width, bb.x + bb.width);
    const rowMidY = (Math.min(ba.y, bb.y) + Math.max(ba.y + ba.height, bb.y + bb.height)) / 2;
    const px = (rightEdge + canvas.x + canvas.width) / 2, py = rowMidY;
    await page.mouse.move(px, py);
    await page.mouse.down();
    await page.mouse.move(px - 140, py - 40, { steps: 6 });
    await page.mouse.up();
    expect(await viewportTransform(page)).not.toEqual(before);
    await expect(page.locator(".react-flow__node.selected")).toHaveCount(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
