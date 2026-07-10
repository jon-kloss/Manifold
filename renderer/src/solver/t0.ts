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
  const edges = Object.values(plan.edges)
    .filter((e) => e.factory === factoryId)
    .map((e) => ({
      id: e.id,
      from: e.from.kind === "group" ? { kind: "group", id: e.from.id } : plan.ports[e.from.id]?.direction === "in" ? { kind: "input", id: e.from.id } : { kind: "output", id: e.from.id },
      to: e.to.kind === "group" ? { kind: "group", id: e.to.id } : plan.ports[e.to.id]?.direction === "in" ? { kind: "input", id: e.to.id } : { kind: "output", id: e.to.id },
      item: e.item,
      capacity: beltCapacity(e.tier),
    }));
  return { groups, edges, inputs, outputs };
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
