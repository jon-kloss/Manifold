// Fracking well PLACEMENT: a fracking satellite now renders on the map and its
// drawer is the WELL drawer — CLAIM WELL stamps one factory with the Pressurizer
// + one Extractor per satellite (per its purity) and a routable fluid OUT port.
// (The extraction math is covered by the Rust integration tests; this pins the
// map render + claim UI wiring.)

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

test("a fracking satellite claims the whole well from its drawer", async ({ page, request }) => {
  await resetView(request);
  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();

    // Search surfaces the (now-rendered) nitrogen satellites; selecting one opens
    // the WELL drawer, not the per-node miner claim.
    await page.locator(".searchbox input").fill("nitrogen");
    await page.keyboard.press("Enter");
    const drawer = page.getByTestId("node-drawer");
    await expect(drawer).toBeVisible();
    await expect(drawer.locator(".t-title")).toContainText("WELL");
    await expect(page.getByTestId("well-purity")).toBeVisible();

    await page.getByTestId("btn-claim-well").click();

    // The well imported as one factory: a Pressurizer + ≥1 fracking extractor
    // group, and a routable nitrogen OUT port.
    const h = await hydrate(request);
    const wellFactory = Object.values<any>(h.plan.factories).find((f) =>
      f.name.includes("NITROGEN"),
    );
    expect(wellFactory, "a NITROGEN … WELL factory was created").toBeTruthy();
    const groups = Object.values<any>(h.plan.groups).filter((g) => g.factory === wellFactory.id);
    expect(groups.some((g) => g.machine === "Build_FrackingSmasher_C")).toBe(true);
    expect(groups.some((g) => g.machine === "Build_FrackingExtractor_C")).toBe(true);
    expect(
      Object.values<any>(h.plan.ports).some(
        (p) => p.factory === wellFactory.id && p.item === "Desc_NitrogenGas_C" && p.direction === "out",
      ),
    ).toBe(true);

    // Clean up so the serial suite's shared plan is unchanged.
    await edit(request, [{ type: "delete_factory", id: wellFactory.id }]);
  } finally {
    await resetView(request);
  }
});
