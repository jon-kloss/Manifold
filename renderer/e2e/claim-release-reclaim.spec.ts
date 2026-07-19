// #120 (playtest: 3 coal nodes released + re-added → 6 ports): the claim ⇄
// port lifecycle must round-trip. RELEASE deletes its port when unwired; a
// WIRED port survives (belts kept) and the next claim REUSES it instead of
// stacking a duplicate.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

test.describe.configure({ mode: "serial" });

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext) {
  return (await request.get(`${API}/hydrate`)).json();
}
const inPortsOf = (h: { plan: { ports: object } }, f: string, item: string) =>
  Object.values(h.plan.ports as Record<string, { factory: string; direction: string; item: string; id: string }>).filter(
    (p) => p.factory === f && p.direction === "in" && p.item === item,
  );

test("release ⇄ re-claim round-trips: wired port reused, unwired port deleted", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "RECLAIM WORKS", position: { x: -2700, y: 2500 }, region: "GRASS FIELDS" }])).created[0];

  try {
    await page.goto("/");
    const skip = page.getByTestId("onboard-skip");
    if (await skip.isVisible().catch(() => false)) await skip.click();

    // ---- claim a coal node via the drawer ----
    await page.locator(".searchbox input").fill("coal");
    await page.keyboard.press("Enter");
    const drawer = page.getByTestId("node-drawer");
    await expect(drawer).toBeVisible();
    const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
    await claimFor.locator("select").first().selectOption({ label: "RECLAIM WORKS" });
    await page.getByTestId("btn-claim").click();
    const claimsSection = drawer.locator("section:has(h3:has-text('CLAIMS'))");
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);

    let h = await hydrate(request);
    expect(inPortsOf(h, f, "Desc_Coal_C")).toHaveLength(1);
    const portId = inPortsOf(h, f, "Desc_Coal_C")[0].id;

    // ---- WIRE the port (a generator eats the coal) ----
    const gid = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_GeneratorCoal_C", recipe: "Recipe_Power_Build_GeneratorCoal_Desc_Coal_C", count: 1, clock: 1, graphPos: { x: 300, y: 80 }, floor: 0 }])).created[0];
    await edit(request, [{ type: "add_edge", factory: f, from: { kind: "port", id: portId }, to: { kind: "group", id: gid }, item: "Desc_Coal_C", tier: 1 }]);

    // API edits don't stream to the open client — reload so the store sees
    // the belt (the drawer's wired-check reads client state), then reselect.
    const reopenDrawer = async () => {
      await page.goto("/");
      await expect(page.getByTestId("map-root")).toBeVisible();
      await page.locator(".searchbox input").fill("coal");
      await page.keyboard.press("Enter");
      await expect(drawer).toBeVisible();
    };
    await reopenDrawer();

    // ---- RELEASE: wired port must SURVIVE (belts kept), claim gone ----
    await claimsSection.locator("button:has-text('RELEASE')").click();
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(0);
    h = await hydrate(request);
    expect(Object.values(h.plan.nodeClaims as object)).toHaveLength(0);
    expect(inPortsOf(h, f, "Desc_Coal_C")).toHaveLength(1); // orphan, still wired
    expect(Object.values(h.plan.edges as object)).toHaveLength(1);

    // ---- RE-CLAIM: must REUSE the orphan, not add a second port ----
    await page.getByTestId("btn-claim").click();
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);
    h = await hydrate(request);
    const ports = inPortsOf(h, f, "Desc_Coal_C");
    expect(ports).toHaveLength(1); // regression: this was 2 (then 6 with 3 nodes)
    expect(ports[0].id).toBe(portId); // the SAME port — its belts relight
    expect(Object.values(h.plan.edges as object)).toHaveLength(1);

    // ---- unwired round-trip: release deletes the port outright ----
    await edit(request, [{ type: "delete_edge", id: Object.keys(h.plan.edges as object)[0] }, { type: "delete_group", id: gid }]);
    await reopenDrawer();
    await claimsSection.locator("button:has-text('RELEASE')").click();
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(0);
    h = await hydrate(request);
    expect(inPortsOf(h, f, "Desc_Coal_C")).toHaveLength(0); // clean slate
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
