// Factory pin dragging on the world map: a normal left-drag of a planned pin
// still commits move_factory_pin (regression guard for the Firefox stuck-drag
// fix — native-drag suppression, mid-drag mutation guard, window release net
// must not break the ordinary drag), and a plain map pan leaves the factory
// position untouched.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext): Promise<any> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}`);
  return res.json();
}

test("dragging a planned pin moves it; panning the map does not", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "DRAGPIN WORKS", position: { x: -1800, y: 1800 }, region: "GRASS FIELDS" }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await expect(page.getByTestId("map-root")).toBeVisible();

    // Find the pin via its label chip and drag it a fixed offset.
    const pin = page.locator(".pin-wrap", { hasText: "DRAGPIN WORKS" });
    await expect(pin).toBeVisible();
    const before = (await hydrate(request)).plan.factories[f].position;
    let b = (await pin.boundingBox())!;
    await page.mouse.move(b.x + b.width / 2, b.y + 8); // the pin head, not the chip
    await page.mouse.down();
    await page.mouse.move(b.x + b.width / 2 + 60, b.y + 8 + 40, { steps: 8 });
    await page.mouse.up();

    // The drag committed a new stored position.
    await expect
      .poll(async () => {
        const p = (await hydrate(request)).plan.factories[f].position;
        return Math.hypot(p.x - before.x, p.y - before.y);
      })
      .toBeGreaterThan(1);
    const afterDrag = (await hydrate(request)).plan.factories[f].position;

    // A plain map pan (drag on empty map, away from the pin) must NOT move it.
    const root = (await page.getByTestId("map-root").boundingBox())!;
    b = (await pin.boundingBox())!;
    // pick a pan start far from the pin
    const px = b.x > root.x + root.width / 2 ? root.x + 120 : root.x + root.width - 120;
    const py = root.y + root.height - 120;
    await page.mouse.move(px, py);
    await page.mouse.down();
    await page.mouse.move(px - 90, py - 60, { steps: 8 });
    await page.mouse.up();
    await page.waitForTimeout(300);

    const afterPan = (await hydrate(request)).plan.factories[f].position;
    expect(Math.hypot(afterPan.x - afterDrag.x, afterPan.y - afterDrag.y)).toBeLessThan(0.5);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
