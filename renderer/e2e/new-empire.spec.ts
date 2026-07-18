// "Start new empire" over the dev-bridge transport (BridgeBackend → SQLite
// Session::new_empire): seed a factory, wipe it from the DATA menu's two-click
// confirm, and assert the plan is empty. Proves the cross-platform reset works
// on the non-web path (the web path has its own IndexedDB round-trip proof in
// e2e-web/web-smoke.spec.ts).

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
  return (await request.get(`${API}/hydrate`)).json();
}

test("DATA menu → Start new empire wipes the whole plan", async ({ page, request }) => {
  await resetView(request);
  await edit(request, [{ type: "create_factory", name: "DOOMED WORKS", position: { x: -2400, y: 2400 }, region: "GRASS FIELDS" }]);
  await edit(request, [{ type: "create_factory", name: "ALSO DOOMED", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" }]);
  // The dev-bridge shares one plan across the serial suite, so don't assume a
  // clean start — just that there's something to wipe (our two, plus any left
  // by earlier specs). new_empire clears ALL of it; the ==0 below is the check.
  expect(Object.keys((await hydrate(request)).plan.factories).length).toBeGreaterThanOrEqual(2);

  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
  await expect(page.getByTestId("map-root")).toBeVisible();

  await page.getByTestId("btn-data-menu").click();
  const reset = page.getByTestId("btn-new-empire");
  await expect(reset).toBeVisible();
  await reset.click(); // arms the two-click confirm
  await expect(reset).toContainText(/Click again/i);
  await reset.click(); // confirms → wipes

  await expect.poll(async () => Object.keys((await hydrate(request)).plan.factories).length, { timeout: 10_000 }).toBe(0);
});
