// WASM T0 wrapper + snapshot builder (mirror of Session::snapshot on the Rust
// side). Runs on drag frames only; the release commit goes through plan.edit
// and T1 settles authoritatively.

import init, { t0_solve } from "../wasm/pkg/solver_wasm.js";
import wasmUrl from "../wasm/pkg/solver_wasm_bg.wasm?url";
import type { DerivedFactory, GameData, Id, Plan, TargetCeiling } from "../state/types";
import { beltCapacity } from "../state/types";

let readyPromise: Promise<void> | null = null;
export function ensureT0(): Promise<void> {
  readyPromise ??= init({ module_or_path: wasmUrl }).then(() => undefined);
  return readyPromise;
}

interface WasmSolveResult {
  groups: Record<Id, { count: number; clock: number; powerMw: number; inRates: Record<string, number>; outRates: Record<string, number> }>;
  edges: Record<Id, { flow: number; saturation: number }>;
  ports: Record<Id, number>;
  totalPowerMw: number;
  targetCeiling: TargetCeiling | null;
  clamped: boolean;
  solveUs: number;
}

export interface FactorySnapshot {
  groups: unknown[];
  edges: unknown[];
  inputs: unknown[];
  outputs: unknown[];
  junctions: string[];
}

export function buildSnapshot(plan: Plan, gamedata: GameData, factoryId: Id): FactorySnapshot | null {
  const factory = plan.factories[factoryId];
  if (!factory) return null;
  const groups = [];
  for (const gid of factory.groups) {
    const g = plan.groups[gid];
    const recipe = g && gamedata.recipes[g.recipe];
    if (!g || !recipe) return null;
    groups.push({
      id: g.id,
      recipe: {
        id: recipe.className,
        machine: g.machine,
        durationS: recipe.durationS,
        inputs: recipe.ingredients,
        outputs: recipe.products,
        powerMw: gamedata.machines[g.machine]?.powerMw ?? 0,
      },
      count: g.count,
      clock: g.clock,
    });
  }
  const inputs = [];
  const outputs = [];
  for (const pid of factory.ports) {
    const p = plan.ports[pid];
    if (!p) return null;
    if (p.direction === "in") inputs.push({ id: p.id, item: p.item, ceiling: p.rateCeiling });
    else outputs.push({ id: p.id, item: p.item, rate: p.rate });
  }
  const toRef = (end: { kind: string; id: string }) => {
    if (end.kind === "group") return { kind: "group", id: end.id };
    if (end.kind === "junction") return { kind: "junction", id: end.id };
    return plan.ports[end.id]?.direction === "in" ? { kind: "input", id: end.id } : { kind: "output", id: end.id };
  };
  const edges = Object.values(plan.edges)
    .filter((e) => e.factory === factoryId)
    .map((e) => ({
      id: e.id,
      from: toRef(e.from),
      to: toRef(e.to),
      item: e.item,
      capacity: beltCapacity(e.tier),
    }));
  const junctions = Object.values(plan.junctions)
    .filter((j) => j.factory === factoryId)
    .map((j) => j.id);
  return { groups, edges, inputs, outputs, junctions };
}

/** Projected drag-frame solve. Returns null if the wasm module isn't ready or errors. */
export function t0SetTarget(
  snapshot: FactorySnapshot,
  port: Id,
  rate: number,
): (DerivedFactory & { clamped: boolean }) | null {
  const started = performance.now();
  try {
    const r = t0_solve(snapshot, { type: "set_target", port, rate }) as WasmSolveResult;
    const solveUs = Math.round((performance.now() - started) * 1000);
    return {
      groups: Object.fromEntries(
        Object.entries(r.groups).map(([id, g]) => [id, { inRates: g.inRates, outRates: g.outRates, powerMw: g.powerMw }]),
      ),
      edges: r.edges,
      ports: r.ports,
      totalPowerMw: r.totalPowerMw,
      targetCeiling: r.targetCeiling,
      solveUs,
      solveOnRelease: false,
      solveError: null,
      clamped: r.clamped,
    };
  } catch {
    return null;
  }
}
