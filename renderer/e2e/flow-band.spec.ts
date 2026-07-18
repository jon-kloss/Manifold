// Efficiency-grammar band boundaries (DECISIONS: belts don't jam — a
// ratio-perfect build runs belts at 100% and that is OPTIMAL). Pure unit
// pins on the single banding authority the graph edges, map routes, audit
// SATURATION tab, and status bar all share.

import { test, expect } from "@playwright/test";
import { flowBand, flowSpeed, routeBottleneck, bottleneckEdges } from "../src/lib/format";
import { footprintFor } from "../src/graph/footprints";
import type { Constraint, DeficitRow, DerivedFactory, GameData } from "../src/state/types";

test("flowBand: under/good boundary sits AT 50% — 30 on a 60-belt is under-used", () => {
  // The user's literal case: a 60/min belt carrying 30/min = exactly 50%.
  expect(flowBand(30 / 60, 30)).toBe("under");
  // Just above the boundary is good.
  expect(flowBand(0.501, 30.06)).toBe("good");
  // A FULL belt that meets demand is optimal, not critical.
  expect(flowBand(1.0, 60)).toBe("good");
  // Zero flow is idle — a distinct band (never "under", never healthy green) so
  // a wired-but-empty belt reads as carrying nothing, not as a full belt.
  expect(flowBand(0, 0)).toBe("idle");
  // Bottleneck evidence outranks utilization in both directions.
  expect(flowBand(1.0, 60, true)).toBe("bottleneck");
  expect(flowBand(0.2, 12, true)).toBe("bottleneck");
});

test("routeBottleneck: red needs a deficit THROUGH the route AND a full route", () => {
  const deficits: DeficitRow[] = [
    { factory: "f-rod", port: "p-in", route: "r1", item: "Desc_IronIngot_C", needed: 100, supplied: 60 },
  ];
  // Full route + downstream starved through it = the link caps demand.
  expect(routeBottleneck("r1", 1.0, deficits)).toBe(true);
  // Starved but slack: upstream under-produces — NOT this route's fault.
  expect(routeBottleneck("r1", 10 / 60, deficits)).toBe(false);
  // Full but nobody starves: ratio-perfect, optimal.
  expect(routeBottleneck("r2", 1.0, deficits)).toBe(false);
  expect(routeBottleneck("r1", 1.0, [])).toBe(false);
  // FULL pin: 95% is NOT "at capacity" — the abolished ≥95% grammar stays dead.
  expect(routeBottleneck("r1", 0.95, deficits)).toBe(false);
});

// ---- bottleneckEdges: the solver-named evidence behind every red edge ----

const df = (over: Partial<DerivedFactory>): DerivedFactory => ({
  groups: {},
  edges: {},
  ports: {},
  totalPowerMw: 0,
  targetCeiling: null,
  solveUs: 0,
  solveOnRelease: false,
  solveError: null,
  ...over,
});

const beltBinding = (edge: string): Constraint => ({
  kind: "belt_capacity",
  edge,
  item: "Desc_IronIngot_C",
  capacity: 60,
});

test("bottleneckEdges shortfall arm: a missing target names its binding belt", () => {
  // Unmet target whose binding is a belt → that belt is the honest red.
  const short = df({
    shortfalls: { "p-out": { requested: 60, missing: 20, binding: beltBinding("e1") } },
  });
  expect(bottleneckEdges(short).has("e1")).toBe(true);
  expect(bottleneckEdges(short).size).toBe(1);
  // missing = 0: the target is met — a named binding alone is not evidence.
  const met = df({
    shortfalls: { "p-out": { requested: 60, missing: 0, binding: beltBinding("e1") } },
  });
  expect(bottleneckEdges(met).size).toBe(0);
  // Non-belt binding (input ceiling caps the run): no belt to blame.
  const ceilingBound = df({
    shortfalls: {
      "p-out": {
        requested: 60,
        missing: 20,
        binding: { kind: "input_ceiling", port: "p-in", item: "Desc_IronIngot_C", ceiling: 40 },
      },
    },
  });
  expect(bottleneckEdges(ceilingBound).size).toBe(0);
});

test("bottleneckEdges ceiling arm: belt-bound target ceiling needs the belt actually FULL", () => {
  const ceiled = (saturation: number) =>
    df({
      targetCeiling: { maxRate: 60, binding: beltBinding("e2") },
      edges: { e2: { flow: 60 * saturation, saturation } },
    });
  // Clamped at the ceiling with the belt running full → the belt caps demand.
  expect(bottleneckEdges(ceiled(1.0)).has("e2")).toBe(true);
  // FULL tolerates solver float noise (0.999)…
  expect(bottleneckEdges(ceiled(0.999)).has("e2")).toBe(true);
  // …but 95% is NOT full — loosening FULL would resurrect the old ≥95% grammar.
  expect(bottleneckEdges(ceiled(0.95)).size).toBe(0);
  // A ceiling merely naming the NEXT constraint while the belt runs slack.
  expect(bottleneckEdges(ceiled(0.4)).size).toBe(0);
  // No derived factory at all → nothing to name.
  expect(bottleneckEdges(undefined).size).toBe(0);
});

// ---- footprintFor: game clearance data wins; community table is the fallback ----

const gd = (machines: GameData["machines"]): GameData => ({
  items: {},
  recipes: {},
  machines,
  belts: {},
  buildables: {},
  buildVersion: "test",
});

test("footprintFor: catalog footprintM overrides the community table", () => {
  // Docs-derived clearance present (deliberately ≠ the 8×10 community row).
  const withClearance = gd({
    Build_ConstructorMk1_C: {
      className: "Build_ConstructorMk1_C",
      displayName: "Constructor",
      powerMw: 4,
      kind: "manufacturer",
      footprintM: [7, 11],
    },
  });
  expect(footprintFor(withClearance, "Build_ConstructorMk1_C")).toEqual({ w: 7, l: 11, derived: true });
  // Machine absent from the catalog → community table, honestly labeled.
  expect(footprintFor(gd({}), "Build_ConstructorMk1_C")).toEqual({ w: 8, l: 10, derived: false });
  // Unknown class everywhere → the generic fallback, still not "derived".
  expect(footprintFor(gd({}), "Build_Mystery_C")).toEqual({ w: 8, l: 8, derived: false });
});

// ---- flowSpeed: dash period from utilization, quantized to eighths ----

test("flowSpeed: 4s trickle → 0.8s saturated, clamped, eighth-buckets", () => {
  expect(flowSpeed(0)).toBe("4.00s");
  expect(flowSpeed(1)).toBe("0.80s");
  // Clamped outside [0,1].
  expect(flowSpeed(-1)).toBe("4.00s");
  expect(flowSpeed(2)).toBe("0.80s");
  // Mid-point lands on an exact bucket.
  expect(flowSpeed(0.5)).toBe("2.40s");
  // Bucket pin: a re-solve nudge inside a bucket must not change the duration
  // (each change is a CSS dash phase jump — only bucket crossings may pay it).
  expect(flowSpeed(0.51)).toBe(flowSpeed(0.5));
});
