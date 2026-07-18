// Number formatting: every number in the app is JetBrains Mono tabular-nums;
// rates carry an attached, smaller unit; projected values are italic (CSS).

import type { DeficitRow, DerivedFactory, Id } from "../state/types";

export function fmtRate(v: number): string {
  if (!isFinite(v)) return "∞";
  const abs = Math.abs(v);
  if (abs >= 1000) return v.toFixed(0);
  if (abs >= 100) return v.toFixed(1).replace(/\.0$/, "");
  const s = v.toFixed(2);
  return s.replace(/\.?0+$/, "") || "0";
}

export function fmtPercent(v: number): string {
  return `${Math.round(v * 100)}%`;
}

export function fmtClock(v: number): string {
  const pct = v * 100;
  return `${pct % 1 === 0 ? pct.toFixed(0) : pct.toFixed(1)}%`;
}

/** m:ss clock from seconds — the transport math block's time grammar. Round the
 *  TOTAL first, then split, so 119.7s reads "2:00", never "1:60". Shared by the
 *  route inspector, the train-answer block, and the clipboard payload. */
export function fmtClockS(s: number): string {
  const t = Math.max(0, Math.round(s));
  const m = Math.floor(t / 60);
  const sec = t % 60;
  return `${m}:${String(sec).padStart(2, "0")}`;
}

export function fmtPower(mw: number): string {
  if (mw >= 1000) return `${(mw / 1000).toFixed(1)} GW`;
  return `${mw % 1 === 0 ? mw.toFixed(0) : mw.toFixed(1)} MW`;
}

export function fmtKm(meters: number): string {
  return meters >= 1000 ? `${(meters / 1000).toFixed(1)} km` : `${Math.round(meters)} m`;
}

/** Human duration from minutes: "5h 12m", "16h", "40m". "—" for a
 *  non-finite or non-positive input (e.g. a zero/absent production rate). */
export function fmtDuration(minutes: number): string {
  if (!isFinite(minutes) || minutes <= 0) return "—";
  const total = Math.floor(minutes);
  const h = Math.floor(total / 60);
  const m = total % 60;
  if (h === 0) return `${m}m`;
  if (m === 0) return `${h}h`;
  return `${h}h ${m}m`;
}

/** Efficiency grammar for belt/route flow (DECISIONS): belts don't jam — a
 *  ratio-perfect build runs its belts at 100% and that is OPTIMAL, not
 *  critical. Bands:
 *  - "under": flowing but ≤50% utilized — over-built or starved upstream
 *    (flow-warn amber; 30 on a 60-belt lands here).
 *  - "good": >50% utilized and not a bottleneck, INCLUDING a full belt whose
 *    consumers are satisfied (flow-ok green).
 *  - "bottleneck": the link provably caps demanded throughput (flow-crit
 *    red). Utilization alone NEVER makes a bottleneck — callers must pass
 *    solver evidence (see bottleneckEdges / routeBottleneck).
 *  - "idle": zero flow — a connected belt carrying nothing (e.g. a downstream
 *    line whose feed is fully exported). Rendered dim/neutral and never
 *    animated, so it reads as "wired but not flowing" instead of healthy green.
 *  The single banding authority for BeltEdgeView strokes, map route
 *  polylines/chips, the audit SATURATION tab, and the status bar. */
export type FlowBand = "idle" | "under" | "good" | "bottleneck";
export function flowBand(saturation: number, flow: number, bottleneck = false): FlowBand {
  if (bottleneck) return "bottleneck";
  if (flow <= 0) return "idle";
  if (saturation <= 0.5) return "under";
  return "good";
}

/** MOTION = FLOW (gate: flow > 0); speed = utilization. One dash period for
 *  the graph's .edge-flowing overlay: 4s trickle at 0% utilization down to
 *  0.8s saturated. Saturation is quantized to EIGHTHS so mid-drag re-solves
 *  only change animation-duration (a CSS phase jump) on bucket crossings —
 *  never per solve frame; the endpoints stay exact (4.00s / 0.80s, 0.40s
 *  steps). Cross-ref the map's CanvasLayer animSpeed: speed encodes
 *  utilization WITHIN a surface; absolute px/s is per-surface tuned (graph
 *  card-scale vs map world-scale) — deliberately not shared. */
export function flowSpeed(saturation: number): string {
  const u = Math.round(Math.max(0, Math.min(1, saturation)) * 8) / 8;
  return `${(4 - 3.2 * u).toFixed(2)}s`;
}

/** "Running at capacity" within solver float noise. */
const FULL = 0.999;

/** Belt edges the solver NAMES as the binding capacity constraint — the
 *  honest bottleneck evidence for a factory's graph. Two sources:
 *  - a reported shortfall (unmet target) whose binding is this belt;
 *  - the target ceiling binding this belt while the belt actually runs full
 *    (the clamped-at-ceiling state the inspector's ⛔ strip declares).
 *  A ceiling that merely names the NEXT constraint while the target sits
 *  below it leaves the belt slack — not a bottleneck. */
export function bottleneckEdges(df: DerivedFactory | undefined | null): Set<Id> {
  const out = new Set<Id>();
  if (!df) return out;
  for (const s of Object.values(df.shortfalls ?? {})) {
    if (s.missing > 1e-9 && s.binding?.kind === "belt_capacity") out.add(s.binding.edge);
  }
  const b = df.targetCeiling?.binding;
  if (b?.kind === "belt_capacity" && (df.edges[b.edge]?.saturation ?? 0) >= FULL) out.add(b.edge);
  return out;
}

/** A map route is a bottleneck only when downstream registers a deficit
 *  THROUGH it while the route runs at full capacity — the link itself caps
 *  the demanded throughput. A deficit on a slack route is an upstream
 *  production problem, not this route's. */
export function routeBottleneck(routeId: Id, saturation: number, deficits: DeficitRow[]): boolean {
  return saturation >= FULL && deficits.some((d) => d.route === routeId);
}

/** Circuit headroom: fraction of generation left as margin (SDD §12; mirrors
 *  the Rust `circuit_level` formula). Demand with no generation reads fully
 *  overdrawn (-1); an idle grid reads full margin (1). */
export function circuitHeadroom(generationMw: number, demandMw: number): number {
  if (generationMw > 0) return (generationMw - demandMw) / generationMw;
  return demandMw > 0 ? -1 : 1;
}

/** Power margin levels keep congestion semantics — grids DO brown out. */
export type PowerLevel = "ok" | "warn" | "crit";

/** Power level from headroom (thresholds: ≥20% OK, 5–20% WARN, <5% CRIT). The
 *  single source for the audit rows, the PWR chip, and the review banner. */
export function powerLevel(headroom: number): PowerLevel {
  if (headroom < 0.05) return "crit";
  if (headroom < 0.2) return "warn";
  return "ok";
}

/** Honest fallback for item classes the bundled catalog doesn't know:
 *  Desc_SteelIngot_C → "Steel Ingot" (never leak raw class names to chips). */
export function prettyClass(cls: string): string {
  return cls
    .replace(/^Desc_/, "")
    .replace(/_C$/, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2");
}

/** Real game names for the raw extractable resources, so a node reads "Bauxite"
 *  rather than a raw class even before the player uploads their Docs.json (the
 *  bundled fixture catalog doesn't carry every resource item). */
const RESOURCE_NAMES: Record<string, string> = {
  Desc_OreIron_C: "Iron Ore",
  Desc_OreCopper_C: "Copper Ore",
  Desc_Stone_C: "Limestone",
  Desc_Coal_C: "Coal",
  Desc_OreGold_C: "Caterium Ore",
  Desc_RawQuartz_C: "Raw Quartz",
  Desc_Sulfur_C: "Sulfur",
  Desc_LiquidOil_C: "Crude Oil",
  Desc_OreBauxite_C: "Bauxite",
  Desc_OreUranium_C: "Uranium",
  Desc_SAM_C: "SAM",
  Desc_NitrogenGas_C: "Nitrogen Gas",
  Desc_Water_C: "Water",
};

/** Friendly name for an item class: the catalog's display name when present,
 *  then a known raw-resource name, then a humanised class — never a raw
 *  `Desc_..._C`. Pass `gamedata.items` (values only need `displayName`). */
export function itemLabel(items: Record<string, { displayName?: string }>, cls: string): string {
  if (!cls) return "";
  return items[cls]?.displayName || RESOURCE_NAMES[cls] || prettyClass(cls);
}
