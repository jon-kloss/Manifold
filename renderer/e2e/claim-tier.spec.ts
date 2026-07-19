// Regression: changing a claim's miner tier (Mk1→Mk2) from the NodeDrawer must
// EDIT the existing claim in place (set_claim) + bump its input port ceiling —
// not stack a second claim (which trips the double-book conflict). Also proves
// CLAIM FOR is idempotent per factory (offers UPDATE, never a duplicate claim).

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

test("claim TIER change edits in place without double-booking", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "CLAIMTIER WORKS", position: { x: -2600, y: 2400 }, region: "GRASS FIELDS" }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();

    await page.locator(".searchbox input").fill("iron ore");
    await page.keyboard.press("Enter");
    const drawer = page.getByTestId("node-drawer");
    await expect(drawer).toBeVisible();

    // Claim for our factory with a Mk.1 miner.
    const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
    await claimFor.locator("select").first().selectOption({ label: "CLAIMTIER WORKS" });
    await claimFor.locator("select").nth(1).selectOption("Build_MinerMk1_C");
    await page.getByTestId("btn-claim").click();

    const claimsSection = drawer.locator("section:has(h3:has-text('CLAIMS'))");
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);

    const before = await hydrate(request);
    const claim0 = Object.values<any>(before.plan.nodeClaims).find((c) => c.factory === f)!;
    expect(claim0.extractor).toBe("Build_MinerMk1_C");
    const ceil0 = Object.values<any>(before.plan.ports).find((p) => p.factory === f && p.direction === "in")!.rateCeiling as number;

    // Change tier Mk.1 → Mk.2 via the per-claim dropdown.
    await page.getByTestId("claim-tier").selectOption("Build_MinerMk2_C");

    // Still exactly one claim (no double-book), extractor updated, no conflict.
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);
    await expect(drawer.locator(".drawer-warn")).toHaveCount(0);

    const after = await hydrate(request);
    const claims = Object.values<any>(after.plan.nodeClaims).filter((c) => c.factory === f);
    expect(claims).toHaveLength(1);
    expect(claims[0].extractor).toBe("Build_MinerMk2_C");
    // One input port still, ceiling bumped to the Mk.2 rate (2× the Mk.1 base).
    const inPorts = Object.values<any>(after.plan.ports).filter((p) => p.factory === f && p.direction === "in");
    expect(inPorts).toHaveLength(1);
    expect(inPorts[0].rateCeiling).toBeGreaterThan(ceil0 + 0.5);

    // CLAIM FOR is idempotent: this factory already claims the node, so the
    // action becomes UPDATE (the CLAIM-FOR extractor still reads Mk.1 ≠ Mk.2),
    // never a second claim.
    await expect(page.getByTestId("btn-claim-update")).toBeVisible();
    await expect(page.getByTestId("btn-claim")).toHaveCount(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

test("oil node offers the Oil Extractor — never a miner (fluid extraction)", async ({ page, request }) => {
  await resetView(request);
  const f = (
    await edit(request, [
      { type: "create_factory", name: "OIL WORKS", position: { x: 900, y: -1400 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  await page.goto("/");
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();

  await page.locator(".searchbox input").fill("crude oil");
  await page.keyboard.press("Enter");
  const drawer = page.getByTestId("node-drawer");
  await expect(drawer).toBeVisible();

  // The extractor picker is item-aware: an oil node lists ONLY the pump.
  const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
  await expect(claimFor.locator("select").nth(1).locator("option")).toHaveText(["Oil Extractor"]);

  await claimFor.locator("select").first().selectOption({ label: "OIL WORKS" });
  await page.getByTestId("btn-claim").click();

  const after = await hydrate(request);
  const claim = Object.values<any>(after.plan.nodeClaims).find((c) => c.factory === f)!;
  expect(claim.extractor).toBe("Build_OilPump_C");
  // Port ceiling = the pump's purity-scaled rate (120/min normal base).
  const ceil = Object.values<any>(after.plan.ports).find((p) => p.factory === f && p.direction === "in")!
    .rateCeiling as number;
  expect([60, 120, 240]).toContain(ceil);
  // The per-claim picker offers no miner either.
  await expect(page.getByTestId("claim-tier").locator("option")).toHaveText(["Oil Extractor"]);
});
