// Typing in the map search live-filters the resource nodes drawn on the canvas
// by resource type. Nodes render to canvas (not the DOM), so the map-root
// carries a `data-nodes-shown` count stamp we can assert against.

import { test, expect } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

test("typing in search live-filters map nodes by resource type", async ({ page, request }) => {
  await resetView(request);
  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();

  const root = page.getByTestId("map-root");
  await expect(root).toBeVisible();
  const shown = async () => Number(await root.getAttribute("data-nodes-shown"));

  const all = await shown();
  expect(all).toBeGreaterThan(50); // the bundled world has hundreds of nodes

  // Filter to iron: strictly fewer nodes, but not zero (iron nodes exist).
  await page.locator(".searchbox input").fill("iron");
  await expect
    .poll(shown, { message: "iron filter narrows the node field" })
    .toBeLessThan(all);
  expect(await shown()).toBeGreaterThan(0);

  const iron = await shown();

  // A rarer resource narrows further still (uranium ⊂ iron count is not
  // guaranteed, so just assert it too filters to a non-empty subset < all).
  await page.locator(".searchbox input").fill("uranium");
  await expect.poll(shown).toBeLessThan(all);

  // Clearing restores the full field.
  await page.locator(".searchbox input").fill("");
  await expect.poll(shown).toBe(all);

  expect(iron).toBeLessThan(all);
});
