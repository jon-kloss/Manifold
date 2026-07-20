// Audit #126 acceptance (promoted from the audit probe suite): drift-accept
// cascade correctness + save-only node synthesis, driven end-to-end through
// the dev bridge's import pipeline (/api/new_empire → /api/import/run →
// /api/proposal/accept → /api/hydrate). Every probe declares its EXPECTED
// (correct) result in the header BEFORE any assertion.
//
// Fixed by audit #126 (import.rs apply_sync + resync_built_wiring):
//   1. a group demolished in game cascades like DeleteGroup — no orphaned
//      belts, no stale boundary port exporting a product nobody makes;
//   2. a count/clock drift accept re-derives boundary port rates and raises
//      outgrown belt tiers, so an expanded bank isn't capped at its stale
//      export;
//   3. a group added in game arrives wired (belts to recipe partners + a
//      refreshed boundary), not as an orphan card.
//
// Seeded entirely through the API against the dev bridge's default fixture
// catalog (Build_SmelterMk1_C / Recipe_IngotIron_C @ 30/min,
// Build_ConstructorMk1_C / Recipe_IronRod_C @ 15/min, Build_MinerMk1_C) and
// the bundled world-node catalog (all nodes within x∈[-2818,4066],
// y∈[-3013,3029] — a miner at 900000,900000 is ~1.27M m from the nearest,
// far past NODE_MATCH_M=250 m). These probes assert purely over /api/hydrate,
// so they need no open page.

import { test, expect, type APIRequestContext } from "@playwright/test";
import { resetView } from "./helpers";

const API = "http://localhost:8791/api";

async function post(request: APIRequestContext, path: string, body: unknown): Promise<unknown> {
  const res = await request.post(`${API}${path}`, { data: JSON.stringify(body) });
  if (!res.ok()) throw new Error(`${path} ${res.status()}: ${await res.text()}`);
  return res.json();
}
interface Group { id: string; factory: string; machine: string; recipe: string; count: number }
interface Port {
  id: string;
  factory: string;
  direction: "in" | "out";
  item: string;
  rate: number;
  rateCeiling: number | null;
}
interface Edge {
  id: string;
  factory: string;
  from: { kind: string; id: string };
  to: { kind: string; id: string };
  item: string;
  tier: number;
}
interface NodeClaim { id: string; node: string; factory: string; extractor: string; saveNodeId: string | null }
interface NodeOverride { id: string; pos: { x: number; y: number; z: number } | null }
interface DerivedFactory { ports: Record<string, number> }
interface Hydrate {
  plan: {
    factories: Record<string, { id: string }>;
    groups: Record<string, Group>;
    ports: Record<string, Port>;
    edges: Record<string, Edge>;
    nodeClaims: Record<string, NodeClaim>;
    nodeOverrides: Record<string, NodeOverride>;
  };
  derived: { factories: Record<string, DerivedFactory> };
}
async function hydrate(request: APIRequestContext): Promise<Hydrate> {
  const res = await request.get(`${API}/hydrate`);
  if (!res.ok()) throw new Error(`hydrate ${res.status()}: ${await res.text()}`);
  return res.json();
}

const vals = <T,>(m: Record<string, T>): T[] => Object.values(m);

// Wipe the plan (fresh-empire probes own the whole plan) so no imported ◆
// layer, claim, or drift proposal leaks into later specs. delete_factory
// can't do this — Built factories are delete-immutable — so start another
// fresh empire instead.
async function cleanup(request: APIRequestContext): Promise<void> {
  try {
    await post(request, "/new_empire", {});
  } catch {
    /* best-effort */
  }
}

// ---------------------------------------------------------------------------
// PROBE 1 — Demolished-group drift accept cascades (no orphaned edges/ports,
// no phantom output).
//
// EXPECTED: After accept the factory has exactly one group (the smelter bank);
// NO edge in plan.edges has from.id or to.id equal to the removed constructor
// group id; NO Desc_IronRod_C port remains on the factory; and the factory's
// derived output does not list Desc_IronRod_C.
// ---------------------------------------------------------------------------
test("demolished-group drift accept cascades — no orphaned edges/ports/output", async ({ request }) => {
  await resetView(request);
  await post(request, "/new_empire", {});
  try {
    // First import: 2 smelters + 1 constructor → one ◆ Built factory.
    await post(request, "/import/run", {
      saveName: "P1",
      machines: [
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 0, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 50, y: 0, z: 0 },
        { class: "Build_ConstructorMk1_C", recipe: "Recipe_IronRod_C", clock: 1, x: 100, y: 0, z: 0 },
      ],
    });

    let h = await hydrate(request);
    // Exactly one clustered factory.
    const facs = vals(h.plan.factories);
    expect(facs).toHaveLength(1);
    const fid = facs[0].id;

    // Its two groups: the smelter bank (count 2) and the constructor.
    const groups = vals(h.plan.groups).filter((g) => g.factory === fid);
    expect(groups).toHaveLength(2);
    const smelter = groups.find((g) => g.machine === "Build_SmelterMk1_C");
    const constructor = groups.find((g) => g.machine === "Build_ConstructorMk1_C");
    expect(smelter).toBeTruthy();
    expect(constructor).toBeTruthy();
    const constructorGid = constructor!.id;

    // The Desc_IronRod_C Out port exists on the factory.
    const rodPortBefore = vals(h.plan.ports).filter(
      (p) => p.factory === fid && p.direction === "out" && p.item === "Desc_IronRod_C",
    );
    expect(rodPortBefore).toHaveLength(1);
    const rodPortId = rodPortBefore[0].id;

    // An internal edge runs smelter group → constructor group (ingot feed).
    const smelterToConstructor = vals(h.plan.edges).filter(
      (e) =>
        e.from.kind === "group" &&
        e.from.id === smelter!.id &&
        e.to.kind === "group" &&
        e.to.id === constructorGid,
    );
    expect(smelterToConstructor).toHaveLength(1);

    // Re-import with only the two smelters (constructor demolished in game) →
    // a SaveReimport drift proposal (never writes on its own).
    const drift = (await post(request, "/import/run", {
      saveName: "P1",
      machines: [
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 0, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 50, y: 0, z: 0 },
      ],
    })) as { outcome: string; proposal: string };
    expect(drift.outcome).toBe("drift");
    expect(drift.proposal).toBeTruthy();

    // Accept the drift → syncs the ◆ layer, removing the constructor.
    await post(request, "/proposal/accept", { id: drift.proposal });

    h = await hydrate(request);
    // (1) exactly one group survives — the smelter bank.
    const groupsAfter = vals(h.plan.groups).filter((g) => g.factory === fid);
    expect(groupsAfter).toHaveLength(1);
    expect(groupsAfter[0].machine).toBe("Build_SmelterMk1_C");

    // (2) NO edge references the removed constructor group id (either end).
    const orphanEdges = vals(h.plan.edges).filter(
      (e) => e.from.id === constructorGid || e.to.id === constructorGid,
    );
    expect(orphanEdges).toEqual([]);

    // (3) NO Desc_IronRod_C Out port remains on the factory.
    const rodPortsAfter = vals(h.plan.ports).filter(
      (p) => p.factory === fid && p.direction === "out" && p.item === "Desc_IronRod_C",
    );
    expect(rodPortsAfter).toEqual([]);

    // (4) the derived factory reports nothing for the REMOVED rod port —
    // keyed by the id captured before accept, so this can't go vacuous by
    // re-reading the (now empty) port list.
    const df = h.derived.factories[fid];
    expect(df.ports[rodPortId]).toBeUndefined();

    // (5) the surviving ingot export absorbs the freed feed: 45 → 60/min.
    const ingotOut = vals(h.plan.ports).filter(
      (p) => p.factory === fid && p.direction === "out" && p.item === "Desc_IronIngot_C",
    );
    expect(ingotOut).toHaveLength(1);
    expect(ingotOut[0].rate).toBeCloseTo(60, 3);
  } finally {
    await cleanup(request);
  }
});

// ---------------------------------------------------------------------------
// PROBE 2 — Count-up drift accept recomputes the exported port rate.
//
// EXPECTED: After accepting the 3→6 count drift, group.count == 6 AND the
// Desc_IronIngot_C Out port rate is 180/min (doubled), the derived factory
// output for Desc_IronIngot_C is 180/min, and the export belt rides at least
// MK.3 (180/min outgrows MK.2's 120 cap).
// ---------------------------------------------------------------------------
test("count-up drift accept recomputes the exported port rate", async ({ request }) => {
  await resetView(request);
  await post(request, "/new_empire", {});
  try {
    // First import: 3 smelters, no consumer → one ◆ Built factory exporting ingot.
    await post(request, "/import/run", {
      saveName: "P2",
      machines: [
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 0, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 50, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 100, y: 0, z: 0 },
      ],
    });

    let h = await hydrate(request);
    const facs = vals(h.plan.factories);
    expect(facs).toHaveLength(1);
    const fid = facs[0].id;

    const smelterGroups = vals(h.plan.groups).filter(
      (g) => g.factory === fid && g.machine === "Build_SmelterMk1_C",
    );
    expect(smelterGroups).toHaveLength(1);
    expect(smelterGroups[0].count).toBe(3);

    // The exported ingot Out port reads 90/min (3 × 30/min).
    const ingotPortBefore = vals(h.plan.ports).filter(
      (p) => p.factory === fid && p.direction === "out" && p.item === "Desc_IronIngot_C",
    );
    expect(ingotPortBefore).toHaveLength(1);
    expect(ingotPortBefore[0].rate).toBeCloseTo(90, 3);
    const ingotPortId = ingotPortBefore[0].id;

    // Re-import with 6 smelters (count doubled in game) → drift proposal.
    const drift = (await post(request, "/import/run", {
      saveName: "P2",
      machines: [
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 0, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 50, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 100, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 150, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 200, y: 0, z: 0 },
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 250, y: 0, z: 0 },
      ],
    })) as { outcome: string; proposal: string };
    expect(drift.outcome).toBe("drift");
    expect(drift.proposal).toBeTruthy();

    await post(request, "/proposal/accept", { id: drift.proposal });

    h = await hydrate(request);
    // group.count doubled to 6.
    const smelterAfter = vals(h.plan.groups).filter(
      (g) => g.factory === fid && g.machine === "Build_SmelterMk1_C",
    );
    expect(smelterAfter).toHaveLength(1);
    expect(smelterAfter[0].count).toBe(6);

    // The exported Out port rate must double to 180/min.
    const ingotPortAfter = vals(h.plan.ports).filter(
      (p) => p.factory === fid && p.direction === "out" && p.item === "Desc_IronIngot_C",
    );
    expect(ingotPortAfter).toHaveLength(1);
    expect(ingotPortAfter[0].rate).toBeCloseTo(180, 3);

    // The export belt was raised to carry it (MK.2's 120 cap would clip 180).
    const exportBelts = vals(h.plan.edges).filter(
      (e) => e.to.kind === "port" && e.to.id === ingotPortId,
    );
    expect(exportBelts.length).toBeGreaterThanOrEqual(1);
    for (const b of exportBelts) {
      expect(b.tier, "export belt tier covers 180/min").toBeGreaterThanOrEqual(3);
    }

    // And the derived factory output for the ingot port is 180/min.
    const df = h.derived.factories[fid];
    expect(df.ports[ingotPortId]).toBeCloseTo(180, 3);
  } finally {
    await cleanup(request);
  }
});

// ---------------------------------------------------------------------------
// PROBE 3 — Import synthesizes a save-only node claim + override for an
// extractor off the catalog.
//
// EXPECTED: The single imported factory owns exactly one node claim whose
// `node` id starts with "save:" (specifically save:BP_ResourceNode_TEST1)
// because the miner at (900000,900000) is beyond NODE_MATCH_M of every bundled
// catalog node, and there is exactly one node override with that same id
// carrying pos≈(900000,900000). The claim's saveNodeId is "BP_ResourceNode_TEST1".
// (Pins import.rs bind_extractors save-only synthesis, which the phase4-import
// e2e never checks — it asserts no node claims/overrides at all.)
// ---------------------------------------------------------------------------
test("import synthesizes a save-only node claim + override off the catalog", async ({ request }) => {
  await resetView(request);
  await post(request, "/new_empire", {});
  try {
    // The whole cluster sits at (900000, 900000) — machines and miner
    // together, so the miner attributes to the cluster (within the ~360 m
    // attribution radius) while remaining ~1.27M m from every catalog node.
    await post(request, "/import/run", {
      saveName: "N1",
      machines: [
        { class: "Build_SmelterMk1_C", recipe: "Recipe_IngotIron_C", clock: 1, x: 900000, y: 900000, z: 0 },
      ],
      extractors: [
        {
          class: "Build_MinerMk1_C",
          clock: 1,
          x: 900050,
          y: 900000,
          z: 0,
          nodeActorId: "BP_ResourceNode_TEST1",
        },
      ],
    });

    const h = await hydrate(request);
    const facs = vals(h.plan.factories);
    expect(facs).toHaveLength(1);
    const fid = facs[0].id;

    // Exactly one node claim, owned by the imported factory, under a save-only id.
    const claims = vals(h.plan.nodeClaims).filter((c) => c.factory === fid);
    expect(claims).toHaveLength(1);
    const claim = claims[0];
    expect(claim.node.startsWith("save:")).toBe(true);
    expect(claim.node).toBe("save:BP_ResourceNode_TEST1");
    // The stable save reference is preserved as the re-match key.
    expect(claim.saveNodeId).toBe("BP_ResourceNode_TEST1");

    // Exactly one node override, keyed by the same save-only id, carrying the
    // miner's own position (≈ 900050,900000) so the node resolves into the map.
    const overrides = vals(h.plan.nodeOverrides).filter((o) => o.id === "save:BP_ResourceNode_TEST1");
    expect(overrides).toHaveLength(1);
    expect(overrides[0].pos).not.toBeNull();
    expect(overrides[0].pos!.x).toBeCloseTo(900050, 1);
    expect(overrides[0].pos!.y).toBeCloseTo(900000, 1);
  } finally {
    await cleanup(request);
  }
});
