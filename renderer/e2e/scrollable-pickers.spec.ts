// Searchable pickers must offer the WHOLE catalog in a bounded, scrollable
// list — a user who can't name an item scrolls to find it. Regression guard:
// the pickers used to hard-cap results with .slice(N), so the list was too
// short to ever overflow its scroll container (acute once a real Docs.json with
// hundreds of items replaced the small fixture).

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

test("the add-port item picker offers the full catalog in a scrollable list", async ({ page, request }) => {
  await resetView(request);
  const f = (
    await edit(request, [
      { type: "create_factory", name: "PICKER TEST", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();
    await page.locator(".searchbox input").fill("PICKER TEST");
    await page.keyboard.press("Enter");
    await page.getByTestId("btn-open-factory").click();

    // Open the IN-port item picker (empty query → whole item catalog).
    await page.getByRole("button", { name: "+ IN PORT" }).click();
    const menu = page.locator(".addgroup-menu");
    await expect(menu).toBeVisible();
    const options = menu.locator(".addgroup-item");

    // The old .slice(0, 10) cap is gone: the fixture catalog has more than 10
    // items and every one is offered.
    const count = await options.count();
    expect(count).toBeGreaterThan(10);

    // ...and the list is actually SCROLLABLE — content taller than the bounded
    // container (the whole point of "scroll to find it").
    const list = menu.locator(".addgroup-list");
    const { scroll, client } = await list.evaluate((el) => ({
      scroll: el.scrollHeight,
      client: el.clientHeight,
    }));
    expect(scroll).toBeGreaterThan(client);

    // A real scroll moves the viewport (an inert overflow wouldn't).
    await list.evaluate((el) => (el.scrollTop = el.scrollHeight));
    expect(await list.evaluate((el) => el.scrollTop)).toBeGreaterThan(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
