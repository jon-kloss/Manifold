// MAKE FROM RESOURCES: (1) a built chain targets the requested rate, so ports
// show real throughput (regression: they were all 0/min); (2) a build that
// exceeds the claimed nodes' extraction is blocked with a warning.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}

test("MAKE targets the requested rate and blocks over-capacity builds", async ({ page, request }) => {
  await resetView(request);
  // Factory fed by 30/min of iron ingot (a capped input, like a node claim).
  const f = (await edit(request, [{ type: "create_factory", name: "MAKE TEST", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" }])).created[0];
  await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 30, graphPos: { x: 0, y: 100 } }]);

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();

    // open the factory graph
    await page.locator(".searchbox input").fill("MAKE TEST");
    await page.keyboard.press("Enter");
    await page.getByTestId("btn-open-factory").click();
    await page.getByTestId("btn-make-from-resources").click();
    const modal = page.getByTestId("make-from-resources");
    await expect(modal).toBeVisible();

    // ---- fitting build: 15/min iron rod (needs 15 ingot ≤ 30) ----
    await modal.getByTestId("mfr-item-Desc_IronRod_C").click();
    await modal.getByTestId("mfr-rate").fill("15");
    await expect(modal.getByTestId("mfr-warn")).toHaveCount(0);
    await modal.getByTestId("mfr-build").click();
    await expect(modal).toBeHidden();

    // the output port now carries the target rate, not 0/min
    const outPort = page.getByTestId("port-out-Desc_IronRod_C");
    await expect(outPort).toContainText("15");
    await expect(outPort).not.toContainText("0/min");

    // ---- over-capacity build: 60/min iron rod (needs 60 ingot > 30) ----
    await page.getByTestId("btn-make-from-resources").click();
    await modal.getByTestId("mfr-item-Desc_IronRod_C").click();
    await modal.getByTestId("mfr-rate").fill("60");
    await expect(modal.getByTestId("mfr-warn")).toBeVisible();
    await expect(modal.getByTestId("mfr-warn")).toContainText(/extraction/i);
    await expect(modal.getByTestId("mfr-build")).toBeDisabled();
    // and it offers to build at the feasible rate
    await expect(modal.getByTestId("mfr-reduce")).toBeVisible();
    await page.screenshot({ path: "/tmp/claude-0/-home-user-Conveyancer/c3647552-1a44-57b2-a23d-6138e2daabbf/scratchpad/make-blocked.png" });
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
