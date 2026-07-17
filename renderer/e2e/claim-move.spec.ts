// Regression: reassigning a claimed resource node from one factory to another
// via the NodeDrawer "MOVE TO…" control must MOVE the claim (and its boundary
// input port), not STACK a second claim — stacking trips the intentional
// double-book conflict, which is what a naive re-claim used to do.

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

test("claim MOVE reassigns to the other factory without double-booking", async ({ page, request }) => {
  await resetView(request);
  const alpha = (await edit(request, [{ type: "create_factory", name: "CLAIMMOVE ALPHA", position: { x: -2600, y: 2600 }, region: "GRASS FIELDS" }])).created[0];
  const beta = (await edit(request, [{ type: "create_factory", name: "CLAIMMOVE BETA", position: { x: -1000, y: 2600 }, region: "GRASS FIELDS" }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();

    // Open a node drawer via the searchbox.
    await page.locator(".searchbox input").fill("iron ore");
    await page.keyboard.press("Enter");
    const drawer = page.getByTestId("node-drawer");
    await expect(drawer).toBeVisible();

    // Claim the node for ALPHA (CLAIM FOR factory select = first, extractor = second).
    const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
    await claimFor.locator("select").first().selectOption({ label: "CLAIMMOVE ALPHA" });
    await claimFor.locator("select").nth(1).selectOption("Build_MinerMk2_C");
    await page.getByTestId("btn-claim").click();

    // One claim, owned by ALPHA, no conflict.
    const claimsSection = drawer.locator("section:has(h3:has-text('CLAIMS'))");
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);
    await expect(claimsSection.locator(".drawer-row-name")).toHaveText(/^CLAIMMOVE ALPHA/);
    await expect(drawer.locator(".drawer-warn")).toHaveCount(0);

    const before = await hydrate(request);
    const nodeId = Object.values<any>(before.plan.nodeClaims).find((c) => c.factory === alpha)!.node;
    expect(Object.values<any>(before.plan.ports).filter((p) => p.factory === alpha && p.direction === "in")).toHaveLength(1);

    // MOVE the claim to BETA.
    await page.getByTestId("claim-move").selectOption({ label: "CLAIMMOVE BETA" });

    // Still one claim — now owned by BETA — and still no conflict warning.
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);
    await expect(claimsSection.locator(".drawer-row-name")).toHaveText(/^CLAIMMOVE BETA/);
    await expect(drawer.locator(".drawer-warn")).toHaveCount(0);

    // State: exactly one claim on the node (BETA); the input port moved A→B.
    const after = await hydrate(request);
    const claims = Object.values<any>(after.plan.nodeClaims).filter((c) => c.node === nodeId);
    expect(claims).toHaveLength(1);
    expect(claims[0].factory).toBe(beta);
    expect(Object.values<any>(after.plan.ports).filter((p) => p.factory === alpha && p.direction === "in")).toHaveLength(0);
    expect(Object.values<any>(after.plan.ports).filter((p) => p.factory === beta && p.direction === "in")).toHaveLength(1);
  } finally {
    // Serial suite shares one plan file — remove our factories (cascades the
    // claim + ports) so later specs see a clean slate.
    await edit(request, [
      { type: "delete_factory", id: alpha },
      { type: "delete_factory", id: beta },
    ]).catch(() => {});
  }
});
