// A Water Extractor is placeable in a factory like any machine. Water has no
// world node (it's drawn from any surface), so unlike node-bound miners/oil
// pumps it can't arrive via a map claim — a synthesized extraction recipe
// (gamedata) makes the pump appear in the recipe-driven add-machine picker.

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
  if (!res.ok()) throw new Error(`hydrate ${res.status()}`);
  return res.json();
}

test("a Water Extractor can be placed in a factory from the add-machine picker", async ({ page, request }) => {
  await resetView(request);
  // Clean plan first: a prior import spec can leave its own "WATER WORKS"
  // factory (imported Water Extractors now build one) — avoid a name collision.
  await request.post(`${API}/new_empire`, { data: "{}" });
  const f = (
    await edit(request, [
      { type: "create_factory", name: "WATER WORKS", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await page.locator(".searchbox input").fill("WATER WORKS");
    await page.keyboard.press("Enter");
    await page.getByTestId("btn-open-factory").click();

    // Open the add-machine picker and search for the water extractor.
    await page.getByTestId("btn-add-machine").click();
    const input = page.locator(".addgroup-menu input");
    await expect(input).toBeVisible();
    await input.fill("water");
    const option = page.locator(".addgroup-item").filter({ hasText: "Water Extractor" });
    await expect(option).toBeVisible();
    await option.first().click();
    await page.waitForTimeout(300);

    // The pump group was added with its synthesized extraction recipe.
    await expect(page.getByTestId("group-Recipe_Extract_Build_WaterPump")).toBeVisible();
    const h = await hydrate(request);
    const groups = Object.values<any>(h.plan.groups).filter(
      (g) => g.factory === f && g.machine === "Build_WaterPump_C",
    );
    expect(groups.length).toBe(1);
    expect(groups[0].recipe).toBe("Recipe_Extract_Build_WaterPump");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
