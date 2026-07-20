// Audit #131 acceptance (promoted from the audit probe suite): map (MapView / CanvasLayer / SearchBox / claim tethers).
// Functional probes for behaviors the shipped e2e suite does not pin. Each test
// declares its EXPECTED (correct) result in a header comment BEFORE any
// assertion; a failing probe is DATA for the mismatch protocol, not a reason to
// weaken the assertion. Seed via the bridge API BEFORE page.goto (API edits do
// not stream to an open client), and clean up every created factory / override
// in finally{}.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

// NOTE: no serial mode — the runner uses --workers=1, and per-test isolation
// (each test seeds + deletes its own factories) means a failure must NOT
// cascade-skip sibling probes: every probe needs a verdict.

const API = "http://localhost:8791/api";
async function edit(request: APIRequestContext, cmds: unknown[]): Promise<{ created: string[] }> {
  const res = await request.post(`${API}/edit`, { data: JSON.stringify(cmds) });
  if (!res.ok()) throw new Error(`edit ${res.status()}: ${await res.text()}`);
  return res.json();
}
async function hydrate(request: APIRequestContext): Promise<{
  plan: {
    nodeClaims: Record<string, { node: string; factory: string }>;
  };
  world: { nodes: { id: string; item: string; x: number; y: number }[] };
}> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}`);
  return res.json();
}

async function dismissOnboarding(page: import("@playwright/test").Page): Promise<void> {
  const skip = page.getByTestId("onboard-skip");
  if (await skip.isVisible().catch(() => false)) await skip.click();
}

// Isolation backstop (PR #61 review): teardown edits are best-effort and a
// probe aborted mid-finally would leak its factory or node override into the
// shared serial plan. Scrub THIS SUITE's leftovers at probe start — a clean
// plan needs nothing, a polluted one is repaired before seeding.
const PROBE_FACTORIES = ["ANCHOR", "ZZQTOP WORKS", "TETHER PROBE", "TETHER REF"];
async function scrubLeftovers(request: APIRequestContext): Promise<void> {
  try {
    const res = await request.get(`${API}/hydrate`);
    if (!res.ok()) return;
    const h = (await res.json()) as {
      plan: {
        factories: Record<string, { id: string; name: string }>;
        nodeOverrides?: Record<string, { id: string }>;
      };
    };
    const cmds: unknown[] = [];
    for (const f of Object.values(h.plan.factories))
      if (PROBE_FACTORIES.includes(f.name)) cmds.push({ type: "delete_factory", id: f.id });
    // At probe start no spec is mid-flight (workers=1), so any surviving
    // override is by definition a leak from an aborted teardown.
    for (const ov of Object.values(h.plan.nodeOverrides ?? {}))
      cmds.push({ type: "set_node_override", id: ov.id, nodeOverride: null });
    if (cmds.length) await edit(request, cmds);
  } catch {
    /* best-effort backstop */
  }
}

// ---------------------------------------------------------------------------
// PROBE 1 — Search-jump lands on a node's CORRECTED (override) position.
//
// EXPECTED: after searching a node whose plan override moved it +5000/+5000
// world units off its catalog spot and pressing Enter, [data-testid=map-root]
// data-center reads approximately '(N.x+5000),(N.y+5000)' (within ~400 world
// units) — the corrected position where the node actually renders and where its
// selection ring is drawn.
// (Fixed by audit #131: SearchBox resolves the hit through plan.nodeOverrides,
// so the jump lands where the node actually renders.)
// ---------------------------------------------------------------------------
test("search-jump lands on a node's corrected override position", async ({ page, request }) => {
  await resetView(request);
  await scrubLeftovers(request);
  const anchor = (
    await edit(request, [
      { type: "create_factory", name: "ANCHOR", position: { x: -2000, y: 2000 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  const h = await hydrate(request);
  const iron = h.world.nodes.find((n) => n.item === "Desc_OreIron_C");
  if (!iron) throw new Error("no iron catalog node in world snapshot");
  const nId = iron.id;
  const nx = iron.x;
  const ny = iron.y;
  // Offset TOWARD the node field's centroid so the corrected position stays
  // inside the camera's pan bounds — an outward offset would clamp the jump
  // short and fail on geometry rather than on the behavior under test.
  const cxAll = h.world.nodes.reduce((s2, n) => s2 + n.x, 0) / h.world.nodes.length;
  const cyAll = h.world.nodes.reduce((s2, n) => s2 + n.y, 0) / h.world.nodes.length;
  // 2000 units keeps the corrected spot inside the world/camera bounds for any
  // catalog node while still dwarfing the 400-unit tolerance — a jump to the
  // stale catalog spot fails by ~1600.
  const wantX = nx + Math.sign(cxAll - nx || 1) * 2000;
  const wantY = ny + Math.sign(cyAll - ny || 1) * 2000;

  try {
    await edit(request, [
      { type: "set_node_override", id: nId, nodeOverride: { id: nId, pos: { x: wantX, y: wantY, z: 0 } } },
    ]);

    await page.goto("/");
    await dismissOnboarding(page);
    const root = page.getByTestId("map-root");
    await expect(root).toBeVisible();
    await page.waitForTimeout(400); // let the boot view settle before the jump

    // SearchBox matches nodes on n.id — Enter takes hits[0], the override node.
    await page.locator(".searchbox input").fill(nId);
    await page.keyboard.press("Enter");

    // The pan animates and moveend stamps data-center more than once along the
    // way — poll until the camera SETTLES on the corrected render position
    // (a fixed sleep reads a mid-flight center and flakes deterministically).
    await expect
      .poll(
        async () => {
          const c = await root.getAttribute("data-center");
          if (!c) return Infinity;
          const [cx, cy] = c.split(",").map(Number);
          return Math.max(Math.abs(cx - wantX), Math.abs(cy - wantY));
        },
        { timeout: 10_000, message: "camera settles on the corrected (override) position" },
      )
      .toBeLessThan(400);
  } finally {
    await edit(request, [{ type: "set_node_override", id: nId, nodeOverride: null }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: anchor }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 2 — Search filter stays inert when the query matches no resource node.
//
// EXPECTED: typing a factory name whose substring hits no item label or purity
// leaves data-nodes-shown at exactly its full-field value N (nodeFilter.active
// = false → the whole node field is drawn; searching a factory never blanks the
// resource field). A subsequent real resource query ('iron') then narrows the
// field to a non-empty subset (0 < shown < N). node-filter.spec pins narrowing
// + clear-restores; this no-match inertness branch is otherwise unpinned.
// ---------------------------------------------------------------------------
test("search filter stays inert when the query matches no resource node", async ({ page, request }) => {
  await resetView(request);
  await scrubLeftovers(request);
  const f = (
    await edit(request, [
      { type: "create_factory", name: "ZZQTOP WORKS", position: { x: -2600, y: 2600 }, region: "GRASS FIELDS" },
    ])
  ).created[0];

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    const root = page.getByTestId("map-root");
    await expect(root).toBeVisible();
    const shown = async () => Number(await root.getAttribute("data-nodes-shown"));

    const full = await shown();
    expect(full).toBeGreaterThan(50); // the bundled world has hundreds of nodes

    // A factory-name query matches zero resource nodes → the field stays FULL,
    // never blanks. Poll then hold to confirm it settled at N (not 0).
    await page.locator(".searchbox input").fill("zzqtop");
    await expect.poll(shown, { message: "no-match query must not narrow the field" }).toBe(full);
    expect(await shown()).toBe(full);

    // A real resource query DOES narrow to a non-empty subset.
    await page.locator(".searchbox input").fill("iron");
    await expect.poll(shown, { message: "iron narrows the node field" }).toBeLessThan(full);
    expect(await shown()).toBeGreaterThan(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
  }
});

// ---------------------------------------------------------------------------
// PROBE 3 — No claim tether is drawn to a node hidden by the search filter.
//
// EXPECTED: a factory claims a coal node (its tether is highlighted signal-
// orange). When the search filter is narrowed to 'iron' — hiding the coal node
// (data-nodes-shown drops, the coal disc is no longer drawn) — NO signal-orange
// claim tether remains along the factory→(hidden node) line: a claim on a
// filtered-out node draws no dangling tether.
// (Fixed by audit #131: claimLinks skips claims on search-hidden nodes, so no
// dangling tether is stroked to empty ground.)
//
// Tethers are canvas-drawn, so this is asserted by sampling the data canvas
// ('.map-canvas-layer') pixels along the factory→node line for signal-orange
// (--signal-500 #F78B23). The projection from world→screen is solved from two
// factory pins (CRS.Simple is an axis-independent affine, so two known
// world/screen pairs pin it exactly). A pre-filter sanity sample asserts the
// orange tether IS present first, so a broken projection fails loudly rather
// than passing vacuously.
// ---------------------------------------------------------------------------
test("no claim tether is drawn to a search-filtered (hidden) node", async ({ page, request }) => {
  await resetView(request);
  await scrubLeftovers(request);
  const f = (
    await edit(request, [
      { type: "create_factory", name: "TETHER PROBE", position: { x: -2700, y: 2500 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  // Second pin at a DISTINCT world x AND y — the reference for the affine solve.
  const g = (
    await edit(request, [
      { type: "create_factory", name: "TETHER REF", position: { x: -1500, y: 1500 }, region: "GRASS FIELDS" },
    ])
  ).created[0];
  const worldF = { x: -2700, y: 2500 };
  const worldG = { x: -1500, y: 1500 };

  // Pixel sampler: solve the world→screen affine from the two named pins, then
  // walk the F-pin → coal-node line counting signal-orange (#F78B23) pixels on
  // the data canvas. Occlusion-immune: it reads the canvas backing store, not
  // the composited screen, so overlaying drawers do not matter.
  const sampleOrange = (args: {
    fName: string;
    gName: string;
    wf: { x: number; y: number };
    wg: { x: number; y: number };
    coal: { x: number; y: number };
  }): { orange: number; onCanvas: number } => {
    const pinCenter = (name: string): { x: number; y: number } | null => {
      const el = [...document.querySelectorAll<HTMLElement>(".pin-icon")].find((e) =>
        (e.textContent ?? "").includes(name),
      );
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
    };
    const canvas = document.querySelector<HTMLCanvasElement>(".map-canvas-layer");
    if (!canvas) return { orange: -1, onCanvas: 0 };
    const rect = canvas.getBoundingClientRect();
    const sc = canvas.width / rect.width; // devicePixelRatio scale
    const fp = pinCenter(args.fName);
    const gp = pinCenter(args.gName);
    if (!fp || !gp) return { orange: -2, onCanvas: 0 };
    // container-relative pin points
    const fpc = { x: fp.x - rect.x, y: fp.y - rect.y };
    const gpc = { x: gp.x - rect.x, y: gp.y - rect.y };
    // axis-independent affine: screen = s*world + o (solve per axis)
    const sx = (fpc.x - gpc.x) / (args.wf.x - args.wg.x);
    const ox = fpc.x - sx * args.wf.x;
    const sy = (fpc.y - gpc.y) / (args.wf.y - args.wg.y);
    const oy = fpc.y - sy * args.wf.y;
    const coalPc = { x: sx * args.coal.x + ox, y: sy * args.coal.y + oy };
    const ctx = canvas.getContext("2d");
    if (!ctx) return { orange: -3, onCanvas: 0 };
    let orange = 0;
    let onCanvas = 0;
    const N = 140;
    for (let i = 0; i <= N; i++) {
      const t = 0.08 + (0.9 * i) / N; // skip the pin/node endcaps
      const px = Math.round((fpc.x + (coalPc.x - fpc.x) * t) * sc);
      const py = Math.round((fpc.y + (coalPc.y - fpc.y) * t) * sc);
      if (px < 0 || py < 0 || px >= canvas.width || py >= canvas.height) continue;
      onCanvas++;
      const d = ctx.getImageData(px, py, 1, 1).data;
      const [r, gc, b, a] = [d[0], d[1], d[2], d[3]];
      // signal-orange (#F78B23 = 247,139,35): high R, mid G, low B, strong
      // R−B / R−G separation. Rejects iron grey (#6E7D8C), coal grey, grid.
      if (a > 40 && r > 150 && gc > 70 && gc < 200 && b < 120 && r - b > 80 && r - gc > 35) orange++;
    }
    return { orange, onCanvas };
  };

  try {
    await page.goto("/");
    await dismissOnboarding(page);
    const root = page.getByTestId("map-root");
    await expect(root).toBeVisible();
    await page.waitForTimeout(400);

    // Terrain off — remove the muted brown underlay so the only orange source
    // along the line is the claim tether itself.
    await page.getByTestId("btn-overlay-terrain").click();

    // Claim a coal node for TETHER PROBE via the drawer.
    await page.locator(".searchbox input").fill("coal");
    await page.keyboard.press("Enter");
    const drawer = page.getByTestId("node-drawer");
    await expect(drawer).toBeVisible();
    const claimFor = drawer.locator("section:has(h3:has-text('CLAIM FOR'))");
    await claimFor.locator("select").first().selectOption({ label: "TETHER PROBE" });
    await page.getByTestId("btn-claim").click();
    const claimsSection = drawer.locator("section:has(h3:has-text('CLAIMS'))");
    await expect(claimsSection.locator(".drawer-row")).toHaveCount(1);

    // Resolve the claimed coal node's world coordinates from the plan.
    const h = await hydrate(request);
    const claim = Object.values(h.plan.nodeClaims).find((c) => c.factory === f);
    if (!claim) throw new Error("coal claim not found for TETHER PROBE");
    const coalNode = h.world.nodes.find((n) => n.id === claim.node);
    if (!coalNode) throw new Error("claimed coal node absent from world snapshot");
    const coal = { x: coalNode.x, y: coalNode.y };

    // Highlight the tether: close the node drawer, select the factory pin.
    await page.keyboard.press("Escape");
    await page.locator(".pin-wrap", { hasText: "TETHER PROBE" }).click();
    await page.waitForTimeout(250);

    // Sanity: with coal visible + tether highlighted, orange IS present. This
    // guards against a broken projection producing a vacuous pass below.
    const before = await page.evaluate(sampleOrange, { fName: "TETHER PROBE", gName: "TETHER REF", wf: worldF, wg: worldG, coal });
    expect(before.onCanvas, "sample line must fall on the canvas").toBeGreaterThan(50);
    expect(before.orange, "highlighted claim tether should be visible pre-filter").toBeGreaterThan(0);

    // Narrow the filter to 'iron' — the coal node is hidden (disc removed).
    const shown = async () => Number(await root.getAttribute("data-nodes-shown"));
    const full = await shown();
    await page.locator(".searchbox input").fill("iron");
    await expect.poll(shown, { message: "iron filter hides the coal node" }).toBeLessThan(full);
    await page.waitForTimeout(200); // let the canvas redraw settle

    // EXPECTED: no signal-orange remains along the factory→(hidden node) line —
    // neither a dangling tether nor the coal disc.
    const after = await page.evaluate(sampleOrange, { fName: "TETHER PROBE", gName: "TETHER REF", wf: worldF, wg: worldG, coal });
    expect(after.onCanvas, "sample line must still fall on the canvas").toBeGreaterThan(50);
    expect(after.orange, "a claim on a filtered-out node must draw no tether").toBe(0);
  } finally {
    await edit(request, [{ type: "delete_factory", id: f }]).catch(() => {});
    await edit(request, [{ type: "delete_factory", id: g }]).catch(() => {});
  }
});
