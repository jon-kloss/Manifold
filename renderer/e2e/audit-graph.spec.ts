// Audit #127 acceptance (promoted from the audit probe suite): factory-graph
// send-out port sizing, cross-floor lift rendering, and floor filtering.
// Every probe declares its EXPECTED (correct) result in the header BEFORE any
// assertion. Seeded through the same command surface the UI uses, against the
// dev bridge's default fixture catalog (Recipe_IronRod_C = 1 ingot -> 1 rod
// @ 4s = 15/min nameplate at clock 1).
//
// Fixed by audit #127:
//   1. send-out sizes the OUT port from the PLANNED clock (no 100% floor), so
//      an underclocked machine no longer over-promises its exports;
//   2. a boundary port is floor-agnostic — a group↔port belt is never a
//      cross-floor lift and never splits into a phantom portal under the
//      floor filter.

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
  if (!res.ok()) throw new Error(`hydrate ${res.status()}: ${await res.text()}`);
  return res.json();
}
// Open the factory graph from the map. API seeds do not stream to an open
// client, so this always runs AFTER page.goto once the plan is fully seeded.
async function openGraph(page: any, name: string): Promise<void> {
  await page.locator(".searchbox input").fill(name);
  await page.keyboard.press("Enter");
  await page.getByTestId("btn-open-factory").click();
  await expect(page.locator(".react-flow__pane")).toBeVisible();
  await page.waitForTimeout(300);
}
async function dismissOnboarding(page: any): Promise<void> {
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
}
const P = (id: string) => ({ kind: "port", id });
const G = (id: string) => ({ kind: "group", id });

// ---------------------------------------------------------------------------
// PROBE 1 — Send-out of an underclocked machine sizes the OUT port to real
// capacity, not nameplate.
//
// EXPECTED: exactly one OUT port for Desc_IronRod_C, and its rate == 7.5
// (nameplate 15/min x 0.5 clock) — not the 15 the old Math.max(effClock, 1)
// floor produced.
// ---------------------------------------------------------------------------
test("send-out sizes the OUT port to the underclocked capacity, not nameplate", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "SURPLUS CLOCK", position: { x: -1000, y: 1000 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const rod = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 0.5, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(ingot), to: G(rod), item: "Desc_IronIngot_C", tier: 3 }]);

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await openGraph(page, "SURPLUS CLOCK");

    await page.locator(`.react-flow__node[data-id="${rod}"]`).click({ button: "right" });
    await expect(page.getByTestId("graph-ctx-menu")).toBeVisible();
    await page.getByTestId("ctx-send-Desc_IronRod_C").click();
    await expect(page.getByTestId("port-out-Desc_IronRod_C")).toBeVisible();

    const h = await hydrate(request);
    const outs = Object.values<any>(h.plan.ports).filter(
      (p) => p.factory === f && p.direction === "out" && p.item === "Desc_IronRod_C",
    );
    // exactly one OUT port, sized to the REAL clocked capacity (7.5), not the
    // nameplate 15 an effClock>=1 floor would produce.
    expect(outs).toHaveLength(1);
    expect(outs[0].rate).toBeCloseTo(7.5, 3);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 2 — A group->boundary-port belt from a raised floor is NOT drawn as a
// lift (all-floors view is default).
//
// EXPECTED: the belt-label for the g->outp edge shows a plain 'n/cap · % MK.3'
// chip with NO '⇅' glyph and NO 'F2→F0' lift tag, and there are 0 lift-pad
// diamonds on it (a port is a floor-agnostic boundary, so this is not a
// cross-floor lift).
// ---------------------------------------------------------------------------
test("group->port belt from a raised floor is not drawn as a lift", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "PORT LIFT", position: { x: -1000, y: 1200 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const g = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  const outp = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 680, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(ingot), to: G(g), item: "Desc_IronIngot_C", tier: 3 }]);
  const E = (await edit(request, [{ type: "add_edge", factory: f, from: G(g), to: P(outp), item: "Desc_IronRod_C", tier: 3 }])).created[0];
  await edit(request, [{ type: "set_group_floor", id: g, floor: 2 }]);

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await openGraph(page, "PORT LIFT");

    // The belt-label chip exists (whole belt drawn, not a portal stub).
    const label = page.getByTestId(`belt-label-${E}`);
    await expect(label).toBeVisible();
    // A port is a floor-agnostic boundary: no lift glyph, no cross-floor tag.
    await expect(label).not.toContainText("⇅");
    await expect(label).not.toContainText("F2→F0");
    // ...and no lift-pad diamonds are drawn on this edge.
    await expect(page.locator(`.react-flow__edge[data-id="${E}"] .lift-pad`)).toHaveCount(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 3 — Filtering to the source floor keeps a group->port belt whole (no
// phantom lift portal). Same fixture as probe 2 (group g on floor 2, out-port
// outp, edge E), but with the F2 floor chip active.
//
// EXPECTED: 0 lift-portal elements for edge E; node g (floor 2) is visible and
// its OUT port outp is visible with the belt drawn between them (the
// belt-label chip present).
// ---------------------------------------------------------------------------
test("filtering to the source floor keeps a group->port belt whole", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "PORT LIFT F2", position: { x: -1000, y: 1400 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const g = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  const outp = (await edit(request, [{ type: "add_port", factory: f, direction: "out", item: "Desc_IronRod_C", rate: 0, rateCeiling: null, graphPos: { x: 680, y: 100 } }])).created[0];
  await edit(request, [{ type: "add_edge", factory: f, from: P(ingot), to: G(g), item: "Desc_IronIngot_C", tier: 3 }]);
  const E = (await edit(request, [{ type: "add_edge", factory: f, from: G(g), to: P(outp), item: "Desc_IronRod_C", tier: 3 }])).created[0];
  await edit(request, [{ type: "set_group_floor", id: g, floor: 2 }]);

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await openGraph(page, "PORT LIFT F2");

    // Filter to the source floor (F2). floors = [0, 2] → chips ALL, F0, F2.
    await page.getByTestId("floor-chips").getByRole("button", { name: "F2", exact: true }).click();
    await page.waitForTimeout(200);

    // No phantom lift portal for a group->port belt: the port is on-floor
    // regardless of the filter, so the belt stays whole.
    await expect(page.getByTestId(`lift-portal-${E}`)).toHaveCount(0);
    // The floor-2 group and its OUT port both render, with the belt between them.
    await expect(page.locator(`.react-flow__node[data-id="${g}"]`)).toBeVisible();
    await expect(page.locator(`.react-flow__node[data-id="${outp}"]`)).toBeVisible();
    await expect(page.getByTestId(`belt-label-${E}`)).toBeVisible();
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 4 — A junction-only floor earns a floor plate (labeled by junction
// count), and a group↔port belt never counts toward any plate's lift stats.
//
// EXPECTED: with groups on floor 0 and ONLY a splitter on floor 1, the graph
// renders floor-plate-0 AND floor-plate-1; floor-plate-1's label reads
// "1 JUNCTION"; and floor-plate-0 shows no lift tallies (its only belts run
// group→port, which are floor-agnostic, and group→junction across floors is
// the single real lift, tallied on both plates).
// ---------------------------------------------------------------------------
test("junction-only floor earns a plate labeled by junction count", async ({ page, request }) => {
  await resetView(request);
  const f = (await edit(request, [{ type: "create_factory", name: "JUNCTION FLOOR", position: { x: -1000, y: 1600 }, region: "GRASS FIELDS" }])).created[0];
  const ingot = (await edit(request, [{ type: "add_port", factory: f, direction: "in", item: "Desc_IronIngot_C", rate: 0, rateCeiling: 200, graphPos: { x: 0, y: 100 } }])).created[0];
  const g = (await edit(request, [{ type: "add_group", factory: f, machine: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", count: 1, clock: 1, graphPos: { x: 320, y: 100 }, floor: 0 }])).created[0];
  const j = (await edit(request, [{ type: "add_junction", factory: f, kind: "splitter", graphPos: { x: 680, y: 100 }, floor: 1 }])).created[0];
  await edit(request, [
    { type: "add_edge", factory: f, from: P(ingot), to: G(g), item: "Desc_IronIngot_C", tier: 3 },
    { type: "add_edge", factory: f, from: G(g), to: { kind: "junction", id: j }, item: "Desc_IronRod_C", tier: 3 },
  ]);

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    await openGraph(page, "JUNCTION FLOOR");

    // Both floors earn plates — floor 1 holds ONLY the junction.
    await expect(page.getByTestId("floor-plate-0")).toBeVisible();
    const plate1 = page.getByTestId("floor-plate-1");
    await expect(plate1).toBeVisible();
    await expect(plate1).toContainText("1 JUNCTION");
    // The real cross-floor lift (group F0 → junction F1) is tallied; the
    // group→port and port→group belts contribute nothing.
    await expect(page.getByTestId("floor-plate-0")).toContainText("1⤒");
    await expect(plate1).toContainText("1 IN");
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});
